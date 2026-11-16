//! One terminal session: a ConPTY child process plus the alacritty VT emulator that parses its
//! output. The PTY I/O runs on alacritty's own event-loop thread; the UI reads the grid under
//! the terminal lock each frame.

use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::event_loop::{EventLoop, EventLoopSender, Msg};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::{self, Term};
use alacritty_terminal::tty;

use crate::agents::{Kind, LaunchOpts};

/// Things the terminal thread reports back to the UI.
#[derive(Default)]
pub struct Shared {
    pub title: Option<String>,
    pub exited: Option<i32>,
    pub bell: bool,
    /// Text the app asked to put on the clipboard (OSC 52).
    pub clipboard: Option<String>,
    /// Replies the emulator wants written back to the PTY (device status reports etc.).
    pub pty_writes: Vec<String>,
}

#[derive(Clone)]
pub struct Listener {
    shared: Arc<Mutex<Shared>>,
    ctx: egui::Context,
    dirty: Arc<AtomicBool>,
    /// Milliseconds (since the session started) of the last output, for idle detection.
    last_output: Arc<std::sync::atomic::AtomicU64>,
    started: std::time::Instant,
}

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        let mut s = self.shared.lock().unwrap();
        match event {
            Event::Title(t) => s.title = Some(t),
            Event::ResetTitle => s.title = None,
            Event::ChildExit(status) => s.exited = Some(status.code().unwrap_or(-1)),
            Event::Exit => {
                if s.exited.is_none() {
                    s.exited = Some(0);
                }
            }
            Event::Bell => s.bell = true,
            Event::ClipboardStore(_, text) => s.clipboard = Some(text),
            Event::PtyWrite(text) => s.pty_writes.push(text),
            Event::Wakeup | Event::MouseCursorDirty | Event::CursorBlinkingChange => {}
            // Colour / size queries are answered with defaults by the emulator itself
            // when unhandled; clipboard reads (OSC 52 load) are refused.
            Event::ClipboardLoad(..) | Event::ColorRequest(..) | Event::TextAreaSizeRequest(_) => {}
        }
        drop(s);
        self.last_output.store(self.started.elapsed().as_millis() as u64, Ordering::Relaxed);
        self.dirty.store(true, Ordering::Relaxed);
        self.ctx.request_repaint();
    }
}

/// Grid size handed to `Term::new` / `Term::resize`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct GridSize {
    pub cols: usize,
    pub rows: usize,
}

impl Dimensions for GridSize {
    fn total_lines(&self) -> usize {
        self.rows
    }
    fn screen_lines(&self) -> usize {
        self.rows
    }
    fn columns(&self) -> usize {
        self.cols
    }
}

/// A shell profile: what a new session runs.
#[derive(Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, Debug)]
pub struct Profile {
    pub name: String,
    pub program: String,
    pub args: Vec<String>,
    #[serde(default)]
    pub kind: Kind,
}

impl Profile {
    /// Profiles: Claude, Codex and the other agent CLIs (when installed), then the shells
    /// pwsh (if on PATH), powershell and cmd. An executable that isn't an .exe (an npm .cmd
    /// shim) is started through `cmd /s /c`.
    pub fn detect() -> Vec<Profile> {
        refresh_path();
        let comspec = std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".into());
        let agent = |name: &str, exe: &str, kind: Kind| -> Option<Profile> {
            if let Some(p) = find_on_path(&format!("{exe}.exe")).or_else(|| extra_agent_dirs(exe).into_iter().find(|p| p.is_file())) {
                return Some(Profile { name: name.into(), program: p.display().to_string(), args: vec![], kind });
            }
            find_on_path(&format!("{exe}.cmd")).map(|_| Profile { name: name.into(), program: comspec.clone(), args: vec!["/s".into(), "/c".into(), exe.into()], kind })
        };
        let mut v: Vec<Profile> = Vec::new();
        v.extend(agent(CLAUDE, "claude", Kind::Claude));
        v.extend(agent(CODEX, "codex", Kind::Codex).or_else(|| {
            // The Codex desktop app (Microsoft Store) bundles the CLI.
            let exe = crate::winsys::package_root("OpenAI.Codex_")?.join("app").join("resources").join("codex.exe");
            exe.is_file().then(|| Profile { name: CODEX.into(), program: exe.display().to_string(), args: vec![], kind: Kind::Codex })
        }));
        for s in crate::agents::AGENTS.iter().filter(|s| !matches!(s.kind, Kind::Claude | Kind::Codex)) {
            v.extend(agent(s.name, s.exe, s.kind));
        }
        if let Some(p) = find_on_path("pwsh.exe") {
            v.push(Profile { name: "pwsh".into(), program: p.display().to_string(), args: vec![], kind: Kind::Shell });
        }
        v.push(Profile { name: "powershell".into(), program: "powershell.exe".into(), args: vec![], kind: Kind::Shell });
        v.push(Profile { name: "cmd".into(), program: comspec, args: vec![], kind: Kind::Shell });
        v
    }

    pub fn is_claude(&self) -> bool {
        self.kind == Kind::Claude
    }

    pub fn is_agent(&self) -> bool {
        self.kind != Kind::Shell
    }

    /// Fills in `kind` for profiles saved before it existed.
    pub fn fix_kind(&mut self) {
        if self.kind == Kind::Shell {
            let n = self.name.to_lowercase();
            if n.contains("claude") {
                self.kind = Kind::Claude;
            } else if n.contains("codex") {
                self.kind = Kind::Codex;
            } else if let Some(s) = crate::agents::AGENTS.iter().find(|s| n == s.name.to_lowercase()) {
                self.kind = s.kind;
            }
        }
    }
}

pub const CLAUDE: &str = "Claude";
pub const CODEX: &str = "Codex";

/// Where we also look for the agent CLIs when they aren't on PATH.
pub(crate) fn extra_agent_dirs(exe: &str) -> Vec<PathBuf> {
    let mut v = Vec::new();
    let file = format!("{exe}.exe");
    if let Some(l) = std::env::var_os("LOCALAPPDATA").map(PathBuf::from) {
        if exe == "claude" {
            v.push(l.join("AnthropicClaude").join(&file));
            v.push(l.join("Programs").join("AnthropicClaude").join(&file));
        } else if exe == "codex" {
            v.push(l.join("Programs").join("OpenAI").join("Codex").join("bin").join(&file));
        } else if exe == "agy" {
            v.push(l.join("agy").join("bin").join(&file));
        }
    }
    if let Some(h) = std::env::var_os("USERPROFILE").map(PathBuf::from) {
        v.push(h.join(".local").join("bin").join(&file));
        v.push(h.join(".npm").join("bin").join(&file));
    }
    v
}

/// Appends to our PATH the folders the registry PATH gained since we started (an agent CLI
/// installed meanwhile), so detection and the sessions we start (which inherit it) see them.
pub(crate) fn refresh_path() {
    #[cfg(windows)]
    {
        let mut cur: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
        let norm = |p: &Path| p.to_string_lossy().trim_end_matches(['\\', '/']).to_lowercase();
        let mut added = false;
        for d in crate::winsys::registry_path() {
            if !cur.iter().any(|c| norm(c) == norm(&d)) {
                cur.push(d);
                added = true;
            }
        }
        if added {
            if let Ok(p) = std::env::join_paths(cur) {
                std::env::set_var("PATH", p);
            }
        }
    }
}

pub(crate) fn find_on_path(exe: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?).map(|d| d.join(exe)).find(|p| p.is_file())
}

pub struct Session {
    pub id: u64,
    pub name: String,
    pub profile: Profile,
    pub cwd: PathBuf,
    pub term: Arc<FairMutex<Term<Listener>>>,
    pub shared: Arc<Mutex<Shared>>,
    sender: EventLoopSender,
    pub size: GridSize,
    pub cell: (u16, u16),
    /// Images pasted into this session, in order (for `[Image #N]` placeholders).
    pub pasted_images: Vec<PathBuf>,
    /// `[Image #N]` maps to `pasted_images[image_base + N - 1]`; reset by `/clear`.
    pub image_base: usize,
    /// What was typed on the current input line (best effort, for detecting `/clear`).
    pub line_buf: String,
    pub title: Option<String>,
    pub exited: Option<i32>,
    /// Output arrived while the session wasn't visible.
    pub unseen: bool,
    pub dirty: Arc<AtomicBool>,
    /// The Claude / Codex conversation this session runs (for resume), once known.
    pub agent_id: Option<String>,
    /// Launch settings (unrestricted, model, effort, CLI parameters), reused on restart.
    pub opts: LaunchOpts,
    /// Name from the transcript (custom title or first prompt).
    pub transcript_title: Option<String>,
    spawned_at: std::time::SystemTime,
    last_scan: Option<std::time::Instant>,
    last_title_scan: Option<std::time::Instant>,
    /// Estimated cost / tokens so far (from the transcript), refreshed every 5 s.
    pub cost: Option<crate::cost::Summary>,
    /// The model answering, from the latest reply in the transcript (e.g. "claude-opus-5-5").
    pub model: Option<String>,
    cost_tracker: crate::cost::Tracker,
    /// Identifies this session in hook events (the AI_TERMINAL_SESSION variable).
    pub hook_key: String,
    /// Claude is waiting for you (Stop / Notification hook, or a bell); the message if any.
    pub attention: Option<String>,
    /// Claude is working (between UserPromptSubmit / PreToolUse and Stop).
    pub busy: bool,
    started: std::time::Instant,
    last_output: Arc<std::sync::atomic::AtomicU64>,
    last_input: std::time::Instant,
}

impl Session {
    /// Starts `profile` in `cwd` with `opts`. For Claude / Codex, `resume` continues that
    /// conversation.
    pub fn spawn(id: u64, name: String, profile: Profile, cwd: PathBuf, resume: Option<String>, opts: LaunchOpts, ctx: &egui::Context) -> std::io::Result<Session> {
        let size = GridSize { cols: 120, rows: 30 };
        let cell = (9u16, 18u16);
        let shared = Arc::new(Mutex::new(Shared::default()));
        let dirty = Arc::new(AtomicBool::new(true));
        let started = std::time::Instant::now();
        let last_output = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let listener = Listener { shared: shared.clone(), ctx: ctx.clone(), dirty: dirty.clone(), last_output: last_output.clone(), started };
        let hook_key = format!("{}-{}-{}", std::process::id(), id, std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0));
        let config = term::Config { scrolling_history: 20000, ..Default::default() };
        let term = Arc::new(FairMutex::new(Term::new(config, &size, listener.clone())));

        let mut env = std::collections::HashMap::new();
        env.insert("TERM".to_string(), "xterm-256color".to_string());
        env.insert("COLORTERM".to_string(), "truecolor".to_string());
        env.insert("TERM_PROGRAM".to_string(), "AI_Terminal".to_string());
        env.insert(crate::hooks::ENV_KEY.to_string(), hook_key.clone());
        let options = tty::Options {
            // alacritty joins program + args into one command line; quote a program path
            // with spaces (C:\Program Files\...).
            shell: Some(tty::Shell::new(
                if profile.program.contains(' ') && !profile.program.starts_with('"') { format!("\"{}\"", profile.program) } else { profile.program.clone() },
                {
                    let mut a = profile.args.clone();
                    a.extend(crate::agents::build_args(profile.kind, &opts, &name, resume.as_deref(), &hook_key));
                    a
                },
            )),
            working_directory: cwd.is_dir().then(|| cwd.clone()),
            drain_on_exit: true,
            env,
            // Quote each argument (names, paths, -c key="value") with Windows CRT rules.
            escape_args: true,
        };
        let ws = WindowSize { num_lines: size.rows as u16, num_cols: size.cols as u16, cell_width: cell.0, cell_height: cell.1 };
        crate::dlog!("spawn #{id} {} in {}: {:?}", profile.name, cwd.display(), options.shell.as_ref().map(|_| crate::agents::build_args(profile.kind, &opts, &name, resume.as_deref(), &hook_key)));
        let pty = tty::new(&options, ws, id)?;
        let event_loop = EventLoop::new(term.clone(), listener, pty, true, false)?;
        let sender = event_loop.channel();
        event_loop.spawn();

        Ok(Session {
            id,
            name,
            profile,
            cwd,
            term,
            shared,
            sender,
            size,
            cell,
            pasted_images: Vec::new(),
            image_base: 0,
            line_buf: String::new(),
            title: None,
            exited: None,
            unseen: false,
            dirty,
            agent_id: resume,
            opts,
            transcript_title: None,
            spawned_at: std::time::SystemTime::now(),
            last_scan: None,
            last_title_scan: None,
            hook_key,
            cost: None,
            model: None,
            cost_tracker: Default::default(),
            attention: None,
            busy: false,
            started,
            last_output,
            last_input: std::time::Instant::now(),
        })
    }

    /// The conversation id Claude reported (SessionStart hook: startup, resume, /clear, compact).
    /// After /clear it's a new conversation: point resume, title and cost at it.
    pub fn set_conversation(&mut self, id: &str) {
        if id.is_empty() || self.agent_id.as_deref() == Some(id) {
            return;
        }
        self.agent_id = Some(id.to_string());
        self.transcript_title = None;
        self.cost = None;
        self.model = None;
        self.cost_tracker = Default::default();
        self.last_title_scan = None;
    }

    /// Links a new Claude / Codex / Antigravity session to its conversation (the agent writes the transcript
    /// with the first message, so this is polled every 2 s), and refreshes the session's name
    /// from the transcript every 5 s.
    pub fn poll_agent(&mut self, claimed: &std::collections::HashSet<String>) {
        if !self.profile.is_agent() {
            return;
        }
        if self.agent_id.is_none() && self.last_scan.is_none_or(|t| t.elapsed() >= std::time::Duration::from_secs(2)) {
            self.last_scan = Some(std::time::Instant::now());
            self.agent_id = match self.profile.kind {
                Kind::Claude => crate::agents::find_claude_id(&self.cwd, self.spawned_at, claimed),
                Kind::Codex => crate::agents::find_codex_id(&self.cwd, self.spawned_at, claimed),
                Kind::Antigravity => crate::agents::find_agy_id(&self.cwd, self.spawned_at, claimed),
                _ => None,
            };
        }
        if let Some(id) = self.agent_id.clone() {
            if self.last_title_scan.is_none_or(|t| t.elapsed() >= std::time::Duration::from_secs(5)) {
                self.last_title_scan = Some(std::time::Instant::now());
                let t = match self.profile.kind {
                    Kind::Claude => crate::agents::claude_title(&self.cwd, &id),
                    Kind::Codex => crate::agents::codex_title(&id),
                    Kind::Antigravity => crate::agents::agy_title(&id),
                    _ => None,
                };
                if t.is_some() {
                    self.transcript_title = t;
                }
                let c = match self.profile.kind {
                    Kind::Claude => self.cost_tracker.update_claude(&self.cwd, &id),
                    Kind::Codex => self.cost_tracker.update_codex(&id),
                    // Antigravity: cost isn't read from its conversations.
                    _ => Default::default(),
                };
                if c.usage.total() > 0 {
                    self.cost = Some(c);
                }
                if let Some(m) = self.cost_tracker.model() {
                    if self.model.as_deref() != Some(m) {
                        self.model = Some(m.to_string());
                    }
                }
            }
        }
    }

    pub fn write(&mut self, bytes: impl Into<Cow<'static, [u8]>>) {
        if self.exited.is_some() {
            return;
        }
        let bytes = bytes.into();
        self.last_input = std::time::Instant::now();
        self.track_line(&bytes);
        let _ = self.sender.send(Msg::Input(bytes));
    }

    pub fn write_str(&mut self, s: &str) {
        self.write(s.as_bytes().to_vec());
    }

    /// Keeps a rough copy of the line being typed, so `/clear` (which restarts Claude Code's
    /// image numbering) can be noticed when Enter is pressed.
    fn track_line(&mut self, bytes: &[u8]) {
        let text = String::from_utf8_lossy(bytes);
        let mut chars = text.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '\r' | '\n' => {
                    let cmd = self.line_buf.trim();
                    if matches!(cmd, "/clear" | "/new" | "/reset") {
                        self.image_base = self.pasted_images.len();
                    }
                    self.line_buf.clear();
                }
                '\x7f' | '\x08' => {
                    self.line_buf.pop();
                }
                '\x03' | '\x15' => self.line_buf.clear(),
                '\x1b' => {
                    // Skip an escape sequence (arrow keys etc.).
                    if chars.peek() == Some(&'[') || chars.peek() == Some(&'O') {
                        chars.next();
                        while let Some(&n) = chars.peek() {
                            chars.next();
                            if n.is_ascii_alphabetic() || n == '~' {
                                break;
                            }
                        }
                    }
                }
                c if !c.is_control() => {
                    if self.line_buf.len() < 256 {
                        self.line_buf.push(c);
                    }
                }
                _ => {}
            }
        }
    }

    pub fn resize(&mut self, size: GridSize, cell: (u16, u16)) {
        if size == self.size && cell == self.cell {
            return;
        }
        self.size = size;
        self.cell = cell;
        self.term.lock().resize(size);
        let ws = WindowSize { num_lines: size.rows as u16, num_cols: size.cols as u16, cell_width: cell.0, cell_height: cell.1 };
        let _ = self.sender.send(Msg::Resize(ws));
    }

    /// Pulls events from the terminal thread: OSC 52 clipboard text, and whether it rang the bell.
    pub fn poll(&mut self) -> (Option<String>, bool) {
        let (writes, clip, bell) = {
            let mut s = self.shared.lock().unwrap();
            self.title = s.title.clone();
            if self.exited.is_none() {
                self.exited = s.exited;
            }
            (std::mem::take(&mut s.pty_writes), s.clipboard.take(), std::mem::take(&mut s.bell))
        };
        for w in writes {
            let _ = self.sender.send(Msg::Input(w.into_bytes().into()));
        }
        (clip, bell)
    }

    /// Time since the last output or keystroke.
    pub fn idle_for(&self) -> std::time::Duration {
        let out = self.started + std::time::Duration::from_millis(self.last_output.load(Ordering::Relaxed));
        let last = out.max(self.last_input);
        last.elapsed()
    }

    /// Label for the sidebar: the custom name, else the title the program set.
    pub fn display_name(&self) -> String {
        if !self.name.is_empty() {
            return self.name.clone();
        }
        if let Some(t) = &self.transcript_title {
            return t.clone();
        }
        if self.profile.kind.tracks_conversation() {
            return "New session".into();
        }
        match self.title.as_deref().map(str::trim) {
            Some(t) if !t.is_empty() => pretty_title(t, &self.profile.name),
            _ => self.profile.name.clone(),
        }
    }

    /// The image `[Image #n]` refers to (1-based), if this session pasted it.
    pub fn image_for_placeholder(&self, n: usize) -> Option<&PathBuf> {
        if n == 0 {
            return None;
        }
        self.pasted_images.get(self.image_base + n - 1).or_else(|| self.pasted_images.get(n - 1))
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.sender.send(Msg::Shutdown);
    }
}

/// Window titles like `C:\WINDOWS\system32\cmd.exe - claude` read badly in the sidebar:
/// a leading executable path becomes the profile name (`Command Prompt - claude`).
pub fn pretty_title(t: &str, profile: &str) -> String {
    // Claude Code prefixes its title with a spinner / status glyph (✳ ✶ ⠂ …); drop it.
    let t = t.trim_start_matches(|c: char| !c.is_alphanumeric() && !"\"'([<".contains(c)).trim();
    let lower = t.to_ascii_lowercase();
    if let Some(i) = lower.find(".exe") {
        let head = &t[..i];
        if head.contains(['\\', '/']) {
            let rest = t[i + 4..].trim_start_matches([' ', '-']).trim();
            return if rest.is_empty() { profile.to_string() } else { format!("{profile} - {rest}") };
        }
    }
    t.to_string()
}

#[cfg(test)]
mod tests {
    #[test]
    fn pretty_titles() {
        use super::pretty_title;
        assert_eq!(pretty_title(r"C:\WINDOWS\system32\cmd.exe", "Command Prompt"), "Command Prompt");
        assert_eq!(pretty_title(r"C:\WINDOWS\system32\cmd.exe - claude", "Claude Code"), "Claude Code - claude");
        assert_eq!(pretty_title("✳ Fix the build", "Claude Code"), "Fix the build");
        assert_eq!(pretty_title("⠂ Build status table", "Claude"), "Build status table");
    }
}
