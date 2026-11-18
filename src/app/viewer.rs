//! The image viewer and toasts.

use super::*;

impl App {
    pub(super) fn viewer(&mut self, ctx: &egui::Context) {
        let Some(v) = &mut self.viewer else { return };
        // Arrow keys step through the session's pasted images.
        if let Some(i) = v.list.iter().position(|p| *p == v.path) {
            if ctx.input(|i| i.key_pressed(Key::ArrowLeft)) && i > 0 {
                v.path = v.list[i - 1].clone();
            }
            if ctx.input(|i| i.key_pressed(Key::ArrowRight)) && i + 1 < v.list.len() {
                v.path = v.list[i + 1].clone();
            }
        }
        let path = v.path.clone();
        let pos = v.list.iter().position(|p| *p == path);
        let total = v.list.len();
        let mut zoom_fit = v.zoom_fit;
        let screen = ctx.content_rect();
        let mut close = false;
        let mut copy_path = false;
        let mut copy_image = false;
        // The image is drawn at its fitted size; ask for a Lanczos copy of exactly that size.
        let slot_info = match self.cache.get(ctx, &path) {
            Slot::Ready { size, bytes, .. } => Some((*size, *bytes)),
            _ => None,
        }
        .and_then(|(size, bytes)| {
            let max = Vec2::new(screen.width() * 0.86, screen.height() * 0.8);
            let s = Vec2::new(size[0] as f32, size[1] as f32);
            let k = if zoom_fit { (max.x / s.x).min((max.y - 70.0) / s.y).min(1.0) } else { 1.0 };
            self.cache.texture_for(ctx, &path, s * k * ctx.pixels_per_point()).map(|t| (t, size, bytes))
        });
        let modal = egui::Modal::new(egui::Id::unique("viewer")).frame(egui::Frame::new().fill(PANEL).corner_radius(10).stroke(Stroke::new(1.0, BORDER)).inner_margin(12)).show(ctx, |ui| {
            let max = Vec2::new(screen.width() * 0.86, screen.height() * 0.8);
            ui.set_max_width(max.x);
            ui.horizontal(|ui| {
                widgets::paint_icon(ui.painter(), Icon::Image, ui.cursor().min + Vec2::new(8.0, 10.0), 14.0, TEXT_DIM);
                ui.add_space(20.0);
                ui.label(RichText::new(path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()).family(bold()).color(TEXT));
                if let Some((_, size, bytes)) = slot_info {
                    ui.label(RichText::new(format!("{}×{}  ·  {}", size[0], size[1], images::human_size(bytes))).color(TEXT_DIM).size(12.5));
                }
                if let (Some(i), true) = (pos, total > 1) {
                    ui.label(RichText::new(format!("{} / {total}  (← →)", i + 1)).color(TEXT_FAINT).size(12.5));
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if widgets::corner_close_button(ui, 12.0).clicked() {
                        close = true;
                    }
                });
            });
            ui.add_space(6.0);
            match slot_info {
                Some((tex, size, _)) => {
                    let s = Vec2::new(size[0] as f32, size[1] as f32);
                    let k = if zoom_fit { (max.x / s.x).min((max.y - 70.0) / s.y).min(1.0) } else { 1.0 };
                    egui::ScrollArea::both().max_width(max.x).max_height(max.y - 70.0).show_hinted(ui, |ui| {
                        let r = ui.add(egui::Image::new((tex, s * k)).sense(Sense::click()));
                        if r.clicked() {
                            zoom_fit = !zoom_fit;
                        }
                        r.on_hover_text(if zoom_fit { "Click for actual size" } else { "Click to fit" });
                    });
                }
                None => {
                    ui.label(match self.cache.get(ctx, &path) {
                        Slot::Failed(e) => format!("Can't load the image: {e}"),
                        _ => "Loading…".into(),
                    });
                }
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new(path.display().to_string()).color(TEXT_FAINT).size(12.0));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.button("Open").on_hover_text("Open in the default image viewer").clicked() {
                        let _ = std::process::Command::new("explorer.exe").arg(&path).spawn();
                    }
                    if ui.button("Show in folder").clicked() {
                        let _ = std::process::Command::new("explorer.exe").arg(format!("/select,{}", path.display())).spawn();
                    }
                    if ui.button("Copy image").clicked() {
                        copy_image = true;
                    }
                    if ui.button("Copy path").clicked() {
                        copy_path = true;
                    }
                });
            });
        });
        if modal.should_close() {
            close = true;
        }
        if let Some(v) = &mut self.viewer {
            v.zoom_fit = zoom_fit;
        }
        if copy_path {
            ctx.copy_text(path.display().to_string());
            self.toast("Path copied".into(), GREEN, ctx);
        }
        if copy_image {
            let res = image::open(&path).map_err(|e| e.to_string()).and_then(|img| {
                let rgba = img.to_rgba8();
                let data = arboard::ImageData { width: rgba.width() as usize, height: rgba.height() as usize, bytes: std::borrow::Cow::Owned(rgba.into_raw()) };
                arboard::Clipboard::new().and_then(|mut c| c.set_image(data)).map_err(|e| e.to_string())
            });
            match res {
                Ok(()) => self.toast("Image copied".into(), GREEN, ctx),
                Err(e) => self.toast(format!("Couldn't copy: {e}"), RED, ctx),
            }
        }
        if close {
            self.viewer = None;
            self.focus_terminal = true;
        }
    }

    pub(super) fn show_toast(&mut self, ctx: &egui::Context) {
        let Some((msg, col, until)) = &self.toast else { return };
        if ctx.input(|i| i.time) > *until {
            self.toast = None;
            return;
        }
        let (msg, col) = (msg.clone(), *col);
        egui::Area::new(egui::Id::unique("toast")).anchor(egui::Align2::CENTER_BOTTOM, Vec2::new(0.0, -44.0)).order(egui::Order::Tooltip).interactable(false).show(ctx, |ui| {
            egui::Frame::new().fill(CARD_HI).stroke(Stroke::new(1.0, col)).corner_radius(8).inner_margin(egui::Margin::symmetric(14, 8)).show(ui, |ui| {
                // One line when it fits the window; otherwise wrap at nearly the window's width
                // (left alone, the area's narrow default width wraps even short messages).
                let max_w = (ctx.content_rect().width() - 120.0).max(200.0);
                let font = egui::TextStyle::Body.resolve(ui.style());
                let w = ui.painter().layout_no_wrap(msg.clone(), font, TEXT).size().x;
                if w > max_w {
                    ui.set_width(max_w);
                    ui.add(egui::Label::new(RichText::new(msg).color(TEXT)).wrap());
                } else {
                    ui.add(egui::Label::new(RichText::new(msg).color(TEXT)).extend());
                }
            });
        });
        ctx.request_repaint_after(Duration::from_millis(250));
    }
}
