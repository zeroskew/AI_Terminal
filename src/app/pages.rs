//! The home page and an empty project's page.

use super::*;

impl App {
    /// Home: a card per project (all of them, whether or not they have a tab).
    pub(super) fn home(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let mut open: Option<PathBuf> = None;
        let mut set_tab: Option<(PathBuf, bool)> = None;
        let mut remove: Option<PathBuf> = None;
        let tab_order = self.tab_projects();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show_hinted(ui, |ui| {
            ui.add_space(24.0);
            ui.horizontal(|ui| {
                ui.add_space(24.0);
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        if let Some(tex) = &self.logo {
                            ui.add(egui::Image::new((tex.id(), Vec2::splat(32.0))));
                        }
                        ui.label(RichText::new("AI Terminal  ·  Projects").family(bold()).size(22.0).color(TEXT));
                    });
                    ui.label(RichText::new("Each project is a folder; its sessions start there. \"Show in tabs\" puts it on the tab bar; Ctrl+1…9 or Ctrl+PgUp / PgDn switch tabs.").color(TEXT_DIM));
                    ui.add_space(14.0);
                    let card_w = 300.0;
                    let cols = ((ui.available_width() - 24.0) / (card_w + 12.0)).floor().max(1.0) as usize;
                    egui::Grid::new("home_cards").spacing([12.0, 12.0]).show(ui, |ui| {
                        let projects = self.s.projects.clone();
                        for (i, proj) in projects.iter().enumerate() {
                            let n = self.project_session_count(&proj.path);
                            let unseen = self.tabs.iter().any(|t| t.project == proj.path && t.sess.unseen);
                            let (rect, resp) = ui.allocate_exact_size(Vec2::new(card_w, 108.0), Sense::click());
                            let p = ui.painter();
                            p.rect_filled(rect, 8.0, if resp.hovered() { CARD_HI } else { CARD });
                            p.rect_stroke(rect, 8.0, Stroke::new(1.0, if resp.hovered() { ACCENT } else { BORDER }), egui::StrokeKind::Inside);
                            widgets::paint_icon(p, Icon::Folder, egui::pos2(rect.left() + 22.0, rect.top() + 24.0), 16.0, Color32::from_rgb(222, 184, 92));
                            p.text(egui::pos2(rect.left() + 40.0, rect.top() + 24.0), egui::Align2::LEFT_CENTER, &proj.name, egui::FontId::new(15.0, bold()), TEXT);
                            let mut job = egui::text::LayoutJob::simple_singleline(proj.path.display().to_string(), egui::FontId::proportional(12.0), TEXT_FAINT);
                            job.wrap.max_width = card_w - 32.0;
                            job.wrap.max_rows = 1;
                            job.wrap.break_anywhere = true;
                            let g = ui.fonts_mut(|f| f.layout_job(job));
                            p.galley(egui::pos2(rect.left() + 16.0, rect.top() + 44.0), g, TEXT_FAINT);
                            let status = format!("{n} session{}{}", if n == 1 { "" } else { "s" }, if unseen { "  ·  new output" } else { "" });
                            p.text(egui::pos2(rect.left() + 16.0, rect.top() + 70.0), egui::Align2::LEFT_CENTER, status, egui::FontId::proportional(12.5), if unseen { ORANGE } else { TEXT_DIM });
                            if let Some(k) = tab_order.iter().position(|x| *x == proj.path).filter(|k| *k < 9) {
                                p.text(egui::pos2(rect.right() - 14.0, rect.top() + 24.0), egui::Align2::RIGHT_CENTER, format!("Ctrl+{}", k + 1), egui::FontId::proportional(11.0), TEXT_FAINT);
                            }
                            // Bottom-right controls, added after the card so they take the clicks.
                            // A child Ui (not a grid cell), so the grid layout is unaffected.
                            let ctl = egui::Rect::from_min_max(egui::pos2(rect.left() + 12.0, rect.bottom() - 30.0), egui::pos2(rect.right() - 10.0, rect.bottom() - 6.0));
                            let mut shown = proj.in_tabs;
                            {
                                let ui = &mut ui.new_child(egui::UiBuilder::new().max_rect(ctl).layout(Layout::right_to_left(Align::Center)));
                                ui.spacing_mut().item_spacing.x = 10.0;
                                // Painted by hand so the text can turn red on hover.
                                let g = ui.painter().layout_no_wrap("Remove".into(), egui::FontId::proportional(12.0), TEXT_DIM);
                                let (rr, rm) = ui.allocate_exact_size(g.size() + Vec2::new(8.0, 4.0), Sense::click());
                                if rm.hovered() {
                                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                                    ui.painter().rect_filled(rr, 4.0, RED.gamma_multiply(0.18));
                                    ui.painter().rect_stroke(rect, 8.0, Stroke::new(1.0, RED), egui::StrokeKind::Inside);
                                }
                                ui.painter().galley_with_override_text_color(rr.center() - g.size() / 2.0, g, if rm.hovered() { RED } else { TEXT_DIM });
                                if rm.on_hover_text("Remove this project from AI Terminal (asks first; files are not deleted)").clicked() {
                                    remove = Some(proj.path.clone());
                                }
                                let cb = ui.checkbox(&mut shown, RichText::new("Show in tabs").size(12.0).color(TEXT_DIM));
                                if cb.on_hover_text("Show this project as a tab in the top bar. Unticking only closes the tab; the project stays here.").changed() {
                                    set_tab = Some((proj.path.clone(), shown));
                                }
                            }
                            if resp.clicked() {
                                open = Some(proj.path.clone());
                            }
                            if (i + 1) % cols == 0 {
                                ui.end_row();
                            }
                        }
                        let (rect, resp) = ui.allocate_exact_size(Vec2::new(card_w, 108.0), Sense::click());
                        let p = ui.painter();
                        p.rect_stroke(rect, 8.0, Stroke::new(1.0, if resp.hovered() { ACCENT_HI } else { BORDER }), egui::StrokeKind::Inside);
                        widgets::paint_icon(p, Icon::Plus, egui::pos2(rect.center().x - 50.0, rect.center().y), 16.0, TEXT_DIM);
                        p.text(egui::pos2(rect.center().x - 36.0, rect.center().y), egui::Align2::LEFT_CENTER, "Add project", egui::FontId::new(14.0, bold()), TEXT_DIM);
                        if resp.clicked() {
                            self.add_project(&ctx);
                        }
                    });
                });
            });
        });
        let toggled = set_tab.is_some();
        if let Some((p, show)) = set_tab {
            if show {
                if let Some(x) = self.s.projects.iter_mut().find(|x| x.path == p) {
                    x.in_tabs = true;
                }
            } else {
                self.hide_project_tab(&p);
            }
        }
        if let Some(p) = remove {
            self.confirm_remove_project = Some(p);
        } else if let Some(p) = open.filter(|_| !toggled) {
            self.select_project(Some(p));
        }
    }

    /// Central area of a project with no session open.
    pub(super) fn empty_project(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let project = self.current_project();
        let name = self.s.projects.iter().find(|p| p.path == project).map(|p| p.name.clone()).unwrap_or_default();
        let listed = self.project_session_count(&project);
        ui.painter().rect_filled(ui.max_rect(), 0.0, BG);
        ui.vertical_centered(|ui| {
            ui.add_space(ui.available_height() * 0.33);
            if let Some(tex) = &self.logo {
                ui.add(egui::Image::new((tex.id(), Vec2::splat(56.0))));
                ui.add_space(8.0);
            }
            ui.label(RichText::new(name).family(bold()).size(20.0).color(TEXT));
            ui.label(RichText::new(project.display().to_string()).color(TEXT_FAINT));
            ui.add_space(16.0);
            let (def, variant) = self.default_launch();
            let label = launch::launch_label(&def, variant);
            // The two buttons side by side, centred (primary_button is text width + 40).
            let text_w = |t: &str| ui.painter().layout_no_wrap(t.to_string(), egui::FontId::new(13.5, bold()), TEXT).size().x + 40.0;
            let gap = 10.0;
            let total = text_w(&label) + gap + text_w("Run script");
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                ui.add_space(((ui.available_width() - total) / 2.0).max(0.0));
                if widgets::primary_button(ui, agent_icon(&def).0, &label, ACCENT, "The default session (set it in the ▼ New-session menu)").clicked() {
                    self.launch_default(&ctx, project.clone());
                }
                ui.add_space(gap);
                if widgets::primary_button(ui, Icon::Prompt, "Run script", CARD_HI, "Run a script from the scripts folder in this project, in the background (Ctrl+Shift+R)").clicked() {
                    self.open_scripts();
                }
            });
            ui.add_space(10.0);
            if is_home_dir(&project) {
                ui.add_space(14.0);
                ui.label(RichText::new("Claude Code asks to trust your home folder every time (it never remembers it there).").color(ORANGE).size(12.5));
                ui.label(RichText::new("Add a project folder (Add Project, top right) to avoid the prompt.").color(TEXT_DIM).size(12.5));
            }
            let hint = if listed > 0 { "or pick a session on the left to resume it" } else { "▼ next to New session on the left starts another shell" };
            ui.label(RichText::new(hint).color(TEXT_DIM).size(12.5));
        });
    }
}
