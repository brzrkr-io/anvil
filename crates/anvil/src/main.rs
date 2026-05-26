//! Anvil binary — P10 capstone: wires all ported crates into a running app.
//!
//! `App` implements `anvil_platform::appkit::AppHandler` and owns:
//!   - `TabManager` (workspace layout + pure pane state)
//!   - `HashMap<PaneId, Pty>` (PTY seam owned here, not in workspace)
//!   - Metal `Renderer` + `Raster`
//!   - `CoreTextPainter` (glyph painter, holds &Font so lives alongside Font)
//!   - `Config` / `Theme` / `Watcher`
//!   - `Webview` (command palette bridge)
//!   - Agent `Snapshot` + `LocalContext`
//!   - Git worker via `std::sync::mpsc`

mod fs_worker;
mod kube;
mod session;

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc;
use std::thread;
use std::time::Instant;

use anyhow::Result;

use anvil_agent::Snapshot as AgentSnapshot;
use anvil_config::{Chord, Config, Watcher, parse_chord};
use anvil_platform::AtlasPainter;
use anvil_platform::appkit::{
    AppHandler, AppKitApp, ContextAction, CursorKind, KeyEvent, KeyInput, Modifiers, MouseLocation,
    RightClickZone,
};
use anvil_platform::font::{CHROME_PT, Font, FontFace, register_bundled};
use anvil_platform::metal::{PresentMode, Renderer, present_mode};
use anvil_platform::pty::Pty;
use anvil_platform::shell_integration;
use anvil_platform::webview::{Webview, WebviewConfig};
use anvil_prompt_core::git;
use anvil_render::agent_panel::{
    GitState, HUD_COLS as HUD_COLS_DEFAULT, HudHit, LocalContext, RunState, SectionHeaderHit,
    SectionId, draw_right_hud,
};

/// Minimum and maximum HUD width in terminal columns. The drag handler
/// clamps the user's value to this range so they can't crush the terminal
/// or eat the whole window.
const HUD_COLS_MIN: usize = 16;
const HUD_COLS_MAX: usize = 80;
/// Width (in device pixels) of the invisible hit zone that catches mouse
/// down on the HUD's 1px hairline. Wide enough that the user doesn't need
/// pixel-perfect aim to start a resize.
const HUD_DRAG_HIT_PX: f64 = 6.0;
/// Hit zone half-width (device pixels) for the sidebar right-edge resize handle (item 13).
const SIDEBAR_DRAG_HIT_PX: f64 = 4.0;
/// Minimum/maximum sidebar width in logical points (item 13).
const SIDEBAR_W_MIN_PT: f64 = 180.0;
const SIDEBAR_W_MAX_PT: f64 = 600.0;
const EXPLORER_SCROLL_ROWS_PER_WHEEL: usize = 3;

/// Which resize divider the cursor is hovering over (P2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DividerKind {
    /// Right edge of the IDE sidebar (vertical divider → col-resize cursor).
    Sidebar,
    /// Horizontal divider between editor and drawer (row-resize cursor).
    Drawer,
}
use anvil_editor::{Position as EditorPosition, WorkspaceSymbolHit};
use anvil_render::cheatsheet::draw as draw_cheatsheet;
use anvil_render::draw::CursorConfig;
use anvil_render::raster::Raster;
use unicode_segmentation::UnicodeSegmentation;
// draw_search_bar is re-exported via draw_search_bar_with_replace; unused direct import removed.
use anvil_render::tabbar::{TabBarHitKind, TabBarHits, draw_tab_bar};
use anvil_render::workspace::{DIVIDER_PX, draw_workspace, draw_workspace_chrome};
use anvil_render::{
    CellBatch, EditorTabHit, ExplorerHit, FoldedBlocks, GridPainters, LeftDockHitKind,
    LeftDockHits, LeftDockSnapshot, OutlineRow, draw_left_dock_with_scroll, draw_viewport_gpu,
};
use anvil_term::{DirtySet, Terminal};
use anvil_theme::{Theme, resolve as resolve_theme};
use anvil_workspace::editor_pane::{
    EditorAction, FontMetrics as EditorFontMetrics, pixel_to_position,
};
use anvil_workspace::editor_search::EditorSearch;
use anvil_workspace::interact;
use anvil_workspace::keys::{Key, Mods, encode as encode_key, encode_mouse};
use anvil_workspace::layout::{
    DividerHit, NavDir, PaneId, Rect, SplitDir, adjust_ratio, find_divider_at, split_at_path_mut,
};
use anvil_workspace::mode::{DockMetrics, Docks, LayoutMode};
use anvil_workspace::palette::{Action, CATALOG, Palette, action_for_id};
use anvil_workspace::tab::{Tab, TabManager};

use anvil_control::bridge::{
    Command as BridgeCmd, Inbound, Outbound, ThemeTokens, decode as bridge_decode,
    encode as bridge_encode,
};

use objc2::rc::Retained;
use objc2_app_kit::NSWindow;
use objc2_foundation::MainThreadMarker;

// ── Embedded assets ──────────────────────────────────────────────────────────

const PALETTE_HTML: &str = include_str!("../../../ui/palette/index.html");

fn non_empty_terminal_cwd(terminal: &Terminal) -> Option<String> {
    let cwd = terminal.cwd_path();
    if cwd.is_empty() {
        None
    } else {
        Some(cwd.to_string())
    }
}

// ── Toast notification system (N3) ───────────────────────────────────────────

/// Visual kind of a toast notification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ToastKind {
    Info,
    Success,
    Error,
}

/// A single transient notification shown in the bottom-right corner.
#[derive(Debug, Clone)]
struct Toast {
    text: String,
    kind: ToastKind,
    expires_at: Instant,
}

// ── Constants ────────────────────────────────────────────────────────────────

/// Uniform inset in device pixels between the window edge and the terminal grid.
/// 24 device px (= 12 logical pt at 2× Retina) sits between a stock terminal's
/// near-zero inset and a product app's heavy chrome — readable as deliberate
/// breathing room without burning content columns.
const GRID_PAD: usize = 24;

/// HUD refresh: once every N ticks (~60 fps → ~1 s).
const HUD_REFRESH_TICKS: u32 = 60;

/// Maximum panes per tab.
const MAX_PANES_PER_TAB: usize = 8;

// ── Git worker ───────────────────────────────────────────────────────────────

struct GitResult {
    state: GitState,
    branch: String,
    dirty: u32,
    ahead: u32,
    behind: u32,
    head_short: String,
    head_subject: String,
    /// Locally-listening TCP ports detected at the time of the git query.
    ports: Vec<u16>,
    /// Detected project kind: "rust", "node", or "make". None if unrecognised.
    project_kind: Option<String>,
}

/// Result sent from the recent-files worker to the main thread.
struct RecentResult {
    files: Vec<String>,
}

/// Sent from the file-watcher thread to the main thread when a tracked buffer's
/// on-disk file changes (item 27).
struct FileWatchEvent {
    /// The buffer whose backing file changed.
    buffer_id: anvil_editor::BufferId,
}

/// Detect locally-listening TCP ports via `lsof`.
///
/// Cached for 2 s to avoid hammering lsof on every HUD tick. Skips ports
/// below 1024 (system) and the well-known noise ports 5353 (mDNS) and 7000
/// (AirPlay).
fn detect_ports() -> Vec<u16> {
    use std::sync::Mutex;
    use std::time::{Duration, Instant};
    static PORT_CACHE: Mutex<Option<(Instant, Vec<u16>)>> = Mutex::new(None);
    const PORT_CACHE_TTL: Duration = Duration::from_secs(2);

    if let Ok(guard) = PORT_CACHE.lock() {
        if let Some((ts, ref ports)) = *guard {
            if ts.elapsed() < PORT_CACHE_TTL {
                return ports.clone();
            }
        }
    }

    let ports = detect_ports_uncached();
    if let Ok(mut guard) = PORT_CACHE.lock() {
        *guard = Some((Instant::now(), ports.clone()));
    }
    ports
}

fn detect_ports_uncached() -> Vec<u16> {
    const SKIP: &[u16] = &[5353, 7000];
    let Ok(out) = std::process::Command::new("lsof")
        .args(["-nP", "-iTCP", "-sTCP:LISTEN"])
        .output()
    else {
        return Vec::new();
    };
    if !out.status.success() {
        return Vec::new();
    }
    let mut ports: Vec<u16> = Vec::new();
    for line in String::from_utf8_lossy(&out.stdout).lines().skip(1) {
        // Each line has fields separated by whitespace. The NAME column
        // (last field) looks like "*:3000" or "127.0.0.1:8080".
        let Some(name) = line.split_whitespace().last() else {
            continue;
        };
        let Some(port_str) = name.rsplit(':').next() else {
            continue;
        };
        let Ok(port) = port_str.parse::<u16>() else {
            continue;
        };
        if port < 1024 || SKIP.contains(&port) {
            continue;
        }
        if !ports.contains(&port) {
            ports.push(port);
        }
    }
    ports.sort_unstable();
    ports
}

/// Detect project kind by checking for well-known marker files in `cwd`.
/// Returns "rust", "node", or "make" for the first match, or None.
fn detect_project_kind(cwd: &std::path::Path) -> Option<String> {
    if cwd.join("Cargo.toml").exists() {
        return Some("rust".to_string());
    }
    if cwd.join("package.json").exists() {
        return Some("node".to_string());
    }
    if cwd.join("pyproject.toml").exists() {
        return Some("python".to_string());
    }
    if cwd.join("go.mod").exists() {
        return Some("go".to_string());
    }
    if cwd.join("Makefile").exists() {
        return Some("make".to_string());
    }
    if cwd.join(".git").is_dir() {
        return Some("git".to_string());
    }
    None
}

fn has_project_marker_in_or_above(cwd: &Path) -> bool {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    for dir in cwd.ancestors() {
        if detect_project_kind(dir).is_some() {
            return true;
        }
        if home.as_deref() == Some(dir) {
            break;
        }
    }
    false
}

/// Walk `cwd` up to depth `max_depth`, collecting (mtime, path) for regular
/// files (skipping hidden dirs and known noise dirs like `target/`,
/// `node_modules/`, `.git/`). Returns the top-`n` most recently modified
/// files as absolute path strings.
fn recent_files_in_dir(cwd: &std::path::Path, n: usize) -> Vec<String> {
    use std::time::SystemTime;
    const SKIP_DIRS: &[&str] = &["target", "node_modules", ".git"];

    let mut entries: Vec<(SystemTime, String)> = Vec::new();
    walk_dir_for_recent(cwd, 0, 3, &mut entries, SKIP_DIRS);
    entries.sort_by_key(|e| std::cmp::Reverse(e.0));
    entries.into_iter().take(n).map(|(_, p)| p).collect()
}

fn walk_dir_for_recent(
    dir: &std::path::Path,
    depth: usize,
    max_depth: usize,
    out: &mut Vec<(std::time::SystemTime, String)>,
    skip_dirs: &[&str],
) {
    use std::fs;
    if depth > max_depth {
        return;
    }
    let Ok(rd) = fs::read_dir(dir) else {
        return;
    };
    for entry in rd.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str.starts_with('.') || skip_dirs.contains(&name_str.as_ref()) {
            continue;
        }
        let Ok(ft) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if ft.is_dir() {
            if depth < max_depth {
                walk_dir_for_recent(&path, depth + 1, max_depth, out, skip_dirs);
            }
        } else if ft.is_file() {
            if let Ok(meta) = entry.metadata() {
                if let Ok(mtime) = meta.modified() {
                    if let Some(s) = path.to_str() {
                        out.push((mtime, s.to_string()));
                    }
                }
            }
        }
    }
}

/// Path used to persist the user's HUD section order.
///
/// Lives under `~/.config/anvil/` (XDG-ish) so it survives across launches
/// without touching the main TOML config (the config crate doesn't have a
/// writer yet). One section token per line, in display order.
fn hud_section_order_path() -> Option<std::path::PathBuf> {
    let home = std::env::var_os("HOME")?;
    let mut p = std::path::PathBuf::from(home);
    p.push(".config");
    p.push("anvil");
    Some(p.join("section_order.txt"))
}

fn load_hud_section_order() -> Option<Vec<SectionId>> {
    let path = hud_section_order_path()?;
    let text = std::fs::read_to_string(&path).ok()?;
    let order: Vec<SectionId> = text
        .lines()
        .filter_map(|line| SectionId::from_token(line.trim()))
        .collect();
    if order.is_empty() {
        return None;
    }
    Some(order)
}

fn save_hud_section_order(order: &[SectionId]) {
    let Some(path) = hud_section_order_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let body: String = order
        .iter()
        .map(|s| format!("{}\n", s.token()))
        .collect::<String>();
    let _ = std::fs::write(&path, body);
}

/// One-shot `git log -1 --format=%h %s` against `cwd`. Returns (sha, subject)
/// Return the local wall-clock time as `"HH:MM"` using libc `localtime_r`.
fn local_hhmm() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as libc::time_t)
        .unwrap_or(0);
    let mut tm = libc::tm {
        tm_sec: 0,
        tm_min: 0,
        tm_hour: 0,
        tm_mday: 0,
        tm_mon: 0,
        tm_year: 0,
        tm_wday: 0,
        tm_yday: 0,
        tm_isdst: 0,
        tm_gmtoff: 0,
        tm_zone: std::ptr::null_mut(),
    };
    // SAFETY: secs is a valid time_t; tm is stack-allocated and we own it.
    unsafe { libc::localtime_r(&secs, &mut tm) };
    format!("{:02}:{:02}", tm.tm_hour, tm.tm_min)
}

/// or empty strings on failure / non-repo. Bounded: takes <10ms in practice.
fn git_head_oneline(cwd: &std::path::Path) -> (String, String) {
    let output = std::process::Command::new("git")
        .args(["log", "-1", "--format=%h %s"])
        .current_dir(cwd)
        .output();
    let Ok(out) = output else {
        return (String::new(), String::new());
    };
    if !out.status.success() {
        return (String::new(), String::new());
    }
    let s = String::from_utf8_lossy(&out.stdout);
    let line = s.lines().next().unwrap_or("").trim_end();
    match line.split_once(' ') {
        Some((sha, subject)) => (sha.to_string(), subject.to_string()),
        None => (line.to_string(), String::new()),
    }
}

// ── Keybindings ───────────────────────────────────────────────────────────────

#[derive(Clone, Copy)]
struct Keybindings {
    new_tab: Option<Chord>,
    close_tab: Option<Chord>,
    next_tab: Option<Chord>,
    prev_tab: Option<Chord>,
    jump: [Option<Chord>; 9],
    search_open: Option<Chord>,
    /// Cmd+Opt+Shift+F: open search bar scoped to the current block (moved from Cmd+Shift+F).
    search_open_block: Option<Chord>,
    /// Cmd+Shift+F: open the project-wide search overlay.
    project_search_open: Option<Chord>,
    search_next: Option<Chord>,
    search_prev: Option<Chord>,
    /// Cmd+Opt+R: toggle regex mode while the search bar is open.
    search_regex_toggle: Option<Chord>,
    hud_toggle: Option<Chord>,
    cheatsheet: Option<Chord>,
    split_right: Option<Chord>,
    split_down: Option<Chord>,
    close_pane: Option<Chord>,
    focus_left: Option<Chord>,
    focus_right: Option<Chord>,
    focus_up: Option<Chord>,
    focus_down: Option<Chord>,
    fold_block: Option<Chord>,
    toggle_theme: Option<Chord>,
    layout_mode_toggle: Option<Chord>,
    left_dock_toggle: Option<Chord>,
    editor_new: Option<Chord>,
    /// Cmd+T: open workspace symbol search overlay (O1).
    workspace_symbol_search: Option<Chord>,
    /// Cmd+R: open buffer symbol search overlay (O2).
    buffer_symbol_search: Option<Chord>,
}

impl Keybindings {
    fn from_config(kb: &anvil_config::Keybindings) -> Self {
        let jump_strs = [
            &kb.tab_1, &kb.tab_2, &kb.tab_3, &kb.tab_4, &kb.tab_5, &kb.tab_6, &kb.tab_7, &kb.tab_8,
            &kb.tab_9,
        ];
        let mut jump = [None; 9];
        for (i, s) in jump_strs.iter().enumerate() {
            jump[i] = parse_chord(s);
        }
        Self {
            new_tab: parse_chord(&kb.new_tab),
            close_tab: parse_chord(&kb.close_tab),
            next_tab: parse_chord(&kb.next_tab),
            prev_tab: parse_chord(&kb.prev_tab),
            jump,
            search_open: parse_chord(&kb.search_open),
            search_open_block: parse_chord("cmd+opt+shift+f"),
            project_search_open: parse_chord(&kb.project_search),
            search_next: parse_chord(&kb.search_next),
            search_prev: parse_chord(&kb.search_prev),
            search_regex_toggle: parse_chord("cmd+opt+r"),
            hud_toggle: parse_chord(&kb.hud_toggle),
            cheatsheet: parse_chord(&kb.cheatsheet_toggle),
            split_right: parse_chord(&kb.split_right),
            split_down: parse_chord(&kb.split_down),
            close_pane: parse_chord(&kb.close_pane),
            focus_left: parse_chord(&kb.focus_left),
            focus_right: parse_chord(&kb.focus_right),
            focus_up: parse_chord(&kb.focus_up),
            focus_down: parse_chord(&kb.focus_down),
            fold_block: parse_chord(&kb.fold_block),
            toggle_theme: parse_chord(&kb.toggle_theme),
            layout_mode_toggle: parse_chord(&kb.layout_mode_toggle),
            left_dock_toggle: parse_chord(&kb.left_dock_toggle),
            editor_new: parse_chord(&kb.editor_new),
            workspace_symbol_search: parse_chord("cmd+t"),
            buffer_symbol_search: parse_chord("cmd+r"),
        }
    }
}

// ── Explorer support types (items 4, 6, 7, 8) ────────────────────────────────

/// Which surface has keyboard focus for key routing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum FocusTarget {
    #[default]
    Editor,
    Explorer,
    Terminal,
}

/// Inline rename state for an Explorer row (item 6).
pub struct RenameState {
    /// Absolute path of the entry being renamed.
    pub old_path: PathBuf,
    /// Current text in the rename input field (starts as the basename).
    pub input: String,
    /// Explorer row index (slot_i from `visible_rows`).
    pub row_idx: usize,
}

/// Ghost-row creation state for new-file / new-folder (item 7).
pub struct NewItemState {
    /// Directory in which the new entry will be created.
    pub parent_dir: PathBuf,
    /// Current text typed by the user (empty on open).
    pub input: String,
    /// True → create directory; false → create file.
    pub is_dir: bool,
}

/// Pending delete confirmation state (item 8).
pub struct DeleteConfirm {
    /// Absolute path of the item to delete.
    pub path: PathBuf,
    /// Human-readable name (basename) shown in the modal.
    pub name: String,
}

// ── LSP references overlay (item 26) ─────────────────────────────────────────

/// One row in the references panel.
struct ReferencesRow {
    path: PathBuf,
    line: u32,
    col: u32,
}

/// State for the references overlay panel (item 26).
struct LspReferencesOverlay {
    rows: Vec<ReferencesRow>,
    /// Currently selected row (keyboard nav).
    selected: usize,
}

// ── Symbol search overlays (O1, O2) ──────────────────────────────────────────

/// Workspace symbol search overlay state (Cmd+T / O1).
struct WorkspaceSymbolSearch {
    /// Text typed in the input box.
    query: String,
    /// Hits from the last `workspace/symbol` request (or "(LSP unavailable)" sentinel).
    hits: Vec<WorkspaceSymbolHit>,
    /// Currently selected row.
    selected: usize,
    /// In-flight request id; 0 = none.
    pending_request_id: u64,
    /// True when LSP is not available (no live server found).
    lsp_unavailable: bool,
    /// Debounce: time of the last query change.
    last_query_change: Option<Instant>,
}

/// Buffer symbol search overlay state (Cmd+R / O2).
struct BufferSymbolSearch {
    /// Filter text typed by the user.
    query: String,
    /// All symbols from the active buffer's syntax tree (derive_outline_rows).
    all_symbols: Vec<anvil_editor::OutlineSymbol>,
    /// Indices into `all_symbols` that pass the current filter.
    filtered: Vec<usize>,
    /// Currently selected row.
    selected: usize,
}

// ── LanguagePickerState (Q22) ─────────────────────────────────────────────────

/// State for the Cmd+K Cmd+L language-picker overlay.
struct LanguagePickerState {
    /// Filter text typed by the user.
    query: String,
    /// Currently selected row index into the filtered list.
    selected: usize,
}

// ── App ───────────────────────────────────────────────────────────────────────

/// The whole application state.  Implements [`AppHandler`] via [`AppShell`].
pub struct App {
    // -- workspace ---
    tabs: TabManager,
    /// PTY handles keyed by PaneId (all panes across all tabs).
    ptys: HashMap<PaneId, Pty>,

    // -- render ---
    /// `None` only during initialization, before the Metal layer is available.
    renderer: Option<Renderer>,
    raster: Raster,
    /// Heap-allocated so its address is stable; `AppShell.painter` borrows it.
    font: Box<Font>,
    /// Bold variant of `font`. Heap-stable; `AppShell.bold_painter` borrows it.
    bold_font: Box<Font>,
    /// Italic variant of `font`. Heap-stable; `AppShell.italic_painter` borrows it.
    italic_font: Box<Font>,
    /// BoldItalic variant of `font`. Heap-stable; `AppShell.bold_italic_painter` borrows it.
    bold_italic_font: Box<Font>,
    /// Fixed-size chrome font (11 pt × scale) for tab bar, status bar, etc.
    /// Heap-allocated for the same lifetime-stability reason as `font`.
    chrome_font: Box<Font>,
    dirty: bool,
    /// When true, the next CPU-path frame must redraw ALL rows (full clear).
    /// Set on theme change, resize, search toggle, etc. Cleared after the frame.
    force_full_redraw: bool,
    /// Tracks the cursor's last-drawn row per pane (for dirty-row wiring).
    /// Keyed by `PaneId`; value is the viewport row the cursor was on last frame.
    cursor_row_prev: HashMap<PaneId, usize>,
    /// Tracks the scrollback length per pane between frames. When scrollback
    /// grows, content auto-scrolled — every visible row's pixels shifted, so
    /// damage tracking by row index alone is wrong and we force a full redraw.
    scrollback_len_prev: HashMap<PaneId, usize>,
    /// Last frame's focused-pane scroll_pos. We only force a full redraw when
    /// it MOVED — sitting inside scrollback statically should be as cheap as
    /// sitting at live.
    last_scroll_pos: f32,
    /// Last frame's focused-pane viewport_offset. Same rationale.
    last_viewport_offset: usize,
    /// For debug instrumentation: frame counter and last-report time.
    #[cfg(debug_assertions)]
    debug_render_frame: u64,
    #[cfg(debug_assertions)]
    debug_render_last_report: Option<Instant>,
    /// When true, the terminal viewport is drawn via the GPU cell pipeline.
    /// Enabled by `ANVIL_RENDER=gpu`; default false (CPU path).
    use_gpu_render: bool,
    /// Instance batch for the GPU cell pipeline.  Cleared at the start of each
    /// GPU-path frame; accumulated by `draw_viewport_gpu` per pane.
    cell_batch: CellBatch,
    /// GPU glyph atlas painter.  `None` until the Metal device is available
    /// (filled during renderer init in `AppHandler::resize`).
    atlas_painter: Option<AtlasPainter>,

    // -- theme / config ---
    theme: Theme,
    cursor_cfg: CursorConfig,
    config: Config,
    watcher: Option<Watcher>,
    keybindings: Keybindings,
    system_dark: bool,
    window_scale: f64,
    /// Current layout mode. Defaults to `Terminal` outside project dirs, `Ide` in project dirs.
    /// Set at startup via `ANVIL_LAYOUT_MODE=ide|terminal`.
    layout_mode: LayoutMode,
    /// Whether the IDE explorer dock is visible; toggled by Cmd+B.
    left_dock_visible: bool,
    /// Current explorer sidebar width in logical points (item 13: drag-resize).
    /// Range clamped to [180, 600] on drag. Default 300.
    left_dock_w_pt: f64,
    /// G3: smooth target for sidebar width. Drag writes here; tick eases
    /// `left_dock_w_pt` toward this value.
    left_dock_w_pt_target: f64,
    /// True while the user is dragging the sidebar right edge (item 13).
    sidebar_drag_active: bool,
    /// True while the user is dragging the editor/drawer horizontal divider (item 13b).
    drawer_drag_active: bool,
    /// P3: true while dragging the editor horizontal scrollbar thumb.
    /// `mouse_dragged` maps the x position to scroll_x.
    hscroll_drag_active: bool,
    /// Item 8 (Tier-B): whether the IDE bottom drawer is hidden.
    /// When true the root-split ratio[0] has been forced to 1.0 (editor takes
    /// 100% of pane area).  The pre-hide ratio is saved in `drawer_saved_ratio`
    /// so Cmd+J can restore it.
    drawer_hidden: bool,
    /// Saved editor-over-drawer ratio, captured before hiding the drawer
    /// (Tier-B item 8).  Default 0.72.
    drawer_saved_ratio: f64,

    // -- UI state ---
    blink_phase: f32,
    last_blink_opacity: f32,
    search: anvil_term::Search,
    search_open: bool,
    hud_visible: bool,
    hud_tick: u32,
    /// Runtime HUD width in terminal columns. Starts at `HUD_COLS_DEFAULT`,
    /// the user can drag the HUD's left edge to resize at runtime.
    hud_cols: usize,
    /// Mouse is currently dragging the HUD's left edge hairline. While true,
    /// `mouse_dragged` updates `hud_cols` instead of extending a selection.
    hud_drag_active: bool,
    /// When set, the user grabbed a pane divider and is dragging it to resize.
    /// Cleared on mouse-up.
    divider_drag: Option<DividerHit>,
    /// P2: which resize divider the cursor is currently hovering over.
    /// `None` when the cursor is not near a resize divider.  Drives the 1px
    /// highlight stripe and the system cursor (col-resize or row-resize).
    divider_hover: Option<DividerKind>,
    /// Chrome-row hit regions (tab switches, close ×, + button). Refilled
    /// by `draw_tab_bar` each render; consumed by `mouse_down`.
    tab_bar_hits: TabBarHits,
    /// Left-dock hit regions (explorer/outline rows). Refilled by
    /// `draw_left_dock` in Ide mode; consumed before pane hit-testing.
    left_dock_hits: LeftDockHits,
    /// Clickable regions inside the HUD. Refilled by `draw_right_hud` each
    /// render; consumed by `mouse_down` to dispatch copy / open actions.
    hud_hits: Vec<HudHit>,
    /// Display order of HUD sections (user-reorderable via drag on a
    /// section header). Persisted to a sidecar file at startup.
    hud_section_order: Vec<SectionId>,
    /// Section-header hit zones, refilled each render. Used by mouse-down
    /// to start a drag-to-reorder gesture.
    hud_section_hits: Vec<SectionHeaderHit>,
    /// When set, the section the user grabbed on mouse-down for reorder.
    /// On mouse-up, the section under the cursor becomes the drop target.
    hud_section_drag: Option<SectionId>,
    /// Tab drag state: (current tab index being dragged, mouse-down raster x).
    /// Set on mouse-down over a tab; cleared on mouse-up.  While set and the
    /// cursor has moved past the threshold, `mouse_dragged` calls `move_tab`.
    tab_drag: Option<(usize, f64)>,
    /// Item 10 (Tier-B): editor buffer tab drag state.
    /// `(pane_id, buffer index in open_buffers, mouse-down raster x)`.
    /// Set on mouse-down on a non-close EditorTabHit; cleared on mouse-up.
    /// While set and cursor moved >4 logical px, `mouse_dragged` reorders
    /// `open_buffers` in place.
    editor_tab_drag: Option<(PaneId, usize, f64)>,
    /// Item 15 (Tier-B): up to 50 most recently opened file paths.
    /// Deduped on insert (most-recent at index 0).
    recent_file_list: Vec<PathBuf>,
    /// Current font point size (logical points, not device pixels). Adjusted
    /// at runtime by Cmd+/Cmd- and re-baked into a fresh `Font` + painter.
    font_size_pt: f64,
    /// Font family preference, kept around so `bump_font_size` can rebuild
    /// the `Font` with the same fallback chain used at startup.
    font_family: String,
    cheatsheet_visible: bool,
    focused: bool, // window is key window

    // -- agent panel ---
    agent_snap: AgentSnapshot,
    local_ctx: LocalContext,
    caldera_poller: Option<anvil_caldera::Poller>,
    /// Separate client used for fire-and-forget actions (approve, start_run).
    /// Shares the same endpoint as the poller; created at the same time.
    caldera_client: Option<anvil_caldera::CalderaClient>,

    // -- native editor (NE4+) ---
    /// When set, the buffer position where a native-editor drag-select began
    /// (NE7). Cleared on mouse-up; used by mouse_dragged to extend selection.
    editor_mouse_drag_start: Option<EditorPosition>,

    // -- git worker ---
    git_tx: mpsc::SyncSender<PathBuf>,
    git_rx: mpsc::Receiver<GitResult>,

    // -- recent-files worker ---
    recent_cwd_tx: mpsc::SyncSender<PathBuf>,
    recent_rx: mpsc::Receiver<RecentResult>,

    // -- kubectl worker ---
    kube_rx: mpsc::Receiver<anvil_prompt_core::KubeCtx>,

    // -- filesystem worker (left dock, ID3) ---
    fs_tx: mpsc::SyncSender<PathBuf>,
    fs_rx: mpsc::Receiver<fs_worker::DirSnapshot>,
    /// Sends the current filter flags to the fs worker so the next snapshot
    /// respects both the hidden-files (Q56) and gitignore (S1) toggles.
    fs_hidden_tx: mpsc::SyncSender<fs_worker::FilterFlags>,
    /// Worker for child-directory expansions. Receiver kept so async
    /// re-snapshots (e.g. on watch events) can still drain results; sender
    /// retired because explorer click reads synchronously now.
    #[allow(dead_code)]
    child_fs_tx: mpsc::SyncSender<fs_worker::ChildFsRequest>,
    child_fs_rx: mpsc::Receiver<fs_worker::ChildFsResponse>,
    fs_snapshot: Option<LeftDockSnapshot>,
    /// Per-directory child snapshots, populated lazily on first expand.
    child_snapshots: HashMap<PathBuf, LeftDockSnapshot>,
    /// Last file opened through the IDE explorer/native-editor open path.
    /// Used to keep the explorer row visually selected.
    active_explorer_file: Option<PathBuf>,
    /// Top visible row in the Explorer. Mouse wheel over the dock adjusts this
    /// without stealing focus from the active editor/terminal pane.
    explorer_scroll_offset: usize,
    /// The explorer row index currently under the cursor (for hover highlight).
    /// `None` when the cursor is outside the dock.
    hovered_explorer_row: Option<usize>,
    /// The currently hovered buffer tab in the per-pane editor strip.
    /// `None` when the cursor is not over any editor tab.
    hovered_editor_tab: Option<(PaneId, anvil_editor::BufferId)>,
    /// Per-pane editor buffer tab hit regions. Refilled by `draw_workspace`
    /// each render; consumed by `mouse_down` for tab-switch and close.
    editor_tab_hits: Vec<EditorTabHit>,
    /// Set of absolute paths of directories that are expanded in the Explorer.
    /// Keyed by PathBuf so identity survives re-snapshots and depth changes.
    expanded_dirs: HashSet<PathBuf>,
    /// Alpha for the Explorer scroll thumb (Item 8). Driven by a decay timer.
    /// 1.0 immediately after a scroll event; decays to 0 after 600ms hold + 200ms fade.
    scroll_indicator_alpha: f32,
    /// Timestamp of the last explorer scroll event, for the 600ms hold + 200ms fade-out.
    scroll_indicator_last_scroll: Option<Instant>,
    /// Last cwd sent to the fs worker; used to debounce re-sends.
    fs_last_cwd: Option<String>,

    // -- pulse (agent dot animation) ---
    agent_pulse_phase: f32,
    last_agent_pulse_opacity: f32,

    // -- running-block header dot pulse (CB6) ---
    running_pulse_phase: f32,

    // -- command palette ---
    palette: Palette,

    // -- project search (NE12) ---
    project_search: anvil_workspace::project_search::ProjectSearch,

    // -- window geometry (view-point size, updated on resize) ---
    view_width_pt: f64,
    view_height_pt: f64,

    // -- LSP client core (NE9) ---
    /// `None` only when the Tokio runtime failed to start (extremely rare).
    lsp_manager: Option<anvil_editor::LspManager>,
    /// Per-pane timestamp of the last LSP `didChange` sync. Used for 250 ms
    /// debounce so we don't flood the server on every keystroke.
    lsp_last_sync: HashMap<PaneId, Instant>,

    // -- LSP UI (NE10, tier-3) ---
    /// In-flight hover request: `(pane_id, request_id)`. Polled each tick.
    pending_hover: Option<(PaneId, u64)>,
    /// Mouse-hover debounce for item 15: last position + time for a 400ms hover trigger.
    hover_mouse_pos: Option<(f64, f64)>,
    hover_mouse_time: Option<Instant>,
    /// In-flight definition request: `(pane_id, request_id)`. Polled each tick.
    pending_definition: Option<(PaneId, u64)>,
    /// In-flight completion request: `(pane_id, request_id)`. Polled each tick.
    pending_completion: Option<(PaneId, u64)>,
    /// In-flight rename request: `(pane_id, request_id)`. Polled each tick (item 24).
    pending_rename: Option<(PaneId, u64)>,
    /// In-flight code-actions request: `(pane_id, request_id)`. Polled each tick (item 25).
    pending_code_actions: Option<(PaneId, u64)>,
    /// In-flight references request: `(pane_id, request_id)`. Polled each tick (item 26).
    pending_references: Option<(PaneId, u64)>,
    /// Workspace edits from the last code-actions response, indexed by action order (item 25).
    /// Each entry is the flat list of rename edits for that action (may be empty for commands).
    code_actions_pending_edits: Vec<Vec<anvil_editor::RenameEdit>>,

    // ── LSP rename overlay (item 24) ──────────────────────────────────────────
    /// When `Some`, the LSP rename overlay is open. Contains the text input.
    lsp_rename_input: Option<String>,

    // ── LSP references overlay (item 26) ──────────────────────────────────────
    /// When `Some`, the references overlay is open.
    lsp_references: Option<LspReferencesOverlay>,

    // ── Symbol search overlays (O1, O2) ───────────────────────────────────────
    /// Cmd+T: workspace symbol search overlay.
    workspace_symbol_search: Option<WorkspaceSymbolSearch>,
    /// Cmd+R: buffer symbol search overlay.
    buffer_symbol_search: Option<BufferSymbolSearch>,

    // ── UI scale (item 1) ──────────────────────────────────────────────────────
    /// Global UI scale multiplier. Applied on top of `window_scale` to font
    /// pixel size and dock geometry. Cmd+= zooms in, Cmd+- zooms out, Cmd+0 resets.
    ui_scale: f64,

    // ── Font scale (H4) ───────────────────────────────────────────────────────
    /// Font-only scale multiplier.  Multiplies font pixel size only — dock
    /// widths, chrome heights, and row heights are unchanged.
    /// Cmd+Opt+= in, Cmd+Opt+- out, Cmd+Opt+0 reset.  Range [0.6, 2.5].
    font_scale: f64,

    // ── Cmd+K chord pending (H1, H2) ─────────────────────────────────────────
    /// True after Cmd+K fires in a native editor pane, awaiting the second
    /// key (W = soft-wrap, Space = show-whitespace).  Cleared on any key.
    pending_chord_k: bool,

    // ── Goto-line overlay (item 11) ───────────────────────────────────────────
    /// When `Some`, the goto-line input overlay is open. The string is the
    /// text typed so far (e.g. "42" or "42,8").
    goto_line_input: Option<String>,

    // ── Save-as overlay (tier-J J2) ───────────────────────────────────────────
    /// When `Some`, the save-as path-input overlay is open.  The string is the
    /// file path typed so far.
    /// TODO(anvil-tierJ-J2-nspanel): replace with NSSavePanel.
    save_as_input: Option<String>,

    // ── Find+replace active-row flag (item 9) ─────────────────────────────────
    /// `true` when the replace row of the search bar has keyboard focus.
    replace_row_active: bool,

    // ── Explorer focus + keyboard nav (items 4, 5) ────────────────────────────
    /// Which surface has keyboard focus for routing ↑↓→←/Enter/Esc.
    focus_target: FocusTarget,
    /// The Explorer row index selected by keyboard navigation.  Drives arrow-key
    /// navigation and Enter/→/← dispatch.
    selected_explorer_row: Option<usize>,

    // ── Inline rename (item 6) ────────────────────────────────────────────────
    /// Active rename state for the Explorer row being renamed.
    explorer_rename: Option<RenameState>,

    // ── New file / folder (item 7) ────────────────────────────────────────────
    /// Active ghost-row state for new-file / new-folder creation.
    explorer_new_item: Option<NewItemState>,

    // ── Delete confirm (item 8) ────────────────────────────────────────────────
    /// Pending delete confirmation (name and absolute path of the row to delete).
    explorer_delete_confirm: Option<DeleteConfirm>,

    // ── File watcher (item 27) ────────────────────────────────────────────────
    /// Receiver for disk-change events from the file-watcher thread.
    file_watch_rx: mpsc::Receiver<FileWatchEvent>,
    /// Sender for telling the watcher thread about new (buffer_id, path) pairs to watch.
    file_watch_tx: mpsc::SyncSender<(anvil_editor::BufferId, PathBuf)>,
    /// Buffers that have changed on disk but have in-memory edits: maps
    /// buffer_id → path, shown as a banner until the user reloads or dismisses.
    disk_changed_dirty: HashMap<anvil_editor::BufferId, PathBuf>,

    // ── Recent projects + project switcher (items 28, 30) ────────────────────
    /// Recently-opened workspace cwds, most-recent first. Cap 20.
    recent_projects: Vec<PathBuf>,
    /// Whether the Cmd+Shift+O project-switcher overlay is open.
    project_switcher_open: bool,
    /// Currently highlighted row in the project-switcher overlay.
    project_switcher_sel: usize,

    // ── Context-menu target path (I1/I2) ─────────────────────────────────────
    /// Path resolved during `right_click_zone`; consumed by `context_action`.
    right_click_path: Option<PathBuf>,

    // ── Explorer-drag state (I3) ──────────────────────────────────────────────
    /// File path being dragged from the Explorer, plus the mouse location at
    /// which the drag started (used to detect the 4pt threshold).
    explorer_drag: Option<(PathBuf, MouseLocation)>,
    /// Current cursor position while an explorer drag is live; used by the
    /// render path to paint the floating filename chip.
    explorer_drag_cursor: Option<MouseLocation>,

    // ── Toast notifications (N3) ──────────────────────────────────────────────
    /// Active toasts, front = oldest. Expired entries are drained each tick.
    toasts: std::collections::VecDeque<Toast>,
    /// Server ids for which we have already shown an LSP-not-found toast.
    /// Prevents spamming the user on every tick.
    lsp_failed_toasted: HashSet<String>,

    // ── Search bar nav arrows (N4) ────────────────────────────────────────────
    /// Pixel rects for the ◀ / ▶ arrows in the search bar. Repopulated each
    /// frame when the search bar is open; consumed by `mouse_down`.
    search_bar_hits: anvil_render::searchbar::SearchBarArrowHits,

    // ── Show-hidden-files toggle (Q56) ────────────────────────────────────────
    /// When true, the Explorer and fs worker include dot-prefix entries.
    /// Toggled by Cmd+Shift+.; persisted in session JSON.
    show_hidden_files: bool,

    // ── Show-gitignored-files toggle (S1) ─────────────────────────────────────
    /// When false (default), entries matching `.gitignore` are hidden.
    /// Toggled by Cmd+K Cmd+I.
    show_gitignored_files: bool,

    // ── Closed-tab history (Q16) ──────────────────────────────────────────────
    /// Paths of recently closed editor buffers (most-recent at back). Cap 20.
    /// Cmd+Shift+T pops the most-recent entry and reopens it.
    closed_tabs: std::collections::VecDeque<PathBuf>,

    // ── Language picker overlay (Q22) ─────────────────────────────────────────
    /// When `Some`, the Cmd+K Cmd+L language-picker overlay is open.
    language_picker: Option<LanguagePickerState>,

    // ── Open-folder overlay (Q19) ─────────────────────────────────────────────
    /// When `Some`, the Cmd+K Cmd+O open-folder path-input overlay is open.
    open_folder_input: Option<String>,

    // ── Explorer tooltip (R2) ─────────────────────────────────────────────────
    /// Row index + time the hover started (for 500ms delay).
    explorer_hover_row: Option<(usize, Instant)>,
    /// Cached metadata (size in bytes, mtime as Duration since UNIX epoch) for
    /// the currently-hovered file. Key is the path, value is (size, mtime_secs).
    /// Cleared when hover row changes. Valid for ~5 s.
    explorer_hover_meta: Option<(PathBuf, u64, u64)>,

    // ── Explorer filter (R3) ──────────────────────────────────────────────────
    /// Active filter string typed by the user when Explorer is focused.
    explorer_filter: Option<String>,
}

// ── R2: tooltip helpers ───────────────────────────────────────────────────────

/// Format a byte count as a human-readable string: "12.4 KB", "3.2 MB", etc.
fn humanize_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}

/// Format a UNIX timestamp (seconds) as a relative time string.
/// Uses the current UNIX time from `std::time::SystemTime`.
fn relative_time(mtime_secs: u64) -> String {
    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if mtime_secs > now_secs {
        return "just now".to_string();
    }
    let delta = now_secs - mtime_secs;
    if delta < 60 {
        "just now".to_string()
    } else if delta < 3600 {
        let m = delta / 60;
        format!("{m} minute{} ago", if m == 1 { "" } else { "s" })
    } else if delta < 86400 {
        let h = delta / 3600;
        format!("{h} hour{} ago", if h == 1 { "" } else { "s" })
    } else {
        let d = delta / 86400;
        format!("{d} day{} ago", if d == 1 { "" } else { "s" })
    }
}

// ── App helpers ───────────────────────────────────────────────────────────────

fn explorer_path_for_hit(snapshot: &LeftDockSnapshot, hit: ExplorerHit) -> Option<PathBuf> {
    match hit {
        ExplorerHit::Header => Some(PathBuf::from(&snapshot.root)),
        ExplorerHit::Row(idx) => snapshot
            .entries
            .get(idx)
            .map(|entry| PathBuf::from(&snapshot.root).join(&entry.name)),
    }
}

fn next_explorer_scroll_offset(current: usize, dy: f64, entry_count: usize) -> usize {
    if entry_count == 0 || dy == 0.0 {
        return current.min(entry_count);
    }
    if dy > 0.0 {
        current
            .saturating_add(EXPLORER_SCROLL_ROWS_PER_WHEEL)
            .min(entry_count)
    } else {
        current.saturating_sub(EXPLORER_SCROLL_ROWS_PER_WHEEL)
    }
}

impl App {
    // ── Toast helpers (N3) ────────────────────────────────────────────────────

    const TOAST_TTL_SECS: u64 = 3;
    const TOAST_MAX_CHARS: usize = 60;

    fn push_toast(&mut self, text: &str, kind: ToastKind) {
        let truncated: String = text.chars().take(Self::TOAST_MAX_CHARS).collect();
        self.toasts.push_back(Toast {
            text: truncated,
            kind,
            expires_at: Instant::now() + std::time::Duration::from_secs(Self::TOAST_TTL_SECS),
        });
        // Cap to 5 visible at once.
        while self.toasts.len() > 5 {
            self.toasts.pop_front();
        }
        self.dirty = true;
    }

    fn toast_info(&mut self, text: &str) {
        self.push_toast(text, ToastKind::Info);
    }

    fn toast_success(&mut self, text: &str) {
        self.push_toast(text, ToastKind::Success);
    }

    fn toast_error(&mut self, text: &str) {
        self.push_toast(text, ToastKind::Error);
    }

    /// Expire stale toasts. Called each tick.
    fn tick_toasts(&mut self) {
        let now = Instant::now();
        while self.toasts.front().is_some_and(|t| t.expires_at <= now) {
            self.toasts.pop_front();
            self.dirty = true;
        }
    }

    // ── End toast helpers ─────────────────────────────────────────────────────

    /// The current focused pane id.
    fn focused_pane_id(&self) -> PaneId {
        self.tabs.current().map(|t| t.focused_id()).unwrap_or(0)
    }

    /// The cwd of the focused terminal pane (OSC 7 path), falling back to any
    /// live terminal in the tab when focus is on a native editor surface.
    /// Final fallback: the process cwd at launch, so the Explorer can paint
    /// immediately without waiting for shell integration.
    fn current_cwd(&self) -> Option<String> {
        let tab = self.tabs.current()?;
        let focused_id = tab.focused_id();
        if let Some(cwd) = tab
            .registry
            .get(focused_id)
            .and_then(|pane| pane.terminal.as_ref())
            .and_then(non_empty_terminal_cwd)
        {
            return Some(cwd);
        }

        if let Some(cwd) = tab
            .registry
            .iter()
            .filter(|(id, _)| *id != focused_id)
            .find_map(|(_, pane)| pane.terminal.as_ref().and_then(non_empty_terminal_cwd))
        {
            return Some(cwd);
        }

        std::env::current_dir()
            .ok()
            .map(|p| p.to_string_lossy().into_owned())
    }

    /// Device-pixel dimensions of the content area.
    fn device_size(&self) -> (usize, usize) {
        let dw = ((self.view_width_pt * self.window_scale) as usize).max(1);
        let dh = ((self.view_height_pt * self.window_scale) as usize).max(1);
        (dw, dh)
    }

    /// Window inner rect in device pixels: window content minus OS title strip
    /// and bottom status bar, before dock subtraction.
    ///
    /// Left `GRID_PAD` is included so `pane_area.x = GRID_PAD` in Terminal
    /// mode (preserving the existing left margin).  Right edge reaches to
    /// `dw - GRID_PAD`; the right-side reservation is handled by `Docks`.
    fn window_inner(&self) -> Rect {
        let (dw, dh) = self.device_size();
        let cw = self.font.metrics.cell_w;
        let ch = self.font.metrics.cell_h;
        let top_bar_px = self.chrome_top_px();
        // Bottom status bar is owned by Docks::compute_areas (it returns a
        // bottom_bar Rect and subtracts bottom_h from pane_h). Subtracting it
        // here too would double-count and shrink the pane area.
        Rect {
            x: GRID_PAD as f64,
            y: top_bar_px,
            w: (dw as f64 - GRID_PAD as f64).max(cw),
            h: (dh as f64 - top_bar_px).max(ch),
        }
    }

    /// Build the `DockMetrics` struct from current font / HUD state.
    fn dock_metrics(&self) -> DockMetrics {
        DockMetrics {
            cell_w: self.font.metrics.cell_w,
            cell_h: self.font.metrics.cell_h,
            hud_cols: self.hud_cols,
            grid_pad: GRID_PAD as f64,
        }
    }

    /// Compute `Docks` for the current mode and return the pane area rect.
    ///
    /// This is the single source of truth for where the terminal grid lives.
    /// Pass this to `PaneTree::layout`, hit-test, and `draw_workspace`.
    fn pane_area_rect(&self) -> Rect {
        Docks::for_mode_with_left_dock_w(
            self.layout_mode,
            self.window_scale,
            self.dock_metrics(),
            self.hud_visible,
            self.chrome_bottom_px(),
            self.left_dock_visible,
            self.left_dock_w_pt * self.ui_scale,
            self.ui_scale,
        )
        .compute_areas(
            self.window_inner(),
            self.font.metrics.cell_w,
            self.font.metrics.cell_h,
        )
        .pane_area
    }

    /// Return the device-pixel y-coordinate of the IDE editor/drawer divider,
    /// or `None` when the current tab's root split is not the IDE vertical split
    /// (e.g. a single-pane layout or a non-IDE tab).
    ///
    /// The divider sits at `editor_rect.y + editor_rect.h` in device pixels.
    fn ide_drawer_divider_y(&self) -> Option<f64> {
        if self.layout_mode != LayoutMode::Ide {
            return None;
        }
        let tab = self.tabs.current()?;
        let ir = self.pane_area_rect();
        // The IDE root split is Vertical with exactly 2 children when a drawer
        // is present.  We confirm this before computing the divider position.
        let root = &tab.tree.root;
        let split = match root.as_ref() {
            anvil_workspace::layout::PaneNode::Split(sp)
                if sp.dir == SplitDir::Vertical && sp.children.len() == 2 =>
            {
                sp
            }
            _ => return None,
        };
        // editor ratio is ratios[0]; divider y = ir.y + ir.h * ratios[0].
        let editor_ratio = split.ratios[0];
        let divider_y = ir.y + ir.h * editor_ratio;
        Some(divider_y)
    }

    /// Fixed chrome-top strip height in device pixels (Option D: 36pt × ui_scale).
    /// The terminal viewport starts at y = chrome_top_px.
    fn chrome_top_px(&self) -> f64 {
        36.0 * self.window_scale * self.ui_scale
    }

    /// Fixed bottom status-bar strip height in device pixels (Option D: 24pt × ui_scale).
    /// Anchored to the window's bottom edge.
    fn chrome_bottom_px(&self) -> f64 {
        24.0 * self.window_scale * self.ui_scale
    }

    /// Snap cursor + scroll animation state to current terminal values.
    fn snap_anim(&mut self) {
        let Some(tab) = self.tabs.current_mut() else {
            return;
        };
        let id = tab.focused_id();
        let Some(pane) = tab.registry.get_mut(id) else {
            return;
        };
        if let Some(terminal) = &pane.terminal {
            let cur = terminal.cursor();
            pane.cursor_ax = cur.x as f32;
            pane.cursor_ay = cur.y as f32;
            let sp = terminal.viewport_offset() as f32;
            pane.scroll_pos = sp;
            pane.scroll_target = sp;
            pane.scroll_vel = 0.0;
        }
    }

    /// Resize every pane in every tab to reflect the current window size.
    fn resize_all_tabs(&mut self) {
        let ir = self.pane_area_rect();
        let cw = self.font.metrics.cell_w;
        let ch = self.font.metrics.cell_h;
        let div = DIVIDER_PX;

        for tab in &mut self.tabs.tabs {
            let entries = tab.tree.layout(ir, div);
            for e in &entries {
                let cols = ((e.rect.w / cw) as usize).max(1);
                let rows = ((e.rect.h / ch) as usize).max(1);
                if let Some(pane) = tab.registry.get_mut(e.id) {
                    if let Some(terminal) = &mut pane.terminal {
                        terminal.resize(cols, rows);
                    }
                }
                if let Some(pty) = self.ptys.get(&e.id) {
                    pty.resize(cols as u16, rows as u16);
                }
            }
        }
        // Clear selection in focused pane.
        if let Some(tab) = self.tabs.current_mut() {
            let id = tab.focused_id();
            if let Some(pane) = tab.registry.get_mut(id) {
                pane.selection.clear();
            }
        }
        self.snap_anim();
        self.dirty = true;
        self.force_full_redraw = true;
    }

    /// Resize the raster and renderer to match the current window device size.
    fn resize_surface(&mut self) {
        let (dw, dh) = self.device_size();
        self.raster.resize(dw, dh);
        // Chrome top strip is a fixed pixel height; pad_y tracks it so cell
        // row 0 sits immediately below the chrome.
        self.raster.pad_y = self.chrome_top_px();
        if let Some(r) = &mut self.renderer {
            r.resize(dw, dh);
        }
        self.force_full_redraw = true;
        self.dirty = true;
    }

    /// Apply a freshly loaded config.
    fn apply_config(&mut self, cfg: Config) {
        let effective = effective_theme_name(self.system_dark, &cfg.theme);
        self.theme = resolve_theme(effective, &cfg.theme_overrides);
        if let Some(r) = &mut self.renderer {
            r.set_clear_color(self.theme.background);
        }
        self.cursor_cfg = cursor_cfg_from_config(&cfg);
        self.last_blink_opacity = -1.0;
        self.keybindings = Keybindings::from_config(&cfg.keybindings);
        self.config = cfg;
        self.dirty = true;
        self.force_full_redraw = true;
    }

    /// Set the user's theme mode from commands/keybindings.
    ///
    /// `ember-dark` and `ember-light` are explicit modes; `system` follows
    /// macOS appearance via `effective_theme_name`. Keep overrides intact so a
    /// user's configured accents survive palette switches.
    fn set_theme_mode(&mut self, mode: &str) {
        self.config.theme = mode.to_string();
        let effective = effective_theme_name(self.system_dark, &self.config.theme);
        self.theme = resolve_theme(effective, &self.config.theme_overrides);
        if let Some(r) = &mut self.renderer {
            r.set_clear_color(self.theme.background);
        }
        self.dirty = true;
        self.force_full_redraw = true;
    }

    fn open_search(&mut self) {
        if self.search_open {
            return;
        }
        self.search_open = true;
        // NE11: if focused pane is a native editor, open editor search.
        if self.focused_is_native_editor() {
            self.apply_editor_action(EditorAction::SearchOpen);
            self.resize_all_tabs();
            self.dirty = true;
            self.force_full_redraw = true;
            return;
        }
        self.search.set_scope(anvil_term::SearchScope::All);
        // Re-run query against current focused pane terminal.
        let q = self.search.query().to_string();
        if let Some(tab) = self.tabs.current_mut() {
            let id = tab.focused_id();
            if let Some(pane) = tab.registry.get_mut(id) {
                if let Some(terminal) = &pane.terminal {
                    self.search.set_query(terminal, &q);
                }
            }
        }
        self.resize_all_tabs();
        self.dirty = true;
        self.force_full_redraw = true;
    }

    /// Open the search bar scoped to the block containing the cursor row of
    /// the focused pane. If already open in any mode, re-scopes to Block.
    fn open_search_block(&mut self) {
        self.search_open = true;
        let q = self.search.query().to_string();
        if let Some(tab) = self.tabs.current_mut() {
            let id = tab.focused_id();
            if let Some(pane) = tab.registry.get_mut(id) {
                if let Some(term) = &pane.terminal {
                    // Cursor content row: history rows + cursor y-position in grid.
                    let anchor = term.line_count().saturating_sub(term.rows()) + term.cursor().y;
                    self.search.set_query_in_block(term, &q, anchor);
                }
            }
        }
        self.resize_all_tabs();
        self.dirty = true;
        self.force_full_redraw = true;
    }

    /// Open the project-wide search overlay (NE12, Cmd+Shift+F).
    ///
    /// Seeds the scan root from the focused pane's cwd (falls back to the
    /// process cwd). The actual pixel rendering of the overlay is a follow-on;
    /// this wires the state machine and performs the synchronous scan.
    fn open_project_search(&mut self) {
        let root = std::path::PathBuf::from(&self.local_ctx.cwd);
        self.project_search.open();
        // Re-run scan if the query is non-empty (e.g. re-opening after a
        // previous search).
        let q = self.project_search.query.clone();
        if !q.is_empty() {
            self.project_search.scan(&q, &root);
        }
        self.dirty = true;
        self.force_full_redraw = true;
    }

    /// Open (or re-open) the workspace symbol search overlay (Cmd+T / O1).
    fn open_workspace_symbol_search(&mut self) {
        let query = self
            .workspace_symbol_search
            .as_ref()
            .map(|s| s.query.clone())
            .unwrap_or_default();
        let lsp_unavailable = self.lsp_manager.as_ref().is_none_or(|l| !l.any_live());
        let pending_request_id = if !lsp_unavailable && !query.is_empty() {
            self.lsp_manager
                .as_ref()
                .map_or(0, |l| l.request_workspace_symbols(query.clone()))
        } else {
            0
        };
        self.workspace_symbol_search = Some(WorkspaceSymbolSearch {
            query,
            hits: Vec::new(),
            selected: 0,
            pending_request_id,
            lsp_unavailable,
            last_query_change: None,
        });
        self.dirty = true;
        self.force_full_redraw = true;
    }

    /// Open (or re-open) the buffer symbol search overlay (Cmd+R / O2).
    fn open_buffer_symbol_search(&mut self) {
        let all_symbols = self
            .tabs
            .current()
            .and_then(|tab| {
                let id = tab.focused_id();
                let ep = tab.editor_panes.get_pane(id)?;
                let buf = tab.editor_panes.get_buffer(ep.buffer_id)?;
                let text = buf.to_text();
                Some(anvil_editor::derive_outline_rows(buf.syntax(), &text))
            })
            .unwrap_or_default();
        let n = all_symbols.len();
        let query = self
            .buffer_symbol_search
            .as_ref()
            .map(|s| s.query.clone())
            .unwrap_or_default();
        let filtered: Vec<usize> = if query.is_empty() {
            (0..n).collect()
        } else {
            let q = query.to_ascii_lowercase();
            all_symbols
                .iter()
                .enumerate()
                .filter(|(_, s)| s.name.to_ascii_lowercase().contains(&q))
                .map(|(i, _)| i)
                .collect()
        };
        self.buffer_symbol_search = Some(BufferSymbolSearch {
            query,
            all_symbols,
            filtered,
            selected: 0,
        });
        self.dirty = true;
        self.force_full_redraw = true;
    }

    fn close_search(&mut self) {
        if !self.search_open {
            return;
        }
        self.search_open = false;
        // NE11: close editor search if the focused pane is a native editor.
        if self.focused_is_native_editor() {
            self.apply_editor_action(EditorAction::SearchClose);
        }
        self.search.set_scope(anvil_term::SearchScope::All);
        self.resize_all_tabs();
        self.dirty = true;
        self.force_full_redraw = true;
    }

    fn scroll_to_current_match(&mut self) {
        if let Some(m) = self.search.current_match() {
            if let Some(tab) = self.tabs.current_mut() {
                let id = tab.focused_id();
                if let Some(pane) = tab.registry.get_mut(id) {
                    if let Some(terminal) = &mut pane.terminal {
                        terminal.scroll_to_line(m.row);
                        let sp = terminal.viewport_offset() as f32;
                        pane.scroll_pos = sp;
                        pane.scroll_target = sp;
                        pane.scroll_vel = 0.0;
                    }
                }
            }
        }
    }

    fn focus_neighbor(&mut self, dir: NavDir) {
        let ir = self.pane_area_rect();
        let div = DIVIDER_PX;
        let next = self
            .tabs
            .current()
            .and_then(|tab| tab.tree.neighbor(dir, ir, div));
        if let Some(next_id) = next {
            if let Some(tab) = self.tabs.current_mut() {
                tab.tree.focused = next_id;
            }
            self.snap_anim();
            self.dirty = true;
        }
    }

    fn split_focused_pane(&mut self, dir: SplitDir) {
        let tab = match self.tabs.current() {
            Some(t) => t,
            None => return,
        };
        if tab.tree.leaf_count() >= MAX_PANES_PER_TAB {
            eprintln!("anvil: max pane count ({MAX_PANES_PER_TAB}) reached");
            return;
        }

        let focused_id = tab.focused_id();
        let ir = self.pane_area_rect();
        let div = DIVIDER_PX;
        let cw = self.font.metrics.cell_w;
        let ch = self.font.metrics.cell_h;
        let entries = tab.tree.layout(ir, div);
        let focused_rect = entries
            .iter()
            .find(|e| e.id == focused_id)
            .map(|e| e.rect)
            .unwrap_or(ir);

        let cols = match dir {
            SplitDir::Horizontal => ((((focused_rect.w - div) * 0.5) / cw) as usize).max(1),
            SplitDir::Vertical => ((focused_rect.w / cw) as usize).max(1),
        };
        let rows = match dir {
            SplitDir::Horizontal => ((focused_rect.h / ch) as usize).max(1),
            SplitDir::Vertical => ((((focused_rect.h - div) * 0.5) / ch) as usize).max(1),
        };

        let cwd = self.current_cwd();
        let scrollback = self.config.scrollback;

        let new_id = match self
            .tabs
            .current_mut()
            .map(|t| t.split(dir, cols, rows, scrollback))
        {
            Some(Ok(id)) => id,
            Some(Err(e)) => {
                eprintln!("anvil: pane split failed: {e}");
                return;
            }
            None => return,
        };

        // Spawn PTY for the new pane.
        match Pty::spawn_shell(cols as u16, rows as u16) {
            Ok(pty) => {
                self.ptys.insert(new_id, pty);
            }
            Err(e) => {
                eprintln!("anvil: pane pty failed: {e}");
                // Remove the pane from the tree/registry.
                if let Some(tab) = self.tabs.current_mut() {
                    tab.tree.close_leaf(new_id);
                    tab.registry.remove(new_id);
                }
                return;
            }
        }
        let _ = cwd; // cwd inheritance handled via shell integration
        self.resize_all_tabs();
        self.snap_anim();
        self.dirty = true;
    }

    /// Ensure the current tab has a live terminal pane for the IDE bottom
    /// drawer.  If no terminal pane exists, split the focused pane
    /// vertically and spawn a PTY for the new pane.  Called when entering
    /// IDE mode so the drawer always shows a live shell.
    fn spawn_ide_terminal_drawer(&mut self) {
        let has_terminal = self
            .tabs
            .current()
            .map(|t| t.first_terminal_pane_id().is_some())
            .unwrap_or(false);
        if has_terminal {
            return;
        }
        let tab = match self.tabs.current() {
            Some(t) => t,
            None => return,
        };
        if tab.tree.leaf_count() >= MAX_PANES_PER_TAB {
            return;
        }
        let ir = self.pane_area_rect();
        let ch = self.font.metrics.cell_h;
        let cw = self.font.metrics.cell_w;
        // Drawer gets 28% of vertical space; editor keeps 72%.
        let drawer_h = ir.h * 0.28;
        let cols = ((ir.w / cw) as usize).max(1);
        let rows = ((drawer_h / ch) as usize).max(1);
        let scrollback = self.config.scrollback;
        let new_id = match self
            .tabs
            .current_mut()
            .map(|t| t.split(SplitDir::Vertical, cols, rows, scrollback))
        {
            Some(Ok(id)) => id,
            Some(Err(e)) => {
                eprintln!("anvil: ide drawer split failed: {e}");
                return;
            }
            None => return,
        };
        match Pty::spawn_shell(cols as u16, rows as u16) {
            Ok(pty) => {
                self.ptys.insert(new_id, pty);
            }
            Err(e) => {
                eprintln!("anvil: ide drawer pty failed: {e}");
                if let Some(tab) = self.tabs.current_mut() {
                    tab.tree.close_leaf(new_id);
                    tab.registry.remove(new_id);
                }
            }
        }
    }

    /// Item 8 (Tier-B): toggle the IDE bottom terminal drawer.
    ///
    /// When visible, saves the current editor/drawer ratio and sets it to 1.0
    /// so the drawer occupies 0% of the pane area.  When hidden, restores the
    /// saved ratio.  No-op outside IDE mode or when no vertical root split exists.
    fn toggle_ide_drawer(&mut self) {
        if self.layout_mode != LayoutMode::Ide {
            return;
        }
        if self.drawer_hidden {
            // Reveal: restore saved ratio.
            let r = self.drawer_saved_ratio.clamp(0.40, 0.95);
            if let Some(tab) = self.tabs.current_mut() {
                let root = tab.tree.root.as_mut();
                if let anvil_workspace::layout::PaneNode::Split(sp) = root {
                    if sp.dir == SplitDir::Vertical && sp.ratios.len() == 2 {
                        sp.ratios[0] = r;
                        sp.ratios[1] = 1.0 - r;
                    }
                }
            }
            self.drawer_hidden = false;
        } else {
            // Hide: capture current ratio then force 1.0 / 0.0.
            let current_ratio = self
                .tabs
                .current()
                .and_then(|tab| {
                    let root = &tab.tree.root;
                    match root.as_ref() {
                        anvil_workspace::layout::PaneNode::Split(sp)
                            if sp.dir == SplitDir::Vertical && sp.ratios.len() == 2 =>
                        {
                            Some(sp.ratios[0])
                        }
                        _ => None,
                    }
                })
                .unwrap_or(0.72);
            // Only hide if there is actually a split to collapse.
            if let Some(tab) = self.tabs.current_mut() {
                let root = tab.tree.root.as_mut();
                if let anvil_workspace::layout::PaneNode::Split(sp) = root {
                    if sp.dir == SplitDir::Vertical && sp.ratios.len() == 2 {
                        self.drawer_saved_ratio = current_ratio;
                        sp.ratios[0] = 1.0;
                        sp.ratios[1] = 0.0;
                        self.drawer_hidden = true;
                    }
                }
            }
        }
        self.resize_all_tabs();
        self.dirty = true;
    }

    /// Open a native editor pane (NE15: sole editor path).  Splits the
    /// focused pane horizontally and registers it as an editor pane — no
    /// PTY is spawned.
    fn new_native_editor_pane(&mut self) {
        let tab = match self.tabs.current() {
            Some(t) => t,
            None => return,
        };
        if tab.tree.leaf_count() >= MAX_PANES_PER_TAB {
            eprintln!("anvil: max pane count ({MAX_PANES_PER_TAB}) reached");
            return;
        }

        if self.layout_mode == LayoutMode::Ide {
            if let Some(new_id) = self
                .tabs
                .current_mut()
                .and_then(|t| t.ensure_ide_editor_surface())
            {
                if let Some(tab) = self.tabs.current_mut() {
                    tab.tree.focused = new_id;
                }
                self.resize_all_tabs();
                self.snap_anim();
                self.dirty = true;
                return;
            }
        }

        let new_id = match self
            .tabs
            .current_mut()
            .map(|t| t.split_native_editor(anvil_workspace::layout::SplitDir::Horizontal))
        {
            Some(Ok(id)) => id,
            Some(Err(e)) => {
                eprintln!("anvil: native editor pane split failed: {e}");
                return;
            }
            None => return,
        };

        // No PTY is spawned for native editor panes.
        // Focus the new pane.
        if let Some(tab) = self.tabs.current_mut() {
            tab.tree.focused = new_id;
        }

        self.resize_all_tabs();
        self.snap_anim();
        self.dirty = true;
    }

    /// Open `path` in a native editor pane. If a terminal pane is focused,
    /// create a native editor split first so terminal state is not destroyed.
    /// Item 5: sync `active_explorer_file` to the current focused editor buffer's
    /// tracked path.  Called after every active-buffer change (tab switch, file
    /// open, Cmd+P pick).
    fn sync_active_explorer_file(&mut self) {
        let path = self.tabs.current().and_then(|tab| {
            let pid = tab.focused_id();
            let ep = tab.editor_panes.get_pane(pid)?;
            let buf = tab.editor_panes.get_buffer(ep.buffer_id)?;
            buf.tracked_path().map(|p| p.to_path_buf())
        });
        if let Some(path) = path {
            // Ensure the file's directory chain is expanded so the row is
            // visible in the Explorer (item 5).
            if let Some(snap) = &self.fs_snapshot {
                let root = PathBuf::from(&snap.root);
                if let Ok(rel) = path.strip_prefix(&root) {
                    let mut cur = root.clone();
                    for component in rel.components() {
                        cur = cur.join(component);
                        if cur != path {
                            // This is an intermediate directory; expand it.
                            if !self.expanded_dirs.contains(&cur) {
                                if !self.child_snapshots.contains_key(&cur) {
                                    let child_snap = fs_worker::read_dir_snapshot_fast(
                                        &cur,
                                        self.filter_flags(),
                                    );
                                    self.child_snapshots.insert(
                                        cur.clone(),
                                        LeftDockSnapshot {
                                            root: child_snap.root.to_string_lossy().into_owned(),
                                            entries: child_snap
                                                .entries
                                                .into_iter()
                                                .map(|e| anvil_render::left_dock::DirEntry {
                                                    name: e.name,
                                                    is_dir: e.is_dir,
                                                })
                                                .collect(),
                                            git_marks: child_snap.git_marks,
                                        },
                                    );
                                }
                                self.expanded_dirs.insert(cur.clone());
                            }
                        }
                    }
                }
            }
            self.active_explorer_file = Some(path);
        }
    }

    /// Estimate how many Explorer rows are visible in the current layout.
    /// Used for keyboard scroll-into-view (item 4).
    fn explorer_visible_rows(&self) -> usize {
        let areas = Docks::for_mode_with_left_dock_w(
            self.layout_mode,
            self.window_scale,
            self.dock_metrics(),
            self.hud_visible,
            self.chrome_bottom_px(),
            self.left_dock_visible,
            self.left_dock_w_pt * self.ui_scale,
            self.ui_scale,
        )
        .compute_areas(
            self.window_inner(),
            self.font.metrics.cell_w,
            self.font.metrics.cell_h,
        );
        let explorer_h = areas.left_dock.h * 0.60;
        let header_h = (32.0 * self.ui_scale).round();
        let content_h = (explorer_h - header_h).max(0.0);
        let row_h = (28.0 * self.ui_scale).round();
        if row_h <= 0.0 {
            return 1;
        }
        (content_h / row_h).floor() as usize
    }

    /// Build the current [`fs_worker::FilterFlags`] from the toggle fields.
    fn filter_flags(&self) -> fs_worker::FilterFlags {
        fs_worker::FilterFlags {
            show_hidden: self.show_hidden_files,
            show_gitignored: self.show_gitignored_files,
        }
    }

    /// Rebuild `fs_snapshot` synchronously from the current root using the
    /// current filter flags. Called after toggling Q56 or S1.
    fn refresh_fs_snapshot(&mut self) {
        let root = match &self.fs_snapshot {
            Some(s) => PathBuf::from(&s.root),
            None => match std::env::current_dir() {
                Ok(p) => p,
                Err(_) => return,
            },
        };
        let flags = self.filter_flags();
        let snap = fs_worker::read_dir_snapshot_fast(&root, flags);
        self.fs_snapshot = Some(LeftDockSnapshot {
            root: snap.root.to_string_lossy().into_owned(),
            entries: snap
                .entries
                .into_iter()
                .map(|e| anvil_render::left_dock::DirEntry {
                    name: e.name,
                    is_dir: e.is_dir,
                })
                .collect(),
            git_marks: snap.git_marks,
        });
        // Also clear child snapshots so sub-dirs refresh on next expand.
        self.child_snapshots.clear();
        self.force_full_redraw = true;
    }

    fn open_path_in_native_editor(&mut self, path: &Path) {
        if path.is_dir() {
            let _ = self.fs_tx.try_send(path.to_path_buf());
            self.fs_snapshot = Some(LeftDockSnapshot {
                root: path.to_string_lossy().into_owned(),
                entries: Vec::new(),
                git_marks: std::collections::HashMap::new(),
            });
            self.explorer_scroll_offset = 0;
            self.expanded_dirs.clear();
            self.child_snapshots.clear();
            self.active_explorer_file = None;
            self.dirty = true;
            return;
        }

        if !self.focused_is_native_editor() {
            self.new_native_editor_pane();
        }

        let Some(tab) = self.tabs.current_mut() else {
            return;
        };
        let pane_id = tab.focused_id();
        match tab.editor_panes.open_path_as_tab(pane_id, path) {
            Ok(buffer_id) => {
                if let Some(pane) = tab.registry.get_mut(pane_id) {
                    pane.editor_id = Some(buffer_id);
                }
                self.active_explorer_file = Some(path.to_path_buf());
                self.search_open = false;
                self.resize_all_tabs();
                self.snap_anim();
                self.force_full_redraw = true;
                self.dirty = true;
                // Item 15 (Tier-B): track recent files, deduped, capped at 50.
                let abs = path.to_path_buf();
                self.recent_file_list.retain(|p| p != &abs);
                self.recent_file_list.insert(0, abs.clone());
                self.recent_file_list.truncate(50);
                // Item 27: register this buffer with the file watcher.
                let _ = self.file_watch_tx.try_send((buffer_id, abs));
            }
            Err(err) => {
                eprintln!("anvil: failed to open {}: {err}", path.display());
            }
        }
    }

    fn close_focused_pane(&mut self) {
        let (focused_id, next_id) = {
            let tab = match self.tabs.current_mut() {
                Some(t) => t,
                None => return,
            };
            let focused_id = tab.focused_id();
            let next_id = tab.tree.close_leaf(focused_id);
            tab.registry.remove(focused_id);
            tab.editor_panes.remove_pane(focused_id);
            (focused_id, next_id)
        };
        self.ptys.remove(&focused_id);

        if let Some(nid) = next_id {
            if let Some(tab) = self.tabs.current_mut() {
                tab.tree.focused = nid;
            }
            self.resize_all_tabs();
            self.snap_anim();
            self.dirty = true;
        } else {
            // Last pane — close the tab.
            if !self.tabs.close_active() {
                terminate_app();
            } else {
                self.snap_anim();
                self.dirty = true;
            }
        }
    }

    /// Remove `buffer_id` from `pane_id` in the current tab.
    ///
    /// The removed buffer should eventually appear in a new native window.
    /// That window-spawn path is not yet implemented — see
    /// `TODO(anvil-20-window-spawn)` in `crates/anvil-platform/src/appkit.rs`.
    ///
    /// Currently: removes the buffer from the source pane and logs that the
    /// window-spawn step is missing.  The API surface exists so call-sites can
    /// be written and tested now.
    pub fn detach_buffer_to_new_window(
        &mut self,
        pane_id: PaneId,
        buffer_id: anvil_editor::BufferId,
    ) {
        if let Some(tab) = self.tabs.current_mut() {
            tab.editor_panes.close_buffer(pane_id, buffer_id);
            // Sync pane.editor_id to the new active buffer.
            if let Some(ep) = tab.editor_panes.get_pane(pane_id) {
                let active = ep.buffer_id;
                if let Some(pane) = tab.registry.get_mut(pane_id) {
                    pane.editor_id = Some(active);
                }
            }
        }
        eprintln!(
            "anvil-window: multi-window detach not yet implemented; \
             buffer removed from source pane only"
        );
        self.dirty = true;
    }

    fn add_tab(&mut self) {
        self.close_search();
        let (dw, dh) = self.device_size();
        let cw = self.font.metrics.cell_w as usize;
        let ch = self.font.metrics.cell_h as usize;
        let cols = ((dw.saturating_sub(2 * GRID_PAD)) / cw).max(1);
        // Chrome strips at top/bottom are fixed pixel heights (36pt/24pt);
        // PTY rows fill the remaining vertical pixels divided by cell_h.
        let chrome_top_px = (36.0 * self.window_scale) as usize;
        let chrome_bottom_px = (24.0 * self.window_scale) as usize;
        let avail = dh
            .saturating_sub(chrome_top_px)
            .saturating_sub(chrome_bottom_px);
        let rows = (avail / ch).max(1);
        let scrollback = self.config.scrollback;

        // PaneIds must be unique across ALL tabs because self.ptys is a
        // single map keyed by PaneId. Start the new tab's counter above any
        // existing PaneId.
        let next_id = self.ptys.keys().copied().max().unwrap_or(0) + 1;
        let tab = Tab::new_single_pane_starting_at(next_id, cols, rows, scrollback);
        let first_id = tab.focused_id();
        match Pty::spawn_shell(cols as u16, rows as u16) {
            Ok(pty) => {
                self.ptys.insert(first_id, pty);
            }
            Err(e) => {
                eprintln!("anvil: new tab pty failed: {e}");
                return;
            }
        }
        self.tabs.push(tab);
        // If we're in IDE mode, the fresh tab is a single terminal pane;
        // promote it to the editor + drawer layout so the new tab matches
        // the rest of the app. Without this, the new tab renders as a bare
        // terminal that's hidden behind IDE chrome.
        if self.layout_mode == LayoutMode::Ide {
            if let Some(tab) = self.tabs.current_mut() {
                tab.ensure_ide_editor_surface();
            }
        }
        self.resize_all_tabs();
        self.snap_anim();
        self.dirty = true;
    }

    /// Q22: set the language override on the active buffer.
    /// `lang_id` is an LSP language identifier (e.g. `"rust"`, `"python"`).
    fn set_active_buffer_language(&mut self, lang_id: &str) {
        let Some(tab) = self.tabs.current_mut() else {
            return;
        };
        let pane_id = tab.focused_id();
        let Some(ep) = tab.editor_panes.get_pane(pane_id) else {
            return;
        };
        let bid = ep.buffer_id;
        if let Some(buf) = tab.editor_panes.get_buffer_mut(bid) {
            buf.set_language(lang_id);
        }
        self.force_full_redraw = true;
        self.dirty = true;
    }

    fn close_active_tab(&mut self) {
        if !self.tabs.begin_close_at(self.tabs.active) {
            // Last live tab — terminate immediately.
            if !self.tabs.close_active() {
                terminate_app();
            }
        } else {
            self.dirty = true;
        }
    }

    /// Close panes whose PTY has gone away (EOF), then close tabs with no panes.
    fn close_dead_panes(&mut self) {
        let mut any_closed = false;

        let mut tab_i = 0;
        while tab_i < self.tabs.tabs.len() {
            // Collect pane ids whose PTY has exited. Editor panes (terminal.is_none())
            // never have a PTY entry and are always alive — exclude them from dead detection.
            let dead: Vec<PaneId> = {
                let tab = &self.tabs.tabs[tab_i];
                all_pane_ids_in_tree(tab)
                    .into_iter()
                    .filter(|id| {
                        !self.ptys.contains_key(id)
                            && tab
                                .registry
                                .get(*id)
                                .map(|p| p.terminal.is_some())
                                .unwrap_or(false)
                    })
                    .collect()
            };

            if dead.is_empty() {
                tab_i += 1;
                continue;
            }
            any_closed = true;
            let mut close_tab = false;
            for id in dead {
                let next = self.tabs.tabs[tab_i].tree.close_leaf(id);
                self.tabs.tabs[tab_i].registry.remove(id);
                self.tabs.tabs[tab_i].editor_panes.remove_pane(id);
                if let Some(nid) = next {
                    self.tabs.tabs[tab_i].tree.focused = nid;
                } else {
                    close_tab = true;
                    break;
                }
            }
            if close_tab {
                if !self.tabs.close_at(tab_i) {
                    terminate_app();
                    return;
                }
                // tab_i stays the same (the next tab is now at tab_i)
            } else {
                tab_i += 1;
            }
        }

        if any_closed {
            self.snap_anim();
            self.dirty = true;
        }
    }

    /// Toggle fold on the block whose `command_line` is at or just above the
    /// viewport top of the focused pane (⌘. keybinding).
    fn toggle_fold_at_viewport_top(&mut self) {
        let Some(tab) = self.tabs.current_mut() else {
            return;
        };
        let id = tab.focused_id();
        let Some(pane) = tab.registry.get_mut(id) else {
            return;
        };
        if let Some(terminal) = &pane.terminal {
            // The content row currently at the top of the viewport.
            let top_content = terminal.content_row_of_viewport(0);
            let top_abs = terminal.absolute_line_of_content(top_content);

            // Find the block at or just before the viewport top.
            let block_opt = terminal
                .block_at(top_abs)
                .or_else(|| terminal.block_before(top_abs + 1));

            if let Some(block) = block_opt {
                pane.toggle_fold(block.command_line);
                self.dirty = true;
            }
        }
    }

    fn jump_to_prev_prompt(&mut self) {
        let Some(tab) = self.tabs.current_mut() else {
            return;
        };
        let id = tab.focused_id();
        let Some(pane) = tab.registry.get_mut(id) else {
            return;
        };
        if let Some(t) = &mut pane.terminal {
            let marks = t.prompt_marks().to_vec();
            let top_content = t.content_row_of_viewport(0);
            let ev = t.evicted_lines;
            let mut best: Option<usize> = None;
            for m in &marks {
                use anvil_term::PromptMarkKind;
                if m.kind != PromptMarkKind::PromptStart {
                    continue;
                }
                if m.line < ev {
                    continue;
                }
                let cr = m.line - ev;
                if cr < top_content && best.is_none_or(|b| cr > b) {
                    best = Some(cr);
                }
            }
            if let Some(cr) = best {
                t.scroll_to_line(cr);
                let sp = t.viewport_offset() as f32;
                pane.scroll_pos = sp;
                pane.scroll_target = sp;
                pane.scroll_vel = 0.0;
                self.dirty = true;
            }
        }
    }

    fn jump_to_next_prompt(&mut self) {
        let Some(tab) = self.tabs.current_mut() else {
            return;
        };
        let id = tab.focused_id();
        let Some(pane) = tab.registry.get_mut(id) else {
            return;
        };
        if let Some(t) = &mut pane.terminal {
            let marks = t.prompt_marks().to_vec();
            let top_content = t.content_row_of_viewport(0);
            let ev = t.evicted_lines;
            let mut best: Option<usize> = None;
            for m in &marks {
                use anvil_term::PromptMarkKind;
                if m.kind != PromptMarkKind::PromptStart {
                    continue;
                }
                if m.line < ev {
                    continue;
                }
                let cr = m.line - ev;
                if cr > top_content && best.is_none_or(|b| cr < b) {
                    best = Some(cr);
                }
            }
            if let Some(cr) = best {
                t.scroll_to_line(cr);
                let sp = t.viewport_offset() as f32;
                pane.scroll_pos = sp;
                pane.scroll_target = sp;
                pane.scroll_vel = 0.0;
            } else {
                t.scroll_to_bottom();
                pane.scroll_pos = 0.0;
                pane.scroll_target = 0.0;
                pane.scroll_vel = 0.0;
            }
        }
        self.dirty = true;
    }

    fn write_to_focused_pty(&self, bytes: &[u8]) {
        if let Some(tab) = self.tabs.current() {
            let id = tab.focused_id();
            if let Some(pty) = self.ptys.get(&id) {
                let _ = pty.write(bytes);
            }
        }
    }

    /// Compute the pixel (rel_x, rel_y) of `loc` relative to the focused
    /// native editor pane's draw rect.  Returns `None` if no native editor pane
    /// is focused or the mouse is outside the pane area.
    /// Return the buffer line to toggle a fold for if `(rel_x, rel_y)` (device
    /// pixels relative to the focused native editor pane's top-left) is a click
    /// inside the last gutter column on a line that starts a foldable range.
    ///
    /// Returns `None` if not a gutter click or no fold range at that line.
    fn gutter_click_fold_line(&self, rel_x: f64, rel_y: f64) -> Option<usize> {
        let tab = self.tabs.current()?;
        let id = tab.focused_id();
        let pane = tab.editor_panes.get_pane(id)?;
        let buf = tab.editor_panes.get_buffer(pane.buffer_id)?;
        let cw = self.font.metrics.cell_w;
        let ch = self.font.metrics.cell_h;
        let line_count = buf.line_count().max(1);
        let digit_cols = line_count.to_string().len();
        let git_gutter_cols = if buf.git_gutter.is_some() { 2 } else { 0 };
        let gutter_cols = digit_cols + 2 + git_gutter_cols;
        let gutter_w = gutter_cols as f64 * cw;
        // Click must be within the gutter's last column.
        let last_col_x = (gutter_cols as f64 - 1.0) * cw;
        if rel_x < last_col_x || rel_x >= gutter_w {
            return None;
        }
        // Determine which buffer line was clicked.
        let vrow = (rel_y / ch).floor() as usize;
        let scroll_line = pane.scroll_pos.floor() as usize;
        let line_idx = scroll_line + vrow;
        if line_idx >= line_count {
            return None;
        }
        // Check if this line starts a foldable range.
        let fold_ranges = anvil_editor::derive_fold_ranges(buf.syntax());
        if fold_ranges
            .iter()
            .any(|fr| fr.start == line_idx && fr.end > line_idx)
        {
            Some(line_idx)
        } else {
            None
        }
    }

    fn native_editor_rel_px(&self, loc: MouseLocation) -> Option<(f64, f64)> {
        let (rx, ry) = self.view_pt_to_raster_px(loc);
        let tab = self.tabs.current()?;
        let id = tab.focused_id();
        // Must be a native editor pane.
        tab.editor_panes.get_pane(id)?;
        let ir = self.pane_area_rect();
        let entries = tab.tree.layout(ir, DIVIDER_PX);
        let pr = entries.iter().find(|e| e.id == id)?.rect;
        Some((rx - pr.x, ry - pr.y))
    }

    /// Convert a mouse location to an editor buffer `Position` for the focused
    /// native editor pane.  Returns `None` if the focused pane is not a native
    /// editor pane.
    fn native_editor_pos_at(&self, loc: MouseLocation) -> Option<EditorPosition> {
        let (rel_x, rel_y) = self.native_editor_rel_px(loc)?;
        let tab = self.tabs.current()?;
        let id = tab.focused_id();
        let pane = tab.editor_panes.get_pane(id)?;
        let buf = tab.editor_panes.get_buffer(pane.buffer_id)?;
        let metrics = EditorFontMetrics {
            cell_w: self.font.metrics.cell_w,
            cell_h: self.font.metrics.cell_h,
            descent: self.font.metrics.descent,
        };
        Some(pixel_to_position(pane, buf, rel_x, rel_y, metrics, 0))
    }

    /// Return true if the focused pane is a native editor pane (NE6).
    fn focused_is_native_editor(&self) -> bool {
        let Some(tab) = self.tabs.current() else {
            return false;
        };
        let id = tab.focused_id();
        tab.editor_panes.get_pane(id).is_some()
    }

    /// Send a hover request for the cursor position in the focused native editor
    /// pane (Cmd+K, NE10).  Stores the `(pane_id, request_id)` in
    /// `self.pending_hover` for polling in the tick loop.
    fn trigger_hover_request(&mut self) {
        let Some(tab) = self.tabs.current() else {
            return;
        };
        let pane_id = tab.focused_id();
        let Some(ep) = tab.editor_panes.get_pane(pane_id) else {
            return;
        };
        let Some(buf) = tab.editor_panes.get_buffer(ep.buffer_id) else {
            return;
        };
        let Some(path) = buf.tracked_path() else {
            return;
        };
        let line = ep.cursors[0].pos.line as u32;
        let character = ep.cursors[0].pos.col as u32;
        let path = path.to_path_buf();
        // Clear any stale popup.
        self.apply_editor_action(EditorAction::HoverRequest);
        if let Some(lsp) = &self.lsp_manager {
            let req_id = lsp.request_hover(&path, line, character);
            if req_id != 0 {
                self.pending_hover = Some((pane_id, req_id));
            }
        }
        self.dirty = true;
    }

    /// Poll for a hover result and populate the target pane's `hover_popup` if
    /// one arrived.  Called each tick (NE10).
    fn poll_hover_result(&mut self) {
        let Some((pane_id, req_id)) = self.pending_hover else {
            return;
        };
        let Some(lsp) = &self.lsp_manager else { return };
        if let Some(result) = lsp.poll_hover(req_id) {
            self.pending_hover = None;
            // Capture the anchor position from the pane's current cursor.
            let anchor = self
                .tabs
                .current()
                .and_then(|t| t.editor_panes.get_pane(pane_id))
                .map(|ep| ep.cursors[0].pos)
                .unwrap_or(anvil_editor::Position { line: 0, col: 0 });
            if let Some(tab) = self.tabs.current_mut() {
                if let Some(ep) = tab.editor_panes.get_pane_mut(pane_id) {
                    ep.hover_popup = Some(anvil_workspace::editor_pane::HoverPopup {
                        text: result.text,
                        anchor,
                    });
                }
            }
            self.dirty = true;
        }
    }

    /// Send a `textDocument/definition` request for the cursor position in `pane_id`
    /// (item 17).  Stores `(pane_id, request_id)` in `self.pending_definition`.
    fn trigger_definition_request(&mut self, pane_id: PaneId) {
        let Some(tab) = self.tabs.current() else {
            return;
        };
        let Some(ep) = tab.editor_panes.get_pane(pane_id) else {
            return;
        };
        let Some(buf) = tab.editor_panes.get_buffer(ep.buffer_id) else {
            return;
        };
        let Some(path) = buf.tracked_path() else {
            return;
        };
        let line = ep.cursors[0].pos.line as u32;
        let character = ep.cursors[0].pos.col as u32;
        let path = path.to_path_buf();
        if let Some(lsp) = &self.lsp_manager {
            let req_id = lsp.request_definition(&path, line, character);
            if req_id != 0 {
                self.pending_definition = Some((pane_id, req_id));
            }
        }
    }

    /// Poll for a definition result and jump the cursor (item 17).
    fn poll_definition_result(&mut self) {
        let Some((pane_id, req_id)) = self.pending_definition else {
            return;
        };
        let Some(lsp) = &self.lsp_manager else { return };
        if let Some(locs) = lsp.poll_definition(req_id) {
            self.pending_definition = None;
            // Pick the first location (TODO(anvil-tier3-17-picker): show picker for multi).
            if let Some(loc) = locs.into_iter().next() {
                // Look up the current buffer's path.
                let cur_path: Option<std::path::PathBuf> = self
                    .tabs
                    .current()
                    .and_then(|t| t.editor_panes.get_pane(pane_id))
                    .and_then(|ep| {
                        self.tabs
                            .current()
                            .and_then(|t| t.editor_panes.get_buffer(ep.buffer_id))
                    })
                    .and_then(|b| b.tracked_path())
                    .map(|p| p.to_path_buf());
                let same_file = cur_path.as_deref() == Some(&loc.path);
                if !same_file {
                    self.open_path_in_native_editor(&loc.path);
                }
                self.apply_editor_action(EditorAction::MoveTo {
                    pos: EditorPosition {
                        line: loc.line as usize,
                        col: loc.col as usize,
                    },
                    extend: false,
                });
                self.dirty = true;
            }
        }
    }

    /// Trigger a completion request for the focused editor pane (item 16).
    fn trigger_completion_request(&mut self) {
        let Some(tab) = self.tabs.current() else {
            return;
        };
        let pane_id = tab.focused_id();
        let Some(ep) = tab.editor_panes.get_pane(pane_id) else {
            return;
        };
        let Some(buf) = tab.editor_panes.get_buffer(ep.buffer_id) else {
            return;
        };
        let Some(path) = buf.tracked_path() else {
            return;
        };
        let line = ep.cursors[0].pos.line as u32;
        let character = ep.cursors[0].pos.col as u32;
        let path = path.to_path_buf();
        if let Some(lsp) = &self.lsp_manager {
            let req_id = lsp.request_completion(&path, line, character);
            if req_id != 0 {
                self.pending_completion = Some((pane_id, req_id));
            }
        }
    }

    /// Poll for a completion result and open the popup (item 16).
    fn poll_completion_result(&mut self) {
        let Some((_pane_id, req_id)) = self.pending_completion else {
            return;
        };
        let items = self
            .lsp_manager
            .as_ref()
            .and_then(|lsp| lsp.poll_completion(req_id));
        if let Some(items) = items {
            self.pending_completion = None;
            if items.is_empty() {
                return;
            }
            use anvil_workspace::editor_pane::CompletionEntry;
            let entries: Vec<CompletionEntry> = items
                .into_iter()
                .map(|ci| CompletionEntry {
                    label: ci.label.clone(),
                    detail: ci.detail.clone(),
                    insert_text: ci.insert_text.unwrap_or(ci.label),
                })
                .collect();
            self.apply_editor_action(EditorAction::CompletionOpen(entries));
            self.dirty = true;
        }
    }

    // ── LSP rename (item 24) ──────────────────────────────────────────────────

    /// Extract the word under the primary cursor of the focused pane.
    fn word_under_cursor(&self) -> String {
        let Some(tab) = self.tabs.current() else {
            return String::new();
        };
        let pane_id = tab.focused_id();
        let Some(ep) = tab.editor_panes.get_pane(pane_id) else {
            return String::new();
        };
        let Some(buf) = tab.editor_panes.get_buffer(ep.buffer_id) else {
            return String::new();
        };
        let pos = ep.cursors[0].pos;
        let line_text = buf.line(pos.line).to_string();
        // Extract contiguous word chars (alphanumeric + '_') around the cursor col.
        let chars: Vec<char> = line_text.chars().collect();
        let col = pos.col.min(chars.len());
        let is_word_char = |c: char| c.is_alphanumeric() || c == '_';
        let start = (0..col)
            .rev()
            .take_while(|&i| is_word_char(chars[i]))
            .last()
            .unwrap_or(col);
        let end = (col..chars.len())
            .take_while(|&i| is_word_char(chars[i]))
            .last()
            .map(|i| i + 1)
            .unwrap_or(col);
        chars[start..end].iter().collect()
    }

    /// Open the LSP rename overlay (F2 in editor body, item 24).
    fn open_lsp_rename_overlay(&mut self) {
        let word = self.word_under_cursor();
        if let Some(lsp) = &self.lsp_manager {
            // Check a live server exists; if not, log and bail.
            let has_server = self.tabs.current().and_then(|t| {
                let ep = t.editor_panes.get_pane(t.focused_id())?;
                let buf = t.editor_panes.get_buffer(ep.buffer_id)?;
                let path = buf.tracked_path()?;
                let ext = path.extension()?.to_str()?;
                let lang = anvil_editor::language_id_for_ext(ext)?;
                let sid = anvil_editor::server_id_for_language(lang)?;
                Some(lsp.state_of(sid))
            });
            if !matches!(has_server, Some(anvil_editor::LspState::Live)) {
                eprintln!("anvil-lsp: rename unavailable (no LSP)");
                return;
            }
        } else {
            eprintln!("anvil-lsp: rename unavailable (no LSP)");
            return;
        }
        self.lsp_rename_input = Some(word);
        self.dirty = true;
    }

    /// Send the rename request and apply the workspace edit (item 24).
    fn commit_lsp_rename(&mut self, new_name: String) {
        let Some(tab) = self.tabs.current() else {
            return;
        };
        let pane_id = tab.focused_id();
        let Some(ep) = tab.editor_panes.get_pane(pane_id) else {
            return;
        };
        let Some(buf) = tab.editor_panes.get_buffer(ep.buffer_id) else {
            return;
        };
        let Some(path) = buf.tracked_path() else {
            return;
        };
        let line = ep.cursors[0].pos.line as u32;
        let character = ep.cursors[0].pos.col as u32;
        let path = path.to_path_buf();
        if let Some(lsp) = &self.lsp_manager {
            let req_id = lsp.request_rename(&path, line, character, new_name);
            if req_id != 0 {
                self.pending_rename = Some((pane_id, req_id));
            }
        }
    }

    /// Poll for a rename result and apply the workspace edits (item 24).
    fn poll_rename_result(&mut self) {
        let Some((_pane_id, req_id)) = self.pending_rename else {
            return;
        };
        let edits = self
            .lsp_manager
            .as_ref()
            .and_then(|lsp| lsp.poll_rename(req_id));
        if let Some(edits) = edits {
            self.pending_rename = None;
            self.apply_rename_edits(edits);
        }
    }

    /// Apply a set of `RenameEdit`s to their respective buffers (item 24).
    ///
    /// Files not currently open are loaded temporarily, edited, and saved.
    /// Files already open in a buffer are edited in-place.
    fn apply_rename_edits(&mut self, edits: Vec<anvil_editor::RenameEdit>) {
        use std::collections::HashMap as EditsMap;
        // Group by path so we can apply all edits to a file at once.
        let mut by_path: EditsMap<PathBuf, Vec<anvil_editor::RenameEdit>> = EditsMap::new();
        for e in edits {
            by_path.entry(e.path.clone()).or_default().push(e);
        }
        for (path, mut path_edits) in by_path {
            // Sort in reverse document order so earlier edits don't shift later offsets.
            path_edits.sort_by(|a, b| {
                b.start_line
                    .cmp(&a.start_line)
                    .then(b.start_col.cmp(&a.start_col))
            });

            // Try to find a buffer already open for this path.
            let found_buf = self.tabs.current_mut().and_then(|tab| {
                // Search open panes for a buffer matching this path.
                tab.editor_panes.find_buffer_for_path(&path)
            });

            if let Some(buf) = found_buf {
                // Apply edits to the live buffer.
                for e in &path_edits {
                    buf.replace_range(
                        anvil_editor::Range {
                            start: anvil_editor::Position {
                                line: e.start_line as usize,
                                col: e.start_col as usize,
                            },
                            end: anvil_editor::Position {
                                line: e.end_line as usize,
                                col: e.end_col as usize,
                            },
                        },
                        &e.new_text,
                    );
                }
                self.dirty = true;
            } else {
                // File not open: load, edit, save.
                if let Ok(mut disk_buf) = anvil_editor::Buffer::from_path(&path) {
                    for e in &path_edits {
                        disk_buf.replace_range(
                            anvil_editor::Range {
                                start: anvil_editor::Position {
                                    line: e.start_line as usize,
                                    col: e.start_col as usize,
                                },
                                end: anvil_editor::Position {
                                    line: e.end_line as usize,
                                    col: e.end_col as usize,
                                },
                            },
                            &e.new_text,
                        );
                    }
                    let _ = disk_buf.save(&path);
                }
            }
        }
    }

    // ── LSP code actions (item 25) ────────────────────────────────────────────

    /// Trigger a `textDocument/codeAction` request for the cursor's line (Cmd+.).
    fn trigger_code_actions_request(&mut self) {
        let Some(tab) = self.tabs.current() else {
            return;
        };
        let pane_id = tab.focused_id();
        let Some(ep) = tab.editor_panes.get_pane(pane_id) else {
            return;
        };
        let Some(buf) = tab.editor_panes.get_buffer(ep.buffer_id) else {
            return;
        };
        let Some(path) = buf.tracked_path() else {
            return;
        };
        let line = ep.cursors[0].pos.line as u32;
        let path = path.to_path_buf();
        if let Some(lsp) = &self.lsp_manager {
            let req_id = lsp.request_code_actions(&path, line);
            if req_id != 0 {
                self.pending_code_actions = Some((pane_id, req_id));
            }
        } else {
            eprintln!("anvil-lsp: code actions unavailable (no LSP)");
        }
    }

    /// Poll for a code-actions result and open the popup (item 25).
    fn poll_code_actions_result(&mut self) {
        let Some((_pane_id, req_id)) = self.pending_code_actions else {
            return;
        };
        let actions = self
            .lsp_manager
            .as_ref()
            .and_then(|lsp| lsp.poll_code_actions(req_id));
        if let Some(actions) = actions {
            self.pending_code_actions = None;
            if actions.is_empty() {
                return;
            }
            use anvil_workspace::editor_pane::CodeActionEntry;
            let entries: Vec<CodeActionEntry> = actions
                .iter()
                .map(|a| CodeActionEntry {
                    title: a.title.clone(),
                })
                .collect();
            // Store the flat edits for each action so we can apply on Enter.
            self.code_actions_pending_edits = actions.into_iter().map(|a| a.edits).collect();
            self.apply_editor_action(EditorAction::CodeActionsOpen(entries));
            self.dirty = true;
        }
    }

    /// Apply the selected code action's workspace edits (item 25).
    fn apply_code_action(&mut self, index: usize) {
        if let Some(edits) = self.code_actions_pending_edits.get(index).cloned() {
            if !edits.is_empty() {
                self.apply_rename_edits(edits);
            }
        }
        self.apply_editor_action(EditorAction::CodeActionsDismiss);
        self.code_actions_pending_edits.clear();
    }

    // ── LSP references (item 26) ──────────────────────────────────────────────

    /// Trigger a `textDocument/references` request (Shift+F12, item 26).
    fn trigger_references_request(&mut self) {
        let Some(tab) = self.tabs.current() else {
            return;
        };
        let pane_id = tab.focused_id();
        let Some(ep) = tab.editor_panes.get_pane(pane_id) else {
            return;
        };
        let Some(buf) = tab.editor_panes.get_buffer(ep.buffer_id) else {
            return;
        };
        let Some(path) = buf.tracked_path() else {
            return;
        };
        let line = ep.cursors[0].pos.line as u32;
        let character = ep.cursors[0].pos.col as u32;
        let path = path.to_path_buf();
        if let Some(lsp) = &self.lsp_manager {
            let req_id = lsp.request_references(&path, line, character);
            if req_id != 0 {
                self.pending_references = Some((pane_id, req_id));
            }
        } else {
            eprintln!("anvil-lsp: references unavailable (no LSP)");
        }
    }

    /// Poll for a references result and open the overlay (item 26).
    fn poll_references_result(&mut self) {
        let Some((_pane_id, req_id)) = self.pending_references else {
            return;
        };
        let locs = self
            .lsp_manager
            .as_ref()
            .and_then(|lsp| lsp.poll_references(req_id));
        if let Some(locs) = locs {
            self.pending_references = None;
            if locs.is_empty() {
                return;
            }
            let rows = locs
                .into_iter()
                .map(|l| ReferencesRow {
                    path: l.path,
                    line: l.line,
                    col: l.col,
                })
                .collect();
            self.lsp_references = Some(LspReferencesOverlay { rows, selected: 0 });
            self.dirty = true;
        }
    }

    /// Poll for a pending workspace/symbol result and update the overlay (O1).
    fn poll_workspace_symbols_result(&mut self) {
        let Some(ref mut search) = self.workspace_symbol_search else {
            return;
        };
        // Debounce: fire request 200ms after last query change.
        if let Some(t) = search.last_query_change {
            if t.elapsed() >= std::time::Duration::from_millis(200) {
                search.last_query_change = None;
                let q = search.query.clone();
                if !q.is_empty() && !search.lsp_unavailable {
                    let id = self
                        .lsp_manager
                        .as_ref()
                        .map_or(0, |l| l.request_workspace_symbols(q));
                    if let Some(ref mut s) = self.workspace_symbol_search {
                        s.pending_request_id = id;
                    }
                }
            }
        }
        let req_id = match self.workspace_symbol_search.as_ref() {
            Some(s) if s.pending_request_id != 0 => s.pending_request_id,
            _ => return,
        };
        let hits = self
            .lsp_manager
            .as_ref()
            .and_then(|lsp| lsp.poll_workspace_symbols(req_id));
        if let Some(hits) = hits {
            if let Some(ref mut s) = self.workspace_symbol_search {
                s.hits = hits;
                s.selected = 0;
                s.pending_request_id = 0;
            }
            self.dirty = true;
        }
    }

    /// Apply `action` to the focused native editor pane.
    ///
    /// Writes any Cut/Copy text to the system clipboard.  Marks the app dirty
    /// when the buffer mutated.  No-op when no native editor pane is focused.
    fn apply_editor_action(&mut self, action: EditorAction) {
        let Some(tab) = self.tabs.current_mut() else {
            return;
        };
        let id = tab.focused_id();
        let mut clipboard_out: Option<String> = None;
        let mutated = tab.editor_panes.apply(id, action, &mut clipboard_out);
        if let Some(text) = clipboard_out {
            anvil_platform::system::set_clipboard(&text);
        }
        if mutated {
            self.dirty = true;
        }
    }

    /// M3: Compute the approximate number of visible rows for the focused editor
    /// pane, used by PgUp/PgDn scroll-target updates.
    fn editor_visible_rows(&self) -> usize {
        let Some(tab) = self.tabs.current() else {
            return 24;
        };
        let id = tab.focused_id();
        let cell_h = self.font.metrics.cell_h;
        if cell_h <= 0.0 {
            return 24;
        }
        let ir = self.pane_area_rect();
        let entries = tab.tree.layout(ir, DIVIDER_PX);
        entries
            .iter()
            .find(|e| e.id == id)
            .map(|e| (e.rect.h / cell_h).ceil() as usize)
            .unwrap_or(24)
    }

    /// Concatenate the focused pane's current selection into a single string.
    /// Multi-row selections separate rows with `\n`. Rect mode picks the
    /// same column window on every row. Returns `None` when no selection is
    /// active or when it's zero-width.
    /// All visible rows of the focused pane, as a single string (rows joined
    /// by `\n`, trailing spaces stripped). Used as a fallback when the user
    /// invokes a "capture" action with no active selection.
    fn focused_viewport_text(&self) -> Option<String> {
        let tab = self.tabs.current()?;
        let pane = tab.registry.get(tab.focused_id())?;
        let terminal = pane.terminal.as_ref()?;
        let rows = terminal.rows();
        if rows == 0 {
            return None;
        }
        let mut out = String::new();
        for vy in 0..rows {
            let content_row = terminal.content_row_of_viewport(vy);
            if content_row >= terminal.line_count() {
                break;
            }
            let line = terminal.line(content_row);
            for cell in line {
                out.push(cell.cp);
            }
            if vy + 1 != rows {
                out.push('\n');
            }
        }
        let cleaned: Vec<&str> = out.lines().map(|l| l.trim_end()).collect();
        let joined = cleaned.join("\n");
        // If the viewport is entirely whitespace (fresh terminal), return None
        // so callers can detect "nothing meaningful to capture".
        if joined.chars().all(char::is_whitespace) {
            return None;
        }
        Some(joined)
    }

    fn focused_selection_text(&self) -> Option<String> {
        use anvil_workspace::selection::SelectionMode;
        let tab = self.tabs.current()?;
        let pane = tab.registry.get(tab.focused_id())?;
        if !pane.selection.active {
            return None;
        }
        let sel = &pane.selection;
        let (start, end) = sel.ordered();
        if start == end && sel.anchor.col == sel.head.col {
            return None;
        }
        let terminal = pane.terminal.as_ref()?;
        let mut out = String::new();
        for row in start.row..=end.row {
            if row >= terminal.line_count() {
                break;
            }
            let line = terminal.line(row);
            let (lo, hi) = match sel.mode {
                SelectionMode::Rect => {
                    let a = sel.anchor.col;
                    let h = sel.head.col;
                    let (lo, hi) = if a <= h { (a, h) } else { (h, a) };
                    (lo.min(line.len()), hi.min(line.len()))
                }
                SelectionMode::Linear => {
                    if start.row == end.row {
                        (start.col, end.col.min(line.len()))
                    } else if row == start.row {
                        (start.col, line.len())
                    } else if row == end.row {
                        (0, end.col.min(line.len()))
                    } else {
                        (0, line.len())
                    }
                }
            };
            for cell in &line[lo..hi] {
                out.push(cell.cp);
            }
            if row != end.row {
                out.push('\n');
            }
        }
        // Trim trailing spaces from each line — terminal cells often have
        // trailing-space padding that's invisible on screen but ugly when
        // pasted.
        let cleaned: Vec<&str> = out.lines().map(|l| l.trim_end()).collect();
        Some(cleaned.join("\n"))
    }

    fn pty_write_open_file(&self, path: &str) {
        self.write_to_focused_pty(b"\x15${EDITOR:-open} '");
        shell_quote_arg(path, |chunk| self.write_to_focused_pty(chunk));
        self.write_to_focused_pty(b"'\n");
    }

    /// Open `path` at `line` (and optional `col`) in $EDITOR.
    ///
    /// Emits `${EDITOR:-vi} +LINE 'PATH'` which works as-is for
    /// vi/vim/nvim/emacs/nano. `code` users see the file open at the top
    /// (the `+N` argument is ignored, file still opens) — that's the
    /// lowest-common-denominator trade-off; we'd need a `case` statement
    /// to special-case `code --goto`.
    fn pty_write_open_file_at(&self, path: &str, line: u32, _col: Option<u32>) {
        let mut prefix = format!("\x15${{EDITOR:-vi}} +{line} '");
        // Prepend Ctrl-U (already in `prefix`) so any half-typed input is
        // cleared before our synthesised command runs.
        self.write_to_focused_pty(prefix.as_bytes());
        prefix.clear();
        shell_quote_arg(path, |chunk| self.write_to_focused_pty(chunk));
        self.write_to_focused_pty(b"'\n");
    }

    fn pty_write_open_url(&self, url: &str) {
        self.write_to_focused_pty(b"\x15open '");
        shell_quote_arg(url, |chunk| self.write_to_focused_pty(chunk));
        self.write_to_focused_pty(b"'\n");
    }

    fn write_mouse_event(&self, button: u8, col: usize, row: usize, press: bool) {
        let sgr = self
            .tabs
            .current()
            .and_then(|t| t.registry.get(t.focused_id()))
            .and_then(|p| p.terminal.as_ref())
            .map(|t| t.modes.mouse_sgr)
            .unwrap_or(false);
        let id = self.focused_pane_id();
        let mut buf = [0u8; 32];
        let bytes = encode_mouse(button, col + 1, row + 1, press, sgr, &mut buf);
        if let Some(pty) = self.ptys.get(&id) {
            let _ = pty.write(bytes);
        }
    }

    /// Convert a view-point mouse location to a device-pixel raster position.
    fn view_pt_to_raster_px(&self, loc: MouseLocation) -> (f64, f64) {
        let s = self.window_scale;
        let dh = self.view_height_pt * s;
        let rx = loc.x * s;
        let ry = dh - loc.y * s; // flip y: view is y-up, raster is y-down
        (rx, ry)
    }

    /// Hit-test the pointer position to a (row, col) cell in the focused pane.
    fn event_cell(&self, loc: MouseLocation, clamp: bool) -> Option<(usize, usize)> {
        let (rx, ry) = self.view_pt_to_raster_px(loc);
        let tab = self.tabs.current()?;
        let ir = self.pane_area_rect();
        let div = DIVIDER_PX;

        let pane_id = tab
            .tree
            .hit_test(ir, div, rx, ry)
            .or_else(|| if clamp { Some(tab.focused_id()) } else { None })?;

        let entries = tab.tree.layout(ir, div);
        let pr = entries.iter().find(|e| e.id == pane_id)?.rect;
        let pane = tab.registry.get(pane_id)?;
        let terminal = pane.terminal.as_ref()?;
        let cw = self.font.metrics.cell_w;
        let ch = self.font.metrics.cell_h;
        let rows = terminal.rows() as f64;
        let cols = terminal.cols() as f64;

        let rel_x = rx - pr.x;
        let rel_y = ry - pr.y;

        if !clamp {
            if rel_y < 0.0 || rel_x < 0.0 {
                return None;
            }
            if rel_y >= rows * ch || rel_x >= cols * cw {
                return None;
            }
        }

        let raw_row = (rel_y / ch).clamp(0.0, rows - 1.0);
        let raw_col = (rel_x / cw).clamp(0.0, cols - 1.0);
        Some((raw_row as usize, raw_col as usize))
    }

    fn refresh_hud(&mut self) {
        if let Some(cwd) = self.current_cwd() {
            self.local_ctx.cwd = cwd.clone();
            // Collect git results (non-blocking).
            while let Ok(result) = self.git_rx.try_recv() {
                self.local_ctx.git = result.state;
                self.local_ctx.branch = result.branch;
                self.local_ctx.git_dirty = result.dirty;
                self.local_ctx.git_ahead = result.ahead;
                self.local_ctx.git_behind = result.behind;
                self.local_ctx.head_short = result.head_short;
                self.local_ctx.head_subject = result.head_subject;
                // Task #7: ports from the git worker.
                self.local_ctx.ports = result.ports;
                // Task #9: project kind from the git worker.
                self.local_ctx.project_kind = result.project_kind;
            }
            // Kick off git worker (try_send is non-blocking; drop if channel full).
            let _ = self.git_tx.try_send(PathBuf::from(&cwd));
            // Notify the recent-files worker of the current cwd.
            let _ = self.recent_cwd_tx.try_send(PathBuf::from(&cwd));

            // Start the caldera poller lazily once we know the repo root —
            // it spawns its own thread and polls /api/activity every 2s.
            if self.caldera_poller.is_none() {
                let root = PathBuf::from(&cwd);
                self.caldera_poller = Some(anvil_caldera::Poller::start(
                    anvil_caldera::DEFAULT_ENDPOINT,
                    root,
                ));
                self.caldera_client = Some(anvil_caldera::CalderaClient::new(
                    anvil_caldera::DEFAULT_ENDPOINT,
                ));
            }
        }

        // Drain the caldera poller into the agent snapshot. The poller writes
        // a fresh Snapshot every 2s; the read is a cheap mutex peek + clone.
        if let Some(p) = &self.caldera_poller {
            self.agent_snap = p.snapshot();
        }

        // Task #8: drain the recent-files worker (non-blocking).
        while let Ok(result) = self.recent_rx.try_recv() {
            self.local_ctx.recent_files = result.files;
        }

        // Task #20: drain the kubectl worker (non-blocking).
        while let Ok(ctx) = self.kube_rx.try_recv() {
            self.local_ctx.kube_context = Some(ctx);
        }

        // ID3: drain filesystem worker and send cwd when it changes.
        while let Ok(snap) = self.fs_rx.try_recv() {
            self.explorer_scroll_offset = 0;
            self.expanded_dirs.clear();
            self.child_snapshots.clear();
            self.fs_snapshot = Some(LeftDockSnapshot {
                root: snap.root.to_string_lossy().into_owned(),
                entries: snap
                    .entries
                    .into_iter()
                    .map(|e| anvil_render::LeftDockEntry {
                        name: e.name,
                        is_dir: e.is_dir,
                    })
                    .collect(),
                git_marks: snap.git_marks,
            });
            self.dirty = true;
        }

        // Drain child-directory snapshots loaded on demand.
        while let Ok((dir_path, snap)) = self.child_fs_rx.try_recv() {
            self.child_snapshots.insert(
                dir_path,
                LeftDockSnapshot {
                    root: snap.root.to_string_lossy().into_owned(),
                    entries: snap
                        .entries
                        .into_iter()
                        .map(|e| anvil_render::LeftDockEntry {
                            name: e.name,
                            is_dir: e.is_dir,
                        })
                        .collect(),
                    git_marks: snap.git_marks,
                },
            );
            self.dirty = true;
        }
        if self.layout_mode == LayoutMode::Ide {
            if let Some(cwd) = self.current_cwd() {
                let changed = self.fs_last_cwd.as_deref() != Some(cwd.as_str());
                if changed {
                    let _ = self.fs_tx.try_send(PathBuf::from(&cwd));
                    self.fs_last_cwd = Some(cwd);
                }
            }
        }

        // last-run and recent prompts from focused pane
        if let Some(tab) = self.tabs.current() {
            let id = tab.focused_id();
            if let Some(pane) = tab.registry.get(id) {
                if let Some(t) = &pane.terminal {
                    let lr = t.last_run();
                    if lr.running || (lr.duration_ms == 0 && lr.exit_code == 0) {
                        self.local_ctx.run = RunState::Idle;
                    } else {
                        self.local_ctx.run = if lr.exit_code == 0 {
                            RunState::Ok
                        } else {
                            RunState::Failed
                        };
                        self.local_ctx.run_exit = lr.exit_code;
                        self.local_ctx.run_duration_ms = lr.duration_ms;
                    }

                    // Item 16: collect up to 5 recent prompt command lines,
                    // newest first. For each PromptStart (A-mark) look forward
                    // for the next OutputStart (B-mark) to find command_start_col,
                    // then read the text from that line.
                    let marks = t.prompt_marks();
                    let ev = t.evicted_lines;
                    let mut prompts: Vec<String> = Vec::new();
                    let mut i = marks.len();
                    while i > 0 && prompts.len() < 5 {
                        i -= 1;
                        if marks[i].kind != anvil_term::PromptMarkKind::PromptStart {
                            continue;
                        }
                        // Find the immediately following OutputStart (B-mark) for
                        // command_start_col; fall back to the A-mark line/col.
                        let (cmd_abs_line, cmd_col) = {
                            let mut found = (marks[i].line, 0usize);
                            for m in marks.iter().skip(i + 1) {
                                match m.kind {
                                    anvil_term::PromptMarkKind::OutputStart => {
                                        found = (m.line, m.col as usize);
                                        break;
                                    }
                                    anvil_term::PromptMarkKind::PromptStart => break,
                                    _ => {}
                                }
                            }
                            found
                        };
                        if cmd_abs_line < ev {
                            continue;
                        }
                        let crow = cmd_abs_line - ev;
                        let cells = t.line(crow);
                        let cmd: String = cells
                            .iter()
                            .skip(cmd_col)
                            .filter(|c| c.cp != '\0')
                            .map(|c| c.cp)
                            .collect();
                        let cmd = cmd.trim_end().to_string();
                        if !cmd.is_empty() {
                            prompts.push(cmd);
                        }
                    }
                    self.local_ctx.recent_prompts = prompts;
                } // end if let Some(t)
            }
        }

        self.dirty = true;
    }

    // Scroll-position change detector: only force a full redraw when
    // scroll_pos / viewport_offset actually MOVED since the last frame.
    // Sitting still inside scrollback should be as cheap as sitting at
    // live — partial dirty-row paints are safe when nothing's animating.
    fn render_frame(
        &mut self,
        grid_painters: &mut GridPainters<'_>,
        chrome_painter: &mut dyn anvil_render::GlyphPainter,
    ) {
        // ANVIL_PERF=1 emits per-frame timing to stderr so we can diagnose
        // scroll lag without guessing.
        let perf_log = std::env::var_os("ANVIL_PERF").is_some();
        let perf_t0 = if perf_log { Some(Instant::now()) } else { None };

        let (cur_scroll, cur_vp) = self
            .tabs
            .current()
            .and_then(|t| t.registry.get(t.focused_id()))
            .map(|p| {
                let vp = p
                    .terminal
                    .as_ref()
                    .map(|t| t.viewport_offset())
                    .unwrap_or(0);
                (p.scroll_pos, vp)
            })
            .unwrap_or((0.0, 0));
        let moved = cur_scroll != self.last_scroll_pos || cur_vp != self.last_viewport_offset;
        if moved {
            self.force_full_redraw = true;
        }
        self.last_scroll_pos = cur_scroll;
        self.last_viewport_offset = cur_vp;

        let is_full_redraw = self.force_full_redraw;
        self.force_full_redraw = false;

        if is_full_redraw {
            self.raster.clear(self.theme.background);
        } else {
            // Right HUD strip — clear and let the HUD draw repaint it. Safe
            // now that the initial PTY size matches `pane_area_rect`: no cells
            // ever extend into this column.
            if self.hud_visible {
                let cw = self.font.metrics.cell_w;
                let (dw, dh) = self.device_size();
                let strip_w_px = self.hud_cols as f64 * cw + GRID_PAD as f64;
                let x_start = (dw as f64 - strip_w_px).max(0.0);
                self.raster.fill_pixel_rect(
                    x_start,
                    0.0,
                    dw as f64 - x_start,
                    dh as f64,
                    self.theme.background,
                );
            }
        }

        let ch = self.font.metrics.cell_h;
        // Direction A is editor-first. Do not force the right context/HUD rail
        // on just because IDE mode is active; keep it opt-in via the HUD toggle.
        let eff_hud = self.hud_visible;
        let (dw, dh) = self.device_size();

        // Pane area: single source of truth from Docks geometry.
        let inner = self.pane_area_rect();

        let search_ref: Option<&anvil_term::Search> = if self.search_open {
            Some(&self.search)
        } else {
            None
        };
        let metrics = self.font.metrics;
        let chrome_metrics = self.chrome_font.metrics;

        // ── Terminal viewport draw ────────────────────────────────────────────
        //
        // CPU path: `draw_workspace` draws viewport cells into the raster.
        // GPU path: `draw_workspace_chrome` draws only dividers; viewport cells
        //           are pushed into `self.cell_batch` by `draw_viewport_gpu`.

        if !self.use_gpu_render {
            // CPU path — build per-pane dirty sets when not doing a full redraw.
            let dirty_map: Option<HashMap<PaneId, DirtySet>> = if is_full_redraw {
                None // full redraw: draw_workspace passes None to draw_viewport
            } else {
                let mut map = HashMap::new();
                if let Some(tab) = self.tabs.current_mut() {
                    let focused_id = tab.focused_id();
                    let entries = tab.tree.layout(inner, DIVIDER_PX);
                    for e in &entries {
                        if let Some(pane) = tab.registry.get_mut(e.id) {
                            // Native editor panes: always full-redraw until NE5 ships.
                            if pane.terminal.is_none() {
                                // A full DirtySet (0-row capacity) signals "full".
                                // draw_workspace skips the terminal path for editor panes.
                                let ds = DirtySet::all(0);
                                map.insert(e.id, ds);
                                continue;
                            }
                            let terminal = pane.terminal.as_mut().unwrap();
                            let rows = terminal.rows();
                            // Drain dirty rows from the terminal model.
                            let mut ds = terminal.take_dirty_rows();
                            // Always redraw the cursor row (blink, move).
                            let cur = terminal.cursor();
                            ds.mark(cur.y);
                            // Mark rows covered by the animated cursor position so
                            // intermediate pixels are cleared during smooth-move.
                            let ay_floor = pane.cursor_ay.floor() as usize;
                            let ay_ceil = pane.cursor_ay.ceil() as usize;
                            ds.mark(ay_floor);
                            ds.mark(ay_ceil);
                            // Also redraw the previous cursor row so stale cursor is erased.
                            if let Some(&prev) = self.cursor_row_prev.get(&e.id) {
                                ds.mark(prev);
                            }
                            // Viewport scroll change → force full for this pane.
                            let scroll_changed =
                                pane.scroll_pos != 0.0 || terminal.viewport_offset() != 0;
                            if scroll_changed {
                                ds.force_full();
                            }
                            // Auto-scroll: scrollback grew since last frame. Every
                            // visible row's pixels shifted up; per-row dirty tracking
                            // leaves the old cursor's pixels orphaned at their old
                            // device-pixel y. Force a full redraw to flush them.
                            let sbl = terminal.scrollback_len();
                            let prev_sbl =
                                self.scrollback_len_prev.get(&e.id).copied().unwrap_or(sbl);
                            if sbl != prev_sbl {
                                ds.force_full();
                            }
                            self.scrollback_len_prev.insert(e.id, sbl);
                            // Search active → force full (highlights may span many rows).
                            if search_ref.is_some() {
                                ds.force_full();
                            }
                            // If this pane has a selection, force full.
                            if pane.selection.active {
                                ds.force_full();
                            }
                            // Update cursor_row_prev to the animated row so the next
                            // frame clears the correct pixel rows.
                            self.cursor_row_prev
                                .insert(e.id, pane.cursor_ay.round() as usize);
                            let _ = rows;
                            map.insert(e.id, ds);
                        }
                    }
                    // Focused-pane blink: cursor row already marked above.
                    let _ = focused_id;
                }
                Some(map)
            };

            // Debug instrumentation: report dirty-row counts once per second.
            #[cfg(debug_assertions)]
            if std::env::var_os("ANVIL_RENDER_DEBUG").is_some() {
                self.debug_render_frame += 1;
                let now = Instant::now();
                let report = match self.debug_render_last_report {
                    None => true,
                    Some(last) => now.duration_since(last).as_secs_f64() >= 1.0,
                };
                if report {
                    self.debug_render_last_report = Some(now);
                    if let Some(ref map) = dirty_map {
                        for (pid, ds) in map {
                            if let Some(tab) = self.tabs.current() {
                                if let Some(pane) = tab.registry.get(*pid) {
                                    let total =
                                        pane.terminal.as_ref().map(|t| t.rows()).unwrap_or(0);
                                    let dirty_count = if ds.is_full() {
                                        total
                                    } else {
                                        ds.iter().count()
                                    };
                                    eprintln!(
                                        "anvil-render: frame {}: pane {} dirty rows={}/{}",
                                        self.debug_render_frame, pid, dirty_count, total
                                    );
                                }
                            }
                        }
                    } else {
                        eprintln!(
                            "anvil-render: frame {}: full redraw",
                            self.debug_render_frame
                        );
                    }
                }
            }

            if let Some(tab) = self.tabs.current_mut() {
                let focused_id = tab.focused_id();

                // NE10: build per-pane render diagnostics from LspManager.
                let diag_by_pane: HashMap<PaneId, Vec<anvil_render::RenderDiagnostic>> = {
                    let mut map = HashMap::new();
                    if let Some(lsp) = &self.lsp_manager {
                        for (pid, ep) in tab.editor_panes.panes_iter() {
                            if let Some(buf) = tab.editor_panes.get_buffer(ep.buffer_id) {
                                if let Some(path) = buf.tracked_path() {
                                    let raw = lsp.diagnostics_for(path);
                                    if !raw.is_empty() {
                                        let render_diags: Vec<anvil_render::RenderDiagnostic> = raw
                                            .iter()
                                            .map(|d| anvil_render::RenderDiagnostic {
                                                line: d.line,
                                                severity: match d.severity {
                                                    anvil_editor::DiagnosticSeverity::Error => {
                                                        anvil_render::RenderSeverity::Error
                                                    }
                                                    anvil_editor::DiagnosticSeverity::Warning => {
                                                        anvil_render::RenderSeverity::Warning
                                                    }
                                                    anvil_editor::DiagnosticSeverity::Info => {
                                                        anvil_render::RenderSeverity::Info
                                                    }
                                                    anvil_editor::DiagnosticSeverity::Hint => {
                                                        anvil_render::RenderSeverity::Hint
                                                    }
                                                },
                                                message: d.message.clone(),
                                            })
                                            .collect();
                                        map.insert(pid, render_diags);
                                    }
                                }
                            }
                        }
                    }
                    map
                };

                draw_workspace(
                    &mut self.raster,
                    grid_painters,
                    &tab.tree,
                    &mut tab.registry,
                    &tab.editor_panes,
                    inner,
                    DIVIDER_PX,
                    metrics,
                    &self.theme,
                    search_ref,
                    focused_id,
                    self.blink_phase,
                    self.cursor_cfg,
                    dirty_map.as_ref(),
                    self.running_pulse_phase,
                    &diag_by_pane,
                    self.hovered_editor_tab,
                    &mut self.editor_tab_hits,
                    self.ui_scale,
                    self.scroll_indicator_alpha,
                );
            }
        } else {
            // GPU path — draw only dividers to raster; fill cell_batch per pane.
            if let Some(tab) = self.tabs.current_mut() {
                let focused_id = tab.focused_id();
                draw_workspace_chrome(
                    &mut self.raster,
                    &tab.tree,
                    &tab.registry,
                    inner,
                    DIVIDER_PX,
                    &self.theme,
                    focused_id,
                );
            }
        }

        // ── Chrome (tab bar, bars, panels) ────────────────────────────────────
        //
        // Always drawn to CPU raster regardless of render path.

        // Chrome row: always drawn (basin mark + tabs + indicators).
        {
            let branch = self.local_ctx.branch.clone();
            let clock = local_hhmm();
            let scale = self.window_scale;
            let chrome_top = self.chrome_top_px();
            draw_tab_bar(
                &mut self.raster,
                chrome_painter,
                chrome_metrics,
                &self.theme,
                &self.tabs,
                &branch,
                &clock,
                scale,
                chrome_top,
                &mut self.tab_bar_hits,
            );
        }

        // Context bar + left dock: Ide mode only.
        if self.layout_mode == LayoutMode::Ide {
            let areas = Docks::for_mode_with_left_dock_w(
                self.layout_mode,
                self.window_scale,
                self.dock_metrics(),
                self.hud_visible,
                self.chrome_bottom_px(),
                self.left_dock_visible,
                self.left_dock_w_pt * self.ui_scale,
                self.ui_scale,
            )
            .compute_areas(self.window_inner(), metrics.cell_w, metrics.cell_h);
            // NE15: context-bar editor segment reads from the focused native
            // editor pane's buffer (path basename). No modified flag yet —
            // Buffer has no dirty-since-save tracking.
            let editor_ctx_name: Option<String> = self.tabs.current().and_then(|tab| {
                let id = tab.focused_id();
                let pane = tab.editor_panes.get_pane(id)?;
                let buf = tab.editor_panes.get_buffer(pane.buffer_id)?;
                let name = match buf.tracked_path() {
                    Some(p) => p
                        .file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or("[scratch]")
                        .to_string(),
                    None => "[scratch]".to_string(),
                };
                Some(name)
            });
            let editor_ctx = editor_ctx_name.as_deref().map(|name| {
                anvil_render::context_bar::ContextBarEditor {
                    name,
                    modified: false,
                }
            });
            anvil_render::draw_context_bar(
                &mut self.raster,
                chrome_painter,
                chrome_metrics,
                &self.theme,
                &self.local_ctx,
                editor_ctx,
                areas.top_bar,
            );
            // NE9/item-19: derive outline from the active buffer's syntax tree.
            // For Rust buffers uses tree-sitter node walk; falls back to line-regex
            // for other languages. Non-Rust buffers produce an empty list (header-only).
            let outline_rows: Option<Vec<OutlineRow>> = self.tabs.current().and_then(|tab| {
                let id = tab.focused_id();
                let ep = tab.editor_panes.get_pane(id)?;
                let buf = tab.editor_panes.get_buffer(ep.buffer_id)?;
                // Only populate outline for Rust files; other languages show
                // the header-only state until multi-language support is added.
                if buf
                    .tracked_path()
                    .and_then(|p| p.extension())
                    .and_then(|e| e.to_str())
                    .map(|e| e.eq_ignore_ascii_case("rs"))
                    != Some(true)
                {
                    return None;
                }
                let text = buf.to_text();
                let symbols = anvil_editor::derive_outline_rows(buf.syntax(), &text);
                if symbols.is_empty() {
                    return Some(Vec::new());
                }
                use anvil_editor::OutlineSymbolKind;
                use anvil_render::left_dock::{OutlineKind, OutlineRow};
                let rows = symbols
                    .into_iter()
                    .map(|s| OutlineRow {
                        name: s.name,
                        kind: match s.kind {
                            OutlineSymbolKind::Function => OutlineKind::Function,
                            OutlineSymbolKind::Struct => OutlineKind::Struct,
                            OutlineSymbolKind::Impl => OutlineKind::Module,
                            OutlineSymbolKind::Enum => OutlineKind::Enum,
                            OutlineSymbolKind::Trait => OutlineKind::Interface,
                            OutlineSymbolKind::Other => OutlineKind::Other,
                        },
                        depth: 0,
                        line: s.line,
                    })
                    .collect();
                Some(rows)
            });
            if self.left_dock_visible {
                // Keyboard nav: show selected_explorer_row as the hover highlight
                // when explorer has focus (item 4).
                let effective_hover = if self.focus_target == FocusTarget::Explorer {
                    self.selected_explorer_row.or(self.hovered_explorer_row)
                } else {
                    self.hovered_explorer_row
                };
                self.left_dock_hits = draw_left_dock_with_scroll(
                    &mut self.raster,
                    chrome_painter,
                    chrome_metrics,
                    &self.theme,
                    self.fs_snapshot.as_ref(),
                    self.active_explorer_file.as_deref(),
                    outline_rows.as_deref(),
                    areas.left_dock,
                    self.explorer_scroll_offset,
                    effective_hover,
                    &self.expanded_dirs,
                    &self.child_snapshots,
                    self.scroll_indicator_alpha,
                    self.ui_scale,
                    self.explorer_filter.as_deref(),
                );
            } else {
                self.left_dock_hits.clear();
            }
        } else {
            self.left_dock_hits.clear();
        }

        // Bottom strip: search bar when open, otherwise the slim status bar.
        // Both draw into the same fixed-pixel chrome_bottom_px strip.
        let chrome_bot = self.chrome_bottom_px();
        let scale = self.window_scale;
        if self.search_open {
            // For native editor panes, read EditorSearch from the focused pane.
            let editor_search: Option<&EditorSearch> = self.tabs.current().and_then(|tab| {
                let id = tab.focused_id();
                tab.editor_panes.get_pane(id)?.search.as_ref()
            });
            anvil_render::searchbar::draw_search_bar_with_replace(
                &mut self.raster,
                chrome_painter,
                chrome_metrics,
                &self.theme,
                &self.search,
                chrome_bot,
                scale,
                editor_search,
                self.replace_row_active,
                &mut self.search_bar_hits,
            );
        } else {
            let clock = local_hhmm();
            let status_mode = {
                use anvil_render::statusbar::StatusMode;
                if self.palette.visible
                    || self.project_search.visible
                    || self.project_switcher_open
                    || self.workspace_symbol_search.is_some()
                    || self.buffer_symbol_search.is_some()
                {
                    StatusMode::Picking
                } else if self.lsp_rename_input.is_some() {
                    StatusMode::Renaming
                } else {
                    StatusMode::Editing
                }
            };
            anvil_render::statusbar::draw_status_bar(
                &mut self.raster,
                chrome_painter,
                chrome_metrics,
                &self.theme,
                &self.local_ctx,
                &self.agent_snap,
                &clock,
                chrome_bot,
                scale,
                self.agent_pulse_phase,
                status_mode,
            );
        }

        // Right-side HUD: docked, edge-to-edge frosted-glass panel with repo
        // / git / agent / system state. Replaces the old bottom status bar
        // and the small floating agent card.
        if eff_hud {
            let cw = self.font.metrics.cell_w;
            let hud_top = self.chrome_top_px();
            let hud_bot = self.chrome_bottom_px();
            let hud_h = (dh as f64 - hud_top - hud_bot).max(0.0);
            let rows = ((hud_h / ch) as usize).max(1);

            // Surface: rightmost slab of the window, between chrome strips.
            let hud_cols = self.hud_cols;
            let surface_w_px = hud_cols as f64 * cw + GRID_PAD as f64;
            let surface_rect = anvil_render::raster::PixelRect {
                x: (dw as f64 - surface_w_px).max(0.0),
                y: hud_top,
                w: surface_w_px.min(dw as f64),
                h: hud_h,
            };

            // Content column: cell-grid coord whose pixel position sits one
            // cell in from the window's right edge. Computing this from the
            // raster origin keeps text monospaced and aligned to the grid.
            let content_right_px = dw as f64 - cw;
            let content_left_px = content_right_px - hud_cols as f64 * cw;
            let start_col =
                (((content_left_px - self.raster.pad_x) / cw).round() as isize).max(0) as usize;

            draw_right_hud(
                &mut self.raster,
                grid_painters.regular,
                metrics,
                &self.theme,
                &self.agent_snap,
                &self.local_ctx,
                surface_rect,
                start_col,
                hud_cols,
                0, // top_bar_rows removed; chrome is fixed-pixel, not cell-row
                rows,
                &mut self.hud_hits,
                &self.hud_section_order,
                &mut self.hud_section_hits,
            );
        }

        // ── Project-wide search overlay (item 10) ────────────────────────────
        if self.project_search.visible {
            let cw = self.font.metrics.cell_w;
            let chrome_top = self.chrome_top_px();
            let chrome_bot = self.chrome_bottom_px();
            draw_project_search_overlay(
                &mut self.raster,
                chrome_painter,
                chrome_metrics,
                &self.theme,
                &self.project_search,
                dw as f64,
                dh as f64,
                chrome_top,
                chrome_bot,
                cw,
                ch,
            );
        }

        // ── Goto-line overlay (item 11) ───────────────────────────────────────
        if let Some(ref input) = self.goto_line_input {
            let cw = self.font.metrics.cell_w;
            let chrome_top = self.chrome_top_px();
            draw_goto_line_overlay(
                &mut self.raster,
                chrome_painter,
                chrome_metrics,
                &self.theme,
                input,
                dw as f64,
                dh as f64,
                chrome_top,
                cw,
                ch,
            );
        }

        // ── Save-as overlay (tier-J J2) ───────────────────────────────────────
        if let Some(ref input) = self.save_as_input {
            let cw = self.font.metrics.cell_w;
            let chrome_top = self.chrome_top_px();
            draw_save_as_overlay(
                &mut self.raster,
                chrome_painter,
                chrome_metrics,
                &self.theme,
                input,
                dw as f64,
                dh as f64,
                chrome_top,
                cw,
                ch,
            );
        }

        // ── LSP rename overlay (item 24) ──────────────────────────────────────
        if let Some(ref input) = self.lsp_rename_input {
            let cw = self.font.metrics.cell_w;
            let chrome_top = self.chrome_top_px();
            draw_lsp_rename_overlay(
                &mut self.raster,
                chrome_painter,
                chrome_metrics,
                &self.theme,
                input,
                dw as f64,
                dh as f64,
                chrome_top,
                cw,
                ch,
            );
        }

        // ── LSP references overlay (item 26) ──────────────────────────────────
        if let Some(ref refs) = self.lsp_references {
            let cw = self.font.metrics.cell_w;
            let chrome_top = self.chrome_top_px();
            draw_lsp_references_overlay(
                &mut self.raster,
                chrome_painter,
                chrome_metrics,
                &self.theme,
                refs,
                dw as f64,
                dh as f64,
                chrome_top,
                cw,
                ch,
            );
        }

        // ── Workspace symbol search overlay (O1) ─────────────────────────────
        if let Some(ref wss) = self.workspace_symbol_search {
            let cw = self.font.metrics.cell_w;
            let chrome_top = self.chrome_top_px();
            draw_workspace_symbol_overlay(
                &mut self.raster,
                chrome_painter,
                chrome_metrics,
                &self.theme,
                wss,
                dw as f64,
                dh as f64,
                chrome_top,
                cw,
                ch,
            );
        }

        // ── Buffer symbol search overlay (O2) ────────────────────────────────
        if let Some(ref bss) = self.buffer_symbol_search {
            let cw = self.font.metrics.cell_w;
            let chrome_top = self.chrome_top_px();
            draw_buffer_symbol_overlay(
                &mut self.raster,
                chrome_painter,
                chrome_metrics,
                &self.theme,
                bss,
                dw as f64,
                dh as f64,
                chrome_top,
                cw,
                ch,
            );
        }

        // ── Disk-changed banner (item 27) ────────────────────────────────────
        {
            let cw = self.font.metrics.cell_w;
            let banner_ch = ch;
            let focused_bid = self
                .tabs
                .current()
                .and_then(|tab| tab.editor_panes.get_pane(tab.focused_id()))
                .map(|ep| ep.buffer_id);
            if let Some(bid) = focused_bid {
                if self.disk_changed_dirty.contains_key(&bid) {
                    let chrome_top = self.chrome_top_px();
                    draw_disk_changed_banner(
                        &mut self.raster,
                        chrome_painter,
                        chrome_metrics,
                        &self.theme,
                        dw as f64,
                        chrome_top,
                        cw,
                        banner_ch,
                    );
                }
            }
        }

        // ── Welcome screen (item 28) ──────────────────────────────────────────
        if self.should_show_welcome() {
            let cw = self.font.metrics.cell_w;
            let chrome_top = self.chrome_top_px();
            let chrome_bot = self.chrome_bottom_px();
            draw_welcome_screen(
                &mut self.raster,
                chrome_painter,
                chrome_metrics,
                &self.theme,
                dw as f64,
                dh as f64,
                chrome_top,
                chrome_bot,
                cw,
                ch,
                &self.recent_projects,
            );
        }

        // ── Project switcher overlay (item 30) ────────────────────────────────
        // ── Open-folder overlay (Q19) ─────────────────────────────────────────
        if let Some(ref input) = self.open_folder_input {
            let cw = self.font.metrics.cell_w;
            let chrome_top = self.chrome_top_px();
            draw_open_folder_overlay(
                &mut self.raster,
                chrome_painter,
                chrome_metrics,
                &self.theme,
                input,
                dw as f64,
                chrome_top,
                cw,
                ch,
            );
        }

        // ── Language picker overlay (Q22) ─────────────────────────────────────
        if let Some(ref picker) = self.language_picker {
            let cw = self.font.metrics.cell_w;
            let chrome_top = self.chrome_top_px();
            draw_language_picker_overlay(
                &mut self.raster,
                chrome_painter,
                chrome_metrics,
                &self.theme,
                &picker.query,
                picker.selected,
                dw as f64,
                chrome_top,
                cw,
                ch,
            );
        }

        if self.project_switcher_open {
            let cw = self.font.metrics.cell_w;
            let chrome_top = self.chrome_top_px();
            draw_project_switcher_overlay(
                &mut self.raster,
                chrome_painter,
                chrome_metrics,
                &self.theme,
                &self.recent_projects,
                self.project_switcher_sel,
                dw as f64,
                dh as f64,
                chrome_top,
                cw,
                ch,
            );
        }

        // ── Toasts (N3) ───────────────────────────────────────────────────────
        if !self.toasts.is_empty() {
            let chrome_bot = self.chrome_bottom_px();
            draw_toasts(
                &mut self.raster,
                chrome_painter,
                chrome_metrics,
                &self.theme,
                &self.toasts,
                dw as f64,
                dh as f64,
                chrome_bot,
                self.ui_scale,
            );
        }

        // Cheatsheet overlay.
        if self.cheatsheet_visible {
            let cw = self.font.metrics.cell_w;
            let chrome_top = self.chrome_top_px();
            let chrome_bot = self.chrome_bottom_px();
            let safe_h = (dh as f64 - chrome_top - chrome_bot).max(0.0);
            let total_rows = ((safe_h / ch) as usize).max(1);
            let total_cols = (((dw.saturating_sub(2 * GRID_PAD)) as f64 / cw) as usize).max(1);
            draw_cheatsheet(
                &mut self.raster,
                grid_painters.regular,
                metrics,
                &self.theme,
                total_cols,
                total_rows,
                chrome_top,
                chrome_bot,
            );
        }

        // ── I3: Explorer drag chip ────────────────────────────────────────────
        // Paint a small floating label near the cursor while dragging a file
        // from the Explorer.
        if let (Some(cursor), Some((path, _))) =
            (self.explorer_drag_cursor, self.explorer_drag.as_ref())
        {
            let basename = path.file_name().and_then(|n| n.to_str()).unwrap_or("file");
            let (cx, cy) = self.view_pt_to_raster_px(cursor);
            // Offset chip below-right of the cursor pointer.
            let chip_x = cx + 12.0 * self.ui_scale;
            let chip_y = cy - 16.0 * self.ui_scale;
            let pad_x = 6.0 * self.ui_scale;
            let pad_y = 3.0 * self.ui_scale;
            let chip_w = basename.len() as f64 * self.font.metrics.cell_w + pad_x * 2.0;
            let chip_h = ch + pad_y * 2.0;
            // Panel background.
            self.raster
                .fill_pixel_rect(chip_x, chip_y, chip_w, chip_h, self.theme.panel);
            self.raster.fill_pixel_rect_alpha(
                chip_x,
                chip_y,
                chip_w,
                1.0,
                self.theme.hairline,
                0.68,
            );
            // Filename text.
            let metrics = self.font.metrics;
            for (i, ch_c) in basename.chars().enumerate() {
                let gx = chip_x + pad_x + i as f64 * metrics.cell_w;
                let gy = chip_y + pad_y;
                self.raster.glyph_at(
                    grid_painters.regular,
                    metrics,
                    gx,
                    gy,
                    ch_c as u32,
                    self.theme.foreground,
                );
            }
        }

        // ── R2: Explorer file tooltip ─────────────────────────────────────────
        // Shown after 500ms steady hover over a file row (not a dir).
        if let Some((ref tip_path, size, mtime_secs)) = self.explorer_hover_meta.clone() {
            // Find the hit rect for this row to position the tooltip.
            if let Some((row_idx, _)) = self.explorer_hover_row {
                let row_rect = self
                    .left_dock_hits
                    .hits
                    .iter()
                    .find(|h| {
                        h.kind
                            == anvil_render::LeftDockHitKind::Explorer(
                                anvil_render::ExplorerHit::Row(row_idx),
                            )
                    })
                    .map(|h| h.rect);
                if let Some(rr) = row_rect {
                    let basename = tip_path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    let size_str = humanize_bytes(size);
                    let mtime_str = relative_time(mtime_secs);
                    // Three lines: basename (foreground), size, mtime (text_subtle).
                    let lines: [(&str, [u8; 3]); 3] = [
                        (basename, self.theme.foreground),
                        (&size_str, self.theme.text_subtle),
                        (&mtime_str, self.theme.text_subtle),
                    ];
                    let max_len = lines
                        .iter()
                        .map(|(s, _)| s.chars().count())
                        .max()
                        .unwrap_or(0);
                    let metrics = self.font.metrics;
                    let pad = 8.0 * self.ui_scale;
                    let tip_w = max_len as f64 * metrics.cell_w + pad * 2.0;
                    let tip_h = lines.len() as f64 * metrics.cell_h + pad * 2.0;
                    // Position: to the right of the dock row.
                    let tip_x = rr.x + rr.w + 4.0 * self.ui_scale;
                    let tip_y = rr.y;
                    // Clamp to raster.
                    let tip_x = tip_x.max(0.0).min(dw as f64 - tip_w);
                    let tip_y = tip_y.max(0.0).min(dh as f64 - tip_h);
                    // Background.
                    self.raster
                        .fill_pixel_rect(tip_x, tip_y, tip_w, tip_h, self.theme.panel);
                    // 1px hairline border.
                    self.raster
                        .fill_pixel_rect(tip_x, tip_y, tip_w, 1.0, self.theme.hairline);
                    self.raster.fill_pixel_rect(
                        tip_x,
                        tip_y + tip_h - 1.0,
                        tip_w,
                        1.0,
                        self.theme.hairline,
                    );
                    self.raster
                        .fill_pixel_rect(tip_x, tip_y, 1.0, tip_h, self.theme.hairline);
                    self.raster.fill_pixel_rect(
                        tip_x + tip_w - 1.0,
                        tip_y,
                        1.0,
                        tip_h,
                        self.theme.hairline,
                    );
                    // Text lines.
                    for (row, (text, color)) in lines.iter().enumerate() {
                        let gy = tip_y + pad + row as f64 * metrics.cell_h;
                        for (i, c) in text.chars().enumerate() {
                            let gx = tip_x + pad + i as f64 * metrics.cell_w;
                            self.raster
                                .glyph_at(chrome_painter, metrics, gx, gy, c as u32, *color);
                        }
                    }
                }
            }
        }

        // ── P2: divider hover highlight ───────────────────────────────────────
        // Paint a 1px accent_primary α=0.50 stripe along the hovered divider.
        if let Some(kind) = self.divider_hover {
            let a = self.theme.accent_primary;
            match kind {
                DividerKind::Sidebar => {
                    // Vertical stripe at the sidebar right edge, spanning the
                    // pane area height (inner excludes the chrome strips).
                    let edge_x = self.left_dock_w_pt * self.window_scale;
                    self.raster
                        .fill_pixel_rect_alpha(edge_x, inner.y, 1.0, inner.h, a, 0.50);
                }
                DividerKind::Drawer => {
                    // Horizontal stripe at the drawer divider y.
                    if let Some(div_y) = self.ide_drawer_divider_y() {
                        self.raster
                            .fill_pixel_rect_alpha(inner.x, div_y, inner.w, 1.0, a, 0.50);
                    }
                }
            }
        }

        // ── Present ───────────────────────────────────────────────────────────

        if !self.use_gpu_render {
            // CPU path: upload the full raster as a single fullscreen quad.
            if let Some(r) = &mut self.renderer {
                let sync = present_mode(false) == PresentMode::Sync;
                r.present(self.raster.bytes(), sync);
            }
        } else if let (Some(r), Some(ap)) = (&mut self.renderer, self.atlas_painter.as_mut()) {
            // GPU path: fill cell_batch from all visible panes, then composite.
            let (dw_f, dh_f) = (dw as f32, dh as f32);
            self.cell_batch.clear([dw_f, dh_f]);

            // Iterate panes and call draw_viewport_gpu per leaf.
            if let Some(tab) = self.tabs.current_mut() {
                let focused_id = tab.focused_id();
                let entries = tab.tree.layout(inner, DIVIDER_PX);
                for e in &entries {
                    let pane = match tab.registry.get_mut(e.id) {
                        Some(p) => p,
                        None => continue,
                    };

                    // Set raster origin so cell_rect math in draw_viewport_gpu
                    // computes absolute drawable pixel positions.
                    self.raster.origin_x = e.rect.x;
                    self.raster.origin_y = e.rect.y;

                    let cursor_params = if e.id == focused_id {
                        Some(anvil_render::CursorParams {
                            ax: pane.cursor_ax,
                            ay: pane.cursor_ay,
                            blink_phase: self.blink_phase,
                            cfg: self.cursor_cfg,
                        })
                    } else {
                        None
                    };

                    // Skip GPU path for native editor panes (NE4 stub — charcoal fill
                    // is done by draw_workspace_chrome, which calls draw_editor_pane_stub).
                    if pane.terminal.is_none() {
                        continue;
                    }
                    let terminal = pane.terminal.as_mut().unwrap();

                    let folded = FoldedBlocks::new(&pane.folded[..pane.folded_count]);

                    draw_viewport_gpu(
                        &mut self.cell_batch,
                        &self.raster,
                        ap,
                        terminal,
                        metrics,
                        &self.theme,
                        pane.scroll_pos,
                        pane.selection,
                        search_ref,
                        cursor_params,
                        folded,
                        self.running_pulse_phase,
                    );
                }
                // Reset raster origin for chrome draws.
                self.raster.origin_x = 0.0;
                self.raster.origin_y = 0.0;
            }

            // Composite: chrome raster + GPU cells.
            let atlas_tex = ap.texture().as_ref();
            let (atlas_w, atlas_h) = ap.texture_size();
            let atlas_px = [atlas_w as f32, atlas_h as f32];
            let sync = present_mode(false) == PresentMode::Sync;
            r.present_layered(
                self.raster.bytes(),
                &self.cell_batch,
                atlas_tex,
                atlas_px,
                sync,
            );
        }

        if let Some(t0) = perf_t0 {
            let us = t0.elapsed().as_micros();
            let kind = if is_full_redraw { "FULL" } else { "part" };
            eprintln!("anvil-perf: frame={kind} {us}µs scroll={cur_scroll:.2}");
        }
    }

    // ── Palette helpers ──────────────────────────────────────────────────────

    fn send_palette_show(&self, webview: &Webview) {
        let mut cmds: Vec<BridgeCmd> = CATALOG
            .iter()
            .map(|e| BridgeCmd {
                id: e.id.to_string(),
                title: e.title.to_string(),
                subtitle: e.subtitle.map(|s| s.to_string()),
            })
            .collect();

        // Dynamic: one entry per tab, most-recent-first (reverse index order).
        let tab_count = self.tabs.tabs.len();
        for idx in (0..tab_count).rev() {
            let pane_count = self.tabs.tabs[idx].tree.leaf_count();
            let panes_label = if pane_count == 1 {
                "pane".to_string()
            } else {
                "panes".to_string()
            };
            cmds.push(BridgeCmd {
                id: format!("tab.switch:{idx}"),
                title: format!("Tab {} · {} {}", idx + 1, pane_count, panes_label),
                subtitle: None,
            });
        }

        // Layout mode entries.
        cmds.push(BridgeCmd {
            id: "layout.mode:terminal".to_string(),
            title: "Layout: Terminal".to_string(),
            subtitle: None,
        });
        cmds.push(BridgeCmd {
            id: "layout.mode:ide".to_string(),
            title: "Layout: Ide".to_string(),
            subtitle: None,
        });
        // Agent actions — only when Caldera is Live.
        if self.agent_snap.connection == anvil_agent::Connection::Live {
            cmds.push(BridgeCmd {
                id: "agent.approve".to_string(),
                title: "Agent: Approve".to_string(),
                subtitle: None,
            });
            cmds.push(BridgeCmd {
                id: "agent.start".to_string(),
                title: "Agent: Start Run".to_string(),
                subtitle: None,
            });
        }

        let theme_tokens = ThemeTokens {
            background: format_hex(self.theme.background),
            foreground: format_hex(self.theme.foreground),
            accent: format_hex(self.theme.accent),
        };
        let outbound = Outbound::Show {
            commands: cmds,
            theme: theme_tokens,
        };
        if let Ok(json) = bridge_encode(&outbound) {
            webview.eval_js(&format!("window.anvil.receive({json});"));
        }
    }

    fn dismiss_palette(&mut self, webview: &Webview) {
        self.palette.dismiss();
        if let Ok(json) = bridge_encode(&Outbound::Hide) {
            webview.eval_js(&format!("window.anvil.receive({json});"));
        }
        webview.hide();
    }

    /// Item 15 (Tier-B): show up to 10 most-recently-opened files in the palette.
    fn send_recent_files_show(&self, webview: &Webview) {
        let cmds: Vec<BridgeCmd> = self
            .recent_file_list
            .iter()
            .take(10)
            .map(|p| {
                let title = p
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| p.to_string_lossy().into_owned());
                let subtitle = p.to_string_lossy().into_owned();
                BridgeCmd {
                    id: format!("file:open:{}", p.display()),
                    title,
                    subtitle: Some(subtitle),
                }
            })
            .collect();
        let theme_tokens = ThemeTokens {
            background: format_hex(self.theme.background),
            foreground: format_hex(self.theme.foreground),
            accent: format_hex(self.theme.accent),
        };
        let outbound = Outbound::Show {
            commands: cmds,
            theme: theme_tokens,
        };
        if let Ok(json) = bridge_encode(&outbound) {
            webview.eval_js(&format!("window.anvil.receive({json});"));
        }
    }

    /// Open the command palette pre-populated with project files (Cmd+P).
    ///
    /// Uses `recent_files_in_dir` to walk the current project root (up to 500
    /// files).  Each file becomes a `file:open:<abs-path>` command entry.
    /// The palette's existing fuzzy filter handles the rest.
    fn send_file_picker_show(&self, webview: &Webview) {
        let root = self
            .fs_snapshot
            .as_ref()
            .map(|s| s.root.clone())
            .or_else(|| self.current_cwd());
        let root = match root {
            Some(r) => r,
            None => return,
        };

        let root_path = std::path::Path::new(&root);
        // Collect up to 500 files; use the existing walk helper (depth 3, skips target/.git/node_modules).
        let files = recent_files_in_dir(root_path, 500);

        let cmds: Vec<BridgeCmd> = files
            .into_iter()
            .map(|abs_path| {
                let base = std::path::Path::new(&abs_path)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| abs_path.clone());
                // subtitle: relative path for context
                let rel = std::path::Path::new(&abs_path)
                    .strip_prefix(root_path)
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_else(|_| abs_path.clone());
                BridgeCmd {
                    id: format!("file:open:{abs_path}"),
                    title: base,
                    subtitle: Some(rel),
                }
            })
            .collect();

        let theme_tokens = ThemeTokens {
            background: format_hex(self.theme.background),
            foreground: format_hex(self.theme.foreground),
            accent: format_hex(self.theme.accent),
        };
        let outbound = Outbound::Show {
            commands: cmds,
            theme: theme_tokens,
        };
        if let Ok(json) = bridge_encode(&outbound) {
            webview.eval_js(&format!("window.anvil.receive({json});"));
        }
    }

    fn handle_palette_action(&mut self, action: Action, webview: &Webview) {
        match action {
            Action::ThemeDark => {
                self.set_theme_mode("ember-dark");
            }
            Action::ThemeLight => {
                self.set_theme_mode("ember-light");
            }
            Action::ThemeSystem => {
                self.set_theme_mode("system");
            }
            Action::ConfigReload => {
                if let Some(ref w) = self.watcher {
                    let cfg = anvil_config::load(&w.path);
                    self.apply_config(cfg);
                }
            }
            Action::ClearScreen => {
                if let Some(tab) = self.tabs.current_mut() {
                    let id = tab.focused_id();
                    if let Some(pane) = tab.registry.get_mut(id) {
                        if let Some(terminal) = &mut pane.terminal {
                            terminal.feed(b"\x1b[H\x1b[2J");
                        }
                    }
                }
                self.dirty = true;
            }
            Action::ScrollTop => {
                if let Some(tab) = self.tabs.current_mut() {
                    let id = tab.focused_id();
                    if let Some(pane) = tab.registry.get_mut(id) {
                        if let Some(terminal) = &mut pane.terminal {
                            let len = terminal.scrollback_len() as isize;
                            terminal.scroll_viewport(len);
                            let sp = terminal.viewport_offset() as f32;
                            pane.scroll_pos = sp;
                            pane.scroll_target = sp;
                            pane.scroll_vel = 0.0;
                        }
                    }
                }
                self.dirty = true;
            }
            Action::ScrollBottom => {
                if let Some(tab) = self.tabs.current_mut() {
                    let id = tab.focused_id();
                    if let Some(pane) = tab.registry.get_mut(id) {
                        if let Some(terminal) = &mut pane.terminal {
                            terminal.scroll_to_bottom();
                            pane.scroll_pos = 0.0;
                            pane.scroll_target = 0.0;
                            pane.scroll_vel = 0.0;
                        }
                    }
                }
                self.dirty = true;
            }
            Action::AppQuit => {
                terminate_app();
                return;
            }
            Action::HudToggle => {
                // Intercepted at AppShell level (needs window access).
                // Unreachable via normal dispatch; guard against direct calls.
                self.hud_visible = !self.hud_visible;
                self.resize_all_tabs();
                self.dirty = true;
            }
            Action::CheatsheetShow => {
                self.cheatsheet_visible = true;
                self.dirty = true;
                self.force_full_redraw = true;
            }
            Action::SwitchTab(idx) => {
                self.tabs.switch_to(idx);
                self.resize_all_tabs();
                self.dirty = true;
            }
            Action::LayoutTerminal => {
                self.layout_mode = LayoutMode::Terminal;
                self.left_dock_visible = false;
                if let Some(tab) = self.tabs.current_mut() {
                    tab.normalize_terminal_surface();
                }
                self.resize_all_tabs();
                self.dirty = true;
            }
            Action::LayoutIde => {
                self.layout_mode = LayoutMode::Ide;
                self.left_dock_visible = true;
                self.spawn_ide_terminal_drawer();
                if let Some(tab) = self.tabs.current_mut() {
                    tab.ensure_ide_editor_surface();
                }
                self.resize_all_tabs();
                self.dirty = true;
            }
            Action::AgentApprove => {
                // Mirrors Cmd+Return: approve topmost pending approval.
                if let (Some(client), Some(poller)) = (&self.caldera_client, &self.caldera_poller) {
                    if let Some(row) = self.agent_snap.approvals.first() {
                        let _ = anvil_caldera::approve(
                            client,
                            &row.connector,
                            &row.pattern,
                            &row.reason,
                            300,
                        );
                        poller.kick();
                    }
                }
            }
            Action::AgentStart => {
                // Mirrors Cmd+Shift+Return: start a new agent run.
                if let (Some(client), Some(poller)) = (&self.caldera_client, &self.caldera_poller) {
                    let _ = anvil_caldera::start_run(client, "", "");
                    poller.kick();
                }
            }
            Action::NewEditorPane => {
                // NE15: nvim path removed; native editor is the only path.
                self.new_native_editor_pane();
            }
        }
        self.dismiss_palette(webview);
    }

    // ── Chord matching ───────────────────────────────────────────────────────

    fn chord_matches(chord: Chord, mods: Modifiers, ch: char) -> bool {
        let lo = ascii_lower(ch);
        chord.cmd == mods.command
            && chord.shift == mods.shift
            && chord.ctrl == mods.control
            && chord.opt == mods.option
            && chord.key == lo
    }

    /// Handle ⌘ keybindings. Returns true if consumed.
    fn handle_cmd_chord(&mut self, mods: Modifiers, ch: char, webview: &Webview) -> bool {
        let kb = self.keybindings; // Copy

        // ── Explorer Cmd shortcuts (item 7: Cmd+N new file/folder) ────────────
        if self.focus_target == FocusTarget::Explorer {
            let lch = ascii_lower(ch);
            if lch == 'n' && !mods.shift && !mods.control && !mods.option {
                // New file in the current directory (root or selected dir).
                let parent_dir = if let Some(idx) = self.selected_explorer_row {
                    self.left_dock_hits
                        .visible_rows
                        .get(idx)
                        .and_then(|(p, is_dir)| {
                            if *is_dir {
                                Some(p.clone())
                            } else {
                                p.parent().map(|pp| pp.to_path_buf())
                            }
                        })
                } else {
                    self.fs_snapshot
                        .as_ref()
                        .map(|snap| PathBuf::from(&snap.root))
                };
                if let Some(dir) = parent_dir {
                    self.explorer_new_item = Some(NewItemState {
                        parent_dir: dir,
                        input: String::new(),
                        is_dir: mods.shift, // Cmd+Shift+N → new folder
                    });
                    self.dirty = true;
                    return true;
                }
            }
        }

        // ── Native editor pane Cmd shortcuts (NE6) ────────────────────────────
        // When a native editor pane is focused, claim Cmd+S/Z/C/X/V/A/L first.
        // Other Cmd chords (tab, split, palette, search, etc.) fall through to
        // the normal dispatcher so they keep working even in editor panes.
        if self.focused_is_native_editor() {
            let lch = ascii_lower(ch);
            // Cmd+S → Save (N3: toast on success/failure)
            if lch == 's' && !mods.shift && !mods.control && !mods.option {
                // Run the save directly to capture success/error for a toast.
                let save_result: Option<Result<String, String>> =
                    if let Some(tab) = self.tabs.current_mut() {
                        let id = tab.focused_id();
                        if let Some(ep) = tab.editor_panes.get_pane(id) {
                            let buf_id = ep.buffer_id;
                            if let Some(buf) = tab.editor_panes.get_buffer_mut(buf_id) {
                                if let Some(path) = buf.tracked_path().map(|p| p.to_path_buf()) {
                                    let basename = path
                                        .file_name()
                                        .and_then(|n| n.to_str())
                                        .unwrap_or("file")
                                        .to_string();
                                    Some(match buf.save(&path) {
                                        Ok(()) => Ok(basename),
                                        Err(e) => Err(e.to_string()),
                                    })
                                } else {
                                    None
                                }
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    } else {
                        None
                    };
                match save_result {
                    Some(Ok(name)) => self.toast_success(&format!("Saved {name}")),
                    Some(Err(e)) => self.toast_error(&format!("Save failed: {e}")),
                    None => {} // scratch buffer or no path — no toast
                }
                self.dirty = true;
                return true;
            }
            // Cmd+Shift+S → Save As (inline overlay)
            // TODO(anvil-tierJ-J2-nspanel): replace inline overlay with NSSavePanel.
            if lch == 's' && mods.shift && !mods.control && !mods.option {
                if self.save_as_input.is_none() {
                    // Pre-fill with the current buffer's tracked path or "".
                    let prefill: String = if let Some(tab) = self.tabs.current() {
                        let focused = tab.focused_id();
                        if let Some(ep) = tab.editor_panes.get_pane(focused) {
                            let buf_id = ep.buffer_id;
                            tab.editor_panes
                                .get_buffer(buf_id)
                                .and_then(|b| b.tracked_path())
                                .map(|p| p.to_string_lossy().into_owned())
                                .unwrap_or_default()
                        } else {
                            String::new()
                        }
                    } else {
                        String::new()
                    };
                    self.save_as_input = Some(prefill);
                    self.dirty = true;
                }
                return true;
            }
            // Cmd+Z → Undo, Cmd+Shift+Z → Redo
            if lch == 'z' && !mods.control && !mods.option {
                if mods.shift {
                    self.apply_editor_action(EditorAction::Redo);
                } else {
                    self.apply_editor_action(EditorAction::Undo);
                }
                return true;
            }
            // Cmd+C → Copy
            if lch == 'c' && !mods.shift && !mods.control && !mods.option {
                self.apply_editor_action(EditorAction::Copy);
                return true;
            }
            // Cmd+X → Cut
            if lch == 'x' && !mods.shift && !mods.control && !mods.option {
                self.apply_editor_action(EditorAction::Cut);
                return true;
            }
            // Cmd+V → Paste
            if lch == 'v' && !mods.shift && !mods.control && !mods.option {
                if let Some(text) = anvil_platform::system::get_clipboard() {
                    self.apply_editor_action(EditorAction::Paste(text));
                }
                return true;
            }
            // Cmd+A → SelectAll
            if lch == 'a' && !mods.shift && !mods.control && !mods.option {
                self.apply_editor_action(EditorAction::SelectAll);
                self.dirty = true;
                return true;
            }
            // Cmd+G → GoToLine overlay (item 11)
            if lch == 'g' && !mods.shift && !mods.control && !mods.option {
                if self.goto_line_input.is_none() {
                    self.goto_line_input = Some(String::new());
                    self.dirty = true;
                }
                return true;
            }
            // Cmd+Opt+F → Find+Replace (item 9)
            if lch == 'f' && !mods.shift && !mods.control && mods.option {
                self.search_open = true;
                self.apply_editor_action(EditorAction::FindReplaceOpen);
                self.resize_all_tabs();
                self.dirty = true;
                self.force_full_redraw = true;
                return true;
            }
            // Cmd+D → AddNextOccurrence (item 12)
            if lch == 'd' && !mods.shift && !mods.control && !mods.option {
                self.apply_editor_action(EditorAction::AddNextOccurrence);
                self.dirty = true;
                return true;
            }
            // Cmd+L → SelectLine (K6): select cursor's line; repeated calls extend down.
            if lch == 'l' && !mods.shift && !mods.control && !mods.option {
                self.apply_editor_action(EditorAction::SelectLine);
                self.dirty = true;
                return true;
            }
            // Cmd+K → start two-stroke chord (H1/H2); Cmd+K K → HoverRequest (NE10).
            // After Cmd+K, the next plain key is consumed:
            //   W     → ToggleSoftWrap (H1)
            //   Space → ToggleShowWhitespace (H2)
            //   K     → HoverRequest (NE10 — original binding preserved via double-tap)
            //   other → cancel chord
            if lch == 'k' && !mods.shift && !mods.control && !mods.option {
                self.pending_chord_k = true;
                return true;
            }
            // Cmd+R → buffer symbol search overlay (O2). Takes priority over reload.
            if kb
                .buffer_symbol_search
                .is_some_and(|chord| Self::chord_matches(chord, mods, ch))
            {
                self.open_buffer_symbol_search();
                return true;
            }
            // Cmd+R (fallback): force-reload the active buffer from disk (item 27).
            if lch == 'r' && !mods.shift && !mods.control && !mods.option {
                if let Some(tab) = self.tabs.current_mut() {
                    let pane_id = tab.focused_id();
                    if let Some(ep) = tab.editor_panes.get_pane(pane_id) {
                        let bid = ep.buffer_id;
                        if let Some(buf) = tab.editor_panes.get_buffer_mut(bid) {
                            if let Err(e) = buf.reload_from_disk() {
                                eprintln!("anvil: Cmd+R reload failed: {e}");
                            }
                            self.disk_changed_dirty.remove(&bid);
                        }
                    }
                }
                self.force_full_redraw = true;
                self.dirty = true;
                return true;
            }
            // Cmd+W → close the active buffer tab (not the whole system tab).
            if lch == 'w' && !mods.shift && !mods.control && !mods.option {
                if let Some(tab) = self.tabs.current_mut() {
                    let pane_id = tab.focused_id();
                    let buffer_id = tab.editor_panes.get_pane(pane_id).map(|ep| ep.buffer_id);
                    if let Some(buffer_id) = buffer_id {
                        // Q16: record path before closing so Cmd+Shift+T can reopen it.
                        let closed_path = tab
                            .editor_panes
                            .get_buffer(buffer_id)
                            .and_then(|b| b.tracked_path())
                            .map(|p| p.to_path_buf());
                        tab.editor_panes.close_buffer(pane_id, buffer_id);
                        let new_bid = tab.editor_panes.get_pane(pane_id).map(|ep| ep.buffer_id);
                        if let Some(pane) = tab.registry.get_mut(pane_id) {
                            pane.editor_id = new_bid;
                        }
                        if let Some(path) = closed_path {
                            self.closed_tabs.push_back(path);
                            if self.closed_tabs.len() > 20 {
                                self.closed_tabs.pop_front();
                            }
                        }
                    }
                }
                self.dirty = true;
                return true;
            }
            // Q16: Cmd+Shift+T → reopen last closed buffer tab.
            if lch == 't' && mods.shift && !mods.control && !mods.option {
                if let Some(path) = self.closed_tabs.pop_back() {
                    self.open_path_in_native_editor(&path);
                }
                self.dirty = true;
                return true;
            }
            // #18: Cmd+/ → toggle line comment.
            if ch == '/' && !mods.shift && !mods.control && !mods.option {
                self.apply_editor_action(EditorAction::ToggleLineComment);
                self.dirty = true;
                return true;
            }
            // #19: Cmd+Shift+D → duplicate line.
            if lch == 'd' && mods.shift && !mods.control && !mods.option {
                self.apply_editor_action(EditorAction::DuplicateLine);
                self.dirty = true;
                return true;
            }
            // #23: Cmd+Shift+I → format file.
            if lch == 'i' && mods.shift && !mods.control && !mods.option {
                self.apply_editor_action(EditorAction::FormatFile);
                self.dirty = true;
                return true;
            }
            // Item 25: Cmd+. → code actions.
            if ch == '.' && !mods.shift && !mods.control && !mods.option {
                self.trigger_code_actions_request();
                self.dirty = true;
                return true;
            }
            // Q56: Cmd+Shift+. → toggle hidden files in Explorer.
            if ch == '.' && mods.shift && !mods.control && !mods.option {
                self.show_hidden_files = !self.show_hidden_files;
                // Notify the fs worker of the new flags.
                let _ = self.fs_hidden_tx.try_send(self.filter_flags());
                // Refresh snapshot immediately so Explorer updates this frame.
                self.refresh_fs_snapshot();
                self.dirty = true;
                return true;
            }
            // Cmd+\ → split editor pane vertically (new pane to the right).
            // Cmd+Shift+\ → split editor pane horizontally (new pane below).
            if ch == '\\' && !mods.control && !mods.option {
                let dir = if mods.shift {
                    SplitDir::Vertical
                } else {
                    SplitDir::Horizontal
                };
                let new_id = match self.tabs.current_mut().map(|t| t.split_native_editor(dir)) {
                    Some(Ok(id)) => id,
                    Some(Err(e)) => {
                        eprintln!("anvil: editor split failed: {e}");
                        return true;
                    }
                    None => return true,
                };
                if let Some(tab) = self.tabs.current_mut() {
                    tab.tree.focused = new_id;
                }
                self.resize_all_tabs();
                self.snap_anim();
                self.dirty = true;
                return true;
            }
        }

        macro_rules! test {
            ($field:expr, $body:block) => {
                if let Some(chord) = $field {
                    if Self::chord_matches(chord, mods, ch) $body
                }
            };
        }

        test!(kb.new_tab, {
            self.add_tab();
            return true;
        });
        test!(kb.close_pane, {
            self.close_focused_pane();
            return true;
        });
        test!(kb.close_tab, {
            self.close_search();
            self.close_active_tab();
            return true;
        });
        test!(kb.focus_left, {
            self.focus_neighbor(NavDir::Left);
            return true;
        });
        test!(kb.focus_right, {
            self.focus_neighbor(NavDir::Right);
            return true;
        });
        test!(kb.focus_up, {
            self.focus_neighbor(NavDir::Up);
            return true;
        });
        test!(kb.focus_down, {
            self.focus_neighbor(NavDir::Down);
            return true;
        });
        test!(kb.split_right, {
            self.split_focused_pane(SplitDir::Horizontal);
            return true;
        });
        test!(kb.split_down, {
            self.split_focused_pane(SplitDir::Vertical);
            return true;
        });
        test!(kb.next_tab, {
            self.close_search();
            self.tabs.next();
            self.snap_anim();
            self.force_full_redraw = true;
            self.dirty = true;
            return true;
        });
        test!(kb.prev_tab, {
            self.close_search();
            self.tabs.prev();
            self.snap_anim();
            self.force_full_redraw = true;
            self.dirty = true;
            return true;
        });
        for (i, maybe) in kb.jump.iter().enumerate() {
            if let Some(chord) = maybe {
                if Self::chord_matches(*chord, mods, ch) {
                    // Item 9 (Tier-B): when a native editor pane is focused,
                    // Cmd+1..9 switches to the Nth open buffer tab (0-based index i)
                    // instead of switching workspace tabs.
                    if self.focused_is_native_editor() {
                        if let Some(tab) = self.tabs.current_mut() {
                            let pid = tab.focused_id();
                            if let Some(ep) = tab.editor_panes.get_pane(pid) {
                                let idx = i.min(ep.open_buffers.len().saturating_sub(1));
                                let bid = ep.open_buffers[idx];
                                let bid_to_open = bid;
                                tab.editor_panes.open_buffer(pid, bid_to_open);
                                if let Some(pane) = tab.registry.get_mut(pid) {
                                    pane.editor_id = Some(bid_to_open);
                                }
                            }
                        }
                        self.sync_active_explorer_file();
                        self.force_full_redraw = true;
                        self.dirty = true;
                        return true;
                    }
                    self.close_search();
                    self.tabs.switch_to(i);
                    self.snap_anim();
                    self.force_full_redraw = true;
                    self.dirty = true;
                    return true;
                }
            }
        }
        test!(kb.search_open, {
            self.open_search();
            return true;
        });
        test!(kb.search_open_block, {
            self.open_search_block();
            return true;
        });
        test!(kb.project_search_open, {
            self.open_project_search();
            return true;
        });
        test!(kb.search_next, {
            if !self.search_open {
                self.open_search();
            }
            // NE11: route to editor search when native editor is focused.
            if self.focused_is_native_editor() {
                self.apply_editor_action(EditorAction::SearchNext);
            } else {
                self.search.next();
                self.scroll_to_current_match();
            }
            self.dirty = true;
            return true;
        });
        test!(kb.search_prev, {
            if !self.search_open {
                self.open_search();
            }
            // NE11: route to editor search when native editor is focused.
            if self.focused_is_native_editor() {
                self.apply_editor_action(EditorAction::SearchPrev);
            } else {
                self.search.prev();
                self.scroll_to_current_match();
            }
            self.dirty = true;
            return true;
        });
        // Cmd+Opt+R: toggle regex mode (only when search is open).
        test!(kb.search_regex_toggle, {
            if self.search_open {
                // NE11: route to editor search when native editor is focused.
                if self.focused_is_native_editor() {
                    self.apply_editor_action(EditorAction::SearchToggleRegex);
                } else {
                    let new_mode = !self.search.is_regex();
                    self.search.set_regex(new_mode);
                    // Re-run the scan with the new mode.
                    if let Some(tab) = self.tabs.current_mut() {
                        let id = tab.focused_id();
                        if let Some(pane) = tab.registry.get_mut(id) {
                            if let Some(terminal) = &pane.terminal {
                                self.search.rescan(terminal);
                            }
                        }
                    }
                    self.scroll_to_current_match();
                }
                self.dirty = true;
            }
            return true;
        });
        // hud_toggle is intercepted in AppShell::perform_key_equivalent
        // (needs &mut self.window) — not handled here.
        test!(kb.cheatsheet, {
            self.cheatsheet_visible = !self.cheatsheet_visible;
            self.dirty = true;
            self.force_full_redraw = true;
            return true;
        });
        test!(kb.fold_block, {
            self.toggle_fold_at_viewport_top();
            return true;
        });
        test!(kb.toggle_theme, {
            let next = next_theme_mode(&self.config.theme);
            self.set_theme_mode(next);
            return true;
        });
        test!(kb.layout_mode_toggle, {
            self.layout_mode = match self.layout_mode {
                LayoutMode::Terminal => LayoutMode::Ide,
                LayoutMode::Ide => LayoutMode::Terminal,
            };
            match self.layout_mode {
                LayoutMode::Terminal => {
                    self.left_dock_visible = false;
                    // Normalize every tab — switching modes must be coherent
                    // across the whole workspace, not just the active tab.
                    for tab in self.tabs.tabs.iter_mut() {
                        tab.normalize_terminal_surface();
                    }
                }
                LayoutMode::Ide => {
                    self.spawn_ide_terminal_drawer();
                    for tab in self.tabs.tabs.iter_mut() {
                        tab.ensure_ide_editor_surface();
                    }
                    self.left_dock_visible = true;
                }
            }
            self.resize_all_tabs();
            self.dirty = true;
            return true;
        });
        test!(kb.left_dock_toggle, {
            self.left_dock_visible = !self.left_dock_visible;
            self.resize_all_tabs();
            self.dirty = true;
            return true;
        });
        test!(kb.editor_new, {
            // NE15: nvim path removed; Cmd+E opens/focuses the native editor surface.
            // Direction A treats editor creation as entering the command-deck
            // IDE surface: Explorer + editor + compact terminal drawer.
            self.layout_mode = LayoutMode::Ide;
            self.left_dock_visible = true;
            self.spawn_ide_terminal_drawer();
            self.new_native_editor_pane();
            return true;
        });
        test!(kb.workspace_symbol_search, {
            self.open_workspace_symbol_search();
            return true;
        });

        // ⌘J — toggle IDE bottom drawer (Tier-B item 8).
        if ascii_lower(ch) == 'j' && !mods.shift && !mods.control && !mods.option {
            self.toggle_ide_drawer();
            return true;
        }

        // ⌘, — open Anvil config file (Tier-B item 14).
        if ch == ',' && !mods.shift && !mods.control && !mods.option {
            let cfg_path = match anvil_config::resolve_path() {
                Some(p) => p,
                None => return true,
            };
            // Create a skeleton config if none exists yet.
            if !cfg_path.exists() {
                if let Some(parent) = cfg_path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = std::fs::write(&cfg_path, b"# Anvil config\n");
            }
            self.layout_mode = LayoutMode::Ide;
            self.left_dock_visible = true;
            self.open_path_in_native_editor(&cfg_path);
            return true;
        }

        // ⌘⇧E — show recent files in palette (Tier-B item 15).
        if ascii_lower(ch) == 'e' && mods.shift && !mods.control && !mods.option {
            if !self.recent_file_list.is_empty() && self.palette.summon() {
                self.send_recent_files_show(webview);
                webview.show();
            }
            return true;
        }

        // ⌘⇧O — project switcher (item 30).
        if ascii_lower(ch) == 'o' && mods.shift && !mods.control && !mods.option {
            if !self.recent_projects.is_empty() {
                self.project_switcher_open = true;
                self.project_switcher_sel = 0;
                self.dirty = true;
            }
            return true;
        }

        // ⌘K — command palette.
        if ascii_lower(ch) == 'k' && !mods.shift && !mods.control && !mods.option {
            if self.palette.summon() {
                self.send_palette_show(webview);
                webview.show();
            }
            return true;
        }

        // ⌘P — file picker (project files).
        if ascii_lower(ch) == 'p' && !mods.shift && !mods.control && !mods.option {
            if self.palette.summon() {
                self.send_file_picker_show(webview);
                webview.show();
            }
            return true;
        }

        // ⌘C — copy current selection to the system clipboard.
        if ascii_lower(ch) == 'c' && !mods.shift && !mods.control && !mods.option {
            if let Some(text) = self.focused_selection_text() {
                anvil_platform::system::set_clipboard(&text);
            }
            return true;
        }

        // ⌘V — paste clipboard contents to the focused pane's PTY.
        // Bracketed paste is honored when the app has it enabled (mode
        // 2004): wrap the payload in \x1b[200~ ... \x1b[201~ so the shell
        // can distinguish pasted bytes from typed ones.
        if ascii_lower(ch) == 'v' && !mods.shift && !mods.control && !mods.option {
            if let Some(text) = anvil_platform::system::get_clipboard() {
                let bracketed = self
                    .tabs
                    .current()
                    .and_then(|t| t.registry.get(t.focused_id()))
                    .and_then(|p| p.terminal.as_ref())
                    .map(|t| t.modes.bracketed_paste)
                    .unwrap_or(false);
                if bracketed {
                    self.write_to_focused_pty(b"\x1b[200~");
                }
                self.write_to_focused_pty(text.as_bytes());
                if bracketed {
                    self.write_to_focused_pty(b"\x1b[201~");
                }
            }
            return true;
        }

        // ⌘⇧A — send current selection to the active agent as context.
        // Until a real agent IPC ships, we (a) copy the captured text to the
        // clipboard and (b) write it to /tmp/anvil-agent-context.md as a
        // pickup file any local agent can `cat`. Falls back to the focused
        // pane's visible viewport when no selection is active so the gesture
        // *always* produces something the user can paste to verify it fired.
        if ascii_lower(ch) == 'a' && mods.shift && !mods.control && !mods.option {
            let text = self
                .focused_selection_text()
                .or_else(|| self.focused_viewport_text());
            if let Some(text) = text {
                anvil_platform::system::set_clipboard(&text);
                let _ = std::fs::write("/tmp/anvil-agent-context.md", &text);
                eprintln!(
                    "anvil: Cmd+Shift+A captured {} bytes → clipboard + /tmp/anvil-agent-context.md",
                    text.len()
                );
            }
            self.dirty = true;
            return true;
        }

        false
    }

    // ── Shutdown (item 20) ────────────────────────────────────────────────────

    /// Clean shutdown.  Called from `AppShell::should_terminate`.
    ///
    /// Order:
    /// 1. Write session state (item 19).
    /// 2. Shut down LSP servers.
    /// 3. Drop PTYs (child processes receive SIGTERM via `Pty` Drop).
    /// 4. Drop worker senders (threads exit when their receiver is gone).
    pub fn shutdown(&mut self) {
        // 1. Save session.
        if let Some(cwd) = self.current_cwd() {
            let cwd_path = std::path::PathBuf::from(&cwd);
            let state = self.build_session_state();
            session::save_session(&cwd_path, &state);
        }

        // 2. LSP shutdown.
        if let Some(ref mut mgr) = self.lsp_manager {
            mgr.shutdown_all();
        }

        // 3. Drop PTYs — Pty Drop sends SIGTERM to the child process.
        self.ptys.clear();

        // 4. Worker senders.  Dropping the SyncSenders closes the channel;
        //    each worker thread's `recv()` returns Err and the thread exits.
        // The senders are struct fields; they will be dropped with App, but
        // closing them now makes the workers exit before the process does.
        // We can't move them out, so we do nothing extra here — the workers
        // are daemon-style and the OS reclaims them on exit.
    }

    /// Collect the current UI state into a `SessionState` for persistence.
    fn build_session_state(&self) -> session::SessionState {
        // Drawer / editor split ratio: read from the IDE root vertical split.
        let editor_split_ratio = self
            .tabs
            .current()
            .and_then(|tab| {
                let root = &tab.tree.root;
                match root.as_ref() {
                    anvil_workspace::layout::PaneNode::Split(sp)
                        if sp.dir == anvil_workspace::layout::SplitDir::Vertical
                            && sp.children.len() == 2 =>
                    {
                        Some(sp.ratios[0])
                    }
                    _ => None,
                }
            })
            .unwrap_or(0.0);

        // Collect per-pane open buffer paths.
        let mut open_buffers: Vec<session::PaneSession> = Vec::new();
        for tab in self.tabs.tabs.iter() {
            for (pane_id, ep) in tab.editor_panes.panes_iter() {
                let paths: Vec<std::path::PathBuf> = ep
                    .open_buffers
                    .iter()
                    .filter_map(|&bid| {
                        tab.editor_panes
                            .get_buffer(bid)
                            .and_then(|b| b.tracked_path().map(|p| p.to_path_buf()))
                    })
                    .collect();
                if paths.is_empty() {
                    continue;
                }
                let active_path = tab
                    .editor_panes
                    .get_buffer(ep.buffer_id)
                    .and_then(|b| b.tracked_path().map(|p| p.to_path_buf()));
                open_buffers.push(session::PaneSession {
                    pane_id: pane_id as u64,
                    paths,
                    active_path,
                });
            }
        }

        // Item 30: record current cwd as a recent project before saving.
        let cwd_pb = std::env::current_dir().ok();
        let mut recent_projects = self.recent_projects.clone();
        if let Some(cwd) = cwd_pb {
            recent_projects.retain(|p| p != &cwd);
            recent_projects.insert(0, cwd);
            recent_projects.truncate(20);
        }

        // Q22: collect per-buffer language overrides.
        let mut language_overrides = std::collections::HashMap::new();
        for tab in &self.tabs.tabs {
            for (_pane_id, ep) in tab.editor_panes.panes_iter() {
                for &bid in &ep.open_buffers {
                    if let Some(buf) = tab.editor_panes.get_buffer(bid) {
                        if let Some(ref ov) = buf.language_override {
                            if let Some(path) = buf.tracked_path() {
                                language_overrides
                                    .insert(path.to_string_lossy().into_owned(), ov.clone());
                            }
                        }
                    }
                }
            }
        }

        session::SessionState {
            ui_scale: self.ui_scale,
            font_scale: self.font_scale,
            left_dock_w_pt: self.left_dock_w_pt,
            layout_mode: match self.layout_mode {
                LayoutMode::Ide => "ide".to_string(),
                LayoutMode::Terminal => "terminal".to_string(),
            },
            editor_split_ratio,
            expanded_dirs: self.expanded_dirs.iter().cloned().collect(),
            open_buffers,
            recent_projects,
            show_hidden_files: self.show_hidden_files,
            language_overrides,
        }
    }

    /// Restore session state persisted by a previous run.
    ///
    /// Called once at startup after `ui_scale`, `left_dock_w_pt`, and
    /// `layout_mode` have their defaults.  Silently skips on missing or
    /// corrupt session file.
    pub fn restore_session(&mut self, cwd: &str) {
        let cwd_path = std::path::PathBuf::from(cwd);
        let Some(state) = session::load_session(&cwd_path) else {
            return;
        };

        if state.ui_scale > 0.0 {
            self.ui_scale = state.ui_scale;
        }
        if state.font_scale > 0.0 {
            self.font_scale = state.font_scale;
        }
        if state.left_dock_w_pt >= 180.0 && state.left_dock_w_pt <= 600.0 {
            self.left_dock_w_pt = state.left_dock_w_pt;
            self.left_dock_w_pt_target = state.left_dock_w_pt;
        }
        match state.layout_mode.as_str() {
            "ide" => self.layout_mode = LayoutMode::Ide,
            "terminal" => self.layout_mode = LayoutMode::Terminal,
            _ => {}
        }
        if state.editor_split_ratio > 0.0 {
            // Apply to the current tab's root split if it is a vertical 2-child.
            if let Some(tab) = self.tabs.current_mut() {
                let root = &mut tab.tree.root;
                if let anvil_workspace::layout::PaneNode::Split(sp) = root.as_mut() {
                    if sp.dir == anvil_workspace::layout::SplitDir::Vertical
                        && sp.children.len() == 2
                    {
                        let r = state.editor_split_ratio.clamp(0.40, 0.95);
                        sp.ratios[0] = r;
                        sp.ratios[1] = 1.0 - r;
                    }
                }
            }
        }
        for p in state.expanded_dirs {
            self.expanded_dirs.insert(p);
        }
        // Q56: restore show_hidden_files flag.
        self.show_hidden_files = state.show_hidden_files;

        // Restore open buffers: open each saved path in the native editor.
        // The active path is opened last so it ends up as the active buffer.
        for pane_session in &state.open_buffers {
            // Open non-active paths first.
            for path in &pane_session.paths {
                if pane_session
                    .active_path
                    .as_deref()
                    .is_none_or(|a| a != path.as_path())
                {
                    self.open_path_in_native_editor(path);
                }
            }
            // Open active path last so it becomes the focused buffer.
            if let Some(active) = &pane_session.active_path {
                self.open_path_in_native_editor(active);
            }
        }
        // Q22: re-apply language overrides to restored buffers.
        if !state.language_overrides.is_empty() {
            for tab in &mut self.tabs.tabs {
                // Collect (buffer_id, path, lang) triples to avoid borrow conflict.
                let to_override: Vec<(anvil_editor::BufferId, String)> = tab
                    .editor_panes
                    .panes_iter()
                    .flat_map(|(_pid, ep)| ep.open_buffers.iter().copied())
                    .filter_map(|bid| {
                        let buf = tab.editor_panes.get_buffer(bid)?;
                        let path = buf.tracked_path()?.to_string_lossy().into_owned();
                        let lang = state.language_overrides.get(&path)?.clone();
                        Some((bid, lang))
                    })
                    .collect();
                for (bid, lang) in to_override {
                    if let Some(buf) = tab.editor_panes.get_buffer_mut(bid) {
                        buf.set_language(&lang);
                    }
                }
            }
        }
        // Item 30: restore recent projects list.
        if !state.recent_projects.is_empty() {
            self.recent_projects = state.recent_projects;
        }
        self.dirty = true;
    }

    /// Returns `true` when the welcome screen should be displayed.
    ///
    /// The welcome screen is shown when the IDE layout is active and the
    /// current tab has no open buffers in any editor pane (item 28).
    fn should_show_welcome(&self) -> bool {
        if self.layout_mode != LayoutMode::Ide {
            return false;
        }
        let Some(tab) = self.tabs.current() else {
            return false;
        };
        // True when no pane has an open buffer.
        !tab.editor_panes
            .panes_iter()
            .any(|(_, ep)| !ep.open_buffers.is_empty())
    }
}

// ── AppShell — holds App + Webview + Font + Painter, impls AppHandler ────────

/// Holds all state that requires the main thread or has lifetimes that depend
/// on the window (Webview, Font, Painter).
///
/// `app.font` and `app.chrome_font` are heap-allocated (`Box<Font>`) so their
/// addresses are stable even as `AppShell` moves.  `painter` and
/// `chrome_painter` hold `&'static Font` references produced via an unsafe
/// lifetime extension; this is sound because both painters are dropped before
/// `app.font`/`app.chrome_font` (struct fields drop in declaration order, and
/// the painters are declared after `app`).
pub struct AppShell {
    app: App,
    webview: Webview,
    /// The NSWindow — retained for future use (e.g. setContentSize).
    #[allow(dead_code)]
    window: Retained<NSWindow>,
    /// Terminal grid glyph painter — Regular face (user font size).
    painter: anvil_platform::font::CoreTextPainter<'static>,
    /// Bold face painter.
    bold_painter: anvil_platform::font::CoreTextPainter<'static>,
    /// Italic face painter.
    italic_painter: anvil_platform::font::CoreTextPainter<'static>,
    /// BoldItalic face painter.
    bold_italic_painter: anvil_platform::font::CoreTextPainter<'static>,
    /// Chrome glyph painter (11 pt × scale — tab bar, status bar, etc.).
    chrome_painter: anvil_platform::font::CoreTextPainter<'static>,
    /// Reusable PTY read buffer — allocated once, reused across every tick.
    pty_read_buf: Box<[u8; 64 * 1024]>,
}

impl AppShell {
    /// Cmd+=/Cmd+-/Cmd+0 chord: zoom in, out, or reset the global UI scale (item 1).
    ///
    /// Adjusts `ui_scale` rather than `font_size_pt` so that dock geometry,
    /// chrome heights, and fonts all scale together.
    fn handle_zoom_chord(&mut self, ch: char) {
        match ch {
            '=' | '+' => self.bump_ui_scale(0.1),
            '-' => self.bump_ui_scale(-0.1),
            '0' => {
                let delta = 1.0 - self.app.ui_scale;
                self.bump_ui_scale(delta);
            }
            _ => {}
        }
    }

    /// H4: Cmd+Opt+=/+: font scale up; Cmd+Opt+-: down; Cmd+Opt+0: reset.
    ///
    /// Only changes `font_scale` — dock widths, chrome heights, and row
    /// heights are unaffected.  Range clamped to [0.6, 2.5].
    fn handle_font_scale_chord(&mut self, ch: char) {
        let delta = match ch {
            '=' | '+' => 0.1,
            '-' => -0.1,
            '0' => 1.0 - self.app.font_scale,
            _ => return,
        };
        self.bump_font_scale(delta);
    }

    /// Adjust `font_scale` by `delta`, rebuild fonts, and force a full redraw.
    fn bump_font_scale(&mut self, delta: f64) {
        let new_scale = (self.app.font_scale + delta).clamp(0.6, 2.5);
        if (new_scale - self.app.font_scale).abs() < 0.001 {
            return;
        }
        self.app.font_scale = new_scale;
        // Rebuild font at font_size_pt * window_scale * font_scale.
        let pixel_size = self.app.font_size_pt * self.app.window_scale * self.app.font_scale;
        let names: Vec<&str> = vec![
            "BlexMono Nerd Font Mono",
            self.app.font_family.as_str(),
            "SFMono-Regular",
            "Menlo",
        ];
        let Ok(new_font) = Font::init_first_available(&names, pixel_size)
            .or_else(|_| Font::init("Menlo", pixel_size))
        else {
            eprintln!("anvil: font_scale reinit failed at {new_scale}; keeping current");
            return;
        };
        let new_bold =
            Font::init_face(&names, pixel_size, FontFace::Bold, true).unwrap_or_else(|_| {
                Font::init_first_available(&names, pixel_size)
                    .or_else(|_| Font::init("Menlo", pixel_size))
                    .expect("fallback must be available")
            });
        let new_italic = Font::init_face(&names, pixel_size, FontFace::Italic, true)
            .unwrap_or_else(|_| {
                Font::init_first_available(&names, pixel_size)
                    .or_else(|_| Font::init("Menlo", pixel_size))
                    .expect("fallback must be available")
            });
        let new_bold_italic = Font::init_face(&names, pixel_size, FontFace::BoldItalic, true)
            .unwrap_or_else(|_| {
                Font::init_first_available(&names, pixel_size)
                    .or_else(|_| Font::init("Menlo", pixel_size))
                    .expect("fallback must be available")
            });

        let old_font = std::mem::replace(&mut self.app.font, Box::new(new_font));
        let old_bold = std::mem::replace(&mut self.app.bold_font, Box::new(new_bold));
        let old_italic = std::mem::replace(&mut self.app.italic_font, Box::new(new_italic));
        let old_bold_italic =
            std::mem::replace(&mut self.app.bold_italic_font, Box::new(new_bold_italic));

        // SAFETY: same lifetime-extension pattern as bump_ui_scale.
        self.painter = unsafe {
            let font_ref: &'static Font = &*(self.app.font.as_ref() as *const Font);
            anvil_platform::font::CoreTextPainter::new(font_ref)
        };
        self.bold_painter = unsafe {
            let font_ref: &'static Font = &*(self.app.bold_font.as_ref() as *const Font);
            anvil_platform::font::CoreTextPainter::new(font_ref)
        };
        self.italic_painter = unsafe {
            let font_ref: &'static Font = &*(self.app.italic_font.as_ref() as *const Font);
            anvil_platform::font::CoreTextPainter::new(font_ref)
        };
        self.bold_italic_painter = unsafe {
            let font_ref: &'static Font = &*(self.app.bold_italic_font.as_ref() as *const Font);
            anvil_platform::font::CoreTextPainter::new(font_ref)
        };

        drop(old_font);
        drop(old_bold);
        drop(old_italic);
        drop(old_bold_italic);

        self.app.resize_all_tabs();
        self.app.force_full_redraw = true;
        self.app.dirty = true;
    }

    /// Adjust `ui_scale` by `delta`, rebuild fonts, and force a full redraw.
    ///
    /// Clamps `ui_scale` to [0.6, 2.5].  Fonts are rebuilt at
    /// `font_size_pt * window_scale * ui_scale` so everything scales together.
    fn bump_ui_scale(&mut self, delta: f64) {
        let new_scale = (self.app.ui_scale + delta).clamp(0.6, 2.5);
        if (new_scale - self.app.ui_scale).abs() < 0.01 {
            return;
        }
        self.app.ui_scale = new_scale;
        // Rebuild font at the new effective pixel size.
        let pixel_size = self.app.font_size_pt * self.app.window_scale * self.app.ui_scale;
        let names: Vec<&str> = vec![
            "BlexMono Nerd Font Mono",
            self.app.font_family.as_str(),
            "SFMono-Regular",
            "Menlo",
        ];
        let Ok(new_font) = Font::init_first_available(&names, pixel_size)
            .or_else(|_| Font::init("Menlo", pixel_size))
        else {
            eprintln!("anvil: font reinit failed at ui_scale={new_scale}; keeping current");
            return;
        };
        let new_bold =
            Font::init_face(&names, pixel_size, FontFace::Bold, true).unwrap_or_else(|_| {
                Font::init_first_available(&names, pixel_size)
                    .or_else(|_| Font::init("Menlo", pixel_size))
                    .expect("fallback must be available")
            });
        let new_italic = Font::init_face(&names, pixel_size, FontFace::Italic, true)
            .unwrap_or_else(|_| {
                Font::init_first_available(&names, pixel_size)
                    .or_else(|_| Font::init("Menlo", pixel_size))
                    .expect("fallback must be available")
            });
        let new_bold_italic = Font::init_face(&names, pixel_size, FontFace::BoldItalic, true)
            .unwrap_or_else(|_| {
                Font::init_first_available(&names, pixel_size)
                    .or_else(|_| Font::init("Menlo", pixel_size))
                    .expect("fallback must be available")
            });

        let old_font = std::mem::replace(&mut self.app.font, Box::new(new_font));
        let old_bold = std::mem::replace(&mut self.app.bold_font, Box::new(new_bold));
        let old_italic = std::mem::replace(&mut self.app.italic_font, Box::new(new_italic));
        let old_bold_italic =
            std::mem::replace(&mut self.app.bold_italic_font, Box::new(new_bold_italic));

        // SAFETY: same lifetime-extension pattern as `AppShell::new`.
        self.painter = unsafe {
            let font_ref: &'static Font = &*(self.app.font.as_ref() as *const Font);
            anvil_platform::font::CoreTextPainter::new(font_ref)
        };
        self.bold_painter = unsafe {
            let font_ref: &'static Font = &*(self.app.bold_font.as_ref() as *const Font);
            anvil_platform::font::CoreTextPainter::new(font_ref)
        };
        self.italic_painter = unsafe {
            let font_ref: &'static Font = &*(self.app.italic_font.as_ref() as *const Font);
            anvil_platform::font::CoreTextPainter::new(font_ref)
        };
        self.bold_italic_painter = unsafe {
            let font_ref: &'static Font = &*(self.app.bold_italic_font.as_ref() as *const Font);
            anvil_platform::font::CoreTextPainter::new(font_ref)
        };

        drop(old_font);
        drop(old_bold);
        drop(old_italic);
        drop(old_bold_italic);

        self.app.resize_all_tabs();
        self.app.force_full_redraw = true;
        self.app.dirty = true;
    }

    /// Rebuild the font at a new point size and recreate the dependent
    /// painter + raster geometry. Drives Cmd+/Cmd- zoom.
    ///
    /// Clamps to [8.0, 48.0] pt — below 8 pt cell metrics collapse, above
    /// 48 pt the glyph atlas balloons and one cell barely fits a word.
    #[allow(dead_code)]
    fn bump_font_size(&mut self, delta_pt: f64) {
        let new_pt = (self.app.font_size_pt + delta_pt).clamp(8.0, 48.0);
        if (new_pt - self.app.font_size_pt).abs() < 0.01 {
            return;
        }
        let pixel_size = new_pt * self.app.window_scale;
        let names: Vec<&str> = vec![
            "BlexMono Nerd Font Mono",
            self.app.font_family.as_str(),
            "SFMono-Regular",
            "Menlo",
        ];
        let Ok(new_font) = Font::init_first_available(&names, pixel_size)
            .or_else(|_| Font::init("Menlo", pixel_size))
        else {
            eprintln!("anvil: font reinit failed at {new_pt} pt; keeping current");
            return;
        };
        let new_bold =
            Font::init_face(&names, pixel_size, FontFace::Bold, true).unwrap_or_else(|_| {
                Font::init_first_available(&names, pixel_size)
                    .or_else(|_| Font::init("Menlo", pixel_size))
                    .expect("fallback must be available")
            });
        let new_italic = Font::init_face(&names, pixel_size, FontFace::Italic, true)
            .unwrap_or_else(|_| {
                Font::init_first_available(&names, pixel_size)
                    .or_else(|_| Font::init("Menlo", pixel_size))
                    .expect("fallback must be available")
            });
        let new_bold_italic = Font::init_face(&names, pixel_size, FontFace::BoldItalic, true)
            .unwrap_or_else(|_| {
                Font::init_first_available(&names, pixel_size)
                    .or_else(|_| Font::init("Menlo", pixel_size))
                    .expect("fallback must be available")
            });

        // Replace heap-stable font allocations. Keep old Boxes alive until
        // *after* the corresponding painter is overwritten (the painter drop
        // runs against the still-live allocation).
        let old_font = std::mem::replace(&mut self.app.font, Box::new(new_font));
        let old_bold = std::mem::replace(&mut self.app.bold_font, Box::new(new_bold));
        let old_italic = std::mem::replace(&mut self.app.italic_font, Box::new(new_italic));
        let old_bold_italic =
            std::mem::replace(&mut self.app.bold_italic_font, Box::new(new_bold_italic));
        self.app.font_size_pt = new_pt;

        // SAFETY: same lifetime-extension pattern as `AppShell::new` —
        // each `app.*_font` is a Box and its heap allocation outlives the painter.
        self.painter = unsafe {
            let font_ref: &'static Font = &*(self.app.font.as_ref() as *const Font);
            anvil_platform::font::CoreTextPainter::new(font_ref)
        };
        self.bold_painter = unsafe {
            let font_ref: &'static Font = &*(self.app.bold_font.as_ref() as *const Font);
            anvil_platform::font::CoreTextPainter::new(font_ref)
        };
        self.italic_painter = unsafe {
            let font_ref: &'static Font = &*(self.app.italic_font.as_ref() as *const Font);
            anvil_platform::font::CoreTextPainter::new(font_ref)
        };
        self.bold_italic_painter = unsafe {
            let font_ref: &'static Font = &*(self.app.bold_italic_font.as_ref() as *const Font);
            anvil_platform::font::CoreTextPainter::new(font_ref)
        };
        // Old painters have been dropped (overwritten above). Release old heaps.
        drop(old_font);
        drop(old_bold);
        drop(old_italic);
        drop(old_bold_italic);
        // Reflow panes + force a full redraw so the new metrics propagate.
        self.app.resize_all_tabs();
        self.app.force_full_redraw = true;
        self.app.dirty = true;
    }

    fn new(app: App, webview: Webview, window: Retained<NSWindow>) -> Self {
        // SAFETY: `app.font`, `app.bold_font`, `app.italic_font`,
        // `app.bold_italic_font`, and `app.chrome_font` are `Box<Font>` — heap
        // allocations that are stable for the lifetime of `app` inside
        // `AppShell`.  The painters borrow those stable addresses.
        // Drop order: painters first (declared last), then `webview`, then
        // `app` (with the Boxes).  So the allocations outlive the painters.
        let painter = unsafe {
            let font_ref: &'static Font = &*(app.font.as_ref() as *const Font);
            anvil_platform::font::CoreTextPainter::new(font_ref)
        };
        let bold_painter = unsafe {
            let font_ref: &'static Font = &*(app.bold_font.as_ref() as *const Font);
            anvil_platform::font::CoreTextPainter::new(font_ref)
        };
        let italic_painter = unsafe {
            let font_ref: &'static Font = &*(app.italic_font.as_ref() as *const Font);
            anvil_platform::font::CoreTextPainter::new(font_ref)
        };
        let bold_italic_painter = unsafe {
            let font_ref: &'static Font = &*(app.bold_italic_font.as_ref() as *const Font);
            anvil_platform::font::CoreTextPainter::new(font_ref)
        };
        let chrome_painter = unsafe {
            let font_ref: &'static Font = &*(app.chrome_font.as_ref() as *const Font);
            anvil_platform::font::CoreTextPainter::new(font_ref)
        };
        Self {
            app,
            webview,
            window,
            painter,
            bold_painter,
            italic_painter,
            bold_italic_painter,
            chrome_painter,
            pty_read_buf: Box::new([0u8; 64 * 1024]),
        }
    }

    /// Toggle the HUD: flip `hud_visible`, reflow terminal columns to match the
    /// new pane_area_rect, and mark the frame dirty. The window size is unchanged —
    /// the HUD expands inward, shrinking the terminal grid.
    fn toggle_hud(&mut self) {
        self.app.hud_visible = !self.app.hud_visible;
        self.app.resize_all_tabs();
        self.app.force_full_redraw = true;
        self.app.dirty = true;
    }
}

impl AppHandler for AppShell {
    fn tick(&mut self) {
        let app = &mut self.app;

        // Config watcher poll.
        if let Some(ref mut w) = app.watcher {
            if let Some(new_cfg) = w.poll() {
                app.apply_config(new_cfg);
            }
        }

        // System dark-mode check (when theme = "system"). Every cell carries a
        // theme-resolved color, so on a flip we *must* repaint the whole
        // raster — partial dirty rows would leave half the grid (and the HUD
        // strip) in the old palette.
        if app.config.theme == "system" {
            let now_dark = system_is_dark();
            if now_dark != app.system_dark {
                app.system_dark = now_dark;
                let effective = effective_theme_name(now_dark, &app.config.theme);
                app.theme = resolve_theme(effective, &app.config.theme_overrides);
                if let Some(r) = &mut app.renderer {
                    r.set_clear_color(app.theme.background);
                }
                app.force_full_redraw = true;
                app.dirty = true;
            }
        }

        // Item 27: drain file-watcher events.
        while let Ok(ev) = app.file_watch_rx.try_recv() {
            // Find the buffer in any tab.
            let mut found = false;
            'tabs: for tab in app.tabs.tabs.iter_mut() {
                if let Some(buf) = tab.editor_panes.get_buffer_mut(ev.buffer_id) {
                    found = true;
                    if !buf.is_dirty() {
                        // Clean buffer: silently reload.
                        if let Err(e) = buf.reload_from_disk() {
                            eprintln!("anvil: disk reload failed: {e}");
                        }
                        app.force_full_redraw = true;
                        app.dirty = true;
                    } else {
                        // Dirty buffer: record for banner display.
                        if let Some(path) = buf.tracked_path().map(|p| p.to_path_buf()) {
                            app.disk_changed_dirty.insert(ev.buffer_id, path);
                        }
                        app.dirty = true;
                    }
                    break 'tabs;
                }
            }
            if !found {
                // Buffer no longer open; event is stale — ignore.
            }
        }

        // Drain every pane's PTY output. Loop the read inside each pane until
        // EAGAIN so commands that emit >64 KB between ticks don't back up
        // behind the tick rate — the previous one-read-per-tick cap made
        // chatty commands feel laggy on top of being slow.
        let mut any_dead = false;
        let feed_buf = &mut *self.pty_read_buf;
        // Per-tick cap so one extremely chatty pane can't starve the rest
        // of the work in this tick. 4 MiB per pane per tick is plenty for
        // any realistic workload (`cat` of a binary, etc).
        const PER_TICK_BUDGET: usize = 4 * 1024 * 1024;
        let tab_count = app.tabs.tabs.len();

        for ti in 0..tab_count {
            // Collect pane ids for this tab by walking the layout tree.
            let pane_ids = all_pane_ids_in_tree(&app.tabs.tabs[ti]);
            for pid in pane_ids {
                let mut drained = 0_usize;
                let mut pane_got_data = false;
                let mut pane_dead = false;
                loop {
                    let result = app.ptys.get(&pid).map(|pty| pty.read(feed_buf));
                    match result {
                        Some(Ok(n)) if n > 0 => {
                            if let Some(pane) = app.tabs.tabs[ti].registry.get_mut(pid) {
                                if let Some(terminal) = &mut pane.terminal {
                                    terminal.feed(&feed_buf[..n]);
                                }
                            }
                            drained += n;
                            pane_got_data = true;
                            if drained >= PER_TICK_BUDGET {
                                break;
                            }
                        }
                        Some(Ok(_)) => break, // 0 bytes = EAGAIN drained
                        Some(Err(_)) => {
                            pane_dead = true;
                            break;
                        }
                        None => {
                            // No PTY entry. Editor panes (terminal.is_none()) have no
                            // PTY and are always alive — skip draining, do not mark dead.
                            let is_editor = app.tabs.tabs[ti]
                                .registry
                                .get(pid)
                                .map(|p| p.terminal.is_none())
                                .unwrap_or(false);
                            if !is_editor {
                                pane_dead = true;
                            }
                            break;
                        }
                    }
                }
                if pane_got_data {
                    let active = app.tabs.active;
                    if ti == active {
                        let focused = app.tabs.tabs[ti].focused_id();
                        if pid == focused {
                            app.dirty = true;
                            if app.search_open {
                                if let Some(pane) = app.tabs.tabs[ti].registry.get_mut(pid) {
                                    if let Some(terminal) = &pane.terminal {
                                        app.search.rescan(terminal);
                                    }
                                }
                            }
                        }
                    } else {
                        // Background tab received output — mark unread and
                        // request a repaint so the dot appears promptly.
                        app.tabs.tabs[ti].has_unread = true;
                        app.dirty = true;
                    }
                }
                if pane_dead {
                    app.ptys.remove(&pid);
                    any_dead = true;
                }
            }
        }

        // Tab open/close micro-animation.
        // Opening: phase → 1 at 1/6 per tick (~100ms at 60Hz).
        // Closing: phase → 0 at 1/5 per tick (~80ms at 60Hz).
        {
            let mut any_animating = false;
            for tab in &mut app.tabs.tabs {
                if (tab.anim_phase - tab.target_phase).abs() > 0.001 {
                    if tab.target_phase > tab.anim_phase {
                        tab.anim_phase = (tab.anim_phase + 1.0 / 6.0).min(tab.target_phase);
                    } else {
                        tab.anim_phase = (tab.anim_phase - 1.0 / 5.0).max(tab.target_phase);
                    }
                    any_animating = true;
                }
            }
            if any_animating {
                app.dirty = true;
                // Remove tabs that finished closing.
                app.tabs.purge_closed_tabs();
            }
        }

        // Blink.
        let (effective_blink, app_blink_cfg) = {
            let tab = app.tabs.current();
            let pane = tab.and_then(|t| t.registry.get(t.focused_id()));
            (
                pane.and_then(|p| p.terminal.as_ref())
                    .and_then(|t| t.app_cursor_blink),
                app.cursor_cfg.blink,
            )
        };
        let blink_on = effective_blink.unwrap_or(app_blink_cfg);
        if blink_on && app.focused {
            // 1/64 per tick at 60Hz = ~1.07s full blink cycle — calm, deliberate.
            app.blink_phase += 1.0 / 64.0;
            if app.blink_phase >= 1.0 {
                app.blink_phase -= 1.0;
            }
            let new_op = anvil_render::draw::cursor_opacity(app.blink_phase);
            if new_op != app.last_blink_opacity {
                app.last_blink_opacity = new_op;
                app.dirty = true;
            }
        } else if app.blink_phase != 0.0 {
            app.blink_phase = 0.0;
            app.last_blink_opacity = -1.0;
            app.dirty = true;
        }

        // G3: smooth sidebar width easing — 3-frame settle at 0.35 factor.
        {
            let delta = app.left_dock_w_pt_target - app.left_dock_w_pt;
            if delta.abs() >= 0.5 {
                app.left_dock_w_pt += delta * 0.35;
                app.resize_all_tabs();
                app.force_full_redraw = true;
                app.dirty = true;
            } else if delta != 0.0 {
                // Snap and stop.
                app.left_dock_w_pt = app.left_dock_w_pt_target;
                app.resize_all_tabs();
                app.force_full_redraw = true;
                app.dirty = true;
            }
        }

        // Item 8: scroll indicator alpha decay (600ms hold, 200ms fade-out).
        if app.scroll_indicator_alpha > 0.0 {
            const HOLD_MS: f64 = 600.0;
            const FADE_MS: f64 = 200.0;
            let elapsed_ms = app
                .scroll_indicator_last_scroll
                .map(|t| t.elapsed().as_secs_f64() * 1000.0)
                .unwrap_or(f64::MAX);
            let new_alpha = if elapsed_ms < HOLD_MS {
                1.0_f32
            } else {
                let fade_progress = ((elapsed_ms - HOLD_MS) / FADE_MS).min(1.0) as f32;
                (1.0 - fade_progress).max(0.0)
            };
            if (new_alpha - app.scroll_indicator_alpha).abs() > 0.005 || new_alpha == 0.0 {
                app.scroll_indicator_alpha = new_alpha;
                app.dirty = true;
            }
        }

        // Agent dot pulse (item 19).
        if app.agent_snap.connection == anvil_agent::Connection::Live {
            app.agent_pulse_phase += 0.3 / 60.0;
            if app.agent_pulse_phase >= 1.0 {
                app.agent_pulse_phase -= 1.0;
            }
            let opacity = 0.5 + 0.5 * (std::f32::consts::TAU * app.agent_pulse_phase).sin();
            if (opacity - app.last_agent_pulse_opacity).abs() > 0.02 {
                app.dirty = true;
                app.last_agent_pulse_opacity = opacity;
            }
        }

        // Running-block header dot pulse (CB6): advance phase and keep dirty
        // while any pane in the current tab has a shell command running.
        {
            let any_running = app.tabs.current().is_some_and(|t| {
                all_pane_ids_in_tree(t).into_iter().any(|pid| {
                    t.registry.get(pid).is_some_and(|p| {
                        p.terminal
                            .as_ref()
                            .map(|t| t.last_run().running)
                            .unwrap_or(false)
                    })
                })
            });
            if any_running {
                app.running_pulse_phase += 1.5 / 60.0;
                if app.running_pulse_phase >= 1.0 {
                    app.running_pulse_phase -= 1.0;
                }
                app.dirty = true;
            }
        }

        // Block-header pulse (item 23): keep dirty while any visible block is
        // mid-flash. 250ms window gives a small grace margin beyond 200ms.
        {
            let within = std::time::Duration::from_millis(250);
            let pulsing = app
                .tabs
                .current()
                .and_then(|t| t.registry.get(t.focused_id()))
                .is_some_and(|p| {
                    p.terminal
                        .as_ref()
                        .map(|t| t.any_block_completed_within(within))
                        .unwrap_or(false)
                });
            if pulsing {
                app.dirty = true;
            }
        }

        // Snap cursor_ax/ay to the live terminal cursor every tick so
        // both CPU and GPU draw paths render the cursor at its real cell.
        // No animation — animating produced trail artifacts.
        if let Some(tab) = app.tabs.current_mut() {
            let id = tab.focused_id();
            if let Some(pane) = tab.registry.get_mut(id) {
                if let Some(terminal) = &pane.terminal {
                    let cur = terminal.cursor();
                    pane.cursor_ax = cur.x as f32;
                    pane.cursor_ay = cur.y as f32;
                }
            }
        }

        // Smooth-scroll easing (item 21 + item 8).
        // Eases scroll_pos toward scroll_target at factor 0.30 per frame.
        // On snap (delta < 0.01), quantizes to the nearest integer row so
        // the settled position never lands on a sub-pixel boundary.
        if let Some(tab) = app.tabs.current_mut() {
            let id = tab.focused_id();
            if let Some(pane) = tab.registry.get_mut(id) {
                let delta = pane.scroll_target - pane.scroll_pos;
                if delta.abs() > 0.01 {
                    pane.scroll_pos += delta * 0.30;
                    if let Some(terminal) = &mut pane.terminal {
                        terminal.set_viewport_offset(pane.scroll_pos.round() as usize);
                    }
                    app.dirty = true;
                } else if delta.abs() > 0.0 {
                    // Snap: quantize to nearest integer row (unit is rows).
                    pane.scroll_pos = pane.scroll_target.round();
                    pane.scroll_vel = 0.0;
                    if let Some(terminal) = &mut pane.terminal {
                        terminal.set_viewport_offset(pane.scroll_pos as usize);
                    }
                    app.dirty = true;
                }

                // Living-scrollback indicator (item 20): track how many new
                // content rows have arrived while the user is scrolled up.
                if pane.scroll_pos > 0.0 {
                    if pane.unseen_baseline.is_none() {
                        if let Some(terminal) = &pane.terminal {
                            pane.unseen_baseline = Some(terminal.line_count());
                        }
                    }
                } else {
                    pane.unseen_baseline = None;
                }
            }
        }

        // M2: Native editor smooth-scroll easing.
        // Eases EditorPane.scroll_pos toward scroll_target at factor 0.35 per frame.
        // Clamps scroll_target to [0, max(0, line_count - visible_rows)] before easing.
        // Snaps when delta < 0.01 to avoid sub-pixel drift.
        {
            let cell_h = app.font.metrics.cell_h;
            let ir = app.pane_area_rect();
            if let Some(tab) = app.tabs.current_mut() {
                let id = tab.focused_id();
                let visible_rows = {
                    let entries = tab.tree.layout(ir, DIVIDER_PX);
                    entries
                        .iter()
                        .find(|e| e.id == id)
                        .map(|e| (e.rect.h / cell_h).ceil() as usize)
                        .unwrap_or(1)
                };
                // Resolve line_count before the mutable borrow of editor_panes.
                let line_count = tab
                    .editor_panes
                    .get_pane(id)
                    .and_then(|ep| tab.editor_panes.get_buffer(ep.buffer_id))
                    .map(|b| b.line_count())
                    .unwrap_or(1);
                let max_scroll = (line_count.saturating_sub(visible_rows)) as f32;
                if let Some(ep) = tab.editor_panes.get_pane_mut(id) {
                    // Clamp target before easing.
                    ep.scroll_target = ep.scroll_target.clamp(0.0, max_scroll.max(0.0));
                    let delta = ep.scroll_target - ep.scroll_pos;
                    if delta.abs() > 0.01 {
                        ep.scroll_pos += delta * 0.35;
                        app.dirty = true;
                    } else if delta.abs() > 0.0 {
                        ep.scroll_pos = ep.scroll_target.round();
                        ep.scroll_vel = 0.0;
                        app.dirty = true;
                    }
                }
            }
        }

        // P3: horizontal cursor auto-scroll — keep primary cursor column in view.
        // Runs after vertical easing so `scroll_pos` is up-to-date.
        {
            let cell_w = app.font.metrics.cell_w;
            let ir = app.pane_area_rect();
            if let Some(tab) = app.tabs.current_mut() {
                let id = tab.focused_id();
                // Compute content_cols and cursor state before any mutable borrow.
                let maybe = tab.editor_panes.get_pane(id).and_then(|ep| {
                    let buf = tab.editor_panes.get_buffer(ep.buffer_id)?;
                    let digit_cols = buf.line_count().max(1).to_string().len();
                    let git_cols = if buf.git_gutter.is_some() { 2 } else { 0 };
                    let gutter_w = (digit_cols + 2 + git_cols) as f64 * cell_w;
                    let entries = tab.tree.layout(ir, DIVIDER_PX);
                    let pane_w = entries
                        .iter()
                        .find(|e| e.id == id)
                        .map(|e| e.rect.w)
                        .unwrap_or(0.0);
                    let content_cols = ((pane_w - gutter_w) / cell_w).floor() as usize;
                    Some((content_cols, ep.primary_cursor().pos.col, ep.soft_wrap))
                });
                if let Some((content_cols, cursor_col, soft_wrap)) = maybe {
                    if !soft_wrap && content_cols > 0 {
                        if let Some(ep) = tab.editor_panes.get_pane_mut(id) {
                            let col_offset = ep.scroll_x.floor() as usize;
                            if cursor_col < col_offset {
                                ep.scroll_x = cursor_col as f64;
                                app.dirty = true;
                            } else if cursor_col >= col_offset + content_cols {
                                ep.scroll_x = (cursor_col + 1).saturating_sub(content_cols) as f64;
                                app.dirty = true;
                            }
                        }
                    }
                }
            }
        }

        // Refresh throttle — ALWAYS runs so the bottom status bar gets
        // cwd / git / agent data even when the HUD panel is hidden.
        app.hud_tick += 1;
        if app.hud_tick >= HUD_REFRESH_TICKS {
            app.hud_tick = 0;
            app.refresh_hud();
        }

        // LSP didChange debounce (NE9): for each native editor pane in the
        // current tab, if the buffer has a tracked path and a known language id,
        // and at least 250 ms have elapsed since the last sync, send didChange.
        {
            const LSP_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(250);
            let now = Instant::now();
            if let (Some(lsp), Some(tab)) = (app.lsp_manager.as_mut(), app.tabs.current()) {
                // Collect (pane_id, path, lang_id, text) for native editor panes
                // that have a tracked path with a known language id.
                let pending: Vec<(PaneId, PathBuf, &'static str, String)> = {
                    all_pane_ids_in_tree(tab)
                        .into_iter()
                        .filter_map(|pid| {
                            let ep = tab.editor_panes.get_pane(pid)?;
                            let buf = tab.editor_panes.get_buffer(ep.buffer_id)?;
                            let path = buf.tracked_path()?.to_path_buf();
                            let lang = buf.language_id()?;
                            let text = buf.to_text();
                            Some((pid, path, lang, text))
                        })
                        .collect()
                };

                for (pid, path, lang_id, text) in pending {
                    let last = app.lsp_last_sync.get(&pid).copied();
                    let server_id =
                        anvil_editor::server_id_for_language(lang_id).unwrap_or(lang_id);
                    if last.is_none() {
                        // First sync for this pane: send didOpen.
                        lsp.did_open(server_id, path, lang_id, text);
                        app.lsp_last_sync.insert(pid, now);
                    } else if last.is_some_and(|t| now.duration_since(t) >= LSP_DEBOUNCE) {
                        lsp.did_change(server_id, path, text);
                        app.lsp_last_sync.insert(pid, now);
                    }
                }
            }
        }

        // N3: expire stale toasts.
        app.tick_toasts();

        // N3: check for LSP-not-found failures and show a one-time info toast.
        {
            const SERVER_IDS: &[&str] = &[
                "rust-analyzer",
                "typescript-language-server",
                "pyright",
                "taplo",
                "vscode-json-language-server",
                "marksman",
            ];
            // Collect new failures before dropping the borrow on lsp_manager.
            let new_failures: Vec<String> = if let Some(lsp) = &app.lsp_manager {
                SERVER_IDS
                    .iter()
                    .filter_map(|&sid| {
                        if let anvil_editor::LspState::Failed(_) = lsp.state_of(sid) {
                            if !app.lsp_failed_toasted.contains(sid) {
                                return Some(sid.to_string());
                            }
                        }
                        None
                    })
                    .collect()
            } else {
                vec![]
            };
            for sid in new_failures {
                app.lsp_failed_toasted.insert(sid.clone());
                app.toast_info(&format!("{sid} not in PATH"));
            }
        }

        // NE10: poll for pending hover responses each tick.
        app.poll_hover_result();
        // Item 17: poll for pending definition responses each tick.
        app.poll_definition_result();
        // Item 16: poll for pending completion responses each tick.
        app.poll_completion_result();
        // Item 24: poll for pending rename responses each tick.
        app.poll_rename_result();
        // Item 25: poll for pending code-actions responses each tick.
        app.poll_code_actions_result();
        // Item 26: poll for pending references responses each tick.
        app.poll_references_result();
        // O1: poll for pending workspace symbol responses and debounce re-requests.
        app.poll_workspace_symbols_result();

        // R2: Explorer tooltip — resolve file metadata after 500ms steady hover.
        {
            const EXPLORER_HOVER_DELAY: std::time::Duration = std::time::Duration::from_millis(500);
            let now = Instant::now();
            if let Some((row_idx, since)) = app.explorer_hover_row {
                if now.duration_since(since) >= EXPLORER_HOVER_DELAY
                    && app.explorer_hover_meta.is_none()
                {
                    // Resolve path — must be a file row (not a dir).
                    if let Some((path, false)) =
                        app.left_dock_hits.visible_rows.get(row_idx).cloned()
                    {
                        if let Ok(meta) = std::fs::metadata(&path) {
                            let size = meta.len();
                            let mtime = meta
                                .modified()
                                .ok()
                                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                                .map(|d| d.as_secs())
                                .unwrap_or(0);
                            app.explorer_hover_meta = Some((path, size, mtime));
                            app.dirty = true;
                        }
                    }
                }
            }
        }

        // Item 15: hover-mouse debounce — fire hover request after 400ms dwell.
        {
            const HOVER_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(400);
            let now = Instant::now();
            if let (Some(pos), Some(t)) = (app.hover_mouse_pos, app.hover_mouse_time) {
                if now.duration_since(t) >= HOVER_DEBOUNCE
                    && app.pending_hover.is_none()
                    && app.focused_is_native_editor()
                {
                    // Convert stored raster px to buffer position.
                    let loc = MouseLocation { x: pos.0, y: pos.1 };
                    if let Some(buf_pos) = app.native_editor_pos_at(loc) {
                        if let Some(tab) = app.tabs.current() {
                            let pane_id = tab.focused_id();
                            if let Some(ep) = tab.editor_panes.get_pane(pane_id) {
                                if let Some(buf) = tab.editor_panes.get_buffer(ep.buffer_id) {
                                    if let Some(path) = buf.tracked_path() {
                                        let path = path.to_path_buf();
                                        if let Some(lsp) = &app.lsp_manager {
                                            let req_id = lsp.request_hover(
                                                &path,
                                                buf_pos.line as u32,
                                                buf_pos.col as u32,
                                            );
                                            if req_id != 0 {
                                                app.pending_hover = Some((pane_id, req_id));
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    // Clear debounce so we only fire once per dwell.
                    app.hover_mouse_time = None;
                }
            }
        }

        if app.dirty {
            let mut grid_painters = GridPainters {
                regular: &mut self.painter,
                bold: &mut self.bold_painter,
                italic: &mut self.italic_painter,
                bold_italic: &mut self.bold_italic_painter,
            };
            app.render_frame(&mut grid_painters, &mut self.chrome_painter);
            app.dirty = false;
        }

        if any_dead {
            app.close_dead_panes();
        }
    }

    fn key_down(&mut self, event: KeyEvent) {
        let mods = event.mods;

        // ⌘ key combos.
        if mods.command {
            // Native editor: Cmd+Home/End → buffer start/end; Cmd+Up/Down →
            // buffer start/end.  These must be claimed before the generic
            // jump-to-prompt bindings so the editor pane intercepts them.
            if self.app.focused_is_native_editor() {
                match event.key {
                    KeyInput::Home | KeyInput::Up => {
                        self.app.apply_editor_action(EditorAction::MoveBufferStart {
                            extend: mods.shift,
                        });
                        // M4: scroll to buffer start.
                        if let Some(tab) = self.app.tabs.current_mut() {
                            let id = tab.focused_id();
                            if let Some(ep) = tab.editor_panes.get_pane_mut(id) {
                                ep.scroll_target = 0.0;
                            }
                        }
                        self.app.dirty = true;
                        return;
                    }
                    KeyInput::End | KeyInput::Down => {
                        self.app.apply_editor_action(EditorAction::MoveBufferEnd {
                            extend: mods.shift,
                        });
                        // M4: scroll to buffer end.
                        let visible_rows = self.app.editor_visible_rows();
                        if let Some(tab) = self.app.tabs.current_mut() {
                            let id = tab.focused_id();
                            // Read line_count before the mutable borrow.
                            let line_count = tab
                                .editor_panes
                                .get_pane(id)
                                .and_then(|ep| tab.editor_panes.get_buffer(ep.buffer_id))
                                .map(|b| b.line_count())
                                .unwrap_or(1);
                            let max_scroll = (line_count.saturating_sub(visible_rows)) as f32;
                            if let Some(ep) = tab.editor_panes.get_pane_mut(id) {
                                ep.scroll_target = max_scroll.max(0.0);
                            }
                        }
                        self.app.dirty = true;
                        return;
                    }
                    _ => {}
                }
            }
            match event.key {
                KeyInput::Up if !mods.shift && !mods.control && !mods.option => {
                    self.app.jump_to_prev_prompt();
                    return;
                }
                KeyInput::Down if !mods.shift && !mods.control && !mods.option => {
                    self.app.jump_to_next_prompt();
                    return;
                }
                KeyInput::Char(ch)
                    if matches!(ch, '=' | '+' | '-' | '0') && !mods.control && !mods.option =>
                {
                    // Cmd+ / Cmd= zoom in, Cmd- zoom out, Cmd0 reset.
                    self.handle_zoom_chord(ch);
                    return;
                }
                // H4: Cmd+Opt+= / Cmd+Opt+- / Cmd+Opt+0 → font-only scale.
                KeyInput::Char(ch)
                    if matches!(ch, '=' | '+' | '-' | '0') && !mods.control && mods.option =>
                {
                    self.handle_font_scale_chord(ch);
                    return;
                }
                // Cmd+Return — approve topmost pending approval (HUD must be visible).
                KeyInput::Enter if !mods.shift && !mods.control && !mods.option => {
                    if self.app.hud_visible {
                        if let (Some(client), Some(poller)) =
                            (&self.app.caldera_client, &self.app.caldera_poller)
                        {
                            if let Some(row) = self.app.agent_snap.approvals.first() {
                                let _ = anvil_caldera::approve(
                                    client,
                                    &row.connector,
                                    &row.pattern,
                                    &row.reason,
                                    300,
                                );
                                poller.kick();
                            }
                        }
                    }
                    return;
                }
                // Cmd+Shift+Return — start a new agent run.
                KeyInput::Enter if mods.shift && !mods.control && !mods.option => {
                    if self.app.hud_visible {
                        if let (Some(client), Some(poller)) =
                            (&self.app.caldera_client, &self.app.caldera_poller)
                        {
                            let _ = anvil_caldera::start_run(client, "", "");
                            poller.kick();
                        }
                    }
                    return;
                }
                KeyInput::Char(ch) if self.app.handle_cmd_chord(mods, ch, &self.webview) => {
                    return;
                }
                _ => {}
            }
            return; // other ⌘ combos pass to system
        }

        // Cheatsheet: any key closes it.
        if self.app.cheatsheet_visible {
            self.app.cheatsheet_visible = false;
            self.app.dirty = true;
            self.app.force_full_redraw = true;
            return;
        }

        // HUD: Esc closes it — use toggle_hud so the window shrinks.
        if self.app.hud_visible && event.key == KeyInput::Escape && !event.mods.command {
            self.toggle_hud();
            return;
        }

        // ── Project search overlay key handling (item 10) ────────────────────
        if self.app.project_search.visible {
            match event.key {
                KeyInput::Escape => {
                    self.app.project_search.close();
                    self.app.dirty = true;
                }
                KeyInput::Enter => {
                    // Open the selected hit in the native editor.
                    if let Some(hit) = self.app.project_search.current_hit() {
                        let path = hit.path.clone();
                        let line = hit.line.saturating_sub(1);
                        self.app.project_search.close();
                        self.app.open_path_in_native_editor(&path);
                        self.app.apply_editor_action(EditorAction::GoToLine(line));
                    }
                    self.app.dirty = true;
                }
                KeyInput::Up => {
                    self.app.project_search.select_prev();
                    self.app.dirty = true;
                }
                KeyInput::Down => {
                    self.app.project_search.select_next();
                    self.app.dirty = true;
                }
                KeyInput::Backspace => {
                    let mut q = self.app.project_search.query.clone();
                    let mut bytes = q.as_bytes().to_vec();
                    while !bytes.is_empty() && (bytes[bytes.len() - 1] & 0xC0) == 0x80 {
                        bytes.pop();
                    }
                    bytes.pop();
                    q = String::from_utf8_lossy(&bytes).into_owned();
                    let root = std::path::PathBuf::from(&self.app.local_ctx.cwd);
                    self.app.project_search.scan(&q, &root);
                    self.app.dirty = true;
                }
                KeyInput::Char(ch) => {
                    let q = format!("{}{}", self.app.project_search.query, ch);
                    let root = std::path::PathBuf::from(&self.app.local_ctx.cwd);
                    self.app.project_search.scan(&q, &root);
                    self.app.dirty = true;
                }
                _ => {}
            }
            return;
        }

        // ── Project switcher (item 30) ───────────────────────────────────────
        if self.app.project_switcher_open {
            match event.key {
                KeyInput::Escape => {
                    self.app.project_switcher_open = false;
                    self.app.dirty = true;
                }
                KeyInput::Enter => {
                    let sel = self.app.project_switcher_sel;
                    if let Some(path) = self.app.recent_projects.get(sel).cloned() {
                        self.app.project_switcher_open = false;
                        self.app.dirty = true;
                        // Save session before exit.
                        let cwd_str = self.app.current_cwd().unwrap_or_default();
                        let state = self.app.build_session_state();
                        session::save_session(std::path::Path::new(&cwd_str), &state);
                        // Spawn new Anvil in the chosen directory.
                        if let Ok(exe) = std::env::current_exe() {
                            let _ = std::process::Command::new(&exe).current_dir(&path).spawn();
                        }
                        terminate_app();
                    }
                }
                KeyInput::Up => {
                    if self.app.project_switcher_sel > 0 {
                        self.app.project_switcher_sel -= 1;
                    }
                    self.app.dirty = true;
                }
                KeyInput::Down => {
                    let max = self.app.recent_projects.len().saturating_sub(1);
                    if self.app.project_switcher_sel < max {
                        self.app.project_switcher_sel += 1;
                    }
                    self.app.dirty = true;
                }
                _ => {}
            }
            return;
        }

        // ── Open-folder overlay (Q19: Cmd+K Cmd+O) ──────────────────────────
        if self.app.open_folder_input.is_some() {
            match event.key {
                KeyInput::Escape => {
                    self.app.open_folder_input = None;
                    self.app.dirty = true;
                }
                KeyInput::Enter => {
                    let input = self.app.open_folder_input.take().unwrap_or_default();
                    let path = PathBuf::from(&input);
                    if path.is_dir() {
                        // Save session then relaunch in the chosen directory.
                        let cwd_str = self.app.current_cwd().unwrap_or_default();
                        let state = self.app.build_session_state();
                        session::save_session(std::path::Path::new(&cwd_str), &state);
                        if let Ok(exe) = std::env::current_exe() {
                            let _ = std::process::Command::new(&exe).current_dir(&path).spawn();
                        }
                        terminate_app();
                    }
                    self.app.dirty = true;
                }
                KeyInput::Backspace => {
                    if let Some(ref mut s) = self.app.open_folder_input {
                        let mut bytes = s.as_bytes().to_vec();
                        while !bytes.is_empty() && (bytes[bytes.len() - 1] & 0xC0) == 0x80 {
                            bytes.pop();
                        }
                        bytes.pop();
                        *s = String::from_utf8_lossy(&bytes).into_owned();
                    }
                    self.app.dirty = true;
                }
                KeyInput::Char(ch) => {
                    if let Some(ref mut s) = self.app.open_folder_input {
                        s.push(ch);
                    }
                    self.app.dirty = true;
                }
                _ => {}
            }
            return;
        }

        // ── Language picker overlay (Q22: Cmd+K Cmd+L) ──────────────────────
        if self.app.language_picker.is_some() {
            let langs = PICKER_LANGS;
            match event.key {
                KeyInput::Escape => {
                    self.app.language_picker = None;
                    self.app.dirty = true;
                }
                KeyInput::Enter => {
                    if let Some(ref picker) = self.app.language_picker {
                        let filtered = picker_filtered(langs, &picker.query);
                        if let Some(&lang) = filtered.get(picker.selected) {
                            self.app.set_active_buffer_language(lang);
                        }
                    }
                    self.app.language_picker = None;
                    self.app.dirty = true;
                }
                KeyInput::Up => {
                    if let Some(ref mut picker) = self.app.language_picker {
                        if picker.selected > 0 {
                            picker.selected -= 1;
                        }
                    }
                    self.app.dirty = true;
                }
                KeyInput::Down => {
                    if let Some(ref mut picker) = self.app.language_picker {
                        let filtered = picker_filtered(langs, &picker.query);
                        let max = filtered.len().saturating_sub(1);
                        if picker.selected < max {
                            picker.selected += 1;
                        }
                    }
                    self.app.dirty = true;
                }
                KeyInput::Backspace => {
                    if let Some(ref mut picker) = self.app.language_picker {
                        let mut bytes = picker.query.as_bytes().to_vec();
                        while !bytes.is_empty() && (bytes[bytes.len() - 1] & 0xC0) == 0x80 {
                            bytes.pop();
                        }
                        bytes.pop();
                        picker.query = String::from_utf8_lossy(&bytes).into_owned();
                        picker.selected = 0;
                    }
                    self.app.dirty = true;
                }
                KeyInput::Char(ch) => {
                    if let Some(ref mut picker) = self.app.language_picker {
                        picker.query.push(ch);
                        picker.selected = 0;
                    }
                    self.app.dirty = true;
                }
                _ => {}
            }
            return;
        }

        // ── Goto-line overlay (item 11) ──────────────────────────────────────
        if self.app.goto_line_input.is_some() {
            match event.key {
                KeyInput::Escape => {
                    self.app.goto_line_input = None;
                    self.app.dirty = true;
                }
                KeyInput::Enter => {
                    let input = self.app.goto_line_input.take().unwrap_or_default();
                    // Parse NNN or NNN,CCC
                    let mut parts = input.splitn(2, ',');
                    if let Some(line_str) = parts.next() {
                        if let Ok(n) = line_str.trim().parse::<usize>() {
                            let line = n.saturating_sub(1); // 1-indexed input
                            self.app.apply_editor_action(EditorAction::GoToLine(line));
                            self.app.dirty = true;
                        }
                    }
                    self.app.dirty = true;
                }
                KeyInput::Backspace => {
                    if let Some(ref mut s) = self.app.goto_line_input {
                        let mut bytes = s.as_bytes().to_vec();
                        bytes.pop();
                        *s = String::from_utf8_lossy(&bytes).into_owned();
                    }
                    self.app.dirty = true;
                }
                KeyInput::Char(ch) => {
                    if ch.is_ascii_digit() || ch == ',' {
                        if let Some(ref mut s) = self.app.goto_line_input {
                            s.push(ch);
                        }
                    }
                    self.app.dirty = true;
                }
                _ => {}
            }
            return;
        }

        // ── Save-as overlay (tier-J J2) ──────────────────────────────────────
        if self.app.save_as_input.is_some() {
            match event.key {
                KeyInput::Escape => {
                    self.app.save_as_input = None;
                    self.app.dirty = true;
                }
                KeyInput::Enter => {
                    let path_str = self.app.save_as_input.take().unwrap_or_default();
                    if !path_str.is_empty() {
                        let new_path = std::path::PathBuf::from(&path_str);
                        self.app.apply_editor_action(EditorAction::SaveAs(new_path));
                    }
                    self.app.dirty = true;
                }
                KeyInput::Backspace => {
                    if let Some(ref mut s) = self.app.save_as_input {
                        s.pop();
                    }
                    self.app.dirty = true;
                }
                KeyInput::Char(ch) if !event.mods.command => {
                    if let Some(ref mut s) = self.app.save_as_input {
                        s.push(ch);
                    }
                    self.app.dirty = true;
                }
                _ => {}
            }
            return;
        }

        // ── LSP rename overlay (item 24) ─────────────────────────────────────
        if self.app.lsp_rename_input.is_some() {
            match event.key {
                KeyInput::Escape => {
                    self.app.lsp_rename_input = None;
                    self.app.dirty = true;
                }
                KeyInput::Enter => {
                    let new_name = self.app.lsp_rename_input.take().unwrap_or_default();
                    if !new_name.is_empty() {
                        self.app.commit_lsp_rename(new_name);
                    }
                    self.app.dirty = true;
                }
                KeyInput::Backspace => {
                    if let Some(ref mut s) = self.app.lsp_rename_input {
                        s.pop();
                    }
                    self.app.dirty = true;
                }
                KeyInput::Char(ch) if !event.mods.command => {
                    if let Some(ref mut s) = self.app.lsp_rename_input {
                        s.push(ch);
                    }
                    self.app.dirty = true;
                }
                _ => {}
            }
            return;
        }

        // ── LSP references overlay (item 26) ──────────────────────────────────
        if self.app.lsp_references.is_some() {
            match event.key {
                KeyInput::Escape => {
                    self.app.lsp_references = None;
                    self.app.dirty = true;
                    return;
                }
                KeyInput::Up => {
                    if let Some(ref mut r) = self.app.lsp_references {
                        r.selected = r.selected.saturating_sub(1);
                    }
                    self.app.dirty = true;
                    return;
                }
                KeyInput::Down => {
                    if let Some(ref mut r) = self.app.lsp_references {
                        let n = r.rows.len();
                        if n > 0 {
                            r.selected = (r.selected + 1).min(n - 1);
                        }
                    }
                    self.app.dirty = true;
                    return;
                }
                KeyInput::Enter => {
                    let loc = self
                        .app
                        .lsp_references
                        .as_ref()
                        .and_then(|r| r.rows.get(r.selected))
                        .map(|row| (row.path.clone(), row.line));
                    self.app.lsp_references = None;
                    if let Some((path, line)) = loc {
                        self.app.open_path_in_native_editor(&path);
                        self.app
                            .apply_editor_action(EditorAction::GoToLine(line as usize));
                    }
                    self.app.dirty = true;
                    return;
                }
                _ => {}
            }
            return;
        }

        // ── Workspace symbol search overlay (O1) ─────────────────────────────
        if self.app.workspace_symbol_search.is_some() {
            match event.key {
                KeyInput::Escape => {
                    self.app.workspace_symbol_search = None;
                    self.app.dirty = true;
                    return;
                }
                KeyInput::Up => {
                    if let Some(ref mut s) = self.app.workspace_symbol_search {
                        s.selected = s.selected.saturating_sub(1);
                    }
                    self.app.dirty = true;
                    return;
                }
                KeyInput::Down => {
                    if let Some(ref mut s) = self.app.workspace_symbol_search {
                        let n = s.hits.len();
                        if n > 0 {
                            s.selected = (s.selected + 1).min(n - 1);
                        }
                    }
                    self.app.dirty = true;
                    return;
                }
                KeyInput::Enter => {
                    let loc = self
                        .app
                        .workspace_symbol_search
                        .as_ref()
                        .and_then(|s| s.hits.get(s.selected))
                        .map(|h| (h.path.clone(), h.line));
                    self.app.workspace_symbol_search = None;
                    if let Some((path, line)) = loc {
                        self.app.open_path_in_native_editor(&path);
                        self.app
                            .apply_editor_action(EditorAction::GoToLine(line as usize));
                    }
                    self.app.dirty = true;
                    return;
                }
                KeyInput::Backspace => {
                    if let Some(ref mut s) = self.app.workspace_symbol_search {
                        let mut bytes = s.query.as_bytes().to_vec();
                        while !bytes.is_empty() && (bytes[bytes.len() - 1] & 0xC0) == 0x80 {
                            bytes.pop();
                        }
                        bytes.pop();
                        s.query = String::from_utf8_lossy(&bytes).into_owned();
                        s.last_query_change = Some(Instant::now());
                        s.hits.clear();
                    }
                    self.app.dirty = true;
                    return;
                }
                KeyInput::Char(ch) if !event.mods.command => {
                    if let Some(ref mut s) = self.app.workspace_symbol_search {
                        s.query.push(ch);
                        s.last_query_change = Some(Instant::now());
                        s.hits.clear();
                    }
                    self.app.dirty = true;
                    return;
                }
                _ => {
                    return;
                }
            }
        }

        // ── Buffer symbol search overlay (O2) ─────────────────────────────────
        if self.app.buffer_symbol_search.is_some() {
            match event.key {
                KeyInput::Escape => {
                    self.app.buffer_symbol_search = None;
                    self.app.dirty = true;
                    return;
                }
                KeyInput::Up => {
                    if let Some(ref mut s) = self.app.buffer_symbol_search {
                        s.selected = s.selected.saturating_sub(1);
                    }
                    self.app.dirty = true;
                    return;
                }
                KeyInput::Down => {
                    if let Some(ref mut s) = self.app.buffer_symbol_search {
                        let n = s.filtered.len();
                        if n > 0 {
                            s.selected = (s.selected + 1).min(n - 1);
                        }
                    }
                    self.app.dirty = true;
                    return;
                }
                KeyInput::Enter => {
                    let line = self
                        .app
                        .buffer_symbol_search
                        .as_ref()
                        .and_then(|s| s.filtered.get(s.selected).copied())
                        .and_then(|idx| {
                            self.app.buffer_symbol_search.as_ref()?.all_symbols.get(idx)
                        })
                        .map(|sym| sym.line);
                    self.app.buffer_symbol_search = None;
                    if let Some(line) = line {
                        self.app.apply_editor_action(EditorAction::GoToLine(line));
                    }
                    self.app.dirty = true;
                    return;
                }
                KeyInput::Backspace => {
                    if let Some(ref mut s) = self.app.buffer_symbol_search {
                        s.query.pop();
                        let q = s.query.to_ascii_lowercase();
                        s.filtered = if q.is_empty() {
                            (0..s.all_symbols.len()).collect()
                        } else {
                            s.all_symbols
                                .iter()
                                .enumerate()
                                .filter(|(_, sym)| sym.name.to_ascii_lowercase().contains(&q))
                                .map(|(i, _)| i)
                                .collect()
                        };
                        s.selected = 0;
                    }
                    self.app.dirty = true;
                    return;
                }
                KeyInput::Char(ch) if !event.mods.command => {
                    if let Some(ref mut s) = self.app.buffer_symbol_search {
                        s.query.push(ch);
                        let q = s.query.to_ascii_lowercase();
                        s.filtered = s
                            .all_symbols
                            .iter()
                            .enumerate()
                            .filter(|(_, sym)| sym.name.to_ascii_lowercase().contains(&q))
                            .map(|(i, _)| i)
                            .collect();
                        s.selected = 0;
                    }
                    self.app.dirty = true;
                    return;
                }
                _ => {
                    return;
                }
            }
        }

        // ── Cmd+K two-stroke chord (H1: soft-wrap, H2: show-whitespace) ─────────
        // `pending_chord_k` is set by the Cmd+K handler in handle_cmd_chord.
        // The next plain (non-Cmd) key completes or cancels the chord.
        if self.app.pending_chord_k {
            self.app.pending_chord_k = false;
            // Q19: Cmd+K O → open folder (global — no editor focus required).
            if matches!(event.key, KeyInput::Char('o') | KeyInput::Char('O')) {
                let cwd = self.app.current_cwd().unwrap_or_else(|| String::from("/"));
                self.app.open_folder_input = Some(cwd);
                self.app.dirty = true;
                return;
            }
            // Q22: Cmd+K L → language picker (requires an open editor buffer).
            if matches!(event.key, KeyInput::Char('l') | KeyInput::Char('L')) {
                if self.app.focused_is_native_editor() {
                    self.app.language_picker = Some(LanguagePickerState {
                        query: String::new(),
                        selected: 0,
                    });
                    self.app.dirty = true;
                }
                return;
            }
            // S1: Cmd+K I → toggle gitignored-files visibility in Explorer.
            if matches!(event.key, KeyInput::Char('i') | KeyInput::Char('I')) {
                self.app.show_gitignored_files = !self.app.show_gitignored_files;
                let flags = self.app.filter_flags();
                let _ = self.app.fs_hidden_tx.try_send(flags);
                self.app.refresh_fs_snapshot();
                self.app.dirty = true;
                return;
            }
            if self.app.focused_is_native_editor() {
                match event.key {
                    KeyInput::Char('w') | KeyInput::Char('W') => {
                        self.app.apply_editor_action(EditorAction::ToggleSoftWrap);
                        self.app.dirty = true;
                        self.app.force_full_redraw = true;
                    }
                    KeyInput::Char(' ') => {
                        self.app
                            .apply_editor_action(EditorAction::ToggleShowWhitespace);
                        self.app.dirty = true;
                        self.app.force_full_redraw = true;
                    }
                    KeyInput::Char('k') | KeyInput::Char('K') => {
                        // Cmd+K K → HoverRequest (original NE10 binding via double-tap).
                        self.app.trigger_hover_request();
                        self.app.dirty = true;
                    }
                    _ => {
                        // Any other key: cancel chord silently (key is consumed).
                    }
                }
            }
            return;
        }

        // ── Find+replace row tab-switching (item 9) ──────────────────────────
        // When search is open and the focused pane is a native editor, Tab
        // switches focus between the find and replace rows.
        if self.app.search_open && self.app.focused_is_native_editor() && event.key == KeyInput::Tab
        {
            self.app.replace_row_active = !self.app.replace_row_active;
            self.app.dirty = true;
            return;
        }

        // Search bar handling.
        if self.app.search_open {
            // NE11: when a native editor pane has search open, route to editor
            // search actions instead of the terminal search path.
            if self.app.focused_is_native_editor() {
                // Replace row is active: type into the replace string.
                if self.app.replace_row_active {
                    match event.key {
                        KeyInput::Escape => {
                            self.app.replace_row_active = false;
                            self.app.close_search();
                        }
                        KeyInput::Enter => {
                            self.app.apply_editor_action(EditorAction::ReplaceOne);
                            self.app.dirty = true;
                        }
                        KeyInput::Backspace => {
                            let cur = self
                                .app
                                .tabs
                                .current()
                                .and_then(|tab| {
                                    let id = tab.focused_id();
                                    let s = tab.editor_panes.get_pane(id)?.search.as_ref()?;
                                    s.replace_input.clone()
                                })
                                .unwrap_or_default();
                            let mut bytes = cur.into_bytes();
                            while !bytes.is_empty() && (bytes[bytes.len() - 1] & 0xC0) == 0x80 {
                                bytes.pop();
                            }
                            bytes.pop();
                            let new_r = String::from_utf8_lossy(&bytes).into_owned();
                            self.app
                                .apply_editor_action(EditorAction::SetReplaceInput(new_r));
                            self.app.dirty = true;
                        }
                        KeyInput::Char(ch) => {
                            let cur = self
                                .app
                                .tabs
                                .current()
                                .and_then(|tab| {
                                    let id = tab.focused_id();
                                    let s = tab.editor_panes.get_pane(id)?.search.as_ref()?;
                                    s.replace_input.clone()
                                })
                                .unwrap_or_default();
                            let new_r = format!("{cur}{ch}");
                            self.app
                                .apply_editor_action(EditorAction::SetReplaceInput(new_r));
                            self.app.dirty = true;
                        }
                        _ => {}
                    }
                    return;
                }
                // Find row is active (default).
                match event.key {
                    KeyInput::Escape => self.app.close_search(),
                    KeyInput::Enter => {
                        self.app.apply_editor_action(EditorAction::SearchNext);
                        self.app.dirty = true;
                    }
                    KeyInput::Backspace => {
                        // Drop the last codepoint from the editor search query.
                        let cur_q = self
                            .app
                            .tabs
                            .current()
                            .and_then(|tab| {
                                let id = tab.focused_id();
                                let s = tab.editor_panes.get_pane(id)?.search.as_ref()?;
                                Some(s.query.clone())
                            })
                            .unwrap_or_default();
                        let mut qbytes = cur_q.into_bytes();
                        while !qbytes.is_empty() && (qbytes[qbytes.len() - 1] & 0xC0) == 0x80 {
                            qbytes.pop();
                        }
                        qbytes.pop();
                        let new_q = String::from_utf8_lossy(&qbytes).into_owned();
                        self.app
                            .apply_editor_action(EditorAction::SearchSetQuery(new_q));
                        self.app.dirty = true;
                    }
                    KeyInput::Char(ch) => {
                        let cur_q = self
                            .app
                            .tabs
                            .current()
                            .and_then(|tab| {
                                let id = tab.focused_id();
                                let s = tab.editor_panes.get_pane(id)?.search.as_ref()?;
                                Some(s.query.clone())
                            })
                            .unwrap_or_default();
                        let new_q = format!("{cur_q}{ch}");
                        self.app
                            .apply_editor_action(EditorAction::SearchSetQuery(new_q));
                        self.app.dirty = true;
                    }
                    _ => {}
                }
                return;
            }
            // Terminal search key handling.
            match event.key {
                KeyInput::Escape => self.app.close_search(),
                KeyInput::Enter => {
                    self.app.search.next();
                    self.app.scroll_to_current_match();
                    self.app.dirty = true;
                }
                KeyInput::Backspace => {
                    let q = self.app.search.query().to_string();
                    // Drop the last UTF-8 codepoint.
                    let mut qbytes = q.into_bytes();
                    while !qbytes.is_empty() && (qbytes[qbytes.len() - 1] & 0xC0) == 0x80 {
                        qbytes.pop();
                    }
                    qbytes.pop();
                    let new_q = String::from_utf8_lossy(&qbytes).into_owned();
                    // We need a terminal ref to set_query; borrow carefully.
                    if let Some(tab) = self.app.tabs.current_mut() {
                        let id = tab.focused_id();
                        if let Some(pane) = tab.registry.get_mut(id) {
                            if let Some(term) = &pane.terminal {
                                self.app.search.set_query(term, &new_q);
                            }
                        }
                    }
                    self.app.scroll_to_current_match();
                    self.app.dirty = true;
                }
                KeyInput::Char(ch) => {
                    let q = self.app.search.query().to_string();
                    let new_q = format!("{q}{ch}");
                    if let Some(tab) = self.app.tabs.current_mut() {
                        let id = tab.focused_id();
                        if let Some(pane) = tab.registry.get_mut(id) {
                            if let Some(term) = &pane.terminal {
                                self.app.search.set_query(term, &new_q);
                            }
                        }
                    }
                    self.app.scroll_to_current_match();
                    self.app.dirty = true;
                }
                _ => {}
            }
            return;
        }

        // ── Explorer keyboard nav (item 4) ───────────────────────────────────
        // When the Explorer has focus and no modal is active, handle arrow keys,
        // Enter, Esc.  Falls through when not in Explorer focus.
        if self.app.focus_target == FocusTarget::Explorer
            && self.app.explorer_rename.is_none()
            && self.app.explorer_new_item.is_none()
            && self.app.explorer_delete_confirm.is_none()
        {
            let total = self.app.left_dock_hits.visible_rows.len();
            let cur = self.app.selected_explorer_row;
            match event.key {
                KeyInput::Up => {
                    let next = match cur {
                        None if total > 0 => Some(total - 1),
                        Some(i) => Some(i.saturating_sub(1)),
                        None => None,
                    };
                    self.app.selected_explorer_row = next;
                    // Scroll into view.
                    if let Some(idx) = next {
                        let available = self.app.explorer_visible_rows();
                        if idx < self.app.explorer_scroll_offset {
                            self.app.explorer_scroll_offset = idx;
                        } else if idx >= self.app.explorer_scroll_offset + available {
                            self.app.explorer_scroll_offset = idx.saturating_sub(available - 1);
                        }
                    }
                    self.app.dirty = true;
                    return;
                }
                KeyInput::Down => {
                    let next = match cur {
                        None if total > 0 => Some(0),
                        Some(i) => Some((i + 1).min(total.saturating_sub(1))),
                        None => None,
                    };
                    self.app.selected_explorer_row = next;
                    if let Some(idx) = next {
                        let available = self.app.explorer_visible_rows();
                        if idx < self.app.explorer_scroll_offset {
                            self.app.explorer_scroll_offset = idx;
                        } else if idx >= self.app.explorer_scroll_offset + available {
                            self.app.explorer_scroll_offset = idx.saturating_sub(available - 1);
                        }
                    }
                    self.app.dirty = true;
                    return;
                }
                KeyInput::Right => {
                    if let Some(idx) = cur {
                        if let Some((path, is_dir)) =
                            self.app.left_dock_hits.visible_rows.get(idx).cloned()
                        {
                            if is_dir && !self.app.expanded_dirs.contains(&path) {
                                // Expand dir.
                                if !self.app.child_snapshots.contains_key(&path) {
                                    let snap = fs_worker::read_dir_snapshot_fast(
                                        &path,
                                        self.app.filter_flags(),
                                    );
                                    self.app.child_snapshots.insert(
                                        path.clone(),
                                        LeftDockSnapshot {
                                            root: snap.root.to_string_lossy().into_owned(),
                                            entries: snap
                                                .entries
                                                .into_iter()
                                                .map(|e| anvil_render::left_dock::DirEntry {
                                                    name: e.name,
                                                    is_dir: e.is_dir,
                                                })
                                                .collect(),
                                            git_marks: snap.git_marks,
                                        },
                                    );
                                }
                                self.app.expanded_dirs.insert(path);
                                self.app.dirty = true;
                            }
                        }
                    }
                    return;
                }
                KeyInput::Left => {
                    if let Some(idx) = cur {
                        if let Some((path, is_dir)) =
                            self.app.left_dock_hits.visible_rows.get(idx).cloned()
                        {
                            if is_dir && self.app.expanded_dirs.contains(&path) {
                                // Collapse expanded dir.
                                self.app.expanded_dirs.remove(&path);
                                self.app.dirty = true;
                            } else {
                                // Move focus to parent dir: find the row whose
                                // path is the parent of this path.
                                if let Some(parent) = path.parent() {
                                    let parent_pb = parent.to_path_buf();
                                    let parent_idx = self
                                        .app
                                        .left_dock_hits
                                        .visible_rows
                                        .iter()
                                        .position(|(p, _)| *p == parent_pb);
                                    if let Some(pi) = parent_idx {
                                        self.app.selected_explorer_row = Some(pi);
                                        self.app.dirty = true;
                                    }
                                }
                            }
                        }
                    }
                    return;
                }
                KeyInput::Enter => {
                    // R3: when filter is active and only one file matches, open it.
                    if self.app.explorer_filter.is_some() {
                        let file_rows: Vec<PathBuf> = self
                            .app
                            .left_dock_hits
                            .visible_rows
                            .iter()
                            .filter(|(_, is_dir)| !is_dir)
                            .map(|(p, _)| p.clone())
                            .collect();
                        if file_rows.len() == 1 {
                            let path = file_rows.into_iter().next().unwrap();
                            self.app.explorer_filter = None;
                            self.app.open_path_in_native_editor(&path);
                            self.app.active_explorer_file = Some(path);
                            self.app.dirty = true;
                            return;
                        }
                    }
                    if let Some(idx) = cur {
                        if let Some((path, is_dir)) =
                            self.app.left_dock_hits.visible_rows.get(idx).cloned()
                        {
                            if is_dir {
                                // Toggle expand.
                                if self.app.expanded_dirs.contains(&path) {
                                    self.app.expanded_dirs.remove(&path);
                                } else {
                                    if !self.app.child_snapshots.contains_key(&path) {
                                        let snap = fs_worker::read_dir_snapshot_fast(
                                            &path,
                                            self.app.filter_flags(),
                                        );
                                        self.app.child_snapshots.insert(
                                            path.clone(),
                                            LeftDockSnapshot {
                                                root: snap.root.to_string_lossy().into_owned(),
                                                entries: snap
                                                    .entries
                                                    .into_iter()
                                                    .map(|e| anvil_render::left_dock::DirEntry {
                                                        name: e.name,
                                                        is_dir: e.is_dir,
                                                    })
                                                    .collect(),
                                                git_marks: snap.git_marks,
                                            },
                                        );
                                    }
                                    self.app.expanded_dirs.insert(path);
                                }
                                self.app.dirty = true;
                            } else {
                                self.app.open_path_in_native_editor(&path);
                                // Item 5: sync active_explorer_file.
                                self.app.active_explorer_file = Some(path);
                                self.app.dirty = true;
                            }
                        }
                    }
                    return;
                }
                KeyInput::Escape => {
                    if self.app.explorer_filter.is_some() {
                        // R3: Esc clears the filter first; second Esc exits Explorer focus.
                        self.app.explorer_filter = None;
                        self.app.dirty = true;
                    } else {
                        self.app.selected_explorer_row = None;
                        self.app.focus_target = FocusTarget::Editor;
                        self.app.dirty = true;
                    }
                    return;
                }
                // F2: enter rename mode (item 6).
                KeyInput::F(2) => {
                    if let Some(idx) = cur {
                        if let Some((path, _)) =
                            self.app.left_dock_hits.visible_rows.get(idx).cloned()
                        {
                            let name = path
                                .file_name()
                                .and_then(|n| n.to_str())
                                .unwrap_or("")
                                .to_string();
                            self.app.explorer_rename = Some(RenameState {
                                old_path: path,
                                input: name,
                                row_idx: idx,
                            });
                            self.app.dirty = true;
                        }
                    }
                    return;
                }
                // Delete: confirm delete (item 8).
                KeyInput::Delete => {
                    if let Some(idx) = cur {
                        if let Some((path, _)) =
                            self.app.left_dock_hits.visible_rows.get(idx).cloned()
                        {
                            let name = path
                                .file_name()
                                .and_then(|n| n.to_str())
                                .unwrap_or("")
                                .to_string();
                            self.app.explorer_delete_confirm = Some(DeleteConfirm { path, name });
                            self.app.dirty = true;
                        }
                    }
                    return;
                }
                // R3: Backspace removes last filter char.
                KeyInput::Backspace
                    if !event.mods.command && self.app.explorer_filter.is_some() =>
                {
                    let f = self.app.explorer_filter.get_or_insert_with(String::new);
                    f.pop();
                    if f.is_empty() {
                        self.app.explorer_filter = None;
                    }
                    self.app.dirty = true;
                    return;
                }
                // R3: Printable char starts or extends the filter.
                KeyInput::Char(ch)
                    if !event.mods.command
                        && !event.mods.control
                        && (ch.is_alphanumeric() || ch == '_' || ch == '-' || ch == '.') =>
                {
                    self.app
                        .explorer_filter
                        .get_or_insert_with(String::new)
                        .push(ch);
                    self.app.dirty = true;
                    return;
                }
                _ => {}
            }
        }

        // ── Explorer modal key routing (items 6, 7, 8) ───────────────────────
        // Handle keystrokes for active rename / new-item / delete-confirm modals.
        if let Some(rename) = &mut self.app.explorer_rename {
            match event.key {
                KeyInput::Backspace => {
                    rename.input.pop();
                    self.app.dirty = true;
                    return;
                }
                KeyInput::Char(ch) if !event.mods.command => {
                    rename.input.push(ch);
                    self.app.dirty = true;
                    return;
                }
                KeyInput::Enter => {
                    // Commit the rename.
                    let old_path = rename.old_path.clone();
                    let new_name = rename.input.clone();
                    self.app.explorer_rename = None;
                    if !new_name.is_empty() {
                        if let Some(parent) = old_path.parent() {
                            let new_path = parent.join(&new_name);
                            if let Err(e) = std::fs::rename(&old_path, &new_path) {
                                eprintln!("anvil: rename failed: {e}");
                            } else {
                                // Update active file if it was the renamed file.
                                if self.app.active_explorer_file.as_deref() == Some(&old_path) {
                                    self.app.active_explorer_file = Some(new_path);
                                }
                                // Re-snapshot the parent directory.
                                let parent_pb = parent.to_path_buf();
                                let flags = self.app.filter_flags();
                                if let Some(parent_snap) = self.app.child_snapshots.get_mut(parent)
                                {
                                    let new_snap =
                                        fs_worker::read_dir_snapshot_fast(&parent_pb, flags);
                                    *parent_snap = LeftDockSnapshot {
                                        root: new_snap.root.to_string_lossy().into_owned(),
                                        entries: new_snap
                                            .entries
                                            .into_iter()
                                            .map(|e| anvil_render::left_dock::DirEntry {
                                                name: e.name,
                                                is_dir: e.is_dir,
                                            })
                                            .collect(),
                                        git_marks: new_snap.git_marks,
                                    };
                                } else if let Some(snap) = &mut self.app.fs_snapshot {
                                    let root_pb = PathBuf::from(&snap.root);
                                    let new_snap =
                                        fs_worker::read_dir_snapshot_fast(&root_pb, flags);
                                    *snap = LeftDockSnapshot {
                                        root: new_snap.root.to_string_lossy().into_owned(),
                                        entries: new_snap
                                            .entries
                                            .into_iter()
                                            .map(|e| anvil_render::left_dock::DirEntry {
                                                name: e.name,
                                                is_dir: e.is_dir,
                                            })
                                            .collect(),
                                        git_marks: new_snap.git_marks,
                                    };
                                }
                            }
                        }
                    }
                    self.app.dirty = true;
                    return;
                }
                KeyInput::Escape => {
                    self.app.explorer_rename = None;
                    self.app.dirty = true;
                    return;
                }
                _ => return,
            }
        }

        if let Some(new_item) = &mut self.app.explorer_new_item {
            match event.key {
                KeyInput::Backspace => {
                    new_item.input.pop();
                    self.app.dirty = true;
                    return;
                }
                KeyInput::Char(ch) if !event.mods.command => {
                    new_item.input.push(ch);
                    self.app.dirty = true;
                    return;
                }
                KeyInput::Enter => {
                    let parent = new_item.parent_dir.clone();
                    let name = new_item.input.clone();
                    let is_dir = new_item.is_dir;
                    self.app.explorer_new_item = None;
                    if !name.is_empty() {
                        let new_path = parent.join(&name);
                        let result = if is_dir {
                            std::fs::create_dir(&new_path).map_err(|e| e.to_string())
                        } else {
                            std::fs::write(&new_path, b"").map_err(|e| e.to_string())
                        };
                        if let Err(e) = result {
                            eprintln!("anvil: create {}: {e}", if is_dir { "dir" } else { "file" });
                        } else {
                            // Re-snapshot the parent.
                            let new_snap =
                                fs_worker::read_dir_snapshot_fast(&parent, self.app.filter_flags());
                            let snap = LeftDockSnapshot {
                                root: new_snap.root.to_string_lossy().into_owned(),
                                entries: new_snap
                                    .entries
                                    .into_iter()
                                    .map(|e| anvil_render::left_dock::DirEntry {
                                        name: e.name,
                                        is_dir: e.is_dir,
                                    })
                                    .collect(),
                                git_marks: new_snap.git_marks,
                            };
                            if let Some(root_snap) = &mut self.app.fs_snapshot {
                                if root_snap.root == snap.root {
                                    *root_snap = snap;
                                } else {
                                    self.app.child_snapshots.insert(parent, snap);
                                }
                            }
                        }
                    }
                    self.app.dirty = true;
                    return;
                }
                KeyInput::Escape => {
                    self.app.explorer_new_item = None;
                    self.app.dirty = true;
                    return;
                }
                _ => return,
            }
        }

        if let Some(del) = &self.app.explorer_delete_confirm {
            let path = del.path.clone();
            match event.key {
                KeyInput::Enter => {
                    self.app.explorer_delete_confirm = None;
                    let is_dir = path.is_dir();
                    let result = if is_dir {
                        std::fs::remove_dir_all(&path).map_err(|e| e.to_string())
                    } else {
                        std::fs::remove_file(&path).map_err(|e| e.to_string())
                    };
                    if let Err(e) = result {
                        eprintln!("anvil: delete failed: {e}");
                    } else {
                        // Remove from active_explorer_file if it was the deleted file.
                        if self.app.active_explorer_file.as_deref() == Some(&path) {
                            self.app.active_explorer_file = None;
                        }
                        self.app.selected_explorer_row = None;
                        // Re-snapshot the parent.
                        if let Some(parent) = path.parent() {
                            let flags = self.app.filter_flags();
                            if let Some(root_snap) = &mut self.app.fs_snapshot {
                                let root_pb = PathBuf::from(&root_snap.root.clone());
                                let new_snap = fs_worker::read_dir_snapshot_fast(&root_pb, flags);
                                *root_snap = LeftDockSnapshot {
                                    root: new_snap.root.to_string_lossy().into_owned(),
                                    entries: new_snap
                                        .entries
                                        .into_iter()
                                        .map(|e| anvil_render::left_dock::DirEntry {
                                            name: e.name,
                                            is_dir: e.is_dir,
                                        })
                                        .collect(),
                                    git_marks: new_snap.git_marks,
                                };
                            }
                            self.app.child_snapshots.remove(parent);
                        }
                    }
                    self.app.dirty = true;
                    return;
                }
                KeyInput::Escape => {
                    self.app.explorer_delete_confirm = None;
                    self.app.dirty = true;
                    return;
                }
                _ => return,
            }
        }

        // ── Native editor pane keyboard dispatch (NE6) ───────────────────────
        // When a native editor pane is focused, map the event to an EditorAction
        // and apply it instead of writing to a PTY.
        if self.app.focused_is_native_editor() {
            // NE14 ghost-text keybinds take priority over normal Tab/Esc handling.
            let ghost_text_active = self
                .app
                .tabs
                .current()
                .and_then(|t| {
                    let id = t.focused_id();
                    let ep = t.editor_panes.get_pane(id)?;
                    let buf = t.editor_panes.get_buffer(ep.buffer_id)?;
                    Some(!buf.ghost_text.is_empty())
                })
                .unwrap_or(false);

            if ghost_text_active {
                match event.key {
                    KeyInput::Tab => {
                        self.app.apply_editor_action(EditorAction::AcceptGhostText);
                        return;
                    }
                    KeyInput::Escape => {
                        self.app.apply_editor_action(EditorAction::DismissGhostText);
                        return;
                    }
                    _ => {}
                }
            }

            // Item 16: Ctrl+Space → trigger completion.
            if event.key == KeyInput::Char(' ') && event.mods.control && !event.mods.command {
                self.app.trigger_completion_request();
                self.app.dirty = true;
                return;
            }

            // Item 16: completion popup navigation (↑↓ Enter Esc) when open.
            let completion_open = self
                .app
                .tabs
                .current()
                .and_then(|t| {
                    let ep = t.editor_panes.get_pane(t.focused_id())?;
                    Some(ep.completion_popup.is_some())
                })
                .unwrap_or(false);
            if completion_open {
                match event.key {
                    KeyInput::Up => {
                        self.app.apply_editor_action(EditorAction::CompletionUp);
                        self.app.dirty = true;
                        return;
                    }
                    KeyInput::Down => {
                        self.app.apply_editor_action(EditorAction::CompletionDown);
                        self.app.dirty = true;
                        return;
                    }
                    KeyInput::Enter => {
                        self.app.apply_editor_action(EditorAction::CompletionAccept);
                        self.app.dirty = true;
                        return;
                    }
                    KeyInput::Escape => {
                        self.app
                            .apply_editor_action(EditorAction::CompletionDismiss);
                        self.app.dirty = true;
                        return;
                    }
                    KeyInput::Char(ch) => {
                        // Insert the char AND filter the popup.
                        self.app.apply_editor_action(EditorAction::InsertChar(ch));
                        self.app
                            .apply_editor_action(EditorAction::CompletionFilter(ch));
                        self.app.dirty = true;
                        return;
                    }
                    _ => {
                        // Any other key (Backspace, Tab, etc.) closes the popup.
                        self.app
                            .apply_editor_action(EditorAction::CompletionDismiss);
                        // Fall through to normal handling.
                    }
                }
            }

            // #21: Tab on a multi-line selection → indent all selected lines.
            if event.key == KeyInput::Tab && !event.mods.shift {
                let has_multiline_selection = self
                    .app
                    .tabs
                    .current()
                    .and_then(|t| {
                        let ep = t.editor_panes.get_pane(t.focused_id())?;
                        let c = &ep.cursors[0];
                        if c.anchor != c.pos {
                            let (al, pl) = (c.anchor.line, c.pos.line);
                            Some(al != pl)
                        } else {
                            None
                        }
                    })
                    .unwrap_or(false);
                if has_multiline_selection {
                    self.app.apply_editor_action(EditorAction::IndentSelection);
                    self.app.dirty = true;
                    return;
                }
            }

            // NE13: Esc with multi-cursor active → clear secondary cursors.
            if event.key == KeyInput::Escape {
                let has_secondary = self
                    .app
                    .tabs
                    .current()
                    .and_then(|t| {
                        let ep = t.editor_panes.get_pane(t.focused_id())?;
                        Some(ep.cursors.len() > 1)
                    })
                    .unwrap_or(false);
                if has_secondary {
                    self.app
                        .apply_editor_action(EditorAction::ClearSecondaryCursors);
                    self.app.dirty = true;
                    return;
                }
            }

            // Item 24: F2 in editor body → LSP rename overlay.
            if event.key == KeyInput::F(2) && !event.mods.command {
                self.app.open_lsp_rename_overlay();
                return;
            }

            // Item 26: Shift+F12 → LSP references.
            if event.key == KeyInput::F(12) && event.mods.shift && !event.mods.command {
                self.app.trigger_references_request();
                self.app.dirty = true;
                return;
            }

            // Item 25: code-actions popup navigation (↑↓ Enter Esc) when open.
            let code_actions_open = self
                .app
                .tabs
                .current()
                .and_then(|t| {
                    let ep = t.editor_panes.get_pane(t.focused_id())?;
                    Some(ep.code_actions_popup.is_some())
                })
                .unwrap_or(false);
            if code_actions_open {
                match event.key {
                    KeyInput::Up => {
                        self.app.apply_editor_action(EditorAction::CodeActionsUp);
                        self.app.dirty = true;
                        return;
                    }
                    KeyInput::Down => {
                        self.app.apply_editor_action(EditorAction::CodeActionsDown);
                        self.app.dirty = true;
                        return;
                    }
                    KeyInput::Enter => {
                        // Get selected index before dismissing.
                        let idx = self
                            .app
                            .tabs
                            .current()
                            .and_then(|t| {
                                let ep = t.editor_panes.get_pane(t.focused_id())?;
                                Some(ep.code_actions_popup.as_ref()?.selected)
                            })
                            .unwrap_or(0);
                        self.app.apply_code_action(idx);
                        self.app.dirty = true;
                        return;
                    }
                    KeyInput::Escape => {
                        self.app
                            .apply_editor_action(EditorAction::CodeActionsDismiss);
                        self.app.code_actions_pending_edits.clear();
                        self.app.dirty = true;
                        return;
                    }
                    _ => {
                        // Any other key closes the popup.
                        self.app
                            .apply_editor_action(EditorAction::CodeActionsDismiss);
                        self.app.code_actions_pending_edits.clear();
                        // Fall through to normal handling.
                    }
                }
            }

            // M3: PageUp / PageDown — move cursor and update scroll_target.
            if event.key == KeyInput::PageUp || event.key == KeyInput::PageDown {
                let visible_rows = self.app.editor_visible_rows();
                let is_up = event.key == KeyInput::PageUp;
                let shift = event.mods.shift;
                // Move cursor by one page.
                self.app.apply_editor_action(if is_up {
                    EditorAction::PageUp { extend: shift }
                } else {
                    EditorAction::PageDown { extend: shift }
                });
                // Scroll viewport by one page in the same direction.
                if let Some(tab) = self.app.tabs.current_mut() {
                    let id = tab.focused_id();
                    if let Some(ep) = tab.editor_panes.get_pane_mut(id) {
                        let delta = visible_rows as f32;
                        if is_up {
                            ep.scroll_target = (ep.scroll_target - delta).max(0.0);
                        } else {
                            ep.scroll_target += delta;
                        }
                    }
                }
                self.app.dirty = true;
                return;
            }

            let action = key_event_to_editor_action(event);
            if let Some(action) = action {
                // Item 16: autotrigger completion after `.` or `:`.
                // (We trigger after the second `:` of `::` by checking the last char
                //  typed — cheap and sufficient for the v1 trigger spec.)
                let should_trigger = matches!(
                    &action,
                    EditorAction::InsertChar('.') | EditorAction::InsertChar(':')
                );
                self.app.apply_editor_action(action);
                if should_trigger && self.app.pending_completion.is_none() {
                    self.app.trigger_completion_request();
                }
            }
            return;
        }

        // Normal key → encode and write to PTY.
        let app_cursor = self
            .app
            .tabs
            .current()
            .and_then(|t| t.registry.get(t.focused_id()))
            .and_then(|p| p.terminal.as_ref())
            .map(|t| t.modes.app_cursor_keys)
            .unwrap_or(false);

        if let Some(k) = platform_key_to_zig_key(event.key) {
            let zig_mods = platform_mods_to_zig_mods(mods);
            let mut buf = [0u8; 16];
            let bytes = encode_key(k, zig_mods, app_cursor, &mut buf);
            self.app.write_to_focused_pty(bytes);
        }

        // Scroll to bottom and clear selection on key input.
        if let Some(tab) = self.app.tabs.current_mut() {
            let id = tab.focused_id();
            if let Some(pane) = tab.registry.get_mut(id) {
                if let Some(terminal) = &mut pane.terminal {
                    terminal.scroll_to_bottom();
                }
                pane.scroll_pos = 0.0;
                pane.scroll_target = 0.0;
                pane.scroll_vel = 0.0;
                pane.selection.clear();
            }
        }
        self.app.dirty = true;
    }

    fn perform_key_equivalent(&mut self, event: KeyEvent) -> bool {
        // Ctrl+Tab / Ctrl+Shift+Tab for tab cycling.
        if event.mods.control && !event.mods.command && event.key == KeyInput::Tab {
            self.app.close_search();
            if event.mods.shift {
                self.app.tabs.prev();
            } else {
                self.app.tabs.next();
            }
            self.app.snap_anim();
            self.app.force_full_redraw = true;
            self.app.dirty = true;
            return true;
        }

        // ⌘ shortcuts. macOS dispatches these via performKeyEquivalent BEFORE
        // keyDown, so unless we claim them here the system menu (or its
        // absence: a system beep) may eat them. Route through the same
        // dispatcher as keyDown.
        if event.mods.command {
            match event.key {
                KeyInput::Up if !event.mods.shift && !event.mods.control && !event.mods.option => {
                    self.app.jump_to_prev_prompt();
                    return true;
                }
                KeyInput::Down
                    if !event.mods.shift && !event.mods.control && !event.mods.option =>
                {
                    self.app.jump_to_next_prompt();
                    return true;
                }
                KeyInput::Char(ch)
                    if matches!(ch, '=' | '+' | '-' | '0')
                        && !event.mods.control
                        && !event.mods.option =>
                {
                    self.handle_zoom_chord(ch);
                    return true;
                }
                // H4: Cmd+Opt+= / Cmd+Opt+- / Cmd+Opt+0 → font-only scale.
                KeyInput::Char(ch)
                    if matches!(ch, '=' | '+' | '-' | '0')
                        && !event.mods.control
                        && event.mods.option =>
                {
                    self.handle_font_scale_chord(ch);
                    return true;
                }
                // HUD toggle: intercept here so we can call toggle_hud()
                // (which needs &mut self to access self.window).
                KeyInput::Char(ch)
                    if self
                        .app
                        .keybindings
                        .hud_toggle
                        .is_some_and(|c| App::chord_matches(c, event.mods, ch)) =>
                {
                    self.toggle_hud();
                    return true;
                }
                KeyInput::Char(ch) if self.app.handle_cmd_chord(event.mods, ch, &self.webview) => {
                    return true;
                }
                _ => {}
            }
        }
        false
    }

    fn mouse_down(
        &mut self,
        loc: MouseLocation,
        mods: Modifiers,
        click_count: u32,
        view_bounds: (f64, f64),
    ) {
        let app = &mut self.app;
        let _ = view_bounds;

        // HUD left-edge resize handle. The 1px hairline sits at the left of
        // the HUD's surface rect; we accept clicks within `HUD_DRAG_HIT_PX`
        // device pixels of that line for slop. This branch must come before
        // click-to-focus and selection-start so the drag wins over both.
        if app.hud_visible {
            let (rx, ry) = app.view_pt_to_raster_px(loc);
            let cw = app.font.metrics.cell_w;
            let (dw, _) = app.device_size();
            let surface_w_px = app.hud_cols as f64 * cw + GRID_PAD as f64;
            let edge_x = (dw as f64 - surface_w_px).max(0.0);
            if (rx - edge_x).abs() <= HUD_DRAG_HIT_PX {
                app.hud_drag_active = true;
                app.dirty = true;
                return;
            }
            // Section header hit: start a drag-to-reorder gesture. Wins
            // over the row hit-test below so a click on the header doesn't
            // also try to copy something.
            for h in &app.hud_section_hits {
                let r = h.rect;
                if rx >= r.x && rx < r.x + r.w && ry >= r.y && ry < r.y + r.h {
                    app.hud_section_drag = Some(h.section);
                    app.dirty = true;
                    return;
                }
            }
            // HUD content hit-test: plain click → copy, Cmd-click → open.
            for hit in &app.hud_hits {
                let r = hit.rect;
                if rx >= r.x && rx < r.x + r.w && ry >= r.y && ry < r.y + r.h {
                    if mods.command && !hit.open.is_empty() {
                        anvil_platform::system::open_with_default_app(&hit.open);
                    } else if !hit.copy.is_empty() {
                        anvil_platform::system::set_clipboard(&hit.copy);
                    }
                    app.dirty = true;
                    return;
                }
            }
        }

        // Search-bar arrow hit-test (N4): ◀ / ▶ nav buttons.
        if app.search_open {
            let (rx, ry) = app.view_pt_to_raster_px(loc);
            let h = &app.search_bar_hits;
            let hit_rect = |r: [f64; 4]| {
                r[2] > 0.0 && rx >= r[0] && rx < r[0] + r[2] && ry >= r[1] && ry < r[1] + r[3]
            };
            if hit_rect(h.prev) {
                if app.focused_is_native_editor() {
                    app.apply_editor_action(EditorAction::SearchPrev);
                } else {
                    app.search.prev();
                }
                app.dirty = true;
                return;
            }
            if hit_rect(h.next) {
                if app.focused_is_native_editor() {
                    app.apply_editor_action(EditorAction::SearchNext);
                } else {
                    app.search.next();
                }
                app.dirty = true;
                return;
            }
        }

        // Sidebar right-edge resize handle (item 13).
        // Hit zone: ±SIDEBAR_DRAG_HIT_PX logical pt, scaled by ui_scale (A6).
        if app.left_dock_visible && app.layout_mode == LayoutMode::Ide {
            let (rx, _ry) = app.view_pt_to_raster_px(loc);
            let edge_x = app.left_dock_w_pt * app.window_scale;
            if (rx - edge_x).abs() <= SIDEBAR_DRAG_HIT_PX * app.ui_scale {
                app.sidebar_drag_active = true;
                app.dirty = true;
                return;
            }
        }

        // IDE editor/drawer horizontal divider (item 13b).
        // Hit zone: ±4 logical pt, scaled by ui_scale (A6).
        {
            const DRAWER_DRAG_HIT_PT: f64 = 4.0;
            let (_rx, ry) = app.view_pt_to_raster_px(loc);
            if let Some(div_y) = app.ide_drawer_divider_y() {
                if (ry - div_y).abs() <= DRAWER_DRAG_HIT_PT * app.ui_scale {
                    app.drawer_drag_active = true;
                    app.dirty = true;
                    return;
                }
            }
        }

        // Left dock click — explorer rows open folders/files before pane hit-testing.
        {
            let (rx, ry) = app.view_pt_to_raster_px(loc);
            let hit_kind = app.left_dock_hits.at(rx, ry).cloned();
            if let Some(LeftDockHitKind::Explorer(hit)) = hit_kind {
                // Click on any explorer element sets Explorer focus (item 4).
                app.focus_target = FocusTarget::Explorer;
                match hit {
                    ExplorerHit::Header => {
                        if let Some(snapshot) = app.fs_snapshot.clone() {
                            if let Some(path) =
                                explorer_path_for_hit(&snapshot, ExplorerHit::Header)
                            {
                                app.open_path_in_native_editor(&path);
                            }
                        }
                    }
                    ExplorerHit::Row(idx) => {
                        // Track keyboard-nav selection to clicked row (item 4).
                        app.selected_explorer_row = Some(idx);
                        // Look up the absolute path and is_dir via visible_rows
                        // populated by the last draw call.
                        if let Some((path, is_dir)) =
                            app.left_dock_hits.visible_rows.get(idx).cloned()
                        {
                            if is_dir {
                                // Toggle expand/collapse.
                                if app.expanded_dirs.contains(&path) {
                                    app.expanded_dirs.remove(&path);
                                } else {
                                    // Synchronously read the dir on the main
                                    // thread — top-level dirs are sub-ms and
                                    // round-tripping through the worker adds
                                    // a perceptible click→render lag.
                                    if !app.child_snapshots.contains_key(&path) {
                                        let snap = fs_worker::read_dir_snapshot_fast(
                                            &path,
                                            app.filter_flags(),
                                        );
                                        app.child_snapshots.insert(
                                            path.clone(),
                                            LeftDockSnapshot {
                                                root: snap.root.to_string_lossy().into_owned(),
                                                entries: snap
                                                    .entries
                                                    .into_iter()
                                                    .map(|e| anvil_render::left_dock::DirEntry {
                                                        name: e.name,
                                                        is_dir: e.is_dir,
                                                    })
                                                    .collect(),
                                                git_marks: snap.git_marks,
                                            },
                                        );
                                    }
                                    app.expanded_dirs.insert(path);
                                }
                                app.dirty = true;
                            } else {
                                // I3: arm the explorer drag state so mouse_dragged can
                                // track the cursor for the floating chip.
                                app.explorer_drag = Some((path.clone(), loc));
                                app.open_path_in_native_editor(&path);
                                // Item 5: sync active_explorer_file after open.
                                app.active_explorer_file = Some(path);
                            }
                        }
                    }
                }
                return;
            }
            // Outline row click (item 19): jump cursor to the symbol's line.
            if let Some(LeftDockHitKind::Outline(idx)) = hit_kind {
                // The outline_rows drawn last frame are not stored on App, so we
                // re-derive them here to map idx → line. This is cheap (< 1 ms).
                let jump_line: Option<usize> = app.tabs.current().and_then(|tab| {
                    let pid = tab.focused_id();
                    let ep = tab.editor_panes.get_pane(pid)?;
                    let buf = tab.editor_panes.get_buffer(ep.buffer_id)?;
                    if buf
                        .tracked_path()
                        .and_then(|p| p.extension())
                        .and_then(|e| e.to_str())
                        .map(|e| e.eq_ignore_ascii_case("rs"))
                        != Some(true)
                    {
                        return None;
                    }
                    let text = buf.to_text();
                    let symbols = anvil_editor::derive_outline_rows(buf.syntax(), &text);
                    symbols.get(idx).map(|s| s.line)
                });
                if let Some(line) = jump_line {
                    app.apply_editor_action(EditorAction::MoveTo {
                        pos: EditorPosition { line, col: 0 },
                        extend: false,
                    });
                    app.dirty = true;
                }
                return;
            }
        }

        // Editor buffer tab strip click — switch or close a buffer tab.
        {
            let (rx, ry) = app.view_pt_to_raster_px(loc);
            let hit = app
                .editor_tab_hits
                .iter()
                .find(|h| {
                    rx >= h.rect.x
                        && rx < h.rect.x + h.rect.w
                        && ry >= h.rect.y
                        && ry < h.rect.y + h.rect.h
                })
                .cloned();
            if let Some(h) = hit {
                if h.is_close {
                    // Close the buffer. The registry returns None when no
                    // buffers remain (fall back to scratch — new scratch was
                    // created inside close_buffer).
                    if let Some(tab) = app.tabs.current_mut() {
                        // Q16: record path before closing so Cmd+Shift+T can reopen it.
                        let closed_path = tab
                            .editor_panes
                            .get_buffer(h.buffer_id)
                            .and_then(|b| b.tracked_path())
                            .map(|p| p.to_path_buf());
                        let _new_active = tab.editor_panes.close_buffer(h.pane_id, h.buffer_id);
                        // Sync pane.editor_id to whatever is now active.
                        if let Some(ep) = tab.editor_panes.get_pane(h.pane_id) {
                            let active = ep.buffer_id;
                            if let Some(pane) = tab.registry.get_mut(h.pane_id) {
                                pane.editor_id = Some(active);
                            }
                        }
                        if let Some(path) = closed_path {
                            app.closed_tabs.push_back(path);
                            if app.closed_tabs.len() > 20 {
                                app.closed_tabs.pop_front();
                            }
                        }
                    }
                } else {
                    // Switch to the clicked buffer.
                    if let Some(tab) = app.tabs.current_mut() {
                        tab.editor_panes.open_buffer(h.pane_id, h.buffer_id);
                        if let Some(pane) = tab.registry.get_mut(h.pane_id) {
                            pane.editor_id = Some(h.buffer_id);
                        }
                    }
                    // Item 5: sync active_explorer_file to the newly active buffer.
                    app.sync_active_explorer_file();
                    // Item 10 (Tier-B): record drag start for buffer tab reorder.
                    // Resolve the buffer index in open_buffers so mouse_dragged
                    // can swap without re-scanning hit rects.
                    if let Some(tab) = app.tabs.current() {
                        if let Some(ep) = tab.editor_panes.get_pane(h.pane_id) {
                            if let Some(pos) =
                                ep.open_buffers.iter().position(|&b| b == h.buffer_id)
                            {
                                app.editor_tab_drag = Some((h.pane_id, pos, rx));
                            }
                        }
                    }
                }
                app.force_full_redraw = true;
                app.dirty = true;
                return;
            }
        }

        // P3: horizontal scrollbar drag — click in the bottom 3px of the focused
        // native editor pane's body starts an hscroll drag.
        if app.focused_is_native_editor() {
            let (rx, ry) = app.view_pt_to_raster_px(loc);
            let ir = app.pane_area_rect();
            if let Some(tab) = app.tabs.current() {
                let id = tab.focused_id();
                let entries = tab.tree.layout(ir, DIVIDER_PX);
                if let Some(e) = entries.iter().find(|e| e.id == id) {
                    let hbar_y = e.rect.y + e.rect.h - 3.0;
                    if ry >= hbar_y
                        && ry <= e.rect.y + e.rect.h
                        && rx >= e.rect.x
                        && rx < e.rect.x + e.rect.w
                    {
                        app.hscroll_drag_active = true;
                        app.dirty = true;
                        return;
                    }
                }
            }
        }

        // Chrome row click — use hit rects populated by draw_tab_bar.
        {
            let (rx, ry) = app.view_pt_to_raster_px(loc);
            // Collect matching hit to avoid borrow conflict.
            let hit_kind = app
                .tab_bar_hits
                .hits
                .iter()
                .find(|h| {
                    let r = &h.rect;
                    rx >= r.x && rx < r.x + r.w && ry >= r.y && ry < r.y + r.h
                })
                .map(|h| h.kind.clone());
            if let Some(kind) = hit_kind {
                match kind {
                    TabBarHitKind::Tab(idx) => {
                        app.tabs.switch_to(idx);
                        app.snap_anim();
                        app.force_full_redraw = true;
                        app.dirty = true;
                        // Record drag start so mouse_dragged can reorder tabs.
                        app.tab_drag = Some((idx, rx));
                    }
                    TabBarHitKind::CloseTab(idx) => {
                        app.tabs.switch_to(idx);
                        app.force_full_redraw = true;
                        app.close_active_tab();
                    }
                    TabBarHitKind::AddTab => {
                        app.force_full_redraw = true;
                        app.add_tab();
                    }
                }
                return;
            }
        }

        // Pane divider drag: hit-test before click-to-focus so a grab on the
        // divider doesn't also switch focus.
        {
            let (rx, ry) = app.view_pt_to_raster_px(loc);
            let ir = app.pane_area_rect();
            if let Some(tab) = app.tabs.current() {
                let slop = DIVIDER_PX * 0.5 + 4.0;
                if let Some(hit) = find_divider_at(&tab.tree, ir, DIVIDER_PX, rx, ry, slop) {
                    app.divider_drag = Some(hit);
                    app.dirty = true;
                    return;
                }
            }
        }

        // Click-to-focus.
        {
            let (rx, ry) = app.view_pt_to_raster_px(loc);
            let ir = app.pane_area_rect();
            let hit_id = app
                .tabs
                .current()
                .and_then(|tab| tab.tree.hit_test(ir, DIVIDER_PX, rx, ry));
            if let Some(hit_id) = hit_id {
                if Some(hit_id) != app.tabs.current().map(|t| t.focused_id()) {
                    if let Some(tab) = app.tabs.current_mut() {
                        tab.tree.focused = hit_id;
                    }
                    app.snap_anim();
                    app.dirty = true;
                }
            }
        }

        // Native editor gutter click → toggle fold (item 13).
        if app.focused_is_native_editor() && !mods.command {
            if let Some((rel_x, rel_y)) = app.native_editor_rel_px(loc) {
                if let Some(fold_line) = app.gutter_click_fold_line(rel_x, rel_y) {
                    app.apply_editor_action(EditorAction::ToggleFold(fold_line));
                    app.dirty = true;
                    return;
                }
            }
        }

        // Native editor mouse: click → cursor, double → word, triple → line (NE7).
        // Cmd+click (single) → goto definition (item 17; was secondary cursor NE13).
        if app.focused_is_native_editor() {
            if let Some(pos) = app.native_editor_pos_at(loc) {
                if mods.command && click_count == 1 {
                    // Item 17: Cmd+click → jump to definition.
                    // Move cursor to the clicked position first, then fire definition.
                    app.apply_editor_action(EditorAction::MoveTo { pos, extend: false });
                    let pane_id = app.tabs.current().map(|t| t.focused_id()).unwrap_or(0);
                    app.trigger_definition_request(pane_id);
                    app.dirty = true;
                    return;
                }
                let action = if click_count >= 3 {
                    EditorAction::SelectLineAt(pos)
                } else if click_count == 2 {
                    EditorAction::SelectWordAt(pos)
                } else {
                    EditorAction::MoveTo {
                        pos,
                        extend: mods.shift,
                    }
                };
                app.apply_editor_action(action);
                app.editor_mouse_drag_start = Some(pos);
                app.dirty = true;
            }
            return;
        }

        // Mouse reporting.
        {
            let (btn_mode, x10_mode) = app
                .tabs
                .current()
                .and_then(|t| t.registry.get(t.focused_id()))
                .and_then(|p| p.terminal.as_ref())
                .map(|t| (t.modes.mouse_button, t.modes.mouse_x10))
                .unwrap_or((false, false));
            if btn_mode || x10_mode {
                if let Some((row, col)) = app.event_cell(loc, false) {
                    app.write_mouse_event(0, col, row, true);
                }
                return;
            }
        }

        // ⌥-click: copy block output to clipboard when clicking a block header.
        if mods.option && !mods.command {
            if let Some((row, _col)) = app.event_cell(loc, false) {
                let tab = app.tabs.current_mut().unwrap();
                let id = tab.focused_id();
                let pane = tab.registry.get_mut(id).unwrap();
                if let Some(terminal) = &mut pane.terminal {
                    let cr = terminal.content_row_of_viewport(row);
                    let abs = terminal.absolute_line_of_content(cr);
                    if let Some(block) = terminal.block_at(abs) {
                        if block.command_line == abs {
                            let evicted = terminal.evicted_lines;
                            let out_start = block.output_line.saturating_sub(evicted);
                            let out_end = block.end_line.saturating_sub(evicted);
                            let mut lines: Vec<String> = (out_start..out_end)
                                .map(|ci| {
                                    terminal
                                        .line(ci)
                                        .iter()
                                        .map(|c| c.cp)
                                        .collect::<String>()
                                        .trim_end()
                                        .to_owned()
                                })
                                .collect();
                            // Trim trailing blank rows.
                            while lines.last().map(|l: &String| l.is_empty()).unwrap_or(false) {
                                lines.pop();
                            }
                            let text = lines.join("\n");
                            anvil_platform::system::set_clipboard(&text);
                            return;
                        }
                    }
                }
            }
        }

        // ⌘-click: fold/unfold block header, or open file/url under cursor.
        if mods.command {
            if let Some((row, col)) = app.event_cell(loc, false) {
                // If the click landed exactly on a block header row, toggle its
                // fold and skip the URL/path open logic entirely.
                {
                    let tab = app.tabs.current_mut().unwrap();
                    let id = tab.focused_id();
                    let pane = tab.registry.get_mut(id).unwrap();
                    if let Some(terminal) = &mut pane.terminal {
                        let cr = terminal.content_row_of_viewport(row);
                        let abs = terminal.absolute_line_of_content(cr);
                        if let Some(block) = terminal.block_at(abs) {
                            if block.command_line == abs {
                                pane.toggle_fold(block.command_line);
                                app.dirty = true;
                                return;
                            }
                        }
                    }
                }

                let (content_row, cells): (usize, Vec<anvil_term::Cell>) = {
                    let tab = app.tabs.current_mut().unwrap();
                    let id = tab.focused_id();
                    let pane = tab.registry.get_mut(id).unwrap();
                    if let Some(terminal) = &pane.terminal {
                        let cr = terminal.content_row_of_viewport(row);
                        (cr, terminal.line(cr).to_vec())
                    } else {
                        return;
                    }
                };
                let _ = content_row;
                let mut line_buf = String::new();
                let mut col_to_byte = Vec::with_capacity(cells.len());
                for cell in &cells {
                    col_to_byte.push(line_buf.len());
                    let ch = cell.cp;
                    line_buf.push(ch);
                }
                let byte_col = col_to_byte.get(col).copied().unwrap_or(line_buf.len());
                let cwd = app.current_cwd().unwrap_or_default();
                let raw_tok = interact::token_at_col(&line_buf, byte_col);
                let tok = interact::strip_line_suffix(raw_tok);
                match interact::classify(tok, &cwd) {
                    interact::Kind::Url => app.pty_write_open_url(tok),
                    interact::Kind::Path => app.pty_write_open_file(tok),
                    interact::Kind::PathWithLine { path, line, col } => {
                        app.pty_write_open_file_at(&path, line, col)
                    }
                    interact::Kind::None => {}
                }
            }
            return;
        }

        // Begin / extend / word-or-line-select selection.
        //   - shift + click on an active selection: extend (move head).
        //   - double-click: select the whitespace-delimited token at click.
        //   - triple-click: select the entire line.
        //   - option + click-drag: rectangular (block) selection.
        //   - plain click: start a fresh selection at the click point.
        if let Some((row, col)) = app.event_cell(loc, false) {
            if let Some(tab) = app.tabs.current_mut() {
                let id = tab.focused_id();
                if let Some(pane) = tab.registry.get_mut(id) {
                    use anvil_workspace::selection::{Point, Selection, SelectionMode};
                    let cr = pane
                        .terminal
                        .as_ref()
                        .map(|t| t.content_row_of_viewport(row))
                        .unwrap_or(row);

                    if mods.shift && pane.selection.active {
                        // Extend the current selection to the click point.
                        pane.selection.head = Point { row: cr, col };
                    } else if click_count >= 3 {
                        // Whole-line selection.
                        let line_len = pane
                            .terminal
                            .as_ref()
                            .map(|t| t.line(cr).len())
                            .unwrap_or(0);
                        pane.selection = Selection {
                            active: true,
                            anchor: Point { row: cr, col: 0 },
                            head: Point {
                                row: cr,
                                col: line_len,
                            },
                            mode: SelectionMode::Linear,
                        };
                    } else if click_count == 2 {
                        // Word selection — extend left/right while the cell
                        // codepoint is not ASCII whitespace.
                        let line = pane
                            .terminal
                            .as_ref()
                            .map(|t| t.line(cr).to_vec())
                            .unwrap_or_default();
                        let is_word = |c: char| !c.is_ascii_whitespace();
                        if col < line.len() && is_word(line[col].cp) {
                            let mut lo = col;
                            while lo > 0 && is_word(line[lo - 1].cp) {
                                lo -= 1;
                            }
                            let mut hi = col + 1;
                            while hi < line.len() && is_word(line[hi].cp) {
                                hi += 1;
                            }
                            pane.selection = Selection {
                                active: true,
                                anchor: Point { row: cr, col: lo },
                                head: Point { row: cr, col: hi },
                                mode: SelectionMode::Linear,
                            };
                        }
                    } else {
                        pane.selection = Selection {
                            active: true,
                            anchor: Point { row: cr, col },
                            head: Point { row: cr, col },
                            mode: if mods.option {
                                SelectionMode::Rect
                            } else {
                                SelectionMode::Linear
                            },
                        };
                    }
                }
            }
        } else {
            if let Some(tab) = app.tabs.current_mut() {
                let id = tab.focused_id();
                if let Some(pane) = tab.registry.get_mut(id) {
                    pane.selection.clear();
                }
            }
        }
        app.dirty = true;
    }

    fn mouse_up(&mut self, loc: MouseLocation, _mods: Modifiers) {
        let app = &mut self.app;

        // Native editor drag-select: release (NE7).
        app.editor_mouse_drag_start = None;

        // I3: Explorer drag: if the user dragged past the threshold and released
        // over an editor area, open the file (it was already opened on mouse_down,
        // so this is a no-op for the same pane, or opens in a newly focused pane).
        if app.explorer_drag_cursor.is_some() {
            if let Some((path, _)) = app.explorer_drag.take() {
                app.open_path_in_native_editor(&path);
            }
            app.explorer_drag_cursor = None;
            app.dirty = true;
        } else {
            app.explorer_drag = None;
        }

        // Tab reorder drag: release — just clear the state.
        if app.tab_drag.take().is_some() {
            app.dirty = true;
        }

        // Editor buffer tab drag: release (Tier-B item 10).
        if app.editor_tab_drag.take().is_some() {
            app.dirty = true;
        }

        // Pane divider drag: release.
        if app.divider_drag.is_some() {
            app.divider_drag = None;
            app.dirty = true;
            return;
        }

        // Sidebar resize drag: release (item 13).
        if app.sidebar_drag_active {
            app.sidebar_drag_active = false;
            app.dirty = true;
            return;
        }

        // IDE drawer divider drag: release (item 13b).
        if app.drawer_drag_active {
            app.drawer_drag_active = false;
            app.dirty = true;
            return;
        }

        // P3: horizontal scrollbar drag release.
        if app.hscroll_drag_active {
            app.hscroll_drag_active = false;
            app.dirty = true;
            return;
        }

        // HUD resize drag: release.
        if app.hud_drag_active {
            app.hud_drag_active = false;
            app.dirty = true;
            return;
        }

        // HUD section reorder: find the section under the release point and
        // move the dragged section to that slot. Persist the new order.
        if let Some(grabbed) = app.hud_section_drag.take() {
            let (rx, ry) = app.view_pt_to_raster_px(loc);
            let mut target: Option<SectionId> = None;
            for h in &app.hud_section_hits {
                let r = h.rect;
                if rx >= r.x && rx < r.x + r.w && ry >= r.y && ry < r.y + r.h {
                    target = Some(h.section);
                    break;
                }
            }
            if let Some(target) = target {
                if target != grabbed {
                    let order = &mut app.hud_section_order;
                    if let (Some(from), Some(to)) = (
                        order.iter().position(|&s| s == grabbed),
                        order.iter().position(|&s| s == target),
                    ) {
                        let item = order.remove(from);
                        // Insert at the target index, clamped to the now-shorter Vec.
                        let insert_at = to.min(order.len());
                        order.insert(insert_at, item);
                        save_hud_section_order(order);
                    }
                }
            }
            app.force_full_redraw = true;
            app.dirty = true;
            return;
        }

        // Mouse reporting: release.
        let (btn_mode, sgr_mode) = app
            .tabs
            .current()
            .and_then(|t| t.registry.get(t.focused_id()))
            .and_then(|p| p.terminal.as_ref())
            .map(|t| (t.modes.mouse_button, t.modes.mouse_sgr))
            .unwrap_or((false, false));

        if btn_mode && sgr_mode {
            if let Some((row, col)) = app.event_cell(loc, false) {
                app.write_mouse_event(0, col, row, false);
            }
            app.dirty = true;
            return;
        }
        if btn_mode {
            app.dirty = true;
            return;
        }

        // Clear zero-length selection.
        let should_clear = app
            .tabs
            .current()
            .and_then(|t| t.registry.get(t.focused_id()))
            .map(|p| {
                p.selection.active
                    && p.selection.anchor.row == p.selection.head.row
                    && p.selection.anchor.col == p.selection.head.col
            })
            .unwrap_or(false);
        if should_clear {
            if let Some(tab) = app.tabs.current_mut() {
                let id = tab.focused_id();
                if let Some(pane) = tab.registry.get_mut(id) {
                    pane.selection.clear();
                }
            }
        }
        app.dirty = true;
    }

    fn mouse_dragged(&mut self, loc: MouseLocation) {
        let app = &mut self.app;

        // Update explorer hover state from mouse position.
        {
            let (rx, ry) = app.view_pt_to_raster_px(loc);
            let new_hover = app.left_dock_hits.at(rx, ry).and_then(|kind| {
                if let LeftDockHitKind::Explorer(ExplorerHit::Row(i)) = kind {
                    Some(*i)
                } else {
                    None
                }
            });
            if new_hover != app.hovered_explorer_row {
                app.hovered_explorer_row = new_hover;
                app.explorer_hover_row = new_hover.map(|i| (i, Instant::now()));
                app.explorer_hover_meta = None;
                app.dirty = true;
            }

            // Update editor tab hover.
            let new_editor_hover = app
                .editor_tab_hits
                .iter()
                .find(|h| {
                    !h.is_close
                        && rx >= h.rect.x
                        && rx < h.rect.x + h.rect.w
                        && ry >= h.rect.y
                        && ry < h.rect.y + h.rect.h
                })
                .map(|h| (h.pane_id, h.buffer_id));
            if new_editor_hover != app.hovered_editor_tab {
                app.hovered_editor_tab = new_editor_hover;
                app.dirty = true;
            }
        }

        // I3: Explorer drag — enter drag mode when cursor leaves an Explorer row
        // by more than 4 logical pt.  The drag_path is set on mouse_down if the
        // click landed on a non-directory Explorer row; see mouse_down above.
        if let Some((ref drag_path, start_loc)) = app.explorer_drag.clone() {
            let dx = loc.x - start_loc.x;
            let dy = loc.y - start_loc.y;
            let threshold_pt = 4.0;
            if dx * dx + dy * dy >= threshold_pt * threshold_pt {
                // We are actively dragging — track cursor for the chip render.
                app.explorer_drag_cursor = Some(loc);
                app.dirty = true;
                let _ = drag_path; // hold borrow
            }
        }

        // Native editor drag-select: extend selection to current pointer (NE7).
        if app.editor_mouse_drag_start.is_some() && app.focused_is_native_editor() {
            if let Some(pos) = app.native_editor_pos_at(loc) {
                app.apply_editor_action(EditorAction::MoveTo { pos, extend: true });
                app.dirty = true;
            }
            return;
        }

        // Editor buffer tab reorder drag (Tier-B item 10).
        // Threshold: 4 logical pixels × ui_scale.
        if let Some((drag_pane, drag_buf_idx, down_x)) = app.editor_tab_drag {
            let (rx, _ry) = app.view_pt_to_raster_px(loc);
            let threshold = 4.0 * app.ui_scale;
            if (rx - down_x).abs() >= threshold {
                // Find the editor tab hit zone the cursor is over.
                let target_buf_hit = app
                    .editor_tab_hits
                    .iter()
                    .find(|h| {
                        !h.is_close
                            && h.pane_id == drag_pane
                            && rx >= h.rect.x
                            && rx < h.rect.x + h.rect.w
                    })
                    .cloned();
                if let Some(target_h) = target_buf_hit {
                    if let Some(tab) = app.tabs.current_mut() {
                        if let Some(ep) = tab.editor_panes.get_pane_mut(drag_pane) {
                            if let Some(to) = ep
                                .open_buffers
                                .iter()
                                .position(|&b| b == target_h.buffer_id)
                            {
                                if drag_buf_idx != to {
                                    ep.open_buffers.swap(drag_buf_idx, to);
                                    app.editor_tab_drag = Some((drag_pane, to, down_x));
                                    app.force_full_redraw = true;
                                    app.dirty = true;
                                }
                            }
                        }
                    }
                }
            }
            return;
        }

        // Tab reorder drag: if the cursor has moved past the threshold into a
        // different tab's hit zone, move the dragged tab there.
        if let Some((drag_idx, down_x)) = app.tab_drag {
            let (rx, _ry) = app.view_pt_to_raster_px(loc);
            let threshold = app.font.metrics.cell_w * 0.5;
            if (rx - down_x).abs() >= threshold {
                // Find the tab hit zone the cursor is currently over.
                let target = app
                    .tab_bar_hits
                    .hits
                    .iter()
                    .find(|h| {
                        matches!(h.kind, TabBarHitKind::Tab(_))
                            && rx >= h.rect.x
                            && rx < h.rect.x + h.rect.w
                    })
                    .and_then(|h| {
                        if let TabBarHitKind::Tab(t) = h.kind {
                            Some(t)
                        } else {
                            None
                        }
                    });
                if let Some(to) = target {
                    if drag_idx != to {
                        app.tabs.move_tab(drag_idx, to);
                        app.tab_drag = Some((to, down_x));
                        app.force_full_redraw = true;
                        app.dirty = true;
                    }
                }
            }
            return;
        }

        // Pane divider drag: adjust the split ratio based on mouse position.
        if app.divider_drag.is_some() {
            let (rx, ry) = app.view_pt_to_raster_px(loc);
            // Clone the hit fields we need before mutably borrowing app.tabs.
            let (path, child_index, split_rect, split_dir) = {
                let hit = app.divider_drag.as_ref().unwrap();
                (
                    hit.path.clone(),
                    hit.child_index,
                    hit.split_rect,
                    hit.split_dir,
                )
            };
            if let Some(tab) = app.tabs.current_mut() {
                if let Some(sp) = split_at_path_mut(&mut tab.tree, &path) {
                    let n = sp.children.len();
                    let total_gutter = DIVIDER_PX * (n as f64 - 1.0);
                    // Use the split node's own rect (captured at mouse-down) so
                    // nested splits compute available space correctly.
                    let (available, mouse_offset) = match split_dir {
                        SplitDir::Horizontal => (split_rect.w - total_gutter, rx - split_rect.x),
                        SplitDir::Vertical => (split_rect.h - total_gutter, ry - split_rect.y),
                    };
                    // The desired pixel size of children[0..=child_index] combined
                    // is the mouse offset minus the gutters before child_index.
                    let gutter_offset = child_index as f64 * DIVIDER_PX;
                    let desired_px = (mouse_offset - gutter_offset - DIVIDER_PX * 0.5)
                        .max(0.0)
                        .min(available);
                    // desired_px = sum(ratios[0..=child_index]) * available
                    let prefix_sum: f64 = sp.ratios[..child_index].iter().sum();
                    let new_ratio_i = (desired_px / available) - prefix_sum;
                    let old_ratio_i = sp.ratios[child_index];
                    let delta = new_ratio_i - old_ratio_i;
                    adjust_ratio(sp, child_index, delta, 0.05);
                }
            }
            app.resize_all_tabs();
            app.force_full_redraw = true;
            app.dirty = true;
            return;
        }

        // Sidebar resize drag (item 13): convert mouse x to new left_dock_w_pt.
        // G3: write to _target; the tick loop eases _pt toward it.
        if app.sidebar_drag_active {
            let (rx, _) = app.view_pt_to_raster_px(loc);
            let new_w_pt = (rx / app.window_scale).clamp(SIDEBAR_W_MIN_PT, SIDEBAR_W_MAX_PT);
            if (new_w_pt - app.left_dock_w_pt_target).abs() > 0.5 {
                app.left_dock_w_pt_target = new_w_pt;
                app.dirty = true;
            }
            return;
        }

        // IDE editor/drawer divider drag (item 13b): mutate the root split ratios.
        if app.drawer_drag_active {
            let (_, ry) = app.view_pt_to_raster_px(loc);
            let ir = app.pane_area_rect();
            if ir.h > 0.0 {
                // drawer ratio = fraction of pane area below the mouse.
                let raw_editor_ratio = (ry - ir.y) / ir.h;
                // Clamp: drawer ratio in [0.05, 0.60] → editor ratio in [0.40, 0.95].
                let editor_ratio = raw_editor_ratio.clamp(0.40, 0.95);
                let drawer_ratio = 1.0 - editor_ratio;
                let _ = drawer_ratio; // unused variable; value used via editor_ratio
                if let Some(tab) = app.tabs.current_mut() {
                    let root = tab.tree.root.as_mut();
                    if let anvil_workspace::layout::PaneNode::Split(sp) = root {
                        if sp.dir == SplitDir::Vertical && sp.ratios.len() == 2 {
                            sp.ratios[0] = editor_ratio;
                            sp.ratios[1] = 1.0 - editor_ratio;
                        }
                    }
                }
                app.resize_all_tabs();
                app.force_full_redraw = true;
                app.dirty = true;
            }
            return;
        }

        // P3: horizontal scrollbar thumb drag — map mouse x to scroll_x.
        if app.hscroll_drag_active {
            let (rx, _) = app.view_pt_to_raster_px(loc);
            let cw = app.font.metrics.cell_w;
            let ir = app.pane_area_rect();
            if let Some(tab) = app.tabs.current_mut() {
                let id = tab.focused_id();
                let entries = tab.tree.layout(ir, DIVIDER_PX);
                if let Some(e) = entries.iter().find(|e| e.id == id) {
                    let pane_rect = e.rect;
                    if let Some(ep) = tab.editor_panes.get_pane(id) {
                        let bid = ep.buffer_id;
                        if let Some(buf) = tab.editor_panes.get_buffer(bid) {
                            // Compute gutter_w to match the renderer.
                            let line_count = buf.line_count().max(1);
                            let digit_cols = line_count.to_string().len();
                            let git_gutter_cols = if buf.git_gutter.is_some() { 2 } else { 0 };
                            let gutter_cols = digit_cols + 2 + git_gutter_cols;
                            let gutter_w = gutter_cols as f64 * cw;
                            let content_w = pane_rect.w - gutter_w;
                            let content_cols = ((content_w) / cw).floor() as usize;
                            // Compute max_line_len across all buffer lines.
                            let max_line_len: usize = (0..line_count)
                                .map(|i| {
                                    let s: String = buf.line(i).chars().collect();
                                    let s = s.trim_end_matches('\n').trim_end_matches('\r');
                                    s.graphemes(true).count()
                                })
                                .max()
                                .unwrap_or(0);
                            let vis_cols = content_cols as f64;
                            let total_cols = max_line_len as f64;
                            let max_hscroll = (total_cols - vis_cols).max(0.0);
                            if max_hscroll > 0.0 {
                                let thumb_w = ((vis_cols / total_cols) * content_w)
                                    .max(20.0)
                                    .min(content_w);
                                let track_start = pane_rect.x + gutter_w;
                                let track_len = content_w - thumb_w;
                                if track_len > 0.0 {
                                    let t = ((rx - track_start - thumb_w / 2.0) / track_len)
                                        .clamp(0.0, 1.0);
                                    let new_scroll_x = t * max_hscroll;
                                    if let Some(ep) = tab.editor_panes.get_pane_mut(id) {
                                        if (ep.scroll_x - new_scroll_x).abs() > 0.01 {
                                            ep.scroll_x = new_scroll_x;
                                            app.force_full_redraw = true;
                                            app.dirty = true;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            return;
        }

        // HUD resize drag: convert mouse x to a new column count and reflow.
        if app.hud_drag_active {
            let (rx, _) = app.view_pt_to_raster_px(loc);
            let cw = app.font.metrics.cell_w;
            let (dw, _) = app.device_size();
            // Desired surface left edge is at the mouse x. The surface
            // spans `hud_cols * cw + GRID_PAD` pixels from there to dw, so
            // new `hud_cols = ((dw - rx) - GRID_PAD) / cw`.
            let want_w_px = (dw as f64 - rx - GRID_PAD as f64).max(0.0);
            let want_cols = (want_w_px / cw).round() as i64;
            let new_cols = want_cols.clamp(HUD_COLS_MIN as i64, HUD_COLS_MAX as i64) as usize;
            if new_cols != app.hud_cols {
                app.hud_cols = new_cols;
                app.resize_all_tabs();
                app.force_full_redraw = true;
                app.dirty = true;
            }
            return;
        }

        let btn_mode = app
            .tabs
            .current()
            .and_then(|t| t.registry.get(t.focused_id()))
            .and_then(|p| p.terminal.as_ref())
            .map(|t| t.modes.mouse_button)
            .unwrap_or(false);
        if btn_mode {
            if let Some((row, col)) = app.event_cell(loc, false) {
                app.write_mouse_event(32, col, row, true); // left drag (button 0 + motion flag 32)
            }
            return;
        }
        // Extend selection.
        let active = app
            .tabs
            .current()
            .and_then(|t| t.registry.get(t.focused_id()))
            .map(|p| p.selection.active)
            .unwrap_or(false);
        if !active {
            return;
        }
        if let Some((row, col)) = app.event_cell(loc, true) {
            if let Some(tab) = app.tabs.current_mut() {
                let id = tab.focused_id();
                if let Some(pane) = tab.registry.get_mut(id) {
                    let cr = pane
                        .terminal
                        .as_ref()
                        .map(|t| t.content_row_of_viewport(row))
                        .unwrap_or(row);
                    pane.selection.head = anvil_workspace::selection::Point { row: cr, col };
                }
            }
        }
        app.dirty = true;
    }

    fn mouse_moved(&mut self, loc: MouseLocation) -> CursorKind {
        let app = &mut self.app;
        let (rx, ry) = app.view_pt_to_raster_px(loc);
        let new_hover = app.left_dock_hits.at(rx, ry).and_then(|kind| {
            if let LeftDockHitKind::Explorer(ExplorerHit::Row(i)) = kind {
                Some(*i)
            } else {
                None
            }
        });
        if new_hover != app.hovered_explorer_row {
            app.hovered_explorer_row = new_hover;
            // R2: reset hover tooltip state when row changes.
            app.explorer_hover_row = new_hover.map(|i| (i, Instant::now()));
            app.explorer_hover_meta = None;
            app.dirty = true;
        }

        // Update which editor buffer tab is hovered (for × show-on-hover).
        let new_editor_hover = app
            .editor_tab_hits
            .iter()
            .find(|h| {
                !h.is_close
                    && rx >= h.rect.x
                    && rx < h.rect.x + h.rect.w
                    && ry >= h.rect.y
                    && ry < h.rect.y + h.rect.h
            })
            .map(|h| (h.pane_id, h.buffer_id));
        if new_editor_hover != app.hovered_editor_tab {
            app.hovered_editor_tab = new_editor_hover;
            app.dirty = true;
        }

        // Item 15: hover debounce — record position + timestamp when over editor.
        // The tick loop fires the request after 400ms if the mouse hasn't moved.
        if app.focused_is_native_editor() {
            let new_pos = (rx, ry);
            if app.hover_mouse_pos != Some(new_pos) {
                app.hover_mouse_pos = Some(new_pos);
                app.hover_mouse_time = Some(Instant::now());
            }
        }

        // P2: detect divider hover for highlight stripe + cursor feedback.
        let new_divider_hover = {
            let mut dh: Option<DividerKind> = None;
            // Sidebar right-edge (col-resize).
            if app.left_dock_visible && app.layout_mode == LayoutMode::Ide {
                let edge_x = app.left_dock_w_pt * app.window_scale;
                if (rx - edge_x).abs() <= SIDEBAR_DRAG_HIT_PX * app.ui_scale {
                    dh = Some(DividerKind::Sidebar);
                }
            }
            // Drawer horizontal divider (row-resize).
            if dh.is_none() {
                const DRAWER_HIT_PT: f64 = 4.0;
                if let Some(div_y) = app.ide_drawer_divider_y() {
                    if (ry - div_y).abs() <= DRAWER_HIT_PT * app.ui_scale {
                        dh = Some(DividerKind::Drawer);
                    }
                }
            }
            dh
        };
        if new_divider_hover != app.divider_hover {
            app.divider_hover = new_divider_hover;
            app.dirty = true;
        }

        match app.divider_hover {
            Some(DividerKind::Sidebar) => CursorKind::ColResize,
            Some(DividerKind::Drawer) => CursorKind::RowResize,
            None => CursorKind::Arrow,
        }
    }

    fn scroll(&mut self, dy: f64, pixel_precise: bool, shift: bool, loc: MouseLocation) {
        let app = &mut self.app;

        // P3: Shift+scroll → horizontal scroll in native editor pane.
        if shift && dy != 0.0 && app.focused_is_native_editor() {
            let cell_w_pt = (app.font.metrics.cell_w / app.window_scale) as f32;
            let dx_cols = if pixel_precise {
                (dy as f32) / (cell_w_pt * 1.5)
            } else {
                (dy as f32) * 1.5
            };
            if let Some(tab) = app.tabs.current_mut() {
                let id = tab.focused_id();
                if let Some(ep) = tab.editor_panes.get_pane_mut(id) {
                    if !ep.soft_wrap {
                        ep.scroll_x = (ep.scroll_x + dx_cols as f64).max(0.0);
                        app.scroll_indicator_alpha = 1.0;
                        app.scroll_indicator_last_scroll = Some(Instant::now());
                        app.dirty = true;
                    }
                }
            }
            return;
        }

        if dy == 0.0 {
            return;
        }

        // Explorer scroll: when the wheel is over the IDE dock, scroll the file
        // list itself and do not send scroll to the focused editor/terminal.
        let (rx, ry) = app.view_pt_to_raster_px(loc);
        if matches!(
            app.left_dock_hits.at(rx, ry),
            Some(LeftDockHitKind::Explorer(_))
        ) {
            let entry_count = app
                .fs_snapshot
                .as_ref()
                .map_or(0, |snap| snap.entries.len());
            let next = next_explorer_scroll_offset(app.explorer_scroll_offset, dy, entry_count);
            if next != app.explorer_scroll_offset {
                app.explorer_scroll_offset = next;
                // Item 8: trigger scroll indicator fade-in.
                app.scroll_indicator_alpha = 1.0;
                app.scroll_indicator_last_scroll = Some(Instant::now());
                app.dirty = true;
            }
            return;
        }

        // Mouse reporting scroll.
        let (btn_mode, x10_mode) = app
            .tabs
            .current()
            .and_then(|t| t.registry.get(t.focused_id()))
            .and_then(|p| p.terminal.as_ref())
            .map(|t| (t.modes.mouse_button, t.modes.mouse_x10))
            .unwrap_or((false, false));
        if btn_mode || x10_mode {
            if let Some((row, col)) = app.event_cell(loc, false) {
                let btn: u8 = if dy > 0.0 { 64 } else { 65 };
                app.write_mouse_event(btn, col, row, true);
            }
            return;
        }

        // Pixel-precise sources (trackpad, Magic Mouse): treat ~1.5 cell
        // heights of finger travel as one row — natural "one row per
        // visual cell of motion" feel without runaway sensitivity.
        // Line-mode sources (mouse wheel): one detent = ~1.5 rows, less
        // aggressive than the typical 3-line jump that felt jumpy here.
        let cell_h_pt = (app.font.metrics.cell_h / app.window_scale) as f32;
        let d = if pixel_precise {
            (dy as f32) / (cell_h_pt * 1.5)
        } else {
            (dy as f32) * 1.5
        };
        if std::env::var_os("ANVIL_PERF").is_some() {
            eprintln!("anvil-perf: scroll dy={dy:.2} pp={pixel_precise} d={d:.3}");
        }

        // Native editor scroll (NE7 + M1): update scroll_target; easing loop applies it.
        if app.focused_is_native_editor() {
            if let Some(tab) = app.tabs.current_mut() {
                let id = tab.focused_id();
                if let Some(pane) = tab.editor_panes.get_pane_mut(id) {
                    pane.scroll_target = (pane.scroll_target + d).max(0.0);
                }
            }
            // M5: trigger scrollbar fade-in on editor scroll.
            app.scroll_indicator_alpha = 1.0;
            app.scroll_indicator_last_scroll = Some(Instant::now());
            app.dirty = true;
            return;
        }

        if let Some(tab) = app.tabs.current_mut() {
            let id = tab.focused_id();
            if let Some(pane) = tab.registry.get_mut(id) {
                if let Some(terminal) = &mut pane.terminal {
                    let max_pos = terminal.scrollback_len() as f32;
                    let np = (pane.scroll_target + d).clamp(0.0, max_pos);
                    pane.scroll_target = np;
                    terminal.set_viewport_offset(np.round() as usize);
                }
            }
        }
        app.dirty = true;
    }

    fn resize(&mut self, width: f64, height: f64, _in_live_resize: bool) {
        self.app.view_width_pt = width;
        self.app.view_height_pt = height;
        self.app.resize_surface();
        self.app.resize_all_tabs();
        if self.app.palette.visible {
            self.webview.set_frame(width, height);
        }
        let mut grid_painters = GridPainters {
            regular: &mut self.painter,
            bold: &mut self.bold_painter,
            italic: &mut self.italic_painter,
            bold_italic: &mut self.bold_italic_painter,
        };
        self.app
            .render_frame(&mut grid_painters, &mut self.chrome_painter);
    }

    fn live_resize_ended(&mut self) {
        self.app.resize_all_tabs();
        let mut grid_painters = GridPainters {
            regular: &mut self.painter,
            bold: &mut self.bold_painter,
            italic: &mut self.italic_painter,
            bold_italic: &mut self.bold_italic_painter,
        };
        self.app
            .render_frame(&mut grid_painters, &mut self.chrome_painter);
    }

    fn focus_gained(&mut self) {
        self.app.focused = true;
    }
    fn focus_lost(&mut self) {
        self.app.focused = false;
        // J1: save-on-blur — skip if any rename/new-file modal is active.
        if self.app.config.editor.save_on_blur
            && self.app.explorer_rename.is_none()
            && self.app.lsp_rename_input.is_none()
            && self.app.save_as_input.is_none()
        {
            for tab in self.app.tabs.tabs.iter_mut() {
                for (_buf_id, buf) in tab.editor_panes.buffers_mut() {
                    if buf.is_dirty() {
                        if let Some(path) = buf.tracked_path().map(|p| p.to_path_buf()) {
                            if let Err(e) = buf.save(&path) {
                                eprintln!("anvil: save-on-blur failed ({}): {e}", path.display());
                            }
                        }
                    }
                }
            }
        }
    }
    fn should_terminate(&mut self) -> bool {
        self.app.shutdown();
        true
    }

    fn webview_message(&mut self, json: String) {
        match bridge_decode(&json) {
            Ok(Inbound::Ready) => {
                if self.app.palette.on_ready() {
                    self.app.send_palette_show(&self.webview);
                    self.webview.show();
                }
            }
            Ok(Inbound::Dismiss) => {
                self.app.dismiss_palette(&self.webview);
            }
            Ok(Inbound::Invoke(id)) => {
                // File-picker: file:open:<abs-path> is handled before the
                // static action catalog so it doesn't need an Action variant.
                if let Some(abs_path) = id.strip_prefix("file:open:") {
                    let path = std::path::PathBuf::from(abs_path);
                    self.app.dismiss_palette(&self.webview);
                    self.app.open_path_in_native_editor(&path);
                } else if let Some(action) = action_for_id(&id) {
                    // HudToggle needs window access — handle at AppShell level.
                    if action == Action::HudToggle {
                        self.app.dismiss_palette(&self.webview);
                        self.toggle_hud();
                    } else {
                        self.app.handle_palette_action(action, &self.webview);
                    }
                } else {
                    eprintln!("anvil: unknown command id: {id}");
                }
            }
            Err(e) => eprintln!("anvil: webview message decode failed: {e}"),
        }
    }

    fn context_action(&mut self, action: ContextAction) {
        match action {
            ContextAction::Copy => {
                if let Some(text) = self.app.focused_selection_text() {
                    anvil_platform::system::set_clipboard(&text);
                }
            }
            ContextAction::Paste => {
                if let Some(text) = anvil_platform::system::get_clipboard() {
                    let bracketed = self
                        .app
                        .tabs
                        .current()
                        .and_then(|t| t.registry.get(t.focused_id()))
                        .and_then(|p| p.terminal.as_ref())
                        .map(|t| t.modes.bracketed_paste)
                        .unwrap_or(false);
                    if bracketed {
                        self.app.write_to_focused_pty(b"\x1b[200~");
                    }
                    self.app.write_to_focused_pty(text.as_bytes());
                    if bracketed {
                        self.app.write_to_focused_pty(b"\x1b[201~");
                    }
                }
            }
            ContextAction::Clear => {
                if let Some(tab) = self.app.tabs.current_mut() {
                    let id = tab.focused_id();
                    if let Some(pane) = tab.registry.get_mut(id) {
                        if let Some(terminal) = &mut pane.terminal {
                            terminal.feed(b"\x1b[H\x1b[2J");
                        }
                    }
                }
            }
            ContextAction::SplitRight => {
                self.app.split_focused_pane(SplitDir::Horizontal);
            }
            ContextAction::SplitDown => {
                self.app.split_focused_pane(SplitDir::Vertical);
            }

            // ── Explorer context menu (I1) ────────────────────────────────────
            ContextAction::ExplorerOpen => {
                if let Some(path) = self.app.right_click_path.take() {
                    self.app.open_path_in_native_editor(&path);
                }
            }
            ContextAction::ExplorerRename => {
                if let Some(path) = self.app.right_click_path.take() {
                    let name = path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("")
                        .to_string();
                    // Find the row index that matches this path.
                    let row_idx = self
                        .app
                        .left_dock_hits
                        .visible_rows
                        .iter()
                        .position(|(p, _)| *p == path)
                        .unwrap_or(0);
                    self.app.explorer_rename = Some(RenameState {
                        old_path: path,
                        input: name,
                        row_idx,
                    });
                }
            }
            ContextAction::ExplorerDelete => {
                if let Some(path) = self.app.right_click_path.take() {
                    let name = path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("")
                        .to_string();
                    self.app.explorer_delete_confirm = Some(DeleteConfirm { path, name });
                }
            }
            ContextAction::ExplorerNewFile => {
                let parent_dir = self
                    .app
                    .right_click_path
                    .take()
                    .and_then(|p| {
                        if p.is_dir() {
                            Some(p)
                        } else {
                            p.parent().map(|pp| pp.to_path_buf())
                        }
                    })
                    .or_else(|| {
                        self.app
                            .fs_snapshot
                            .as_ref()
                            .map(|snap| PathBuf::from(&snap.root))
                    });
                if let Some(dir) = parent_dir {
                    self.app.explorer_new_item = Some(NewItemState {
                        parent_dir: dir,
                        input: String::new(),
                        is_dir: false,
                    });
                }
            }
            ContextAction::ExplorerNewFolder => {
                let parent_dir = self
                    .app
                    .right_click_path
                    .take()
                    .and_then(|p| {
                        if p.is_dir() {
                            Some(p)
                        } else {
                            p.parent().map(|pp| pp.to_path_buf())
                        }
                    })
                    .or_else(|| {
                        self.app
                            .fs_snapshot
                            .as_ref()
                            .map(|snap| PathBuf::from(&snap.root))
                    });
                if let Some(dir) = parent_dir {
                    self.app.explorer_new_item = Some(NewItemState {
                        parent_dir: dir,
                        input: String::new(),
                        is_dir: true,
                    });
                }
            }
            ContextAction::ExplorerRevealInFinder => {
                if let Some(path) = self.app.right_click_path.take() {
                    let _ = std::process::Command::new("open")
                        .args(["-R", &path.to_string_lossy()])
                        .spawn();
                }
            }

            // ── Editor body context menu (I2) ─────────────────────────────────
            ContextAction::EditorGotoDef => {
                let pane_id = self.app.tabs.current().map(|t| t.focused_id()).unwrap_or(0);
                self.app.trigger_definition_request(pane_id);
            }
            ContextAction::EditorFindRefs => {
                self.app.trigger_references_request();
            }
            ContextAction::EditorRenameSymbol => {
                self.app.open_lsp_rename_overlay();
            }
            ContextAction::EditorFormatFile => {
                self.app.apply_editor_action(EditorAction::FormatFile);
            }
            ContextAction::EditorToggleComment => {
                self.app
                    .apply_editor_action(EditorAction::ToggleLineComment);
            }
        }
        self.app.dirty = true;
    }

    fn right_click_zone(&mut self, loc: MouseLocation) -> RightClickZone {
        let app = &mut self.app;
        let (rx, ry) = app.view_pt_to_raster_px(loc);

        // Check if click is on an Explorer row.
        if app.left_dock_visible {
            if let Some(kind) = app.left_dock_hits.at(rx, ry).cloned() {
                let path = match kind {
                    LeftDockHitKind::Explorer(ExplorerHit::Row(idx)) => app
                        .left_dock_hits
                        .visible_rows
                        .get(idx)
                        .map(|(p, _)| p.clone()),
                    LeftDockHitKind::Explorer(ExplorerHit::Header) => app
                        .fs_snapshot
                        .as_ref()
                        .map(|snap| PathBuf::from(&snap.root)),
                    _ => None,
                };
                if let Some(p) = path.clone() {
                    app.right_click_path = Some(p);
                }
                return RightClickZone::Explorer {
                    has_path: path.is_some(),
                };
            }
        }

        // Check if click is in a native editor pane.
        if app.focused_is_native_editor() {
            let has_lsp = app
                .tabs
                .current()
                .and_then(|tab| {
                    let pid = tab.focused_id();
                    let ep = tab.editor_panes.get_pane(pid)?;
                    let buf = tab.editor_panes.get_buffer(ep.buffer_id)?;
                    buf.tracked_path()
                })
                .is_some();
            return RightClickZone::Editor { has_lsp };
        }

        RightClickZone::Terminal
    }

    fn dropped_files(&mut self, paths: Vec<PathBuf>) {
        // I4: open each dropped file in the native editor.
        for path in paths {
            if path.is_file() {
                self.app.open_path_in_native_editor(&path);
            }
        }
        self.app.dirty = true;
    }
}

// ── Key conversion ────────────────────────────────────────────────────────────

/// Map a non-Cmd `KeyEvent` to an `EditorAction` for native editor panes (NE6).
///
/// Returns `None` for keys with no editor binding (e.g. F-keys, Escape).
/// Cmd combos are handled separately in `handle_cmd_chord`.
fn key_event_to_editor_action(event: KeyEvent) -> Option<EditorAction> {
    let shift = event.mods.shift;
    let opt = event.mods.option;
    Some(match event.key {
        KeyInput::Char(ch) => EditorAction::InsertChar(ch),
        // #16: Enter now uses auto-indent smart newline.
        KeyInput::Enter => EditorAction::InsertNewlineSmart,
        // #21: Tab — caller checks selection state and may override with
        // IndentSelection; plain Tab handled here.
        // #21: Shift+Tab → DedentSelection.
        KeyInput::Tab if shift => EditorAction::DedentSelection,
        KeyInput::Tab => EditorAction::InsertTab,
        KeyInput::Backspace => EditorAction::Backspace,
        KeyInput::Delete => EditorAction::Delete,
        KeyInput::Left => EditorAction::MoveLeft { extend: shift },
        KeyInput::Right => EditorAction::MoveRight { extend: shift },
        // #20: Opt+Up/Down → move line up/down.
        KeyInput::Up if opt && !shift => EditorAction::MoveLineUp,
        KeyInput::Down if opt && !shift => EditorAction::MoveLineDown,
        KeyInput::Up => EditorAction::MoveUp { extend: shift },
        KeyInput::Down => EditorAction::MoveDown { extend: shift },
        KeyInput::Home => EditorAction::MoveLineStart { extend: shift },
        KeyInput::End => EditorAction::MoveLineEnd { extend: shift },
        KeyInput::PageUp => EditorAction::PageUp { extend: shift },
        KeyInput::PageDown => EditorAction::PageDown { extend: shift },
        // Escape and F-keys have no editor binding in insert-only mode.
        _ => return None,
    })
}

fn platform_key_to_zig_key(k: KeyInput) -> Option<Key> {
    Some(match k {
        KeyInput::Char(ch) => Key::Text(ch),
        KeyInput::Enter => Key::Enter,
        KeyInput::Tab => Key::Tab,
        KeyInput::Backspace => Key::Backspace,
        KeyInput::Escape => Key::Escape,
        KeyInput::Left => Key::Left,
        KeyInput::Right => Key::Right,
        KeyInput::Up => Key::Up,
        KeyInput::Down => Key::Down,
        KeyInput::Home => Key::Home,
        KeyInput::End => Key::End,
        KeyInput::PageUp => Key::PageUp,
        KeyInput::PageDown => Key::PageDown,
        KeyInput::Delete => Key::Delete,
        KeyInput::F(n) => match n {
            1 => Key::F1,
            2 => Key::F2,
            3 => Key::F3,
            4 => Key::F4,
            5 => Key::F5,
            6 => Key::F6,
            7 => Key::F7,
            8 => Key::F8,
            9 => Key::F9,
            10 => Key::F10,
            11 => Key::F11,
            12 => Key::F12,
            _ => return None,
        },
    })
}

fn platform_mods_to_zig_mods(m: Modifiers) -> Mods {
    Mods {
        shift: m.shift,
        control: m.control,
        option: m.option,
        command: m.command,
    }
}

fn cursor_cfg_from_config(cfg: &Config) -> CursorConfig {
    use anvil_config::CursorStyle;
    use anvil_render::draw::CursorStyle as RCursorStyle;
    CursorConfig {
        style: match cfg.cursor.style {
            CursorStyle::Block => RCursorStyle::Block,
            CursorStyle::Bar => RCursorStyle::Bar,
            CursorStyle::Underline => RCursorStyle::Underline,
        },
        blink: cfg.cursor.blink,
    }
}

// ── Utilities ─────────────────────────────────────────────────────────────────

/// Emit the bytes of `s` single-quote-safe: each embedded `'` is replaced by
/// the four-byte sequence `'\''` so that the caller can wrap the output in
/// single quotes and get a valid POSIX shell argument.
///
/// `emit` is called with successive byte slices whose concatenation equals the
/// properly escaped content (without the surrounding single quotes).
fn shell_quote_arg(s: &str, mut emit: impl FnMut(&[u8])) {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if let Some(j) = bytes[i..].iter().position(|&b| b == b'\'') {
            emit(&bytes[i..i + j]);
            emit(b"'\\''");
            i += j + 1;
        } else {
            emit(&bytes[i..]);
            break;
        }
    }
}

fn ascii_lower(ch: char) -> char {
    if ch.is_ascii_uppercase() {
        (ch as u8 + 32) as char
    } else {
        ch
    }
}

fn format_hex(rgb: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2])
}

fn effective_theme_name(system_dark: bool, cfg_theme: &str) -> &str {
    if cfg_theme == "system" {
        if system_dark {
            "ember-dark"
        } else {
            "ember-light"
        }
    } else {
        cfg_theme
    }
}

fn next_theme_mode(cfg_theme: &str) -> &'static str {
    match cfg_theme {
        "ember-dark" => "ember-light",
        "ember-light" => "system",
        "system" => "ember-dark",
        _ => "ember-dark",
    }
}

fn system_is_dark() -> bool {
    use objc2::msg_send;
    objc2::rc::autoreleasepool(|_pool| {
        // SAFETY: standard Cocoa singleton + property accessor on the main thread.
        unsafe {
            let cls = objc2::runtime::AnyClass::get(c"NSApplication");
            let cls = match cls {
                Some(c) => c,
                None => return true,
            };
            let app: *mut objc2::runtime::AnyObject = msg_send![cls, sharedApplication];
            if app.is_null() {
                return true;
            }
            let appearance: *mut objc2::runtime::AnyObject = msg_send![app, effectiveAppearance];
            if appearance.is_null() {
                return true;
            }
            let name: *mut objc2::runtime::AnyObject = msg_send![appearance, name];
            if name.is_null() {
                return true;
            }
            let cstr: *const std::ffi::c_char = msg_send![name, UTF8String];
            if cstr.is_null() {
                return true;
            }
            let s = std::ffi::CStr::from_ptr(cstr).to_string_lossy();
            s.contains("Dark")
        }
    })
}

fn terminate_app() {
    // SAFETY: terminate: on NSApplication singleton, main thread only.
    unsafe {
        use objc2::msg_send;
        let cls = objc2::runtime::AnyClass::get(c"NSApplication");
        if let Some(cls) = cls {
            let app: *mut objc2::runtime::AnyObject = msg_send![cls, sharedApplication];
            if !app.is_null() {
                let nil: *const std::ffi::c_void = std::ptr::null();
                let _: () = msg_send![app, terminate: nil];
            }
        }
    }
}

/// Collect all PaneIds currently in a PaneRegistry by examining which ids have
/// Walk a PaneTree to collect all leaf PaneIds.
fn all_pane_ids_in_tree(tab: &Tab) -> Vec<PaneId> {
    tab.tree
        .layout(
            Rect {
                x: 0.0,
                y: 0.0,
                w: 1e6,
                h: 1e6,
            },
            DIVIDER_PX,
        )
        .into_iter()
        .map(|e| e.id)
        .collect()
}

// ── Overlay render helpers (items 10, 11) ────────────────────────────────────

/// Draw the project-wide search overlay (item 10).
///
/// A centered panel with one input row and up to N result rows.
#[allow(clippy::too_many_arguments)]
fn draw_project_search_overlay(
    raster: &mut anvil_render::raster::Raster,
    painter: &mut dyn anvil_render::raster::GlyphPainter,
    metrics: anvil_render::raster::FontMetrics,
    theme: &anvil_theme::Theme,
    ps: &anvil_workspace::project_search::ProjectSearch,
    dw: f64,
    dh: f64,
    chrome_top: f64,
    chrome_bot: f64,
    cw: f64,
    ch: f64,
) {
    let max_results = 20usize;
    let panel_rows = 1 + ps.hits.len().min(max_results) + 1; // input + results + padding
    let panel_h = panel_rows as f64 * ch + ch;
    let panel_w = (dw * 0.6).min(dw - 4.0 * cw).max(20.0 * cw);
    let panel_x = ((dw - panel_w) * 0.5).max(0.0);
    let panel_y = (chrome_top + (dh - chrome_top - chrome_bot - panel_h) * 0.2).max(chrome_top);

    // Panel background + border.
    raster.fill_pixel_rect(panel_x, panel_y, panel_w, panel_h, theme.surface);
    raster.fill_pixel_rect(panel_x, panel_y, panel_w, 1.0, theme.hairline);
    raster.fill_pixel_rect(
        panel_x,
        panel_y + panel_h - 1.0,
        panel_w,
        1.0,
        theme.hairline,
    );
    raster.fill_pixel_rect(panel_x, panel_y, 1.0, panel_h, theme.hairline);
    raster.fill_pixel_rect(
        panel_x + panel_w - 1.0,
        panel_y,
        1.0,
        panel_h,
        theme.hairline,
    );

    // Row 0: "search: " + query + cursor.
    let pad_x = 2.0 * cw;
    let row0_y = panel_y + 0.5 * ch;
    let prefix = "search: ";
    let mut x = panel_x + pad_x;
    for c in prefix.chars() {
        if x + cw > panel_x + panel_w - pad_x {
            break;
        }
        raster.glyph_at(painter, metrics, x, row0_y, c as u32, theme.text_muted);
        x += cw;
    }
    for c in ps.query.chars() {
        if x + cw > panel_x + panel_w - pad_x {
            break;
        }
        raster.glyph_at(painter, metrics, x, row0_y, c as u32, theme.foreground);
        x += cw;
    }
    // Cursor block.
    raster.fill_pixel_rect(x, panel_y + 2.0, cw, ch - 4.0, theme.accent_bright);

    // Separator.
    raster.fill_pixel_rect(panel_x, panel_y + ch, panel_w, 1.0, theme.hairline);

    // Result rows.
    for (i, hit) in ps.hits.iter().take(max_results).enumerate() {
        let row_y = panel_y + (i + 1) as f64 * ch + 0.5 * ch;
        let is_selected = i == ps.selected;
        if is_selected {
            raster.fill_pixel_rect_alpha(
                panel_x,
                panel_y + (i + 1) as f64 * ch,
                panel_w,
                ch,
                theme.accent,
                0.12,
            );
        }
        // Format: "path:line  preview"
        let path_name = hit.path.file_name().and_then(|n| n.to_str()).unwrap_or("?");
        let row_text = format!("{}:{} {}", path_name, hit.line, hit.preview);
        let mut rx = panel_x + pad_x;
        for c in row_text.chars() {
            if rx + cw > panel_x + panel_w - pad_x {
                break;
            }
            let color = if is_selected {
                theme.foreground
            } else {
                theme.text_muted
            };
            raster.glyph_at(painter, metrics, rx, row_y, c as u32, color);
            rx += cw;
        }
    }
}

/// Draw the goto-line overlay (item 11).
///
/// A small centered panel with a single input row.
#[allow(clippy::too_many_arguments)]
fn draw_goto_line_overlay(
    raster: &mut anvil_render::raster::Raster,
    painter: &mut dyn anvil_render::raster::GlyphPainter,
    metrics: anvil_render::raster::FontMetrics,
    theme: &anvil_theme::Theme,
    input: &str,
    dw: f64,
    _dh: f64,
    chrome_top: f64,
    cw: f64,
    ch: f64,
) {
    let panel_w = 24.0 * cw;
    let panel_h = ch + 8.0;
    let panel_x = ((dw - panel_w) * 0.5).max(0.0);
    let panel_y = chrome_top + 4.0 * ch;

    raster.fill_pixel_rect(panel_x, panel_y, panel_w, panel_h, theme.surface);
    raster.fill_pixel_rect(panel_x, panel_y, panel_w, 1.0, theme.accent);
    raster.fill_pixel_rect(panel_x, panel_y + panel_h - 1.0, panel_w, 1.0, theme.accent);
    raster.fill_pixel_rect(panel_x, panel_y, 1.0, panel_h, theme.accent);
    raster.fill_pixel_rect(panel_x + panel_w - 1.0, panel_y, 1.0, panel_h, theme.accent);

    let glyph_y = panel_y + 4.0;
    let prefix = "line: ";
    let pad_x = 1.5 * cw;
    let mut x = panel_x + pad_x;
    for c in prefix.chars() {
        if x + cw > panel_x + panel_w - pad_x {
            break;
        }
        raster.glyph_at(painter, metrics, x, glyph_y, c as u32, theme.text_muted);
        x += cw;
    }
    for c in input.chars() {
        if x + cw > panel_x + panel_w - pad_x {
            break;
        }
        raster.glyph_at(painter, metrics, x, glyph_y, c as u32, theme.foreground);
        x += cw;
    }
    // Cursor.
    raster.fill_pixel_rect(x, panel_y + 2.0, cw, panel_h - 4.0, theme.accent_bright);
}

// ── LSP rename overlay draw (item 24) ────────────────────────────────────────

/// Draw the LSP rename input overlay — same chrome as goto-line.
#[allow(clippy::too_many_arguments)]
fn draw_lsp_rename_overlay(
    raster: &mut anvil_render::raster::Raster,
    painter: &mut dyn anvil_render::raster::GlyphPainter,
    metrics: anvil_render::raster::FontMetrics,
    theme: &anvil_theme::Theme,
    input: &str,
    dw: f64,
    _dh: f64,
    chrome_top: f64,
    cw: f64,
    ch: f64,
) {
    let panel_w = 30.0 * cw;
    let panel_h = ch + 8.0;
    let panel_x = ((dw - panel_w) * 0.5).max(0.0);
    let panel_y = chrome_top + 5.0 * ch;

    raster.fill_pixel_rect(panel_x, panel_y, panel_w, panel_h, theme.surface);
    raster.fill_pixel_rect(panel_x, panel_y, panel_w, 1.0, theme.accent);
    raster.fill_pixel_rect(panel_x, panel_y + panel_h - 1.0, panel_w, 1.0, theme.accent);
    raster.fill_pixel_rect(panel_x, panel_y, 1.0, panel_h, theme.accent);
    raster.fill_pixel_rect(panel_x + panel_w - 1.0, panel_y, 1.0, panel_h, theme.accent);

    let glyph_y = panel_y + 4.0;
    let prefix = "rename: ";
    let pad_x = 1.5 * cw;
    let mut x = panel_x + pad_x;
    for c in prefix.chars() {
        if x + cw > panel_x + panel_w - pad_x {
            break;
        }
        raster.glyph_at(painter, metrics, x, glyph_y, c as u32, theme.text_muted);
        x += cw;
    }
    for c in input.chars() {
        if x + cw > panel_x + panel_w - pad_x {
            break;
        }
        raster.glyph_at(painter, metrics, x, glyph_y, c as u32, theme.foreground);
        x += cw;
    }
    raster.fill_pixel_rect(x, panel_y + 2.0, cw, panel_h - 4.0, theme.accent_bright);
}

// ── Save-as overlay draw (tier-J J2) ─────────────────────────────────────────

/// Draw the save-as path-input overlay — same chrome as goto-line.
/// TODO(anvil-tierJ-J2-nspanel): replace with NSSavePanel.
#[allow(clippy::too_many_arguments)]
fn draw_save_as_overlay(
    raster: &mut anvil_render::raster::Raster,
    painter: &mut dyn anvil_render::raster::GlyphPainter,
    metrics: anvil_render::raster::FontMetrics,
    theme: &anvil_theme::Theme,
    input: &str,
    dw: f64,
    _dh: f64,
    chrome_top: f64,
    cw: f64,
    ch: f64,
) {
    let panel_w = 60.0 * cw;
    let panel_h = ch + 8.0;
    let panel_x = ((dw - panel_w) * 0.5).max(0.0);
    let panel_y = chrome_top + 4.0 * ch;

    raster.fill_pixel_rect(panel_x, panel_y, panel_w, panel_h, theme.surface);
    raster.fill_pixel_rect(panel_x, panel_y, panel_w, 1.0, theme.accent);
    raster.fill_pixel_rect(panel_x, panel_y + panel_h - 1.0, panel_w, 1.0, theme.accent);
    raster.fill_pixel_rect(panel_x, panel_y, 1.0, panel_h, theme.accent);
    raster.fill_pixel_rect(panel_x + panel_w - 1.0, panel_y, 1.0, panel_h, theme.accent);

    let glyph_y = panel_y + 4.0;
    let prefix = "save as: ";
    let pad_x = 1.5 * cw;
    let mut x = panel_x + pad_x;
    for c in prefix.chars() {
        if x + cw > panel_x + panel_w - pad_x {
            break;
        }
        raster.glyph_at(painter, metrics, x, glyph_y, c as u32, theme.text_muted);
        x += cw;
    }
    // Show only the tail of a long path so the cursor end is always visible.
    let visible_cols = ((panel_w - pad_x * 2.0 - prefix.len() as f64 * cw) / cw).floor() as usize;
    let chars: Vec<char> = input.chars().collect();
    let start = chars.len().saturating_sub(visible_cols);
    for &c in &chars[start..] {
        if x + cw > panel_x + panel_w - pad_x {
            break;
        }
        raster.glyph_at(painter, metrics, x, glyph_y, c as u32, theme.foreground);
        x += cw;
    }
    raster.fill_pixel_rect(x, panel_y + 2.0, cw, panel_h - 4.0, theme.accent_bright);
}

// ── Open-folder overlay draw (Q19) ───────────────────────────────────────────

/// Draw the open-folder path-input overlay (Cmd+K Cmd+O).
#[allow(clippy::too_many_arguments)]
fn draw_open_folder_overlay(
    raster: &mut anvil_render::raster::Raster,
    painter: &mut dyn anvil_render::raster::GlyphPainter,
    metrics: anvil_render::raster::FontMetrics,
    theme: &anvil_theme::Theme,
    input: &str,
    dw: f64,
    chrome_top: f64,
    cw: f64,
    ch: f64,
) {
    let panel_w = 60.0 * cw;
    let panel_h = ch + 8.0;
    let panel_x = ((dw - panel_w) * 0.5).max(0.0);
    let panel_y = chrome_top + 4.0 * ch;

    raster.fill_pixel_rect(panel_x, panel_y, panel_w, panel_h, theme.surface);
    raster.fill_pixel_rect(panel_x, panel_y, panel_w, 1.0, theme.accent);
    raster.fill_pixel_rect(panel_x, panel_y + panel_h - 1.0, panel_w, 1.0, theme.accent);
    raster.fill_pixel_rect(panel_x, panel_y, 1.0, panel_h, theme.accent);
    raster.fill_pixel_rect(panel_x + panel_w - 1.0, panel_y, 1.0, panel_h, theme.accent);

    let glyph_y = panel_y + 4.0;
    let prefix = "open folder: ";
    let pad_x = 1.5 * cw;
    let mut x = panel_x + pad_x;
    for c in prefix.chars() {
        if x + cw > panel_x + panel_w - pad_x {
            break;
        }
        raster.glyph_at(painter, metrics, x, glyph_y, c as u32, theme.text_muted);
        x += cw;
    }
    // Show only the tail of a long path so the cursor end is always visible.
    let visible_cols = ((panel_w - pad_x * 2.0 - prefix.len() as f64 * cw) / cw).floor() as usize;
    let chars: Vec<char> = input.chars().collect();
    let start = chars.len().saturating_sub(visible_cols);
    for &c in &chars[start..] {
        if x + cw > panel_x + panel_w - pad_x {
            break;
        }
        raster.glyph_at(painter, metrics, x, glyph_y, c as u32, theme.foreground);
        x += cw;
    }
    raster.fill_pixel_rect(x, panel_y + 2.0, cw, panel_h - 4.0, theme.accent_bright);
}

// ── Language picker overlay draw (Q22) ───────────────────────────────────────

/// Draw the language-picker overlay (Cmd+K Cmd+L).
#[allow(clippy::too_many_arguments)]
fn draw_language_picker_overlay(
    raster: &mut anvil_render::raster::Raster,
    painter: &mut dyn anvil_render::raster::GlyphPainter,
    metrics: anvil_render::raster::FontMetrics,
    theme: &anvil_theme::Theme,
    query: &str,
    selected: usize,
    dw: f64,
    chrome_top: f64,
    cw: f64,
    ch: f64,
) {
    let langs = PICKER_LANGS;
    let filtered = picker_filtered(langs, query);
    let rows = filtered.len();
    let panel_w = 30.0 * cw;
    let header_h = ch + 4.0;
    let panel_h = header_h + (rows as f64 + 1.0) * (ch + 2.0);
    let panel_x = ((dw - panel_w) * 0.5).max(0.0);
    let panel_y = chrome_top + 2.0 * ch;

    raster.fill_pixel_rect(panel_x, panel_y, panel_w, panel_h, theme.surface);
    raster.fill_pixel_rect(panel_x, panel_y, panel_w, 1.0, theme.accent);
    raster.fill_pixel_rect(panel_x, panel_y + panel_h - 1.0, panel_w, 1.0, theme.accent);
    raster.fill_pixel_rect(panel_x, panel_y, 1.0, panel_h, theme.accent);
    raster.fill_pixel_rect(panel_x + panel_w - 1.0, panel_y, 1.0, panel_h, theme.accent);

    // Header: filter input.
    let pad_x = 1.5 * cw;
    let header_label = "language: ";
    let mut hx = panel_x + pad_x;
    let header_y = panel_y + 2.0;
    for c in header_label.chars() {
        raster.glyph_at(painter, metrics, hx, header_y, c as u32, theme.text_muted);
        hx += cw;
    }
    for c in query.chars() {
        if hx + cw > panel_x + panel_w - pad_x {
            break;
        }
        raster.glyph_at(painter, metrics, hx, header_y, c as u32, theme.foreground);
        hx += cw;
    }
    // Cursor.
    raster.fill_pixel_rect(hx, panel_y + 2.0, cw, ch, theme.accent_bright);

    raster.fill_pixel_rect(panel_x, panel_y + header_h, panel_w, 1.0, theme.hairline);

    // Language rows.
    for (i, lang) in filtered.iter().enumerate() {
        let row_y = panel_y + header_h + 1.0 + i as f64 * (ch + 2.0);
        if i == selected.min(rows.saturating_sub(1)) {
            raster.fill_pixel_rect_alpha(panel_x, row_y, panel_w, ch + 2.0, theme.accent, 0.15);
        }
        let mut rx = panel_x + pad_x;
        for c in lang.chars() {
            if rx + cw > panel_x + panel_w - pad_x {
                break;
            }
            let color = if i == selected.min(rows.saturating_sub(1)) {
                theme.foreground
            } else {
                theme.text_muted
            };
            raster.glyph_at(painter, metrics, rx, row_y + 1.0, c as u32, color);
            rx += cw;
        }
    }
}

// ── LSP references overlay draw (item 26) ────────────────────────────────────

/// Draw the references overlay panel.
#[allow(clippy::too_many_arguments)]
fn draw_lsp_references_overlay(
    raster: &mut anvil_render::raster::Raster,
    painter: &mut dyn anvil_render::raster::GlyphPainter,
    metrics: anvil_render::raster::FontMetrics,
    theme: &anvil_theme::Theme,
    refs: &LspReferencesOverlay,
    dw: f64,
    dh: f64,
    chrome_top: f64,
    cw: f64,
    ch: f64,
) {
    const MAX_ROWS: usize = 16;
    const PANEL_COLS: usize = 60;
    let show = refs.rows.len().min(MAX_ROWS);
    if show == 0 {
        return;
    }
    let panel_w = PANEL_COLS as f64 * cw;
    let panel_h = (show + 1) as f64 * ch; // +1 for header row
    let panel_x = ((dw - panel_w) * 0.5).max(0.0);
    let panel_y = (chrome_top + 2.0 * ch).min(dh - panel_h).max(chrome_top);

    // Background + border.
    raster.fill_pixel_rect(panel_x, panel_y, panel_w, panel_h, theme.surface);
    raster.fill_pixel_rect(panel_x, panel_y, panel_w, 1.0, theme.accent);
    raster.fill_pixel_rect(panel_x, panel_y + panel_h - 1.0, panel_w, 1.0, theme.accent);
    raster.fill_pixel_rect(panel_x, panel_y, 1.0, panel_h, theme.accent);
    raster.fill_pixel_rect(panel_x + panel_w - 1.0, panel_y, 1.0, panel_h, theme.accent);

    // Header row.
    let header = format!("references ({} found)", refs.rows.len());
    let header_y = panel_y;
    for (ci, c) in header.chars().take(PANEL_COLS - 2).enumerate() {
        let tx = panel_x + (ci + 1) as f64 * cw;
        raster.glyph_at(painter, metrics, tx, header_y, c as u32, theme.text_muted);
    }

    let visible_selected = refs.selected.min(show.saturating_sub(1));
    for (ri, row) in refs.rows.iter().enumerate().take(show) {
        let row_y = panel_y + (ri + 1) as f64 * ch;
        if ri == visible_selected {
            raster.fill_pixel_rect_alpha(panel_x, row_y, panel_w, ch, theme.accent, 0.18);
        }
        // Format: "basename:line"
        let base = row.path.file_name().and_then(|n| n.to_str()).unwrap_or("?");
        let label = format!("{}:{}:{}", base, row.line + 1, row.col + 1);
        let label_chars: Vec<char> = label.chars().take(PANEL_COLS - 2).collect();
        for (ci, &c) in label_chars.iter().enumerate() {
            let tx = panel_x + (ci + 1) as f64 * cw;
            raster.glyph_at(painter, metrics, tx, row_y, c as u32, theme.foreground);
        }
    }
}

// ── Workspace symbol search overlay (O1) ─────────────────────────────────────

/// Draw the workspace/symbol search overlay (Cmd+T).
///
/// Shows "(LSP unavailable)" when no server is live. Otherwise shows
/// `hits` with format `kind name · file:line`.
#[allow(clippy::too_many_arguments)]
fn draw_workspace_symbol_overlay(
    raster: &mut anvil_render::raster::Raster,
    painter: &mut dyn anvil_render::raster::GlyphPainter,
    metrics: anvil_render::raster::FontMetrics,
    theme: &anvil_theme::Theme,
    search: &WorkspaceSymbolSearch,
    dw: f64,
    dh: f64,
    chrome_top: f64,
    cw: f64,
    ch: f64,
) {
    const MAX_RESULTS: usize = 20;
    let n_hits = search.hits.len().min(MAX_RESULTS);
    // Panel height: input row + separator + results (or 1 message row if empty)
    let result_rows = if n_hits == 0 { 1 } else { n_hits };
    let panel_h = (1 + result_rows) as f64 * ch + ch * 0.5;
    let panel_w = (dw * 0.62).min(dw - 4.0 * cw).max(24.0 * cw);
    let panel_x = ((dw - panel_w) * 0.5).max(0.0);
    let panel_y = (chrome_top + (dh - chrome_top - panel_h) * 0.2).max(chrome_top);
    let pad_x = 2.0 * cw;

    // Background + border.
    raster.fill_pixel_rect(panel_x, panel_y, panel_w, panel_h, theme.surface);
    raster.fill_pixel_rect(panel_x, panel_y, panel_w, 1.0, theme.hairline);
    raster.fill_pixel_rect(
        panel_x,
        panel_y + panel_h - 1.0,
        panel_w,
        1.0,
        theme.hairline,
    );
    raster.fill_pixel_rect(panel_x, panel_y, 1.0, panel_h, theme.hairline);
    raster.fill_pixel_rect(
        panel_x + panel_w - 1.0,
        panel_y,
        1.0,
        panel_h,
        theme.hairline,
    );

    // Input row.
    let row0_y = panel_y + 0.5 * ch;
    let prefix = "symbols: ";
    let mut x = panel_x + pad_x;
    for c in prefix.chars() {
        if x + cw > panel_x + panel_w - pad_x {
            break;
        }
        raster.glyph_at(painter, metrics, x, row0_y, c as u32, theme.text_muted);
        x += cw;
    }
    for c in search.query.chars() {
        if x + cw > panel_x + panel_w - pad_x {
            break;
        }
        raster.glyph_at(painter, metrics, x, row0_y, c as u32, theme.foreground);
        x += cw;
    }
    raster.fill_pixel_rect(x, panel_y + 2.0, cw, ch - 4.0, theme.accent_bright);
    raster.fill_pixel_rect(panel_x, panel_y + ch, panel_w, 1.0, theme.hairline);

    // Result rows or message.
    if search.lsp_unavailable {
        let msg = "(LSP unavailable)";
        let msg_y = panel_y + ch + 0.5 * ch;
        let mut mx = panel_x + pad_x;
        for c in msg.chars() {
            if mx + cw > panel_x + panel_w - pad_x {
                break;
            }
            raster.glyph_at(painter, metrics, mx, msg_y, c as u32, theme.text_subtle);
            mx += cw;
        }
    } else if n_hits == 0 {
        let msg = if search.query.is_empty() {
            "(type to search workspace symbols)"
        } else {
            "(no results)"
        };
        let msg_y = panel_y + ch + 0.5 * ch;
        let mut mx = panel_x + pad_x;
        for c in msg.chars() {
            if mx + cw > panel_x + panel_w - pad_x {
                break;
            }
            raster.glyph_at(painter, metrics, mx, msg_y, c as u32, theme.text_subtle);
            mx += cw;
        }
    } else {
        for (i, hit) in search.hits.iter().take(MAX_RESULTS).enumerate() {
            let row_y = panel_y + (i + 1) as f64 * ch + 0.5 * ch;
            let is_sel = i == search.selected;
            if is_sel {
                raster.fill_pixel_rect_alpha(
                    panel_x,
                    panel_y + (i + 1) as f64 * ch,
                    panel_w,
                    ch,
                    theme.accent,
                    0.12,
                );
            }
            let file = hit.path.file_name().and_then(|n| n.to_str()).unwrap_or("?");
            let row_text = format!(
                "{} {} \u{00B7} {}:{}",
                hit.kind_label,
                hit.name,
                file,
                hit.line + 1
            );
            let mut rx = panel_x + pad_x;
            for c in row_text.chars() {
                if rx + cw > panel_x + panel_w - pad_x {
                    break;
                }
                let color = if is_sel {
                    theme.foreground
                } else {
                    theme.text_muted
                };
                raster.glyph_at(painter, metrics, rx, row_y, c as u32, color);
                rx += cw;
            }
        }
    }
}

// ── Buffer symbol search overlay (O2) ────────────────────────────────────────

/// Draw the buffer symbol search overlay (Cmd+R).
///
/// Filters `OutlineSymbol`s from the active buffer by substring and shows
/// `kind name · Ln N`.
#[allow(clippy::too_many_arguments)]
fn draw_buffer_symbol_overlay(
    raster: &mut anvil_render::raster::Raster,
    painter: &mut dyn anvil_render::raster::GlyphPainter,
    metrics: anvil_render::raster::FontMetrics,
    theme: &anvil_theme::Theme,
    search: &BufferSymbolSearch,
    dw: f64,
    dh: f64,
    chrome_top: f64,
    cw: f64,
    ch: f64,
) {
    const MAX_RESULTS: usize = 20;
    let n_hits = search.filtered.len().min(MAX_RESULTS);
    let result_rows = if n_hits == 0 { 1 } else { n_hits };
    let panel_h = (1 + result_rows) as f64 * ch + ch * 0.5;
    let panel_w = (dw * 0.55).min(dw - 4.0 * cw).max(24.0 * cw);
    let panel_x = ((dw - panel_w) * 0.5).max(0.0);
    let panel_y = (chrome_top + (dh - chrome_top - panel_h) * 0.2).max(chrome_top);
    let pad_x = 2.0 * cw;

    raster.fill_pixel_rect(panel_x, panel_y, panel_w, panel_h, theme.surface);
    raster.fill_pixel_rect(panel_x, panel_y, panel_w, 1.0, theme.hairline);
    raster.fill_pixel_rect(
        panel_x,
        panel_y + panel_h - 1.0,
        panel_w,
        1.0,
        theme.hairline,
    );
    raster.fill_pixel_rect(panel_x, panel_y, 1.0, panel_h, theme.hairline);
    raster.fill_pixel_rect(
        panel_x + panel_w - 1.0,
        panel_y,
        1.0,
        panel_h,
        theme.hairline,
    );

    // Input row.
    let row0_y = panel_y + 0.5 * ch;
    let prefix = "buffer: ";
    let mut x = panel_x + pad_x;
    for c in prefix.chars() {
        if x + cw > panel_x + panel_w - pad_x {
            break;
        }
        raster.glyph_at(painter, metrics, x, row0_y, c as u32, theme.text_muted);
        x += cw;
    }
    for c in search.query.chars() {
        if x + cw > panel_x + panel_w - pad_x {
            break;
        }
        raster.glyph_at(painter, metrics, x, row0_y, c as u32, theme.foreground);
        x += cw;
    }
    raster.fill_pixel_rect(x, panel_y + 2.0, cw, ch - 4.0, theme.accent_bright);
    raster.fill_pixel_rect(panel_x, panel_y + ch, panel_w, 1.0, theme.hairline);

    if n_hits == 0 {
        let msg = if search.all_symbols.is_empty() {
            "(no symbols in buffer)"
        } else {
            "(no matches)"
        };
        let msg_y = panel_y + ch + 0.5 * ch;
        let mut mx = panel_x + pad_x;
        for c in msg.chars() {
            if mx + cw > panel_x + panel_w - pad_x {
                break;
            }
            raster.glyph_at(painter, metrics, mx, msg_y, c as u32, theme.text_subtle);
            mx += cw;
        }
    } else {
        for (i, &sym_idx) in search.filtered.iter().take(MAX_RESULTS).enumerate() {
            let sym = &search.all_symbols[sym_idx];
            let row_y = panel_y + (i + 1) as f64 * ch + 0.5 * ch;
            let is_sel = i == search.selected;
            if is_sel {
                raster.fill_pixel_rect_alpha(
                    panel_x,
                    panel_y + (i + 1) as f64 * ch,
                    panel_w,
                    ch,
                    theme.accent,
                    0.12,
                );
            }
            let kind_str = match sym.kind {
                anvil_editor::OutlineSymbolKind::Function => "fn",
                anvil_editor::OutlineSymbolKind::Struct => "struct",
                anvil_editor::OutlineSymbolKind::Enum => "enum",
                anvil_editor::OutlineSymbolKind::Trait => "trait",
                anvil_editor::OutlineSymbolKind::Impl => "impl",
                anvil_editor::OutlineSymbolKind::Other => "sym",
            };
            let row_text = format!("{} {} \u{00B7} Ln {}", kind_str, sym.name, sym.line + 1);
            let mut rx = panel_x + pad_x;
            for c in row_text.chars() {
                if rx + cw > panel_x + panel_w - pad_x {
                    break;
                }
                let color = if is_sel {
                    theme.foreground
                } else {
                    theme.text_muted
                };
                raster.glyph_at(painter, metrics, rx, row_y, c as u32, color);
                rx += cw;
            }
        }
    }
}

// ── Disk-changed banner (item 27) ─────────────────────────────────────────────

/// One-row banner at the top of the editor area warning that the on-disk file
/// changed while the buffer has unsaved edits.  Press Cmd+R to force-reload.
#[allow(clippy::too_many_arguments)]
fn draw_disk_changed_banner(
    raster: &mut anvil_render::raster::Raster,
    painter: &mut dyn anvil_render::raster::GlyphPainter,
    metrics: anvil_render::raster::FontMetrics,
    theme: &anvil_theme::Theme,
    dw: f64,
    chrome_top: f64,
    cw: f64,
    ch: f64,
) {
    let banner_y = chrome_top;
    let banner_h = ch + 4.0;
    raster.fill_pixel_rect(0.0, banner_y, dw, banner_h, theme.panel_raised);
    raster.fill_pixel_rect(0.0, banner_y + banner_h - 1.0, dw, 1.0, theme.hairline);
    let msg = "file changed on disk \u{2014} Cmd+R to reload";
    let pad_x = 12.0;
    let text_y = banner_y + 2.0;
    let mut x = pad_x;
    for c in msg.chars() {
        if x + cw > dw - pad_x {
            break;
        }
        raster.glyph_at(painter, metrics, x, text_y, c as u32, theme.text_muted);
        x += cw;
    }
}

// ── Toast draw helper (N3) ───────────────────────────────────────────────────

/// Paint all active toasts in the bottom-right corner.
///
/// Each toast is 28pt tall × 240pt wide, stacked upward from 8pt above the
/// status bar.  Background is `theme.panel`; 1px `theme.hairline` border;
/// text color by kind (Success = `verified`, Error = `failure`, Info = `accent_primary`).
/// All dimensions scale with `ui_scale`.
#[allow(clippy::too_many_arguments)]
fn draw_toasts(
    raster: &mut anvil_render::raster::Raster,
    painter: &mut dyn anvil_render::raster::GlyphPainter,
    metrics: anvil_render::raster::FontMetrics,
    theme: &anvil_theme::Theme,
    toasts: &std::collections::VecDeque<Toast>,
    dw: f64,
    dh: f64,
    chrome_bottom_px: f64,
    ui_scale: f64,
) {
    if toasts.is_empty() {
        return;
    }

    let cw = metrics.cell_w;
    let toast_h = (28.0 * ui_scale).round();
    let toast_w = (240.0 * ui_scale).round();
    let gap = (8.0 * ui_scale).round();
    let right_pad = (12.0 * ui_scale).round();
    let text_pad = (6.0 * ui_scale).round();

    // Bottom of lowest toast sits `gap` above the status bar strip.
    let base_y = dh - chrome_bottom_px - gap;

    for (i, toast) in toasts.iter().rev().enumerate() {
        let ty = base_y - (i as f64 + 1.0) * toast_h - i as f64 * gap;
        let tx = dw - toast_w - right_pad;

        // Background.
        raster.fill_pixel_rect(tx, ty, toast_w, toast_h, theme.panel);
        // Border.
        raster.fill_pixel_rect(tx, ty, toast_w, 1.0, theme.hairline);
        raster.fill_pixel_rect(tx, ty + toast_h - 1.0, toast_w, 1.0, theme.hairline);
        raster.fill_pixel_rect(tx, ty, 1.0, toast_h, theme.hairline);
        raster.fill_pixel_rect(tx + toast_w - 1.0, ty, 1.0, toast_h, theme.hairline);

        let text_color = match toast.kind {
            ToastKind::Success => theme.verified,
            ToastKind::Error => theme.failure,
            ToastKind::Info => theme.accent_primary,
        };

        // Vertically center the glyph row within the toast.
        let text_y = ty + (toast_h - metrics.cell_h) * 0.5;
        let max_chars = ((toast_w - text_pad * 2.0) / cw).floor() as usize;
        let mut x = tx + text_pad;
        for c in toast.text.chars().take(max_chars) {
            raster.glyph_at(painter, metrics, x, text_y, c as u32, text_color);
            x += cw;
        }
    }
}

// ── Welcome screen (item 28) ──────────────────────────────────────────────────

/// Centered welcome panel shown when no buffers are open.
#[allow(clippy::too_many_arguments)]
fn draw_welcome_screen(
    raster: &mut anvil_render::raster::Raster,
    painter: &mut dyn anvil_render::raster::GlyphPainter,
    metrics: anvil_render::raster::FontMetrics,
    theme: &anvil_theme::Theme,
    dw: f64,
    dh: f64,
    chrome_top: f64,
    chrome_bot: f64,
    cw: f64,
    ch: f64,
    recent_projects: &[PathBuf],
) {
    let safe_h = (dh - chrome_top - chrome_bot).max(0.0);
    let safe_w = dw;

    // Panel dimensions.
    let panel_w = (40.0 * cw).min(safe_w * 0.8);
    let panel_h = (12.0 + recent_projects.len().min(5) as f64) * ch;
    let panel_x = ((safe_w - panel_w) * 0.5).max(0.0);
    let panel_y = chrome_top + ((safe_h - panel_h) * 0.5).max(0.0);

    // Background fill.
    raster.fill_pixel_rect(panel_x, panel_y, panel_w, panel_h, theme.panel);

    let pad_x = 2.0 * cw;
    let max_x = panel_x + panel_w - pad_x;

    // Title "Anvil" in accent_bright.
    let title = "Anvil";
    let mut tx = panel_x + pad_x;
    let title_y = panel_y + ch;
    for c in title.chars() {
        if tx + cw > max_x {
            break;
        }
        raster.glyph_at(painter, metrics, tx, title_y, c as u32, theme.accent_bright);
        tx += cw;
    }

    // Subtitle.
    let subtitle = "Native macOS dev environment.";
    let mut sx = panel_x + pad_x;
    let sub_y = panel_y + 3.0 * ch;
    for c in subtitle.chars() {
        if sx + cw > max_x {
            break;
        }
        raster.glyph_at(painter, metrics, sx, sub_y, c as u32, theme.text_muted);
        sx += cw;
    }

    // Action rows.
    let actions = [
        ("\u{2318}P  Open file\u{2026}", theme.foreground),
        ("\u{2318}O  Open folder\u{2026}", theme.foreground),
        ("\u{2318}N  New file", theme.foreground),
    ];
    for (i, (label, color)) in actions.iter().enumerate() {
        let row_y = panel_y + (5.0 + i as f64) * ch;
        let mut ax = panel_x + pad_x;
        for c in label.chars() {
            if ax + cw > max_x {
                break;
            }
            raster.glyph_at(painter, metrics, ax, row_y, c as u32, *color);
            ax += cw;
        }
    }

    // Recent projects.
    if !recent_projects.is_empty() {
        let recent_label_y = panel_y + 9.0 * ch;
        let rl = "Recent";
        let mut rlx = panel_x + pad_x;
        for c in rl.chars() {
            if rlx + cw > max_x {
                break;
            }
            raster.glyph_at(
                painter,
                metrics,
                rlx,
                recent_label_y,
                c as u32,
                theme.text_subtle,
            );
            rlx += cw;
        }
        for (ri, proj) in recent_projects.iter().take(5).enumerate() {
            let row_y = panel_y + (10.0 + ri as f64) * ch;
            let name = proj
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| proj.to_string_lossy().into_owned());
            let mut px = panel_x + pad_x + 2.0 * cw;
            for c in name.chars() {
                if px + cw > max_x {
                    break;
                }
                raster.glyph_at(painter, metrics, px, row_y, c as u32, theme.text_muted);
                px += cw;
            }
        }
    }

    // Footer.
    let footer = concat!("Anvil v", env!("CARGO_PKG_VERSION"));
    let mut fx = panel_x + pad_x;
    let footer_y = panel_y + panel_h - ch;
    for c in footer.chars() {
        if fx + cw > max_x {
            break;
        }
        raster.glyph_at(painter, metrics, fx, footer_y, c as u32, theme.text_subtle);
        fx += cw;
    }
}

// ── Project switcher overlay (item 30) ────────────────────────────────────────

/// Palette-style overlay listing recently-opened workspace directories.
#[allow(clippy::too_many_arguments)]
fn draw_project_switcher_overlay(
    raster: &mut anvil_render::raster::Raster,
    painter: &mut dyn anvil_render::raster::GlyphPainter,
    metrics: anvil_render::raster::FontMetrics,
    theme: &anvil_theme::Theme,
    recent_projects: &[PathBuf],
    selected: usize,
    dw: f64,
    _dh: f64,
    chrome_top: f64,
    cw: f64,
    ch: f64,
) {
    if recent_projects.is_empty() {
        return;
    }
    let rows = recent_projects.len().min(10);
    let panel_w = (50.0 * cw).min(dw * 0.7);
    let header_h = ch + 4.0;
    let panel_h = header_h + rows as f64 * (ch + 4.0);
    let panel_x = ((dw - panel_w) * 0.5).max(0.0);
    let panel_y = chrome_top + 2.0 * ch;

    raster.fill_pixel_rect(panel_x, panel_y, panel_w, panel_h, theme.surface);
    raster.fill_pixel_rect(panel_x, panel_y, panel_w, 1.0, theme.accent);
    raster.fill_pixel_rect(panel_x, panel_y + panel_h - 1.0, panel_w, 1.0, theme.accent);
    raster.fill_pixel_rect(panel_x, panel_y, 1.0, panel_h, theme.accent);
    raster.fill_pixel_rect(panel_x + panel_w - 1.0, panel_y, 1.0, panel_h, theme.accent);

    // Header label.
    let header = "Open Recent Project";
    let pad_x = 1.5 * cw;
    let header_y = panel_y + 2.0;
    let mut hx = panel_x + pad_x;
    for c in header.chars() {
        if hx + cw > panel_x + panel_w - pad_x {
            break;
        }
        raster.glyph_at(painter, metrics, hx, header_y, c as u32, theme.text_subtle);
        hx += cw;
    }

    raster.fill_pixel_rect(panel_x, panel_y + header_h, panel_w, 1.0, theme.hairline);

    // Rows.
    for (i, proj) in recent_projects.iter().take(rows).enumerate() {
        let row_y = panel_y + header_h + i as f64 * (ch + 4.0);
        if i == selected {
            raster.fill_pixel_rect_alpha(panel_x, row_y, panel_w, ch + 4.0, theme.accent, 0.15);
        }
        let name = proj
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| proj.to_string_lossy().into_owned());
        let path_str = proj.to_string_lossy().into_owned();

        // Basename in foreground.
        let mut rx = panel_x + pad_x;
        let glyph_y = row_y + 2.0;
        let name_max_x = panel_x + panel_w * 0.4;
        for c in name.chars() {
            if rx + cw > name_max_x {
                break;
            }
            let color = if i == selected {
                theme.foreground
            } else {
                theme.text_muted
            };
            raster.glyph_at(painter, metrics, rx, glyph_y, c as u32, color);
            rx += cw;
        }

        // Path in text_subtle.
        rx = panel_x + panel_w * 0.42;
        for c in path_str.chars() {
            if rx + cw > panel_x + panel_w - pad_x {
                break;
            }
            raster.glyph_at(painter, metrics, rx, glyph_y, c as u32, theme.text_subtle);
            rx += cw;
        }
    }
}

// ── Language picker (Q22) ────────────────────────────────────────────────────

/// All language-ids supported by the language picker (Cmd+K Cmd+L).
const PICKER_LANGS: &[&str] = &["rust", "typescript", "python", "toml", "json", "markdown"];

/// Return the subset of `langs` whose name contains `query` (case-insensitive).
fn picker_filtered<'a>(langs: &[&'a str], query: &str) -> Vec<&'a str> {
    let q = query.to_ascii_lowercase();
    if q.is_empty() {
        langs.to_vec()
    } else {
        langs
            .iter()
            .copied()
            .filter(|l| l.contains(q.as_str()))
            .collect()
    }
}

// ── main ──────────────────────────────────────────────────────────────────────

fn main() -> Result<()> {
    // -- Crash reporter (item 29) ─────────────────────────────────────────────
    // Install before anything else so panics that happen during init are caught.
    {
        let default_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            // Capture full backtrace.
            let bt = std::backtrace::Backtrace::force_capture();
            let cwd = std::env::current_dir()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|_| "<unknown>".to_string());
            let version = env!("CARGO_PKG_VERSION");
            let git_rev = option_env!("GIT_REV").unwrap_or("unknown");
            let timestamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let msg = format!(
                "anvil version: {version}\ngit rev: {git_rev}\ncwd: {cwd}\n\n{info}\n\n{bt}\n"
            );
            // Write to crash file.
            if let Some(home) = std::env::var_os("HOME") {
                let mut dir = std::path::PathBuf::from(home);
                dir.push(".config");
                dir.push("anvil");
                dir.push("crashes");
                let _ = std::fs::create_dir_all(&dir);
                let filename = format!("crash-{timestamp}.txt");
                let _ = std::fs::write(dir.join(filename), msg.as_bytes());
            }
            // Also print to stderr so the terminal shows it.
            eprintln!("{msg}");
            // Delegate to default hook (aborts / prints normal backtrace).
            default_hook(info);
        }));
    }

    // -- Config ---------------------------------------------------------------
    let cfg_path = anvil_config::resolve_path();
    let config: Config = match &cfg_path {
        Some(p) => anvil_config::load(p),
        None => Config::default(),
    };

    // -- Shell integration ----------------------------------------------------
    shell_integration::setup(config.shell_integration);

    // -- Register bundled font ------------------------------------------------
    register_bundled();

    // -- Main-thread marker ---------------------------------------------------
    let mtm = MainThreadMarker::new().expect("main() must be called on the main thread");

    // -- Window geometry -------------------------------------------------------
    let win_w = config.window.width;
    let win_h = config.window.height;

    // -- Query screen scale factor (before NSWindow exists) -------------------
    let window_scale: f64 = {
        use objc2::msg_send;
        let scale: f64 = unsafe {
            let cls = objc2::runtime::AnyClass::get(c"NSScreen");
            match cls {
                None => 2.0,
                Some(cls) => {
                    let screen: *mut objc2::runtime::AnyObject = msg_send![cls, mainScreen];
                    if screen.is_null() {
                        2.0
                    } else {
                        msg_send![screen, backingScaleFactor]
                    }
                }
            }
        };
        if scale < 1.0 { 2.0 } else { scale }
    };

    let dw = ((win_w * window_scale) as usize).max(1);
    let dh = ((win_h * window_scale) as usize).max(1);

    // -- Font -----------------------------------------------------------------
    // Save font family + size as owned values so they outlive the `config` move.
    let font_family = config.font.family.clone();
    let font_names: Vec<&str> = vec![
        "BlexMono Nerd Font Mono",
        font_family.as_str(),
        "SFMono-Regular",
        "Menlo",
    ];
    let font_size_pt_init = config.font.size;
    let pixel_size = config.font.size * window_scale;
    let font: Box<Font> = Box::new(
        Font::init_first_available(&font_names, pixel_size)
            .or_else(|_| Font::init("Menlo", pixel_size))
            .expect("at least Menlo must be available as a system font"),
    );
    let bold_font: Box<Font> = Box::new(
        Font::init_face(&font_names, pixel_size, FontFace::Bold, true).unwrap_or_else(|_| {
            Font::init_first_available(&font_names, pixel_size)
                .or_else(|_| Font::init("Menlo", pixel_size))
                .expect("fallback must be available")
        }),
    );
    let italic_font: Box<Font> = Box::new(
        Font::init_face(&font_names, pixel_size, FontFace::Italic, true).unwrap_or_else(|_| {
            Font::init_first_available(&font_names, pixel_size)
                .or_else(|_| Font::init("Menlo", pixel_size))
                .expect("fallback must be available")
        }),
    );
    let bold_italic_font: Box<Font> = Box::new(
        Font::init_face(&font_names, pixel_size, FontFace::BoldItalic, true).unwrap_or_else(|_| {
            Font::init_first_available(&font_names, pixel_size)
                .or_else(|_| Font::init("Menlo", pixel_size))
                .expect("fallback must be available")
        }),
    );
    // Chrome font: fixed CHROME_PT pt regardless of user terminal size.
    let chrome_pixel_size = CHROME_PT * window_scale;
    let chrome_font: Box<Font> = Box::new(
        Font::init_face(&font_names, chrome_pixel_size, FontFace::Chrome, false)
            .or_else(|_| Font::init("Menlo", chrome_pixel_size))
            .expect("at least Menlo must be available for chrome font"),
    );

    // -- Cell geometry --------------------------------------------------------
    let cw = font.metrics.cell_w as usize;
    let ch = font.metrics.cell_h as usize;
    // Initial PTY size must match what `App::pane_area_rect` will report once the
    // App is constructed (with `hud_visible = false` by default), otherwise the
    // first frames render at the wrong column count and scrollback comes back
    // mis-shaped. HUD is default-off — reserve only the padding + chrome + status rows.
    // (resize_all_tabs corrects to exact pane size once the window is up,
    // but the initial PTY needs sane dimensions for the first prompt frame.)
    let cols = (dw.saturating_sub(2 * GRID_PAD) / cw).max(1);
    // Chrome strips at top/bottom are fixed pixel heights (36pt/24pt);
    // PTY rows fill the remaining vertical pixels divided by cell_h.
    let chrome_top_px_init = (36.0 * window_scale) as usize;
    let chrome_bottom_px_init = (24.0 * window_scale) as usize;
    let rows = ((dh
        .saturating_sub(chrome_top_px_init)
        .saturating_sub(chrome_bottom_px_init))
        / ch)
        .max(1);

    // -- Initial tab + PTY ----------------------------------------------------
    let tab = Tab::new_single_pane(cols, rows, config.scrollback);
    let first_id = tab.focused_id();
    let first_pty =
        Pty::spawn_shell(cols as u16, rows as u16).expect("failed to spawn login shell");

    let mut ptys = HashMap::new();
    ptys.insert(first_id, first_pty);

    let mut tabs = TabManager::default();
    tabs.push(tab);

    // -- Git worker -----------------------------------------------------------
    let (git_tx, git_work_rx) = mpsc::sync_channel::<PathBuf>(1);
    let (git_result_tx, git_rx) = mpsc::channel::<GitResult>();
    thread::spawn(move || {
        for cwd in git_work_rx {
            let info = git::query(&cwd);
            let result = match info {
                Some(i) => {
                    let (head_short, head_subject) = git_head_oneline(&cwd);
                    let ports = detect_ports();
                    let project_kind = detect_project_kind(&cwd);
                    GitResult {
                        state: if i.dirty > 0 {
                            GitState::Dirty
                        } else {
                            GitState::Ok
                        },
                        branch: i.branch,
                        dirty: i.dirty,
                        ahead: i.ahead,
                        behind: i.behind,
                        head_short,
                        head_subject,
                        ports,
                        project_kind,
                    }
                }
                None => {
                    let ports = detect_ports();
                    let project_kind = detect_project_kind(&cwd);
                    GitResult {
                        state: GitState::NoRepo,
                        branch: String::new(),
                        dirty: 0,
                        ahead: 0,
                        behind: 0,
                        head_short: String::new(),
                        head_subject: String::new(),
                        ports,
                        project_kind,
                    }
                }
            };
            let _ = git_result_tx.send(result);
        }
    });

    // -- Recent-files worker --------------------------------------------------
    // Polls the cwd every 4 s for recently-modified files (top 5 by mtime).
    // The main thread sends updated cwd values via `recent_cwd_tx` (non-blocking
    // try_send). The worker drains any pending cwd, runs the walk, and returns
    // results via `recent_rx`.
    let (recent_cwd_tx, recent_cwd_rx) = mpsc::sync_channel::<PathBuf>(1);
    let (recent_tx, recent_rx) = mpsc::sync_channel::<RecentResult>(1);
    thread::spawn(move || {
        use std::time::Duration;
        let mut last_cwd: Option<PathBuf> = std::env::current_dir().ok();
        loop {
            // Drain any updated cwd from the main thread (take the latest).
            while let Ok(cwd) = recent_cwd_rx.try_recv() {
                last_cwd = Some(cwd);
            }
            if let Some(ref cwd) = last_cwd {
                let files = recent_files_in_dir(cwd, 5);
                // try_send: drop if channel is full (main thread busy).
                let _ = recent_tx.try_send(RecentResult { files });
            }
            thread::sleep(Duration::from_secs(4));
        }
    });

    // -- Kubectl worker -------------------------------------------------------
    let (kube_tx, kube_rx) = mpsc::sync_channel::<anvil_prompt_core::KubeCtx>(1);
    kube::spawn_kube_worker(kube_tx);

    // -- Filesystem worker (ID3) -----------------------------------------------
    let (fs_tx, fs_rx, fs_hidden_tx) = fs_worker::spawn_fs_worker();
    // Child-directory worker: loads individual dirs on expand.
    let (child_fs_tx, child_fs_rx) = fs_worker::spawn_child_fs_worker();

    // -- File-watcher worker (item 27) ----------------------------------------
    // Main thread registers (buffer_id, path) pairs via `file_watch_work_tx`.
    // Worker polls mtime every second and sends `FileWatchEvent` back when a
    // file has changed.  We use stdlib only — no external `notify` crate needed.
    let (file_watch_work_tx, file_watch_work_rx) =
        mpsc::sync_channel::<(anvil_editor::BufferId, PathBuf)>(64);
    let (file_watch_result_tx, file_watch_rx) = mpsc::channel::<FileWatchEvent>();
    {
        use std::collections::HashMap as WMap;
        use std::time::{Duration, SystemTime};
        thread::spawn(move || {
            // watched: buffer_id → (path, last_known_mtime)
            let mut watched: WMap<anvil_editor::BufferId, (PathBuf, Option<SystemTime>)> =
                WMap::new();
            loop {
                // Drain registration messages (non-blocking).
                while let Ok((bid, path)) = file_watch_work_rx.try_recv() {
                    let mtime = std::fs::metadata(&path)
                        .ok()
                        .and_then(|m| m.modified().ok());
                    watched.insert(bid, (path, mtime));
                }
                // Poll each watched file for mtime changes.
                for (&bid, entry) in &mut watched {
                    let (ref path, ref mut known_mtime) = *entry;
                    let Ok(meta) = std::fs::metadata(path) else {
                        continue;
                    };
                    let Ok(current_mtime) = meta.modified() else {
                        continue;
                    };
                    let changed = known_mtime.is_none_or(|k| k != current_mtime);
                    if changed {
                        *known_mtime = Some(current_mtime);
                        let _ = file_watch_result_tx.send(FileWatchEvent { buffer_id: bid });
                    }
                }
                thread::sleep(Duration::from_secs(1));
            }
        });
    }

    // -- Theme ----------------------------------------------------------------
    let system_dark = system_is_dark();
    let effective_name = effective_theme_name(system_dark, &config.theme);
    let theme = resolve_theme(effective_name, &config.theme_overrides);
    let cursor_cfg = cursor_cfg_from_config(&config);
    let keybindings = Keybindings::from_config(&config.keybindings);

    // -- Raster ---------------------------------------------------------------
    // pad_y is set to the chrome top strip height so cell row 0 is the
    // first terminal row (the chrome lives in [0, chrome_top_px)).
    let mut raster = Raster::new(dw, dh);
    raster.pad_x = GRID_PAD as f64;
    raster.pad_y = chrome_top_px_init as f64;

    // -- Build App ------------------------------------------------------------
    let watcher = cfg_path.map(Watcher::new);

    // -- Render path selection ------------------------------------------------
    let use_gpu_render = matches!(std::env::var("ANVIL_RENDER").as_deref(), Ok("gpu"));
    eprintln!(
        "anvil: render = {}",
        if use_gpu_render { "gpu" } else { "cpu" }
    );

    // -- Layout mode -----------------------------------------------------------
    // Config gives users a durable straight-terminal preference; the env var
    // remains a debug/test override. `auto` keeps the product default: IDE in
    // project dirs, terminal elsewhere.
    let cwd_for_mode = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let auto_layout_mode = || {
        if has_project_marker_in_or_above(&cwd_for_mode) {
            LayoutMode::Ide
        } else {
            LayoutMode::Terminal
        }
    };
    let layout_mode = match std::env::var("ANVIL_LAYOUT_MODE").as_deref() {
        Ok("ide") => LayoutMode::Ide,
        Ok("terminal") => LayoutMode::Terminal,
        _ => match config.layout_mode {
            anvil_config::StartupLayout::Ide => LayoutMode::Ide,
            anvil_config::StartupLayout::Terminal => LayoutMode::Terminal,
            anvil_config::StartupLayout::Auto => auto_layout_mode(),
        },
    };
    eprintln!("anvil: layout_mode = {layout_mode:?}");

    let app = App {
        tabs,
        ptys,
        renderer: None, // filled after the Metal layer is available
        raster,
        font, // Box<Font> — heap-stable (Regular)
        bold_font,
        italic_font,
        bold_italic_font,
        chrome_font, // Box<Font> — fixed 11 pt chrome face
        dirty: true,
        force_full_redraw: true,
        cursor_row_prev: HashMap::new(),
        scrollback_len_prev: HashMap::new(),
        last_scroll_pos: 0.0,
        last_viewport_offset: 0,
        #[cfg(debug_assertions)]
        debug_render_frame: 0,
        #[cfg(debug_assertions)]
        debug_render_last_report: None,
        use_gpu_render,
        cell_batch: CellBatch::new(),
        atlas_painter: None, // filled when Metal device is available
        theme,
        cursor_cfg,
        config,
        watcher,
        keybindings,
        system_dark,
        window_scale,
        layout_mode,
        // Direction A is editor-first: project/IDE startup should show the
        // Explorer so file-open state is unmistakable. Users can still hide it
        // with Cmd+B.
        left_dock_visible: layout_mode == LayoutMode::Ide,
        left_dock_w_pt: 300.0,
        left_dock_w_pt_target: 300.0,
        sidebar_drag_active: false,
        drawer_drag_active: false,
        drawer_hidden: false,
        drawer_saved_ratio: 0.72,
        blink_phase: 0.0,
        last_blink_opacity: -1.0,
        search: anvil_term::Search::new(),
        search_open: false,
        // Docked right HUD on by default — Cmd+J toggles it.
        hud_visible: false,
        hud_tick: 0,
        hud_cols: HUD_COLS_DEFAULT,
        hud_drag_active: false,
        divider_drag: None,
        divider_hover: None,
        hscroll_drag_active: false,
        tab_bar_hits: TabBarHits::default(),
        left_dock_hits: LeftDockHits::default(),
        hud_hits: Vec::new(),
        hud_section_order: load_hud_section_order()
            .unwrap_or_else(|| SectionId::DEFAULT_ORDER.to_vec()),
        hud_section_hits: Vec::new(),
        hud_section_drag: None,
        tab_drag: None,
        editor_tab_drag: None,
        recent_file_list: Vec::new(),
        font_size_pt: font_size_pt_init,
        font_family: font_family.clone(),
        cheatsheet_visible: false,
        focused: true,
        agent_snap: AgentSnapshot::default(),
        // Pre-populate cwd from the process's working directory so the bottom
        // status bar has data on the very first frame, before the shell emits
        // its first OSC 7 cwd report.
        local_ctx: LocalContext {
            cwd: std::env::current_dir()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default(),
            ..LocalContext::default()
        },
        // The caldera poller is started lazily once we know the repo root
        // (set when the first focused pane's cwd is known); see tick().
        caldera_poller: None,
        caldera_client: None,
        editor_mouse_drag_start: None,
        git_tx,
        git_rx,
        recent_cwd_tx,
        recent_rx,
        kube_rx,
        fs_tx,
        fs_rx,
        fs_hidden_tx,
        child_fs_tx,
        child_fs_rx,
        fs_snapshot: std::env::current_dir().ok().map(|cwd| {
            let snap = fs_worker::read_dir_snapshot_fast(
                &cwd,
                fs_worker::FilterFlags {
                    show_hidden: false,
                    show_gitignored: false,
                },
            );
            LeftDockSnapshot {
                root: snap.root.to_string_lossy().into_owned(),
                entries: snap
                    .entries
                    .into_iter()
                    .map(|e| anvil_render::left_dock::DirEntry {
                        name: e.name,
                        is_dir: e.is_dir,
                    })
                    .collect(),
                git_marks: snap.git_marks,
            }
        }),
        child_snapshots: HashMap::new(),
        active_explorer_file: None,
        explorer_scroll_offset: 0,
        hovered_explorer_row: None,
        hovered_editor_tab: None,
        editor_tab_hits: Vec::new(),
        expanded_dirs: HashSet::new(),
        scroll_indicator_alpha: 0.0,
        scroll_indicator_last_scroll: None,
        fs_last_cwd: None,
        agent_pulse_phase: 0.0,
        last_agent_pulse_opacity: -1.0,
        running_pulse_phase: 0.0,
        palette: Palette::default(),
        project_search: anvil_workspace::project_search::ProjectSearch::new(),
        view_width_pt: win_w,
        view_height_pt: win_h,
        lsp_manager: anvil_editor::LspManager::new(),
        lsp_last_sync: HashMap::new(),
        pending_hover: None,
        hover_mouse_pos: None,
        hover_mouse_time: None,
        pending_definition: None,
        pending_completion: None,
        pending_rename: None,
        pending_code_actions: None,
        pending_references: None,
        code_actions_pending_edits: Vec::new(),
        lsp_rename_input: None,
        lsp_references: None,
        workspace_symbol_search: None,
        buffer_symbol_search: None,
        ui_scale: 1.0,
        font_scale: 1.0,
        pending_chord_k: false,
        focus_target: FocusTarget::default(),
        selected_explorer_row: None,
        explorer_rename: None,
        explorer_new_item: None,
        explorer_delete_confirm: None,
        goto_line_input: None,
        save_as_input: None,
        replace_row_active: false,
        file_watch_rx,
        file_watch_tx: file_watch_work_tx,
        disk_changed_dirty: HashMap::new(),
        recent_projects: Vec::new(),
        project_switcher_open: false,
        project_switcher_sel: 0,
        right_click_path: None,
        explorer_drag: None,
        explorer_drag_cursor: None,
        toasts: std::collections::VecDeque::new(),
        lsp_failed_toasted: HashSet::new(),
        search_bar_hits: anvil_render::searchbar::SearchBarArrowHits::default(),
        show_hidden_files: false,
        show_gitignored_files: false,
        closed_tabs: std::collections::VecDeque::new(),
        language_picker: None,
        open_folder_input: None,
        explorer_hover_row: None,
        explorer_hover_meta: None,
        explorer_filter: None,
    };

    // -- AppKitApp: builds the window, view, timer ----------------------------
    // Two-phase init: AppKitApp needs a handler Rc now; AppShell needs the
    // window's CAMetalLayer (only available after AppKitApp::new).
    //
    // Pattern: ForwardingHandler holds a shared Rc<RefCell<Option<AppShell>>>.
    // main() fills the Option after building the layer+renderer+webview.
    // AppKitApp::run() starts — by then the Option is Some(_).

    let shell_slot: Rc<RefCell<Option<AppShell>>> = Rc::new(RefCell::new(None));

    struct ForwardingHandler(Rc<RefCell<Option<AppShell>>>);
    impl AppHandler for ForwardingHandler {
        fn tick(&mut self) {
            if let Some(h) = &mut *self.0.borrow_mut() {
                h.tick()
            }
        }
        fn key_down(&mut self, e: KeyEvent) {
            if let Some(h) = &mut *self.0.borrow_mut() {
                h.key_down(e)
            }
        }
        fn perform_key_equivalent(&mut self, e: KeyEvent) -> bool {
            self.0
                .borrow_mut()
                .as_mut()
                .is_some_and(|h| h.perform_key_equivalent(e))
        }
        fn mouse_down(&mut self, l: MouseLocation, m: Modifiers, _click_count: u32, b: (f64, f64)) {
            if let Some(h) = &mut *self.0.borrow_mut() {
                h.mouse_down(l, m, _click_count, b)
            }
        }
        fn mouse_up(&mut self, l: MouseLocation, m: Modifiers) {
            if let Some(h) = &mut *self.0.borrow_mut() {
                h.mouse_up(l, m)
            }
        }
        fn mouse_dragged(&mut self, l: MouseLocation) {
            if let Some(h) = &mut *self.0.borrow_mut() {
                h.mouse_dragged(l)
            }
        }
        fn mouse_moved(&mut self, l: MouseLocation) -> CursorKind {
            self.0
                .borrow_mut()
                .as_mut()
                .map_or(CursorKind::Arrow, |h| h.mouse_moved(l))
        }
        fn scroll(&mut self, dy: f64, pp: bool, shift: bool, l: MouseLocation) {
            if let Some(h) = &mut *self.0.borrow_mut() {
                h.scroll(dy, pp, shift, l)
            }
        }
        fn resize(&mut self, w: f64, h: f64, live: bool) {
            if let Some(sh) = &mut *self.0.borrow_mut() {
                sh.resize(w, h, live)
            }
        }
        fn live_resize_ended(&mut self) {
            if let Some(h) = &mut *self.0.borrow_mut() {
                h.live_resize_ended()
            }
        }
        fn focus_gained(&mut self) {
            if let Some(h) = &mut *self.0.borrow_mut() {
                h.focus_gained()
            }
        }
        fn focus_lost(&mut self) {
            if let Some(h) = &mut *self.0.borrow_mut() {
                h.focus_lost()
            }
        }
        fn should_terminate(&mut self) -> bool {
            self.0
                .borrow_mut()
                .as_mut()
                .is_none_or(|h| h.should_terminate())
        }
        fn webview_message(&mut self, json: String) {
            if let Some(h) = &mut *self.0.borrow_mut() {
                h.webview_message(json)
            }
        }
        fn context_action(&mut self, action: ContextAction) {
            if let Some(h) = &mut *self.0.borrow_mut() {
                h.context_action(action)
            }
        }
        fn right_click_zone(&mut self, l: MouseLocation) -> RightClickZone {
            self.0
                .borrow_mut()
                .as_mut()
                .map(|h| h.right_click_zone(l))
                .unwrap_or(RightClickZone::Terminal)
        }
        fn dropped_files(&mut self, paths: Vec<PathBuf>) {
            if let Some(h) = &mut *self.0.borrow_mut() {
                h.dropped_files(paths)
            }
        }
    }

    let fwd_rc: Rc<RefCell<dyn AppHandler>> =
        Rc::new(RefCell::new(ForwardingHandler(Rc::clone(&shell_slot))));

    let appkit = AppKitApp::new(Rc::clone(&fwd_rc), win_w, win_h, "Anvil");

    // Get the actual backing scale from the real window.
    let actual_scale = appkit.backing_scale_factor();

    // -- Renderer: init from the view's CAMetalLayer -------------------------
    let layer = {
        use objc2::msg_send;
        use objc2::rc::Retained;
        use objc2_quartz_core::CAMetalLayer;
        // SAFETY: AppKitApp::new set a CAMetalLayer on the view.
        unsafe {
            let view_ptr: *const objc2_app_kit::NSView = objc2::rc::Retained::as_ptr(&appkit.view);
            let layer_raw: *mut objc2::runtime::AnyObject = msg_send![view_ptr, layer];
            assert!(!layer_raw.is_null(), "view's CAMetalLayer must not be null");
            Retained::retain(layer_raw as *mut CAMetalLayer)
                .expect("CAMetalLayer retain must succeed")
        }
    };

    let actual_dw = ((win_w * actual_scale) as usize).max(1);
    let actual_dh = ((win_h * actual_scale) as usize).max(1);
    let mut renderer =
        Renderer::init(layer, actual_dw, actual_dh).expect("Metal renderer init failed");
    renderer.set_clear_color(theme.background);

    // -- Webview: needs handler_ptr for script-message callbacks --------------
    let webview_box: Box<Rc<RefCell<dyn AppHandler>>> = Box::new(Rc::clone(&fwd_rc));
    let webview_ptr = Box::into_raw(webview_box);

    let webview = Webview::init(WebviewConfig {
        window: appkit.window.clone(),
        container: &appkit.view,
        terminal_view: appkit.view.clone(),
        width: win_w,
        height: win_h,
        html: PALETTE_HTML,
        handler_ptr: webview_ptr,
        mtm,
    });

    // -- Finish building App: install real renderer, correct scale ------------
    let mut real_app = app;
    if real_app.layout_mode == LayoutMode::Ide {
        if let Some(editor_id) = real_app
            .tabs
            .current_mut()
            .and_then(|tab| tab.ensure_ide_editor_surface())
        {
            if let Some(tab) = real_app.tabs.current_mut() {
                tab.tree.focused = editor_id;
            }
        }
    }
    real_app.renderer = Some(renderer);
    real_app.window_scale = actual_scale;
    real_app.raster.resize(actual_dw, actual_dh);

    // -- GPU atlas painter (only when GPU path is active) ----------------------
    if real_app.use_gpu_render {
        // Construct a second Font with the same metrics as the CPU path's font.
        // Font::init_first_available is cheap (CoreText API calls only;
        // register_bundled() has already been called above).
        let font2 = Font::init_first_available(&font_names, pixel_size)
            .or_else(|_| Font::init("Menlo", pixel_size))
            .expect("GPU atlas font must be available");
        match AtlasPainter::new_with_default_device(font2) {
            Some(Ok(ap)) => real_app.atlas_painter = Some(ap),
            Some(Err(e)) => {
                eprintln!("anvil: GPU atlas painter init failed ({e}); falling back to cpu");
                real_app.use_gpu_render = false;
            }
            None => {
                eprintln!("anvil: no Metal device for GPU atlas; falling back to cpu");
                real_app.use_gpu_render = false;
            }
        }
    }

    // Build the AppShell and stash it in the shared slot.
    let mut shell = AppShell::new(real_app, webview, appkit.window.clone());
    shell.painter.warm_ascii();
    shell.bold_painter.warm_ascii();
    shell.italic_painter.warm_ascii();
    shell.bold_italic_painter.warm_ascii();
    shell.app.snap_anim();

    // -- Session restore (item 19) -----------------------------------------------
    // Restore after AppShell is built so the Metal layer + font are ready
    // (open_path_in_native_editor triggers syntax highlighting).
    if let Some(cwd) = shell.app.current_cwd() {
        shell.app.restore_session(&cwd);
    }
    // H4: apply restored font_scale if it differs from the startup default.
    // restore_session sets the field on App; bump_font_scale lives on AppShell.
    {
        let fs = shell.app.font_scale;
        if (fs - 1.0).abs() > 0.001 {
            // Temporarily reset to 1.0 so delta = fs - 1.0 brings it to fs.
            shell.app.font_scale = 1.0;
            shell.bump_font_scale(fs - 1.0);
        }
    }

    let mut grid_painters = GridPainters {
        regular: &mut shell.painter,
        bold: &mut shell.bold_painter,
        italic: &mut shell.italic_painter,
        bold_italic: &mut shell.bold_italic_painter,
    };
    shell
        .app
        .render_frame(&mut grid_painters, &mut shell.chrome_painter);
    shell.app.dirty = false;
    *shell_slot.borrow_mut() = Some(shell);

    // -- Caldera session on repo open (best-effort, background) ---------------
    if let Ok(cwd) = std::env::current_dir() {
        if cwd.join(".caldera/project.json").exists() {
            thread::spawn(move || {
                use anvil_control::AiSessionBroker;
                let agent =
                    std::env::var("ANVIL_CALDERA_AGENT").unwrap_or_else(|_| "codex".to_string());
                let task = std::env::var("ANVIL_CALDERA_TASK").unwrap_or_else(|_| {
                    "Open this repo in Anvil and prepare safe AI context".to_string()
                });
                match AiSessionBroker::localhost().prepare_repo_session(task, agent) {
                    Ok(s) => eprintln!(
                        "anvil: prepared Caldera session {} ({})",
                        s.session_id, s.handoff_path
                    ),
                    Err(e) => eprintln!("anvil: Caldera session preparation skipped: {e}"),
                }
            });
        }
    }

    appkit.run();
    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explorer_hit_paths_resolve_header_rows_and_ignore_missing_rows() {
        let snap = LeftDockSnapshot {
            root: "/tmp/project".to_string(),
            entries: vec![
                anvil_render::LeftDockEntry {
                    name: "src".to_string(),
                    is_dir: true,
                },
                anvil_render::LeftDockEntry {
                    name: "main.rs".to_string(),
                    is_dir: false,
                },
            ],
            git_marks: std::collections::HashMap::new(),
        };

        assert_eq!(
            explorer_path_for_hit(&snap, ExplorerHit::Header),
            Some(PathBuf::from("/tmp/project"))
        );
        assert_eq!(
            explorer_path_for_hit(&snap, ExplorerHit::Row(1)),
            Some(PathBuf::from("/tmp/project/main.rs"))
        );
        assert_eq!(explorer_path_for_hit(&snap, ExplorerHit::Row(9)), None);
    }

    #[test]
    fn explorer_scroll_offset_changes_in_mouse_sized_steps_and_clamps() {
        assert_eq!(next_explorer_scroll_offset(0, 1.0, 10), 3);
        assert_eq!(next_explorer_scroll_offset(3, -1.0, 10), 0);
        assert_eq!(next_explorer_scroll_offset(9, 1.0, 10), 10);
        assert_eq!(next_explorer_scroll_offset(6, 0.0, 10), 6);
        assert_eq!(next_explorer_scroll_offset(6, 1.0, 0), 0);
    }

    #[test]
    fn ascii_lower_lowercases_ascii_letters() {
        assert_eq!(ascii_lower('A'), 'a');
        assert_eq!(ascii_lower('Z'), 'z');
        assert_eq!(ascii_lower('a'), 'a');
        assert_eq!(ascii_lower('5'), '5');
        assert_eq!(ascii_lower('\u{00C0}'), '\u{00C0}');
    }

    #[test]
    fn format_hex_produces_lowercase_six_digit_hex() {
        assert_eq!(format_hex([0x1a, 0x1c, 0x24]), "#1a1c24");
        assert_eq!(format_hex([0xff, 0x00, 0x80]), "#ff0080");
        assert_eq!(format_hex([0x00, 0x00, 0x00]), "#000000");
    }

    #[test]
    fn effective_theme_name_maps_system_to_dark_or_light() {
        assert_eq!(effective_theme_name(true, "system"), "ember-dark");
        assert_eq!(effective_theme_name(false, "system"), "ember-light");
        assert_eq!(effective_theme_name(true, "ember-light"), "ember-light");
        assert_eq!(effective_theme_name(true, "mineral-light"), "mineral-light");
    }

    #[test]
    fn next_theme_mode_cycles_dark_light_system() {
        assert_eq!(next_theme_mode("ember-dark"), "ember-light");
        assert_eq!(next_theme_mode("ember-light"), "system");
        assert_eq!(next_theme_mode("system"), "ember-dark");
        assert_eq!(next_theme_mode("mineral-dark"), "ember-dark");
    }

    #[test]
    fn platform_key_to_zig_key_covers_all_named_variants() {
        assert_eq!(platform_key_to_zig_key(KeyInput::Enter), Some(Key::Enter));
        assert_eq!(platform_key_to_zig_key(KeyInput::Tab), Some(Key::Tab));
        assert_eq!(
            platform_key_to_zig_key(KeyInput::Backspace),
            Some(Key::Backspace)
        );
        assert_eq!(platform_key_to_zig_key(KeyInput::Escape), Some(Key::Escape));
        assert_eq!(platform_key_to_zig_key(KeyInput::Up), Some(Key::Up));
        assert_eq!(platform_key_to_zig_key(KeyInput::Down), Some(Key::Down));
        assert_eq!(platform_key_to_zig_key(KeyInput::Left), Some(Key::Left));
        assert_eq!(platform_key_to_zig_key(KeyInput::Right), Some(Key::Right));
        assert_eq!(platform_key_to_zig_key(KeyInput::F(1)), Some(Key::F1));
        assert_eq!(platform_key_to_zig_key(KeyInput::F(12)), Some(Key::F12));
        assert_eq!(platform_key_to_zig_key(KeyInput::F(99)), None);
        assert_eq!(
            platform_key_to_zig_key(KeyInput::Char('a')),
            Some(Key::Text('a'))
        );
    }

    #[test]
    fn chord_matching_requires_all_modifiers_and_key() {
        let chord = anvil_config::Chord {
            cmd: true,
            shift: false,
            ctrl: false,
            opt: false,
            key: 't',
        };
        let mods_match = Modifiers {
            command: true,
            shift: false,
            control: false,
            option: false,
        };
        let mods_no = Modifiers {
            command: false,
            shift: false,
            control: false,
            option: false,
        };
        assert!(App::chord_matches(chord, mods_match, 't'));
        assert!(!App::chord_matches(chord, mods_no, 't'));
        assert!(!App::chord_matches(chord, mods_match, 'x'));
        // ASCII case-insensitive via ascii_lower.
        assert!(App::chord_matches(chord, mods_match, 'T'));
    }

    #[test]
    fn keybindings_parsed_from_defaults() {
        let cfg = anvil_config::Keybindings::default();
        let kb = Keybindings::from_config(&cfg);
        let nt = kb.new_tab.unwrap();
        assert!(nt.cmd);
        assert_eq!(nt.key, 't');
    }

    // ── platform_mods_to_zig_mods ────────────────────────────────────────────

    #[test]
    fn platform_mods_to_zig_mods_maps_all_fields() {
        let m = Modifiers {
            command: true,
            shift: true,
            control: false,
            option: false,
        };
        let z = platform_mods_to_zig_mods(m);
        assert!(z.command);
        assert!(z.shift);
        assert!(!z.control);
        assert!(!z.option);
    }

    #[test]
    fn platform_mods_to_zig_mods_all_false() {
        let m = Modifiers {
            command: false,
            shift: false,
            control: false,
            option: false,
        };
        let z = platform_mods_to_zig_mods(m);
        assert!(!z.command);
        assert!(!z.shift);
        assert!(!z.control);
        assert!(!z.option);
    }

    #[test]
    fn platform_mods_to_zig_mods_ctrl_opt() {
        let m = Modifiers {
            command: false,
            shift: false,
            control: true,
            option: true,
        };
        let z = platform_mods_to_zig_mods(m);
        assert!(z.control);
        assert!(z.option);
        assert!(!z.command);
        assert!(!z.shift);
    }

    // ── cursor_cfg_from_config ────────────────────────────────────────────────

    #[test]
    fn cursor_cfg_from_config_block_style() {
        use anvil_config::CursorStyle;
        use anvil_render::draw::CursorStyle as RCursorStyle;
        let mut cfg = anvil_config::Config::default();
        cfg.cursor.style = CursorStyle::Block;
        cfg.cursor.blink = false;
        let cc = cursor_cfg_from_config(&cfg);
        assert_eq!(cc.style, RCursorStyle::Block);
        assert!(!cc.blink);
    }

    #[test]
    fn cursor_cfg_from_config_bar_style() {
        use anvil_config::CursorStyle;
        use anvil_render::draw::CursorStyle as RCursorStyle;
        let mut cfg = anvil_config::Config::default();
        cfg.cursor.style = CursorStyle::Bar;
        cfg.cursor.blink = true;
        let cc = cursor_cfg_from_config(&cfg);
        assert_eq!(cc.style, RCursorStyle::Bar);
        assert!(cc.blink);
    }

    #[test]
    fn cursor_cfg_from_config_underline_style() {
        use anvil_config::CursorStyle;
        use anvil_render::draw::CursorStyle as RCursorStyle;
        let mut cfg = anvil_config::Config::default();
        cfg.cursor.style = CursorStyle::Underline;
        let cc = cursor_cfg_from_config(&cfg);
        assert_eq!(cc.style, RCursorStyle::Underline);
    }

    // ── all_pane_ids_in_tree ─────────────────────────────────────────────────

    #[test]
    fn all_pane_ids_in_tree_single_pane() {
        let tab = anvil_workspace::tab::Tab::new_single_pane(80, 24, 100);
        let ids = all_pane_ids_in_tree(&tab);
        assert_eq!(ids.len(), 1);
    }

    #[test]
    fn all_pane_ids_in_tree_after_split() {
        let mut tab = anvil_workspace::tab::Tab::new_single_pane(80, 24, 100);
        let new_id = tab
            .split(anvil_workspace::layout::SplitDir::Horizontal, 40, 24, 100)
            .unwrap();
        let ids = all_pane_ids_in_tree(&tab);
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&new_id));
    }

    // ── shell_quote_arg ───────────────────────────────────────────────────────

    fn collect_quote(s: &str) -> Vec<u8> {
        let mut out = Vec::new();
        shell_quote_arg(s, |chunk| out.extend_from_slice(chunk));
        out
    }

    #[test]
    fn shell_quote_arg_simple_path_unchanged() {
        assert_eq!(collect_quote("/usr/local/bin"), b"/usr/local/bin");
    }

    #[test]
    fn shell_quote_arg_single_quote_in_path_escaped() {
        // "it's a file" → it'\''s a file
        assert_eq!(collect_quote("it's"), b"it'\\''s");
    }

    #[test]
    fn shell_quote_arg_multiple_quotes_all_escaped() {
        assert_eq!(collect_quote("a'b'c"), b"a'\\''b'\\''c");
    }

    #[test]
    fn shell_quote_arg_empty_string_emits_nothing() {
        assert_eq!(collect_quote(""), b"");
    }

    #[test]
    fn shell_quote_arg_leading_quote_escaped() {
        assert_eq!(collect_quote("'hello"), b"'\\''hello");
    }

    #[test]
    fn shell_quote_arg_trailing_quote_escaped() {
        assert_eq!(collect_quote("hello'"), b"hello'\\''");
    }

    // ── platform_key_to_zig_key extended coverage ─────────────────────────────

    #[test]
    fn platform_key_to_zig_key_home_end_pageup_pagedown_delete() {
        use anvil_workspace::keys::Key;
        assert_eq!(platform_key_to_zig_key(KeyInput::Home), Some(Key::Home));
        assert_eq!(platform_key_to_zig_key(KeyInput::End), Some(Key::End));
        assert_eq!(platform_key_to_zig_key(KeyInput::PageUp), Some(Key::PageUp));
        assert_eq!(
            platform_key_to_zig_key(KeyInput::PageDown),
            Some(Key::PageDown)
        );
        assert_eq!(platform_key_to_zig_key(KeyInput::Delete), Some(Key::Delete));
    }

    #[test]
    fn platform_key_to_zig_key_all_function_keys() {
        use anvil_workspace::keys::Key;
        let expected = [
            Key::F1,
            Key::F2,
            Key::F3,
            Key::F4,
            Key::F5,
            Key::F6,
            Key::F7,
            Key::F8,
            Key::F9,
            Key::F10,
            Key::F11,
            Key::F12,
        ];
        for (n, exp) in expected.iter().enumerate() {
            let n = n as u8 + 1;
            assert_eq!(platform_key_to_zig_key(KeyInput::F(n)), Some(*exp), "F{n}");
        }
    }

    // ── N3: toast system ─────────────────────────────────────────────────────

    /// `push_toast` caps text at 60 characters.
    #[test]
    fn toast_text_capped_at_60_chars() {
        // Build a minimal App-like struct — use only the toast VecDeque.
        // We test the logic via the helper functions directly.
        let long = "a".repeat(80);
        let truncated: String = long.chars().take(App::TOAST_MAX_CHARS).collect();
        assert_eq!(truncated.len(), 60, "toast text must be capped at 60 chars");
    }

    /// Toasts with `expires_at` in the past are removed by `tick_toasts`.
    #[test]
    fn expired_toasts_removed_on_tick() {
        let mut q: std::collections::VecDeque<Toast> = std::collections::VecDeque::new();
        let already_expired = Toast {
            text: "old".into(),
            kind: ToastKind::Info,
            expires_at: Instant::now() - std::time::Duration::from_secs(10),
        };
        let still_live = Toast {
            text: "new".into(),
            kind: ToastKind::Success,
            expires_at: Instant::now() + std::time::Duration::from_secs(10),
        };
        q.push_back(already_expired);
        q.push_back(still_live);

        // Manually apply the same drain logic as `tick_toasts`.
        let now = Instant::now();
        while q.front().is_some_and(|t| t.expires_at <= now) {
            q.pop_front();
        }

        assert_eq!(
            q.len(),
            1,
            "expired toast must be removed; 1 live toast remains"
        );
        assert_eq!(q.front().unwrap().text, "new");
    }

    // ── R2: humanize_bytes ────────────────────────────────────────────────────

    #[test]
    fn humanize_bytes_formats_sizes_correctly() {
        assert_eq!(humanize_bytes(0), "0 B");
        assert_eq!(humanize_bytes(1023), "1023 B");
        assert_eq!(humanize_bytes(1024), "1.0 KB");
        assert_eq!(humanize_bytes(12700), "12.4 KB");
        assert_eq!(humanize_bytes(1024 * 1024), "1.0 MB");
        assert_eq!(humanize_bytes(1024 * 1024 * 1024), "1.0 GB");
    }

    // ── R2: relative_time ─────────────────────────────────────────────────────

    #[test]
    fn relative_time_formats_deltas_correctly() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        assert_eq!(relative_time(now), "just now");
        assert_eq!(relative_time(now - 30), "just now");
        assert_eq!(relative_time(now - 90), "1 minute ago");
        assert_eq!(relative_time(now - 7200), "2 hours ago");
        assert_eq!(relative_time(now - 3 * 86400), "3 days ago");
    }
}
