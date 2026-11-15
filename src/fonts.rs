//! Installed-font lookup for the terminal's font-family list.
//!
//! The setting is a CSS-style font-family list (`'MesloLGS NF', 'JetBrainsMono Nerd Font', …,
//! monospace`), as in a browser terminal; here every installed font of the list joins a fallback chain (the first draws the text,
//! the others supply glyphs it lacks, e.g. Nerd Font icons). Fonts are found by the family name
//! stored in the font file (its `name` table), since file names vary between distributions.

use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The default terminal font list.
pub const DEFAULT_LIST: &str = "'MesloLGS NF', 'JetBrainsMono Nerd Font', 'FiraCode Nerd Font', 'Hack Nerd Font', 'Cascadia Code PL', 'Cascadia Code', 'Consolas', Menlo, 'Courier New', monospace";

#[derive(Clone, Debug)]
pub struct Face {
    pub path: PathBuf,
    /// Index inside a .ttc collection.
    pub index: u32,
    pub family: String,
    pub subfamily: String,
    pub mono: bool,
}

impl Face {
    fn is_regular(&self) -> bool {
        matches!(self.subfamily.to_lowercase().as_str(), "regular" | "book" | "normal" | "roman" | "retina")
    }
    fn is_bold(&self) -> bool {
        self.subfamily.eq_ignore_ascii_case("bold")
    }
}

/// Splits a CSS font-family list: `'A B', C, "D"` → [A B, C, D].
pub fn parse_list(s: &str) -> Vec<String> {
    s.split(',').map(|p| p.trim().trim_matches(|c| c == '\'' || c == '"').trim().to_string()).filter(|p| !p.is_empty()).collect()
}

fn font_dirs() -> Vec<PathBuf> {
    let mut v = vec![std::env::var_os("WINDIR").map(PathBuf::from).unwrap_or_else(|| r"C:\Windows".into()).join("Fonts")];
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        v.push(PathBuf::from(local).join("Microsoft").join("Windows").join("Fonts"));
    }
    v
}

fn be16(b: &[u8], o: usize) -> Option<u16> {
    b.get(o..o + 2).map(|x| u16::from_be_bytes([x[0], x[1]]))
}
fn be32(b: &[u8], o: usize) -> Option<u32> {
    b.get(o..o + 4).map(|x| u32::from_be_bytes([x[0], x[1], x[2], x[3]]))
}

fn read_at(f: &mut std::fs::File, off: u64, len: usize) -> Option<Vec<u8>> {
    f.seek(SeekFrom::Start(off)).ok()?;
    let mut buf = vec![0u8; len];
    f.read_exact(&mut buf).ok()?;
    Some(buf)
}

/// Reads family / subfamily (preferring the typographic names 16/17) and the fixed-pitch flag
/// of the font at `offset` (the table directory of one face).
fn read_face(f: &mut std::fs::File, offset: u64) -> Option<(String, String, bool)> {
    let head = read_at(f, offset, 12)?;
    let n = be16(&head, 4)? as usize;
    let dir = read_at(f, offset + 12, n * 16)?;
    let mut name_tab = None;
    let mut post_tab = None;
    for i in 0..n {
        let tag = &dir[i * 16..i * 16 + 4];
        let off = be32(&dir, i * 16 + 8)? as u64;
        let len = be32(&dir, i * 16 + 12)? as usize;
        match tag {
            b"name" => name_tab = Some((off, len)),
            b"post" => post_tab = Some((off, len)),
            _ => {}
        }
    }
    let (noff, nlen) = name_tab?;
    let t = read_at(f, noff, nlen.min(1 << 20))?;
    let count = be16(&t, 2)? as usize;
    let strings = be16(&t, 4)? as usize;
    let mut names: HashMap<u16, (u8, String)> = HashMap::new(); // id → (priority, text)
    for i in 0..count {
        let r = 6 + i * 12;
        let (platform, encoding, lang, id) = (be16(&t, r)?, be16(&t, r + 2)?, be16(&t, r + 4)?, be16(&t, r + 6)?);
        if !matches!(id, 1 | 2 | 16 | 17) {
            continue;
        }
        let (len, off) = (be16(&t, r + 8)? as usize, be16(&t, r + 10)? as usize);
        let Some(raw) = t.get(strings + off..strings + off + len) else { continue };
        let (prio, text) = match (platform, encoding) {
            // Windows Unicode, English (US) first.
            (3, 1) | (3, 10) | (0, _) => {
                let u: Vec<u16> = raw.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
                (if lang == 0x409 || platform == 0 { 3 } else { 2 }, String::from_utf16_lossy(&u))
            }
            (1, 0) => (1, raw.iter().map(|&b| b as char).collect()),
            _ => continue,
        };
        if names.get(&id).is_none_or(|(p, _)| prio > *p) {
            names.insert(id, (prio, text));
        }
    }
    let family = names.get(&16).or(names.get(&1))?.1.trim().to_string();
    let subfamily = names.get(&17).or(names.get(&2)).map(|x| x.1.trim().to_string()).unwrap_or_default();
    let mono = post_tab.and_then(|(o, _)| read_at(f, o + 12, 4)).and_then(|b| be32(&b, 0)).is_some_and(|v| v != 0);
    Some((family, subfamily, mono))
}

fn scan_file(path: &Path) -> Vec<Face> {
    let Ok(mut f) = std::fs::File::open(path) else { return Vec::new() };
    let Some(head) = read_at(&mut f, 0, 12) else { return Vec::new() };
    let mut offsets = Vec::new();
    if &head[0..4] == b"ttcf" {
        let n = be32(&head, 8).unwrap_or(0).min(64) as usize;
        if let Some(b) = read_at(&mut f, 12, n * 4) {
            offsets.extend((0..n).filter_map(|i| be32(&b, i * 4)).map(|o| o as u64));
        }
    } else {
        offsets.push(0);
    }
    offsets
        .into_iter()
        .enumerate()
        .filter_map(|(i, off)| read_face(&mut f, off).map(|(family, subfamily, mono)| Face { path: path.to_path_buf(), index: i as u32, family, subfamily, mono }))
        .collect()
}

/// Every installed font face (scanned once per run, ~100 ms).
pub fn installed() -> &'static [Face] {
    static ALL: OnceLock<Vec<Face>> = OnceLock::new();
    ALL.get_or_init(|| {
        let mut v = Vec::new();
        for d in font_dirs() {
            let Ok(rd) = std::fs::read_dir(d) else { continue };
            for e in rd.flatten() {
                let p = e.path();
                let ext = p.extension().and_then(|x| x.to_str()).map(|x| x.to_ascii_lowercase()).unwrap_or_default();
                if matches!(ext.as_str(), "ttf" | "otf" | "ttc") {
                    v.extend(scan_file(&p));
                }
            }
        }
        v
    })
}

/// The regular (and bold, if any) face of an installed family.
pub fn find(family: &str) -> Option<(Face, Option<Face>)> {
    let faces: Vec<&Face> = installed().iter().filter(|f| f.family.eq_ignore_ascii_case(family)).collect();
    if faces.is_empty() {
        return None;
    }
    let regular = faces.iter().find(|f| f.is_regular()).or(faces.first()).map(|f| (*f).clone())?;
    let bold = faces.iter().find(|f| f.is_bold()).map(|f| (*f).clone());
    Some((regular, bold))
}

/// Installed monospaced families, sorted, for the Options dropdown.
pub fn mono_families() -> Vec<String> {
    let mut v: Vec<String> = installed().iter().filter(|f| f.mono).map(|f| f.family.clone()).collect();
    v.sort_by_key(|s| s.to_lowercase());
    v.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    v
}

/// What the list resolves to: (name, installed?) for each entry. `monospace` is the
/// built-in Hack font.
pub fn resolve(list: &str) -> Vec<(String, bool)> {
    parse_list(list).into_iter().map(|n| {
        let ok = n.eq_ignore_ascii_case("monospace") || find(&n).is_some();
        (n, ok)
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn css_list() {
        assert_eq!(parse_list(DEFAULT_LIST)[0], "MesloLGS NF");
        assert_eq!(parse_list(DEFAULT_LIST).len(), 10);
        assert_eq!(parse_list(r#""A B",  C ,'D'"#), vec!["A B", "C", "D"]);
    }

    #[test]
    fn finds_installed_windows_fonts() {
        // Consolas ships with Windows: found by its internal name, regular + bold.
        let (r, b) = find("Consolas").expect("Consolas installed");
        assert!(r.mono);
        assert!(b.is_some());
    }
}
