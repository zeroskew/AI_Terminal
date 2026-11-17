//! Color emoji. egui draws glyphs from outlines only, so 🟢🔴✅ come out as grey shapes.
//! A browser-based terminal shows them in color, so we do the same here:
//! DirectWrite + Direct2D rasterize the emoji with Segoe UI Emoji's color layers into a
//! bitmap, cached as a texture per (emoji, pixel size, color). The terminal draws that texture
//! in the emoji's cells.

use std::collections::HashMap;

use egui::{ColorImage, TextureHandle, TextureId, TextureOptions};

/// Whether a cell's text should be drawn as a color emoji: anything in the emoji planes
/// (U+1F000…), characters that default to emoji presentation in the BMP (⚪ ✅ ⭐ …), and any
/// character followed by VS16 (U+FE0F). Text symbols Claude Code uses (✻ ✓ ⎿ ●) stay text.
pub fn is_emoji(c: char, extra: Option<&[char]>) -> bool {
    if extra.is_some_and(|z| z.contains(&'\u{FE0F}')) {
        return true;
    }
    let u = c as u32;
    if (0x1F000..=0x1FAFF).contains(&u) {
        // Exclude the few text-style symbols there (playing cards / mahjong are emoji-ish, fine).
        return !matches!(u, 0x1F100..=0x1F1E5);
    }
    matches!(
        u,
        0x231A | 0x231B | 0x23E9..=0x23EC | 0x23F0 | 0x23F3 | 0x25FD | 0x25FE | 0x2614 | 0x2615 | 0x2648..=0x2653 | 0x267F | 0x2693 | 0x26A1 | 0x26AA | 0x26AB
            | 0x26BD | 0x26BE | 0x26C4 | 0x26C5 | 0x26CE | 0x26D4 | 0x26EA | 0x26F2 | 0x26F3 | 0x26F5 | 0x26FA | 0x26FD | 0x2705 | 0x270A | 0x270B | 0x2728 | 0x274C
            | 0x274E | 0x2753..=0x2755 | 0x2757 | 0x2795..=0x2797 | 0x27B0 | 0x27BF | 0x2B1B | 0x2B1C | 0x2B50 | 0x2B55
    )
}

type Key = (String, [u32; 2], [u8; 3]);

thread_local! {
    static CACHE: std::cell::RefCell<HashMap<Key, Option<TextureHandle>>> = std::cell::RefCell::new(HashMap::new());
}

/// The texture for `text` rendered to `size` pixels (None if the system can't render it).
pub fn texture(ctx: &egui::Context, text: &str, size: [u32; 2], fg: egui::Color32) -> Option<TextureId> {
    let key: Key = (text.to_string(), size, [fg.r(), fg.g(), fg.b()]);
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        if c.len() > 4000 {
            c.clear();
        }
        c.entry(key)
            .or_insert_with(|| render(text, size, fg).map(|img| ctx.load_texture(format!("emoji {text} {size:?}"), img, TextureOptions::LINEAR)))
            .as_ref()
            .map(|t| t.id())
    })
}

#[cfg(windows)]
fn render(text: &str, size: [u32; 2], fg: egui::Color32) -> Option<ColorImage> {
    imp::render(text, size, fg).ok()
}

#[cfg(not(windows))]
fn render(_: &str, _: [u32; 2], _: egui::Color32) -> Option<ColorImage> {
    None
}

#[cfg(windows)]
mod imp {
    use windows::core::{w, Result};
    use windows::Win32::Graphics::Direct2D::Common::*;
    use windows::Win32::Graphics::Direct2D::*;
    use windows::Win32::Graphics::DirectWrite::*;
    use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
    use windows::Win32::Graphics::Imaging::*;
    use windows::Win32::System::Com::*;

    struct Factories {
        wic: IWICImagingFactory,
        d2d: ID2D1Factory,
        dw: IDWriteFactory,
    }

    thread_local! {
        static F: Option<Factories> = unsafe { make().ok() };
    }

    unsafe fn make() -> Result<Factories> {
        // The UI thread already has COM (winit / OLE drag and drop); a second init is harmless.
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let wic: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
        let d2d: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
        let dw: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
        Ok(Factories { wic, d2d, dw })
    }

    pub fn render(text: &str, size: [u32; 2], fg: egui::Color32) -> Result<egui::ColorImage> {
        let raw = raster(text, size, fg, false)?;
        let px: Vec<egui::Color32> = raw.chunks_exact(4).map(|p| egui::Color32::from_rgba_premultiplied(p[2], p[1], p[0], p[3])).collect();
        Ok(egui::ColorImage::new([size[0].clamp(4, 512) as usize, size[1].clamp(4, 512) as usize], px))
    }

    /// Bold Segoe UI text, centred, as straight-alpha RGBA (for the taskbar badge).
    pub fn render_text(text: &str, size: [u32; 2], fg: [u8; 3]) -> Result<Vec<u8>> {
        let raw = raster(text, size, egui::Color32::from_rgb(fg[0], fg[1], fg[2]), true)?;
        Ok(raw
            .chunks_exact(4)
            .flat_map(|p| {
                let a = p[3];
                let un = |c: u8| if a == 0 { 0 } else { ((c as u32 * 255) / a as u32).min(255) as u8 };
                [un(p[2]), un(p[1]), un(p[0]), a]
            })
            .collect())
    }

    /// Premultiplied BGRA pixels of `text` drawn into a size[0] × size[1] bitmap.
    fn raster(text: &str, size: [u32; 2], fg: egui::Color32, plain: bool) -> Result<Vec<u8>> {
        F.with(|f| {
            let f = f.as_ref().ok_or_else(windows::core::Error::empty)?;
            unsafe { draw(f, text, size, fg, plain) }
        })
    }

    unsafe fn draw(f: &Factories, text: &str, [w, h]: [u32; 2], fg: egui::Color32, plain: bool) -> Result<Vec<u8>> {
        let (w, h) = (w.clamp(4, 512), h.clamp(4, 512));
        let bitmap = f.wic.CreateBitmap(w, h, &GUID_WICPixelFormat32bppPBGRA, WICBitmapCacheOnLoad)?;
        let props = D2D1_RENDER_TARGET_PROPERTIES {
            r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
            pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
            dpiX: 96.0,
            dpiY: 96.0,
            usage: D2D1_RENDER_TARGET_USAGE_NONE,
            minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
        };
        let rt = f.d2d.CreateWicBitmapRenderTarget(&bitmap, &props)?;
        // Font size: fill the cell height (emoji glyphs sit inside ~1.2 em boxes).
        let em = (h as f32 * 0.78).min(w as f32 * 0.78);
        let format = if plain {
            f.dw.CreateTextFormat(w!("Segoe UI"), None, DWRITE_FONT_WEIGHT_BOLD, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_STRETCH_NORMAL, em * 0.95, w!("en-us"))?
        } else {
            f.dw.CreateTextFormat(w!("Segoe UI Emoji"), None, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_STRETCH_NORMAL, em, w!("en-us"))?
        };
        format.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
        format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
        format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
        let color = D2D1_COLOR_F { r: fg.r() as f32 / 255.0, g: fg.g() as f32 / 255.0, b: fg.b() as f32 / 255.0, a: 1.0 };
        let brush = rt.CreateSolidColorBrush(&color, None)?;
        let utf16: Vec<u16> = text.encode_utf16().collect();
        rt.BeginDraw();
        rt.Clear(Some(&D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.0 }));
        let rect = D2D_RECT_F { left: 0.0, top: 0.0, right: w as f32, bottom: h as f32 };
        rt.DrawText(&utf16, &format, &rect, &brush, D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT, DWRITE_MEASURING_MODE_NATURAL);
        rt.EndDraw(None, None)?;
        let stride = w * 4;
        let mut buf = vec![0u8; (stride * h) as usize];
        bitmap.CopyPixels(std::ptr::null(), stride, &mut buf)?;
        // Premultiplied BGRA → egui's premultiplied RGBA.
        Ok(buf)
    }
}

/// Text rendered with bold Segoe UI as straight RGBA (None off Windows or on failure).
pub fn render_text_rgba(text: &str, size: [u32; 2], fg: [u8; 3], _bold: bool) -> Option<Vec<u8>> {
    #[cfg(windows)]
    {
        imp::render_text(text, size, fg).ok()
    }
    #[cfg(not(windows))]
    {
        let _ = (text, size, fg);
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn which_chars_are_emoji() {
        for c in ['🟢', '🔴', '🟡', '⚪', '⚫', '✅', '❌', '⭐', '🚀'] {
            assert!(is_emoji(c, None), "{c}");
        }
        // Claude Code's own UI symbols stay text.
        for c in ['✻', '✓', '⎿', '●', '❯', '─', '▶'] {
            assert!(!is_emoji(c, None), "{c}");
        }
        assert!(is_emoji('▶', Some(&['\u{FE0F}'])));
    }

    #[test]
    fn renders_in_color() {
        let img = imp::render("🟢", [40, 40], egui::Color32::WHITE).expect("render");
        // A green circle: some opaque pixels with green clearly dominant.
        let green = img.pixels.iter().filter(|p| p.a() > 200 && p.g() as i32 > p.r() as i32 + 60).count();
        assert!(green > 100, "green pixels: {green}");
    }
}
