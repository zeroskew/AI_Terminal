//! The tools AI Terminal's MCP server offers to agent sessions (definitions only; the app runs
//! them in `App::mcp_call`). The core set: list projects
//! and sessions, the selection, "who am I", rename my session, open a project or session —
//! plus session messaging and handoff.

use serde_json::{json, Value};

pub const TARGET_DESCRIPTION: &str = "What the user called the session: its name in the Sessions list, a project tab as they said it ('tab 2': counting from 1 after Home), a project name, or a terminalKey or sessionId from list_sessions.";

pub fn definitions() -> Vec<Value> {
    let mut v = vec![
        json!({
            "name": "list_projects",
            "description": "List the projects open in AI Terminal: name, root path, whether it is the active tab, and how many sessions it holds.",
            "inputSchema": {"type": "object", "properties": {}},
            "annotations": {"readOnlyHint": true}
        }),
        json!({
            "name": "list_sessions",
            "description": "List sessions across all projects (or one project when project_path is given): id, terminal key, name, state (Idle/Processing/NeedsAttention/NotRunning), running flag, and owning project.",
            "inputSchema": {"type": "object", "properties": {"project_path": {"type": "string", "description": "Root folder of one project to list; omit for all projects."}}},
            "annotations": {"readOnlyHint": true}
        }),
        json!({
            "name": "get_selection",
            "description": "The currently active project tab and its selected session.",
            "inputSchema": {"type": "object", "properties": {}},
            "annotations": {"readOnlyHint": true}
        }),
        json!({
            "name": "whoami",
            "description": "Identify the calling session's own row in AI Terminal - its session id, terminal key, name, state, and owning project. Works only for sessions started by AI Terminal.",
            "inputSchema": {"type": "object", "properties": {}},
            "annotations": {"readOnlyHint": true}
        }),
        json!({
            "name": "rename_session",
            "description": "Rename the calling session's own row in the AI Terminal Sessions list. Whitespace is folded to single spaces and the name is capped at 120 characters; the result reports the name applied. Works only for sessions started by AI Terminal, and only on the caller's own row.",
            "inputSchema": {"type": "object", "properties": {"name": {"type": "string", "minLength": 1, "description": "New session name (must not be blank)."}}, "required": ["name"]},
            "annotations": {"readOnlyHint": false}
        }),
        json!({
            "name": "open_project",
            "description": "Switch AI Terminal to an open project tab, matched by root path or display name. Never adds or removes projects.",
            "inputSchema": {"type": "object", "properties": {"project": {"type": "string", "description": "Root folder path or display name of an open project."}}, "required": ["project"]},
            "annotations": {"readOnlyHint": false}
        }),
        json!({
            "name": "open_session",
            "description": "Select and show an existing session, switching to its project first if needed (a session that isn't running resumes). Accepts a session id (UUID) or terminal key from list_sessions. Never creates sessions.",
            "inputSchema": {"type": "object", "properties": {"session_id": {"type": "string", "description": "Session UUID or terminal key."}}, "required": ["session_id"]},
            "annotations": {"readOnlyHint": false}
        }),
        json!({
            "name": "messaging_send",
            "description": "Sends a message to another AI Terminal session. It arrives there as a prompt, marked as coming from your session, and that session's agent acts on it. Use it whenever the user asks you to tell, ask, notify or hand something to another session, for example \"when you're done, let the Spotting session know so it can continue\": finish the work, then send. A busy session gets the message when it finishes its turn; with `after`, the message waits until that session has finished its turn.",
            "inputSchema": {"type": "object", "additionalProperties": false, "properties": {
                "to": {"type": "string", "minLength": 1, "maxLength": 300, "description": TARGET_DESCRIPTION},
                "message": {"type": "string", "minLength": 1, "maxLength": 20000, "description": "Self-contained message for the other session's agent."},
                "after": {"type": "string", "minLength": 1, "maxLength": 300, "description": "Optional session to wait for: the message is held until it has finished its turn. Same forms as to."}
            }, "required": ["to", "message"]},
            "annotations": {"readOnlyHint": false}
        }),
        json!({
            "name": "messaging_notify_when_done",
            "description": "Asks AI Terminal to tell you (as a prompt in your session) when another session finishes its current turn, e.g. to continue once it is done.",
            "inputSchema": {"type": "object", "additionalProperties": false, "properties": {
                "session": {"type": "string", "minLength": 1, "maxLength": 300, "description": TARGET_DESCRIPTION},
                "note": {"type": "string", "minLength": 1, "maxLength": 20000, "description": "Optional reminder for yourself, returned with the notice."}
            }, "required": ["session"]},
            "annotations": {"readOnlyHint": false}
        }),
        json!({
            "name": "messaging_pending",
            "description": "Lists the session messages and notices still waiting to be delivered: id, from, to, what it waits for, and the text.",
            "inputSchema": {"type": "object", "properties": {}},
            "annotations": {"readOnlyHint": true}
        }),
        json!({
            "name": "messaging_cancel",
            "description": "Cancels a waiting session message or notice by its id (from messaging_send, messaging_notify_when_done or messaging_pending).",
            "inputSchema": {"type": "object", "additionalProperties": false, "properties": {"id": {"type": "string", "minLength": 1, "maxLength": 40, "description": "The waiting message's id."}}, "required": ["id"]},
            "annotations": {"readOnlyHint": false}
        }),
        json!({
            "name": "handoff_create_session",
            "description": "Starts a new session of the same kind in the same project and hands it your notes as its first prompt, then selects it - for continuing the work in a fresh context. Write handoff_notes so the new agent can carry on without this conversation.",
            "inputSchema": {"type": "object", "additionalProperties": false, "properties": {
                "handoff_notes": {"type": "string", "minLength": 1, "maxLength": 65536, "description": "Everything the new session needs: goal, state, decisions, next steps."},
                "display_name": {"type": "string", "maxLength": 120, "description": "Optional name for the new session."},
                "model": {"type": "string", "maxLength": 200, "description": "Optional model for the new session."},
                "effort": {"type": "string", "maxLength": 40, "description": "Optional effort level for the new session."}
            }, "required": ["handoff_notes"]},
            "annotations": {"readOnlyHint": false}
        }),
    ];
    v.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    v
}

/// Name rule for rename: whitespace folded, trimmed, at most 120 characters.
pub fn clean_name(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(120).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tools_are_well_formed() {
        let d = definitions();
        assert_eq!(d.len(), 12);
        for t in &d {
            assert!(t["name"].is_string() && t["description"].is_string() && t["inputSchema"]["type"] == "object");
        }
        assert_eq!(clean_name("  a \n  b\tc "), "a b c");
        assert_eq!(clean_name(&"x".repeat(200)).len(), 120);
    }
}
