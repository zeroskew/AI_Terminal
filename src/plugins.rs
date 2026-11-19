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
    /// The plugin's files in the local marketplace checkout (relative sources only).
    pub dir: Option<PathBuf>,
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
    /// Where Claude Code unpacked it (`installPath`).
    pub path: Option<PathBuf>,
    /// The project a project / local scope install belongs to.
    pub project: Option<PathBuf>,
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
                dir: (source.starts_with("./") || source.starts_with("../")).then(|| loc.join(&source)),
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
            let user = user_settings().and_then(|p| enabled_in(&p, id));
            let entries = list
                .as_array()
                .map(|a| {
                    a.iter()
                        .map(|e| {
                            let scope = s(&e["scope"]);
                            let project = e["projectPath"].as_str().map(PathBuf::from);
                            // Its own scope's setting; installs write `true`, so missing counts as on.
                            let enabled = match (scope.as_str(), &project) {
                                ("project", Some(p)) => enabled_in(&p.join(".claude").join("settings.json"), id),
                                ("local", Some(p)) => enabled_in(&p.join(".claude").join("settings.local.json"), id),
                                _ => user,
                            }
                            .unwrap_or(true);
                            Installed { scope, version: s(&e["version"]), enabled, path: e["installPath"].as_str().map(PathBuf::from), project }
                        })
                        .collect()
                })
                .unwrap_or_default();
            out.insert(id.clone(), entries);
        }
    }
    out
}

/// `~/.claude/settings.json`.
fn user_settings() -> Option<PathBuf> {
    plugins_dir().and_then(|d| d.parent().map(|c| c.join("settings.json")))
}

/// A settings file's `enabledPlugins` entry for `id`, if it has one.
fn enabled_in(settings: &Path, id: &str) -> Option<bool> {
    let text = std::fs::read_to_string(settings).ok()?;
    serde_json::from_str::<serde_json::Value>(&text).ok()?["enabledPlugins"][id].as_bool()
}

/// Where a plugin's on / off in one project comes from (later layers win).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Layer {
    /// Nothing set anywhere.
    Default,
    /// `~/.claude/settings.json`: every project.
    User,
    /// The project's `.claude/settings.json` (shared, usually committed).
    Project,
    /// The project's `.claude/settings.local.json` (personal).
    Local,
}

/// A plugin's state in one project.
#[derive(Clone, Debug)]
pub struct ProjectState {
    pub path: PathBuf,
    pub on: bool,
    pub from: Layer,
    /// Installed in this project only (project / local scope), not system wide.
    pub installed_here: bool,
}

/// A plugin's on / off system wide and in each project of `projects` it's available in
/// (all of them for a user install; otherwise the projects it was installed into).
#[derive(Clone, Debug)]
pub struct Enablement {
    /// Installed system wide (user scope), and its user-settings entry.
    pub user_install: bool,
    pub user: Option<bool>,
    pub projects: Vec<ProjectState>,
}

pub fn enablement(id: &str, inst: &[Installed], projects: &[PathBuf]) -> Enablement {
    let user_install = inst.iter().any(|i| i.scope == "user");
    let user = user_settings().and_then(|p| enabled_in(&p, id));
    let here: Vec<&PathBuf> = inst.iter().filter(|i| i.scope != "user").filter_map(|i| i.project.as_ref()).collect();
    let same = |a: &Path, b: &Path| a.to_string_lossy().trim_end_matches(['\\', '/']).eq_ignore_ascii_case(b.to_string_lossy().trim_end_matches(['\\', '/']));
    let mut paths: Vec<PathBuf> = projects.to_vec();
    for p in &here {
        if !paths.iter().any(|x| same(x, p)) {
            paths.push((*p).clone());
        }
    }
    let projects = paths
        .into_iter()
        .filter_map(|path| {
            let installed_here = here.iter().any(|p| same(p, &path));
            if !user_install && !installed_here {
                return None;
            }
            let claude = path.join(".claude");
            let (on, from) = match (enabled_in(&claude.join("settings.local.json"), id), enabled_in(&claude.join("settings.json"), id), user) {
                (Some(v), ..) => (v, Layer::Local),
                (None, Some(v), _) => (v, Layer::Project),
                (None, None, Some(v)) => (v, Layer::User),
                (None, None, None) => (true, Layer::Default),
            };
            Some(ProjectState { path, on, from, installed_here })
        })
        .collect();
    Enablement { user_install, user, projects }
}

/// Removes a project's personal on / off for `id` (back to the project / system-wide
/// setting). The CLI can only set it, not clear it.
pub fn clear_local_override(project: &Path, id: &str) -> Result<(), String> {
    let file = project.join(".claude").join("settings.local.json");
    let text = std::fs::read_to_string(&file).map_err(|e| e.to_string())?;
    let mut v: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let Some(map) = v.get_mut("enabledPlugins").and_then(|m| m.as_object_mut()) else { return Ok(()) };
    if map.shift_remove(id).is_none() {
        return Ok(());
    }
    if map.is_empty() {
        v.as_object_mut().map(|o| o.shift_remove("enabledPlugins"));
    }
    let mut out = serde_json::to_string_pretty(&v).map_err(|e| e.to_string())?;
    out.push('\n');
    std::fs::write(&file, out).map_err(|e| e.to_string())
}

/// What a Claude Code plugin adds to the context, estimated from its files (~3 characters a
/// token; fast enough for every list row, unlike `claude plugin details`). Skills, commands and agents are listed by name and description in every request;
/// their full text loads only when used. MCP tool definitions are only known once the server
/// runs, so those servers are counted but not sized.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TokenCost {
    /// Tokens sent with every request (the listings).
    pub always: u64,
    /// Tokens of the full skill / command / agent text, loaded when one is used.
    pub on_use: u64,
    pub skills: usize,
    pub commands: usize,
    pub agents: usize,
    pub mcp_servers: usize,
    pub hooks: bool,
}

/// ~3 characters a token: within about 10% of `claude plugin details` on typical plugins
/// (4 undercounted by a third).
fn est_tokens(chars: usize) -> u64 {
    chars.div_ceil(3) as u64
}

/// A markdown file's YAML front matter and body.
fn split_front_matter(text: &str) -> (&str, &str) {
    let t = text.trim_start_matches('\u{feff}');
    let Some(rest) = t.strip_prefix("---") else { return ("", t) };
    let rest = rest.trim_start_matches(['\r', '\n']);
    match rest.find("\n---") {
        Some(i) => (&rest[..i], rest[i + 4..].trim_start_matches(['-', '\r', '\n'])),
        None => ("", t),
    }
}

/// A front-matter key's value, including indented continuation lines (`|` / `>` blocks).
fn fm_value(fm: &str, key: &str) -> String {
    let mut out = String::new();
    let mut inside = false;
    for line in fm.lines() {
        if !line.starts_with([' ', '\t']) && !line.trim().is_empty() {
            if inside {
                break;
            }
            if let Some(v) = line.strip_prefix(key).and_then(|r| r.strip_prefix(':')) {
                inside = true;
                let v = v.trim();
                if !matches!(v, "|" | ">" | "|-" | ">-") {
                    out.push_str(v.trim_matches(['"', '\'']));
                }
            }
        } else if inside {
            out.push(' ');
            out.push_str(line.trim());
        }
    }
    out.trim().to_string()
}

/// Adds one skill / command / agent file: its listing (name + description) and full size.
/// Returns false when the file can't be read.
fn add_entry(cost: &mut TokenCost, path: &Path, fallback_name: &str) -> bool {
    let Ok(text) = std::fs::read_to_string(path) else { return false };
    cost.on_use += est_tokens(text.len());
    let (fm, body) = split_front_matter(&text);
    // Hidden from the model: not listed.
    if fm_value(fm, "disable-model-invocation") == "true" {
        return true;
    }
    let name = fm_value(fm, "name");
    let mut desc = fm_value(fm, "description");
    if desc.is_empty() {
        // Without a description Claude Code shows the first line of the body.
        desc = body.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or_default().to_string();
    }
    let name = if name.is_empty() { fallback_name } else { &name };
    // "- name: description" plus a little list framing.
    cost.always += est_tokens(name.len() + desc.len() + 8);
    true
}

/// `.md` files under `dir`, recursively.
fn md_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            md_files(&p, out);
        } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("md")) {
            out.push(p);
        }
    }
}

/// The paths a manifest key names (a string or a list), relative to the plugin root.
fn manifest_paths(root: &Path, v: &serde_json::Value) -> Vec<PathBuf> {
    match v {
        serde_json::Value::String(s) => vec![root.join(s)],
        serde_json::Value::Array(a) => a.iter().filter_map(|x| x.as_str()).map(|s| root.join(s)).collect(),
        _ => Vec::new(),
    }
}

/// Estimates a Claude Code plugin's context cost from its folder; `None` if it isn't there.
pub fn token_cost(root: &Path) -> Option<TokenCost> {
    if !root.is_dir() {
        return None;
    }
    let read = |p: &Path| std::fs::read_to_string(p).ok().and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok());
    let manifest = read(&root.join(".claude-plugin").join("plugin.json")).unwrap_or_default();
    let mut cost = TokenCost::default();
    let mut seen = std::collections::HashSet::new();
    // Skills: <dir>/<skill>/SKILL.md (or a SKILL.md directly in a listed path).
    for d in std::iter::once(root.join("skills")).chain(manifest_paths(root, &manifest["skills"])) {
        let mut found = Vec::new();
        if d.join("SKILL.md").is_file() {
            found.push(d.join("SKILL.md"));
        } else if let Ok(rd) = std::fs::read_dir(&d) {
            found.extend(rd.flatten().map(|e| e.path().join("SKILL.md")).filter(|p| p.is_file()));
        }
        for f in found {
            let name = f.parent().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            if seen.insert(f.clone()) && add_entry(&mut cost, &f, &name) {
                cost.skills += 1;
            }
        }
    }
    // Commands and agents: .md files under their folders (or listed .md paths).
    for key in ["commands", "agents"] {
        let mut files = Vec::new();
        for d in std::iter::once(root.join(key)).chain(manifest_paths(root, &manifest[key])) {
            if d.is_file() {
                files.push(d);
            } else {
                md_files(&d, &mut files);
            }
        }
        for f in files {
            let name = f.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            if seen.insert(f.clone()) && add_entry(&mut cost, &f, &name) {
                *if key == "commands" { &mut cost.commands } else { &mut cost.agents } += 1;
            }
        }
    }
    // MCP servers: inline in the manifest, a file it names, or .mcp.json at the root.
    let servers = |v: serde_json::Value| v.get("mcpServers").unwrap_or(&v).as_object().map_or(0, |o| o.len());
    cost.mcp_servers = match &manifest["mcpServers"] {
        serde_json::Value::Object(o) => o.len(),
        serde_json::Value::String(s) => read(&root.join(s)).map_or(0, servers),
        _ => read(&root.join(".mcp.json")).map_or(0, servers),
    };
    cost.hooks = !manifest["hooks"].is_null() || root.join("hooks").join("hooks.json").is_file();
    Some(cost)
}

/// Claude Code's own cost figures for a plugin (`claude plugin details`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CliCost {
    /// "Always-on" tokens: added to every session.
    pub always: u64,
    /// Per component: (name, always-on, on-invoke).
    pub components: Vec<(String, u64, u64)>,
    /// The component inventory: (kind, count), e.g. ("LSP servers", 1).
    pub inventory: Vec<(String, usize)>,
}

impl CliCost {
    /// Tokens paid when every component is used once.
    pub fn on_invoke(&self) -> u64 {
        self.components.iter().map(|c| c.2).sum()
    }
}

/// "~759", "~9.7k", "1.2M" → tokens.
fn parse_tok(s: &str) -> Option<u64> {
    let s = s.trim().trim_start_matches('~').trim_end_matches("tok").trim();
    let (num, mul) = match s.as_bytes().last()? {
        b'k' | b'K' => (&s[..s.len() - 1], 1e3),
        b'm' | b'M' => (&s[..s.len() - 1], 1e6),
        _ => (s, 1.0),
    };
    num.trim().parse::<f64>().ok().map(|n| (n * mul).round() as u64)
}

/// Reads `claude plugin details` text output (there's no JSON form).
fn parse_details(text: &str) -> Result<CliCost, String> {
    let mut cost = CliCost::default();
    let mut found = false;
    let mut part = "";
    for line in text.lines() {
        let t = line.trim();
        if t.is_empty() {
            part = "";
            continue;
        }
        if !line.starts_with(' ') {
            part = match t {
                "Component inventory" => "inventory",
                _ if t.starts_with("Per-component") => "components",
                _ => "",
            };
            continue;
        }
        if let Some(v) = t.strip_prefix("Always-on:") {
            cost.always = parse_tok(v.split_whitespace().next().unwrap_or_default()).ok_or_else(|| format!("unexpected line: {t}"))?;
            found = true;
            continue;
        }
        match part {
            "inventory" => {
                // "Skills (2)  a, b"
                if let (Some(open), Some(close)) = (t.find('('), t.find(')')) {
                    if let Ok(n) = t[open + 1..close].parse() {
                        cost.inventory.push((t[..open].trim().to_string(), n));
                    }
                }
            }
            "components" => {
                let w: Vec<&str> = t.split_whitespace().collect();
                if w.len() >= 3 {
                    if let (Some(a), Some(b)) = (parse_tok(w[w.len() - 2]), parse_tok(w[w.len() - 1])) {
                        cost.components.push((w[..w.len() - 2].join(" "), a, b));
                    }
                }
            }
            _ => {}
        }
    }
    if found {
        Ok(cost)
    } else {
        Err(text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("no output").to_string())
    }
}

pub type DetailsRef = Arc<Mutex<Option<Result<CliCost, String>>>>;

/// Runs `claude plugin details` on a thread: by id for an installed plugin (in `cwd`, so
/// project installs are found), otherwise loading the local copy with `--plugin-dir`.
pub fn details_async(program: &str, base_args: &[String], id: &str, name: &str, dir: Option<&Path>, installed: bool, cwd: &Path, ctx: egui::Context) -> DetailsRef {
    let slot: DetailsRef = Arc::default();
    let mut args = base_args.to_vec();
    match (installed, dir) {
        (true, _) => args.extend(["plugin".into(), "details".into(), id.to_string()]),
        (false, Some(d)) => args.extend(["--plugin-dir".into(), d.display().to_string(), "plugin".into(), "details".into(), name.to_string()]),
        (false, None) => {
            *slot.lock().unwrap() = Some(Err("no local copy".into()));
            return slot;
        }
    }
    let (out, program, cwd) = (slot.clone(), program.to_string(), cwd.to_path_buf());
    std::thread::spawn(move || {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let r = std::process::Command::new(&program)
            .args(&args)
            .current_dir(&cwd)
            .stdin(std::process::Stdio::null())
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map_err(|e| format!("couldn't run claude: {e}"))
            .and_then(|o| parse_details(&format!("{}\n{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))));
        *out.lock().unwrap() = Some(r);
        ctx.request_repaint();
    });
    slot
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
            installed.entry(id.clone()).or_insert_with(|| vec![Installed { scope: String::new(), version: s(&p["version"]), enabled: p["enabled"].as_bool().unwrap_or(true), path: None, project: None }]);
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
    fn estimates_token_cost() {
        let dir = std::env::temp_dir().join(format!("ait-plugin-cost-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for d in ["skills/demo", "commands/sub", ".claude-plugin"] {
            std::fs::create_dir_all(dir.join(d)).unwrap();
        }
        std::fs::write(dir.join(".claude-plugin/plugin.json"), r#"{"name":"x","mcpServers":{"a":{},"b":{}}}"#).unwrap();
        std::fs::write(dir.join("skills/demo/SKILL.md"), format!("---\nname: demo\ndescription: >\n  Does the\n  demo thing.\nallowed-tools: Bash\n---\n\n# Body\n{}", "x".repeat(400))).unwrap();
        std::fs::write(dir.join("commands/sub/go.md"), "First line describes it.\nMore.").unwrap();
        std::fs::write(dir.join("commands/hidden.md"), "---\ndisable-model-invocation: true\ndescription: never listed\n---\nbody").unwrap();
        let c = token_cost(&dir).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!((c.skills, c.commands, c.agents, c.mcp_servers, c.hooks), (1, 2, 0, 2, false));
        assert_eq!(c.always, est_tokens("demo".len() + "Does the demo thing.".len() + 8) + est_tokens("go".len() + "First line describes it.".len() + 8));
        assert!(c.on_use > 100);
        assert!(token_cost(Path::new("Z:/definitely/missing")).is_none());
    }

    #[test]
    fn layers_project_enablement() {
        let root = std::env::temp_dir().join(format!("ait-plugin-en-{}", std::process::id()));
        let (a, b, c) = (root.join("a"), root.join("b"), root.join("c"));
        for p in [&a, &b] {
            std::fs::create_dir_all(p.join(".claude")).unwrap();
        }
        let id = "zz-test-plugin@nowhere";
        std::fs::write(a.join(".claude/settings.json"), format!(r#"{{"enabledPlugins":{{"{id}":true}}}}"#)).unwrap();
        std::fs::write(a.join(".claude/settings.local.json"), format!(r#"{{"model":"x","enabledPlugins":{{"{id}":false}}}}"#)).unwrap();
        std::fs::write(b.join(".claude/settings.json"), format!(r#"{{"enabledPlugins":{{"{id}":false}}}}"#)).unwrap();
        let user = [Installed { scope: "user".into(), version: String::new(), enabled: true, path: None, project: None }];
        let e = enablement(id, &user, &[a.clone(), b.clone(), c.clone()]);
        let got: Vec<(bool, Layer)> = e.projects.iter().map(|p| (p.on, p.from)).collect();
        assert_eq!(got, vec![(false, Layer::Local), (false, Layer::Project), (true, Layer::Default)]);
        // A local-scope install shows only where it was installed.
        let local = [Installed { scope: "local".into(), version: String::new(), enabled: true, path: None, project: Some(b.clone()) }];
        let e = enablement(id, &local, &[a.clone(), c.clone()]);
        assert!(!e.user_install);
        assert_eq!(e.projects.len(), 1);
        assert!(e.projects[0].installed_here && e.projects[0].path == b);
        // Reset drops the personal override and keeps the rest of the file.
        clear_local_override(&a, id).unwrap();
        assert_eq!(std::fs::read_to_string(a.join(".claude/settings.local.json")).unwrap().trim(), "{\n  \"model\": \"x\"\n}");
        assert_eq!(enablement(id, &user, std::slice::from_ref(&a)).projects[0].from, Layer::Project);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn parses_cli_details() {
        let text = "modern-web-guidance 0.0.194\n  Keep up to date\n  Source: x@y\n\nComponent inventory\n  Skills (2)  chrome-extensions, modern-web-guidance\n  Agents (0)\n  LSP servers (1)\n\nProjected token cost\n  Always-on:   ~759 tok   added to every session\n\nPer-component (rounded)\n  component            always-on  on-invoke\n  chrome-extensions         ~360      ~9.7k\n  my skill                   ~40        ~85\n\n  On-invoke cost is paid each time a skill or agent fires.\n";
        let c = parse_details(text).unwrap();
        assert_eq!(c.always, 759);
        assert_eq!(c.components, vec![("chrome-extensions".to_string(), 360, 9700), ("my skill".to_string(), 40, 85)]);
        assert_eq!(c.inventory, vec![("Skills".to_string(), 2), ("Agents".to_string(), 0), ("LSP servers".to_string(), 1)]);
        assert_eq!(c.on_invoke(), 9785);
        assert_eq!(parse_details("Plugin \"x\" not found. Run ...\n").unwrap_err(), "Plugin \"x\" not found. Run ...");
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
