//! Agent CLIs (Claude Code, Codex, Antigravity CLI, Qwen Code, OpenCode, Kimi CLI) launched with command-line flags
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
    /// Google's Antigravity CLI (`agy`), which replaced Gemini CLI; sessions saved as Gemini load as it.
    #[serde(alias = "Gemini")]
    Antigravity,
    Qwen,
    OpenCode,
    Kimi,
}

/// An agent CLI the app knows: how it's found, installed and uninstalled. The New-session
/// menu, detection and Options > Agents all read this table, so a new agent goes here
/// (plus its `Kind`).
pub struct AgentSpec {
    pub kind: Kind,
    /// Menu name.
    pub name: &'static str,
    /// Executable name, without `.exe` / `.cmd`.
    pub exe: &'static str,
    /// PowerShell command that installs it.
    pub install: &'static str,
    /// Project page.
    pub url: &'static str,
    /// npm package, when it can be installed with npm (uninstalled with `npm uninstall -g`).
    pub npm: Option<&'static str>,
    /// uv tool name, when installed with `uv tool install`.
    pub uv: Option<&'static str>,
}

pub const AGENTS: [AgentSpec; 6] = [
    AgentSpec { kind: Kind::Claude, name: "Claude", exe: "claude", install: "irm https://claude.ai/install.ps1 | iex", url: "https://docs.claude.com/en/docs/claude-code/setup", npm: Some("@anthropic-ai/claude-code"), uv: None },
    AgentSpec { kind: Kind::Codex, name: "Codex", exe: "codex", install: "npm install -g '@openai/codex'", url: "https://github.com/openai/codex", npm: Some("@openai/codex"), uv: None },
    AgentSpec { kind: Kind::Antigravity, name: "Antigravity", exe: "agy", install: "irm https://antigravity.google/cli/install.ps1 | iex", url: "https://antigravity.google", npm: None, uv: None },
    AgentSpec { kind: Kind::Qwen, name: "Qwen", exe: "qwen", install: "npm install -g '@qwen-code/qwen-code@latest'", url: "https://github.com/QwenLM/qwen-code", npm: Some("@qwen-code/qwen-code"), uv: None },
    AgentSpec { kind: Kind::OpenCode, name: "OpenCode", exe: "opencode", install: "npm install -g opencode-ai@latest", url: "https://opencode.ai", npm: Some("opencode-ai"), uv: None },
    AgentSpec { kind: Kind::Kimi, name: "Kimi", exe: "kimi", install: "uv tool install --python 3.13 kimi-cli", url: "https://github.com/MoonshotAI/kimi-cli", npm: None, uv: Some("kimi-cli") },
];

impl Kind {
    /// Claude / Codex: the app finds their conversations (resume, titles, cost, model).
    /// The other agents run as plain terminal sessions.
    pub fn tracks_conversation(self) -> bool {
        matches!(self, Kind::Claude | Kind::Codex)
    }

    /// The flag that starts the agent without permission prompts, if it has one.
    pub fn unrestricted_flag(self) -> Option<&'static str> {
        match self {
            Kind::Claude | Kind::Antigravity => Some("--dangerously-skip-permissions"),
            Kind::Codex => Some("--dangerously-bypass-approvals-and-sandbox"),
            Kind::Qwen | Kind::Kimi => Some("--yolo"),
            Kind::OpenCode | Kind::Shell => None,
        }
    }

    /// Takes a reasoning effort setting (Claude `--effort`, Codex `model_reasoning_effort`).
    pub fn has_effort(self) -> bool {
        matches!(self, Kind::Claude | Kind::Codex)
    }
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

/// When set, new agent sessions (not resumed, not named at launch) are asked to name themselves
/// after their first task with the `rename_session` MCP tool. Claude gets it through
/// `--append-system-prompt`, Codex through `-c developer_instructions=...`.
pub static NAME_SESSION_PROMPT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The naming instruction (see `NAME_SESSION_PROMPT`).
pub const NAME_SESSION_TEXT: &str = "This session runs in AI Terminal, whose sidebar lists sessions by name. When the user gives you the first task of this conversation, call the rename_session tool (ai-terminal MCP server) once with a short name for that task (2 to 6 words, e.g. \"Fix login redirect bug\"), then carry on with the task. Do this only once: don't rename the session again later in the conversation. Don't mention the renaming to the user.";

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

/// The naming instruction goes only to a new conversation the user didn't name at launch.
fn wants_naming(name_sessions: bool, name: &str, resume: Option<&str>) -> bool {
    name_sessions && resume.is_none() && name.trim().is_empty()
}

/// A TOML basic string (for Codex `-c key=value`, whose value is parsed as TOML).
fn toml_string(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The `--append-system-prompt` text for a Claude session: the colour-tag guide and/or the
/// naming instruction (only for a new conversation the user didn't name at launch).
fn appended_prompt(color_tags: bool, name_sessions: bool, name: &str, resume: Option<&str>) -> Option<String> {
    let mut parts: Vec<&str> = Vec::new();
    if color_tags {
        parts.push(crate::colortags::PROMPT);
    }
    if wants_naming(name_sessions, name, resume) {
        parts.push(NAME_SESSION_TEXT);
    }
    (!parts.is_empty()).then(|| parts.join("

"))
}

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
            // Our instructions, in one --append-system-prompt, unless the user passes their own.
            if !a.iter().any(|x| x == "--append-system-prompt") {
                let color = COLOR_TAG_PROMPT.load(std::sync::atomic::Ordering::Relaxed);
                let naming = NAME_SESSION_PROMPT.load(std::sync::atomic::Ordering::Relaxed);
                if let Some(prompt) = appended_prompt(color, naming, name, resume) {
                    a.extend(["--append-system-prompt".into(), prompt]);
                }
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
            // The naming instruction, unless the user sets developer_instructions themselves.
            let naming = NAME_SESSION_PROMPT.load(std::sync::atomic::Ordering::Relaxed);
            if wants_naming(naming, name, resume) && !a.iter().any(|x| x.starts_with("developer_instructions")) {
                a.extend(["-c".into(), format!("developer_instructions={}", toml_string(NAME_SESSION_TEXT))]);
            }
            if resume.is_none() {
                if let Some(p) = opts.initial_prompt.as_deref().filter(|p| !p.trim().is_empty()) {
                    a.extend(["--".into(), p.to_string()]);
                }
            }
        }
        Kind::Antigravity | Kind::Qwen | Kind::OpenCode | Kind::Kimi => {
            if let Some(f) = kind.unrestricted_flag().filter(|_| opts.unrestricted) {
                if !params.iter().any(|p| p == f) {
                    a.push(f.into());
                }
            }
            if opts.model.is_some() {
                strip_flag(&mut params, &["--model", "-m"]);
            }
            a.extend(params);
            if let Some(m) = opts.model.as_deref().filter(|m| !m.is_empty()) {
                a.extend(["--model".into(), m.to_string()]);
            }
            // First prompt (handoff): Antigravity / Qwen run it and stay interactive.
            if let Some(p) = opts.initial_prompt.as_deref().filter(|p| !p.trim().is_empty()) {
                match kind {
                    Kind::Antigravity | Kind::Qwen => a.extend(["--prompt-interactive".into(), p.to_string()]),
                    Kind::OpenCode => a.extend(["--prompt".into(), p.to_string()]),
                    _ => {}
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

impl Catalog {
    /// The model list for an agent (only Claude and Codex report one).
    pub fn models(&self, kind: Kind) -> &[ModelInfo] {
        match kind {
            Kind::Claude => &self.claude_models,
            Kind::Codex => &self.codex_models,
            _ => &[],
        }
    }
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

/// Claude Code's model aliases; each always points to the latest model of its family, so the
/// list doesn't go stale. `claude --help` has no real model list (only examples in prose).
const CLAUDE_ALIASES: &[&str] = &["fable", "opus", "sonnet", "haiku"];

/// The Claude model list: the built-in aliases, plus any other alias `claude --help` mentions.
/// Full model names from the help text (e.g. `claude-sonnet-4-6`) are examples, not a list of
/// what's current, so they're left out (they can still be typed as `--model` in CLI Parameters).
pub fn claude_models(help_models: &[String], efforts: &[String]) -> Vec<ModelInfo> {
    let mut ids: Vec<String> = CLAUDE_ALIASES.iter().map(|s| s.to_string()).collect();
    for m in help_models {
        let alias = !m.starts_with("claude-") && !m.chars().any(|c| c.is_ascii_digit()) && m != "default";
        if alias && !ids.contains(m) {
            ids.push(m.clone());
        }
    }
    ids.into_iter()
        .map(|id| {
            let mut name: Vec<char> = id.chars().collect();
            name[0] = name[0].to_ascii_uppercase();
            ModelInfo { label: format!("{} (latest)", name.into_iter().collect::<String>()), id, efforts: efforts.to_vec(), default_effort: None }
        })
        .collect()
}

pub fn load_catalog(claude: Option<String>, codex: Option<String>) -> Catalog {
    let mut c = Catalog::default();
    // The aliases are listed even when `claude --help` can't be read.
    let (models, efforts) = claude.and_then(|p| run_quiet(&p, &["--help"])).map(|h| parse_claude_help(&h)).unwrap_or_default();
    c.claude_models = claude_models(&models, &efforts);
    c.claude_efforts = efforts;
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

/// Codex rollout files (`sessions/YYYY/MM/DD/rollout-*.jsonl`) in the day folders from about `since` on.
fn codex_rollouts_since(since: SystemTime) -> Vec<PathBuf> {
    let Some(root) = codex_sessions() else { return Vec::new() };
    let mut out = Vec::new();
    // Only descend into the year / month / day folders (named by the local date the conversation
    // started) from a day before `since` on (time-zone slack). A folder's modified time is no
    // guide: it only changes when an entry is added directly to it, not deeper down.
    let secs = since.duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let from = civil_date((secs / 86_400) as i64 - 1);
    let num = |p: &Path| p.file_name().and_then(|n| n.to_str()).and_then(|n| n.parse::<i64>().ok());
    let dirs = |p: &Path| std::fs::read_dir(p).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect::<Vec<_>>();
    for y in dirs(&root) {
        let Some(yn) = num(&y).filter(|n| *n >= from.0) else { continue };
        for m in dirs(&y) {
            let Some(mn) = num(&m).filter(|n| (yn, *n) >= (from.0, from.1)) else { continue };
            for d in dirs(&m) {
                if num(&d).is_none_or(|dn| (yn, mn, dn) < from) {
                    continue;
                }
                out.extend(std::fs::read_dir(&d).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.extension().and_then(|x| x.to_str()) == Some("jsonl")));
            }
        }
    }
    out
}

/// Days since 1970-01-01 → (year, month, day) (H. Hinnant's civil_from_days).
fn civil_date(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
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

/// A plan-limit window for the status bar: short label ("5h"), long name ("5-hour limit") and usage.
pub type LimitWindow = (String, String, crate::hooks::Window);

/// Codex's plan limits, from the newest rollout that reports them: its `token_count` events
/// carry `rate_limits` (primary / secondary window: percent used, length, reset time). The
/// limits are per account, so any recent conversation will do.
pub fn codex_limits() -> Vec<LimitWindow> {
    let since = SystemTime::now() - std::time::Duration::from_secs(8 * 86_400);
    let mut files: Vec<(SystemTime, PathBuf)> = codex_rollouts_since(since).into_iter().filter_map(|p| Some((std::fs::metadata(&p).ok()?.modified().ok()?, p))).collect();
    files.sort_by(|a, b| b.0.cmp(&a.0));
    for (mtime, p) in files.into_iter().take(10) {
        let Ok(mut f) = std::fs::File::open(&p) else { continue };
        let len = f.metadata().map(|m| m.len()).unwrap_or(0);
        let tail = 512 * 1024u64;
        if f.seek(SeekFrom::Start(len.saturating_sub(tail))).is_err() {
            continue;
        }
        let mut bytes = Vec::new();
        let _ = f.take(tail).read_to_end(&mut bytes);
        let text = String::from_utf8_lossy(&bytes);
        for line in text.lines().rev().filter(|l| l.contains("\"rate_limits\"")) {
            let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { continue };
            let w = parse_codex_limits(&v["payload"]["rate_limits"], mtime);
            if !w.is_empty() {
                return w;
            }
        }
    }
    Vec::new()
}

/// `rate_limits` → windows. `resets_at` is Unix seconds; older CLIs gave `resets_in_seconds`
/// (counted from `written`, the file's modification time).
fn parse_codex_limits(rl: &serde_json::Value, written: SystemTime) -> Vec<LimitWindow> {
    let written = written.duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    ["primary", "secondary"]
        .iter()
        .filter_map(|k| {
            let w = &rl[*k];
            let used = w["used_percent"].as_f64()?;
            let resets_at = w["resets_at"].as_u64().or_else(|| w["resets_at"].as_f64().map(|f| f as u64)).or_else(|| w["resets_in_seconds"].as_u64().map(|s| written + s));
            let (label, long) = window_names(w["window_minutes"].as_u64().unwrap_or(0));
            Some((label, long, crate::hooks::Window { used, resets_at }))
        })
        .collect()
}

/// One line of `agy -p /usage`: a model group's limit window.
#[derive(Clone, Debug, PartialEq)]
pub struct AgyQuota {
    /// "Gemini Models", "Claude and GPT models" or a single model ("Gemini 4 Argon").
    pub group: String,
    /// "Five Hour Limit Remaining", "Weekly Limit Remaining", ...
    pub name: String,
    pub left: f64,
    pub resets_at: Option<u64>,
}

/// Antigravity's quota: `agy -p /usage` prints one tab-separated line per window (group, window,
/// percent left, reset time in UTC). Takes a couple of seconds; run it off the UI thread.
pub fn antigravity_usage(program: &str) -> Vec<AgyQuota> {
    run_quiet(program, &["-p", "/usage"]).map(|t| parse_agy_usage(&t)).unwrap_or_default()
}

fn parse_agy_usage(text: &str) -> Vec<AgyQuota> {
    text.lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split('\t').map(str::trim).collect();
            let [group, name, left, resets] = f[..] else { return None };
            let left = left.strip_suffix('%')?.trim().parse::<f64>().ok()?;
            Some(AgyQuota { group: group.into(), name: name.into(), left, resets_at: parse_utc(resets) })
        })
        .collect()
}

/// "2026-11-23T01:54:39Z" → Unix seconds.
fn parse_utc(s: &str) -> Option<u64> {
    let s = s.strip_suffix('Z')?;
    let (date, time) = s.split_once('T')?;
    let d: Vec<i64> = date.split('-').map(|x| x.parse().ok()).collect::<Option<_>>()?;
    let t: Vec<f64> = time.split(':').map(|x| x.parse().ok()).collect::<Option<_>>()?;
    let (&[y, m, d], &[hh, mm, ss]) = (&d[..], &t[..]) else { return None };
    // Days from civil (H. Hinnant), the inverse of civil_date.
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    u64::try_from(days * 86_400 + (hh * 3600.0 + mm * 60.0 + ss) as i64).ok()
}

/// The status bar's windows from Antigravity's quota: the group the session's model belongs to
/// ("Claude and GPT models" for those, else the Gemini group), and that group's name.
pub fn antigravity_limits(rows: &[AgyQuota], model: &str) -> (String, Vec<LimitWindow>) {
    let m = model.to_lowercase();
    let other = m.contains("claude") || m.contains("gpt");
    let group = |g: &str| {
        let g = g.to_lowercase();
        if other { g.contains("claude") || g.contains("gpt") } else { g.starts_with("gemini") && g.ends_with("models") }
    };
    let mut w: Vec<LimitWindow> = rows
        .iter()
        .filter(|r| group(&r.group))
        .map(|r| {
            let n = r.name.to_lowercase();
            let (a, b) = if n.contains("five hour") || n.contains("5 hour") {
                ("5h", "5-hour limit")
            } else if n.contains("week") {
                ("Week", "Weekly limit")
            } else if n.contains("day") {
                ("Day", "Daily limit")
            } else {
                ("Limit", "Plan limit")
            };
            (a.to_string(), b.to_string(), crate::hooks::Window { used: 100.0 - r.left, resets_at: r.resets_at })
        })
        .collect();
    // Shortest window first, as for the other providers.
    let order = |s: &str| ["5h", "Day", "Week"].iter().position(|x| *x == s).unwrap_or(9);
    w.sort_by_key(|x| order(&x.0));
    (rows.iter().find(|r| group(&r.group)).map(|r| r.group.clone()).unwrap_or_default(), w)
}

/// Names for a limit window of `minutes`: ("5h", "5-hour limit"), ("Week", "Weekly limit"), …
fn window_names(minutes: u64) -> (String, String) {
    match minutes {
        10080 => ("Week".into(), "Weekly limit".into()),
        1440 => ("Day".into(), "Daily limit".into()),
        0 => ("Limit".into(), "Plan limit".into()),
        m if m % 1440 == 0 => (format!("{}d", m / 1440), format!("{}-day limit", m / 1440)),
        m if m % 60 == 0 => (format!("{}h", m / 60), format!("{}-hour limit", m / 60)),
        m => (format!("{m}m"), format!("{m}-minute limit")),
    }
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
    fn naming_prompt_only_for_new_unnamed_sessions() {
        let both = appended_prompt(true, true, "", None).unwrap();
        assert!(both.starts_with(crate::colortags::PROMPT) && both.ends_with(NAME_SESSION_TEXT));
        assert_eq!(appended_prompt(false, true, "", None).as_deref(), Some(NAME_SESSION_TEXT));
        assert_eq!(appended_prompt(false, true, "", Some("id")), None);
        assert_eq!(appended_prompt(false, true, "Fix build", None), None);
        assert_eq!(appended_prompt(true, true, "", Some("id")).as_deref(), Some(crate::colortags::PROMPT));
        assert_eq!(appended_prompt(false, false, "", None), None);
    }

    #[test]
    fn codex_rate_limits_parse() {
        let v: serde_json::Value = serde_json::from_str(r#"{"primary":{"used_percent":2.0,"window_minutes":10080,"resets_at":1795208420},"secondary":{"used_percent":40.5,"window_minutes":300,"resets_in_seconds":60}}"#).unwrap();
        let written = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1000);
        let w = parse_codex_limits(&v, written);
        assert_eq!(w.len(), 2);
        assert_eq!((w[0].0.as_str(), w[0].1.as_str(), w[0].2.used, w[0].2.resets_at), ("Week", "Weekly limit", 2.0, Some(1795208420)));
        assert_eq!((w[1].0.as_str(), w[1].2.used, w[1].2.resets_at), ("5h", 40.5, Some(1060)));
        let none: serde_json::Value = serde_json::from_str(r#"{"primary":null,"secondary":null}"#).unwrap();
        assert!(parse_codex_limits(&none, written).is_empty());
        assert_eq!(window_names(2880).0, "2d");
        assert_eq!(civil_date(0), (1970, 1, 1));
        assert_eq!(civil_date(20_407), (2025, 11, 15));
        assert_eq!(civil_date(19_782), (2024, 2, 29));
    }

    #[test]
    fn antigravity_usage_parse() {
        let out = "Gemini Models\tWeekly Limit Remaining\t100%\t2026-11-23T01:54:39Z\n\
                   Gemini Models\tFive Hour Limit Remaining\t87%\t2026-11-16T06:54:39Z\n\
                   Claude and GPT models\tWeekly Limit Remaining\t60%\t2026-11-23T01:54:39Z\n\
                   Claude and GPT models\tFive Hour Limit Remaining\t40%\t2026-11-16T06:54:39Z\n\
                   Gemini 4 Argon\tGemini 4 Argon Weekly Limit\t99%\t2026-11-23T01:24:36Z\n\
                   some other line\n";
        let rows = parse_agy_usage(out);
        assert_eq!(rows.len(), 5);
        assert_eq!(rows[0].resets_at, Some(1_795_398_879)); // 2026-11-23 01:54:39 UTC
        let (name, g) = antigravity_limits(&rows, "gemini-4-argon");
        assert_eq!(name, "Gemini Models");
        assert_eq!(g.iter().map(|w| (w.0.as_str(), w.2.used)).collect::<Vec<_>>(), [("5h", 13.0), ("Week", 0.0)]);
        let (name, c) = antigravity_limits(&rows, "Claude Opus 5.5");
        assert_eq!(name, "Claude and GPT models");
        assert_eq!(c.iter().map(|w| (w.0.as_str(), w.2.used)).collect::<Vec<_>>(), [("5h", 60.0), ("Week", 40.0)]);
        assert_eq!(parse_utc("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_utc("2000-03-01T00:00:00Z"), Some(951_868_800));
    }

    #[test]
    #[ignore] // reads the real ~/.codex: cargo test codex_limits_from_disk -- --ignored --nocapture
    fn codex_limits_from_disk() {
        eprintln!("{:?}", codex_limits());
    }

    #[test]
    fn codex_instruction_is_valid_toml_string() {
        assert_eq!(toml_string("a \"b\" c\\d\ne"), r#""a \"b\" c\\d\ne""#);
        assert!(wants_naming(true, " ", None));
        assert!(!wants_naming(true, "", Some("id")) && !wants_naming(true, "Fix", None) && !wants_naming(false, "", None));
    }

    #[test]
    fn params_split_and_flags() {
        assert_eq!(split_params(r#"--add-dir "C:\My Dir" -v"#), vec!["--add-dir", r"C:\My Dir", "-v"]);
        let o = LaunchOpts { unrestricted: false, model: Some("opus".into()), effort: Some("high".into()), cli_params: "--model sonnet --verbose".into(), one_m: true, initial_prompt: None };
        assert_eq!(build_args(Kind::Claude, &o, "Fix build", Some("abc"), "k"), vec!["--permission-mode", "auto", "--verbose", "--model", "opus[1m]", "--effort", "high", "-n", "Fix build", "--resume", "abc"]);
        let u = LaunchOpts { unrestricted: true, ..Default::default() };
        assert_eq!(build_args(Kind::Claude, &u, "", None, "k"), vec!["--dangerously-skip-permissions"]);
        let c = LaunchOpts { unrestricted: true, model: Some("gpt-x".into()), effort: Some("low".into()), ..Default::default() };
        assert_eq!(build_args(Kind::Codex, &c, "", Some("id1"), "k"), vec!["resume", "id1", "--dangerously-bypass-approvals-and-sandbox", "--model", "gpt-x", "-c", "model_reasoning_effort=\"low\""]);
        let g = LaunchOpts { unrestricted: true, model: Some("gemini-x".into()), cli_params: "-m old --debug".into(), initial_prompt: Some("hi".into()), ..Default::default() };
        assert_eq!(build_args(Kind::Antigravity, &g, "", None, "k"), vec!["--dangerously-skip-permissions", "--debug", "--model", "gemini-x", "--prompt-interactive", "hi"]);
        assert_eq!(build_args(Kind::OpenCode, &g, "", None, "k"), vec!["--debug", "--model", "gemini-x", "--prompt", "hi"]);
        // Codex handoff: the first prompt is passed once, and not when resuming.
        let h = LaunchOpts { initial_prompt: Some("hi".into()), ..Default::default() };
        let args = build_args(Kind::Codex, &h, "Named", None, "k");
        assert_eq!(args.iter().filter(|x| *x == "hi").count(), 1);
        assert_eq!(args[args.len() - 2..], ["--", "hi"]);
        assert!(!build_args(Kind::Codex, &h, "Named", Some("id1"), "k").iter().any(|x| x == "hi"));
    }

    #[test]
    fn claude_help_parsing() {
        let help = "  --effort <level>        Effort level for the current session\n                          (low, medium, high, xhigh, max)\n  --model <model>         Model for the current session. Provide\n                          an alias for the latest model (e.g.\n                          'fable', 'opus', or 'sonnet') or a\n                          model's full name.\n  -n, --name <name>       Set a display name\n";
        let (m, e) = parse_claude_help(help);
        assert_eq!(m, vec!["fable", "opus", "sonnet"]);
        assert_eq!(e, vec!["low", "medium", "high", "xhigh", "max"]);
    }

    #[test]
    fn claude_model_list() {
        let help: Vec<String> = ["sonnet", "opus", "claude-sonnet-4-6", "mythos"].iter().map(|s| s.to_string()).collect();
        let m = claude_models(&help, &[]);
        let ids: Vec<&str> = m.iter().map(|x| x.id.as_str()).collect();
        assert_eq!(ids, vec!["fable", "opus", "sonnet", "haiku", "mythos"]);
        assert_eq!(m[1].label, "Opus (latest)");
    }
}
