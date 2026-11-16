//! Small custom widgets in the ASM Test Bench style: painted icons, icon buttons, pills.

use egui::{Color32, Pos2, Rect, Response, Sense, Shape, Stroke, Ui, Vec2};

use crate::theme::*;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Plus,
    Close,
    Folder,
    Image,
    Restart,
    Home,
    Gear,
    /// Claude-style asterisk.
    Sparkle,
    ChevronDown,
    /// Codex: an interlaced knot (three overlapping rings).
    Codex,
    /// Shells: ">_".
    Prompt,
    /// Plugins: a puzzle piece.
    Plugin,
    /// Qwen: its interlaced-triangle logo, two-tone.
    Qwen,
    /// Antigravity: its arch logo.
    Antigravity,
    /// Kimi: its "K" with a blue dot.
    Kimi,
    /// OpenCode: its square-ring logo.
    OpenCode,
    /// A magnifying glass.
    Search,
}

/// Fills a logo given as convex pieces in -1..1 coordinates, scaled by `h` around `c`. A hairline
/// stroke in the fill colour hides the anti-aliasing seams between neighbouring pieces.
fn fill_pieces(p: &egui::Painter, c: Pos2, h: f32, pieces: &[&[(f32, f32)]], fill: Color32) {
    for piece in pieces {
        let pts = piece.iter().map(|&(x, y)| c + Vec2::new(x, y) * h).collect();
        p.add(Shape::convex_polygon(pts, fill, Stroke::new(0.35, fill)));
    }
}

pub fn paint_icon(p: &egui::Painter, icon: Icon, c: Pos2, s: f32, col: Color32) {
    let h = s / 2.0;
    let st = Stroke::new(1.6, col);
    match icon {
        Icon::Plus => {
            p.line_segment([Pos2::new(c.x - h * 0.7, c.y), Pos2::new(c.x + h * 0.7, c.y)], st);
            p.line_segment([Pos2::new(c.x, c.y - h * 0.7), Pos2::new(c.x, c.y + h * 0.7)], st);
        }
        Icon::Close => {
            let k = h * 0.55;
            p.line_segment([Pos2::new(c.x - k, c.y - k), Pos2::new(c.x + k, c.y + k)], st);
            p.line_segment([Pos2::new(c.x - k, c.y + k), Pos2::new(c.x + k, c.y - k)], st);
        }
        Icon::Folder => {
            let body = Rect::from_min_max(Pos2::new(c.x - h * 0.85, c.y - h * 0.45), Pos2::new(c.x + h * 0.85, c.y + h * 0.7));
            let tab = Rect::from_min_max(Pos2::new(c.x - h * 0.85, c.y - h * 0.7), Pos2::new(c.x - h * 0.1, c.y - h * 0.3));
            p.rect_filled(tab, 1.5, col);
            p.rect_filled(body, 2.0, col);
        }
        Icon::Image => {
            let r = Rect::from_center_size(c, Vec2::new(h * 1.8, h * 1.5));
            p.rect_stroke(r, 2.5, Stroke::new(1.3, col), egui::StrokeKind::Inside);
            p.circle_filled(Pos2::new(r.left() + h * 0.5, r.top() + h * 0.45), h * 0.17, col);
            p.add(Shape::convex_polygon(vec![Pos2::new(r.left() + h * 0.2, r.bottom() - h * 0.15), Pos2::new(c.x - h * 0.05, c.y - h * 0.05), Pos2::new(c.x + h * 0.4, r.bottom() - h * 0.15)], col, Stroke::NONE));
            p.add(Shape::convex_polygon(vec![Pos2::new(c.x + h * 0.1, r.bottom() - h * 0.15), Pos2::new(c.x + h * 0.45, c.y + h * 0.1), Pos2::new(r.right() - h * 0.2, r.bottom() - h * 0.15)], col, Stroke::NONE));
        }
        Icon::Home => {
            let roof = vec![Pos2::new(c.x - h * 0.85, c.y - h * 0.05), Pos2::new(c.x, c.y - h * 0.8), Pos2::new(c.x + h * 0.85, c.y - h * 0.05)];
            p.add(Shape::line(roof, st));
            let body = Rect::from_min_max(Pos2::new(c.x - h * 0.6, c.y - h * 0.2), Pos2::new(c.x + h * 0.6, c.y + h * 0.75));
            p.add(Shape::line(vec![body.left_top(), body.left_bottom(), body.right_bottom(), body.right_top()], st));
            p.rect_filled(Rect::from_min_max(Pos2::new(c.x - h * 0.18, c.y + h * 0.2), Pos2::new(c.x + h * 0.18, c.y + h * 0.75)), 1.0, col);
        }
        Icon::Search => {
            let lc = c + Vec2::new(-h * 0.18, -h * 0.18);
            p.circle_stroke(lc, h * 0.52, st);
            let d = h * 0.52 * std::f32::consts::FRAC_1_SQRT_2;
            p.line_segment([lc + Vec2::splat(d), c + Vec2::splat(h * 0.8)], Stroke::new(2.0, col));
        }
        Icon::Gear => {
            let n = 8;
            let pts: Vec<Pos2> = (0..n * 4)
                .map(|i| {
                    let t = i as f32 / (n * 4) as f32 * std::f32::consts::TAU;
                    let r = if (i / 2) % 2 == 0 { h * 0.85 } else { h * 0.62 };
                    Pos2::new(c.x + r * t.cos(), c.y + r * t.sin())
                })
                .collect();
            p.add(Shape::closed_line(pts, Stroke::new(1.4, col)));
            p.circle_stroke(c, h * 0.28, Stroke::new(1.4, col));
        }
        Icon::Sparkle => {
            for i in 0..6 {
                let t = i as f32 / 6.0 * std::f32::consts::TAU + std::f32::consts::FRAC_PI_2;
                let d = Vec2::new(t.cos(), t.sin());
                p.line_segment([c + d * h * 0.15, c + d * h * 0.9], Stroke::new(1.9, col));
            }
        }
        Icon::Codex => {
            for i in 0..3 {
                let t = i as f32 / 3.0 * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2;
                let cc = c + Vec2::new(t.cos(), t.sin()) * h * 0.32;
                p.circle_stroke(cc, h * 0.5, Stroke::new(1.3, col));
            }
        }
        Icon::Prompt => {
            let s2 = Stroke::new(1.8, col);
            p.add(Shape::line(vec![Pos2::new(c.x - h * 0.8, c.y - h * 0.45), Pos2::new(c.x - h * 0.25, c.y), Pos2::new(c.x - h * 0.8, c.y + h * 0.45)], s2));
            p.line_segment([Pos2::new(c.x - h * 0.05, c.y + h * 0.5), Pos2::new(c.x + h * 0.8, c.y + h * 0.5)], s2);
        }
        Icon::Plugin => {
            let st = Stroke::new(1.5, col);
            let r = Rect::from_center_size(c + Vec2::new(-h * 0.1, h * 0.1), Vec2::splat(h * 1.3));
            p.rect_stroke(r, 2.0, st, egui::StrokeKind::Middle);
            // Knobs on the top and right edges.
            p.circle_stroke(Pos2::new(r.center().x, r.top() - h * 0.18), h * 0.22, st);
            p.circle_stroke(Pos2::new(r.right() + h * 0.18, r.center().y), h * 0.22, st);
        }
        Icon::Qwen => {
            // Light band over the coloured sides, like the logo's white ribbon.
            fill_pieces(p, c, h, QWEN_SIDES, col);
            fill_pieces(p, c, h, QWEN_BAND, col.lerp_to_gamma(Color32::WHITE, 0.8));
        }
        Icon::Antigravity => {
            // Brand gradient regardless of `col`: a per-vertex-coloured mesh (no seams, but no
            // anti-aliasing), then a thin gradient outline to soften the edge.
            let h = h * 0.9;
            let mut mesh = egui::epaint::Mesh::default();
            for piece in ANTIGRAVITY {
                let base = mesh.vertices.len() as u32;
                for &(x, y) in *piece {
                    mesh.colored_vertex(c + Vec2::new(x, y) * h, antigravity_colour(x, y));
                }
                for i in 1..piece.len() as u32 - 1 {
                    mesh.add_triangle(base, base + i, base + i + 1);
                }
            }
            p.add(Shape::mesh(mesh));
            let outline = ANTIGRAVITY_OUTLINE.iter().map(|&(x, y)| c + Vec2::new(x, y) * h).collect();
            let stroke = egui::epaint::PathStroke::new_uv(0.6, move |_, pos| {
                let v = (pos - c) / h;
                antigravity_colour(v.x, v.y)
            });
            p.add(Shape::closed_line(outline, stroke));
        }
        Icon::Kimi => {
            fill_pieces(p, c, h * 0.9, KIMI_K, col);
            fill_pieces(p, c, h * 0.9, KIMI_DOT, Color32::from_rgb(23, 131, 255));
        }
        Icon::OpenCode => fill_pieces(p, c, h * 0.9, OPENCODE, col),
        Icon::ChevronDown => {
            p.add(Shape::convex_polygon(vec![Pos2::new(c.x - h * 0.55, c.y - h * 0.3), Pos2::new(c.x + h * 0.55, c.y - h * 0.3), Pos2::new(c.x, c.y + h * 0.35)], col, Stroke::NONE));
        }
        Icon::Restart => {
            use std::f32::consts::PI;
            let pts: Vec<Pos2> = (0..=20)
                .map(|i| {
                    let t = -PI * 0.35 + PI * 1.75 * i as f32 / 20.0;
                    Pos2::new(c.x + h * 0.7 * t.cos(), c.y + h * 0.7 * t.sin())
                })
                .collect();
            let end = pts[0];
            p.add(Shape::line(pts, st));
            p.add(Shape::convex_polygon(vec![end + Vec2::new(-h * 0.35, -h * 0.25), end + Vec2::new(h * 0.35, -h * 0.25), end + Vec2::new(0.0, h * 0.35)], col, Stroke::NONE));
        }
    }
}

/// Icon-only button with a tooltip.
pub fn icon_button(ui: &mut Ui, icon: Icon, size: f32, color: Color32, tip: &str) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    let p = ui.painter();
    if resp.hovered() {
        p.rect_filled(rect, 5.0, if resp.is_pointer_button_down_on() { Color32::from_rgb(62, 66, 76) } else { CARD_HI });
    }
    paint_icon(p, icon, rect.center(), size * 0.55, if resp.hovered() { TEXT } else { color });
    resp.on_hover_text(tip)
}

/// Filled pill button with an icon and text (the primary action).
pub fn primary_button(ui: &mut Ui, icon: Icon, text: &str, fill: Color32, tip: &str) -> Response {
    let font = egui::FontId::new(13.5, bold());
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, Color32::WHITE);
    let size = Vec2::new(galley.size().x + 40.0, 28.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let fill = if resp.is_pointer_button_down_on() {
        fill.gamma_multiply(0.8)
    } else if resp.hovered() {
        fill.gamma_multiply(1.15)
    } else {
        fill
    };
    let p = ui.painter();
    p.rect_filled(rect, 6.0, fill);
    paint_icon(p, icon, Pos2::new(rect.left() + 16.0, rect.center().y), 13.0, Color32::WHITE);
    p.galley(Pos2::new(rect.left() + 28.0, rect.center().y - galley.size().y / 2.0), galley, Color32::WHITE);
    resp.on_hover_text(tip)
}

/// Small rounded label (status pills, tags).
pub fn pill(ui: &mut Ui, text: &str, fg: Color32, bg: Color32) -> Response {
    let font = egui::FontId::new(11.5, bold());
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, fg);
    let (rect, resp) = ui.allocate_exact_size(galley.size() + Vec2::new(14.0, 4.0), Sense::hover());
    ui.painter().rect_filled(rect, rect.height() / 2.0, bg);
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, fg);
    resp
}

/// A filled split button: the main part (optional icon + text) and a ▼ part for a menu.
/// Fills the available width when `height` > 30 (the big "New session" button), otherwise
/// sizes to the text. Returns (main, arrow) responses.
pub fn split_button(ui: &mut Ui, icon: Option<Icon>, text: &str, fill: Color32, height: f32) -> (Response, Response) {
    let font = egui::FontId::new(13.5, bold());
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, Color32::WHITE);
    let arrow_w = 30.0;
    let icon_w = if icon.is_some() { 22.0 } else { 0.0 };
    let w = if height > 30.0 { ui.available_width() } else { galley.size().x + icon_w + 24.0 + arrow_w };
    let (rect, _) = ui.allocate_exact_size(Vec2::new(w, height), Sense::hover());
    let main_r = Rect::from_min_max(rect.min, Pos2::new(rect.right() - arrow_w, rect.bottom()));
    let arrow_r = Rect::from_min_max(Pos2::new(rect.right() - arrow_w, rect.top()), rect.max);
    // Ids by position, not text: a label change (e.g. a new default session) must not drop the open menu.
    let main = ui.interact(main_r, ui.auto_id_with("split_main"), Sense::click());
    let arrow = ui.interact(arrow_r, ui.auto_id_with("split_arrow"), Sense::click());
    let shade = |r: &Response| {
        if r.is_pointer_button_down_on() {
            fill.gamma_multiply(0.8)
        } else if r.hovered() {
            fill.gamma_multiply(1.15)
        } else {
            fill
        }
    };
    let p = ui.painter();
    let cr = 6u8;
    p.rect_filled(main_r, egui::CornerRadius { nw: cr, sw: cr, ne: 0, se: 0 }, shade(&main));
    p.rect_filled(arrow_r, egui::CornerRadius { nw: 0, sw: 0, ne: cr, se: cr }, shade(&arrow));
    p.line_segment([Pos2::new(arrow_r.left(), rect.top() + 6.0), Pos2::new(arrow_r.left(), rect.bottom() - 6.0)], Stroke::new(1.0, Color32::from_white_alpha(70)));
    let content_w = icon_w + galley.size().x;
    let mut x = if height > 30.0 { main_r.center().x - content_w / 2.0 } else { main_r.left() + 12.0 };
    if let Some(i) = icon {
        paint_icon(p, i, Pos2::new(x + 8.0, rect.center().y), 14.0, Color32::WHITE);
        x += icon_w;
    }
    p.galley(Pos2::new(x, rect.center().y - galley.size().y / 2.0), galley, Color32::WHITE);
    paint_icon(p, Icon::ChevronDown, arrow_r.center(), 12.0, Color32::WHITE);
    (main, arrow)
}

/// The Qwen logo (an interlaced triangle on a six-point star), as convex pieces in a unit circle.
/// Traced from the official SVG: the shaded sides plus the centre triangle, then the light band.
const QWEN_SIDES: &[&[(f32, f32)]] = &[
    &[(-0.224, -0.922), (-0.091, -0.684), (0.185, -0.724), (0.039, -0.941), (-0.224, -0.941)],
    &[(-0.794, -0.452), (-0.509, -0.452), (-0.509, -0.485), (-0.771, -0.485)],
    &[(-0.091, -0.684), (-0.224, -0.452), (0.790, -0.452), (0.854, -0.481), (0.710, -0.704), (0.185, -0.724)],
    &[(0.658, -0.229), (0.862, 0.127), (0.710, -0.197)],
    &[(0.854, -0.481), (0.790, -0.452), (0.658, -0.229), (0.710, -0.197)],
    &[(-0.224, -0.941), (-0.509, -0.485), (-0.509, -0.452), (-0.224, -0.922)],
    &[(-0.660, 0.706), (-0.826, 0.483), (-0.660, 0.726)],
    &[(-0.169, 0.726), (-0.022, 0.944), (-0.169, 0.706)],
    &[(-0.713, 0.224), (-0.509, 0.469), (-0.257, 0.469), (-0.800, -0.443), (-0.938, -0.242)],
    &[(-0.169, 0.726), (-0.169, 0.706), (-0.660, 0.706), (-0.660, 0.726)],
    &[(-0.713, 0.224), (-0.826, 0.483), (-0.660, 0.706), (-0.509, 0.469)],
    &[(0.658, 0.209), (0.526, 0.487), (0.785, 0.468), (0.917, 0.244), (0.901, 0.209)],
    &[(0.658, 0.209), (0.526, -0.024), (0.008, 0.944), (0.240, 0.944), (0.526, 0.487)],
    &[(-0.316, -0.134), (0.008, 0.414), (0.354, -0.134)],
];
const QWEN_BAND: &[&[(f32, f32)]] = &[
    &[(0.526, -0.024), (0.658, 0.209), (0.901, 0.209), (0.862, 0.127), (0.658, -0.229)],
    &[(-0.794, -0.452), (-0.800, -0.443), (-0.316, -0.134), (0.354, -0.134), (0.658, -0.229), (0.790, -0.452)],
    &[(-0.224, -0.452), (-0.091, -0.684), (-0.224, -0.922), (-0.509, -0.452)],
    &[(-0.660, 0.706), (-0.169, 0.706), (-0.257, 0.469), (-0.509, 0.469)],
    &[(0.354, -0.134), (0.008, 0.414), (-0.169, 0.706), (-0.022, 0.944), (0.008, 0.944), (0.526, -0.024)],
    &[(-0.800, -0.443), (-0.257, 0.469), (0.008, 0.414), (-0.316, -0.134)],
    &[(0.526, -0.024), (0.658, -0.229), (0.354, -0.134)],
    &[(0.008, 0.414), (-0.257, 0.469), (-0.169, 0.706)],
];

// Logos below are traced from the LobeHub icon set (24x24 viewBox), mapped to -1..1 around the centre.
/// Antigravity logo: a rounded arch.
const ANTIGRAVITY: &[&[(f32, f32)]] = &[
    &[(0.594, 0.687), (0.713, 0.815), (0.871, 0.923), (0.942, 0.930), (0.990, 0.905), (0.999, 0.863), (0.970, 0.803), (0.834, 0.640)],
    &[(-0.183, -0.855), (-0.278, -0.767), (-0.391, -0.573), (0.003, 0.083), (0.136, 0.105), (0.458, -0.414), (0.363, -0.645), (0.239, -0.815), (0.133, -0.884), (0.071, -0.902), (-0.065, -0.902)],
    &[(-0.391, -0.573), (-0.706, 0.404), (-0.342, 0.298), (-0.263, 0.196)],
    &[(-0.554, 0.642), (-0.828, 0.640), (-0.979, 0.824), (-0.999, 0.869), (-0.995, 0.902), (-0.964, 0.928), (-0.897, 0.931), (-0.836, 0.910), (-0.755, 0.854), (-0.664, 0.773)],
    &[(-0.342, 0.298), (-0.706, 0.404), (-0.828, 0.640), (-0.554, 0.642)],
    &[(0.269, 0.196), (0.594, 0.687), (0.834, 0.640), (0.712, 0.404)],
    &[(0.136, 0.105), (0.269, 0.196), (0.712, 0.404), (0.458, -0.414)],
    &[(-0.130, 0.105), (-0.391, -0.573), (-0.263, 0.196)],
    &[(0.003, 0.083), (-0.391, -0.573), (-0.130, 0.105)],
];
/// Kimi logo: a "K" ...
const KIMI_K: &[&[(f32, f32)]] = &[
    &[(0.184, -0.699), (-0.479, -0.046), (-0.078, -0.067), (0.527, -0.667), (0.526, -0.697)],
    &[(-0.495, -0.065), (-0.497, -0.690), (-0.510, -0.701), (-0.745, -0.696), (-0.744, 0.663), (-0.500, 0.662), (-0.493, 0.368)],
    &[(0.303, 0.578), (0.444, 0.640), (0.532, 0.661), (0.570, 0.659), (0.570, 0.376), (0.473, 0.347)],
    &[(-0.277, 0.157), (0.303, 0.578), (0.473, 0.347), (0.392, 0.304)],
    &[(-0.479, -0.046), (-0.277, 0.157), (-0.077, -0.036), (-0.078, -0.067)],
    &[(-0.495, -0.065), (-0.493, 0.368), (-0.479, -0.046)],
    &[(-0.077, -0.036), (-0.277, 0.157), (0.392, 0.304)],
    &[(-0.479, -0.046), (-0.493, 0.368), (-0.277, 0.157)],
];
/// ... and its dot, top right.
const KIMI_DOT: &[&[(f32, f32)]] = &[
    &[(0.901, -0.979), (0.821, -1.000), (0.740, -0.978), (0.677, -0.912), (0.660, -0.840), (0.660, -0.697), (0.671, -0.681), (0.862, -0.685), (0.918, -0.713), (0.969, -0.778), (0.979, -0.861), (0.959, -0.920)],
];
/// OpenCode logo: a square ring.
const OPENCODE: &[&[(f32, f32)]] = &[
    &[(-0.667, 0.833), (-0.333, 0.500), (-0.333, -0.500), (-0.667, -0.833)],
    &[(-0.667, -0.833), (-0.333, -0.500), (0.333, -0.500), (0.667, -0.833)],
    &[(-0.333, 0.500), (-0.667, 0.833), (0.667, 0.833), (0.333, 0.500)],
    &[(0.667, -0.833), (0.333, -0.500), (0.333, 0.500), (0.667, 0.833)],
];
/// Antigravity outline, for an anti-aliased edge around the gradient mesh.
const ANTIGRAVITY_OUTLINE: &[(f32, f32)] = &[(0.713, 0.815), (0.871, 0.923), (0.942, 0.930), (0.990, 0.905), (0.999, 0.863), (0.970, 0.803), (0.834, 0.640), (0.712, 0.404), (0.458, -0.414), (0.363, -0.645), (0.239, -0.815), (0.133, -0.884), (0.071, -0.902), (-0.065, -0.902), (-0.183, -0.855), (-0.278, -0.767), (-0.391, -0.573), (-0.706, 0.404), (-0.828, 0.640), (-0.979, 0.824), (-0.999, 0.869), (-0.995, 0.902), (-0.964, 0.928), (-0.897, 0.931), (-0.836, 0.910), (-0.755, 0.854), (-0.664, 0.773), (-0.554, 0.642), (-0.342, 0.298), (-0.263, 0.196), (-0.130, 0.105), (0.003, 0.083), (0.136, 0.105), (0.269, 0.196), (0.594, 0.687)];
/// Antigravity's colour stops (yellow top, red right, green left, blue base), blended by distance.
const ANTIGRAVITY_STOPS: &[((f32, f32), [u8; 3])] = &[
    ((-0.45, -1.26), [0xFF, 0xE4, 0x32]),
    ((0.0, -1.33), [0xFB, 0xBC, 0x04]),
    ((-0.78, -0.65), [0xFF, 0xEE, 0x48]),
    ((0.97, -0.52), [0xFC, 0x41, 0x3D]),
    ((0.88, -0.79), [0xFC, 0x41, 0x3D]),
    ((-1.1, -0.36), [0x00, 0xB9, 0x5C]),
    ((-0.1, 0.94), [0x31, 0x86, 0xFF]),
    ((0.47, 1.15), [0x31, 0x86, 0xFF]),
    ((0.92, 0.05), [0x74, 0x9B, 0xFF]),
];

/// The Antigravity gradient at a point in the logo's -1..1 coordinates (Gaussian-weighted stops).
fn antigravity_colour(x: f32, y: f32) -> Color32 {
    let (mut sum, mut rgb) = (0.0, [0.0f32; 3]);
    for &((sx, sy), c) in ANTIGRAVITY_STOPS {
        let w = (-((x - sx).powi(2) + (y - sy).powi(2)) / 0.36).exp();
        sum += w;
        for k in 0..3 {
            rgb[k] += w * c[k] as f32;
        }
    }
    let ch = |k: usize| (rgb[k] / sum.max(1e-6)).round() as u8;
    Color32::from_rgb(ch(0), ch(1), ch(2))
}
