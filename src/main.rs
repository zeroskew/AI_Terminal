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
mod update;
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

/// Release builds have no console, so a failed start (no OpenGL 2+ driver, a panic) would just
/// exit silently. Write it to `%LOCALAPPDATA%\AI_Terminal\logs\startup_error.log` and show it.
fn report_fatal(msg: &str) {
    let dir = dlog::dir();
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("startup_error.log");
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let _ = std::fs::write(&path, format!("time (unix): {t}\nversion: {}\n\n{msg}\n", update::VERSION));
    #[cfg(windows)]
    {
        use windows::core::HSTRING;
        use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};
        let text = HSTRING::from(format!("AI Terminal couldn't start:\n\n{msg}\n\nSaved to {}", path.display()));
        unsafe {
            MessageBoxW(None, &text, &HSTRING::from("AI Terminal"), MB_OK | MB_ICONERROR);
        }
    }
    #[cfg(not(windows))]
    eprintln!("AI Terminal couldn't start: {msg}");
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
    // `ai_terminal.exe --version`: the build version (the publish script tags the release with it).
    if args.len() >= 2 && args[1] == "--version" {
        println!("{}", update::VERSION);
        return Ok(());
    }
    // Any panic (on any thread, before or after the window opens) is reported, not lost.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let bt = std::backtrace::Backtrace::force_capture();
        let thread = std::thread::current().name().unwrap_or("<unnamed>").to_owned();
        report_fatal(&format!("panic on thread '{thread}': {info}\n\n{bt}"));
        default_hook(info);
    }));
    drop_parent_claude_env();
    update::init();
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
    let result = eframe::run_native(app_name, options, Box::new(|cc| Ok(Box::new(app::App::new(cc)))));
    if let Err(e) = &result {
        report_fatal(&format!("{e}\n\n(debug: {e:?})\n\nIf this mentions OpenGL / glutin / WGL: the PC needs a graphics driver with OpenGL 2.0 or newer (Remote Desktop sessions, VMs and the \"Microsoft Basic Display Adapter\" often lack one)."));
    }
    // The window is gone: install a downloaded update (and start again for "Restart now").
    update::apply_on_exit();
    result
}
