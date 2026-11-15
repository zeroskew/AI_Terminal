//! Debug logging (Options > General > Diagnostics): low-level notes appended to
//! `%LOCALAPPDATA%\AI_Terminal\logs\ai_terminal.log` while the option is on.

use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

pub static ENABLED: AtomicBool = AtomicBool::new(false);

pub fn dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join("AI_Terminal").join("logs")
}

pub fn write(msg: &str) {
    if !ENABLED.load(Ordering::Relaxed) {
        return;
    }
    let d = dir();
    let _ = std::fs::create_dir_all(&d);
    let path = d.join("ai_terminal.log");
    // Keep the file bounded: start over past 5 MB.
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > 5 << 20) {
        let _ = std::fs::rename(&path, d.join("ai_terminal.old.log"));
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
        let _ = writeln!(f, "{t} [{}] {msg}", std::process::id());
    }
}

/// `dlog!("spawned {}", x)` — formats only when logging is on.
#[macro_export]
macro_rules! dlog {
    ($($arg:tt)*) => {
        if $crate::dlog::ENABLED.load(std::sync::atomic::Ordering::Relaxed) {
            $crate::dlog::write(&format!($($arg)*));
        }
    };
}
