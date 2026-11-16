//! The Options window and its pages.

use super::*;

impl App {
    pub(super) fn options_shortcuts(&mut self, ui: &mut Ui) {
        ui.label(RichText::new("Shortcuts").family(bold()).size(18.0).color(TEXT));
        ui.label(RichText::new("Click a shortcut and press the new keys. Esc cancels, Backspace / Delete removes the shortcut.").size(12.5).color(TEXT_DIM));
        // Recording: the next key press (with its modifiers) becomes the binding.
        if let Some(cmd) = self.capturing {
            let pressed = ui.input(|i| i.events.iter().find_map(|e| match e {
                egui::Event::Key { key, pressed: true, modifiers, .. } => Some((*key, *modifiers)),
                _ => None,
            }));
            if let Some((key, mods)) = pressed {
                self.capturing = None;
                match key {
                    Key::Escape if !mods.any() => {}
                    Key::Backspace | Key::Delete if !mods.any() => {
                        self.s.keybindings.insert(cmd.id().to_string(), String::new());
                    }
                    _ => {
                        let mut m = Modifiers::NONE;
                        if mods.ctrl || mods.command {
                            m = m | Modifiers::CTRL;
                        }
                        if mods.shift {
                            m = m | Modifiers::SHIFT;
                        }
                        if mods.alt {
                            m = m | Modifiers::ALT;
                        }
                        let sc = KeyboardShortcut::new(m, key);
                        // A combination belongs to one command: take it away from the other.
                        for other in keys::ALL {
                            if other != cmd && keys::binding(&self.s.keybindings, other) == Some(sc) {
                                self.s.keybindings.insert(other.id().to_string(), String::new());
                                self.toast(format!("{} was moved from \"{}\"", keys::display(&sc), other.label()), ORANGE, &ui.ctx().clone());
                            }
                        }
                        self.s.keybindings.insert(cmd.id().to_string(), keys::to_text(&sc));
                    }
                }
            }
        }
        Self::opt_heading(ui, "COMMANDS");
        egui::Grid::new("keybinds").num_columns(3).spacing([24.0, 6.0]).striped(true).show(ui, |ui| {
            for cmd in keys::ALL {
                ui.label(RichText::new(cmd.label()).color(TEXT));
                let current = keys::binding(&self.s.keybindings, cmd);
                let recording = self.capturing == Some(cmd);
                let text = if recording { "Press keys…".to_string() } else { current.map(|c| keys::display(&c)).unwrap_or_else(|| "—".into()) };
                let btn = egui::Button::new(RichText::new(text).monospace().color(if recording { ACCENT_HI } else { TEXT })).min_size(Vec2::new(180.0, 24.0));
                if ui.add(btn).on_hover_text("Click, then press the new shortcut").clicked() {
                    self.capturing = Some(cmd);
                }
                let changed = self.s.keybindings.contains_key(cmd.id());
                if ui.add_enabled(changed, egui::Button::new("Reset")).on_hover_text(cmd.default_binding().map(|d| format!("Default: {}", keys::display(&d))).unwrap_or_else(|| "Default: none".into())).clicked() {
                    self.s.keybindings.remove(cmd.id());
                }
                ui.end_row();
            }
        });
        ui.add_space(6.0);
        if ui.add_enabled(!self.s.keybindings.is_empty(), egui::Button::new("Reset all shortcuts")).clicked() {
            self.s.keybindings.clear();
        }
        Self::opt_heading(ui, "FIXED");
        egui::Grid::new("keys_fixed").num_columns(2).spacing([28.0, 6.0]).striped(true).show(ui, |ui| {
            for (k, d) in SHORTCUTS {
                ui.label(RichText::new(*k).family(bold()).color(TEXT));
                ui.label(RichText::new(*d).color(TEXT_DIM));
                ui.end_row();
            }
        });
    }

    /// The gear menu: view options, settings, paste folder, help.
    /// Options window (gear button): pages on the left,
    /// settings on the right.
    pub(super) fn options_window(&mut self, ctx: &egui::Context) {
        let Some(page) = self.options else { return };
        let screen = ctx.content_rect();
        let size = Vec2::new((screen.width() * 0.8).min(1000.0), (screen.height() * 0.85).min(860.0));
        let mut page_now = page;
        let mut close = false;
        let m = egui::Modal::new(egui::Id::unique("options")).frame(egui::Frame::new().fill(PANEL).corner_radius(10).stroke(Stroke::new(1.0, BORDER)).inner_margin(0)).show(ctx, |ui| {
            ui.set_width(size.x);
            ui.set_height(size.y);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing = Vec2::ZERO;
                // Page list.
                ui.allocate_ui_with_layout(Vec2::new(190.0, size.y), Layout::top_down(Align::Min), |ui| {
                    ui.add_space(16.0);
                    ui.horizontal(|ui| {
                        ui.add_space(18.0);
                        ui.label(RichText::new("OPTIONS").size(11.5).family(bold()).color(TEXT_DIM));
                    });
                    ui.add_space(14.0);
                    for (p, label) in [(OptPage::General, "General"), (OptPage::Terminal, "Terminal"), (OptPage::Images, "Images"), (OptPage::Snippets, "Snippets"), (OptPage::Scripts, "Scripts"), (OptPage::Agents, "Agents"), (OptPage::Shortcuts, "Shortcuts"), (OptPage::About, "About")] {
                        let sel = page_now == p;
                        let (r, resp) = ui.allocate_exact_size(Vec2::new(190.0, 34.0), Sense::click());
                        let painter = ui.painter();
                        if sel {
                            painter.rect_filled(r, 0.0, CARD);
                            painter.rect_filled(egui::Rect::from_min_size(r.min, Vec2::new(3.0, r.height())), 0.0, ACCENT_HI);
                        } else if resp.hovered() {
                            painter.rect_filled(r, 0.0, CARD);
                        }
                        painter.text(egui::pos2(r.left() + 22.0, r.center().y), egui::Align2::LEFT_CENTER, label, egui::FontId::new(14.0, if sel { bold() } else { egui::FontFamily::Proportional }), if sel { TEXT } else { TEXT_DIM });
                        if resp.clicked() {
                            page_now = p;
                        }
                    }
                });
                let (sep, _) = ui.allocate_exact_size(Vec2::new(1.0, size.y), Sense::hover());
                ui.painter().rect_filled(sep, 0.0, BORDER);
                // Page content.
                ui.allocate_ui_with_layout(Vec2::new(size.x - 191.0, size.y), Layout::top_down(Align::Min), |ui| {
                    ui.horizontal(|ui| {
                        ui.add_space(ui.available_width() - 40.0);
                        if widgets::icon_button(ui, Icon::Close, 28.0, TEXT_DIM, "Close (Esc)").clicked() {
                            close = true;
                        }
                    });
                    egui::ScrollArea::vertical().auto_shrink([false, false]).show_hinted(ui, |ui| {
                        egui::Frame::new().inner_margin(egui::Margin { left: 28, right: 28, top: 10, bottom: 24 }).show(ui, |ui| {
                            ui.spacing_mut().item_spacing = Vec2::new(8.0, 6.0);
                            match page_now {
                                OptPage::Terminal => self.options_terminal(ui),
                                OptPage::Images => self.options_images(ui),
                                OptPage::General => self.options_general(ui),
                                OptPage::Shortcuts => self.options_shortcuts(ui),
                                OptPage::Snippets => self.options_snippets(ui),
                                OptPage::Scripts => self.options_scripts(ui),
                                OptPage::Agents => self.options_agents(ui),
                                OptPage::About => self.options_about(ui),
                            }
                        });
                    });
                });
            });
        });
        if m.should_close() {
            close = true;
        }
        self.options = if close { None } else { Some(page_now) };
        if self.options != Some(OptPage::Agents) {
            self.agent_confirm = None;
        }
        if self.options != Some(OptPage::Scripts) {
            // Reload from disk next time the page is shown.
            self.scripts_synced = false;
        }
        if close {
            self.focus_terminal = true;
        }
    }

    /// Fixed-width, left-aligned label in front of an option.
    pub(super) fn opt_label(ui: &mut Ui, text: &str) {
        ui.allocate_ui_with_layout(Vec2::new(150.0, 26.0), Layout::left_to_right(Align::Center), |ui| {
            ui.set_min_width(150.0);
            ui.label(RichText::new(text).color(TEXT));
        });
    }

    pub(super) fn opt_heading(ui: &mut Ui, text: &str) {
        ui.add_space(14.0);
        ui.horizontal(|ui| {
            let r = ui.label(RichText::new(text).size(11.5).family(bold()).color(TEXT_DIM)).rect;
            // A hairline from just after the heading to the right edge separates the sections.
            let x0 = r.right() + 8.0;
            let x1 = ui.max_rect().right();
            if x1 > x0 {
                ui.painter().hline(x0..=x1, r.center().y.round() + 0.5, Stroke::new(1.0, BORDER));
            }
        });
        ui.add_space(2.0);
    }

    /// One colour setting: label, swatch with a picker, and an editable hex value.
    pub(super) fn color_row(ui: &mut Ui, hex_edit: &mut std::collections::HashMap<String, String>, label: &str, v: &mut term_theme::Rgb) {
        ui.horizontal(|ui| {
            ui.add_space(16.0);
            Self::opt_label(ui, label);
            let mut c = term_theme::c(*v);
            if egui::color_picker::color_edit_button_srgba(ui, &mut c, egui::color_picker::Alpha::Opaque).changed() {
                *v = [c.r(), c.g(), c.b()];
                hex_edit.remove(label);
            }
            let id = ui.make_persistent_id(("hex", label));
            let focused = ui.memory(|m| m.has_focus(id));
            let mut text = if focused { hex_edit.get(label).cloned().unwrap_or_else(|| term_theme::to_hex(*v)) } else { term_theme::to_hex(*v) };
            let r = ui.add(egui::TextEdit::singleline(&mut text).id(id).desired_width(80.0).font(egui::TextStyle::Monospace));
            if r.changed() {
                if let Some(p) = term_theme::parse_hex(&text) {
                    *v = p;
                }
                hex_edit.insert(label.to_string(), text);
            }
            if r.lost_focus() {
                hex_edit.remove(label);
            }
        });
    }

    pub(super) fn options_terminal(&mut self, ui: &mut Ui) {
        ui.label(RichText::new("Terminal").family(bold()).size(18.0).color(TEXT));
        Self::opt_heading(ui, "FONT");
        ui.horizontal(|ui| {
            ui.add_space(16.0);
            Self::opt_label(ui, "Family");
            // Editable list; ▾ offers the default list and installed mono fonts.
            ui.add(egui::TextEdit::singleline(&mut self.s.code_font).desired_width(520.0).font(egui::FontId::proportional(13.0)));
            let pick = ui.button("▾").on_hover_text("Choose a font");
            egui::Popup::menu(&pick).show(|ui| {
                ui.set_max_height(420.0);
                if ui.button("Default font list").clicked() {
                    self.s.code_font = crate::fonts::DEFAULT_LIST.into();
                    ui.close();
                }
                ui.separator();
                egui::ScrollArea::vertical().max_height(360.0).show_hinted(ui, |ui| {
                    for f in crate::fonts::mono_families() {
                        if ui.button(&f).on_hover_text("Use this font first; the rest of the list stays as fallback").clicked() {
                            let rest: Vec<String> = crate::fonts::parse_list(&self.s.code_font).into_iter().filter(|x| !x.eq_ignore_ascii_case(&f)).collect();
                            self.s.code_font = std::iter::once(format!("'{f}'")).chain(rest.into_iter().map(|x| if x.contains(' ') { format!("'{x}'") } else { x })).collect::<Vec<_>>().join(", ");
                            ui.close();
                        }
                    }
                });
            });
        });
        // Which entries are installed (like the browser, the first installed one is used).
        ui.horizontal_wrapped(|ui| {
            ui.add_space(16.0 + 150.0 + 8.0);
            let res = crate::fonts::resolve(&self.s.code_font);
            let mut first = true;
            for (name, ok) in res {
                let (text, col) = if ok && first {
                    first = false;
                    (format!("✔ {name} (used)"), GREEN)
                } else if ok {
                    (format!("✔ {name}"), TEXT_DIM)
                } else {
                    (format!("✖ {name}"), TEXT_FAINT)
                };
                ui.label(RichText::new(text).size(12.0).color(col)).on_hover_text(if ok { "Installed" } else { "Not installed (skipped)" });
            }
        });
        ui.horizontal(|ui| {
            ui.add_space(16.0);
            Self::opt_label(ui, "Size");
            egui::ComboBox::from_id_salt("font_size").selected_text(format!("{}", self.s.font_size as i32)).width(80.0).show_ui(ui, |ui| {
                for sz in 8..=32 {
                    ui.selectable_value(&mut self.s.font_size, sz as f32, sz.to_string());
                }
            });
            ui.label(RichText::new("Ctrl+= / Ctrl+- / Ctrl+wheel").size(12.0).color(TEXT_FAINT));
        });

        Self::opt_heading(ui, "SCHEME");
        let presets = term_theme::presets();
        let current = presets.iter().find(|(_, t)| t.same_colors(&self.s.term_theme)).map(|(n, _)| *n).unwrap_or("Custom");
        ui.horizontal(|ui| {
            ui.add_space(16.0);
            Self::opt_label(ui, "Preset");
            egui::ComboBox::from_id_salt("scheme").selected_text(current).width(280.0).show_ui(ui, |ui| {
                for (name, t) in &presets {
                    if ui.selectable_label(current == *name, *name).clicked() {
                        let (img, op) = (self.s.term_theme.bg_image.clone(), self.s.term_theme.bg_opacity);
                        self.s.term_theme = t.clone();
                        self.s.term_theme.bg_image = img;
                        self.s.term_theme.bg_opacity = op;
                    }
                }
            });
            if ui.button("Reset to default").clicked() {
                let (img, op) = (self.s.term_theme.bg_image.clone(), self.s.term_theme.bg_opacity);
                self.s.term_theme = term_theme::TermTheme::default();
                self.s.term_theme.bg_image = img;
                self.s.term_theme.bg_opacity = op;
            }
        });
        // Live sample of the scheme.
        ui.horizontal(|ui| {
            ui.add_space(16.0);
            let t = &self.s.term_theme;
            let (r, _) = ui.allocate_exact_size(Vec2::new(ui.available_width().min(640.0), 74.0), Sense::hover());
            let p = ui.painter();
            p.rect_filled(r, 6.0, term_theme::c(t.background));
            p.rect_stroke(r, 6.0, Stroke::new(1.0, BORDER), egui::StrokeKind::Inside);
            let font = code_font(14.0);
            let mut x = r.left() + 12.0;
            let y0 = r.top() + 10.0;
            for (i, c) in t.ansi.iter().enumerate() {
                let row = i / 8;
                if i == 8 {
                    x = r.left() + 12.0;
                }
                p.text(egui::pos2(x, y0 + row as f32 * 20.0), egui::Align2::LEFT_TOP, "■ abc", font.clone(), term_theme::c(*c));
                x += 72.0;
            }
            let sel = egui::Rect::from_min_size(egui::pos2(r.left() + 12.0 + 9.0 * 8.4, y0 + 40.0), Vec2::new(8.4 * 8.0, 19.0));
            p.text(egui::pos2(r.left() + 12.0, y0 + 40.0), egui::Align2::LEFT_TOP, "C:\\Work>", font.clone(), term_theme::c(t.foreground));
            p.rect_filled(sel, 0.0, term_theme::c(t.selection));
            p.text(sel.min, egui::Align2::LEFT_TOP, "selected", font.clone(), term_theme::c(t.foreground));
            p.rect_filled(egui::Rect::from_min_size(egui::pos2(sel.right() + 8.0, y0 + 40.0), Vec2::new(9.0, 19.0)), 1.0, term_theme::c(t.cursor));
        });

        Self::opt_heading(ui, "COLOUR TAGS");
        ui.horizontal(|ui| {
            ui.add_space(16.0);
            ui.vertical(|ui| {
                ui.checkbox(&mut self.s.color_tags, "Show colour tags in replies as coloured text: {green}OK{/}, {red}Failed{/}, {#ff8800}…{/}");
                ui.add_enabled(self.s.color_tags, egui::Checkbox::new(&mut self.s.color_tag_prompt, "Tell Claude about them in new / resumed sessions (--append-system-prompt)"));
                ui.label(RichText::new("Claude Code's markdown has no text colour, so the agent writes tags and AI Terminal colours them. Other terminals would show the tags as text.").size(12.0).color(TEXT_FAINT));
            });
        });

        let mut theme = self.s.term_theme.clone();
        Self::opt_heading(ui, "CORE");
        Self::color_row(ui, &mut self.hex_edit, "Background", &mut theme.background);
        Self::color_row(ui, &mut self.hex_edit, "Foreground", &mut theme.foreground);
        Self::color_row(ui, &mut self.hex_edit, "Cursor", &mut theme.cursor);
        Self::color_row(ui, &mut self.hex_edit, "Selection", &mut theme.selection);
        Self::opt_heading(ui, "STANDARD COLORS");
        for i in 0..8 {
            Self::color_row(ui, &mut self.hex_edit, term_theme::COLOR_NAMES[i], &mut theme.ansi[i]);
        }
        Self::opt_heading(ui, "BRIGHT COLORS");
        for i in 8..16 {
            Self::color_row(ui, &mut self.hex_edit, term_theme::COLOR_NAMES[i], &mut theme.ansi[i]);
        }

        Self::opt_heading(ui, "BACKGROUND");
        ui.horizontal(|ui| {
            ui.add_space(16.0);
            Self::opt_label(ui, "Image");
            let shown = theme.bg_image.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "None".into());
            ui.label(RichText::new(shown).color(TEXT_DIM));
            if ui.button("Browse…").clicked() {
                if let Some(f) = rfd::FileDialog::new().set_title("Terminal background image").add_filter("Images", images::IMAGE_EXTS).pick_file() {
                    theme.bg_image = Some(f);
                }
            }
            if theme.bg_image.is_some() && ui.button("Clear").clicked() {
                theme.bg_image = None;
            }
        });
        ui.horizontal(|ui| {
            ui.add_space(16.0);
            Self::opt_label(ui, "Opacity");
            ui.add_enabled(theme.bg_image.is_some(), egui::Slider::new(&mut theme.bg_opacity, 0.05..=1.0).custom_formatter(|v, _| format!("{:.0}%", v * 100.0)));
        });
        self.s.term_theme = theme;
    }

    pub(super) fn options_images(&mut self, ui: &mut Ui) {
        ui.label(RichText::new("Images").family(bold()).size(18.0).color(TEXT));
        Self::opt_heading(ui, "PREVIEWS");
        ui.checkbox(&mut self.s.inline_previews, "Inline previews next to image paths and [Image #N]   (Ctrl+Shift+I)");
        ui.checkbox(&mut self.s.show_image_strip, "Pasted images strip in the session header");
        ui.horizontal(|ui| {
            ui.label("Max preview size");
            ui.add(egui::Slider::new(&mut self.s.preview_max_px, 80.0..=900.0).step_by(10.0).suffix(" px")).on_hover_text("Longest side of an inline preview");
        });
        ui.horizontal(|ui| {
            ui.label("Placement");
            ui.radio_value(&mut self.s.preview_over_text, false, "Never over text").on_hover_text("Previews only go into the empty space right of the text (shrinking, or a few lines lower); with no room there is no inline preview - hover and the header strip still show it.");
            ui.radio_value(&mut self.s.preview_over_text, true, "Allow over text").on_hover_text("Previews stay next to their line and may cover the end of long lines.");
        });
        Self::opt_heading(ui, "PASTE FOLDER");
        ui.label(RichText::new(images::paste_dir().display().to_string()).color(TEXT_DIM));
        ui.label(RichText::new("Clipboard images are saved here as paste-<hash>.png. The folder is trimmed at start-up and after each pasted image: files older than the limit go, then the oldest files until it fits the size limit.").size(12.5).color(TEXT_FAINT));
        ui.horizontal(|ui| {
            ui.label("Keep images for");
            ui.add(egui::DragValue::new(&mut self.s.paste_keep_days).range(0..=3650).suffix(" days")).on_hover_text("0 = keep forever");
            ui.label(RichText::new("0 = unlimited").size(12.5).color(TEXT_FAINT));
        });
        ui.horizontal(|ui| {
            ui.label("Folder size limit");
            ui.add(egui::DragValue::new(&mut self.s.paste_max_mb).range(0..=1_048_576).speed(16).suffix(" MB")).on_hover_text("0 = no limit; 1024 MB = 1 GB");
            ui.label(RichText::new("0 = unlimited").size(12.5).color(TEXT_FAINT));
        });
        ui.horizontal(|ui| {
            if ui.button("Open paste folder").clicked() {
                let d = images::paste_dir();
                let _ = std::fs::create_dir_all(&d);
                let _ = std::process::Command::new("explorer.exe").arg(&d).spawn();
            }
            if ui.button("Clean up now").on_hover_text("Apply the limits above right away").clicked() {
                images::cleanup_async(self.s.paste_keep_days, self.s.paste_limit_bytes());
            }
        });
    }

    /// One option with a description underneath.
    pub(super) fn opt_check(ui: &mut Ui, value: &mut bool, label: &str, help: &str) -> bool {
        let r = ui.checkbox(value, RichText::new(label).size(14.5).color(TEXT));
        if !help.is_empty() {
            ui.horizontal_wrapped(|ui| {
                ui.add_space(28.0);
                ui.label(RichText::new(help).size(12.5).color(TEXT_DIM));
            });
        }
        ui.add_space(4.0);
        r.changed()
    }

    pub(super) fn options_general(&mut self, ui: &mut Ui) {
        ui.label(RichText::new("General").family(bold()).size(18.0).color(TEXT));

        Self::opt_heading(ui, "WINDOW");
        let mut startup = self.s.launch_on_startup;
        if Self::opt_check(ui, &mut startup, "Launch on Windows startup", "Automatically start AI Terminal when you log in (minimized to tray).") {
            match winsys::set_autostart(startup) {
                Ok(()) => {
                    self.s.launch_on_startup = startup;
                    if startup && !self.s.minimize_to_tray {
                        self.s.minimize_to_tray = true;
                    }
                }
                Err(e) => self.toast(e, RED, &ui.ctx().clone()),
            }
        }
        Self::opt_check(ui, &mut self.s.minimize_to_tray, "Minimize to system tray", "When minimizing or closing the window, hide it to the system tray and keep sessions running. Quit from the tray icon's menu.");

        Self::opt_heading(ui, "UPDATES");
        Self::opt_check(ui, &mut self.s.auto_update, "Update automatically", "Check GitHub for a new release at start-up and every few hours, download it in the background, and install it when AI Terminal restarts.");
        ui.horizontal(|ui| {
            ui.add_space(28.0);
            if ui.button("Check for updates…").clicked() {
                self.open_update_dialog(ui.ctx());
            }
            ui.label(RichText::new(format!("Version {}", crate::update::VERSION)).size(12.5).color(TEXT_DIM));
        });
        Self::opt_heading(ui, "NOTIFICATIONS");
        Self::opt_check(ui, &mut self.s.notifications, "Desktop notifications", "Show a notification when a session needs attention (Claude finished, or asks for permission) and you're not looking at it.");
        ui.horizontal(|ui| {
            ui.add_space(28.0);
            ui.add_enabled(self.s.notifications, egui::Checkbox::new(&mut self.s.notify_sound, RichText::new("Play sound on notification").size(14.5)));
        });
        ui.add_space(8.0);
        Self::opt_check(ui, &mut self.s.flash_taskbar, "Flash the taskbar when a session needs attention", "Flash the taskbar button a few times when a session needs attention and the window isn't focused.");
        Self::opt_check(ui, &mut self.s.badge_count, "Show attention count on the taskbar badge", "Show the number of sessions needing attention on the taskbar badge instead of a single mark.");
        for (label, value, default) in [("Badge color", 0usize, [0xe5u8, 0x14, 0x14]), ("Badge text color", 1, [0xff, 0xff, 0xff])] {
            ui.horizontal(|ui| {
                ui.add_space(28.0);
                ui.allocate_ui_with_layout(Vec2::new(130.0, 26.0), Layout::left_to_right(Align::Center), |ui| {
                    ui.set_min_width(130.0);
                    ui.label(RichText::new(label).color(TEXT));
                });
                let v = if value == 0 { &mut self.s.badge_color } else { &mut self.s.badge_text_color };
                let mut c = term_theme::c(*v);
                if egui::color_picker::color_edit_button_srgba(ui, &mut c, egui::color_picker::Alpha::Opaque).changed() {
                    *v = [c.r(), c.g(), c.b()];
                    self.last_badge = None;
                }
                if ui.add_enabled(*v != default, egui::Button::new("Reset")).clicked() {
                    *v = default;
                    self.last_badge = None;
                }
            });
        }

        Self::opt_heading(ui, "DIAGNOSTICS");
        Self::opt_check(ui, &mut self.s.debug_logging, "Debug logging", "Write extra low-level diagnostics to the app log (sessions started, hook events, auto-closed sessions). Verbose; turn on only when reproducing a problem.");
        ui.horizontal(|ui| {
            ui.add_space(28.0);
            if ui.button("Open log folder").clicked() {
                let d = crate::dlog::dir();
                let _ = std::fs::create_dir_all(&d);
                let _ = std::process::Command::new("explorer.exe").arg(&d).spawn();
            }
        });

        Self::opt_heading(ui, "SESSION MANAGEMENT");
        let row = |ui: &mut Ui, label: &str, help: &str, add: &mut dyn FnMut(&mut Ui)| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(label).size(14.5).color(TEXT));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| add(ui));
            });
            ui.label(RichText::new(help).size(12.5).color(TEXT_DIM));
            ui.add_space(6.0);
        };
        let idle = &mut self.s.idle_close_minutes;
        row(ui, "Auto-close idle timeout", "Close idle Claude / Codex sessions after this many minutes. They stay in the list and resume their conversation when clicked. Set to 0 to disable.", &mut |ui| {
            ui.label("min");
            ui.add(egui::DragValue::new(idle).range(0..=1440).speed(1));
        });
        let mem = &mut self.s.memory_threshold_gb;
        let used = winsys::memory_used_gb();
        row(ui, "Memory threshold", &format!("When memory in use exceeds this, close the oldest idle Claude / Codex session (it stays resumable). Set to 0 to disable. In use now: {used:.1} GB."), &mut |ui| {
            ui.label("GB");
            ui.add(egui::DragValue::new(mem).range(0.0..=1024.0).speed(0.5).max_decimals(1));
        });

        Self::opt_heading(ui, "LINKS");
        ui.label(RichText::new("Hold Ctrl over a URL or a file path (with :line) in the terminal and click to open it.").size(12.5).color(TEXT_DIM));
        ui.horizontal(|ui| {
            ui.label("Open files with");
            let cur = crate::links::EDITORS.iter().find(|e| e.0 == self.s.editor).map(|e| e.1).unwrap_or("Automatic");
            egui::ComboBox::from_id_salt("editor").selected_text(cur).width(330.0).show_ui(ui, |ui| {
                for (k, label) in crate::links::EDITORS {
                    ui.selectable_value(&mut self.s.editor, k.to_string(), *label);
                }
            });
        });
        if self.s.editor == "custom" {
            ui.horizontal(|ui| {
                ui.label("Command");
                ui.add(egui::TextEdit::singleline(&mut self.s.editor_custom).hint_text("notepad++ -n{line} \"{file}\"").desired_width(420.0).font(egui::TextStyle::Monospace));
            });
            ui.label(RichText::new("{file}, {line} and {col} are replaced; it runs through cmd.").size(12.0).color(TEXT_FAINT));
        }

        Self::opt_heading(ui, "SESSIONS");
        ui.horizontal(|ui| {
            ui.label("Default session");
            egui::ComboBox::from_id_salt("default_profile").selected_text(self.s.default_profile.clone()).width(220.0).show_ui(ui, |ui| {
                for p in &self.profiles {
                    ui.selectable_value(&mut self.s.default_profile, p.name.clone(), &p.name);
                }
            });
        });
        ui.checkbox(&mut self.s.reopen_sessions, "Keep sessions in the list after restart (resume on click)");
        ui.horizontal(|ui| {
            ui.label("Keep the last");
            ui.add(egui::DragValue::new(&mut self.s.keep_sessions).range(0..=500).speed(0.2));
            ui.label("sessions per project in the list");
        });
        ui.label(RichText::new("Older ones move to the Archived group at the bottom of the list (still resumable; right-click > Restore to list brings one back). Running and pinned sessions are never archived. 0 = keep all.").size(12.0).color(TEXT_FAINT));
        ui.checkbox(&mut self.s.active_on_top, "Move running sessions to the top of the list");
        ui.label(RichText::new("They show in an Active group below Pinned; a session drops back to its day once it's closed or exits.").size(12.0).color(TEXT_FAINT));
        ui.checkbox(&mut self.s.name_sessions, "Ask agents to name new sessions after their first task");
        ui.label(RichText::new("Sent as an instruction with each new session of an agent that supports it (currently Claude and Codex); the agent renames the session through AI Terminal's rename_session tool. Not used when you name the session at launch.").size(12.0).color(TEXT_FAINT));
        ui.checkbox(&mut self.s.shift_enter_alt, "Shift+Enter inserts a newline (sends ESC+CR, for Claude Code)");
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.s.show_cost, "Show session usage as");
            ui.add_enabled_ui(self.s.show_cost, |ui| {
                let label = |v: &str| if v == "plan" { "Plan usage (subscription)" } else { "Estimated API cost" };
                egui::ComboBox::from_id_salt("usage_as").selected_text(label(&self.s.usage_as)).width(220.0).show_ui(ui, |ui| {
                    for v in ["plan", "cost"] {
                        ui.selectable_value(&mut self.s.usage_as, v.to_string(), label(v));
                    }
                });
            });
        });
        ui.label(
            RichText::new(if self.s.usage_as == "plan" {
                "Each session's share of your Claude plan's 5-hour and weekly limits, counted after each reply (approximate when several sessions work at once). Tokens are shown too."
            } else {
                "What the session's tokens would cost at API prices. On a Pro / Max subscription you aren't charged this."
            })
            .size(12.0)
            .color(TEXT_FAINT),
        );
        Self::opt_heading(ui, "APPEARANCE");
        ui.horizontal(|ui| {
            ui.label("UI size");
            ui.add(egui::Slider::new(&mut self.s.ui_size, 11.0..=20.0).step_by(0.5));
        });
    }

    /// About, in the style of ASM Test Bench's.
    pub(super) fn options_about(&mut self, ui: &mut Ui) {
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if let Some(tex) = &self.logo {
                ui.add(egui::Image::new((tex.id(), Vec2::splat(96.0))).corner_radius(18.0));
            }
            ui.add_space(14.0);
            ui.vertical(|ui| {
                ui.add_space(10.0);
                ui.label(RichText::new("AI Terminal").family(bold()).size(30.0).color(Color32::WHITE));
                ui.label(RichText::new("Terminal sessions for Claude Code, Codex and shells").size(15.0).color(TEXT_DIM));
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("Version {}", crate::update::VERSION)).size(13.0).color(TEXT_FAINT));
                    if ui.small_button("Check for updates…").clicked() {
                        self.open_update_dialog(ui.ctx());
                    }
                });
            });
        });
        ui.add_space(18.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("Brought to you by").size(15.0).color(TEXT));
            ui.label(RichText::new("Dima Polyakov").size(15.0).family(bold()).color(Color32::from_rgb(235, 200, 90)));
        });
        ui.add_space(12.0);
        ui.label(RichText::new("Projects with Claude / Codex / shell sessions, pasted images previewed right in the terminal, colour emoji, colour tags in Claude's replies, and sessions that resume their conversation.").size(14.5).color(TEXT));
        ui.add_space(8.0);
        let days = match self.s.paste_keep_days {
            0 => "with no age limit".to_string(),
            1 => "for 1 day".to_string(),
            d => format!("for {d} days"),
        };
        let size = match self.s.paste_max_mb {
            0 => String::new(),
            mb if mb % 1024 == 0 => format!(" (up to {} GB)", mb / 1024),
            mb => format!(" (up to {mb} MB)"),
        };
        ui.label(RichText::new(format!("Pasted images are kept in {} {days}{size}. Settings, hook events and logs live in %APPDATA%\\AI Terminal and %LOCALAPPDATA%\\AI_Terminal.", images::paste_dir().display())).size(12.5).color(TEXT_FAINT));
        ui.add_space(8.0);
        ui.label(RichText::new("Built with Rust and egui; the terminal is alacritty_terminal on ConPTY.").size(12.5).color(TEXT_FAINT));
    }
}
