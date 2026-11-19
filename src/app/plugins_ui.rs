//! The plugins window: one vertical tab per provider (Claude Code, Codex), each with its plugin
//! catalogue, sources and install / uninstall.

use super::*;
use std::collections::HashMap;

/// The providers the window has tabs for: (kind, tab label, CLI name).
const PROVIDERS: [(Kind, &str, &str); 2] = [(Kind::Claude, "Claude Code", "claude"), (Kind::Codex, "Codex", "codex")];

fn provider_icon(kind: Kind) -> (Icon, Color32) {
    match kind {
        Kind::Codex => (Icon::Codex, Color32::from_rgb(200, 204, 212)),
        _ => (Icon::Sparkle, Color32::from_rgb(217, 119, 87)),
    }
}

/// A Claude Code plugin's context cost, measured once from its installed copy (or the
/// marketplace checkout when not installed); `None` when there's no local copy.
fn plugin_cost(costs: &mut HashMap<String, Option<plugins::TokenCost>>, installed: &HashMap<String, Vec<plugins::Installed>>, p: &plugins::Plugin) -> Option<plugins::TokenCost> {
    costs
        .entry(p.id().to_string())
        .or_insert_with(|| {
            let inst = installed.get(p.id()).into_iter().flatten().filter_map(|i| i.path.clone());
            inst.chain(p.dir.clone()).find_map(|d| plugins::token_cost(&d))
        })
        .clone()
}

/// Claude Code's own figures for a plugin (`claude plugin details`), started on first ask;
/// `None` while it runs. Installed plugins are looked up by id (in their project for project
/// installs), others loaded from the marketplace checkout.
fn plugin_cli_cost(slots: &mut HashMap<String, plugins::DetailsRef>, cli: &(String, Vec<String>), cwd: &Path, installed: &HashMap<String, Vec<plugins::Installed>>, p: &plugins::Plugin, ctx: &egui::Context) -> Option<Result<plugins::CliCost, String>> {
    let slot = slots.entry(p.id().to_string()).or_insert_with(|| {
        let inst = installed.get(p.id());
        let cwd = inst.and_then(|v| if v.iter().any(|i| i.scope == "user") { None } else { v.iter().find_map(|i| i.project.clone()) }).unwrap_or_else(|| cwd.to_path_buf());
        plugins::details_async(&cli.0, &cli.1, p.id(), &p.name, p.dir.as_deref(), inst.is_some(), &cwd, ctx.clone())
    });
    let r = slot.lock().unwrap().clone();
    r
}

/// Tokens a plugin adds to every request: Claude Code's figure once fetched, else the estimate.
fn always_tokens(costs: &mut HashMap<String, Option<plugins::TokenCost>>, cli_costs: &HashMap<String, plugins::DetailsRef>, installed: &HashMap<String, Vec<plugins::Installed>>, p: &plugins::Plugin) -> Option<u64> {
    if let Some(Ok(c)) = cli_costs.get(p.id()).and_then(|s| s.lock().unwrap().clone()) {
        return Some(c.always);
    }
    plugin_cost(costs, installed, p).map(|c| c.always)
}

impl App {
    // ---------------------------------------------------------------------------------------
    // Plugins

    /// Opens the plugins window on the active session's provider (Claude Code otherwise).
    pub(super) fn open_plugins(&mut self) {
        let active = self.active_tab().map(|t| t.sess.profile.kind);
        let agent = match active {
            Some(Kind::Codex) if self.plugin_cli(Kind::Codex).is_some() => Kind::Codex,
            _ if self.plugin_cli(Kind::Claude).is_none() && self.plugin_cli(Kind::Codex).is_some() => Kind::Codex,
            _ => Kind::Claude,
        };
        self.plugins = Some(PluginsUi { agent, needs_load: true, scope: "user".into(), ..Default::default() });
    }

    /// How to run the claude CLI: (program, leading args).
    pub(super) fn claude_cli(&self) -> (String, Vec<String>) {
        self.plugin_cli(Kind::Claude).unwrap_or_else(|| ("claude".into(), Vec::new()))
    }

    /// How to run a provider's CLI, when it's installed: (program, leading args).
    fn plugin_cli(&self, kind: Kind) -> Option<(String, Vec<String>)> {
        self.profiles.iter().find(|p| p.kind == kind).map(|p| (p.program.clone(), p.args.clone()))
    }

    /// Shows a finished load (keeping the selection when the plugin is still listed).
    fn apply_plugins(st: &mut PluginsUi, loaded: plugins::Loaded) {
        st.catalogue = loaded.catalogue;
        st.installed = loaded.installed;
        st.markets = loaded.markets;
        if st.market_filter.as_ref().is_some_and(|m| !st.markets.iter().any(|x| &x.name == m)) {
            st.market_filter = None;
        }
        if !st.selected.as_ref().is_some_and(|id| st.catalogue.iter().any(|p| p.id() == id)) {
            st.selected = st.catalogue.first().map(|x| x.id().to_string());
        }
        st.load_error = None;
        st.costs.clear();
        st.enablement.clear();
        st.cli_costs.clear();
    }

    /// The plugins window (a modal).
    pub(super) fn plugins_window(&mut self, ctx: &egui::Context) {
        if self.plugins.is_none() {
            return;
        }
        let available: Vec<bool> = PROVIDERS.iter().map(|(k, ..)| self.plugin_cli(*k).is_some()).collect();
        let agent = self.plugins.as_ref().unwrap().agent;
        let cli_name = PROVIDERS.iter().find(|p| p.0 == agent).map(|p| p.2).unwrap_or("claude");
        let (cli, cli_args) = self.plugin_cli(agent).unwrap_or_else(|| (cli_name.to_string(), Vec::new()));
        let codex = agent == Kind::Codex;
        let st = self.plugins.as_mut().unwrap();
        // (Re)load the provider's plugins: Claude Code's from its files right away, Codex's
        // through its CLI on a thread.
        if std::mem::take(&mut st.needs_load) {
            if codex {
                st.loading = Some(plugins::codex_load_async(&cli, &cli_args, ctx.clone()));
            } else {
                Self::apply_plugins(st, plugins::Loaded { catalogue: plugins::catalogue(), installed: plugins::installed(), markets: plugins::markets() });
            }
        }
        if let Some(r) = st.loading.as_ref().and_then(|l| l.lock().unwrap().take()) {
            st.loading = None;
            match r {
                Ok(loaded) => Self::apply_plugins(st, loaded),
                Err(e) => st.load_error = Some(e),
            }
        }
        // Reload after a job finishes.
        let finished = st.job.lock().unwrap().last_ok.take();
        if finished.is_some() {
            if codex {
                st.needs_load = true;
                st.refreshing = false;
                ctx.request_repaint();
            } else {
                if std::mem::take(&mut st.refreshing) {
                    st.catalogue = plugins::catalogue();
                    st.markets = plugins::markets();
                    if st.market_filter.as_ref().is_some_and(|m| !st.markets.iter().any(|x| &x.name == m)) {
                        st.market_filter = None;
                    }
                }
                st.installed = plugins::installed();
                st.costs.clear();
                st.enablement.clear();
                st.cli_costs.clear();
            }
        }
        let screen = ctx.content_rect();
        let size = Vec2::new((screen.width() * 0.85).min(1450.0), (screen.height() * 0.85).min(900.0));
        let project = self.s.selected_project.clone().unwrap_or_else(|| self.s.projects[0].path.clone());
        let mut close = false;
        let mut run: Option<Vec<String>> = None;
        let mut refresh = false;
        let mut source_cmd: Option<Vec<String>> = None;
        let mut switch_to: Option<Kind> = None;
        let projects: Vec<PathBuf> = self.s.projects.iter().map(|p| p.path.clone()).collect();
        let st = self.plugins.as_mut().unwrap();
        st.projects = projects;
        st.current_project = project.clone();
        st.cli = (cli.clone(), cli_args.clone());
        let running = st.job.lock().unwrap().running;
        let loading = st.loading.is_some();
        let m = egui::Modal::new(egui::Id::unique("plugins")).frame(egui::Frame::new().fill(PANEL).corner_radius(10).stroke(Stroke::new(1.0, BORDER)).inner_margin(18)).show(ctx, |ui| {
            ui.set_width(size.x);
            ui.set_height(size.y);
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(Vec2::splat(20.0), Sense::hover());
                widgets::paint_icon(ui.painter(), Icon::Plugin, r.center(), 15.0, TEXT_DIM);
                ui.label(RichText::new("PLUGINS").size(12.5).family(bold()).color(TEXT_DIM));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if widgets::corner_close_button(ui, 18.0).clicked() {
                        close = true;
                    }
                    let tip = if codex { "codex plugin marketplace upgrade" } else { "claude plugin marketplace update" };
                    if ui.add_enabled(!running && !loading, egui::Button::new("⟳ Refresh")).on_hover_text(tip).clicked() {
                        refresh = true;
                    }
                });
            });
            ui.add_space(10.0);
            let tabs_w = 170.0;
            let inner_h = size.y - 40.0;
            ui.horizontal_top(|ui| {
                // Provider tabs.
                ui.allocate_ui_with_layout(Vec2::new(tabs_w, inner_h), Layout::top_down(Align::Min), |ui| {
                    ui.set_width(tabs_w);
                    ui.spacing_mut().item_spacing.y = 2.0;
                    for (i, (kind, label, _)) in PROVIDERS.iter().enumerate() {
                        let sel = *kind == agent;
                        let ok = available[i];
                        let (r, resp) = ui.allocate_exact_size(Vec2::new(tabs_w, 40.0), if ok { Sense::click() } else { Sense::hover() });
                        let painter = ui.painter();
                        if sel {
                            painter.rect_filled(r, 6.0, CARD_HI);
                            painter.rect_filled(egui::Rect::from_min_size(r.min, Vec2::new(3.0, r.height())), 0.0, ACCENT_HI);
                        } else if ok && resp.hovered() {
                            painter.rect_filled(r, 6.0, CARD);
                        }
                        let (icon, color) = provider_icon(*kind);
                        let dim = |c: Color32| if ok { c } else { c.gamma_multiply(0.4) };
                        widgets::paint_icon(painter, icon, egui::pos2(r.left() + 22.0, r.center().y), 15.0, dim(color));
                        painter.text(egui::pos2(r.left() + 40.0, r.center().y), egui::Align2::LEFT_CENTER, *label, egui::FontId::new(14.0, if sel { bold() } else { egui::FontFamily::Proportional }), dim(if sel { Color32::WHITE } else { TEXT }));
                        if sel && !st.installed.is_empty() {
                            painter.text(egui::pos2(r.right() - 10.0, r.center().y), egui::Align2::RIGHT_CENTER, st.installed.len().to_string(), egui::FontId::proportional(12.0), TEXT_FAINT);
                        }
                        let resp = if ok { resp.on_hover_text(format!("{label} plugins")) } else { resp.on_hover_text(format!("{label} isn't installed")) };
                        if ok && !sel && resp.clicked() && !running {
                            switch_to = Some(*kind);
                        }
                    }
                });
                let (sep, _) = ui.allocate_exact_size(Vec2::new(1.0, inner_h), Sense::hover());
                ui.painter().rect_filled(sep, 0.0, BORDER);
                ui.add_space(16.0);
                // Whatever is left, so the content ends where the header's close button does.
                let content_w = ui.available_width();
                ui.allocate_ui_with_layout(Vec2::new(content_w, inner_h), Layout::top_down(Align::Min), |ui| {
                    ui.set_width(content_w);
                    Self::plugins_content(ui, st, codex, cli_name, content_w, inner_h, running, loading, &mut run, &mut source_cmd);
                });
            });
        });
        if m.should_close() {
            close = true;
        }
        let st = self.plugins.as_mut().unwrap();
        if let Some(k) = switch_to {
            *st = PluginsUi { agent: k, needs_load: true, scope: "user".into(), show_log: st.show_log, job: st.job.clone(), ..Default::default() };
            ctx.request_repaint();
        }
        if refresh {
            st.refreshing = true;
            st.show_log = true;
            let args = if codex { vec!["plugin".into(), "marketplace".into(), "upgrade".into()] } else { vec!["plugin".into(), "marketplace".into(), "update".into()] };
            plugins::run(&st.job, cli_name, &cli, &cli_args, args, &project, ctx.clone());
        }
        if let Some(args) = run {
            st.show_log = true;
            plugins::run(&st.job, cli_name, &cli, &cli_args, args, &project, ctx.clone());
        }
        if let Some((args, cwd)) = st.toggle.take() {
            st.show_log = true;
            plugins::run(&st.job, cli_name, &cli, &cli_args, args, &cwd, ctx.clone());
        }
        if let Some(args) = source_cmd {
            // Reloads the sources and the catalogue when it finishes.
            st.refreshing = true;
            st.show_log = true;
            plugins::run(&st.job, cli_name, &cli, &cli_args, args, &project, ctx.clone());
        }
        if close {
            self.plugins = None;
            self.focus_terminal = true;
        }
    }

    /// The selected provider's side of the window: filters, sources, list, details, log, actions.
    #[allow(clippy::too_many_arguments)]
    fn plugins_content(ui: &mut Ui, st: &mut PluginsUi, codex: bool, cli_name: &str, width: f32, height: f32, running: bool, loading: bool, run: &mut Option<Vec<String>>, source_cmd: &mut Option<Vec<String>>) {
        ui.horizontal(|ui| {
            let filters: &[(PluginFilter, &str)] = if codex {
                &[(PluginFilter::All, "All"), (PluginFilter::Installed, "Installed")]
            } else {
                &[(PluginFilter::All, "All"), (PluginFilter::Bundles, "Bundles"), (PluginFilter::Plugins, "Plugins"), (PluginFilter::Installed, "Installed")]
            };
            for &(f, label) in filters {
                let sel = st.filter == f;
                let b = egui::Button::new(RichText::new(label).color(if sel { Color32::WHITE } else { TEXT })).fill(if sel { ACCENT } else { CARD_HI }).corner_radius(14).min_size(Vec2::new(70.0, 28.0));
                if ui.add(b).clicked() {
                    st.filter = f;
                }
            }
            ui.add_space(16.0);
            ui.label(RichText::new("Source").color(TEXT_DIM));
            let shown = st.market_filter.clone().unwrap_or_else(|| "All sources".into());
            egui::ComboBox::from_id_salt("plugin_source").selected_text(shown).width(240.0).show_ui(ui, |ui| {
                ui.selectable_value(&mut st.market_filter, None, "All sources");
                for m in &st.markets {
                    let n = st.catalogue.iter().filter(|p| p.marketplace == m.name).count();
                    ui.selectable_value(&mut st.market_filter, Some(m.name.clone()), format!("{}  ({n})", m.name)).on_hover_text(&m.source);
                }
            });
            let label = if st.show_sources { "Done" } else { "Manage sources…" };
            if ui.button(label).on_hover_text("Add or remove plugin sources (marketplaces)").clicked() {
                st.show_sources = !st.show_sources;
                st.confirm_remove = None;
            }
        });
        let (update_verb, remove_tip) = if codex {
            ("upgrade", "Removes the source from Codex (codex plugin marketplace remove).")
        } else {
            ("update", "Removes the source from Claude Code (claude plugin marketplace remove). Plugins installed from it may be removed too.")
        };
        if st.show_sources {
            ui.add_space(8.0);
            egui::Frame::new().fill(CARD).corner_radius(8).stroke(Stroke::new(1.0, BORDER)).inner_margin(12).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new("SOURCES").size(12.0).family(bold()).color(TEXT_DIM));
                ui.add_space(4.0);
                egui::Grid::new("plugin_sources").num_columns(4).spacing([16.0, 6.0]).striped(true).show(ui, |ui| {
                    for m in st.markets.clone() {
                        let n = st.catalogue.iter().filter(|p| p.marketplace == m.name).count();
                        ui.label(RichText::new(&m.name).family(bold()).color(TEXT));
                        ui.label(RichText::new(format!("{}  ·  {}", m.kind, m.source)).color(LINK));
                        ui.label(RichText::new(format!("{n} plugins")).color(TEXT_DIM));
                        ui.horizontal(|ui| {
                            if ui.add_enabled(!running, egui::Button::new("Update")).on_hover_text(format!("{cli_name} plugin marketplace {update_verb} {}", m.name)).clicked() {
                                *source_cmd = Some(vec!["plugin".into(), "marketplace".into(), update_verb.into(), m.name.clone()]);
                            }
                            let confirming = st.confirm_remove.as_deref() == Some(m.name.as_str());
                            let b = if confirming { egui::Button::new(RichText::new("Confirm remove").color(Color32::WHITE)).fill(Color32::from_rgb(150, 40, 40)) } else { egui::Button::new("Remove") };
                            let builtin = if codex { plugins::codex_builtin_market(&m.name) } else { m.name == plugins::DIRECTORY };
                            let tip = if builtin { if codex { "Managed by Codex; it can't be removed." } else { "Built into Claude Code; it can't be removed." } } else { remove_tip };
                            if ui.add_enabled(!running && !builtin, b).on_hover_text(tip).on_disabled_hover_text(tip).clicked() {
                                if confirming {
                                    *source_cmd = Some(vec!["plugin".into(), "marketplace".into(), "remove".into(), m.name.clone()]);
                                    st.confirm_remove = None;
                                } else {
                                    st.confirm_remove = Some(m.name.clone());
                                }
                            }
                        });
                        ui.end_row();
                    }
                });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Add source").color(TEXT));
                    let te = ui.add(egui::TextEdit::singleline(&mut st.new_source).hint_text("git URL, GitHub owner/repo, or a local folder").desired_width(480.0));
                    let ok = !st.new_source.trim().is_empty() && !running;
                    let add = ui.add_enabled(ok, egui::Button::new(RichText::new("Add").color(Color32::WHITE)).fill(ACCENT)).on_hover_text(format!("{cli_name} plugin marketplace add <source>")).clicked()
                        || (ok && te.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)));
                    if add {
                        *source_cmd = Some(vec!["plugin".into(), "marketplace".into(), "add".into(), st.new_source.trim().to_string()]);
                        st.new_source.clear();
                    }
                });
            });
        }
        ui.add_space(6.0);
        let installed = &st.installed;
        let query = st.search.to_lowercase();
        let shown: Vec<&plugins::Plugin> = st
            .catalogue
            .iter()
            .filter(|p| match st.filter {
                PluginFilter::All => true,
                PluginFilter::Bundles => p.is_bundle(),
                PluginFilter::Plugins => !p.is_bundle(),
                PluginFilter::Installed => installed.contains_key(p.id()),
            })
            .filter(|p| st.market_filter.as_ref().is_none_or(|m| &p.marketplace == m))
            .filter(|p| p.matches(&query))
            .collect();
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut st.search).hint_text("Filter by name, description, keyword, category, or author").desired_width(width - 120.0));
            ui.label(RichText::new(format!("{} plugins", shown.len())).color(TEXT_FAINT));
        });
        ui.add_space(8.0);
        let body_h = height - 130.0 - if st.show_log { 150.0 } else { 0.0 } - if st.show_sources { 70.0 + 30.0 * st.markets.len() as f32 + 50.0 } else { 0.0 };
        let body_h = body_h.max(120.0);
        ui.horizontal_top(|ui| {
            // The list's scroll bar sits right against the divider, and the bars here are solid
            // (always shown when there's more to scroll) rather than the app's hover-only floating ones.
            let gap = ui.spacing().item_spacing.x;
            ui.spacing_mut().item_spacing.x = 0.0;
            ui.spacing_mut().scroll = crate::theme::solid_scroll();
            // List (only the rows in view are drawn: Codex lists thousands of plugins).
            ui.allocate_ui_with_layout(Vec2::new(width * 0.38, body_h), Layout::top_down(Align::Min), |ui| {
                egui::ScrollArea::vertical().id_salt(("plugin_list", codex)).max_height(body_h).auto_shrink([false, false]).show_hinted(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    if loading && shown.is_empty() {
                        ui.horizontal(|ui| {
                            ui.add(egui::Spinner::new());
                            ui.label(RichText::new("Loading Codex plugins…").color(TEXT_DIM));
                        });
                        return;
                    }
                    if let Some(e) = &st.load_error {
                        ui.label(RichText::new(e).color(Color32::from_rgb(230, 110, 110)));
                        return;
                    }
                    const ROW_H: f32 = 34.0;
                    let top = ui.cursor().top();
                    let clip = ui.clip_rect();
                    let first = (((clip.top() - top) / ROW_H).floor().max(0.0) as usize).min(shown.len());
                    let last = ((((clip.bottom() - top) / ROW_H).ceil().max(0.0) as usize) + 1).clamp(first, shown.len());
                    if first > 0 {
                        ui.add_space(first as f32 * ROW_H);
                    }
                    for p in &shown[first..last] {
                        let id = p.id();
                        let sel = st.selected.as_deref() == Some(id);
                        let (r, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_H), Sense::click());
                        let painter = ui.painter();
                        if sel {
                            painter.rect_filled(r, 0.0, CARD_HI);
                            painter.rect_filled(egui::Rect::from_min_size(r.min, Vec2::new(3.0, r.height())), 0.0, ACCENT_HI);
                        } else if resp.hovered() {
                            painter.rect_filled(r, 0.0, CARD);
                        }
                        // Right side, right to left: installed dot, token estimate, BUNDLE badge.
                        let mut right = r.right() - 34.0;
                        if let Some(n) = (!codex).then(|| always_tokens(&mut st.costs, &st.cli_costs, installed, p)).flatten() {
                            let g = painter.layout_no_wrap(format!("~{} tok", crate::cost::fmt_tokens(n)), egui::FontId::proportional(12.0), TEXT_FAINT);
                            right -= g.size().x;
                            painter.galley(egui::pos2(right, r.center().y - g.size().y / 2.0), g, TEXT_FAINT);
                            right -= 10.0;
                        }
                        let badge_left = right - 66.0;
                        let mut job = egui::text::LayoutJob::simple_singleline(p.name.clone(), egui::FontId::proportional(14.0), TEXT);
                        job.wrap.max_width = (if p.is_bundle() { badge_left } else { right }) - r.left() - 22.0;
                        job.wrap.max_rows = 1;
                        job.wrap.break_anywhere = true;
                        let g = ui.fonts_mut(|f| f.layout_job(job));
                        let painter = ui.painter();
                        painter.galley(egui::pos2(r.left() + 14.0, r.center().y - g.size().y / 2.0), g, TEXT);
                        if p.is_bundle() {
                            let br = egui::Rect::from_min_size(egui::pos2(badge_left, r.center().y - 10.0), Vec2::new(66.0, 20.0));
                            painter.rect_filled(br, 3.0, BADGE_BLUE);
                            painter.text(br.center(), egui::Align2::CENTER_CENTER, "BUNDLE", egui::FontId::new(11.5, bold()), Color32::WHITE);
                        }
                        if let Some(inst) = installed.get(id) {
                            let on = inst.iter().any(|i| i.enabled);
                            let c = egui::pos2(r.right() - 18.0, r.center().y);
                            if on {
                                painter.circle_filled(c, 4.5, GREEN);
                            } else {
                                painter.circle_stroke(c, 4.0, Stroke::new(1.5, TEXT_DIM));
                            }
                        }
                        if resp.clicked() {
                            st.selected = Some(id.to_string());
                        }
                    }
                    if last < shown.len() {
                        ui.add_space((shown.len() - last) as f32 * ROW_H);
                    }
                });
            });
            let (sep, _) = ui.allocate_exact_size(Vec2::new(1.0, body_h), Sense::hover());
            ui.painter().rect_filled(sep, 0.0, BORDER);
            ui.add_space(16.0);
            // Details.
            ui.allocate_ui_with_layout(Vec2::new(ui.available_width(), body_h), Layout::top_down(Align::Min), |ui| {
                ui.spacing_mut().item_spacing.x = gap;
                egui::ScrollArea::vertical().id_salt("plugin_detail").max_height(body_h).auto_shrink([false, false]).show_hinted(ui, |ui| {
                    let Some(p) = st.selected.as_ref().and_then(|id| st.catalogue.iter().find(|p| p.id() == id)) else {
                        if !loading {
                            ui.label(RichText::new("Pick a plugin on the left.").color(TEXT_FAINT));
                        }
                        return;
                    };
                    ui.label(RichText::new(&p.name).family(bold()).size(24.0).color(Color32::WHITE));
                    ui.label(RichText::new(format!("Version {}", if p.version.is_empty() { "?" } else { &p.version })).size(15.0).color(TEXT));
                    ui.horizontal(|ui| {
                        if p.is_bundle() {
                            widgets::pill(ui, "BUNDLE", Color32::WHITE, BADGE_BLUE);
                        }
                        widgets::pill(ui, &p.marketplace, TEXT_DIM, CARD_HI);
                        for inst in installed.get(p.id()).into_iter().flatten() {
                            let text = if inst.scope.is_empty() { format!("installed · {}", inst.version) } else { format!("installed · {} · {}", inst.scope, inst.version) };
                            widgets::pill(ui, &text, Color32::WHITE, Color32::from_rgb(30, 90, 50));
                            if !inst.enabled {
                                widgets::pill(ui, "disabled", TEXT_DIM, CARD_HI);
                            }
                        }
                    });
                    let section = |ui: &mut Ui, t: &str| {
                        ui.add_space(14.0);
                        ui.label(RichText::new(t).size(12.0).family(bold()).color(TEXT_DIM));
                    };
                    section(ui, "DESCRIPTION");
                    if p.description.is_empty() {
                        ui.label(RichText::new("No description (the source doesn't publish one).").size(14.5).color(TEXT_FAINT));
                    } else {
                        ui.label(RichText::new(&p.description).size(14.5).color(TEXT));
                    }
                    if let Some(inst) = installed.get(p.id()).filter(|_| !codex) {
                        section(ui, "ENABLED");
                        let id = p.id().to_string();
                        let en = st.enablement.entry(id.clone()).or_insert_with(|| plugins::enablement(&id, inst, &st.projects)).clone();
                        let on_off = |ui: &mut Ui, on: bool| {
                            if on {
                                widgets::pill(ui, "ON", Color32::WHITE, Color32::from_rgb(30, 90, 50));
                            } else {
                                widgets::pill(ui, "OFF", TEXT_DIM, CARD_HI);
                            }
                        };
                        let toggle_args = |on: bool, scope: &str| vec!["plugin".into(), if on { "disable" } else { "enable" }.into(), id.clone(), "--scope".into(), scope.into()];
                        if en.user_install {
                            ui.horizontal(|ui| {
                                let on = en.user.unwrap_or(true);
                                on_off(ui, on);
                                ui.label(RichText::new("System wide").family(bold()).color(TEXT)).on_hover_text("~/.claude/settings.json: the default for every project");
                                let b = egui::Button::new(if on { "Turn off" } else { "Turn on" });
                                if ui.add_enabled(!running, b).on_hover_text(format!("{cli_name} plugin {} {id} --scope user", if on { "disable" } else { "enable" })).clicked() {
                                    st.toggle = Some((toggle_args(on, "user"), st.current_project.clone()));
                                }
                            });
                        } else {
                            ui.label(RichText::new("Not installed system wide: only in the projects below.").color(TEXT_DIM));
                        }
                        if en.projects.is_empty() {
                            ui.label(RichText::new("No projects to show.").color(TEXT_FAINT));
                        } else {
                            ui.add_space(6.0);
                            egui::Grid::new(("plugin_projects", &id)).num_columns(4).spacing([14.0, 6.0]).striped(true).show(ui, |ui| {
                                for ps in &en.projects {
                                    on_off(ui, ps.on);
                                    let name = ps.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| ps.path.display().to_string());
                                    let current = ps.path == st.current_project;
                                    let label = RichText::new(if current { format!("{name}  (current)") } else { name }).color(TEXT);
                                    ui.label(if current { label.family(bold()) } else { label }).on_hover_text(ps.path.display().to_string());
                                    let (from, tip) = match ps.from {
                                        plugins::Layer::Local => ("your setting for this project", ".claude/settings.local.json (personal, not shared)"),
                                        plugins::Layer::Project => ("project settings", ".claude/settings.json (shared with everyone on the project)"),
                                        plugins::Layer::User => ("system wide", "~/.claude/settings.json"),
                                        plugins::Layer::Default => ("default", "Not set anywhere; installed plugins are on"),
                                    };
                                    let from = if ps.installed_here { format!("{from}  ·  installed here") } else { from.to_string() };
                                    ui.label(RichText::new(from).color(TEXT_DIM)).on_hover_text(tip);
                                    ui.horizontal(|ui| {
                                        let b = egui::Button::new(if ps.on { "Turn off here" } else { "Turn on here" });
                                        let tip = format!("{cli_name} plugin {} {id} --scope local  (in {})", if ps.on { "disable" } else { "enable" }, ps.path.display());
                                        if ui.add_enabled(!running, b).on_hover_text(tip).clicked() {
                                            st.toggle = Some((toggle_args(ps.on, "local"), ps.path.clone()));
                                        }
                                        if ps.from == plugins::Layer::Local {
                                            let r = ui.add_enabled(!running, egui::Button::new("Reset")).on_hover_text("Remove your setting for this project, going back to the project / system-wide one");
                                            if r.clicked() {
                                                let mut job = st.job.lock().unwrap();
                                                match plugins::clear_local_override(&ps.path, &id) {
                                                    Ok(()) => job.log.push(format!("Cleared {id} from {}", ps.path.join(".claude").join("settings.local.json").display())),
                                                    Err(e) => job.log.push(format!("Couldn't reset {id} in {}: {e}", ps.path.display())),
                                                }
                                                st.enablement.remove(&id);
                                            }
                                        }
                                    });
                                    ui.end_row();
                                }
                            });
                        }
                    }
                    if !codex {
                        section(ui, "CONTEXT COST");
                        let ctx = ui.ctx().clone();
                        let cli = plugin_cli_cost(&mut st.cli_costs, &st.cli, &st.current_project, installed, p, &ctx);
                        match plugin_cost(&mut st.costs, installed, p) {
                            None => {
                                ui.label(RichText::new("Unknown: there's no local copy to measure until it's installed.").color(TEXT_FAINT));
                            }
                            Some(c) => {
                                let t = crate::cost::fmt_tokens;
                                let every_tip = "The names and descriptions of its skills, commands and agents, listed so the model knows when to use them.";
                                match &cli {
                                    Some(Ok(cc)) => {
                                        ui.label(RichText::new(format!("~{} tokens in every request", t(cc.always))).size(14.5).color(TEXT)).on_hover_text(every_tip);
                                        if cc.on_invoke() > 0 {
                                            ui.label(RichText::new(format!("~{} more when its skills, commands or agents are used (each time one fires)", t(cc.on_invoke()))).color(TEXT_DIM));
                                        }
                                        ui.label(RichText::new(format!("From Claude Code ({} plugin details).", cli_name)).size(12.0).color(TEXT_FAINT));
                                        if !cc.components.is_empty() {
                                            ui.add_space(4.0);
                                            egui::Grid::new(("plugin_components", p.id())).num_columns(3).spacing([24.0, 3.0]).striped(true).show(ui, |ui| {
                                                for h in ["Component", "Every request", "When used"] {
                                                    ui.label(RichText::new(h).size(12.0).family(bold()).color(TEXT_DIM));
                                                }
                                                ui.end_row();
                                                for (name, a, b) in &cc.components {
                                                    ui.label(RichText::new(name).color(TEXT));
                                                    ui.label(RichText::new(format!("~{}", t(*a))).color(TEXT));
                                                    ui.label(RichText::new(format!("~{}", t(*b))).color(TEXT_DIM));
                                                    ui.end_row();
                                                }
                                            });
                                        }
                                    }
                                    _ => {
                                        ui.label(RichText::new(format!("~{} tokens in every request", t(c.always))).size(14.5).color(TEXT)).on_hover_text(every_tip);
                                        if c.on_use > 0 {
                                            ui.label(RichText::new(format!("~{} more when its skills, commands or agents are used (full text, loaded on demand)", t(c.on_use))).color(TEXT_DIM));
                                        }
                                        match &cli {
                                            None => {
                                                ui.horizontal(|ui| {
                                                    ui.add(egui::Spinner::new().size(12.0));
                                                    ui.label(RichText::new("Estimated here; asking Claude Code for its figures…").size(12.0).color(TEXT_FAINT));
                                                });
                                            }
                                            Some(Err(e)) => {
                                                ui.label(RichText::new("Estimated from its files.").size(12.0).color(TEXT_FAINT)).on_hover_text(format!("{cli_name} plugin details: {e}"));
                                            }
                                            Some(Ok(_)) => {}
                                        }
                                    }
                                }
                                let parts: Vec<String> = match &cli {
                                    // Claude Code's inventory ("Skills", "LSP servers", …), singular for 1.
                                    Some(Ok(cc)) if !cc.inventory.is_empty() => cc
                                        .inventory
                                        .iter()
                                        .filter(|(_, n)| *n > 0)
                                        .map(|(kind, n)| {
                                            let kind = if kind.chars().nth(1).is_some_and(|ch| ch.is_lowercase()) { kind.to_lowercase() } else { kind.clone() };
                                            format!("{n} {}", if *n == 1 { kind.trim_end_matches('s') } else { &kind })
                                        })
                                        .collect(),
                                    _ => {
                                        let mut v: Vec<String> = [(c.skills, "skill"), (c.commands, "command"), (c.agents, "agent"), (c.mcp_servers, "MCP server")]
                                            .iter()
                                            .filter(|(n, _)| *n > 0)
                                            .map(|(n, w)| format!("{n} {w}{}", if *n == 1 { "" } else { "s" }))
                                            .collect();
                                        if c.hooks {
                                            v.push("hooks".into());
                                        }
                                        v
                                    }
                                };
                                if !parts.is_empty() {
                                    ui.label(RichText::new(parts.join("  ·  ")).color(TEXT_DIM));
                                }
                                if c.mcp_servers > 0 {
                                    ui.label(RichText::new("MCP tool definitions add more, sized only once the server runs (see the context pill in a session).").color(TEXT_FAINT));
                                }
                            }
                        }
                    }
                    if !p.author.is_empty() {
                        section(ui, "AUTHOR");
                        ui.label(RichText::new(&p.author).color(TEXT));
                    }
                    if !p.keywords.is_empty() {
                        section(ui, "KEYWORDS");
                        ui.horizontal_wrapped(|ui| {
                            for k in &p.keywords {
                                widgets::pill(ui, k, TEXT_DIM, CARD_HI);
                            }
                        });
                    }
                    if !p.category.is_empty() {
                        section(ui, "CATEGORY");
                        ui.label(RichText::new(&p.category).color(TEXT));
                    }
                    section(ui, "SOURCE");
                    // Web sources open in the browser; "owner/repo" is GitHub shorthand.
                    let link = |ui: &mut Ui, text: &str, url: Option<String>| match url {
                        Some(url) => {
                            let r = ui.add(egui::Label::new(RichText::new(text).color(LINK)).sense(egui::Sense::click())).on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(&url);
                            if r.hovered() {
                                ui.painter().hline(r.rect.x_range(), r.rect.bottom(), Stroke::new(1.0, LINK));
                            }
                            if r.clicked() {
                                crate::links::open_url(&url);
                            }
                        }
                        None => {
                            ui.label(RichText::new(text).color(TEXT));
                        }
                    };
                    let src = p.source.trim();
                    let src_url = if src.starts_with("http://") || src.starts_with("https://") {
                        Some(src.trim_end_matches(".git").to_string())
                    } else if !src.starts_with('.') && !src.contains(':') && !src.contains('\\') && !src.contains(' ') && src.matches('/').count() == 1 {
                        Some(format!("https://github.com/{src}"))
                    } else {
                        None
                    };
                    link(ui, &p.source, src_url);
                    if !p.source_ref.is_empty() {
                        ui.label(RichText::new(format!("ref {}", p.source_ref)).color(TEXT));
                    }
                    if !p.homepage.is_empty() {
                        link(ui, &p.homepage, Some(p.homepage.clone()));
                    }
                });
            });
        });
        if st.show_log {
            ui.add_space(6.0);
            egui::Frame::new().fill(TERM_LOG_BG).corner_radius(6).inner_margin(8).show(ui, |ui| {
                ui.set_width(ui.available_width());
                egui::ScrollArea::vertical().id_salt("plugin_log").max_height(130.0).stick_to_bottom(true).auto_shrink([false, false]).show_hinted(ui, |ui| {
                    for l in st.job.lock().unwrap().log.iter() {
                        ui.label(RichText::new(l).monospace().size(12.0).color(TEXT_DIM));
                    }
                });
            });
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.button(format!("Output log {}", if st.show_log { "▼" } else { "▲" })).clicked() {
                st.show_log = !st.show_log;
            }
            let status = if running {
                "Working…".to_string()
            } else if loading {
                "Loading…".to_string()
            } else if codex || installed.is_empty() {
                format!("{} plugins available.", st.catalogue.len())
            } else {
                // The installed plugins' combined per-request cost (Claude Code's figures, asked
                // for up front: there are only a few).
                let ctx = ui.ctx().clone();
                let total: u64 = st
                    .catalogue
                    .iter()
                    .filter(|p| installed.contains_key(p.id()))
                    .filter_map(|p| {
                        plugin_cli_cost(&mut st.cli_costs, &st.cli, &st.current_project, installed, p, &ctx);
                        always_tokens(&mut st.costs, &st.cli_costs, installed, p)
                    })
                    .sum();
                format!("{} plugins available.  Installed plugins add ~{} tokens to every request.", st.catalogue.len(), crate::cost::fmt_tokens(total))
            };
            ui.label(RichText::new(status).color(TEXT_DIM));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let Some(p) = st.selected.as_ref().and_then(|id| st.catalogue.iter().find(|p| p.id() == id)) else { return };
                let id = p.id().to_string();
                let inst: Vec<String> = installed.get(&id).map(|v| v.iter().map(|i| i.scope.clone()).collect()).unwrap_or_default();
                if codex {
                    // Codex has no install scopes.
                    let b = egui::Button::new(RichText::new("Install").color(Color32::WHITE).family(bold())).fill(ACCENT).min_size(Vec2::new(90.0, 30.0));
                    if ui.add_enabled(!running && !loading && inst.is_empty(), b).on_hover_text(format!("codex plugin add {id}")).clicked() {
                        *run = Some(vec!["plugin".into(), "add".into(), id.clone()]);
                    }
                    let b = egui::Button::new(RichText::new("Uninstall").family(bold())).fill(CARD_HI).min_size(Vec2::new(90.0, 30.0));
                    if ui.add_enabled(!running && !loading && !inst.is_empty(), b).on_hover_text(format!("codex plugin remove {id}")).clicked() {
                        *run = Some(vec!["plugin".into(), "remove".into(), id.clone()]);
                    }
                    return;
                }
                let scopes = [("user", "User"), ("project", "Project"), ("local", "Local")];
                // Install (scope via ▼).
                let label = format!("Install ({})", scopes.iter().find(|s| s.0 == st.scope).map(|s| s.1).unwrap_or("User"));
                ui.add_enabled_ui(!running, |ui| {
                    let (main, arrow) = widgets::split_button(ui, None, &label, ACCENT, 30.0);
                    if main.clicked() {
                        *run = Some(vec!["plugin".into(), "install".into(), id.clone(), "--scope".into(), st.scope.clone()]);
                    }
                    egui::Popup::menu(&arrow).show(|ui| {
                        for (k, l) in scopes {
                            if ui.radio(st.scope == k, format!("{l}{}", if k == "user" { "" } else { " (this project)" })).clicked() {
                                st.scope = k.to_string();
                                ui.close();
                            }
                        }
                    });
                });
                // Uninstall: from the scopes it's installed in.
                ui.add_enabled_ui(!running && !inst.is_empty(), |ui| {
                    let scope = inst.first().cloned().unwrap_or_else(|| "user".into());
                    let (main, arrow) = widgets::split_button(ui, None, &format!("Uninstall ({scope})"), CARD_HI, 30.0);
                    if main.clicked() {
                        *run = Some(vec!["plugin".into(), "uninstall".into(), id.clone(), "--scope".into(), scope.clone(), "--prune".into(), "-y".into()]);
                    }
                    egui::Popup::menu(&arrow).show(|ui| {
                        for sc in &inst {
                            if ui.button(format!("Uninstall from {sc}")).clicked() {
                                *run = Some(vec!["plugin".into(), "uninstall".into(), id.clone(), "--scope".into(), sc.clone(), "--prune".into(), "-y".into()]);
                                ui.close();
                            }
                        }
                    });
                });
            });
        });
    }
}
