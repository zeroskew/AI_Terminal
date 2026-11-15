//! "Needs attention" for Claude sessions: Claude Code hooks.
//!
//! Claude sessions start with `--settings <hooks.json>`, which registers our own exe as an
//! async hook: `ai_terminal.exe --hook <event>`. That instance doesn't open a window; it reads
//! the hook's JSON from stdin, drops a tiny file in the events folder named after the session
//! (the `AI_TERMINAL_SESSION` variable every session gets) and exits. The app picks the files
//! up a few times a second.
//!
//! Events: UserPromptSubmit / PreToolUse → `active` (working); Stop → `stop` (done,
//! waiting for you); Notification → `notify` (e.g. a permission prompt; its message is kept).
//! Ours: SessionStart → `start` with the conversation id (new after /clear), so resume, title
//! and cost follow the current conversation.

use std::io::Read;
use std::path::PathBuf;

pub const ENV_KEY: &str = "AI_TERMINAL_SESSION";

pub fn events_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join("AI_Terminal").join("events")
}

/// `ai_terminal.exe --hook <event>`: record the event for the session and exit.
pub fn hook_main(event: &str) -> ! {
    let mut input = String::new();
    let _ = std::io::stdin().take(1 << 20).read_to_string(&mut input);
    if let Ok(key) = std::env::var(ENV_KEY) {
        let key: String = key.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').collect();
        let event: String = event.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
        let v = serde_json::from_str::<serde_json::Value>(&input).unwrap_or_default();
        // `start` (SessionStart) carries the conversation id; the others a message, if any.
        let message = if event == "start" {
            v["session_id"].as_str().unwrap_or_default().chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').collect()
        } else {
            v["message"].as_str().unwrap_or_default().to_string()
        };
        let dir = events_dir();
        let _ = std::fs::create_dir_all(&dir);
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        let _ = std::fs::write(dir.join(format!("{key}~{event}~{nanos}.evt")), message);
    }
    std::process::exit(0)
}

/// Where the latest plan usage limits are kept (shared by all sessions: they're per account).
pub fn limits_path() -> PathBuf {
    events_dir().with_file_name("limits.json")
}

/// One usage window: percent used and when it resets (Unix seconds).
#[derive(Clone, Copy, Debug)]
pub struct Window {
    pub used: f64,
    pub resets_at: Option<u64>,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Limits {
    pub five_hour: Option<Window>,
    pub seven_day: Option<Window>,
}

fn parse_window(v: &serde_json::Value) -> Option<Window> {
    let used = v["used_percentage"].as_f64().or_else(|| v["utilization"].as_f64())?;
    let resets_at = v["resets_at"].as_u64().or_else(|| v["resets_at"].as_f64().map(|f| f as u64));
    Some(Window { used, resets_at })
}

/// Reads the limits file written by `--statusline`.
pub fn read_limits() -> Option<Limits> {
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(limits_path()).ok()?).ok()?;
    let l = Limits { five_hour: parse_window(&v["five_hour"]), seven_day: parse_window(&v["seven_day"]) };
    (l.five_hour.is_some() || l.seven_day.is_some()).then_some(l)
}

/// Per-conversation plan usage (`plan_usage.json`): how many percentage points of the 5-hour
/// and weekly limits each conversation used.
pub fn plan_usage_path() -> PathBuf {
    events_dir().with_file_name("plan_usage.json")
}

/// A conversation's share of the plan limits, in percentage points.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PlanShare {
    pub five_hour: f64,
    pub seven_day: f64,
}

/// Reads the per-conversation plan usage: conversation id → share.
pub fn read_plan_usage() -> std::collections::HashMap<String, PlanShare> {
    let Ok(text) = std::fs::read_to_string(plan_usage_path()) else { return Default::default() };
    let v: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
    v["sessions"]
        .as_object()
        .map(|o| o.iter().map(|(k, s)| (k.clone(), PlanShare { five_hour: s["five_hour"].as_f64().unwrap_or(0.0), seven_day: s["seven_day"].as_f64().unwrap_or(0.0) })).collect())
        .unwrap_or_default()
}

/// How far a window moved since the last report: the increase within the same window, or
/// everything used so far once it has reset (a new `resets_at`, or the percentage went down).
fn window_delta(last: &serde_json::Value, now: Option<Window>) -> f64 {
    let (Some(now), Some(prev)) = (now, parse_window(last)) else { return 0.0 };
    let same = now.resets_at.zip(prev.resets_at).is_none_or(|(a, b)| a.abs_diff(b) < 120);
    if same && now.used >= prev.used {
        now.used - prev.used
    } else {
        now.used
    }
}

/// Credits the change in plan usage since the last status line update (from any session) to
/// the conversation reporting it. The status line runs right after a reply, so the increase
/// mostly belongs to that conversation; when several sessions work at once it's approximate.
fn record_plan_usage(session_id: &str, rl: &serde_json::Value) {
    let path = plan_usage_path();
    let _ = std::fs::create_dir_all(path.parent().unwrap_or(&path));
    // Status lines of several sessions can run at once: a lock file serialises the update.
    let lock = path.with_extension("lock");
    let mut locked = false;
    for _ in 0..50 {
        if std::fs::OpenOptions::new().write(true).create_new(true).open(&lock).is_ok() {
            locked = true;
            break;
        }
        // A lock left behind by a killed process.
        if std::fs::metadata(&lock).and_then(|m| m.modified()).is_ok_and(|t| t.elapsed().unwrap_or_default().as_secs() > 5) {
            let _ = std::fs::remove_file(&lock);
            continue;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    if !locked {
        return;
    }
    let mut v: serde_json::Value = std::fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_else(|| serde_json::json!({}));
    if !v.is_object() {
        v = serde_json::json!({});
    }
    let d5 = window_delta(&v["last"]["five_hour"], parse_window(&rl["five_hour"]));
    let d7 = window_delta(&v["last"]["seven_day"], parse_window(&rl["seven_day"]));
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    if !v["sessions"].is_object() {
        v["sessions"] = serde_json::json!({});
    }
    let sessions = v["sessions"].as_object_mut().unwrap();
    if d5 > 0.0 || d7 > 0.0 || !sessions.contains_key(session_id) {
        let e = sessions.entry(session_id.to_string()).or_insert_with(|| serde_json::json!({ "five_hour": 0.0, "seven_day": 0.0 }));
        e["five_hour"] = (e["five_hour"].as_f64().unwrap_or(0.0) + d5).into();
        e["seven_day"] = (e["seven_day"].as_f64().unwrap_or(0.0) + d7).into();
        e["t"] = now.into();
    }
    // Forget conversations untouched for 90 days.
    sessions.retain(|_, s| now.saturating_sub(s["t"].as_u64().unwrap_or(now)) < 90 * 86_400);
    // Keep the previous values of a window Claude didn't report this time.
    let mut last = v["last"].clone();
    if !last.is_object() {
        last = serde_json::json!({});
    }
    for w in ["five_hour", "seven_day"] {
        if parse_window(&rl[w]).is_some() {
            last[w] = rl[w].clone();
        }
    }
    v["last"] = last;
    let tmp = path.with_extension("tmp");
    if std::fs::write(&tmp, v.to_string()).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    }
    let _ = std::fs::remove_file(&lock);
}

/// The statusLine command a project's or the user's own settings define, if any (so ours can
/// run it and keep showing its output inside Claude Code).
fn own_statusline(cwd: &str) -> Option<String> {
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from);
    let mut files: Vec<PathBuf> = Vec::new();
    if !cwd.is_empty() {
        let c = PathBuf::from(cwd).join(".claude");
        files.push(c.join("settings.local.json"));
        files.push(c.join("settings.json"));
    }
    if let Some(h) = home {
        files.push(h.join(".claude").join("settings.json"));
    }
    let me = std::env::current_exe().ok().map(|p| p.display().to_string()).unwrap_or_default();
    files.iter().find_map(|f| {
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(f).ok()?).ok()?;
        let cmd = v["statusLine"]["command"].as_str()?.to_string();
        (!cmd.contains("--statusline") || !cmd.contains(&me)).then_some(cmd)
    })
}

/// `ai_terminal.exe --statusline`: Claude Code's status line. Saves the `rate_limits` it gets
/// (5-hour and weekly plan usage) for the app's status bar, then prints the user's own status
/// line, if they have one.
pub fn statusline_main() -> ! {
    let mut input = String::new();
    let _ = std::io::stdin().take(1 << 20).read_to_string(&mut input);
    let v: serde_json::Value = serde_json::from_str(&input).unwrap_or_default();
    let rl = &v["rate_limits"];
    if rl.is_object() && (parse_window(&rl["five_hour"]).is_some() || parse_window(&rl["seven_day"]).is_some()) {
        let path = limits_path();
        let _ = std::fs::create_dir_all(path.parent().unwrap_or(&path));
        let tmp = path.with_extension("tmp");
        if std::fs::write(&tmp, rl.to_string()).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
        let sid: String = v["session_id"].as_str().unwrap_or_default().chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').collect();
        if !sid.is_empty() {
            record_plan_usage(&sid, rl);
        }
    }
    let cwd = v["workspace"]["current_dir"].as_str().or(v["cwd"].as_str()).unwrap_or_default().to_string();
    if let Some(cmd) = own_statusline(&cwd) {
        use std::io::Write;
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let child = std::process::Command::new("cmd.exe")
            .arg("/c")
            .raw_arg(&cmd)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .creation_flags(CREATE_NO_WINDOW)
            .spawn();
        if let Ok(mut c) = child {
            if let Some(mut si) = c.stdin.take() {
                let _ = si.write_all(input.as_bytes());
            }
            if let Ok(out) = c.wait_with_output() {
                let _ = std::io::stdout().write_all(&out.stdout);
            }
        }
    }
    std::process::exit(0)
}

/// Writes the hooks settings file for this exe and returns its path.
pub fn write_settings() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let exe_s = exe.display().to_string();
    let cmd = |ev: &str| serde_json::json!({ "type": "command", "command": format!("\"{exe_s}\" --hook {ev}"), "async": true });
    let entry = |ev: &str| serde_json::json!([{ "hooks": [cmd(ev)] }]);
    let settings = serde_json::json!({
        // Claude Code hands the status line its plan usage (rate_limits); see statusline_main.
        "statusLine": { "type": "command", "command": format!("\"{exe_s}\" --statusline"), "padding": 0 },
        "hooks": {
            "UserPromptSubmit": entry("active"),
            "PreToolUse": entry("active"),
            "Stop": entry("stop"),
            "Notification": entry("notify"),
            // Startup, resume, /clear and compact: the conversation id (it changes on /clear).
            "SessionStart": entry("start"),
        }
    });
    // One file per exe location (a test build and the installed one may run side by side).
    let mut h = std::collections::hash_map::DefaultHasher::new();
    std::hash::Hash::hash(&exe_s, &mut h);
    let path = events_dir().parent()?.join(format!("hooks-{:08x}.json", std::hash::Hasher::finish(&h) as u32));
    std::fs::create_dir_all(path.parent()?).ok()?;
    std::fs::write(&path, serde_json::to_string_pretty(&settings).ok()?).ok()?;
    Some(path)
}

pub struct HookEvent {
    pub key: String,
    pub event: String,
    pub message: String,
}

/// Collects (and removes) the events written since the last call. Files nobody claims are
/// dropped after a minute.
pub fn poll(known: &dyn Fn(&str) -> bool) -> Vec<HookEvent> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(events_dir()) else { return out };
    let mut files: Vec<_> = rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "evt")).collect();
    files.sort();
    for p in files {
        let Some(stem) = p.file_stem().and_then(|s| s.to_str()).map(str::to_string) else { continue };
        let mut parts = stem.splitn(3, '~');
        let (Some(key), Some(event)) = (parts.next(), parts.next()) else {
            let _ = std::fs::remove_file(&p);
            continue;
        };
        if known(key) {
            out.push(HookEvent { key: key.to_string(), event: event.to_string(), message: std::fs::read_to_string(&p).unwrap_or_default() });
            let _ = std::fs::remove_file(&p);
        } else if std::fs::metadata(&p).and_then(|m| m.modified()).is_ok_and(|t| t.elapsed().unwrap_or_default().as_secs() > 60) {
            let _ = std::fs::remove_file(&p);
        }
    }
    out
}

/// Wakes the UI when hook events arrive (a cheap folder check, 4 times a second), so the app
/// doesn't have to redraw periodically just to look for them.
pub fn watch(ctx: egui::Context) {
    std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_millis(250));
        let any = std::fs::read_dir(events_dir()).map(|rd| rd.flatten().any(|e| e.path().extension().is_some_and(|x| x == "evt"))).unwrap_or(false);
        if any {
            ctx.request_repaint();
        }
    });
}

#[cfg(test)]
mod plan_tests {
    use super::*;

    #[test]
    fn window_deltas() {
        let last = serde_json::json!({ "used_percentage": 15, "resets_at": 1000 });
        let w = |used: f64, r: u64| Some(Window { used, resets_at: Some(r) });
        assert_eq!(window_delta(&last, w(18.0, 1000)), 3.0);
        assert_eq!(window_delta(&last, w(15.0, 1000)), 0.0);
        // A new window: everything used in it counts.
        assert_eq!(window_delta(&last, w(4.0, 19000)), 4.0);
        // No previous report: nothing is credited.
        assert_eq!(window_delta(&serde_json::Value::Null, w(40.0, 1000)), 0.0);
    }
}
