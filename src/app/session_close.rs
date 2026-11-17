//! Closing and deleting sessions from the sidebar.
//!
//! Close stops a running session but keeps it in the list (it resumes when clicked); while it's
//! working, it asks first. Delete (always asks) stops it, removes it from the list and deletes
//! the conversation's files.

use super::*;

/// The sidebar's confirmation dialog: the session (running or listed) and what was picked.
#[derive(Clone, Copy)]
pub(super) struct ConfirmClose {
    pub id: u64,
    pub delete: bool,
}

impl App {
    /// Close from the sidebar: stops the session but keeps it listed (asks first while it's
    /// working). Sessions without a conversation to resume (shells, an agent that hasn't been
    /// sent anything yet) just go.
    pub(super) fn request_close(&mut self, id: u64) {
        let Some(t) = self.tabs.iter().find(|t| t.sess.id == id) else { return };
        if t.sess.busy && t.sess.exited.is_none() {
            self.confirm_close = Some(ConfirmClose { id, delete: false });
        } else {
            self.close_to_list(id);
        }
    }

    fn close_to_list(&mut self, id: u64) {
        let Some(t) = self.tabs.iter().find(|t| t.sess.id == id) else { return };
        if !(t.sess.profile.is_agent() && t.sess.agent_id.is_some()) {
            return self.close(id);
        }
        let project = t.project.clone();
        let same = self.project_sessions(&project);
        let pos = same.iter().position(|x| *x == id).unwrap_or(0);
        let was_active = self.active == Some(id);
        self.park(id, "closed");
        if was_active {
            // The neighbouring running session of the same project.
            self.remembered.remove(&project);
            let same = self.project_sessions(&project);
            if let Some(&next) = same.get(pos.min(same.len().saturating_sub(1))) {
                self.select_session(next);
            }
        }
    }

    /// Stops the session, removes it from the list and deletes its conversation files.
    pub(super) fn delete_session(&mut self, id: u64) {
        let found = self.tabs.iter().find(|t| t.sess.id == id).map(|t| (t.sess.profile.kind, t.sess.agent_id.clone(), t.project.clone()));
        let found = found.or_else(|| self.dormant.iter().find(|d| d.id == id).map(|d| (d.saved.profile.kind, d.saved.agent_id.clone(), d.saved.project.clone())));
        self.dormant.retain(|d| d.id != id);
        self.close(id);
        if let Some((kind, Some(agent_id), project)) = found {
            delete_conversation_files(kind, agent_id, project);
        }
    }

    pub(super) fn confirm_close_dialog(&mut self, ctx: &egui::Context) {
        let Some(c) = self.confirm_close else { return };
        let info = self.tabs.iter().find(|t| t.sess.id == c.id).map(|t| (t.sess.display_name(), t.sess.agent_id.is_some(), t.sess.busy && t.sess.exited.is_none()));
        let info = info.or_else(|| self.dormant.iter().find(|d| d.id == c.id).map(|d| (d.display_name(), d.saved.agent_id.is_some(), false)));
        let Some((name, has_chat, busy)) = info else {
            self.confirm_close = None;
            return;
        };
        let mut go = false;
        let mut cancel = false;
        let m = egui::Modal::new(egui::Id::unique("confirm_close")).frame(egui::Frame::new().fill(PANEL).corner_radius(10).stroke(Stroke::new(1.0, BORDER)).inner_margin(20)).show(ctx, |ui| {
            ui.set_width(480.0);
            let title = if c.delete { "Delete this session?" } else { "Close this session?" };
            ui.label(RichText::new(title).size(18.0).family(bold()).color(Color32::WHITE));
            ui.add_space(2.0);
            ui.label(RichText::new(&name).family(bold()).color(TEXT));
            ui.add_space(8.0);
            if busy {
                ui.label(RichText::new("It's still working; this stops it mid-task.").color(ORANGE));
                ui.add_space(4.0);
            }
            let text = if !c.delete {
                "It stays in the list; click it to resume the conversation."
            } else if has_chat {
                "It's removed from the list and the conversation is deleted from disk. This can't be undone."
            } else {
                "It's removed from the list."
            };
            ui.label(RichText::new(text).color(TEXT_DIM));
            ui.add_space(14.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let (label, col) = if c.delete { ("Delete", Color32::from_rgb(170, 45, 45)) } else { ("Stop and close", ACCENT) };
                if ui.add(egui::Button::new(RichText::new(label).color(Color32::WHITE)).fill(col)).clicked() {
                    go = true;
                }
                if ui.button("Cancel").clicked() {
                    cancel = true;
                }
            });
        });
        if m.should_close() {
            cancel = true;
        }
        if go {
            self.confirm_close = None;
            if c.delete {
                self.delete_session(c.id);
            } else {
                self.close_to_list(c.id);
            }
        } else if cancel {
            self.confirm_close = None;
        }
    }
}

/// Deletes a conversation's files, in the background: the agent may take a moment to exit
/// and let go of them.
fn delete_conversation_files(kind: Kind, agent_id: String, project: PathBuf) {
    std::thread::spawn(move || {
        let mut paths: Vec<PathBuf> = Vec::new();
        match kind {
            Kind::Claude => {
                if let Some(dir) = crate::agents::claude_dir(&project) {
                    paths.push(dir.join(format!("{agent_id}.jsonl")));
                    // Subagent transcripts and saved tool results.
                    paths.push(dir.join(&agent_id));
                }
                if let Some(home) = crate::agents::home() {
                    let claude = home.join(".claude");
                    paths.push(claude.join("file-history").join(&agent_id));
                    paths.push(claude.join("session-env").join(&agent_id));
                    for f in std::fs::read_dir(claude.join("todos")).into_iter().flatten().flatten() {
                        if f.file_name().to_string_lossy().starts_with(&agent_id) {
                            paths.push(f.path());
                        }
                    }
                }
            }
            Kind::Codex => paths.extend(crate::agents::codex_rollout(&agent_id)),
            _ => {}
        }
        for _ in 0..10 {
            std::thread::sleep(Duration::from_millis(500));
            paths.retain(|p| {
                let r = if p.is_dir() { std::fs::remove_dir_all(p) } else { std::fs::remove_file(p) };
                r.is_err() && p.exists()
            });
            if paths.is_empty() {
                break;
            }
        }
        for p in &paths {
            crate::dlog!("delete session: couldn't remove {}", p.display());
        }
    });
}
