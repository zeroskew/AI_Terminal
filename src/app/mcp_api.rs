//! AI Terminal's MCP tools (sessions / projects), answered on the UI thread.

use super::*;

impl App {
    // ---------------------------------------------------------------------------------------
    // MCP tools (mcp.rs / mcp_tools.rs)

    pub(super) fn session_json(&self, id: u64) -> Option<serde_json::Value> {
        use serde_json::json;
        if let Some(t) = self.tabs.iter().find(|t| t.sess.id == id) {
            let state = if t.sess.attention.is_some() {
                "NeedsAttention"
            } else if t.sess.busy {
                "Processing"
            } else {
                "Idle"
            };
            return Some(json!({
                "sessionId": t.sess.agent_id, "terminalKey": t.sess.hook_key, "name": t.sess.display_name(),
                "kind": t.sess.profile.name, "state": state, "isRunning": t.sess.exited.is_none(),
                "isSelected": self.active == Some(id), "projectPath": t.project.display().to_string()
            }));
        }
        let d = self.dormant.iter().find(|d| d.id == id)?;
        Some(json!({
            "sessionId": d.saved.agent_id, "terminalKey": format!("listed-{}", d.id), "name": d.display_name(),
            "kind": d.saved.profile.name, "state": "NotRunning", "isRunning": false, "isSelected": false, "archived": d.saved.archived,
            "projectPath": d.saved.project.display().to_string()
        }))
    }

    pub(super) fn project_json(&self, p: &Project) -> serde_json::Value {
        serde_json::json!({ "name": p.name, "path": p.path.display().to_string(), "isActive": self.s.selected_project.as_ref() == Some(&p.path), "inTabs": p.in_tabs, "sessionCount": self.project_session_count(&p.path) })
    }

    /// All session ids (running and listed).
    pub(super) fn all_session_ids(&self) -> Vec<u64> {
        let mut v: Vec<u64> = self.tabs.iter().map(|t| t.sess.id).collect();
        v.extend(self.dormant.iter().map(|d| d.id));
        v
    }

    /// A session by UUID, terminal key, name, "tab N" or project name.
    pub(super) fn resolve_session(&self, q: &str) -> Result<u64, String> {
        let q = q.trim();
        let ql = q.to_lowercase();
        for t in &self.tabs {
            if t.sess.hook_key == q || t.sess.agent_id.as_deref() == Some(q) {
                return Ok(t.sess.id);
            }
        }
        for d in &self.dormant {
            if format!("listed-{}", d.id) == q || d.saved.agent_id.as_deref() == Some(q) {
                return Ok(d.id);
            }
        }
        let by_name: Vec<u64> = self.all_session_ids().into_iter().filter(|id| self.session_json(*id).and_then(|j| j["name"].as_str().map(|n| n.to_lowercase() == ql)).unwrap_or(false)).collect();
        if by_name.len() == 1 {
            return Ok(by_name[0]);
        }
        if by_name.len() > 1 {
            return Err(format!("More than one session is called '{q}'; use its terminalKey from list_sessions."));
        }
        let project = if let Some(n) = ql.strip_prefix("tab ").and_then(|n| n.trim().parse::<usize>().ok()) {
            self.tab_projects().get(n.saturating_sub(1)).cloned()
        } else {
            self.s.projects.iter().find(|p| p.name.to_lowercase() == ql).map(|p| p.path.clone())
        };
        if let Some(p) = project {
            let ids = self.project_sessions(&p);
            if let Some(id) = self.remembered.get(&p).copied().filter(|id| ids.contains(id)).or(ids.first().copied()) {
                return Ok(id);
            }
            return Err(format!("Project '{q}' has no running session."));
        }
        Err(format!("No session matches '{q}'. Use list_sessions."))
    }

    pub(super) fn caller(&self, key: &Option<String>) -> Result<u64, String> {
        let Some(k) = key.as_deref().filter(|k| !k.is_empty()) else {
            return Err("This session was not started by AI Terminal (no session identity in the environment), so it has no row. Pass explicit ids to the other tools instead.".into());
        };
        self.tabs.iter().find(|t| t.sess.hook_key == k).map(|t| t.sess.id).ok_or_else(|| format!("Terminal key '{k}' does not match any session in this AI Terminal (the session may have been closed)."))
    }

    pub(super) fn mcp_call(&mut self, ctx: &egui::Context, tool: &str, args: &serde_json::Value, key: &Option<String>) -> Result<serde_json::Value, String> {
        use serde_json::json;
        let arg = |n: &str| args.get(n).and_then(|v| v.as_str()).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
        crate::dlog!("mcp {tool} {args} from {key:?}");
        match tool {
            "list_projects" => Ok(json!({ "projects": self.s.projects.iter().map(|p| self.project_json(p)).collect::<Vec<_>>() })),
            "list_sessions" => {
                let filter = arg("project_path").map(PathBuf::from);
                if let Some(f) = &filter {
                    if !self.s.projects.iter().any(|p| &p.path == f) {
                        return Err(format!("No open project with root path '{}'. Use list_projects.", f.display()));
                    }
                }
                let mut out = Vec::new();
                for p in &self.s.projects {
                    if filter.as_ref().is_some_and(|f| f != &p.path) {
                        continue;
                    }
                    for r in self.rows(&p.path) {
                        out.extend(self.session_json(r.id));
                    }
                }
                Ok(json!({ "sessions": out }))
            }
            "get_selection" => match self.s.selected_project.clone().and_then(|p| self.s.projects.iter().find(|x| x.path == p).cloned()) {
                Some(p) => Ok(json!({ "project": self.project_json(&p), "session": self.active.and_then(|id| self.session_json(id)) })),
                None => Ok(json!({ "project": null, "session": null, "note": "The Home tab (or nothing) is active; no project is selected." })),
            },
            "whoami" => {
                let id = self.caller(key)?;
                let mut j = self.session_json(id).unwrap_or_default();
                if j["sessionId"].is_null() {
                    j["note"] = json!("sessionId is null because this brand-new session has not been linked to its conversation yet; retry after the first prompt completes.");
                }
                Ok(j)
            }
            "rename_session" => {
                let name = arg("name").map(|n| crate::mcp_tools::clean_name(&n)).filter(|n| !n.is_empty()).ok_or("Missing required argument 'name'; it must not be blank.")?;
                let id = self.caller(key)?;
                if let Some(t) = self.tabs.iter_mut().find(|t| t.sess.id == id) {
                    t.sess.name = name;
                }
                Ok(self.session_json(id).unwrap_or_default())
            }
            "open_project" => {
                let q = arg("project").ok_or("Missing required argument 'project' (root path or display name).")?;
                let p = self
                    .s
                    .projects
                    .iter()
                    .find(|p| p.path.display().to_string().eq_ignore_ascii_case(q.trim_end_matches(['\\', '/'])))
                    .or_else(|| self.s.projects.iter().find(|p| p.name.eq_ignore_ascii_case(&q)))
                    .cloned()
                    .ok_or_else(|| format!("No open project matches '{q}'. This tool only switches between open projects; use list_projects to see them."))?;
                self.select_project(Some(p.path.clone()));
                Ok(json!(format!("Switched to project '{}' ({}).", p.name, p.path.display())))
            }
            "open_session" => {
                let q = arg("session_id").ok_or("Missing required argument 'session_id'.")?;
                let id = self.resolve_session(&q).map_err(|_| format!("No session with id or terminal key '{q}' in any project. Use list_sessions."))?;
                let id = if self.dormant.iter().any(|d| d.id == id) {
                    self.resume(ctx, id);
                    self.active.ok_or("Couldn't start the session.")?
                } else {
                    self.select_session(id);
                    id
                };
                let j = self.session_json(id).unwrap_or_default();
                let project = self.s.projects.iter().find(|p| Some(p.path.display().to_string()) == j["projectPath"].as_str().map(str::to_string)).map(|p| p.name.clone()).unwrap_or_default();
                Ok(json!(format!("Selected session '{}' in project '{project}'.", j["name"].as_str().unwrap_or(""))))
            }
            "messaging_send" | "messaging_notify_when_done" => {
                let send = tool == "messaging_send";
                let from = self.caller(key).ok();
                let from_name = from.and_then(|id| self.session_json(id)).and_then(|j| j["name"].as_str().map(str::to_string)).unwrap_or_else(|| "an agent".into());
                let (to, waiting_for, text) = if send {
                    let to = self.resolve_session(&arg("to").ok_or("Missing required argument 'to'.")?)?;
                    let message = arg("message").ok_or("Missing required argument 'message'.")?;
                    let after = match arg("after") {
                        Some(a) => Some(self.resolve_session(&a)?),
                        None => None,
                    };
                    let key = key.clone().unwrap_or_default();
                    (to, after, format!("[Message from session \"{from_name}\" ({key}) via AI Terminal]\n{message}"))
                } else {
                    let from = from.ok_or("messaging_notify_when_done only works from a session started by AI Terminal.")?;
                    let target = self.resolve_session(&arg("session").ok_or("Missing required argument 'session'.")?)?;
                    let tname = self.session_json(target).and_then(|j| j["name"].as_str().map(str::to_string)).unwrap_or_default();
                    let note = arg("note").map(|n| format!("\nYour note: {n}")).unwrap_or_default();
                    (from, Some(target), format!("[AI Terminal notice] Session \"{tname}\" has finished its turn.{note}"))
                };
                if Some(to) == from && send {
                    return Err("That's your own session; messaging_send delivers to another session.".into());
                }
                let waiting_key = waiting_for.and_then(|w| self.tabs.iter().find(|t| t.sess.id == w).map(|t| t.sess.hook_key.clone()));
                if waiting_for.is_some() && waiting_key.is_none() {
                    return Err("The session to wait for isn't running.".into());
                }
                self.next_msg += 1;
                let id = format!("m{}", self.next_msg);
                let to_name = self.session_json(to).and_then(|j| j["name"].as_str().map(str::to_string)).unwrap_or_default();
                self.outbox.push(Pending { id: id.clone(), from_key: key.clone().unwrap_or_default(), to, waiting_for: waiting_key, text, created: std::time::Instant::now() });
                Ok(json!({ "id": id, "to": to_name, "status": if waiting_for.is_some() { "waiting" } else { "queued" }, "note": "Delivered as a prompt when the session is free (not mid-turn)." }))
            }
            "messaging_pending" => {
                let list: Vec<serde_json::Value> = self
                    .outbox
                    .iter()
                    .map(|m| {
                        let from = self.tabs.iter().find(|t| t.sess.hook_key == m.from_key).map(|t| t.sess.display_name()).unwrap_or_else(|| m.from_key.clone());
                        let to = self.session_json(m.to).and_then(|j| j["name"].as_str().map(str::to_string)).unwrap_or_default();
                        let waits = m.waiting_for.as_ref().and_then(|k| self.tabs.iter().find(|t| &t.sess.hook_key == k)).map(|t| t.sess.display_name());
                        json!({ "id": m.id, "from": from, "to": to, "waitingFor": waits, "ageSeconds": m.created.elapsed().as_secs(), "text": m.text })
                    })
                    .collect();
                Ok(json!({ "pending": list }))
            }
            "messaging_cancel" => {
                let id = arg("id").ok_or("Missing required argument 'id'.")?;
                let n = self.outbox.len();
                self.outbox.retain(|m| m.id != id);
                if self.outbox.len() == n {
                    return Err(format!("No waiting message with id '{id}'."));
                }
                Ok(json!(format!("Cancelled {id}.")))
            }
            "handoff_create_session" => {
                let notes = arg("handoff_notes").ok_or("Missing required argument 'handoff_notes'.")?;
                let from = self.caller(key)?;
                let t = self.tabs.iter().find(|t| t.sess.id == from).ok_or("Caller not found.")?;
                let (project, profile) = (t.project.clone(), t.sess.profile.clone());
                let mut opts = t.sess.opts.clone();
                if let Some(m) = arg("model") {
                    opts.model = Some(m);
                }
                if let Some(e) = arg("effort") {
                    opts.effort = Some(e);
                }
                opts.initial_prompt = Some(notes);
                let name = arg("display_name").map(|n| crate::mcp_tools::clean_name(&n)).unwrap_or_default();
                self.spawn_opts(ctx, project.clone(), profile, name, opts);
                let id = self.active.ok_or("Couldn't start the session.")?;
                let j = self.session_json(id).unwrap_or_default();
                Ok(json!({ "terminalKey": j["terminalKey"], "name": j["name"], "projectPath": project.display().to_string(), "status": "created_and_selected", "note": "The new session got your notes as its first prompt." }))
            }
            _ => Err(format!("Unknown tool: {tool}")),
        }
    }

    /// Serves tool calls, and delivers queued session messages when their target is free.
    pub(super) fn mcp_pump(&mut self, ctx: &egui::Context) {
        let calls: Vec<mcp::Call> = match &self.mcp {
            Some(srv) => std::iter::from_fn(|| srv.calls.try_recv().ok()).collect(),
            None => Vec::new(),
        };
        for c in calls {
            let r = self.mcp_call(ctx, &c.tool, &c.args, &c.session);
            let _ = c.reply.send(r);
        }
        if !self.enter_due.is_empty() || !self.cmd_due.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(60));
        }
        // Press Enter for messages pasted a moment ago (Claude needs the paste to land first).
        let now = std::time::Instant::now();
        let due: Vec<u64> = self.enter_due.iter().filter(|(_, t)| now >= *t).map(|(id, _)| *id).collect();
        self.enter_due.retain(|(_, t)| now < *t);
        for id in due {
            if let Some(t) = self.tabs.iter_mut().find(|t| t.sess.id == id) {
                t.sess.write(b"\r".to_vec());
            }
        }
        // Queued commands, once their time has come and no Enter is pending for the session.
        let mut i = 0;
        while i < self.cmd_due.len() {
            let (id, at, _) = &self.cmd_due[i];
            if now >= *at && !self.enter_due.iter().any(|(e, _)| e == id) {
                let (id, _, cmd) = self.cmd_due.remove(i);
                self.send_command_to(id, &cmd);
            } else {
                i += 1;
            }
        }
        // Deliver.
        let mut delivered = Vec::new();
        for m in &self.outbox {
            if m.waiting_for.is_some() {
                continue;
            }
            if self.dormant.iter().any(|d| d.id == m.to) {
                continue; // resumed below
            }
            let Some(t) = self.tabs.iter().find(|t| t.sess.id == m.to) else {
                delivered.push((m.id.clone(), None));
                continue;
            };
            if t.sess.busy || t.sess.exited.is_some() || self.enter_due.iter().any(|(id, _)| *id == m.to) {
                continue;
            }
            delivered.push((m.id.clone(), Some(m.to)));
        }
        // A message for a listed (not running) session resumes it first.
        let wake: Vec<u64> = self.outbox.iter().filter(|m| m.waiting_for.is_none() && self.dormant.iter().any(|d| d.id == m.to)).map(|m| m.to).collect();
        for d in wake {
            let before = self.active;
            self.resume(ctx, d);
            if let Some(new_id) = self.active.filter(|a| Some(*a) != before) {
                for m in self.outbox.iter_mut().filter(|m| m.to == d) {
                    m.to = new_id;
                }
            }
        }
        for (mid, to) in delivered {
            let Some(pos) = self.outbox.iter().position(|m| m.id == mid) else { continue };
            let m = self.outbox.remove(pos);
            let Some(to) = to else { continue };
            if let Some(t) = self.tabs.iter_mut().find(|t| t.sess.id == to) {
                let bracketed = t.sess.term.lock().mode().contains(alacritty_terminal::term::TermMode::BRACKETED_PASTE);
                let data = if bracketed { format!("\x1b[200~{}\x1b[201~", m.text.replace('\x1b', "")) } else { m.text.replace('\n', " ") };
                t.sess.write(data.into_bytes());
                crate::dlog!("delivered {} to #{to}", m.id);
                self.enter_due.push((to, now + Duration::from_millis(250)));
            }
        }
    }
}
