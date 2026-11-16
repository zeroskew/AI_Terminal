//! Clickable links in terminal text (Ctrl+click): URLs, and file paths with an optional
//! `:line[:col]` (absolute, or relative to the session's folder), opened in the browser / editor.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Link {
    Url(String),
    File { path: String, line: Option<u32>, col: Option<u32> },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub start: usize,
    pub end: usize,
    pub link: Link,
}

fn re() -> &'static regex::Regex {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    RE.get_or_init(|| {
        regex::Regex::new(concat!(
            r#"(?P<url>https?://[^\s"'<>`\[\]{}|\\^]+)"#,
            r#"|(?P<file>(?:[A-Za-z]:[\\/]|\.{1,2}[\\/]|[\w.-]+[\\/])(?:[\w.@+-]*[\\/])*[\w.@+-]*\.[A-Za-z0-9]{1,8})(?:[:(](?P<line>\d+)(?:[:,](?P<col>\d+))?\)?)?"#,
        ))
        .unwrap()
    })
}

/// URLs and file-like paths in `text`.
pub fn find(text: &str) -> Vec<Found> {
    let mut out = Vec::new();
    for c in re().captures_iter(text) {
        if let Some(m) = c.name("url") {
            // Trailing punctuation belongs to the sentence, not the URL.
            let mut u = m.as_str();
            while let Some(last) = u.chars().last() {
                let unbalanced_paren = last == ')' && u.matches('(').count() < u.matches(')').count();
                if ".,;:!?'\"".contains(last) || unbalanced_paren {
                    u = &u[..u.len() - last.len_utf8()];
                } else {
                    break;
                }
            }
            out.push(Found { start: m.start(), end: m.start() + u.len(), link: Link::Url(u.to_string()) });
        } else if let Some(m) = c.name("file") {
            let whole = c.get(0).unwrap();
            let path = m.as_str().trim().to_string();
            // A trailing ")" of "(file.rs:12)" stays out unless it closes "file(12)".
            let end = if whole.as_str().ends_with(')') && !whole.as_str()[m.len()..].starts_with('(') { whole.end() - 1 } else { whole.end() };
            out.push(Found {
                start: m.start(),
                end,
                link: Link::File { path, line: c.name("line").and_then(|l| l.as_str().parse().ok()), col: c.name("col").and_then(|l| l.as_str().parse().ok()) },
            });
        }
    }
    out
}

/// Resolves a file link against the session folder; None when it doesn't exist.
pub fn resolve(path: &str, cwd: &Path) -> Option<PathBuf> {
    let p = PathBuf::from(path);
    let full = if p.is_absolute() { p } else { cwd.join(p) };
    full.exists().then_some(full)
}

fn on_path(exe: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(exe).is_file()))
}

/// Opens a URL in the default browser.
pub fn open_url(url: &str) {
    let _ = std::process::Command::new("explorer.exe").arg(url).spawn();
}

/// Editors offered in Options. "auto" = VS Code if installed, else Rider, else the default app.
pub const EDITORS: &[(&str, &str)] = &[("auto", "Automatic (VS Code, then Rider, then the default app)"), ("vscode", "VS Code"), ("rider", "Rider"), ("default", "Default app for the file type"), ("custom", "Custom command")];

/// Opens a file at a line in the chosen editor. `custom` may use {file}, {line}, {col}.
pub fn open_file(path: &Path, line: Option<u32>, col: Option<u32>, editor: &str, custom: &str) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let file = path.display().to_string();
    let (l, c) = (line.unwrap_or(1), col.unwrap_or(1));
    let editor = match editor {
        "auto" if on_path("code.cmd") || on_path("code.exe") => "vscode",
        "auto" if on_path("rider.cmd") || on_path("rider64.exe") => "rider",
        "auto" => "default",
        e => e,
    };
    let spawn = |args: Vec<String>| std::process::Command::new("cmd.exe").arg("/c").args(args).creation_flags(CREATE_NO_WINDOW).spawn().map(|_| ()).map_err(|e| e.to_string());
    match editor {
        "vscode" => spawn(vec!["code".into(), "-g".into(), format!("{file}:{l}:{c}")]),
        "rider" => spawn(vec!["rider".into(), "--line".into(), l.to_string(), "--column".into(), c.saturating_sub(1).to_string(), file]),
        "custom" if !custom.trim().is_empty() => {
            let cmd = custom.replace("{file}", &file).replace("{line}", &l.to_string()).replace("{col}", &c.to_string());
            std::process::Command::new("cmd.exe").arg("/c").raw_arg(cmd).creation_flags(CREATE_NO_WINDOW).spawn().map(|_| ()).map_err(|e| e.to_string())
        }
        // Opening a script through its file association would run it: use Notepad.
        _ if path.extension().and_then(|e| e.to_str()).is_some_and(|e| ["cmd", "bat", "ps1", "py", "vbs", "js", "sh"].iter().any(|x| e.eq_ignore_ascii_case(x))) => {
            std::process::Command::new("notepad.exe").arg(&file).spawn().map(|_| ()).map_err(|e| e.to_string())
        }
        _ => std::process::Command::new("explorer.exe").arg(&file).spawn().map(|_| ()).map_err(|e| e.to_string()),
    }
}

/// The GitHub page of the git repository containing `dir` (its `origin` remote, else the first
/// remote on github.com); None when it isn't a git repository or isn't on GitHub. Reads the git
/// config directly (no git process), following worktree / submodule `.git` files.
pub fn github_page(dir: &Path) -> Option<String> {
    let git_dir = dir.ancestors().find_map(|d| {
        let g = d.join(".git");
        if g.is_dir() {
            return Some(g);
        }
        // Worktrees and submodules: a `.git` file with "gitdir: <path>".
        let text = std::fs::read_to_string(&g).ok()?;
        let p = PathBuf::from(text.trim().strip_prefix("gitdir:")?.trim());
        Some(if p.is_absolute() { p } else { d.join(p) })
    })?;
    // A worktree's config lives in the main repository (its `commondir`).
    let common = std::fs::read_to_string(git_dir.join("commondir")).ok().map(|c| {
        let p = PathBuf::from(c.trim());
        if p.is_absolute() { p } else { git_dir.join(p) }
    });
    let config = std::fs::read_to_string(common.unwrap_or(git_dir).join("config")).ok()?;
    let remotes = remote_urls(&config);
    remotes.iter().find(|(n, _)| n == "origin").into_iter().chain(remotes.iter()).find_map(|(_, u)| github_web_url(u))
}

/// (remote name, url) pairs from a git config file.
fn remote_urls(config: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut remote: Option<String> = None;
    for line in config.lines().map(str::trim) {
        if line.starts_with('[') {
            remote = line.strip_prefix("[remote \"").and_then(|r| r.strip_suffix("\"]")).map(str::to_string);
        } else if let (Some(name), Some((k, v))) = (&remote, line.split_once('=')) {
            if k.trim().eq_ignore_ascii_case("url") {
                out.push((name.clone(), v.trim().trim_matches('"').to_string()));
            }
        }
    }
    out
}

/// https://github.com/owner/repo for a GitHub remote URL (https, ssh or scp-style).
fn github_web_url(url: &str) -> Option<String> {
    let rest = url.strip_prefix("git@github.com:").or_else(|| {
        let after_scheme = url.split_once("://")?.1;
        let host_path = after_scheme.rsplit_once('@').map_or(after_scheme, |(_, h)| h);
        let (host, path) = host_path.split_once('/')?;
        // Host may carry a port (ssh://git@github.com:22/...).
        host.split(':').next()?.eq_ignore_ascii_case("github.com").then_some(path)
    })?;
    let path = rest.trim_end_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    let mut parts = path.split('/').filter(|s| !s.is_empty());
    let (owner, repo) = (parts.next()?, parts.next()?);
    Some(format!("https://github.com/{owner}/{repo}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn github_urls() {
        for u in ["https://github.com/a/b.git", "https://github.com/a/b", "git@github.com:a/b.git", "ssh://git@github.com/a/b.git", "ssh://git@github.com:22/a/b", "https://user:tok@github.com/a/b/"] {
            assert_eq!(github_web_url(u).as_deref(), Some("https://github.com/a/b"), "{u}");
        }
        assert_eq!(github_web_url("https://gitlab.com/a/b.git"), None);
        assert_eq!(github_web_url("git@example.com:a/b.git"), None);
        let cfg = "[core]\n\tbare = false\n[remote \"upstream\"]\n\turl = https://github.com/up/x.git\n[remote \"origin\"]\n\turl = git@github.com:me/x.git\n\tfetch = +refs/heads/*:refs/remotes/origin/*\n";
        let r = remote_urls(cfg);
        assert_eq!(r.len(), 2);
        assert_eq!(r.iter().find(|(n, _)| n == "origin").and_then(|(_, u)| github_web_url(u)).as_deref(), Some("https://github.com/me/x"));
    }

    #[test]
    fn urls() {
        let f = find("see https://example.com/a?b=1, and (https://x.org/p).");
        assert_eq!(f[0].link, Link::Url("https://example.com/a?b=1".into()));
        assert_eq!(f[1].link, Link::Url("https://x.org/p".into()));
    }

    #[test]
    fn files_with_lines() {
        let f = find(r"error at src\app.rs:123:7 and C:\Work\x.txt (lib/mod.rs:9)");
        assert_eq!(f[0].link, Link::File { path: r"src\app.rs".into(), line: Some(123), col: Some(7) });
        assert_eq!(f[1].link, Link::File { path: r"C:\Work\x.txt".into(), line: None, col: None });
        assert_eq!(f[2].link, Link::File { path: "lib/mod.rs".into(), line: Some(9), col: None });
        let t = r"(lib/mod.rs:9)";
        let g = find(t);
        assert_eq!(&t[g[0].start..g[0].end], "lib/mod.rs:9");
    }
}
