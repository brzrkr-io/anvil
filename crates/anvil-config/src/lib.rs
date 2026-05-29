//! User configuration, loaded from `~/.config/anvil/config.toml`.
//!
//! The file is TOML. All fields are optional; an absent field keeps the
//! struct default, preserving the Zig "absent field = default" semantics.
//!
//! After deserialization, out-of-range values are clamped via [`Config::clamp`]
//! — matching the Zig `config.clamp()` logic verbatim.

use std::{fs, path::PathBuf, time::SystemTime};

use serde::Deserialize;
use thiserror::Error;

pub use anvil_theme::{AnsiOverrides, ThemeOverrides};

// ── Error type ───────────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("config parse error: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("config file too large")]
    TooLarge,
    #[error("config i/o error: {0}")]
    Io(#[from] std::io::Error),
}

// ── Sub-structs ───────────────────────────────────────────────────────────────

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CursorStyle {
    #[default]
    Block,
    Bar,
    Underline,
}

/// UI (chrome) font configuration — used by the proportional text path.
///
/// Nested under `[font.ui]` in `config.toml`.  All fields are optional;
/// missing fields fall back to the defaults below.
///
/// ```toml
/// [font.ui]
/// family = "SF Pro Text"
/// size = 13.0
/// weight_regular = "regular"
/// weight_strong  = "semibold"
/// ```
#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct UiFontCfg {
    /// Font family name as recognised by CoreText (e.g. `"SF Pro Text"`).
    pub family: String,
    /// Default size in logical pt. Per-surface sizes are defined in
    /// `anvil_render::ui_text_sizes`.
    pub size: f64,
    /// Weight token for regular-weight surfaces.  Currently unused beyond
    /// being stored; weight selection is binary (regular vs. strong).
    pub weight_regular: String,
    /// Weight token for stronger-weight surfaces (medium, semibold).
    pub weight_strong: String,
}

impl Default for UiFontCfg {
    fn default() -> Self {
        UiFontCfg {
            family: "SF Pro Text".into(),
            size: 13.0,
            weight_regular: "regular".into(),
            weight_strong: "semibold".into(),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FontCfg {
    pub family: String,
    pub size: f64,
    /// Proportional UI font settings.  Nested as `[font.ui]` in TOML.
    pub ui: UiFontCfg,
}

impl Default for FontCfg {
    fn default() -> Self {
        FontCfg {
            family: "BlexMono Nerd Font Mono".into(),
            // BlexMono is IBM Plex Mono patched with Nerd Font icons. Its
            // letterforms are visually compact; 15pt reads
            // closer to other terminals' 14pt and gives the grid real presence
            // on Retina without crowding columns. Override in TOML if needed.
            size: 15.0,
            ui: UiFontCfg::default(),
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CursorCfg {
    pub style: CursorStyle,
    pub blink: bool,
    /// Optional override for the cursor color as a `#rrggbb` hex string (AA7).
    /// When absent, the theme's `accent_primary` is used.
    pub color: Option<String>,
}

// CursorCfg needs blink to default to true, but #[serde(default)] on the
// struct calls Default::default() which gives false. Use field-level default.
impl CursorCfg {
    fn new_default() -> Self {
        CursorCfg {
            style: CursorStyle::Block,
            blink: true,
            color: None,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StartupLayout {
    /// Pick IDE mode in project directories and straight-terminal elsewhere.
    #[default]
    Auto,
    /// Force the full editor/explorer IDE surface at startup.
    Ide,
    /// Force a straight terminal at startup; Cmd+Shift+E can restore IDE mode.
    Terminal,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WindowCfg {
    pub width: f64,
    pub height: f64,
}

impl Default for WindowCfg {
    fn default() -> Self {
        WindowCfg {
            width: 1440.0,
            height: 900.0,
        }
    }
}

/// A single custom prompt segment: a label and a shell command.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CustomPromptSegment {
    pub label: String,
    pub command: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PromptCfg {
    pub enabled: bool,
    pub transient: bool,
    pub custom: Vec<CustomPromptSegment>,
}

impl Default for PromptCfg {
    fn default() -> Self {
        PromptCfg {
            enabled: true,
            transient: true,
            custom: Vec::new(),
        }
    }
}

/// Chord strings for tab/pane/search/UI actions. Live-reloadable.
/// Each string is parsed via [`parse_chord`]; an unparseable string falls
/// back to that field's default.
#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Keybindings {
    pub new_tab: String,
    pub close_tab: String,
    pub next_tab: String,
    pub prev_tab: String,
    pub tab_1: String,
    pub tab_2: String,
    pub tab_3: String,
    pub tab_4: String,
    pub tab_5: String,
    pub tab_6: String,
    pub tab_7: String,
    pub tab_8: String,
    pub tab_9: String,
    pub search_open: String,
    pub search_next: String,
    pub search_prev: String,
    pub hud_toggle: String,
    pub cheatsheet_toggle: String,
    pub split_right: String,
    pub split_down: String,
    pub close_pane: String,
    pub focus_left: String,
    pub focus_right: String,
    pub focus_up: String,
    pub focus_down: String,
    pub fold_block: String,
    pub toggle_theme: String,
    /// Approve the topmost pending approval when the HUD is visible.
    pub agent_approve: String,
    /// Start a new agent run via the task-handoff endpoint.
    pub agent_start: String,
    /// Cycle layout mode: Terminal ↔ Ide.
    pub layout_mode_toggle: String,
    /// Toggle the left explorer dock while staying in IDE mode.
    pub left_dock_toggle: String,
    /// Open the recent-files palette without conflicting with layout switching.
    pub recent_files: String,
    /// Open a new native editor pane (NE15: nvim path removed; this is the
    /// only editor path).
    pub editor_new: String,
    /// Cmd+Shift+F: open the project-wide search overlay.
    pub project_search: String,
}

impl Default for Keybindings {
    fn default() -> Self {
        Keybindings {
            new_tab: "cmd+t".into(),
            close_tab: "cmd+w".into(),
            next_tab: "cmd+shift+]".into(),
            prev_tab: "cmd+shift+[".into(),
            tab_1: "cmd+1".into(),
            tab_2: "cmd+2".into(),
            tab_3: "cmd+3".into(),
            tab_4: "cmd+4".into(),
            tab_5: "cmd+5".into(),
            tab_6: "cmd+6".into(),
            tab_7: "cmd+7".into(),
            tab_8: "cmd+8".into(),
            tab_9: "cmd+9".into(),
            search_open: "cmd+f".into(),
            search_next: "cmd+g".into(),
            search_prev: "cmd+shift+g".into(),
            hud_toggle: "cmd+\\".into(),
            cheatsheet_toggle: "cmd+/".into(),
            split_right: "cmd+d".into(),
            split_down: "cmd+shift+d".into(),
            close_pane: "cmd+shift+q".into(),
            focus_left: "cmd+shift+h".into(),
            focus_right: "cmd+shift+l".into(),
            focus_up: "cmd+shift+k".into(),
            focus_down: "cmd+shift+j".into(),
            fold_block: "cmd+.".into(),
            toggle_theme: "cmd+shift+t".into(),
            agent_approve: "cmd+return".into(),
            agent_start: "cmd+shift+return".into(),
            layout_mode_toggle: "cmd+shift+e".into(),
            left_dock_toggle: "cmd+b".into(),
            recent_files: "cmd+opt+p".into(),
            editor_new: "cmd+e".into(),
            project_search: "cmd+shift+f".into(),
        }
    }
}

// ── Config ────────────────────────────────────────────────────────────────────

// ── ThemeSectionCfg (AA3) ─────────────────────────────────────────────────────

/// Per-token overrides inside `[theme.tokens]` (AA3).
///
/// Each field is an optional `#rrggbb` hex string applied on top of the
/// base theme.  Fields here mirror the most-useful `Theme` tokens.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ThemeTokenOverrides {
    pub background: Option<String>,
    pub foreground: Option<String>,
    pub accent: Option<String>,
    pub verified: Option<String>,
    pub failure: Option<String>,
    pub attention: Option<String>,
    pub agent: Option<String>,
    pub info: Option<String>,
}

/// A `[theme]` table section in `config.toml` (AA3).
///
/// Supports two equivalent spellings in TOML:
///
/// ```toml
/// # simple — bare string on one line
/// theme = "system"
///
/// # extended — table with optional per-token overrides
/// [theme]
/// override = "mineral-dark"
///
/// [theme.tokens]
/// accent = "#ff0000"
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ThemeSectionCfg {
    /// Base theme name; empty string means "use the hardcoded default".
    #[serde(rename = "override")]
    pub theme_override: String,
    /// Per-token color overrides applied on top of the base theme.
    pub tokens: ThemeTokenOverrides,
}

/// Accepts either a bare string (`theme = "name"`) or a `[theme]` table.
/// The untagged representation means serde tries each variant in order.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum ThemeEntry {
    /// `theme = "system"` — simple bare string.
    Name(String),
    /// `[theme]\n override = "mineral-dark"` — extended table.
    Section(ThemeSectionCfg),
}

impl ThemeEntry {
    /// The resolved base theme name; falls back to `"system"` when absent.
    pub fn theme_name(&self) -> &str {
        match self {
            ThemeEntry::Name(s) => s.as_str(),
            ThemeEntry::Section(sec) => {
                if sec.theme_override.is_empty() {
                    "system"
                } else {
                    sec.theme_override.as_str()
                }
            }
        }
    }

    /// Per-token overrides; only available when the table form is used.
    pub fn token_overrides(&self) -> Option<&ThemeTokenOverrides> {
        match self {
            ThemeEntry::Name(_) => None,
            ThemeEntry::Section(sec) => Some(&sec.tokens),
        }
    }
}

impl Default for ThemeEntry {
    fn default() -> Self {
        ThemeEntry::Name("system".into())
    }
}

// ── EditorCfg ─────────────────────────────────────────────────────────────────

fn default_italic_comments() -> bool {
    true
}

/// Editor-specific configuration options.
#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct EditorCfg {
    /// When `true`, automatically save dirty editor buffers when the window
    /// loses focus (`windowDidResignKey`).  Silent — errors are logged to
    /// stderr.  Default: `false`.
    pub save_on_blur: bool,
    /// When `true`, comment tokens are rendered in italic. Default: `true` (AA5).
    #[serde(default = "default_italic_comments")]
    pub italic_comments: bool,
    /// When `true`, keyword tokens are rendered in bold. Default: `false` (AA6).
    pub bold_keywords: bool,
}

impl Default for EditorCfg {
    fn default() -> Self {
        EditorCfg {
            save_on_blur: false,
            italic_comments: true,
            bold_keywords: false,
        }
    }
}

/// Neovim/LazyVim terminal-pane integration.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct NvimCfg {
    /// Optional NVIM_APPNAME. Set to "LazyVim" to launch that config tree.
    pub appname: String,
    /// Export Anvil theme tokens and send safe startup `--cmd` theme hints.
    pub theme_sync: bool,
    /// Optional colorscheme name to request after nvim starts.
    pub colorscheme: String,
}

impl Default for NvimCfg {
    fn default() -> Self {
        NvimCfg {
            appname: String::new(),
            theme_sync: true,
            colorscheme: String::new(),
        }
    }
}

/// Explorer-specific configuration options (Y1).
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ExplorerCfg {
    /// When `true`, expanding a directory at depth > 2 collapses sibling
    /// directories at the same depth.  Default: `false`.
    pub auto_collapse_siblings: bool,
}

/// Per-language syntax color overrides (AA4).
///
/// Each field is an optional `#rrggbb` hex string.  Applied on top of the
/// theme's `SyntaxTheme` for the named language.
///
/// Example in `config.toml`:
/// ```toml
/// [syntax.rust]
/// keyword = "#ff8800"
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LangSyntaxOverride {
    pub keyword: Option<String>,
    pub string: Option<String>,
    pub number: Option<String>,
    pub comment: Option<String>,
    pub function: Option<String>,
    pub type_: Option<String>,
    pub variable: Option<String>,
    pub operator: Option<String>,
    pub punctuation: Option<String>,
}

/// A named task defined in `[tasks]` (Y12).
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskDef {
    /// Shell command to run in the drawer terminal.
    pub cmd: String,
}

/// Top-level configuration. Every field is optional in TOML; missing fields
/// keep their defaults.
#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub scrollback: usize,
    pub font: FontCfg,
    #[serde(default = "CursorCfg::new_default")]
    pub cursor: CursorCfg,
    pub window: WindowCfg,
    /// Base theme name or extended `[theme]` table (AA3).
    /// Simple: `theme = "system"`.
    /// Extended: `[theme]\n override = "mineral-dark"\n [theme.tokens]\n accent = "#ff0000"`.
    pub theme: ThemeEntry,
    pub layout_mode: StartupLayout,
    pub theme_overrides: ThemeOverrides,
    pub keybindings: Keybindings,
    pub shell_integration: bool,
    pub prompt: PromptCfg,
    pub editor: EditorCfg,
    pub nvim: NvimCfg,
    pub explorer: ExplorerCfg,
    /// Per-language syntax color overrides (AA4). Key is the language name
    /// as returned by the syntax layer (e.g. `"rust"`, `"python"`).
    #[serde(default)]
    pub syntax: std::collections::HashMap<String, LangSyntaxOverride>,
    /// Named tasks (`[tasks.<name>]`). Y12.
    #[serde(default)]
    pub tasks: std::collections::HashMap<String, TaskDef>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            scrollback: 100_000,
            font: FontCfg::default(),
            cursor: CursorCfg::new_default(),
            window: WindowCfg::default(),
            theme: ThemeEntry::default(),
            layout_mode: StartupLayout::Ide,
            theme_overrides: ThemeOverrides::default(),
            keybindings: Keybindings::default(),
            shell_integration: true,
            prompt: PromptCfg::default(),
            editor: EditorCfg::default(),
            nvim: NvimCfg::default(),
            explorer: ExplorerCfg::default(),
            syntax: std::collections::HashMap::new(),
            tasks: std::collections::HashMap::new(),
        }
    }
}

impl Config {
    /// Pull every out-of-range value back to a usable minimum.
    /// Mirrors the Zig `config.clamp()` exactly.
    pub fn clamp(&mut self) {
        if self.scrollback < 1 {
            self.scrollback = 1;
        }
        // NaN-safe: replace if less than the minimum or not finite.
        if self.font.size < 4.0 || !self.font.size.is_finite() {
            self.font.size = 14.0;
        }
        if self.window.width < 200.0 || !self.window.width.is_finite() {
            self.window.width = 1024.0;
        }
        if self.window.height < 150.0 || !self.window.height.is_finite() {
            self.window.height = 640.0;
        }
    }
}

// ── Parse / load ─────────────────────────────────────────────────────────────

/// Maximum config file size (1 MiB), mirroring the Zig 1<<20 limit.
const MAX_CONFIG_BYTES: usize = 1 << 20;

/// Parse a TOML source string into a [`Config`].
/// On a TOML error, returns [`ConfigError::Parse`].
pub fn parse_str(source: &str) -> Result<Config, ConfigError> {
    let mut cfg: Config = toml::from_str(source)?;
    cfg.clamp();
    Ok(cfg)
}

/// Resolve the absolute path to the config file: `~/.config/anvil/config.toml`.
/// Returns `None` if `$HOME` is unset.
pub fn resolve_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    let mut p = PathBuf::from(home);
    p.push(".config/anvil/config.toml");
    Some(p)
}

/// Read and parse the config file at `path`. A missing file or any
/// read/parse error yields [`Config::default`] — running the app is never
/// blocked by a bad config.
pub fn load(path: &std::path::Path) -> Config {
    let source = match fs::read(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Config::default(),
        Err(e) => {
            eprintln!("anvil: cannot read config: {e}");
            return Config::default();
        }
        Ok(bytes) if bytes.len() >= MAX_CONFIG_BYTES => {
            eprintln!("anvil: config file too large, using defaults");
            return Config::default();
        }
        Ok(bytes) => match String::from_utf8(bytes) {
            Ok(s) => s,
            Err(_) => {
                eprintln!("anvil: config file is not valid UTF-8, using defaults");
                return Config::default();
            }
        },
    };
    match parse_str(&source) {
        Ok(cfg) => cfg,
        Err(e) => {
            eprintln!("anvil: config parse error:\n{e}");
            Config::default()
        }
    }
}

// ── Watcher ───────────────────────────────────────────────────────────────────

/// Polls the config file's modification time so the render loop can reload it
/// without a file-watcher thread. Cheap: one `metadata` call per poll.
/// Mirrors the Zig `config.Watcher` mtime-poll strategy exactly.
pub struct Watcher {
    /// Borrowed path — must outlive the Watcher.
    pub path: PathBuf,
    /// Last observed mtime, or `None` if nothing has been seen yet.
    last_mtime: Option<SystemTime>,
    /// Whether `last_mtime` has been initialized (even with absence).
    initialized: bool,
}

impl Watcher {
    pub fn new(path: PathBuf) -> Self {
        Watcher {
            path,
            last_mtime: None,
            initialized: false,
        }
    }

    /// Current mtime of the file, or `None` if it cannot be stat'd.
    fn mtime(&self) -> Option<SystemTime> {
        fs::metadata(&self.path)
            .ok()
            .and_then(|m| m.modified().ok())
    }

    /// If the file changed since the last call, load and return the new config;
    /// otherwise return `None`. A parse failure still advances the recorded
    /// mtime so the error is reported once, not every poll.
    pub fn poll(&mut self) -> Option<Config> {
        let current = self.mtime();
        if self.initialized && current == self.last_mtime {
            return None;
        }
        self.last_mtime = current;
        self.initialized = true;
        if current.is_none() {
            // File was removed (or never existed) -> defaults.
            return Some(Config::default());
        }
        Some(load(&self.path))
    }
}

// ── Chord parsing ─────────────────────────────────────────────────────────────

/// A parsed key chord: modifier flags plus one key character (lowercased for
/// ASCII letters). Mirrors the Zig `Chord` struct exactly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Chord {
    pub cmd: bool,
    pub shift: bool,
    pub ctrl: bool,
    pub opt: bool,
    /// The key character (lowercased ASCII letter; or the literal char otherwise).
    pub key: char,
}

/// Parse a chord string like `"cmd+shift+]"` or `"cmd+t"`.
/// Modifier tokens are `cmd`/`shift`/`ctrl`/`opt` (case-insensitive);
/// the final token must be a single character. Returns `None` on a malformed
/// string. Mirrors the Zig `parseChord` logic exactly.
pub fn parse_chord(s: &str) -> Option<Chord> {
    let mut ch = Chord::default();
    let mut have_key = false;
    for tok_raw in s.split('+') {
        let tok = tok_raw.trim();
        if tok.is_empty() {
            return None;
        }
        if tok.eq_ignore_ascii_case("cmd") {
            ch.cmd = true;
        } else if tok.eq_ignore_ascii_case("shift") {
            ch.shift = true;
        } else if tok.eq_ignore_ascii_case("ctrl") {
            ch.ctrl = true;
        } else if tok.eq_ignore_ascii_case("opt") {
            ch.opt = true;
        } else {
            // Must be the key — exactly one ASCII character, and last.
            let mut chars = tok.chars();
            let first = chars.next()?;
            if chars.next().is_some() {
                // More than one char — but wait: "cmd+/" is `[cmd, /]` and
                // "/" is a single char. "cmd+ab" has tok="ab" which has two
                // chars. Reject.
                return None;
            }
            if have_key {
                // Two keys — reject.
                return None;
            }
            ch.key = first.to_ascii_lowercase();
            have_key = true;
        }
    }
    if !have_key {
        return None;
    }
    Some(ch)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── parse_str / full config ───────────────────────────────────────────────

    #[test]
    fn parses_a_full_config() {
        let src = r##"
scrollback = 5000
theme = "mineral-light"

[font]
family = "Menlo"
size = 16.0

[cursor]
style = "bar"
blink = false

[window]
width = 800.0
height = 600.0

[theme_overrides]
accent = "#3aa0a8"
"##;
        let cfg = parse_str(src).unwrap();
        assert_eq!(cfg.scrollback, 5000);
        assert_eq!(cfg.font.family, "Menlo");
        assert!((cfg.font.size - 16.0).abs() < f64::EPSILON);
        assert_eq!(cfg.cursor.style, CursorStyle::Bar);
        assert!(!cfg.cursor.blink);
        assert_eq!(cfg.theme.theme_name(), "mineral-light");
        assert_eq!(cfg.layout_mode, StartupLayout::Ide); // default when not set
        assert_eq!(cfg.theme_overrides.accent.as_deref(), Some("#3aa0a8"));
    }

    #[test]
    fn partial_config_keeps_defaults_for_absent_fields() {
        let cfg = parse_str("scrollback = 200").unwrap();
        assert_eq!(cfg.scrollback, 200);
        assert_eq!(cfg.font.family, "BlexMono Nerd Font Mono");
        assert_eq!(cfg.cursor.style, CursorStyle::Block);
        assert_eq!(cfg.theme.theme_name(), "system");
        assert_eq!(cfg.layout_mode, StartupLayout::Ide); // default when not set
    }

    #[test]
    fn layout_mode_parses_auto_terminal_and_ide() {
        assert_eq!(
            parse_str("layout_mode = \"auto\"").unwrap().layout_mode,
            StartupLayout::Auto
        );
        assert_eq!(
            parse_str("layout_mode = \"terminal\"").unwrap().layout_mode,
            StartupLayout::Terminal
        );
        assert_eq!(
            parse_str("layout_mode = \"ide\"").unwrap().layout_mode,
            StartupLayout::Ide
        );
    }

    #[test]
    fn malformed_toml_returns_parse_error() {
        assert!(parse_str("scrollback =").is_err());
        assert!(parse_str("[font\nfamily = \"X\"").is_err());
    }

    #[test]
    fn unknown_field_returns_parse_error() {
        // Restores Zig behavior: unknown keys are rejected, catching user typos.
        assert!(parse_str("nonsense = 1").is_err());
        // Unknown key inside a sub-table is also rejected.
        assert!(parse_str("[font]\ntypo_key = 1").is_err());
    }

    #[test]
    fn out_of_range_values_are_clamped() {
        let src = r#"
scrollback = 0

[font]
size = 0.0

[window]
width = 1.0
height = 1.0
"#;
        let cfg = parse_str(src).unwrap();
        assert_eq!(cfg.scrollback, 1);
        assert!((cfg.font.size - 14.0).abs() < f64::EPSILON);
        assert!((cfg.window.width - 1024.0).abs() < f64::EPSILON);
        assert!((cfg.window.height - 640.0).abs() < f64::EPSILON);
    }

    #[test]
    fn defaults_has_expected_values() {
        let cfg = Config::default();
        assert_eq!(cfg.scrollback, 100_000);
        assert_eq!(cfg.font.family, "BlexMono Nerd Font Mono");
        assert!(cfg.cursor.blink);
        assert_eq!(cfg.theme.theme_name(), "system");
        assert_eq!(cfg.layout_mode, StartupLayout::Ide);
        assert!(cfg.shell_integration);
        assert!(cfg.prompt.enabled);
        assert!(cfg.prompt.transient);
        assert!(cfg.prompt.custom.is_empty());
    }

    #[test]
    fn nvim_defaults_enable_theme_sync_without_forced_appname() {
        let cfg = Config::default();
        assert!(cfg.nvim.theme_sync);
        assert!(cfg.nvim.appname.is_empty());
        assert!(cfg.nvim.colorscheme.is_empty());
    }

    #[test]
    fn nvim_section_parses_lazyvim_options() {
        let cfg = parse_str(
            r#"
[nvim]
appname = "LazyVim"
theme_sync = false
colorscheme = "catppuccin-mocha"
"#,
        )
        .unwrap();

        assert_eq!(cfg.nvim.appname, "LazyVim");
        assert!(!cfg.nvim.theme_sync);
        assert_eq!(cfg.nvim.colorscheme, "catppuccin-mocha");
    }

    #[test]
    fn load_of_missing_file_yields_defaults() {
        let cfg = load(std::path::Path::new("/nonexistent/anvil-test-config.toml"));
        assert_eq!(cfg.scrollback, 100_000);
    }

    #[test]
    fn load_of_unreadable_file_yields_defaults() {
        // Write a file, make it unreadable, then load it.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, "scrollback = 99").unwrap();
        // chmod 000 so the read fails with a non-NotFound error.
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();
        let cfg = load(&path);
        // Restore permissions so tempdir can clean up.
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(cfg.scrollback, 100_000);
    }

    #[test]
    fn load_of_oversized_file_yields_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.toml");
        // Write MAX_CONFIG_BYTES + 1 bytes.
        let big = vec![b'#'; MAX_CONFIG_BYTES + 1];
        fs::write(&path, &big).unwrap();
        let cfg = load(&path);
        assert_eq!(cfg.scrollback, 100_000);
    }

    #[test]
    fn load_of_non_utf8_file_yields_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.toml");
        // Write invalid UTF-8 bytes.
        fs::write(&path, [0xFF, 0xFE, 0x00]).unwrap();
        let cfg = load(&path);
        assert_eq!(cfg.scrollback, 100_000);
    }

    #[test]
    fn parse_chord_ctrl_and_opt_modifiers() {
        let c = parse_chord("ctrl+a").unwrap();
        assert!(c.ctrl);
        assert_eq!(c.key, 'a');

        let o = parse_chord("opt+b").unwrap();
        assert!(o.opt);
        assert_eq!(o.key, 'b');

        let co = parse_chord("ctrl+opt+c").unwrap();
        assert!(co.ctrl && co.opt);
        assert_eq!(co.key, 'c');
    }

    // ── Watcher ───────────────────────────────────────────────────────────────

    #[test]
    fn watcher_detects_change_and_reloads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, "scrollback = 11").unwrap();

        let mut w = Watcher::new(path.clone());

        let first = w.poll().expect("expected reload on first poll");
        assert_eq!(first.scrollback, 11);

        // No change -> no reload.
        assert!(w.poll().is_none());
    }

    #[test]
    fn watcher_file_removed_returns_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, "scrollback = 42").unwrap();

        let mut w = Watcher::new(path.clone());

        let first = w.poll().expect("expected reload");
        assert_eq!(first.scrollback, 42);

        fs::remove_file(&path).unwrap();

        let second = w.poll().expect("expected defaults after removal");
        assert_eq!(second.scrollback, 100_000);
    }

    #[test]
    fn watcher_parse_failure_advances_mtime_not_re_reported() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, "scrollback = 77").unwrap();

        let mut w = Watcher::new(path.clone());

        let first = w.poll().expect("expected reload");
        assert_eq!(first.scrollback, 77);

        // Sleep 10 ms to ensure distinct mtime on overwrite.
        std::thread::sleep(std::time::Duration::from_millis(10));

        // Overwrite with malformed TOML.
        fs::write(&path, "scrollback =").unwrap();

        let second = w.poll().expect("expected defaults after parse failure");
        assert_eq!(second.scrollback, 100_000); // falls back to defaults

        // Third poll: file unchanged, mtime already recorded -> None.
        assert!(w.poll().is_none());
    }

    // ── parse_chord ───────────────────────────────────────────────────────────

    #[test]
    fn parse_chord_parses_modifiers_and_key() {
        let c = parse_chord("cmd+shift+]").unwrap();
        assert!(c.cmd && c.shift && !c.ctrl && !c.opt);
        assert_eq!(c.key, ']');

        let t = parse_chord("cmd+t").unwrap();
        assert!(t.cmd);
        assert_eq!(t.key, 't');

        // Case-insensitive; letter lowercased.
        let u = parse_chord("CMD+T").unwrap();
        assert_eq!(u.key, 't');
    }

    #[test]
    fn parse_chord_rejects_malformed_strings() {
        assert!(parse_chord("").is_none());
        assert!(parse_chord("cmd+").is_none());
        assert!(parse_chord("cmd+ab").is_none()); // key not single char
        assert!(parse_chord("cmd").is_none()); // no key
        assert!(parse_chord("cmd+t+w").is_none()); // two keys
    }

    // ── Keybindings ───────────────────────────────────────────────────────────

    #[test]
    fn config_parses_keybindings_override() {
        let src = r#"
[keybindings]
new_tab = "ctrl+n"
"#;
        let cfg = parse_str(src).unwrap();
        assert_eq!(cfg.keybindings.new_tab, "ctrl+n");
        assert_eq!(cfg.keybindings.close_tab, "cmd+w"); // pane close is cmd+shift+q (W1#18)
    }

    // NE15: Cmd+E is the sole editor pane keybind (nvim path removed).
    #[test]
    fn editor_new_default_chord_is_cmd_e() {
        let kb = Keybindings::default();
        assert_eq!(kb.editor_new, "cmd+e");
    }

    #[test]
    fn left_dock_toggle_default_chord_is_cmd_b() {
        let kb = Keybindings::default();
        assert_eq!(kb.left_dock_toggle, "cmd+b");
    }

    #[test]
    fn recent_files_default_does_not_conflict_with_layout_toggle() {
        let kb = Keybindings::default();
        assert_eq!(kb.layout_mode_toggle, "cmd+shift+e");
        assert_eq!(kb.recent_files, "cmd+opt+p");
        assert_ne!(kb.recent_files, kb.layout_mode_toggle);
    }

    #[test]
    fn config_parses_recent_files_keybinding_override() {
        let src = r#"
[keybindings]
recent_files = "cmd+shift+p"
"#;
        let cfg = parse_str(src).unwrap();
        assert_eq!(cfg.keybindings.recent_files, "cmd+shift+p");
        assert_eq!(cfg.keybindings.layout_mode_toggle, "cmd+shift+e");
    }

    #[test]
    fn config_parses_search_keybinding_override() {
        let src = r#"
[keybindings]
search_open = "ctrl+s"
"#;
        let cfg = parse_str(src).unwrap();
        assert_eq!(cfg.keybindings.search_open, "ctrl+s");
        assert_eq!(cfg.keybindings.search_next, "cmd+g"); // default
    }

    // ── shell_integration ─────────────────────────────────────────────────────

    #[test]
    fn config_parses_shell_integration() {
        let on = parse_str("scrollback = 100").unwrap();
        assert!(on.shell_integration); // default

        let off = parse_str("shell_integration = false").unwrap();
        assert!(!off.shell_integration);
    }

    // ── Prompt ────────────────────────────────────────────────────────────────

    #[test]
    fn config_defaults_prompt_section_on() {
        let cfg = parse_str("scrollback = 100").unwrap();
        assert!(cfg.prompt.enabled);
        assert!(cfg.prompt.transient);
        assert!(cfg.prompt.custom.is_empty());
    }

    #[test]
    fn config_parses_custom_prompt_segment() {
        let src = r#"
[[prompt.custom]]
label = "aws"
command = "echo prod"
"#;
        let cfg = parse_str(src).unwrap();
        assert_eq!(cfg.prompt.custom.len(), 1);
        assert_eq!(cfg.prompt.custom[0].label, "aws");
        assert_eq!(cfg.prompt.custom[0].command, "echo prod");
    }

    // ── resolve_path ──────────────────────────────────────────────────────────

    #[test]
    fn resolve_path_contains_config_toml() {
        if let Some(p) = resolve_path() {
            let s = p.to_str().unwrap();
            assert!(s.ends_with("/.config/anvil/config.toml"), "got: {s}");
        }
        // If $HOME is unset the function returns None — that's fine.
    }

    // ── Y1: ExplorerCfg ──────────────────────────────────────────────────────

    #[test]
    fn explorer_auto_collapse_siblings_defaults_false() {
        let cfg = Config::default();
        assert!(
            !cfg.explorer.auto_collapse_siblings,
            "auto_collapse_siblings must default to false (Y1)"
        );
    }

    #[test]
    fn explorer_auto_collapse_siblings_parses_from_toml() {
        let src = "[explorer]\nauto_collapse_siblings = true\n";
        let cfg = parse_str(src).unwrap();
        assert!(
            cfg.explorer.auto_collapse_siblings,
            "auto_collapse_siblings should be true when set in TOML (Y1)"
        );
    }

    // ── Y12: Tasks ───────────────────────────────────────────────────────────

    #[test]
    fn tasks_empty_by_default() {
        let cfg = Config::default();
        assert!(cfg.tasks.is_empty(), "tasks must be empty by default (Y12)");
    }

    #[test]
    fn tasks_parses_from_toml() {
        let src = r#"
[tasks.test]
cmd = "cargo test"

[tasks.build]
cmd = "cargo build"
"#;
        let cfg = parse_str(src).unwrap();
        assert_eq!(cfg.tasks.len(), 2, "should parse 2 tasks (Y12)");
        assert_eq!(cfg.tasks["test"].cmd, "cargo test");
        assert_eq!(cfg.tasks["build"].cmd, "cargo build");
    }

    // ── AA3: [theme] table section ────────────────────────────────────────────

    #[test]
    fn theme_bare_string_still_parses() {
        let cfg = parse_str("theme = \"mineral-dark\"").unwrap();
        assert_eq!(cfg.theme.theme_name(), "mineral-dark", "AA3: bare string");
        assert!(cfg.theme.token_overrides().is_none());
    }

    #[test]
    fn theme_section_override_parses() {
        let src = r#"
[theme]
override = "ember-light"
"#;
        let cfg = parse_str(src).unwrap();
        assert_eq!(cfg.theme.theme_name(), "ember-light", "AA3: table override");
    }

    #[test]
    fn theme_section_token_overrides_parse() {
        let src = r##"
[theme]
override = "ember-dark"

[theme.tokens]
accent = "#ff0000"
"##;
        let cfg = parse_str(src).unwrap();
        assert_eq!(cfg.theme.theme_name(), "ember-dark");
        let toks = cfg.theme.token_overrides().expect("tokens must be present");
        assert_eq!(
            toks.accent.as_deref(),
            Some("#ff0000"),
            "AA3: per-token accent"
        );
    }

    #[test]
    fn theme_section_empty_override_falls_back_to_default() {
        // [theme] with no override key → theme_name() returns the hardcoded default.
        let src = "[theme]\n";
        let cfg = parse_str(src).unwrap();
        assert_eq!(
            cfg.theme.theme_name(),
            "system",
            "AA3: empty override = default"
        );
    }

    // ── AA4: per-language syntax overrides ────────────────────────────────────

    #[test]
    fn syntax_override_parses_for_rust() {
        let src = r##"
[syntax.rust]
keyword = "#ff8800"
string = "#44aa44"
"##;
        let cfg = parse_str(src).unwrap();
        let rust = cfg
            .syntax
            .get("rust")
            .expect("rust syntax override must parse (AA4)");
        assert_eq!(rust.keyword.as_deref(), Some("#ff8800"));
        assert_eq!(rust.string.as_deref(), Some("#44aa44"));
        assert!(rust.number.is_none(), "absent field must be None");
    }

    #[test]
    fn syntax_empty_by_default() {
        let cfg = Config::default();
        assert!(
            cfg.syntax.is_empty(),
            "syntax must be empty by default (AA4)"
        );
    }

    // ── AA5/AA6: editor italic_comments / bold_keywords ──────────────────────

    #[test]
    fn editor_italic_comments_defaults_true() {
        let cfg = Config::default();
        assert!(
            cfg.editor.italic_comments,
            "italic_comments must default to true (AA5)"
        );
    }

    #[test]
    fn editor_bold_keywords_defaults_false() {
        let cfg = Config::default();
        assert!(
            !cfg.editor.bold_keywords,
            "bold_keywords must default to false (AA6)"
        );
    }

    #[test]
    fn editor_italic_bold_parse_from_toml() {
        let src = "[editor]\nitalic_comments = false\nbold_keywords = true\n";
        let cfg = parse_str(src).unwrap();
        assert!(!cfg.editor.italic_comments, "AA5: italic_comments = false");
        assert!(cfg.editor.bold_keywords, "AA6: bold_keywords = true");
    }

    // ── AA7: cursor color override ────────────────────────────────────────────

    #[test]
    fn cursor_color_defaults_none() {
        let cfg = Config::default();
        assert!(
            cfg.cursor.color.is_none(),
            "cursor.color must default to None (AA7)"
        );
    }

    #[test]
    fn cursor_color_parses_from_toml() {
        let src = "[cursor]\ncolor = \"#ff4400\"\n";
        let cfg = parse_str(src).unwrap();
        assert_eq!(
            cfg.cursor.color.as_deref(),
            Some("#ff4400"),
            "AA7: cursor color"
        );
    }
}
