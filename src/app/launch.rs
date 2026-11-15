//! Starting sessions: the new-session menu and the launch dialog.

use super::*;

impl App {
    /// The New-session menu: per agent "New X session", "(unrestricted)",
    /// "(with settings)", then "New Shell (…)" per shell.
    pub(super) fn new_session_menu(&mut self, ui: &mut Ui, ctx: &egui::Context, project: &Path) {
        ui.set_min_width(330.0);
        let profiles = self.profiles.clone();
        let mut chosen: Option<(Profile, Option<bool>)> = None; // (profile, Some(unrestricted) | None = with settings)
        for p in profiles.iter().filter(|p| p.is_agent()) {
            let n = &p.name;
            if menu_item(ui, agent_icon(p), &format!("New {n} session")) {
                chosen = Some((p.clone(), Some(false)));
            }
            if menu_item(ui, agent_icon(p), &format!("New {n} session (unrestricted)")) {
                chosen = Some((p.clone(), Some(true)));
            }
            if menu_item(ui, agent_icon(p), &format!("New {n} session (with settings)")) {
                chosen = Some((p.clone(), None));
            }
        }
        for p in profiles.iter().filter(|p| !p.is_agent()) {
            if menu_item(ui, (Icon::Prompt, Color32::from_rgb(78, 201, 176)), &format!("New Shell ({})", p.name)) {
                chosen = Some((p.clone(), Some(false)));
            }
        }
        if let Some((p, mode)) = chosen {
            ui.close();
            match mode {
                Some(unrestricted) => {
                    let mut opts = if p.is_agent() { self.last_opts(p.kind).clone() } else { LaunchOpts::default() };
                    opts.unrestricted = unrestricted && p.is_agent();
                    self.spawn_opts(ctx, project.to_path_buf(), p, String::new(), opts);
                }
                None => self.open_launch_dialog(project.to_path_buf(), p, None),
            }
        }
    }

    /// Last confirmed "with settings" values for an agent (the dialog starts from them; plain
    /// new sessions reuse model / effort / parameters but not unrestricted).
    pub(super) fn last_opts(&self, kind: Kind) -> &LaunchOpts {
        match kind {
            Kind::Codex => &self.s.last_codex_opts,
            _ => &self.s.last_claude_opts,
        }
    }

    pub(super) fn open_launch_dialog(&mut self, project: PathBuf, profile: Profile, restart: Option<u64>) {
        let (opts, name) = match restart.and_then(|id| self.tabs.iter().find(|t| t.sess.id == id)) {
            Some(t) => (t.sess.opts.clone(), t.sess.name.clone()),
            None => match restart.and_then(|id| self.dormant.iter().find(|d| d.id == id)) {
                Some(d) => (d.saved.opts.clone(), d.saved.name.clone()),
                None => (self.last_opts(profile.kind).clone(), String::new()),
            },
        };
        self.launch = Some(LaunchDialog { project, profile, name, opts, restart });
    }

    pub(super) fn launch_dialog(&mut self, ctx: &egui::Context) {
        let Some(d) = &mut self.launch else { return };
        let catalog = self.catalog.lock().unwrap().clone().unwrap_or_default();
        let is_claude = d.profile.is_claude();
        let models: Vec<agents::ModelInfo> = if is_claude { catalog.claude_models.clone() } else { catalog.codex_models.clone() };
        let mut result: Option<bool> = None;
        let history = self.s.cli_history.clone();
        let mut forget: Option<String> = None;
        let title = match d.restart {
            Some(_) => format!("Restart {} session with settings", d.profile.name),
            None => format!("New {} session", d.profile.name),
        };
        let m = egui::Modal::new(egui::Id::unique("launch")).frame(egui::Frame::new().fill(PANEL).corner_radius(10).stroke(Stroke::new(1.0, BORDER)).inner_margin(20)).show(ctx, |ui| {
            ui.set_width(560.0);
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(Vec2::splat(22.0), Sense::hover());
                let (ic, col) = agent_icon(&d.profile);
                widgets::paint_icon(ui.painter(), ic, r.center(), 16.0, col);
                ui.label(RichText::new(title).family(bold()).size(16.0).color(TEXT));
            });
            ui.label(RichText::new(d.project.display().to_string()).size(12.0).color(TEXT_FAINT));
            ui.add_space(10.0);
            egui::Grid::new("launch_grid").num_columns(2).spacing([14.0, 10.0]).show(ui, |ui| {
                ui.label(RichText::new("Session Name").color(TEXT));
                ui.add(egui::TextEdit::singleline(&mut d.name).hint_text("New session (named from the first prompt if left empty)").desired_width(380.0));
                ui.end_row();

                ui.label(RichText::new("Model").color(TEXT));
                let shown = d.opts.model.clone().map(|m| models.iter().find(|x| x.id == m).map(|x| x.label.clone()).unwrap_or(m)).unwrap_or_else(|| "Use CLI default".into());
                egui::ComboBox::from_id_salt("launch_model").selected_text(shown).width(380.0).show_ui(ui, |ui| {
                    ui.selectable_value(&mut d.opts.model, None, "Use CLI default");
                    for m in &models {
                        ui.selectable_value(&mut d.opts.model, Some(m.id.clone()), &m.label);
                    }
                    if let Some(saved) = d.opts.model.clone().filter(|m| !models.iter().any(|x| &x.id == m)) {
                        ui.selectable_value(&mut d.opts.model, Some(saved.clone()), format!("{saved} (saved)"));
                    }
                });
                ui.end_row();

                ui.label(RichText::new("Effort").color(TEXT));
                let sel_model = d.opts.model.as_ref().and_then(|m| models.iter().find(|x| &x.id == m));
                let mut levels: Vec<String> = match sel_model {
                    Some(m) if !m.efforts.is_empty() => m.efforts.clone(),
                    _ if is_claude => catalog.claude_efforts.clone(),
                    _ => {
                        let mut all: Vec<String> = Vec::new();
                        for m in &models {
                            for e in &m.efforts {
                                if !all.contains(e) {
                                    all.push(e.clone());
                                }
                            }
                        }
                        all
                    }
                };
                if let Some(e) = d.opts.effort.clone().filter(|e| !levels.contains(e)) {
                    levels.push(e);
                }
                let default_label = match sel_model.and_then(|m| m.default_effort.clone()) {
                    Some(e) => format!("Use model default ({e})"),
                    None => "Use model default".into(),
                };
                egui::ComboBox::from_id_salt("launch_effort").selected_text(d.opts.effort.clone().unwrap_or_else(|| default_label.clone())).width(380.0).show_ui(ui, |ui| {
                    ui.selectable_value(&mut d.opts.effort, None, default_label.clone());
                    for e in &levels {
                        ui.selectable_value(&mut d.opts.effort, Some(e.clone()), e);
                    }
                });
                ui.end_row();

                ui.label(RichText::new("CLI Parameters").color(TEXT));
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut d.opts.cli_params).desired_width(340.0).font(egui::TextStyle::Monospace));
                    let hist = ui.add_enabled(!history.is_empty(), egui::Button::new("▾")).on_hover_text("Recent parameters");
                    egui::Popup::menu(&hist).show(|ui| {
                        for h in &history {
                            ui.horizontal(|ui| {
                                if ui.button(RichText::new(h).monospace()).clicked() {
                                    d.opts.cli_params = h.clone();
                                    ui.close();
                                }
                                if ui.small_button("×").on_hover_text("Remove from history").clicked() {
                                    forget = Some(h.clone());
                                }
                            });
                        }
                    });
                });
                ui.end_row();
                ui.label("");
                ui.label(RichText::new(format!("Extra parameters passed to {}. Model and effort are appended when launching.", if is_claude { "claude" } else { "codex" })).size(12.0).color(TEXT_FAINT));
                ui.end_row();
            });
            ui.add_space(6.0);
            if is_claude {
                ui.add_enabled(d.opts.model.is_some(), egui::Checkbox::new(&mut d.opts.one_m, "1M context")).on_disabled_hover_text("Pick a model first");
            }
            let flag = if is_claude { "--dangerously-skip-permissions" } else { "--dangerously-bypass-approvals-and-sandbox" };
            ui.checkbox(&mut d.opts.unrestricted, "Unrestricted mode").on_hover_text(format!("Starts with {flag}: the agent runs commands and edits files without asking."));
            if models.is_empty() {
                ui.label(RichText::new("No model list was found (the CLI didn't report one); you can still type --model in CLI Parameters.").size(12.0).color(ORANGE));
            }
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                if ui.button("Clear CLI").clicked() {
                    d.opts.cli_params.clear();
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let ok = if d.restart.is_some() { "Save and Restart" } else { "Create" };
                    if widgets::primary_button(ui, Icon::Plus, ok, ACCENT, "").clicked() {
                        result = Some(true);
                    }
                    if ui.button("Cancel").clicked() {
                        result = Some(false);
                    }
                });
            });
            if ui.input(|i| i.key_pressed(Key::Enter)) && !egui::Popup::is_any_open(ui.ctx()) {
                result = Some(true);
            }
        });
        if let Some(h) = forget {
            self.s.cli_history.retain(|x| *x != h);
        }
        if m.should_close() && result.is_none() {
            result = Some(false);
        }
        let Some(ok) = result else { return };
        let d = self.launch.take().unwrap();
        self.focus_terminal = true;
        if !ok {
            return;
        }
        if d.opts.model.is_none() {
            // 1M context needs a model.
        }
        let mut opts = d.opts.clone();
        if opts.model.is_none() {
            opts.one_m = false;
        }
        let p = opts.cli_params.trim().to_string();
        if !p.is_empty() {
            self.s.cli_history.retain(|x| *x != p);
            self.s.cli_history.insert(0, p);
            self.s.cli_history.truncate(12);
        }
        match d.profile.kind {
            Kind::Codex => self.s.last_codex_opts = LaunchOpts { unrestricted: false, ..opts.clone() },
            _ => self.s.last_claude_opts = LaunchOpts { unrestricted: false, ..opts.clone() },
        }
        match d.restart {
            Some(id) if self.dormant.iter().any(|x| x.id == id) => {
                if let Some(x) = self.dormant.iter_mut().find(|x| x.id == id) {
                    x.saved.opts = opts;
                    x.saved.name = d.name.trim().to_string();
                }
                self.resume(ctx, id);
            }
            Some(id) => {
                if let Some(t) = self.tabs.iter_mut().find(|t| t.sess.id == id) {
                    t.sess.name = d.name.trim().to_string();
                }
                self.restart_with(ctx, id, Some(opts));
            }
            None => self.spawn_opts(ctx, d.project, d.profile, d.name.trim().to_string(), opts),
        }
    }
}
