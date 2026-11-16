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
        let mut edit = false;
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
                    if ui.button("Edit scripts…").clicked() {
                        edit = true;
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
                ui.label(RichText::new("No scripts yet. Put .ps1, .cmd, .bat or .py files in the scripts folder (Open folder), or ask an agent to write one (Edit scripts… > Copy prompt).").color(TEXT_FAINT));
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
        if edit {
            self.options = Some(OptPage::Scripts);
            close = true;
        }
        if close {
            self.scripts_open = false;
            self.script_filter.clear();
            self.focus_terminal = true;
        }
    }
}

/// A script being edited in Options > Scripts. Unsaved edits survive closing Options (until the
/// app exits); nothing is written until Save.
pub(super) struct ScriptDraft {
    /// The file on disk (None = new, not saved yet).
    path: Option<PathBuf>,
    name: String,
    ext: String,
    text: String,
    saved_name: String,
    saved_text: String,
    confirm_delete: bool,
    error: Option<String>,
}

impl ScriptDraft {
    fn dirty(&self) -> bool {
        self.path.is_none() || self.name != self.saved_name || self.text != self.saved_text
    }

    fn from_file(s: &Script) -> Self {
        let text = std::fs::read_to_string(&s.path).unwrap_or_default().trim_start_matches('\u{feff}').replace("\r\n", "\n");
        let ext = s.path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
        ScriptDraft { path: Some(s.path.clone()), name: s.name.clone(), ext, saved_name: s.name.clone(), saved_text: text.clone(), text, confirm_delete: false, error: None }
    }

    /// Writes the file, renaming it first when the name changed. Batch files get CRLF.
    fn save(&mut self) -> Result<(), String> {
        self.save_in(&scripts_dir())
    }

    fn save_in(&mut self, dir: &Path) -> Result<(), String> {
        let name = self.name.trim().to_string();
        if name.is_empty() {
            return Err("Give the script a name.".into());
        }
        if name.chars().any(|c| r#"<>:"/\|?*"#.contains(c) || c.is_control()) {
            return Err(r#"The name can't contain < > : " / \ | ? *"#.into());
        }
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let target = dir.join(format!("{name}.{}", self.ext));
        let same = |a: &Path, b: &Path| a.to_string_lossy().eq_ignore_ascii_case(&b.to_string_lossy());
        let moved = self.path.as_deref().is_some_and(|p| p != target);
        // A different file already has that name (a case-only rename is the same file).
        if target.exists() && !self.path.as_deref().is_some_and(|p| same(p, &target)) {
            return Err(format!("{name}.{} already exists.", self.ext));
        }
        if let (Some(old), true) = (&self.path, moved) {
            std::fs::rename(old, &target).map_err(|e| format!("Couldn't rename: {e}"))?;
            self.path = Some(target.clone());
        }
        let body = if matches!(self.ext.as_str(), "cmd" | "bat") { self.text.replace('\n', "\r\n") } else { self.text.clone() };
        std::fs::write(&target, body).map_err(|e| format!("Couldn't save: {e}"))?;
        self.path = Some(target);
        self.name = name.clone();
        self.saved_name = name;
        self.saved_text = self.text.clone();
        Ok(())
    }
}

/// A starter request for an agent to write a script (Options > Scripts > Copy prompt).
fn agent_prompt(dir: &Path) -> String {
    format!(
        "Write a script for AI Terminal's Scripts palette and save it in {} (a .ps1 file unless I ask otherwise; .cmd, .bat and .py also work). \
The file name, without the extension, is the name shown in the palette. Start the file with a one-line comment describing what it does: it's shown as the description. \
It runs in the current project's folder in a terminal tab, so it can print output and ask questions. For git commands, set $env:GIT_PAGER = 'cat'. \
If it needs an LLM, call `claude -p --model haiku --tools '\"\"' --strict-mcp-config --no-session-persistence --system-prompt \"...\"` with only the input it needs, to keep it cheap. \
The script should: ",
        dir.display()
    )
}

/// Starting text for a new script of that type.
fn template(ext: &str) -> String {
    match ext {
        "cmd" | "bat" => "@echo off\nREM What this script does\n".into(),
        "ps1" => "# What this script does\n$env:GIT_PAGER = 'cat'\n".into(),
        _ => "# What this script does\n".into(),
    }
}

impl App {
    /// Reloads the editor list from disk, keeping unsaved edits and new scripts.
    fn sync_script_drafts(&mut self) {
        let mut old = std::mem::take(&mut self.script_drafts);
        let mut v = Vec::new();
        for f in scan() {
            match old.iter().position(|d| d.path.as_ref() == Some(&f.path) && d.dirty()) {
                Some(i) => v.push(old.remove(i)),
                None => v.push(ScriptDraft::from_file(&f)),
            }
        }
        v.extend(old.into_iter().filter(|d| d.path.is_none()));
        self.script_drafts = v;
    }

    pub(super) fn options_scripts(&mut self, ui: &mut Ui) {
        if !self.scripts_synced {
            self.sync_script_drafts();
            self.scripts_synced = true;
        }
        let dir = scripts_dir();
        ui.label(RichText::new("Scripts").family(bold()).size(18.0).color(TEXT));
        ui.label(
            RichText::new(format!(
                "Files in {}. Run one from the Scripts button in the session header or the palette (Ctrl+Shift+R): it runs in the current project as its own session. The first comment line is shown as its description.",
                dir.display()
            ))
            .size(12.5)
            .color(TEXT_DIM),
        );
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Tip: ask an agent to write a script for you. Copy prompt gives you the start of a request with the folder and conventions filled in; add what the script should do.").size(12.5).color(TEXT_DIM));
        });
        ui.horizontal(|ui| {
            if ui.button("Copy prompt").on_hover_text("Copy a starter request to paste into a Claude or Codex session").clicked() {
                ui.ctx().copy_text(agent_prompt(&dir));
                self.toast("Prompt copied: paste it into a session and say what the script should do".into(), GREEN, ui.ctx());
            }
            if ui.button("Open folder").clicked() {
                let _ = std::fs::create_dir_all(&dir);
                let _ = std::process::Command::new("explorer.exe").arg(&dir).spawn();
            }
            if ui.button("Refresh").on_hover_text("Reload from disk (unsaved edits are kept)").clicked() {
                self.sync_script_drafts();
            }
        });
        ui.add_space(8.0);
        let mut remove = None;
        let mut open: Option<PathBuf> = None;
        for (i, d) in self.script_drafts.iter_mut().enumerate() {
            egui::Frame::new().fill(CARD).corner_radius(8).stroke(Stroke::new(1.0, BORDER)).inner_margin(10).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut d.name).hint_text("Name").desired_width(260.0));
                    ui.label(RichText::new(format!(".{}", d.ext)).color(TEXT_FAINT));
                    if d.dirty() {
                        widgets::pill(ui, if d.path.is_none() { "new" } else { "unsaved" }, Color32::WHITE, Color32::from_rgb(150, 80, 20));
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let del = if d.confirm_delete { egui::Button::new(RichText::new("Delete file?").color(Color32::WHITE)).fill(Color32::from_rgb(150, 40, 40)) } else { egui::Button::new("Delete") };
                        if ui.add(del).clicked() {
                            if d.confirm_delete || d.path.is_none() {
                                remove = Some(i);
                            } else {
                                d.confirm_delete = true;
                            }
                        }
                        if d.path.is_some() && ui.button("Open in editor").clicked() {
                            open = d.path.clone();
                        }
                        if d.path.is_some() && d.dirty() && ui.button("Revert").clicked() {
                            d.name = d.saved_name.clone();
                            d.text = d.saved_text.clone();
                            d.error = None;
                        }
                        if ui.add_enabled(d.dirty(), egui::Button::new(RichText::new("Save").color(Color32::WHITE)).fill(ACCENT)).clicked() {
                            d.error = d.save().err();
                            d.confirm_delete = false;
                        }
                    });
                });
                egui::ScrollArea::vertical().id_salt(("script_text", i)).max_height(360.0).show(ui, |ui| {
                    ui.add(egui::TextEdit::multiline(&mut d.text).code_editor().desired_rows(4).desired_width(f32::INFINITY));
                });
                if let Some(e) = &d.error {
                    ui.label(RichText::new(e).color(RED));
                }
            });
            ui.add_space(6.0);
        }
        if let Some(i) = remove {
            let d = self.script_drafts.remove(i);
            if let Some(p) = d.path {
                if let Err(e) = std::fs::remove_file(&p) {
                    self.toast(format!("Couldn't delete {}: {e}", p.display()), RED, ui.ctx());
                }
            }
        }
        if let Some(p) = open {
            if let Err(e) = crate::links::open_file(&p, None, None, &self.s.editor, &self.s.editor_custom) {
                self.toast(format!("Couldn't open the editor: {e}"), RED, ui.ctx());
            }
        }
        ui.horizontal(|ui| {
            if ui.add(egui::Button::new(RichText::new("+ Add script").color(Color32::WHITE)).fill(ACCENT)).clicked() {
                let taken = |n: &str| self.script_drafts.iter().any(|d| d.name.eq_ignore_ascii_case(n));
                let name = (1..).map(|k| if k == 1 { "New script".to_string() } else { format!("New script {k}") }).find(|n| !taken(n)).unwrap();
                let ext = self.script_new_ext.clone();
                self.script_drafts.push(ScriptDraft { path: None, saved_name: String::new(), saved_text: String::new(), text: template(&ext), name, ext, confirm_delete: false, error: None });
            }
            egui::ComboBox::from_id_salt("script_new_ext").selected_text(format!(".{}", self.script_new_ext)).width(70.0).show_ui(ui, |ui| {
                for k in KINDS {
                    ui.selectable_value(&mut self.script_new_ext, k.to_string(), format!(".{k}"));
                }
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::{first_comment, ScriptDraft};

    #[test]
    fn save_rename_and_conflicts() {
        let dir = std::env::temp_dir().join(format!("ai_terminal_scripts_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let new = |name: &str, ext: &str, text: &str| ScriptDraft { path: None, name: name.into(), ext: ext.into(), text: text.into(), saved_name: String::new(), saved_text: String::new(), confirm_delete: false, error: None };
        let mut a = new("One", "cmd", "@echo off\necho 1\n");
        a.save_in(&dir).unwrap();
        assert!(!a.dirty());
        assert_eq!(std::fs::read_to_string(dir.join("One.cmd")).unwrap(), "@echo off\r\necho 1\r\n");
        // Same name as an existing file.
        assert!(new("one", "cmd", "").save_in(&dir).is_err());
        assert!(new("bad:name", "ps1", "").save_in(&dir).is_err());
        // Rename, then a case-only rename.
        a.name = "Two".into();
        a.save_in(&dir).unwrap();
        assert!(!dir.join("One.cmd").exists() && dir.join("Two.cmd").exists());
        a.name = "two".into();
        a.save_in(&dir).unwrap();
        let names: Vec<String> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(names, vec!["two.cmd".to_string()]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn script_descriptions() {
        assert_eq!(first_comment("# Push it\nrest").as_deref(), Some("Push it"));
        assert_eq!(first_comment("\u{feff}@echo off\r\nREM Build all\r\n").as_deref(), Some("Build all"));
        assert_eq!(first_comment(":: Hi").as_deref(), Some("Hi"));
        assert_eq!(first_comment("#!/usr/bin/env python\n# Tool").as_deref(), Some("Tool"));
        assert_eq!(first_comment("git push").as_deref(), None);
    }
}
