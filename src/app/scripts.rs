//! Scripts: files in the `scripts` folder next to the executable, run from a palette in the
//! current project as their own shell session (output stays in the tab; Restart runs it again).

use super::*;

/// A runnable file in the scripts folder.
#[derive(Clone)]
pub(super) struct Script {
    pub name: String,
    pub path: PathBuf,
    /// The first comment line, else the file type.
    pub desc: String,
}

/// File types the palette lists.
const KINDS: [&str; 4] = ["ps1", "cmd", "bat", "py"];

const SAMPLE_NAME: &str = "Commit and push.ps1";
const SAMPLE: &str = include_str!("../../assets/scripts/Commit and push.ps1");

/// `<exe folder>\scripts`.
pub(super) fn scripts_dir() -> PathBuf {
    std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join("scripts"))).unwrap_or_else(|| PathBuf::from("scripts"))
}

/// Lists the scripts, sorted by name. Creates the folder with a sample the first time.
fn scan() -> Vec<Script> {
    let dir = scripts_dir();
    if !dir.exists() && std::fs::create_dir_all(&dir).is_ok() {
        let _ = std::fs::write(dir.join(SAMPLE_NAME), SAMPLE);
    }
    let mut v: Vec<Script> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .filter_map(|path| {
            let ext = path.extension()?.to_str()?.to_lowercase();
            if !KINDS.contains(&ext.as_str()) {
                return None;
            }
            let name = path.file_stem()?.to_string_lossy().into_owned();
            let desc = first_comment(&std::fs::read_to_string(&path).unwrap_or_default()).unwrap_or_else(|| format!(".{ext} script"));
            Some(Script { name, path, desc })
        })
        .collect();
    v.sort_by_key(|s| s.name.to_lowercase());
    v
}

/// The first comment line (`#`, `::`, `REM`), skipping `@echo off` and blank lines.
fn first_comment(text: &str) -> Option<String> {
    for line in text.trim_start_matches('\u{feff}').lines().take(10) {
        let l = line.trim();
        if l.is_empty() || l.eq_ignore_ascii_case("@echo off") || l.starts_with("#!") {
            continue;
        }
        let body = if let Some(b) = l.strip_prefix("::") {
            b
        } else if l.len() >= 4 && l[..4].eq_ignore_ascii_case("rem ") {
            &l[4..]
        } else if let Some(b) = l.strip_prefix('#') {
            b
        } else {
            return None;
        };
        let b = body.trim();
        if !b.is_empty() {
            return Some(b.to_string());
        }
    }
    None
}

impl App {
    pub(super) fn open_scripts(&mut self) {
        if self.s.projects.is_empty() {
            return;
        }
        self.scripts = scan();
        self.scripts_open = true;
        self.script_focus = true;
    }

    /// Test hook (--run-script <name>): runs the script with that name in the current project.
    pub(super) fn run_script_named(&mut self, ctx: &egui::Context, name: &str) {
        if let Some(sc) = scan().into_iter().find(|s| s.name.eq_ignore_ascii_case(name)) {
            self.run_script(ctx, &sc);
        }
    }

    /// Starts a script in the current project as a new session named after it.
    fn run_script(&mut self, ctx: &egui::Context, sc: &Script) {
        let path = sc.path.display().to_string();
        let ext = sc.path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
        let (program, args): (String, Vec<String>) = match ext.as_str() {
            "ps1" => {
                let ps = self.profiles.iter().find(|p| p.name == "pwsh").map(|p| p.program.clone()).unwrap_or_else(|| "powershell.exe".into());
                (ps, ["-NoLogo", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", &path].map(String::from).to_vec())
            }
            "py" => ("python.exe".into(), vec![path]),
            _ => (std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".into()), vec!["/c".into(), path]),
        };
        let profile = Profile { name: sc.name.clone(), program, args, kind: Kind::Shell };
        let project = self.current_project();
        self.spawn_with(ctx, project, profile, sc.name.clone(), None, LaunchOpts::default(), now_secs());
    }

    /// The scripts palette (Ctrl+Shift+R or the header button): filter, Enter = first match.
    pub(super) fn scripts_palette(&mut self, ctx: &egui::Context) {
        if !self.scripts_open {
            return;
        }
        let mut pick: Option<usize> = None;
        let mut close = false;
        let project = project_name(&self.current_project());
        let m = egui::Modal::new(egui::Id::unique("scripts")).frame(egui::Frame::new().fill(PANEL).corner_radius(10).stroke(Stroke::new(1.0, BORDER)).inner_margin(16)).show(ctx, |ui| {
            ui.set_width(560.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("SCRIPTS").size(12.0).family(bold()).color(TEXT_DIM));
                ui.label(RichText::new(format!("run in {project}")).size(12.0).color(TEXT_FAINT));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if widgets::icon_button(ui, Icon::Close, 24.0, TEXT_DIM, "Close (Esc)").clicked() {
                        close = true;
                    }
                    if ui.button("Open folder").on_hover_text(scripts_dir().display().to_string()).clicked() {
                        let _ = std::process::Command::new("explorer.exe").arg(scripts_dir()).spawn();
                    }
                    if ui.button("Refresh").clicked() {
                        self.scripts = scan();
                    }
                });
            });
            let te = ui.add(egui::TextEdit::singleline(&mut self.script_filter).hint_text("Filter").desired_width(f32::INFINITY));
            if std::mem::take(&mut self.script_focus) {
                te.request_focus();
            }
            ui.add_space(6.0);
            let f = self.script_filter.to_lowercase();
            let shown: Vec<usize> = (0..self.scripts.len()).filter(|&i| f.is_empty() || self.scripts[i].name.to_lowercase().contains(&f) || self.scripts[i].desc.to_lowercase().contains(&f)).collect();
            if te.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                pick = shown.first().copied();
            }
            if self.scripts.is_empty() {
                ui.label(RichText::new("No scripts yet. Put .ps1, .cmd, .bat or .py files in the scripts folder (Open folder).").color(TEXT_FAINT));
            }
            egui::ScrollArea::vertical().max_height(420.0).show(ui, |ui| {
                for i in shown {
                    let sc = &self.scripts[i];
                    let ext = sc.path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
                    if super::snippets::palette_row(ui, &sc.name, &sc.desc, &ext).clicked() {
                        pick = Some(i);
                    }
                }
            });
        });
        if m.should_close() {
            close = true;
        }
        if let Some(i) = pick {
            let sc = self.scripts[i].clone();
            self.run_script(ctx, &sc);
            close = true;
        }
        if close {
            self.scripts_open = false;
            self.script_filter.clear();
            self.focus_terminal = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::first_comment;

    #[test]
    fn script_descriptions() {
        assert_eq!(first_comment("# Push it\nrest").as_deref(), Some("Push it"));
        assert_eq!(first_comment("\u{feff}@echo off\r\nREM Build all\r\n").as_deref(), Some("Build all"));
        assert_eq!(first_comment(":: Hi").as_deref(), Some("Hi"));
        assert_eq!(first_comment("#!/usr/bin/env python\n# Tool").as_deref(), Some("Tool"));
        assert_eq!(first_comment("git push").as_deref(), None);
    }
}
