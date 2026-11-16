//! The session header's "change" link: a dialog listing the Claude models with a short
//! description; picking one sends `/model <id>` to the running session.

use super::*;

/// (id, name, description) of the models offered, most capable first.
const CLAUDE_MODELS: &[(&str, &str, &str)] = &[
    ("claude-fable-5-2", "Fable 5.2", "Most capable. Best for the hardest problems, deep reasoning and large refactors; slowest and most expensive."),
    ("claude-opus-5-5", "Opus 5.5", "Very capable for complex coding and long agentic tasks. A strong default for demanding work."),
    ("claude-sonnet-5-5", "Sonnet 5.5", "Balanced speed, quality and cost. Good for everyday coding."),
    ("claude-haiku-5-5", "Haiku 5.5", "Fastest and cheapest. Good for quick questions, simple edits and small chores."),
];

impl App {
    pub(super) fn model_switch_dialog(&mut self, ctx: &egui::Context) {
        let Some(id) = self.model_switch else { return };
        let Some(t) = self.tabs.iter().find(|t| t.sess.id == id && t.sess.exited.is_none()) else {
            self.model_switch = None;
            return;
        };
        let current = t.sess.model.clone().or_else(|| t.sess.opts.model.clone()).unwrap_or_default();
        let mut pick: Option<&str> = None;
        let mut close = false;
        let m = egui::Modal::new(egui::Id::unique("model_switch")).frame(egui::Frame::new().fill(PANEL).corner_radius(10).stroke(Stroke::new(1.0, BORDER)).inner_margin(20)).show(ctx, |ui| {
            ui.set_width(560.0);
            ui.label(RichText::new("Change model").size(18.0).family(bold()).color(Color32::WHITE));
            ui.add_space(2.0);
            ui.label(RichText::new("Sends /model to this session. The conversation is kept; the next reply re-reads it without the prompt cache, so it costs a bit more once.").color(TEXT_DIM));
            ui.add_space(12.0);
            for (mid, name, desc) in CLAUDE_MODELS {
                let is_current = current.starts_with(mid);
                let fill = if is_current { CARD_HI } else { CARD };
                let r = egui::Frame::new().fill(fill).corner_radius(6).inner_margin(10).stroke(Stroke::new(1.0, if is_current { ACCENT_HI } else { BORDER })).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(*name).family(bold()).size(14.5).color(TEXT));
                        ui.label(RichText::new(*mid).monospace().size(11.5).color(TEXT_FAINT));
                        if is_current {
                            widgets::pill(ui, "current", Color32::WHITE, ACCENT);
                        }
                    });
                    ui.label(RichText::new(*desc).color(TEXT_DIM));
                });
                let resp = ui.interact(r.response.rect, egui::Id::unique(("model_pick", *mid)), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
                if resp.hovered() && !is_current {
                    ui.painter().rect_stroke(r.response.rect, 6.0, Stroke::new(1.0, LINK), egui::StrokeKind::Inside);
                }
                if resp.clicked() && !is_current {
                    pick = Some(mid);
                }
                ui.add_space(6.0);
            }
            ui.add_space(8.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button("Cancel").clicked() {
                    close = true;
                }
            });
        });
        if m.should_close() {
            close = true;
        }
        if let Some(mid) = pick {
            if let Some(t) = self.tabs.iter_mut().find(|t| t.sess.id == id) {
                // Restarts / resumes keep the new model.
                t.sess.opts.model = Some(mid.to_string());
                t.sess.opts.one_m = false;
            }
            self.send_command_to(id, &format!("/model {mid}"));
            close = true;
        }
        if close {
            self.model_switch = None;
            self.focus_terminal = true;
        }
    }
}
