//! The terminal widget: paints the alacritty grid with egui, turns keyboard / mouse input into
//! PTY bytes, handles selection and scrollback, and draws previews of the images the text
//! refers to (pasted image paths and Claude Code's `[Image #N]` placeholders).

use std::path::PathBuf;

use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::color::Colors;
use alacritty_terminal::term::TermMode;
use alacritty_terminal::vte::ansi::{Color, CursorShape, NamedColor};
use egui::{Color32, Event, Key, Modifiers, PointerButton, Pos2, Rect, Sense, Stroke, Ui, Vec2};

use crate::images::{self, ImageCache, RefKind, Slot};
use crate::session::{GridSize, Session};
use crate::theme::*;

/// What the view asks the app to do.
pub enum Action {
    /// Ctrl+V / Shift+Insert / right-click: read the clipboard (files, image or text) and paste.
    Paste,
    /// Open the image viewer.
    View(PathBuf),
    Copy(String),
    /// Ctrl+click on a URL or file path.
    OpenLink(crate::links::Link),
}

pub struct ViewOptions<'a> {
    pub font_size: f32,
    pub theme: &'a crate::term_theme::TermTheme,
    pub inline_previews: bool,
    /// Longest side of an inline preview, in points.
    pub preview_max_px: f32,
    /// Inline previews may cover the end of long lines (else they only go where there is room).
    pub preview_over_text: bool,
    pub shift_enter_alt: bool,
    /// Render {red}…{/} colour tags (see colortags.rs).
    pub color_tags: bool,
}

/// Per-session UI state that isn't part of the terminal itself.
#[derive(Default)]
pub struct ViewState {
    /// Find bar (Ctrl+Shift+F).
    pub search: Search,
    selecting: bool,
    last_click: Option<(f64, Point, u8)>,
    scroll_accum: f32,
    mouse_down_reported: Option<u8>,
}

#[derive(Clone)]
struct SnapCell {
    c: char,
    fg: Color32,
    bg: Color32,
    flags: Flags,
    selected: bool,
    extra: Option<Vec<char>>,
}

struct Placed {
    rect: Rect,
    path: PathBuf,
    label: String,
}

const PAD: f32 = 6.0;

pub fn cell_metrics(ui: &Ui, font_size: f32) -> (f32, f32) {
    let font = code_font(font_size);
    let cw = ui.fonts_mut(|f| f.glyph_width(&font, 'M'));
    let ch = (ui.fonts_mut(|f| f.row_height(&font)) * 1.08).round();
    (cw.max(4.0), ch.max(8.0))
}

pub fn show(ui: &mut Ui, s: &mut Session, st: &mut ViewState, cache: &mut ImageCache, opt: &ViewOptions, want_focus: bool) -> Vec<Action> {
    let mut actions = Vec::new();
    let pal = opt.theme.palette();
    let (term_bg, term_fg, term_cursor, term_selection) = (pal.bg, pal.fg, pal.cursor, pal.selection);
    let full = ui.available_rect_before_wrap();
    let (cw, ch) = cell_metrics(ui, opt.font_size);
    let inner = full.shrink(PAD);
    let cols = ((inner.width() / cw).floor() as usize).max(10);
    let rows = ((inner.height() / ch).floor() as usize).max(3);
    s.resize(GridSize { cols, rows }, (cw.round() as u16, ch as u16));
    let origin = inner.min;

    let id = ui.scope_id().with(("terminal", s.id));
    let resp = ui.interact(full, id, Sense::click_and_drag());
    ui.painter().rect_filled(full, 0.0, term_bg);
    // Optional background image, cover-fit, at the chosen opacity.
    if let Some(img) = opt.theme.bg_image.as_ref().filter(|p| cache.exists(p)) {
        if let Slot::Ready { tex, size, .. } = cache.get(ui.ctx(), img) {
            let (iw, ih) = (size[0] as f32, size[1] as f32);
            let (k_w, k_h) = (full.width() / iw, full.height() / ih);
            let k = k_w.max(k_h);
            let (uw, uh) = (full.width() / (iw * k), full.height() / (ih * k));
            let uv = Rect::from_min_max(Pos2::new((1.0 - uw) / 2.0, (1.0 - uh) / 2.0), Pos2::new((1.0 + uw) / 2.0, (1.0 + uh) / 2.0));
            ui.painter().image(tex.id(), full, uv, Color32::from_white_alpha((opt.theme.bg_opacity.clamp(0.0, 1.0) * 255.0) as u8));
        }
    }
    if want_focus || resp.clicked() || resp.drag_started() || resp.secondary_clicked() {
        resp.request_focus();
    }
    let focused = resp.has_focus();
    if st.search.open {
        search_bar(ui, full, s, &mut st.search);
    }
    if focused {
        ui.memory_mut(|m| m.set_focus_lock_filter(id, egui::EventFilter { tab: true, horizontal_arrows: true, vertical_arrows: true, escape: true }));
    }

    let to_cell = |p: Pos2| -> (usize, usize, Side) {
        let x = ((p.x - origin.x) / cw).max(0.0);
        let col = (x.floor() as usize).min(cols - 1);
        let row = (((p.y - origin.y) / ch).max(0.0).floor() as usize).min(rows - 1);
        let side = if x.fract() < 0.5 { Side::Left } else { Side::Right };
        (row, col, side)
    };

    // ---- Input -----------------------------------------------------------------------------
    let (events, mods, time) = ui.input(|i| (i.events.clone(), i.modifiers, i.time));
    let mode = *s.term.lock().mode();
    let mouse_mode = mode.intersects(TermMode::MOUSE_MODE) && !mods.shift;

    if focused {
        for ev in &events {
            match ev {
                Event::Text(t) => {
                    // Alt+key is sent by the Key handler (ESC prefix); Ctrl+Alt is AltGr text.
                    if !(mods.alt && !mods.ctrl) {
                        s.term.lock().scroll_display(Scroll::Bottom);
                        let t = t.clone();
                        s.write_str(&t);
                    }
                }
                Event::Copy => {
                    let text = s.term.lock().selection_to_string();
                    match text {
                        Some(t) if !t.is_empty() => {
                            actions.push(Action::Copy(t));
                            s.term.lock().selection = None;
                        }
                        _ => s.write(b"\x03".to_vec()),
                    }
                }
                Event::Cut => s.write(b"\x18".to_vec()),
                Event::Paste(_) | Event::PasteImage(_) => actions.push(Action::Paste),
                Event::Key { key, pressed: true, modifiers, .. } => {
                    if let Some(bytes) = key_bytes(*key, *modifiers, mode, opt.shift_enter_alt) {
                        {
                            let mut t = s.term.lock();
                            t.scroll_display(Scroll::Bottom);
                            t.selection = None;
                        }
                        s.write(bytes);
                    } else if modifiers.shift && !modifiers.ctrl {
                        let delta = match key {
                            Key::PageUp => Some(Scroll::PageUp),
                            Key::PageDown => Some(Scroll::PageDown),
                            Key::Home => Some(Scroll::Top),
                            Key::End => Some(Scroll::Bottom),
                            _ => None,
                        };
                        if let Some(d) = delta {
                            s.term.lock().scroll_display(d);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    // Mouse wheel.
    if resp.hovered() || resp.contains_pointer() {
        let dy: f32 = events
            .iter()
            .filter_map(|e| match e {
                Event::MouseWheel { unit, delta, modifiers, .. } if !modifiers.ctrl => Some(match unit {
                    egui::MouseWheelUnit::Line => delta.y * 3.0,
                    egui::MouseWheelUnit::Page => delta.y * rows as f32,
                    egui::MouseWheelUnit::Point => delta.y / ch,
                }),
                _ => None,
            })
            .sum();
        if dy != 0.0 {
            st.scroll_accum += dy;
            let lines = st.scroll_accum.trunc() as i32;
            st.scroll_accum -= lines as f32;
            if lines != 0 {
                if mouse_mode {
                    if let Some(p) = resp.hover_pos() {
                        let (r, c, _) = to_cell(p);
                        let btn = if lines > 0 { 64 } else { 65 };
                        for _ in 0..lines.unsigned_abs() {
                            send_mouse(s, mode, btn, c, r, true);
                        }
                    }
                } else if mode.contains(TermMode::ALT_SCREEN) && mode.contains(TermMode::ALTERNATE_SCROLL) {
                    let seq: &[u8] = match (lines > 0, mode.contains(TermMode::APP_CURSOR)) {
                        (true, true) => b"\x1bOA",
                        (true, false) => b"\x1b[A",
                        (false, true) => b"\x1bOB",
                        (false, false) => b"\x1b[B",
                    };
                    let mut v = Vec::new();
                    for _ in 0..lines.unsigned_abs() {
                        v.extend_from_slice(seq);
                    }
                    s.write(v);
                } else {
                    s.term.lock().scroll_display(Scroll::Delta(lines));
                }
            }
        }
    }

    // Mouse buttons: reported to the app in mouse mode, otherwise selection.
    let pointer = resp.interact_pointer_pos().or(resp.hover_pos());
    if mouse_mode {
        if let Some(p) = pointer {
            let (r, c, _) = to_cell(p);
            for (b, code) in [(PointerButton::Primary, 0u8), (PointerButton::Middle, 1), (PointerButton::Secondary, 2)] {
                if ui.input(|i| i.pointer.button_pressed(b)) && resp.contains_pointer() {
                    send_mouse(s, mode, code, c, r, true);
                    st.mouse_down_reported = Some(code);
                }
                if ui.input(|i| i.pointer.button_released(b)) && st.mouse_down_reported == Some(code) {
                    send_mouse(s, mode, code, c, r, false);
                    st.mouse_down_reported = None;
                }
            }
            if resp.dragged() && mode.intersects(TermMode::MOUSE_DRAG | TermMode::MOUSE_MOTION) {
                if let Some(code) = st.mouse_down_reported {
                    send_mouse(s, mode, code + 32, c, r, true);
                }
            }
        }
    } else {
        let offset = s.term.lock().grid().display_offset() as i32;
        let grid_point = |row: usize, col: usize| Point::new(Line(row as i32 - offset), Column(col));
        if resp.drag_started_by(PointerButton::Primary) {
            if let Some(p) = resp.interact_pointer_pos() {
                let (r, c, side) = to_cell(p);
                s.term.lock().selection = Some(Selection::new(SelectionType::Simple, grid_point(r, c), side));
                st.selecting = true;
            }
        }
        if st.selecting && resp.dragged_by(PointerButton::Primary) {
            if let Some(p) = resp.interact_pointer_pos() {
                // Auto-scroll when dragging past the top / bottom edge.
                if p.y < inner.top() {
                    s.term.lock().scroll_display(Scroll::Delta(1));
                } else if p.y > inner.bottom() {
                    s.term.lock().scroll_display(Scroll::Delta(-1));
                }
                let offset = s.term.lock().grid().display_offset() as i32;
                let (r, c, side) = to_cell(p);
                let pt = Point::new(Line(r as i32 - offset), Column(c));
                if let Some(sel) = s.term.lock().selection.as_mut() {
                    sel.update(pt, side);
                }
            }
        }
        if resp.drag_stopped() {
            st.selecting = false;
        }
        if resp.clicked_by(PointerButton::Primary) {
            if let Some(p) = resp.interact_pointer_pos() {
                let (r, c, side) = to_cell(p);
                let pt = grid_point(r, c);
                let count = match st.last_click {
                    Some((t, lp, n)) if time - t < 0.4 && lp == pt => n % 3 + 1,
                    _ => 1,
                };
                st.last_click = Some((time, pt, count));
                let mut t = s.term.lock();
                t.selection = match count {
                    2 => Some(Selection::new(SelectionType::Semantic, pt, side)),
                    3 => Some(Selection::new(SelectionType::Lines, pt, side)),
                    _ => None,
                };
            }
        }
        if resp.secondary_clicked() {
            // Windows Terminal style: right-click copies a selection, otherwise pastes.
            let text = s.term.lock().selection_to_string();
            match text {
                Some(t) if !t.is_empty() => {
                    actions.push(Action::Copy(t));
                    s.term.lock().selection = None;
                }
                _ => actions.push(Action::Paste),
            }
        }
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
        }
    }

    // ---- Snapshot the grid under the lock --------------------------------------------------
    let (mut grid, wrapped, cursor, display_offset, history) = {
        let t = s.term.lock();
        let content = t.renderable_content();
        let colors = content.colors;
        let sel = content.selection;
        let off = content.display_offset as i32;
        let mut grid: Vec<Vec<SnapCell>> = (0..rows)
            .map(|_| (0..cols).map(|_| SnapCell { c: ' ', fg: term_fg, bg: term_bg, flags: Flags::empty(), selected: false, extra: None }).collect())
            .collect();
        let mut wrapped = vec![false; rows];
        for item in content.display_iter {
            let vr = item.point.line.0 + off;
            if vr < 0 || vr as usize >= rows || item.point.column.0 >= cols {
                continue;
            }
            let cell = item.cell;
            let flags = cell.flags;
            let mut fg = resolve(cell.fg, colors, &pal, flags.contains(Flags::BOLD), true);
            let mut bg = resolve(cell.bg, colors, &pal, false, false);
            if flags.contains(Flags::DIM) {
                fg = fg.linear_multiply(0.66);
            }
            if flags.contains(Flags::INVERSE) {
                std::mem::swap(&mut fg, &mut bg);
            }
            if flags.contains(Flags::HIDDEN) {
                fg = bg;
            }
            let selected = sel.is_some_and(|r| r.contains(item.point));
            if item.point.column.0 == cols - 1 && flags.contains(Flags::WRAPLINE) {
                wrapped[vr as usize] = true;
            }
            grid[vr as usize][item.point.column.0] = SnapCell { c: cell.c, fg, bg, flags, selected, extra: cell.zerowidth().map(|z| z.to_vec()) };
        }
        let cur = content.cursor;
        let cvr = cur.point.line.0 + off;
        let cursor = (cvr >= 0 && (cvr as usize) < rows && cur.shape != CursorShape::Hidden).then_some((cvr as usize, cur.point.column.0, cur.shape));
        (grid, wrapped, cursor, content.display_offset, t.grid().history_size())
    };
    let _ = s.dirty.swap(false, std::sync::atomic::Ordering::Relaxed);

    // Colour tags in the agent's text ({green}OK{/}); not on the row being typed.
    if opt.color_tags {
        let cursor_row = cursor.map(|c| c.0);
        for (r, row) in grid.iter_mut().enumerate() {
            if Some(r) == cursor_row {
                continue;
            }
            let chars: Vec<char> = row.iter().map(|c| c.c).collect();
            let Some(lay) = crate::colortags::layout(&chars) else { continue };
            let fg_of = |c: crate::colortags::TagColor| match c {
                crate::colortags::TagColor::Ansi(k) => pal.ansi[k],
                crate::colortags::TagColor::Rgb(v) => Color32::from_rgb(v[0], v[1], v[2]),
            };
            // Named backgrounds use Claude Code's dark diff shades (rows darker, spans brighter);
            // #rrggbb is used as given.
            let bg_of = |c: crate::colortags::TagColor, row: bool| match crate::colortags::bg_shade(c, row) {
                Some(v) => Color32::from_rgb(v[0], v[1], v[2]),
                None => fg_of(c),
            };
            let row_bg = lay.row_bg.map(|c| bg_of(c, true));
            let blank = SnapCell { c: ' ', fg: term_fg, bg: row_bg.unwrap_or(term_bg), flags: Flags::empty(), selected: false, extra: None };
            let new_row: Vec<SnapCell> = lay
                .cols
                .iter()
                .map(|o| match *o {
                    crate::colortags::Out::Src(i, st) => {
                        let mut cell = row[i].clone();
                        if let Some(c) = st.fg {
                            cell.fg = fg_of(c);
                        }
                        if let Some(c) = st.bg {
                            cell.bg = bg_of(c, false);
                        } else if let Some(rb) = row_bg {
                            if cell.bg == term_bg {
                                cell.bg = rb;
                            }
                        }
                        cell
                    }
                    crate::colortags::Out::Blank => blank.clone(),
                })
                .collect();
            *row = new_row;
        }
    }

    // ---- Paint -----------------------------------------------------------------------------
    let painter = ui.painter_at(full);
    let font = code_font(opt.font_size);
    let bold_font = egui::FontId::new(opt.font_size, code_bold());
    let cell_rect = |r: usize, c: usize, w: usize| Rect::from_min_size(Pos2::new(origin.x + c as f32 * cw, origin.y + r as f32 * ch), Vec2::new(cw * w as f32, ch));

    for (r, row) in grid.iter().enumerate() {
        // Backgrounds, merged into runs.
        let mut c = 0;
        while c < cols {
            let bg = if row[c].selected { term_selection } else { row[c].bg };
            let start = c;
            while c < cols && (if row[c].selected { term_selection } else { row[c].bg }) == bg {
                c += 1;
            }
            if bg != term_bg {
                painter.rect_filled(cell_rect(r, start, c - start).expand2(Vec2::new(0.3, 0.0)), 0.0, bg);
            }
        }
        // Find matches on this row (the current one brighter), under the text.
        if st.search.open && !st.search.matches.is_empty() {
            let line = r as i32 - display_offset as i32;
            for (k, &(l, a, b)) in st.search.matches.iter().enumerate() {
                if l == line {
                    let cur = st.search.cur == Some(k);
                    let col = if cur { Color32::from_rgb(200, 130, 10) } else { Color32::from_rgb(90, 75, 15) };
                    painter.rect_filled(cell_rect(r, a, b - a + 1), 2.0, col);
                }
            }
        }
        // Text: ASCII runs of one colour go out as one string; anything else is placed in its
        // own cell, so fallback-font glyphs with a different advance can't push the rest.
        let mut c = 0;
        while c < cols {
            let cell = &row[c];
            if cell.flags.contains(Flags::WIDE_CHAR_SPACER) || cell.flags.contains(Flags::LEADING_WIDE_CHAR_SPACER) {
                c += 1;
                continue;
            }
            let y = origin.y + r as f32 * ch;
            if cell.c == ' ' || cell.c == '\0' {
                decorate(&painter, cell, cell_rect(r, c, 1));
                c += 1;
                continue;
            }
            if draw_box_char(&painter, cell.c, cell_rect(r, c, 1), cell.fg) {
                c += 1;
                continue;
            }
            if cell.c.is_ascii() && cell.extra.is_none() {
                let start = c;
                let mut text = String::new();
                while c < cols {
                    let n = &row[c];
                    if !(n.c.is_ascii() && n.extra.is_none() && n.fg == cell.fg && (n.flags & (Flags::ITALIC | Flags::ALL_UNDERLINES | Flags::STRIKEOUT)) == (cell.flags & (Flags::ITALIC | Flags::ALL_UNDERLINES | Flags::STRIKEOUT))) || is_box(n.c) {
                        break;
                    }
                    text.push(if n.c == '\0' { ' ' } else { n.c });
                    c += 1;
                }
                let trimmed = text.trim_end();
                if !trimmed.is_empty() {
                    // egui snaps a galley's origin to a whole pixel but places its glyphs at
                    // fractional advances, so a run started mid-row can land a pixel off from
                    // the same text drawn as part of a longer run. Anchoring every run at the
                    // row start (padded with spaces) keeps each glyph's x independent of how
                    // the row happens to split into runs, so text doesn't jitter as it redraws.
                    let padded = format!("{:start$}{trimmed}", "");
                    let (x, trimmed) = (origin.x, padded.as_str());
                    let ty = y + (ch - opt.font_size * 1.25).max(0.0) / 2.0;
                    let bold = cell.flags.contains(Flags::BOLD);
                    if bold && has_code_bold() {
                        painter.text(Pos2::new(x, ty), egui::Align2::LEFT_TOP, trimmed, bold_font.clone(), cell.fg);
                    } else {
                        painter.text(Pos2::new(x, ty), egui::Align2::LEFT_TOP, trimmed, font.clone(), cell.fg);
                        if bold {
                            painter.text(Pos2::new(x + 0.6, ty), egui::Align2::LEFT_TOP, trimmed, font.clone(), cell.fg);
                        }
                    }
                }
                decorate(&painter, cell, cell_rect(r, start, c - start));
                continue;
            }
            let w = if cell.flags.contains(Flags::WIDE_CHAR) { 2 } else { 1 };
            let mut g = cell.c.to_string();
            if let Some(z) = &cell.extra {
                g.extend(z.iter());
            }
            let rect = cell_rect(r, c, w);
            // Colour emoji (🟢 ✅ …) are drawn from a Direct2D rendering of Segoe UI Emoji,
            // since egui only has monochrome outlines.
            if crate::emoji::is_emoji(cell.c, cell.extra.as_deref()) {
                let ppp = ui.ctx().pixels_per_point();
                let px = [(rect.width() * ppp).round() as u32, (rect.height() * ppp).round() as u32];
                if let Some(tex) = crate::emoji::texture(ui.ctx(), &g, px, cell.fg) {
                    painter.image(tex, rect, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
                    decorate(&painter, cell, rect);
                    c += w;
                    continue;
                }
            }
            painter.text(rect.center(), egui::Align2::CENTER_CENTER, g, font.clone(), cell.fg);
            decorate(&painter, cell, rect);
            c += w;
        }
    }

    // Cursor.
    if let Some((r, c, shape)) = cursor {
        {
            let w = if grid[r][c].flags.contains(Flags::WIDE_CHAR) { 2 } else { 1 };
            let rect = cell_rect(r, c, w);
            match (shape, focused) {
                (CursorShape::Block, true) => {
                    painter.rect_filled(rect, 1.0, term_cursor);
                    let cell = &grid[r][c];
                    if cell.c != ' ' && cell.c != '\0' {
                        painter.text(rect.center(), egui::Align2::CENTER_CENTER, cell.c.to_string(), font.clone(), term_bg);
                    }
                }
                (CursorShape::Underline, _) => {
                    painter.rect_filled(Rect::from_min_max(Pos2::new(rect.left(), rect.bottom() - 2.0), rect.max), 0.0, term_cursor);
                }
                (CursorShape::Beam, _) => {
                    painter.rect_filled(Rect::from_min_size(rect.min, Vec2::new(2.0, ch)), 0.0, term_cursor);
                }
                _ => {
                    painter.rect_stroke(rect.shrink(0.5), 1.0, Stroke::new(1.0, term_cursor), egui::StrokeKind::Inside);
                }
            }
        }
    }

    // Scrollback indicator.
    if display_offset > 0 {
        let txt = format!("↑ {display_offset} / {history} lines  ·  Shift+End to return");
        let g = painter.layout_no_wrap(txt, egui::FontId::proportional(12.0), TEXT);
        let r = Rect::from_min_size(Pos2::new(full.right() - g.size().x - 22.0, full.top() + 6.0), g.size() + Vec2::new(14.0, 6.0));
        painter.rect_filled(r, 5.0, Color32::from_rgba_unmultiplied(40, 44, 52, 230));
        painter.galley(r.min + Vec2::new(7.0, 3.0), g, TEXT);
    }

    // ---- Image references ------------------------------------------------------------------
    let mut refs: Vec<(usize, Rect, PathBuf, String)> = Vec::new(); // (first row, span rect, path, label)
    let mut r = 0;
    while r < rows {
        // Join soft-wrapped rows into one logical line; remember where each byte came from.
        let mut text = String::new();
        let mut at: Vec<(usize, usize)> = Vec::new();
        loop {
            for (c, cell) in grid[r].iter().enumerate() {
                if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                    continue;
                }
                let ch_ = if cell.c == '\0' { ' ' } else { cell.c };
                for _ in 0..ch_.len_utf8() {
                    at.push((r, c));
                }
                text.push(ch_);
            }
            if wrapped[r] && r + 1 < rows {
                r += 1;
            } else {
                break;
            }
        }
        r += 1;
        if !text.contains(".") && !text.contains("[Image") {
            continue;
        }
        for f in images::find_refs(&text) {
            let (path, label) = match &f.kind {
                RefKind::Path(p) => (p.clone(), p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()),
                RefKind::Placeholder(n) => match s.image_for_placeholder(*n) {
                    Some(p) => (p.clone(), format!("Image #{n}")),
                    None => continue,
                },
            };
            if !cache.exists(&path) {
                continue;
            }
            let (r0, c0) = at[f.start];
            let (r1, c1) = at[(f.end - 1).min(at.len() - 1)];
            // Underline the span (it may wrap over several rows).
            for rr in r0..=r1 {
                let a = if rr == r0 { c0 } else { 0 };
                let b = if rr == r1 { c1 } else { cols - 1 };
                let rect = cell_rect(rr, a, b - a + 1);
                painter.line_segment([Pos2::new(rect.left(), rect.bottom() - 1.0), Pos2::new(rect.right(), rect.bottom() - 1.0)], Stroke::new(1.0, LINK.gamma_multiply(0.8)));
                let hr = ui.interact(rect, id.with(("ref", rr, a)), Sense::click());
                if hr.hovered() && !st.selecting {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    hover_preview(ui, cache, &path, &label, "Click to open");
                }
                if hr.clicked() {
                    actions.push(Action::View(path.clone()));
                }
            }
            refs.push((r0, cell_rect(r0, c0, 1), path, label));
        }
    }

    // Links: with Ctrl held, the URL / file path under the mouse is underlined; Ctrl+click opens
    // it. Only the line under the mouse is examined.
    if mods.ctrl {
        if let Some(pos) = resp.hover_pos() {
            let (pr, pc, _) = to_cell(pos);
            // The logical line (soft-wrapped rows joined) around the pointer's row.
            let mut r0 = pr;
            while r0 > 0 && wrapped[r0 - 1] {
                r0 -= 1;
            }
            let mut text = String::new();
            let mut at: Vec<(usize, usize)> = Vec::new();
            let mut r = r0;
            loop {
                for (c, cell) in grid[r].iter().enumerate() {
                    if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                        continue;
                    }
                    let ch_ = if cell.c == '\0' { ' ' } else { cell.c };
                    for _ in 0..ch_.len_utf8() {
                        at.push((r, c));
                    }
                    text.push(ch_);
                }
                if wrapped[r] && r + 1 < rows {
                    r += 1;
                } else {
                    break;
                }
            }
            for f in crate::links::find(&text) {
                let (Some(&(ra, ca)), Some(&(rb, cb))) = (at.get(f.start), at.get(f.end.saturating_sub(1))) else { continue };
                let inside = (pr, pc) >= (ra, ca) && (pr, pc) <= (rb, cb);
                if !inside {
                    continue;
                }
                let ok = match &f.link {
                    crate::links::Link::Url(_) => true,
                    crate::links::Link::File { path, .. } => crate::links::resolve(path, &s.cwd).is_some(),
                };
                if !ok {
                    continue;
                }
                for rr in ra..=rb {
                    let a = if rr == ra { ca } else { 0 };
                    let b = if rr == rb { cb } else { cols - 1 };
                    let rect = cell_rect(rr, a, b - a + 1);
                    painter.line_segment([Pos2::new(rect.left(), rect.bottom() - 1.0), Pos2::new(rect.right(), rect.bottom() - 1.0)], Stroke::new(1.5, LINK));
                }
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                let tip = match &f.link {
                    crate::links::Link::Url(u) => format!("Ctrl+click to open {u}"),
                    crate::links::Link::File { line: Some(l), .. } => format!("Ctrl+click to open in the editor at line {l}"),
                    crate::links::Link::File { .. } => "Ctrl+click to open".to_string(),
                };
                egui::Tooltip::always_open(ui.ctx().clone(), ui.layer_id(), egui::Id::unique("link_tip"), egui::PopupAnchor::Pointer).show(|ui| {
                    ui.label(tip);
                });
                if resp.clicked() {
                    actions.push(Action::OpenLink(f.link.clone()));
                }
                break;
            }
        }
    }

    // Inline previews, at the right edge next to the row that mentions the image. A preview
    // shrinks to the free space right of the text it covers; previews that would collide stack
    // downwards.
    if opt.inline_previews && !refs.is_empty() {
        // Last non-blank column of each row (-1 = empty row).
        let row_end: Vec<i32> = grid.iter().map(|r| r.iter().rposition(|c| c.c != ' ' && c.c != '\0').map_or(-1, |i| i as i32)).collect();
        let free_left = |top: f32, bottom: f32| -> f32 {
            let r0 = (((top - origin.y) / ch).floor().max(0.0) as usize).min(rows - 1);
            // At least the first covered row (a degenerate height must not cover nothing).
            let r1 = ((((bottom - origin.y) / ch).ceil().max(1.0) as usize).saturating_sub(1)).clamp(r0, rows - 1);
            let end = (r0..=r1).map(|r| row_end[r]).max().unwrap_or(-1);
            origin.x + (end + 2) as f32 * cw
        };
        let mut placed: Vec<Placed> = Vec::new();
        let right = inner.right() - 6.0;
        // Smallest preview worth showing when it has to shrink into the space beside the text.
        let min_w = 64.0_f32.min(opt.preview_max_px);
        for (row, _span, path, label) in refs.iter() {
            if placed.iter().any(|p| &p.path == path) {
                continue;
            }
            let Slot::Ready { size, .. } = cache.get(ui.ctx(), path) else { continue };
            let aspect = size[0] as f32 / size[1].max(1) as f32;
            // Full size: the longest side at most max size (and at most 60% of the terminal).
            let mut fw = if aspect >= 1.0 { opt.preview_max_px } else { opt.preview_max_px * aspect };
            fw = fw.min(inner.width() * 0.6).min(inner.height() * 0.6 * aspect).max(8.0);
            let row_top = origin.y + *row as f32 * ch;
            let mut top = row_top;
            let mut ok = None;
            for _ in 0..10 {
                let (mut w, mut h) = (fw, fw / aspect);
                if top + h > inner.bottom() {
                    top = (inner.bottom() - h).max(inner.top());
                }
                let avail = right - free_left(top, top + h);
                if avail < w {
                    if opt.preview_over_text {
                        // May cover the end of long lines, but never below a usable size.
                        w = avail.max((10.0 * cw).min(fw));
                        h = w / aspect;
                    } else {
                        // Shrink into the free space; a shorter preview covers fewer lines, so check again.
                        if avail < min_w {
                            top += ch;
                            if top > row_top + 8.0 * ch || top >= inner.bottom() - min_w / aspect {
                                break;
                            }
                            continue;
                        }
                        w = avail;
                        h = w / aspect;
                        let avail2 = right - free_left(top, top + h);
                        if avail2 < w {
                            w = avail2;
                            h = w / aspect;
                        }
                        if w < min_w {
                            // No room beside these lines: try a little further down.
                            top += ch;
                            if top > row_top + 8.0 * ch || top >= inner.bottom() - min_w / aspect {
                                break;
                            }
                            continue;
                        }
                    }
                }
                let rect = Rect::from_min_max(Pos2::new(right - w, top), Pos2::new(right, top + h));
                match placed.iter().find(|p| p.rect.expand(4.0).intersects(rect)) {
                    Some(p) if p.rect.bottom() + 8.0 + h <= inner.bottom() => top = p.rect.bottom() + 8.0,
                    Some(_) => break,
                    None => {
                        ok = Some(rect);
                        break;
                    }
                }
            }
            if let Some(rect) = ok {
                placed.push(Placed { rect, path: path.clone(), label: label.clone() });
            }
        }
        let ppp = ui.ctx().pixels_per_point();
        for p in &placed {
            // A Lanczos copy at the exact on-screen size (mipmapped texture until it's ready).
            let Some(tex_id) = cache.texture_for(ui.ctx(), &p.path, p.rect.size() * ppp) else { continue };
            let frame = p.rect.expand(3.0);
            let hr = ui.interact(frame, id.with(("thumb", &p.path)), Sense::click());
            let hovered = hr.hovered();
            painter.rect_filled(frame.translate(Vec2::new(0.0, 3.0)), 7.0, Color32::from_black_alpha(90));
            painter.rect_filled(frame, 6.0, CARD);
            painter.image(tex_id, p.rect, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
            painter.rect_stroke(frame, 6.0, Stroke::new(1.0, if hovered { ACCENT_HI } else { BORDER }), egui::StrokeKind::Inside);
            // No filename chip on the image (it hid the picture); the hover preview names the file.
            if hovered {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                hover_preview(ui, cache, &p.path, &p.label, "Click to open");
            }
            if hr.clicked() {
                actions.push(Action::View(p.path.clone()));
            }
        }
    }

    if !focused && !want_focus {
        // A faint frame shows the terminal doesn't have the keyboard.
        painter.rect_stroke(full.shrink(0.5), 0.0, Stroke::new(1.0, Color32::from_rgb(40, 43, 50)), egui::StrokeKind::Inside);
    }
    actions
}

/// A large preview in a tooltip.
pub fn hover_preview(ui: &Ui, cache: &mut ImageCache, path: &std::path::Path, label: &str, hint: &str) {
    let ctx = ui.ctx().clone();
    let screen = ctx.content_rect().size();
    let max = Vec2::new((screen.x * 0.45).min(720.0), (screen.y * 0.55).min(540.0));
    let slot = cache.get(&ctx, path);
    let (size, bytes) = match slot {
        Slot::Ready { size, bytes, .. } => (*size, *bytes),
        Slot::Loading => ([0, 0], 0),
        Slot::Failed(e) => {
            let e = e.clone();
            egui::Tooltip::always_open(ctx, ui.layer_id(), egui::Id::unique("img_tip"), egui::PopupAnchor::Pointer).show(|ui| {
                ui.label(format!("Can't load {}: {e}", path.display()));
            });
            return;
        }
    };
    let s = Vec2::new(size[0] as f32, size[1] as f32);
    let k = if size[0] == 0 { 1.0 } else { (max.x / s.x).min(max.y / s.y).min(1.0) };
    let tex = if size[0] == 0 { None } else { cache.texture_for(&ctx, path, s * k * ctx.pixels_per_point()) };
    egui::Tooltip::always_open(ctx.clone(), ui.layer_id(), egui::Id::unique("img_tip"), egui::PopupAnchor::Pointer).show(|ui| {
        match tex {
            Some(tex) => {
                ui.add(egui::Image::new((tex, s * k)).corner_radius(4.0));
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(label).family(bold()).color(TEXT));
                    ui.label(egui::RichText::new(format!("{}×{}  ·  {}  ·  {hint}", size[0], size[1], images::human_size(bytes))).color(TEXT_DIM).size(12.0));
                });
            }
            None => {
                ui.label("Loading preview…");
            }
        }
    });
}

/// Find bar state (Ctrl+Shift+F) of one terminal.
#[derive(Default)]
pub struct Search {
    pub open: bool,
    pub query: String,
    pub case: bool,
    /// Matches as (grid line, first column, last column), top to bottom.
    matches: Vec<(i32, usize, usize)>,
    cur: Option<usize>,
    /// What `matches` was computed for (query, case, history size, newest line text).
    computed: Option<(String, bool, usize, u64)>,
    pub focus: bool,
}

impl Search {
    pub fn open(&mut self) {
        self.open = true;
        self.focus = true;
    }
}

/// Finds `q` in every line of the terminal (scrollback included).
fn search_grid(t: &alacritty_terminal::Term<crate::session::Listener>, q: &str, case: bool) -> Vec<(i32, usize, usize)> {
    let mut out = Vec::new();
    if q.is_empty() {
        return out;
    }
    let grid = t.grid();
    let cols = grid.columns();
    let top = -(grid.history_size() as i32);
    let bottom = grid.screen_lines() as i32 - 1;
    let needle: Vec<char> = if case { q.chars().collect() } else { q.to_lowercase().chars().collect() };
    let mut line: Vec<char> = Vec::with_capacity(cols);
    for l in top..=bottom {
        line.clear();
        let row = &grid[Line(l)];
        for c in 0..cols {
            let ch = row[Column(c)].c;
            let ch = if ch == '\0' { ' ' } else { ch };
            line.push(if case { ch } else { ch.to_lowercase().next().unwrap_or(ch) });
        }
        if line.len() < needle.len() {
            continue;
        }
        let mut i = 0;
        while i + needle.len() <= line.len() {
            if line[i..i + needle.len()] == needle[..] {
                out.push((l, i, i + needle.len() - 1));
                i += needle.len();
            } else {
                i += 1;
            }
        }
    }
    out
}

/// Fingerprint of the screen (to notice new output while the find bar is open).
fn tail_hash(t: &alacritty_terminal::Term<crate::session::Listener>) -> u64 {
    use std::hash::{Hash, Hasher};
    let grid = t.grid();
    let mut h = std::collections::hash_map::DefaultHasher::new();
    let bottom = grid.screen_lines() as i32 - 1;
    for l in 0..=bottom {
        for c in 0..grid.columns() {
            grid[Line(l)][Column(c)].c.hash(&mut h);
        }
    }
    h.finish()
}

/// Recomputes matches when the query or the content changed; moves to `step` (+1 next, -1 previous).
fn search_update(s: &mut Session, sr: &mut Search, step: i32) {
    let mut t = s.term.lock();
    let key = (sr.query.clone(), sr.case, t.grid().history_size(), tail_hash(&t));
    if sr.computed.as_ref() != Some(&key) {
        let query_changed = sr.computed.as_ref().is_none_or(|k| k.0 != key.0 || k.1 != key.1);
        sr.matches = search_grid(&t, &sr.query, sr.case);
        sr.computed = Some(key);
        if query_changed {
            // Start from the newest match (bottom), like Windows Terminal.
            sr.cur = sr.matches.len().checked_sub(1);
            if let Some(i) = sr.cur {
                let (l, c, _) = sr.matches[i];
                t.scroll_to_point(Point::new(Line(l), Column(c)));
            }
        } else if let Some(i) = sr.cur {
            sr.cur = Some(i.min(sr.matches.len().saturating_sub(1))).filter(|_| !sr.matches.is_empty());
        } else {
            // Results came in after the query (new output): select the newest.
            sr.cur = sr.matches.len().checked_sub(1);
        }
    }
    if step != 0 && !sr.matches.is_empty() {
        let n = sr.matches.len() as i32;
        let i = sr.cur.map(|i| i as i32).unwrap_or(n - 1);
        let j = ((i + step) % n + n) % n;
        sr.cur = Some(j as usize);
        let (l, c, _) = sr.matches[j as usize];
        t.scroll_to_point(Point::new(Line(l), Column(c)));
    }
}

/// The find bar, at the top right of the terminal.
fn search_bar(ui: &mut Ui, area: Rect, s: &mut Session, sr: &mut Search) {
    let w = 430.0;
    let r = Rect::from_min_size(Pos2::new(area.right() - w - 14.0, area.top() + 8.0), Vec2::new(w, 34.0));
    let mut step = 0;
    let mut close = false;
    egui::Area::new(egui::Id::unique(("find_bar", s.id))).fixed_pos(r.min).order(egui::Order::Foreground).show(ui.ctx(), |ui| {
        ui.set_width(w);
        egui::Frame::new().fill(CARD_HI).stroke(Stroke::new(1.0, BORDER)).corner_radius(6).inner_margin(egui::Margin::symmetric(6, 4)).show(ui, |ui| {
            ui.horizontal(|ui| {
                let id = ui.make_persistent_id(("find", s.id));
                let te = ui.add(egui::TextEdit::singleline(&mut sr.query).id(id).hint_text("Find in terminal").desired_width(190.0));
                if std::mem::take(&mut sr.focus) {
                    te.request_focus();
                }
                // Enter = next, Shift+Enter = previous (Enter ends the edit; take the focus back).
                if te.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                    step = if ui.input(|i| i.modifiers.shift) { -1 } else { 1 };
                    te.request_focus();
                }
                if te.has_focus() && ui.input(|i| i.key_pressed(Key::Escape)) {
                    close = true;
                }
                let count = match (sr.cur, sr.matches.len()) {
                    (_, 0) if sr.query.is_empty() => String::new(),
                    (_, 0) => "No results".into(),
                    (Some(i), n) => format!("{} of {n}", i + 1),
                    (None, n) => format!("{n}"),
                };
                ui.add_sized(Vec2::new(78.0, 20.0), egui::Label::new(egui::RichText::new(count).size(12.0).color(TEXT_DIM)));
                if ui.small_button("↑").on_hover_text("Previous (Shift+Enter)").clicked() {
                    step = -1;
                }
                if ui.small_button("↓").on_hover_text("Next (Enter)").clicked() {
                    step = 1;
                }
                let aa = egui::Button::new(egui::RichText::new("Aa").size(12.0).color(if sr.case { Color32::WHITE } else { TEXT_DIM })).fill(if sr.case { ACCENT } else { Color32::TRANSPARENT });
                if ui.add(aa).on_hover_text("Match case").clicked() {
                    sr.case = !sr.case;
                }
                if ui.small_button("✕").on_hover_text("Close (Esc)").clicked() {
                    close = true;
                }
            });
        });
    });
    if close {
        sr.open = false;
        sr.matches.clear();
        sr.computed = None;
        sr.cur = None;
        s.term.lock().scroll_display(Scroll::Bottom);
        return;
    }
    search_update(s, sr, step);
}

fn decorate(p: &egui::Painter, cell: &SnapCell, rect: Rect) {
    if cell.flags.intersects(Flags::ALL_UNDERLINES) {
        let y = rect.bottom() - 2.0;
        p.line_segment([Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)], Stroke::new(1.0, cell.fg));
    }
    if cell.flags.contains(Flags::STRIKEOUT) {
        let y = rect.center().y;
        p.line_segment([Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)], Stroke::new(1.0, cell.fg));
    }
}

fn is_box(c: char) -> bool {
    matches!(c, '\u{2500}'..='\u{259F}')
}

/// Draws box-drawing and block characters as shapes, so lines join seamlessly between cells
/// whatever the font and line height. Returns false for characters it doesn't handle.
fn draw_box_char(p: &egui::Painter, c: char, r: Rect, col: Color32) -> bool {
    if !is_box(c) {
        return false;
    }
    let (cx, cy) = (r.center().x.round() + 0.5, r.center().y.round() + 0.5);
    let light = Stroke::new(1.0, col);
    let heavy = Stroke::new(2.0, col);
    let h = |p: &egui::Painter, x0: f32, x1: f32, s: Stroke| { p.line_segment([Pos2::new(x0, cy), Pos2::new(x1, cy)], s); };
    let v = |p: &egui::Painter, y0: f32, y1: f32, s: Stroke| { p.line_segment([Pos2::new(cx, y0), Pos2::new(cx, y1)], s); };
    // (left, right, up, down): 0 = none, 1 = light, 2 = heavy, 3 = double.
    let arms: Option<(u8, u8, u8, u8)> = match c {
        '─' => Some((1, 1, 0, 0)),
        '━' => Some((2, 2, 0, 0)),
        '│' => Some((0, 0, 1, 1)),
        '┃' => Some((0, 0, 2, 2)),
        '┌' => Some((0, 1, 0, 1)),
        '┐' => Some((1, 0, 0, 1)),
        '└' => Some((0, 1, 1, 0)),
        '┘' => Some((1, 0, 1, 0)),
        '├' => Some((0, 1, 1, 1)),
        '┤' => Some((1, 0, 1, 1)),
        '┬' => Some((1, 1, 0, 1)),
        '┴' => Some((1, 1, 1, 0)),
        '┼' => Some((1, 1, 1, 1)),
        '┏' => Some((0, 2, 0, 2)),
        '┓' => Some((2, 0, 0, 2)),
        '┗' => Some((0, 2, 2, 0)),
        '┛' => Some((2, 0, 2, 0)),
        '═' => Some((3, 3, 0, 0)),
        '║' => Some((0, 0, 3, 3)),
        '╔' => Some((0, 3, 0, 3)),
        '╗' => Some((3, 0, 0, 3)),
        '╚' => Some((0, 3, 3, 0)),
        '╝' => Some((3, 0, 3, 0)),
        '╴' => Some((1, 0, 0, 0)),
        '╶' => Some((0, 1, 0, 0)),
        '╵' => Some((0, 0, 1, 0)),
        '╷' => Some((0, 0, 0, 1)),
        _ => None,
    };
    if let Some((l, rt, u, d)) = arms {
        let st = |k: u8| if k == 2 { heavy } else { light };
        let dbl_h = |p: &egui::Painter, x0: f32, x1: f32| {
            p.line_segment([Pos2::new(x0, cy - 2.0), Pos2::new(x1, cy - 2.0)], light);
            p.line_segment([Pos2::new(x0, cy + 2.0), Pos2::new(x1, cy + 2.0)], light);
        };
        let dbl_v = |p: &egui::Painter, y0: f32, y1: f32| {
            p.line_segment([Pos2::new(cx - 2.0, y0), Pos2::new(cx - 2.0, y1)], light);
            p.line_segment([Pos2::new(cx + 2.0, y0), Pos2::new(cx + 2.0, y1)], light);
        };
        let ext = 0.5;
        if l > 0 {
            if l == 3 { dbl_h(p, r.left(), cx + 2.0) } else { h(p, r.left(), cx + ext, st(l)) }
        }
        if rt > 0 {
            if rt == 3 { dbl_h(p, cx - 2.0, r.right()) } else { h(p, cx - ext, r.right(), st(rt)) }
        }
        if u > 0 {
            if u == 3 { dbl_v(p, r.top(), cy + 2.0) } else { v(p, r.top(), cy + ext, st(u)) }
        }
        if d > 0 {
            if d == 3 { dbl_v(p, cy - 2.0, r.bottom()) } else { v(p, cy - ext, r.bottom(), st(d)) }
        }
        return true;
    }
    match c {
        // Rounded corners (Claude Code's input box).
        '╭' | '╮' | '╰' | '╯' => {
            let rad = (r.width().min(r.height()) * 0.5).min(6.0);
            let (sx, sy) = match c {
                '╭' => (1.0, 1.0),
                '╮' => (-1.0, 1.0),
                '╰' => (1.0, -1.0),
                _ => (-1.0, -1.0),
            };
            // From the vertical arm through a quarter circle into the horizontal arm.
            let y_end = if sy > 0.0 { r.bottom() } else { r.top() };
            let x_end = if sx > 0.0 { r.right() } else { r.left() };
            let mut pts = vec![Pos2::new(cx, y_end), Pos2::new(cx, cy + sy * rad)];
            let ccx = cx + sx * rad;
            let ccy = cy + sy * rad;
            for i in 0..=8 {
                let t = i as f32 / 8.0 * std::f32::consts::FRAC_PI_2;
                pts.push(Pos2::new(ccx - sx * rad * t.cos(), ccy - sy * rad * t.sin()));
            }
            pts.push(Pos2::new(x_end, cy));
            p.add(egui::Shape::line(pts, light));
            true
        }
        '█' => {
            p.rect_filled(r.expand(0.3), 0.0, col);
            true
        }
        '▀' => {
            p.rect_filled(Rect::from_min_max(r.min, Pos2::new(r.right(), r.center().y)).expand(0.3), 0.0, col);
            true
        }
        '▄' => {
            p.rect_filled(Rect::from_min_max(Pos2::new(r.left(), r.center().y), r.max).expand(0.3), 0.0, col);
            true
        }
        '▌' => {
            p.rect_filled(Rect::from_min_max(r.min, Pos2::new(r.center().x, r.bottom())).expand(0.3), 0.0, col);
            true
        }
        '▐' => {
            p.rect_filled(Rect::from_min_max(Pos2::new(r.center().x, r.top()), r.max).expand(0.3), 0.0, col);
            true
        }
        '░' | '▒' | '▓' => {
            let a = match c {
                '░' => 0.25,
                '▒' => 0.5,
                _ => 0.75,
            };
            p.rect_filled(r.expand(0.3), 0.0, col.gamma_multiply(a));
            true
        }
        _ => false,
    }
}

fn resolve(c: Color, colors: &Colors, pal: &crate::term_theme::Palette, bold: bool, is_fg: bool) -> Color32 {
    let idx = |i: usize| -> Color32 {
        if let Some(rgb) = colors[i] {
            return Color32::from_rgb(rgb.r, rgb.g, rgb.b);
        }
        palette(pal, i)
    };
    match c {
        Color::Spec(rgb) => Color32::from_rgb(rgb.r, rgb.g, rgb.b),
        Color::Indexed(i) => {
            let i = i as usize;
            idx(if bold && i < 8 { i + 8 } else { i })
        }
        Color::Named(n) => {
            let i = n as usize;
            match n {
                NamedColor::Foreground | NamedColor::BrightForeground => colors[i].map(|c| Color32::from_rgb(c.r, c.g, c.b)).unwrap_or(if bold { pal.ansi[15] } else { pal.fg }),
                NamedColor::Background => colors[i].map(|c| Color32::from_rgb(c.r, c.g, c.b)).unwrap_or(pal.bg),
                NamedColor::Cursor => pal.cursor,
                NamedColor::DimForeground => pal.fg.linear_multiply(0.66),
                _ if i < 16 => idx(if bold && i < 8 { i + 8 } else { i }),
                _ if (259..267).contains(&i) => idx(i - 259).linear_multiply(0.66),
                _ => {
                    if is_fg {
                        pal.fg
                    } else {
                        pal.bg
                    }
                }
            }
        }
    }
}

/// xterm-256 palette with the ANSI 16 from the theme.
fn palette(pal: &crate::term_theme::Palette, i: usize) -> Color32 {
    match i {
        0..=15 => pal.ansi[i],
        16..=231 => {
            let i = i - 16;
            let v = |x: usize| if x == 0 { 0 } else { (55 + x * 40) as u8 };
            Color32::from_rgb(v(i / 36), v((i / 6) % 6), v(i % 6))
        }
        232..=255 => {
            let g = (8 + (i - 232) * 10) as u8;
            Color32::from_rgb(g, g, g)
        }
        _ => pal.fg,
    }
}

fn send_mouse(s: &mut Session, mode: TermMode, button: u8, col: usize, row: usize, pressed: bool) {
    let mut b = button;
    let msg = if mode.contains(TermMode::SGR_MOUSE) {
        format!("\x1b[<{};{};{}{}", b, col + 1, row + 1, if pressed { 'M' } else { 'm' })
    } else {
        if !pressed {
            b = 3;
        }
        if col > 222 || row > 222 {
            return;
        }
        let mut v = vec![0x1b, b'[', b'M', 32 + b, 33 + col as u8, 33 + row as u8];
        s.write(std::mem::take(&mut v));
        return;
    };
    s.write(msg.into_bytes());
}

/// The bytes a key press sends, or None for keys the terminal doesn't handle.
fn key_bytes(key: Key, m: Modifiers, mode: TermMode, shift_enter_alt: bool) -> Option<Vec<u8>> {
    let app_cursor = mode.contains(TermMode::APP_CURSOR);
    let modnum = 1 + (m.shift as u8) + 2 * (m.alt as u8) + 4 * (m.ctrl as u8);
    let csi = |final_: char| -> Vec<u8> {
        if modnum > 1 {
            format!("\x1b[1;{modnum}{final_}").into_bytes()
        } else if app_cursor {
            format!("\x1bO{final_}").into_bytes()
        } else {
            format!("\x1b[{final_}").into_bytes()
        }
    };
    let tilde = |n: u8| -> Vec<u8> {
        if modnum > 1 {
            format!("\x1b[{n};{modnum}~").into_bytes()
        } else {
            format!("\x1b[{n}~").into_bytes()
        }
    };
    let esc_prefix = |mut v: Vec<u8>| -> Vec<u8> {
        if m.alt {
            v.insert(0, 0x1b);
        }
        v
    };
    Some(match key {
        Key::Enter => {
            if m.shift && shift_enter_alt {
                b"\x1b\r".to_vec()
            } else {
                esc_prefix(b"\r".to_vec())
            }
        }
        Key::Backspace => esc_prefix(if m.ctrl { b"\x08".to_vec() } else { b"\x7f".to_vec() }),
        Key::Tab => {
            if m.shift {
                b"\x1b[Z".to_vec()
            } else {
                b"\t".to_vec()
            }
        }
        Key::Escape => b"\x1b".to_vec(),
        Key::ArrowUp => csi('A'),
        Key::ArrowDown => csi('B'),
        Key::ArrowRight => csi('C'),
        Key::ArrowLeft => csi('D'),
        Key::Home if !m.shift => csi('H'),
        Key::End if !m.shift => csi('F'),
        Key::Insert if !m.shift => tilde(2),
        Key::Delete => tilde(3),
        Key::PageUp if !m.shift => tilde(5),
        Key::PageDown if !m.shift => tilde(6),
        Key::F1 => if modnum > 1 { format!("\x1b[1;{modnum}P").into_bytes() } else { b"\x1bOP".to_vec() },
        Key::F2 => if modnum > 1 { format!("\x1b[1;{modnum}Q").into_bytes() } else { b"\x1bOQ".to_vec() },
        Key::F3 => if modnum > 1 { format!("\x1b[1;{modnum}R").into_bytes() } else { b"\x1bOR".to_vec() },
        Key::F4 => if modnum > 1 { format!("\x1b[1;{modnum}S").into_bytes() } else { b"\x1bOS".to_vec() },
        Key::F5 => tilde(15),
        Key::F6 => tilde(17),
        Key::F7 => tilde(18),
        Key::F8 => tilde(19),
        Key::F9 => tilde(20),
        Key::F10 => tilde(21),
        Key::F11 => tilde(23),
        Key::F12 => tilde(24),
        Key::Space if m.ctrl => vec![0],
        _ if m.ctrl && !m.alt => {
            let name = key.name();
            let b = name.as_bytes();
            if b.len() == 1 && b[0].is_ascii_alphabetic() {
                vec![b[0].to_ascii_lowercase() - b'a' + 1]
            } else {
                match key {
                    Key::OpenBracket => vec![0x1b],
                    Key::Backslash => vec![0x1c],
                    Key::CloseBracket => vec![0x1d],
                    Key::Num6 if m.shift => vec![0x1e],
                    Key::Minus if m.shift => vec![0x1f],
                    Key::Slash => vec![0x1f],
                    _ => return None,
                }
            }
        }
        _ if m.alt && !m.ctrl => {
            // Alt+letter / digit: ESC prefix (egui sends no text event for it).
            let name = key.name();
            let b = name.as_bytes();
            if b.len() == 1 && b[0].is_ascii_alphanumeric() {
                let c = if m.shift { b[0].to_ascii_uppercase() } else { b[0].to_ascii_lowercase() };
                vec![0x1b, c]
            } else {
                return None;
            }
        }
        _ => return None,
    })
}
