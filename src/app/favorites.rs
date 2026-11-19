//! The sidebar's Favourites panel: starred snippets and scripts, one group each (snippets
//! first), in the order the user dragged them into (else by name) within a group. Click a row
//! to insert the snippet / run the script; drag it to reorder.

use super::*;

/// Header height (the panel's collapsed height) and smallest open height.
const HEADER_H: f32 = 28.0;
const MIN_H: f32 = 64.0;
const ROW_H: f32 = 26.0;

/// Snippet icon colour (the script one is GREEN).
const SNIPPET_COL: Color32 = LINK;

#[derive(Clone, Copy)]
enum Fav {
    /// Index into `s.snippets`.
    Snippet(usize),
    /// Index into `scripts`.
    Script(usize),
}

/// Which of a project's favourite lists.
#[derive(Clone, Copy)]
pub(super) enum FavList {
    /// Snippet names.
    Snippets,
    /// Script keys (lowercase file names).
    Scripts,
}

impl App {
    /// The current project's favourite list (empty when there's no project).
    pub(super) fn fav_list(&self, list: FavList) -> &[String] {
        if self.s.projects.is_empty() {
            return &[];
        }
        let cur = self.current_project();
        match self.s.projects.iter().find(|p| p.path == cur) {
            Some(p) => match list {
                FavList::Snippets => &p.fav_snippets,
                FavList::Scripts => &p.fav_scripts,
            },
            None => &[],
        }
    }

    /// The favourites pinned to every project.
    fn pinned_mut(&mut self, list: FavList) -> &mut Vec<String> {
        match list {
            FavList::Snippets => &mut self.s.pinned_snippets,
            FavList::Scripts => &mut self.s.pinned_scripts,
        }
    }

    pub(super) fn is_pinned(&self, list: FavList, key: &str) -> bool {
        let v = match list {
            FavList::Snippets => &self.s.pinned_snippets,
            FavList::Scripts => &self.s.pinned_scripts,
        };
        v.iter().any(|k| k == key)
    }

    /// Starred in the current project, or pinned to all.
    pub(super) fn is_fav(&self, list: FavList, key: &str) -> bool {
        self.is_pinned(list, key) || self.fav_list(list).iter().any(|k| k == key)
    }

    /// Stars `key` in the current project, or unstars it (and unpins it, if pinned).
    pub(super) fn toggle_fav(&mut self, list: FavList, key: &str) {
        if self.s.projects.is_empty() || key.is_empty() {
            return;
        }
        let on = self.is_fav(list, key);
        self.pinned_mut(list).retain(|k| k != key);
        let cur = self.current_project();
        let Some(p) = self.s.projects.iter_mut().find(|p| p.path == cur) else { return };
        let v = match list {
            FavList::Snippets => &mut p.fav_snippets,
            FavList::Scripts => &mut p.fav_scripts,
        };
        v.retain(|k| k != key);
        if !on {
            v.push(key.to_string());
        }
    }

    /// Pins `key` to every project, or unpins it (it then stays a favourite of the current
    /// project only).
    fn toggle_pin(&mut self, list: FavList, key: &str) {
        if self.is_pinned(list, key) {
            self.pinned_mut(list).retain(|k| k != key);
            if !self.fav_list(list).iter().any(|k| k == key) {
                self.toggle_fav(list, key);
            }
        } else {
            self.pinned_mut(list).push(key.to_string());
        }
    }

    /// Every project's list of that kind, plus the pinned one.
    fn all_fav_lists(&mut self, list: FavList) -> impl Iterator<Item = &mut Vec<String>> {
        let (projects, pinned) = (&mut self.s.projects, match list {
            FavList::Snippets => &mut self.s.pinned_snippets,
            FavList::Scripts => &mut self.s.pinned_scripts,
        });
        projects
            .iter_mut()
            .map(move |p| match list {
                FavList::Snippets => &mut p.fav_snippets,
                FavList::Scripts => &mut p.fav_scripts,
            })
            .chain(std::iter::once(pinned))
    }

    /// The current project's Favourites order (see `Project::fav_order`).
    fn fav_order(&self) -> &[String] {
        let cur = self.current_project();
        self.s.projects.iter().find(|p| p.path == cur).map_or(&[], |p| &p.fav_order)
    }

    /// A snippet or script was renamed: its stars and pin (in every project) follow it.
    pub(super) fn rename_fav(&mut self, list: FavList, old: &str, new: &str) {
        let (old_tag, new_tag) = (order_tag(list, old), order_tag(list, new));
        for p in &mut self.s.projects {
            for k in p.fav_order.iter_mut() {
                if *k == old_tag {
                    *k = new_tag.clone();
                }
            }
            let mut seen = std::collections::HashSet::new();
            p.fav_order.retain(|k| seen.insert(k.clone()));
        }
        for v in self.all_fav_lists(list) {
            for k in v.iter_mut() {
                if k == old {
                    *k = new.to_string();
                }
            }
            // A case-only script rename could leave a duplicate.
            let mut seen = std::collections::HashSet::new();
            v.retain(|k| seen.insert(k.clone()));
        }
    }

    /// A snippet or script was deleted: its stars and pin go too.
    pub(super) fn forget_fav(&mut self, list: FavList, key: &str) {
        let tag = order_tag(list, key);
        for p in &mut self.s.projects {
            p.fav_order.retain(|k| *k != tag);
        }
        for v in self.all_fav_lists(list) {
            v.retain(|k| k != key);
        }
    }

    /// A favourite's list and key.
    fn fav_key(&self, f: Fav) -> (FavList, String) {
        match f {
            Fav::Snippet(i) => (FavList::Snippets, self.s.snippets[i].name.clone()),
            Fav::Script(i) => (FavList::Scripts, scripts::script_key(&self.scripts[i].path).unwrap_or_default()),
        }
    }

    /// The favourites in display order: snippets, then scripts, each in the project's dragged
    /// order (unordered ones last, by name).
    fn favorites(&mut self) -> Vec<Fav> {
        let sn = (0..self.s.snippets.len()).filter(|&i| self.is_fav(FavList::Snippets, &self.s.snippets[i].name)).map(Fav::Snippet);
        let mut favs: Vec<Fav> = sn.collect();
        favs.extend(self.fav_script_list().into_iter().map(Fav::Script));
        let order = self.fav_order();
        favs.sort_by_cached_key(|&f| {
            let (list, key) = self.fav_key(f);
            let tag = order_tag(list, &key);
            let name = match f {
                Fav::Snippet(i) => &self.s.snippets[i].name,
                Fav::Script(i) => &self.scripts[i].name,
            };
            (matches!(f, Fav::Script(_)), order.iter().position(|k| *k == tag).unwrap_or(usize::MAX), name.to_lowercase())
        });
        favs
    }

    /// Moves `favs[from]` to just before `favs[to]` (`favs.len()` = the end) and saves the
    /// result as the current project's order.
    fn move_fav(&mut self, favs: &[Fav], from: usize, to: usize) {
        let mut tags: Vec<String> = favs.iter().map(|&f| {
            let (list, key) = self.fav_key(f);
            order_tag(list, &key)
        }).collect();
        let t = tags.remove(from);
        tags.insert(if to > from { to - 1 } else { to }, t);
        let cur = self.current_project();
        let Some(p) = self.s.projects.iter_mut().find(|p| p.path == cur) else { return };
        // Keep the ranks of favourites not shown now (say, a script that's missing for a while).
        let rest: Vec<String> = p.fav_order.iter().filter(|k| !tags.contains(k)).cloned().collect();
        tags.extend(rest);
        p.fav_order = tags;
    }

    /// The panel's height in a sidebar `avail` px tall: just its header when nothing is starred,
    /// else the saved height (dragged from its top edge), at most 60% of the sidebar.
    pub(super) fn favorites_height(&mut self, avail: f32) -> f32 {
        if self.favorites().is_empty() {
            return HEADER_H;
        }
        self.s.favorites_h.clamp(MIN_H, (avail * 0.6).max(MIN_H))
    }

    /// The Favourites panel in `rect` (the bottom of the sidebar).
    pub(super) fn favorites_panel(&mut self, ui: &mut Ui, rect: egui::Rect) {
        let favs = self.favorites();
        let mut ui = ui.new_child(egui::UiBuilder::new().max_rect(rect));
        ui.set_clip_rect(rect.intersect(ui.clip_rect()));

        // Resize handle along the top edge (only when there's something to show).
        let mut line = BORDER;
        if !favs.is_empty() {
            let edge = egui::Rect::from_center_size(egui::pos2(rect.center().x, rect.top()), Vec2::new(rect.width(), 8.0));
            let h = ui.interact(edge, egui::Id::unique("favorites_resize"), Sense::drag());
            if h.hovered() || h.dragged() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
                line = ACCENT_HI;
            }
            // The top edge follows the pointer (keeping the offset it was grabbed at), so going
            // past the cap and coming back doesn't leave the edge lagging away from the cursor.
            let grab_id = egui::Id::unique("favorites_resize_grab");
            if h.drag_started() {
                if let Some(p) = h.interact_pointer_pos() {
                    ui.ctx().data_mut(|d| d.insert_temp(grab_id, p.y - rect.top()));
                }
            }
            if h.dragged() {
                if let Some(p) = h.interact_pointer_pos() {
                    let grab = ui.ctx().data(|d| d.get_temp::<f32>(grab_id)).unwrap_or(0.0);
                    // Clamped to the 60% cap when laid out (favorites_height).
                    let max = ui.ctx().content_rect().height();
                    self.s.favorites_h = (rect.bottom() - (p.y - grab)).clamp(MIN_H, max);
                }
            }
        }
        ui.painter().hline(rect.x_range(), rect.top(), Stroke::new(1.0, line));

        // Header.
        let head = egui::Rect::from_min_size(rect.min, Vec2::new(rect.width(), HEADER_H));
        let p = ui.painter();
        p.text(egui::pos2(head.left(), head.center().y + 1.0), egui::Align2::LEFT_CENTER, "FAVOURITES", egui::FontId::new(11.0, bold()), TEXT_DIM);
        let note = if favs.is_empty() { "none".to_string() } else { favs.len().to_string() };
        p.text(egui::pos2(head.right() - 4.0, head.center().y + 1.0), egui::Align2::RIGHT_CENTER, note, egui::FontId::proportional(11.5), TEXT_FAINT);
        let tip = if favs.is_empty() {
            "Star a snippet or script (☆) in the Snippets / Scripts palette to list it here"
        } else {
            "Click a snippet to insert it into the current session, or a script to run it in the current project. Drag a row to reorder it, or the top edge to resize."
        };
        ui.interact(head, egui::Id::unique("favorites_head"), Sense::hover()).on_hover_text(tip);
        if favs.is_empty() {
            return;
        }

        // Rows.
        let project = project_name(&self.current_project());
        let mut pick = None;
        // (list, key): unstar it / pin or unpin it.
        let mut unstar: Option<(FavList, String)> = None;
        let mut pin: Option<(FavList, String)> = None;
        let mut edit = None;
        // Drag to reorder: (index of the dragged row, released this frame); (from, to) on drop.
        let mut dragging: Option<(usize, bool)> = None;
        let mut moved: Option<(usize, usize)> = None;
        let body = egui::Rect::from_min_max(egui::pos2(rect.left(), head.bottom()), rect.max);
        let mut body_ui = ui.new_child(egui::UiBuilder::new().max_rect(body));
        egui::ScrollArea::vertical().id_salt("favorites_scroll").auto_shrink([false, false]).show(&mut body_ui, |ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            let mut prev_script = None;
            let mut rows: Vec<(egui::Rect, bool)> = Vec::with_capacity(favs.len());
            for (idx, &f) in favs.iter().enumerate() {
                let (name, mark, mut tip, script, list, key) = match f {
                    Fav::Snippet(i) => {
                        let sn = &self.s.snippets[i];
                        let text = sn.text.lines().take(6).collect::<Vec<_>>().join("\n");
                        let how = if sn.send { "Snippet: click to insert and send" } else { "Snippet: click to insert" };
                        (sn.name.clone(), if sn.send { "↵" } else { "" }, format!("{text}\n\n{how}"), false, FavList::Snippets, sn.name.clone())
                    }
                    Fav::Script(i) => {
                        let sc = &self.scripts[i];
                        let key = scripts::script_key(&sc.path).unwrap_or_default();
                        (sc.name.clone(), "", format!("{}\n\nScript: click to run it in the background in {project}", sc.desc), true, FavList::Scripts, key)
                    }
                };
                let pinned = self.is_pinned(list, &key);
                if pinned {
                    tip.push_str("\nPinned: shown in every project");
                }
                // A thin divider between the snippets and the scripts.
                if prev_script == Some(false) && script {
                    let (r, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 7.0), Sense::hover());
                    ui.painter().hline(r.x_range().shrink(8.0), r.center().y, Stroke::new(1.0, BORDER));
                }
                prev_script = Some(script);

                let (r, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_H), Sense::click_and_drag());
                rows.push((r, script));
                if resp.dragged() || resp.drag_stopped() {
                    dragging = Some((idx, resp.drag_stopped()));
                }
                let lifted = resp.dragged();
                let hot = resp.hovered() || lifted;
                let p = ui.painter();
                if lifted {
                    p.rect_stroke(r, 5.0, Stroke::new(1.0, BORDER), egui::StrokeKind::Inside);
                } else if hot {
                    p.rect_filled(r, 5.0, CARD_HI);
                }
                let icon = egui::Rect::from_center_size(egui::pos2(r.left() + 12.0, r.center().y), Vec2::splat(11.0));
                if script {
                    paint_script_icon(p, icon, GREEN);
                } else {
                    paint_snippet_icon(p, icon, SNIPPET_COL);
                }
                // Right side: the pin (pinned to every project), then the ↵ mark.
                let mut right = 8.0;
                if pinned {
                    paint_pin_icon(p, egui::Rect::from_center_size(egui::pos2(r.right() - 14.0, r.center().y), Vec2::splat(12.0)), TEXT_FAINT);
                    right += 18.0;
                }
                if !mark.is_empty() {
                    p.text(egui::pos2(r.right() - right, r.center().y), egui::Align2::RIGHT_CENTER, mark, egui::FontId::proportional(12.0), TEXT_FAINT);
                    right += 16.0;
                }
                let mut job = egui::text::LayoutJob::simple_singleline(name, egui::FontId::proportional(13.0), if hot { TEXT } else { TEXT_DIM });
                job.wrap.max_width = r.width() - 26.0 - right;
                job.wrap.max_rows = 1;
                job.wrap.break_anywhere = true;
                let g = ui.fonts_mut(|fo| fo.layout_job(job));
                p.galley(egui::pos2(r.left() + 26.0, r.center().y - g.size().y / 2.0), g, TEXT_DIM);

                let resp = resp.on_hover_text(tip);
                if resp.clicked() {
                    pick = Some(f);
                }
                resp.context_menu(|ui| {
                    let (label, hint) = if pinned {
                        ("Unpin", "Show it only in this project's Favourites")
                    } else {
                        ("Pin to all projects", "Show it in the Favourites of every project")
                    };
                    if ui.button(label).on_hover_text(hint).clicked() {
                        pin = Some((list, key.clone()));
                    }
                    let remove = if pinned { "Remove from favourites (all projects)" } else { "Remove from favourites" };
                    if ui.button(remove).clicked() {
                        unstar = Some((list, key.clone()));
                    }
                    if ui.button(if script { "Edit scripts…" } else { "Edit snippets…" }).clicked() {
                        edit = Some(if script { OptPage::Scripts } else { OptPage::Snippets });
                    }
                });
            }

            // While dragging: a line where the row would land, within its own group (snippets
            // stay above scripts); dropping there saves the new order.
            let Some((from, released)) = dragging else { return };
            let Some(y) = ui.ctx().pointer_latest_pos().map(|p| p.y) else { return };
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
            let group = rows[from].1;
            let in_group: Vec<usize> = (0..rows.len()).filter(|&i| rows[i].1 == group).collect();
            let (first, last) = (in_group[0], in_group[in_group.len() - 1]);
            let to = in_group.iter().copied().find(|&i| y < rows[i].0.center().y).unwrap_or(last + 1);
            if to != from && to != from + 1 {
                let line_y = if to > last { rows[last].0.bottom() } else if to == first { rows[first].0.top() } else { (rows[to - 1].0.bottom() + rows[to].0.top()) / 2.0 };
                ui.painter().hline(rows[from].0.x_range().shrink(4.0), line_y, Stroke::new(2.0, ACCENT_HI));
                if released {
                    moved = Some((from, to));
                }
            }
            // Scroll when held near the top or bottom edge.
            let clip = ui.clip_rect();
            let edge = (ROW_H).min(clip.height() / 4.0);
            if !released && y < clip.top() + edge {
                ui.scroll_with_delta(Vec2::new(0.0, 6.0));
                ui.ctx().request_repaint();
            } else if !released && y > clip.bottom() - edge {
                ui.scroll_with_delta(Vec2::new(0.0, -6.0));
                ui.ctx().request_repaint();
            }
        });
        if let Some((from, to)) = moved {
            self.move_fav(&favs, from, to);
        }

        // A double-click is two clicks: the second one (soon after) doesn't run it again.
        if pick.is_some() {
            let now = std::time::Instant::now();
            if self.fav_last_pick.is_some_and(|t| now.duration_since(t) < Duration::from_millis(250)) {
                pick = None;
            } else {
                self.fav_last_pick = Some(now);
            }
        }
        match pick {
            Some(Fav::Snippet(i)) => self.insert_snippet(i),
            Some(Fav::Script(i)) => {
                let ctx = ui.ctx().clone();
                let sc = self.scripts[i].clone();
                let n = self.script_runs.len();
                self.run_script(&ctx, &sc);
                // A started script's answer box takes the keyboard, so the terminal doesn't.
                self.focus_terminal = self.script_runs.len() == n;
            }
            None => {}
        }
        if let Some((list, key)) = unstar {
            self.toggle_fav(list, &key);
        }
        if let Some((list, key)) = pin {
            self.toggle_pin(list, &key);
        }
        if let Some(page) = edit {
            self.options = Some(page);
        }
    }
}

/// A favourite's entry in `Project::fav_order`.
fn order_tag(list: FavList, key: &str) -> String {
    match list {
        FavList::Snippets => format!("snippet:{key}"),
        FavList::Scripts => format!("script:{key}"),
    }
}

/// Snippet icon: three lines of "text" of different lengths.
fn paint_snippet_icon(p: &egui::Painter, r: egui::Rect, col: Color32) {
    let s = Stroke::new(1.6, col);
    for (k, w) in [(0, 1.0), (1, 0.75), (2, 0.9)] {
        let y = r.top() + 1.5 + k as f32 * (r.height() - 3.0) / 2.0;
        p.hline(r.left()..=r.left() + r.width() * w, y, s);
    }
}

/// Script icon: a ">_" prompt.
fn paint_script_icon(p: &egui::Painter, r: egui::Rect, col: Color32) {
    let s = Stroke::new(1.6, col);
    let (l, t, b) = (r.left(), r.top() + 1.0, r.bottom() - 1.0);
    let mid = (t + b) / 2.0;
    p.add(egui::Shape::line(vec![egui::pos2(l, t), egui::pos2(l + r.width() * 0.4, mid), egui::pos2(l, b)], s));
    p.hline(l + r.width() * 0.5..=r.right(), b, s);
}

/// Pin icon: a pushpin leaning right (round head, collar, needle down to the left).
fn paint_pin_icon(p: &egui::Painter, r: egui::Rect, col: Color32) {
    let s = Stroke::new(1.5, col);
    let w = r.width();
    let head = r.right_top() + Vec2::new(-w * 0.3, w * 0.3);
    let dir = Vec2::new(-1.0, 1.0).normalized();
    let side = Vec2::new(dir.y, -dir.x);
    p.circle_filled(head, w * 0.26, col);
    let c = head + dir * w * 0.36;
    p.line_segment([c - side * w * 0.3, c + side * w * 0.3], s);
    p.line_segment([c, r.left_bottom() + Vec2::new(w * 0.05, -w * 0.05)], s);
}
