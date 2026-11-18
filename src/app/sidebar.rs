//! The project / session sidebar.

use super::*;

impl App {
    /// Running sessions of a project.
    pub(super) fn project_sessions(&self, p: &Path) -> Vec<u64> {
        self.tabs.iter().filter(|t| t.project == p).map(|t| t.sess.id).collect()
    }

    /// Running + listed (not running, not archived or saved for later) sessions of a project.
    pub(super) fn project_session_count(&self, p: &Path) -> usize {
        self.project_sessions(p).len() + self.dormant.iter().filter(|d| d.saved.project == p && !d.saved.archived && !d.saved.later).count()
    }

    /// Lists the Claude conversations found by the start-up scan that aren't listed yet (each
    /// under the project of the folder it ran in; a new folder becomes a project on Home, not a
    /// tab). Older ones then go to Archived per "keep the last N".
    pub(super) fn merge_found_claude(&mut self) {
        let Some(found) = self.found_claude.lock().unwrap().take() else { return };
        let Some(profile) = self.profiles.iter().find(|p| p.kind == Kind::Claude).cloned() else { return };
        let mut known: std::collections::HashSet<String> =
            self.tabs.iter().filter_map(|t| t.sess.agent_id.clone()).chain(self.dormant.iter().filter_map(|d| d.saved.agent_id.clone())).collect();
        let norm = |p: &Path| p.display().to_string().to_lowercase().trim_end_matches('\\').to_string();
        let mut added = 0;
        for f in found {
            if !known.insert(f.id.clone()) {
                continue;
            }
            let key = norm(&f.cwd);
            let project = match self.s.projects.iter().find(|p| norm(&p.path) == key) {
                Some(p) => p.path.clone(),
                None => {
                    self.s.projects.push(Project { name: project_name(&f.cwd), path: f.cwd.clone(), collapsed: true, in_tabs: false, accent: None, image: None, background: None });
                    f.cwd.clone()
                }
            };
            let saved = SavedSession {
                project,
                profile: profile.clone(),
                name: String::new(),
                created: f.created,
                title: f.title,
                agent_id: Some(f.id),
                opts: Default::default(),
                cost: None,
                pin: 0,
                archived: false,
                later: false,
                used: f.used,
            };
            let id = self.next_id;
            self.next_id += 1;
            self.dormant.push(Dormant { id, saved });
            added += 1;
        }
        if added > 0 {
            crate::dlog!("listed {added} Claude conversation(s) from other apps");
            self.auto_archive();
        }
    }

    /// "Keep the last N sessions": per project, listed sessions past the N most recently used
    /// move to Archived. Running, pinned and saved-for-later sessions are never archived (running
    /// ones count towards N).
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
                .filter(|(_, d)| d.saved.project == p && !d.saved.archived && !d.saved.later && d.saved.pin == 0)
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
                d.saved.later = false;
                d.saved.pin = 0;
            }
        } else if let Some(d) = self.dormant.iter_mut().find(|d| d.id == id) {
            d.saved.archived = true;
            d.saved.later = false;
            d.saved.pin = 0;
        }
    }

    /// Sets a session aside in Saved for later (a running one is closed first; it stays
    /// resumable). Unlike Archived, it's never moved there automatically, nor out of it.
    pub(super) fn save_for_later(&mut self, id: u64) {
        if self.tabs.iter().any(|t| t.sess.id == id) {
            self.park(id, "saved for later");
            if let Some(d) = self.dormant.last_mut() {
                d.saved.later = true;
                d.saved.archived = false;
                d.saved.pin = 0;
            }
        } else if let Some(d) = self.dormant.iter_mut().find(|d| d.id == id) {
            d.saved.later = true;
            d.saved.archived = false;
            d.saved.pin = 0;
        }
    }

    /// Back from Archived into the list (counts as just used, so the limit keeps it).
    pub(super) fn unarchive(&mut self, id: u64) {
        if let Some(d) = self.dormant.iter_mut().find(|d| d.id == id) {
            d.saved.archived = false;
            d.saved.later = false;
            d.saved.used = now_secs();
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
                used: t.sess.used.max(t.created),
                exited: t.sess.exited,
                unseen: t.sess.unseen,
                images: t.sess.pasted_images.len(),
                resumable: t.sess.agent_id.is_some(),
            unrestricted: t.sess.opts.unrestricted,
            attention: t.sess.attention.is_some(),
            busy: t.sess.busy && t.sess.exited.is_none(),
            attention_msg: t.sess.attention.clone(),
            pin: t.pin,
            archived: false,
            later: false,
            created: t.created,
            cost: t.sess.cost.map(|c| c.usd),
            model: t.sess.model.clone().or_else(|| t.sess.opts.model.clone()),
            effort: t.sess.effort(),
            })
            .collect();
        v.extend(self.dormant.iter().filter(|d| d.saved.project == p).map(|d| Row {
            id: d.id,
            live: false,
            name: d.display_name(),
            profile: d.saved.profile.clone(),
            used: d.saved.used.max(d.saved.created),
            exited: None,
            unseen: false,
            images: 0,
            resumable: d.saved.agent_id.is_some(),
            unrestricted: d.saved.opts.unrestricted,
            attention: false,
            busy: false,
            attention_msg: None,
            pin: d.saved.pin,
            archived: d.saved.archived,
            later: d.saved.later,
            created: d.saved.created,
            cost: d.saved.cost,
            model: d.saved.opts.model.clone(),
            effort: d.saved.opts.effort.clone(),
        }));
        let on_top = self.s.active_on_top;
        // Pinned first (in their order), then the active (running) ones if set, then the rest;
        // most recently used first within each; then Saved for later, then Archived.
        v.sort_by(|a, b| {
            (a.archived, a.later)
                .cmp(&(b.archived, b.later))
                .then_with(|| (a.pin == 0, a.pin).cmp(&(b.pin == 0, b.pin)))
                .then_with(|| if on_top { (!a.is_active()).cmp(&!b.is_active()) } else { std::cmp::Ordering::Equal })
                .then_with(|| (b.used, b.id).cmp(&(a.used, a.id)))
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
        if let Some(x) = self.s.projects.iter().find(|x| x.path == p) {
            for img in x.image.iter().chain(x.background.as_ref().and_then(|b| b.image.as_ref())) {
                project_settings::discard_image(img);
            }
        }
        self.s.projects.retain(|x| x.path != p);
        if self.s.projects.is_empty() {
            let home = std::env::var_os("USERPROFILE").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("C:\\"));
            self.s.projects.push(Project { name: project_name(&home), path: home, collapsed: false, in_tabs: true, accent: None, image: None, background: None });
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

    /// Sets (or clears, with None) a project's accent color.
    pub(super) fn set_project_accent(&mut self, p: &Path, accent: Option<[u8; 3]>) {
        if let Some(x) = self.s.projects.iter_mut().find(|x| x.path == p) {
            x.accent = accent;
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

    /// Left panel for the selected project: New session, the sessions list grouped by day.
    pub(super) fn sidebar(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let project = self.current_project();
        ui.add_space(10.0);
        // New session split button: main part = the default row of the menu, ▼ = the full menu.
        let (def, variant) = self.default_launch();
        let (main, arrow) = widgets::split_button(ui, Some(agent_icon(&def).0), &launch::launch_label(&def, variant), ACCENT, 34.0);
        if main.on_hover_text("The default session: hover a row in the ▼ menu and click \"set as default\" to change it.\nCtrl+N: new Claude session  (Ctrl+Shift+N: unrestricted)").clicked() {
            self.launch_default(&ctx, project.clone());
        }
        // Closes on a click outside; the launch rows close it themselves, "set as default" keeps it open.
        let mut popup = egui::Popup::menu(&arrow).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside);
        if std::mem::take(&mut self.open_new_menu) {
            popup = popup.open_memory(egui::SetOpenCommand::Bool(true));
        }
        popup.show(|ui| self.new_session_menu(ui, &ctx, &project));
        ui.add_space(12.0);

        let rows = self.rows(&project);
        let archived = rows.iter().filter(|r| r.archived).count();
        let later = rows.iter().filter(|r| r.later && !r.archived).count();
        ui.horizontal(|ui| {
            ui.label(RichText::new("SESSIONS").size(11.0).family(bold()).color(TEXT_DIM));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let n = rows.len() - archived - later;
                ui.label(RichText::new(format!("{n} session{}", if n == 1 { "" } else { "s" })).size(11.5).color(TEXT_FAINT));
            });
        });
        ui.add_space(4.0);
        ui.add(egui::TextEdit::singleline(&mut self.filter).hint_text("Filter sessions…").desired_width(f32::INFINITY));
        ui.add_space(6.0);

        let filter = self.filter.to_lowercase();
        let mut close_req = None;
        let mut delete_req = None;
        let mut restart_req = None;
        let mut select_req = None;
        let mut resume_req = None;
        let mut mode_req: Option<(u64, bool)> = None;
        let mut pin_req: Option<(u64, bool)> = None;
        let mut settings_req: Option<u64> = None;
        let mut archive_req: Option<(u64, bool)> = None;
        let mut toggle_archived = false;
        let mut toggle_later = false;
        let mut group_req: Option<bool> = None;
        let mut later_req: Option<(u64, bool)> = None;
        let mut move_req: Option<(u64, PathBuf)> = None;
        // "Move to project" targets: the other projects, in their color.
        let other_projects: Vec<(String, PathBuf, Option<Color32>)> = self
            .s
            .projects
            .iter()
            .filter(|p| p.path != project)
            .map(|p| (p.name.clone(), p.path.clone(), p.accent.map(|[r, g, b]| Color32::from_rgb(r, g, b))))
            .collect();
        // While filtering, matching archived / saved-for-later sessions show too.
        let show_archived = self.show_archived || !filter.is_empty();
        let show_later = self.show_later || !filter.is_empty();
        let today = local_day(now_secs());
        // The list reaches into the panel's right margin, so its scroll bar sits next to the
        // panel edge and wakes up when the pointer is anywhere near it (the rows keep their width:
        // the extra strip is a content margin under the bar). It stops short of the panel's
        // resize handle.
        const GUTTER: f32 = 6.0;
        // Only the panel's own area is reported as used: reporting the widened rect would make
        // the side panel grow by GUTTER every frame (it sizes itself to its content).
        let full_rect = ui.available_rect_before_wrap();
        // The Favourite Snippets panel takes the bottom; the list gets the rest.
        let fav_h = self.fav_snippets_height(full_rect.height());
        let panel_rect = egui::Rect::from_min_max(full_rect.min, egui::pos2(full_rect.max.x, full_rect.max.y - fav_h));
        let list_rect = egui::Rect::from_min_max(panel_rect.min, egui::pos2(panel_rect.max.x + GUTTER, panel_rect.max.y));
        let mut list_ui = ui.new_child(egui::UiBuilder::new().max_rect(list_rect));
        list_ui.set_clip_rect(list_rect.intersect(ui.clip_rect().expand2(Vec2::new(GUTTER, 0.0))));
        self.fav_snippets_panel(ui, egui::Rect::from_min_max(egui::pos2(full_rect.min.x, panel_rect.max.y), full_rect.max));
        ui.advance_cursor_after_rect(full_rect);
        egui::ScrollArea::vertical().content_margin(egui::Margin { right: GUTTER as i8, ..Default::default() }).auto_shrink([false, false]).show_hinted(&mut list_ui, |ui| {
            let mut last_group: Option<String> = None;
            let mut pinned_rects: Vec<(u64, egui::Rect)> = Vec::new();
            for row in &rows {
                let id = row.id;
                let name = row.name.clone();
                if !filter.is_empty() && !name.to_lowercase().contains(&filter) && !row.profile.name.to_lowercase().contains(&filter) {
                    continue;
                }
                let group = if row.archived {
                    "Archived".to_string()
                } else if row.later {
                    "Saved for later".to_string()
                } else if row.pin > 0 {
                    "Pinned".to_string()
                } else if self.s.active_on_top && row.is_active() {
                    "Active".to_string()
                } else {
                    day_label(today, local_day(row.used))
                };
                // Saved for later and Archived are collapsible: their header shows either way.
                let open = if row.archived { show_archived } else if row.later { show_later } else { true };
                if last_group.as_ref() != Some(&group) {
                    ui.add_space(4.0);
                    if row.archived {
                        let r = section_header(ui, "ARCHIVED", archived, open);
                        toggle_archived |= r.clicked();
                        r.context_menu(|ui| {
                            if ui.button("Delete all…").on_hover_text("Delete every archived session of this project and its conversation (asks first)").clicked() {
                                group_req = Some(true);
                            }
                        });
                    } else if row.later {
                        let r = section_header(ui, "SAVED FOR LATER", later, open);
                        toggle_later |= r.clicked();
                        r.context_menu(|ui| {
                            if ui.button("Archive all…").on_hover_text("Move every saved-for-later session of this project to Archived (asks first)").clicked() {
                                group_req = Some(false);
                            }
                        });
                    } else {
                        ui.label(RichText::new(group.to_uppercase()).size(10.5).family(bold()).color(TEXT_FAINT));
                    }
                    last_group = Some(group);
                }
                if !open {
                    continue;
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
                // The hover buttons (pin; close on running ones) are always registered (drawn only
                // on hover), so the set of widgets doesn't change with the hover.
                let br = egui::Rect::from_center_size(egui::pos2(rect.right() - 15.0, rect.center().y), Vec2::splat(22.0));
                let pr = if row.live { br.translate(Vec2::new(-24.0, 0.0)) } else { br };
                let close_resp = ui.interact(br, ui.scope_id().with(("close_session", id)), Sense::click());
                let pin_resp = ui.interact(pr, ui.scope_id().with(("pin_session", id)), Sense::click());
                let hovered = resp.hovered() || close_resp.hovered() || pin_resp.hovered();
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
                    widgets::paint_busy_ring(p, icon_c, 10.5, t);
                    ui.ctx().request_repaint_after(Duration::from_millis(33));
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
                // The name gets everything left of what's drawn on the right (status or image
                // count; the close button on hover), plus a small gap.
                let small = egui::FontId::proportional(11.5);
                let text_w = |s: String| ui.fonts_mut(|f| f.layout_no_wrap(s, small.clone(), TEXT_DIM).size().x);
                let set_aside = row.archived || row.later;
                let buttons_w = if row.live { 50.0 } else if set_aside { 0.0 } else { 26.0 };
                let right_w = if hovered {
                    buttons_w
                } else if let Some(label) = status_label(&row) {
                    text_w(label.to_string())
                } else {
                    if row.images > 0 { text_w(row.images.to_string()) + 21.0 } else { 0.0 }
                };
                let mut job = egui::text::LayoutJob::simple_singleline(name.clone(), font, col);
                // Nothing on the right: let the name run almost to the row's edge.
                // The right-hand items end 10 px from the edge (the close button's box 4 px).
                let reserve = if hovered { buttons_w + 2.0 } else if right_w > 0.0 { right_w + 16.0 } else { 10.0 };
                job.wrap.max_width = rect.width() - 34.0 - reserve;
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
                    let hover_bg = |r: &egui::Response, rect: egui::Rect| {
                        if r.hovered() {
                            p.rect_filled(rect, 5.0, if r.is_pointer_button_down_on() { Color32::from_rgb(62, 66, 76) } else { CARD_HI });
                        }
                    };
                    let pinned = row.pin > 0;
                    if !set_aside {
                        hover_bg(&pin_resp, pr);
                        let pin_col = if pinned { ACCENT_HI } else if pin_resp.hovered() { TEXT } else { TEXT_DIM };
                        widgets::paint_icon(p, if pinned { Icon::PinFilled } else { Icon::Pin }, pr.center(), 14.0, pin_col);
                        if pin_resp.clicked() {
                            pin_req = Some((id, !pinned));
                            x_clicked = true;
                        }
                        pin_resp.on_hover_text(if pinned { "Unpin" } else { "Pin to top" });
                    }
                    if row.live {
                        hover_bg(&close_resp, br);
                        widgets::paint_icon(p, Icon::Close, br.center(), 12.0, if close_resp.hovered() { TEXT } else { TEXT_DIM });
                        if close_resp.clicked() {
                            close_req = Some(id);
                            x_clicked = true;
                        }
                        close_resp.on_hover_text("Close: stop it, keep it in the list");
                    }
                } else if let Some(label) = status_label(&row) {
                    p.text(egui::pos2(rect.right() - 10.0, rect.center().y), egui::Align2::RIGHT_CENTER, label, egui::FontId::proportional(11.5), ACCENT_HI);
                } else if n > 0 {
                    let x = rect.right() - 10.0;
                    p.text(egui::pos2(x, rect.center().y), egui::Align2::RIGHT_CENTER, n.to_string(), egui::FontId::proportional(11.5), TEXT_DIM);
                    widgets::paint_icon(p, Icon::Image, egui::pos2(x - 15.0, rect.center().y), 12.0, TEXT_DIM);
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
                } else if row.later {
                    "\nSaved for later — click to resume".to_string()
                } else if !row.live {
                    String::new()
                } else {
                    row.exited.map(|c| format!("\nExited with code {c}")).unwrap_or_default()
                };
                // Tooltip: the full session name (the row may cut it off), then type and state.
                let now = if row.busy {
                    "\nWorking…".to_string()
                } else {
                    row.attention_msg.as_ref().map(|m| format!("\nNeeds you: {m}")).unwrap_or_default()
                };
                // Profile · model · effort, then when it was created / last used and its cost.
                let mut head = vec![row.profile.name.clone()];
                head.extend(row.model.as_deref().filter(|m| !m.is_empty()).map(crate::cost::model_label));
                head.extend(row.effort.clone().filter(|e| !e.is_empty()));
                if row.unrestricted {
                    head.push("unrestricted".into());
                }
                let mut info = String::new();
                if row.created > 0 {
                    info += &format!("\nCreated {}", search_ui::fmt_time(row.created));
                }
                if row.used > row.created {
                    info += &format!("\nLast used {}", search_ui::fmt_time(row.used));
                }
                if let Some(c) = row.cost.filter(|c| *c > 0.0) {
                    info += &format!("\nCost {}", crate::cost::fmt_usd(c));
                }
                let detail = format!("{}{info}{state}{now}", head.join(" · "));
                let (live, resumable, unrestricted, agent) = (row.live, row.resumable, row.unrestricted, row.profile.is_agent());
                let pinned = row.pin > 0;
                let is_archived = row.archived;
                let is_later = row.later;
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
                    if !is_archived && !is_later && ui.button(if pinned { "Unpin" } else { "Pin to top" }).clicked() {
                        pin_req = Some((id, !pinned));
                        ui.close();
                    }
                    if other_projects.is_empty() {
                        ui.add_enabled(false, egui::Button::new("Move to project")).on_disabled_hover_text("There's no other project");
                    } else {
                        ui.menu_button("Move to project", |ui| {
                            for (pname, ppath, accent) in &other_projects {
                                let text = RichText::new(pname).color(accent.unwrap_or(TEXT));
                                if ui.button(text).on_hover_text(ppath.display().to_string()).clicked() {
                                    move_req = Some((id, ppath.clone()));
                                    ui.close();
                                }
                            }
                        });
                    }
                    if ui.button(if is_later { "Back to list" } else { "Save for later" }).on_hover_text(if is_later { "Back into the list" } else { "Set it aside in the Saved for later group (a running one is closed; it stays resumable)" }).clicked() {
                        later_req = Some((id, !is_later));
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
                    if live && ui.button("Close").on_hover_text("Stop it; it stays in the list").clicked() {
                        close_req = Some(id);
                        ui.close();
                    }
                    if ui.button("Delete…").on_hover_text("Remove it from the list and delete the conversation (asks first)").clicked() {
                        delete_req = Some(id);
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
            if rows.len() == archived + later {
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
        if toggle_later && filter.is_empty() {
            self.show_later = !self.show_later;
        }
        if let Some((id, on)) = later_req {
            if on {
                self.save_for_later(id);
            } else {
                self.unarchive(id);
            }
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
        if let Some((id, to)) = move_req {
            self.move_session(&ctx, id, to);
        }
        if let Some(id) = close_req {
            self.request_close(id);
        }
        if let Some(id) = delete_req {
            self.confirm_close = Some(super::session_close::ConfirmClose { id, delete: true });
        }
        if let Some(id) = restart_req {
            self.restart(&ctx, id);
        }
        if let Some(delete) = group_req {
            self.confirm_group = Some((project.clone(), delete));
        }
    }

    /// The listed (not running) sessions of a project in Archived (`archived`) or Saved for later.
    fn group_ids(&self, project: &Path, archived: bool) -> Vec<u64> {
        self.dormant
            .iter()
            .filter(|d| d.saved.project == project && if archived { d.saved.archived } else { d.saved.later && !d.saved.archived })
            .map(|d| d.id)
            .collect()
    }

    /// Confirmation for the group menus: Archive all (Saved for later) / Delete all (Archived).
    pub(super) fn group_dialog(&mut self, ctx: &egui::Context) {
        let Some((project, delete)) = self.confirm_group.clone() else { return };
        let n = self.group_ids(&project, delete).len();
        if n == 0 {
            self.confirm_group = None;
            return;
        }
        let s = if n == 1 { "" } else { "s" };
        let mut choice: Option<bool> = None;
        let m = egui::Modal::new(egui::Id::unique("confirm_group")).frame(egui::Frame::new().fill(PANEL).corner_radius(10).stroke(Stroke::new(1.0, BORDER)).inner_margin(20)).show(ctx, |ui| {
            ui.set_width(480.0);
            let title = if delete { format!("Delete {n} archived session{s}?") } else { format!("Archive {n} saved-for-later session{s}?") };
            ui.label(RichText::new(title).size(18.0).family(bold()).color(Color32::WHITE));
            ui.add_space(8.0);
            let text = if delete {
                "They're removed from the list and their conversations are deleted from disk. This can't be undone."
            } else {
                "They move to the Archived group (still resumable; right-click > Restore to list brings one back)."
            };
            ui.label(RichText::new(text).color(TEXT_DIM));
            ui.add_space(14.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let (label, col) = if delete { ("Delete all", Color32::from_rgb(170, 45, 45)) } else { ("Archive all", ACCENT) };
                if ui.add(egui::Button::new(RichText::new(label).color(Color32::WHITE)).fill(col).min_size(Vec2::new(90.0, 28.0))).clicked() {
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
            self.confirm_group = None;
            if ok {
                for id in self.group_ids(&project, delete) {
                    if delete {
                        self.delete_session(id);
                    } else {
                        self.archive(id);
                    }
                }
            }
        }
    }
}

/// A collapsible group's header (Saved for later, Archived): "TITLE (n)" with an arrow; click
/// to expand / collapse (the response, for its context menu).
fn section_header(ui: &mut Ui, title: &str, n: usize, open: bool) -> egui::Response {
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
    ui.painter().text(egui::pos2(r.left() + 15.0, r.center().y), egui::Align2::LEFT_CENTER, format!("{title} ({n})"), egui::FontId::new(10.5, bold()), col);
    resp
}
