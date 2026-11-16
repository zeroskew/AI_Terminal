//! Starting sessions: the new-session menu and the launch dialog.

use super::*;

/// The CLI's command name for the launch dialog ("claude", "agy", ...).
fn cli_name(p: &Profile) -> String {
    let prog = if p.args.len() == 3 && p.args[0] == "/s" { &p.args[2] } else { &p.program };
    prog.rsplit(['\\', '/']).next().unwrap_or(prog).trim_end_matches(".exe").to_lowercase()
}

pub(super) const VARIANT_UNRESTRICTED: &str = "unrestricted";
pub(super) const VARIANT_SETTINGS: &str = "settings";

/// A launch row's label: "New Codex session (unrestricted)", "New Shell (cmd)", ...
pub(super) fn launch_label(p: &Profile, variant: &str) -> String {
    if !p.is_agent() {
        return format!("New Shell ({})", p.name);
    }
    match variant {
        VARIANT_UNRESTRICTED => format!("New {} session (unrestricted)", p.name),
        VARIANT_SETTINGS => format!("New {} session (with settings)", p.name),
        _ => format!("New {} session", p.name),
    }
}

impl App {
    /// The default launch (New-session button, project page, the "new default session" key):
    /// its profile and variant; plain when the saved variant doesn't apply to the profile.
    pub(super) fn default_launch(&self) -> (Profile, &'static str) {
        let p = self.default_profile();
        let v = match self.s.default_variant.as_str() {
            VARIANT_UNRESTRICTED if p.kind.unrestricted_flag().is_some() => VARIANT_UNRESTRICTED,
            VARIANT_SETTINGS if p.is_agent() => VARIANT_SETTINGS,
            _ => "",
        };
        (p, v)
    }

    /// Runs the default launch in `project`.
    pub(super) fn launch_default(&mut self, ctx: &egui::Context, project: PathBuf) {
        let (p, v) = self.default_launch();
        match v {
            VARIANT_SETTINGS => self.open_launch_dialog(project, p, None),
            _ => self.new_default(ctx, project, p, v == VARIANT_UNRESTRICTED),
        }
    }

    /// The New-session menu: per agent "New X session", "(unrestricted)", "(model) ▸",
    /// "(with settings)", then "New Shell (…)" per shell.
    pub(super) fn new_session_menu(&mut self, ui: &mut Ui, ctx: &egui::Context, project: &Path) {
        ui.set_min_width(330.0);
        // Pick up agent CLIs installed (or removed) while the app runs; a handful of file checks.
        if self.profiles_at.elapsed() > std::time::Duration::from_secs(3) {
            let fresh = Profile::detect();
            self.profiles_at = std::time::Instant::now();
            if fresh != self.profiles {
                self.profiles = fresh;
                self.refresh_catalog(ctx);
            }
        }
        let profiles = self.profiles.clone();
        let catalog = self.catalog.lock().unwrap().clone().unwrap_or_default();
        let mut chosen: Option<(Profile, Option<bool>)> = None; // (profile, Some(unrestricted) | None = with settings)
        let mut with_model: Option<(Profile, Option<String>)> = None; // (profile, model | None = CLI default)
        let loading = self.catalog_loading();
        let mut refresh = false;
        let mut set_default: Option<(String, &str)> = None; // (profile name, variant)
        let (def_profile, def_variant) = self.default_launch();
        let def_profile = def_profile.name;
        // A launch row with its default marker; true when clicked.
        let mut row = |ui: &mut Ui, p: &Profile, variant: &'static str| -> bool {
            let (clicked, make_default) = menu_item_default(ui, agent_icon(p), &launch_label(p, variant), p.name == def_profile && def_variant == variant);
            if make_default {
                set_default = Some((p.name.clone(), variant));
            }
            clicked
        };
        for p in profiles.iter().filter(|p| p.is_agent()) {
            let n = &p.name;
            if row(ui, p, "") {
                chosen = Some((p.clone(), Some(false)));
            }
            if p.kind.unrestricted_flag().is_some() && row(ui, p, VARIANT_UNRESTRICTED) {
                chosen = Some((p.clone(), Some(true)));
            }
            let models = catalog.models(p.kind);
            {
                let row = menu_row(ui, agent_icon(p), &format!("New {n} session (model)"), true);
                let current = self.last_opts(p.kind).model;
                egui::containers::menu::SubMenu::new().show(ui, &row, |ui| {
                    ui.set_min_width(220.0);
                    let mut pick = |ui: &mut Ui, id: Option<String>, label: &str| {
                        if ui.add(egui::Button::selectable(current == id, label)).clicked() {
                            with_model = Some((p.clone(), id));
                            ui.close();
                        }
                    };
                    pick(ui, None, "Use CLI default");
                    for m in models {
                        pick(ui, Some(m.id.clone()), &m.label);
                    }
                    if models.is_empty() && !loading {
                        ui.label(RichText::new("No model list (the CLI didn't report one)").size(12.0).color(TEXT_FAINT));
                    }
                    ui.separator();
                    // Keeps the menu open so the new list shows up in place.
                    if ui.add_enabled(!loading, egui::Button::new(if loading { "Refreshing models…" } else { "⟳ Refresh models" })).clicked() {
                        refresh = true;
                    }
                });
            }
            if row(ui, p, VARIANT_SETTINGS) {
                chosen = Some((p.clone(), None));
            }
        }
        for p in profiles.iter().filter(|p| !p.is_agent()) {
            if row(ui, p, "") {
                chosen = Some((p.clone(), Some(false)));
            }
        }
        if let Some((name, variant)) = set_default {
            self.s.default_profile = name;
            self.s.default_variant = variant.to_string();
        }
        // Agent CLIs that weren't found: install them from here.
        // Agent CLIs that weren't found: install them from here (same table and action as
        // Options > Agents, which also uninstalls).
        let missing: Vec<&'static crate::agents::AgentSpec> = crate::agents::AGENTS.iter().filter(|s| !profiles.iter().any(|p| p.kind == s.kind)).collect();
        let mut install: Option<&'static crate::agents::AgentSpec> = None;
        let mut redetect = false;
        let mut manage = false;
        ui.separator();
        if !missing.is_empty() {
            let row = menu_row(ui, (Icon::Plus, TEXT_FAINT), &format!("Need to install ({})", missing.len()), true);
            egui::containers::menu::SubMenu::new().show(ui, &row, |ui| {
                ui.set_min_width(300.0);
                for spec in &missing {
                    let busy = self.agent_job_running(spec.kind);
                    ui.horizontal(|ui| {
                        let (r, _) = ui.allocate_exact_size(Vec2::splat(20.0), Sense::hover());
                        let probe = Profile { name: spec.name.to_string(), program: String::new(), args: vec![], kind: spec.kind };
                        let (ic, col) = agent_icon(&probe);
                        widgets::paint_icon(ui.painter(), ic, r.center(), 14.0, col);
                        ui.label(RichText::new(spec.name).color(TEXT));
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let label = if busy { "Installing…" } else { "Install" };
                            if ui.add_enabled(!busy, egui::Button::new(label)).on_hover_text(format!("Runs in the background:\n{}", spec.install)).clicked() {
                                install = Some(spec);
                                ui.close();
                            }
                            if ui.link("website").on_hover_text(spec.url).clicked() {
                                crate::links::open_url(spec.url);
                            }
                        });
                    });
                }
                ui.separator();
                if ui.button("⟳ Re-detect agents").on_hover_text("Look for the agent CLIs again (after installing one)").clicked() {
                    redetect = true;
                }
            });
        }
        if menu_row(ui, (Icon::Gear, TEXT_FAINT), "Manage agents…", false).on_hover_text("Options > Agents: install and uninstall agent CLIs").clicked() {
            manage = true;
        }
        if manage {
            ui.close();
            self.options = Some(OptPage::Agents);
            return;
        }
        if redetect {
            self.redetect_agents(ctx);
            let found: Vec<&str> = missing.iter().filter(|s| self.profiles.iter().any(|p| p.kind == s.kind)).map(|s| s.name).collect();
            let msg = if found.is_empty() { "No new agent CLIs found".to_string() } else { format!("Found: {}", found.join(", ")) };
            self.toast(msg, if found.is_empty() { TEXT_FAINT } else { GREEN }, ctx);
        }
        if let Some(spec) = install {
            ui.close();
            self.install_agent(ctx, spec);
            return;
        }
        if refresh {
            self.refresh_catalog(ctx);
        }
        if let Some((p, model)) = with_model {
            // One-off: the last settings with this model (not remembered as the default).
            ui.close();
            let mut opts = self.last_opts(p.kind);
            opts.unrestricted = false;
            if let Some(info) = model.as_ref().and_then(|m| catalog.models(p.kind).iter().find(|x| &x.id == m)) {
                if opts.effort.as_ref().is_some_and(|e| !info.efforts.is_empty() && !info.efforts.contains(e)) {
                    opts.effort = None;
                }
            }
            if model.is_none() {
                opts.one_m = false;
            }
            opts.model = model;
            self.spawn_opts(ctx, project.to_path_buf(), p, String::new(), opts);
            return;
        }
        if let Some((p, mode)) = chosen {
            ui.close();
            match mode {
                Some(unrestricted) => {
                    let mut opts = if p.is_agent() { self.last_opts(p.kind) } else { LaunchOpts::default() };
                    opts.unrestricted = unrestricted && p.is_agent();
                    self.spawn_opts(ctx, project.to_path_buf(), p, String::new(), opts);
                }
                None => self.open_launch_dialog(project.to_path_buf(), p, None),
            }
        }
    }

    /// Re-reads the model lists from the CLIs (`claude --help`, `codex debug models`) in the
    /// background; the old list stays until the new one arrives.
    pub(super) fn refresh_catalog(&self, ctx: &egui::Context) {
        use std::sync::atomic::Ordering;
        if self.catalog_loading.swap(true, Ordering::SeqCst) {
            return;
        }
        let exe = |kind: Kind| self.profiles.iter().find(|p| p.kind == kind).map(|p| p.program.clone()).filter(|p| p.to_lowercase().ends_with(".exe") && !p.to_lowercase().ends_with("cmd.exe"));
        let (claude, codex, agy) = (exe(Kind::Claude), exe(Kind::Codex), exe(Kind::Antigravity));
        let (cat, loading, ctx) = (self.catalog.clone(), self.catalog_loading.clone(), ctx.clone());
        std::thread::spawn(move || {
            let c = agents::load_catalog(claude, codex, agy);
            *cat.lock().unwrap() = Some(c);
            loading.store(false, Ordering::SeqCst);
            ctx.request_repaint();
        });
    }

    fn catalog_loading(&self) -> bool {
        self.catalog_loading.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Last confirmed "with settings" values for an agent (the dialog starts from them; plain
    /// new sessions reuse model / effort / parameters but not unrestricted).
    pub(super) fn last_opts(&self, kind: Kind) -> LaunchOpts {
        match kind {
            Kind::Codex => self.s.last_codex_opts.clone(),
            Kind::Claude | Kind::Shell => self.s.last_claude_opts.clone(),
            _ => self.s.last_agent_opts.get(&format!("{kind:?}")).cloned().unwrap_or_default(),
        }
    }

    pub(super) fn open_launch_dialog(&mut self, project: PathBuf, profile: Profile, restart: Option<u64>) {
        let (opts, name) = match restart.and_then(|id| self.tabs.iter().find(|t| t.sess.id == id)) {
            Some(t) => (t.sess.opts.clone(), t.sess.name.clone()),
            None => match restart.and_then(|id| self.dormant.iter().find(|d| d.id == id)) {
                Some(d) => (d.saved.opts.clone(), d.saved.name.clone()),
                None => (self.last_opts(profile.kind), String::new()),
            },
        };
        self.launch = Some(LaunchDialog { project, profile, name, opts, restart });
    }

    pub(super) fn launch_dialog(&mut self, ctx: &egui::Context) {
        let loading = self.catalog_loading();
        let mut refresh = false;
        let Some(d) = &mut self.launch else { return };
        let catalog = self.catalog.lock().unwrap().clone().unwrap_or_default();
        let is_claude = d.profile.is_claude();
        let kind = d.profile.kind;
        let models: Vec<agents::ModelInfo> = catalog.models(kind).to_vec();
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
                if !kind.has_effort() {
                    // These CLIs don't report a model list: type the id (OpenCode: provider/model).
                    let mut m = d.opts.model.clone().unwrap_or_default();
                    let hint = if kind == Kind::OpenCode { "CLI default (or provider/model, e.g. deepseek/deepseek-chat)" } else { "CLI default (or a model id)" };
                    if ui.add(egui::TextEdit::singleline(&mut m).hint_text(hint).desired_width(380.0)).changed() {
                        d.opts.model = Some(m.trim().to_string()).filter(|m| !m.is_empty());
                    }
                    ui.end_row();
                } else {
                let shown = d.opts.model.clone().map(|m| models.iter().find(|x| x.id == m).map(|x| x.label.clone()).unwrap_or(m)).unwrap_or_else(|| "Use CLI default".into());
                ui.horizontal(|ui| {
                    egui::ComboBox::from_id_salt("launch_model").selected_text(shown).width(340.0).show_ui(ui, |ui| {
                        ui.selectable_value(&mut d.opts.model, None, "Use CLI default");
                        for m in &models {
                            ui.selectable_value(&mut d.opts.model, Some(m.id.clone()), &m.label);
                        }
                        if let Some(saved) = d.opts.model.clone().filter(|m| !models.iter().any(|x| &x.id == m)) {
                            ui.selectable_value(&mut d.opts.model, Some(saved.clone()), format!("{saved} (saved)"));
                        }
                    });
                    let tip = if loading { "Refreshing models…" } else { "Refresh the model list from the CLI" };
                    if ui.add_enabled(!loading, egui::Button::new("⟳")).on_hover_text(tip).on_disabled_hover_text(tip).clicked() {
                        refresh = true;
                    }
                });
                ui.end_row();
                }

                if kind.has_effort() {
                ui.label(RichText::new("Effort").color(TEXT));
                let sel_model = d.opts.model.as_ref().and_then(|m| models.iter().find(|x| &x.id == m));
                let mut levels = catalog.efforts(kind, d.opts.model.as_deref());
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
                }

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
                ui.label(RichText::new(format!("Extra parameters passed to {}. Model and effort are appended when launching.", cli_name(&d.profile))).size(12.0).color(TEXT_FAINT));
                ui.end_row();
            });
            ui.add_space(6.0);
            if is_claude {
                ui.add_enabled(d.opts.model.is_some(), egui::Checkbox::new(&mut d.opts.one_m, "1M context")).on_disabled_hover_text("Pick a model first");
            }
            if let Some(flag) = kind.unrestricted_flag() {
                ui.checkbox(&mut d.opts.unrestricted, "Unrestricted mode").on_hover_text(format!("Starts with {flag}: the agent runs commands and edits files without asking."));
            }
            if models.is_empty() && kind.has_effort() {
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
        if refresh {
            self.refresh_catalog(ctx);
        }
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
        let keep = LaunchOpts { unrestricted: false, ..opts.clone() };
        match d.profile.kind {
            Kind::Codex => self.s.last_codex_opts = keep,
            Kind::Claude | Kind::Shell => self.s.last_claude_opts = keep,
            k => {
                self.s.last_agent_opts.insert(format!("{k:?}"), keep);
            }
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
