//! Colors, egui style and fonts (the ASM Test Bench dark theme).

use egui::{Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Stroke};
use std::path::PathBuf;
use std::sync::Arc;

pub const BG: Color32 = Color32::from_rgb(22, 23, 26);
pub const PANEL: Color32 = Color32::from_rgb(28, 29, 33);
pub const BAR: Color32 = Color32::from_rgb(33, 34, 39);
pub const CARD: Color32 = Color32::from_rgb(36, 38, 43);
pub const CARD_HI: Color32 = Color32::from_rgb(44, 46, 53);
pub const BORDER: Color32 = Color32::from_rgb(52, 55, 62);
pub const TEXT: Color32 = Color32::from_rgb(220, 223, 228);
pub const TEXT_DIM: Color32 = Color32::from_rgb(140, 146, 158);
pub const TEXT_FAINT: Color32 = Color32::from_rgb(92, 97, 108);
pub const ACCENT: Color32 = Color32::from_rgb(0, 120, 212);
pub const ACCENT_HI: Color32 = Color32::from_rgb(56, 152, 236);
pub const LINK: Color32 = Color32::from_rgb(110, 180, 255);
pub const GREEN: Color32 = Color32::from_rgb(78, 190, 105);
pub const RED: Color32 = Color32::from_rgb(232, 85, 85);
pub const ORANGE: Color32 = Color32::from_rgb(240, 160, 60);


pub fn bold() -> FontFamily {
    FontFamily::Name("bold".into())
}

pub fn code() -> FontFamily {
    FontFamily::Name("code".into())
}

pub fn code_font(size: f32) -> FontId {
    FontId::new(size, code())
}

/// Base text size; Options > Appearance > UI Scale zooms the whole window on top of it.
const UI_TEXT: f32 = 14.0;

pub fn apply_style(ctx: &egui::Context) {
    let mut style = egui::Style::default();
    let v = &mut style.visuals;
    *v = egui::Visuals::dark();
    v.panel_fill = PANEL;
    v.window_fill = CARD;
    v.extreme_bg_color = BG;
    v.faint_bg_color = Color32::from_rgb(32, 33, 38);
    v.code_bg_color = BG;
    v.window_stroke = Stroke::new(1.0, BORDER);
    v.window_corner_radius = CornerRadius::same(8);
    v.menu_corner_radius = CornerRadius::same(6);
    v.selection.bg_fill = Color32::from_rgb(38, 79, 120);
    v.selection.stroke = Stroke::new(1.0, ACCENT_HI);
    v.hyperlink_color = LINK;
    v.override_text_color = None;
    let r = CornerRadius::same(5);
    for (w, fill, stroke) in [
        (&mut v.widgets.noninteractive, PANEL, BORDER),
        (&mut v.widgets.inactive, CARD_HI, Color32::from_rgb(58, 61, 69)),
        (&mut v.widgets.hovered, Color32::from_rgb(54, 57, 66), Color32::from_rgb(80, 84, 95)),
        (&mut v.widgets.active, Color32::from_rgb(60, 64, 74), ACCENT),
        (&mut v.widgets.open, Color32::from_rgb(48, 51, 59), BORDER),
    ] {
        w.corner_radius = r;
        w.bg_fill = fill;
        w.weak_bg_fill = fill;
        w.bg_stroke = Stroke::new(1.0, stroke);
    }
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    v.widgets.hovered.fg_stroke = Stroke::new(1.0, Color32::WHITE);
    v.widgets.active.fg_stroke = Stroke::new(1.0, Color32::WHITE);
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);

    let s = &mut style.spacing;
    s.item_spacing = egui::vec2(8.0, 6.0);
    s.button_padding = egui::vec2(8.0, 4.0);
    s.menu_margin = egui::Margin::same(6);
    s.interact_size.y = 24.0;
    s.scroll.bar_width = SCROLL_BAR_W;
    s.scroll.floating = true;

    use egui::TextStyle::*;
    style.text_styles.insert(Body, FontId::proportional(UI_TEXT));
    style.text_styles.insert(Button, FontId::proportional(UI_TEXT));
    style.text_styles.insert(Small, FontId::proportional(UI_TEXT - 2.5));
    style.text_styles.insert(Heading, FontId::new(UI_TEXT + 4.0, bold()));
    style.text_styles.insert(Monospace, FontId::new(UI_TEXT, code()));
    ctx.set_theme(egui::Theme::Dark);
    ctx.set_style_of(egui::Theme::Dark, style);
}

/// Width of every scroll bar (floating ones at full size, and solid ones).
pub const SCROLL_BAR_W: f32 = 8.0;

/// A scroll bar that takes its own space (for content the floating bar would cover), drawn the
/// same as the app's floating bars at full size so the two kinds look alike.
pub fn solid_scroll() -> egui::style::ScrollStyle {
    egui::style::ScrollStyle { bar_width: SCROLL_BAR_W, bar_inner_margin: 4.0, foreground_color: true, ..egui::style::ScrollStyle::solid() }
}

/// Font folders: the system one and the per-user one.
fn font_dirs() -> Vec<PathBuf> {
    let mut v = vec![std::env::var_os("WINDIR").map(PathBuf::from).unwrap_or_else(|| r"C:\Windows".into()).join("Fonts")];
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        v.push(PathBuf::from(local).join("Microsoft").join("Windows").join("Fonts"));
    }
    v
}

fn load(file: &str) -> Option<Vec<u8>> {
    font_dirs().into_iter().map(|d| d.join(file)).find(|p| p.exists()).and_then(|p| std::fs::read(p).ok())
}

static HAS_BOLD: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// True when the terminal font has a real bold face (else bold is drawn twice, offset).
pub fn has_code_bold() -> bool {
    HAS_BOLD.load(std::sync::atomic::Ordering::Relaxed)
}

pub fn code_bold() -> FontFamily {
    FontFamily::Name("code_bold".into())
}

fn face_data(face: &crate::fonts::Face) -> Option<FontData> {
    let bytes = std::fs::read(&face.path).ok()?;
    let mut d = FontData::from_owned(bytes);
    d.index = face.index;
    Some(d)
}

/// The bold instance of a variable font (one with a `wght` axis, like Cascadia Code, which
/// ships as a single file with no separate Bold face), sharing the regular face's bytes.
fn variable_bold(regular: &FontData) -> Option<FontData> {
    let axis = regular.variation_axes().into_iter().find(|a| &a.tag.to_be_bytes() == b"wght")?;
    let weight = 700.0f32.clamp(axis.range.min, axis.range.max);
    if weight <= axis.default {
        return None;
    }
    let tweak = egui::epaint::text::FontTweak { coords: egui::epaint::text::VariationCoords::new([(b"wght", weight)]), ..regular.tweak.clone() };
    Some(regular.clone().tweak(tweak))
}

/// Installs the UI fonts and the terminal font chain for `family_list` (a CSS-style list like
/// `'MesloLGS NF', …, 'Consolas', monospace`): every installed entry is added
/// in order, so later ones supply glyphs the first lacks; `monospace` is egui's built-in Hack.
pub fn install_fonts(ctx: &egui::Context, family_list: &str) {
    let mut defs = FontDefinitions::default();
    if let Some(regular) = load("segoeui.ttf") {
        defs.font_data.insert("ui".into(), Arc::new(FontData::from_owned(regular)));
        defs.families.entry(FontFamily::Proportional).or_default().insert(0, "ui".into());
    }
    let proportional = defs.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    let mut bold_family = proportional.clone();
    if let Some(b) = load("segoeuisb.ttf").or_else(|| load("segoeuib.ttf")) {
        defs.font_data.insert("ui_bold".into(), Arc::new(FontData::from_owned(b)));
        bold_family.insert(0, "ui_bold".into());
    }
    defs.families.insert(bold(), bold_family);

    let builtin_mono = defs.families.get(&FontFamily::Monospace).cloned().unwrap_or_default();
    let mut code_family: Vec<String> = Vec::new();
    let mut bold_chain: Vec<String> = Vec::new();
    let mut first_bold = None;
    for (i, name) in crate::fonts::parse_list(family_list).iter().enumerate() {
        if name.eq_ignore_ascii_case("monospace") {
            for k in &builtin_mono {
                if !code_family.contains(k) {
                    code_family.push(k.clone());
                }
            }
            continue;
        }
        let Some((regular, bold_face)) = crate::fonts::find(name) else { continue };
        let Some(data) = face_data(&regular) else { continue };
        let key = format!("code_{i}");
        if code_family.is_empty() {
            first_bold = bold_face.as_ref().and_then(face_data).or_else(|| variable_bold(&data));
        }
        defs.font_data.insert(key.clone(), Arc::new(data));
        code_family.push(key);
    }
    if code_family.is_empty() {
        code_family = builtin_mono.clone();
    }
    for k in &builtin_mono {
        if !code_family.contains(k) {
            code_family.push(k.clone());
        }
    }
    HAS_BOLD.store(first_bold.is_some(), std::sync::atomic::Ordering::Relaxed);
    if let Some(bd) = first_bold {
        defs.font_data.insert("code_bold_face".into(), Arc::new(bd));
        bold_chain.push("code_bold_face".into());
    }
    bold_chain.extend(code_family.iter().cloned());
    defs.families.insert(code(), code_family.clone());
    defs.families.insert(code_bold(), bold_chain);
    defs.families.insert(FontFamily::Monospace, code_family);

    // Symbol fallbacks so arrows / bullets never render as boxes.
    let mut extra = Vec::new();
    for (key, file) in [("segoe_symbol", "seguisym.ttf"), ("segoe_emoji", "seguiemj.ttf")] {
        if let Some(bytes) = load(file) {
            defs.font_data.insert(key.into(), Arc::new(FontData::from_owned(bytes)));
            extra.push(key.to_string());
        }
    }
    for family in defs.families.values_mut() {
        for k in &extra {
            if !family.contains(k) {
                family.push(k.clone());
            }
        }
    }
    ctx.set_fonts(defs);
}

#[cfg(test)]
mod tests {
    #[test]
    fn variable_font_gets_a_bold_instance() {
        // Cascadia Code installs as one variable file: bold comes from its wght axis.
        let Some((regular, None)) = crate::fonts::find("Cascadia Code") else { return };
        let data = super::face_data(&regular).unwrap();
        let bold = super::variable_bold(&data).expect("wght axis");
        assert_eq!(bold.tweak.coords.as_ref()[0].1, 700.0);
    }
}
