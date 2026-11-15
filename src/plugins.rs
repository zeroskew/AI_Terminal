//! Claude Code plugins browser: the catalogue comes from the
//! marketplaces Claude Code already knows (`~/.claude/plugins/marketplaces/*/.claude-plugin/
//! marketplace.json`), installed state from `~/.claude/plugins/installed_plugins.json`, and
//! install / uninstall run Claude Code's own commands:
//!
//!     claude plugin install <name>@<marketplace> --scope user|project|local
//!     claude plugin uninstall <name>@<marketplace> --scope <scope> --prune -y
//!     claude plugin marketplace update            (refresh)

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
}

impl Plugin {
    /// `name@marketplace`, the id the CLI and installed_plugins.json use.
    pub fn id(&self) -> String {
        format!("{}@{}", self.name, self.marketplace)
    }
    pub fn is_bundle(&self) -> bool {
        self.category.eq_ignore_ascii_case("bundle")
    }
    pub fn matches(&self, filter: &str) -> bool {
        let f = filter.to_lowercase();
        f.is_empty()
            || [&self.name, &self.description, &self.category, &self.author, &self.marketplace].iter().any(|s| s.to_lowercase().contains(&f))
            || self.keywords.iter().any(|k| k.to_lowercase().contains(&f))
    }
}

#[derive(Clone, Debug)]
pub struct Installed {
    pub scope: String,
    pub version: String,
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
            });
        }
    }
    out.extend(directory_plugins(&dir));
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
                .map(|a| a.iter().map(|e| Installed { scope: s(&e["scope"]), version: s(&e["version"]) }).collect())
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

/// Runs `claude <args>` in `cwd` on a thread, streaming output into the job's log.
pub fn run(job: &JobRef, program: &str, base_args: &[String], args: Vec<String>, cwd: &Path, ctx: egui::Context) {
    {
        let mut j = job.lock().unwrap();
        if j.running {
            return;
        }
        j.running = true;
        j.log.push(format!("> claude {}", args.join(" ")));
    }
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
                job.lock().unwrap().log.push(format!("Couldn't run claude: {e}"));
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
    }
}
