//! The context pill's window: Claude Code's `/context` report for the session (what each
//! request sends, by category), the usage and token counts, and a Compact button.
//!
//! The report comes from `claude -p /context --resume <id> --fork-session
//! --no-session-persistence`, run in the background: it's computed by Claude Code itself
//! without calling the model, and nothing is written to the conversation or saved to disk.

use super::*;
use std::sync::{Arc, Mutex};

/// The open window: the session, and the `/context` report once it's in (Claude only).
pub(super) struct ContextInfo {
    pub id: u64,
    report: Arc<Mutex<Option<Result<String, String>>>>,
    /// Whether a `/context` run was started (only for Claude sessions with a conversation).
    asked: bool,
}

impl ContextInfo {
    pub fn new(id: u64) -> Self {
        Self { id, report: Default::default(), asked: false }
    }
}

/// The details shown for a session's context pill: the context size, the plan usage (or the
/// estimated cost) and the token counts. `plan` is `None` when usage is shown as cost,
/// `Some(None)` when no plan usage has been counted yet.
pub(super) fn details(c: &crate::cost::Summary, plan: Option<Option<crate::hooks::PlanShare>>) -> String {
    let u = c.usage;
    let about = match plan {
        Some(Some(p)) => format!(
            "Plan usage of this conversation: {} of a 5-hour limit, {} of the weekly limit.\nCounted from Claude Code's plan usage after each reply; approximate while other sessions work at the same time.\nEstimated API cost (not charged on a subscription): {}\n\nTokens, from the transcript:",
            fmt_plan(p.five_hour),
            fmt_plan(p.seven_day),
            crate::cost::fmt_usd(c.usd)
        ),
        Some(None) => format!("No plan usage counted for this conversation yet (it updates after Claude's next reply).\nEstimated API cost (not charged on a subscription): {}\n\nTokens, from the transcript:", crate::cost::fmt_usd(c.usd)),
        None => format!("Estimated cost at API prices: {}\n\nTokens, from the transcript:", crate::cost::fmt_usd(c.usd)),
    };
    format!(
        "Context: {} tokens sent with every request (system prompt, tools, memory and the conversation so far), from the latest request.\n\n{about}\nInput {}  ·  output {}\nCache read {}  ·  cache write {}{}",
        crate::cost::fmt_tokens(c.context),
        crate::cost::fmt_tokens(u.input),
        crate::cost::fmt_tokens(u.output),
        crate::cost::fmt_tokens(u.cache_read),
        crate::cost::fmt_tokens(u.cache_write),
        if u.web_searches > 0 { format!("\nWeb searches {}", u.web_searches) } else { String::new() },
    )
}

/// Runs `/context` for a Claude conversation without touching it; the report is markdown.
fn run_context(program: String, pre: Vec<String>, cwd: PathBuf, agent_id: String, model: Option<String>) -> Result<String, String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut cmd = std::process::Command::new(&program);
    cmd.args(&pre).args(["-p", "/context", "--resume", &agent_id, "--fork-session", "--no-session-persistence"]);
    if let Some(m) = &model {
        cmd.args(["--model", m]);
    }
    // Not reported as the session's own hook events.
    cmd.env_remove(crate::hooks::ENV_KEY).current_dir(&cwd).creation_flags(CREATE_NO_WINDOW).stdin(std::process::Stdio::null());
    let out = cmd.output().map_err(|e| format!("Couldn't run Claude Code: {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if out.status.success() && !text.is_empty() {
        Ok(text)
    } else {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        Err(if err.is_empty() { if text.is_empty() { "Claude Code printed nothing.".into() } else { text } } else { err })
    }
}

/// Draws the `/context` markdown: headings, `**bold**` runs, and tables as grids.
fn show_markdown(ui: &mut Ui, md: &str) {
    let lines: Vec<&str> = md.lines().collect();
    let mut i = 0;
    let mut table_n = 0;
    while i < lines.len() {
        let line = lines[i].trim_end();
        let t = line.trim_start();
        if t.starts_with('|') {
            let mut rows: Vec<Vec<String>> = Vec::new();
            while i < lines.len() && lines[i].trim_start().starts_with('|') {
                let r = lines[i].trim();
                let cells: Vec<String> = r.trim_matches('|').split('|').map(|c| c.trim().to_string()).collect();
                if !cells.iter().all(|c| !c.is_empty() && c.chars().all(|ch| ch == '-' || ch == ':')) {
                    rows.push(cells);
                }
                i += 1;
            }
            table_n += 1;
            egui::Grid::new(("context_table", table_n)).striped(true).spacing([18.0, 3.0]).show(ui, |ui| {
                for (n, row) in rows.iter().enumerate() {
                    for (k, cell) in row.iter().enumerate() {
                        let mut rt = RichText::new(cell.replace("**", "")).size(12.5);
                        rt = if n == 0 { rt.family(bold()).color(TEXT) } else { rt.color(if k == 0 { TEXT } else { TEXT_DIM }) };
                        ui.add(egui::Label::new(rt).wrap_mode(egui::TextWrapMode::Extend));
                    }
                    ui.end_row();
                }
            });
            ui.add_space(6.0);
            continue;
        }
        if let Some(h) = t.strip_prefix("### ") {
            ui.add_space(6.0);
            ui.label(RichText::new(h).family(bold()).size(14.0).color(Color32::WHITE));
            ui.add_space(2.0);
        } else if let Some(h) = t.strip_prefix("## ").or_else(|| t.strip_prefix("# ")) {
            ui.label(RichText::new(h).family(bold()).size(16.0).color(Color32::WHITE));
            ui.add_space(4.0);
        } else if !t.is_empty() {
            // `**Label:** value` runs, alternating plain and bold.
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                for (k, part) in t.split("**").enumerate() {
                    if !part.is_empty() {
                        let rt = RichText::new(part).color(TEXT);
                        ui.label(if k % 2 == 1 { rt.family(bold()) } else { rt });
                    }
                }
            });
        }
        i += 1;
    }
}

impl App {
    /// The window opened by clicking the context pill.
    pub(super) fn context_info_dialog(&mut self, ctx: &egui::Context) {
        let Some(info) = &mut self.context_info else { return };
        let id = info.id;
        let Some(tab) = self.tabs.iter().find(|t| t.sess.id == id) else {
            self.context_info = None;
            return;
        };
        let name = tab.sess.display_name();
        let kind = tab.sess.profile.kind;
        let plan = if self.s.usage_as == "plan" { Some(tab.sess.agent_id.as_ref().and_then(|a| self.plan_usage.get(a)).copied()) } else { None };
        let text = tab.sess.cost.map(|c| details(&c, plan));
        let context = tab.sess.cost.map(|c| c.context).unwrap_or(0);
        let can_compact = tab.sess.exited.is_none() && (kind == Kind::Claude || kind == Kind::Codex) && context > 0;
        let busy = tab.sess.busy;
        // Claude: ask Claude Code for its /context report (again after Refresh).
        let can_report = kind == Kind::Claude && tab.sess.agent_id.is_some();
        if can_report && !info.asked {
            info.asked = true;
            let program = tab.sess.profile.program.clone();
            let pre = tab.sess.profile.args.clone();
            let cwd = tab.sess.cwd.clone();
            let agent_id = tab.sess.agent_id.clone().unwrap_or_default();
            let o = &tab.sess.opts;
            let model = o.model.clone().filter(|m| !m.is_empty()).or_else(|| tab.sess.model.clone()).map(|m| if o.one_m && !m.ends_with("[1m]") { format!("{m}[1m]") } else { m });
            let (slot, ctx2) = (info.report.clone(), ctx.clone());
            *slot.lock().unwrap() = None;
            std::thread::spawn(move || {
                let r = run_context(program, pre, cwd, agent_id, model);
                *slot.lock().unwrap() = Some(r);
                ctx2.request_repaint();
            });
        }
        let report = info.report.lock().unwrap().clone();
        let mut compact = false;
        let mut close = false;
        let mut refresh = false;
        let m = egui::Modal::new(egui::Id::unique("context_info")).frame(egui::Frame::new().fill(PANEL).corner_radius(10).stroke(Stroke::new(1.0, BORDER)).inner_margin(20)).show(ctx, |ui| {
            let screen = ctx.content_rect();
            ui.set_width(760.0_f32.min(screen.width() - 80.0));
            ui.horizontal(|ui| {
                ui.label(RichText::new("Context").size(18.0).family(bold()).color(Color32::WHITE));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if widgets::icon_button(ui, Icon::Close, 24.0, TEXT_DIM, "Close (Esc)").clicked() {
                        close = true;
                    }
                });
            });
            ui.add_space(2.0);
            ui.label(RichText::new(&name).family(bold()).color(TEXT));
            ui.add_space(10.0);
            let max_h = (screen.height() - 300.0).clamp(160.0, 480.0);
            // A solid scroll bar that takes its own space: the app's floating ones are drawn
            // over the content, on top of the table's last column.
            ui.scope(|ui| {
                ui.spacing_mut().scroll = egui::style::ScrollStyle::solid();
                egui::ScrollArea::both().max_height(max_h).auto_shrink([false, true]).show_hinted(ui, |ui| {
                    if can_report {
                        match &report {
                            None => {
                                ui.horizontal(|ui| {
                                    ui.spinner();
                                    ui.label(RichText::new("Running /context…").color(TEXT_DIM));
                                });
                            }
                            Some(Ok(md)) => show_markdown(ui, md),
                            Some(Err(e)) => {
                                ui.label(RichText::new("Couldn't get the /context report:").color(ORANGE));
                                ui.label(RichText::new(e).size(12.0).color(TEXT_DIM));
                            }
                        }
                        if let Some(t) = &text {
                            ui.add_space(8.0);
                            egui::CollapsingHeader::new(RichText::new("Usage and tokens").color(TEXT)).id_salt("context_usage").show(ui, |ui| {
                                ui.label(RichText::new(t).color(TEXT_DIM));
                            });
                        }
                    } else if let Some(t) = &text {
                        ui.label(RichText::new(t).color(TEXT_DIM));
                    }
                    // Keeps the last column clear of the scroll bar's edge.
                    ui.add_space(4.0);
                });
            });
            ui.add_space(10.0);
            ui.label(RichText::new("Compact replaces the conversation so far with a summary, so each request sends less context.").size(12.0).color(TEXT_FAINT));
            if busy && can_compact {
                ui.label(RichText::new("It's still working on a reply; /compact will be typed into its prompt now.").size(12.0).color(ORANGE));
            }
            ui.add_space(14.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let tip = if can_compact { "Run /compact in this session" } else { "Not available for this session" };
                let b = ui.add_enabled(can_compact, egui::Button::new(RichText::new("Compact").color(Color32::WHITE)).fill(ACCENT)).on_hover_text(tip).on_disabled_hover_text(tip);
                if b.clicked() {
                    compact = true;
                }
                if can_report && ui.add_enabled(report.is_some(), egui::Button::new("Refresh")).on_hover_text("Run /context again").clicked() {
                    refresh = true;
                }
            });
        });
        if compact {
            self.context_info = None;
            self.send_command_to(id, "/compact");
        } else if close || m.should_close() {
            self.context_info = None;
        } else if refresh {
            if let Some(info) = &mut self.context_info {
                info.asked = false;
            }
        }
    }
}
