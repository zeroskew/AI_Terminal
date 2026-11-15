//! Reassignable keyboard shortcuts (Options > Shortcuts). Each command has a default; the user
//! can record another combination or unbind it. Bindings are saved as text ("Ctrl+Shift+N").

use egui::{Key, KeyboardShortcut, Modifiers};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Cmd {
    NewClaude,
    NewClaudeUnrestricted,
    NewDefault,
    CloseSession,
    RenameSession,
    NextSession,
    PrevSession,
    NextProject,
    PrevProject,
    FontBigger,
    FontSmaller,
    FontReset,
    TogglePreviews,
    Options,
    Plugins,
    Find,
    Snippets,
}

pub const ALL: [Cmd; 17] = [
    Cmd::Find,
    Cmd::Snippets,
    Cmd::NewClaude,
    Cmd::NewClaudeUnrestricted,
    Cmd::NewDefault,
    Cmd::CloseSession,
    Cmd::RenameSession,
    Cmd::NextSession,
    Cmd::PrevSession,
    Cmd::NextProject,
    Cmd::PrevProject,
    Cmd::FontBigger,
    Cmd::FontSmaller,
    Cmd::FontReset,
    Cmd::TogglePreviews,
    Cmd::Options,
    Cmd::Plugins,
];

impl Cmd {
    /// Saved-settings id.
    pub fn id(self) -> &'static str {
        match self {
            Cmd::NewClaude => "claude.session.new",
            Cmd::NewClaudeUnrestricted => "claude.session.new-unrestricted",
            Cmd::NewDefault => "session.new-default",
            Cmd::CloseSession => "session.close",
            Cmd::RenameSession => "session.rename",
            Cmd::NextSession => "session.next",
            Cmd::PrevSession => "session.previous",
            Cmd::NextProject => "project.next",
            Cmd::PrevProject => "project.previous",
            Cmd::FontBigger => "font.bigger",
            Cmd::FontSmaller => "font.smaller",
            Cmd::FontReset => "font.reset",
            Cmd::TogglePreviews => "images.toggle-previews",
            Cmd::Options => "app.options",
            Cmd::Plugins => "app.plugins",
            Cmd::Find => "terminal.find",
            Cmd::Snippets => "snippets.palette",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Cmd::NewClaude => "New Claude session",
            Cmd::NewClaudeUnrestricted => "New Claude session (unrestricted)",
            Cmd::NewDefault => "New session (default type)",
            Cmd::CloseSession => "Close session",
            Cmd::RenameSession => "Rename session",
            Cmd::NextSession => "Next session in the project",
            Cmd::PrevSession => "Previous session in the project",
            Cmd::NextProject => "Next project tab",
            Cmd::PrevProject => "Previous project tab",
            Cmd::FontBigger => "Terminal font bigger",
            Cmd::FontSmaller => "Terminal font smaller",
            Cmd::FontReset => "Terminal font default size",
            Cmd::TogglePreviews => "Inline image previews on / off",
            Cmd::Options => "Options",
            Cmd::Plugins => "Claude Code plugins",
            Cmd::Find => "Find in terminal",
            Cmd::Snippets => "Insert a snippet",
        }
    }

    pub fn default_binding(self) -> Option<KeyboardShortcut> {
        let c = Modifiers::CTRL;
        let cs = Modifiers::CTRL | Modifiers::SHIFT;
        Some(match self {
            Cmd::NewClaude => KeyboardShortcut::new(c, Key::N),
            Cmd::NewClaudeUnrestricted => KeyboardShortcut::new(cs, Key::N),
            Cmd::NewDefault => KeyboardShortcut::new(cs, Key::T),
            Cmd::CloseSession => KeyboardShortcut::new(cs, Key::W),
            Cmd::RenameSession => KeyboardShortcut::new(Modifiers::NONE, Key::F2),
            Cmd::NextSession => KeyboardShortcut::new(c, Key::Tab),
            Cmd::PrevSession => KeyboardShortcut::new(cs, Key::Tab),
            Cmd::NextProject => KeyboardShortcut::new(c, Key::PageDown),
            Cmd::PrevProject => KeyboardShortcut::new(c, Key::PageUp),
            Cmd::FontBigger => KeyboardShortcut::new(c, Key::Equals),
            Cmd::FontSmaller => KeyboardShortcut::new(c, Key::Minus),
            Cmd::FontReset => KeyboardShortcut::new(c, Key::Num0),
            Cmd::TogglePreviews => KeyboardShortcut::new(cs, Key::I),
            Cmd::Options => KeyboardShortcut::new(c, Key::Comma),
            Cmd::Plugins => return None,
            Cmd::Find => KeyboardShortcut::new(cs, Key::F),
            Cmd::Snippets => KeyboardShortcut::new(cs, Key::S),
        })
    }
}

/// "Ctrl+Shift+N" (key part is egui's key name).
pub fn to_text(sc: &KeyboardShortcut) -> String {
    let m = sc.modifiers;
    let mut s = String::new();
    if m.ctrl || m.command {
        s.push_str("Ctrl+");
    }
    if m.alt {
        s.push_str("Alt+");
    }
    if m.shift {
        s.push_str("Shift+");
    }
    s.push_str(sc.logical_key.name());
    s
}

/// For display: symbols where egui has them ("Ctrl+=", "Ctrl+,").
pub fn display(sc: &KeyboardShortcut) -> String {
    let t = to_text(sc);
    let name = sc.logical_key.name();
    let sym = sc.logical_key.symbol_or_name();
    match t.strip_suffix(name) {
        Some(prefix) => format!("{prefix}{sym}"),
        None => t,
    }
}

pub fn parse(text: &str) -> Option<KeyboardShortcut> {
    let parts: Vec<&str> = text.split('+').map(str::trim).collect();
    let (key, mods) = parts.split_last()?;
    let mut m = Modifiers::NONE;
    for p in mods {
        match p.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => m = m | Modifiers::CTRL,
            "shift" => m = m | Modifiers::SHIFT,
            "alt" => m = m | Modifiers::ALT,
            _ => return None,
        }
    }
    Some(KeyboardShortcut::new(m, Key::from_name(key)?))
}

/// The binding in effect: the user's (empty text = unbound) or the default.
pub fn binding(overrides: &std::collections::HashMap<String, String>, cmd: Cmd) -> Option<KeyboardShortcut> {
    match overrides.get(cmd.id()) {
        Some(t) if t.is_empty() => None,
        Some(t) => parse(t).or_else(|| cmd.default_binding()),
        None => cmd.default_binding(),
    }
}

/// Is `sc` pressed this frame with exactly its modifiers? (So Ctrl+N doesn't also fire for
/// Ctrl+Shift+N.) Consumes the key event when it matches.
pub fn consume_exact(ctx: &egui::Context, sc: &KeyboardShortcut) -> bool {
    ctx.input_mut(|i| {
        let pos = i.events.iter().position(|e| matches!(e, egui::Event::Key { key, pressed: true, modifiers, .. } if *key == sc.logical_key && same_mods(*modifiers, sc.modifiers)));
        match pos {
            Some(p) => {
                i.events.remove(p);
                true
            }
            None => false,
        }
    })
}

pub fn same_mods(a: Modifiers, b: Modifiers) -> bool {
    (a.ctrl || a.command) == (b.ctrl || b.command) && a.shift == b.shift && a.alt == b.alt
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_round_trip() {
        for cmd in ALL {
            if let Some(sc) = cmd.default_binding() {
                assert_eq!(parse(&to_text(&sc)), Some(sc), "{cmd:?} {}", to_text(&sc));
            }
        }
        assert_eq!(parse("Ctrl+Shift+N"), Some(KeyboardShortcut::new(Modifiers::CTRL | Modifiers::SHIFT, Key::N)));
        assert_eq!(parse("Bogus+N"), None);
    }

    #[test]
    fn overrides() {
        let mut o = std::collections::HashMap::new();
        assert_eq!(binding(&o, Cmd::NewClaude), Cmd::NewClaude.default_binding());
        o.insert(Cmd::NewClaude.id().to_string(), String::new());
        assert_eq!(binding(&o, Cmd::NewClaude), None);
        o.insert(Cmd::NewClaude.id().to_string(), "Alt+F1".into());
        assert_eq!(binding(&o, Cmd::NewClaude), Some(KeyboardShortcut::new(Modifiers::ALT, Key::F1)));
    }
}
