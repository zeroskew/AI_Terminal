//! The session header's model / effort pill opens this dialog: the Claude models with a short
//! description and the reasoning effort levels. Apply sends `/model <id>` and / or
//! `/effort <level>` to the running session.

use super::*;

/// (id, name, description) of the models offered, most capable first.
const CLAUDE_MODELS: &[(&str, &str, &str)] = &[
    ("claude-fable-5-2", "Fable 5.2", "Most capable. Best for the hardest problems, deep reasoning and large refactors; slowest and most expensive."),
    ("claude-opus-5-5", "Opus 5.5", "Very capable for complex coding and long agentic tasks. A strong default for demanding work."),
    ("claude-sonnet-5-5", "Sonnet 5.5", "Balanced speed, quality and cost. Good for everyday coding."),
    ("claude-haiku-5-5", "Haiku 5.5", "Fastest and cheapest. Good for quick questions, simple edits and small chores."),
];

/// The open dialog: the session it's for and the picks so far.
pub(super) struct ModelSwitch {
    id: u64,
    /// The listed model in use when the dialog opened (None: not one of the list).
    start_model: Option<&'static str>,
    model: Option<&'static str>,
    /// The effort in use when the dialog opened ("auto" when none is set).
    start_effort: String,
    effort: String,
}

impl ModelSwitch {
    pub(super) fn new(sess: &crate::session::Session) -> Self {
        let current = sess.model.clone().or_else(|| sess.opts.model.clone()).unwrap_or_default();
        let model = CLAUDE_MODELS.iter().map(|m| m.0).find(|mid| current.starts_with(mid));
        let effort = sess.effort().unwrap_or_else(|| "auto".to_string());
        Self { id: sess.id, start_model: model, model, start_effort: effort.clone(), effort }
    }
}

impl App {
    pub(super) fn model_switch_dialog(&mut self, ctx: &egui::Context) {
        let Some(st) = self.model_switch.as_mut() else { return };
        let id = st.id;
        let Some(t) = self.tabs.iter().find(|t| t.sess.id == id && t.sess.exited.is_none()) else {
            self.model_switch = None;
            return;
        };
        let live = t.sess.model.clone().or_else(|| t.sess.opts.model.clone());
        let catalog = self.catalog.lock().unwrap().clone().unwrap_or_default();
        let for_model = st.model.map(str::to_string).or(live);
        let levels = catalog.efforts(Kind::Claude, for_model.as_deref());
        let default = catalog.default_effort(Kind::Claude, for_model.as_deref());
        if st.effort != "auto" && !levels.contains(&st.effort) {
            st.effort = "auto".to_string();
        }
        let changed = st.model != st.start_model || st.effort != st.start_effort;
        let mut apply = false;
        let mut close = false;
        let m = egui::Modal::new(egui::Id::unique("model_switch")).frame(egui::Frame::new().fill(PANEL).corner_radius(10).stroke(Stroke::new(1.0, BORDER)).inner_margin(20)).show(ctx, |ui| {
            ui.set_width(560.0);
            ui.label(RichText::new("Model and effort").size(18.0).family(bold()).color(Color32::WHITE));
            ui.add_space(2.0);
            ui.label(RichText::new("Changing the model sends /model to this session. The conversation is kept; the next reply re-reads it without the prompt cache, so it costs a bit more once.").color(TEXT_DIM));
            ui.add_space(12.0);
            for (mid, name, desc) in CLAUDE_MODELS {
                let picked = st.model == Some(*mid);
                let is_current = st.start_model == Some(*mid);
                let fill = if picked { CARD_HI } else { CARD };
                let r = egui::Frame::new().fill(fill).corner_radius(6).inner_margin(10).stroke(Stroke::new(1.0, if picked { ACCENT_HI } else { BORDER })).show(ui, |ui| {
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
                if resp.hovered() && !picked {
                    ui.painter().rect_stroke(r.response.rect, 6.0, Stroke::new(1.0, LINK), egui::StrokeKind::Inside);
                }
                if resp.clicked() {
                    st.model = Some(mid);
                }
                if resp.double_clicked() {
                    apply = true;
                }
                ui.add_space(6.0);
            }
            ui.add_space(8.0);
            ui.label(RichText::new("Reasoning effort").size(14.5).family(bold()).color(TEXT));
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                let auto = match &default {
                    Some(d) => format!("auto ({d})"),
                    None => "auto".to_string(),
                };
                if ui.selectable_label(st.effort == "auto", auto).on_hover_text("The model's default effort").clicked() {
                    st.effort = "auto".to_string();
                }
                for e in &levels {
                    if ui.selectable_label(&st.effort == e, e).clicked() {
                        st.effort = e.clone();
                    }
                }
            });
            ui.add_space(2.0);
            ui.label(RichText::new("Sends /effort. Claude Code also saves it as your default, except max.").size(11.5).color(TEXT_FAINT));
            ui.add_space(12.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.add_enabled(changed, egui::Button::new("Apply")).clicked() {
                    apply = true;
                }
                if ui.button("Cancel").clicked() {
                    close = true;
                }
            });
        });
        if m.should_close() {
            close = true;
        }
        if apply {
            let st = self.model_switch.take().unwrap();
            let mut after = std::time::Instant::now();
            if let Some(mid) = st.model.filter(|_| st.model != st.start_model) {
                if let Some(t) = self.tabs.iter_mut().find(|t| t.sess.id == id) {
                    // Restarts / resumes keep the new model.
                    t.sess.opts.model = Some(mid.to_string());
                    t.sess.opts.one_m = false;
                }
                self.send_command_to(id, &format!("/model {mid}"));
                // Let /model finish before /effort is typed.
                after += Duration::from_millis(600);
            }
            if st.effort != st.start_effort {
                // The session notes it from the typed line (restarts / resumes keep it).
                self.cmd_due.push((id, after, format!("/effort {}", st.effort)));
            }
            close = true;
        }
        if close {
            self.model_switch = None;
            self.focus_terminal = true;
        }
    }
}
