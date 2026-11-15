//! Agent CLIs (Claude Code, Codex) launched with command-line flags
//! for unrestricted mode / model / effort / extra parameters, conversation ids for resuming,
//! model catalogs for the "with settings" dialog, and session names read from transcripts.

use std::io::{BufRead, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug, Default)]
pub enum Kind {
    #[default]
    Shell,
    Claude,
    Codex,
}

/// Per-session launch settings (the "with settings" dialog), kept for Restart / resume.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Debug, Default)]
#[serde(default)]
pub struct LaunchOpts {
    /// Claude: `--dangerously-skip-permissions`; Codex: `--dangerously-bypass-approvals-and-sandbox`.
    pub unrestricted: bool,
    pub model: Option<String>,
    pub effort: Option<String>,
    /// Extra CLI parameters (split on whitespace, double quotes group words).
    pub cli_params: String,
    /// Claude: `[1m]` suffix on the model (1M context).
    pub one_m: bool,
    /// First prompt for a new session (handoff); not kept.
    #[serde(skip)]
    pub initial_prompt: Option<String>,
}

/// Splits CLI parameters: whitespace-separated, double quotes group words.
pub fn split_params(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut any = false;
    for c in s.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                any = true;
            }
            c if c.is_whitespace() && !quoted => {
                if any || !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                    any = false;
                }
            }
            c => cur.push(c),
        }
    }
    if any || !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Removes `flag value` / `flag=value` pairs (user --model / --effort are stripped before adding
/// the structured ones).
fn strip_flag(params: &mut Vec<String>, flags: &[&str]) {
    let mut i = 0;
    while i < params.len() {
        let p = &params[i];
        if flags.iter().any(|f| p == f) {
            params.remove(i);
            if i < params.len() && !params[i].starts_with('-') {
                params.remove(i);
            }
        } else if flags.iter().any(|f| p.starts_with(&format!("{f}="))) {
            params.remove(i);
        } else {
            i += 1;
        }
    }
}

/// When set, Claude sessions get `--append-system-prompt` describing AI Terminal's colour tags.
pub static COLOR_TAG_PROMPT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The hooks settings file Claude sessions get (`--settings`), so the app hears when a session
/// needs attention (see hooks.rs).
pub static HOOK_SETTINGS: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// AI Terminal's MCP server, for sessions: Claude gets `--mcp-config <file>` (the file reads the
/// URL / token / session key from the environment), Codex gets `-c mcp_servers.ai-terminal.*`.
pub static MCP_CONFIG: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);
pub static MCP_URL: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);
pub const MCP_TOKEN_ENV: &str = "AI_TERMINAL_MCP_TOKEN";
pub const MCP_URL_ENV: &str = "AI_TERMINAL_MCP_URL";

/// Writes the Claude MCP config (static: values come from the environment) and returns its path.
pub fn write_mcp_config() -> Option<PathBuf> {
    let cfg = serde_json::json!({
        "mcpServers": {
            crate::mcp::SERVER_NAME: {
                "type": "http",
                "url": format!("${{{MCP_URL_ENV}}}"),
                "headers": {
                    "Authorization": format!("Bearer ${{{MCP_TOKEN_ENV}}}"),
                    crate::mcp::SESSION_HEADER: format!("${{{}:-}}", crate::hooks::ENV_KEY)
                }
            }
        }
    });
    let dir = crate::hooks::events_dir().parent()?.to_path_buf();
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join("mcp.json");
    std::fs::write(&path, serde_json::to_string_pretty(&cfg).ok()?).ok()?;
    Some(path)
}

/// Arguments for a session (after the program's own base args). `session_key` identifies the
/// session to AI Terminal's MCP server.
pub fn build_args(kind: Kind, opts: &LaunchOpts, name: &str, resume: Option<&str>, session_key: &str) -> Vec<String> {
    let mut params = split_params(&opts.cli_params);
    let mut a: Vec<String> = Vec::new();
    match kind {
        Kind::Shell => a = params,
        Kind::Claude => {
            if opts.unrestricted {
                a.push("--dangerously-skip-permissions".into());
            } else if !params.iter().any(|p| p == "--permission-mode" || p.starts_with("--permission-mode=") || p == "--dangerously-skip-permissions") {
                // The default: auto permission mode.
                a.extend(["--permission-mode".into(), "auto".into()]);
            }
            if opts.model.is_some() {
                strip_flag(&mut params, &["--model"]);
            }
            if opts.effort.is_some() {
                strip_flag(&mut params, &["--effort"]);
            }
            a.extend(params);
            if let Some(m) = opts.model.as_deref().filter(|m| !m.is_empty()) {
                let m = if opts.one_m && !m.ends_with("[1m]") { format!("{m}[1m]") } else { m.to_string() };
                a.extend(["--model".into(), m]);
            }
            if let Some(e) = opts.effort.as_deref().filter(|e| !e.is_empty()) {
                a.extend(["--effort".into(), e.to_string()]);
            }
            // Our hooks file, unless the user passes their own --settings.
            if let Some(path) = HOOK_SETTINGS.lock().unwrap().clone() {
                if !a.iter().any(|x| x == "--settings" || x.starts_with("--settings=")) {
                    a.splice(0..0, ["--settings".to_string(), path]);
                }
            }
            if COLOR_TAG_PROMPT.load(std::sync::atomic::Ordering::Relaxed) && !a.iter().any(|x| x == "--append-system-prompt") {
                a.extend(["--append-system-prompt".into(), crate::colortags::PROMPT.into()]);
            }
            if !name.trim().is_empty() {
                a.extend(["-n".into(), name.trim().to_string()]);
            }
            if let Some(path) = MCP_CONFIG.lock().unwrap().clone() {
                a.extend(["--mcp-config".into(), path]);
            }
            if let Some(id) = resume {
                a.extend(["--resume".into(), id.to_string()]);
            } else if let Some(p) = opts.initial_prompt.as_deref().filter(|p| !p.trim().is_empty()) {
                a.push(p.to_string());
            }
        }
        Kind::Codex => {
            if let Some(id) = resume {
                a.extend(["resume".into(), id.to_string()]);
            }
            if let Some(url) = MCP_URL.lock().unwrap().clone() {
                let s = crate::mcp::SERVER_NAME;
                a.extend(["-c".into(), format!("mcp_servers.{s}.url={url}")]);
                a.extend(["-c".into(), format!("mcp_servers.{s}.bearer_token_env_var={MCP_TOKEN_ENV}")]);
                a.extend(["-c".into(), format!("mcp_servers.{s}.http_headers.{}={session_key}", crate::mcp::SESSION_HEADER)]);
            }
            if opts.unrestricted && !params.iter().any(|p| p == "--dangerously-bypass-approvals-and-sandbox") {
                a.push("--dangerously-bypass-approvals-and-sandbox".into());
            }
            if opts.model.is_some() {
                strip_flag(&mut params, &["--model", "-m"]);
            }
            a.extend(params);
            if let Some(m) = opts.model.as_deref().filter(|m| !m.is_empty()) {
                a.extend(["--model".into(), m.to_string()]);
            }
            if let Some(e) = opts.effort.as_deref().filter(|e| !e.is_empty()) {
                a.extend(["-c".into(), format!("model_reasoning_effort=\"{e}\"")]);
            }
            if resume.is_none() {
                if let Some(p) = opts.initial_prompt.as_deref().filter(|p| !p.trim().is_empty()) {
                    a.extend(["--".into(), p.to_string()]);
                }
            }
            if resume.is_none() {
                if let Some(p) = opts.initial_prompt.as_deref().filter(|p| !p.trim().is_empty()) {
                    a.extend(["--".into(), p.to_string()]);
                }
            }
        }
    }
    a
}

// ---------------------------------------------------------------------------------------------
// Model catalogs (for the "with settings" dialog), loaded once in the background.

#[derive(Clone, Default, Debug)]
pub struct ModelInfo {
    pub id: String,
    pub label: String,
    pub efforts: Vec<String>,
    pub default_effort: Option<String>,
}

#[derive(Clone, Default, Debug)]
pub struct Catalog {
    pub claude_models: Vec<ModelInfo>,
    pub claude_efforts: Vec<String>,
    pub codex_models: Vec<ModelInfo>,
}

fn run_quiet(program: &str, args: &[&str]) -> Option<String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let out = std::process::Command::new(program).args(args).creation_flags(CREATE_NO_WINDOW).stdin(std::process::Stdio::null()).output().ok()?;
    Some(String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr))
}

/// Parses `claude --help`: quoted aliases in the --model block, the (…) list of --effort.
pub fn parse_claude_help(help: &str) -> (Vec<String>, Vec<String>) {
    let block = |flag: &str| -> String {
        let mut lines = help.lines().skip_while(|l| !l.trim_start().starts_with(flag));
        let mut s = String::new();
        if let Some(first) = lines.next() {
            s.push_str(first);
            for l in lines {
                if l.trim_start().starts_with('-') {
                    break;
                }
                s.push(' ');
                s.push_str(l.trim());
            }
        }
        s
    };
    let model_block = block("--model");
    let mut models = Vec::new();
    let mut rest = model_block.as_str();
    while let Some(i) = rest.find('\'') {
        let after = &rest[i + 1..];
        let Some(j) = after.find('\'') else { break };
        let m = &after[..j];
        if !m.is_empty() && m.chars().all(|c| c.is_ascii_alphanumeric() || "-._[]".contains(c)) && !models.contains(&m.to_string()) {
            models.push(m.to_string());
        }
        rest = &after[j + 1..];
    }
    let effort_block = block("--effort");
    let efforts = match (effort_block.find('('), effort_block.find(')')) {
        (Some(a), Some(b)) if b > a => effort_block[a + 1..b].split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect(),
        _ => Vec::new(),
    };
    (models, efforts)
}

pub fn parse_codex_models(json: &str) -> Vec<ModelInfo> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json.trim()) else { return Vec::new() };
    v["models"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter(|m| m["visibility"].as_str().is_none_or(|s| s == "list"))
                .filter_map(|m| {
                    let id = m["slug"].as_str()?.to_string();
                    Some(ModelInfo {
                        label: m["display_name"].as_str().unwrap_or(&id).to_string(),
                        efforts: m["supported_reasoning_levels"].as_array().map(|l| l.iter().filter_map(|e| e["effort"].as_str().map(str::to_string)).collect()).unwrap_or_default(),
                        default_effort: m["default_reasoning_level"].as_str().map(str::to_string),
                        id,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

pub fn load_catalog(claude: Option<String>, codex: Option<String>) -> Catalog {
    let mut c = Catalog::default();
    if let Some(help) = claude.and_then(|p| run_quiet(&p, &["--help"])) {
        let (models, efforts) = parse_claude_help(&help);
        c.claude_models = models.into_iter().map(|m| ModelInfo { label: m.clone(), id: m, efforts: efforts.clone(), default_effort: None }).collect();
        c.claude_efforts = efforts;
    }
    if let Some(json) = codex.and_then(|p| run_quiet(&p, &["debug", "models"])) {
        c.codex_models = parse_codex_models(&json);
    }
    c
}

// ---------------------------------------------------------------------------------------------
// Transcripts: conversation ids and names

fn home() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from)
}

/// `~/.claude/projects/<cwd with every non-alphanumeric character as '-'>`.
pub fn claude_dir(cwd: &Path) -> Option<PathBuf> {
    let mangled: String = cwd.display().to_string().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    Some(home()?.join(".claude").join("projects").join(mangled))
}

/// The oldest Claude transcript in `cwd`'s folder created since `since`, not in `claimed`.
/// Claude writes it with the first message, so callers poll.
pub fn find_claude_id(cwd: &Path, since: SystemTime, claimed: &std::collections::HashSet<String>) -> Option<String> {
    let rd = std::fs::read_dir(claude_dir(cwd)?).ok()?;
    let since = since - std::time::Duration::from_secs(2);
    let mut best: Option<(SystemTime, String)> = None;
    for e in rd.flatten() {
        let p = e.path();
        if p.extension().and_then(|x| x.to_str()) != Some("jsonl") {
            continue;
        }
        let Some(stem) = p.file_stem().and_then(|x| x.to_str()).map(str::to_string) else { continue };
        if claimed.contains(&stem) {
            continue;
        }
        let Ok(m) = e.metadata() else { continue };
        let created = m.created().or_else(|_| m.modified()).unwrap_or(std::time::UNIX_EPOCH);
        if created >= since && best.as_ref().is_none_or(|(t, _)| created < *t) {
            best = Some((created, stem));
        }
    }
    best.map(|(_, id)| id)
}

fn codex_sessions() -> Option<PathBuf> {
    let base = std::env::var_os("CODEX_HOME").map(PathBuf::from).or_else(|| home().map(|h| h.join(".codex")))?;
    Some(base.join("sessions"))
}

/// Codex rollout files (`sessions/YYYY/MM/DD/rollout-*.jsonl`) modified since `since`.
fn codex_rollouts_since(since: SystemTime) -> Vec<PathBuf> {
    let Some(root) = codex_sessions() else { return Vec::new() };
    let mut out = Vec::new();
    let newer = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).is_ok_and(|t| t >= since);
    // Only descend into recently touched year / month / day folders.
    for y in std::fs::read_dir(&root).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| newer(p)) {
        for m in std::fs::read_dir(&y).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| newer(p)) {
            for d in std::fs::read_dir(&m).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| newer(p)) {
                out.extend(std::fs::read_dir(&d).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.extension().and_then(|x| x.to_str()) == Some("jsonl")));
            }
        }
    }
    out
}

fn first_line(p: &Path) -> Option<String> {
    let f = std::fs::File::open(p).ok()?;
    let mut line = String::new();
    std::io::BufReader::new(f).take(1 << 20).read_line(&mut line).ok()?;
    Some(line)
}

/// The Codex conversation started in `cwd` since `since` (oldest unclaimed rollout whose
/// session_meta matches).
pub fn find_codex_id(cwd: &Path, since: SystemTime, claimed: &std::collections::HashSet<String>) -> Option<String> {
    let since = since - std::time::Duration::from_secs(2);
    let want = cwd.display().to_string().to_lowercase();
    let mut best: Option<(SystemTime, String)> = None;
    for p in codex_rollouts_since(since) {
        let Some(line) = first_line(&p) else { continue };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else { continue };
        let meta = &v["payload"];
        let (Some(id), Some(c)) = (meta["id"].as_str(), meta["cwd"].as_str()) else { continue };
        if claimed.contains(id) || c.to_lowercase().trim_end_matches('\\') != want.trim_end_matches('\\') {
            continue;
        }
        let created = std::fs::metadata(&p).and_then(|m| m.created()).unwrap_or(std::time::UNIX_EPOCH);
        if created >= since && best.as_ref().is_none_or(|(t, _)| created < *t) {
            best = Some((created, id.to_string()));
        }
    }
    best.map(|(_, id)| id)
}

fn tidy_prompt(s: &str) -> Option<String> {
    let s = s.trim();
    if s.is_empty() || s.starts_with('<') {
        return None;
    }
    let line = s.lines().next().unwrap_or(s).trim();
    let mut out: String = line.chars().take(70).collect();
    if line.chars().count() > 70 {
        out.push('…');
    }
    Some(out)
}

/// A Claude session's name: its `custom-title` (set by /rename, `-n` or Claude's auto-title),
/// else its first prompt.
pub fn claude_title(cwd: &Path, id: &str) -> Option<String> {
    let path = claude_dir(cwd)?.join(format!("{id}.jsonl"));
    let mut f = std::fs::File::open(&path).ok()?;
    let len = f.metadata().ok()?.len();
    // custom-title records are appended; look at the tail.
    let tail = 512 * 1024u64;
    f.seek(SeekFrom::Start(len.saturating_sub(tail))).ok()?;
    let mut buf = String::new();
    let _ = f.take(tail).read_to_string(&mut buf);
    let mut title = None;
    for line in buf.lines().filter(|l| l.contains("\"custom-title\"")) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
            if let Some(t) = v["customTitle"].as_str().filter(|t| !t.trim().is_empty()) {
                title = Some(t.trim().to_string());
            }
        }
    }
    if title.is_some() {
        return title;
    }
    let f = std::fs::File::open(&path).ok()?;
    for line in std::io::BufReader::new(f).take(4 << 20).lines().map_while(Result::ok) {
        if !line.contains("\"type\":\"user\"") {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else { continue };
        let c = &v["message"]["content"];
        let text = c.as_str().map(str::to_string).or_else(|| c.as_array().and_then(|a| a.iter().find(|p| p["type"] == "text").and_then(|p| p["text"].as_str()).map(str::to_string)));
        if let Some(t) = text.as_deref().and_then(tidy_prompt) {
            return Some(t);
        }
    }
    None
}

/// The rollout file of a Codex conversation (its name ends with the id).
pub fn codex_rollout(id: &str) -> Option<PathBuf> {
    let root = codex_sessions()?;
    for y in std::fs::read_dir(&root).ok()?.flatten() {
        for m in std::fs::read_dir(y.path()).into_iter().flatten().flatten() {
            for d in std::fs::read_dir(m.path()).into_iter().flatten().flatten() {
                for f in std::fs::read_dir(d.path()).into_iter().flatten().flatten() {
                    if f.file_name().to_string_lossy().ends_with(&format!("{id}.jsonl")) {
                        return Some(f.path());
                    }
                }
            }
        }
    }
    None
}

/// A Codex session's name: its first user message.
pub fn codex_title(id: &str) -> Option<String> {
    let root = codex_sessions()?;
    // Find the rollout by id (file name ends with the id).
    let mut found = None;
    'outer: for y in std::fs::read_dir(&root).ok()?.flatten() {
        for m in std::fs::read_dir(y.path()).into_iter().flatten().flatten() {
            for d in std::fs::read_dir(m.path()).into_iter().flatten().flatten() {
                for f in std::fs::read_dir(d.path()).into_iter().flatten().flatten() {
                    if f.file_name().to_string_lossy().ends_with(&format!("{id}.jsonl")) {
                        found = Some(f.path());
                        break 'outer;
                    }
                }
            }
        }
    }
    let f = std::fs::File::open(found?).ok()?;
    for line in std::io::BufReader::new(f).take(4 << 20).lines().map_while(Result::ok) {
        if !line.contains("\"role\":\"user\"") {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else { continue };
        if let Some(parts) = v["payload"]["content"].as_array() {
            for p in parts {
                if let Some(t) = p["text"].as_str().and_then(tidy_prompt) {
                    return Some(t);
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn params_split_and_flags() {
        assert_eq!(split_params(r#"--add-dir "C:\My Dir" -v"#), vec!["--add-dir", r"C:\My Dir", "-v"]);
        let o = LaunchOpts { unrestricted: false, model: Some("opus".into()), effort: Some("high".into()), cli_params: "--model sonnet --verbose".into(), one_m: true, initial_prompt: None };
        assert_eq!(build_args(Kind::Claude, &o, "Fix build", Some("abc"), "k"), vec!["--permission-mode", "auto", "--verbose", "--model", "opus[1m]", "--effort", "high", "-n", "Fix build", "--resume", "abc"]);
        let u = LaunchOpts { unrestricted: true, ..Default::default() };
        assert_eq!(build_args(Kind::Claude, &u, "", None, "k"), vec!["--dangerously-skip-permissions"]);
        let c = LaunchOpts { unrestricted: true, model: Some("gpt-x".into()), effort: Some("low".into()), ..Default::default() };
        assert_eq!(build_args(Kind::Codex, &c, "", Some("id1"), "k"), vec!["resume", "id1", "--dangerously-bypass-approvals-and-sandbox", "--model", "gpt-x", "-c", "model_reasoning_effort=\"low\""]);
    }

    #[test]
    fn claude_help_parsing() {
        let help = "  --effort <level>        Effort level for the current session\n                          (low, medium, high, xhigh, max)\n  --model <model>         Model for the current session. Provide\n                          an alias for the latest model (e.g.\n                          'fable', 'opus', or 'sonnet') or a\n                          model's full name.\n  -n, --name <name>       Set a display name\n";
        let (m, e) = parse_claude_help(help);
        assert_eq!(m, vec!["fable", "opus", "sonnet"]);
        assert_eq!(e, vec!["low", "medium", "high", "xhigh", "max"]);
    }
}
