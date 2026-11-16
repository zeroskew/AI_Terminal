#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
//! AI Terminal: terminal sessions for AI CLIs with inline previews of pasted images.

mod agent_install;
mod agents;
mod app;
mod colortags;
mod cost;
mod dlog;
mod emoji;
mod fonts;
mod hooks;
mod images;
mod keys;
mod links;
mod mcp;
mod mcp_servers;
mod mcp_tools;
mod plugins;
mod search;
mod session;
mod term_theme;
mod theme;
mod titlebar;
mod view;
mod widgets;
mod winsys;

/// Started from inside a Claude Code session (e.g. from a terminal Claude runs), we inherit
/// that session's markers; passed on, they would make every Claude we start a "child" session
/// (no transcript saving, wrong session id). Drop them so sessions start clean.
fn drop_parent_claude_env() {
    let markers = [
        "CLAUDECODE",
        "CLAUDE_PID",
        "CLAUDE_EFFORT",
        "CLAUDE_CODE_CHILD_SESSION",
        "CLAUDE_CODE_SESSION_ID",
        "CLAUDE_CODE_SESSION_ATTENDED",
        "CLAUDE_CODE_ENTRYPOINT",
        "CLAUDE_CODE_EXECPATH",
        "CLAUDE_CODE_MESSAGING_SOCKET",
        "CLAUDE_CODE_MESSAGING_TOKEN",
    ];
    for k in markers {
        std::env::remove_var(k);
    }
}

fn main() -> eframe::Result<()> {
    // `ai_terminal.exe --hook <event>`: a Claude Code hook call (see hooks.rs); no window.
    let args: Vec<String> = std::env::args().collect();
    if args.len() >= 3 && args[1] == "--hook" {
        hooks::hook_main(&args[2]);
    }
    // `ai_terminal.exe --statusline`: Claude Code's status line command (records usage limits).
    if args.len() >= 2 && args[1] == "--statusline" {
        hooks::statusline_main();
    }
    drop_parent_claude_env();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("AI Terminal")
            .with_inner_size([1500.0, 950.0])
            .with_min_inner_size([800.0, 500.0])
            .with_drag_and_drop(true)
            .with_icon(eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon_256.png")).unwrap_or_default()),
        ..Default::default()
    };
    // `--test-instance` keeps its own settings (%APPDATA%\AI Terminal Test), so automated test
    // runs never touch the user's projects and sessions.
    let app_name = if std::env::args().any(|a| a == "--test-instance") { "AI Terminal Test" } else { "AI Terminal" };
    eframe::run_native(app_name, options, Box::new(|cc| Ok(Box::new(app::App::new(cc)))))
}
