//! Per-project MCP server switches, kept where Claude Code's own `/mcp` enable / disable keeps
//! them: `~/.claude.json` → `projects["<project path>"].disabledMcpServers` (server names).
//! Claude Code reads the list when a session starts.
//!
//! The servers come from `claude mcp list` (user, project and plugin servers and claude.ai
//! connectors), which health-checks each one, so it runs on a thread.

use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct Server {
    pub name: String,
    /// Command or URL.
    pub target: String,
    /// e.g. "✓ Connected", "! Needs authentication".
    pub status: String,
}

/// Claude Code's global config file (`$CLAUDE_CONFIG_DIR/.claude.json` when that's set).
fn claude_json() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("CLAUDE_CONFIG_DIR") {
        return Some(PathBuf::from(d).join(".claude.json"));
    }
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"))?;
    Some(PathBuf::from(home).join(".claude.json"))
}

fn norm(p: &str) -> String {
    p.replace('\\', "/").trim_end_matches('/').to_string()
}

/// The key Claude Code uses for the project (forward slashes; an existing entry wins,
/// compared without case).
fn project_key(projects: Option<&serde_json::Map<String, serde_json::Value>>, project: &Path) -> String {
    let want = norm(&project.display().to_string());
    projects.and_then(|o| o.keys().find(|k| norm(k).eq_ignore_ascii_case(&want)).cloned()).unwrap_or(want)
}

fn read_config() -> Result<serde_json::Value, String> {
    let path = claude_json().ok_or("No home folder")?;
    match std::fs::read_to_string(&path) {
        Ok(t) => serde_json::from_str(&t).map_err(|e| format!("Can't read {}: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(serde_json::json!({})),
        Err(e) => Err(format!("Can't read {}: {e}", path.display())),
    }
}

/// Names of the servers disabled for the project.
pub fn disabled(project: &Path) -> Vec<String> {
    let Ok(v) = read_config() else { return Vec::new() };
    let key = project_key(v["projects"].as_object(), project);
    v["projects"][&key]["disabledMcpServers"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default()
}

/// Saves the project's disabled servers, leaving the rest of the file as it is.
pub fn set_disabled(project: &Path, names: &[String]) -> Result<(), String> {
    let path = claude_json().ok_or("No home folder")?;
    // Claude Code locks the file while it writes (a `.lock` folder next to it); wait a moment.
    let lock = path.with_file_name(".claude.json.lock");
    for _ in 0..20 {
        if !lock.exists() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    let mut v = read_config()?;
    if !v.is_object() {
        return Err("~/.claude.json isn't a JSON object".into());
    }
    let key = project_key(v["projects"].as_object(), project);
    if !v["projects"].is_object() {
        v["projects"] = serde_json::json!({});
    }
    if !v["projects"][&key].is_object() {
        v["projects"][&key] = serde_json::json!({});
    }
    v["projects"][&key]["disabledMcpServers"] = serde_json::json!(names);
    let text = serde_json::to_string_pretty(&v).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.ai-terminal.tmp");
    std::fs::write(&tmp, text).map_err(|e| format!("Can't write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, &path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("Can't save {}: {e}", path.display())
    })
}

/// Claude Code's built-in Artifacts server. It isn't in `claude mcp list` or `disabledMcpServers`;
/// Claude Code leaves it out when the `Artifact` tool is denied by a permission rule (or when
/// `CLAUDE_CODE_ARTIFACT=0`). Found in Claude Code 2.1.325's startup code.
pub const ARTIFACTS: &str = "claude-artifacts";
const ARTIFACT_TOOL: &str = "Artifact";

/// What a server is, for the dialog (and for remembering later).
pub fn describe(name: &str) -> &'static str {
    match name {
        ARTIFACTS => "Built into Claude Code (the native install), not set up anywhere by you. It publishes web pages (reports, dashboards, slide decks, designs) to claude.ai as private artifacts you can later share. Not needed for coding. Costs about 8k tokens on every request, plus about 13k of deferred tools. Off = a deny rule for the \"Artifact\" tool in this project's .claude/settings.local.json. To turn it off in every project, set CLAUDE_CODE_ARTIFACT=0.",
        "claude.ai Claude Docs" => "A connector from your claude.ai account. It creates and edits Claude Docs: shared documents on claude.ai that people can comment on, exportable to Word, PDF or Google Docs. Not needed for coding.",
        "claude.ai Google Drive" => "A connector from your claude.ai account. It reads and edits files in your Google Drive (Docs, Sheets, Slides). It does nothing until you sign in to Google from a session.",
        n if n.starts_with("claude.ai ") => "A connector from your claude.ai account (claude.ai → Settings → Connectors). To turn off all claude.ai connectors in every project, set ENABLE_CLAUDEAI_MCP_SERVERS=false.",
        n if n.starts_with("plugin:") => "Comes with an installed Claude Code plugin (see the plugins window).",
        _ => "Added with `claude mcp add` (user or local scope) or by the project's .mcp.json.",
    }
}

fn settings_files(project: &Path) -> [PathBuf; 3] {
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from).unwrap_or_default();
    let c = project.join(".claude");
    [c.join("settings.local.json"), c.join("settings.json"), home.join(".claude").join("settings.json")]
}

fn denies_artifact(file: &Path) -> bool {
    std::fs::read_to_string(file)
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .is_some_and(|v| v["permissions"]["deny"].as_array().is_some_and(|a| a.iter().any(|x| x == ARTIFACT_TOOL)))
}

/// Whether the Artifact tool is denied for the project: (in its settings.local.json, in another
/// settings file this app doesn't edit (the project's settings.json or the user's)).
pub fn artifacts_denied(project: &Path) -> (bool, Option<PathBuf>) {
    let [local, shared, user] = settings_files(project);
    (denies_artifact(&local), [shared, user].into_iter().find(|f| denies_artifact(f)))
}

/// Adds or removes the `Artifact` deny rule in the project's `.claude/settings.local.json`,
/// keeping everything else in the file.
pub fn set_artifacts_denied(project: &Path, deny: bool) -> Result<(), String> {
    let [path, ..] = settings_files(project);
    let mut v: serde_json::Value = match std::fs::read_to_string(&path) {
        Ok(t) => serde_json::from_str(&t).map_err(|e| format!("Can't read {}: {e}", path.display()))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            if !deny {
                return Ok(());
            }
            serde_json::json!({})
        }
        Err(e) => return Err(format!("Can't read {}: {e}", path.display())),
    };
    if !v.is_object() {
        return Err(format!("{} isn't a JSON object", path.display()));
    }
    if !v["permissions"].is_object() {
        v["permissions"] = serde_json::json!({});
    }
    let mut list: Vec<serde_json::Value> = v["permissions"]["deny"].as_array().cloned().unwrap_or_default();
    list.retain(|x| x != ARTIFACT_TOOL);
    if deny {
        list.push(ARTIFACT_TOOL.into());
    }
    v["permissions"]["deny"] = serde_json::Value::Array(list);
    std::fs::create_dir_all(path.parent().unwrap_or(project)).map_err(|e| e.to_string())?;
    std::fs::write(&path, serde_json::to_string_pretty(&v).map_err(|e| e.to_string())? + "\n").map_err(|e| format!("Can't write {}: {e}", path.display()))
}

/// Parses `claude mcp list`: lines like `name: command-or-url - ✓ Connected`.
fn parse_list(out: &str) -> Vec<Server> {
    out.lines()
        .filter_map(|l| {
            let (left, status) = l.trim().rsplit_once(" - ")?;
            let (name, target) = left.split_once(": ")?;
            Some(Server { name: name.trim().to_string(), target: target.trim().to_string(), status: status.trim().to_string() })
        })
        .collect()
}

/// Runs `claude mcp list` in the project folder (slow: it connects to every server).
pub fn list(program: &str, base_args: &[String], cwd: &Path) -> Result<Vec<Server>, String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let out = std::process::Command::new(program)
        .args(base_args)
        .args(["mcp", "list"])
        .current_dir(cwd)
        .stdin(std::process::Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("Couldn't run claude: {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout);
    let servers = parse_list(&text);
    if servers.is_empty() && !out.status.success() {
        return Err(format!("claude mcp list failed: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(servers)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_mcp_list() {
        let s = parse_list("Checking MCP server health…\n\nclaude.ai Claude Docs: https://api.anthropic.com/v1/pages/mcp - ✓ Connected\nplugin:github:github: npx -y @x/server - ✗ Failed to connect\n");
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].name, "claude.ai Claude Docs");
        assert_eq!(s[0].status, "✓ Connected");
        assert_eq!(s[1].name, "plugin:github:github");
        assert_eq!(s[1].target, "npx -y @x/server");
    }

    #[test]
    fn artifact_deny_rule_round_trip() {
        let dir = std::env::temp_dir().join(format!("ai_terminal_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".claude")).unwrap();
        let f = dir.join(".claude").join("settings.local.json");
        std::fs::write(&f, r#"{"permissions":{"allow":["Bash(ls)"],"deny":["WebFetch"]},"model":"x"}"#).unwrap();
        set_artifacts_denied(&dir, true).unwrap();
        assert!(artifacts_denied(&dir).0);
        set_artifacts_denied(&dir, true).unwrap();
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&f).unwrap()).unwrap();
        assert_eq!(v["permissions"]["deny"], serde_json::json!(["WebFetch", "Artifact"]));
        assert_eq!(v["permissions"]["allow"], serde_json::json!(["Bash(ls)"]));
        assert_eq!(v["model"], "x");
        set_artifacts_denied(&dir, false).unwrap();
        assert!(!artifacts_denied(&dir).0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn project_key_matches_existing_entry() {
        let m: serde_json::Map<String, serde_json::Value> = serde_json::from_str(r#"{"X:/Games/Proj": {}}"#).unwrap();
        assert_eq!(project_key(Some(&m), Path::new(r"x:\Games\Proj\")), "X:/Games/Proj");
        assert_eq!(project_key(Some(&m), Path::new(r"C:\Other")), "C:/Other");
    }
}
