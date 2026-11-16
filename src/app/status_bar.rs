//! The status bar at the bottom.

use super::*;

impl App {
    /// Link buttons for a running Claude, Codex or Antigravity session: compact, context,
    /// clear and MCP servers, each sent as that CLI's own slash command.
    fn session_links(&mut self, ui: &mut Ui) {
        let Some(t) = self.active_tab() else { return };
        let kind = t.sess.profile.kind;
        if t.sess.exited.is_some() || !matches!(kind, Kind::Claude | Kind::Codex | Kind::Antigravity) {
            return;
        }
        let context_link = match kind {
            Kind::Codex => ("/status", "Run /status: context window use, model, and rate limits"),
            Kind::Antigravity => ("/context", "Run /context: show the files in the conversation"),
            _ => ("/context", "Run /context: show what's using the context window (system prompt, tools, MCP, memory, messages)"),
        };
        let project = t.project.clone();
        ui.add_space(14.0);
        let sep = |ui: &mut Ui| {
            ui.label(RichText::new("|").color(BORDER).size(12.5));
        };
        sep(ui);
        let id = t.sess.id;
        let mut send = None;
        for (label, cmd, tip) in [
            ("compact", "/compact", "Run /compact: replace the conversation so far with a summary, so each reply re-sends less"),
            ("context", context_link.0, context_link.1),
        ] {
            if link_button(ui, label, tip) {
                send = Some(cmd);
            }
            sep(ui);
        }
        if link_button(ui, "clear", "Run /clear: start a fresh conversation in this session (asks first)") {
            self.confirm_clear = Some(id);
        }
        sep(ui);
        if kind == Kind::Claude {
            if link_button(ui, "MCP servers", "Choose which MCP servers are enabled for this project") {
                self.open_mcp_servers(project, &ui.ctx().clone());
            }
        } else if link_button(ui, "MCP servers", "Run /mcp: the MCP servers this session has") {
            send = Some("/mcp");
        }
        if let Some(cmd) = send {
            self.send_command(cmd);
        }
    }

    /// Types a slash command into the active session and presses Enter a moment later (like a
    /// snippet with "send").
    fn send_command(&mut self, cmd: &str) {
        if let Some(id) = self.active {
            self.send_command_to(id, cmd);
        }
    }

    pub(super) fn send_command_to(&mut self, id: u64, cmd: &str) {
        let Some(tab) = self.tabs.iter_mut().find(|t| t.sess.id == id) else { return };
        tab.sess.term.lock().scroll_display(alacritty_terminal::grid::Scroll::Bottom);
        tab.sess.write(cmd.as_bytes().to_vec());
        let id = tab.sess.id;
        self.enter_due.push((id, std::time::Instant::now() + Duration::from_millis(200)));
        self.focus_terminal = true;
    }

    /// Confirmation for the status bar's "clear" link: what /clear does before it's sent.
    pub(super) fn clear_dialog(&mut self, ctx: &egui::Context) {
        let Some(id) = self.confirm_clear else { return };
        let Some(t) = self.tabs.iter().find(|t| t.sess.id == id && t.sess.exited.is_none()) else {
            self.confirm_clear = None;
            return;
        };
        let name = t.sess.transcript_title.clone().unwrap_or_else(|| t.sess.profile.name.clone());
        let busy = t.sess.busy;
        let kind = t.sess.profile.kind;
        let agent = t.sess.profile.name.clone();
        let notes = if kind == Kind::Claude { "CLAUDE.md" } else { "AGENTS.md" };
        let mut choice: Option<Option<&str>> = None;
        let m = egui::Modal::new(egui::Id::unique("confirm_clear")).frame(egui::Frame::new().fill(PANEL).corner_radius(10).stroke(Stroke::new(1.0, BORDER)).inner_margin(20)).show(ctx, |ui| {
            ui.set_width(560.0);
            ui.label(RichText::new("Clear the conversation?").size(18.0).family(bold()).color(Color32::WHITE));
            ui.add_space(2.0);
            ui.label(RichText::new(format!("Sends /clear to \"{name}\". {agent} starts a fresh conversation in this session.")).color(TEXT_DIM));
            ui.add_space(12.0);
            let point = |ui: &mut Ui, head: &str, body: &str, col: Color32| {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    ui.label(RichText::new("•").color(col));
                    ui.label(RichText::new(head).family(bold()).color(col));
                    ui.label(RichText::new(body).color(TEXT));
                });
                ui.add_space(4.0);
            };
            point(ui, "Forgotten:", "everything said in this conversation so far. The next message starts from an empty context, so earlier messages are no longer re-sent (and paid for) with every request.", ORANGE);
            point(ui, "Kept:", &format!("your files and code changes, {notes}, settings, permissions and MCP servers."), GREEN);
            if kind == Kind::Antigravity {
                point(ui, "Saved:", "the old conversation stays on disk.", GREEN);
            } else {
                point(ui, "Saved:", "the old conversation stays on disk; /resume in the session opens it again.", GREEN);
            }
            if kind == Kind::Claude {
                point(ui, "In AI Terminal:", "this session switches to the new conversation (its title and cost start over; resume after a restart opens the new one).", TEXT_DIM);
            }
            ui.add_space(8.0);
            ui.label(RichText::new(format!("Something worth keeping? Ask {agent} to note it in {notes} or write a handoff first, or use Compact, which keeps a summary instead of nothing.")).color(TEXT_FAINT).size(12.5));
            if busy {
                ui.add_space(6.0);
                ui.label(RichText::new(format!("{agent} is still working on a reply; /clear will be typed into its prompt now.")).color(ORANGE).size(12.5));
            }
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.add(egui::Button::new(RichText::new("Clear").color(Color32::WHITE)).fill(RED).min_size(Vec2::new(80.0, 28.0))).on_hover_text("Send /clear").clicked() {
                        choice = Some(Some("/clear"));
                    }
                    if ui.add(egui::Button::new("Compact instead").min_size(Vec2::new(0.0, 28.0))).on_hover_text("Send /compact: replace the conversation with a summary").clicked() {
                        choice = Some(Some("/compact"));
                    }
                    if ui.add(egui::Button::new("Cancel").min_size(Vec2::new(70.0, 28.0))).clicked() {
                        choice = Some(None);
                    }
                });
            });
        });
        if m.should_close() && choice.is_none() {
            choice = Some(None);
        }
        if let Some(c) = choice {
            self.confirm_clear = None;
            if let Some(cmd) = c {
                self.send_command_to(id, cmd);
            }
        }
    }

    pub(super) fn status_bar(&mut self, ui: &mut Ui) {
        ui.horizontal_centered(|ui| {
            let (dot, msg) = match self.active_tab() {
                Some(t) => match t.sess.exited {
                    Some(c) => (TEXT_FAINT, format!("{} exited with code {c}", t.sess.profile.name)),
                    None => (GREEN, format!("{}  ·  {}×{}", t.sess.profile.name, t.sess.size.cols, t.sess.size.rows)),
                },
                None => (TEXT_FAINT, "No session — Ctrl+N starts a Claude session".to_string()),
            };
            let (r, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), Sense::hover());
            ui.painter().circle_filled(r.center(), 4.0, dot);
            ui.label(RichText::new(msg).color(TEXT).size(12.5));
            self.usage_limits(ui);
            self.session_links(ui);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(RichText::new("Ctrl+V pastes images  ·  Ctrl+N new Claude session  ·  Ctrl+Tab switch").color(TEXT_DIM).size(12.5));
                if let Some(t) = self.active_tab() {
                    let n = t.sess.pasted_images.len();
                    if n > 0 {
                        ui.label(RichText::new(format!("{n} image{} pasted   ", if n == 1 { "" } else { "s" })).color(TEXT_DIM).size(12.5));
                    }
                }
            });
        });
    }

    /// Plan usage in the status bar for the active session's provider: how much of the plan
    /// limits is left. Claude (Anthropic): Claude Code passes the numbers to its status line
    /// command (our `--statusline`), which saves them to a file. Codex (OpenAI): read from its
    /// newest conversation file, every few seconds. Antigravity (Google): `agy -p /usage`, once a
    /// minute on a thread. The other agents don't report theirs, so nothing is shown.
    pub(super) fn usage_limits(&mut self, ui: &mut Ui) {
        let now = std::time::Instant::now();
        if self.limits_read.is_none_or(|(t, _)| now.duration_since(t).as_secs() >= 3) {
            let mtime = std::fs::metadata(crate::hooks::limits_path()).and_then(|m| m.modified()).ok();
            if self.limits_read.is_none_or(|(_, m)| m != mtime) {
                self.limits = crate::hooks::read_limits();
            }
            let pmtime = std::fs::metadata(crate::hooks::plan_usage_path()).and_then(|m| m.modified()).ok();
            if pmtime != self.plan_usage_read {
                self.plan_usage = crate::hooks::read_plan_usage();
                self.plan_usage_read = pmtime;
            }
            self.limits_read = Some((now, mtime));
        }
        // No session: Claude's (Ctrl+N starts one).
        let kind = self.active_tab().map_or(Kind::Claude, |t| t.sess.profile.kind);
        if kind == Kind::Codex && self.codex_limits_read.is_none_or(|t| now.duration_since(t).as_secs() >= 5) {
            self.codex_limits = agents::codex_limits();
            self.codex_limits_read = Some(now);
        }
        // Antigravity: `agy -p /usage` takes a couple of seconds, so a thread asks once a minute.
        if kind == Kind::Antigravity && self.agy_quota_asked.is_none_or(|t| now.duration_since(t).as_secs() >= 60) {
            self.agy_quota_asked = Some(now);
            if let Some(t) = self.active_tab() {
                let (program, slot, ctx) = (t.sess.profile.program.clone(), self.agy_quota.clone(), ui.ctx().clone());
                std::thread::spawn(move || {
                    let rows = agents::antigravity_usage(&program);
                    if !rows.is_empty() {
                        *slot.lock().unwrap() = rows;
                        ctx.request_repaint();
                    }
                });
            }
        }
        // Keep the countdowns (and the file check) going while the app is idle.
        ui.ctx().request_repaint_after(std::time::Duration::from_secs(30));
        let (provider, source, windows): (&str, String, Vec<agents::LimitWindow>) = match kind {
            Kind::Claude => {
                let Some(l) = self.limits else { return };
                let w = [("5h", "5-hour limit", l.five_hour), ("Week", "Weekly limit", l.seven_day)];
                ("Anthropic", "Updated by Claude Code's status line after each reply.".into(), w.into_iter().filter_map(|(a, b, w)| Some((a.into(), b.into(), w?))).collect())
            }
            Kind::Codex => ("OpenAI", "Read from Codex's newest conversation file (updated after each reply).".into(), self.codex_limits.clone()),
            Kind::Antigravity => {
                let rows = self.agy_quota.lock().unwrap().clone();
                let model = self.active_tab().and_then(|t| t.sess.model.clone().or_else(|| t.sess.opts.model.clone())).unwrap_or_default();
                let (group, w) = agents::antigravity_limits(&rows, &model);
                let all: Vec<String> = rows.iter().map(|r| format!("  {} · {}: {:.0}% left", r.group, r.name, r.left)).collect();
                ("Google", format!("{group}. From `agy -p /usage`, asked once a minute.\n\nAll Antigravity quotas:\n{}", all.join("\n")), w)
            }
            _ => return,
        };
        if windows.is_empty() {
            return;
        }
        let unix_now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let human = |secs: u64| {
            let (d, h, m) = (secs / 86_400, secs % 86_400 / 3600, secs % 3600 / 60);
            if d > 0 { format!("{d}d {h}h") } else if h > 0 { format!("{h}h {m}m") } else { format!("{}m", m.max(1)) }
        };
        ui.add_space(14.0);
        ui.label(RichText::new(provider).color(TEXT_FAINT).size(12.5)).on_hover_text(format!("{provider} plan limits"));
        for (i, (name, long, w)) in windows.iter().enumerate() {
            // Past its reset time the window has started over.
            let expired = w.resets_at.is_some_and(|r| r <= unix_now);
            let left = if expired { 100.0 } else { (100.0 - w.used).clamp(0.0, 100.0) };
            let col = if left > 50.0 { GREEN } else if left > 20.0 { ORANGE } else { RED };
            let resets = w.resets_at.filter(|_| !expired).map(|r| human(r - unix_now));
            ui.add_space(if i == 0 { 2.0 } else { 14.0 });
            let r = ui
                .horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    ui.label(RichText::new(name).color(TEXT_DIM).size(12.5));
                    let (bar, _) = ui.allocate_exact_size(egui::vec2(46.0, 6.0), Sense::hover());
                    ui.painter().rect_filled(bar, 3.0, BORDER);
                    let mut fill = bar;
                    fill.set_width(bar.width() * (left / 100.0) as f32);
                    ui.painter().rect_filled(fill, 3.0, col);
                    ui.label(RichText::new(format!("{left:.0}% left")).color(col).size(12.5));
                    if let Some(t) = &resets {
                        ui.label(RichText::new(format!("· resets in {t}")).color(TEXT_FAINT).size(12.5));
                    }
                })
                .response;
            let tip = match &resets {
                Some(t) => format!("{long}: {:.0}% used, {left:.0}% left. Resets in {t}.", w.used),
                None if expired => format!("{long}: reset since the last update."),
                None => format!("{long}: {:.0}% used, {left:.0}% left.", w.used),
            };
            r.on_hover_text(format!("{provider} {tip}
{source}"));
        }
    }
}

/// A small text link (underlined on hover); true when clicked.
pub(super) fn link_button(ui: &mut Ui, text: &str, tip: &str) -> bool {
    let r = ui.add(egui::Label::new(RichText::new(text).color(LINK).size(12.5)).sense(Sense::click())).on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tip);
    if r.hovered() {
        ui.painter().hline(r.rect.x_range(), r.rect.bottom(), Stroke::new(1.0, LINK));
    }
    r.clicked()
}
