//! Session cost and token use, estimated from the transcripts the agents write: Claude's per-message `usage` (deduplicated by message id,
//! subagent transcripts included) and Codex's `token_count` totals, priced with a per-model
//! table (`assets/pricing.json`, overridable by `%APPDATA%\AI Terminal\pricing.json`).

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde_json::Value;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rates {
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write: f64,
    pub web_search: f64,
}

struct Provider {
    fallback: String,
    rules: Vec<(Vec<String>, String)>,
    models: HashMap<String, Rates>,
}

fn pricing() -> &'static HashMap<String, Provider> {
    static P: OnceLock<HashMap<String, Provider>> = OnceLock::new();
    P.get_or_init(|| {
        let user = std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("AI Terminal").join("pricing.json")).and_then(|p| std::fs::read_to_string(p).ok());
        let text = user.unwrap_or_else(|| include_str!("../assets/pricing.json").to_string());
        parse(&text).unwrap_or_else(|| parse(include_str!("../assets/pricing.json")).unwrap_or_default())
    })
}

fn parse(text: &str) -> Option<HashMap<String, Provider>> {
    let v: Value = serde_json::from_str(text).ok()?;
    let mut out = HashMap::new();
    for (name, p) in v["providers"].as_object()? {
        let num = |m: &Value, k: &str| m[k].as_f64().unwrap_or(0.0);
        let models = p["models"]
            .as_object()
            .map(|o| {
                o.iter()
                    .map(|(k, m)| {
                        let cache_read = m["cacheReadPerMillion"].as_f64().or(m["cachedInputPerMillion"].as_f64()).unwrap_or(0.0);
                        (k.clone(), Rates { input: num(m, "inputPerMillion"), output: num(m, "outputPerMillion"), cache_read, cache_write: num(m, "cacheWritePerMillion"), web_search: num(m, "webSearchPerRequest") })
                    })
                    .collect()
            })
            .unwrap_or_default();
        let rules = p["rules"]
            .as_array()
            .map(|a| a.iter().filter_map(|r| Some((r["match"].as_array()?.iter().filter_map(|m| m.as_str().map(str::to_string)).collect(), r["model"].as_str()?.to_string()))).collect())
            .unwrap_or_default();
        out.insert(name.clone(), Provider { fallback: p["fallbackTier"].as_str().unwrap_or_default().to_string(), rules, models });
    }
    Some(out)
}

/// `claude-opus-5-5-20260101` → `claude-opus-5.5-20260101` (Claude ids write versions with dashes).
fn normalize(model: &str) -> String {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    RE.get_or_init(|| regex::Regex::new(r"-(\d+)-(\d{1,2})($|[-\[])").unwrap()).replace(&model.to_lowercase(), "-$1.$2$3").to_string()
}

/// Rates for a model: the first rule whose strings are all tokens of the id, else the fallback.
pub fn rates(provider: &str, model: &str) -> Rates {
    let Some(p) = pricing().get(provider) else { return Rates::default() };
    let id = normalize(model);
    let tokens: Vec<&str> = id.split(|c: char| !(c.is_ascii_alphanumeric() || c == '.')).filter(|t| !t.is_empty()).collect();
    let tier = p.rules.iter().find(|(m, _)| m.iter().all(|x| tokens.contains(&x.as_str()))).map(|(_, t)| t.as_str()).unwrap_or(&p.fallback);
    p.models.get(tier).or_else(|| p.models.get(&p.fallback)).copied().unwrap_or_default()
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Usage {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub web_searches: u64,
}

impl Usage {
    fn max(self, o: Usage) -> Usage {
        Usage { input: self.input.max(o.input), output: self.output.max(o.output), cache_read: self.cache_read.max(o.cache_read), cache_write: self.cache_write.max(o.cache_write), web_searches: self.web_searches.max(o.web_searches) }
    }
    pub fn total(&self) -> u64 {
        self.input + self.output + self.cache_read + self.cache_write
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Summary {
    pub usd: f64,
    pub usage: Usage,
    /// Size of the latest request's prompt (system prompt, tools, memory, conversation): what
    /// is sent to the server with every request, in tokens.
    pub context: u64,
}

/// Reads a session's transcripts incrementally (only new bytes each time).
#[derive(Default)]
pub struct Tracker {
    offsets: HashMap<PathBuf, u64>,
    /// Claude: message id → (model, usage), max per field (streamed lines repeat the message).
    by_msg: HashMap<String, (String, Usage)>,
    /// Codex: latest totals and model.
    codex: Option<(String, Usage)>,
    codex_file: Option<PathBuf>,
    /// The latest request's prompt size (main conversation only, not subagents).
    context: u64,
    /// The model of the latest reply in the main conversation (Codex: the current turn's).
    model: String,
}

fn new_lines(path: &Path, offsets: &mut HashMap<PathBuf, u64>) -> Vec<String> {
    let Ok(mut f) = std::fs::File::open(path) else { return Vec::new() };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let off = offsets.get(path).copied().unwrap_or(0).min(len);
    if off == len || f.seek(SeekFrom::Start(off)).is_err() {
        return Vec::new();
    }
    let mut lines = Vec::new();
    let mut read = off;
    let mut r = BufReader::new(f);
    let mut buf = String::new();
    loop {
        buf.clear();
        match r.read_line(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                // A line still being written (no newline yet) is read next time.
                if !buf.ends_with('\n') {
                    break;
                }
                read += n as u64;
                lines.push(buf.trim_end().to_string());
            }
        }
    }
    offsets.insert(path.to_path_buf(), read);
    lines
}

impl Tracker {
    /// Claude: the session transcript plus its subagents' transcripts.
    pub fn update_claude(&mut self, cwd: &Path, id: &str) -> Summary {
        let Some(dir) = crate::agents::claude_dir(cwd) else { return Summary::default() };
        let mut files = vec![dir.join(format!("{id}.jsonl"))];
        if let Ok(rd) = std::fs::read_dir(dir.join(id).join("subagents")) {
            files.extend(rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "jsonl")));
        }
        for (i, f) in files.into_iter().enumerate() {
            for line in new_lines(&f, &mut self.offsets) {
                // /compact writes a boundary entry with the new prompt size; the next reply's
                // usage replaces it with the exact figure.
                if i == 0 && line.contains("\"compact_boundary\"") {
                    if let Ok(v) = serde_json::from_str::<Value>(&line) {
                        if let Some(n) = v["compactMetadata"]["postTokens"].as_u64().filter(|&n| n > 0) {
                            self.context = n;
                        }
                    }
                    continue;
                }
                if !line.contains("\"usage\"") {
                    continue;
                }
                let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
                let m = &v["message"];
                let u = &m["usage"];
                if !u.is_object() {
                    continue;
                }
                let key = m["id"].as_str().or(v["requestId"].as_str()).or(v["uuid"].as_str()).unwrap_or_default().to_string();
                let n = |k: &str| u[k].as_u64().unwrap_or(0);
                let usage = Usage {
                    input: n("input_tokens"),
                    output: n("output_tokens"),
                    cache_read: n("cache_read_input_tokens"),
                    cache_write: n("cache_creation_input_tokens"),
                    web_searches: u["server_tool_use"]["web_search_requests"].as_u64().unwrap_or(0),
                };
                let model = m["model"].as_str().unwrap_or_default().to_string();
                if model == "<synthetic>" {
                    continue;
                }
                // Lines come in order, so the main transcript's last one is the latest request.
                if i == 0 && v["isSidechain"].as_bool() != Some(true) {
                    let ctx = usage.input + usage.cache_read + usage.cache_write;
                    if ctx > 0 {
                        self.context = ctx;
                    }
                    if !model.is_empty() {
                        self.model = model.clone();
                    }
                }
                let e = self.by_msg.entry(key).or_insert((model, Usage::default()));
                e.1 = e.1.max(usage);
            }
        }
        let mut s = Summary { context: self.context, ..Default::default() };
        for (model, u) in self.by_msg.values() {
            let r = rates("claude", model);
            s.usd += (u.input as f64 * r.input + u.output as f64 * r.output + u.cache_read as f64 * r.cache_read + u.cache_write as f64 * r.cache_write) / 1e6 + u.web_searches as f64 * r.web_search;
            s.usage.input += u.input;
            s.usage.output += u.output;
            s.usage.cache_read += u.cache_read;
            s.usage.cache_write += u.cache_write;
            s.usage.web_searches += u.web_searches;
        }
        s
    }

    /// The model currently answering (from the latest reply), if seen yet.
    pub fn model(&self) -> Option<&str> {
        Some(self.model.as_str()).filter(|m| !m.is_empty())
    }

    /// Codex: the rollout's latest `total_token_usage` and model.
    pub fn update_codex(&mut self, id: &str) -> Summary {
        if self.codex_file.is_none() {
            self.codex_file = crate::agents::codex_rollout(id);
        }
        let Some(f) = self.codex_file.clone() else { return Summary::default() };
        for line in new_lines(&f, &mut self.offsets) {
            if line.contains("\"turn_context\"") {
                if let Ok(v) = serde_json::from_str::<Value>(&line) {
                    if let Some(m) = v["payload"]["model"].as_str() {
                        self.model = m.to_string();
                        let u = self.codex.take().map(|c| c.1).unwrap_or_default();
                        self.codex = Some((m.to_string(), u));
                    }
                }
            } else if line.contains("\"token_count\"") {
                if let Ok(v) = serde_json::from_str::<Value>(&line) {
                    let t = &v["payload"]["info"]["total_token_usage"];
                    if t.is_object() {
                        let n = |k: &str| t[k].as_u64().unwrap_or(0);
                        // OpenAI counts cached tokens inside input_tokens.
                        let cached = n("cached_input_tokens");
                        let u = Usage { input: n("input_tokens").saturating_sub(cached), output: n("output_tokens"), cache_read: cached, cache_write: n("cache_write_input_tokens"), web_searches: 0 };
                        let model = self.codex.take().map(|c| c.0).unwrap_or_default();
                        self.codex = Some((model, u));
                        // The latest request's prompt (OpenAI's input_tokens include the cached part).
                        let ctx = v["payload"]["info"]["last_token_usage"]["input_tokens"].as_u64().unwrap_or(0);
                        if ctx > 0 {
                            self.context = ctx;
                        }
                    }
                }
            }
        }
        let Some((model, u)) = &self.codex else { return Summary::default() };
        let r = rates("codex", model);
        Summary { usd: (u.input as f64 * r.input + u.output as f64 * r.output + u.cache_read as f64 * r.cache_read + u.cache_write as f64 * r.cache_write) / 1e6, usage: *u, context: self.context }
    }
}

/// A short display name for a model id: "claude-opus-5-5" → "Opus 5.5",
/// "claude-3-5-sonnet-20241022" → "Sonnet 3.5", "claude-sonnet-4-5[1m]" → "Sonnet 4.5 1M".
/// "gpt-6-astra" → "GPT-6 Astra", "gpt-5.1-codex-max" → "GPT-5.1 Codex Max". Other ids as they are.
pub fn model_label(id: &str) -> String {
    let cap = |w: &str| {
        let mut c = w.chars();
        c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
    };
    if let Some(rest) = id.strip_prefix("gpt-") {
        let mut parts = rest.split('-');
        let ver = parts.next().unwrap_or_default();
        return std::iter::once(format!("GPT-{ver}")).chain(parts.filter(|p| !p.is_empty()).map(cap)).collect::<Vec<_>>().join(" ");
    }
    let Some(rest) = id.strip_prefix("claude-") else { return id.to_string() };
    let (rest, ext) = match rest.find('[') {
        Some(i) => (&rest[..i], rest[i..].trim_matches(|c| c == '[' || c == ']').to_uppercase()),
        None => (rest, String::new()),
    };
    let (mut words, mut nums) = (Vec::new(), Vec::new());
    for part in rest.split('-') {
        if part.is_empty() || (part.len() >= 8 && part.chars().all(|c| c.is_ascii_digit())) {
            continue; // date stamp
        }
        if part.chars().all(|c| c.is_ascii_digit()) {
            nums.push(part);
        } else {
            words.push(cap(part));
        }
    }
    let mut out = words.join(" ");
    if !nums.is_empty() {
        out = format!("{out} {}", nums.join("."));
    }
    if !ext.is_empty() {
        out = format!("{out} {ext}");
    }
    if out.trim().is_empty() { id.to_string() } else { out.trim().to_string() }
}

/// "$1.23" / "$0.042" / "<$0.01".
pub fn fmt_usd(v: f64) -> String {
    if v <= 0.0 {
        "$0".into()
    } else if v < 0.01 {
        "<$0.01".into()
    } else if v < 10.0 {
        format!("${v:.2}")
    } else {
        format!("${v:.0}")
    }
}

/// 1234 → "1.2k", 3_400_000 → "3.4M".
pub fn fmt_tokens(n: u64) -> String {
    match n {
        0..=999 => n.to_string(),
        1_000..=999_999 => format!("{:.1}k", n as f64 / 1e3),
        _ => format!("{:.1}M", n as f64 / 1e6),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_labels() {
        assert_eq!(super::model_label("claude-opus-5-5"), "Opus 5.5");
        assert_eq!(super::model_label("claude-3-5-sonnet-20241022"), "Sonnet 3.5");
        assert_eq!(super::model_label("claude-opus-4-1-20250805"), "Opus 4.1");
        assert_eq!(super::model_label("claude-sonnet-4-5[1m]"), "Sonnet 4.5 1M");
        assert_eq!(super::model_label("claude-haiku-5-5"), "Haiku 5.5");
        assert_eq!(super::model_label("gpt-6-astra"), "GPT-6 Astra");
        assert_eq!(super::model_label("gpt-5.1-codex-max"), "GPT-5.1 Codex Max");
        assert_eq!(super::model_label("o3"), "o3");
    }

    #[test]
    fn model_ids_resolve_like_tt() {
        assert_eq!(normalize("claude-opus-5-5"), "claude-opus-5.5");
        assert_eq!(normalize("claude-sonnet-4-6-20260101"), "claude-sonnet-4.6-20260101");
        let opus55 = rates("claude", "claude-opus-5-5");
        assert_eq!((opus55.input, opus55.output), (4.0, 20.0));
        // Unknown ids fall back instead of costing nothing.
        assert!(rates("claude", "claude-unknown-model").input > 0.0);
        assert_eq!(fmt_tokens(1234), "1.2k");
        assert_eq!(fmt_usd(0.004), "<$0.01");
        assert_eq!(fmt_usd(1.234), "$1.23");
    }
}

#[cfg(test)]
mod live_check {
    /// Manual check against a real transcript: COST_CWD / COST_ID env vars (ignored by default).
    #[test]
    #[ignore]
    fn real_transcript() {
        let (cwd, id) = (std::env::var("COST_CWD").unwrap(), std::env::var("COST_ID").unwrap());
        let mut t = super::Tracker::default();
        let s = t.update_claude(std::path::Path::new(&cwd), &id);
        println!("CONTEXT {}", s.context);
        println!("COST {} tokens in {} out {} cache-read {} cache-write {}", super::fmt_usd(s.usd), s.usage.input, s.usage.output, s.usage.cache_read, s.usage.cache_write);
        let s2 = t.update_claude(std::path::Path::new(&cwd), &id);
        assert_eq!(s, s2, "a second read without new lines must not change the totals");
    }
}
