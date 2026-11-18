//! The project tabs bar and the session header.

use super::*;

impl App {
    // ---------------------------------------------------------------------------------------
    // UI

    /// Top bar: Home + one tab per project and a "+" to add one; search, plugins and options on the right.
    pub(super) fn topbar(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let bar = ui.max_rect();
        let mut acts = TabActions::default();
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
                acts.select = Some(None);
            }
            ui.painter().line_segment([egui::pos2(r.right(), bar.top() + 8.0), egui::pos2(r.right(), bar.bottom() - 8.0)], Stroke::new(1.0, BORDER));

            // Project tabs, placed by hand on the full bar height (so they line up with Home).
            // Names shrink (cut with "…") before any tab moves into the "»" menu; the selected
            // tab always stays in the bar. A "+" (add project) follows the last tab.
            let right_w = 110.0;
            let x_end = bar.right() - right_w;
            let mut x = r.right() + 4.0;
            let tabs: Vec<TabInfo> = self
                .s
                .projects
                .clone()
                .into_iter()
                .filter(|p| p.in_tabs)
                .map(|proj| {
                    let sel = self.s.selected_project.as_ref() == Some(&proj.path);
                    let dot = self.project_dot(&proj.path);
                    let image = self.project_image(&proj.path);
                    let font = egui::FontId::new(13.5, if sel { bold() } else { egui::FontFamily::Proportional });
                    let name_w = ui.painter().layout_no_wrap(proj.name.clone(), font, TEXT).size().x;
                    let extra = if dot.is_some() { 14.0 } else { 0.0 } + if image.is_some() { 24.0 } else { 0.0 } + 32.0 + TAB_CLOSE_W;
                    TabInfo { proj, sel, dot, image, name_w, extra }
                })
                .collect();
            let room = x_end - x;
            let sizes: Vec<(f32, f32)> = tabs.iter().map(|t| (t.extra, t.name_w)).collect();
            let fitted = fit_tabs(&sizes, tabs.iter().position(|t| t.sel), room - 32.0, room - 64.0);
            let shown: Vec<usize> = fitted.iter().map(|x| x.0).collect();
            let mut tab_rects: Vec<(PathBuf, egui::Rect)> = Vec::new();
            for &(i, name_w) in &fitted {
                let t = &tabs[i];
                let (proj, sel, dot) = (&t.proj, t.sel, t.dot);
                let font = egui::FontId::new(13.5, if sel { bold() } else { egui::FontFamily::Proportional });
                let cut = t.name_w > name_w + 0.5;
                let mut job = egui::text::LayoutJob::simple_singleline(proj.name.clone(), font, TEXT);
                if cut {
                    job.wrap.max_width = name_w;
                    job.wrap.max_rows = 1;
                    job.wrap.break_anywhere = true;
                    job.wrap.overflow_character = Some('…');
                }
                let g = ui.fonts_mut(|f| f.layout_job(job));
                let w = t.extra + name_w;
                let tr = egui::Rect::from_min_max(egui::pos2(x, bar.top()), egui::pos2(x + w, bar.bottom()));
                x += w;
                let resp = ui.interact(tr, egui::Id::unique(("project_tab", &proj.path)), Sense::click_and_drag());
                tab_rects.push((proj.path.clone(), tr));
                if resp.drag_started() {
                    self.drag_project = Some(proj.path.clone());
                }
                let p = ui.painter();
                let accent = proj.accent_color();
                match accent {
                    // An accented tab: a glow rising from the bottom edge (stronger when hovered
                    // or selected) and a short accent bar under the name.
                    Some(a) => {
                        let strength = if sel { 0.30 } else if resp.hovered() { 0.22 } else { 0.11 };
                        let glow = egui::Rect::from_min_max(egui::pos2(tr.left() + 3.0, bar.top() + 4.0), egui::pos2(tr.right() - 3.0, bar.bottom()));
                        widgets::paint_glow_up(p, glow, a.gamma_multiply(strength));
                        if !sel {
                            let bw = (tr.width() * 0.35).min(28.0);
                            let by = bar.bottom() - 1.5;
                            p.line_segment([egui::pos2(tr.center().x - bw / 2.0, by), egui::pos2(tr.center().x + bw / 2.0, by)], Stroke::new(2.0, a.gamma_multiply(if resp.hovered() { 0.9 } else { 0.55 })));
                        }
                    }
                    None => {
                        if resp.hovered() && !sel {
                            p.rect_filled(tr.shrink2(Vec2::new(2.0, 6.0)), 5.0, CARD);
                        }
                    }
                }
                let mut tx = tr.left() + 16.0;
                if let Some(c) = dot {
                    p.circle_filled(egui::pos2(tx + 3.0, tr.center().y), 3.5, c);
                    tx += 14.0;
                }
                if let Some(img) = &t.image {
                    let ir = egui::Rect::from_min_size(egui::pos2(tx, tr.center().y - 9.0), Vec2::splat(18.0));
                    self.paint_project_image(ui, img, ir, 3);
                    tx += 24.0;
                }
                let p = ui.painter();
                let tc = if sel || resp.hovered() { TEXT } else { TEXT_DIM };
                p.galley_with_override_text_color(egui::pos2(tx, tr.center().y - g.size().y / 2.0), g, tc);
                // × (on hover / selected): closes the tab; the project stays on Home.
                let xr = egui::Rect::from_center_size(egui::pos2(tr.right() - 8.0 - TAB_CLOSE_W / 2.0, tr.center().y), Vec2::splat(16.0));
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
                    acts.hide = Some(proj.path.clone());
                }
                if sel {
                    let (x0, x1) = match accent {
                        Some(_) => (tr.left() + 3.0, tr.right() - 3.0),
                        None => (tr.left() + 8.0, tr.right() - 8.0),
                    };
                    p.line_segment([egui::pos2(x0, bar.bottom() - 1.0), egui::pos2(x1, bar.bottom() - 1.0)], Stroke::new(if accent.is_some() { 2.5 } else { 2.0 }, accent.unwrap_or(ACCENT_HI)));
                }
                if resp.clicked() {
                    acts.select = Some(Some(proj.path.clone()));
                }
                let name_line = if cut { format!("{}\n", proj.name) } else { String::new() };
                let tip = format!("{name_line}{}\n{}", proj.path.display(), self.project_tip(&proj.path));
                resp.on_hover_text(tip).context_menu(|ui| project_menu(ui, proj, &mut acts));
            }
            let overflow: Vec<&TabInfo> = tabs.iter().enumerate().filter(|(i, _)| !shown.contains(i)).map(|(_, t)| t).collect();

            // "»": the tabs that don't fit. Its dot shows the most urgent state among them, and
            // a tab dropped on it moves to the end (into the menu).
            let menu_rect = (!overflow.is_empty()).then(|| egui::Rect::from_min_size(egui::pos2(x + 2.0, bar.center().y - 14.0), Vec2::new(28.0, 28.0)));
            let pointer = ui.input(|i| i.pointer.interact_pos());
            let drop_on_menu = self.drag_project.is_some() && matches!((menu_rect, pointer), (Some(m), Some(p)) if m.contains(p));
            if let Some(or) = menu_rect {
                let resp = ui.interact(or, egui::Id::unique("project_overflow"), Sense::click());
                if resp.hovered() || drop_on_menu {
                    ui.painter().rect_filled(or, 5.0, CARD);
                }
                if drop_on_menu {
                    ui.painter().rect_stroke(or, 5.0, Stroke::new(1.5, ACCENT_HI), egui::StrokeKind::Inside);
                }
                ui.painter().text(or.center(), egui::Align2::CENTER_CENTER, "»", egui::FontId::new(16.0, bold()), if resp.hovered() { TEXT } else { TEXT_DIM });
                let urgent = [ACCENT_HI, ORANGE].into_iter().find(|c| overflow.iter().any(|t| t.dot == Some(*c)));
                if let Some(c) = urgent {
                    ui.painter().circle_filled(or.right_top() + Vec2::new(-5.0, 5.0), 3.5, c);
                }
                let waiting = overflow.iter().filter(|t| t.dot == Some(ACCENT_HI) || t.dot == Some(ORANGE)).count();
                let n = overflow.len();
                let mut tip = format!("{n} more project{}", if n == 1 { "" } else { "s" });
                if waiting > 0 {
                    tip += &format!(" ({waiting} with new activity)");
                }
                let resp = resp.on_hover_text(tip);
                if resp.clicked() {
                    self.tab_menu_filter.clear();
                    self.tab_menu_focus = true;
                }
                egui::Popup::menu(&resp).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
                    ui.set_min_width(260.0);
                    let filtering = overflow.len() > TAB_MENU_FILTER_FROM;
                    let mut enter = false;
                    if filtering {
                        let te = ui.add(egui::TextEdit::singleline(&mut self.tab_menu_filter).hint_text("Filter projects").desired_width(f32::INFINITY));
                        if std::mem::take(&mut self.tab_menu_focus) {
                            te.request_focus();
                        }
                        enter = te.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                        ui.add_space(4.0);
                    }
                    let q = self.tab_menu_filter.to_lowercase();
                    let matches: Vec<&TabInfo> = overflow.iter().copied().filter(|t| !filtering || q.is_empty() || t.proj.name.to_lowercase().contains(&q)).collect();
                    if enter {
                        if let Some(t) = matches.first() {
                            acts.bring_in = Some(t.proj.path.clone());
                            ui.close();
                        }
                    }
                    egui::ScrollArea::vertical().max_height(TAB_MENU_MAX_H).show(ui, |ui| {
                        if matches.is_empty() {
                            ui.label(RichText::new("No matching project").color(TEXT_FAINT));
                        }
                        for t in matches {
                            let (rr, rresp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 26.0), Sense::click());
                            let p = ui.painter();
                            if rresp.hovered() {
                                p.rect_filled(rr, 4.0, CARD_HI);
                            }
                            if let Some(a) = t.proj.accent_color() {
                                p.rect_filled(egui::Rect::from_min_size(egui::pos2(rr.left() + 2.0, rr.top() + 6.0), Vec2::new(3.0, rr.height() - 12.0)), 1.5, a);
                            }
                            if let Some(c) = t.dot {
                                p.circle_filled(egui::pos2(rr.left() + 15.0, rr.center().y), 3.5, c);
                            }
                            let mut tx = rr.left() + 26.0;
                            if let Some(img) = &t.image {
                                let ir = egui::Rect::from_min_size(egui::pos2(tx, rr.center().y - 9.0), Vec2::splat(18.0));
                                self.paint_project_image(ui, img, ir, 3);
                                tx += 24.0;
                            }
                            let mut job = egui::text::LayoutJob::simple_singleline(t.proj.name.clone(), egui::FontId::new(13.5, egui::FontFamily::Proportional), TEXT);
                            job.wrap.max_width = (rr.right() - 8.0 - tx).max(20.0);
                            job.wrap.max_rows = 1;
                            job.wrap.break_anywhere = true;
                            job.wrap.overflow_character = Some('…');
                            let g = ui.fonts_mut(|f| f.layout_job(job));
                            ui.painter().galley_with_override_text_color(egui::pos2(tx, rr.center().y - g.size().y / 2.0), g, if rresp.hovered() { TEXT } else { TEXT_DIM });
                            let tip = format!("{}\n{}", t.proj.path.display(), self.project_tip(&t.proj.path));
                            let rresp = rresp.on_hover_text(tip);
                            rresp.context_menu(|ui| project_menu(ui, &t.proj, &mut acts));
                            if rresp.clicked() {
                                acts.bring_in = Some(t.proj.path.clone());
                                ui.close();
                            }
                        }
                    });
                });
                x += 30.0;
            }

            // Dragging a project tab: a marker where it will go; dropping reorders the projects
            // (dropping on "»" sends the tab to the end, into the menu).
            if let Some(dragged) = self.drag_project.clone() {
                let released = ui.input(|i| i.pointer.any_released());
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                if drop_on_menu {
                    if released {
                        acts.move_to = Some((dragged.clone(), None));
                    }
                } else if let (Some(pos), false) = (pointer, tab_rects.is_empty()) {
                    let slot = tab_rects.iter().position(|(_, r)| pos.x < r.center().x).unwrap_or(tab_rects.len());
                    let xm = if slot < tab_rects.len() { tab_rects[slot].1.left() } else { tab_rects.last().unwrap().1.right() };
                    ui.painter().line_segment([egui::pos2(xm, bar.top() + 8.0), egui::pos2(xm, bar.bottom() - 8.0)], Stroke::new(2.0, ACCENT_HI));
                    if released {
                        let before = match tab_rects.get(slot) {
                            Some((p, _)) => Some(p.clone()),
                            // After the last tab in the bar: before whatever follows it in the list.
                            None => {
                                let last = &tab_rects.last().unwrap().0;
                                let i = self.s.projects.iter().position(|p| &p.path == last).unwrap_or(0);
                                self.s.projects.iter().skip(i + 1).find(|p| p.path != dragged).map(|p| p.path.clone())
                            }
                        };
                        acts.move_to = Some((dragged.clone(), before));
                    }
                }
                if released {
                    self.drag_project = None;
                }
            }
            // A project picked in the "»" menu moves into the bar, in place of the last tab there.
            if let Some(p) = &acts.bring_in {
                let before = tab_rects.last().map(|(p, _)| p.clone()).or_else(|| self.s.projects.iter().find(|x| x.in_tabs).map(|x| x.path.clone()));
                acts.move_to = Some((p.clone(), before));
                acts.select = Some(Some(p.clone()));
            }

            // "+": add a project folder (browser-style, right after the tabs).
            {
                let pr = egui::Rect::from_min_size(egui::pos2(x + 2.0, bar.center().y - 14.0), Vec2::new(28.0, 28.0));
                let resp = ui.interact(pr, egui::Id::unique("add_project"), Sense::click());
                if resp.hovered() {
                    ui.painter().rect_filled(pr, 5.0, CARD);
                }
                widgets::paint_icon(ui.painter(), Icon::Plus, pr.center(), 14.0, if resp.hovered() { TEXT } else { TEXT_DIM });
                if resp.on_hover_text("Add a project folder").clicked() {
                    self.add_project(&ctx);
                }
            }

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(6.0);
                if widgets::icon_button(ui, Icon::Gear, 30.0, TEXT_DIM, "Options").clicked() {
                    self.options = Some(OptPage::General);
                }
                if widgets::icon_button(ui, Icon::Plugin, 30.0, TEXT_DIM, "Plugins").clicked() {
                    self.open_plugins();
                }
                let search_tip = format!("Search session messages{}", keys::binding(&self.s.keybindings, keys::Cmd::SearchSessions).map(|b| format!(" ({})", keys::to_text(&b))).unwrap_or_default());
                if widgets::icon_button(ui, Icon::Search, 30.0, TEXT_DIM, &search_tip).clicked() {
                    self.open_search();
                }
            });
        });
        if let Some((p, before)) = acts.move_to {
            self.move_project(&p, before.as_deref());
        }
        if let Some(sel) = acts.select {
            self.select_project(sel);
        }
        if let Some(p) = acts.hide {
            self.hide_project_tab(&p);
        }
        if let Some((p, c)) = acts.set_accent {
            self.set_project_accent(&p, c);
        }
        if let Some(p) = acts.remove {
            self.confirm_remove_project = Some(p);
        }
        if let Some(p) = acts.open_settings {
            self.open_project_settings(&p);
        }
    }

    /// A project's activity dot: needs you, unseen output, running, or has saved sessions.
    fn project_dot(&self, p: &Path) -> Option<Color32> {
        if self.tabs.iter().any(|t| t.project == p && t.sess.attention.is_some()) {
            Some(ACCENT_HI)
        } else if self.tabs.iter().any(|t| t.project == p && t.sess.unseen) {
            Some(ORANGE)
        } else if !self.project_sessions(p).is_empty() {
            Some(GREEN.gamma_multiply(0.8))
        } else if self.project_session_count(p) > 0 {
            Some(TEXT_FAINT)
        } else {
            None
        }
    }

    /// Tooltip lines for a project: its session count and cost (or plan usage).
    fn project_tip(&self, path: &Path) -> String {
        let n = self.project_session_count(path);
        let cost_line = if !self.s.show_cost {
            String::new()
        } else if self.s.usage_as == "plan" {
            let ids = self.tabs.iter().filter(|t| t.project == path).filter_map(|t| t.sess.agent_id.clone()).chain(self.dormant.iter().filter(|d| d.saved.project == path).filter_map(|d| d.saved.agent_id.clone()));
            let (mut h5, mut wk) = (0.0, 0.0);
            for id in ids {
                if let Some(p) = self.plan_usage.get(&id) {
                    h5 += p.five_hour;
                    wk += p.seven_day;
                }
            }
            if h5 > 0.0 || wk > 0.0 { format!("\nPlan usage: {} of a 5-hour limit, {} of the weekly", fmt_plan(h5), fmt_plan(wk)) } else { String::new() }
        } else {
            let total: f64 = self.tabs.iter().filter(|t| t.project == path).filter_map(|t| t.sess.cost.map(|c| c.usd)).sum::<f64>() + self.dormant.iter().filter(|d| d.saved.project == path).filter_map(|d| d.saved.cost).sum::<f64>();
            if total > 0.0 { format!("\nEstimated cost {}", crate::cost::fmt_usd(total)) } else { String::new() }
        };
        format!("{n} session{}{cost_line}", if n == 1 { "" } else { "s" })
    }

    /// Moves a project in the project list (the tab order) to just before `before` (None = the end).
    fn move_project(&mut self, p: &Path, before: Option<&Path>) {
        if before == Some(p) {
            return;
        }
        let Some(from) = self.s.projects.iter().position(|x| x.path == p) else { return };
        let item = self.s.projects.remove(from);
        let to = before.and_then(|b| self.s.projects.iter().position(|x| x.path == b)).unwrap_or(self.s.projects.len());
        self.s.projects.insert(to, item);
    }

    /// Header above the terminal: session name, shell, folder, and the pasted images strip.
    /// The project's GitHub page, re-read from its git config at most every 10 s.
    fn github_page(&mut self, project: &Path) -> Option<String> {
        let now = std::time::Instant::now();
        match self.github_pages.get(project) {
            Some((at, url)) if now.duration_since(*at) < Duration::from_secs(10) => url.clone(),
            _ => {
                let url = crate::links::github_page(project);
                self.github_pages.insert(project.to_path_buf(), (now, url.clone()));
                url
            }
        }
    }

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
        let project = tab.project.clone();
        // One pill: profile · model (the one actually answering, from the transcript, else the
        // launch model) · effort. Clicking it opens the model / effort picker.
        let live_model = tab.sess.model.clone();
        let model = live_model.clone().or_else(|| tab.sess.opts.model.clone());
        let kind = tab.sess.profile.kind;
        let effort_text = kind.has_effort().then(|| {
            let catalog = self.catalog.lock().unwrap().clone().unwrap_or_default();
            let launch = tab.sess.opts.model.clone().or_else(|| live_model.clone());
            match (tab.sess.effort(), catalog.default_effort(kind, launch.as_deref())) {
                (Some(e), _) => format!("effort {e}"),
                (None, Some(d)) => format!("effort {d}"),
                (None, None) => "effort auto".to_string(),
            }
        });
        let mut model_parts: Vec<String> = Vec::new();
        if model.is_some() || profile != name {
            model_parts.push(profile.clone());
        }
        if let Some(m) = &model {
            model_parts.push(crate::cost::model_label(m));
        }
        model_parts.extend(effort_text);
        let model_pill = model_parts.join(" · ");
        let model_tip = match (&live_model, &model) {
            (Some(m), _) => format!("Model of the latest reply: {m}"),
            (None, Some(m)) => format!("Launch model: {m}"),
            _ => String::new(),
        };
        let ctx = ui.ctx().clone();
        let mut open: Option<PathBuf> = None;
        let mut restart = false;
        let mut show_context = false;
        let mut change_model = false;
        // Claude and Codex both have /model and /compact.
        let is_codex = tab.sess.profile.kind == Kind::Codex;
        let can_compact = exited.is_none() && (tab.sess.profile.kind == Kind::Claude || is_codex);
        let github = if self.s.show_github_button { self.github_page(&project) } else { None };
        ui.horizontal_centered(|ui| {
            ui.label(RichText::new(name).family(bold()).size(14.0).color(TEXT));
            if !model_pill.is_empty() {
                let pill = widgets::pill(ui, &model_pill, TEXT_DIM, CARD_HI);
                if can_compact {
                    let resp = ui.interact(pill.rect, ui.auto_id_with("model_pill"), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
                    if resp.hovered() {
                        ui.painter().rect_stroke(pill.rect, pill.rect.height() / 2.0, Stroke::new(1.0, LINK), egui::StrokeKind::Inside);
                    }
                    let how = if is_codex { "Click to change the model or reasoning effort (opens Codex's /model picker)." } else { "Click to change the model or reasoning effort." };
                    let tip = if model_tip.is_empty() { how.to_string() } else { format!("{model_tip}
{how}") };
                    if resp.on_hover_text(tip).clicked() {
                        change_model = true;
                    }
                } else if !model_tip.is_empty() {
                    pill.on_hover_text(model_tip);
                }
            }
            if busy_pill {
                widgets::pill(ui, "working…", Color32::WHITE, Color32::from_rgb(28, 92, 160));
            } else if let Some(m) = &attention_pill {
                widgets::pill(ui, if m.to_lowercase().contains("permission") { "needs permission" } else { "needs you" }, Color32::WHITE, ACCENT).on_hover_text(m);
            }
            if let Some(c) = cost_pill {
                // Plan mode: the share of the plan limits (once Claude has reported one); the
                // tokens either way.
                let head = match plan_pill {
                    Some(Some(p)) => format!("5h {} · week {} · ", fmt_plan(p.five_hour), fmt_plan(p.seven_day)),
                    Some(None) => String::new(),
                    None => format!("{} · ", crate::cost::fmt_usd(c.usd)),
                };
                let ctx = if c.context > 0 { format!("context {} · ", crate::cost::fmt_tokens(c.context)) } else { String::new() };
                let how = match (kind == Kind::Claude && resumable, can_compact && c.context > 0) {
                    (true, true) => "Click for the /context breakdown and a Compact button.",
                    (true, false) => "Click for the /context breakdown.",
                    (false, true) => "Click to open these details, with a Compact button.",
                    (false, false) => "Click to open these details.",
                };
                let tip = format!("{}\n\n{how}", super::context_info::details(&c, plan_pill));
                let pill = widgets::pill(ui, &format!("{ctx}{head}{} tokens", crate::cost::fmt_tokens(c.usage.total())), TEXT_DIM, CARD_HI);
                let resp = ui.interact(pill.rect, ui.auto_id_with("context_pill"), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
                if resp.hovered() {
                    ui.painter().rect_stroke(pill.rect, pill.rect.height() / 2.0, Stroke::new(1.0, LINK), egui::StrokeKind::Inside);
                }
                if resp.on_hover_text(tip).clicked() {
                    show_context = true;
                }
            }
            if unrestricted_pill {
                widgets::pill(ui, "unrestricted", Color32::WHITE, Color32::from_rgb(150, 80, 20)).on_hover_text("Permission prompts are skipped");
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
                if widgets::ghost_button(ui, "Scripts").on_hover_text("Run a script from the scripts folder in this project, as its own session (Ctrl+Shift+R)").clicked() {
                    self.open_scripts();
                }
                if widgets::ghost_button(ui, "Snippets").on_hover_text("Insert a saved prompt (Ctrl+Shift+S)").clicked() {
                    self.snippets_open = true;
                    self.snippet_focus = true;
                }
                // The search box. Clicking into it drops down the recent searches: click one to
                // search it again, × to forget it.
                let mut find: Option<String> = None;
                let mut forget: Option<usize> = None;
                let mut forget_all = false;
                let edit = ui.add(
                    egui::TextEdit::singleline(&mut self.header_query)
                        .hint_text("Search this session…")
                        .desired_width(180.0)
                        .margin(egui::Margin { left: 8, right: 26, top: 4, bottom: 4 }),
                );
                if edit.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                    find = Some(std::mem::take(&mut self.header_query));
                }
                // A magnifying glass inside the box's right end, once there's text: click to search.
                if !self.header_query.trim().is_empty() {
                    let gr = egui::Rect::from_center_size(egui::pos2(edit.rect.right() - 13.0, edit.rect.center().y), Vec2::splat(20.0));
                    let go = ui.interact(gr, ui.auto_id_with("header_find_go"), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
                    if go.hovered() {
                        ui.painter().rect_filled(gr, 4.0, CARD_HI);
                    }
                    widgets::paint_icon(ui.painter(), Icon::Search, gr.center(), 13.0, if go.hovered() { TEXT } else { TEXT_DIM });
                    if go.on_hover_text("Search (Enter)").clicked() {
                        find = Some(std::mem::take(&mut self.header_query));
                    }
                }
                if (edit.gained_focus() || edit.clicked()) && !self.s.find_history.is_empty() {
                    self.find_hist_open = true;
                }
                let mut hist_open = self.find_hist_open;
                let popup = egui::Popup::from_response(&edit).open_bool(&mut hist_open).close_behavior(egui::PopupCloseBehavior::IgnoreClicks).align(egui::RectAlign::BOTTOM_START).show(|ui| {
                    // As wide as the longest entry (text + left pad + × button), between the box's
                    // width and a cap; longer entries are cut with "…".
                    let font = egui::FontId::new(13.5, egui::FontFamily::Proportional);
                    let text_w = self.s.find_history.iter().map(|q| ui.painter().layout_no_wrap(q.clone(), font.clone(), TEXT).size().x).fold(0.0, f32::max);
                    let min_w = edit.rect.width().max(240.0);
                    ui.set_width((text_w + 8.0 + 30.0).clamp(min_w, 480.0));
                    if self.s.find_history.is_empty() {
                        ui.label(RichText::new("No recent searches").color(TEXT_FAINT));
                        return;
                    }
                    egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                        for (i, q) in self.s.find_history.iter().enumerate() {
                            let (rr, rresp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 24.0), Sense::click());
                            let xr = egui::Rect::from_center_size(egui::pos2(rr.right() - 12.0, rr.center().y), Vec2::splat(18.0));
                            let xresp = ui.interact(xr, ui.auto_id_with(("find_hist_x", i)), Sense::click());
                            let p = ui.painter();
                            if rresp.hovered() || xresp.hovered() {
                                p.rect_filled(rr, 4.0, CARD_HI);
                            }
                            let mut job = egui::text::LayoutJob::simple_singleline(q.clone(), font.clone(), TEXT);
                            job.wrap.max_width = (xr.left() - 4.0 - (rr.left() + 8.0)).max(20.0);
                            job.wrap.max_rows = 1;
                            job.wrap.break_anywhere = true;
                            job.wrap.overflow_character = Some('…');
                            let g = ui.fonts_mut(|f| f.layout_job(job));
                            p.galley_with_override_text_color(egui::pos2(rr.left() + 8.0, rr.center().y - g.size().y / 2.0), g, if rresp.hovered() { TEXT } else { TEXT_DIM });
                            if rresp.hovered() || xresp.hovered() {
                                if xresp.hovered() {
                                    p.rect_filled(xr, 3.0, CARD);
                                }
                                widgets::paint_icon(p, Icon::Close, xr.center(), 11.0, if xresp.hovered() { TEXT } else { TEXT_DIM });
                            }
                            if xresp.on_hover_text("Remove from history").clicked() {
                                forget = Some(i);
                            } else if rresp.clicked() {
                                find = Some(q.clone());
                                ui.close();
                            }
                        }
                    });
                    ui.separator();
                    if super::status_bar::link_button(ui, "Clear history", "Forget all recent searches (asks first)") {
                        forget_all = true;
                        ui.close();
                    }
                });
                // Clicks in the box or the list keep it open; anywhere else closes it.
                if hist_open && ui.input(|i| i.pointer.any_click()) {
                    let pos = ui.input(|i| i.pointer.interact_pos());
                    let inside = pos.is_some_and(|p| edit.rect.contains(p) || popup.as_ref().is_some_and(|r| r.response.rect.contains(p)));
                    if !inside {
                        hist_open = false;
                    }
                }
                if find.is_some() || forget_all {
                    hist_open = false;
                }
                self.find_hist_open = hist_open;
                if let Some(i) = forget {
                    self.s.find_history.remove(i);
                }
                if forget_all {
                    self.confirm_clear_find_history = true;
                }
                if edit.lost_focus() && ui.input(|i| i.key_pressed(Key::Escape)) {
                    self.header_query.clear();
                }
                edit.on_hover_text("Find text in this terminal (Enter)");
                if let Some(q) = find.map(|q| q.trim().to_string()).filter(|q| !q.is_empty()) {
                    self.s.find_history.retain(|h| *h != q);
                    self.s.find_history.insert(0, q.clone());
                    self.s.find_history.truncate(FIND_HISTORY_MAX);
                    if let Some(t) = self.active_tab_mut() {
                        t.view.search.open = true;
                        t.view.search.focus = false;
                        t.view.search.query = q;
                    }
                }
                if let Some(url) = &github {
                    if widgets::icon_button(ui, Icon::GitHub, 26.0, TEXT_DIM, &format!("Open on GitHub: {url}")).clicked() {
                        crate::links::open_url(url);
                    }
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
        if show_context {
            self.context_info = Some(super::context_info::ContextInfo::new(id));
        }
        if change_model {
            if is_codex {
                // Codex has its own model and reasoning-effort picker.
                self.send_command_to(id, "/model");
            } else {
                self.model_switch = self.tabs.iter().find(|t| t.sess.id == id).map(|t| super::model_switch::ModelSwitch::new(&t.sess));
            }
        }
    }
}

/// Searches kept in the header search box's history.
const FIND_HISTORY_MAX: usize = 30;
/// Width of a project tab's close button area.
const TAB_CLOSE_W: f32 = 18.0;
/// The narrowest a tab's name gets before tabs start moving into the "»" menu.
const TAB_NAME_MIN: f32 = 96.0;
/// The widest a tab's name gets (longer names are cut with "…").
const TAB_NAME_MAX: f32 = 180.0;
/// The "»" menu gets a filter box when it holds more projects than this.
const TAB_MENU_FILTER_FROM: usize = 8;
/// The "»" menu's list scrolls past this height.
const TAB_MENU_MAX_H: f32 = 420.0;

/// A project tab, measured for the top bar.
struct TabInfo {
    proj: Project,
    sel: bool,
    dot: Option<Color32>,
    image: Option<PathBuf>,
    /// Full width of the name.
    name_w: f32,
    /// Width of everything but the name (padding, dot, image, close button).
    extra: f32,
}

/// What the user did in the top bar this frame (applied once drawing is done).
#[derive(Default)]
struct TabActions {
    select: Option<Option<PathBuf>>,
    remove: Option<PathBuf>,
    hide: Option<PathBuf>,
    set_accent: Option<(PathBuf, Option<[u8; 3]>)>,
    open_settings: Option<PathBuf>,
    /// A project picked in the "»" menu: it moves into the bar and gets selected.
    bring_in: Option<PathBuf>,
    /// Move a project to just before another one (None = the end of the list).
    move_to: Option<(PathBuf, Option<PathBuf>)>,
}

/// The right-click menu of a project tab (or of its row in the "»" menu).
fn project_menu(ui: &mut Ui, proj: &Project, acts: &mut TabActions) {
    if ui.button("Project settings…").clicked() {
        acts.open_settings = Some(proj.path.clone());
        ui.close();
    }
    let cfg = egui::containers::menu::MenuConfig::new().close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside);
    egui::containers::menu::SubMenuButton::new("Accent color").config(cfg).ui(ui, |ui| {
        if let Some(c) = widgets::accent_picker(ui, proj.accent) {
            acts.set_accent = Some((proj.path.clone(), c));
        }
    });
    if ui.button("Open in Explorer").clicked() {
        let _ = std::process::Command::new("explorer.exe").arg(&proj.path).spawn();
        ui.close();
    }
    ui.separator();
    if ui.button("Close tab (stays on Home)").clicked() {
        acts.hide = Some(proj.path.clone());
        ui.close();
    }
    if ui.button("Remove project…").clicked() {
        acts.remove = Some(proj.path.clone());
        ui.close();
    }
}

/// Chooses which project tabs show in the bar and how wide each name is drawn.
/// `tabs[i]` is (width without the name, full name width); `sel` is the selected tab's index.
/// The selected tab keeps its whole name (up to TAB_NAME_MAX); the others may be cut down to
/// TAB_NAME_MIN. All tabs show if they fit in `room_all` that way. Otherwise tabs fill
/// `room_menu` (which leaves space for "»") in order, and the selected tab, if it didn't make
/// it, replaces the last ones that did. Returns (index, name width) for the shown tabs, in order;
/// the other names share the widest cap that fits (names shorter than it keep their width).
fn fit_tabs(tabs: &[(f32, f32)], sel: Option<usize>, room_all: f32, room_menu: f32) -> Vec<(usize, f32)> {
    let narrow = |i: usize| tabs[i].0 + tabs[i].1.min(if Some(i) == sel { TAB_NAME_MAX } else { TAB_NAME_MIN });
    let (shown, room) = if (0..tabs.len()).map(narrow).sum::<f32>() <= room_all {
        ((0..tabs.len()).collect::<Vec<_>>(), room_all)
    } else {
        let mut shown = Vec::new();
        let mut used = 0.0;
        for i in 0..tabs.len() {
            if used + narrow(i) > room_menu {
                break;
            }
            used += narrow(i);
            shown.push(i);
        }
        if let Some(s) = sel.filter(|s| !shown.contains(s)) {
            while let Some(&last) = shown.last() {
                if used + narrow(s) <= room_menu {
                    break;
                }
                used -= narrow(last);
                shown.pop();
            }
            shown.push(s);
        }
        (shown, room_menu)
    };
    // The selected name: whole, unless it alone is wider than the bar (a very narrow window).
    let sel_w = sel.filter(|s| shown.contains(s)).map(|s| tabs[s].1.min(TAB_NAME_MAX).min((room - tabs[s].0).max(24.0)));
    let fixed = sel.filter(|s| shown.contains(s)).map(|s| tabs[s].0 + sel_w.unwrap_or(0.0)).unwrap_or(0.0);
    let others = || shown.iter().copied().filter(|&i| Some(i) != sel);
    let width = |cap: f32| fixed + others().map(|i| tabs[i].0 + tabs[i].1.min(cap)).sum::<f32>();
    let cap = if width(TAB_NAME_MAX) <= room {
        TAB_NAME_MAX
    } else {
        // The widest cap that fits (at least TAB_NAME_MIN, as the tabs were picked to fit that).
        let (mut lo, mut hi) = (TAB_NAME_MIN, TAB_NAME_MAX);
        for _ in 0..20 {
            let mid = (lo + hi) / 2.0;
            if width(mid) <= room {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        lo
    };
    shown.iter().map(|&i| (i, if Some(i) == sel { sel_w.unwrap_or(0.0) } else { tabs[i].1.min(cap) })).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(v: &[(usize, f32)]) -> Vec<usize> {
        v.iter().map(|x| x.0).collect()
    }

    #[test]
    fn all_tabs_fit_at_full_width() {
        let v = fit_tabs(&[(50.0, 80.0), (50.0, 100.0)], None, 1000.0, 968.0);
        assert_eq!(v, vec![(0, 80.0), (1, 100.0)]);
        // Over-long names are cut at TAB_NAME_MAX even with room to spare.
        let v = fit_tabs(&[(50.0, 400.0)], None, 1000.0, 968.0);
        assert_eq!(v, vec![(0, TAB_NAME_MAX)]);
    }

    #[test]
    fn names_shrink_before_tabs_overflow() {
        // Full width 2 * (50 + 150) = 400; at the narrowest 2 * (50 + 96) = 292.
        let v = fit_tabs(&[(50.0, 150.0), (50.0, 150.0)], None, 300.0, 268.0);
        assert_eq!(ids(&v), vec![0, 1]);
        let w = v[0].1;
        assert!(w < 150.0 && w >= TAB_NAME_MIN && 2.0 * (50.0 + w) <= 300.0);
    }

    #[test]
    fn short_names_are_never_cut() {
        // Names shorter than TAB_NAME_MIN keep their width; long ones shrink around them.
        let v = fit_tabs(&[(50.0, 40.0), (50.0, 300.0)], None, 300.0, 268.0);
        assert_eq!(v[0], (0, 40.0));
        assert!(v[1].1 >= TAB_NAME_MIN && 100.0 + 40.0 + v[1].1 <= 300.0);
    }

    #[test]
    fn overflow_keeps_order() {
        // Each tab is 50 + 96 = 146 at its narrowest; 450 holds 3.
        let tabs = vec![(50.0, 120.0); 10];
        assert_eq!(ids(&fit_tabs(&tabs, None, 482.0, 450.0)), vec![0, 1, 2]);
        // A short name later in the list doesn't jump ahead of the ones that overflowed.
        let mut tabs = vec![(50.0, 120.0); 4];
        tabs.push((50.0, 5.0));
        assert_eq!(ids(&fit_tabs(&tabs, None, 482.0, 450.0)), vec![0, 1, 2]);
    }

    #[test]
    fn selected_tab_stays_whole_and_in_the_bar() {
        let mut tabs = vec![(50.0, 120.0); 10];
        tabs[8] = (50.0, 170.0);
        // The selected tab (220 wide) replaces the last tab that fit; the others share the rest.
        // 520 holds 3 narrow tabs; with the selected one, 2 + it.
        let v = fit_tabs(&tabs, Some(8), 552.0, 520.0);
        assert_eq!(ids(&v), vec![0, 1, 8]);
        assert_eq!(v[2].1, 170.0);
        assert!(220.0 + 2.0 * (50.0 + v[0].1) <= 520.0 && v[0].1 >= TAB_NAME_MIN);
        // Selected and already in the bar: still drawn whole.
        let v = fit_tabs(&tabs, Some(1), 552.0, 520.0);
        assert_eq!(v[1], (1, 120.0));
    }

    #[test]
    fn selected_tab_alone_in_a_narrow_bar() {
        let v = fit_tabs(&[(50.0, 100.0), (50.0, 100.0)], Some(1), 90.0, 58.0);
        assert_eq!(v, vec![(1, 24.0)]);
    }
}
