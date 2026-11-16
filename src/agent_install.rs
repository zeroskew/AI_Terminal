//! Where the agent CLIs in [`AGENTS`] are installed, and how to remove each copy (Options >
//! Agents). An agent can be installed more than once (say npm and the native installer);
//! each copy is listed with its own uninstall.

use std::path::{Path, PathBuf};

use crate::agents::{AgentSpec, Kind};
use crate::session::{extra_agent_dirs, refresh_path};

/// How a copy was installed, which decides how it's removed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Method {
    /// `npm install -g <package>`.
    Npm(&'static str),
    /// `uv tool install <tool>`.
    Uv(&'static str),
    /// Claude Code's native installer (`~/.local/bin/claude.exe` + `~/.local/share/claude`).
    ClaudeNative,
    /// The Antigravity CLI installer (`%LOCALAPPDATA%\agy\bin`, added to the user PATH).
    AgyNative,
    /// Bundled with a Microsoft Store app (the Codex app): removed with the app.
    StoreApp,
    /// Something else; the user removes it.
    Manual,
}

/// One installed copy of an agent CLI.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Installation {
    pub kind: Kind,
    pub path: PathBuf,
    pub method: Method,
}

impl Installation {
    pub fn how(&self) -> String {
        match self.method {
            Method::Npm(p) => format!("npm package {p}"),
            Method::Uv(t) => format!("uv tool {t}"),
            Method::ClaudeNative | Method::AgyNative => "native installer".into(),
            Method::StoreApp => "part of a Microsoft Store app".into(),
            Method::Manual => "installed another way".into(),
        }
    }

    /// The PowerShell commands that remove this copy (None: it can't be removed from here).
    pub fn uninstall_command(&self) -> Option<String> {
        let q = |p: &Path| format!("'{}'", p.display().to_string().replace('\'', "''"));
        let head = "$ErrorActionPreference = 'Stop'\n";
        Some(match self.method {
            Method::Npm(p) => format!("{head}npm uninstall -g {p}\nif ($LASTEXITCODE) {{ exit $LASTEXITCODE }}\n"),
            Method::Uv(t) => format!("{head}uv tool uninstall {t}\nif ($LASTEXITCODE) {{ exit $LASTEXITCODE }}\n"),
            Method::ClaudeNative => {
                let data = home()?.join(".local").join("share").join("claude");
                format!(
                    "{head}Remove-Item -LiteralPath {} -Force\nif (Test-Path -LiteralPath {data}) {{ Remove-Item -LiteralPath {data} -Recurse -Force }}\nWrite-Host 'Removed. Settings and conversations in ~/.claude are kept.'\n",
                    q(&self.path),
                    data = q(&data)
                )
            }
            Method::AgyNative => {
                // ...\agy\bin\agy.exe: remove the agy folder and its PATH entry.
                let bin = self.path.parent()?;
                let root = if bin.file_name().is_some_and(|n| n.eq_ignore_ascii_case("bin")) { bin.parent()? } else { bin };
                let staging = std::env::var_os("LOCALAPPDATA").map(|l| PathBuf::from(l).join("antigravity").join("staging"));
                let mut s = format!("{head}Remove-Item -LiteralPath {} -Recurse -Force\n", q(root));
                if let Some(st) = staging {
                    s += &format!("if (Test-Path -LiteralPath {st}) {{ Remove-Item -LiteralPath {st} -Recurse -Force }}\n", st = q(&st));
                }
                s += &remove_from_user_path(bin);
                s += "Write-Host 'Removed.'\n";
                s
            }
            Method::StoreApp | Method::Manual => return None,
        })
    }
}

/// Removes `dir` from the user PATH in the registry, keeping it REG_EXPAND_SZ (unlike
/// [Environment]::SetEnvironmentVariable, which would stop %VAR% entries expanding).
fn remove_from_user_path(dir: &Path) -> String {
    let d = dir.display().to_string().trim_end_matches('\\').replace('\'', "''");
    format!(
        r#"$k = Get-Item 'HKCU:\Environment'
$raw = $k.GetValue('Path', '', 'DoNotExpandEnvironmentNames')
$keep = @($raw -split ';' | Where-Object {{ $_ -and [Environment]::ExpandEnvironmentVariables($_).TrimEnd('\') -ne '{d}' }})
$new = $keep -join ';'
if ($new -ne $raw) {{ Set-ItemProperty -Path 'HKCU:\Environment' -Name Path -Value $new -Type ExpandString; Write-Host 'Removed {d} from your PATH.' }}
"#
    )
}

fn home() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE").map(PathBuf::from)
}

fn norm(p: &Path) -> String {
    p.to_string_lossy().trim_end_matches(['\\', '/']).to_lowercase()
}

fn under(p: &Path, dir: &Path) -> bool {
    norm(p).starts_with(&(norm(dir) + "\\"))
}

/// Every copy of every agent in [`crate::agents::AGENTS`].
pub fn find_all() -> Vec<Installation> {
    refresh_path();
    let path: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    crate::agents::AGENTS.iter().flat_map(|s| find(s, &path)).collect()
}

fn find(spec: &AgentSpec, path: &[PathBuf]) -> Vec<Installation> {
    let mut v: Vec<Installation> = Vec::new();
    let mut add = |path: PathBuf, method: Method| {
        if !v.iter().any(|i| norm(&i.path) == norm(&path)) {
            v.push(Installation { kind: spec.kind, path, method });
        }
    };
    let exe = format!("{}.exe", spec.exe);
    let mut exes: Vec<PathBuf> = path.iter().map(|d| d.join(&exe)).filter(|p| p.is_file()).collect();
    exes.extend(extra_agent_dirs(spec.exe).into_iter().filter(|p| p.is_file()));
    let home = home();
    let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    for p in exes {
        let in_local_bin = home.as_ref().is_some_and(|h| p.parent().is_some_and(|d| norm(d) == norm(&h.join(".local").join("bin"))));
        let method = if spec.kind == Kind::Claude && in_local_bin {
            Method::ClaudeNative
        } else if spec.kind == Kind::Antigravity && local.as_ref().is_some_and(|l| under(&p, &l.join("agy"))) {
            Method::AgyNative
        } else if let (Some(t), true) = (spec.uv, in_local_bin) {
            Method::Uv(t)
        } else if let Some(pkg) = spec.npm.filter(|pkg| p.parent().is_some_and(|d| d.join("node_modules").join(pkg).is_dir())) {
            Method::Npm(pkg)
        } else {
            Method::Manual
        };
        add(p, method);
    }
    // npm shims (claude.cmd next to node_modules).
    for d in path {
        let shim = d.join(format!("{}.cmd", spec.exe));
        if shim.is_file() {
            let method = match spec.npm {
                Some(pkg) if d.join("node_modules").join(pkg).is_dir() => Method::Npm(pkg),
                _ => Method::Manual,
            };
            add(shim, method);
        }
    }
    if spec.kind == Kind::Codex {
        if let Some(exe) = crate::winsys::package_root("OpenAI.Codex_").map(|r| r.join("app").join("resources").join("codex.exe")).filter(|p| p.is_file()) {
            add(exe, Method::StoreApp);
        }
    }
    v
}

/// `-EncodedCommand` text: the script as base64 UTF-16LE (no quoting issues).
pub fn encode_ps(script: &str) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes: Vec<u8> = script.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
    let mut out = String::with_capacity(bytes.len() * 4 / 3 + 4);
    for c in bytes.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= c.len() {
                out.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_utf16() {
        // "a" -> 61 00 -> YQA=
        assert_eq!(encode_ps("a"), "YQA=");
        assert_eq!(encode_ps("ab"), "YQBiAA==");
        assert_eq!(encode_ps("abc"), "YQBiAGMA");
    }

    #[test]
    fn uninstall_commands() {
        let npm = Installation { kind: Kind::Codex, path: "C:\\n\\codex.cmd".into(), method: Method::Npm("@openai/codex") };
        assert!(npm.uninstall_command().unwrap().contains("npm uninstall -g @openai/codex"));
        let agy = Installation { kind: Kind::Antigravity, path: "C:\\L\\agy\\bin\\agy.exe".into(), method: Method::AgyNative };
        let c = agy.uninstall_command().unwrap();
        assert!(c.contains("Remove-Item -LiteralPath 'C:\\L\\agy' -Recurse"), "{c}");
        assert!(c.contains("-ne 'C:\\L\\agy\\bin'"), "{c}");
        assert!(Installation { kind: Kind::Codex, path: "x".into(), method: Method::StoreApp }.uninstall_command().is_none());
    }

    #[test]
    fn finds_something_here() {
        for i in find_all() {
            eprintln!("{:?} {} {}", i.kind, i.path.display(), i.how());
        }
    }
}
