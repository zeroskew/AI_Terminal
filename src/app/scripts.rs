//! Scripts: files in the `scripts` folder next to the executable, run from a palette in the
//! current project in the background (no session): output shows in a small panel, which also
//! takes typed answers for scripts that ask questions.

use super::*;

/// A runnable file in the scripts folder.
#[derive(Clone)]
pub(super) struct Script {
    pub name: String,
    pub path: PathBuf,
    /// The first comment line, else the file type.
    pub desc: String,
}

/// File types the palette lists.
const KINDS: [&str; 4] = ["ps1", "cmd", "bat", "py"];

const SAMPLE_NAME: &str = "Commit and push.ps1";
const SAMPLE: &str = include_str!("../../assets/scripts/Commit and push.ps1");

/// `<exe folder>\scripts`.
pub(super) fn scripts_dir() -> PathBuf {
    std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join("scripts"))).unwrap_or_else(|| PathBuf::from("scripts"))
}

/// Lists the scripts, sorted by name. Creates the folder with a sample the first time.
fn scan() -> Vec<Script> {
    let dir = scripts_dir();
    if !dir.exists() && std::fs::create_dir_all(&dir).is_ok() {
        let _ = std::fs::write(dir.join(SAMPLE_NAME), SAMPLE);
    }
    let mut v: Vec<Script> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .filter_map(|path| {
            let ext = path.extension()?.to_str()?.to_lowercase();
            if !KINDS.contains(&ext.as_str()) {
                return None;
            }
            let name = path.file_stem()?.to_string_lossy().into_owned();
            let desc = first_comment(&std::fs::read_to_string(&path).unwrap_or_default()).unwrap_or_else(|| format!(".{ext} script"));
            Some(Script { name, path, desc })
        })
        .collect();
    v.sort_by_key(|s| s.name.to_lowercase());
    v
}

/// The first comment line (`#`, `::`, `REM`), skipping `@echo off` and blank lines.
fn first_comment(text: &str) -> Option<String> {
    for line in text.trim_start_matches('\u{feff}').lines().take(10) {
        let l = line.trim();
        if l.is_empty() || l.eq_ignore_ascii_case("@echo off") || l.starts_with("#!") {
            continue;
        }
        let body = if let Some(b) = l.strip_prefix("::") {
            b
        } else if l.len() >= 4 && l[..4].eq_ignore_ascii_case("rem ") {
            &l[4..]
        } else if let Some(b) = l.strip_prefix('#') {
            b
        } else {
            return None;
        };
        let b = body.trim();
        if !b.is_empty() {
            return Some(b.to_string());
        }
    }
    None
}

impl App {
    pub(super) fn open_scripts(&mut self) {
        if self.s.projects.is_empty() {
            return;
        }
        self.scripts = scan();
        self.scripts_open = true;
        self.script_focus = true;
    }

    /// Test hook (--run-script <name>): runs the script with that name in the current project.
    pub(super) fn run_script_named(&mut self, ctx: &egui::Context, name: &str) {
        if let Some(sc) = scan().into_iter().find(|s| s.name.eq_ignore_ascii_case(name)) {
            self.run_script(ctx, &sc);
        }
    }

    /// Starts a script in the background in the current project; its panel shows the output.
    fn run_script(&mut self, ctx: &egui::Context, sc: &Script) {
        let project = self.current_project();
        if !project.is_dir() {
            return self.toast(format!("Folder not found: {}", project.display()), RED, ctx);
        }
        let (program, args) = script_command(sc, self.pwsh());
        self.run_in_background(ctx, sc.name.clone(), program, args, project, None);
    }

    /// pwsh if it's installed, else Windows PowerShell.
    pub(super) fn pwsh(&self) -> String {
        self.profiles.iter().find(|p| p.name == "pwsh").map(|p| p.program.clone()).unwrap_or_else(|| "powershell.exe".into())
    }

    /// Runs a program hidden, with its output in a panel at the bottom right. `agent`: an
    /// install / uninstall of that agent CLI (the agent list is re-detected when it ends).
    pub(super) fn run_in_background(&mut self, ctx: &egui::Context, name: String, program: String, args: Vec<String>, cwd: PathBuf, agent: Option<Kind>) {
        match ScriptRun::start(ctx, name.clone(), program, args, cwd, agent) {
            Ok(r) => self.script_runs.push(r),
            Err(e) => self.toast(format!("Couldn't run {name}: {e}"), RED, ctx),
        }
    }

    /// Whether an install / uninstall of that agent is running.
    pub(super) fn agent_job_running(&self, kind: Kind) -> bool {
        self.script_runs.iter().any(|r| r.agent == Some(kind) && r.exit.is_none())
    }

    /// The panels of running and finished scripts, stacked up from the bottom right.
    pub(super) fn script_runs_ui(&mut self, ctx: &egui::Context) {
        let mut close = None;
        let mut again = None;
        let mut done_toasts = Vec::new();
        let mut redetect = false;
        let mut y = 34.0;
        for (i, r) in self.script_runs.iter_mut().enumerate() {
            if let Some(code) = r.poll() {
                redetect |= r.agent.is_some();
                done_toasts.push(if code == 0 { (format!("{}: done", r.name), GREEN) } else { (format!("{}: failed (exit code {code})", r.name), RED) });
            }
            if r.exit.is_none() {
                ctx.request_repaint_after(Duration::from_millis(250));
            }
            let text = r.text();
            let (status, col) = match r.exit {
                None => ("running…".to_string(), ACCENT_HI),
                Some(0) => ("done".to_string(), GREEN),
                Some(c) => (format!("failed ({c})"), RED),
            };
            let area = egui::Area::new(egui::Id::unique(("script_run", r.id))).anchor(egui::Align2::RIGHT_BOTTOM, Vec2::new(-12.0, -y)).order(egui::Order::Foreground);
            let resp = area.show(ctx, |ui| {
                egui::Frame::new().fill(PANEL).corner_radius(8).stroke(Stroke::new(1.0, BORDER)).inner_margin(10).show(ui, |ui| {
                    ui.set_width(520.0);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(&r.name).family(bold()).color(TEXT));
                        if r.agent.is_none() {
                            ui.label(RichText::new(format!("in {}", project_name(&r.cwd))).size(12.0).color(TEXT_FAINT));
                        }
                        ui.label(RichText::new(status).size(12.0).color(col));
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let tip = if r.exit.is_none() { "Stop it and close" } else { "Close" };
                            if widgets::icon_button(ui, Icon::Close, 22.0, TEXT_DIM, tip).clicked() {
                                close = Some(i);
                            }
                            if r.exit.is_none() {
                                if ui.button("Stop").clicked() {
                                    r.stop();
                                }
                            } else if ui.button("Run again").clicked() {
                                again = Some(i);
                            }
                            if ui.button(if r.collapsed { "Show output" } else { "Hide output" }).clicked() {
                                r.collapsed = !r.collapsed;
                            }
                        });
                    });
                    if !r.collapsed {
                        egui::Frame::new().fill(BG).corner_radius(4).inner_margin(6).show(ui, |ui| {
                            egui::ScrollArea::vertical().id_salt(("script_out", r.id)).max_height(260.0).stick_to_bottom(true).auto_shrink([false, true]).show(ui, |ui| {
                                let t = if text.is_empty() { "(no output yet)" } else { text.as_str() };
                                ui.add(egui::Label::new(RichText::new(t).monospace().size(12.0).color(TEXT_DIM)).wrap().selectable(true));
                            });
                        });
                    }
                    if r.exit.is_none() && r.stdin.is_some() {
                        ui.horizontal(|ui| {
                            let te = ui.add(egui::TextEdit::singleline(&mut r.input).hint_text("Type an answer (Enter sends)").desired_width(ui.available_width() - 60.0));
                            let enter = te.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                            if ui.button("Send").clicked() || enter {
                                r.send_line();
                                te.request_focus();
                            }
                        });
                    }
                });
            });
            y += resp.response.rect.height() + 8.0;
        }
        for (m, c) in done_toasts {
            self.toast(m, c, ctx);
        }
        if redetect {
            self.redetect_agents(ctx);
        }
        if let Some(i) = again {
            let r = self.script_runs.remove(i);
            match ScriptRun::start(ctx, r.name.clone(), r.program.clone(), r.args.clone(), r.cwd.clone(), r.agent) {
                Ok(n) => self.script_runs.insert(i, n),
                Err(e) => self.toast(format!("Couldn't run {}: {e}", r.name), RED, ctx),
            }
        } else if let Some(i) = close {
            self.script_runs.remove(i); // Drop stops it if it's still running.
        }
    }

    /// The scripts palette (Ctrl+Shift+R or the header button): filter, Enter = first match.
    pub(super) fn scripts_palette(&mut self, ctx: &egui::Context) {
        if !self.scripts_open {
            return;
        }
        let mut pick: Option<usize> = None;
        let mut close = false;
        let mut edit = false;
        let project = project_name(&self.current_project());
        let m = egui::Modal::new(egui::Id::unique("scripts")).frame(egui::Frame::new().fill(PANEL).corner_radius(10).stroke(Stroke::new(1.0, BORDER)).inner_margin(16)).show(ctx, |ui| {
            ui.set_width(560.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("SCRIPTS").size(12.0).family(bold()).color(TEXT_DIM));
                ui.label(RichText::new(format!("runs in the background in {project}")).size(12.0).color(TEXT_FAINT));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if widgets::icon_button(ui, Icon::Close, 24.0, TEXT_DIM, "Close (Esc)").clicked() {
                        close = true;
                    }
                    if ui.button("Open folder").on_hover_text(scripts_dir().display().to_string()).clicked() {
                        let _ = std::process::Command::new("explorer.exe").arg(scripts_dir()).spawn();
                    }
                    if ui.button("Refresh").clicked() {
                        self.scripts = scan();
                    }
                    if ui.button("Edit scripts…").clicked() {
                        edit = true;
                    }
                });
            });
            let te = ui.add(egui::TextEdit::singleline(&mut self.script_filter).hint_text("Filter").desired_width(f32::INFINITY));
            if std::mem::take(&mut self.script_focus) {
                te.request_focus();
            }
            ui.add_space(6.0);
            let f = self.script_filter.to_lowercase();
            let shown: Vec<usize> = (0..self.scripts.len()).filter(|&i| f.is_empty() || self.scripts[i].name.to_lowercase().contains(&f) || self.scripts[i].desc.to_lowercase().contains(&f)).collect();
            if te.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                pick = shown.first().copied();
            }
            if self.scripts.is_empty() {
                ui.label(RichText::new("No scripts yet. Put .ps1, .cmd, .bat or .py files in the scripts folder (Open folder), or ask an agent to write one (Edit scripts… > Copy prompt).").color(TEXT_FAINT));
            }
            egui::ScrollArea::vertical().max_height(420.0).show(ui, |ui| {
                for i in shown {
                    let sc = &self.scripts[i];
                    let ext = sc.path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
                    if super::snippets::palette_row(ui, &sc.name, &sc.desc, &ext).clicked() {
                        pick = Some(i);
                    }
                }
            });
        });
        if m.should_close() {
            close = true;
        }
        if let Some(i) = pick {
            let sc = self.scripts[i].clone();
            self.run_script(ctx, &sc);
            close = true;
        }
        if edit {
            self.options = Some(OptPage::Scripts);
            close = true;
        }
        if close {
            self.scripts_open = false;
            self.script_filter.clear();
            self.focus_terminal = true;
        }
    }
}

/// How a script file is run: (program, arguments).
fn script_command(sc: &Script, pwsh: String) -> (String, Vec<String>) {
    let path = sc.path.display().to_string();
    match sc.path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase().as_str() {
        "ps1" => (pwsh, ["-NoLogo", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", &path].map(String::from).to_vec()),
        "py" => ("python.exe".into(), vec!["-u".into(), path]),
        _ => (std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".into()), vec!["/c".into(), path]),
    }
}

/// A script or command running in the background (no console window), stdout and stderr captured.
pub(super) struct ScriptRun {
    id: u64,
    name: String,
    program: String,
    args: Vec<String>,
    cwd: PathBuf,
    /// An install / uninstall of this agent CLI.
    agent: Option<Kind>,
    child: std::process::Child,
    stdin: Option<std::process::ChildStdin>,
    /// Raw output from both pipes, appended by reader threads.
    out: std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
    /// Decoded output and the byte count it was made from.
    text: (usize, String),
    exit: Option<i32>,
    input: String,
    collapsed: bool,
}

/// Output kept per run (older output is dropped).
const MAX_OUT: usize = 512 * 1024;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

impl ScriptRun {
    fn start(ctx: &egui::Context, name: String, program: String, args: Vec<String>, cwd: PathBuf, agent: Option<Kind>) -> std::io::Result<Self> {
        use std::io::Read;
        use std::os::windows::process::CommandExt;
        use std::process::{Command, Stdio};
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let mut child = Command::new(&program)
            .args(&args)
            .current_dir(&cwd)
            .env("PYTHONIOENCODING", "utf-8")
            .env("NO_COLOR", "1")
            .env("AI_TERMINAL_SCRIPT", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()?;
        let out = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let pipes: [Option<Box<dyn Read + Send>>; 2] = [child.stdout.take().map(|p| Box::new(p) as _), child.stderr.take().map(|p| Box::new(p) as _)];
        for mut pipe in pipes.into_iter().flatten() {
            let (out, ctx) = (out.clone(), ctx.clone());
            std::thread::spawn(move || {
                let mut buf = [0u8; 4096];
                while let Ok(n) = pipe.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    let mut o = out.lock().unwrap();
                    o.extend_from_slice(&buf[..n]);
                    if o.len() > MAX_OUT {
                        let cut = o.len() - MAX_OUT;
                        o.drain(..cut);
                    }
                    drop(o);
                    ctx.request_repaint();
                }
            });
        }
        Ok(ScriptRun {
            id: NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            name,
            program,
            args,
            cwd,
            agent,
            stdin: child.stdin.take(),
            child,
            out,
            text: (usize::MAX, String::new()),
            exit: None,
            input: String::new(),
            collapsed: false,
        })
    }

    /// Checks whether it ended; returns the exit code the first time it does.
    fn poll(&mut self) -> Option<i32> {
        if self.exit.is_some() {
            return None;
        }
        let code = self.child.try_wait().ok().flatten()?.code().unwrap_or(-1);
        self.exit = Some(code);
        self.stdin = None;
        Some(code)
    }

    /// The output so far: UTF-8, without colour codes, CRLF as LF.
    fn text(&mut self) -> String {
        let o = self.out.lock().unwrap();
        if self.text.0 != o.len() {
            self.text = (o.len(), clean_output(&String::from_utf8_lossy(&o)));
        }
        self.text.1.clone()
    }

    /// Sends the typed answer as a line on stdin and echoes it in the output.
    fn send_line(&mut self) {
        use std::io::Write;
        let line = std::mem::take(&mut self.input);
        if let Some(w) = &mut self.stdin {
            let _ = w.write_all(format!("{line}\r\n").as_bytes()).and_then(|_| w.flush());
        }
        self.out.lock().unwrap().extend_from_slice(format!("> {line}\n").as_bytes());
    }

    /// Ends the script and everything it started.
    fn stop(&mut self) {
        use std::os::windows::process::CommandExt;
        if self.exit.is_some() || self.child.try_wait().ok().flatten().is_some() {
            return;
        }
        let _ = std::process::Command::new("taskkill.exe").args(["/T", "/F", "/PID", &self.child.id().to_string()]).creation_flags(CREATE_NO_WINDOW).status();
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.exit = Some(-1);
        self.stdin = None;
        self.out.lock().unwrap().extend_from_slice(b"\n[stopped]\n");
    }
}

impl Drop for ScriptRun {
    fn drop(&mut self) {
        // Otherwise it would keep running hidden, and a waiting prompt would read end-of-input.
        self.stop();
    }
}

/// Strips ANSI escape sequences and carriage returns.
fn clean_output(s: &str) -> String {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| regex::Regex::new(r"\x1b(\[[0-?]*[ -/]*[@-~]|\][^\x07\x1b]*(\x07|\x1b\\)|[@-Z\\-_])").unwrap());
    re.replace_all(s, "").replace("\r\n", "\n").replace('\r', "")
}

/// A script being edited in Options > Scripts. Unsaved edits survive closing Options (until the
/// app exits); nothing is written until Save.
pub(super) struct ScriptDraft {
    /// The file on disk (None = new, not saved yet).
    path: Option<PathBuf>,
    name: String,
    ext: String,
    text: String,
    saved_name: String,
    saved_text: String,
    confirm_delete: bool,
    error: Option<String>,
}

impl ScriptDraft {
    fn dirty(&self) -> bool {
        self.path.is_none() || self.name != self.saved_name || self.text != self.saved_text
    }

    fn from_file(s: &Script) -> Self {
        let text = std::fs::read_to_string(&s.path).unwrap_or_default().trim_start_matches('\u{feff}').replace("\r\n", "\n");
        let ext = s.path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
        ScriptDraft { path: Some(s.path.clone()), name: s.name.clone(), ext, saved_name: s.name.clone(), saved_text: text.clone(), text, confirm_delete: false, error: None }
    }

    /// Writes the file, renaming it first when the name changed. Batch files get CRLF.
    fn save(&mut self) -> Result<(), String> {
        self.save_in(&scripts_dir())
    }

    fn save_in(&mut self, dir: &Path) -> Result<(), String> {
        let name = self.name.trim().to_string();
        if name.is_empty() {
            return Err("Give the script a name.".into());
        }
        if name.chars().any(|c| r#"<>:"/\|?*"#.contains(c) || c.is_control()) {
            return Err(r#"The name can't contain < > : " / \ | ? *"#.into());
        }
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let target = dir.join(format!("{name}.{}", self.ext));
        let same = |a: &Path, b: &Path| a.to_string_lossy().eq_ignore_ascii_case(&b.to_string_lossy());
        let moved = self.path.as_deref().is_some_and(|p| p != target);
        // A different file already has that name (a case-only rename is the same file).
        if target.exists() && !self.path.as_deref().is_some_and(|p| same(p, &target)) {
            return Err(format!("{name}.{} already exists.", self.ext));
        }
        if let (Some(old), true) = (&self.path, moved) {
            std::fs::rename(old, &target).map_err(|e| format!("Couldn't rename: {e}"))?;
            self.path = Some(target.clone());
        }
        let body = if matches!(self.ext.as_str(), "cmd" | "bat") { self.text.replace('\n', "\r\n") } else { self.text.clone() };
        std::fs::write(&target, body).map_err(|e| format!("Couldn't save: {e}"))?;
        self.path = Some(target);
        self.name = name.clone();
        self.saved_name = name;
        self.saved_text = self.text.clone();
        Ok(())
    }
}

/// A starter request for an agent to write a script (Options > Scripts > Copy prompt).
fn agent_prompt(dir: &Path) -> String {
    format!(
        "Write a script for AI Terminal's Scripts palette and save it in {} (a .ps1 file unless I ask otherwise; .cmd, .bat and .py also work). \
The file name, without the extension, is the name shown in the palette. Start the file with a one-line comment describing what it does: it's shown as the description. \
It runs hidden in the current project's folder, with its output shown in a small panel that also has a text box for answers, so it can ask questions. Read-Host doesn't show its prompt there: Write-Host the question first, and treat no answer ($null, end of input) as cancel. For git commands, set $env:GIT_PAGER = 'cat'. \
If it needs an LLM, call `claude -p --model haiku --tools '\"\"' --strict-mcp-config --no-session-persistence --system-prompt \"...\"` with only the input it needs, to keep it cheap. \
The script should: ",
        dir.display()
    )
}

/// Starting text for a new script of that type.
fn template(ext: &str) -> String {
    match ext {
        "cmd" | "bat" => "@echo off\nREM What this script does\n".into(),
        "ps1" => "# What this script does\n$env:GIT_PAGER = 'cat'\n".into(),
        _ => "# What this script does\n".into(),
    }
}

impl App {
    /// Reloads the editor list from disk, keeping unsaved edits and new scripts.
    fn sync_script_drafts(&mut self) {
        let mut old = std::mem::take(&mut self.script_drafts);
        let mut v = Vec::new();
        for f in scan() {
            match old.iter().position(|d| d.path.as_ref() == Some(&f.path) && d.dirty()) {
                Some(i) => v.push(old.remove(i)),
                None => v.push(ScriptDraft::from_file(&f)),
            }
        }
        v.extend(old.into_iter().filter(|d| d.path.is_none()));
        self.script_drafts = v;
    }

    pub(super) fn options_scripts(&mut self, ui: &mut Ui) {
        if !self.scripts_synced {
            self.sync_script_drafts();
            self.scripts_synced = true;
        }
        let dir = scripts_dir();
        ui.label(RichText::new("Scripts").family(bold()).size(18.0).color(TEXT));
        ui.label(
            RichText::new(format!(
                "Files in {}. Run one from the Scripts button in the session header or the palette (Ctrl+Shift+R): it runs in the background in the current project, with its output in a small panel at the bottom right (type answers there if it asks something). The first comment line is shown as its description.",
                dir.display()
            ))
            .size(12.5)
            .color(TEXT_DIM),
        );
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Tip: ask an agent to write a script for you. Copy prompt gives you the start of a request with the folder and conventions filled in; add what the script should do.").size(12.5).color(TEXT_DIM));
        });
        ui.horizontal(|ui| {
            if ui.button("Copy prompt").on_hover_text("Copy a starter request to paste into a Claude or Codex session").clicked() {
                ui.ctx().copy_text(agent_prompt(&dir));
                self.toast("Prompt copied: paste it into a session and say what the script should do".into(), GREEN, ui.ctx());
            }
            if ui.button("Open folder").clicked() {
                let _ = std::fs::create_dir_all(&dir);
                let _ = std::process::Command::new("explorer.exe").arg(&dir).spawn();
            }
            if ui.button("Refresh").on_hover_text("Reload from disk (unsaved edits are kept)").clicked() {
                self.sync_script_drafts();
            }
        });
        ui.add_space(8.0);
        let mut remove = None;
        let mut open: Option<PathBuf> = None;
        for (i, d) in self.script_drafts.iter_mut().enumerate() {
            egui::Frame::new().fill(CARD).corner_radius(8).stroke(Stroke::new(1.0, BORDER)).inner_margin(10).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut d.name).hint_text("Name").desired_width(260.0));
                    ui.label(RichText::new(format!(".{}", d.ext)).color(TEXT_FAINT));
                    if d.dirty() {
                        widgets::pill(ui, if d.path.is_none() { "new" } else { "unsaved" }, Color32::WHITE, Color32::from_rgb(150, 80, 20));
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let del = if d.confirm_delete { egui::Button::new(RichText::new("Delete file?").color(Color32::WHITE)).fill(Color32::from_rgb(150, 40, 40)) } else { egui::Button::new("Delete") };
                        if ui.add(del).clicked() {
                            if d.confirm_delete || d.path.is_none() {
                                remove = Some(i);
                            } else {
                                d.confirm_delete = true;
                            }
                        }
                        if d.path.is_some() && ui.button("Open in editor").clicked() {
                            open = d.path.clone();
                        }
                        if d.path.is_some() && d.dirty() && ui.button("Revert").clicked() {
                            d.name = d.saved_name.clone();
                            d.text = d.saved_text.clone();
                            d.error = None;
                        }
                        if ui.add_enabled(d.dirty(), egui::Button::new(RichText::new("Save").color(Color32::WHITE)).fill(ACCENT)).clicked() {
                            d.error = d.save().err();
                            d.confirm_delete = false;
                        }
                    });
                });
                egui::ScrollArea::vertical().id_salt(("script_text", i)).max_height(360.0).show(ui, |ui| {
                    ui.add(egui::TextEdit::multiline(&mut d.text).code_editor().desired_rows(4).desired_width(f32::INFINITY));
                });
                if let Some(e) = &d.error {
                    ui.label(RichText::new(e).color(RED));
                }
            });
            ui.add_space(6.0);
        }
        if let Some(i) = remove {
            let d = self.script_drafts.remove(i);
            if let Some(p) = d.path {
                if let Err(e) = std::fs::remove_file(&p) {
                    self.toast(format!("Couldn't delete {}: {e}", p.display()), RED, ui.ctx());
                }
            }
        }
        if let Some(p) = open {
            if let Err(e) = crate::links::open_file(&p, None, None, &self.s.editor, &self.s.editor_custom) {
                self.toast(format!("Couldn't open the editor: {e}"), RED, ui.ctx());
            }
        }
        ui.horizontal(|ui| {
            if ui.add(egui::Button::new(RichText::new("+ Add script").color(Color32::WHITE)).fill(ACCENT)).clicked() {
                let taken = |n: &str| self.script_drafts.iter().any(|d| d.name.eq_ignore_ascii_case(n));
                let name = (1..).map(|k| if k == 1 { "New script".to_string() } else { format!("New script {k}") }).find(|n| !taken(n)).unwrap();
                let ext = self.script_new_ext.clone();
                self.script_drafts.push(ScriptDraft { path: None, saved_name: String::new(), saved_text: String::new(), text: template(&ext), name, ext, confirm_delete: false, error: None });
            }
            egui::ComboBox::from_id_salt("script_new_ext").selected_text(format!(".{}", self.script_new_ext)).width(70.0).show_ui(ui, |ui| {
                for k in KINDS {
                    ui.selectable_value(&mut self.script_new_ext, k.to_string(), format!(".{k}"));
                }
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::{first_comment, ScriptDraft};

    #[test]
    fn save_rename_and_conflicts() {
        let dir = std::env::temp_dir().join(format!("ai_terminal_scripts_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let new = |name: &str, ext: &str, text: &str| ScriptDraft { path: None, name: name.into(), ext: ext.into(), text: text.into(), saved_name: String::new(), saved_text: String::new(), confirm_delete: false, error: None };
        let mut a = new("One", "cmd", "@echo off\necho 1\n");
        a.save_in(&dir).unwrap();
        assert!(!a.dirty());
        assert_eq!(std::fs::read_to_string(dir.join("One.cmd")).unwrap(), "@echo off\r\necho 1\r\n");
        // Same name as an existing file.
        assert!(new("one", "cmd", "").save_in(&dir).is_err());
        assert!(new("bad:name", "ps1", "").save_in(&dir).is_err());
        // Rename, then a case-only rename.
        a.name = "Two".into();
        a.save_in(&dir).unwrap();
        assert!(!dir.join("One.cmd").exists() && dir.join("Two.cmd").exists());
        a.name = "two".into();
        a.save_in(&dir).unwrap();
        let names: Vec<String> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(names, vec!["two.cmd".to_string()]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn script_descriptions() {
        assert_eq!(first_comment("# Push it\nrest").as_deref(), Some("Push it"));
        assert_eq!(first_comment("\u{feff}@echo off\r\nREM Build all\r\n").as_deref(), Some("Build all"));
        assert_eq!(first_comment(":: Hi").as_deref(), Some("Hi"));
        assert_eq!(first_comment("#!/usr/bin/env python\n# Tool").as_deref(), Some("Tool"));
        assert_eq!(first_comment("git push").as_deref(), None);
    }

    #[test]
    fn runs_in_background_with_input() {
        let dir = std::env::temp_dir().join(format!("ai_terminal_run_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("Ask.ps1");
        std::fs::write(&path, "Write-Host 'Name?'
$a = Read-Host
if ($null -eq $a) { exit 3 }
Write-Host \"hi $a\"
exit 0
").unwrap();
        let sc = super::Script { name: "Ask".into(), path, desc: String::new() };
        let (program, args) = super::script_command(&sc, "powershell.exe".into());
        let mut r = super::ScriptRun::start(&egui::Context::default(), sc.name.clone(), program, args, dir.clone(), None).unwrap();
        r.input = "Bob".into();
        r.send_line();
        let t0 = std::time::Instant::now();
        while r.poll().is_none() && r.exit.is_none() {
            assert!(t0.elapsed().as_secs() < 30);
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
        assert_eq!(r.exit, Some(0));
        assert!(r.text().contains("hi Bob"), "{}", r.text());
        drop(r);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn output_cleanup() {
        assert_eq!(super::clean_output("\x1b[32mok\x1b[0m\r\nnext\r"), "ok\nnext");
    }
}
