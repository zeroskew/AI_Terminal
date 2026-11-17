//! The home page and an empty project's page.

use super::*;

impl App {
    /// Home: a card per project (all of them, whether or not they have a tab).
    pub(super) fn home(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let mut open: Option<PathBuf> = None;
        let mut set_tab: Option<(PathBuf, bool)> = None;
        let mut remove: Option<PathBuf> = None;
        let mut settings: Option<PathBuf> = None;
        let mut set_accent: Option<(PathBuf, Option<[u8; 3]>)> = None;
        let mut swatch_clicked = false;
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
                        // Home has no sidebar, so Options (gear) and plugins get their buttons here.
                        ui.add_space(12.0);
                        if widgets::icon_button(ui, Icon::Plugin, 28.0, TEXT_DIM, "Plugins").clicked() {
                            self.open_plugins();
                        }
                        if widgets::icon_button(ui, Icon::Gear, 28.0, TEXT_DIM, "Options").clicked() {
                            self.options = Some(OptPage::General);
                        }
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
                            let accent = proj.accent_color();
                            p.rect_filled(rect, 8.0, if resp.hovered() { CARD_HI } else { CARD });
                            if let Some(a) = accent {
                                // A soft wash from the top and an accent edge on the left, clipped to the card.
                                // Both shapes follow the card's 8px corners rather than overhanging them.
                                const R: f32 = 8.0;
                                const ARC_STEPS: usize = 8;
                                // Point on the corner circle centred at `c`, at angle `t` (radians, screen space).
                                let arc = |c: egui::Pos2, t: f32| egui::pos2(c.x + R * t.cos(), c.y + R * t.sin());
                                use std::f32::consts::{FRAC_PI_2, PI};
                                let head_h = 48.0;
                                let c = a.gamma_multiply(if resp.hovered() { 0.16 } else { 0.10 });
                                // Outline of the top band, clockwise: top-left arc, top-right arc, then straight down.
                                let mut pts = Vec::with_capacity(2 * ARC_STEPS + 4);
                                let tl = egui::pos2(rect.left() + R, rect.top() + R);
                                let tr = egui::pos2(rect.right() - R, rect.top() + R);
                                for i in 0..=ARC_STEPS {
                                    pts.push(arc(tl, PI + FRAC_PI_2 * i as f32 / ARC_STEPS as f32));
                                }
                                for i in 0..=ARC_STEPS {
                                    pts.push(arc(tr, PI + FRAC_PI_2 + FRAC_PI_2 * i as f32 / ARC_STEPS as f32));
                                }
                                pts.push(egui::pos2(rect.right(), rect.top() + head_h));
                                pts.push(egui::pos2(rect.left(), rect.top() + head_h));
                                let mut mesh = egui::Mesh::default();
                                let shade = |pt: egui::Pos2| {
                                    let f = 1.0 - ((pt.y - rect.top()) / head_h).clamp(0.0, 1.0);
                                    c.gamma_multiply(f)
                                };
                                let centre = egui::pos2(rect.center().x, rect.top() + head_h * 0.5);
                                mesh.colored_vertex(centre, shade(centre));
                                for &pt in &pts {
                                    mesh.colored_vertex(pt, shade(pt));
                                }
                                let n = pts.len() as u32;
                                for i in 0..n {
                                    mesh.add_triangle(0, 1 + i, 1 + (i + 1) % n);
                                }
                                p.add(egui::Shape::mesh(mesh));
                                // Left accent edge: the slice of the card left of `w`, so its ends curve with the corners.
                                let w = 3.5;
                                let t0 = ((R - w) / R).acos(); // angle (from the horizontal) where the arc meets x = left + w
                                let bl = egui::pos2(rect.left() + R, rect.bottom() - R);
                                let mut edge = Vec::with_capacity(2 * ARC_STEPS + 2);
                                // Clockwise on screen: up the left side from bottom to top, then down the right side.
                                for i in 0..=ARC_STEPS {
                                    edge.push(arc(bl, PI - t0 + t0 * i as f32 / ARC_STEPS as f32));
                                }
                                for i in 0..=ARC_STEPS {
                                    edge.push(arc(tl, PI + t0 * i as f32 / ARC_STEPS as f32));
                                }
                                p.add(egui::Shape::convex_polygon(edge, a, Stroke::NONE));
                            }
                            let border = match (resp.hovered(), accent) {
                                (true, Some(a)) => a,
                                (true, None) => ACCENT,
                                _ => BORDER,
                            };
                            p.rect_stroke(rect, 8.0, Stroke::new(1.0, border), egui::StrokeKind::Inside);
                            // The project's image if it has one, else the folder icon.
                            let image = self.project_image(&proj.path);
                            let name_x = if image.is_some() { 48.0 } else { 40.0 };
                            let drawn = image.is_some_and(|img| {
                                let ir = egui::Rect::from_center_size(egui::pos2(rect.left() + 26.0, rect.top() + 24.0), Vec2::splat(26.0));
                                self.paint_project_image(ui, &img, ir, 4)
                            });
                            let p = ui.painter();
                            if !drawn {
                                widgets::paint_icon(p, Icon::Folder, egui::pos2(rect.left() + 22.0, rect.top() + 24.0), 16.0, accent.unwrap_or(Color32::from_rgb(222, 184, 92)));
                            }
                            p.text(egui::pos2(rect.left() + name_x, rect.top() + 24.0), egui::Align2::LEFT_CENTER, &proj.name, egui::FontId::new(15.0, bold()), TEXT);
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
                                // Text links painted by hand so they can take a color on hover.
                                let link = |ui: &mut Ui, text: &str, hover: Color32| {
                                    let g = ui.painter().layout_no_wrap(text.into(), egui::FontId::proportional(12.0), TEXT_DIM);
                                    let (rr, r) = ui.allocate_exact_size(g.size() + Vec2::new(8.0, 4.0), Sense::click());
                                    if r.hovered() {
                                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                                        ui.painter().rect_filled(rr, 4.0, hover.gamma_multiply(0.18));
                                        ui.painter().rect_stroke(rect, 8.0, Stroke::new(1.0, hover), egui::StrokeKind::Inside);
                                    }
                                    ui.painter().galley_with_override_text_color(rr.center() - g.size() / 2.0, g, if r.hovered() { hover } else { TEXT_DIM });
                                    r
                                };
                                if link(ui, "Remove", RED).on_hover_text("Remove this project from AI Terminal (asks first; files are not deleted)").clicked() {
                                    remove = Some(proj.path.clone());
                                }
                                if link(ui, "Settings", ACCENT_HI).on_hover_text("Project settings: name, image, accent color").clicked() {
                                    settings = Some(proj.path.clone());
                                }
                                let cb = ui.checkbox(&mut shown, RichText::new("Show in tabs").size(12.0).color(TEXT_DIM));
                                if cb.on_hover_text("Show this project as a tab in the top bar. Unticking only closes the tab; the project stays here.").changed() {
                                    set_tab = Some((proj.path.clone(), shown));
                                }
                            }
                            // Accent swatch, bottom-left: opens the color picker.
                            let sr = egui::Rect::from_center_size(egui::pos2(rect.left() + 24.0, ctl.center().y), Vec2::splat(22.0));
                            let sresp = ui.interact(sr, egui::Id::unique(("project_accent", &proj.path)), Sense::click());
                            if sresp.hovered() {
                                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                                ui.painter().rect_filled(sr, 5.0, CARD_HI.gamma_multiply(1.3));
                            }
                            widgets::paint_swatch(ui.painter(), sr.center(), 6.5, accent, false, sresp.hovered());
                            let sresp = sresp.on_hover_text(if accent.is_some() { "Accent color (tints this project's tab)" } else { "Set an accent color for this project's tab" });
                            egui::Popup::menu(&sresp).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
                                ui.set_min_width(170.0);
                                if let Some(c) = widgets::accent_picker(ui, proj.accent) {
                                    set_accent = Some((proj.path.clone(), c));
                                }
                            });
                            if sresp.clicked() {
                                swatch_clicked = true;
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
        let toggled = set_tab.is_some() || swatch_clicked || set_accent.is_some();
        if let Some((p, c)) = set_accent {
            self.set_project_accent(&p, c);
        }
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
        } else if let Some(p) = settings {
            self.open_project_settings(&p);
        } else if let Some(p) = open.filter(|_| !toggled) {
            self.select_project(Some(p));
        }
    }

    /// Central area of a project with no session open.
    pub(super) fn empty_project(&mut self, ui: &mut Ui, theme: &crate::term_theme::TermTheme) {
        let ctx = ui.ctx().clone();
        let project = self.current_project();
        let name = self.s.projects.iter().find(|p| p.path == project).map(|p| p.name.clone()).unwrap_or_default();
        let listed = self.project_session_count(&project);
        // With a background image (global or the project's own), show it here as in a session.
        let full = ui.max_rect();
        let has_image = theme.active_bg_image().is_some_and(|p| self.cache.exists(p));
        ui.painter().rect_filled(full, 0.0, if has_image { crate::term_theme::c(theme.effective_background()) } else { BG });
        if has_image {
            crate::view::paint_theme_bg_image(ui, &mut self.cache, theme, full);
        }
        ui.vertical_centered(|ui| {
            ui.add_space(ui.available_height() * 0.33);
            if let Some(img) = self.project_image(&project) {
                let (r, _) = ui.allocate_exact_size(Vec2::splat(72.0), Sense::hover());
                self.paint_project_image(ui, &img, r, 8);
                ui.add_space(8.0);
            } else if let Some(tex) = &self.logo {
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
                ui.label(RichText::new("Add a project folder (the + after the project tabs) to avoid the prompt.").color(TEXT_DIM).size(12.5));
            }
            let hint = if listed > 0 { "or pick a session on the left to resume it" } else { "▼ next to New session on the left starts another shell" };
            ui.label(RichText::new(hint).color(TEXT_DIM).size(12.5));
        });
    }
}
