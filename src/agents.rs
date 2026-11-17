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
    /// Claude / Codex / Antigravity: the app finds their conversations (resume, titles, search;
    /// cost and model for Claude / Codex). The other agents run as plain terminal sessions.
    pub fn tracks_conversation(self) -> bool {
        matches!(self, Kind::Claude | Kind::Codex | Kind::Antigravity)
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

/// When set, Claude sessions get `--append-system-prompt` describing AI Terminal's color tags.
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

/// The `--append-system-prompt` text for a Claude session: the color-tag guide and/or the
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
            if let (Kind::Antigravity, Some(id)) = (kind, resume) {
                a.extend(["--conversation".into(), id.to_string()]);
            }
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
            if let Some(p) = opts.initial_prompt.as_deref().filter(|p| !p.trim().is_empty() && resume.is_none()) {
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
    /// From `agy models` (only the script model picker uses it so far).
    pub agy_models: Vec<ModelInfo>,
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

    /// The effort levels an agent offers for `model` (the model's own list when known, else
    /// Claude's `--effort` list, else every level the agent's models report).
    pub fn efforts(&self, kind: Kind, model: Option<&str>) -> Vec<String> {
        if !kind.has_effort() {
            return Vec::new();
        }
        let models = self.models(kind);
        if let Some(m) = model.and_then(|id| models.iter().find(|x| x.id == id)).filter(|m| !m.efforts.is_empty()) {
            return m.efforts.clone();
        }
        if kind == Kind::Claude {
            return if self.claude_efforts.is_empty() { CLAUDE_EFFORTS.iter().map(|s| s.to_string()).collect() } else { self.claude_efforts.clone() };
        }
        let mut all: Vec<String> = Vec::new();
        for e in models.iter().flat_map(|m| &m.efforts) {
            if !all.contains(e) {
                all.push(e.clone());
            }
        }
        all
    }

    /// The model's default effort, when the CLI reports one (Codex).
    pub fn default_effort(&self, kind: Kind, model: Option<&str>) -> Option<String> {
        model.and_then(|id| self.models(kind).iter().find(|x| x.id == id)).and_then(|m| m.default_effort.clone())
    }
}

/// Claude Code's effort levels, when `claude --help` couldn't be read.
const CLAUDE_EFFORTS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];

/// The effort Claude Code falls back to without `--effort`: `effortLevel` from the settings
/// (local, then project, then user; `/effort` saves it in the user settings).
pub fn claude_settings_effort(cwd: &Path) -> Option<String> {
    let user = home().map(|h| h.join(".claude").join("settings.json"));
    [Some(cwd.join(".claude").join("settings.local.json")), Some(cwd.join(".claude").join("settings.json")), user].into_iter().flatten().find_map(|f| {
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(f).ok()?).ok()?;
        v["effortLevel"].as_str().filter(|e| !e.is_empty()).map(str::to_string)
    })
}

fn run_quiet(program: &str, args: &[&str]) -> Option<String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let out = std::process::Command::new(program)
        .args(args)
        // `agy` starts its daily background updater in a new console of its own, which our
        // CREATE_NO_WINDOW doesn't reach: a Windows Terminal window flashed up at start-up.
        // (It must be "true"; "1" is ignored.)
        .env("AGY_CLI_DISABLE_AUTO_UPDATE", "true")
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(std::process::Stdio::null())
        .output()
        .ok()?;
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

/// Parses `agy models`: one `id<TAB>name` line per model (other lines are progress text).
pub fn parse_agy_models(out: &str) -> Vec<ModelInfo> {
    out.lines()
        .filter_map(|l| {
            let (id, label) = l.trim().split_once('\t')?;
            (!id.is_empty() && !id.contains(' ')).then(|| ModelInfo { id: id.into(), label: label.trim().into(), efforts: Vec::new(), default_effort: None })
        })
        .collect()
}

pub fn load_catalog(claude: Option<String>, codex: Option<String>, agy: Option<String>) -> Catalog {
    let mut c = Catalog::default();
    // The aliases are listed even when `claude --help` can't be read.
    let (models, efforts) = claude.and_then(|p| run_quiet(&p, &["--help"])).map(|h| parse_claude_help(&h)).unwrap_or_default();
    c.claude_models = claude_models(&models, &efforts);
    c.claude_efforts = efforts;
    if let Some(json) = codex.and_then(|p| run_quiet(&p, &["debug", "models"])) {
        c.codex_models = parse_codex_models(&json);
    }
    if let Some(out) = agy.and_then(|p| run_quiet(&p, &["models"])) {
        c.agy_models = parse_agy_models(&out);
    }
    c
}

// ---------------------------------------------------------------------------------------------
// Transcripts: conversation ids and names

pub fn home() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from)
}

/// `~/.claude/projects/<cwd with every non-alphanumeric character as '-'>`.
pub fn claude_dir(cwd: &Path) -> Option<PathBuf> {
    let mangled: String = cwd.display().to_string().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    Some(home()?.join(".claude").join("projects").join(mangled))
}

/// Whether conversation `id` (of `kind`, run in `cwd`) can be resumed. Claude reports its id
/// (SessionStart hook) at startup but writes the transcript only with the first message, so a
/// session left untouched has an id with nothing behind it. The other agents' ids are only
/// known once their files exist.
pub fn conversation_exists(kind: Kind, cwd: &Path, id: Option<&str>) -> bool {
    match (kind, id) {
        (_, None) => false,
        (Kind::Claude, Some(id)) => claude_dir(cwd).is_some_and(|d| d.join(format!("{id}.jsonl")).is_file()),
        _ => true,
    }
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
pub fn civil_date(days: i64) -> (i64, i64, i64) {
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
    // Pasted images arrive as quoted paths ("C:\…\paste-….png"); they make poor names.
    let s = strip_image_paths(s);
    let line = s.lines().map(str::trim).find(|l| !l.is_empty())?;
    let mut out: String = line.chars().take(70).collect();
    if line.chars().count() > 70 {
        out.push('…');
    }
    Some(out)
}

/// `s` without quoted image file paths (and the space or colon left before them).
fn strip_image_paths(s: &str) -> String {
    const EXTS: [&str; 7] = [".png", ".jpg", ".jpeg", ".gif", ".bmp", ".webp", ".tif"];
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('"') {
        let after = &rest[i + 1..];
        let Some(j) = after.find('"') else { break };
        let inner = &after[..j];
        let is_img = !inner.contains('\n') && (inner.contains('\\') || inner.contains('/')) && EXTS.iter().any(|e| inner.to_lowercase().ends_with(e));
        if is_img {
            out.push_str(rest[..i].trim_end_matches([' ', ':']));
            rest = &after[j + 1..];
        } else {
            out.push_str(&rest[..i + 1]);
            rest = after;
        }
    }
    out.push_str(rest);
    out
}

/// A Claude session's name: its `custom-title` (set by /rename or `-n`), else Claude's
/// `ai-title`, else its first prompt.
pub fn claude_title(cwd: &Path, id: &str) -> Option<String> {
    let path = claude_dir(cwd)?.join(format!("{id}.jsonl"));
    if let Some(t) = claude_tail_title(&path) {
        return Some(t);
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

/// A Claude Code conversation found on disk (started by any app: the CLI, Claude's desktop app,
/// other terminals), for listing it here so it can be resumed.
#[derive(Clone, Debug)]
pub struct FoundClaude {
    pub id: String,
    /// The folder it ran in (Claude resumes it only from there).
    pub cwd: PathBuf,
    pub title: Option<String>,
    /// Unix seconds: first message, last write.
    pub created: u64,
    pub used: u64,
}

/// Every resumable Claude conversation in `~/.claude/projects` (top-level transcripts only, so
/// subagent runs are left out), skipping scripted runs (`claude -p` / SDK), conversations
/// without a typed prompt, folders that no longer exist or live under the temp folder, and
/// transcripts created at or after `before` (those belong to sessions starting right now).
pub fn scan_claude_conversations(before: SystemTime) -> Vec<FoundClaude> {
    let Some(root) = home().map(|h| h.join(".claude").join("projects")) else { return Vec::new() };
    let temp: Vec<String> = [std::env::var_os("TEMP"), std::env::var_os("TMP")]
        .into_iter()
        .flatten()
        .map(|t| norm_path(&PathBuf::from(t)))
        .chain(["c:\\temp\\claude".to_string()])
        .collect();
    let mut out = Vec::new();
    for dir in std::fs::read_dir(&root).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.is_dir()) {
        for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) != Some("jsonl") {
                continue;
            }
            let Ok(m) = e.metadata() else { continue };
            let created = m.created().or_else(|_| m.modified()).unwrap_or(SystemTime::UNIX_EPOCH);
            if created >= before {
                continue;
            }
            let Some(mut f) = read_claude_head(&p) else { continue };
            let n = norm_path(&f.cwd);
            if temp.iter().any(|t| n == *t || n.starts_with(&format!("{t}\\"))) || !f.cwd.is_dir() {
                continue;
            }
            let secs = |t: SystemTime| t.duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
            if f.created == 0 {
                f.created = secs(created);
            }
            f.used = secs(m.modified().unwrap_or(created));
            f.title = claude_tail_title(&p).or(f.title);
            out.push(f);
        }
    }
    out
}

fn norm_path(p: &Path) -> String {
    p.display().to_string().to_lowercase().replace('/', "\\").trim_end_matches('\\').to_string()
}

/// The transcript's id, folder, start time and first typed prompt (from its first lines), or
/// None when it isn't an interactive conversation.
fn read_claude_head(p: &Path) -> Option<FoundClaude> {
    let id = p.file_stem()?.to_str()?.to_string();
    let f = std::fs::File::open(p).ok()?;
    let mut cwd = None;
    let mut created = 0;
    for line in std::io::BufReader::new(f).take(8 << 20).lines().map_while(Result::ok).take(2000) {
        if !line.contains("\"type\":\"user\"") {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else { continue };
        if v["isSidechain"].as_bool() == Some(true) || v["entrypoint"].as_str().is_some_and(|e| e.starts_with("sdk")) {
            return None;
        }
        if cwd.is_none() {
            cwd = v["cwd"].as_str().filter(|c| !c.is_empty()).map(PathBuf::from);
            created = v["timestamp"].as_str().and_then(parse_iso_secs).unwrap_or(0);
        }
        let c = &v["message"]["content"];
        let text = c.as_str().map(str::to_string).or_else(|| c.as_array().and_then(|a| a.iter().find(|p| p["type"] == "text").and_then(|p| p["text"].as_str()).map(str::to_string)));
        if let Some(t) = text.as_deref().and_then(tidy_prompt) {
            return Some(FoundClaude { id, cwd: cwd?, title: Some(t), created, used: 0 });
        }
    }
    None
}

/// The last `custom-title` (set by /rename or -n), else the last `ai-title`, from the tail.
fn claude_tail_title(p: &Path) -> Option<String> {
    let mut f = std::fs::File::open(p).ok()?;
    let len = f.metadata().ok()?.len();
    let tail = 512 * 1024u64;
    f.seek(SeekFrom::Start(len.saturating_sub(tail))).ok()?;
    let mut bytes = Vec::new();
    let _ = f.take(tail).read_to_end(&mut bytes);
    let text = String::from_utf8_lossy(&bytes);
    let (mut custom, mut ai) = (None, None);
    for line in text.lines().filter(|l| l.contains("-title\"")) {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { continue };
        let pick = |k: &str| v[k].as_str().map(str::trim).filter(|t| !t.is_empty()).map(str::to_string);
        match v["type"].as_str() {
            Some("custom-title") => custom = pick("customTitle").or(custom),
            Some("ai-title") => ai = pick("aiTitle").or(ai),
            _ => {}
        }
    }
    custom.or(ai)
}

/// "2026-11-15T17:32:57.263Z" → Unix seconds (UTC).
fn parse_iso_secs(s: &str) -> Option<u64> {
    let num = |r: std::ops::Range<usize>| s.get(r)?.parse::<i64>().ok();
    let (y, mo, d, h, mi, se) = (num(0..4)?, num(5..7)?, num(8..10)?, num(11..13)?, num(14..16)?, num(17..19)?);
    // Days from the civil date (Howard Hinnant's algorithm).
    let y2 = if mo <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let doy = (153 * (if mo > 2 { mo - 3 } else { mo + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    u64::try_from(days * 86_400 + h * 3600 + mi * 60 + se).ok()
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
pub fn parse_utc(s: &str) -> Option<u64> {
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

// ---------------------------------------------------------------------------------------------
// Antigravity CLI conversations: `~/.gemini/antigravity-cli/conversations/<id>.db`, one SQLite
// file each. Its `steps` table holds protobuf messages: step type 14 = the user's input (text in
// payload field 19.2), 15 = the agent's turn (reply text in 20.1, absent for tool calls); a
// step's metadata starts with a Timestamp (field 1). `trajectory_metadata_blob` field 1.1 is the
// workspace as a file:// URI. Read without the schema, by field number.

/// A protobuf field value (wire types varint and length-delimited; fixed-size ones are skipped).
enum Pb<'a> {
    Int(u64),
    Bytes(&'a [u8]),
}

fn pb_varint(b: &[u8], i: &mut usize) -> Option<u64> {
    let mut v = 0u64;
    for shift in (0..64).step_by(7) {
        let x = *b.get(*i)?;
        *i += 1;
        v |= ((x & 0x7f) as u64) << shift;
        if x < 0x80 {
            return Some(v);
        }
    }
    None
}

/// The fields of a message, in order (stops at the first malformed one).
fn pb_fields(b: &[u8]) -> Vec<(u64, Pb<'_>)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let Some(key) = pb_varint(b, &mut i) else { break };
        let (num, wire) = (key >> 3, key & 7);
        let v = match wire {
            0 => match pb_varint(b, &mut i) {
                Some(v) => Pb::Int(v),
                None => break,
            },
            2 => {
                let Some(n) = pb_varint(b, &mut i).and_then(|n| usize::try_from(n).ok()) else { break };
                let Some(s) = b.get(i..i.saturating_add(n)) else { break };
                i += n;
                Pb::Bytes(s)
            }
            1 => {
                i += 8;
                continue;
            }
            5 => {
                i += 4;
                continue;
            }
            _ => break,
        };
        out.push((num, v));
    }
    out
}

/// The first length-delimited field along `path` (field numbers, descending into each).
fn pb_get<'a>(b: &'a [u8], path: &[u64]) -> Option<&'a [u8]> {
    let (first, rest) = path.split_first()?;
    let v = pb_fields(b).into_iter().find_map(|(n, v)| match v {
        Pb::Bytes(s) if n == *first => Some(s),
        _ => None,
    })?;
    if rest.is_empty() {
        Some(v)
    } else {
        pb_get(v, rest)
    }
}

fn pb_int(b: &[u8], num: u64) -> Option<u64> {
    pb_fields(b).into_iter().find_map(|(n, v)| match v {
        Pb::Int(x) if n == num => Some(x),
        _ => None,
    })
}

fn agy_conversations() -> Option<PathBuf> {
    Some(home()?.join(".gemini").join("antigravity-cli").join("conversations"))
}

/// An Antigravity conversation's database file.
pub fn agy_db(id: &str) -> Option<PathBuf> {
    let p = agy_conversations()?.join(format!("{id}.db"));
    p.is_file().then_some(p)
}

fn agy_open(p: &Path) -> Option<rusqlite::Connection> {
    use rusqlite::OpenFlags;
    let c = rusqlite::Connection::open_with_flags(p, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX).ok()?;
    let _ = c.busy_timeout(std::time::Duration::from_millis(500));
    Some(c)
}

/// "file:///x%3A/Some/Dir" → "x:\some\dir" (to compare with a project folder).
fn file_uri_path(u: &str) -> String {
    let s = u.strip_prefix("file:///").unwrap_or(u);
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            if let Some(x) = s.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(x);
                i += 3;
                continue;
            }
        }
        out.push(if b[i] == b'/' { b'\\' } else { b[i] });
        i += 1;
    }
    String::from_utf8_lossy(&out).to_lowercase().trim_end_matches('\\').to_string()
}

/// The workspace folder an Antigravity conversation was started in.
fn agy_workspace(c: &rusqlite::Connection) -> Option<String> {
    let data: Vec<u8> = c.query_row("SELECT data FROM trajectory_metadata_blob LIMIT 1", [], |r| r.get(0)).ok()?;
    Some(file_uri_path(std::str::from_utf8(pb_get(&data, &[1, 1])?).ok()?))
}

/// The Antigravity conversation started in `cwd` since `since` (the oldest unclaimed one).
pub fn find_agy_id(cwd: &Path, since: SystemTime, claimed: &std::collections::HashSet<String>) -> Option<String> {
    let since = since - std::time::Duration::from_secs(2);
    let want = cwd.display().to_string().to_lowercase().trim_end_matches('\\').to_string();
    let mut best: Option<(SystemTime, String)> = None;
    for e in std::fs::read_dir(agy_conversations()?).ok()?.flatten() {
        let p = e.path();
        if p.extension().and_then(|x| x.to_str()) != Some("db") {
            continue;
        }
        let Some(stem) = p.file_stem().and_then(|x| x.to_str()).map(str::to_string) else { continue };
        let Ok(m) = e.metadata() else { continue };
        let created = m.created().or_else(|_| m.modified()).unwrap_or(std::time::UNIX_EPOCH);
        if claimed.contains(&stem) || created < since || best.as_ref().is_some_and(|(t, _)| created >= *t) {
            continue;
        }
        if agy_open(&p).and_then(|c| agy_workspace(&c)).as_deref() == Some(want.as_str()) {
            best = Some((created, stem));
        }
    }
    best.map(|(_, id)| id)
}

/// One message of an Antigravity conversation.
pub struct AgyMessage {
    pub user: bool,
    /// Unix seconds.
    pub time: Option<u64>,
    pub text: String,
}

/// The user's and the agent's messages, in order (tool calls left out).
pub fn agy_messages(id: &str) -> Vec<AgyMessage> {
    let Some(c) = agy_db(id).and_then(|p| agy_open(&p)) else { return Vec::new() };
    let Ok(mut q) = c.prepare("SELECT step_type, metadata, step_payload FROM steps WHERE step_type IN (14, 15) ORDER BY idx") else { return Vec::new() };
    let rows = q.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Option<Vec<u8>>>(1)?, r.get::<_, Option<Vec<u8>>>(2)?)));
    let Ok(rows) = rows else { return Vec::new() };
    rows.flatten()
        .filter_map(|(kind, meta, payload)| {
            let user = kind == 14;
            let text = pb_get(payload.as_deref()?, if user { &[19, 2] } else { &[20, 1] })?;
            let text = std::str::from_utf8(text).ok()?.to_string();
            let time = meta.as_deref().and_then(|m| pb_get(m, &[1])).and_then(|t| pb_int(t, 1));
            (!text.trim().is_empty()).then_some(AgyMessage { user, time, text })
        })
        .collect()
}

/// An Antigravity session's name: its first prompt.
pub fn agy_title(id: &str) -> Option<String> {
    agy_messages(id).into_iter().filter(|m| m.user).find_map(|m| tidy_prompt(&m.text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_names_skip_pasted_images() {
        assert_eq!(tidy_prompt("\"C:\\Temp\\AI_Terminal\\paste\\paste-1.png\"\nwhy I do not see publish here?").as_deref(), Some("why I do not see publish here?"));
        assert_eq!(tidy_prompt("what does this say: \"C:\\Temp\\p\\paste-2.png\" ?").as_deref(), Some("what does this say ?"));
        assert_eq!(tidy_prompt("\"C:\\Temp\\p\\paste-3.png\""), None);
        assert_eq!(tidy_prompt("say \"hello\" twice").as_deref(), Some("say \"hello\" twice"));
    }

    #[test]
    fn iso_timestamps() {
        assert_eq!(parse_iso_secs("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_iso_secs("2026-11-15T17:32:57.263Z"), Some(1_794_763_977));
        assert_eq!(parse_iso_secs("2000-02-29T12:00:00Z"), Some(951_825_600));
        assert_eq!(parse_iso_secs("garbage"), None);
    }

    #[test]
    fn claude_transcripts_found() {
        let dir = std::env::temp_dir().join(format!("ait_scan_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let cwd = r"X:\\Proj";
        let write = |name: &str, lines: &[String]| {
            let p = dir.join(format!("{name}.jsonl"));
            std::fs::write(&p, lines.join("\n")).unwrap();
            p
        };
        let user = |extra: &str, text: &str| format!(r#"{{"type":"user","isSidechain":false,"cwd":"{cwd}","timestamp":"2026-11-15T17:32:57Z"{extra},"message":{{"role":"user","content":"{text}"}}}}"#);
        // Interactive: titled by the last custom-title, else the ai-title.
        let a = write("a", &[r#"{"type":"mode"}"#.into(), user(r#","entrypoint":"cli""#, "<command-name>/clear</command-name>"), user("", "Fix the build"), r#"{"type":"ai-title","aiTitle":"Fixing build"}"#.into()]);
        let f = read_claude_head(&a).unwrap();
        assert_eq!((f.id.as_str(), f.cwd, f.title.as_deref(), f.created), ("a", PathBuf::from(r"X:\Proj"), Some("Fix the build"), 1_794_763_977));
        assert_eq!(claude_tail_title(&a).as_deref(), Some("Fixing build"));
        let b = write("b", &[user("", "hi"), r#"{"type":"custom-title","customTitle":"Mine"}"#.into(), r#"{"type":"ai-title","aiTitle":"Later"}"#.into()]);
        assert_eq!(claude_tail_title(&b).as_deref(), Some("Mine"));
        // Scripted runs and conversations without a typed prompt are skipped.
        assert!(read_claude_head(&write("c", &[user(r#","entrypoint":"sdk-cli""#, "commit msg")])).is_none());
        assert!(read_claude_head(&write("d", &[r#"{"type":"mode"}"#.into()])).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn antigravity_resumes_by_conversation() {
        let o = LaunchOpts { initial_prompt: Some("hi".into()), ..Default::default() };
        assert_eq!(build_args(Kind::Antigravity, &o, "", Some("abc"), "k"), vec!["--conversation", "abc"]);
    }

    #[test]
    fn protobuf_paths_and_uris() {
        // Field 19 { field 2: "hi" }, then field 1 = varint 150.
        let msg = [0x9a, 0x01, 0x04, 0x12, 0x02, b'h', b'i', 0x08, 0x96, 0x01];
        assert_eq!(pb_get(&msg, &[19, 2]), Some(&b"hi"[..]));
        assert_eq!(pb_int(&msg, 1), Some(150));
        assert!(pb_get(&msg, &[20, 1]).is_none());
        assert_eq!(file_uri_path("file:///x%3A/GameArtists/Other/AI_Terminal/"), "x:\\gameartists\\other\\ai_terminal");
    }

    /// Reads the real Antigravity store (run by hand: `build.cmd test -- --ignored agy`).
    #[test]
    #[ignore]
    fn agy_live_messages() {
        for e in std::fs::read_dir(agy_conversations().unwrap()).unwrap().flatten().filter(|e| e.path().extension().is_some_and(|x| x == "db")) {
            let id = e.path().file_stem().unwrap().to_string_lossy().to_string();
            let ws = agy_open(&e.path()).and_then(|c| agy_workspace(&c));
            let msgs = agy_messages(&id);
            println!("{id} {ws:?} {} messages, title {:?}", msgs.len(), agy_title(&id));
            for m in msgs.iter().take(4) {
                println!("  {} {:?} {}", if m.user { "user " } else { "agent" }, m.time, m.text.chars().take(80).collect::<String>());
            }
        }
    }

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

    #[test]
    fn agy_model_list() {
        let m = parse_agy_models("Fetching available models...\ngemini-3.8-flash-low\tGemini 3.8 Flash (Low)\nclaude-opus-5-5-high\tClaude Opus 5.5 (High)\n");
        let ids: Vec<&str> = m.iter().map(|x| x.id.as_str()).collect();
        assert_eq!(ids, vec!["gemini-3.8-flash-low", "claude-opus-5-5-high"]);
        assert_eq!(m[0].label, "Gemini 3.8 Flash (Low)");
    }
}
