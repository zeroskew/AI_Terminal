//! A project's Settings dialog (name, image, accent color, tab, terminal background) and its
//! custom images.

use super::*;

/// The dialog's working copy. It's previewed live on the project while the dialog is open
/// (Cancel puts `original` back); Save applies it for good, copying images in.
pub(super) struct ProjectDraft {
    pub path: PathBuf,
    pub name: String,
    pub accent: Option<[u8; 3]>,
    /// The image shown now: the stored one, a newly picked file (copied in on Save), or None.
    pub image: Option<PathBuf>,
    pub in_tabs: bool,
    /// The project's terminal background (`enabled` = override Options > Terminal).
    pub bg: term_theme::ProjectBg,
    /// Focus the name field on the first frame only.
    focused: bool,
    /// The project as it was when the dialog opened.
    original: Project,
}

/// Where project images are copied, so they survive the original being moved or cleaned up
/// (a pasted screenshot, say).
fn image_dir() -> PathBuf {
    std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join("AI Terminal").join("project_images")
}

/// A copy we made (and so may delete), as opposed to a file elsewhere.
fn is_stored(p: &Path) -> bool {
    p.parent() == Some(image_dir().as_path())
}

/// Copies `src` into the image folder under a fresh name and returns the copy.
fn store_image(src: &Path, project: &Path) -> Result<PathBuf, String> {
    let dir = image_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let stem: String = project_name(project).chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    let ext = src.extension().and_then(|e| e.to_str()).unwrap_or("png").to_ascii_lowercase();
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let dst = dir.join(format!("{stem}-{stamp}.{ext}"));
    std::fs::copy(src, &dst).map_err(|e| e.to_string())?;
    Ok(dst)
}

/// Deletes a project image if it's one of our copies.
pub(super) fn discard_image(p: &Path) {
    if is_stored(p) {
        let _ = std::fs::remove_file(p);
    }
}

impl App {
    pub(super) fn open_project_settings(&mut self, path: &Path) {
        if let Some(p) = self.s.projects.iter().find(|x| x.path == path) {
            let bg = p.background.clone().unwrap_or_else(|| term_theme::ProjectBg::from_theme(&self.s.term_theme));
            self.project_settings = Some(ProjectDraft { path: p.path.clone(), name: p.name.clone(), accent: p.accent, image: p.image.clone(), in_tabs: p.in_tabs, bg, focused: false, original: p.clone() });
        }
    }

    /// Paints a project image fitted (aspect kept) into `rect`, with rounded corners. False when
    /// there's nothing to draw yet (still loading, or the file is missing or unreadable).
    pub(super) fn paint_project_image(&mut self, ui: &Ui, path: &Path, rect: egui::Rect, rounding: u8) -> bool {
        let ctx = ui.ctx().clone();
        let size = match self.cache.get(&ctx, path) {
            Slot::Ready { size, .. } => Vec2::new(size[0] as f32, size[1] as f32),
            _ => return false,
        };
        let fit = size * (rect.width() / size.x).min(rect.height() / size.y);
        let r = egui::Rect::from_center_size(rect.center(), fit);
        let Some(tex) = self.cache.texture_for(&ctx, path, fit * ctx.pixels_per_point()) else { return false };
        egui::Image::new((tex, fit)).corner_radius(rounding).paint_at(ui, r);
        true
    }

    /// The project's image if it has one that can be drawn (missing files fall back to the icon).
    pub(super) fn project_image(&mut self, project: &Path) -> Option<PathBuf> {
        let img = self.s.projects.iter().find(|x| x.path == project)?.image.clone()?;
        self.cache.exists(&img).then_some(img)
    }

    /// A row laid out like the Options pages: indent, a fixed-width label, then the controls.
    fn settings_row(ui: &mut Ui, label: &str, add: impl FnOnce(&mut Ui)) {
        ui.horizontal(|ui| {
            ui.add_space(16.0);
            Self::opt_label(ui, label);
            add(ui);
        });
    }

    pub(super) fn project_settings_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut d) = self.project_settings.take() else { return };
        let mut done: Option<bool> = None;
        // File dialogs are modal and blocking, so they run after the egui closure.
        let mut pick_image = false;
        let mut pick_bg = false;
        let m = egui::Modal::new(egui::Id::unique("project_settings")).frame(egui::Frame::new().fill(PANEL).corner_radius(10).stroke(Stroke::new(1.0, BORDER)).inner_margin(20)).show(ctx, |ui| {
            ui.set_width(620.0);
            ui.label(RichText::new("Project settings").family(bold()).size(17.0).color(TEXT));
            ui.horizontal(|ui| {
                ui.label(RichText::new(d.path.display().to_string()).size(12.0).color(TEXT_FAINT));
                if ui.add(egui::Link::new(RichText::new("Open folder").size(12.0).color(ACCENT_HI))).on_hover_text("Open in Explorer").clicked() {
                    let _ = std::process::Command::new("explorer.exe").arg(&d.path).spawn();
                }
            });

            Self::opt_heading(ui, "PROJECT");
            Self::settings_row(ui, "Name", |ui| {
                let te = ui.add(egui::TextEdit::singleline(&mut d.name).desired_width(f32::INFINITY).hint_text(project_name(&d.path)));
                if !d.focused {
                    te.request_focus();
                    d.focused = true;
                }
                if te.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                    done = Some(true);
                }
            });
            Self::settings_row(ui, "", |ui| {
                ui.label(RichText::new("Only the name in AI Terminal; the folder is not renamed.").size(11.5).color(TEXT_FAINT));
            });
            ui.add_space(4.0);
            Self::settings_row(ui, "Image", |ui| {
                let (r, _) = ui.allocate_exact_size(Vec2::splat(64.0), Sense::hover());
                ui.painter().rect_filled(r, 8.0, CARD);
                let drawn = match d.image.clone() {
                    Some(img) => self.paint_project_image(ui, &img, r.shrink(4.0), 6),
                    None => false,
                };
                if !drawn {
                    let col = d.accent.map(widgets::accent_color).unwrap_or(Color32::from_rgb(222, 184, 92));
                    widgets::paint_icon(ui.painter(), Icon::Folder, r.center(), 26.0, col);
                }
                ui.painter().rect_stroke(r, 8.0, Stroke::new(1.0, BORDER), egui::StrokeKind::Inside);
                ui.add_space(8.0);
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        if ui.button("Choose image…").clicked() {
                            pick_image = true;
                        }
                        if d.image.is_some() && ui.button("Remove").clicked() {
                            d.image = None;
                        }
                    });
                    let note = match &d.image {
                        Some(p) if !self.cache.exists(p) => RichText::new(format!("Not found: {}", p.display())).color(ORANGE),
                        Some(_) => RichText::new("Shown on the Home card, the tab and the project's start page.").color(TEXT_FAINT),
                        None => RichText::new("PNG, JPG, GIF, BMP or WebP. A copy is kept, so the original can move.").color(TEXT_FAINT),
                    };
                    ui.label(note.size(11.5));
                });
            });
            ui.add_space(4.0);
            Self::settings_row(ui, "Accent color", |ui| {
                let (sr, sresp) = ui.allocate_exact_size(Vec2::splat(26.0), Sense::click());
                if sresp.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    ui.painter().rect_filled(sr, 5.0, CARD_HI);
                }
                widgets::paint_swatch(ui.painter(), sr.center(), 8.0, d.accent.map(widgets::accent_color), false, sresp.hovered());
                let sresp = sresp.on_hover_text("Tints this project's tab and Home card");
                egui::Popup::menu(&sresp).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
                    ui.set_min_width(170.0);
                    if let Some(c) = widgets::accent_picker(ui, d.accent) {
                        d.accent = c;
                    }
                });
                ui.label(RichText::new(if d.accent.is_some() { "click to change" } else { "none" }).size(11.5).color(TEXT_FAINT));
            });
            Self::settings_row(ui, "Tab", |ui| {
                ui.checkbox(&mut d.in_tabs, "Show in tabs");
            });

            Self::opt_heading(ui, "BACKGROUND");
            let bg = &mut d.bg;
            Self::settings_row(ui, "Custom background", |ui| {
                ui.checkbox(&mut bg.enabled, "Override the terminal background from Options")
                    .on_hover_text("Use these background settings for this project's sessions instead of the ones in Options > Terminal. With no image, the project gets a plain background.");
            });
            ui.add_enabled_ui(bg.enabled, |ui| {
                Self::settings_row(ui, "Image", |ui| {
                    match &bg.image {
                        Some(p) => {
                            let missing = !self.cache.exists(p);
                            let text = RichText::new(p.display().to_string()).color(if missing { ORANGE } else { TEXT_DIM });
                            ui.add(egui::Label::new(text).truncate()).on_hover_text(if missing { format!("Not found: {}", p.display()) } else { p.display().to_string() });
                        }
                        None => {
                            ui.label(RichText::new("None").color(TEXT_DIM));
                        }
                    }
                    if ui.button("Browse…").clicked() {
                        pick_bg = true;
                    }
                    if bg.image.is_some() && ui.button("Clear").clicked() {
                        bg.image = None;
                    }
                });
                let has = bg.image.is_some();
                Self::settings_row(ui, "Opacity", |ui| {
                    ui.add_enabled(has, egui::Slider::new(&mut bg.opacity, 0.01..=1.0).custom_formatter(|v, _| format!("{:.0}%", v * 100.0)));
                });
                Self::settings_row(ui, "Blur", |ui| {
                    ui.add_enabled(has, egui::Slider::new(&mut bg.blur, 0.0..=40.0).step_by(0.5).custom_formatter(|v, _| if v == 0.0 { "Off".into() } else { format!("{v:.1} px") }))
                        .on_hover_text("Gaussian blur radius (sigma) in pixels; 0 = sharp");
                });
                Self::settings_row(ui, "Layout", |ui| {
                    ui.add_enabled_ui(has, |ui| {
                        egui::ComboBox::from_id_salt("project_bg_fit").selected_text(bg.fit.label()).width(280.0).show_ui(ui, |ui| {
                            for f in term_theme::BgFit::ALL {
                                ui.selectable_value(&mut bg.fit, f, f.label());
                            }
                        });
                    });
                });
                Self::settings_row(ui, "", |ui| {
                    ui.add_enabled(has, egui::Checkbox::new(&mut bg.color_override, "Override the core Background color behind the image"));
                });
                ui.add_enabled_ui(has && bg.color_override, |ui| {
                    Self::color_row(ui, &mut self.hex_edit, "Image background", &mut bg.color);
                });
            });
            ui.add_space(16.0);

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.add(egui::Button::new(RichText::new("Save").color(Color32::WHITE)).fill(ACCENT).min_size(Vec2::new(80.0, 28.0))).clicked() {
                    done = Some(true);
                }
                if ui.add(egui::Button::new("Cancel").min_size(Vec2::new(70.0, 28.0))).clicked() {
                    done = Some(false);
                }
            });
        });
        let pick = |title: &str| rfd::FileDialog::new().set_title(title).add_filter("Images", images::IMAGE_EXTS).pick_file();
        if pick_image {
            if let Some(f) = pick("Project image") {
                d.image = Some(f);
            }
        }
        if pick_bg {
            if let Some(f) = pick("Project background image") {
                d.bg.image = Some(f);
            }
        }
        if m.should_close() && done.is_none() && !pick_image && !pick_bg {
            done = Some(false);
        }
        match done {
            None => {
                self.preview_project_settings(&d, ctx);
                self.project_settings = Some(d);
            }
            Some(false) => {
                self.restore_project(&d.original);
                self.focus_terminal = true;
            }
            Some(true) => {
                self.restore_project(&d.original);
                self.apply_project_settings(d, ctx);
                self.focus_terminal = true;
            }
        }
    }

    /// Shows the draft on the project right away (picked images are used in place; they're only
    /// copied in on Save). The tab setting waits for Save.
    fn preview_project_settings(&mut self, d: &ProjectDraft, ctx: &egui::Context) {
        let Some(p) = self.s.projects.iter_mut().find(|x| x.path == d.path) else { return };
        let name = d.name.trim();
        let name = if name.is_empty() { project_name(&d.path) } else { name.to_string() };
        let background = (d.bg.enabled || d.original.background.is_some()).then(|| d.bg.clone());
        if p.name == name && p.accent == d.accent && p.image == d.image && p.background == background {
            return;
        }
        p.name = name;
        p.accent = d.accent;
        p.image = d.image.clone();
        p.background = background;
        // The terminal background was already drawn this frame from the old values.
        ctx.request_repaint();
    }

    /// Undoes a live preview.
    pub(super) fn restore_project(&mut self, original: &Project) {
        if let Some(p) = self.s.projects.iter_mut().find(|x| x.path == original.path) {
            *p = original.clone();
        }
    }

    /// The project settings as last saved, for writing to disk while a preview is showing.
    pub(super) fn project_settings_original(&self) -> Option<&Project> {
        self.project_settings.as_ref().map(|d| &d.original)
    }

    /// Copies a newly picked image in (a stored one is kept as is) and deletes the stored copy it
    /// replaces. If the copy fails, keeps `old` and says so.
    fn store_changed_image(&mut self, new: Option<PathBuf>, old: Option<PathBuf>, project: &Path, ctx: &egui::Context) -> Option<PathBuf> {
        let image = match new {
            Some(img) if old.as_ref() != Some(&img) && !is_stored(&img) => match store_image(&img, project) {
                Ok(copy) => Some(copy),
                Err(e) => {
                    self.toast(format!("Couldn't copy the image: {e}"), RED, ctx);
                    old.clone()
                }
            },
            other => other,
        };
        if let Some(o) = old.as_ref().filter(|o| image.as_ref() != Some(*o)) {
            self.cache.forget(o);
            discard_image(o);
        }
        image
    }

    fn apply_project_settings(&mut self, mut d: ProjectDraft, ctx: &egui::Context) {
        let Some((old_image, old_bg)) = self.s.projects.iter().find(|x| x.path == d.path).map(|x| (x.image.clone(), x.background.clone())) else { return };
        let image = self.store_changed_image(d.image.take(), old_image, &d.path, ctx);
        let old_bg_image = old_bg.as_ref().and_then(|b| b.image.clone());
        // The global background image isn't ours to copy-and-own until the override is on.
        d.bg.image = if d.bg.enabled || old_bg.is_some() { self.store_changed_image(d.bg.image.take(), old_bg_image, &d.path, ctx) } else { None };
        // Kept once set (even switched off), so the choices come back when it's ticked again.
        let background = (d.bg.enabled || old_bg.is_some()).then_some(d.bg);
        if let Some(p) = self.s.projects.iter_mut().find(|x| x.path == d.path) {
            let name = d.name.trim();
            p.name = if name.is_empty() { project_name(&d.path) } else { name.to_string() };
            p.accent = d.accent;
            p.image = image;
            p.background = background;
            if d.in_tabs {
                p.in_tabs = true;
            }
        }
        if !d.in_tabs {
            self.hide_project_tab(&d.path);
        }
    }
}
