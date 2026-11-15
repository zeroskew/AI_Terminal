//! Build script: embeds the app icon (assets/ai_terminal.ico) by compiling assets/ai_terminal.rc
//! with the Windows SDK's rc.exe and handing the .res to the linker. rc.exe is on PATH under
//! the VS developer environment (build.cmd); otherwise the newest SDK copy is used. Without one
//! the exe still builds, just without an icon.

use std::path::{Path, PathBuf};
use std::process::Command;

fn find_rc() -> Option<PathBuf> {
    if Command::new("rc.exe").arg("/?").output().is_ok_and(|o| o.status.success() || !o.stdout.is_empty()) {
        return Some(PathBuf::from("rc.exe"));
    }
    let kits = Path::new(r"C:\Program Files (x86)\Windows Kits\10\bin");
    let mut versions: Vec<PathBuf> = std::fs::read_dir(kits).ok()?.flatten().map(|e| e.path()).filter(|p| p.join(r"x64\rc.exe").exists()).collect();
    versions.sort();
    versions.pop().map(|v| v.join(r"x64\rc.exe"))
}

fn main() {
    println!("cargo:rerun-if-changed=assets/ai_terminal.rc");
    println!("cargo:rerun-if-changed=assets/ai_terminal.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let Some(rc) = find_rc() else {
        println!("cargo:warning=rc.exe not found; building without the exe icon");
        return;
    };
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("ai_terminal.res");
    let status = Command::new(&rc).args(["/nologo", "/fo"]).arg(&out).arg("assets/ai_terminal.rc").status();
    match status {
        Ok(s) if s.success() => println!("cargo:rustc-link-arg-bins={}", out.display()),
        other => println!("cargo:warning=rc.exe failed ({other:?}); building without the exe icon"),
    }
}
