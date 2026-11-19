//! The Search window: finds text in the messages of the current session, or of every saved
//! session (all projects), within the last N days; lists the sessions it's in, and opens them.

use super::*;
use crate::search::{self, Target};
use std::sync::Arc;

#[derive(Default)]
pub(super) struct SearchUi {
    query: String,
    focus: bool,
    job: Option<Arc<search::Job>>,
    /// What the shown results were searched with: (all sessions, days).
    searched: Option<(bool, u32)>,
    /// Conversations whose matches are all shown (else the first few).
    expanded: std::collections::HashSet<String>,
}

impl SearchUi {
    pub(super) fn set_query(&mut self, q: &str) {
        self.query = q.to_string();
    }
}

/// A session the app lists (running or not), found by conversation id.
struct SessInfo {
    name: String,
    project: PathBuf,
    profile: Profile,
    live: bool,
    archived: bool,
    later: bool,
}

/// Matches shown per session until "show all" is clicked.
const FEW: usize = 3;

/// "Today 14:03", "Yesterday 09:12", "2026-11-03 18:40".
pub(super) fn fmt_time(secs: u64) -> String {
    let local = secs as i64 + utc_offset();
    let (day, sod) = (local.div_euclid(86_400), local.rem_euclid(86_400));
    let hm = format!("{:02}:{:02}", sod / 3600, sod % 3600 / 60);
    match local_day(now_secs()) - day {
        0 => format!("Today {hm}"),
        1 => format!("Yesterday {hm}"),
        _ => {
            let (y, m, d) = agents::civil_date(day);
            format!("{y}-{m:02}-{d:02} {hm}")
        }
    }
}

/// The snippet with the match highlighted.
fn snippet_job(h: &search::Hit, wrap: f32) -> egui::text::LayoutJob {
    use egui::text::{LayoutJob, TextFormat};
    let font = egui::FontId::proportional(13.0);
    let plain = TextFormat { font_id: font.clone(), color: TEXT_DIM, ..Default::default() };
    let mark = TextFormat { font_id: font, color: Color32::WHITE, background: ACCENT.gamma_multiply(0.75), ..Default::default() };
    let (a, b) = h.range;
    let mut job = LayoutJob::default();
    job.append(&h.snippet[..a], 0.0, plain.clone());
    job.append(&h.snippet[a..b], 0.0, mark);
    job.append(&h.snippet[b..], 0.0, plain);
    job.wrap.max_width = wrap;
    job.wrap.max_rows = 3;
    job
}

impl App {
    pub(super) fn open_search(&mut self) {
        let st = self.search_ui.get_or_insert_with(Default::default);
        st.focus = true;
        // Text selected in the terminal is a likely query.
        if st.query.is_empty() {
            if let Some(sel) = self.active_tab().and_then(|t| t.sess.term.lock().selection_to_string()) {
                let sel = sel.trim();
                if !sel.is_empty() && !sel.contains('\n') && sel.chars().count() <= 200 {
                    self.search_ui.as_mut().unwrap().query = sel.to_string();
                }
            }
        }
    }

    fn session_by_agent(&self, agent_id: &str) -> Option<SessInfo> {
        if let Some(t) = self.tabs.iter().find(|t| t.sess.agent_id.as_deref() == Some(agent_id)) {
            return Some(SessInfo { name: t.sess.display_name(), project: t.project.clone(), profile: t.sess.profile.clone(), live: t.sess.exited.is_none(), archived: false, later: false });
        }
        let d = self.dormant.iter().find(|d| d.saved.agent_id.as_deref() == Some(agent_id))?;
        Some(SessInfo { name: d.display_name(), project: d.saved.project.clone(), profile: d.saved.profile.clone(), live: false, archived: d.saved.archived, later: d.saved.later })
    }

    /// The active session's conversation, when it has one to search.
    fn active_target(&self) -> Option<Target> {
        let t = self.active_tab()?;
        let kind = t.sess.profile.kind;
        if !kind.tracks_conversation() {
            return None;
        }
        Some(Target { agent_id: t.sess.agent_id.clone()?, kind, project: t.project.clone() })
    }

    /// Every listed session with a conversation (running, listed and archived; all projects).
    fn all_targets(&self) -> Vec<Target> {
        let mut seen = std::collections::HashSet::new();
        let live = self.tabs.iter().filter_map(|t| Some((t.sess.agent_id.clone()?, t.sess.profile.kind, t.project.clone())));
        let listed = self.dormant.iter().filter_map(|d| Some((d.saved.agent_id.clone()?, d.saved.profile.kind, d.saved.project.clone())));
        live.chain(listed)
            .filter(|(_, k, _)| k.tracks_conversation())
            .filter(|(id, _, _)| seen.insert(id.clone()))
            .map(|(agent_id, kind, project)| Target { agent_id, kind, project })
            .collect()
    }

    pub(super) fn run_search(&mut self, ctx: &egui::Context) {
        let all = self.s.search_all || self.active_target().is_none();
        let targets = if all { self.all_targets() } else { self.active_target().into_iter().collect() };
        let days = self.s.search_days;
        let Some(st) = self.search_ui.as_mut() else { return };
        let q = st.query.trim().to_string();
        st.expanded.clear();
        if q.is_empty() {
            st.job = None;
            st.searched = None;
            return;
        }
        st.job = Some(search::start(targets, q, days, ctx.clone()));
        st.searched = Some((all, days));
    }

    /// Opens a found session (resumes it if it isn't running) and shows the terminal's find bar
    /// with the query.
    fn jump_to(&mut self, ctx: &egui::Context, agent_id: &str, query: &str) {
        let id = if let Some(t) = self.tabs.iter().find(|t| t.sess.agent_id.as_deref() == Some(agent_id)) {
            t.sess.id
        } else if let Some(d) = self.dormant.iter().find(|d| d.saved.agent_id.as_deref() == Some(agent_id)) {
            let id = d.id;
            let before = self.active;
            self.resume(ctx, id);
            match self.active.filter(|a| Some(*a) != before) {
                Some(a) => a,
                None => return,
            }
        } else {
            return self.toast("That session isn't in the list any more".into(), RED, ctx);
        };
        self.select_session(id);
        if let Some(t) = self.active_tab_mut() {
            t.view.search.open = true;
            t.view.search.focus = false;
            t.view.search.query = query.to_string();
        }
    }

    pub(super) fn search_window(&mut self, ctx: &egui::Context) {
        let Some(st) = self.search_ui.as_mut() else { return };
        let mut query = std::mem::take(&mut st.query);
        let focus = std::mem::take(&mut st.focus);
        let job = st.job.clone();
        let searched = st.searched;
        let mut expanded = std::mem::take(&mut st.expanded);

        let active = self.active_target();
        let active_name = self.active_tab().map(|t| t.sess.display_name());
        let mut all = self.s.search_all;
        let mut days = self.s.search_days;
        let mut go = false;
        let mut close = false;
        let mut jump: Option<String> = None;

        let mut open = true;
        // Most of the main window by default (capped on very large screens), centred.
        let screen = ctx.content_rect();
        let size = Vec2::new((screen.width() * 0.8).clamp(480.0, 1500.0), (screen.height() * 0.82).clamp(260.0, 1100.0));
        let frame = egui::Frame::new().fill(PANEL).corner_radius(10).stroke(Stroke::new(1.0, BORDER)).inner_margin(16);
        // Foreground and kept on top so the terminal's find bar (a Foreground area) can't draw over it.
        let win_id = egui::Id::unique("search_window_v2");
        ctx.move_to_top(egui::LayerId::new(egui::Order::Foreground, win_id));
        egui::Window::new("Search")
            .id(win_id)
            .order(egui::Order::Foreground)
            .open(&mut open)
            .title_bar(false)
            .frame(frame)
            .default_size(size)
            .min_size([480.0, 260.0])
            .resizable(true)
            .collapsible(false)
            .default_pos(screen.center() - size / 2.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("SEARCH").size(12.5).family(bold()).color(TEXT_DIM));
                    ui.label(RichText::new("messages in session transcripts").size(12.5).color(TEXT_FAINT));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if widgets::corner_close_button(ui, 16.0).clicked() {
                            close = true;
                        }
                    });
                });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let w = ui.available_width() - 90.0;
                    let edit = ui.add(egui::TextEdit::singleline(&mut query).hint_text("Find text…").desired_width(w).margin(egui::Margin::symmetric(8, 5)));
                    if focus {
                        edit.request_focus();
                    }
                    if edit.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                        go = true;
                        edit.request_focus();
                    }
                    if edit.has_focus() && ui.input(|i| i.key_pressed(Key::Escape)) {
                        close = true;
                    }
                    if ui.add_enabled(!query.trim().is_empty(), egui::Button::new(RichText::new("Search").color(Color32::WHITE)).fill(ACCENT).min_size(Vec2::new(80.0, 28.0))).clicked() {
                        go = true;
                    }
                });
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    let forced = active.is_none();
                    let mut shown = all || forced;
                    let tip = if forced { "No session with a Claude / Codex / Antigravity conversation is open, so every session is searched" } else { "Search every session in every project (running, listed and archived); off = only the current session" };
                    if ui.add_enabled(!forced, egui::Checkbox::new(&mut shown, "All sessions")).on_hover_text(tip).on_disabled_hover_text(tip).changed() {
                        all = shown;
                    }
                    if !(all || forced) {
                        if let Some(n) = &active_name {
                            ui.label(RichText::new(format!("(only “{n}”)")).color(TEXT_FAINT).size(12.5));
                        }
                    }
                    ui.add_space(16.0);
                    ui.label(RichText::new("Days back").color(TEXT_DIM));
                    ui.add(egui::DragValue::new(&mut days).range(0..=3650).speed(0.2)).on_hover_text("Only messages from the last N days. 0 = everything there is.");
                    ui.label(RichText::new(if days == 0 { "all available" } else if days == 1 { "day" } else { "days" }).color(TEXT_FAINT).size(12.5));
                });
                ui.add_space(8.0);
                ui.separator();

                // Status line.
                let found: Vec<search::Found> = job.as_ref().map(|j| j.found.lock().unwrap().clone()).unwrap_or_default();
                if let Some(j) = &job {
                    ui.horizontal(|ui| {
                        let total: usize = found.iter().map(|f| f.count).sum();
                        let summary = format!("{total} message{} in {} session{}", if total == 1 { "" } else { "s" }, found.len(), if found.len() == 1 { "" } else { "s" });
                        if !j.is_done() {
                            ui.spinner();
                            ui.label(RichText::new(format!("Searching {} of {}…  {summary}", j.scanned.load(std::sync::atomic::Ordering::Relaxed), j.total)).color(TEXT_DIM));
                        } else if found.is_empty() {
                            ui.label(RichText::new(format!("No messages contain “{}”.", j.query)).color(TEXT_DIM));
                        } else {
                            ui.label(RichText::new(summary).color(TEXT_DIM));
                        }
                        if searched.is_some_and(|(a, d)| a != (all || active.is_none()) || d != days) && j.is_done() {
                            ui.label(RichText::new("· settings changed, press Enter to search again").color(ORANGE).size(12.5));
                        }
                    });
                    ui.add_space(4.0);
                }

                // Results: one card per session, newest match first.
                let mut found = found;
                found.sort_by(|a, b| b.newest.cmp(&a.newest));
                let wrap = ui.available_width() - 40.0;
                egui::ScrollArea::vertical().auto_shrink([false, false]).show_hinted(ui, |ui| {
                    for f in &found {
                        let info = self.session_by_agent(&f.agent_id);
                        let (name, project, icon, state) = match &info {
                            Some(i) => {
                                let state = if i.live { "running" } else if i.archived { "archived" } else if i.later { "saved for later" } else { "" };
                                (i.name.clone(), self.s.projects.iter().find(|p| p.path == i.project).map(|p| p.name.clone()).unwrap_or_else(|| project_name(&i.project)), agent_icon(&i.profile), state)
                            }
                            None => (f.agent_id.clone(), String::new(), (Icon::Prompt, TEXT_FAINT), ""),
                        };
                        egui::Frame::new().fill(CARD).corner_radius(6).inner_margin(egui::Margin::symmetric(10, 8)).show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            // Header: icon, name, project · state; count and Open on the right.
                            let head = ui.horizontal(|ui| {
                                let (r, _) = ui.allocate_exact_size(Vec2::splat(18.0), Sense::hover());
                                widgets::paint_icon(ui.painter(), icon.0, r.center(), 15.0, icon.1);
                                ui.label(RichText::new(&name).family(bold()).color(TEXT));
                                let mut sub = project.clone();
                                if !state.is_empty() {
                                    sub = if sub.is_empty() { state.to_string() } else { format!("{sub} · {state}") };
                                }
                                if !sub.is_empty() {
                                    ui.label(RichText::new(sub).color(TEXT_FAINT).size(12.5));
                                }
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    let tip = if info.as_ref().is_some_and(|i| i.live) { "Show this session" } else { "Resume this session" };
                                    if ui.add(egui::Button::new(RichText::new("Open").color(Color32::WHITE)).fill(ACCENT)).on_hover_text(tip).clicked() {
                                        jump = Some(f.agent_id.clone());
                                    }
                                    ui.label(RichText::new(format!("{} match{}", f.count, if f.count == 1 { "" } else { "es" })).color(TEXT_DIM).size(12.5));
                                });
                            });
                            let _ = head;
                            ui.add_space(4.0);
                            let all_shown = expanded.contains(&f.agent_id);
                            let n = if all_shown { f.hits.len() } else { f.hits.len().min(FEW) };
                            // Newest first.
                            for h in f.hits.iter().rev().take(n) {
                                let resp = ui.horizontal(|ui| {
                                    ui.add_space(26.0);
                                    ui.vertical(|ui| {
                                        let who = if h.user { "You" } else { "Agent" };
                                        let when = h.time.map(fmt_time).unwrap_or_default();
                                        ui.label(RichText::new(format!("{who}  {when}")).color(if h.user { ACCENT_HI } else { TEXT_FAINT }).size(11.5));
                                        ui.add(egui::Label::new(snippet_job(h, wrap)).selectable(false));
                                    });
                                });
                                let r = ui.interact(resp.response.rect, ui.scope_id().with((&f.agent_id, &h.snippet)), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
                                if r.hovered() {
                                    ui.painter().rect_stroke(resp.response.rect.expand(2.0), 4.0, Stroke::new(1.0, BORDER), egui::StrokeKind::Outside);
                                }
                                if r.clicked() {
                                    jump = Some(f.agent_id.clone());
                                }
                                ui.add_space(4.0);
                            }
                            let more = f.count.saturating_sub(n);
                            if more > 0 || all_shown && f.hits.len() > FEW {
                                ui.horizontal(|ui| {
                                    ui.add_space(26.0);
                                    let label = if all_shown {
                                        "Show fewer".to_string()
                                    } else if f.hits.len() > n {
                                        format!("Show {} more", f.hits.len() - n)
                                    } else {
                                        String::new()
                                    };
                                    if !label.is_empty() && super::status_bar::link_button(ui, &label, "Show every match listed for this session") {
                                        if all_shown {
                                            expanded.remove(&f.agent_id);
                                        } else {
                                            expanded.insert(f.agent_id.clone());
                                        }
                                    }
                                    let hidden = f.count - f.hits.len();
                                    if all_shown && hidden > 0 {
                                        ui.label(RichText::new(format!("({hidden} more not listed)")).color(TEXT_FAINT).size(12.0));
                                    }
                                });
                            }
                        });
                        ui.add_space(6.0);
                    }
                });
            });

        if all != self.s.search_all || days != self.s.search_days {
            self.s.search_all = all;
            self.s.search_days = days;
        }
        if let Some(st) = self.search_ui.as_mut() {
            st.query = query;
            st.expanded = expanded;
        }
        if !open || close {
            self.search_ui = None;
            return;
        }
        if go {
            self.run_search(ctx);
        }
        if let Some(id) = jump {
            let q = job.map(|j| j.query.clone()).unwrap_or_default();
            self.jump_to(ctx, &id, &q);
        }
    }
}
