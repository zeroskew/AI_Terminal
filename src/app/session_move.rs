//! Moving a session to another project (sidebar menu: Move to Project).
//!
//! A running agent session with a conversation is stopped first (it stays listed, under the new
//! project) so it resumes in the new folder. Claude keeps its transcripts per folder, so the
//! conversation's files are copied to the new project's folder and the old ones removed.

use super::*;

impl App {
    pub(super) fn move_session(&mut self, ctx: &egui::Context, id: u64, to: PathBuf) {
        let mut id = id;
        let was_active = self.active == Some(id);
        let from = if let Some(t) = self.tabs.iter_mut().find(|t| t.sess.id == id) {
            let from = t.project.clone();
            if from == to {
                return;
            }
            if !(t.sess.profile.is_agent() && t.sess.agent_id.is_some()) {
                // Nothing to carry over (a shell, or an agent not sent anything yet): regroup only.
                t.project = to;
                if was_active {
                    self.select_project(Some(from));
                }
                return;
            }
            self.park(id, "moved to another project");
            match self.dormant.last() {
                Some(d) => id = d.id,
                None => return,
            }
            from
        } else if let Some(d) = self.dormant.iter().find(|d| d.id == id) {
            if d.saved.project == to {
                return;
            }
            d.saved.project.clone()
        } else {
            return;
        };
        let Some(d) = self.dormant.iter_mut().find(|d| d.id == id) else { return };
        if let (Kind::Claude, Some(agent_id)) = (d.saved.profile.kind, d.saved.agent_id.clone()) {
            if let Err(e) = move_claude_conversation(&agent_id, &from, &to) {
                if was_active {
                    self.select_project(Some(from));
                }
                return self.toast(format!("Couldn't move the conversation: {e}"), RED, ctx);
            }
        }
        d.saved.project = to.clone();
        // Counts as just used, so "keep the last N" keeps it in the new project.
        d.saved.used = now_secs();
        if was_active {
            self.select_project(Some(from));
        }
        let name = self.s.projects.iter().find(|p| p.path == to).map(|p| p.name.clone()).unwrap_or_else(|| project_name(&to));
        self.toast(format!("Moved to {name}"), ACCENT_HI, ctx);
    }
}

/// Copies a Claude conversation (transcript + its folder of subagent transcripts / tool results)
/// from `from`'s Claude folder to `to`'s, then removes the originals in the background (the
/// stopped agent may hold them for a moment). Nothing to copy is fine.
fn move_claude_conversation(agent_id: &str, from: &Path, to: &Path) -> std::io::Result<()> {
    let (Some(src), Some(dst)) = (crate::agents::claude_dir(from), crate::agents::claude_dir(to)) else { return Ok(()) };
    let file = format!("{agent_id}.jsonl");
    let items = [src.join(&file), src.join(agent_id)];
    if !items.iter().any(|p| p.exists()) {
        return Ok(());
    }
    std::fs::create_dir_all(&dst)?;
    copy_any(&items[0], &dst.join(&file))?;
    copy_any(&items[1], &dst.join(agent_id))?;
    std::thread::spawn(move || {
        let mut left: Vec<PathBuf> = items.into_iter().filter(|p| p.exists()).collect();
        for _ in 0..10 {
            std::thread::sleep(Duration::from_millis(500));
            left.retain(|p| {
                let r = if p.is_dir() { std::fs::remove_dir_all(p) } else { std::fs::remove_file(p) };
                r.is_err() && p.exists()
            });
            if left.is_empty() {
                break;
            }
        }
        for p in &left {
            crate::dlog!("move session: couldn't remove {}", p.display());
        }
    });
    Ok(())
}

/// Copies a file or a folder tree; a missing source is skipped.
fn copy_any(src: &Path, dst: &Path) -> std::io::Result<()> {
    if src.is_dir() {
        std::fs::create_dir_all(dst)?;
        for e in std::fs::read_dir(src)? {
            let e = e?;
            copy_any(&e.path(), &dst.join(e.file_name()))?;
        }
    } else if src.is_file() {
        std::fs::copy(src, dst)?;
    }
    Ok(())
}
