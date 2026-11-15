//! Snippets: the palette, inserting into a session, and their Options page.

use super::*;

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
        let m = egui::Modal::new(egui::Id::unique("snippets")).frame(egui::Frame::new().fill(PANEL).corner_radius(10).stroke(Stroke::new(1.0, BORDER)).inner_margin(16)).show(ctx, |ui| {
            ui.set_width(560.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("SNIPPETS").size(12.0).family(bold()).color(TEXT_DIM));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if widgets::icon_button(ui, Icon::Close, 24.0, TEXT_DIM, "Close (Esc)").clicked() {
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
            egui::ScrollArea::vertical().max_height(420.0).show(ui, |ui| {
                for i in shown {
                    let sn = &self.s.snippets[i];
                    let (r, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 44.0), Sense::click());
                    let p = ui.painter();
                    if resp.hovered() {
                        p.rect_filled(r, 5.0, CARD_HI);
                    }
                    p.text(egui::pos2(r.left() + 10.0, r.top() + 13.0), egui::Align2::LEFT_CENTER, &sn.name, egui::FontId::new(14.0, bold()), TEXT);
                    let first = sn.text.lines().next().unwrap_or("");
                    let mut job = egui::text::LayoutJob::simple_singleline(first.to_string(), egui::FontId::monospace(12.0), TEXT_DIM);
                    job.wrap.max_width = r.width() - 110.0;
                    job.wrap.max_rows = 1;
                    job.wrap.break_anywhere = true;
                    let g = ui.fonts_mut(|f| f.layout_job(job));
                    p.galley(egui::pos2(r.left() + 10.0, r.top() + 24.0), g, TEXT_DIM);
                    if sn.send {
                        p.text(egui::pos2(r.right() - 10.0, r.center().y), egui::Align2::RIGHT_CENTER, "↵ sends", egui::FontId::proportional(11.5), TEXT_FAINT);
                    }
                    if resp.clicked() {
                        pick = Some(i);
                    }
                }
            });
        });
        if m.should_close() {
            close = true;
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
        for i in 0..n {
            egui::Frame::new().fill(CARD).corner_radius(8).stroke(Stroke::new(1.0, BORDER)).inner_margin(10).show(ui, |ui| {
                ui.set_width(ui.available_width());
                let sn = &mut self.s.snippets[i];
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut sn.name).hint_text("Name").desired_width(260.0));
                    ui.checkbox(&mut sn.send, "Press Enter after inserting");
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
        if let Some(i) = remove {
            self.s.snippets.remove(i);
        }
        if let Some((a, b)) = swap {
            self.s.snippets.swap(a, b);
        }
        if ui.add(egui::Button::new(RichText::new("+ Add snippet").color(Color32::WHITE)).fill(ACCENT)).clicked() {
            self.s.snippets.push(Snippet { name: format!("Snippet {}", n + 1), text: String::new(), send: false });
        }
    }
}
