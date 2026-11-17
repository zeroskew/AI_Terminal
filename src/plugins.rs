//! Plugins browser for Claude Code and Codex.
//!
//! Claude Code: the catalogue comes from the
//! marketplaces Claude Code already knows (`~/.claude/plugins/marketplaces/*/.claude-plugin/
//! marketplace.json`), installed state from `~/.claude/plugins/installed_plugins.json`, and
//! install / uninstall run Claude Code's own commands:
//!
//!     claude plugin install <name>@<marketplace> --scope user|project|local
//!     claude plugin uninstall <name>@<marketplace> --scope <scope> --prune -y
//!     claude plugin marketplace update            (refresh)
//!
//! Codex: catalogue and installed state come from `codex plugin list --available --json`, the
//! sources from `codex plugin marketplace list`, and descriptions from the plugins'
//! `.codex-plugin/plugin.json` where a local copy exists (local marketplaces, the curated
//! checkout, the install cache). Install / uninstall / refresh:
//!
//!     codex plugin add <name>@<marketplace>
//!     codex plugin remove <name>@<marketplace>
//!     codex plugin marketplace upgrade            (refresh)

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, Default)]
pub struct Plugin {
    pub name: String,
    pub marketplace: String,
    pub description: String,
    pub version: String,
    pub author: String,
    pub keywords: Vec<String>,
    pub category: String,
    pub source: String,
    pub source_ref: String,
    pub homepage: String,
    /// `name@marketplace` and the lowercased search text, filled in by `finish`.
    id: String,
    hay: String,
}

impl Plugin {
    /// `name@marketplace`, the id the CLIs (and installed_plugins.json) use.
    pub fn id(&self) -> &str {
        &self.id
    }
    fn finish(&mut self) {
        self.id = format!("{}@{}", self.name, self.marketplace);
        self.hay = [&self.name, &self.description, &self.category, &self.author, &self.marketplace].iter().map(|s| s.to_lowercase()).chain(self.keywords.iter().map(|k| k.to_lowercase())).collect::<Vec<_>>().join("\n");
    }
    pub fn is_bundle(&self) -> bool {
        self.category.eq_ignore_ascii_case("bundle")
    }
    /// `filter` must already be lowercase.
    pub fn matches(&self, filter: &str) -> bool {
        filter.is_empty() || self.hay.contains(filter)
    }
}

#[derive(Clone, Debug)]
pub struct Installed {
    /// user / project / local for Claude Code; empty for Codex (no scopes).
    pub scope: String,
    pub version: String,
    pub enabled: bool,
}

fn plugins_dir() -> Option<PathBuf> {
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"))?;
    Some(PathBuf::from(home).join(".claude").join("plugins"))
}

fn s(v: &serde_json::Value) -> String {
    v.as_str().unwrap_or_default().to_string()
}

/// A plugin source (marketplace) Claude Code knows: its name and where it comes from.
#[derive(Clone, Debug)]
pub struct Market {
    pub name: String,
    /// GitHub owner/repo, git URL or local path.
    pub source: String,
    pub kind: String,
}

/// The configured marketplaces (`~/.claude/plugins/known_marketplaces.json`), by name.
pub fn markets() -> Vec<Market> {
    let Some(dir) = plugins_dir() else { return Vec::new() };
    let known: serde_json::Value = std::fs::read_to_string(dir.join("known_marketplaces.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    let mut v: Vec<Market> = known
        .as_object()
        .map(|o| {
            o.iter()
                .map(|(name, m)| {
                    let src = &m["source"];
                    let kind = s(&src["source"]);
                    let source = src["repo"].as_str().or(src["url"].as_str()).or(src["path"].as_str()).map(str::to_string).unwrap_or_else(|| kind.clone());
                    Market { name: name.clone(), source, kind }
                })
                .collect()
        })
        .unwrap_or_default();
    // Claude Code's built-in Anthropic directory (not in known_marketplaces.json).
    if dir.join(DIRECTORY_CACHE).is_file() {
        v.push(Market { name: DIRECTORY.into(), source: "Anthropic Directory (built in)".into(), kind: "built-in".into() });
    }
    v.sort_by(|a, b| a.name.cmp(&b.name));
    v
}

/// The built-in marketplace and the cache Claude Code keeps of it.
pub const DIRECTORY: &str = "anthropic-plugin-directory";
const DIRECTORY_CACHE: &str = "plugin-directory-cache-v2.json";

/// Listings of the built-in directory that work in Claude Code; installed as
/// `<entry name>@anthropic-plugin-directory`.
fn directory_plugins(dir: &Path) -> Vec<Plugin> {
    let Ok(text) = std::fs::read_to_string(dir.join(DIRECTORY_CACHE)) else { return Vec::new() };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else { return Vec::new() };
    let names = &v["entryNames"];
    v["listings"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|l| l["surfaces"].as_array().is_none_or(|a| a.iter().any(|x| x == "claude_code")))
        .filter_map(|l| {
            let id = l["id"].as_str()?;
            let name = names[id].as_str()?.to_string();
            let title = s(&l["name"]);
            let summary = s(&l["summary"]);
            Some(Plugin {
                description: if !title.is_empty() && !title.eq_ignore_ascii_case(&name) { format!("{title}: {summary}") } else { summary },
                version: s(&l["release"]["version"]),
                author: l["publisher"]["name"].as_str().unwrap_or_default().to_string(),
                source: s(&l["source"]["repository"]["url"]),
                homepage: l["links"].as_object().and_then(|o| o.values().find_map(|x| x.as_str())).unwrap_or_default().to_string(),
                marketplace: DIRECTORY.into(),
                name,
                ..Default::default()
            })
        })
        .collect()
}

/// Every plugin of every known marketplace, bundles first, then by name.
pub fn catalogue() -> Vec<Plugin> {
    let mut out = Vec::new();
    let Some(dir) = plugins_dir() else { return out };
    let known: serde_json::Value = std::fs::read_to_string(dir.join("known_marketplaces.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    let mut locations: Vec<(String, PathBuf)> = known
        .as_object()
        .map(|o| o.iter().map(|(k, v)| (k.clone(), v["installLocation"].as_str().map(PathBuf::from).unwrap_or_else(|| dir.join("marketplaces").join(k)))).collect())
        .unwrap_or_default();
    locations.sort();
    for (market, loc) in locations {
        let Ok(text) = std::fs::read_to_string(loc.join(".claude-plugin").join("marketplace.json")) else { continue };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else { continue };
        for p in v["plugins"].as_array().into_iter().flatten() {
            let src = &p["source"];
            let (source, source_ref) = if let Some(t) = src.as_str() { (t.to_string(), String::new()) } else { (src["url"].as_str().or(src["repo"].as_str()).unwrap_or_default().to_string(), s(&src["ref"])) };
            out.push(Plugin {
                name: s(&p["name"]),
                marketplace: market.clone(),
                description: s(&p["description"]),
                version: s(&p["version"]),
                author: p["author"]["name"].as_str().map(str::to_string).unwrap_or_else(|| s(&p["author"])),
                keywords: p["keywords"].as_array().map(|a| a.iter().filter_map(|k| k.as_str().map(str::to_string)).collect()).unwrap_or_default(),
                category: s(&p["category"]),
                source,
                source_ref,
                homepage: p["homepage"].as_str().or(p["repository"].as_str()).unwrap_or_default().to_string(),
                ..Default::default()
            });
        }
    }
    out.extend(directory_plugins(&dir));
    out.iter_mut().for_each(Plugin::finish);
    out.sort_by(|a, b| b.is_bundle().cmp(&a.is_bundle()).then_with(|| a.name.cmp(&b.name)));
    out
}

/// Installed plugins: id → installations (scope, version, project for project/local scope).
pub fn installed() -> HashMap<String, Vec<Installed>> {
    let mut out = HashMap::new();
    let Some(dir) = plugins_dir() else { return out };
    let Ok(text) = std::fs::read_to_string(dir.join("installed_plugins.json")) else { return out };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else { return out };
    if let Some(o) = v["plugins"].as_object() {
        for (id, list) in o {
            let entries = list
                .as_array()
                .map(|a| a.iter().map(|e| Installed { scope: s(&e["scope"]), version: s(&e["version"]), enabled: true }).collect())
                .unwrap_or_default();
            out.insert(id.clone(), entries);
        }
    }
    out
}

/// A background `claude plugin …` run, with its output.
#[derive(Default)]
pub struct Job {
    pub running: bool,
    pub log: Vec<String>,
    pub last_ok: Option<bool>,
}

pub type JobRef = Arc<Mutex<Job>>;

/// Runs `<cli> <args>` (`name` is the CLI's name for the log) in `cwd` on a thread, streaming
/// output into the job's log.
pub fn run(job: &JobRef, name: &str, program: &str, base_args: &[String], args: Vec<String>, cwd: &Path, ctx: egui::Context) {
    {
        let mut j = job.lock().unwrap();
        if j.running {
            return;
        }
        j.running = true;
        j.log.push(format!("> {name} {}", args.join(" ")));
    }
    let name = name.to_string();
    let job = job.clone();
    let program = program.to_string();
    let mut all_args: Vec<String> = base_args.to_vec();
    all_args.extend(args);
    let cwd = cwd.to_path_buf();
    std::thread::spawn(move || {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let child = std::process::Command::new(&program)
            .args(&all_args)
            .current_dir(&cwd)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .creation_flags(CREATE_NO_WINDOW)
            .spawn();
        let ok = match child {
            Ok(mut c) => {
                let pipe = |r: Box<dyn std::io::Read + Send>, job: JobRef, ctx: egui::Context| {
                    std::thread::spawn(move || {
                        for line in BufReader::new(r).lines().map_while(Result::ok) {
                            job.lock().unwrap().log.push(line);
                            ctx.request_repaint();
                        }
                    })
                };
                let a = pipe(Box::new(c.stdout.take().unwrap()), job.clone(), ctx.clone());
                let b = pipe(Box::new(c.stderr.take().unwrap()), job.clone(), ctx.clone());
                let status = c.wait();
                let _ = (a.join(), b.join());
                status.is_ok_and(|s| s.success())
            }
            Err(e) => {
                job.lock().unwrap().log.push(format!("Couldn't run {name}: {e}"));
                false
            }
        };
        let mut j = job.lock().unwrap();
        j.log.push(if ok { "Done.".into() } else { "Failed (see the output above).".into() });
        j.running = false;
        j.last_ok = Some(ok);
        drop(j);
        ctx.request_repaint();
    });
}

// -------------------------------------------------------------------------------------------
// Codex

/// What the Codex side of the window shows, loaded by `codex_load`.
#[derive(Default)]
pub struct Loaded {
    pub catalogue: Vec<Plugin>,
    pub installed: HashMap<String, Vec<Installed>>,
    pub markets: Vec<Market>,
}

/// Marketplaces Codex manages itself (bundled with the app, its runtime, the remote catalogue);
/// they can't be removed with `codex plugin marketplace remove`.
pub fn codex_builtin_market(name: &str) -> bool {
    name.starts_with("openai-")
}

fn run_capture(program: &str, base_args: &[String], args: &[&str]) -> Result<String, String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let out = std::process::Command::new(program)
        .args(base_args)
        .args(args)
        .stdin(std::process::Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("Couldn't run codex: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(if err.is_empty() { format!("codex {} failed", args.join(" ")) } else { err });
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// `codex plugin marketplace list`: a MARKETPLACE / ROOT table.
fn parse_codex_markets(text: &str) -> Vec<(String, String)> {
    text.lines()
        .skip_while(|l| !l.trim_start().starts_with("MARKETPLACE"))
        .skip(1)
        .filter_map(|l| {
            let (name, root) = l.trim().split_once(char::is_whitespace)?;
            Some((name.to_string(), root.trim().to_string()))
        })
        .collect()
}

/// Details a plugin's `.codex-plugin/plugin.json` adds to the CLI listing.
#[derive(Default, Clone)]
struct CodexMeta {
    description: String,
    author: String,
    category: String,
    keywords: Vec<String>,
    homepage: String,
}

fn read_codex_meta(plugin_dir: &Path, category: &str) -> Option<CodexMeta> {
    let text = std::fs::read_to_string(plugin_dir.join(".codex-plugin").join("plugin.json")).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    let i = &v["interface"];
    let name = s(&v["name"]);
    let title = s(&i["displayName"]);
    let short = [&i["shortDescription"], &v["description"]].into_iter().map(s).find(|x| !x.is_empty()).unwrap_or_default();
    let long = s(&i["longDescription"]);
    let mut description = if !title.is_empty() && !title.eq_ignore_ascii_case(&name) { format!("{title}: {short}") } else { short.clone() };
    if !long.is_empty() && long != short {
        description = format!("{description}\n\n{long}");
    }
    let author = [&v["author"]["name"], &i["developerName"], &v["author"]].into_iter().map(s).find(|x| !x.is_empty()).unwrap_or_default();
    let homepage = [&v["homepage"], &i["websiteURL"], &v["repository"]].into_iter().map(s).find(|x| !x.is_empty()).unwrap_or_default();
    let category = if category.is_empty() { s(&i["category"]) } else { category.to_string() };
    let keywords = v["keywords"].as_array().map(|a| a.iter().filter_map(|k| k.as_str().map(str::to_string)).collect()).unwrap_or_default();
    Some(CodexMeta { description, author, category, keywords, homepage })
}

/// Plugin details from local copies: id → meta and name → meta (remote catalogue entries have
/// no local copy of their own, but the curated checkout often has the same plugin by name).
fn codex_meta(roots: &[(String, String)]) -> (HashMap<String, CodexMeta>, HashMap<String, CodexMeta>) {
    let (mut by_id, mut by_name) = (HashMap::new(), HashMap::new());
    for (market, root) in roots {
        let root = PathBuf::from(root.trim_start_matches(r"\\?\"));
        let Ok(text) = std::fs::read_to_string(root.join(".agents").join("plugins").join("marketplace.json")) else { continue };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else { continue };
        for p in v["plugins"].as_array().into_iter().flatten() {
            let name = s(&p["name"]);
            let path = p["source"]["path"].as_str().map(str::to_string).unwrap_or_else(|| format!("./plugins/{name}"));
            if let Some(m) = read_codex_meta(&root.join(path.trim_start_matches("./")), p["category"].as_str().unwrap_or_default()) {
                by_name.entry(name.clone()).or_insert_with(|| m.clone());
                by_id.insert(format!("{name}@{market}"), m);
            }
        }
    }
    // The install cache: <market>/<plugin>/<version>/.codex-plugin/plugin.json.
    if let Some(cache) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(|h| PathBuf::from(h).join(".codex").join("plugins").join("cache")) {
        for market in std::fs::read_dir(cache).into_iter().flatten().flatten() {
            let mname = market.file_name().to_string_lossy().into_owned();
            for plugin in std::fs::read_dir(market.path()).into_iter().flatten().flatten() {
                let pname = plugin.file_name().to_string_lossy().into_owned();
                let id = format!("{pname}@{mname}");
                if by_id.contains_key(&id) {
                    continue;
                }
                let meta = std::fs::read_dir(plugin.path()).into_iter().flatten().flatten().filter(|d| d.path().is_dir()).find_map(|d| read_codex_meta(&d.path(), ""));
                if let Some(m) = meta {
                    by_name.entry(pname).or_insert_with(|| m.clone());
                    by_id.insert(id, m);
                }
            }
        }
    }
    (by_id, by_name)
}

/// Parses `codex plugin list --available --json` into the catalogue and the installed state.
fn parse_codex_list(json: &str, by_id: &HashMap<String, CodexMeta>, by_name: &HashMap<String, CodexMeta>) -> Result<(Vec<Plugin>, HashMap<String, Vec<Installed>>), String> {
    let v: serde_json::Value = serde_json::from_str(json).map_err(|e| format!("Couldn't read the codex plugin list: {e}"))?;
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    let mut installed = HashMap::new();
    for p in v["installed"].as_array().into_iter().flatten().chain(v["available"].as_array().into_iter().flatten()) {
        let (name, marketplace) = (s(&p["name"]), s(&p["marketplaceName"]));
        if name.is_empty() {
            continue;
        }
        let id = format!("{name}@{marketplace}");
        if p["installed"].as_bool() == Some(true) {
            installed.entry(id.clone()).or_insert_with(|| vec![Installed { scope: String::new(), version: s(&p["version"]), enabled: p["enabled"].as_bool().unwrap_or(true) }]);
        }
        if !seen.insert(id.clone()) {
            continue;
        }
        let src = &p["source"];
        let source = match src["source"].as_str() {
            Some("remote") => format!("{marketplace} (remote)"),
            _ => src["path"].as_str().or(src["url"].as_str()).unwrap_or_default().to_string(),
        };
        let meta = by_id.get(&id).or_else(|| by_name.get(&name)).cloned().unwrap_or_default();
        let mut plugin = Plugin { name, marketplace, version: s(&p["version"]), source, description: meta.description, author: meta.author, category: meta.category, keywords: meta.keywords, homepage: meta.homepage, ..Default::default() };
        plugin.finish();
        out.push(plugin);
    }
    // Installed first, then the ones with a description, then by name.
    out.sort_by(|a, b| installed.contains_key(&b.id).cmp(&installed.contains_key(&a.id)).then_with(|| a.description.is_empty().cmp(&b.description.is_empty())).then_with(|| a.name.cmp(&b.name)));
    Ok((out, installed))
}

/// Loads the Codex catalogue, installed state and sources (runs the codex CLI; call it off the UI thread).
pub fn codex_load(program: &str, base_args: &[String]) -> Result<Loaded, String> {
    let roots = parse_codex_markets(&run_capture(program, base_args, &["plugin", "marketplace", "list"]).unwrap_or_default());
    let json = run_capture(program, base_args, &["plugin", "list", "--available", "--json"])?;
    let (by_id, by_name) = codex_meta(&roots);
    let (catalogue, installed) = parse_codex_list(&json, &by_id, &by_name)?;
    let mut markets: Vec<Market> = roots.iter().map(|(name, root)| Market { name: name.clone(), source: root.trim_start_matches(r"\\?\").to_string(), kind: "local".into() }).collect();
    for p in &catalogue {
        if !markets.iter().any(|m| m.name == p.marketplace) {
            markets.push(Market { name: p.marketplace.clone(), source: "OpenAI plugin directory".into(), kind: "remote".into() });
        }
    }
    markets.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(Loaded { catalogue, installed, markets })
}

/// A background `codex_load`; the slot is filled when it finishes.
pub type LoadRef = Arc<Mutex<Option<Result<Loaded, String>>>>;

pub fn codex_load_async(program: &str, base_args: &[String], ctx: egui::Context) -> LoadRef {
    let slot: LoadRef = Arc::default();
    let (out, program, base_args) = (slot.clone(), program.to_string(), base_args.to_vec());
    std::thread::spawn(move || {
        let r = codex_load(&program, &base_args);
        *out.lock().unwrap() = Some(r);
        ctx.request_repaint();
    });
    slot
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_local_catalogue() {
        // Uses this machine's Claude Code marketplaces when present (skips otherwise).
        let c = catalogue();
        if c.is_empty() {
            return;
        }
        assert!(c.iter().all(|p| !p.name.is_empty() && !p.marketplace.is_empty()));
        // Bundles sort first.
        let first_plugin = c.iter().position(|p| !p.is_bundle()).unwrap_or(c.len());
        assert!(c[first_plugin..].iter().all(|p| !p.is_bundle()));
        assert!(c.iter().all(|p| p.id() == format!("{}@{}", p.name, p.marketplace)));
    }

    #[test]
    fn parses_codex_markets() {
        let m = parse_codex_markets("MARKETPLACE             ROOT\nopenai-bundled          C:\\x\\openai bundled\nlocal-one  D:\\p\n");
        assert_eq!(m, vec![("openai-bundled".to_string(), "C:\\x\\openai bundled".to_string()), ("local-one".to_string(), "D:\\p".to_string())]);
    }

    #[test]
    fn parses_codex_list() {
        let json = r#"{"installed":[{"name":"pdf","marketplaceName":"rt","version":"1.0","installed":true,"enabled":false,"source":{"source":"local","path":"C:\\p\\pdf"}}],
            "available":[{"name":"pdf","marketplaceName":"rt","version":"1.0","installed":true,"enabled":false,"source":{"source":"local","path":"C:\\p\\pdf"}},
                         {"name":"linear","marketplaceName":"remote","version":"0.1","installed":false,"source":{"source":"remote","id":"x"}}]}"#;
        let mut by_name = HashMap::new();
        by_name.insert("linear".to_string(), CodexMeta { description: "Issues".into(), ..Default::default() });
        let (c, inst) = parse_codex_list(json, &HashMap::new(), &by_name).unwrap();
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].id(), "pdf@rt");
        assert!(!inst["pdf@rt"][0].enabled);
        assert_eq!(c[1].description, "Issues");
        assert_eq!(c[1].source, "remote (remote)");
        assert!(c[1].matches("issues"));
    }
}
