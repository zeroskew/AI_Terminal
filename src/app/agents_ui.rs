//! Options > Agents: every agent CLI in `agents::AGENTS`, where each copy is installed, and
//! Install / Uninstall. Both run in the background (a panel at the bottom right shows the
//! output), and the New-session menu picks up the change when they finish.

use super::*;
use crate::agent_install::{self, Installation, Method};
use crate::agents::{AgentSpec, AGENTS};

/// PowerShell arguments that run `script` (passed encoded, so no quoting issues).
fn ps_args(script: &str) -> Vec<String> {
    ["-NoLogo", "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-EncodedCommand"].map(String::from).into_iter().chain([agent_install::encode_ps(script)]).collect()
}

fn home_dir() -> PathBuf {
    std::env::var_os("USERPROFILE").map(PathBuf::from).unwrap_or_else(std::env::temp_dir)
}

impl App {
    /// Looks for the agent CLIs again: the New-session menu and Options > Agents.
    pub(super) fn redetect_agents(&mut self, ctx: &egui::Context) {
        self.profiles = Profile::detect();
        self.profiles_at = std::time::Instant::now();
        self.agent_installs = None;
        self.refresh_catalog(ctx);
    }

    /// Runs the agent's install command in the background.
    pub(super) fn install_agent(&mut self, ctx: &egui::Context, spec: &AgentSpec) {
        let script = format!("$ErrorActionPreference = 'Stop'\n{}\nif ($LASTEXITCODE) {{ exit $LASTEXITCODE }}\n", spec.install);
        self.run_in_background(ctx, format!("Install {}", spec.name), self.pwsh(), ps_args(&script), home_dir(), Some(spec.kind));
    }

    fn uninstall_agent(&mut self, ctx: &egui::Context, inst: &Installation, name: &str) {
        let Some(script) = inst.uninstall_command() else { return };
        self.run_in_background(ctx, format!("Uninstall {name}"), self.pwsh(), ps_args(&script), home_dir(), Some(inst.kind));
    }

    pub(super) fn options_agents(&mut self, ui: &mut Ui) {
        // Re-scan every few seconds while the page is open (installs made elsewhere show up).
        let stale = self.agent_installs.as_ref().is_none_or(|(at, _)| at.elapsed() > Duration::from_secs(3));
        if stale {
            self.agent_installs = Some((std::time::Instant::now(), agent_install::find_all()));
        }
        let installs = self.agent_installs.as_ref().map(|(_, v)| v.clone()).unwrap_or_default();
        let ctx = ui.ctx().clone();
        ui.label(RichText::new("Agents").family(bold()).size(18.0).color(TEXT));
        ui.label(
            RichText::new("The agent CLIs AI Terminal can start. Install or uninstall them here; it runs in the background (output in a panel at the bottom right) and the New-session menu updates when it's done. Uninstalling removes the program only: settings, logins and conversations stay.")
                .size(12.5)
                .color(TEXT_DIM),
        );
        ui.horizontal(|ui| {
            if ui.button("⟳ Re-detect").on_hover_text("Look for the agent CLIs again").clicked() {
                self.redetect_agents(&ctx);
            }
        });
        ui.add_space(8.0);
        let mut install: Option<&AgentSpec> = None;
        let mut uninstall: Option<(Installation, &str)> = None;
        for spec in AGENTS.iter() {
            let mine: Vec<&Installation> = installs.iter().filter(|i| i.kind == spec.kind).collect();
            let busy = self.agent_job_running(spec.kind);
            // The copy new sessions start (detection's pick).
            let used = self.profiles.iter().find(|p| p.kind == spec.kind).map(|p| p.program.to_lowercase());
            let open = self.tabs.iter().filter(|t| t.sess.profile.kind == spec.kind && t.sess.exited.is_none()).count();
            egui::Frame::new().fill(CARD).corner_radius(8).stroke(Stroke::new(1.0, BORDER)).inner_margin(10).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    let probe = Profile { name: spec.name.into(), program: String::new(), args: vec![], kind: spec.kind };
                    let (r, _) = ui.allocate_exact_size(Vec2::splat(20.0), Sense::hover());
                    let (ic, col) = agent_icon(&probe);
                    widgets::paint_icon(ui.painter(), ic, r.center(), 15.0, col);
                    ui.label(RichText::new(spec.name).family(bold()).size(15.0).color(TEXT));
                    if busy {
                        widgets::pill(ui, "working…", Color32::WHITE, Color32::from_rgb(40, 90, 150));
                    } else if mine.is_empty() {
                        widgets::pill(ui, "not installed", TEXT_DIM, CARD_HI);
                    } else {
                        widgets::pill(ui, "installed", Color32::WHITE, Color32::from_rgb(30, 90, 50));
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.link("website").on_hover_text(spec.url).clicked() {
                            crate::links::open_url(spec.url);
                        }
                        let tip = format!("Runs in the background:\n{}", spec.install);
                        if mine.is_empty() && ui.add_enabled(!busy, egui::Button::new("Install")).on_hover_text(tip).clicked() {
                            install = Some(spec);
                        }
                    });
                });
                for inst in &mine {
                    ui.horizontal(|ui| {
                        ui.add_space(28.0);
                        let full = inst.path.display().to_string();
                        ui.label(RichText::new(shorten_path(&full, 48)).monospace().size(12.0).color(TEXT_DIM)).on_hover_text(&full);
                        ui.label(RichText::new(inst.how()).size(12.0).color(TEXT_FAINT));
                        let program = inst.path.display().to_string().to_lowercase();
                        if mine.len() > 1 && used.as_deref() == Some(program.as_str()) {
                            ui.label(RichText::new("· used for new sessions").size(12.0).color(GREEN));
                        }
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            match inst.method {
                                Method::StoreApp => {
                                    if ui.button("Apps settings").on_hover_text("It comes with the Codex app: uninstall the app in Settings > Apps").clicked() {
                                        crate::links::open_url("ms-settings:appsfeatures");
                                    }
                                }
                                Method::Manual => {
                                    ui.label(RichText::new("remove it yourself").size(12.0).color(TEXT_FAINT));
                                }
                                _ => {
                                    let confirming = self.agent_confirm.as_ref() == Some(&inst.path);
                                    let btn = if confirming { egui::Button::new(RichText::new("Uninstall?").color(Color32::WHITE)).fill(Color32::from_rgb(150, 40, 40)) } else { egui::Button::new("Uninstall") };
                                    let tip = format!("Runs in the background:\n{}", inst.uninstall_command().unwrap_or_default().trim());
                                    if ui.add_enabled(!busy, btn).on_hover_text(tip).clicked() {
                                        if confirming {
                                            uninstall = Some(((*inst).clone(), spec.name));
                                            self.agent_confirm = None;
                                        } else {
                                            self.agent_confirm = Some(inst.path.clone());
                                        }
                                    }
                                }
                            }
                            if ui.button("Open folder").clicked() {
                                if let Some(d) = inst.path.parent() {
                                    let _ = std::process::Command::new("explorer.exe").arg(d).spawn();
                                }
                            }
                        });
                    });
                }
                if open > 0 && !mine.is_empty() {
                    ui.horizontal(|ui| {
                        ui.add_space(28.0);
                        let s = if open == 1 { "" } else { "s" };
                        ui.label(RichText::new(format!("{open} {} session{s} open: close them before uninstalling, or files may be in use.", spec.name)).size(12.0).color(ORANGE));
                    });
                }
            });
            ui.add_space(6.0);
        }
        if let Some(spec) = install {
            self.install_agent(&ctx, spec);
        }
        if let Some((inst, name)) = uninstall {
            self.uninstall_agent(&ctx, &inst, name);
        }
    }
}

/// Shortens a long path for display by dropping middle folders:
/// `C:\Program Files\WindowsApps\…\resources\codex.exe`. Keeps the root
/// part and as many trailing components as fit in `max` characters.
fn shorten_path(path: &str, max: usize) -> String {
    if path.chars().count() <= max {
        return path.to_string();
    }
    let parts: Vec<&str> = path.split('\\').collect();
    if parts.len() < 3 {
        return path.to_string();
    }
    let len = |v: &[&str]| v.iter().map(|s| s.chars().count() + 1).sum::<usize>();
    // Always keep the first component (drive) and the file name.
    let mut head = 1;
    let mut tail = 1;
    // Grow the tail first (nearest folders matter most), then the head.
    while head + tail < parts.len() {
        let next = parts.len() - tail - 1;
        if len(&parts[..head]) + len(&parts[next..]) + 1 <= max {
            tail += 1;
        } else if len(&parts[..head + 1]) + len(&parts[parts.len() - tail..]) + 1 <= max {
            head += 1;
        } else {
            break;
        }
    }
    if head + tail >= parts.len() {
        return path.to_string();
    }
    format!("{}\\…\\{}", parts[..head].join("\\"), parts[parts.len() - tail..].join("\\"))
}
