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

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc;
use std::thread;
#[cfg(debug_assertions)]
use std::time::Instant;

use anyhow::Result;

use anvil_agent::Snapshot as AgentSnapshot;
use anvil_config::{Chord, Config, Watcher, parse_chord};
use anvil_platform::AtlasPainter;
use anvil_platform::appkit::{AppHandler, AppKitApp, KeyEvent, KeyInput, Modifiers, MouseLocation};
use anvil_platform::font::{Font, register_bundled};
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
use anvil_render::cheatsheet::draw as draw_cheatsheet;
use anvil_render::draw::CursorConfig;
use anvil_render::raster::Raster;
use anvil_render::searchbar::draw_search_bar;
use anvil_render::tabbar::{TabBarHitKind, TabBarHits, draw_tab_bar};
use anvil_render::workspace::{DIVIDER_PX, draw_workspace, draw_workspace_chrome};
use anvil_render::{CellBatch, FoldedBlocks, draw_viewport_gpu};
use anvil_term::DirtySet;
use anvil_theme::{Theme, resolve as resolve_theme};
use anvil_workspace::interact;
use anvil_workspace::keys::{Key, Mods, encode as encode_key, encode_mouse};
use anvil_workspace::layout::{NavDir, PaneId, Rect, SplitDir};
use anvil_workspace::palette::{Action, CATALOG, Palette, action_for_id};
use anvil_workspace::tab::{Tab, TabManager};

use anvil_control::bridge::{
    Command as BridgeCmd, Inbound, Outbound, ThemeTokens, decode as bridge_decode,
    encode as bridge_encode,
};

use objc2_foundation::MainThreadMarker;

// ── Embedded assets ──────────────────────────────────────────────────────────

const PALETTE_HTML: &str = include_str!("../../../ui/palette/index.html");

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
    if cwd.join("Makefile").exists() {
        return Some("make".to_string());
    }
    None
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
        }
    }
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
    /// Chrome-row hit regions (tab switches, close ×, + button). Refilled
    /// by `draw_tab_bar` each render; consumed by `mouse_down`.
    tab_bar_hits: TabBarHits,
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

    // -- git worker ---
    git_tx: mpsc::SyncSender<PathBuf>,
    git_rx: mpsc::Receiver<GitResult>,

    // -- recent-files worker ---
    recent_cwd_tx: mpsc::SyncSender<PathBuf>,
    recent_rx: mpsc::Receiver<RecentResult>,

    // -- command palette ---
    palette: Palette,

    // -- window geometry (view-point size, updated on resize) ---
    view_width_pt: f64,
    view_height_pt: f64,
}

// ── App helpers ───────────────────────────────────────────────────────────────

impl App {
    /// The current focused pane id.
    fn focused_pane_id(&self) -> PaneId {
        self.tabs.current().map(|t| t.focused_id()).unwrap_or(0)
    }

    /// The cwd of the focused pane (OSC 7 path), or `None` if unset.
    fn current_cwd(&self) -> Option<String> {
        let tab = self.tabs.current()?;
        let id = tab.focused_id();
        let pane = tab.registry.get(id)?;
        let cwd = pane.terminal.cwd_path();
        if cwd.is_empty() {
            None
        } else {
            Some(cwd.to_string())
        }
    }

    /// Device-pixel dimensions of the content area.
    fn device_size(&self) -> (usize, usize) {
        let dw = ((self.view_width_pt * self.window_scale) as usize).max(1);
        let dh = ((self.view_height_pt * self.window_scale) as usize).max(1);
        (dw, dh)
    }

    /// Inner content rect in device pixels: window minus bars, padding, and
    /// (when shown) the docked right HUD column.
    ///
    /// The HUD touches the window's right edge, so when it is visible the
    /// grid skips the usual right `GRID_PAD` — the HUD absorbs it — and
    /// instead reserves `self.hud_cols * cw` plus one cell of breathing room.
    fn inner_rect(&self) -> Rect {
        let (dw, dh) = self.device_size();
        let pad = GRID_PAD as f64;
        let cw = self.font.metrics.cell_w;
        let ch = self.font.metrics.cell_h;

        let top_bar_px = self.top_bar_rows() as f64 * ch;
        let bot_bar_px = self.bottom_bar_rows() as f64 * ch;
        let (right_margin_px, right_gutter_px) = if self.hud_visible {
            (0.0, self.hud_cols as f64 * cw + cw)
        } else {
            (pad, 0.0)
        };
        Rect {
            x: pad,
            y: pad + top_bar_px,
            w: (dw as f64 - pad - right_margin_px - right_gutter_px).max(cw),
            h: dh as f64 - 2.0 * pad - top_bar_px - bot_bar_px,
        }
    }

    fn top_bar_rows(&self) -> usize {
        // Chrome row is always present (basin mark + tabs + indicators).
        1
    }

    fn bottom_bar_rows(&self) -> usize {
        // Always one row: status bar normally, search bar when open
        // (they swap in the same row).
        1
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
        let cur = pane.terminal.cursor();
        pane.cursor_ax = cur.x as f32;
        pane.cursor_ay = cur.y as f32;
        pane.scroll_pos = pane.terminal.viewport_offset() as f32;
        pane.overscroll = 0.0;
        pane.overscroll_target = 0.0;
    }

    /// Resize every pane in every tab to reflect the current window size.
    fn resize_all_tabs(&mut self) {
        let ir = self.inner_rect();
        let cw = self.font.metrics.cell_w;
        let ch = self.font.metrics.cell_h;
        let div = DIVIDER_PX;

        for tab in &mut self.tabs.tabs {
            let entries = tab.tree.layout(ir, div);
            for e in &entries {
                let cols = ((e.rect.w / cw) as usize).max(1);
                let rows = ((e.rect.h / ch) as usize).max(1);
                if let Some(pane) = tab.registry.get_mut(e.id) {
                    pane.terminal.resize(cols, rows);
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
        if let Some(r) = &mut self.renderer {
            r.resize(dw, dh);
        }
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

    fn open_search(&mut self) {
        if self.search_open {
            return;
        }
        self.search_open = true;
        // Re-run query against current focused pane terminal.
        let q = self.search.query().to_string();
        if let Some(tab) = self.tabs.current_mut() {
            let id = tab.focused_id();
            if let Some(pane) = tab.registry.get_mut(id) {
                self.search.set_query(&pane.terminal, &q);
            }
        }
        self.resize_all_tabs();
        self.dirty = true;
        self.force_full_redraw = true;
    }

    fn close_search(&mut self) {
        if !self.search_open {
            return;
        }
        self.search_open = false;
        self.resize_all_tabs();
        self.dirty = true;
        self.force_full_redraw = true;
    }

    fn scroll_to_current_match(&mut self) {
        if let Some(m) = self.search.current_match() {
            if let Some(tab) = self.tabs.current_mut() {
                let id = tab.focused_id();
                if let Some(pane) = tab.registry.get_mut(id) {
                    pane.terminal.scroll_to_line(m.row);
                    pane.scroll_pos = pane.terminal.viewport_offset() as f32;
                }
            }
        }
    }

    fn bounce_impulse(&self) -> f32 {
        (self.font.metrics.cell_h * 0.5) as f32
    }

    fn focus_neighbor(&mut self, dir: NavDir) {
        let ir = self.inner_rect();
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
        let ir = self.inner_rect();
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

    fn close_focused_pane(&mut self) {
        let (focused_id, next_id) = {
            let tab = match self.tabs.current_mut() {
                Some(t) => t,
                None => return,
            };
            let focused_id = tab.focused_id();
            let next_id = tab.tree.close_leaf(focused_id);
            tab.registry.remove(focused_id);
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
            let bar_before = self.top_bar_rows();
            if !self.tabs.close_active() {
                terminate_app();
            } else {
                if self.top_bar_rows() != bar_before {
                    self.resize_all_tabs();
                }
                self.snap_anim();
                self.dirty = true;
            }
        }
    }

    fn add_tab(&mut self) {
        self.close_search();
        let (dw, dh) = self.device_size();
        let cw = self.font.metrics.cell_w as usize;
        let ch = self.font.metrics.cell_h as usize;
        let cols = ((dw.saturating_sub(2 * GRID_PAD)) / cw).max(1);
        // Subtract 2 rows: the chrome row at top AND the status row at
        // bottom. Subtracting only 1 made the PTY think it had a free row
        // that the renderer was actually using for the status bar — output
        // and status bar drew on the same pixel band → jumbled glyphs.
        let rows = (((dh.saturating_sub(2 * GRID_PAD)) / ch).saturating_sub(2)).max(1);
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
        self.resize_all_tabs();
        self.snap_anim();
        self.dirty = true;
    }

    fn close_active_tab(&mut self) {
        let bar_before = self.top_bar_rows();
        if !self.tabs.close_active() {
            terminate_app();
        } else {
            if self.top_bar_rows() != bar_before {
                self.resize_all_tabs();
            }
            self.dirty = true;
        }
    }

    /// Close panes whose PTY has gone away (EOF), then close tabs with no panes.
    fn close_dead_panes(&mut self) {
        let bar_before = self.top_bar_rows();
        let mut any_closed = false;

        let mut tab_i = 0;
        while tab_i < self.tabs.tabs.len() {
            // Collect pane ids that no longer have a PTY.
            let dead: Vec<PaneId> = {
                let tab = &self.tabs.tabs[tab_i];
                all_pane_ids_in_tree(tab)
                    .into_iter()
                    .filter(|id| !self.ptys.contains_key(id))
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
            if self.top_bar_rows() != bar_before {
                self.resize_all_tabs();
            }
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
        // The content row currently at the top of the viewport.
        let top_content = pane.terminal.content_row_of_viewport(0);
        let top_abs = pane.terminal.absolute_line_of_content(top_content);

        // Find the block at or just before the viewport top.
        let block_opt = pane
            .terminal
            .block_at(top_abs)
            .or_else(|| pane.terminal.block_before(top_abs + 1));

        if let Some(block) = block_opt {
            pane.toggle_fold(block.command_line);
            self.dirty = true;
        }
    }

    fn jump_to_prev_prompt(&mut self) {
        let bounce = self.bounce_impulse();
        let Some(tab) = self.tabs.current_mut() else {
            return;
        };
        let id = tab.focused_id();
        let Some(pane) = tab.registry.get_mut(id) else {
            return;
        };
        let t = &mut pane.terminal;
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
            pane.scroll_pos = pane.terminal.viewport_offset() as f32;
            pane.overscroll_target = bounce;
            self.dirty = true;
        }
    }

    fn jump_to_next_prompt(&mut self) {
        let bounce = self.bounce_impulse();
        let Some(tab) = self.tabs.current_mut() else {
            return;
        };
        let id = tab.focused_id();
        let Some(pane) = tab.registry.get_mut(id) else {
            return;
        };
        let t = &mut pane.terminal;
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
            pane.scroll_pos = pane.terminal.viewport_offset() as f32;
            pane.overscroll_target = -bounce;
        } else {
            t.scroll_to_bottom();
            pane.scroll_pos = 0.0;
            pane.overscroll_target = -bounce;
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
        let rows = pane.terminal.rows();
        if rows == 0 {
            return None;
        }
        let mut out = String::new();
        for vy in 0..rows {
            let content_row = pane.terminal.content_row_of_viewport(vy);
            if content_row >= pane.terminal.line_count() {
                break;
            }
            let line = pane.terminal.line(content_row);
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
        let mut out = String::new();
        for row in start.row..=end.row {
            if row >= pane.terminal.line_count() {
                break;
            }
            let line = pane.terminal.line(row);
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
            .map(|p| p.terminal.modes.mouse_sgr)
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
        let ir = self.inner_rect();
        let div = DIVIDER_PX;

        let pane_id = tab
            .tree
            .hit_test(ir, div, rx, ry)
            .or_else(|| if clamp { Some(tab.focused_id()) } else { None })?;

        let entries = tab.tree.layout(ir, div);
        let pr = entries.iter().find(|e| e.id == pane_id)?.rect;
        let pane = tab.registry.get(pane_id)?;
        let cw = self.font.metrics.cell_w;
        let ch = self.font.metrics.cell_h;
        let rows = pane.terminal.rows() as f64;
        let cols = pane.terminal.cols() as f64;

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

        // last-run from focused pane
        if let Some(tab) = self.tabs.current() {
            let id = tab.focused_id();
            if let Some(pane) = tab.registry.get(id) {
                let lr = pane.terminal.last_run();
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
            }
        }

        self.dirty = true;
    }

    fn render_frame(&mut self, painter: &mut dyn anvil_render::GlyphPainter) {
        // Scroll animation safeguard: when the focused pane is mid-animation
        // (smooth scroll, overscroll bounce, or the terminal's viewport is
        // offset), force a full raster redraw. Partial-frame dirty-row
        // drawing assumes cell pixels stay at the same y position between
        // frames — that breaks for smooth scroll where the y offset shifts
        // sub-pixel each frame and old cell pixels linger as vertical streaks.
        let animating = self
            .tabs
            .current()
            .and_then(|t| t.registry.get(t.focused_id()))
            .map(|p| {
                p.scroll_pos != 0.0
                    || p.overscroll != 0.0
                    || p.overscroll_target != 0.0
                    || p.terminal.viewport_offset() != 0
            })
            .unwrap_or(false);
        if animating {
            self.force_full_redraw = true;
        }

        let is_full_redraw = self.force_full_redraw;
        self.force_full_redraw = false;

        if is_full_redraw {
            self.raster.clear(self.theme.background);
        } else {
            // On a partial frame, clear only the chrome rows so that chrome
            // draws (tab bar, status bar, panels) always paint on a clean
            // background regardless of what the terminal wrote there before.
            let ch = self.font.metrics.cell_h;
            let pad = GRID_PAD;
            let top_bar_h = (self.top_bar_rows() as f64 * ch) as usize;
            let bot_bar_h = (self.bottom_bar_rows() as f64 * ch) as usize;
            let (_, dh) = self.device_size();
            // Clear top chrome (tab bar).
            if top_bar_h > 0 {
                self.raster
                    .clear_pixel_rows(0, pad + top_bar_h, self.theme.background);
            }
            // Clear bottom chrome (search bar + status bar).
            if bot_bar_h > 0 {
                let bot_start = dh.saturating_sub(pad + bot_bar_h);
                self.raster
                    .clear_pixel_rows(bot_start, dh, self.theme.background);
            }
            // Right HUD strip — clear and let the HUD draw repaint it. Safe
            // now that the initial PTY size matches `inner_rect`: no cells
            // ever extend into this column.
            if self.hud_visible {
                let cw = self.font.metrics.cell_w;
                let (dw, _) = self.device_size();
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
        let pad = GRID_PAD as f64;
        let eff_hud = self.hud_visible;
        let (dw, dh) = self.device_size();

        let inner = Rect {
            x: self.raster.pad_x,
            y: self.raster.pad_y + self.top_bar_rows() as f64 * ch,
            w: dw as f64 - 2.0 * pad,
            h: dh as f64
                - 2.0 * pad
                - self.top_bar_rows() as f64 * ch
                - self.bottom_bar_rows() as f64 * ch,
        };

        let search_ref: Option<&anvil_term::Search> = if self.search_open {
            Some(&self.search)
        } else {
            None
        };
        let metrics = self.font.metrics;

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
                            let rows = pane.terminal.rows();
                            // Drain dirty rows from the terminal model.
                            let mut ds = pane.terminal.take_dirty_rows();
                            // Always redraw the cursor row (blink, move).
                            let cur = pane.terminal.cursor();
                            ds.mark(cur.y);
                            // Also redraw the previous cursor row so stale cursor is erased.
                            if let Some(&prev) = self.cursor_row_prev.get(&e.id) {
                                ds.mark(prev);
                            }
                            // Viewport scroll change → force full for this pane.
                            let scroll_changed = pane.scroll_pos != 0.0
                                || pane.overscroll != 0.0
                                || pane.terminal.viewport_offset() != 0;
                            if scroll_changed {
                                ds.force_full();
                            }
                            // Auto-scroll: scrollback grew since last frame. Every
                            // visible row's pixels shifted up; per-row dirty tracking
                            // leaves the old cursor's pixels orphaned at their old
                            // device-pixel y. Force a full redraw to flush them.
                            let sbl = pane.terminal.scrollback_len();
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
                            // Update cursor_row_prev for next frame.
                            self.cursor_row_prev.insert(e.id, cur.y);
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
                                    let total = pane.terminal.rows();
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
                draw_workspace(
                    &mut self.raster,
                    painter,
                    &tab.tree,
                    &mut tab.registry,
                    inner,
                    DIVIDER_PX,
                    metrics,
                    &self.theme,
                    search_ref,
                    focused_id,
                    self.blink_phase,
                    self.cursor_cfg,
                    dirty_map.as_ref(),
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
            draw_tab_bar(
                &mut self.raster,
                painter,
                metrics,
                &self.theme,
                &self.tabs,
                &branch,
                &clock,
                self.window_scale,
                &mut self.tab_bar_hits,
            );
        }

        // Bottom row: search bar when open, otherwise the slim status bar.
        let total_rows = (((dh.saturating_sub(2 * GRID_PAD)) as f64 / ch) as usize).max(1);
        let bottom_row = total_rows.saturating_sub(1);
        if self.search_open {
            draw_search_bar(
                &mut self.raster,
                painter,
                metrics,
                &self.theme,
                &self.search,
                bottom_row,
            );
        } else {
            let clock = local_hhmm();
            anvil_render::statusbar::draw_status_bar(
                &mut self.raster,
                painter,
                metrics,
                &self.theme,
                &self.local_ctx,
                &self.agent_snap,
                &clock,
                bottom_row,
            );
        }

        // Right-side HUD: docked, edge-to-edge frosted-glass panel with repo
        // / git / agent / system state. Replaces the old bottom status bar
        // and the small floating agent card.
        if eff_hud {
            let cw = self.font.metrics.cell_w;
            let total_rows = (((dh.saturating_sub(2 * GRID_PAD)) as f64 / ch) as usize).max(1);
            let top_offset = self.top_bar_rows();
            let rows = total_rows.saturating_sub(top_offset);

            // Surface: rightmost slab of the window, edge-to-edge.
            let hud_cols = self.hud_cols;
            let surface_w_px = hud_cols as f64 * cw + GRID_PAD as f64;
            let surface_rect = anvil_render::raster::PixelRect {
                x: (dw as f64 - surface_w_px).max(0.0),
                y: 0.0,
                w: surface_w_px.min(dw as f64),
                h: dh as f64,
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
                painter,
                metrics,
                &self.theme,
                &self.agent_snap,
                &self.local_ctx,
                surface_rect,
                start_col,
                hud_cols,
                top_offset,
                rows,
                &mut self.hud_hits,
                &self.hud_section_order,
                &mut self.hud_section_hits,
            );
        }

        // Cheatsheet overlay.
        if self.cheatsheet_visible {
            let cw = self.font.metrics.cell_w;
            let total_rows = (((dh.saturating_sub(2 * GRID_PAD)) as f64 / ch) as usize).max(1);
            let total_cols = (((dw.saturating_sub(2 * GRID_PAD)) as f64 / cw) as usize).max(1);
            draw_cheatsheet(
                &mut self.raster,
                painter,
                metrics,
                &self.theme,
                total_cols,
                total_rows,
            );
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

                    let folded = FoldedBlocks::new(&pane.folded[..pane.folded_count]);

                    draw_viewport_gpu(
                        &mut self.cell_batch,
                        &self.raster,
                        ap,
                        &mut pane.terminal,
                        metrics,
                        &self.theme,
                        pane.scroll_pos,
                        pane.overscroll,
                        pane.selection,
                        search_ref,
                        0, // top_bar_rows: encoded in origin_y
                        cursor_params,
                        folded,
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
    }

    // ── Palette helpers ──────────────────────────────────────────────────────

    fn send_palette_show(&self, webview: &Webview) {
        let cmds: Vec<BridgeCmd> = CATALOG
            .iter()
            .map(|e| BridgeCmd {
                id: e.id.to_string(),
                title: e.title.to_string(),
                subtitle: e.subtitle.map(|s| s.to_string()),
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

    fn dismiss_palette(&mut self, webview: &Webview) {
        self.palette.dismiss();
        if let Ok(json) = bridge_encode(&Outbound::Hide) {
            webview.eval_js(&format!("window.anvil.receive({json});"));
        }
        webview.hide();
    }

    fn handle_palette_action(&mut self, action: Action, webview: &Webview) {
        match action {
            Action::ThemeDark => {
                self.theme = resolve_theme("mineral-dark", &anvil_theme::ThemeOverrides::default());
                if let Some(r) = &mut self.renderer {
                    r.set_clear_color(self.theme.background);
                }
                self.dirty = true;
            }
            Action::ThemeLight => {
                self.theme =
                    resolve_theme("mineral-light", &anvil_theme::ThemeOverrides::default());
                if let Some(r) = &mut self.renderer {
                    r.set_clear_color(self.theme.background);
                }
                self.dirty = true;
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
                        pane.terminal.feed(b"\x1b[H\x1b[2J");
                    }
                }
                self.dirty = true;
            }
            Action::ScrollTop => {
                let bounce = self.bounce_impulse();
                if let Some(tab) = self.tabs.current_mut() {
                    let id = tab.focused_id();
                    if let Some(pane) = tab.registry.get_mut(id) {
                        let len = pane.terminal.scrollback_len() as isize;
                        pane.terminal.scroll_viewport(len);
                        pane.scroll_pos = pane.terminal.viewport_offset() as f32;
                        pane.overscroll_target = bounce;
                    }
                }
                self.dirty = true;
            }
            Action::ScrollBottom => {
                let bounce = self.bounce_impulse();
                if let Some(tab) = self.tabs.current_mut() {
                    let id = tab.focused_id();
                    if let Some(pane) = tab.registry.get_mut(id) {
                        pane.terminal.scroll_to_bottom();
                        pane.scroll_pos = 0.0;
                        pane.overscroll_target = -bounce;
                    }
                }
                self.dirty = true;
            }
            Action::AppQuit => {
                terminate_app();
                return;
            }
            Action::HudToggle => {
                self.hud_visible = !self.hud_visible;
                // Grid width changes when the HUD toggles — reflow the panes
                // and PTYs to the new inner rect.
                self.resize_all_tabs();
                self.dirty = true;
            }
            Action::CheatsheetShow => {
                self.cheatsheet_visible = true;
                self.dirty = true;
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
            self.dirty = true;
            return true;
        });
        test!(kb.prev_tab, {
            self.close_search();
            self.tabs.prev();
            self.snap_anim();
            self.dirty = true;
            return true;
        });
        for (i, maybe) in kb.jump.iter().enumerate() {
            if let Some(chord) = maybe {
                if Self::chord_matches(*chord, mods, ch) {
                    self.close_search();
                    self.tabs.switch_to(i);
                    self.snap_anim();
                    self.dirty = true;
                    return true;
                }
            }
        }
        test!(kb.search_open, {
            self.open_search();
            return true;
        });
        test!(kb.search_next, {
            if !self.search_open {
                self.open_search();
            }
            self.search.next();
            self.scroll_to_current_match();
            self.dirty = true;
            return true;
        });
        test!(kb.search_prev, {
            if !self.search_open {
                self.open_search();
            }
            self.search.prev();
            self.scroll_to_current_match();
            self.dirty = true;
            return true;
        });
        // Cmd+Opt+R: toggle regex mode (only when search is open).
        test!(kb.search_regex_toggle, {
            if self.search_open {
                let new_mode = !self.search.is_regex();
                self.search.set_regex(new_mode);
                // Re-run the scan with the new mode.
                if let Some(tab) = self.tabs.current_mut() {
                    let id = tab.focused_id();
                    if let Some(pane) = tab.registry.get_mut(id) {
                        self.search.rescan(&pane.terminal);
                    }
                }
                self.scroll_to_current_match();
                self.dirty = true;
            }
            return true;
        });
        test!(kb.hud_toggle, {
            self.hud_visible = !self.hud_visible;
            self.resize_all_tabs();
            self.dirty = true;
            return true;
        });
        test!(kb.cheatsheet, {
            self.cheatsheet_visible = !self.cheatsheet_visible;
            self.dirty = true;
            return true;
        });
        test!(kb.fold_block, {
            self.toggle_fold_at_viewport_top();
            return true;
        });

        // ⌘K — command palette.
        if ascii_lower(ch) == 'k' && !mods.shift && !mods.control && !mods.option {
            if self.palette.summon() {
                self.send_palette_show(webview);
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
                    .map(|p| p.terminal.modes.bracketed_paste)
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
}

// ── AppShell — holds App + Webview + Font + Painter, impls AppHandler ────────

/// Holds all state that requires the main thread or has lifetimes that depend
/// on the window (Webview, Font, Painter).
///
/// `app.font` is heap-allocated (`Box<Font>`) so its address is stable even as
/// `AppShell` moves.  `painter` holds a `&'static Font` produced via an unsafe
/// lifetime extension; this is sound because `painter` is dropped before
/// `app.font` (struct fields drop in declaration order, and `painter` is
/// declared after `app`).
pub struct AppShell {
    app: App,
    webview: Webview,
    painter: anvil_platform::font::CoreTextPainter<'static>,
}

impl AppShell {
    /// Cmd+/Cmd-/Cmd0 chord: zoom in, out, or reset the font size.
    fn handle_zoom_chord(&mut self, ch: char) {
        match ch {
            '=' | '+' => self.bump_font_size(1.0),
            '-' => self.bump_font_size(-1.0),
            '0' => {
                // Reset to default 15 pt (matches startup config default).
                let delta = 15.0 - self.app.font_size_pt;
                self.bump_font_size(delta);
            }
            _ => {}
        }
    }

    /// Rebuild the font at a new point size and recreate the dependent
    /// painter + raster geometry. Drives Cmd+/Cmd- zoom.
    ///
    /// Clamps to [8.0, 48.0] pt — below 8 pt cell metrics collapse, above
    /// 48 pt the glyph atlas balloons and one cell barely fits a word.
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
        // Replace the heap-stable font allocation. `std::mem::replace`
        // returns the old Box — KEEP IT ALIVE until *after* we've
        // rebuilt the painter, because the current painter borrows the
        // old heap allocation. Dropping the painter (when we overwrite
        // it below) re-reads from that pointer.
        let old_font = std::mem::replace(&mut self.app.font, Box::new(new_font));
        self.app.font_size_pt = new_pt;
        // SAFETY: same lifetime-extension pattern as `AppShell::new` —
        // `app.font` is a Box and its heap allocation outlives the painter.
        self.painter = unsafe {
            let font_ref: &'static Font = &*(self.app.font.as_ref() as *const Font);
            anvil_platform::font::CoreTextPainter::new(font_ref)
        };
        // Now the old painter has been dropped (it was overwritten just
        // above, with its destructor running against the still-live
        // `old_font` allocation). Release the old heap.
        drop(old_font);
        // Reflow panes + force a full redraw so the new metrics propagate.
        self.app.resize_all_tabs();
        self.app.force_full_redraw = true;
        self.app.dirty = true;
    }

    fn new(app: App, webview: Webview) -> Self {
        // SAFETY: `app.font` is a `Box<Font>` — the heap allocation is stable.
        // `painter` borrows the Font inside the box; the box (and its allocation)
        // lives inside `app` which lives inside `AppShell`.  Drop order is
        // `painter` first (declared last), then `webview`, then `app` (with the
        // Box).  So the allocation outlives `painter`.
        let painter = unsafe {
            let font_ref: &'static Font = &*(app.font.as_ref() as *const Font);
            anvil_platform::font::CoreTextPainter::new(font_ref)
        };
        Self {
            app,
            webview,
            painter,
        }
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

        // Drain every pane's PTY output.
        let mut any_dead = false;
        let mut feed_buf = vec![0u8; 64 * 1024];
        let tab_count = app.tabs.tabs.len();

        for ti in 0..tab_count {
            // Collect pane ids for this tab by walking the layout tree.
            let pane_ids = all_pane_ids_in_tree(&app.tabs.tabs[ti]);
            for pid in pane_ids {
                let result = app.ptys.get(&pid).map(|pty| pty.read(&mut feed_buf));
                match result {
                    Some(Ok(n)) if n > 0 => {
                        let bytes = feed_buf[..n].to_vec();
                        if let Some(pane) = app.tabs.tabs[ti].registry.get_mut(pid) {
                            pane.terminal.feed(&bytes);
                        }
                        let active = app.tabs.active;
                        if ti == active {
                            let focused = app.tabs.tabs[ti].focused_id();
                            if pid == focused {
                                app.dirty = true;
                                if app.search_open {
                                    if let Some(pane) = app.tabs.tabs[ti].registry.get_mut(pid) {
                                        app.search.rescan(&pane.terminal);
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
                    Some(Ok(_)) => {}
                    Some(Err(_)) | None => {
                        app.ptys.remove(&pid);
                        any_dead = true;
                    }
                }
            }
        }

        // Blink.
        let (effective_blink, app_blink_cfg) = {
            let tab = app.tabs.current();
            let pane = tab.and_then(|t| t.registry.get(t.focused_id()));
            (
                pane.and_then(|p| p.terminal.app_cursor_blink),
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

        // Cursor: snap to target every frame. No animation.
        //
        // The previous code glided the cursor over ~6 ticks (~100ms). During
        // that glide the cursor was drawn at fractional rows that the dirty
        // tracker didn't know about, leaving stale cursor pixels at every
        // intermediate row — the "cursor trail" bug.
        let approach = |cur: f32, target: f32, rate: f32| -> f32 { cur + (target - cur) * rate };
        if let Some(tab) = app.tabs.current_mut() {
            let id = tab.focused_id();
            if let Some(pane) = tab.registry.get_mut(id) {
                let cur = pane.terminal.cursor();
                let tx = cur.x as f32;
                let ty = cur.y as f32;
                if (tx - pane.cursor_ax).abs() > 0.0 || (ty - pane.cursor_ay).abs() > 0.0 {
                    pane.cursor_ax = tx;
                    pane.cursor_ay = ty;
                    app.dirty = true;
                }
                if pane.overscroll != 0.0 || pane.overscroll_target != 0.0 {
                    pane.overscroll_target = approach(pane.overscroll_target, 0.0, 0.32);
                    pane.overscroll = approach(pane.overscroll, pane.overscroll_target, 0.55);
                    if pane.overscroll_target.abs() < 0.5 {
                        pane.overscroll_target = 0.0;
                    }
                    if pane.overscroll.abs() < 0.5 && pane.overscroll_target == 0.0 {
                        pane.overscroll = 0.0;
                    }
                    app.dirty = true;
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

        if app.dirty {
            app.render_frame(&mut self.painter);
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
            return;
        }

        // HUD: Esc closes it (Cmd+J still toggles).
        if self.app.hud_visible && event.key == KeyInput::Escape && !event.mods.command {
            self.app.hud_visible = false;
            self.app.resize_all_tabs();
            self.app.dirty = true;
            return;
        }

        // Search bar handling.
        if self.app.search_open {
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
                            self.app.search.set_query(&pane.terminal, &new_q);
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
                            self.app.search.set_query(&pane.terminal, &new_q);
                        }
                    }
                    self.app.scroll_to_current_match();
                    self.app.dirty = true;
                }
                _ => {}
            }
            return;
        }

        // Normal key → encode and write to PTY.
        let app_cursor = self
            .app
            .tabs
            .current()
            .and_then(|t| t.registry.get(t.focused_id()))
            .map(|p| p.terminal.modes.app_cursor_keys)
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
                pane.terminal.scroll_to_bottom();
                pane.scroll_pos = 0.0;
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
                KeyInput::Char(ch) => {
                    if self.app.handle_cmd_chord(event.mods, ch, &self.webview) {
                        return true;
                    }
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
                        app.dirty = true;
                    }
                    TabBarHitKind::CloseTab(idx) => {
                        app.tabs.switch_to(idx);
                        app.close_active_tab();
                    }
                    TabBarHitKind::AddTab => {
                        app.add_tab();
                    }
                }
                return;
            }
        }

        // Click-to-focus.
        {
            let (rx, ry) = app.view_pt_to_raster_px(loc);
            let ir = app.inner_rect();
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

        // Mouse reporting.
        {
            let (btn_mode, x10_mode) = app
                .tabs
                .current()
                .and_then(|t| t.registry.get(t.focused_id()))
                .map(|p| (p.terminal.modes.mouse_button, p.terminal.modes.mouse_x10))
                .unwrap_or((false, false));
            if btn_mode || x10_mode {
                if let Some((row, col)) = app.event_cell(loc, false) {
                    app.write_mouse_event(0, col, row, true);
                }
                return;
            }
        }

        // ⌘-click: open file/url under cursor.
        if mods.command {
            if let Some((row, col)) = app.event_cell(loc, false) {
                let (content_row, cells): (usize, Vec<anvil_term::Cell>) = {
                    let tab = app.tabs.current_mut().unwrap();
                    let id = tab.focused_id();
                    let pane = tab.registry.get_mut(id).unwrap();
                    let cr = pane.terminal.content_row_of_viewport(row);
                    (cr, pane.terminal.line(cr).to_vec())
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
                    let cr = pane.terminal.content_row_of_viewport(row);

                    if mods.shift && pane.selection.active {
                        // Extend the current selection to the click point.
                        pane.selection.head = Point { row: cr, col };
                    } else if click_count >= 3 {
                        // Whole-line selection.
                        let line_len = pane.terminal.line(cr).len();
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
                        let line = pane.terminal.line(cr).to_vec();
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
                        let insert_at = if to > from { to } else { to };
                        let insert_at = insert_at.min(order.len());
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
            .map(|p| (p.terminal.modes.mouse_button, p.terminal.modes.mouse_sgr))
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
            .map(|p| p.terminal.modes.mouse_button)
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
                    let cr = pane.terminal.content_row_of_viewport(row);
                    pane.selection.head = anvil_workspace::selection::Point { row: cr, col };
                }
            }
        }
        app.dirty = true;
    }

    fn scroll(&mut self, dy: f64, loc: MouseLocation) {
        let app = &mut self.app;
        if dy == 0.0 {
            return;
        }

        // Mouse reporting scroll.
        let (btn_mode, x10_mode) = app
            .tabs
            .current()
            .and_then(|t| t.registry.get(t.focused_id()))
            .map(|p| (p.terminal.modes.mouse_button, p.terminal.modes.mouse_x10))
            .unwrap_or((false, false));
        if btn_mode || x10_mode {
            if let Some((row, col)) = app.event_cell(loc, false) {
                let btn: u8 = if dy > 0.0 { 64 } else { 65 };
                app.write_mouse_event(btn, col, row, true);
            }
            return;
        }

        let d = (dy / 8.0) as f32;
        let ch = app.font.metrics.cell_h as f32;
        let limit = ch * 1.5;

        if let Some(tab) = app.tabs.current_mut() {
            let id = tab.focused_id();
            if let Some(pane) = tab.registry.get_mut(id) {
                let max_pos = pane.terminal.scrollback_len() as f32;
                let mut np = pane.scroll_pos + d;
                if np > max_pos {
                    let excess = np - max_pos;
                    let resist = 1.0 - (pane.overscroll_target.abs() / limit).min(1.0);
                    pane.overscroll_target =
                        (pane.overscroll_target + excess * ch * 0.30 * resist).clamp(-limit, limit);
                    np = max_pos;
                } else if np < 0.0 {
                    let excess = np;
                    let resist = 1.0 - (pane.overscroll_target.abs() / limit).min(1.0);
                    pane.overscroll_target =
                        (pane.overscroll_target + excess * ch * 0.30 * resist).clamp(-limit, limit);
                    np = 0.0;
                }
                pane.scroll_pos = np;
                pane.terminal.set_viewport_offset(np.round() as usize);
            }
        }
        app.dirty = true;
    }

    fn resize(&mut self, width: f64, height: f64, in_live_resize: bool) {
        self.app.view_width_pt = width;
        self.app.view_height_pt = height;
        self.app.resize_surface();
        if !in_live_resize {
            self.app.resize_all_tabs();
        }
        self.app.render_frame(&mut self.painter);
    }

    fn live_resize_ended(&mut self) {
        self.app.resize_all_tabs();
        self.app.render_frame(&mut self.painter);
    }

    fn focus_gained(&mut self) {
        self.app.focused = true;
    }
    fn focus_lost(&mut self) {
        self.app.focused = false;
    }
    fn should_terminate(&mut self) -> bool {
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
                if let Some(action) = action_for_id(&id) {
                    self.app.handle_palette_action(action, &self.webview);
                } else {
                    eprintln!("anvil: unknown command id: {id}");
                }
            }
            Err(e) => eprintln!("anvil: webview message decode failed: {e}"),
        }
    }
}

// ── Key conversion ────────────────────────────────────────────────────────────

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
            "mineral-dark"
        } else {
            "mineral-light"
        }
    } else {
        cfg_theme
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

// ── main ──────────────────────────────────────────────────────────────────────

fn main() -> Result<()> {
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

    // -- Cell geometry --------------------------------------------------------
    let cw = font.metrics.cell_w as usize;
    let ch = font.metrics.cell_h as usize;
    // Initial PTY size must match what `App::inner_rect` will report once the
    // App is constructed (with `hud_visible = true` by default), otherwise the
    // first frames render at the wrong column count and scrollback comes back
    // mis-shaped. Mirror the reservation: drop the right `GRID_PAD` (the HUD
    // absorbs it) and reserve `HUD_COLS_DEFAULT + 1` cells for the docked panel.
    // HUD is now default-off — reserve only the padding + chrome + status rows.
    // (resize_all_tabs corrects to exact pane size once the window is up,
    // but the initial PTY needs sane dimensions for the first prompt frame.)
    let cols = (dw.saturating_sub(2 * GRID_PAD) / cw).max(1);
    let rows = (((dh.saturating_sub(2 * GRID_PAD)) / ch).saturating_sub(2)).max(1);

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

    // -- Theme ----------------------------------------------------------------
    let system_dark = system_is_dark();
    let effective_name = effective_theme_name(system_dark, &config.theme);
    let theme = resolve_theme(effective_name, &config.theme_overrides);
    let cursor_cfg = cursor_cfg_from_config(&config);
    let keybindings = Keybindings::from_config(&config.keybindings);

    // -- Raster ---------------------------------------------------------------
    let mut raster = Raster::new(dw, dh);
    raster.pad_x = GRID_PAD as f64;
    raster.pad_y = GRID_PAD as f64;

    // -- Build App ------------------------------------------------------------
    let watcher = cfg_path.map(Watcher::new);

    // -- Render path selection ------------------------------------------------
    let use_gpu_render = matches!(std::env::var("ANVIL_RENDER").as_deref(), Ok("gpu"));
    eprintln!(
        "anvil: render = {}",
        if use_gpu_render { "gpu" } else { "cpu" }
    );

    let app = App {
        tabs,
        ptys,
        renderer: None, // filled after the Metal layer is available
        raster,
        font, // Box<Font> — heap-stable
        dirty: true,
        force_full_redraw: true,
        cursor_row_prev: HashMap::new(),
        scrollback_len_prev: HashMap::new(),
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
        blink_phase: 0.0,
        last_blink_opacity: -1.0,
        search: anvil_term::Search::new(),
        search_open: false,
        // Docked right HUD on by default — Cmd+J toggles it.
        hud_visible: false,
        hud_tick: 0,
        hud_cols: HUD_COLS_DEFAULT,
        hud_drag_active: false,
        tab_bar_hits: TabBarHits::default(),
        hud_hits: Vec::new(),
        hud_section_order: load_hud_section_order()
            .unwrap_or_else(|| SectionId::DEFAULT_ORDER.to_vec()),
        hud_section_hits: Vec::new(),
        hud_section_drag: None,
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
        git_tx,
        git_rx,
        recent_cwd_tx,
        recent_rx,
        palette: Palette::default(),
        view_width_pt: win_w,
        view_height_pt: win_h,
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
        fn scroll(&mut self, dy: f64, l: MouseLocation) {
            if let Some(h) = &mut *self.0.borrow_mut() {
                h.scroll(dy, l)
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
    let mut shell = AppShell::new(real_app, webview);
    shell.app.snap_anim();
    shell.app.render_frame(&mut shell.painter);
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
        assert_eq!(effective_theme_name(true, "system"), "mineral-dark");
        assert_eq!(effective_theme_name(false, "system"), "mineral-light");
        assert_eq!(effective_theme_name(true, "mineral-light"), "mineral-light");
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
}
