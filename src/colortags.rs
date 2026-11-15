//! Colour tags: Claude Code's markdown has no colour, so AI Terminal renders inline tags the
//! agent writes:
//!
//! - `{red}Failed{/}`, `{green}OK{/green}`, `{#ff8800}warm{/}`: text colour,
//! - `{bg:green}added{/}`, `{bg:#203a20}…{/}`: background behind the text,
//! - `{row:green}+ line{/}`: background across the whole terminal row (diff-style lines).
//!
//! Tags nest (`{bg:red}{white}x{/}{/}`) and only count when they open and close on the same row,
//! so code such as `f"{red}"` is left alone. The tag characters disappear; the columns they took
//! are added back as blanks just before the next table border (│) or at the end of the row, so
//! tables the agent laid out with the tags included stay aligned.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TagColor {
    /// Index into the theme's 16 ANSI colours.
    Ansi(usize),
    Rgb([u8; 3]),
}

const NAMED_RGB: [[u8; 3]; 3] = [[0xf0, 0xa0, 0x3c], [0xbe, 0x91, 0xff], [0xff, 0x79, 0xc6]];

/// Background shades for named colours, like Claude Code diffs: a dark
/// tint for whole rows (`{row:red}` = removed line `#3d0100`, `{row:green}` = added line
/// `#022800`) and a brighter one for spans (`{bg:red}` = changed characters `#5c0200`,
/// `{bg:green}` `#044700`). `#rrggbb` backgrounds are used as given (None here).
pub fn bg_shade(c: TagColor, row: bool) -> Option<[u8; 3]> {
    let (r, s): ([u8; 3], [u8; 3]) = match c {
        TagColor::Ansi(0) => ([0x0c, 0x0c, 0x0e], [0x00, 0x00, 0x00]),
        TagColor::Ansi(9) | TagColor::Ansi(1) => ([0x3d, 0x01, 0x00], [0x5c, 0x02, 0x00]),
        TagColor::Ansi(10) | TagColor::Ansi(2) => ([0x02, 0x28, 0x00], [0x04, 0x47, 0x00]),
        TagColor::Ansi(11) | TagColor::Ansi(3) => ([0x38, 0x2e, 0x00], [0x55, 0x46, 0x00]),
        TagColor::Ansi(12) | TagColor::Ansi(4) => ([0x05, 0x22, 0x44], [0x0a, 0x36, 0x66]),
        TagColor::Ansi(13) | TagColor::Ansi(5) => ([0x36, 0x08, 0x36], [0x52, 0x10, 0x52]),
        TagColor::Ansi(14) | TagColor::Ansi(6) => ([0x00, 0x30, 0x34], [0x00, 0x4a, 0x50]),
        TagColor::Ansi(15) | TagColor::Ansi(7) => ([0x38, 0x3a, 0x3f], [0x50, 0x53, 0x5a]),
        TagColor::Ansi(8) => ([0x2a, 0x2d, 0x33], [0x3a, 0x3e, 0x46]),
        TagColor::Rgb(v) if v == NAMED_RGB[0] => ([0x3d, 0x20, 0x00], [0x5c, 0x30, 0x00]),
        TagColor::Rgb(v) if v == NAMED_RGB[1] => ([0x26, 0x16, 0x46], [0x3a, 0x24, 0x66]),
        TagColor::Rgb(v) if v == NAMED_RGB[2] => ([0x3d, 0x08, 0x26], [0x5c, 0x10, 0x38]),
        _ => return None,
    };
    Some(if row { r } else { s })
}

/// Style of one character.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Style {
    pub fg: Option<TagColor>,
    pub bg: Option<TagColor>,
}

/// What goes in an output column.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Out {
    /// Input column `idx`, with the style of the tags around it.
    Src(usize, Style),
    /// Padding where tag characters were.
    Blank,
}

/// A row after tag processing: the columns, plus a whole-row background from `{row:…}`.
pub struct Layout {
    pub cols: Vec<Out>,
    pub row_bg: Option<TagColor>,
}

pub fn named(name: &str) -> Option<TagColor> {
    Some(match name {
        "black" => TagColor::Ansi(0),
        "red" => TagColor::Ansi(9),
        "green" => TagColor::Ansi(10),
        "yellow" => TagColor::Ansi(11),
        "blue" => TagColor::Ansi(12),
        "magenta" => TagColor::Ansi(13),
        "cyan" => TagColor::Ansi(14),
        "white" => TagColor::Ansi(15),
        "gray" | "grey" => TagColor::Ansi(8),
        "orange" => TagColor::Rgb(NAMED_RGB[0]),
        "purple" => TagColor::Rgb(NAMED_RGB[1]),
        "pink" => TagColor::Rgb(NAMED_RGB[2]),
        _ => {
            let hex = name.strip_prefix('#')?;
            if hex.len() != 6 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
                return None;
            }
            let v = u32::from_str_radix(hex, 16).ok()?;
            TagColor::Rgb([(v >> 16) as u8, (v >> 8) as u8, v as u8])
        }
    })
}

/// The instruction sessions get through `--append-system-prompt`.
pub const PROMPT: &str = "Your replies are shown in AI Terminal, which renders colour tags in your text (Claude Code's markdown itself has no colour). Text colour: {red}...{/}, {green}...{/}, {yellow}...{/}, {blue}...{/}, {magenta}...{/}, {cyan}...{/}, {orange}...{/}, {purple}...{/}, {pink}...{/}, {gray}...{/}, {white}...{/} or {#rrggbb}...{/}. Background behind text: {bg:green}...{/} (any of those names, or {bg:#rrggbb}). Background across the whole line, e.g. for diff-style lines: {row:green}+ added line{/} and {row:red}- removed line{/}; inside such a line, mark the characters that actually changed with {bg:green}...{/} / {bg:red}...{/} (a brighter shade), like Claude Code's own diff view. Tags nest ({bg:red}{white}text{/}{/}), {/} closes the innermost one, and every tag must close on the same line. Use colour when it helps the reader (status values, pass/fail, warnings, table cells, diffs you show in your reply), not everywhere. Tags do not work inside code blocks or inline code (they are shown literally there), so to show coloured code or a coloured diff, write the lines as plain text (not in a code block). Never put colour tags in commands, file contents or anything you write to disk.";

#[derive(Clone, Copy)]
enum Tok {
    Fg(TagColor),
    Bg(TagColor),
    Row(TagColor),
    Close,
}

/// Finds `{name}`, `{bg:name}`, `{row:name}`, `{/}`, `{/name}` at `i`; returns token + length.
fn token_at(cells: &[char], i: usize) -> Option<(Tok, usize)> {
    if cells[i] != '{' {
        return None;
    }
    let mut j = i + 1;
    let mut name = String::new();
    while j < cells.len() && j - i <= 14 && cells[j] != '}' {
        name.push(cells[j]);
        j += 1;
    }
    if j >= cells.len() || cells[j] != '}' {
        return None;
    }
    let len = j - i + 1;
    if let Some(n) = name.strip_prefix('/') {
        let n = n.strip_prefix("bg:").or_else(|| n.strip_prefix("row:")).unwrap_or(n);
        return (n.is_empty() || n == "bg" || n == "row" || named(n).is_some()).then_some((Tok::Close, len));
    }
    if let Some(n) = name.strip_prefix("bg:") {
        return named(n).map(|c| (Tok::Bg(c), len));
    }
    if let Some(n) = name.strip_prefix("row:") {
        return named(n).map(|c| (Tok::Row(c), len));
    }
    named(&name).map(|c| (Tok::Fg(c), len))
}

fn is_border(c: char) -> bool {
    matches!(c, '│' | '┃' | '║' | '|' | '┆' | '┊')
}

/// Maps a row of cells to output columns (same length). None when the row has no paired tags.
pub fn layout(cells: &[char]) -> Option<Layout> {
    if !cells.contains(&'{') {
        return None;
    }
    let mut toks: Vec<(usize, usize, Tok)> = Vec::new();
    let mut i = 0;
    while i < cells.len() {
        if let Some((t, len)) = token_at(cells, i) {
            toks.push((i, len, t));
            i += len;
        } else {
            i += 1;
        }
    }
    // Pair openers with closers (a stack, so tags nest).
    let mut stack: Vec<usize> = Vec::new();
    let mut pairs: Vec<(usize, usize)> = Vec::new();
    for (k, (_, _, t)) in toks.iter().enumerate() {
        match t {
            Tok::Close => {
                if let Some(o) = stack.pop() {
                    pairs.push((o, k));
                }
            }
            _ => stack.push(k),
        }
    }
    if pairs.is_empty() {
        return None;
    }
    let mut hidden = vec![false; cells.len()];
    let mut style = vec![Style::default(); cells.len()];
    let mut row_bg = None;
    // Outer pairs first, so inner tags override.
    pairs.sort_by_key(|&(o, c)| std::cmp::Reverse(toks[c].0 - toks[o].0));
    for &(o, c) in &pairs {
        let (os, ol, ot) = toks[o];
        let (cs, cl, _) = toks[c];
        hidden[os..os + ol].iter_mut().for_each(|h| *h = true);
        hidden[cs..cs + cl].iter_mut().for_each(|h| *h = true);
        match ot {
            Tok::Fg(col) => style.iter_mut().take(cs).skip(os + ol).for_each(|s| s.fg = Some(col)),
            Tok::Bg(col) => style.iter_mut().take(cs).skip(os + ol).for_each(|s| s.bg = Some(col)),
            Tok::Row(col) => row_bg = Some(col),
            Tok::Close => {}
        }
    }
    let mut cols = Vec::with_capacity(cells.len());
    let mut pending = 0;
    for (i, &c) in cells.iter().enumerate() {
        if hidden[i] {
            pending += 1;
            continue;
        }
        if is_border(c) {
            cols.extend(std::iter::repeat_n(Out::Blank, pending));
            pending = 0;
        }
        cols.push(Out::Src(i, style[i]));
    }
    cols.extend(std::iter::repeat_n(Out::Blank, pending));
    Some(Layout { cols, row_bg })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(s: &str) -> (String, Vec<Style>, Option<TagColor>) {
        let cells: Vec<char> = s.chars().collect();
        let (cols, row) = match layout(&cells) {
            Some(l) => (l.cols, l.row_bg),
            None => ((0..cells.len()).map(|i| Out::Src(i, Style::default())).collect(), None),
        };
        let text = cols.iter().map(|o| match o { Out::Src(i, _) => cells[*i], Out::Blank => ' ' }).collect();
        let st = cols.iter().map(|o| match o { Out::Src(_, s) => *s, Out::Blank => Style::default() }).collect();
        (text, st, row)
    }

    #[test]
    fn colours_and_keeps_table_borders() {
        let src = "│ {green}OK{/}     │ next";
        let (t, s, _) = render(src);
        assert_eq!(t, "│ OK               │ next");
        assert_eq!(t.chars().count(), src.chars().count());
        assert_eq!(s[2].fg, Some(TagColor::Ansi(10)));
        assert_eq!(s[3].fg, Some(TagColor::Ansi(10)));
        assert_eq!(s[4].fg, None);
    }

    #[test]
    fn named_close_hex_and_nesting() {
        let (t, s, _) = render("{red}a{#00ff00}b{/}c{/red}");
        assert_eq!(t.trim_end(), "abc");
        assert_eq!(s[0].fg, Some(TagColor::Ansi(9)));
        assert_eq!(s[1].fg, Some(TagColor::Rgb([0, 255, 0])));
        assert_eq!(s[2].fg, Some(TagColor::Ansi(9)));
    }

    #[test]
    fn backgrounds_and_rows() {
        let (t, s, row) = render("x {bg:red}{white}err{/}{/} y");
        assert_eq!(t.trim_end(), "x err y");
        assert_eq!(s[2], Style { fg: Some(TagColor::Ansi(15)), bg: Some(TagColor::Ansi(9)) });
        assert_eq!(s[6], Style::default());
        assert_eq!(row, None);
        let (t, _, row) = render("{row:green}2808 + let a = 1;{/}");
        assert_eq!(t.trim_end(), "2808 + let a = 1;");
        assert_eq!(row, Some(TagColor::Ansi(10)));
        let (_, s, _) = render("{bg:#203a20}a{/bg}");
        assert_eq!(s[0].bg, Some(TagColor::Rgb([0x20, 0x3a, 0x20])));
    }

    #[test]
    fn leaves_code_alone() {
        assert!(layout(&"print(f\"{red} {x}\")".chars().collect::<Vec<_>>()).is_none());
        assert!(layout(&"fn main() { let red = 1; }".chars().collect::<Vec<_>>()).is_none());
        assert!(layout(&"{unknown}x{/}".chars().collect::<Vec<_>>()).is_none());
        assert!(layout(&"{bg:nope}x{/}".chars().collect::<Vec<_>>()).is_none());
    }
}
