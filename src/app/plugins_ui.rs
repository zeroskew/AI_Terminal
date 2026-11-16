//! The Claude Code plugins window.

use super::*;

impl App {
    // ---------------------------------------------------------------------------------------
    // Claude Code plugins

    pub(super) fn open_plugins(&mut self) {
        let mut p = PluginsUi { catalogue: plugins::catalogue(), installed: plugins::installed(), markets: plugins::markets(), scope: "user".into(), ..Default::default() };
        p.selected = p.catalogue.first().map(|x| x.id());
        self.plugins = Some(p);
    }

    /// How to run the claude CLI: (program, leading args).
    pub(super) fn claude_cli(&self) -> (String, Vec<String>) {
        let p = self.profiles.iter().find(|p| p.is_claude());
        (p.map(|p| p.program.clone()).unwrap_or_else(|| "claude".into()), p.map(|p| p.args.clone()).unwrap_or_default())
    }

    /// The plugins window (a modal).
    pub(super) fn plugins_window(&mut self, ctx: &egui::Context) {
        let Some(st) = &mut self.plugins else { return };
        // Reload the installed state after a job finishes.
        let finished = {
            let mut j = st.job.lock().unwrap();
            j.last_ok.take()
        };
        if finished.is_some() {
            if st.refreshing {
                st.catalogue = plugins::catalogue();
                st.markets = plugins::markets();
                if st.market_filter.as_ref().is_some_and(|m| !st.markets.iter().any(|x| &x.name == m)) {
                    st.market_filter = None;
                }
                st.refreshing = false;
            }
            st.installed = plugins::installed();
        }
        let screen = ctx.content_rect();
        let size = Vec2::new((screen.width() * 0.85).min(1300.0), (screen.height() * 0.85).min(900.0));
        let project = self.s.selected_project.clone().unwrap_or_else(|| self.s.projects[0].path.clone());
        let (cli, cli_args) = self.claude_cli();
        let mut close = false;
        let mut run: Option<Vec<String>> = None;
        let mut refresh = false;
        let mut source_cmd: Option<Vec<String>> = None;
        let st = self.plugins.as_mut().unwrap();
        let running = st.job.lock().unwrap().running;
        let m = egui::Modal::new(egui::Id::unique("plugins")).frame(egui::Frame::new().fill(PANEL).corner_radius(10).stroke(Stroke::new(1.0, BORDER)).inner_margin(18)).show(ctx, |ui| {
            ui.set_width(size.x);
            ui.set_height(size.y);
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(Vec2::splat(20.0), Sense::hover());
                widgets::paint_icon(ui.painter(), Icon::Sparkle, r.center(), 15.0, Color32::from_rgb(217, 119, 87));
                ui.label(RichText::new("CLAUDE CODE PLUGINS").size(12.5).family(bold()).color(TEXT_DIM));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if widgets::icon_button(ui, Icon::Close, 26.0, TEXT_DIM, "Close (Esc)").clicked() {
                        close = true;
                    }
                    if ui.add_enabled(!running, egui::Button::new("⟳ Refresh")).on_hover_text("claude plugin marketplace update").clicked() {
                        refresh = true;
                    }
                });
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                for (f, label) in [(PluginFilter::All, "All"), (PluginFilter::Bundles, "Bundles"), (PluginFilter::Plugins, "Plugins"), (PluginFilter::Installed, "Installed")] {
                    let sel = st.filter == f;
                    let b = egui::Button::new(RichText::new(label).color(if sel { Color32::WHITE } else { TEXT })).fill(if sel { ACCENT } else { CARD_HI }).corner_radius(14).min_size(Vec2::new(70.0, 28.0));
                    if ui.add(b).clicked() {
                        st.filter = f;
                    }
                }
                ui.add_space(16.0);
                ui.label(RichText::new("Source").color(TEXT_DIM));
                let shown = st.market_filter.clone().unwrap_or_else(|| "All sources".into());
                egui::ComboBox::from_id_salt("plugin_source").selected_text(shown).width(260.0).show_ui(ui, |ui| {
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
                                if ui.add_enabled(!running, egui::Button::new("Update")).on_hover_text(format!("claude plugin marketplace update {}", m.name)).clicked() {
                                    source_cmd = Some(vec!["plugin".into(), "marketplace".into(), "update".into(), m.name.clone()]);
                                }
                                let confirming = st.confirm_remove.as_deref() == Some(m.name.as_str());
                                let b = if confirming { egui::Button::new(RichText::new("Confirm remove").color(Color32::WHITE)).fill(Color32::from_rgb(150, 40, 40)) } else { egui::Button::new("Remove") };
                                let tip = "Removes the source from Claude Code (claude plugin marketplace remove). Plugins installed from it may be removed too.";
                                let builtin = m.name == plugins::DIRECTORY;
                                let tip = if builtin { "Built into Claude Code; it can't be removed." } else { tip };
                                if ui.add_enabled(!running && !builtin, b).on_hover_text(tip).on_disabled_hover_text(tip).clicked() {
                                    if confirming {
                                        source_cmd = Some(vec!["plugin".into(), "marketplace".into(), "remove".into(), m.name.clone()]);
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
                        let te = ui.add(egui::TextEdit::singleline(&mut st.new_source).hint_text("git URL, GitHub owner/repo, or a local folder").desired_width(520.0));
                        let ok = !st.new_source.trim().is_empty() && !running;
                        let add = ui.add_enabled(ok, egui::Button::new(RichText::new("Add").color(Color32::WHITE)).fill(ACCENT)).on_hover_text("claude plugin marketplace add <source>").clicked()
                            || (ok && te.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)));
                        if add {
                            source_cmd = Some(vec!["plugin".into(), "marketplace".into(), "add".into(), st.new_source.trim().to_string()]);
                            st.new_source.clear();
                        }
                    });
                });
            }
            ui.add_space(6.0);
            let installed = &st.installed;
            let shown: Vec<&plugins::Plugin> = st
                .catalogue
                .iter()
                .filter(|p| match st.filter {
                    PluginFilter::All => true,
                    PluginFilter::Bundles => p.is_bundle(),
                    PluginFilter::Plugins => !p.is_bundle(),
                    PluginFilter::Installed => installed.contains_key(&p.id()),
                })
                .filter(|p| st.market_filter.as_ref().is_none_or(|m| &p.marketplace == m))
                .filter(|p| p.matches(&st.search))
                .collect();
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut st.search).hint_text("Filter by name, description, keyword, category, or author").desired_width(size.x - 120.0));
                ui.label(RichText::new(format!("{} plugins", shown.len())).color(TEXT_FAINT));
            });
            ui.add_space(8.0);
            let body_h = size.y - 170.0 - if st.show_log { 150.0 } else { 0.0 } - if st.show_sources { 70.0 + 30.0 * st.markets.len() as f32 + 50.0 } else { 0.0 };
            ui.horizontal_top(|ui| {
                // List.
                ui.allocate_ui_with_layout(Vec2::new(size.x * 0.38, body_h), Layout::top_down(Align::Min), |ui| {
                    egui::ScrollArea::vertical().id_salt("plugin_list").max_height(body_h).auto_shrink([false, false]).show_hinted(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        for p in &shown {
                            let id = p.id();
                            let sel = st.selected.as_deref() == Some(id.as_str());
                            let (r, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 34.0), Sense::click());
                            let painter = ui.painter();
                            if sel {
                                painter.rect_filled(r, 0.0, CARD_HI);
                                painter.rect_filled(egui::Rect::from_min_size(r.min, Vec2::new(3.0, r.height())), 0.0, ACCENT_HI);
                            } else if resp.hovered() {
                                painter.rect_filled(r, 0.0, CARD);
                            }
                            let mut job = egui::text::LayoutJob::simple_singleline(p.name.clone(), egui::FontId::proportional(14.0), TEXT);
                            job.wrap.max_width = r.width() - 130.0;
                            job.wrap.max_rows = 1;
                            job.wrap.break_anywhere = true;
                            let g = ui.fonts_mut(|f| f.layout_job(job));
                            painter.galley(egui::pos2(r.left() + 14.0, r.center().y - g.size().y / 2.0), g, TEXT);
                            if p.is_bundle() {
                                let br = egui::Rect::from_min_size(egui::pos2(r.right() - 100.0, r.center().y - 10.0), Vec2::new(66.0, 20.0));
                                painter.rect_filled(br, 3.0, BADGE_BLUE);
                                painter.text(br.center(), egui::Align2::CENTER_CENTER, "BUNDLE", egui::FontId::new(11.5, bold()), Color32::WHITE);
                            }
                            if installed.contains_key(&id) {
                                painter.circle_filled(egui::pos2(r.right() - 18.0, r.center().y), 4.5, GREEN);
                            }
                            if resp.clicked() {
                                st.selected = Some(id);
                            }
                        }
                    });
                });
                let (sep, _) = ui.allocate_exact_size(Vec2::new(1.0, body_h), Sense::hover());
                ui.painter().rect_filled(sep, 0.0, BORDER);
                ui.add_space(16.0);
                // Details.
                ui.allocate_ui_with_layout(Vec2::new(ui.available_width(), body_h), Layout::top_down(Align::Min), |ui| {
                    egui::ScrollArea::vertical().id_salt("plugin_detail").max_height(body_h).auto_shrink([false, false]).show_hinted(ui, |ui| {
                        let Some(p) = st.selected.as_ref().and_then(|id| st.catalogue.iter().find(|p| &p.id() == id)) else {
                            ui.label(RichText::new("Pick a plugin on the left.").color(TEXT_FAINT));
                            return;
                        };
                        ui.label(RichText::new(&p.name).family(bold()).size(24.0).color(Color32::WHITE));
                        ui.label(RichText::new(format!("Version {}", if p.version.is_empty() { "?" } else { &p.version })).size(15.0).color(TEXT));
                        ui.horizontal(|ui| {
                            if p.is_bundle() {
                                widgets::pill(ui, "BUNDLE", Color32::WHITE, BADGE_BLUE);
                            }
                            widgets::pill(ui, &p.marketplace, TEXT_DIM, CARD_HI);
                            for inst in installed.get(&p.id()).into_iter().flatten() {
                                widgets::pill(ui, &format!("installed · {} · {}", inst.scope, inst.version), Color32::WHITE, Color32::from_rgb(30, 90, 50));
                            }
                        });
                        let section = |ui: &mut Ui, t: &str| {
                            ui.add_space(14.0);
                            ui.label(RichText::new(t).size(12.0).family(bold()).color(TEXT_DIM));
                        };
                        section(ui, "DESCRIPTION");
                        ui.label(RichText::new(&p.description).size(14.5).color(TEXT));
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
                        } else if !src.starts_with('.') && !src.contains(':') && !src.contains('\\') && src.matches('/').count() == 1 {
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
                let status = if running { "Working…".to_string() } else { format!("{} plugins available.", st.catalogue.len()) };
                ui.label(RichText::new(status).color(TEXT_DIM));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let Some(p) = st.selected.as_ref().and_then(|id| st.catalogue.iter().find(|p| &p.id() == id)) else { return };
                    let id = p.id();
                    let scopes = [("user", "User"), ("project", "Project"), ("local", "Local")];
                    // Install (scope via ▼).
                    let label = format!("Install ({})", scopes.iter().find(|s| s.0 == st.scope).map(|s| s.1).unwrap_or("User"));
                    ui.add_enabled_ui(!running, |ui| {
                        let (main, arrow) = widgets::split_button(ui, None, &label, ACCENT, 30.0);
                        if main.clicked() {
                            run = Some(vec!["plugin".into(), "install".into(), id.clone(), "--scope".into(), st.scope.clone()]);
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
                    let inst: Vec<String> = installed.get(&id).map(|v| v.iter().map(|i| i.scope.clone()).collect()).unwrap_or_default();
                    ui.add_enabled_ui(!running && !inst.is_empty(), |ui| {
                        let scope = inst.first().cloned().unwrap_or_else(|| "user".into());
                        let (main, arrow) = widgets::split_button(ui, None, &format!("Uninstall ({scope})"), CARD_HI, 30.0);
                        if main.clicked() {
                            run = Some(vec!["plugin".into(), "uninstall".into(), id.clone(), "--scope".into(), scope.clone(), "--prune".into(), "-y".into()]);
                        }
                        egui::Popup::menu(&arrow).show(|ui| {
                            for sc in &inst {
                                if ui.button(format!("Uninstall from {sc}")).clicked() {
                                    run = Some(vec!["plugin".into(), "uninstall".into(), id.clone(), "--scope".into(), sc.clone(), "--prune".into(), "-y".into()]);
                                    ui.close();
                                }
                            }
                        });
                    });
                });
            });
        });
        if m.should_close() {
            close = true;
        }
        let st = self.plugins.as_mut().unwrap();
        if refresh {
            st.refreshing = true;
            st.show_log = true;
            plugins::run(&st.job, &cli, &cli_args, vec!["plugin".into(), "marketplace".into(), "update".into()], &project, ctx.clone());
        }
        if let Some(args) = run {
            st.show_log = true;
            plugins::run(&st.job, &cli, &cli_args, args, &project, ctx.clone());
        }
        if let Some(args) = source_cmd {
            // Reloads the sources and the catalogue when it finishes.
            st.refreshing = true;
            st.show_log = true;
            plugins::run(&st.job, &cli, &cli_args, args, &project, ctx.clone());
        }
        if close {
            self.plugins = None;
            self.focus_terminal = true;
        }
    }
}
