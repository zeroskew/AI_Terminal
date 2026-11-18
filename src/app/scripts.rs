//! Scripts: files in the `scripts` folder next to the executable, run from a palette in the
//! current project in the background (no session): output shows in a small panel, which also
//! takes typed answers for scripts that ask questions.

use super::*;
use std::time::Instant;

/// A runnable file in the scripts folder.
#[derive(Clone)]
pub(super) struct Script {
    pub name: String,
    pub path: PathBuf,
    /// The first comment line, else the file type.
    pub desc: String,
    /// It takes its AI model from the app (see `uses_ai`).
    pub ai: bool,
}

/// File types the palette lists.
const KINDS: [&str; 4] = ["ps1", "cmd", "bat", "py"];

/// `<exe folder>\scripts`.
pub(super) fn scripts_dir() -> PathBuf {
    std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join("scripts"))).unwrap_or_else(|| PathBuf::from("scripts"))
}

/// Lists the scripts, sorted by name. Creates the folder the first time.
fn scan() -> Vec<Script> {
    let dir = scripts_dir();
    if !dir.exists() {
        let _ = std::fs::create_dir_all(&dir);
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
            let text = std::fs::read_to_string(&path).unwrap_or_default();
            let desc = first_comment(&text).unwrap_or_else(|| format!(".{ext} script"));
            Some(Script { name, path, desc, ai: uses_ai(&text) })
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
    pub(super) fn run_script(&mut self, ctx: &egui::Context, sc: &Script) {
        let project = self.current_project();
        if !project.is_dir() {
            return self.toast(format!("Folder not found: {}", project.display()), RED, ctx);
        }
        let (program, args) = script_command(sc, self.pwsh());
        let env = self.script_model(&sc.path).map(|m| vec![("AI_TERMINAL_CLI".to_string(), m.cli), ("AI_TERMINAL_MODEL".to_string(), m.model), ("AI_TERMINAL_EFFORT".to_string(), m.effort)]).unwrap_or_default();
        match ScriptRun::start(ctx, sc.name.clone(), program, args, project, None, env) {
            Ok(r) => self.script_runs.push(r),
            Err(e) => self.toast(format!("Couldn't run {}: {e}", sc.name), RED, ctx),
        }
    }

    /// The model picked for a script in Options > Scripts (None = the script's own default).
    fn script_model(&self, path: &Path) -> Option<ScriptModel> {
        self.s.script_models.get(&script_key(path)?).filter(|m| !m.model.is_empty()).cloned()
    }

    /// pwsh if it's installed, else Windows PowerShell.
    pub(super) fn pwsh(&self) -> String {
        self.profiles.iter().find(|p| p.name == "pwsh").map(|p| p.program.clone()).unwrap_or_else(|| "powershell.exe".into())
    }

    /// Runs a program hidden, with its output in a panel at the bottom right. `agent`: an
    /// install / uninstall of that agent CLI (the agent list is re-detected when it ends).
    pub(super) fn run_in_background(&mut self, ctx: &egui::Context, name: String, program: String, args: Vec<String>, cwd: PathBuf, agent: Option<Kind>) {
        match ScriptRun::start(ctx, name.clone(), program, args, cwd, agent, Vec::new()) {
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
        // Ids of panels to close, and the run to start again.
        let mut close: Vec<u64> = Vec::new();
        let mut again = None;
        let mut done_toasts = Vec::new();
        let mut redetect = false;
        let mut y = 34.0;
        let auto_dismiss = self.s.script_auto_dismiss;
        // Panel size: all panels share it; dragging any panel's top / left edge changes it.
        let mut size = self.s.script_panel_size;
        let screen = ctx.content_rect();
        let project_accent: Vec<(PathBuf, Color32)> = self.s.projects.iter().filter_map(|p| Some((p.path.clone(), p.accent_color()?))).collect();
        for r in self.script_runs.iter_mut() {
            let [w, out_h] = size.unwrap_or([520.0, 260.0]);
            let (w, out_h) = (w.clamp(320.0, (screen.width() - 24.0).max(320.0)), out_h.clamp(60.0, (screen.height() - 200.0).max(60.0)));
            if let Some(code) = r.poll() {
                redetect |= r.agent.is_some();
                done_toasts.push(if code == 0 { (format!("{}: done", r.name), GREEN) } else { (format!("{}: failed (exit code {code})", r.name), RED) });
                if code == 0 && auto_dismiss {
                    r.dismiss_from = Some(Instant::now());
                }
            }
            if !auto_dismiss {
                r.dismiss_from = None;
            }
            // Share of the countdown left (1 = just started), then a quick fade-out once it runs out.
            let elapsed = r.dismiss_from.map(|t| t.elapsed().as_secs_f32());
            let left = elapsed.map(|e| (1.0 - e / DISMISS_AFTER.as_secs_f32()).max(0.0));
            let opacity = elapsed.map_or(1.0, |e| 1.0 - (e - DISMISS_AFTER.as_secs_f32()).max(0.0) / FADE_OUT.as_secs_f32());
            if opacity <= 0.0 {
                close.push(r.id);
                continue;
            }
            if r.exit.is_none() {
                ctx.request_repaint_after(Duration::from_millis(250));
            } else if left.is_some() {
                ctx.request_repaint();
            }
            let text = r.text();
            let (status, col) = match r.exit {
                None => ("running…".to_string(), ACCENT_HI),
                Some(0) => ("done".to_string(), GREEN),
                Some(c) => (format!("failed ({c})"), RED),
            };
            let area = egui::Area::new(egui::Id::unique(("script_run", r.id))).anchor(egui::Align2::RIGHT_BOTTOM, Vec2::new(-12.0, -y)).order(egui::Order::Foreground);
            let resp = area.show(ctx, |ui| {
                ui.multiply_opacity(opacity);
                // Accented by its status: a tinted border, a soft glow along the header and a status dot.
                let shadow = egui::epaint::Shadow { offset: [0, 6], blur: 24, spread: 0, color: Color32::from_black_alpha(150) };
                let frame = egui::Frame::new().fill(PANEL).corner_radius(8).stroke(Stroke::new(1.5, col.gamma_multiply(0.6))).shadow(shadow).inner_margin(10).show(ui, |ui| {
                    ui.set_width(w);
                    // Painted under the header once its height is known.
                    let glow = ui.painter().add(egui::Shape::Noop);
                    let header = ui.horizontal(|ui| {
                        let (dot, _) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
                        ui.painter().circle_filled(dot.center(), 4.0, col);
                        if r.exit.is_none() {
                            // A ring pulsing out of the dot while it runs.
                            let t = (ui.input(|i| i.time) as f32 * 1.2).fract();
                            ui.painter().circle_stroke(dot.center(), 4.0 + 4.0 * t, Stroke::new(1.5, col.gamma_multiply(1.0 - t)));
                            ctx.request_repaint();
                        }
                        ui.label(RichText::new(&r.name).family(bold()).color(TEXT));
                        if r.agent.is_none() {
                            let proj = project_accent.iter().find(|(p, _)| p == &r.cwd).map(|(_, c)| *c);
                            ui.label(RichText::new(format!("in {}", project_name(&r.cwd))).size(12.0).color(proj.unwrap_or(TEXT_FAINT)));
                        }
                        ui.label(RichText::new(status).size(12.0).color(col));
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let tip = if r.exit.is_none() { "Stop it and close" } else { "Close" };
                            if widgets::icon_button(ui, Icon::Close, 22.0, TEXT_DIM, tip).clicked() {
                                close.push(r.id);
                            }
                            if r.exit.is_none() {
                                if ui.button("Stop").clicked() {
                                    r.stop();
                                }
                            } else if ui.button("Run again").clicked() {
                                again = Some(r.id);
                            }
                            if ui.button(if r.collapsed { "Show output" } else { "Hide output" }).clicked() {
                                r.collapsed = !r.collapsed;
                            }
                        });
                    });
                    // The header glow: the status color fading out downwards from the top edge.
                    let hr = header.response.rect;
                    // It reaches the border's inner edge (inner margin 10), with top corners rounded
                    // like the border's inner edge (radius 8 minus the 1.5 stroke), so no dark gap shows.
                    // Pushed 0.5 under the stroke so its anti-aliased edge leaves no seam.
                    let (edge, radius) = (10.5, 7.0);
                    let band = egui::Rect::from_min_max(egui::pos2(hr.left() - edge, hr.top() - edge), egui::pos2(hr.right() + edge, hr.bottom() + 6.0));
                    let mut mesh = egui::Mesh::default();
                    let shade = |y: f32| col.gamma_multiply(0.16 * (band.bottom() - y) / band.height());
                    let mut points = vec![egui::pos2(band.left(), band.bottom()), egui::pos2(band.right(), band.bottom())];
                    let steps = 8;
                    for (cx, from) in [(band.right() - radius, 0.0f32), (band.left() + radius, 90.0)] {
                        for i in 0..=steps {
                            let a = (from + 90.0 * i as f32 / steps as f32).to_radians();
                            points.push(egui::pos2(cx + radius * a.cos(), band.top() + radius - radius * a.sin()));
                        }
                    }
                    for p in &points {
                        mesh.colored_vertex(*p, shade(p.y));
                    }
                    // Convex, so a fan from the first point covers it.
                    for i in 1..points.len() as u32 - 1 {
                        mesh.add_triangle(0, i, i + 1);
                    }
                    ui.painter().set(glow, mesh);
                    if !r.collapsed {
                        egui::Frame::new().fill(BG).corner_radius(4).inner_margin(6).show(ui, |ui| {
                            // Once resized, the output keeps that height; by default it fits short output.
                            egui::ScrollArea::vertical().id_salt(("script_out", r.id)).max_height(out_h).stick_to_bottom(true).auto_shrink([false, size.is_none()]).show_hinted(ui, |ui| {
                                let t = if text.is_empty() { "(no output yet)" } else { text.as_str() };
                                ui.add(egui::Label::new(RichText::new(t).monospace().size(12.0).color(TEXT_DIM)).wrap().selectable(true));
                            });
                        });
                    }
                    if r.exit.is_none() && r.stdin.is_some() {
                        ui.horizontal(|ui| {
                            let te = ui.add(egui::TextEdit::singleline(&mut r.input).hint_text("Type an answer (Enter sends)").desired_width(ui.available_width() - 60.0));
                            // Keep asking until it has the keyboard: the request is dropped while
                            // the Scripts palette (a modal) is still closing.
                            if !r.input_focused {
                                if te.has_focus() {
                                    r.input_focused = true;
                                } else {
                                    te.request_focus();
                                }
                            }
                            let enter = te.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                            if ui.button("Send").clicked() || enter {
                                r.send_line();
                                te.request_focus();
                            }
                        });
                    }
                    if let Some(l) = left {
                        // Auto-dismiss countdown: a bar that empties to the right.
                        ui.add_space(4.0);
                        let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 3.0), Sense::hover());
                        ui.painter().rect_filled(rect, 1.5, BORDER);
                        let mut fill = rect;
                        fill.set_width(rect.width() * l.clamp(0.0, 1.0));
                        ui.painter().rect_filled(fill, 1.5, GREEN);
                    }
                });
                // Resize handles: left edge, top edge, top-left corner (added last so it wins).
                // Double-click one for the default size.
                let fr = frame.response.rect;
                let handles = [
                    ("l", egui::Rect::from_min_max(egui::pos2(fr.left() - 3.0, fr.top() + 10.0), egui::pos2(fr.left() + 4.0, fr.bottom())), egui::CursorIcon::ResizeHorizontal, (true, false)),
                    ("t", egui::Rect::from_min_max(egui::pos2(fr.left() + 10.0, fr.top() - 3.0), egui::pos2(fr.right(), fr.top() + 4.0)), egui::CursorIcon::ResizeVertical, (false, true)),
                    ("c", egui::Rect::from_min_size(fr.min - Vec2::splat(3.0), Vec2::splat(13.0)), egui::CursorIcon::ResizeNorthWest, (true, true)),
                ];
                for (key, rect, cursor, (dx, dy)) in handles {
                    let h = ui.interact(rect, egui::Id::unique(("script_resize", r.id, key)), Sense::click_and_drag());
                    if h.hovered() || h.dragged() {
                        ui.ctx().set_cursor_icon(cursor);
                    }
                    if h.double_clicked() {
                        size = None;
                    } else if h.dragged() {
                        let d = h.drag_delta();
                        size = Some([w - if dx { d.x } else { 0.0 }, out_h - if dy { d.y } else { 0.0 }]);
                    }
                }
            });
            // A click in the panel keeps it open (cancels the countdown).
            if r.dismiss_from.is_some() && resp.response.contains_pointer() && ctx.input(|i| i.pointer.any_pressed()) {
                r.dismiss_from = None;
            }
            y += resp.response.rect.height() + 8.0;
        }
        self.s.script_panel_size = size;
        for (m, c) in done_toasts {
            self.toast(m, c, ctx);
        }
        if redetect {
            self.redetect_agents(ctx);
        }
        if let Some(i) = again.and_then(|id| self.script_runs.iter().position(|r| r.id == id)) {
            let r = &self.script_runs[i];
            match ScriptRun::start(ctx, r.name.clone(), r.program.clone(), r.args.clone(), r.cwd.clone(), r.agent, r.env.clone()) {
                Ok(n) => self.script_runs[i] = n,
                Err(e) => {
                    let msg = format!("Couldn't run {}: {e}", r.name);
                    self.toast(msg, RED, ctx);
                }
            }
        }
        // Drop stops a run that's still going.
        self.script_runs.retain(|r| !close.contains(&r.id));
    }

    /// The current project's starred scripts that exist in the scripts folder (reads it once if
    /// the palette hasn't yet).
    pub(super) fn fav_script_list(&mut self) -> Vec<usize> {
        if self.fav_list(super::favorites::FavList::Scripts).is_empty() && self.s.pinned_scripts.is_empty() {
            return Vec::new();
        }
        if !self.scripts_scanned && self.scripts.is_empty() {
            self.scripts = scan();
        }
        self.scripts_scanned = true;
        (0..self.scripts.len()).filter(|&i| script_key(&self.scripts[i].path).is_some_and(|k| self.is_fav(super::favorites::FavList::Scripts, &k))).collect()
    }

    /// The scripts palette (Ctrl+Shift+R or the header button): filter, Enter = first match.
    pub(super) fn scripts_palette(&mut self, ctx: &egui::Context) {
        if !self.scripts_open {
            return;
        }
        let mut pick: Option<usize> = None;
        let mut close = false;
        let mut edit = false;
        let mut star: Option<String> = None;
        let project = project_name(&self.current_project());
        let catalog = self.catalog.lock().unwrap().clone().unwrap_or_default();
        let m = egui::Modal::new(egui::Id::unique("scripts")).frame(egui::Frame::new().fill(PANEL).corner_radius(10).stroke(Stroke::new(1.0, BORDER)).inner_margin(16)).show(ctx, |ui| {
            ui.set_width(560.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("SCRIPTS").size(12.0).family(bold()).color(TEXT_DIM));
                ui.label(RichText::new(format!("runs in the background in {project}")).size(12.0).color(TEXT_FAINT));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if widgets::corner_close_button(ui, 16.0).clicked() {
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
            super::snippets::palette_scroll(ui, shown.len()).show_hinted(ui, |ui| {
                for i in shown {
                    let sc = &self.scripts[i];
                    // The AI model it will use, else the file type.
                    let note = if sc.ai { model_note(&catalog, self.script_model(&sc.path).as_ref()) } else { sc.path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase() };
                    let mut row = super::snippets::palette_row(ui, &sc.name, &sc.desc, &note, 28.0);
                    if sc.ai {
                        row = row.on_hover_text("Uses this AI model. Change it in Edit scripts…");
                    }
                    let key = script_key(&sc.path).unwrap_or_default();
                    let fav = self.is_fav(super::favorites::FavList::Scripts, &key);
                    let sr = egui::Rect::from_center_size(egui::pos2(row.rect.right() - 20.0, row.rect.center().y), Vec2::splat(26.0));
                    if super::snippets::star_button(ui, sr, egui::Id::unique(("script_star", i)), fav).clicked() {
                        star = Some(key);
                    } else if row.clicked() {
                        pick = Some(i);
                    }
                }
            });
        });
        if m.should_close() {
            close = true;
        }
        if let Some(key) = star {
            self.toggle_fav(super::favorites::FavList::Scripts, &key);
        }
        // A started script's answer box takes the keyboard, so the terminal doesn't.
        let mut started = false;
        if let Some(i) = pick {
            let sc = self.scripts[i].clone();
            let n = self.script_runs.len();
            self.run_script(ctx, &sc);
            started = self.script_runs.len() > n;
            close = true;
        }
        if edit {
            self.options = Some(OptPage::Scripts);
            close = true;
        }
        if close {
            self.scripts_open = false;
            self.script_filter.clear();
            self.focus_terminal = !started;
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

/// The settings key for a script's model: its lowercase file name.
pub(super) fn script_key(path: &Path) -> Option<String> {
    Some(path.file_name()?.to_string_lossy().to_lowercase())
}

/// Whether a script takes its AI model from the app (reads AI_TERMINAL_MODEL): only those get a
/// model picker, since a script that names its own model would ignore it.
fn uses_ai(text: &str) -> bool {
    text.to_lowercase().contains("ai_terminal_model")
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
    /// Extra environment variables (the script's AI model).
    env: Vec<(String, String)>,
    child: std::process::Child,
    stdin: Option<std::process::ChildStdin>,
    /// Raw output from both pipes, appended by reader threads.
    out: std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
    /// Decoded output and the byte count it was made from.
    text: (usize, String),
    exit: Option<i32>,
    input: String,
    /// The answer box has had the keyboard once (it takes it when it first appears).
    input_focused: bool,
    collapsed: bool,
    /// When the auto-dismiss countdown started (it finished successfully; a click in the panel cancels it).
    dismiss_from: Option<Instant>,
}

/// How long a finished panel stays before it closes itself.
const DISMISS_AFTER: Duration = Duration::from_secs(10);
/// How long the auto-dismissed panel takes to fade out.
const FADE_OUT: Duration = Duration::from_millis(150);

/// Output kept per run (older output is dropped).
const MAX_OUT: usize = 512 * 1024;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

impl ScriptRun {
    fn start(ctx: &egui::Context, name: String, program: String, args: Vec<String>, cwd: PathBuf, agent: Option<Kind>, env: Vec<(String, String)>) -> std::io::Result<Self> {
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
            .envs(env.iter().map(|(k, v)| (k, v)))
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
            env,
            stdin: child.stdin.take(),
            child,
            out,
            text: (usize::MAX, String::new()),
            exit: None,
            input: String::new(),
            input_focused: false,
            collapsed: false,
            dismiss_from: None,
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

    /// The output so far: UTF-8, without color codes, CRLF as LF.
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
    /// The source box is shown (collapsed by default so a long list stays short).
    expanded: bool,
}

impl ScriptDraft {
    fn dirty(&self) -> bool {
        self.path.is_none() || self.name != self.saved_name || self.text != self.saved_text
    }

    fn from_file(s: &Script) -> Self {
        let text = std::fs::read_to_string(&s.path).unwrap_or_default().trim_start_matches('\u{feff}').replace("\r\n", "\n");
        let ext = s.path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
        ScriptDraft { path: Some(s.path.clone()), name: s.name.clone(), ext, saved_name: s.name.clone(), saved_text: text.clone(), text, confirm_delete: false, error: None, expanded: false }
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

/// The CLIs a script's model can come from: settings value, menu name.
const SCRIPT_CLIS: [(&str, &str); 3] = [("claude", "Claude"), ("codex", "Codex"), ("antigravity", "Antigravity")];

fn cli_name(cli: &str) -> &'static str {
    SCRIPT_CLIS.iter().find(|c| c.0 == cli).map_or("Claude", |c| c.1)
}

fn cli_models<'a>(catalog: &'a agents::Catalog, cli: &str) -> &'a [agents::ModelInfo] {
    match cli {
        "codex" => &catalog.codex_models,
        "antigravity" => &catalog.agy_models,
        _ => &catalog.claude_models,
    }
}

/// The palette's note for a script that takes its model from the app.
fn model_note(catalog: &agents::Catalog, m: Option<&ScriptModel>) -> String {
    match m {
        None => "script's default model".into(),
        Some(m) => {
            let label = cli_models(catalog, &m.cli).iter().find(|x| x.id == m.model).map_or(m.model.as_str(), |x| x.label.as_str());
            let effort = if m.effort.is_empty() { String::new() } else { format!(" · {}", m.effort) };
            format!("{} · {label}{effort}", cli_name(&m.cli))
        }
    }
}

/// The agent kind behind a script CLI value.
fn cli_kind(cli: &str) -> Kind {
    match cli {
        "codex" => Kind::Codex,
        "antigravity" => Kind::Antigravity,
        _ => Kind::Claude,
    }
}

/// The effort levels a script's model offers (none for Antigravity).
fn cli_efforts(catalog: &agents::Catalog, m: &ScriptModel) -> Vec<String> {
    catalog.efforts(cli_kind(&m.cli), Some(&m.model))
}

/// The effort dropdown next to a script's model (only for a CLI with effort levels).
fn effort_picker(ui: &mut Ui, i: usize, catalog: &agents::Catalog, models: &mut std::collections::HashMap<String, ScriptModel>, key: &str) {
    let Some(m) = models.get_mut(key) else { return };
    let levels = cli_efforts(catalog, m);
    if levels.is_empty() {
        return;
    }
    let default = match catalog.default_effort(cli_kind(&m.cli), Some(&m.model)) {
        Some(d) => format!("default ({d})"),
        None => "default".to_string(),
    };
    let shown = if m.effort.is_empty() { format!("Effort: {default}") } else { format!("Effort: {}", m.effort) };
    egui::ComboBox::from_id_salt(("script_effort", i)).selected_text(shown).width(130.0).show_ui(ui, |ui| {
        ui.selectable_value(&mut m.effort, String::new(), default).on_hover_text("The model's default effort");
        for e in &levels {
            ui.selectable_value(&mut m.effort, e.clone(), e);
        }
    });
}

/// The AI model dropdown on a script's card: the script's own default, a model from one of the
/// CLIs' lists, or a typed model id.
fn model_picker(ui: &mut Ui, i: usize, catalog: &agents::Catalog, models: &mut std::collections::HashMap<String, ScriptModel>, key: &str) {
    let cur = models.get(key).cloned();
    let list = |cli: &str| cli_models(catalog, cli);
    let known = cur.as_ref().and_then(|m| list(&m.cli).iter().find(|x| x.id == m.model));
    let shown = match (&cur, known) {
        (None, _) => "AI: script default".to_string(),
        (Some(m), Some(x)) => format!("{}: {}", cli_name(&m.cli), x.label),
        (Some(m), None) => format!("{}: other", cli_name(&m.cli)),
    };
    let mut pick: Option<Option<ScriptModel>> = None;
    let tall = ui.ctx().content_rect().height() * 0.7;
    egui::ComboBox::from_id_salt(("script_model", i)).selected_text(shown).width(240.0).height(tall).show_ui(ui, |ui| {
        if ui.selectable_label(cur.is_none(), "Script default").on_hover_text("Whatever model the script itself names").clicked() {
            pick = Some(None);
        }
        for (cli, name) in SCRIPT_CLIS {
            ui.separator();
            ui.label(RichText::new(name).size(11.5).color(TEXT_FAINT));
            if list(cli).is_empty() {
                ui.label(RichText::new("No model list (is it installed?)").size(11.5).color(TEXT_FAINT));
            }
            for x in list(cli) {
                let on = cur.as_ref().is_some_and(|m| m.cli == cli && m.model == x.id);
                if ui.selectable_label(on, &x.label).on_hover_text(&x.id).clicked() {
                    pick = Some(Some(ScriptModel { cli: cli.into(), model: x.id.clone(), effort: String::new() }));
                }
            }
            let other = cur.as_ref().is_some_and(|m| m.cli == cli) && known.is_none();
            if ui.selectable_label(other, "Other model id…").clicked() {
                pick = Some(Some(ScriptModel { cli: cli.into(), model: String::new(), effort: String::new() }));
            }
        }
    });
    match pick {
        Some(None) => {
            models.remove(key);
        }
        Some(Some(mut m)) => {
            // Keep the effort across model changes when the new model offers it.
            if let Some(old) = models.get(key).filter(|o| o.cli == m.cli) {
                m.effort = old.effort.clone();
            }
            if !m.effort.is_empty() && !cli_efforts(catalog, &m).contains(&m.effort) {
                m.effort.clear();
            }
            models.insert(key.to_string(), m);
        }
        None => {}
    }
    // A typed model id (not in the lists), e.g. claude-haiku-5-5.
    if let Some(m) = models.get_mut(key).filter(|m| !list(&m.cli).iter().any(|x| x.id == m.model)) {
        let hint = if m.cli == "claude" { "e.g. claude-haiku-5-5" } else { "model id" };
        ui.add(egui::TextEdit::singleline(&mut m.model).hint_text(hint).desired_width(170.0));
    }
    effort_picker(ui, i, catalog, models, key);
}

/// A starter request for an agent to write a script (Options > Scripts > Copy prompt).
fn agent_prompt(dir: &Path) -> String {
    format!(
        "Write a script for AI Terminal's Scripts palette and save it in {} (a .ps1 file unless I ask otherwise; .cmd, .bat and .py also work). \
The file name, without the extension, is the name shown in the palette. Start the file with a one-line comment describing what it does: it's shown as the description. \
It runs hidden in the current project's folder, with its output shown in a small panel that also has a text box for answers, so it can ask questions. Read-Host doesn't show its prompt there: Write-Host the question first, and treat no answer ($null, end of input) as cancel. For git commands, set $env:GIT_PAGER = 'cat'. \
If it needs an LLM, call `claude -p --model haiku --tools '\"\"' --strict-mcp-config --no-session-persistence --system-prompt \"...\"` with only the input it needs, to keep it cheap. The model can be picked per script in the app: when set, it's in $env:AI_TERMINAL_CLI ('claude', 'codex' or 'antigravity') and $env:AI_TERMINAL_MODEL; use them when present. For codex: `codex exec -m <model> --skip-git-repo-check --sandbox read-only --output-last-message <file> -` with the prompt on stdin, then read the file. For antigravity: `agy --model <model> --disable-slash-commands --input-format stream-json --output-format stream-json -p \"\"` with one line on stdin, `{{\"event\":\"user\",\"message\":{{\"content\":\"<prompt>\"}}}}` (build it with ConvertTo-Json -Compress); the answer is in result.response of the output line whose event is \"result\". \
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
                None => {
                    let expanded = old.iter().any(|d| d.path.as_ref() == Some(&f.path) && d.expanded);
                    v.push(ScriptDraft { expanded, ..ScriptDraft::from_file(&f) });
                }
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
        ui.checkbox(&mut self.s.script_auto_dismiss, format!("Close the output panel {} s after a script finishes successfully (click it to keep it open)", DISMISS_AFTER.as_secs()));
        ui.label(RichText::new("Resize the panel by dragging its top or left edge; double-click the edge for the default size.").size(12.0).color(TEXT_FAINT));
        ui.label(RichText::new("Scripts that read $env:AI_TERMINAL_MODEL (and AI_TERMINAL_CLI) get a model picker: the Claude, Codex or Antigravity model you pick there is passed to them.").size(12.0).color(TEXT_FAINT));
        ui.add_space(8.0);
        let mut remove = None;
        let mut open: Option<PathBuf> = None;
        let catalog = self.catalog.lock().unwrap().clone().unwrap_or_default();
        let models = &mut self.s.script_models;
        for (i, d) in self.script_drafts.iter_mut().enumerate() {
            egui::Frame::new().fill(CARD).corner_radius(8).stroke(Stroke::new(1.0, BORDER)).inner_margin(10).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut d.name).hint_text("Name").desired_width(260.0));
                    ui.label(RichText::new(format!(".{}", d.ext)).color(TEXT_FAINT));
                    let label = if d.expanded { "Hide script" } else { "Show script" };
                    if ui.button(label).clicked() {
                        d.expanded = !d.expanded;
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
                            let old = d.path.as_deref().and_then(script_key);
                            d.error = d.save().err();
                            // A rename keeps the script's model.
                            if let (Some(old), Some(new)) = (old, d.path.as_deref().and_then(script_key)) {
                                if old != new {
                                    if let Some(m) = models.remove(&old) {
                                        models.insert(new, m);
                                    }
                                }
                            }
                            d.confirm_delete = false;
                        }
                        // In the right-to-left run (left of Save) so the buttons can't paint over it.
                        if d.dirty() {
                            widgets::pill(ui, if d.path.is_none() { "new" } else { "unsaved" }, Color32::WHITE, Color32::from_rgb(150, 80, 20));
                        }
                    });
                });
                // Second line: the AI model (scripts that call one) and, collapsed, the description.
                let ai_key = d.path.as_deref().and_then(script_key).filter(|_| uses_ai(&d.saved_text));
                if ai_key.is_some() || !d.expanded {
                    ui.horizontal(|ui| {
                        if let Some(key) = &ai_key {
                            model_picker(ui, i, &catalog, models, key);
                        }
                        if !d.expanded {
                            let desc = first_comment(&d.text).unwrap_or_else(|| format!("{} lines", d.text.lines().count()));
                            ui.add(egui::Label::new(RichText::new(desc).size(12.5).color(TEXT_DIM)).truncate());
                        }
                    });
                }
                if d.expanded {
                    // Fixed height from the line count (up to 360): inside the page's scroll area a
                    // ScrollArea would otherwise shrink to whatever is left of the visible window.
                    let row = ui.text_style_height(&egui::TextStyle::Monospace);
                    let h = ((d.text.lines().count().max(4) + 1) as f32 * row + 8.0).min(360.0);
                    egui::ScrollArea::vertical().id_salt(("script_text", i)).min_scrolled_height(h).max_height(h).show_hinted(ui, |ui| {
                        ui.add(egui::TextEdit::multiline(&mut d.text).code_editor().desired_rows(4).desired_width(f32::INFINITY));
                    });
                }
                if let Some(e) = &d.error {
                    ui.label(RichText::new(e).color(RED));
                }
            });
            ui.add_space(6.0);
        }
        if let Some(i) = remove {
            let d = self.script_drafts.remove(i);
            if let Some(key) = d.path.as_deref().and_then(script_key) {
                self.s.script_models.remove(&key);
            }
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
                self.script_drafts.push(ScriptDraft { path: None, saved_name: String::new(), saved_text: String::new(), text: template(&ext), name, ext, confirm_delete: false, error: None, expanded: true });
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
        let new = |name: &str, ext: &str, text: &str| ScriptDraft { path: None, name: name.into(), ext: ext.into(), text: text.into(), saved_name: String::new(), saved_text: String::new(), confirm_delete: false, error: None, expanded: false };
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
        let sc = super::Script { name: "Ask".into(), path, desc: String::new(), ai: false };
        let (program, args) = super::script_command(&sc, "powershell.exe".into());
        let mut r = super::ScriptRun::start(&egui::Context::default(), sc.name.clone(), program, args, dir.clone(), None, Vec::new()).unwrap();
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
