//! The main window: project tabs across the top, the selected
//! project's sessions on the left, the active terminal with its pasted-image strip, the image
//! viewer and the status bar.

use std::path::{Path, PathBuf};
use std::time::Duration;

use egui::{Align, Color32, Key, KeyboardShortcut, Layout, Modifiers, RichText, Sense, Stroke, Ui, Vec2};
use serde::{Deserialize, Serialize};

use crate::images::{self, ImageCache, Slot};
use crate::agents::{self, Kind, LaunchOpts};
use crate::session::{Profile, Session};
use crate::term_theme;
use crate::theme::*;
use crate::view::{self, Action, ViewOptions, ViewState};
use crate::widgets::{self, Icon, ScrollMoreHint};
use crate::winsys;
use crate::{keys, mcp, plugins};

mod plugins_ui;
mod scripts;
mod agents_ui;
mod snippets;
mod favorites;
mod options;
mod mcp_api;
mod launch;
mod model_switch;
mod session_close;
mod session_move;
mod project_settings;
mod context_info;
mod mcp_servers_ui;
mod sidebar;
mod topbar;
mod pages;
mod status_bar;
mod viewer;
mod search_ui;
mod update_ui;

#[derive(Serialize, Deserialize, Clone)]
pub struct Project {
    pub path: PathBuf,
    pub name: String,
    pub collapsed: bool,
    /// Shown as a tab in the top bar. Home always lists every project; closing a tab only
    /// clears this (the project, its sessions and its settings stay).
    #[serde(default = "yes")]
    pub in_tabs: bool,
    /// Optional accent color (sRGB) that tints the project's tab and its Home card.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accent: Option<[u8; 3]>,
    /// Optional custom image (a copy under %APPDATA%\AI Terminal\project_images) shown in place
    /// of the folder icon on the Home card, the tab and the empty project page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<PathBuf>,
    /// The project's own terminal background, overriding Options > Terminal when enabled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<term_theme::ProjectBg>,
    /// Snippets (by name) in this project's Favourites panel.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fav_snippets: Vec<String>,
    /// Scripts (lowercase file names, like `script_models`) in this project's Favourites panel.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fav_scripts: Vec<String>,
    /// The Favourites panel's order (dragged by the user), as `snippet:<name>` / `script:<key>`.
    /// Favourites missing from it go after the listed ones, sorted by name.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fav_order: Vec<String>,
}

impl Project {
    pub fn accent_color(&self) -> Option<Color32> {
        self.accent.map(widgets::accent_color)
    }
}

fn yes() -> bool {
    true
}

#[derive(Serialize, Deserialize, Clone)]
pub struct SavedSession {
    pub project: PathBuf,
    pub profile: Profile,
    pub name: String,
    /// Unix seconds when the session was first created (for the Today / Yesterday groups).
    #[serde(default)]
    pub created: u64,
    /// The title the program last set (shown while the session isn't running).
    #[serde(default)]
    pub title: Option<String>,
    /// Claude / Codex conversation id, so the session resumes it.
    #[serde(default, alias = "claude_id")]
    pub agent_id: Option<String>,
    /// Launch settings (unrestricted, model, effort, CLI parameters).
    #[serde(default)]
    pub opts: LaunchOpts,
    /// Estimated cost so far (shown while the session isn't running).
    #[serde(default)]
    pub cost: Option<f64>,
    /// Pinned to the top (order among pinned); 0 = not pinned.
    #[serde(default)]
    pub pin: u32,
    /// Moved to the Archived group (hidden from the list until expanded).
    #[serde(default)]
    pub archived: bool,
    /// Set aside in the Saved for later group (hidden until expanded; never auto-archived).
    #[serde(default)]
    pub later: bool,
    /// Unix seconds when it was last used (a line submitted; 0 = unknown: `created` is used).
    /// Orders the sidebar and decides which sessions the "keep the last N" limit archives.
    #[serde(default)]
    pub used: u64,
}

/// A session from a previous run that isn't running: listed in the sidebar, started (resumed)
/// when clicked.
#[derive(Clone)]
struct Dormant {
    id: u64,
    saved: SavedSession,
}

impl Dormant {
    fn display_name(&self) -> String {
        if !self.saved.name.is_empty() {
            return self.saved.name.clone();
        }
        match self.saved.title.as_deref().map(str::trim) {
            Some(t) if !t.is_empty() => crate::session::pretty_title(t, &self.saved.profile.name),
            _ => self.saved.profile.name.clone(),
        }
    }
}

/// One row of the sessions list (running or not).
struct Row {
    id: u64,
    live: bool,
    name: String,
    profile: Profile,
    /// Last used (else created), Unix seconds: the order and the day group.
    used: u64,
    exited: Option<i32>,
    unseen: bool,
    images: usize,
    resumable: bool,
    unrestricted: bool,
    attention: bool,
    /// Claude is working (hooks).
    busy: bool,
    /// Why it needs you (Claude's message), when it does.
    attention_msg: Option<String>,
    pin: u32,
    /// In the Archived group.
    archived: bool,
    /// In the Saved for later group.
    later: bool,
    /// Unix seconds when it was created (0 = unknown).
    created: u64,
    /// Estimated cost so far.
    cost: Option<f64>,
    model: Option<String>,
    effort: Option<String>,
}

impl Row {
    /// Running (a live tab whose process hasn't exited).
    fn is_active(&self) -> bool {
        self.live && self.exited.is_none()
    }
}

/// Percentage points of a plan limit: "0%", "<1%", "7%" (over 100% = more than one window).
fn fmt_plan(p: f64) -> String {
    if p <= 0.0 {
        "0%".into()
    } else if p < 1.0 {
        "<1%".into()
    } else {
        format!("{p:.0}%")
    }
}

/// "New … session (with settings)" / "Restart with settings…" dialog.
struct LaunchDialog {
    project: PathBuf,
    profile: Profile,
    name: String,
    opts: LaunchOpts,
    /// Restart (or resume) this session instead of creating one.
    restart: Option<u64>,
}

/// A launch row that can be the default (what the New-session button runs): "default" at the
/// right of the default row, a "set as default" link there on the others while hovered.
/// Returns (row clicked, "set as default" clicked).
fn menu_item_default(ui: &mut Ui, icon: (Icon, Color32), label: &str, is_default: bool) -> (bool, bool) {
    let resp = menu_row(ui, icon, label, false);
    let r = resp.rect;
    let font = egui::FontId::proportional(12.5);
    let at = egui::pos2(r.right() - 12.0, r.center().y);
    if is_default {
        ui.painter().text(at, egui::Align2::RIGHT_CENTER, "default", font, TEXT_FAINT);
        return (resp.clicked(), false);
    }
    if !resp.contains_pointer() {
        return (resp.clicked(), false);
    }
    let size = ui.painter().layout_no_wrap("set as default".into(), font.clone(), LINK).size();
    let lr = egui::Rect::from_min_size(egui::pos2(at.x - size.x, at.y - size.y / 2.0), size);
    let link = ui.interact(lr.expand(4.0), resp.id.with("set_default"), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
    let col = if link.hovered() { Color32::WHITE } else { LINK };
    ui.painter().text(at, egui::Align2::RIGHT_CENTER, "set as default", font, col);
    if link.hovered() {
        ui.painter().hline(lr.x_range(), lr.bottom(), Stroke::new(1.0, col));
    }
    (resp.clicked(), link.clicked())
}

/// A menu row; `submenu` draws a right arrow (pair it with `egui::containers::menu::SubMenu`).
fn menu_row(ui: &mut Ui, (icon, col): (Icon, Color32), label: &str, submenu: bool) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width().max(300.0), 30.0), Sense::click());
    use egui::containers::menu::{MenuState, SubMenu};
    let open = submenu && MenuState::from_ui(ui, |s, _| s.open_item == Some(SubMenu::id_from_widget_id(resp.id)));
    // contains_pointer: stays lit while the pointer is on the row's "set as default" link.
    let hot = resp.hovered() || open || (!submenu && resp.contains_pointer());
    if hot {
        ui.painter().rect_filled(r, 4.0, CARD_HI);
    }
    widgets::paint_icon(ui.painter(), icon, egui::pos2(r.left() + 16.0, r.center().y), 15.0, col);
    let fg = if hot { Color32::WHITE } else { TEXT };
    ui.painter().text(egui::pos2(r.left() + 36.0, r.center().y), egui::Align2::LEFT_CENTER, label, egui::FontId::proportional(14.0), fg);
    if submenu {
        let a = egui::Rect::from_center_size(egui::pos2(r.right() - 14.0, r.center().y), Vec2::splat(10.0));
        ui.painter().add(egui::containers::menu::SubMenuButton::arrow_shape(a, fg));
    }
    resp
}

/// Each agent's logo (Claude's orange asterisk, Codex's knot, ...), a prompt for shells.
fn agent_icon(p: &Profile) -> (Icon, Color32) {
    match p.kind {
        Kind::Claude => (Icon::Sparkle, Color32::from_rgb(217, 119, 87)),
        Kind::Codex => (Icon::Codex, Color32::from_rgb(200, 204, 212)),
        Kind::Antigravity => (Icon::Antigravity, Color32::from_rgb(110, 155, 245)),
        Kind::Qwen => (Icon::Qwen, Color32::from_rgb(118, 108, 240)),
        Kind::OpenCode => (Icon::OpenCode, Color32::from_rgb(235, 235, 235)),
        Kind::Kimi => (Icon::Kimi, Color32::from_rgb(235, 235, 235)),
        Kind::Shell => (Icon::Prompt, Color32::from_rgb(78, 201, 176)),
    }
}

#[derive(Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub projects: Vec<Project>,
    pub sessions: Vec<SavedSession>,
    pub reopen_sessions: bool,
    /// At start-up, list Claude conversations started elsewhere (the CLI, Claude's desktop app,
    /// other terminals) so they can be resumed here; their folders are added as projects.
    pub import_claude_sessions: bool,
    /// At start-up, resume the session that was selected when the app last closed.
    pub open_last_session: bool,
    /// That session's conversation id (Claude / Codex), kept for `open_last_session`.
    pub last_session: Option<String>,
    pub selected_project: Option<PathBuf>,
    /// The last project tab selected (kept while Home is showing); start-up reopens it.
    pub last_project: Option<PathBuf>,
    pub default_profile: String,
    /// Which row of that profile is the default: "" (plain), "unrestricted" or "settings".
    pub default_variant: String,
    pub font_size: f32,
    /// Zoom of the whole window (1.0 = 100%).
    pub ui_scale: f32,
    /// Whether UI Scale also zooms the terminal text; when off the terminal keeps its own font size.
    pub scale_terminal: bool,
    pub code_font: String,
    pub inline_previews: bool,
    pub preview_rows: usize,
    /// Inline previews: longest side in points, and whether they may cover text.
    pub preview_max_px: f32,
    pub preview_over_text: bool,
    pub shift_enter_alt: bool,
    pub show_image_strip: bool,
    /// A GitHub button in the session header when the project's git remote is on GitHub.
    pub show_github_button: bool,
    /// Paste folder limits: days to keep pasted images and folder size in MB (0 = unlimited).
    pub paste_keep_days: u32,
    pub paste_max_mb: u32,
    pub term_theme: term_theme::TermTheme,
    pub last_claude_opts: LaunchOpts,
    pub last_codex_opts: LaunchOpts,
    /// The other agents' last "with settings" values, by kind.
    pub last_agent_opts: std::collections::HashMap<String, LaunchOpts>,
    pub cli_history: Vec<String>,
    /// Render {red}…{/} color tags in the terminal.
    pub color_tags: bool,
    /// Tell Claude about the tags (`--append-system-prompt`) in sessions started here.
    pub color_tag_prompt: bool,
    /// Ask new agent sessions (Claude, Codex) to name themselves after their first task (rename_session).
    pub name_sessions: bool,
    // Options > General.
    pub launch_on_startup: bool,
    /// Launching the app again brings the running one to the front instead of opening another.
    pub single_instance: bool,
    pub minimize_to_tray: bool,
    /// What closing the window does: "exit", "minimize" (to the taskbar) or "tray" (hide it;
    /// the tray icon brings it back). Quit from the tray always exits.
    pub close_action: String,
    pub notifications: bool,
    pub notify_sound: bool,
    pub flash_taskbar: bool,
    pub badge_count: bool,
    /// Show an indeterminate progress bar on the taskbar button while a session is working.
    pub taskbar_progress: bool,
    pub badge_color: [u8; 3],
    pub badge_text_color: [u8; 3],
    pub debug_logging: bool,
    pub idle_close_minutes: u32,
    pub memory_threshold_gb: f32,
    /// Shortcut overrides: command id → "Ctrl+Shift+N" ("" = unbound). See keys.rs.
    pub keybindings: std::collections::HashMap<String, String>,
    /// Ctrl+click on a file path opens it with: auto / vscode / rider / default / custom.
    pub editor: String,
    /// For "custom": a command using {file}, {line}, {col}.
    pub editor_custom: String,
    pub snippets: Vec<Snippet>,
    /// Height of the sidebar's Favourites panel (drag its top edge).
    pub favorites_h: f32,
    /// Favourites pinned to every project's Favourites panel: snippet names, script keys.
    pub pinned_snippets: Vec<String>,
    pub pinned_scripts: Vec<String>,
    /// Show estimated session cost in the list and header.
    pub show_cost: bool,
    /// What the session usage shows: "plan" (share of the subscription's 5-hour / weekly
    /// limits) or "cost" (estimated API price).
    pub usage_as: String,
    /// Keep this many most recently used sessions per project in the list; older ones (not
    /// pinned, not running) move to Archived. 0 = no limit.
    pub keep_sessions: u32,
    /// Running sessions go to an Active group at the top of the list (below Pinned).
    pub active_on_top: bool,
    /// Open a session in the list with a double click (else a single click; a double click renames).
    pub sessions_double_click: bool,
    /// Run a Favourites snippet or script with a double click (else a single click).
    pub favorites_double_click: bool,
    /// Close a script's panel a few seconds after it finishes successfully.
    pub script_auto_dismiss: bool,
    /// Seconds a finished panel stays before it closes itself (1-10).
    pub script_dismiss_secs: u32,
    /// Start a script's panel minimized to a pill (it opens when the script asks a question).
    pub script_start_minimized: bool,
    /// The AI model per script (key: lowercase file name, e.g. "commit and push.ps1"), passed to it
    /// as AI_TERMINAL_CLI / AI_TERMINAL_MODEL / AI_TERMINAL_EFFORT. No entry = the script's own default.
    pub script_models: std::collections::HashMap<String, ScriptModel>,
    /// Scripts limited to some projects (key: lowercase file name, like `script_models`): listed
    /// only in those projects. No entry = every project.
    pub script_projects: std::collections::HashMap<String, Vec<PathBuf>>,
    /// Script panel width and output height, once resized (None = default, output fits its text).
    pub script_panel_size: Option<[f32; 2]>,
    /// Search window: every session (else the current one), and how many days back (0 = all).
    pub search_all: bool,
    pub search_days: u32,
    /// Searches made from the header's "Search this session" box, newest first.
    pub find_history: Vec<String>,
    /// Check GitHub for a new release now and then, download it, and install it at the next restart.
    pub auto_update: bool,
    /// When automatic checks run: shortly after start-up, and/or twice a day (every 12 hours from
    /// start-up).
    pub update_on_startup: bool,
    pub update_periodic: bool,
    /// Settings format: 4 = current profile names, Claude the default session type. Missing in older
    /// files, so it reads as 0 there (not the struct default).
    #[serde(default)]
    pub version: u32,
}

impl Settings {
    /// The paste folder size cap in bytes (0 = unlimited).
    pub fn paste_limit_bytes(&self) -> u64 {
        self.paste_max_mb as u64 * 1024 * 1024
    }
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            projects: Vec::new(),
            sessions: Vec::new(),
            reopen_sessions: true,
            import_claude_sessions: true,
            open_last_session: true,
            last_session: None,
            selected_project: None,
            last_project: None,
            default_profile: crate::session::CLAUDE.into(),
            default_variant: String::new(),
            font_size: 15.0,
            ui_scale: 1.0,
            scale_terminal: true,
            code_font: crate::fonts::DEFAULT_LIST.into(),
            inline_previews: true,
            preview_rows: 8,
            preview_max_px: 320.0,
            preview_over_text: false,
            shift_enter_alt: true,
            show_image_strip: true,
            show_github_button: true,
            paste_keep_days: 30,
            paste_max_mb: 1024,
            term_theme: Default::default(),
            last_claude_opts: Default::default(),
            last_codex_opts: Default::default(),
            last_agent_opts: Default::default(),
            cli_history: Vec::new(),
            color_tags: true,
            color_tag_prompt: true,
            name_sessions: true,
            launch_on_startup: false,
            single_instance: true,
            minimize_to_tray: false,
            close_action: "exit".into(),
            notifications: true,
            notify_sound: true,
            flash_taskbar: false,
            badge_count: false,
            taskbar_progress: true,
            badge_color: [0xe5, 0x14, 0x14],
            badge_text_color: [0xff, 0xff, 0xff],
            debug_logging: false,
            idle_close_minutes: 0,
            memory_threshold_gb: 0.0,
            keybindings: Default::default(),
            editor: "auto".into(),
            editor_custom: String::new(),
            show_cost: true,
            favorites_h: 160.0,
            pinned_snippets: Vec::new(),
            pinned_scripts: Vec::new(),
            usage_as: "plan".into(),
            keep_sessions: 20,
            active_on_top: true,
            sessions_double_click: false,
            favorites_double_click: false,
            script_auto_dismiss: true,
            script_dismiss_secs: 10,
            script_start_minimized: true,
            script_models: Default::default(),
            script_projects: Default::default(),
            script_panel_size: None,
            search_all: true,
            search_days: 0,
            find_history: Vec::new(),
            auto_update: true,
            update_on_startup: true,
            update_periodic: false,
            snippets: Vec::new(),
            version: 8,
        }
    }
}

struct Tab {
    project: PathBuf,
    created: u64,
    /// Pinned to the top of the list (order among pinned, 1-based); 0 = not pinned.
    pin: u32,
    /// The terminal rang the bell since the last frame.
    bell: bool,
    sess: Session,
    view: ViewState,
}

/// The AI a script should use (Options > Scripts): "claude", "codex" or "antigravity", a model
/// id and an effort level ("" = the model's default).
#[derive(Serialize, Deserialize, Clone, Default, PartialEq)]
#[serde(default)]
pub struct ScriptModel {
    pub cli: String,
    pub model: String,
    pub effort: String,
}

/// A saved prompt / command (Options > Snippets).
#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(default)]
pub struct Snippet {
    pub name: String,
    pub text: String,
    /// Press Enter after inserting.
    pub send: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum OptPage {
    General,
    Appearance,
    Terminal,
    Images,
    Snippets,
    Scripts,
    Agents,
    Shortcuts,
    About,
}

/// Range of Options > Appearance > UI Scale.
const UI_SCALE_MIN: f32 = 0.75;
const UI_SCALE_MAX: f32 = 2.0;

/// Fixed (not reassignable) keys and mouse actions, listed under Options > Shortcuts.
const SHORTCUTS: &[(&str, &str)] = &[
    ("Ctrl+V / Shift+Insert / right-click", "Paste (files → paths, image → saved PNG path, else text)"),
    ("Drag files onto the window", "Paste their paths"),
    ("Ctrl+C", "Copy the selection, else send Ctrl+C"),
    ("Shift+Enter", "Newline in Claude Code"),
    ("Ctrl+1 … Ctrl+9", "Project tab 1 … 9"),
    ("Ctrl + mouse wheel", "Terminal font size"),
    ("Shift+PgUp / PgDn / Home / End", "Scroll back"),
    ("Double / triple click", "Select word / line"),
    ("Click an image or its path", "Open the image viewer"),
];

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum PluginFilter {
    #[default]
    All,
    Bundles,
    Plugins,
    Installed,
}

/// State of the plugins window.
#[derive(Default)]
struct PluginsUi {
    /// The provider whose plugins are shown (Claude or Codex, the vertical tabs).
    agent: Kind,
    /// Load the current provider's plugins on the next frame (set on open / tab switch).
    needs_load: bool,
    /// A Codex load running in the background, and the last load error.
    loading: Option<plugins::LoadRef>,
    load_error: Option<String>,
    catalogue: Vec<plugins::Plugin>,
    installed: std::collections::HashMap<String, Vec<plugins::Installed>>,
    filter: PluginFilter,
    search: String,
    selected: Option<String>,
    /// Install scope: user / project / local.
    scope: String,
    show_log: bool,
    refreshing: bool,
    job: plugins::JobRef,
    /// Sources (marketplaces) and the source filter (None = all sources).
    markets: Vec<plugins::Market>,
    market_filter: Option<String>,
    show_sources: bool,
    new_source: String,
    /// A source whose Remove was clicked once (the second click confirms).
    confirm_remove: Option<String>,
    /// Context token estimates by plugin id (None = no local copy to measure).
    costs: std::collections::HashMap<String, Option<plugins::TokenCost>>,
    /// Claude Code's own figures (`claude plugin details`), fetched for plugins once selected.
    cli_costs: std::collections::HashMap<String, plugins::DetailsRef>,
    /// How to run the claude CLI: (program, leading args).
    cli: (String, Vec<String>),
    /// On / off system wide and per project, by plugin id (read when first shown).
    enablement: std::collections::HashMap<String, plugins::Enablement>,
    /// The app's projects (for the per-project on / off) and the selected one.
    projects: Vec<PathBuf>,
    current_project: PathBuf,
    /// An enable / disable to run in a given project folder (`--scope local` acts on the cwd).
    toggle: Option<(Vec<String>, PathBuf)>,
}

/// BUNDLE badges in the plugins window (the app's blue, a shade deeper than buttons).
const BADGE_BLUE: Color32 = Color32::from_rgb(28, 92, 160);

/// Background of the plugins output log.
const TERM_LOG_BG: Color32 = Color32::from_rgb(20, 21, 24);

/// A session message (or "done" notice) waiting to be typed into its target session.
struct Pending {
    id: String,
    from_key: String,
    to: u64,
    /// Held until this session (hook key) finishes its turn.
    waiting_for: Option<String>,
    text: String,
    created: std::time::Instant,
}

struct Viewer {
    path: PathBuf,
    /// Images to step through with the arrow keys (the session's pastes).
    list: Vec<PathBuf>,
    zoom_fit: bool,
}

pub struct App {
    s: Settings,
    tabs: Vec<Tab>,
    dormant: Vec<Dormant>,
    active: Option<u64>,
    next_id: u64,
    profiles: Vec<Profile>,
    /// When `profiles` was last detected (the New-session menu re-checks, throttled).
    profiles_at: std::time::Instant,
    cache: ImageCache,
    viewer: Option<Viewer>,
    renaming: Option<(u64, String)>,
    /// The rename box takes the keyboard once, when it opens (asking every frame kept it from
    /// ever losing focus, so Enter / clicking away never finished the rename).
    rename_focus: bool,
    project_settings: Option<project_settings::ProjectDraft>,
    /// Each project's GitHub page (None = not on GitHub), with when it was read from the git config.
    github_pages: std::collections::HashMap<PathBuf, (std::time::Instant, Option<String>)>,
    /// The session last shown in each project (restored when its tab is selected again).
    remembered: std::collections::HashMap<PathBuf, u64>,
    filter: String,
    toast: Option<(String, Color32, f64)>,
    logo: Option<egui::TextureHandle>,
    focus_terminal: bool,
    applied: (String, f32),
    options: Option<OptPage>,
    /// How far the Options window has been dragged from the screen centre.
    options_offset: Vec2,
    /// Options > Appearance > UI Scale while its slider is being dragged (applied on release).
    ui_scale_edit: Option<f32>,
    /// Options > Shortcuts is recording a new binding for this command.
    capturing: Option<keys::Cmd>,
    plugins: Option<PluginsUi>,
    mcp_servers: Option<mcp_servers_ui::McpServersUi>,
    /// The status bar's "clear" asks first: the session it's for.
    confirm_clear: Option<u64>,
    /// The header's model / effort dialog.
    model_switch: Option<model_switch::ModelSwitch>,
    /// The sidebar's Close (of a working session) / Delete asks first.
    confirm_close: Option<session_close::ConfirmClose>,
    /// The session whose context details window (from the context pill) is open.
    context_info: Option<context_info::ContextInfo>,
    /// Home's "Remove" on a project asks first: the project it's for.
    confirm_remove_project: Option<PathBuf>,
    /// The sidebar's group menus ask first: (project, true = delete all Archived,
    /// false = archive all Saved for later).
    confirm_group: Option<(PathBuf, bool)>,
    /// Closing the window while sessions are working asks first.
    confirm_quit: bool,
    /// "Clear history" in the header search box's dropdown asks first.
    confirm_clear_find_history: bool,
    /// "Close Anyway" was picked: let the next close request through.
    quit_confirmed: bool,
    /// Plan usage limits (5-hour / weekly) from Claude's status line, and when they were read.
    limits: Option<crate::hooks::Limits>,
    limits_read: Option<(std::time::Instant, Option<std::time::SystemTime>)>,
    /// Codex's plan limits (read while a Codex session is active) and when they were read.
    codex_limits: Vec<agents::LimitWindow>,
    codex_limits_read: Option<std::time::Instant>,
    /// Antigravity's quota (`agy -p /usage`, run on a thread while an Antigravity session is
    /// active) and when it was last asked for.
    agy_quota: std::sync::Arc<std::sync::Mutex<Vec<agents::AgyQuota>>>,
    agy_quota_asked: Option<std::time::Instant>,
    /// Each conversation's share of the plan limits (conversation id → share), and its file time.
    plan_usage: std::collections::HashMap<String, crate::hooks::PlanShare>,
    plan_usage_read: Option<std::time::SystemTime>,
    mcp: Option<mcp::Server>,
    last_title: String,
    drag_project: Option<PathBuf>,
    /// The filter typed in the project tabs' "»" menu, and whether it should take focus.
    tab_menu_filter: String,
    tab_menu_focus: bool,
    drag_session: Option<u64>,
    snippets_open: bool,
    snippet_filter: String,
    snippet_focus: bool,
    scripts_open: bool,
    script_filter: String,
    script_focus: bool,
    scripts: Vec<scripts::Script>,
    /// Options > Scripts editor state.
    script_drafts: Vec<scripts::ScriptDraft>,
    scripts_synced: bool,
    /// The scripts folder was read for the Favourites panel.
    scripts_scanned: bool,
    /// Options > Snippets: (row, name) when its name box got focus, to move stars on rename.
    snippet_name_before: Option<(usize, String)>,
    /// When a Favourites row last ran, so a double-click doesn't run it twice.
    fav_last_pick: Option<std::time::Instant>,
    script_new_ext: String,
    /// Scripts running (or finished, until closed) in the background, each in a small panel.
    script_runs: Vec<scripts::ScriptRun>,
    /// Options > Agents: installed copies of the agent CLIs (rescanned every few seconds).
    agent_installs: Option<(std::time::Instant, Vec<crate::agent_install::Installation>)>,
    /// The copy whose Uninstall was clicked once (the second click confirms).
    agent_confirm: Option<PathBuf>,
    outbox: Vec<Pending>,
    next_msg: u64,
    /// Sessions that get Enter shortly (after a pasted message landed).
    enter_due: Vec<(u64, std::time::Instant)>,
    /// Slash commands to type into a session once it's due (after an earlier command's Enter).
    cmd_due: Vec<(u64, std::time::Instant, String)>,
    /// Taskbar badge last drawn (Some(0) = mark, Some(n) = count, None = nothing yet).
    last_badge: Option<Option<usize>>,
    last_progress: Option<bool>,
    /// The session the last notification was about (clicking the toast opens it).
    notified: Option<u64>,
    last_manage: std::time::Instant,
    /// Started with --minimized (launch at sign-in): hide into the tray on the first frame.
    start_hidden: bool,
    launch: Option<LaunchDialog>,
    /// Models / efforts for the launch dialog, loaded in the background at start-up and on
    /// "Refresh models".
    catalog: std::sync::Arc<std::sync::Mutex<Option<agents::Catalog>>>,
    /// A model refresh is running.
    catalog_loading: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// Claude conversations from other apps, scanned in the background at start-up and merged
    /// into the list once ready.
    found_claude: std::sync::Arc<std::sync::Mutex<Option<Vec<agents::FoundClaude>>>>,
    /// Hex fields being edited in Options (keyed by color name).
    hex_edit: std::collections::HashMap<String, String>,
    /// The sidebar's Archived group is expanded.
    show_archived: bool,
    /// The sidebar's Saved for later group is expanded.
    show_later: bool,
    /// Test hook (`--new-menu`): open the New session ▼ menu on the next frame.
    open_new_menu: bool,
    /// The Search window (open while Some).
    search_ui: Option<search_ui::SearchUi>,
    /// The header's "Search this session" box.
    header_query: String,
    /// The header search box's recent-searches dropdown is showing.
    find_hist_open: bool,
    /// The Check for updates dialog is open.
    update_dialog: bool,
    /// The update whose "downloaded" toast was shown (once per tag).
    update_announced: Option<String>,
    /// While the Options window is open: the settings as last seen (sessions left out), and when
    /// they last changed (not yet saved).
    opts_snapshot: Option<String>,
    opts_changed_at: Option<f64>,
}

fn project_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| p.display().to_string())
}

impl App {
    pub fn new(cc: &eframe::CreationContext) -> Self {
        // The window exists but is still hidden: darken its title bar before it is first shown.
        crate::titlebar::apply();
        let mut s: Settings = cc.storage.and_then(|st| eframe::get_value(st, eframe::APP_KEY)).unwrap_or_default();
        install_fonts(&cc.egui_ctx, &s.code_font);
        apply_style(&cc.egui_ctx);
        s.ui_scale = s.ui_scale.clamp(UI_SCALE_MIN, UI_SCALE_MAX);
        cc.egui_ctx.set_zoom_factor(s.ui_scale);
        cc.egui_ctx.options_mut(|o| o.zoom_with_keyboard = false);
        if s.editor.is_empty() {
            s.editor = "auto".into();
        }
        if s.version < 10 {
            // Closing used to follow "minimize to tray".
            s.close_action = if s.minimize_to_tray { "tray" } else { "exit" }.into();
        }
        if !matches!(s.close_action.as_str(), "exit" | "minimize" | "tray") {
            s.close_action = "exit".into();
        }
        if s.version < 9 {
            s.name_sessions = true;
        }
        if s.version < 8 {
            s.notifications = true;
            s.notify_sound = true;
            s.badge_color = [0xe5, 0x14, 0x14];
            s.badge_text_color = [0xff, 0xff, 0xff];
        }
        if s.version < 7 {
            s.color_tags = true;
            s.color_tag_prompt = true;
        }
        if s.version < 5 {
            // The terminal font is a CSS-style family list now (the default list).
            s.code_font = crate::fonts::DEFAULT_LIST.into();
        }
        if s.version < 4 {
            // Profile names are now ("Claude", "Codex", "pwsh", "powershell", "cmd").
            s.default_profile = crate::session::CLAUDE.into();
            s.version = 4;
        }
        s.version = s.version.max(10);
        // The registry is the truth for "launch on startup" (the user may have removed it).
        s.launch_on_startup = winsys::autostart_enabled();
        crate::dlog::ENABLED.store(s.debug_logging, std::sync::atomic::Ordering::Relaxed);
        *agents::HOOK_SETTINGS.lock().unwrap() = crate::hooks::write_settings().map(|p| p.display().to_string());
        crate::hooks::watch(cc.egui_ctx.clone());
        if s.projects.is_empty() {
            let home = std::env::var_os("USERPROFILE").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("C:\\"));
            s.projects.push(Project { name: project_name(&home), path: home, collapsed: false, in_tabs: true, accent: None, image: None, background: None, fav_snippets: Vec::new(), fav_scripts: Vec::new(), fav_order: Vec::new() });
        }
        images::cleanup_async(s.paste_keep_days, s.paste_limit_bytes());
        let logo = eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon_64.png")).ok().map(|icon| {
            let img = egui::ColorImage::from_rgba_unmultiplied([icon.width as usize, icon.height as usize], &icon.rgba);
            cc.egui_ctx.load_texture("app_logo", img, egui::TextureOptions::LINEAR)
        });
        agents::COLOR_TAG_PROMPT.store(s.color_tag_prompt && s.color_tags, std::sync::atomic::Ordering::Relaxed);
        agents::NAME_SESSION_PROMPT.store(s.name_sessions, std::sync::atomic::Ordering::Relaxed);
        let applied = (s.code_font.clone(), s.ui_scale);
        let mut app = App {
            s,
            tabs: Vec::new(),
            dormant: Vec::new(),
            active: None,
            next_id: 1,
            profiles: Profile::detect(),
            profiles_at: std::time::Instant::now(),
            cache: ImageCache::new(),
            viewer: None,
            renaming: None,
            rename_focus: false,
            project_settings: None,
            github_pages: Default::default(),
            remembered: Default::default(),
            filter: String::new(),
            toast: None,
            logo,
            focus_terminal: true,
            applied,
            options: None,
            options_offset: Vec2::ZERO,
            ui_scale_edit: None,
            capturing: None,
            plugins: None,
            mcp_servers: None,
            confirm_clear: None,
            model_switch: None,
            confirm_close: None,
            context_info: None,
            confirm_remove_project: None,
            confirm_group: None,
            confirm_quit: false,
            confirm_clear_find_history: false,
            quit_confirmed: false,
            limits: None,
            limits_read: None,
            codex_limits: Vec::new(),
            codex_limits_read: None,
            agy_quota: Default::default(),
            agy_quota_asked: None,
            plan_usage: Default::default(),
            plan_usage_read: None,
            mcp: None,
            last_title: String::new(),
            drag_project: None,
            tab_menu_filter: String::new(),
            tab_menu_focus: false,
            drag_session: None,
            snippets_open: false,
            snippet_filter: String::new(),
            snippet_focus: false,
            scripts_open: false,
            script_filter: String::new(),
            script_focus: false,
            scripts: Vec::new(),
            script_drafts: Vec::new(),
            scripts_synced: false,
            scripts_scanned: false,
            snippet_name_before: None,
            fav_last_pick: None,
            script_new_ext: "ps1".into(),
            script_runs: Vec::new(),
            agent_installs: None,
            agent_confirm: None,
            outbox: Vec::new(),
            next_msg: 0,
            enter_due: Vec::new(),
            cmd_due: Vec::new(),
            last_badge: None,
            last_progress: None,
            notified: None,
            last_manage: std::time::Instant::now(),
            start_hidden: std::env::args().any(|a| a == "--minimized"),
            launch: None,
            catalog: Default::default(),
            catalog_loading: Default::default(),
            found_claude: Default::default(),
            hex_edit: Default::default(),
            show_archived: false,
            show_later: false,
            open_new_menu: std::env::args().any(|a| a == "--new-menu"),
            search_ui: None,
            header_query: String::new(),
            find_hist_open: false,
            update_dialog: std::env::args().any(|a| a == "--check-updates"),
            update_announced: None,
            opts_snapshot: None,
            opts_changed_at: None,
        };
        let ctx = cc.egui_ctx.clone();
        // MCP server for the sessions (tools to work with AI Terminal itself).
        match mcp::start(std::sync::Arc::new(std::sync::Mutex::new(crate::mcp_tools::definitions())), ctx.clone()) {
            Ok(srv) => {
                // Sessions inherit these; Claude's MCP config reads them.
                std::env::set_var(agents::MCP_URL_ENV, &srv.url);
                std::env::set_var(agents::MCP_TOKEN_ENV, &srv.token);
                *agents::MCP_URL.lock().unwrap() = Some(srv.url.clone());
                *agents::MCP_CONFIG.lock().unwrap() = agents::write_mcp_config().map(|p| p.display().to_string());
                crate::dlog!("mcp server at {}", srv.url);
                app.mcp = Some(srv);
            }
            Err(e) => crate::dlog!("mcp server failed: {e}"),
        }
        app.refresh_catalog(&ctx);
        let selected = app.s.selected_project.clone().or_else(|| app.s.last_project.clone());
        // Sessions from last time are listed and resume when clicked (only the last selected one
        // may start by itself, with "Open the last session at start-up").
        let saved = if app.s.reopen_sessions { std::mem::take(&mut app.s.sessions) } else { Vec::new() };
        for mut ss in saved {
            if app.s.projects.iter().any(|p| p.path == ss.project) {
                if ss.created == 0 {
                    ss.created = now_secs();
                }
                ss.profile.fix_kind();
                if !ss.profile.kind.tracks_conversation() {
                    // Shell sessions (and agents whose conversations we can't resume) aren't kept between runs.
                    continue;
                }
                // Pick up profile changes (e.g. Claude now started directly).
                match app.profiles.iter().find(|p| p.kind == ss.profile.kind) {
                    Some(p) => ss.profile = p.clone(),
                    None => continue,
                }
                // A session saved before its first message (older versions did): nothing to resume.
                if !agents::conversation_exists(ss.profile.kind, &ss.project, ss.agent_id.as_deref()) {
                    continue;
                }
                if let Some(id) = &ss.agent_id {
                    let t = match ss.profile.kind {
                        Kind::Claude => agents::claude_title(&ss.project, id),
                        Kind::Codex => agents::codex_title(id),
                        Kind::Antigravity => agents::agy_title(id),
                        _ => None,
                    };
                    if t.is_some() {
                        ss.title = t;
                    }
                }
                let id = app.next_id;
                app.next_id += 1;
                app.dormant.push(Dormant { id, saved: ss });
            }
        }
        if app.s.reopen_sessions && app.s.import_claude_sessions {
            // Only transcripts that existed before now: newer ones belong to sessions starting here.
            let (found, ctx, before) = (app.found_claude.clone(), ctx.clone(), std::time::SystemTime::now());
            std::thread::spawn(move || {
                let list = agents::scan_claude_conversations(before);
                *found.lock().unwrap() = Some(list);
                ctx.request_repaint();
            });
        }
        // Fall back to the first project still shown as a tab (or Home): selecting a project
        // re-shows its tab, so falling back to a hidden one would undo the user's closing it.
        let selected = selected.filter(|p| app.s.projects.iter().any(|x| &x.path == p && x.in_tabs)).or_else(|| app.tab_projects().into_iter().next());
        app.select_project(selected);
        if app.s.open_last_session {
            let last = app.s.last_session.clone();
            if let Some(id) = last.and_then(|a| app.dormant.iter().find(|d| d.saved.agent_id.as_deref() == Some(a.as_str())).map(|d| d.id)) {
                app.resume(&ctx, id);
            }
        }
        // Test hooks: `--add-project <dir>` adds (and selects) a project tab; `--start <profile>` starts a session; `--paste-image <file>` registers a file as pasted into the active session
        // (as if pasted with Ctrl+V); `--type <text>` types a line into it.
        let args: Vec<String> = std::env::args().collect();
        for w in args.windows(2) {
            if w[0] == "--add-project" {
                let dir = PathBuf::from(&w[1]);
                if !app.s.projects.iter().any(|p| p.path == dir) {
                    app.s.projects.push(Project { name: project_name(&dir), path: dir.clone(), collapsed: false, in_tabs: true, accent: None, image: None, background: None, fav_snippets: Vec::new(), fav_scripts: Vec::new(), fav_order: Vec::new() });
                }
                app.s.selected_project = Some(dir);
            }
        }
        if args.iter().any(|a| a == "--launch-dialog") {
            if let Some(p) = app.profiles.iter().find(|p| p.is_claude()).cloned() {
                let pr = app.current_project();
                app.open_launch_dialog(pr, p, None);
            }
        }
        if args.iter().any(|a| a == "--home") {
            app.select_project(None);
        }
        if args.iter().any(|a| a == "--options") {
            app.options = Some(OptPage::Terminal);
        }
        for w in args.windows(2) {
            if w[0] == "--options-page" {
                app.options = Some(match w[1].as_str() {
                    "general" => OptPage::General,
                    "images" => OptPage::Images,
                    "shortcuts" => OptPage::Shortcuts,
                    "snippets" => OptPage::Snippets,
                    "scripts" => OptPage::Scripts,
                    "agents" => OptPage::Agents,
                    "about" => OptPage::About,
                    _ => OptPage::Terminal,
                });
            }
        }
        for w in args.windows(2) {
            if w[0] == "--view-image" {
                let p = PathBuf::from(&w[1]);
                app.viewer = Some(Viewer { path: p.clone(), list: vec![p], zoom_fit: true });
            }
        }
        for w in args.windows(2) {
            if w[0] == "--pin" {
                let p = app.current_project();
                if let Some(r) = w[1].parse::<usize>().ok().and_then(|n| app.rows(&p).get(n.saturating_sub(1)).map(|r| r.id)) {
                    app.set_pin(r, 1);
                }
            }
        }
        if args.iter().any(|a| a == "--no-previews") {
            app.s.inline_previews = false;
        }
        if args.iter().any(|a| a == "--debug-log") {
            app.s.debug_logging = true;
            crate::dlog::ENABLED.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        if args.iter().any(|a| a == "--scripts") {
            app.open_scripts();
        }
        for w in args.windows(2) {
            if w[0] == "--search" {
                app.open_search();
                if let Some(st) = app.search_ui.as_mut() {
                    st.set_query(&w[1]);
                }
                app.run_search(&ctx);
            }
        }
        if args.iter().any(|a| a == "--plugins") {
            app.open_plugins();
        }
        if args.iter().any(|a| a == "--plugins-sources") {
            app.open_plugins();
            if let Some(p) = app.plugins.as_mut() {
                p.show_sources = true;
            }
        }
        for w in args.windows(2) {
            if w[0] == "--start" {
                if let Some(p) = app.profiles.iter().find(|p| p.name == w[1]).cloned() {
                    let pr = app.current_project();
                    app.spawn(&ctx, pr, p, String::new());
                }
            }
        }
        for w in args.windows(2) {
            if w[0] == "--run-script" {
                app.run_script_named(&ctx, &w[1]);
            }
        }
        for w in args.windows(2) {
            if let Some(tab) = app.active_tab_mut() {
                match w[0].as_str() {
                    "--paste-image" => tab.sess.pasted_images.push(PathBuf::from(&w[1])),
                    "--type" => tab.sess.write(format!("{}\r", w[1]).into_bytes()),
                    "--find" => {
                        tab.view.search.open();
                        tab.view.search.query = w[1].clone();
                    }
                    _ => {}
                }
            }
        }
        app
    }

    fn default_profile(&self) -> Profile {
        self.profiles.iter().find(|p| p.name == self.s.default_profile).or(self.profiles.first()).cloned().unwrap()
    }

    /// A plain new session of `profile` (agents reuse the last model / effort / parameters).
    fn new_default(&mut self, ctx: &egui::Context, project: PathBuf, profile: Profile, unrestricted: bool) {
        let mut opts = if profile.is_agent() { self.last_opts(profile.kind).clone() } else { LaunchOpts::default() };
        opts.unrestricted = unrestricted && profile.is_agent();
        self.spawn_opts(ctx, project, profile, String::new(), opts);
    }

    /// The selected project tab, or the first project when Home is selected.
    fn current_project(&self) -> PathBuf {
        self.s.selected_project.clone().filter(|p| self.s.projects.iter().any(|x| &x.path == p)).unwrap_or_else(|| self.s.projects[0].path.clone())
    }

    /// Projects shown as tabs, in tab order (Ctrl+1…9 and "tab N" count these).
    fn tab_projects(&self) -> Vec<PathBuf> {
        self.s.projects.iter().filter(|p| p.in_tabs).map(|p| p.path.clone()).collect()
    }

    /// Selects a project tab (None = Home) and shows the session last used there. Opening a
    /// project whose tab was closed brings its tab back.
    fn select_project(&mut self, p: Option<PathBuf>) {
        if let Some(proj) = p.as_ref().and_then(|p| self.s.projects.iter_mut().find(|x| &x.path == p)) {
            proj.in_tabs = true;
        }
        self.active = p.as_ref().and_then(|p| {
            let ids = self.project_sessions(p);
            self.remembered.get(p).copied().filter(|id| ids.contains(id)).or(ids.first().copied())
        });
        if p.is_some() {
            self.s.last_project = p.clone();
        }
        self.s.selected_project = p;
        self.filter.clear();
        self.focus_terminal = true;
    }

    fn select_session(&mut self, id: u64) {
        if let Some(t) = self.tabs.iter().find(|t| t.sess.id == id) {
            self.remembered.insert(t.project.clone(), id);
            self.s.selected_project = Some(t.project.clone());
            self.s.last_project = Some(t.project.clone());
            self.active = Some(id);
            self.focus_terminal = true;
        }
    }

    fn active_tab(&self) -> Option<&Tab> {
        self.tabs.iter().find(|t| Some(t.sess.id) == self.active)
    }

    fn active_tab_mut(&mut self) -> Option<&mut Tab> {
        let a = self.active;
        self.tabs.iter_mut().find(|t| Some(t.sess.id) == a)
    }

    fn spawn(&mut self, ctx: &egui::Context, project: PathBuf, profile: Profile, name: String) {
        self.new_default(ctx, project, profile, false);
        let _ = name;
    }

    fn spawn_opts(&mut self, ctx: &egui::Context, project: PathBuf, profile: Profile, name: String, opts: LaunchOpts) {
        self.spawn_with(ctx, project, profile, name, None, opts, now_secs());
    }

    #[allow(clippy::too_many_arguments)]
    fn spawn_with(&mut self, ctx: &egui::Context, project: PathBuf, profile: Profile, name: String, resume: Option<String>, opts: LaunchOpts, created: u64) -> bool {
        let id = self.next_id;
        self.next_id += 1;
        if !project.is_dir() {
            self.toast(format!("Folder not found: {}", project.display()), RED, ctx);
            return false;
        }
        match Session::spawn(id, name, profile.clone(), project.clone(), resume, opts, ctx) {
            Ok(sess) => {
                self.tabs.push(Tab { project, created, pin: 0, sess, view: ViewState::default(), bell: false });
                self.select_session(id);
                true
            }
            Err(e) => {
                self.toast(format!("Couldn't start {}: {e}", profile.name), RED, ctx);
                false
            }
        }
    }

    /// Starts a session from the last run (Claude Code resumes its conversation).
    fn resume(&mut self, ctx: &egui::Context, id: u64) {
        let Some(i) = self.dormant.iter().position(|d| d.id == id) else { return };
        let d = self.dormant[i].clone();
        let ss = d.saved;
        let name = if ss.name.is_empty() { String::new() } else { ss.name.clone() };
        if self.spawn_with(ctx, ss.project.clone(), ss.profile.clone(), name, ss.agent_id.clone(), ss.opts.clone(), ss.created) {
            self.dormant.remove(i);
            if let Some(t) = self.tabs.last_mut() {
                t.pin = ss.pin;
                t.sess.used = ss.used;
                // Keep the old title until the program sets a new one.
                if t.sess.title.is_none() {
                    t.sess.title = ss.title;
                }
            }
        }
    }

    fn restart(&mut self, ctx: &egui::Context, id: u64) {
        self.restart_with(ctx, id, None);
    }

    /// Restarts a session (resuming its conversation), optionally with new launch settings.
    fn restart_with(&mut self, ctx: &egui::Context, id: u64, opts: Option<LaunchOpts>) {
        let Some(i) = self.tabs.iter().position(|t| t.sess.id == id) else { return };
        let opts = opts.unwrap_or_else(|| self.tabs[i].sess.opts.clone());
        let pasted = self.tabs[i].sess.pasted_images.clone();
        let tt = self.tabs[i].sess.transcript_title.clone();
        let (project, profile, name, created) = (self.tabs[i].project.clone(), self.tabs[i].sess.profile.clone(), self.tabs[i].sess.name.clone(), self.tabs[i].created);
        let pin = self.tabs[i].pin;
        let used = self.tabs[i].sess.used;
        let new_id = self.next_id;
        self.next_id += 1;
        let resume = self.tabs[i].sess.agent_id.clone();
        match Session::spawn(new_id, name, profile, project.clone(), resume, opts, ctx) {
            Ok(mut sess) => {
                sess.pasted_images = pasted;
                sess.transcript_title = tt;
                sess.used = used;
                self.tabs[i] = Tab { project, created, pin, sess, view: ViewState::default(), bell: false };
                self.select_session(new_id);
            }
            Err(e) => self.toast(format!("Couldn't restart: {e}"), RED, ctx),
        }
    }

    fn close(&mut self, id: u64) {
        let Some(i) = self.tabs.iter().position(|t| t.sess.id == id) else { return };
        let project = self.tabs[i].project.clone();
        let pos = self.project_sessions(&project).iter().position(|x| *x == id).unwrap_or(0);
        self.tabs.remove(i);
        if self.active == Some(id) {
            // The neighbouring session of the same project (the project stays selected).
            let same = self.project_sessions(&project);
            self.remembered.remove(&project);
            match same.get(pos.min(same.len().saturating_sub(1))) {
                Some(&next) => self.select_session(next),
                None => self.active = None,
            }
        }
    }

    fn toast(&mut self, msg: String, col: Color32, ctx: &egui::Context) {
        self.toast = Some((msg, col, ctx.input(|i| i.time) + 4.0));
    }

    /// Ctrl+V: files copied in Explorer paste as their paths; a bitmap is saved as PNG and
    /// pastes as its path (Claude Code attaches it); otherwise text (bracketed when multi-line).
    fn paste(&mut self, ctx: &egui::Context) {
        let mut cb = match arboard::Clipboard::new() {
            Ok(c) => c,
            Err(e) => return self.toast(format!("Clipboard unavailable: {e}"), RED, ctx),
        };
        let files = cb.get().file_list().unwrap_or_default();
        let (text, images, kind): (String, Vec<PathBuf>, &str) = if !files.is_empty() {
            let imgs = files.iter().filter(|p| images::is_image_path(p)).cloned().collect();
            (files.iter().map(|p| images::quote(p)).collect::<Vec<_>>().join(" "), imgs, "files")
        } else if let Ok(img) = cb.get_image() {
            match images::save_clipboard_image(&img) {
                Ok(p) => {
                    self.cache.forget(&p);
                    images::cleanup_async(self.s.paste_keep_days, self.s.paste_limit_bytes());
                    (images::quote(&p), vec![p], "image")
                }
                Err(e) => return self.toast(e, RED, ctx),
            }
        } else if let Ok(t) = cb.get_text() {
            (t, Vec::new(), "text")
        } else {
            return;
        };
        let Some(tab) = self.active_tab_mut() else { return };
        let bracketed = tab.sess.term.lock().mode().contains(alacritty_terminal::term::TermMode::BRACKETED_PASTE);
        let n_images = images.len();
        tab.sess.pasted_images.extend(images);
        let data = if kind == "text" && (text.contains('\n') || text.contains('\r')) {
            let t = text.replace("\r\n", "\n");
            if bracketed {
                format!("\x1b[200~{}\x1b[201~", t.replace('\x1b', ""))
            } else {
                t.replace('\n', "\r")
            }
        } else {
            text
        };
        tab.sess.term.lock().scroll_display(alacritty_terminal::grid::Scroll::Bottom);
        tab.sess.write(data.into_bytes());
        self.focus_terminal = true;
        if n_images > 0 && kind == "image" {
            self.toast("Image pasted — preview shown in the terminal".into(), GREEN, ctx);
        }
    }

    fn handle_drops(&mut self, ctx: &egui::Context) {
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).filter(|p| !p.as_os_str().is_empty()).collect());
        if dropped.is_empty() {
            return;
        }
        let Some(tab) = self.active_tab_mut() else { return };
        let text = dropped.iter().map(|p| images::quote(p)).collect::<Vec<_>>().join(" ") + " ";
        tab.sess.pasted_images.extend(dropped.iter().filter(|p| images::is_image_path(p)).cloned());
        tab.sess.write(text.into_bytes());
        self.focus_terminal = true;
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        // Reassignable commands (Options > Shortcuts). Longer combinations first, so a
        // Ctrl+Shift binding is matched before a Ctrl one with the same key.
        let mut cmds: Vec<(keys::Cmd, KeyboardShortcut)> = keys::ALL.iter().filter_map(|c| keys::binding(&self.s.keybindings, *c).map(|b| (*c, b))).collect();
        cmds.sort_by_key(|(_, b)| std::cmp::Reverse(b.modifiers.ctrl as u8 + b.modifiers.shift as u8 + b.modifiers.alt as u8));
        for (cmd, b) in cmds {
            if keys::consume_exact(ctx, &b) {
                self.run_cmd(ctx, cmd);
            }
        }
        // Fixed: Ctrl+1..9 = project tab n, Ctrl+wheel = font size.
        for (k, key) in [Key::Num1, Key::Num2, Key::Num3, Key::Num4, Key::Num5, Key::Num6, Key::Num7, Key::Num8, Key::Num9].into_iter().enumerate() {
            if keys::consume_exact(ctx, &KeyboardShortcut::new(Modifiers::CTRL, key)) {
                if let Some(p) = self.tab_projects().get(k).cloned() {
                    self.select_project(Some(p));
                }
            }
        }
        let zoom = ctx.input(|i| i.events.iter().filter_map(|e| if let egui::Event::MouseWheel { delta, modifiers, .. } = e { modifiers.ctrl.then_some(delta.y) } else { None }).sum::<f32>());
        if zoom != 0.0 {
            self.s.font_size = (self.s.font_size + zoom.signum()).clamp(8.0, 32.0);
        }
    }

    fn run_cmd(&mut self, ctx: &egui::Context, cmd: keys::Cmd) {
        use keys::Cmd::*;
        match cmd {
            NewClaude | NewClaudeUnrestricted => {
                let p = self.current_project();
                if let Some(pr) = self.profiles.iter().find(|p| p.is_claude()).cloned().or_else(|| Some(self.default_profile())) {
                    self.new_default(ctx, p, pr, cmd == NewClaudeUnrestricted);
                }
            }
            NewDefault => {
                let p = self.current_project();
                self.launch_default(ctx, p);
            }
            CloseSession => {
                if let Some(id) = self.active {
                    self.request_close(id);
                }
            }
            RenameSession => {
                if let Some(t) = self.active_tab() {
                    self.renaming = Some((t.sess.id, t.sess.display_name()));
                    self.rename_focus = true;
                }
            }
            NextSession | PrevSession => {
                let order = self.project_sessions(&self.current_project());
                if !order.is_empty() {
                    let i = order.iter().position(|id| Some(*id) == self.active).unwrap_or(0);
                    let n = order.len();
                    self.select_session(order[if cmd == PrevSession { (i + n - 1) % n } else { (i + 1) % n }]);
                }
            }
            NextProject | PrevProject => {
                let mut tabs: Vec<Option<PathBuf>> = vec![None];
                tabs.extend(self.tab_projects().into_iter().map(Some));
                let i = tabs.iter().position(|t| *t == self.s.selected_project).unwrap_or(0);
                let n = tabs.len();
                let j = if cmd == PrevProject { (i + n - 1) % n } else { (i + 1) % n };
                self.select_project(tabs[j].clone());
            }
            FontBigger => self.s.font_size = (self.s.font_size + 1.0).min(32.0),
            FontSmaller => self.s.font_size = (self.s.font_size - 1.0).max(8.0),
            FontReset => self.s.font_size = 15.0,
            TogglePreviews => self.s.inline_previews = !self.s.inline_previews,
            Options => self.options = Some(OptPage::General),
            Plugins => self.open_plugins(),
            Snippets => {
                if self.active.is_some() {
                    self.snippets_open = true;
                    self.snippet_focus = true;
                }
            }
            Scripts => self.open_scripts(),
            SearchSessions => self.open_search(),
            Find => {
                if let Some(t) = self.active_tab_mut() {
                    t.view.search.open();
                }
            }
        }
    }

    fn set_pin(&mut self, id: u64, pin: u32) {
        if let Some(t) = self.tabs.iter_mut().find(|t| t.sess.id == id) {
            t.pin = pin;
        }
        if let Some(d) = self.dormant.iter_mut().find(|d| d.id == id) {
            d.saved.pin = pin;
        }
    }

    /// Turns a running session into a listed one (it resumes when clicked): auto-close.
    fn park(&mut self, id: u64, why: &str) {
        let Some(i) = self.tabs.iter().position(|t| t.sess.id == id) else { return };
        let t = &self.tabs[i];
        let saved = SavedSession {
            project: t.project.clone(),
            profile: t.sess.profile.clone(),
            name: t.sess.name.clone(),
            created: t.created,
            title: t.sess.transcript_title.clone().or_else(|| t.sess.title.clone()),
            agent_id: t.sess.agent_id.clone(),
            opts: t.sess.opts.clone(),
            cost: t.sess.cost.map(|c| c.usd),
            pin: t.pin,
            archived: false,
            later: false,
            used: t.sess.used.max(t.created),
        };
        crate::dlog!("auto-close #{id} ({why}): {}", t.sess.display_name());
        let nid = self.next_id;
        self.next_id += 1;
        self.dormant.push(Dormant { id: nid, saved });
        self.tabs.remove(i);
        if self.active == Some(id) {
            self.active = None;
        }
    }

    /// Window / tray / notification / auto-close housekeeping, once per frame.
    fn housekeeping(&mut self, ctx: &egui::Context) {
        crate::dlog::ENABLED.store(self.s.debug_logging, std::sync::atomic::Ordering::Relaxed);
        self.update_housekeeping(ctx);
        let focused = ctx.input(|i| i.viewport().focused.unwrap_or(true)) && winsys::is_main_visible();
        self.auto_archive();

        winsys::set_single_instance(ctx, self.s.single_instance);
        // Tray: needed for minimize / close to tray and for notifications.
        let tray_window = self.s.minimize_to_tray || self.s.close_action == "tray";
        if tray_window || self.s.notifications {
            winsys::tray_show(ctx);
        } else {
            winsys::tray_hide();
        }
        // Started at sign-in (--minimized): into the tray, or just minimized without it.
        if std::mem::take(&mut self.start_hidden) {
            if tray_window {
                winsys::hide_main();
            } else {
                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
            }
        }
        let quitting = winsys::QUITTING.load(std::sync::atomic::Ordering::Relaxed);
        // Closing the window: exit, minimize, or hide to the tray (Quit from the tray really exits).
        let close_exits = quitting || self.s.close_action == "exit";
        if !close_exits && ctx.input(|i| i.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            if self.s.close_action == "tray" {
                winsys::hide_main();
            } else {
                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
            }
        }
        if self.s.minimize_to_tray && !quitting && ctx.input(|i| i.viewport().minimized) == Some(true) && winsys::is_main_visible() {
            winsys::hide_main();
        }
        // Really closing with work in flight: ask first.
        if close_exits && !self.quit_confirmed && ctx.input(|i| i.viewport().close_requested()) && self.busy_count() > 0 {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            winsys::show_main();
            self.confirm_quit = true;
        }
        for e in winsys::take_events() {
            if e == winsys::TrayEvent::NotificationClicked {
                if let Some(id) = self.notified.take() {
                    self.select_session(id);
                }
            }
        }

        // Hook events (Claude) and bells.
        let keys: std::collections::HashSet<String> = self.tabs.iter().map(|t| t.sess.hook_key.clone()).collect();
        let mut alerts: Vec<(u64, String)> = Vec::new();
        for ev in crate::hooks::poll(&|k| keys.contains(k)) {
            let active = self.active;
            let Some(t) = self.tabs.iter_mut().find(|t| t.sess.hook_key == ev.key) else { continue };
            crate::dlog!("hook {} for #{} {}", ev.event, t.sess.id, ev.message);
            match ev.event.as_str() {
                "start" => t.sess.set_conversation(ev.message.trim()),
                "active" => {
                    t.sess.busy = true;
                    t.sess.attention = None;
                }
                "stop" | "notify" => {
                    // Waiting for you either way (a Notification is e.g. a permission prompt).
                    t.sess.busy = false;
                    if ev.event == "stop" {
                        t.sess.busy = false;
                        // Messages waiting for this session to finish its turn may go now.
                        let key = t.sess.hook_key.clone();
                        for m in self.outbox.iter_mut().filter(|m| m.waiting_for.as_deref() == Some(key.as_str())) {
                            m.waiting_for = None;
                        }
                    }
                    let msg = if ev.message.trim().is_empty() {
                        if ev.event == "stop" { "Claude is waiting for your input".to_string() } else { "Claude needs your attention".to_string() }
                    } else {
                        ev.message.clone()
                    };
                    crate::dlog!("attention check #{}: window focused={focused}, active={:?}", t.sess.id, active);
                    if !(focused && active == Some(t.sess.id)) {
                        t.sess.attention = Some(msg.clone());
                        alerts.push((t.sess.id, msg));
                    }
                }
                _ => {}
            }
        }
        let active = self.active;
        for t in &mut self.tabs {
            // Interrupted turns may not send Stop; Claude's spinner keeps output flowing while it
            // works, so 20 s of silence means it isn't working any more.
            if t.sess.busy && t.sess.idle_for() > Duration::from_secs(20) {
                t.sess.busy = false;
            }
            if t.bell {
                t.bell = false;
                if t.sess.attention.is_none() && !(focused && active == Some(t.sess.id)) {
                    t.sess.attention = Some("Bell".into());
                    alerts.push((t.sess.id, "The session rang the bell".into()));
                }
            }
            // Looking at it clears the mark.
            if focused && active == Some(t.sess.id) {
                t.sess.attention = None;
            }
        }
        for (id, msg) in &alerts {
            crate::dlog!("alert #{id}: {msg} (notifications={}, sound={}, flash={})", self.s.notifications, self.s.notify_sound, self.s.flash_taskbar);
        }
        for (id, msg) in alerts {
            let Some(t) = self.tabs.iter().find(|t| t.sess.id == id) else { continue };
            let project = self.s.projects.iter().find(|p| p.path == t.project).map(|p| p.name.clone()).unwrap_or_default();
            let title = format!("{} · {}", t.sess.display_name(), project);
            if self.s.notifications {
                winsys::notify(ctx, &title, &msg);
                if self.s.notify_sound {
                    winsys::play_notification_sound();
                }
                self.notified = Some(id);
            }
            if self.s.flash_taskbar && !focused {
                ctx.send_viewport_cmd(egui::ViewportCommand::RequestUserAttention(egui::UserAttentionType::Informational));
            }
        }

        // Taskbar badge: a mark (or the count) while sessions need attention.
        let count = self.tabs.iter().filter(|t| t.sess.attention.is_some()).count();
        let want = (count > 0).then(|| if self.s.badge_count { count } else { 0 });
        if self.last_badge != Some(want) {
            match want {
                Some(n) => {
                    let (px, s) = winsys::badge_rgba(self.s.badge_count.then_some(n), self.s.badge_color, self.s.badge_text_color);
                    winsys::set_badge(Some((&px, s)), &format!("{count} session(s) need attention"));
                }
                None => winsys::set_badge(None, ""),
            }
            self.last_badge = Some(want);
        }

        // Taskbar progress: an indeterminate (marquee) bar while any session is working.
        let working = self.s.taskbar_progress && self.tabs.iter().any(|t| t.sess.busy && t.sess.exited.is_none());
        if self.last_progress != Some(working) {
            let who: Vec<String> = self.tabs.iter().filter(|t| t.sess.busy && t.sess.exited.is_none()).map(|t| format!("#{} {}", t.sess.id, t.sess.display_name())).collect();
            crate::dlog!("taskbar progress: {working} (busy: {})", who.join(", "));
            winsys::set_progress_busy(working);
            self.last_progress = Some(working);
        }
        // Come back to notice a turn that ended without a Stop (20 s of silence, above).
        if self.tabs.iter().any(|t| t.sess.busy && t.sess.exited.is_none()) {
            ctx.request_repaint_after(Duration::from_secs(5));
        }

        // Session management: idle timeout and memory threshold (every 15 s).
        if self.last_manage.elapsed() >= Duration::from_secs(15) {
            self.last_manage = std::time::Instant::now();
            let manageable = |t: &Tab, active: Option<u64>| t.sess.profile.is_agent() && agents::conversation_exists(t.sess.profile.kind, &t.sess.cwd, t.sess.agent_id.as_deref()) && t.sess.exited.is_none() && !t.sess.busy && active != Some(t.sess.id);
            if self.s.idle_close_minutes > 0 {
                let limit = Duration::from_secs(self.s.idle_close_minutes as u64 * 60);
                let ids: Vec<u64> = self.tabs.iter().filter(|t| manageable(t, self.active) && t.sess.idle_for() >= limit).map(|t| t.sess.id).collect();
                for id in ids {
                    self.park(id, "idle timeout");
                }
            }
            if self.s.memory_threshold_gb > 0.0 && winsys::memory_used_gb() > self.s.memory_threshold_gb as f64 {
                let oldest = self.tabs.iter().filter(|t| manageable(t, self.active) && t.sess.idle_for() >= Duration::from_secs(60)).max_by_key(|t| t.sess.idle_for()).map(|t| t.sess.id);
                if let Some(id) = oldest {
                    self.park(id, "memory threshold");
                }
            }
        }
        // Hook events wake the UI themselves (hooks::watch); session management runs every 15 s.
        if self.s.idle_close_minutes > 0 || self.s.memory_threshold_gb > 0.0 {
            ctx.request_repaint_after(Duration::from_secs(15));
        }
    }

    fn add_project(&mut self, ctx: &egui::Context) {
        if let Some(dir) = rfd::FileDialog::new().set_title("Add project folder").pick_folder() {
            if !self.s.projects.iter().any(|p| p.path == dir) {
                self.s.projects.push(Project { name: project_name(&dir), path: dir.clone(), collapsed: false, in_tabs: true, accent: None, image: None, background: None, fav_snippets: Vec::new(), fav_scripts: Vec::new(), fav_order: Vec::new() });
            }
            let _ = ctx;
            self.select_project(Some(dir));
        }
    }

}

impl eframe::App for App {
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        // The selected session (running or not), for "Open the last session at start-up".
        // Home showing (nothing selected) keeps the previous one.
        if let Some(a) = self.active {
            let id = self.tabs.iter().find(|t| t.sess.id == a).map(|t| t.sess.agent_id.clone())
                .or_else(|| self.dormant.iter().find(|d| d.id == a).map(|d| d.saved.agent_id.clone()));
            if let Some(id) = id {
                self.s.last_session = id;
            }
        }
        self.s.sessions = self.sidebar_order().iter().filter_map(|id| self.tabs.iter().find(|t| t.sess.id == *id)).filter(|t| t.sess.profile.kind.tracks_conversation())
            // Sessions never used have nothing to resume: they aren't listed next time.
            .filter(|t| agents::conversation_exists(t.sess.profile.kind, &t.sess.cwd, t.sess.agent_id.as_deref()))
            .map(|t| SavedSession {
                project: t.project.clone(),
                profile: t.sess.profile.clone(),
                name: t.sess.name.clone(),
                created: t.created,
                title: t.sess.transcript_title.clone().or_else(|| t.sess.title.clone()),
                agent_id: t.sess.agent_id.clone(),
                opts: t.sess.opts.clone(),
                cost: t.sess.cost.map(|c| c.usd),
                pin: t.pin,
                archived: false,
                later: false,
                used: t.sess.used.max(t.created),
            })
            .collect();
        // Listed sessions that weren't started this time stay in the list.
        self.s.sessions.extend(self.dormant.iter().map(|d| d.saved.clone()));
        // A Project settings preview isn't saved: write the project as it was before the dialog.
        let preview = self.project_settings_original().cloned().and_then(|orig| {
            let p = self.s.projects.iter_mut().find(|x| x.path == orig.path)?;
            Some(std::mem::replace(p, orig))
        });
        eframe::set_value(storage, eframe::APP_KEY, &self.s);
        if let Some(prev) = preview {
            self.restore_project(&prev);
        }
    }

    /// Everything that isn't drawing: sessions' output, hook events, MCP calls, housekeeping.
    /// eframe keeps calling this while the window is minimized or hidden in the tray (and `ui`
    /// isn't), so a session finishing in the background still clears its "working" state, the
    /// taskbar progress and sends its notification right away.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let ctx = ctx.clone();
        agents::COLOR_TAG_PROMPT.store(self.s.color_tag_prompt && self.s.color_tags, std::sync::atomic::Ordering::Relaxed);
        agents::NAME_SESSION_PROMPT.store(self.s.name_sessions, std::sync::atomic::Ordering::Relaxed);

        // Terminal events (title, exit, OSC 52 clipboard) and activity marks.
        let active = self.active;
        let claimed: std::collections::HashSet<String> =
            self.tabs.iter().filter_map(|t| t.sess.agent_id.clone()).chain(self.dormant.iter().filter_map(|d| d.saved.agent_id.clone())).collect();
        let color_tags = self.s.color_tags;
        for t in &mut self.tabs {
            t.sess.poll_agent(&claimed);
            let (clip, bell) = t.sess.poll();
            if let Some(clip) = clip {
                // Claude Code's own copy (its in-app selection) carries the raw tags.
                ctx.copy_text(if color_tags { crate::colortags::strip(&clip) } else { clip });
            }
            t.bell |= bell;
            // Only a change to the screen's text counts as unseen output: Claude repaints and
            // retitles itself without saying anything new.
            let changed = t.sess.dirty.swap(false, std::sync::atomic::Ordering::Relaxed);
            if Some(t.sess.id) == active {
                // Also on arriving at a session that changed while away (its `seen_hash` is stale).
                if changed || t.sess.unseen {
                    t.sess.seen_hash = t.sess.screen_hash();
                }
                t.sess.unseen = false;
            } else if changed && t.sess.screen_hash() != t.sess.seen_hash {
                t.sess.unseen = true;
            }
        }
        self.mcp_pump(&ctx);
        self.merge_found_claude();
        self.housekeeping(&ctx);
    }

    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if ctx.input(|i| i.time) < 6.0 {
            crate::titlebar::apply();
            ctx.request_repaint_after(Duration::from_millis(250));
        }
        if self.applied.0 != self.s.code_font {
            install_fonts(&ctx, &self.s.code_font);
            self.applied.0 = self.s.code_font.clone();
        }
        if self.applied.1 != self.s.ui_scale {
            ctx.set_zoom_factor(self.s.ui_scale);
            self.applied.1 = self.s.ui_scale;
        }
        self.cache.pump(&ctx);
        // egui hears of window focus only from Windows' focus messages; if it never had it, make
        // sure the keyboard really comes to us while the window is in front.
        if !ctx.input(|i| i.focused) && winsys::claim_keyboard_focus() {
            ctx.request_repaint();
        }
        if crate::dlog::ENABLED.load(std::sync::atomic::Ordering::Relaxed) {
            // Focus diagnostics: where a click lands, and who held the keyboard before it.
            let (click, pos, win_focused) = ctx.input(|i| (i.pointer.primary_pressed(), i.pointer.interact_pos(), i.focused));
            if click {
                let layer = pos.and_then(|p| ctx.layer_id_at(p));
                let (focused, modal) = ctx.memory(|m| (m.focused(), m.top_modal_layer()));
                crate::dlog!("click at {pos:?}: layer {layer:?}, keyboard focus {focused:?}, modal layer {modal:?}, window focused {win_focused}");
            }
        }
        let title = self.active_tab().map(|t| format!("{} — AI Terminal", t.sess.display_name())).unwrap_or_else(|| "AI Terminal".into());
        // Only when it changes: every viewport command requests another frame, so sending it
        // each frame kept the app redrawing nonstop.
        if self.last_title != title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.last_title = title;
        }

        let modal_open = self.viewer.is_some() || self.options.is_some() || self.launch.is_some() || self.project_settings.is_some() || self.plugins.is_some() || self.mcp_servers.is_some() || self.confirm_clear.is_some() || self.model_switch.is_some() || self.confirm_close.is_some() || self.context_info.is_some() || self.confirm_remove_project.is_some() || self.confirm_group.is_some() || self.confirm_quit || self.confirm_clear_find_history || self.snippets_open || self.scripts_open || self.update_dialog;
        if !modal_open {
            self.shortcuts(&ctx);
            self.handle_drops(&ctx);
        }

        egui::Panel::top("topbar")
            .exact_size(44.0)
            .frame(egui::Frame::new().fill(BAR).inner_margin(egui::Margin { left: 4, right: 4, top: 0, bottom: 0 }).stroke(Stroke::new(1.0, BORDER)))
            .show(ui, |ui| self.topbar(ui));
        egui::Panel::bottom("status").exact_size(26.0).frame(egui::Frame::new().fill(BAR).inner_margin(egui::Margin::symmetric(10, 2))).show(ui, |ui| self.status_bar(ui));
        let home = self.s.selected_project.is_none();
        if !home {
            egui::Panel::left("sidebar")
                .resizable(true)
                .default_size(300.0)
                .min_size(200.0)
                .frame(egui::Frame::new().fill(PANEL).inner_margin(egui::Margin::symmetric(10, 4)))
                .show(ui, |ui| self.sidebar(ui));
        }
        if self.active.is_some() {
            egui::Panel::top("session_header").exact_size(40.0).frame(egui::Frame::new().fill(PANEL).inner_margin(egui::Margin::symmetric(12, 4)).stroke(Stroke::new(1.0, BORDER))).show(ui, |ui| self.header(ui));
        }

        let mut actions = Vec::new();
        // The project's own background (Project settings) replaces the global one when set.
        let bg_project = self.tabs.iter().find(|t| Some(t.sess.id) == self.active).map(|t| t.project.clone()).unwrap_or_else(|| self.current_project());
        let project_bg = if home { None } else { self.s.projects.iter().find(|p| p.path == bg_project).and_then(|p| p.background.clone()) };
        let theme = self.s.term_theme.with_project_bg(project_bg.as_ref());
        egui::CentralPanel::default().frame(egui::Frame::new().fill(term_theme::c(theme.effective_background()))).show(ui, |ui| {
            // With "Scale the terminal" off, undo the window zoom so terminal text keeps its size.
            let font_size = if self.s.scale_terminal { self.s.font_size } else { self.s.font_size / self.s.ui_scale };
            let opt = ViewOptions { font_size, theme: &theme, inline_previews: self.s.inline_previews, preview_max_px: self.s.preview_max_px, preview_over_text: self.s.preview_over_text, shift_enter_alt: self.s.shift_enter_alt, color_tags: self.s.color_tags };
            let focus = std::mem::take(&mut self.focus_terminal) && !modal_open && self.renaming.is_none() && self.project_settings.is_none();
            let a = self.active;
            if home {
                ui.painter().rect_filled(ui.max_rect(), 0.0, BG);
                self.home(ui);
            } else if let Some(tab) = self.tabs.iter_mut().find(|t| Some(t.sess.id) == a) {
                actions = view::show(ui, &mut tab.sess, &mut tab.view, &mut self.cache, &opt, focus);
            } else {
                self.empty_project(ui, &theme);
            }
            // Drop hint.
            if ctx.input(|i| !i.raw.hovered_files.is_empty()) {
                let r = ui.max_rect().shrink(20.0);
                ui.painter().rect_filled(r, 10.0, Color32::from_rgba_unmultiplied(0, 120, 212, 40));
                ui.painter().rect_stroke(r, 10.0, Stroke::new(2.0, ACCENT_HI), egui::StrokeKind::Inside);
                ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, "Drop to paste the file paths", egui::FontId::new(18.0, bold()), TEXT);
            }
        });
        for a in actions {
            match a {
                Action::Paste => self.paste(&ctx),
                Action::Copy(t) => ctx.copy_text(t),
                Action::OpenLink(link) => match link {
                    crate::links::Link::Url(u) => crate::links::open_url(&u),
                    crate::links::Link::File { path, line, col } => {
                        let cwd = self.active_tab().map(|t| t.sess.cwd.clone()).unwrap_or_default();
                        match crate::links::resolve(&path, &cwd) {
                            Some(full) => {
                                if let Err(e) = crate::links::open_file(&full, line, col, &self.s.editor, &self.s.editor_custom) {
                                    self.toast(format!("Couldn't open {}: {e}", full.display()), RED, &ctx);
                                }
                            }
                            None => self.toast(format!("{path} not found"), RED, &ctx),
                        }
                    }
                },
                Action::View(p) => {
                    let list = self.active_tab().map(|t| t.sess.pasted_images.clone()).unwrap_or_default();
                    let list = if list.contains(&p) { list } else { vec![p.clone()] };
                    self.viewer = Some(Viewer { path: p, list, zoom_fit: true });
                }
            }
        }

        self.viewer(&ctx);
        self.options_window(&ctx);
        self.options_autosave(&ctx, _frame);
        self.launch_dialog(&ctx);
        self.snippets_palette(&ctx);
        self.scripts_palette(&ctx);
        self.script_runs_ui(&ctx);
        self.plugins_window(&ctx);
        self.mcp_servers_window(&ctx);
        self.search_window(&ctx);
        self.clear_dialog(&ctx);
        self.model_switch_dialog(&ctx);
        self.confirm_close_dialog(&ctx);
        self.context_info_dialog(&ctx);
        self.remove_project_dialog(&ctx);
        self.group_dialog(&ctx);
        self.project_settings_dialog(&ctx);
        self.quit_dialog(&ctx);
        self.clear_find_history_dialog(&ctx);
        self.update_dialog(&ctx);
        self.show_toast(&ctx);
    }
}

impl App {
    /// Sessions still working on a task.
    fn busy_count(&self) -> usize {
        self.tabs.iter().filter(|t| t.sess.busy && t.sess.exited.is_none()).count()
    }

    /// "Clear search history?": Cancel | Clear.
    fn clear_find_history_dialog(&mut self, ctx: &egui::Context) {
        if !self.confirm_clear_find_history {
            return;
        }
        let n = self.s.find_history.len();
        let mut choice: Option<bool> = None;
        let m = egui::Modal::new(egui::Id::unique("confirm_clear_find_history")).frame(egui::Frame::new().fill(PANEL).corner_radius(10).stroke(Stroke::new(1.0, BORDER)).inner_margin(20)).show(ctx, |ui| {
            ui.set_width(400.0);
            ui.label(RichText::new("Clear search history?").size(18.0).family(bold()).color(Color32::WHITE));
            ui.add_space(10.0);
            let what = if n == 1 { "the recent search".to_string() } else { format!("all {n} recent searches") };
            ui.label(RichText::new(format!("This removes {what} from the list.")).color(TEXT));
            ui.add_space(14.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.add(egui::Button::new(RichText::new("Clear").color(Color32::WHITE)).fill(RED).min_size(Vec2::new(80.0, 28.0))).clicked() {
                    choice = Some(true);
                }
                if ui.add(egui::Button::new("Cancel").min_size(Vec2::new(70.0, 28.0))).clicked() {
                    choice = Some(false);
                }
            });
        });
        if m.should_close() && choice.is_none() {
            choice = Some(false);
        }
        if let Some(clear) = choice {
            self.confirm_clear_find_history = false;
            if clear {
                self.s.find_history.clear();
            }
        }
    }

    /// "There is a running task": Cancel | Close Anyway.
    fn quit_dialog(&mut self, ctx: &egui::Context) {
        if !self.confirm_quit {
            return;
        }
        let busy = self.busy_count();
        let mut choice: Option<bool> = None;
        let m = egui::Modal::new(egui::Id::unique("confirm_quit")).frame(egui::Frame::new().fill(PANEL).corner_radius(10).stroke(Stroke::new(1.0, BORDER)).inner_margin(20)).show(ctx, |ui| {
            ui.set_width(460.0);
            let title = if busy == 1 { "There is a running task".to_string() } else { format!("There are {busy} running tasks") };
            ui.label(RichText::new(title).size(18.0).family(bold()).color(Color32::WHITE));
            ui.add_space(10.0);
            let names: Vec<String> = self.tabs.iter().filter(|t| t.sess.busy && t.sess.exited.is_none()).map(|t| t.sess.display_name()).collect();
            for n in &names {
                ui.label(RichText::new(format!("• {n}")).color(ORANGE));
            }
            ui.add_space(6.0);
            ui.label(RichText::new("Closing AI Terminal now will stop it.").color(TEXT));
            ui.add_space(14.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.add(egui::Button::new(RichText::new("Close Anyway").color(Color32::WHITE)).fill(RED).min_size(Vec2::new(110.0, 28.0))).clicked() {
                    choice = Some(true);
                }
                if ui.add(egui::Button::new("Cancel").min_size(Vec2::new(70.0, 28.0))).clicked() {
                    choice = Some(false);
                }
            });
        });
        if m.should_close() && choice.is_none() {
            choice = Some(false);
        }
        if let Some(close) = choice {
            self.confirm_quit = false;
            if close {
                self.quit_confirmed = true;
                // Past minimize-to-tray too: this is a real quit.
                winsys::QUITTING.store(true, std::sync::atomic::Ordering::Relaxed);
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            } else {
                winsys::QUITTING.store(false, std::sync::atomic::Ordering::Relaxed);
                crate::update::RELAUNCH.store(false, std::sync::atomic::Ordering::Relaxed);
            }
        }
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Local UTC offset in seconds (from the OS clock; no time-zone crate needed).
fn utc_offset() -> i64 {
    #[cfg(windows)]
    {
        #[repr(C)]
        #[derive(Default)]
        struct SystemTime {
            year: u16,
            month: u16,
            dow: u16,
            day: u16,
            hour: u16,
            minute: u16,
            second: u16,
            ms: u16,
        }
        #[link(name = "kernel32")]
        extern "system" {
            fn GetLocalTime(t: *mut SystemTime);
            fn GetSystemTime(t: *mut SystemTime);
        }
        let (mut l, mut u) = (SystemTime::default(), SystemTime::default());
        unsafe {
            GetLocalTime(&mut l);
            GetSystemTime(&mut u);
        }
        let mins = |t: &SystemTime| t.hour as i64 * 60 + t.minute as i64;
        let mut d = mins(&l) - mins(&u);
        // Different calendar days: the offset wrapped around midnight.
        if (l.year, l.month, l.day) > (u.year, u.month, u.day) {
            d += 24 * 60;
        } else if (l.year, l.month, l.day) < (u.year, u.month, u.day) {
            d -= 24 * 60;
        }
        d * 60
    }
    #[cfg(not(windows))]
    {
        0
    }
}

/// Local day number (days since 1970-01-01 in local time).
fn local_day(secs: u64) -> i64 {
    (secs as i64 + utc_offset()).div_euclid(86400)
}

fn day_label(today: i64, day: i64) -> String {
    match today - day {
        i64::MIN..=0 => "Today".into(),
        1 => "Yesterday".into(),
        2..=6 => "This week".into(),
        7..=30 => "This month".into(),
        _ => "Older".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn day_groups() {
        assert_eq!(day_label(100, 100), "Today");
        assert_eq!(day_label(100, 99), "Yesterday");
        assert_eq!(day_label(100, 95), "This week");
        assert_eq!(day_label(100, 80), "This month");
        assert_eq!(day_label(100, 1), "Older");
        assert!(utc_offset().abs() <= 14 * 3600);
    }
}

fn is_home_dir(p: &Path) -> bool {
    std::env::var_os("USERPROFILE").is_some_and(|h| Path::new(&h) == p)
}

/// Short status shown at the right of a session row: working / waiting for permission / needs you.
fn status_label(row: &Row) -> Option<&'static str> {
    if row.busy {
        return Some("working…");
    }
    let m = row.attention_msg.as_ref()?;
    Some(if m.to_lowercase().contains("permission") { "permission" } else { "needs you" })
}
