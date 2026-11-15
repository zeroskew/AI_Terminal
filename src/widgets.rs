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
    let main = ui.interact(main_r, ui.auto_id_with(("split_main", text)), Sense::click());
    let arrow = ui.interact(arrow_r, ui.auto_id_with(("split_arrow", text)), Sense::click());
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
