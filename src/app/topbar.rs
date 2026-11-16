//! The project tabs bar and the session header.

use super::*;

impl App {
    // ---------------------------------------------------------------------------------------
    // UI

    /// Top bar: Home + one tab per project, "Add Project" on the right.
    pub(super) fn topbar(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let bar = ui.max_rect();
        let mut select: Option<Option<PathBuf>> = None;
        let mut remove: Option<PathBuf> = None;
        let mut hide: Option<PathBuf> = None;
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            // Home.
            let home_sel = self.s.selected_project.is_none();
            let (r, resp) = ui.allocate_exact_size(Vec2::new(40.0, bar.height()), Sense::click());
            if resp.hovered() {
                ui.painter().rect_filled(r.shrink2(Vec2::new(2.0, 6.0)), 5.0, CARD);
            }
            widgets::paint_icon(ui.painter(), Icon::Home, r.center(), 16.0, if home_sel || resp.hovered() { TEXT } else { TEXT_DIM });
            if home_sel {
                ui.painter().line_segment([egui::pos2(r.left() + 6.0, bar.bottom() - 1.0), egui::pos2(r.right() - 6.0, bar.bottom() - 1.0)], Stroke::new(2.0, ACCENT_HI));
            }
            if resp.on_hover_text("Home: all projects").clicked() {
                select = Some(None);
            }
            ui.painter().line_segment([egui::pos2(r.right(), bar.top() + 8.0), egui::pos2(r.right(), bar.bottom() - 8.0)], Stroke::new(1.0, BORDER));

            // Project tabs, placed by hand on the full bar height (so they line up with Home);
            // the ones that don't fit go into a "»" menu.
            let right_w = 170.0;
            let x_end = bar.right() - right_w;
            let mut x = r.right() + 4.0;
            let mut overflow: Vec<Project> = Vec::new();
            let mut tab_rects: Vec<(PathBuf, egui::Rect)> = Vec::new();
            for proj in self.s.projects.clone().into_iter().filter(|p| p.in_tabs) {
                let sel = self.s.selected_project.as_ref() == Some(&proj.path);
                let ids = self.project_sessions(&proj.path);
                let unseen = self.tabs.iter().any(|t| t.project == proj.path && t.sess.unseen);
                let attention = self.tabs.iter().any(|t| t.project == proj.path && t.sess.attention.is_some());
                let dot = if attention {
                    Some(ACCENT_HI)
                } else if unseen {
                    Some(ORANGE)
                } else if !ids.is_empty() {
                    Some(GREEN.gamma_multiply(0.8))
                } else if self.project_session_count(&proj.path) > 0 {
                    Some(TEXT_FAINT)
                } else {
                    None
                };
                let font = egui::FontId::new(13.5, if sel { bold() } else { egui::FontFamily::Proportional });
                let g = ui.painter().layout_no_wrap(proj.name.clone(), font, TEXT);
                let dot_w = if dot.is_some() { 14.0 } else { 0.0 };
                let close_w = 18.0;
                let w = g.size().x + dot_w + 32.0 + close_w;
                if x + w > x_end - 30.0 && !sel {
                    overflow.push(proj);
                    continue;
                }
                let tr = egui::Rect::from_min_max(egui::pos2(x, bar.top()), egui::pos2(x + w, bar.bottom()));
                x += w;
                let resp = ui.interact(tr, egui::Id::unique(("project_tab", &proj.path)), Sense::click_and_drag());
                tab_rects.push((proj.path.clone(), tr));
                if resp.drag_started() {
                    self.drag_project = Some(proj.path.clone());
                }
                let p = ui.painter();
                if resp.hovered() && !sel {
                    p.rect_filled(tr.shrink2(Vec2::new(2.0, 6.0)), 5.0, CARD);
                }
                let mut tx = tr.left() + 16.0;
                if let Some(c) = dot {
                    p.circle_filled(egui::pos2(tx + 3.0, tr.center().y), 3.5, c);
                    tx += dot_w;
                }
                let tc = if sel || resp.hovered() { TEXT } else { TEXT_DIM };
                p.galley_with_override_text_color(egui::pos2(tx, tr.center().y - g.size().y / 2.0), g, tc);
                // × (on hover / selected): closes the tab; the project stays on Home.
                let xr = egui::Rect::from_center_size(egui::pos2(tr.right() - 8.0 - close_w / 2.0, tr.center().y), Vec2::splat(16.0));
                let xresp = ui.interact(xr, egui::Id::unique(("project_tab_close", &proj.path)), Sense::click());
                if resp.hovered() || xresp.hovered() || sel {
                    let p = ui.painter();
                    if xresp.hovered() {
                        p.rect_filled(xr, 3.0, CARD_HI);
                    }
                    let (c, k) = (xr.center(), 3.5);
                    let st = Stroke::new(1.3, if xresp.hovered() { TEXT } else { TEXT_FAINT });
                    p.line_segment([c + Vec2::new(-k, -k), c + Vec2::new(k, k)], st);
                    p.line_segment([c + Vec2::new(-k, k), c + Vec2::new(k, -k)], st);
                }
                if xresp.on_hover_text("Close tab (the project stays on Home)").clicked() || resp.middle_clicked() {
                    hide = Some(proj.path.clone());
                }
                if sel {
                    p.line_segment([egui::pos2(tr.left() + 8.0, bar.bottom() - 1.0), egui::pos2(tr.right() - 8.0, bar.bottom() - 1.0)], Stroke::new(2.0, ACCENT_HI));
                }
                if resp.clicked() {
                    select = Some(Some(proj.path.clone()));
                }
                let n = self.project_session_count(&proj.path);
                let cost_line = if !self.s.show_cost {
                    String::new()
                } else if self.s.usage_as == "plan" {
                    let ids = self.tabs.iter().filter(|t| t.project == proj.path).filter_map(|t| t.sess.agent_id.clone()).chain(self.dormant.iter().filter(|d| d.saved.project == proj.path).filter_map(|d| d.saved.agent_id.clone()));
                    let (mut h5, mut wk) = (0.0, 0.0);
                    for id in ids {
                        if let Some(p) = self.plan_usage.get(&id) {
                            h5 += p.five_hour;
                            wk += p.seven_day;
                        }
                    }
                    if h5 > 0.0 || wk > 0.0 { format!("\nPlan usage: {} of a 5-hour limit, {} of the weekly", fmt_plan(h5), fmt_plan(wk)) } else { String::new() }
                } else {
                    let total: f64 = self.tabs.iter().filter(|t| t.project == proj.path).filter_map(|t| t.sess.cost.map(|c| c.usd)).sum::<f64>() + self.dormant.iter().filter(|d| d.saved.project == proj.path).filter_map(|d| d.saved.cost).sum::<f64>();
                    if total > 0.0 { format!("\nEstimated cost {}", crate::cost::fmt_usd(total)) } else { String::new() }
                };
                resp.on_hover_text(format!("{}\n{n} session{}{cost_line}", proj.path.display(), if n == 1 { "" } else { "s" })).context_menu(|ui| {
                    if ui.button("Rename").clicked() {
                        self.renaming_project = Some((proj.path.clone(), proj.name.clone()));
                        ui.close();
                    }
                    if ui.button("Open in Explorer").clicked() {
                        let _ = std::process::Command::new("explorer.exe").arg(&proj.path).spawn();
                        ui.close();
                    }
                    ui.separator();
                    if ui.button("Close tab (stays on Home)").clicked() {
                        hide = Some(proj.path.clone());
                        ui.close();
                    }
                    if ui.button("Remove project…").clicked() {
                        remove = Some(proj.path.clone());
                        ui.close();
                    }
                });
            }
            // Dragging a project tab: a marker where it will go; dropping reorders the projects.
            if let Some(dragged) = self.drag_project.clone() {
                let pos = ui.input(|i| i.pointer.interact_pos());
                let released = ui.input(|i| i.pointer.any_released());
                if let (Some(pos), false) = (pos, tab_rects.is_empty()) {
                    let slot = tab_rects.iter().position(|(_, r)| pos.x < r.center().x).unwrap_or(tab_rects.len());
                    let xm = if slot < tab_rects.len() { tab_rects[slot].1.left() } else { tab_rects.last().unwrap().1.right() };
                    ui.painter().line_segment([egui::pos2(xm, bar.top() + 8.0), egui::pos2(xm, bar.bottom() - 8.0)], Stroke::new(2.0, ACCENT_HI));
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                    if released {
                        let target: Option<PathBuf> = tab_rects.get(slot).map(|(p, _)| p.clone());
                        if let Some(from) = self.s.projects.iter().position(|p| p.path == dragged) {
                            let item = self.s.projects.remove(from);
                            let to = match target {
                                Some(t) => self.s.projects.iter().position(|p| p.path == t).unwrap_or(self.s.projects.len()),
                                None => tab_rects.last().and_then(|(p, _)| self.s.projects.iter().position(|x| &x.path == p)).map(|i| i + 1).unwrap_or(self.s.projects.len()),
                            };
                            self.s.projects.insert(to.min(self.s.projects.len()), item);
                        }
                    }
                }
                if released {
                    self.drag_project = None;
                }
            }
            if !overflow.is_empty() {
                let or = egui::Rect::from_min_size(egui::pos2(x + 2.0, bar.center().y - 14.0), Vec2::new(28.0, 28.0));
                let resp = ui.interact(or, egui::Id::unique("project_overflow"), Sense::click());
                if resp.hovered() {
                    ui.painter().rect_filled(or, 5.0, CARD);
                }
                ui.painter().text(or.center(), egui::Align2::CENTER_CENTER, "»", egui::FontId::new(16.0, bold()), TEXT_DIM);
                let resp = resp.on_hover_text(format!("{} more project{}", overflow.len(), if overflow.len() == 1 { "" } else { "s" }));
                egui::Popup::menu(&resp).show(|ui| {
                    for proj in &overflow {
                        if ui.button(&proj.name).clicked() {
                            select = Some(Some(proj.path.clone()));
                            ui.close();
                        }
                    }
                });
            }

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(8.0);
                let (main, arrow) = widgets::split_button(ui, None, "Add Project", ACCENT, 28.0);
                if main.on_hover_text("Add a project folder").clicked() {
                    self.add_project(&ctx);
                }
                egui::Popup::menu(&arrow).show(|ui| {
                    if ui.button("Add project folder…").clicked() {
                        ui.close();
                        self.add_project(&ctx);
                    }
                    if let Some(p) = self.s.selected_project.clone() {
                        if ui.button("Rename this project").clicked() {
                            let name = self.s.projects.iter().find(|x| x.path == p).map(|x| x.name.clone()).unwrap_or_default();
                            self.renaming_project = Some((p.clone(), name));
                            ui.close();
                        }
                        if ui.button("Close this tab (stays on Home)").clicked() {
                            hide = Some(p.clone());
                            ui.close();
                        }
                        if ui.button("Remove this project…").clicked() {
                            remove = Some(p);
                            ui.close();
                        }
                    }
                });
            });
        });
        if let Some(sel) = select {
            self.select_project(sel);
        }
        if let Some(p) = hide {
            self.hide_project_tab(&p);
        }
        if let Some(p) = remove {
            self.confirm_remove_project = Some(p);
        }
        self.rename_project_dialog(&ctx);
    }

    /// Header above the terminal: session name, shell, folder, and the pasted images strip.
    pub(super) fn header(&mut self, ui: &mut Ui) {
        let Some(tab) = self.active_tab() else { return };
        let name = tab.sess.display_name();
        let profile = tab.sess.profile.name.clone();
        let cwd = tab.sess.cwd.display().to_string();
        let exited = tab.sess.exited;
        let list = tab.sess.pasted_images.clone();
        let id = tab.sess.id;
        let resumable = tab.sess.agent_id.is_some();
        let unrestricted_pill = tab.sess.opts.unrestricted;
        let busy_pill = tab.sess.busy && tab.sess.exited.is_none();
        let cost_pill = tab.sess.cost.filter(|_| self.s.show_cost);
        let plan_pill = if self.s.usage_as == "plan" { Some(tab.sess.agent_id.as_ref().and_then(|a| self.plan_usage.get(a)).copied()) } else { None };
        let attention_pill = tab.sess.attention.clone();
        // The model actually answering (from the transcript) goes in the profile pill; the
        // launch options pill then only adds the effort.
        let live_model = tab.sess.model.clone();
        let model_pill = if live_model.is_some() {
            tab.sess.opts.effort.clone().map(|e| format!("effort {e}"))
        } else {
            tab.sess.opts.model.clone().map(|m| match &tab.sess.opts.effort {
                Some(e) => format!("{m} · {e}"),
                None => m,
            })
        };
        let ctx = ui.ctx().clone();
        let mut open: Option<PathBuf> = None;
        let mut restart = false;
        let mut compact = false;
        let mut change_model = false;
        // Claude and Codex both have /model and /compact.
        let is_codex = tab.sess.profile.kind == Kind::Codex;
        let can_compact = exited.is_none() && (tab.sess.profile.kind == Kind::Claude || is_codex);
        ui.horizontal_centered(|ui| {
            let name_for_pill = name.clone();
            ui.label(RichText::new(name).family(bold()).size(14.0).color(TEXT));
            if let Some(m) = &live_model {
                widgets::pill(ui, &format!("{profile} · {}", crate::cost::model_label(m)), TEXT_DIM, CARD_HI).on_hover_text(format!("Model of the latest reply: {m}"));
            } else if profile != name_for_pill {
                widgets::pill(ui, &profile, TEXT_DIM, CARD_HI);
            }
            if can_compact && super::status_bar::link_button(ui, "change", if is_codex { "Switch this session to another model or reasoning effort (opens Codex's /model picker)" } else { "Switch this session to another model (sends /model)" }) {
                change_model = true;
            }
            if busy_pill {
                widgets::pill(ui, "working…", Color32::WHITE, Color32::from_rgb(28, 92, 160));
            } else if let Some(m) = &attention_pill {
                widgets::pill(ui, if m.to_lowercase().contains("permission") { "needs permission" } else { "needs you" }, Color32::WHITE, ACCENT).on_hover_text(m);
            }
            if let Some(c) = cost_pill {
                let u = c.usage;
                // Plan mode: the share of the plan limits (once Claude has reported one); the
                // tokens either way.
                let (head, about) = match plan_pill {
                    Some(Some(p)) => (
                        format!("5h {} · week {} · ", fmt_plan(p.five_hour), fmt_plan(p.seven_day)),
                        format!(
                            "Plan usage of this conversation: {} of a 5-hour limit, {} of the weekly limit.\nCounted from Claude Code's plan usage after each reply; approximate while other sessions work at the same time.\nEstimated API cost (not charged on a subscription): {}\n\nTokens, from the transcript:",
                            fmt_plan(p.five_hour),
                            fmt_plan(p.seven_day),
                            crate::cost::fmt_usd(c.usd)
                        ),
                    ),
                    Some(None) => (String::new(), format!("No plan usage counted for this conversation yet (it updates after Claude's next reply).\nEstimated API cost (not charged on a subscription): {}\n\nTokens, from the transcript:", crate::cost::fmt_usd(c.usd))),
                    None => (format!("{} · ", crate::cost::fmt_usd(c.usd)), "Estimated from the transcript at API prices".to_string()),
                };
                let ctx = if c.context > 0 { format!("context {} · ", crate::cost::fmt_tokens(c.context)) } else { String::new() };
                widgets::pill(ui, &format!("{ctx}{head}{} tokens", crate::cost::fmt_tokens(u.total())), TEXT_DIM, CARD_HI).on_hover_text(format!(
                    "Context: {} tokens sent with every request (system prompt, tools, memory and the conversation so far), from the latest request. Run /compact to shrink it.\n\n{about}\nInput {}  ·  output {}\nCache read {}  ·  cache write {}{}",
                    crate::cost::fmt_tokens(c.context),
                    crate::cost::fmt_tokens(u.input),
                    crate::cost::fmt_tokens(u.output),
                    crate::cost::fmt_tokens(u.cache_read),
                    crate::cost::fmt_tokens(u.cache_write),
                    if u.web_searches > 0 { format!("\nWeb searches {}", u.web_searches) } else { String::new() }
                ));
                if can_compact && c.context > 0 && super::status_bar::link_button(ui, "compact", "Run /compact: replace the conversation so far with a summary, so each request sends less context") {
                    compact = true;
                }
            }
            if unrestricted_pill {
                widgets::pill(ui, "unrestricted", Color32::WHITE, Color32::from_rgb(150, 80, 20)).on_hover_text("Permission prompts are skipped");
            }
            if let Some(m) = &model_pill {
                widgets::pill(ui, m, TEXT_DIM, CARD_HI);
            }
            ui.label(RichText::new(cwd).size(12.0).color(TEXT_FAINT));
            if let Some(code) = exited {
                widgets::pill(ui, &format!("exited ({code})"), Color32::WHITE, Color32::from_rgb(130, 35, 35));
                let (label, tip) = if resumable { ("Resume", "Start Claude again and continue this conversation") } else { ("Restart", "Start it again in the same folder") };
                if widgets::primary_button(ui, Icon::Restart, label, ACCENT, tip).clicked() {
                    restart = true;
                }
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button("Scripts").on_hover_text("Run a script from the scripts folder in this project, as its own session (Ctrl+Shift+R)").clicked() {
                    self.open_scripts();
                }
                if ui.button("Snippets").on_hover_text("Insert a saved prompt (Ctrl+Shift+S)").clicked() {
                    self.snippets_open = true;
                    self.snippet_focus = true;
                }
                ui.add_space(6.0);
                if !self.s.show_image_strip || list.is_empty() {
                    return;
                }
                // Newest first, at most 12.
                let n = list.len();
                for (i, path) in list.iter().enumerate().rev().take(12) {
                    let h = 30.0;
                    let w = match self.cache.get(&ctx, path) {
                        Slot::Ready { size, .. } => (h * size[0] as f32 / size[1].max(1) as f32).clamp(h * 0.6, h * 2.2),
                        _ => h,
                    };
                    let tex = self.cache.texture_for(&ctx, path, Vec2::new(w - 2.0, h - 2.0) * ctx.pixels_per_point());
                    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, h), Sense::click());
                    let p = ui.painter();
                    p.rect_filled(rect, 4.0, CARD);
                    match tex {
                        Some(t) => {
                            p.image(t, rect.shrink(1.0), egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
                        }
                        None => widgets::paint_icon(p, Icon::Image, rect.center(), 14.0, TEXT_FAINT),
                    }
                    p.rect_stroke(rect, 4.0, Stroke::new(1.0, if resp.hovered() { ACCENT_HI } else { BORDER }), egui::StrokeKind::Inside);
                    if resp.hovered() {
                        view::hover_preview(ui, &mut self.cache, path, &format!("Pasted image {} of {n}", i + 1), "Click to open");
                    }
                    if resp.clicked() {
                        open = Some(path.clone());
                    }
                }
                ui.label(RichText::new("Pasted").size(11.5).color(TEXT_FAINT));
            });
        });
        if let Some(p) = open {
            self.viewer = Some(Viewer { path: p, list, zoom_fit: true });
        }
        if restart {
            self.restart(&ctx, id);
        }
        if compact {
            self.send_command_to(id, "/compact");
        }
        if change_model {
            if is_codex {
                // Codex has its own model and reasoning-effort picker.
                self.send_command_to(id, "/model");
            } else {
                self.model_switch = Some(id);
            }
        }
    }
}
