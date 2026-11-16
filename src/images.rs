//! Pasted images: saving clipboard bitmaps, loading textures in the background, and finding
//! image references (file paths and Claude Code's `[Image #N]` placeholders) in terminal text.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant, SystemTime};

use egui::{ColorImage, TextureHandle, TextureOptions};

pub const IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "gif", "bmp", "webp"];

pub fn is_image_path(p: &Path) -> bool {
    p.extension().and_then(|e| e.to_str()).is_some_and(|e| IMAGE_EXTS.iter().any(|x| e.eq_ignore_ascii_case(x)))
}

/// `%TEMP%\AI_Terminal\paste`, where clipboard images are written.
pub fn paste_dir() -> PathBuf {
    std::env::temp_dir().join("AI_Terminal").join("paste")
}

/// Writes a clipboard bitmap as `paste-<hash>.png`. The same image pasted twice reuses the
/// file.
pub fn save_clipboard_image(img: &arboard::ImageData) -> Result<PathBuf, String> {
    let dir = paste_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("Can't create {}: {e}", dir.display()))?;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (img.width, img.height).hash(&mut h);
    img.bytes.hash(&mut h);
    let path = dir.join(format!("paste-{:016X}.png", h.finish()));
    if path.exists() {
        if let Ok(f) = std::fs::File::options().append(true).open(&path) {
            let _ = f.set_modified(SystemTime::now());
        }
        return Ok(path);
    }
    let buf = image::RgbaImage::from_raw(img.width as u32, img.height as u32, img.bytes.to_vec()).ok_or("Clipboard image has an unexpected size")?;
    buf.save_with_format(&path, image::ImageFormat::Png).map_err(|e| format!("Can't save the image: {e}"))?;
    Ok(path)
}

/// Trims the paste folder (best effort): deletes files older than `keep_days`, then the oldest
/// files until the folder is at most `max_bytes`. 0 means no limit for either. The newest file
/// is always kept (it was probably just pasted).
pub fn cleanup(keep_days: u32, max_bytes: u64) {
    let Ok(rd) = std::fs::read_dir(paste_dir()) else { return };
    let mut files: Vec<(PathBuf, SystemTime, u64)> = rd
        .flatten()
        .filter_map(|e| {
            let m = e.metadata().ok().filter(|m| m.is_file())?;
            Some((e.path(), m.modified().unwrap_or(SystemTime::UNIX_EPOCH), m.len()))
        })
        .collect();
    // Newest first.
    files.sort_by(|a, b| b.1.cmp(&a.1));
    let cutoff = (keep_days > 0).then(|| SystemTime::now() - Duration::from_secs(keep_days as u64 * 24 * 3600));
    let mut total = 0u64;
    let mut over = false;
    for (i, (path, modified, len)) in files.iter().enumerate() {
        let too_old = cutoff.is_some_and(|c| *modified < c);
        // Once the cap is reached, everything older goes too.
        over = over || (max_bytes > 0 && total + len > max_bytes);
        if i > 0 && (too_old || over) {
            let _ = std::fs::remove_file(path);
        } else {
            total += len;
        }
    }
}

/// Runs [`cleanup`] on a background thread.
pub fn cleanup_async(keep_days: u32, max_bytes: u64) {
    std::thread::spawn(move || cleanup(keep_days, max_bytes));
}

/// Quotes a path for typing into the PTY (always quoted).
pub fn quote(p: &Path) -> String {
    format!("\"{}\"", p.display())
}

// ---------------------------------------------------------------------------------------------
// Texture cache
//
// Downscaling quality: a big screenshot shown small through plain bilinear sampling aliases
// badly. So (1) every image texture has mipmaps (trilinear filtering, smooth at any size right
// away), and (2) for what's on screen, `texture_for` resamples the original image to the exact
// pixel size shown with Lanczos3 on a background thread and swaps that sharp copy in when ready.

pub enum Slot {
    Loading,
    Ready { tex: TextureHandle, size: [usize; 2], bytes: u64, src: Arc<image::RgbaImage> },
    Failed(String),
}

type Loaded = Result<(ColorImage, [usize; 2], u64, Arc<image::RgbaImage>), String>;

struct Sized {
    tex: TextureHandle,
    used: u64,
}

pub struct ImageCache {
    slots: HashMap<PathBuf, Slot>,
    tx: Sender<(PathBuf, Loaded)>,
    rx: Receiver<(PathBuf, Loaded)>,
    exists: HashMap<PathBuf, (bool, Instant)>,
    /// Exact-size Lanczos copies: (path, width, height) → texture.
    sized: HashMap<(PathBuf, u32, u32), Sized>,
    sized_tx: Sender<(PathBuf, u32, u32, ColorImage)>,
    sized_rx: Receiver<(PathBuf, u32, u32, ColorImage)>,
    /// Paths with a resample running (one at a time per image).
    resampling: std::collections::HashSet<PathBuf>,
    frame: u64,
}

/// Longest side of the base texture (mipmapped); exact-size copies come from the source.
const MAX_TEX: u32 = 2048;
/// Longest side kept of the source for resampling.
const MAX_SRC: u32 = 4096;
/// Exact-size copies kept (least recently used ones go).
const MAX_SIZED: usize = 48;

fn mip_options() -> TextureOptions {
    TextureOptions { mipmap_mode: Some(egui::TextureFilter::Linear), ..TextureOptions::LINEAR }
}

impl ImageCache {
    pub fn new() -> Self {
        let (tx, rx) = channel();
        let (sized_tx, sized_rx) = channel();
        ImageCache { slots: HashMap::new(), tx, rx, exists: HashMap::new(), sized: HashMap::new(), sized_tx, sized_rx, resampling: Default::default(), frame: 0 }
    }

    /// Cached `is_file`, refreshed every few seconds (it's asked every frame for every
    /// reference on screen).
    pub fn exists(&mut self, p: &Path) -> bool {
        let now = Instant::now();
        if let Some((e, t)) = self.exists.get(p) {
            if now.duration_since(*t) < Duration::from_secs(3) {
                return *e;
            }
        }
        let e = p.is_file();
        self.exists.insert(p.to_path_buf(), (e, now));
        e
    }

    /// Moves finished background work into textures.
    pub fn pump(&mut self, ctx: &egui::Context) {
        self.frame += 1;
        while let Ok((path, res)) = self.rx.try_recv() {
            let slot = match res {
                Ok((img, size, bytes, src)) => Slot::Ready { tex: ctx.load_texture(path.display().to_string(), img, mip_options()), size, bytes, src },
                Err(e) => Slot::Failed(e),
            };
            self.slots.insert(path, slot);
        }
        while let Ok((path, w, h, img)) = self.sized_rx.try_recv() {
            self.resampling.remove(&path);
            let tex = ctx.load_texture(format!("{} {w}x{h}", path.display()), img, TextureOptions::LINEAR);
            self.sized.insert((path, w, h), Sized { tex, used: self.frame });
        }
        if self.sized.len() > MAX_SIZED {
            let mut keys: Vec<_> = self.sized.iter().map(|(k, v)| (v.used, k.clone())).collect();
            keys.sort();
            for (_, k) in keys.into_iter().take(self.sized.len() - MAX_SIZED) {
                self.sized.remove(&k);
            }
        }
    }

    /// The texture for `path`, starting a background load on first use.
    pub fn get(&mut self, ctx: &egui::Context, path: &Path) -> &Slot {
        if !self.slots.contains_key(path) {
            self.slots.insert(path.to_path_buf(), Slot::Loading);
            let tx = self.tx.clone();
            let p = path.to_path_buf();
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                let res = load(&p);
                let _ = tx.send((p, res));
                ctx.request_repaint();
            });
        }
        &self.slots[path]
    }

    /// The best texture to draw `path` at `px` (physical pixels): an exact-size Lanczos copy
    /// when there is one, else the mipmapped texture (and a copy is made in the background).
    pub fn texture_for(&mut self, ctx: &egui::Context, path: &Path, px: egui::Vec2) -> Option<egui::TextureId> {
        let (w, h) = (px.x.round().max(1.0) as u32, px.y.round().max(1.0) as u32);
        let (base, src) = match self.get(ctx, path) {
            Slot::Ready { tex, src, .. } => (tex.id(), src.clone()),
            _ => return None,
        };
        // Shown at (or above) the source size: nothing to resample.
        if w as f32 >= src.width() as f32 * 0.95 || w > MAX_SRC || h > MAX_SRC {
            return Some(base);
        }
        let key = (path.to_path_buf(), w, h);
        if let Some(s) = self.sized.get_mut(&key) {
            s.used = self.frame;
            return Some(s.tex.id());
        }
        if !self.resampling.contains(path) {
            self.resampling.insert(path.to_path_buf());
            let tx = self.sized_tx.clone();
            let ctx = ctx.clone();
            let p = path.to_path_buf();
            std::thread::spawn(move || {
                let out = image::imageops::resize(&*src, w, h, image::imageops::FilterType::Lanczos3);
                let ci = ColorImage::from_rgba_unmultiplied([w as usize, h as usize], out.as_raw());
                let _ = tx.send((p, w, h, ci));
                ctx.request_repaint();
            });
        }
        // Meanwhile the closest copy already made for this image, else the mipmapped one.
        let near = self
            .sized
            .iter()
            .filter(|((p, sw, _), _)| p == path && (*sw as i64 - w as i64).abs() < (w as i64 / 4).max(8))
            .min_by_key(|((_, sw, _), _)| (*sw as i64 - w as i64).abs())
            .map(|(_, s)| s.tex.id());
        Some(near.unwrap_or(base))
    }

    /// Forgets a path so it's reloaded (e.g. the file changed).
    pub fn forget(&mut self, path: &Path) {
        self.slots.remove(path);
        self.exists.remove(path);
        self.sized.retain(|(p, _, _), _| p != path);
    }
}

fn load(p: &Path) -> Loaded {
    let bytes = std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
    let img = image::ImageReader::open(p).map_err(|e| e.to_string())?.with_guessed_format().map_err(|e| e.to_string())?.decode().map_err(|e| e.to_string())?;
    let size = [img.width() as usize, img.height() as usize];
    let lanczos = image::imageops::FilterType::Lanczos3;
    let src = if img.width() > MAX_SRC || img.height() > MAX_SRC { img.resize(MAX_SRC, MAX_SRC, lanczos) } else { img };
    let src = src.to_rgba8();
    let base = if src.width() > MAX_TEX || src.height() > MAX_TEX {
        image::DynamicImage::ImageRgba8(src.clone()).resize(MAX_TEX, MAX_TEX, lanczos).to_rgba8()
    } else {
        src.clone()
    };
    let ci = ColorImage::from_rgba_unmultiplied([base.width() as usize, base.height() as usize], base.as_raw());
    Ok((ci, size, bytes, Arc::new(src)))
}

// ---------------------------------------------------------------------------------------------
// References in terminal text

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum RefKind {
    Path(PathBuf),
    /// Claude Code's `[Image #N]`.
    Placeholder(usize),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    /// Byte range in the searched text.
    pub start: usize,
    pub end: usize,
    pub kind: RefKind,
}

fn re() -> &'static regex::Regex {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    RE.get_or_init(|| {
        let ext = r"\.(?i:png|jpe?g|gif|bmp|webp)";
        regex::Regex::new(&format!(
            concat!(
                r#""([A-Za-z]:[\\/][^"\r\n]*?{ext})""#,
                r#"|'([A-Za-z]:[\\/][^'\r\n]*?{ext})'"#,
                r#"|([A-Za-z]:[\\/][^\s"'<>|*?]*?{ext})(?:[^\w.]|$)"#,
                r#"|\[Image #(\d{{1,4}})\]"#,
            ),
            ext = ext
        ))
        .unwrap()
    })
}

/// Finds image paths (quoted or not) and `[Image #N]` placeholders in `text`.
pub fn find_refs(text: &str) -> Vec<Found> {
    let mut out = Vec::new();
    for c in re().captures_iter(text) {
        if let Some(m) = c.get(1).or_else(|| c.get(2)) {
            // Include the quotes in the highlighted span.
            out.push(Found { start: m.start() - 1, end: m.end() + 1, kind: RefKind::Path(PathBuf::from(m.as_str())) });
        } else if let Some(m) = c.get(3) {
            out.push(Found { start: m.start(), end: m.end(), kind: RefKind::Path(PathBuf::from(m.as_str())) });
        } else if let Some(m) = c.get(4) {
            let whole = c.get(0).unwrap();
            if let Ok(n) = m.as_str().parse() {
                out.push(Found { start: whole.start(), end: whole.end(), kind: RefKind::Placeholder(n) });
            }
        }
    }
    out
}

pub fn human_size(bytes: u64) -> String {
    if bytes >= 1 << 20 {
        format!("{:.1} MB", bytes as f64 / (1u64 << 20) as f64)
    } else {
        format!("{:.0} KB", (bytes as f64 / 1024.0).max(1.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_quoted_temp_paths() {
        let t = r#"> "C:\Users\Me\AppData\Local\Temp\OtherApp\paste\paste-2FDCD05C8532139A.png" what is this"#;
        let f = find_refs(t);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].kind, RefKind::Path(PathBuf::from(r"C:\Users\Me\AppData\Local\Temp\OtherApp\paste\paste-2FDCD05C8532139A.png")));
        assert_eq!(&t[f[0].start..f[0].end], r#""C:\Users\Me\AppData\Local\Temp\OtherApp\paste\paste-2FDCD05C8532139A.png""#);
    }

    #[test]
    fn finds_unquoted_and_spaces_in_quotes() {
        let f = find_refs(r"see C:\tmp\a.PNG, and 'D:\My Pics\b c.jpeg' ok");
        assert_eq!(f.len(), 2);
        assert_eq!(f[0].kind, RefKind::Path(PathBuf::from(r"C:\tmp\a.PNG")));
        assert_eq!(f[1].kind, RefKind::Path(PathBuf::from(r"D:\My Pics\b c.jpeg")));
        assert!(find_refs(r"C:\tmp\a.pngx").is_empty());
        assert!(find_refs(r"C:\tmp\a.txt").is_empty());
    }

    #[test]
    fn finds_placeholders() {
        let f = find_refs("> [Image #1] [Image #12] describe");
        assert_eq!(f.iter().map(|f| f.kind.clone()).collect::<Vec<_>>(), vec![RefKind::Placeholder(1), RefKind::Placeholder(12)]);
        assert_eq!(f[0].start, 2);
        assert_eq!(f[0].end, 12);
    }
}

#[cfg(test)]
mod save_tests {
    use super::*;

    #[test]
    fn clipboard_image_round_trip_and_dedup() {
        let mut px = vec![0u8; 4 * 3 * 2];
        px[0] = 255;
        px[3] = 255;
        let img = arboard::ImageData { width: 3, height: 2, bytes: std::borrow::Cow::Owned(px) };
        let a = save_clipboard_image(&img).unwrap();
        let b = save_clipboard_image(&img).unwrap();
        assert_eq!(a, b, "same image reuses the file");
        assert!(a.file_name().unwrap().to_string_lossy().starts_with("paste-"));
        let back = image::open(&a).unwrap().to_rgba8();
        assert_eq!((back.width(), back.height()), (3, 2));
        assert_eq!(back.get_pixel(0, 0).0, [255, 0, 0, 255]);
        let _ = std::fs::remove_file(a);
    }
}
