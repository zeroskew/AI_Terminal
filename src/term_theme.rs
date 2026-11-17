//! Terminal colour scheme (Options > Terminal): core colours, the 16 ANSI colours, an optional
//! background image, and built-in presets.

use egui::Color32;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub type Rgb = [u8; 3];

#[derive(Clone, PartialEq, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct TermTheme {
    pub background: Rgb,
    pub foreground: Rgb,
    pub cursor: Rgb,
    pub selection: Rgb,
    /// Black, Red, Green, Yellow, Blue, Magenta, Cyan, White, then the bright eight.
    pub ansi: [Rgb; 16],
    /// Master switch for the background image (keeps the path etc. when off).
    pub bg_image_enabled: bool,
    /// Drawn under the text, laid out per `bg_fit`, at `bg_opacity`.
    pub bg_image: Option<PathBuf>,
    pub bg_opacity: f32,
    pub bg_fit: BgFit,
    /// Gaussian blur of the image, in (logical) pixels; 0 = sharp.
    pub bg_blur: f32,
    /// While the image is shown, use `bg_color` instead of `background` behind it.
    pub bg_color_override: bool,
    pub bg_color: Rgb,
}

/// How the background image is laid out in the terminal pane.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug, Default)]
pub enum BgFit {
    /// Scale to cover the whole pane, cropping the overflow (keeps aspect).
    #[default]
    Fill,
    /// Scale to fit inside the pane, leaving bars (keeps aspect).
    Fit,
    /// Stretch to the pane's exact size (ignores aspect).
    Stretch,
    /// Repeat at actual size from the top-left corner.
    Tile,
    /// Actual size, centred.
    Center,
}

impl BgFit {
    pub const ALL: [BgFit; 5] = [BgFit::Fill, BgFit::Fit, BgFit::Stretch, BgFit::Tile, BgFit::Center];

    pub fn label(self) -> &'static str {
        match self {
            BgFit::Fill => "Fill (crop to cover)",
            BgFit::Fit => "Fit (show whole image)",
            BgFit::Stretch => "Stretch",
            BgFit::Tile => "Tile",
            BgFit::Center => "Center (actual size)",
        }
    }
}

pub const COLOR_NAMES: [&str; 16] = [
    "Black", "Red", "Green", "Yellow", "Blue", "Magenta", "Cyan", "White",
    "Bright Black", "Bright Red", "Bright Green", "Bright Yellow", "Bright Blue", "Bright Magenta", "Bright Cyan", "Bright White",
];

const fn hex(v: u32) -> Rgb {
    [(v >> 16) as u8, (v >> 8) as u8, v as u8]
}

fn scheme(bg: u32, fg: u32, cursor: u32, sel: u32, ansi: [u32; 16]) -> TermTheme {
    TermTheme { background: hex(bg), foreground: hex(fg), cursor: hex(cursor), selection: hex(sel), ansi: ansi.map(hex), bg_image_enabled: true, bg_image: None, bg_opacity: 0.5, bg_fit: BgFit::Fill, bg_blur: 0.0, bg_color_override: false, bg_color: hex(bg) }
}

impl Default for TermTheme {
    fn default() -> Self {
        presets()[0].1.clone()
    }
}

/// Built-in schemes. The first is the default.
pub fn presets() -> Vec<(&'static str, TermTheme)> {
    vec![
        (
            "Default",
            scheme(0x1a1d21, 0xd1d2d3, 0xd1d2d3, 0x1f3a5f, [
                0x000000, 0xcd3131, 0x0dbc79, 0xe5e510, 0x2472c8, 0xbc3fbc, 0x11a8cd, 0xe5e5e5,
                0x666666, 0xf14c4c, 0x23d18b, 0xf5f543, 0x3b8eea, 0xd670d6, 0x29b8db, 0xe5e5e5,
            ]),
        ),
        (
            "Campbell (Windows Terminal)",
            scheme(0x0c0c0c, 0xcccccc, 0xffffff, 0x264f78, [
                0x0c0c0c, 0xc50f1f, 0x13a10e, 0xc19c00, 0x0037da, 0x881798, 0x3a96dd, 0xcccccc,
                0x767676, 0xe74856, 0x16c60c, 0xf9f1a5, 0x3b78ff, 0xb4009e, 0x61d6d6, 0xf2f2f2,
            ]),
        ),
        (
            "One Half Dark",
            scheme(0x282c34, 0xdcdfe4, 0xa3b3cc, 0x3e4451, [
                0x282c34, 0xe06c75, 0x98c379, 0xe5c07b, 0x61afef, 0xc678dd, 0x56b6c2, 0xdcdfe4,
                0x5a6374, 0xe06c75, 0x98c379, 0xe5c07b, 0x61afef, 0xc678dd, 0x56b6c2, 0xdcdfe4,
            ]),
        ),
        (
            "Dracula",
            scheme(0x282a36, 0xf8f8f2, 0xf8f8f2, 0x44475a, [
                0x21222c, 0xff5555, 0x50fa7b, 0xf1fa8c, 0xbd93f9, 0xff79c6, 0x8be9fd, 0xf8f8f2,
                0x6272a4, 0xff6e6e, 0x69ff94, 0xffffa5, 0xd6acff, 0xff92df, 0xa4ffff, 0xffffff,
            ]),
        ),
        (
            "Solarized Dark",
            scheme(0x002b36, 0x839496, 0x93a1a1, 0x073642, [
                0x073642, 0xdc322f, 0x859900, 0xb58900, 0x268bd2, 0xd33682, 0x2aa198, 0xeee8d5,
                0x586e75, 0xcb4b16, 0x586e75, 0x657b83, 0x839496, 0x6c71c4, 0x93a1a1, 0xfdf6e3,
            ]),
        ),
        (
            "Tango Dark",
            scheme(0x000000, 0xd3d7cf, 0xffffff, 0x2e3436, [
                0x000000, 0xcc0000, 0x4e9a06, 0xc4a000, 0x3465a4, 0x75507b, 0x06989a, 0xd3d7cf,
                0x555753, 0xef2929, 0x8ae234, 0xfce94f, 0x729fcf, 0xad7fa8, 0x34e2e2, 0xeeeeec,
            ]),
        ),
        (
            "Monokai",
            scheme(0x272822, 0xf8f8f2, 0xf8f8f0, 0x49483e, [
                0x272822, 0xf92672, 0xa6e22e, 0xf4bf75, 0x66d9ef, 0xae81ff, 0xa1efe4, 0xf8f8f2,
                0x75715e, 0xf92672, 0xa6e22e, 0xf4bf75, 0x66d9ef, 0xae81ff, 0xa1efe4, 0xf9f8f5,
            ]),
        ),
        (
            "Light (One Half Light)",
            scheme(0xfafafa, 0x383a42, 0x4f525d, 0xc5d5f0, [
                0x383a42, 0xe45649, 0x50a14f, 0xc18301, 0x0184bc, 0xa626a4, 0x0997b3, 0xfafafa,
                0x4f525d, 0xdf6c75, 0x98c379, 0xe4c07a, 0x61afef, 0xc577dd, 0x56b5c1, 0xffffff,
            ]),
        ),
    ]
}

pub fn c(v: Rgb) -> Color32 {
    Color32::from_rgb(v[0], v[1], v[2])
}

pub fn to_hex(v: Rgb) -> String {
    format!("#{:02x}{:02x}{:02x}", v[0], v[1], v[2])
}

pub fn parse_hex(s: &str) -> Option<Rgb> {
    let s = s.trim().trim_start_matches('#');
    if s.len() != 6 || !s.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let v = u32::from_str_radix(s, 16).ok()?;
    Some(hex(v))
}

/// The theme as egui colours, built once per frame for the renderer.
pub struct Palette {
    pub bg: Color32,
    pub fg: Color32,
    pub cursor: Color32,
    pub selection: Color32,
    pub ansi: [Color32; 16],
}

impl TermTheme {
    pub fn palette(&self) -> Palette {
        Palette { bg: c(self.effective_background()), fg: c(self.foreground), cursor: c(self.cursor), selection: c(self.selection), ansi: self.ansi.map(c) }
    }

    /// The image to draw, if the image is switched on and set.
    pub fn active_bg_image(&self) -> Option<&PathBuf> {
        self.bg_image.as_ref().filter(|_| self.bg_image_enabled)
    }

    /// The terminal background: the image's override colour while an image is active, else `background`.
    pub fn effective_background(&self) -> Rgb {
        if self.bg_color_override && self.active_bg_image().is_some() { self.bg_color } else { self.background }
    }

    /// Copies the background-image settings (image, opacity, layout) from `other`.
    pub fn keep_background_of(&mut self, other: &TermTheme) {
        self.bg_image = other.bg_image.clone();
        self.bg_opacity = other.bg_opacity;
        self.bg_fit = other.bg_fit;
        self.bg_blur = other.bg_blur;
        self.bg_image_enabled = other.bg_image_enabled;
        self.bg_color_override = other.bg_color_override;
        self.bg_color = other.bg_color;
    }

    pub fn same_colors(&self, other: &TermTheme) -> bool {
        self.background == other.background && self.foreground == other.foreground && self.cursor == other.cursor && self.selection == other.selection && self.ansi == other.ansi
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trip() {
        assert_eq!(parse_hex("#1a1d21"), Some([0x1a, 0x1d, 0x21]));
        assert_eq!(parse_hex("E5E5E5"), Some([0xe5, 0xe5, 0xe5]));
        assert_eq!(parse_hex("#12345"), None);
        assert_eq!(to_hex([0x1f, 0x3a, 0x5f]), "#1f3a5f");
        assert_eq!(presets().len(), 8);
    }
}
