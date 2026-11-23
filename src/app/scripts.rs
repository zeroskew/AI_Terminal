//! Scripts: files in the project's `scripts` folder (that project only) and in `scripts_common`
//! next to the executable (every project, unless switched off), run from a palette in the
//! current project in the background (no session): output shows in a small panel, which also
//! takes typed answers for scripts that ask questions.

use super::*;
use std::time::Instant;

/// A runnable file in a scripts folder.
#[derive(Clone)]
pub(super) struct Script {
    pub name: String,
    pub path: PathBuf,
    /// The first comment line, else the file type.
    pub desc: String,
    /// It takes its AI model from the app (see `uses_ai`).
    pub ai: bool,
    /// From `scripts_common` (shared by all projects), else from the project's `scripts` folder.
    pub common: bool,
}

impl Script {
    /// The key for favourites and project limits (see `key_for`).
    pub fn key(&self) -> String {
        key_for(&self.path, self.common)
    }

    /// The key in `script_models` (see `model_key_for`).
    fn model_key(&self) -> String {
        model_key_for(&self.path, self.common)
    }
}

/// Favourites / project-limit key: the lowercase file name, with "common/" in front for a
/// common script (a project's own lists only ever see that project's scripts).
fn key_for(path: &Path, common: bool) -> String {
    let file = path.file_name().map(|f| f.to_string_lossy().to_lowercase()).unwrap_or_default();
    if common { format!("common/{file}") } else { file }
}

/// `script_models` key: "common/<file>", or "<project folder>|<file>" (lowercase) for a project
/// script, since one file name can be in several projects.
fn model_key_for(path: &Path, common: bool) -> String {
    if common {
        return key_for(path, true);
    }
    let project = path.parent().and_then(|d| d.parent()).map(|p| p.display().to_string().to_lowercase()).unwrap_or_default();
    format!("{project}|{}", key_for(path, false))
}

/// File types the palette lists.
const KINDS: [&str; 4] = ["ps1", "cmd", "bat", "py"];

/// `<exe folder>\scripts_common`: scripts listed in every project.
pub(super) fn common_dir() -> PathBuf {
    std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join("scripts_common"))).unwrap_or_else(|| PathBuf::from("scripts_common"))
}

/// `<project>\scripts`: that project's own scripts.
pub(super) fn project_dir(project: &Path) -> PathBuf {
    project.join("scripts")
}

/// The project's scripts, then the common ones, each sorted by name. Creates the common folder
/// the first time (not the project's: that waits for its first script).
fn scan(project: &Path) -> Vec<Script> {
    let common = common_dir();
    if !common.exists() {
        let _ = std::fs::create_dir_all(&common);
    }
    let mut v = scan_dir(&project_dir(project), false);
    v.extend(scan_dir(&common, true));
    v
}

fn scan_dir(dir: &Path, common: bool) -> Vec<Script> {
    let mut v: Vec<Script> = std::fs::read_dir(dir)
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
            Some(Script { name, path, desc, ai: uses_ai(&text), common })
        })
        .collect();
    v.sort_by_key(|s| s.name.to_lowercase());
    v
}

/// Moves script settings to the per-project folders (settings version 11). They used to be keyed
/// by file name for the one folder next to the exe; now a file name is either a project's script
/// (found in `<project>\scripts`) or a common one (found in `scripts_common`).
pub(super) fn migrate_settings(s: &mut Settings) {
    let common = common_dir();
    let old = |k: &str| !k.starts_with("common/") && !k.contains('|');
    let in_common = |k: &str| common.join(k).is_file();
    let models = std::mem::take(&mut s.script_models);
    for (k, m) in models {
        if !old(&k) {
            s.script_models.insert(k, m);
            continue;
        }
        if in_common(&k) {
            s.script_models.insert(format!("common/{k}"), m.clone());
        }
        for p in &s.projects {
            let f = project_dir(&p.path).join(&k);
            if f.is_file() {
                s.script_models.insert(model_key_for(&f, false), m.clone());
            }
        }
    }
    // Project limits only apply to common scripts now (a project's scripts are its own).
    let scopes = std::mem::take(&mut s.script_projects);
    s.script_projects = scopes.into_iter().filter_map(|(k, v)| if !old(&k) { Some((k, v)) } else { in_common(&k).then(|| (format!("common/{k}"), v)) }).collect();
    // Favourites: a name that's now only in the common folder becomes its common key.
    let to_common = |k: &mut String, project: Option<&Path>| {
        if old(k) && !project.is_some_and(|p| project_dir(p).join(&*k).is_file()) && in_common(k) {
            *k = format!("common/{k}");
        }
    };
    for k in s.pinned_scripts.iter_mut() {
        to_common(k, None);
    }
    for p in s.projects.iter_mut() {
        let path = p.path.clone();
        for k in p.fav_scripts.iter_mut() {
            to_common(k, Some(&path));
        }
        for t in p.fav_order.iter_mut() {
            if let Some(k) = t.strip_prefix("script:") {
                let mut k = k.to_string();
                to_common(&mut k, Some(&path));
                *t = format!("script:{k}");
            }
        }
    }
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
        self.refresh_scripts();
        self.scripts_open = true;
        self.script_focus = true;
    }

    /// Reads the current project's scripts and the common ones.
    pub(super) fn refresh_scripts(&mut self) {
        let cur = self.current_project();
        self.scripts = scan(&cur);
        self.scripts_for = Some(cur);
    }

    /// Whether the project lists the common scripts (not switched off for it).
    pub(super) fn common_scripts_on(&self, project: &Path) -> bool {
        !self.s.common_scripts_off.iter().any(|p| p == project)
    }

    /// Switches the common scripts on or off for a project.
    pub(super) fn set_common_scripts(&mut self, project: &Path, on: bool) {
        self.s.common_scripts_off.retain(|p| p != project);
        if !on {
            self.s.common_scripts_off.push(project.to_path_buf());
        }
    }

    /// Test hook (--run-script <name>): runs the script with that name in the current project.
    pub(super) fn run_script_named(&mut self, ctx: &egui::Context, name: &str) {
        if let Some(sc) = scan(&self.current_project()).into_iter().find(|s| s.name.eq_ignore_ascii_case(name)) {
            self.run_script(ctx, &sc);
        }
    }

    /// Starts a script in the background in the current project; its panel shows the output.
    pub(super) fn run_script(&mut self, ctx: &egui::Context, sc: &Script) {
        let project = self.current_project();
        if !project.is_dir() {
            return self.toast(format!("Folder not found: {}", project.display()), RED, ctx);
        }
        if !sc.path.is_file() {
            // Renamed or deleted outside the app: reload the list so it stops showing.
            self.refresh_scripts();
            return self.toast(format!("Script not found: {}", sc.path.display()), RED, ctx);
        }
        let (program, args) = script_command(sc, self.pwsh());
        let env = self.script_model(sc).map(|m| vec![("AI_TERMINAL_CLI".to_string(), m.cli), ("AI_TERMINAL_MODEL".to_string(), m.model), ("AI_TERMINAL_EFFORT".to_string(), m.effort)]).unwrap_or_default();
        match ScriptRun::start(ctx, sc.name.clone(), program, args, project, None, env) {
            Ok(mut r) => {
                r.minimized = self.s.script_start_minimized;
                self.script_runs.push(r);
            }
            Err(e) => self.toast(format!("Couldn't run {}: {e}", sc.name), RED, ctx),
        }
    }

    /// Whether a script is listed in the current project: its own scripts always, a common one
    /// unless common scripts are off there or it's limited to other projects.
    pub(super) fn script_in_project(&self, sc: &Script) -> bool {
        if !sc.common {
            return true;
        }
        let cur = self.current_project();
        self.common_scripts_on(&cur) && self.s.script_projects.get(&sc.key()).is_none_or(|ps| ps.is_empty() || ps.contains(&cur))
    }

    /// The model picked for a script in Options > Scripts (None = the script's own default).
    fn script_model(&self, sc: &Script) -> Option<ScriptModel> {
        self.s.script_models.get(&sc.model_key()).filter(|m| !m.model.is_empty()).cloned()
    }

    /// pwsh if it's installed, else Windows PowerShell.
    pub(super) fn pwsh(&self) -> String {
        self.profiles.iter().find(|p| p.name == "pwsh").map(|p| p.program.clone()).unwrap_or_else(|| "powershell.exe".into())
    }

    /// Runs a program hidden, with its output in a panel at the bottom right. `agent`: an
    /// install / uninstall of that agent CLI (the agent list is re-detected when it ends).
    pub(super) fn run_in_background(&mut self, ctx: &egui::Context, name: String, program: String, args: Vec<String>, cwd: PathBuf, agent: Option<Kind>) {
        match ScriptRun::start(ctx, name.clone(), program, args, cwd, agent, Vec::new()) {
            Ok(mut r) => {
                r.minimized = self.s.script_start_minimized;
                self.script_runs.push(r);
            }
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
        let dismiss_after = self.s.script_dismiss_secs.clamp(1, 10) as f32;
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
            let left = elapsed.map(|e| (1.0 - e / dismiss_after).max(0.0));
            let opacity = elapsed.map_or(1.0, |e| 1.0 - (e - dismiss_after).max(0.0) / FADE_OUT.as_secs_f32());
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
            // Minimized and it seems to be asking something (the output ends with a question and
            // has gone quiet): show the panel so the answer box can be seen. Once per question.
            if r.minimized && r.exit.is_none() && r.stdin.is_some() && r.text.0 != r.asked_at && r.out_at.elapsed() >= ASK_IDLE && asks_question(&text) {
                r.minimized = false;
                r.asked_at = r.text.0;
            }
            if r.minimized {
                // Minimized: a pill with the status and a progress bar; a click restores the panel.
                let pct = if r.exit.is_none() { last_percent(&text) } else { None };
                let resp = egui::Area::new(egui::Id::unique(("script_pill", r.id))).anchor(egui::Align2::RIGHT_BOTTOM, Vec2::new(-12.0, -y)).order(egui::Order::Foreground).show(ctx, |ui| {
                    ui.multiply_opacity(opacity);
                    let hover = ui.response().contains_pointer();
                    // Finished: an [x] at the right end closes it without opening the panel.
                    let mut close_slot = None;
                    let shadow = egui::epaint::Shadow { offset: [0, 4], blur: 16, spread: 0, color: Color32::from_black_alpha(140) };
                    let stroke = Stroke::new(1.5, col.gamma_multiply(if hover { 0.9 } else { 0.6 }));
                    let frame = egui::Frame::new().fill(if hover { CARD_HI } else { PANEL }).corner_radius(18).stroke(stroke).shadow(shadow).inner_margin(egui::Margin { left: 12, right: 14, top: 7, bottom: 7 }).show(ui, |ui| {
                        // A fixed compact width: the name truncates to leave room for the status and timer.
                        ui.set_width(PILL_W);
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 6.0;
                            let (dot, _) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
                            ui.painter().circle_filled(dot.center(), 4.0, col);
                            ui.scope(|ui| {
                                let reserve = if r.exit.is_some() { 125.0 + PILL_CLOSE + 6.0 } else { 125.0 };
                                ui.set_max_width((ui.available_width() - reserve).max(40.0));
                                ui.add(egui::Label::new(RichText::new(&r.name).family(bold()).color(TEXT)).truncate().selectable(false));
                            });
                            let st = match (pct, r.exit) {
                                (Some(p), None) => format!("{p:.0}%"),
                                _ => status.clone(),
                            };
                            ui.add(egui::Label::new(RichText::new(st).size(12.0).color(col)).selectable(false));
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if r.exit.is_some() {
                                    close_slot = Some(ui.allocate_exact_size(Vec2::splat(PILL_CLOSE), Sense::hover()).0);
                                }
                                ui.add(egui::Label::new(RichText::new(r.timer()).size(12.0).monospace().color(TEXT_DIM)).selectable(false));
                            });
                        });
                        ui.add_space(3.0);
                        let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 3.0), Sense::hover());
                        let p = ui.painter();
                        p.rect_filled(rect, 1.5, BORDER);
                        match (r.exit, pct) {
                            // Running with a percentage in its last output line: that much.
                            (None, Some(v)) => {
                                let mut f = rect;
                                f.set_width(rect.width() * v / 100.0);
                                p.rect_filled(f, 1.5, col);
                            }
                            // Running: a segment sliding across.
                            (None, None) => {
                                let t = (ui.input(|i| i.time) as f32 * 0.6).fract();
                                let seg = rect.width() * 0.3;
                                let x0 = rect.left() - seg + (rect.width() + seg) * t;
                                let f = egui::Rect::from_min_max(egui::pos2(x0.max(rect.left()), rect.top()), egui::pos2((x0 + seg).min(rect.right()), rect.bottom()));
                                p.rect_filled(f, 1.5, col);
                                ctx.request_repaint();
                            }
                            // Finished: full, or the auto-dismiss countdown.
                            (Some(_), _) => {
                                let mut f = rect;
                                f.set_width(rect.width() * left.unwrap_or(1.0).clamp(0.0, 1.0));
                                p.rect_filled(f, 1.5, col);
                            }
                        }
                    });
                    let open = frame.response.interact(Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text("Show the panel");
                    // Registered after the pill's own click so it is the one hit over the [x].
                    let closed = close_slot.is_some_and(|rect| {
                        let x = ui.interact(rect, egui::Id::unique(("script_pill_close", r.id)), Sense::click());
                        let p = ui.painter();
                        if x.hovered() {
                            p.rect_filled(rect, 5.0, if x.is_pointer_button_down_on() { Color32::from_rgb(62, 66, 76) } else { BORDER });
                        }
                        widgets::paint_icon(p, Icon::Close, rect.center(), PILL_CLOSE * 0.55, if x.hovered() { TEXT } else { TEXT_DIM });
                        x.on_hover_text("Close").clicked()
                    });
                    (open.clicked(), closed)
                });
                let (open, closed) = resp.inner;
                if closed {
                    close.push(r.id);
                } else if open {
                    r.minimized = false;
                    r.dismiss_from = None;
                }
                y += resp.response.rect.height() + 8.0;
                continue;
            }
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
                            if widgets::icon_button(ui, Icon::Minimize, 22.0, TEXT_DIM, "Minimize to a pill").clicked() {
                                r.minimized = true;
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
                            ui.label(RichText::new(r.timer()).size(12.0).monospace().color(TEXT_DIM));
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
                Ok(mut n) => {
                    n.minimized = r.minimized;
                    self.script_runs[i] = n;
                }
                Err(e) => {
                    let msg = format!("Couldn't run {}: {e}", r.name);
                    self.toast(msg, RED, ctx);
                }
            }
        }
        // Drop stops a run that's still going.
        self.script_runs.retain(|r| !close.contains(&r.id));
    }

    /// The current project's starred scripts that exist (reads the folders when the project changed).
    pub(super) fn fav_script_list(&mut self) -> Vec<usize> {
        if self.fav_list(super::favorites::FavList::Scripts).is_empty() && self.s.pinned_scripts.is_empty() {
            return Vec::new();
        }
        if self.scripts_for.as_ref() != Some(&self.current_project()) {
            self.refresh_scripts();
        }
        (0..self.scripts.len()).filter(|&i| self.script_in_project(&self.scripts[i]) && self.is_fav(super::favorites::FavList::Scripts, &self.scripts[i].key())).collect()
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
        let cur = self.current_project();
        let project = project_name(&cur);
        let mut common_on = self.common_scripts_on(&cur);
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
                    let dir = project_dir(&cur);
                    if ui.button("Open folder").on_hover_text(format!("This project's scripts: {}", dir.display())).clicked() {
                        let _ = std::fs::create_dir_all(&dir);
                        let _ = std::process::Command::new("explorer.exe").arg(&dir).spawn();
                    }
                    if ui.button("Refresh").clicked() {
                        self.refresh_scripts();
                    }
                    if ui.button("Edit scripts…").clicked() {
                        edit = true;
                    }
                });
            });
            // The filter, with the common-scripts switch at its right end.
            let te = ui
                .horizontal(|ui| {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.checkbox(&mut common_on, "Common scripts").on_hover_text(format!("List the common scripts (shared by every project) in {project}: {}", common_dir().display()));
                        ui.add(egui::TextEdit::singleline(&mut self.script_filter).hint_text("Filter").desired_width(ui.available_width()))
                    })
                    .inner
                })
                .inner;
            if std::mem::take(&mut self.script_focus) {
                te.request_focus();
            }
            ui.add_space(6.0);
            let f = self.script_filter.to_lowercase();
            let here: Vec<usize> = (0..self.scripts.len()).filter(|&i| self.script_in_project(&self.scripts[i])).collect();
            let shown: Vec<usize> = here.iter().copied().filter(|&i| f.is_empty() || self.scripts[i].name.to_lowercase().contains(&f) || self.scripts[i].desc.to_lowercase().contains(&f)).collect();
            if te.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                pick = shown.first().copied();
            }
            if here.is_empty() && !self.scripts.is_empty() {
                ui.label(RichText::new(format!("No scripts for {project}. The common ones are switched off here or limited to other projects (Edit scripts…).")).color(TEXT_FAINT));
            } else if self.scripts.is_empty() {
                ui.label(RichText::new(format!("No scripts yet. Put .ps1, .cmd, .bat or .py files in {project}'s scripts folder (Open folder) or in the common folder for every project, or ask an agent to write one (Edit scripts… > Copy prompt).")).color(TEXT_FAINT));
            }
            super::snippets::palette_scroll(ui, shown.len()).show_hinted(ui, |ui| {
                for i in shown {
                    let sc = &self.scripts[i];
                    // The AI model it will use, else the file type.
                    let note = if sc.ai { model_note(&catalog, self.script_model(sc).as_ref()) } else { sc.path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase() };
                    let note = if sc.common { format!("common · {note}") } else { note };
                    let mut row = super::snippets::palette_row(ui, &sc.name, &sc.desc, &note, 28.0);
                    if sc.ai {
                        row = row.on_hover_text("Uses this AI model. Change it in Edit scripts…");
                    } else if sc.common {
                        row = row.on_hover_text("A common script, listed in every project");
                    }
                    let key = sc.key();
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
        if common_on != self.common_scripts_on(&cur) {
            self.set_common_scripts(&cur, common_on);
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
    /// Shown as a small pill (status and progress bar) instead of the panel.
    minimized: bool,
    /// When the output last changed (see `text`).
    out_at: Instant,
    /// The output length when the panel was last opened for a question (so it opens once per question).
    asked_at: usize,
    /// When the auto-dismiss countdown started (it finished successfully; a click in the panel cancels it).
    dismiss_from: Option<Instant>,
    /// When it started, and how long it ran once it ended (for the timer).
    started: Instant,
    took: Option<Duration>,
}

/// Width of a minimized panel's pill (inside its margins).
const PILL_W: f32 = 300.0;
/// Size of the [x] on a finished script's pill.
const PILL_CLOSE: f32 = 18.0;
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
            minimized: false,
            out_at: Instant::now(),
            asked_at: 0,
            dismiss_from: None,
            started: Instant::now(),
            took: None,
        })
    }

    /// Checks whether it ended; returns the exit code the first time it does.
    fn poll(&mut self) -> Option<i32> {
        if self.exit.is_some() {
            return None;
        }
        let code = self.child.try_wait().ok().flatten()?.code().unwrap_or(-1);
        self.exit = Some(code);
        self.took = Some(self.started.elapsed());
        self.stdin = None;
        Some(code)
    }

    /// Time it has been running (or ran): m:ss, or h:mm:ss past an hour.
    fn timer(&self) -> String {
        let s = self.took.unwrap_or_else(|| self.started.elapsed()).as_secs();
        if s >= 3600 { format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60) } else { format!("{}:{:02}", s / 60, s % 60) }
    }

    /// The output so far: UTF-8, without color codes, CRLF as LF.
    fn text(&mut self) -> String {
        let o = self.out.lock().unwrap();
        if self.text.0 != o.len() {
            self.text = (o.len(), clean_output(&String::from_utf8_lossy(&o)));
            self.out_at = Instant::now();
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

/// How long the output must stay quiet, ending in a question, before a minimized panel opens.
const ASK_IDLE: Duration = Duration::from_millis(500);

/// Whether the output's last non-empty line looks like a prompt ("Name?", "Continue (y/n):").
fn asks_question(text: &str) -> bool {
    text.lines().rev().find(|l| !l.trim().is_empty()).is_some_and(|l| {
        let l = l.trim_end();
        !l.starts_with("> ") && (l.ends_with('?') || l.ends_with(':'))
    })
}

/// The last percentage (0-100) in the output's last non-empty line, e.g. "Downloading 42%".
fn last_percent(text: &str) -> Option<f32> {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| regex::Regex::new(r"(\d{1,3}(?:\.\d+)?)\s?%").unwrap());
    let line = text.lines().rev().find(|l| !l.trim().is_empty())?;
    re.captures_iter(line).filter_map(|c| c[1].parse::<f32>().ok()).filter(|v| (0.0..=100.0).contains(v)).last()
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
    /// A common script (`scripts_common`), else one of the current project's.
    common: bool,
}

impl ScriptDraft {
    fn dirty(&self) -> bool {
        self.path.is_none() || self.name != self.saved_name || self.text != self.saved_text
    }

    fn from_file(s: &Script) -> Self {
        let text = std::fs::read_to_string(&s.path).unwrap_or_default().trim_start_matches('\u{feff}').replace("\r\n", "\n");
        let ext = s.path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
        ScriptDraft { path: Some(s.path.clone()), name: s.name.clone(), ext, saved_name: s.name.clone(), saved_text: text.clone(), text, confirm_delete: false, error: None, expanded: false, common: s.common }
    }

    /// Its favourites / project-limit key, once saved (see `key_for`).
    fn key(&self) -> Option<String> {
        self.path.as_deref().map(|p| key_for(p, self.common))
    }

    /// Its `script_models` key, once saved.
    fn model_key(&self) -> Option<String> {
        self.path.as_deref().map(|p| model_key_for(p, self.common))
    }

    /// Writes the file into `dir`, renaming it first when the name changed. Batch files get CRLF.
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

/// The dropdown on a script's card for the projects it's listed in: all, or the ticked ones.
fn project_picker(ui: &mut Ui, i: usize, projects: &[Project], scopes: &mut std::collections::HashMap<String, Vec<PathBuf>>, key: &str) {
    let cur: Vec<PathBuf> = scopes.get(key).cloned().unwrap_or_default();
    let names: Vec<&str> = projects.iter().filter(|p| cur.contains(&p.path)).map(|p| p.name.as_str()).collect();
    let shown = match names.as_slice() {
        [] if cur.is_empty() => "Projects: all".to_string(),
        [] => "Projects: none open".to_string(),
        [one] => format!("Project: {one}"),
        more => format!("Projects: {}", more.len()),
    };
    let mut next = cur.clone();
    let r = egui::ComboBox::from_id_salt(("script_projects", i)).selected_text(shown).width(170.0).show_ui(ui, |ui| {
        if ui.selectable_label(next.is_empty(), "All projects").clicked() {
            next.clear();
        }
        ui.separator();
        for p in projects {
            let mut on = next.contains(&p.path);
            if ui.checkbox(&mut on, &p.name).on_hover_text(p.path.display().to_string()).changed() {
                if on {
                    next.push(p.path.clone());
                } else {
                    next.retain(|x| x != &p.path);
                }
            }
        }
    });
    let tip = if names.len() > 1 { format!("Listed in: {}", names.join(", ")) } else { "The projects whose Scripts palette and Favourites list this script".to_string() };
    r.response.on_hover_text(tip);
    if next != cur {
        if next.is_empty() {
            scopes.remove(key);
        } else {
            scopes.insert(key.to_string(), next);
        }
    }
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
fn agent_prompt(dir: &Path, common: &Path) -> String {
    format!(
        "Write a script for AI Terminal's Scripts palette and save it in {} (this project's scripts; a script every project should list goes in {} instead). Make it a .ps1 file unless I ask otherwise; .cmd, .bat and .py also work. \
The file name, without the extension, is the name shown in the palette. Start the file with a one-line comment describing what it does: it's shown as the description. \
It runs hidden in the current project's folder, with its output shown in a small panel that also has a text box for answers, so it can ask questions. Read-Host doesn't show its prompt there: Write-Host the question first, and treat no answer ($null, end of input) as cancel. For git commands, set $env:GIT_PAGER = 'cat'. \
If it needs an LLM, call `claude -p --model haiku --tools '\"\"' --strict-mcp-config --no-session-persistence --system-prompt \"...\"` with only the input it needs, to keep it cheap. The model can be picked per script in the app: when set, it's in $env:AI_TERMINAL_CLI ('claude', 'codex' or 'antigravity') and $env:AI_TERMINAL_MODEL; use them when present. For codex: `codex exec -m <model> --skip-git-repo-check --sandbox read-only --output-last-message <file> -` with the prompt on stdin, then read the file. For antigravity: `agy --model <model> --disable-slash-commands --input-format stream-json --output-format stream-json -p \"\"` with one line on stdin, `{{\"event\":\"user\",\"message\":{{\"content\":\"<prompt>\"}}}}` (build it with ConvertTo-Json -Compress); the answer is in result.response of the output line whose event is \"result\". \
The script should: ",
        dir.display(),
        common.display()
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
    /// Reloads the editor list from disk (the current project's scripts and the common ones),
    /// keeping unsaved edits and new scripts. Switching projects drops the other project's drafts.
    fn sync_script_drafts(&mut self) {
        let cur = self.current_project();
        let mut old = std::mem::take(&mut self.script_drafts);
        if self.script_drafts_for.as_ref() != Some(&cur) {
            old.retain(|d| d.common);
        }
        let mut v = Vec::new();
        for f in scan(&cur) {
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
        self.script_drafts_for = Some(cur);
    }

    pub(super) fn options_scripts(&mut self, ui: &mut Ui) {
        let cur = self.current_project();
        if !self.scripts_synced || self.script_drafts_for.as_ref() != Some(&cur) {
            self.sync_script_drafts();
            self.scripts_synced = true;
        }
        let project = project_name(&cur);
        let (pdir, cdir) = (project_dir(&cur), common_dir());
        ui.label(RichText::new("Scripts").family(bold()).size(18.0).color(TEXT));
        ui.label(
            RichText::new(
                "Each project has its own scripts (its scripts folder), and common scripts (next to AI Terminal) are listed in every project. Run one from the Scripts button in the session header or the palette (Ctrl+Shift+R): it runs in the background in the current project, with its output in a small panel at the bottom right (type answers there if it asks something). The first comment line is shown as its description.",
            )
            .size(12.5)
            .color(TEXT_DIM),
        );
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Tip: ask an agent to write a script for you. Copy prompt gives you the start of a request with the folder and conventions filled in; add what the script should do.").size(12.5).color(TEXT_DIM));
        });
        ui.horizontal(|ui| {
            if ui.button("Copy prompt").on_hover_text("Copy a starter request to paste into a Claude or Codex session").clicked() {
                ui.ctx().copy_text(agent_prompt(&pdir, &cdir));
                self.toast("Prompt copied: paste it into a session and say what the script should do".into(), GREEN, ui.ctx());
            }
            if ui.button("Refresh").on_hover_text("Reload from disk (unsaved edits are kept)").clicked() {
                self.sync_script_drafts();
            }
        });
        ui.checkbox(&mut self.s.script_start_minimized, "Start scripts minimized to a pill (the panel opens when a script asks a question)");
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.s.script_auto_dismiss, "Close the output panel");
            let unit = if self.s.script_dismiss_secs == 1 { " second" } else { " seconds" };
            ui.add_enabled(self.s.script_auto_dismiss, egui::DragValue::new(&mut self.s.script_dismiss_secs).range(1..=10).speed(0.1).suffix(unit));
            ui.label("after a script finishes successfully (click it to keep it open)");
        });
        ui.label(RichText::new("Resize the panel by dragging its top or left edge; double-click the edge for the default size.").size(12.0).color(TEXT_FAINT));
        ui.label(RichText::new("Scripts that read $env:AI_TERMINAL_MODEL (and AI_TERMINAL_CLI) get a model picker: the Claude, Codex or Antigravity model you pick there is passed to them.").size(12.0).color(TEXT_FAINT));
        let mut common_on = self.common_scripts_on(&cur);
        // Section buttons: add a script (true = common), open a folder.
        let mut add: Option<bool> = None;
        let mut open_dir: Option<PathBuf> = None;
        let mut remove = None;
        let mut open: Option<PathBuf> = None;
        // A save changed what's on disk; (old key, new key) of a rename.
        let mut saved = false;
        let mut renamed: Vec<(String, String)> = Vec::new();
        let catalog = self.catalog.lock().unwrap().clone().unwrap_or_default();
        let models = &mut self.s.script_models;
        let scopes = &mut self.s.script_projects;
        let projects = &self.s.projects;
        for common in [false, true] {
            let dir = if common { &cdir } else { &pdir };
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                let title = if common { "COMMON SCRIPTS".to_string() } else { format!("{} SCRIPTS", project.to_uppercase()) };
                ui.label(RichText::new(title).size(12.0).family(bold()).color(TEXT_DIM));
                ui.add(egui::Label::new(RichText::new(dir.display().to_string()).size(12.0).color(TEXT_FAINT)).truncate());
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.button("Open folder").on_hover_text(dir.display().to_string()).clicked() {
                        open_dir = Some(dir.clone());
                    }
                });
            });
            if common {
                ui.label(RichText::new("Listed in every project. Projects on a script limits it to some projects.").size(12.0).color(TEXT_FAINT));
                ui.checkbox(&mut common_on, format!("List the common scripts in {project}")).on_hover_text("Also in the Scripts palette (Common)");
            } else {
                ui.label(RichText::new(format!("Only listed in {project}. Select another project to edit its scripts.")).size(12.0).color(TEXT_FAINT));
            }
            ui.add_space(4.0);
            let mut any = false;
            for (i, d) in self.script_drafts.iter_mut().enumerate().filter(|(_, d)| d.common == common) {
                any = true;
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
                            // Unsaved edits (or a new script) outline Save instead of a separate badge.
                            let mut save_btn = egui::Button::new(RichText::new("Save").color(Color32::WHITE)).fill(ACCENT);
                            if d.dirty() {
                                save_btn = save_btn.stroke(egui::Stroke::new(2.0, Color32::from_rgb(230, 140, 40)));
                            }
                            let save_resp = ui.add_enabled(d.dirty(), save_btn);
                            let save_resp = if d.dirty() {
                                save_resp.on_hover_text(if d.path.is_none() { "New script: not saved yet" } else { "Unsaved changes" })
                            } else {
                                save_resp
                            };
                            if save_resp.clicked() {
                                let (old, old_model) = (d.key(), d.model_key());
                                d.error = d.save_in(dir).err();
                                saved = true;
                                // A rename keeps the script's model, project limits and favourite stars.
                                if let (Some(om), Some(nm)) = (old_model, d.model_key()) {
                                    if om != nm {
                                        if let Some(m) = models.remove(&om) {
                                            models.insert(nm, m);
                                        }
                                    }
                                }
                                if let (Some(old), Some(new)) = (old, d.key()) {
                                    if old != new {
                                        if let Some(ps) = scopes.remove(&old) {
                                            scopes.insert(new.clone(), ps);
                                        }
                                        renamed.push((old, new));
                                    }
                                }
                                d.confirm_delete = false;
                            }
                        });
                    });
                    // Second line: the projects it's listed in (a saved common script), the AI model
                    // (scripts that call one) and, collapsed, the description.
                    let key = d.key();
                    let ai_key = d.model_key().filter(|_| uses_ai(&d.saved_text));
                    if key.is_some() || !d.expanded {
                        ui.horizontal(|ui| {
                            if let Some(key) = key.as_ref().filter(|_| d.common) {
                                project_picker(ui, i, projects, scopes, key);
                            }
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
            if !any {
                ui.label(RichText::new("None yet.").size(12.5).color(TEXT_FAINT));
                ui.add_space(4.0);
            }
            ui.horizontal(|ui| {
                if ui.add(egui::Button::new(RichText::new("+ Add script").color(Color32::WHITE)).fill(ACCENT)).clicked() {
                    add = Some(common);
                }
                egui::ComboBox::from_id_salt(("script_new_ext", common)).selected_text(format!(".{}", self.script_new_ext)).width(70.0).show_ui(ui, |ui| {
                    for k in KINDS {
                        ui.selectable_value(&mut self.script_new_ext, k.to_string(), format!(".{k}"));
                    }
                });
            });
        }
        for (old, new) in &renamed {
            self.rename_fav(super::favorites::FavList::Scripts, old, new);
        }
        if let Some(i) = remove {
            let d = self.script_drafts.remove(i);
            if let Some(key) = d.model_key() {
                self.s.script_models.remove(&key);
            }
            if let Some(key) = d.key() {
                self.s.script_projects.remove(&key);
                self.forget_fav(super::favorites::FavList::Scripts, &key);
            }
            if let Some(p) = d.path {
                if let Err(e) = std::fs::remove_file(&p) {
                    self.toast(format!("Couldn't delete {}: {e}", p.display()), RED, ui.ctx());
                }
            }
            saved = true;
        }
        if common_on != self.common_scripts_on(&cur) {
            self.set_common_scripts(&cur, common_on);
        }
        // The palette and Favourites panel run from this list: keep it in step with the folders.
        if saved {
            self.refresh_scripts();
        }
        if let Some(p) = open {
            if let Err(e) = crate::links::open_file(&p, None, None, &self.s.editor, &self.s.editor_custom) {
                self.toast(format!("Couldn't open the editor: {e}"), RED, ui.ctx());
            }
        }
        if let Some(dir) = open_dir {
            let _ = std::fs::create_dir_all(&dir);
            let _ = std::process::Command::new("explorer.exe").arg(&dir).spawn();
        }
        if let Some(common) = add {
            let taken = |n: &str| self.script_drafts.iter().any(|d| d.common == common && d.name.eq_ignore_ascii_case(n));
            let name = (1..).map(|k| if k == 1 { "New script".to_string() } else { format!("New script {k}") }).find(|n| !taken(n)).unwrap();
            let ext = self.script_new_ext.clone();
            self.script_drafts.push(ScriptDraft { path: None, saved_name: String::new(), saved_text: String::new(), text: template(&ext), name, ext, confirm_delete: false, error: None, expanded: true, common });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{first_comment, ScriptDraft};

    #[test]
    fn save_rename_and_conflicts() {
        let dir = std::env::temp_dir().join(format!("ai_terminal_scripts_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let new = |name: &str, ext: &str, text: &str| ScriptDraft { path: None, name: name.into(), ext: ext.into(), text: text.into(), saved_name: String::new(), saved_text: String::new(), confirm_delete: false, error: None, expanded: false, common: false };
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
    fn script_keys() {
        let p = std::path::Path::new(r"X:\Proj\scripts\Push It.cmd");
        assert_eq!(super::key_for(p, false), "push it.cmd");
        assert_eq!(super::key_for(p, true), "common/push it.cmd");
        assert_eq!(super::model_key_for(p, false), r"x:\proj|push it.cmd");
        assert_eq!(super::model_key_for(p, true), "common/push it.cmd");
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
        let sc = super::Script { name: "Ask".into(), path, desc: String::new(), ai: false, common: false };
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
    fn detects_questions() {
        assert!(super::asks_question("Working
Name?
"));
        assert!(super::asks_question("Continue (y/n): "));
        assert!(!super::asks_question("Name?
> Bob
"));
        assert!(!super::asks_question("Downloading 40%"));
        assert!(!super::asks_question(""));
    }

    #[test]
    fn output_cleanup() {
        assert_eq!(super::clean_output("\x1b[32mok\x1b[0m\r\nnext\r"), "ok\nnext");
    }
}
