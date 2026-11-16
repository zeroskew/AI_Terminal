//! The project / session sidebar.

use super::*;

impl App {
    /// Running sessions of a project.
    pub(super) fn project_sessions(&self, p: &Path) -> Vec<u64> {
        self.tabs.iter().filter(|t| t.project == p).map(|t| t.sess.id).collect()
    }

    /// Running + listed (not running, not archived) sessions of a project.
    pub(super) fn project_session_count(&self, p: &Path) -> usize {
        self.project_sessions(p).len() + self.dormant.iter().filter(|d| d.saved.project == p && !d.saved.archived).count()
    }

    /// "Keep the last N sessions": per project, listed sessions past the N most recently used
    /// move to Archived. Running and pinned sessions are never archived (running ones count
    /// towards N).
    pub(super) fn auto_archive(&mut self) {
        let keep = self.s.keep_sessions as usize;
        if keep == 0 {
            return;
        }
        for p in self.s.projects.iter().map(|p| p.path.clone()).collect::<Vec<_>>() {
            let running = self.tabs.iter().filter(|t| t.project == p && t.pin == 0).count();
            let mut listed: Vec<(u64, usize)> = self
                .dormant
                .iter()
                .enumerate()
                .filter(|(_, d)| d.saved.project == p && !d.saved.archived && d.saved.pin == 0)
                .map(|(i, d)| (d.saved.used.max(d.saved.created), i))
                .collect();
            if running + listed.len() <= keep {
                continue;
            }
            listed.sort_by(|a, b| b.cmp(a));
            for &(_, i) in listed.iter().skip(keep.saturating_sub(running)) {
                self.dormant[i].saved.archived = true;
            }
        }
    }

    /// Moves a session to Archived (a running one is closed first; it stays resumable).
    pub(super) fn archive(&mut self, id: u64) {
        if self.tabs.iter().any(|t| t.sess.id == id) {
            self.park(id, "archived");
            if let Some(d) = self.dormant.last_mut() {
                d.saved.archived = true;
                d.saved.pin = 0;
            }
        } else if let Some(d) = self.dormant.iter_mut().find(|d| d.id == id) {
            d.saved.archived = true;
            d.saved.pin = 0;
        }
    }

    /// Back from Archived into the list (counts as just used, so the limit keeps it).
    pub(super) fn unarchive(&mut self, id: u64) {
        if let Some(d) = self.dormant.iter_mut().find(|d| d.id == id) {
            d.saved.archived = false;
            d.saved.used = now_secs();
        }
    }

    /// The usage shown on a session's row: its share of the 5-hour plan limit, or its
    /// estimated cost (Options > General > Sessions).
    pub(super) fn usage_label(&self, row: &Row) -> Option<String> {
        if !self.s.show_cost {
            return None;
        }
        if self.s.usage_as == "plan" {
            row.plan.map(fmt_plan)
        } else {
            row.cost.map(crate::cost::fmt_usd)
        }
    }

    pub(super) fn rows(&self, p: &Path) -> Vec<Row> {
        let mut v: Vec<Row> = self
            .tabs
            .iter()
            .filter(|t| t.project == p)
            .map(|t| Row {
                id: t.sess.id,
                live: true,
                name: t.sess.display_name(),
                profile: t.sess.profile.clone(),
                created: t.created,
                exited: t.sess.exited,
                unseen: t.sess.unseen,
                images: t.sess.pasted_images.len(),
                resumable: t.sess.agent_id.is_some(),
            unrestricted: t.sess.opts.unrestricted,
            attention: t.sess.attention.is_some(),
            busy: t.sess.busy && t.sess.exited.is_none(),
            attention_msg: t.sess.attention.clone(),
            cost: t.sess.cost.map(|c| c.usd),
            plan: t.sess.agent_id.as_ref().and_then(|a| self.plan_usage.get(a)).map(|p| p.five_hour),
            pin: t.pin,
            archived: false,
            })
            .collect();
        v.extend(self.dormant.iter().filter(|d| d.saved.project == p).map(|d| Row {
            id: d.id,
            live: false,
            name: d.display_name(),
            profile: d.saved.profile.clone(),
            created: d.saved.created,
            exited: None,
            unseen: false,
            images: 0,
            resumable: d.saved.agent_id.is_some(),
            unrestricted: d.saved.opts.unrestricted,
            attention: false,
            busy: false,
            attention_msg: None,
            cost: d.saved.cost,
            plan: d.saved.agent_id.as_ref().and_then(|a| self.plan_usage.get(a)).map(|p| p.five_hour),
            pin: d.saved.pin,
            archived: d.saved.archived,
        }));
        let on_top = self.s.active_on_top;
        // Pinned first (in their order), then the active (running) ones if set, then the rest;
        // newest first within each; archived ones last.
        v.sort_by(|a, b| {
            a.archived
                .cmp(&b.archived)
                .then_with(|| (a.pin == 0, a.pin).cmp(&(b.pin == 0, b.pin)))
                .then_with(|| if on_top { (!a.is_active()).cmp(&!b.is_active()) } else { std::cmp::Ordering::Equal })
                .then_with(|| (b.created, b.id).cmp(&(a.created, a.id)))
        });
        v
    }

    /// Session ids in sidebar order (grouped by project).
    pub(super) fn sidebar_order(&self) -> Vec<u64> {
        let mut v = Vec::new();
        for p in &self.s.projects {
            v.extend(self.tabs.iter().filter(|t| t.project == p.path).map(|t| t.sess.id));
        }
        v
    }

    pub(super) fn remove_project(&mut self, p: &Path) {
        let ids = self.project_sessions(p);
        for id in ids {
            self.close(id);
        }
        self.remembered.remove(p);
        self.s.projects.retain(|x| x.path != p);
        if self.s.projects.is_empty() {
            let home = std::env::var_os("USERPROFILE").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("C:\\"));
            self.s.projects.push(Project { name: project_name(&home), path: home, collapsed: false, in_tabs: true });
        }
        if self.s.selected_project.as_deref() == Some(p) {
            self.select_project(None);
        }
    }

    /// Closes a project's tab: the project stays on Home (with its sessions, which keep
    /// running); opening it again from Home brings the tab back.
    pub(super) fn hide_project_tab(&mut self, p: &Path) {
        if let Some(x) = self.s.projects.iter_mut().find(|x| x.path == p) {
            x.in_tabs = false;
        }
        if self.s.selected_project.as_deref() == Some(p) {
            self.select_project(None);
        }
    }

    /// Confirmation for removing a project (from the app only; files stay on disk).
    pub(super) fn remove_project_dialog(&mut self, ctx: &egui::Context) {
        let Some(path) = self.confirm_remove_project.clone() else { return };
        let Some(name) = self.s.projects.iter().find(|p| p.path == path).map(|p| p.name.clone()) else {
            self.confirm_remove_project = None;
            return;
        };
        let running = self.project_sessions(&path).len();
        let mut choice: Option<bool> = None;
        let m = egui::Modal::new(egui::Id::unique("confirm_remove_project")).frame(egui::Frame::new().fill(PANEL).corner_radius(10).stroke(Stroke::new(1.0, BORDER)).inner_margin(20)).show(ctx, |ui| {
            ui.set_width(520.0);
            ui.label(RichText::new(format!("Remove \"{name}\" from AI Terminal?")).size(18.0).family(bold()).color(Color32::WHITE));
            ui.add_space(2.0);
            ui.label(RichText::new(path.display().to_string()).size(12.0).color(TEXT_FAINT));
            ui.add_space(12.0);
            ui.label(RichText::new("The project is removed from this list only. Nothing is deleted from disk: the folder and all its files stay as they are.").color(TEXT));
            if running > 0 {
                ui.add_space(6.0);
                ui.label(RichText::new(format!("Its {running} running session{} will be closed.", if running == 1 { "" } else { "s" })).color(ORANGE));
            }
            ui.add_space(14.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.add(egui::Button::new(RichText::new("Remove").color(Color32::WHITE)).fill(RED).min_size(Vec2::new(80.0, 28.0))).clicked() {
                    choice = Some(true);
                }
                if ui.add(egui::Button::new("Cancel").min_size(Vec2::new(70.0, 28.0))).clicked() {
                    choice = Some(false);
                }
            });
        });
        if m.should_close() && choice.is_none() {
            choice = Some(false);
        }
        if let Some(ok) = choice {
            self.confirm_remove_project = None;
            if ok {
                self.remove_project(&path);
            }
        }
    }

    pub(super) fn rename_project_dialog(&mut self, ctx: &egui::Context) {
        let Some((path, buf)) = &mut self.renaming_project else { return };
        let path = path.clone();
        let mut done = None;
        let m = egui::Modal::new(egui::Id::unique("rename_project")).frame(egui::Frame::new().fill(PANEL).corner_radius(10).stroke(Stroke::new(1.0, BORDER)).inner_margin(16)).show(ctx, |ui| {
            ui.set_width(360.0);
            ui.label(RichText::new("Rename project").family(bold()).size(15.0).color(TEXT));
            ui.label(RichText::new(path.display().to_string()).size(12.0).color(TEXT_FAINT));
            ui.add_space(6.0);
            let te = ui.add(egui::TextEdit::singleline(buf).desired_width(f32::INFINITY));
            te.request_focus();
            if te.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                done = Some(true);
            }
            ui.add_space(8.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button("Rename").clicked() {
                    done = Some(true);
                }
                if ui.button("Cancel").clicked() {
                    done = Some(false);
                }
            });
        });
        if m.should_close() && done.is_none() {
            done = Some(false);
        }
        if let Some(ok) = done {
            let (_, name) = self.renaming_project.take().unwrap();
            if ok && !name.trim().is_empty() {
                if let Some(p) = self.s.projects.iter_mut().find(|x| x.path == path) {
                    p.name = name.trim().to_string();
                }
            }
            self.focus_terminal = true;
        }
    }

    /// Left panel for the selected project: New session, the sessions list grouped by day.
    pub(super) fn sidebar(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let project = self.current_project();
        ui.add_space(10.0);
        // New session split button: main part = the default (Claude), ▼ = the full menu.
        let def = self.default_profile();
        let label = if def.is_agent() { format!("New {} session", def.name) } else { format!("New Shell ({})", def.name) };
        let (main, arrow) = widgets::split_button(ui, Some(agent_icon(&def).0), &label, ACCENT, 34.0);
        if main.on_hover_text("Ctrl+N  (Ctrl+Shift+N: unrestricted)").clicked() {
            self.new_default(&ctx, project.clone(), def.clone(), false);
        }
        egui::Popup::menu(&arrow).show(|ui| self.new_session_menu(ui, &ctx, &project));
        ui.add_space(12.0);

        let rows = self.rows(&project);
        let archived = rows.iter().filter(|r| r.archived).count();
        ui.horizontal(|ui| {
            ui.label(RichText::new("SESSIONS").size(11.0).family(bold()).color(TEXT_DIM));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if widgets::icon_button(ui, Icon::Gear, 24.0, TEXT_DIM, "Options").clicked() {
                    self.options = Some(OptPage::General);
                }
                if widgets::icon_button(ui, Icon::Plugin, 24.0, TEXT_DIM, "Claude Code plugins").clicked() {
                    self.open_plugins();
                }
                let n = rows.len() - archived;
                ui.label(RichText::new(format!("{n} session{}", if n == 1 { "" } else { "s" })).size(11.5).color(TEXT_FAINT));
            });
        });
        ui.add_space(4.0);
        ui.add(egui::TextEdit::singleline(&mut self.filter).hint_text("Filter sessions…").desired_width(f32::INFINITY));
        ui.add_space(6.0);

        let filter = self.filter.to_lowercase();
        let mut close_req = None;
        let mut restart_req = None;
        let mut select_req = None;
        let mut resume_req = None;
        let mut mode_req: Option<(u64, bool)> = None;
        let mut pin_req: Option<(u64, bool)> = None;
        let mut settings_req: Option<u64> = None;
        let mut archive_req: Option<(u64, bool)> = None;
        let mut toggle_archived = false;
        // While filtering, matching archived sessions show too.
        let show_archived = self.show_archived || !filter.is_empty();
        let today = local_day(now_secs());
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let mut last_group: Option<String> = None;
            let mut pinned_rects: Vec<(u64, egui::Rect)> = Vec::new();
            for row in &rows {
                let id = row.id;
                let name = row.name.clone();
                if !filter.is_empty() && !name.to_lowercase().contains(&filter) && !row.profile.name.to_lowercase().contains(&filter) {
                    continue;
                }
                if row.archived && !show_archived {
                    continue;
                }
                let group = if row.archived { "Archived".to_string() } else if row.pin > 0 { "Pinned".to_string() } else if self.s.active_on_top && row.is_active() { "Active".to_string() } else { day_label(today, local_day(row.created)) };
                if last_group.as_ref() != Some(&group) {
                    ui.add_space(4.0);
                    if row.archived {
                        toggle_archived |= archived_header(ui, archived, true);
                    } else {
                        ui.label(RichText::new(group.to_uppercase()).size(10.5).family(bold()).color(TEXT_FAINT));
                    }
                    last_group = Some(group);
                }
                let selected = self.active == Some(id);
                // Rows touch (no item gap), so the pointer is always over exactly one of them;
                // the highlight is drawn 1 px inset to keep a visual gap. (With gaps, egui widens
                // both neighbours over the gap and the hover flips between them every frame.)
                ui.spacing_mut().item_spacing.y = 0.0;
                let (full, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 32.0), if row.pin > 0 { Sense::click_and_drag() } else { Sense::click() });
                if row.pin > 0 {
                    pinned_rects.push((id, full));
                    if resp.drag_started() {
                        self.drag_session = Some(id);
                    }
                }
                let rect = full.shrink2(Vec2::new(0.0, 1.0));
                // The close button is always registered (drawn only on hover), so the set of
                // widgets doesn't change with the hover.
                let br = egui::Rect::from_center_size(egui::pos2(rect.right() - 15.0, rect.center().y), Vec2::splat(22.0));
                let close_resp = ui.interact(br, ui.scope_id().with(("close_session", id)), Sense::click());
                let hovered = resp.hovered() || close_resp.hovered();
                let p = ui.painter();
                if selected {
                    p.rect_filled(rect, 5.0, Color32::from_rgb(28, 52, 84));
                    p.rect_stroke(rect, 5.0, Stroke::new(1.0, Color32::from_rgb(44, 86, 140)), egui::StrokeKind::Inside);
                } else if hovered {
                    p.rect_filled(rect, 5.0, CARD);
                }
                let exited = row.exited.is_some() || !row.live;
                let (ic, icol) = agent_icon(&row.profile);
                let icol = if exited { icol.gamma_multiply(0.55) } else { icol };
                let icon_c = egui::pos2(rect.left() + 16.0, rect.center().y);
                widgets::paint_icon(p, ic, icon_c, 14.0, icol);
                if row.busy {
                    // Working: a ring spinning around the icon (redraws only while one is busy).
                    let t = ui.input(|i| i.time) as f32;
                    let a0 = t * 5.0;
                    let pts: Vec<egui::Pos2> = (0..=16).map(|k| {
                        let a = a0 + k as f32 / 16.0 * std::f32::consts::PI * 1.3;
                        icon_c + Vec2::new(a.cos(), a.sin()) * 10.5
                    }).collect();
                    p.add(egui::Shape::line(pts, Stroke::new(1.8, ACCENT_HI)));
                    ui.ctx().request_repaint_after(Duration::from_millis(50));
                }
                if row.attention {
                    p.circle_filled(egui::pos2(rect.left() + 24.0, rect.center().y - 6.0), 3.5, ACCENT_HI);
                } else if row.unseen && !selected {
                    p.circle_filled(egui::pos2(rect.left() + 23.0, rect.center().y - 6.0), 3.0, ORANGE);
                }
                let col = if !row.live {
                    TEXT_DIM
                } else if exited {
                    TEXT_FAINT
                } else if selected {
                    Color32::WHITE
                } else {
                    Color32::from_rgb(196, 200, 208)
                };
                let font = egui::FontId::new(13.0, if selected { bold() } else { egui::FontFamily::Proportional });
                let mut job = egui::text::LayoutJob::simple_singleline(name.clone(), font, col);
                job.wrap.max_width = rect.width() - if status_label(&row).is_some() && !hovered { 120.0 } else if self.usage_label(&row).is_some() { 110.0 } else { 80.0 };
                job.wrap.max_rows = 1;
                job.wrap.break_anywhere = true;
                let g = ui.fonts_mut(|f| f.layout_job(job));
                p.galley(egui::pos2(rect.left() + 34.0, rect.center().y - g.size().y / 2.0), g, col);
                let n = row.images;
                if row.unrestricted {
                    // Unrestricted badge.
                    p.text(egui::pos2(rect.left() + 26.0, rect.bottom() - 7.0), egui::Align2::CENTER_CENTER, "!", egui::FontId::new(11.0, bold()), ORANGE);
                }
                let mut x_clicked = false;
                if hovered {
                    if close_resp.hovered() {
                        p.rect_filled(br, 5.0, if close_resp.is_pointer_button_down_on() { Color32::from_rgb(62, 66, 76) } else { CARD_HI });
                    }
                    widgets::paint_icon(p, Icon::Close, br.center(), 12.0, if close_resp.hovered() { TEXT } else { TEXT_DIM });
                    if close_resp.clicked() {
                        close_req = Some(id);
                        x_clicked = true;
                    }
                    close_resp.on_hover_text(if row.archived { "Delete from the archive" } else { "Close session" });
                } else if let Some(label) = status_label(&row) {
                    p.text(egui::pos2(rect.right() - 10.0, rect.center().y), egui::Align2::RIGHT_CENTER, label, egui::FontId::proportional(11.5), ACCENT_HI);
                } else {
                    let mut x = rect.right() - 10.0;
                    if let Some(label) = self.usage_label(&row) {
                        let g = p.layout_no_wrap(label, egui::FontId::proportional(11.5), TEXT_DIM);
                        p.galley(egui::pos2(x - g.size().x, rect.center().y - g.size().y / 2.0), g.clone(), TEXT_DIM);
                        x -= g.size().x + 8.0;
                    }
                    if n > 0 {
                        p.text(egui::pos2(x, rect.center().y), egui::Align2::RIGHT_CENTER, n.to_string(), egui::FontId::proportional(11.5), TEXT_DIM);
                        widgets::paint_icon(p, Icon::Image, egui::pos2(x - 15.0, rect.center().y), 12.0, TEXT_DIM);
                    }
                }
                if resp.clicked() && !x_clicked {
                    if row.live {
                        select_req = Some(id);
                    } else {
                        resume_req = Some(id);
                    }
                }
                if resp.double_clicked() {
                    self.renaming = Some((id, name.clone()));
                    self.rename_focus = true;
                }
                let state = if row.archived {
                    "\nArchived — click to resume".to_string()
                } else if !row.live {
                    if row.resumable { "\nNot running — click to resume the conversation".to_string() } else { "\nNot running — click to start".to_string() }
                } else {
                    row.exited.map(|c| format!("\nExited with code {c}")).unwrap_or_default()
                };
                // Tooltip: the full session name (the row may cut it off), then type and state.
                let now = if row.busy {
                    "\nWorking…".to_string()
                } else {
                    row.attention_msg.as_ref().map(|m| format!("\nNeeds you: {m}")).unwrap_or_default()
                };
                let detail = format!("{}{}{}{now}", row.profile.name, if row.unrestricted { " · unrestricted" } else { "" }, state);
                let (live, resumable, unrestricted, agent) = (row.live, row.resumable, row.unrestricted, row.profile.is_agent());
                let pinned = row.pin > 0;
                let is_archived = row.archived;
                let tip_name = name.clone();
                resp.on_hover_ui(|ui| {
                    ui.label(RichText::new(tip_name).family(bold()).color(TEXT));
                    ui.label(RichText::new(detail).size(12.0).color(TEXT_DIM));
                })
                .context_menu(|ui| {
                    if ui.button("Rename").clicked() {
                        self.renaming = Some((id, name.clone()));
                        self.rename_focus = true;
                        ui.close();
                    }
                    if !is_archived && ui.button(if pinned { "Unpin" } else { "Pin to top" }).clicked() {
                        pin_req = Some((id, !pinned));
                        ui.close();
                    }
                    if ui.button(if is_archived { "Restore to list" } else { "Archive" }).clicked() {
                        archive_req = Some((id, !is_archived));
                        ui.close();
                    }
                    let label = if !live || resumable { "Resume" } else { "Restart" };
                    if ui.button(label).clicked() {
                        if live {
                            restart_req = Some(id);
                        } else {
                            resume_req = Some(id);
                        }
                        ui.close();
                    }
                    if agent {
                        ui.menu_button(if live { "Restart as" } else { "Resume as" }, |ui| {
                            if ui.add_enabled(!unrestricted, egui::Button::new("Unrestricted")).clicked() {
                                mode_req = Some((id, true));
                                ui.close();
                            }
                            if ui.add_enabled(unrestricted, egui::Button::new("Restricted")).clicked() {
                                mode_req = Some((id, false));
                                ui.close();
                            }
                        });
                        if ui.button(if live { "Restart with settings…" } else { "Resume with settings…" }).clicked() {
                            settings_req = Some(id);
                            ui.close();
                        }
                    }
                    if ui.button(if is_archived { "Delete" } else { "Close" }).clicked() {
                        close_req = Some(id);
                        ui.close();
                    }
                });
                // Inline rename.
                if self.renaming.as_ref().is_some_and(|(r, _)| *r == id) {
                    let r = egui::Rect::from_min_max(egui::pos2(rect.left() + 30.0, rect.top() + 3.0), egui::pos2(rect.right() - 6.0, rect.bottom() - 3.0));
                    let mut done = false;
                    let mut cancel = false;
                    if let Some((_, buf)) = self.renaming.as_mut() {
                        ui.painter().rect_filled(rect, 5.0, Color32::from_rgb(28, 52, 84));
                        let te = ui.put(r, egui::TextEdit::singleline(buf));
                        if std::mem::take(&mut self.rename_focus) {
                            te.request_focus();
                            // Select the old name, so typing replaces it.
                            if let Some(mut st) = egui::TextEdit::load_state(ui.ctx(), te.id) {
                                let end = egui::text::CCursor::new(buf.chars().count());
                                st.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(0), end)));
                                st.store(ui.ctx(), te.id);
                            }
                        }
                        cancel = te.has_focus() && ui.input(|i| i.key_pressed(Key::Escape));
                        // Enter or clicking elsewhere finishes the rename; Esc cancels it.
                        done = te.lost_focus() && !cancel;
                    }
                    if cancel {
                        self.renaming = None;
                        self.focus_terminal = true;
                    }
                    if done {
                        let (rid, name) = self.renaming.take().unwrap();
                        if let Some(t) = self.tabs.iter_mut().find(|t| t.sess.id == rid) {
                            t.sess.name = name.trim().to_string();
                        }
                        if let Some(d) = self.dormant.iter_mut().find(|d| d.id == rid) {
                            d.saved.name = name.trim().to_string();
                        }
                        self.focus_terminal = true;
                    }
                }
            }
            if archived > 0 && !show_archived {
                ui.add_space(4.0);
                toggle_archived |= archived_header(ui, archived, false);
            }
            if rows.len() == archived {
                ui.add_space(10.0);
                ui.label(RichText::new(if archived > 0 { "No sessions in the list." } else { "No sessions yet." }).color(TEXT_FAINT));
            }
            // Dragging a pinned session: a marker where it will go; dropping reorders.
            if let Some(dragged) = self.drag_session {
                let pos = ui.input(|i| i.pointer.interact_pos());
                let released = ui.input(|i| i.pointer.any_released());
                if let (Some(pos), false) = (pos, pinned_rects.is_empty()) {
                    let slot = pinned_rects.iter().position(|(_, r)| pos.y < r.center().y).unwrap_or(pinned_rects.len());
                    let y = if slot < pinned_rects.len() { pinned_rects[slot].1.top() } else { pinned_rects.last().unwrap().1.bottom() };
                    let x = pinned_rects[0].1.x_range();
                    ui.painter().line_segment([egui::pos2(x.min, y), egui::pos2(x.max, y)], Stroke::new(2.0, ACCENT_HI));
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                    if released {
                        let mut order: Vec<u64> = pinned_rects.iter().map(|(id, _)| *id).collect();
                        if let Some(from) = order.iter().position(|x| *x == dragged) {
                            let id = order.remove(from);
                            let to = if slot > from { slot - 1 } else { slot };
                            order.insert(to.min(order.len()), id);
                            for (k, id) in order.iter().enumerate() {
                                self.set_pin(*id, k as u32 + 1);
                            }
                        }
                    }
                }
                if released {
                    self.drag_session = None;
                }
            }
        });
        if toggle_archived && filter.is_empty() {
            self.show_archived = !self.show_archived;
        }
        if let Some((id, on)) = archive_req {
            if on {
                self.archive(id);
            } else {
                self.unarchive(id);
            }
        }
        if let Some(id) = select_req {
            self.select_session(id);
        }
        if let Some(id) = resume_req {
            self.resume(&ctx, id);
        }
        if let Some((id, on)) = pin_req {
            if on {
                let next = self.tabs.iter().map(|t| t.pin).chain(self.dormant.iter().map(|d| d.saved.pin)).max().unwrap_or(0) + 1;
                self.set_pin(id, next);
            } else {
                self.set_pin(id, 0);
            }
        }
        if let Some((id, unrestricted)) = mode_req {
            if let Some(d) = self.dormant.iter_mut().find(|d| d.id == id) {
                d.saved.opts.unrestricted = unrestricted;
                self.resume(&ctx, id);
            } else if let Some(t) = self.tabs.iter().find(|t| t.sess.id == id) {
                let mut o = t.sess.opts.clone();
                o.unrestricted = unrestricted;
                self.restart_with(&ctx, id, Some(o));
            }
        }
        if let Some(id) = settings_req {
            let found = self.tabs.iter().find(|t| t.sess.id == id).map(|t| (t.project.clone(), t.sess.profile.clone())).or_else(|| self.dormant.iter().find(|d| d.id == id).map(|d| (d.saved.project.clone(), d.saved.profile.clone())));
            if let Some((p, pr)) = found {
                self.open_launch_dialog(p, pr, Some(id));
            }
        }
        if let Some(id) = close_req {
            self.dormant.retain(|d| d.id != id);
            self.close(id);
        }
        if let Some(id) = restart_req {
            self.restart(&ctx, id);
        }
    }
}

/// The Archived group's header: "ARCHIVED (n)" with an arrow; click to expand / collapse.
fn archived_header(ui: &mut Ui, n: usize, open: bool) -> bool {
    ui.spacing_mut().item_spacing.y = 0.0;
    let (r, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 20.0), Sense::click());
    let col = if resp.hovered() { TEXT } else { TEXT_FAINT };
    let a = egui::Rect::from_center_size(egui::pos2(r.left() + 5.0, r.center().y), Vec2::splat(8.0));
    let pts = if open {
        vec![a.left_top(), a.right_top(), egui::pos2(a.center().x, a.bottom())]
    } else {
        vec![a.left_top(), egui::pos2(a.right(), a.center().y), a.left_bottom()]
    };
    ui.painter().add(egui::Shape::convex_polygon(pts, col, Stroke::NONE));
    ui.painter().text(egui::pos2(r.left() + 15.0, r.center().y), egui::Align2::LEFT_CENTER, format!("ARCHIVED ({n})"), egui::FontId::new(10.5, bold()), col);
    resp.on_hover_text(if open { "Hide archived sessions" } else { "Show archived sessions" }).clicked()
}
