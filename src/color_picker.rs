//! A colour picker in the style of a DCC tool: a hue/saturation wheel with saturation and value
//! bars, new/old swatches, RGB and HSV sliders with numeric fields, and a hex field.
//!
//! HSV here is plain sRGB-space HSV (what hex codes and other tools use), not egui's linear Hsva.

use egui::{Align, Color32, Id, Layout, Pos2, Rect, Response, RichText, Sense, Shape, Stroke, Ui, Vec2};

use crate::theme::*;

pub type Rgb = [u8; 3];

/// Per-picker state kept in egui memory between frames.
#[derive(Clone, Copy)]
struct State {
    /// Hue in degrees (0..360), saturation and value in 0..1.
    h: f32,
    s: f32,
    v: f32,
    /// The colour the picker was opened with (shown as "Old", restored by Cancel).
    orig: Rgb,
}

pub fn rgb_to_hsv(c: Rgb) -> (f32, f32, f32) {
    let [r, g, b] = c.map(|x| x as f32 / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let h = if d <= 0.0 {
        0.0
    } else if max == r {
        60.0 * ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    let s = if max <= 0.0 { 0.0 } else { d / max };
    (h, s, max)
}

pub fn hsv_to_rgb(h: f32, s: f32, v: f32) -> Rgb {
    let f = hsv_to_rgbf(h, s, v);
    f.map(|x| (x * 255.0).round().clamp(0.0, 255.0) as u8)
}

fn hsv_to_rgbf(h: f32, s: f32, v: f32) -> [f32; 3] {
    let h = h.rem_euclid(360.0) / 60.0;
    let c = v * s;
    let x = c * (1.0 - (h % 2.0 - 1.0).abs());
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    [r + m, g + m, b + m]
}

fn hsv(h: f32, s: f32, v: f32) -> Color32 {
    let [r, g, b] = hsv_to_rgb(h, s, v);
    Color32::from_rgb(r, g, b)
}

fn rgb(c: Rgb) -> Color32 {
    Color32::from_rgb(c[0], c[1], c[2])
}

fn hex(c: Rgb) -> String {
    format!("{:02X}{:02X}{:02X}", c[0], c[1], c[2])
}

fn parse_hex(s: &str) -> Option<Rgb> {
    let s = s.trim().trim_start_matches('#');
    if !s.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let s: String = match s.len() {
        3 => s.chars().flat_map(|c| [c, c]).collect(),
        6 => s.to_string(),
        _ => return None,
    };
    let v = u32::from_str_radix(&s, 16).ok()?;
    Some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

/// Bring the stored HSV in line with `c` if it was changed from outside. Hue (and saturation, for
/// black) is kept when the colour doesn't determine it, so dragging to grey and back keeps the hue.
fn sync(st: &mut State, c: Rgb) {
    if hsv_to_rgb(st.h, st.s, st.v) == c {
        return;
    }
    let (h, s, v) = rgb_to_hsv(c);
    if v > 0.0 {
        if s > 0.0 {
            st.h = h;
        }
        st.s = s;
    }
    st.v = v;
}

/// A swatch button that opens the full picker in a draggable window (one at a time), opened beside
/// the swatch. Edits apply live; Cancel, Escape or the close button restore
/// the colour it was opened with. The response is marked changed on any edit.
pub fn color_button(ui: &mut Ui, c: &mut Rgb) -> Response {
    let my_id = ui.auto_id_with("color_picker_button");
    let win_id = Id::NULL.with("color_picker_window");
    let active_id = win_id.with("active");
    let state_id = win_id.with("state");
    let active = ui.data(|d| d.get_temp::<Id>(active_id));
    let was_open = active == Some(my_id);
    let size = ui.spacing().interact_size;
    let (rect, mut resp) = ui.allocate_exact_size(size, Sense::click());
    if ui.is_rect_visible(rect) {
        let vis = if was_open { &ui.visuals().widgets.open } else { ui.style().interact(&resp) };
        let rect = rect.expand(vis.expansion);
        ui.painter().rect_filled(rect.shrink(1.0), vis.corner_radius, rgb(*c));
        ui.painter().rect_stroke(rect, vis.corner_radius, if was_open { Stroke::new(1.5, ACCENT_HI) } else { vis.bg_stroke }, egui::StrokeKind::Inside);
    }
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    if resp.clicked() && !was_open {
        let (h, s, v) = rgb_to_hsv(*c);
        ui.data_mut(|d| {
            d.insert_temp(state_id, State { h, s, v, orig: *c });
            d.insert_temp(active_id, my_id);
            d.insert_temp(win_id.with("place"), true);
        });
    }
    let is_open = ui.data(|d| d.get_temp::<Id>(active_id)) == Some(my_id);
    if !is_open {
        return resp;
    }

    let before = *c;
    let (mut keep, mut cancel) = (false, false);
    let mut open = true;
    // On opening, place the window beside the swatch (right of it, or left if there's no room),
    // kept on screen; after that it stays wherever the user drags it.
    let place = ui.data_mut(|d| d.remove_temp::<bool>(win_id.with("place"))).unwrap_or(false);
    let mut window = egui::Window::new("Colour Picker");
    if place {
        let screen = ui.ctx().content_rect();
        let size = ui.memory(|m| m.area_rect(win_id)).map(|r| r.size()).unwrap_or(Vec2::new(430.0, 470.0));
        let margin = 8.0;
        let mut x = rect.right() + 12.0;
        if x + size.x > screen.right() - margin {
            x = rect.left() - 12.0 - size.x;
        }
        let x = x.clamp(screen.left() + margin, (screen.right() - margin - size.x).max(screen.left() + margin));
        let y = (rect.top() - 40.0).clamp(screen.top() + margin, (screen.bottom() - margin - size.y).max(screen.top() + margin));
        window = window.current_pos(Pos2::new(x, y));
    }
    window
        .id(win_id)
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .constrain(true)
        // Above everything, including modal dialogs such as Options, which sit in Foreground.
        .order(egui::Order::Tooltip)
        .show(ui.ctx(), |ui| {
            ui.spacing_mut().item_spacing = Vec2::new(8.0, 8.0);
            panel(ui, state_id, c, true);
            ui.add_space(4.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.add(egui::Button::new("Cancel").min_size(Vec2::new(84.0, 28.0))).clicked() {
                    cancel = true;
                }
                if ui.add(egui::Button::new("OK").min_size(Vec2::new(84.0, 28.0))).clicked() {
                    keep = true;
                }
            });
        });
    // Consume Escape so a modal underneath (Options) doesn't close as well.
    if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) || !open {
        cancel = true;
    }
    if ui.input(|i| i.key_pressed(egui::Key::Enter)) && !ui.memory(|m| m.focused().is_some()) {
        keep = true;
    }
    if cancel {
        if let Some(st) = ui.data(|d| d.get_temp::<State>(state_id)) {
            *c = st.orig;
        }
    }
    if cancel || keep {
        ui.data_mut(|d| d.remove::<Id>(active_id));
    }
    if *c != before {
        resp.mark_changed();
    }
    resp
}

/// The picker panel itself, for embedding inline. `show_old` adds the "Old" swatch under "New"
/// (clicking it restores that colour). Returns true when the colour changed.
pub fn panel(ui: &mut Ui, id: Id, c: &mut Rgb, show_old: bool) -> bool {
    let mut st = ui.data(|d| d.get_temp::<State>(id)).unwrap_or_else(|| {
        let (h, s, v) = rgb_to_hsv(*c);
        State { h, s, v, orig: *c }
    });
    sync(&mut st, *c);
    let before = *c;
    // What changed this frame: HSV edits rebuild RGB, RGB edits rebuild HSV.
    let mut hsv_edit = false;
    let mut rgb_edit: Option<Rgb> = None;

    const WHEEL: f32 = 184.0;
    const BAR_W: f32 = 22.0;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 14.0;
        hsv_edit |= wheel(ui, WHEEL, &mut st);
        ui.spacing_mut().item_spacing.x = 10.0;
        let (h, v) = (st.h, st.v);
        hsv_edit |= bar(ui, Vec2::new(BAR_W, WHEEL), &[hsv(h, 1.0, v), hsv(h, 0.0, v)], &mut st.s, true, true);
        let (h, s) = (st.h, st.s);
        ui.spacing_mut().item_spacing.x = 14.0;
        hsv_edit |= bar(ui, Vec2::new(BAR_W, WHEEL), &[hsv(h, s, 1.0), Color32::BLACK], &mut st.v, true, true);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            let w = 110.0;
            let cur = hsv_to_rgb(st.h, st.s, st.v);
            ui.label(RichText::new("New").size(11.5).color(TEXT_DIM));
            swatch(ui, Vec2::new(w, 42.0), rgb(cur));
            if show_old {
                ui.add_space(2.0);
                ui.label(RichText::new("Old").size(11.5).color(TEXT_DIM));
                if swatch(ui, Vec2::new(w, 42.0), rgb(st.orig)).on_hover_text("Click to go back to this colour").clicked() {
                    rgb_edit = Some(st.orig);
                }
            }
            ui.add_space(8.0);
            ui.label(RichText::new("Hex").size(11.5).color(TEXT_DIM));
            if let Some(p) = hex_field(ui, id, cur, w) {
                rgb_edit = Some(p);
            }
        });
    });

    ui.add_space(6.0);
    let cur = hsv_to_rgb(st.h, st.s, st.v);
    let slider_w = 150.0;
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 20.0;
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            for (i, name) in ["R", "G", "B"].iter().enumerate() {
                let mut lo = cur;
                lo[i] = 0;
                let mut hi = cur;
                hi[i] = 255;
                let mut t = cur[i] as f32 / 255.0;
                let mut n = cur[i] as i32;
                let mut changed = false;
                slider_row(ui, name, slider_w, &[rgb(lo), rgb(hi)], &mut t, &mut n, 0..=255, "", &mut changed);
                if changed {
                    let mut next = rgb_edit.unwrap_or(cur);
                    next[i] = if n != cur[i] as i32 { n.clamp(0, 255) as u8 } else { (t * 255.0).round() as u8 };
                    rgb_edit = Some(next);
                }
            }
        });
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            let hue_stops: Vec<Color32> = (0..=6).map(|k| hsv(k as f32 * 60.0, 1.0, 1.0)).collect();
            let rows: [(&str, Vec<Color32>, f32, i32, i32, &str); 3] = [
                ("H", hue_stops, st.h / 360.0, st.h.round() as i32, 360, "°"),
                ("S", vec![hsv(st.h, 0.0, st.v), hsv(st.h, 1.0, st.v)], st.s, (st.s * 100.0).round() as i32, 100, "%"),
                ("V", vec![Color32::BLACK, hsv(st.h, st.s, 1.0)], st.v, (st.v * 100.0).round() as i32, 100, "%"),
            ];
            for (k, (name, stops, t0, n0, max, suffix)) in rows.into_iter().enumerate() {
                let mut t = t0;
                let mut n = n0;
                let mut changed = false;
                slider_row(ui, name, slider_w, &stops, &mut t, &mut n, 0..=max, suffix, &mut changed);
                if changed {
                    let val = if n != n0 { n.clamp(0, max) as f32 / max as f32 } else { t };
                    match k {
                        0 => st.h = (val * 360.0).min(359.999),
                        1 => st.s = val,
                        _ => st.v = val,
                    }
                    hsv_edit = true;
                }
            }
        });
    });

    if let Some(p) = rgb_edit {
        *c = p;
        sync(&mut st, p);
    } else if hsv_edit {
        *c = hsv_to_rgb(st.h, st.s, st.v);
    }
    ui.data_mut(|d| d.insert_temp(id, st));
    *c != before
}

/// A plain colour swatch with a hairline border; clickable.
fn swatch(ui: &mut Ui, size: Vec2, col: Color32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    ui.painter().rect_filled(rect, 4.0, col);
    ui.painter().rect_stroke(rect, 4.0, Stroke::new(1.0, BORDER), egui::StrokeKind::Inside);
    resp
}

/// The hex text field. Keeps the user's partial text while focused; returns a parsed colour when
/// the text is a valid 3- or 6-digit hex code.
fn hex_field(ui: &mut Ui, id: Id, cur: Rgb, width: f32) -> Option<Rgb> {
    let edit_id = id.with("hex");
    let buf_id = id.with("hex_buf");
    let focused = ui.memory(|m| m.has_focus(edit_id));
    let mut text = if focused { ui.data(|d| d.get_temp::<String>(buf_id)).unwrap_or_else(|| hex(cur)) } else { hex(cur) };
    let r = ui.add(egui::TextEdit::singleline(&mut text).id(edit_id).desired_width(width - 8.0).font(egui::TextStyle::Monospace).char_limit(7));
    let mut out = None;
    if r.changed() {
        out = parse_hex(&text);
        ui.data_mut(|d| d.insert_temp(buf_id, text));
    }
    if r.lost_focus() {
        ui.data_mut(|d| d.remove::<String>(buf_id));
    }
    out
}

/// Label, gradient slider and numeric field on one row.
#[allow(clippy::too_many_arguments)]
fn slider_row(ui: &mut Ui, name: &str, w: f32, stops: &[Color32], t: &mut f32, n: &mut i32, range: std::ops::RangeInclusive<i32>, suffix: &str, changed: &mut bool) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.allocate_ui_with_layout(Vec2::new(12.0, 22.0), Layout::left_to_right(Align::Center), |ui| {
            ui.set_min_width(12.0);
            ui.label(RichText::new(name).color(TEXT_DIM));
        });
        *changed |= bar(ui, Vec2::new(w, 22.0), stops, t, false, false);
        let r = ui.add_sized(Vec2::new(58.0, 22.0), egui::DragValue::new(n).range(range).suffix(suffix).speed(0.5));
        *changed |= r.changed();
    });
}

/// A gradient bar through `stops` with a draggable marker. Vertical bars run top to bottom
/// through the stops; `invert` makes the value 1 at the top.
fn bar(ui: &mut Ui, size: Vec2, stops: &[Color32], t: &mut f32, vertical: bool, invert: bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
    let mut changed = false;
    if let Some(p) = resp.interact_pointer_pos() {
        if resp.is_pointer_button_down_on() || resp.clicked() {
            let f = if vertical { (p.y - rect.top()) / rect.height() } else { (p.x - rect.left()) / rect.width() };
            let f = f.clamp(0.0, 1.0);
            let nt = if invert { 1.0 - f } else { f };
            if nt != *t {
                *t = nt;
                changed = true;
            }
        }
    }
    if resp.hovered() || resp.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let p = ui.painter();
    let mut mesh = egui::Mesh::default();
    let n = stops.len().max(2) - 1;
    for (i, &col) in stops.iter().enumerate() {
        let f = i as f32 / n as f32;
        if vertical {
            let y = rect.top() + f * rect.height();
            mesh.colored_vertex(Pos2::new(rect.left(), y), col);
            mesh.colored_vertex(Pos2::new(rect.right(), y), col);
        } else {
            let x = rect.left() + f * rect.width();
            mesh.colored_vertex(Pos2::new(x, rect.top()), col);
            mesh.colored_vertex(Pos2::new(x, rect.bottom()), col);
        }
        if i > 0 {
            let b = (i as u32 - 1) * 2;
            mesh.add_triangle(b, b + 1, b + 2);
            mesh.add_triangle(b + 1, b + 3, b + 2);
        }
    }
    p.add(Shape::mesh(mesh));
    p.rect_stroke(rect, 1.0, Stroke::new(1.0, Color32::from_black_alpha(160)), egui::StrokeKind::Outside);
    // The marker: a white bar with a dark outline, across the full thickness.
    let f = if invert { 1.0 - *t } else { *t };
    let m = if vertical {
        let y = rect.top() + f * rect.height();
        Rect::from_min_max(Pos2::new(rect.left() - 2.0, y - 2.0), Pos2::new(rect.right() + 2.0, y + 2.0))
    } else {
        let x = rect.left() + f * rect.width();
        Rect::from_min_max(Pos2::new(x - 2.0, rect.top() - 2.0), Pos2::new(x + 2.0, rect.bottom() + 2.0))
    };
    p.rect_filled(m, 1.0, Color32::WHITE);
    p.rect_stroke(m, 1.0, Stroke::new(1.0, Color32::from_black_alpha(200)), egui::StrokeKind::Outside);
    changed
}

/// The hue/saturation wheel: hue around the rim (red at the right, counter-clockwise), saturation
/// from the centre out, drawn at the current value.
fn wheel(ui: &mut Ui, d: f32, st: &mut State) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(d), Sense::click_and_drag());
    let c = rect.center();
    let r = d * 0.5 - 2.0;
    let mut changed = false;
    if let Some(p) = resp.interact_pointer_pos() {
        if resp.is_pointer_button_down_on() || resp.clicked() {
            let v = p - c;
            let s = (v.length() / r).min(1.0);
            let h = (-v.y).atan2(v.x).to_degrees().rem_euclid(360.0);
            if s > 0.0 {
                st.h = h;
            }
            st.s = s;
            changed = true;
        }
    }
    if resp.hovered() || resp.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
    }
    let p = ui.painter();
    // A triangle fan: centre vertex is grey at the current value, rim vertices are full hues.
    // 6 * 16 segments keeps the rim's corners on the hue sector boundaries, so the linear
    // interpolation between vertices matches HSV exactly along the rim.
    let segs = 96;
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(c, hsv(0.0, 0.0, st.v));
    for i in 0..=segs {
        let a = i as f32 / segs as f32 * std::f32::consts::TAU;
        let h = a.to_degrees();
        mesh.colored_vertex(c + Vec2::new(a.cos(), -a.sin()) * r, hsv(h, 1.0, st.v));
        if i > 0 {
            mesh.add_triangle(0, i as u32, i as u32 + 1);
        }
    }
    p.add(Shape::mesh(mesh));
    // Soften the polygon edge with a thin ring in the panel colour, then a hairline.
    p.circle_stroke(c, r + 1.0, Stroke::new(2.0, ui.visuals().window_fill));
    p.circle_stroke(c, r + 1.5, Stroke::new(1.0, Color32::from_black_alpha(120)));
    // The marker.
    let a = st.h.to_radians();
    let m = c + Vec2::new(a.cos(), -a.sin()) * (st.s * r);
    let ring = if st.v > 0.6 { Color32::BLACK } else { Color32::WHITE };
    p.circle_stroke(m, 5.0, Stroke::new(1.5, ring));
    p.circle_stroke(m, 6.5, Stroke::new(1.0, if ring == Color32::BLACK { Color32::from_white_alpha(140) } else { Color32::from_black_alpha(140) }));
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        for c in [[0, 0, 0], [255, 255, 255], [255, 0, 0], [35, 209, 139], [12, 200, 77], [128, 128, 128], [250, 1, 130]] {
            let (h, s, v) = rgb_to_hsv(c);
            assert_eq!(hsv_to_rgb(h, s, v), c);
        }
    }

    #[test]
    fn hex_parse() {
        assert_eq!(parse_hex("#23d18b"), Some([0x23, 0xd1, 0x8b]));
        assert_eq!(parse_hex("fff"), Some([255, 255, 255]));
        assert_eq!(parse_hex("12345"), None);
    }
}
