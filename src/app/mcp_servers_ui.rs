//! The MCP servers dialog: which of Claude Code's MCP servers are enabled for a project.
//!
//! - Servers from `claude mcp list` (claude.ai connectors, plugin, user and project servers) are
//!   switched with `disabledMcpServers` in `~/.claude.json` (the same list `/mcp` edits).
//! - Claude Code's built-in Artifacts server (`claude-artifacts`) isn't in that list; it's
//!   switched off with a deny rule for the `Artifact` tool in `.claude/settings.local.json`.

use super::*;
use crate::mcp_servers::{self, Server, ARTIFACTS};
use std::sync::{Arc, Mutex};

/// Servers this app provides to its own sessions; not Claude Code's to switch.
const OWN: &[&str] = &["ai-terminal"];

pub(super) struct McpServersUi {
    project: PathBuf,
    /// `claude mcp list` result, filled in by a thread.
    listed: Arc<Mutex<Option<Result<Vec<Server>, String>>>>,
    disabled: Vec<String>,
    saved: Vec<String>,
    /// Built-in Artifacts server: off for this project (deny rule in settings.local.json).
    artifacts_off: bool,
    artifacts_saved: bool,
    /// A deny rule in a settings file this dialog doesn't edit.
    artifacts_elsewhere: Option<PathBuf>,
    error: Option<String>,
}

/// One server row: checkbox, status on the right, then what it is and its address.
fn server_row(ui: &mut Ui, name: &str, status: &str, status_col: Color32, about: &str, target: &str, on: &mut bool, enabled: bool) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        let name_col = if *on { TEXT } else { TEXT_FAINT };
        changed = ui.add_enabled(enabled, egui::Checkbox::new(on, RichText::new(name).color(name_col))).changed();
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(RichText::new(status).color(status_col).size(12.0));
        });
    });
    ui.horizontal(|ui| {
        ui.add_space(26.0);
        ui.vertical(|ui| {
            ui.label(RichText::new(about).color(TEXT_DIM).size(12.0));
            if !target.is_empty() {
                ui.label(RichText::new(target).color(TEXT_FAINT).size(11.5));
            }
        });
    });
    ui.add_space(8.0);
    changed
}

impl App {
    pub(super) fn open_mcp_servers(&mut self, project: PathBuf, ctx: &egui::Context) {
        let listed = Arc::new(Mutex::new(None));
        let (cli, cli_args) = self.claude_cli();
        let (out, cwd, ctx) = (listed.clone(), project.clone(), ctx.clone());
        std::thread::spawn(move || {
            let r = mcp_servers::list(&cli, &cli_args, &cwd);
            *out.lock().unwrap() = Some(r);
            ctx.request_repaint();
        });
        let disabled = mcp_servers::disabled(&project);
        let (artifacts_off, artifacts_elsewhere) = mcp_servers::artifacts_denied(&project);
        self.mcp_servers = Some(McpServersUi { project, listed, saved: disabled.clone(), disabled, artifacts_off, artifacts_saved: artifacts_off, artifacts_elsewhere, error: None });
    }

    pub(super) fn mcp_servers_window(&mut self, ctx: &egui::Context) {
        let Some(st) = &mut self.mcp_servers else { return };
        let listed = st.listed.lock().unwrap().clone();
        // Disabled servers may not show in the list; keep them so they can be switched back on.
        let mut rows: Vec<Server> = match &listed {
            Some(Ok(v)) => v.iter().filter(|s| !OWN.contains(&s.name.as_str())).cloned().collect(),
            _ => Vec::new(),
        };
        for d in &st.disabled {
            if !rows.iter().any(|s| &s.name == d) {
                rows.push(Server { name: d.clone(), target: String::new(), status: "disabled".into() });
            }
        }
        let mut close = false;
        let mut save = false;
        let m = egui::Modal::new(egui::Id::unique("mcp_servers")).frame(egui::Frame::new().fill(PANEL).corner_radius(10).stroke(Stroke::new(1.0, BORDER)).inner_margin(18)).show(ctx, |ui| {
            ui.set_width(680.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("MCP SERVERS").size(12.5).family(bold()).color(TEXT_DIM));
                ui.label(RichText::new(project_name(&st.project)).size(12.5).color(TEXT));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if widgets::close_button(ui).clicked() {
                        close = true;
                    }
                });
            });
            ui.add_space(4.0);
            ui.label(RichText::new("Each enabled server adds its tools to every request. Switch off the ones this project doesn't need. Run /context in a session to see what each one costs.").color(TEXT_DIM).size(12.5));
            ui.add_space(12.0);
            egui::ScrollArea::vertical().max_height(480.0).auto_shrink([false, true]).show_hinted(ui, |ui| {
                // Built in.
                ui.label(RichText::new("BUILT INTO CLAUDE CODE").size(11.5).family(bold()).color(TEXT_FAINT));
                ui.add_space(6.0);
                let mut on = !st.artifacts_off && st.artifacts_elsewhere.is_none();
                let (status, col) = match &st.artifacts_elsewhere {
                    Some(_) => ("off (other settings)", TEXT_FAINT),
                    None if on => ("on", GREEN),
                    None => ("off", TEXT_FAINT),
                };
                let changed = server_row(ui, ARTIFACTS, status, col, mcp_servers::describe(ARTIFACTS), "https://api.anthropic.com/api/artifacts/mcp", &mut on, st.artifacts_elsewhere.is_none());
                if changed {
                    st.artifacts_off = !on;
                }
                if let Some(f) = &st.artifacts_elsewhere {
                    ui.label(RichText::new(format!("Denied in {}; change it there.", f.display())).color(ORANGE).size(12.0));
                    ui.add_space(8.0);
                }
                // From `claude mcp list`.
                ui.add_space(6.0);
                ui.label(RichText::new("CONNECTORS, PLUGINS AND YOUR SERVERS").size(11.5).family(bold()).color(TEXT_FAINT));
                ui.add_space(6.0);
                match &listed {
                    None => {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(RichText::new("Checking servers (claude mcp list)…").color(TEXT_DIM));
                        });
                    }
                    Some(Err(e)) => {
                        ui.label(RichText::new(e).color(RED));
                    }
                    Some(Ok(_)) if rows.is_empty() => {
                        ui.label(RichText::new("None for this project.").color(TEXT_FAINT));
                    }
                    _ => {}
                }
                for s in &rows {
                    let mut on = !st.disabled.contains(&s.name);
                    let col = if s.status.contains("Connected") { GREEN } else if s.status == "disabled" { TEXT_FAINT } else { ORANGE };
                    if server_row(ui, &s.name, &s.status, col, mcp_servers::describe(&s.name), &s.target, &mut on, true) {
                        if on {
                            st.disabled.retain(|d| d != &s.name);
                        } else {
                            st.disabled.push(s.name.clone());
                        }
                    }
                }
            });
            if let Some(e) = &st.error {
                ui.label(RichText::new(e).color(RED));
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Applies to sessions started or restarted after saving.").color(TEXT_FAINT).size(12.0));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let changed = st.artifacts_off != st.artifacts_saved || {
                        let (mut a, mut b) = (st.disabled.clone(), st.saved.clone());
                        a.sort();
                        b.sort();
                        a != b
                    };
                    let tip = "Connectors / plugins / your servers: disabledMcpServers for this project in ~/.claude.json (what /mcp uses).\nclaude-artifacts: a deny rule for the Artifact tool in <project>\\.claude\\settings.local.json.";
                    if ui.add_enabled(changed, egui::Button::new(RichText::new("Save").color(Color32::WHITE)).fill(ACCENT)).on_hover_text(tip).clicked() {
                        save = true;
                    }
                    if ui.button("Cancel").clicked() {
                        close = true;
                    }
                    // Switch every server off, or back on once they all are.
                    let artifacts_switchable = st.artifacts_elsewhere.is_none();
                    let any_on = (artifacts_switchable && !st.artifacts_off) || rows.iter().any(|s| !st.disabled.contains(&s.name));
                    let can = artifacts_switchable || !rows.is_empty();
                    let label = if any_on { "Disable all" } else { "Enable all" };
                    if ui.add_enabled(can, egui::Button::new(label)).clicked() {
                        if any_on {
                            for s in &rows {
                                if !st.disabled.contains(&s.name) {
                                    st.disabled.push(s.name.clone());
                                }
                            }
                        } else {
                            st.disabled.retain(|d| !rows.iter().any(|s| &s.name == d));
                        }
                        if artifacts_switchable {
                            st.artifacts_off = any_on;
                        }
                    }
                });
            });
        });
        if save {
            let mut r = Ok(());
            if st.disabled.iter().collect::<std::collections::BTreeSet<_>>() != st.saved.iter().collect() {
                r = mcp_servers::set_disabled(&st.project, &st.disabled);
            }
            if r.is_ok() && st.artifacts_off != st.artifacts_saved {
                r = mcp_servers::set_artifacts_denied(&st.project, st.artifacts_off);
            }
            match r {
                Ok(()) => {
                    let n = st.disabled.len() + st.artifacts_off as usize;
                    self.mcp_servers = None;
                    self.toast(format!("MCP servers saved ({n} off). Restart sessions to apply."), GREEN, ctx);
                    return;
                }
                Err(e) => st.error = Some(e),
            }
        }
        if close || m.should_close() {
            self.mcp_servers = None;
        }
    }
}
