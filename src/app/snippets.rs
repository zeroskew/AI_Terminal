//! Snippets: the palette, inserting into a session, and their Options page.

use super::favorites::FavList;
use super::*;

/// One palette entry (snippets and scripts): bold name, a one-line monospace detail, and a
/// faint note on the right.
/// `pad` px at the right edge stay free (for a button drawn over it).
pub(super) fn palette_row(ui: &mut Ui, name: &str, detail: &str, note: &str, pad: f32) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 44.0), Sense::click());
    let p = ui.painter();
    // By pointer position, so the row stays lit while the pointer is on a button over it.
    if ui.rect_contains_pointer(r) {
        p.rect_filled(r, 5.0, CARD_HI);
    }
    p.text(egui::pos2(r.left() + 10.0, r.top() + 13.0), egui::Align2::LEFT_CENTER, name, egui::FontId::new(14.0, bold()), TEXT);
    // The note on the right, laid out first so the detail line stops before it.
    let note_g = (!note.is_empty()).then(|| ui.fonts_mut(|f| f.layout_no_wrap(note.to_string(), egui::FontId::proportional(11.5), TEXT_FAINT)));
    let note_w = note_g.as_ref().map_or(0.0, |g| g.size().x);
    let mut job = egui::text::LayoutJob::simple_singleline(detail.to_string(), egui::FontId::monospace(12.0), TEXT_DIM);
    job.wrap.max_width = r.width() - pad - (note_w + 40.0).max(110.0);
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    let g = ui.fonts_mut(|f| f.layout_job(job));
    p.galley(egui::pos2(r.left() + 10.0, r.top() + 24.0), g, TEXT_DIM);
    if let Some(g) = note_g {
        let pos = egui::pos2(r.right() - 10.0 - pad - g.size().x, r.center().y - g.size().y / 2.0);
        p.galley(pos, g, TEXT_FAINT);
    }
    resp
}

/// Gold of a favourite's star.
pub(super) const STAR_ON: Color32 = Color32::from_rgb(240, 190, 60);

/// Paints a five-pointed star in `r`: filled in `col` when `on`, else an outline.
pub(super) fn paint_star(p: &egui::Painter, r: egui::Rect, on: bool, col: Color32) {
    let c = r.center() + Vec2::new(0.0, r.height() * 0.04);
    let (ro, ri) = (r.width().min(r.height()) / 2.0, r.width().min(r.height()) / 2.0 * 0.42);
    let pt = |k: usize, rad: f32| {
        let a = -std::f32::consts::FRAC_PI_2 + k as f32 * std::f32::consts::PI / 5.0;
        c + Vec2::new(a.cos(), a.sin()) * rad
    };
    let pts: Vec<egui::Pos2> = (0..10).map(|k| pt(k, if k % 2 == 0 { ro } else { ri })).collect();
    if on {
        // Concave, so: the inner pentagon plus a triangle per point.
        p.add(egui::Shape::convex_polygon((0..5).map(|k| pts[k * 2 + 1]).collect(), col, Stroke::NONE));
        for k in 0..5 {
            p.add(egui::Shape::convex_polygon(vec![pts[(k * 2 + 9) % 10], pts[k * 2], pts[k * 2 + 1]], col, Stroke::NONE));
        }
    } else {
        p.add(egui::Shape::closed_line(pts, Stroke::new(1.3, col)));
    }
}

/// A star toggle in `r` (☆ off / ★ on).
pub(super) fn star_button(ui: &mut Ui, r: egui::Rect, id: egui::Id, on: bool) -> egui::Response {
    let resp = ui.interact(r, id, Sense::click());
    let col = if on { STAR_ON } else if resp.hovered() { TEXT } else { TEXT_FAINT };
    if resp.hovered() {
        ui.painter().rect_filled(r, 4.0, PANEL);
    }
    paint_star(ui.painter(), r.shrink(5.0), on, col);
    resp.on_hover_text(if on { "Remove from favourites" } else { "Add to favourites (bottom of the sidebar)" })
}

/// A scroll area for `rows` palette rows that is exactly as tall as its rows, up to 70% of the
/// window. (Sized explicitly: a plain ScrollArea inside a modal keeps last frame's height, so a
/// list that grows, e.g. after Refresh, gets clipped.)
pub(super) fn palette_scroll(ui: &Ui, rows: usize) -> egui::ScrollArea {
    let gap = ui.spacing().item_spacing.y;
    let content = rows as f32 * (44.0 + gap) - if rows > 0 { gap } else { 0.0 };
    let cap = (ui.ctx().content_rect().height() * 0.7).max(200.0);
    let h = content.min(cap);
    egui::ScrollArea::vertical().min_scrolled_height(h).max_height(h)
}

impl App {
    // ---------------------------------------------------------------------------------------
    // Snippets

    /// Types a snippet into the active session (as a paste; Enter afterwards when it says so).
    pub(super) fn insert_snippet(&mut self, i: usize) {
        let Some(sn) = self.s.snippets.get(i).cloned() else { return };
        let Some(tab) = self.active_tab_mut() else { return };
        let bracketed = tab.sess.term.lock().mode().contains(alacritty_terminal::term::TermMode::BRACKETED_PASTE);
        let text = sn.text.replace("\r\n", "\n");
        let data = if bracketed { format!("\x1b[200~{}\x1b[201~", text.replace('\x1b', "")) } else { text.replace('\n', "\r") };
        tab.sess.term.lock().scroll_display(alacritty_terminal::grid::Scroll::Bottom);
        tab.sess.write(data.into_bytes());
        let id = tab.sess.id;
        if sn.send {
            self.enter_due.push((id, std::time::Instant::now() + Duration::from_millis(200)));
        }
        self.focus_terminal = true;
    }

    /// The snippets palette (Ctrl+Shift+S or the header button): filter, Enter = first match.
    pub(super) fn snippets_palette(&mut self, ctx: &egui::Context) {
        if !self.snippets_open {
            return;
        }
        let mut pick: Option<usize> = None;
        let mut close = false;
        let mut edit = false;
        let mut star: Option<usize> = None;
        let m = egui::Modal::new(egui::Id::unique("snippets")).frame(egui::Frame::new().fill(PANEL).corner_radius(10).stroke(Stroke::new(1.0, BORDER)).inner_margin(16)).show(ctx, |ui| {
            ui.set_width(560.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("SNIPPETS").size(12.0).family(bold()).color(TEXT_DIM));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if widgets::corner_close_button(ui, 16.0).clicked() {
                        close = true;
                    }
                    if ui.button("Edit snippets…").clicked() {
                        edit = true;
                    }
                });
            });
            let te = ui.add(egui::TextEdit::singleline(&mut self.snippet_filter).hint_text("Filter").desired_width(f32::INFINITY));
            if std::mem::take(&mut self.snippet_focus) {
                te.request_focus();
            }
            ui.add_space(6.0);
            let f = self.snippet_filter.to_lowercase();
            let shown: Vec<usize> = (0..self.s.snippets.len()).filter(|&i| f.is_empty() || self.s.snippets[i].name.to_lowercase().contains(&f) || self.s.snippets[i].text.to_lowercase().contains(&f)).collect();
            if te.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                pick = shown.first().copied();
            }
            if self.s.snippets.is_empty() {
                ui.label(RichText::new("No snippets yet. Add some in Options > Snippets.").color(TEXT_FAINT));
            }
            palette_scroll(ui, shown.len()).show_hinted(ui, |ui| {
                for i in shown {
                    let sn = &self.s.snippets[i];
                    let fav = self.is_fav(FavList::Snippets, &sn.name);
                    let row = palette_row(ui, &sn.name, sn.text.lines().next().unwrap_or(""), if sn.send { "↵ send" } else { "" }, 28.0);
                    let sr = egui::Rect::from_center_size(egui::pos2(row.rect.right() - 20.0, row.rect.center().y), Vec2::splat(26.0));
                    if star_button(ui, sr, egui::Id::unique(("snippet_star", i)), fav).clicked() {
                        star = Some(i);
                    } else if row.clicked() {
                        pick = Some(i);
                    }
                }
            });
        });
        if m.should_close() {
            close = true;
        }
        if let Some(i) = star {
            let name = self.s.snippets[i].name.clone();
            self.toggle_fav(FavList::Snippets, &name);
        }
        if let Some(i) = pick {
            self.insert_snippet(i);
            close = true;
        }
        if edit {
            self.options = Some(OptPage::Snippets);
            close = true;
        }
        if close {
            self.snippets_open = false;
            self.snippet_filter.clear();
            self.focus_terminal = true;
        }
    }

    pub(super) fn options_snippets(&mut self, ui: &mut Ui) {
        ui.label(RichText::new("Snippets").family(bold()).size(18.0).color(TEXT));
        ui.label(RichText::new("Saved prompts and commands. Insert one from the Snippets button in the session header or the palette (Ctrl+Shift+S).").size(12.5).color(TEXT_DIM));
        ui.add_space(8.0);
        let mut remove = None;
        let mut swap = None;
        let n = self.s.snippets.len();
        let project = if self.s.projects.is_empty() { String::new() } else { project_name(&self.current_project()) };
        let mut star = None;
        let mut renamed = None;
        for i in 0..n {
            let fav = self.is_fav(FavList::Snippets, &self.s.snippets[i].name);
            egui::Frame::new().fill(CARD).corner_radius(8).stroke(Stroke::new(1.0, BORDER)).inner_margin(10).show(ui, |ui| {
                ui.set_width(ui.available_width());
                let sn = &mut self.s.snippets[i];
                ui.horizontal(|ui| {
                    let te = ui.add(egui::TextEdit::singleline(&mut sn.name).hint_text("Name").desired_width(260.0));
                    // Stars are kept by name: when an edit ends with a new name, they follow it.
                    if te.gained_focus() {
                        self.snippet_name_before = Some((i, sn.name.clone()));
                    }
                    if te.lost_focus() {
                        if let Some((k, old)) = self.snippet_name_before.take() {
                            if k == i && old != sn.name {
                                renamed = Some((old, sn.name.clone()));
                            }
                        }
                    }
                    ui.checkbox(&mut sn.send, "Press Enter after inserting");
                    if !project.is_empty() {
                        let mut on = fav;
                        if ui.checkbox(&mut on, format!("Favourite in {project}")).on_hover_text("List it in this project's Favourites at the bottom of the sidebar (each project has its own)").changed() {
                            star = Some(i);
                        }
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.button("Delete").clicked() {
                            remove = Some(i);
                        }
                        if ui.add_enabled(i + 1 < n, egui::Button::new("↓")).clicked() {
                            swap = Some((i, i + 1));
                        }
                        if ui.add_enabled(i > 0, egui::Button::new("↑")).clicked() {
                            swap = Some((i - 1, i));
                        }
                    });
                });
                ui.add(egui::TextEdit::multiline(&mut sn.text).hint_text("Text to insert").desired_rows(2).desired_width(f32::INFINITY).font(egui::TextStyle::Monospace));
            });
            ui.add_space(6.0);
        }
        if let Some((old, new)) = renamed {
            self.rename_fav_snippet(&old, &new);
        }
        if let Some(i) = star {
            let name = self.s.snippets[i].name.clone();
            self.toggle_fav(FavList::Snippets, &name);
        }
        if let Some(i) = remove {
            let sn = self.s.snippets.remove(i);
            // Its stars go too, unless another snippet has the same name.
            if !self.s.snippets.iter().any(|o| o.name == sn.name) {
                self.forget_fav_snippet(&sn.name);
            }
        }
        if let Some((a, b)) = swap {
            self.s.snippets.swap(a, b);
        }
        if ui.add(egui::Button::new(RichText::new("+ Add snippet").color(Color32::WHITE)).fill(ACCENT)).clicked() {
            self.s.snippets.push(Snippet { name: format!("Snippet {}", n + 1), text: String::new(), send: false });
        }
    }
}
