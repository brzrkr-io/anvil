//! Agent-panel — the primary developer-context surface for Anvil.
//!
//! Draws a floating card (top-right corner by default) showing:
//!   - A header row: status bullet + "agents" label + a one-line summary.
//!   - Up to 3 priority rows (pending approvals → running runs → failure findings).
//!   - A footer: cwd · branch · last-run.
//!
//! Brand: Mineral palette, IBM Plex Mono (the raster font), alloy-grey labels,
//! semantic status colors (verified green / failure red / attention amber /
//! agent violet / info teal).

use std::fmt::Write as FmtWrite;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

use anvil_agent::{Connection, FindingSeverity, RunStatus, Snapshot};
use anvil_theme::Theme;

use crate::raster::{FontMetrics, GlyphPainter, PixelRect, Raster};

/// HUD-local theme tone — keeps text legible against the deep-glass surface
/// regardless of the active app theme (light/dark canvas, same dark panel).
struct TonedTheme {
    foreground: [u8; 3],
}

// --- Brand color constants (Mineral palette) --------------------------------

/// alloy: muted labels / metadata (#86919a)
const ALLOY: [u8; 3] = [0x86, 0x91, 0x9a];
/// status.verified: success / passing (#3f8a5b)
const VERIFIED: [u8; 3] = [0x3f, 0x8a, 0x5b];
/// status.failure: failed check (#b13a30)
const FAILURE: [u8; 3] = [0xb1, 0x3a, 0x30];
/// status.attention: reviewable warning / pending action (#b07a14)
const ATTENTION: [u8; 3] = [0xb0, 0x7a, 0x14];
/// status.agent: agent / automation / model activity — violet (#6a5fa3)
const AGENT_VIOLET: [u8; 3] = [0x6a, 0x5f, 0xa3];

// --- Data types -------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GitState {
    Ok,
    Dirty,
    NoRepo,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunState {
    Idle,
    Ok,
    Failed,
}

/// Local context: cwd, git, last-run. Drives the right-side HUD.
pub struct LocalContext {
    // cwd section
    pub cwd: String,

    // git section
    pub git: GitState,
    pub branch: String,
    pub git_dirty: u32,
    pub git_ahead: u32,
    pub git_behind: u32,
    /// Short HEAD sha (e.g. `0d6726f`). Empty when unknown.
    pub head_short: String,
    /// First line of the HEAD commit message. Empty when unknown.
    pub head_subject: String,

    // last-run section
    pub run: RunState,
    pub run_exit: i32,
    pub run_duration_ms: i64,

    // BUILD section (task #9): detected project kind ("rust", "node", "make").
    pub project_kind: Option<String>,

    // PORTS section (task #7): locally-listening TCP ports (dev servers).
    pub ports: Vec<u16>,

    // RECENT section (task #8): recently-modified files (basenames), max 5.
    pub recent_files: Vec<String>,

    // KUBE section (task #20): current kubectl context.
    pub kube_context: Option<anvil_prompt_core::KubeCtx>,

    // CI section: last CI run state on the active branch.
    // None until a gh-ci poller is wired; section is hidden when None.
    pub ci_status: Option<anvil_prompt_core::CiStatus>,
}

impl Default for LocalContext {
    fn default() -> Self {
        Self {
            cwd: String::new(),
            git: GitState::NoRepo,
            branch: String::new(),
            git_dirty: 0,
            git_ahead: 0,
            git_behind: 0,
            head_short: String::new(),
            head_subject: String::new(),
            run: RunState::Idle,
            run_exit: 0,
            run_duration_ms: 0,
            project_kind: None,
            ports: Vec::new(),
            recent_files: Vec::new(),
            kube_context: None,
            ci_status: None,
        }
    }
}

/// Where to position the card.
pub enum Placement {
    /// Floating card in the top-right corner of the terminal area.
    Floating {
        total_cols: usize,
        total_rows: usize,
        top_offset: usize,
    },
    /// Full-height right-side HUD column.
    Right {
        /// First terminal column the HUD occupies (the 1-col gutter sits to its left).
        start_col: usize,
        hud_cols: usize,
        top_row: usize,
        rows: usize,
    },
}

/// Width of the agent-panel card in terminal columns.
pub const PANEL_COLS: usize = 36;

/// Width of the docked right-side HUD in terminal columns.
pub const HUD_COLS: usize = 34;

/// Dynamic card height: 4 base rows + up to 3 priority rows.
fn card_rows(snap: &Snapshot) -> usize {
    let approvals = snap.approvals.len().min(3);
    let running = snap
        .runs
        .iter()
        .filter(|r| r.status == RunStatus::Running)
        .count()
        .min(3);
    let failures = snap
        .findings
        .iter()
        .filter(|f| f.severity == FindingSeverity::Failure)
        .count()
        .min(3);
    let priority = (approvals + running + failures).min(3);
    4 + priority
}

// --- Formatting helpers (pure, unit-testable) --------------------------------

/// Format a duration in milliseconds as a compact human string.
/// Returns e.g. "0.3s", "1.2s", "72s".
pub fn format_duration(ms: i64) -> String {
    if ms < 0 {
        return "0s".to_string();
    }
    let s = ms / 1000;
    let frac = (ms % 1000) / 100; // tenths
    if s < 10 {
        format!("{s}.{frac}s")
    } else {
        format!("{s}s")
    }
}

/// Format the last-run outcome as a compact status string.
/// E.g. "ok · 1.2s", "failed 1 · 0.5s", "idle"
pub fn format_run_status(run: RunState, exit_code: i32, duration_ms: i64) -> String {
    let dur = format_duration(duration_ms);
    // U+00B7 middle dot
    match run {
        RunState::Idle => "idle".to_string(),
        RunState::Ok => format!("ok \u{00b7} {dur}"),
        RunState::Failed => format!("failed {exit_code} \u{00b7} {dur}"),
    }
}

/// Format ahead/behind counts as a compact string.
pub fn format_ahead_behind(ahead: u32, behind: u32) -> String {
    if ahead == 0 && behind == 0 {
        return String::new();
    }
    // U+2191 ↑, U+2193 ↓
    if ahead > 0 && behind == 0 {
        return format!("\u{2191}{ahead}");
    }
    if ahead == 0 && behind > 0 {
        return format!("\u{2193}{behind}");
    }
    format!("\u{2191}{ahead} \u{2193}{behind}")
}

/// Shorten a filesystem path to its last two components, prefixed with "…/".
pub fn format_cwd(path: &str) -> String {
    if path.is_empty() {
        return String::new();
    }
    let p = if path.len() > 1 && path.ends_with('/') {
        &path[..path.len() - 1]
    } else {
        path
    };

    let last = match p.rfind('/') {
        None => return p.to_string(),
        Some(i) => i,
    };
    if last == 0 {
        return p.to_string();
    }
    let prev = match p[..last].rfind('/') {
        None => return p.to_string(),
        Some(i) => i,
    };
    // U+2026 HORIZONTAL ELLIPSIS …
    format!("\u{2026}/{}", &p[prev + 1..])
}

// --- Header helpers ---------------------------------------------------------

/// Determine the bullet color from the current snapshot state.
pub fn header_bullet_color(snap: &Snapshot) -> [u8; 3] {
    match snap.connection {
        Connection::NotInstalled
        | Connection::NoProject
        | Connection::Disabled
        | Connection::Offline
        | Connection::ErrorState => ALLOY,
        Connection::Live => {
            // Worst-state priority: failure > attention > agent-active > all-clear.
            if snap
                .findings
                .iter()
                .any(|f| f.severity == FindingSeverity::Failure)
            {
                return FAILURE;
            }
            if snap.pending_approvals_count > 0 {
                return ATTENTION;
            }
            if snap.running_count > 0 {
                return AGENT_VIOLET;
            }
            VERIFIED
        }
    }
}

/// Build the single-line summary that appears next to "agents" in the header.
pub fn build_header_summary(snap: &Snapshot) -> String {
    match snap.connection {
        // No-signal states: stay quiet. The dim header bullet already
        // says "no signal"; a diagnostic sentence in the HUD reads as a
        // complaint, not context. If the user wants details, status
        // commands surface them.
        Connection::NotInstalled | Connection::NoProject | Connection::Disabled => String::new(),
        Connection::Offline => "offline".to_string(),
        Connection::ErrorState => "error".to_string(),
        Connection::Live => {
            if snap.running_count == 0
                && snap.pending_approvals_count == 0
                && snap.attention_count == 0
            {
                return "idle".to_string();
            }
            let mut parts: Vec<String> = Vec::with_capacity(3);
            if snap.running_count > 0 {
                parts.push(format!("{} running", snap.running_count));
            }
            if snap.pending_approvals_count > 0 {
                let n = snap.pending_approvals_count;
                let plural = if n == 1 { "" } else { "s" };
                parts.push(format!("{n} approval{plural}"));
            }
            if snap.attention_count > 0 {
                parts.push(format!("{} attention", snap.attention_count));
            }
            if parts.is_empty() {
                return "no active runs".to_string();
            }
            // Join with " · " (U+00B7 middle dot)
            parts.join(" \u{00b7} ")
        }
    }
}

// --- Draw -------------------------------------------------------------------

/// Draw the agent-panel card.
///
/// `snap`      — current agent state.
/// `local`     — cwd / git / last-run for the footer row.
/// `placement` — where to position the card.
/// `expanded`  — when true, a taller version with section headers is drawn.
///               In AG1, collapsed is the priority.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    raster: &mut Raster,
    painter: &mut dyn GlyphPainter,
    metrics: FontMetrics,
    theme: &Theme,
    snap: &Snapshot,
    local: &LocalContext,
    placement: &Placement,
    _expanded: bool,
) {
    // Resolve card coordinates from placement.
    let (card_col, card_row, available) = match placement {
        Placement::Floating {
            total_cols,
            total_rows,
            top_offset,
        } => {
            let tc = *total_cols;
            let tr = *total_rows;
            let to = *top_offset;
            if tr == 0 || tc < PANEL_COLS + 2 {
                return;
            }
            let cc = tc - PANEL_COLS - 2;
            let cr = to + 1;
            (cc, cr, tr)
        }
        // The docked HUD has its own entry point (`draw_right_hud`); this
        // variant is not handled by the floating-card renderer.
        Placement::Right { .. } => return,
    };

    let actual_rows = card_rows(snap).min(available);
    if actual_rows == 0 {
        return;
    }

    // --- Panel background & border ------------------------------------------
    let cw = metrics.cell_w;
    let ch = metrics.cell_h;
    let left_px = raster.pad_x + card_col as f64 * cw;
    let top_px = raster.pad_y + card_row as f64 * ch;
    let card_w_px = PANEL_COLS as f64 * cw;
    let card_h_px = actual_rows as f64 * ch;

    // Subtle: surface fill only — no border. The fill differentiates the
    // panel from terminal content; the absence of a border lets it read as
    // an inset note, not a chrome window.
    raster.fill_pixel_rect(left_px, top_px, card_w_px, card_h_px, theme.surface);

    // --- Content rows --------------------------------------------------------
    let mut row = card_row + 1; // one row breathing room at the top
    let max_row = card_row + actual_rows;

    // --- Header row: bullet + "agents" + summary ----------------------------
    if row < max_row {
        let bullet_color = header_bullet_color(snap);
        let summary = build_header_summary(snap);
        draw_agent_header(
            raster,
            painter,
            metrics,
            theme,
            card_col,
            row,
            bullet_color,
            &summary,
            PANEL_COLS,
        );
        row += 1;
    }

    // --- Priority rows (up to 3): approvals → running → failures ------------
    let mut priority_count = 0_usize;

    // Pending approvals first.
    for ap in &snap.approvals {
        if priority_count >= 3 || row >= max_row {
            break;
        }
        // U+25B8 BLACK RIGHT-POINTING SMALL TRIANGLE ▸
        draw_priority_row(
            raster,
            painter,
            metrics,
            card_col,
            row,
            "\u{25b8}",
            ATTENTION,
            &ap.connector,
            PANEL_COLS,
        );
        row += 1;
        priority_count += 1;
    }

    // Running runs.
    for run in &snap.runs {
        if priority_count >= 3 || row >= max_row {
            break;
        }
        if run.status != RunStatus::Running {
            continue;
        }
        // U+25C6 BLACK DIAMOND ◆
        draw_priority_row(
            raster,
            painter,
            metrics,
            card_col,
            row,
            "\u{25c6}",
            AGENT_VIOLET,
            &run.agent,
            PANEL_COLS,
        );
        row += 1;
        priority_count += 1;
    }

    // Failure findings.
    for finding in &snap.findings {
        if priority_count >= 3 || row >= max_row {
            break;
        }
        if finding.severity != FindingSeverity::Failure {
            continue;
        }
        // U+2717 BALLOT X ✗
        draw_priority_row(
            raster,
            painter,
            metrics,
            card_col,
            row,
            "\u{2717}",
            FAILURE,
            &finding.summary,
            PANEL_COLS,
        );
        row += 1;
        priority_count += 1;
    }

    // Separator before the footer.
    if row < max_row {
        draw_hairline(raster, metrics, theme, card_col, row, PANEL_COLS);
    }
    row += 1;

    // --- Footer: Local context (cwd · branch · last-run) --------------------
    if row < max_row {
        draw_local_footer(raster, painter, metrics, card_col, row, local, PANEL_COLS);
    }
}

// --- Right-side HUD ---------------------------------------------------------

/// Brand color constants exported for callers that need to compose rows.
const INFO_TEAL: [u8; 3] = [0x3a, 0x8a, 0x9d];

/// Theme-aware tones for the docked HUD's frosted-glass surface. Computed
/// per-frame from `theme.background` luminance so the panel feels like the
/// right material on either light or dark canvases — light mode gets a warm
/// pale glass with dark ink; dark mode gets a deep cool slate with light ink.
struct GlassTones {
    /// Surface fill color (composited at `surface_alpha` over the canvas).
    surface: [u8; 3],
    /// How much of the canvas shows through the surface (0.0–1.0).
    surface_alpha: f64,
    /// 1px hairline on the HUD's left edge.
    edge: [u8; 3],
    /// Section header text (REPO / GIT / …) — quieter than body text.
    label: [u8; 3],
    /// Primary body text on the glass.
    foreground: [u8; 3],
    /// Dimmer metadata text on the glass (parent path, time, idle state).
    meta: [u8; 3],
}

/// Relative luminance, ITU-R BT.709, on 0–255 sRGB (approximate; not gamma
/// corrected — good enough to choose a light/dark palette).
fn luma(rgb: [u8; 3]) -> f64 {
    0.2126 * rgb[0] as f64 + 0.7152 * rgb[1] as f64 + 0.0722 * rgb[2] as f64
}

fn glass_tones_for(theme: &Theme) -> GlassTones {
    if luma(theme.background) < 128.0 {
        // Dark canvas → deep cool slate panel with warm off-white ink.
        GlassTones {
            surface: [0x14, 0x18, 0x21],
            surface_alpha: 0.88,
            edge: [0x2a, 0x30, 0x3c],
            label: [0x6b, 0x76, 0x82],
            foreground: [0xd6, 0xdc, 0xe4],
            meta: [0x86, 0x91, 0x9a],
        }
    } else {
        // Light canvas → warm pale glass with cool dark ink — same role as
        // a macOS Mail / Finder sidebar in light mode.
        GlassTones {
            surface: [0xe3, 0xe7, 0xed],
            surface_alpha: 0.72,
            edge: [0xc6, 0xcd, 0xd6],
            label: [0x7a, 0x83, 0x90],
            foreground: [0x24, 0x2a, 0x33],
            meta: [0x55, 0x5e, 0x6b],
        }
    }
}

/// A clickable region inside the HUD. Click → copy text; Cmd-click → open
/// path/URL in the user's default app. Empty strings disable that gesture.
#[derive(Clone, Debug)]
pub struct HudHit {
    /// Hit rect in device pixels.
    pub rect: PixelRect,
    /// Text copied to clipboard on plain click. Empty disables copy.
    pub copy: String,
    /// Path or URL opened on Cmd-click. Empty disables open.
    pub open: String,
}

/// Identifies a HUD section so the caller can persist a custom display
/// order and route drag-to-reorder gestures.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SectionId {
    Context,
    RepoGit,
    Ci,
    Ports,
    Recent,
    Agents,
    System,
}

impl SectionId {
    /// Default top-to-bottom order: CONTEXT → REPO+GIT → CI → AGENTS → RECENT → PORTS → SYSTEM.
    pub const DEFAULT_ORDER: [SectionId; 7] = [
        SectionId::Context,
        SectionId::RepoGit,
        SectionId::Ci,
        SectionId::Agents,
        SectionId::Recent,
        SectionId::Ports,
        SectionId::System,
    ];

    /// Stable string token used when persisting the order to disk.
    pub fn token(self) -> &'static str {
        match self {
            SectionId::Context => "context",
            SectionId::RepoGit => "repo_git",
            SectionId::Ci => "ci",
            SectionId::Ports => "ports",
            SectionId::Recent => "recent",
            SectionId::Agents => "agents",
            SectionId::System => "system",
        }
    }

    /// Inverse of `token` — parse the persisted name back to an id.
    pub fn from_token(s: &str) -> Option<SectionId> {
        match s.trim() {
            "context" => Some(SectionId::Context),
            "repo_git" => Some(SectionId::RepoGit),
            "ci" => Some(SectionId::Ci),
            "ports" => Some(SectionId::Ports),
            "recent" => Some(SectionId::Recent),
            "agents" => Some(SectionId::Agents),
            "system" => Some(SectionId::System),
            _ => None,
        }
    }
}

/// A section-header hit zone reported back by `draw_right_hud`. The caller
/// uses it to start a drag-to-reorder gesture; on release the section
/// whose `rect` contains the release point becomes the drop target.
#[derive(Clone, Debug)]
pub struct SectionHeaderHit {
    pub section: SectionId,
    pub rect: PixelRect,
}

/// Render the always-on right-side HUD.
///
/// `surface_rect` is the *pixel* rect the HUD's frosted surface fills —
/// usually the rightmost slab of the window, top to bottom. Content rows are
/// positioned by cell coords (`content_col`, `top_row`, `content_cols`,
/// `rows`) so text aligns to the monospace grid.
///
/// `hits` is cleared and refilled each call with the clickable regions
/// (REPO/branch/HEAD-sha rows). Callers route mouse-down events through it.
///
/// Layout (top-to-bottom, no card chrome — just the glass fill + a 1px
/// left hairline so it reads as a docked panel, not a popup):
///   REPO     — repo basename + parent path
///   GIT      — branch · dirty · ahead/behind  (or "no repo" when none)
///   LAST RUN — outcome + duration
///   AGENTS   — connection bullet + summary + up to 3 priority rows
///   SYSTEM   — load 1m + local HH:MM
#[allow(clippy::too_many_arguments)]
pub fn draw_right_hud(
    raster: &mut Raster,
    painter: &mut dyn GlyphPainter,
    metrics: FontMetrics,
    app_theme: &Theme,
    snap: &Snapshot,
    local: &LocalContext,
    surface_rect: PixelRect,
    content_col: usize,
    content_cols: usize,
    top_row: usize,
    rows: usize,
    hits: &mut Vec<HudHit>,
    order: &[SectionId],
    section_hits: &mut Vec<SectionHeaderHit>,
) {
    hits.clear();
    section_hits.clear();
    if rows == 0 || content_cols < 12 {
        return;
    }

    let tones = glass_tones_for(app_theme);

    // Frosted glass: composite the surface tone over whatever's behind with
    // `surface_alpha` < 1, so the canvas tints through. A 1px hairline on
    // the left edge separates the panel from the terminal grid.
    raster.fill_pixel_rect_alpha(
        surface_rect.x,
        surface_rect.y,
        surface_rect.w,
        surface_rect.h,
        tones.surface,
        tones.surface_alpha,
    );
    raster.fill_pixel_rect(
        surface_rect.x,
        surface_rect.y,
        1.0,
        surface_rect.h,
        tones.edge,
    );

    // Bind cell-grid coords for the rest of the function.
    let start_col = content_col;
    let hud_cols = content_cols;
    let theme = TonedTheme {
        foreground: tones.foreground,
    };
    let label_color = tones.label;
    let meta_color = tones.meta;

    let inner_col = start_col + 2; // 2-col left pad
    let max_col = start_col + hud_cols - 1; // 1-col right pad
    let mut r = top_row + 1; // 1-row top breathing room
    let bottom = top_row + rows;

    // Section dispatch — iterate the caller-supplied order, falling back to
    // the default order for any sections not listed. This is the entry
    // point for drag-to-reorder: the App persists the order to disk and
    // hands it back here every frame.
    let mut visited = [false; 7];
    let resolved_order: Vec<SectionId> = order
        .iter()
        .copied()
        .chain(
            SectionId::DEFAULT_ORDER
                .iter()
                .copied()
                .filter(|s| !order.contains(s)),
        )
        .collect();
    for &sid in &resolved_order {
        let idx = SectionId::DEFAULT_ORDER
            .iter()
            .position(|&s| s == sid)
            .unwrap_or(0);
        if visited[idx] {
            continue;
        }
        visited[idx] = true;
        if r >= bottom {
            break;
        }
        match sid {
            SectionId::Context => {
                // --- CONTEXT ------------------------------------------
                // Hidden when no kubectl context is available.
                let Some(ref kube) = local.kube_context else {
                    continue;
                };
                let header_row = r;
                draw_section_header(
                    raster,
                    painter,
                    metrics,
                    inner_col,
                    r,
                    "CONTEXT",
                    label_color,
                    max_col,
                );
                push_section_header_hit(
                    section_hits,
                    raster,
                    metrics,
                    sid,
                    inner_col,
                    header_row,
                    hud_cols,
                );
                r += 1;

                // Row: env-tint dot · cluster · · · namespace
                if r < bottom {
                    let dot_color = match kube.env_kind {
                        anvil_prompt_core::EnvKind::Prod => ATTENTION,
                        anvil_prompt_core::EnvKind::Staging => INFO_TEAL,
                        anvil_prompt_core::EnvKind::Dev => ALLOY,
                    };
                    let mut c = inner_col;
                    // U+25CF BLACK CIRCLE dot
                    raster.cell_glyph(painter, metrics, c, r, 0x25CF, dot_color);
                    c += 2; // dot + space
                    for ch in kube.cluster.chars() {
                        if c >= max_col {
                            break;
                        }
                        raster.cell_glyph(painter, metrics, c, r, ch as u32, theme.foreground);
                        c += 1;
                    }
                    // · separator
                    if c + 2 < max_col {
                        raster.cell_glyph(painter, metrics, c, r, ' ' as u32, meta_color);
                        c += 1;
                        raster.cell_glyph(painter, metrics, c, r, 0x00b7, meta_color);
                        c += 1;
                        raster.cell_glyph(painter, metrics, c, r, ' ' as u32, meta_color);
                        c += 1;
                    }
                    for ch in kube.namespace.chars() {
                        if c >= max_col {
                            break;
                        }
                        raster.cell_glyph(painter, metrics, c, r, ch as u32, meta_color);
                        c += 1;
                    }
                    let ctx_copy = format!("{}/{}", kube.cluster, kube.namespace);
                    push_row_hit(
                        hits,
                        raster,
                        metrics,
                        inner_col,
                        r,
                        hud_cols - 3,
                        &ctx_copy,
                        "",
                    );
                    r += 1;
                }
            }
            SectionId::RepoGit => {
                // --- REPO + GIT (merged) ---------------------------------
                let header_row = r;
                draw_section_header(
                    raster,
                    painter,
                    metrics,
                    inner_col,
                    r,
                    "REPO + GIT",
                    label_color,
                    max_col,
                );
                push_section_header_hit(
                    section_hits,
                    raster,
                    metrics,
                    sid,
                    inner_col,
                    header_row,
                    hud_cols,
                );
                r += 1;

                // Row 1: repo basename in foreground. Click → copy full cwd.
                if r < bottom {
                    let name = repo_display_name(local);
                    draw_text(
                        raster,
                        painter,
                        metrics,
                        inner_col,
                        r,
                        &name,
                        theme.foreground,
                        max_col,
                    );
                    push_row_hit(
                        hits,
                        raster,
                        metrics,
                        inner_col,
                        r,
                        hud_cols - 3,
                        &local.cwd,
                        &local.cwd,
                    );
                    r += 1;
                }
                // Row 2: parent path, dim.
                if r < bottom {
                    if let Some(parent) = parent_path_compact(&local.cwd, hud_cols - 4) {
                        draw_text(
                            raster, painter, metrics, inner_col, r, &parent, meta_color, max_col,
                        );
                        push_row_hit(
                            hits,
                            raster,
                            metrics,
                            inner_col,
                            r,
                            hud_cols - 3,
                            &local.cwd,
                            &local.cwd,
                        );
                        r += 1;
                    }
                }

                if local.git == GitState::NoRepo || local.branch.is_empty() {
                    if r < bottom {
                        draw_text(
                            raster, painter, metrics, inner_col, r, "no repo", meta_color, max_col,
                        );
                        r += 1;
                    }
                } else {
                    // Row 3: branch line — U+23B7 ⎇ glyph + branch + dirty/ahead-behind
                    if r < bottom {
                        // U+23B7 SYMBOL FOR COMBINING LONG VERTICAL LINE BELOW
                        // Use U+238B (⎋) or simply the Nerd Font branch glyph 
                        raster.cell_glyph(painter, metrics, inner_col, r, 0xe0a0, INFO_TEAL);
                        draw_text(
                            raster,
                            painter,
                            metrics,
                            inner_col + 2,
                            r,
                            &local.branch,
                            theme.foreground,
                            max_col,
                        );
                        // Dirty / ahead / behind indicator after branch name
                        let branch_len = local.branch.chars().count();
                        let after_branch = inner_col + 2 + branch_len;
                        if after_branch + 1 < max_col {
                            let mut bits: Vec<(String, [u8; 3])> = Vec::new();
                            if local.git_dirty > 0 {
                                bits.push((
                                    format!(" *{} modified", local.git_dirty),
                                    ATTENTION,
                                ));
                            }
                            let ab = format_ahead_behind(local.git_ahead, local.git_behind);
                            if !ab.is_empty() {
                                bits.push((format!(" {ab}"), INFO_TEAL));
                            }
                            let mut c = after_branch;
                            for (txt, col) in &bits {
                                for ch in txt.chars() {
                                    if c >= max_col {
                                        break;
                                    }
                                    raster.cell_glyph(painter, metrics, c, r, ch as u32, *col);
                                    c += 1;
                                }
                            }
                        }
                        push_row_hit(
                            hits,
                            raster,
                            metrics,
                            inner_col,
                            r,
                            hud_cols - 3,
                            &local.branch,
                            "",
                        );
                        r += 1;
                    }

                    // Row 4: HEAD short SHA + commit subject.
                    if r < bottom && !local.head_short.is_empty() {
                        let mut c = inner_col;
                        for ch in local.head_short.chars() {
                            if c >= max_col {
                                break;
                            }
                            raster.cell_glyph(painter, metrics, c, r, ch as u32, INFO_TEAL);
                            c += 1;
                        }
                        if !local.head_subject.is_empty() && c + 1 < max_col {
                            raster.cell_glyph(painter, metrics, c, r, ' ' as u32, meta_color);
                            c += 1;
                            for ch in local.head_subject.chars() {
                                if c >= max_col {
                                    break;
                                }
                                raster
                                    .cell_glyph(painter, metrics, c, r, ch as u32, meta_color);
                                c += 1;
                            }
                        }
                        push_row_hit(
                            hits,
                            raster,
                            metrics,
                            inner_col,
                            r,
                            hud_cols - 3,
                            &local.head_short,
                            "",
                        );
                        r += 1;
                    }
                }
            }
            SectionId::Ci => {
                // --- CI -----------------------------------------------
                // Hidden when ci_status is None (poller not wired yet).
                let Some(ref ci) = local.ci_status else {
                    continue;
                };
                let header_row = r;
                draw_section_header(
                    raster,
                    painter,
                    metrics,
                    inner_col,
                    r,
                    "CI",
                    label_color,
                    max_col,
                );
                push_section_header_hit(
                    section_hits,
                    raster,
                    metrics,
                    sid,
                    inner_col,
                    header_row,
                    hud_cols,
                );
                r += 1;

                // Row 1: status glyph + label + duration.
                if r < bottom {
                    use anvil_prompt_core::CiState;
                    let (glyph, gcol, label) = match ci.state {
                        CiState::Ok => (
                            "\u{2713}",
                            VERIFIED,
                            format!("{} \u{00b7} {}s", ci.branch, ci.duration_s),
                        ),
                        CiState::Failed => (
                            "\u{2717}",
                            FAILURE,
                            format!("{} \u{00b7} {}s", ci.branch, ci.duration_s),
                        ),
                        CiState::Running => (
                            "\u{25CF}",
                            INFO_TEAL,
                            format!("{} running\u{2026}", ci.branch),
                        ),
                        CiState::Unknown => ("\u{00b7}", ALLOY, "no data".to_string()),
                    };
                    raster.cell_glyph(
                        painter,
                        metrics,
                        inner_col,
                        r,
                        glyph.chars().next().unwrap() as u32,
                        gcol,
                    );
                    draw_text(
                        raster,
                        painter,
                        metrics,
                        inner_col + 2,
                        r,
                        &label,
                        theme.foreground,
                        max_col,
                    );
                    r += 1;
                }

                // Row 2: open PRs count (when > 0).
                if r < bottom && ci.open_prs > 0 {
                    let pr_label = format!("{} open PR{}", ci.open_prs, if ci.open_prs == 1 { "" } else { "s" });
                    draw_text(
                        raster,
                        painter,
                        metrics,
                        inner_col,
                        r,
                        &pr_label,
                        INFO_TEAL,
                        max_col,
                    );
                    if !ci.pr_url.is_empty() {
                        push_row_hit(
                            hits,
                            raster,
                            metrics,
                            inner_col,
                            r,
                            hud_cols - 3,
                            &ci.pr_url,
                            &ci.pr_url,
                        );
                    }
                    r += 1;
                }
            }
            SectionId::Ports => {
                // --- PORTS ---------------------------------------------
                if local.ports.is_empty() {
                    continue;
                }
                let header_row = r;
                if r < bottom {
                    draw_section_header(
                        raster,
                        painter,
                        metrics,
                        inner_col,
                        r,
                        "PORTS",
                        label_color,
                        max_col,
                    );
                    push_section_header_hit(
                        section_hits,
                        raster,
                        metrics,
                        sid,
                        inner_col,
                        header_row,
                        hud_cols,
                    );
                    r += 1;
                }
                if r < bottom {
                    let mut c = inner_col;
                    for (i, &port) in local.ports.iter().enumerate() {
                        let s = format!(":{port}");
                        if i > 0 {
                            // space separator
                            if c + 1 >= max_col {
                                break;
                            }
                            raster.cell_glyph(painter, metrics, c, r, ' ' as u32, meta_color);
                            c += 1;
                        }
                        let label_start = c;
                        for ch in s.chars() {
                            if c >= max_col {
                                break;
                            }
                            raster.cell_glyph(painter, metrics, c, r, ch as u32, INFO_TEAL);
                            c += 1;
                        }
                        // Per-port click region: plain → copy URL, Cmd → open URL.
                        let url = format!("http://localhost:{port}");
                        push_row_hit(
                            hits,
                            raster,
                            metrics,
                            label_start,
                            r,
                            c - label_start,
                            &url,
                            &url,
                        );
                    }
                    r += 1;
                }
            }
            SectionId::Recent => {
                // --- RECENT --------------------------------------------
                if local.recent_files.is_empty() {
                    continue;
                }
                let header_row = r;
                if r < bottom {
                    draw_section_header(
                        raster,
                        painter,
                        metrics,
                        inner_col,
                        r,
                        "RECENT",
                        label_color,
                        max_col,
                    );
                    push_section_header_hit(
                        section_hits,
                        raster,
                        metrics,
                        sid,
                        inner_col,
                        header_row,
                        hud_cols,
                    );
                    r += 1;
                }
                for (fi, full_path) in local.recent_files.iter().enumerate() {
                    if r >= bottom {
                        break;
                    }
                    let basename: &str = full_path
                        .rsplit('/')
                        .next()
                        .filter(|s| !s.is_empty())
                        .unwrap_or(full_path.as_str());
                    draw_text(
                        raster, painter, metrics, inner_col, r, basename, meta_color, max_col,
                    );
                    // HudHit: Cmd-click opens the full path in the default editor.
                    let hit_copy = full_path.clone();
                    let hit_open = full_path.clone();
                    push_row_hit(
                        hits,
                        raster,
                        metrics,
                        inner_col,
                        r,
                        hud_cols - 3,
                        &hit_copy,
                        &hit_open,
                    );
                    let _ = fi; // fi available for future use
                    r += 1;
                }
            }
            SectionId::Agents => {
                // --- AGENTS --------------------------------------------
                let header_row = r;
                draw_section_header(
                    raster,
                    painter,
                    metrics,
                    inner_col,
                    r,
                    "AGENTS",
                    label_color,
                    max_col,
                );
                push_section_header_hit(
                    section_hits,
                    raster,
                    metrics,
                    sid,
                    inner_col,
                    header_row,
                    hud_cols,
                );
                r += 1;
                if r < bottom {
                    let bullet_color = header_bullet_color(snap);
                    raster.cell_glyph(painter, metrics, inner_col, r, 0x25CF, bullet_color);
                    let summary = build_header_summary(snap);
                    let label = if summary.is_empty() {
                        "no signal".to_string()
                    } else {
                        summary
                    };
                    draw_text(
                        raster,
                        painter,
                        metrics,
                        inner_col + 2,
                        r,
                        &label,
                        theme.foreground,
                        max_col,
                    );
                    r += 1;
                }

                // All priority rows (capped only by remaining HUD vertical space —
                // the AGENTS section is the de-facto agent dock when there are many
                // approvals / runs / failures to show).
                for ap in &snap.approvals {
                    if r >= bottom {
                        break;
                    }
                    draw_hud_row(
                        raster,
                        painter,
                        metrics,
                        inner_col,
                        r,
                        "\u{25b8}",
                        ATTENTION,
                        &ap.connector,
                        max_col,
                        theme.foreground,
                    );
                    r += 1;
                }
                for run in &snap.runs {
                    if r >= bottom {
                        break;
                    }
                    if run.status != RunStatus::Running {
                        continue;
                    }
                    draw_hud_row(
                        raster,
                        painter,
                        metrics,
                        inner_col,
                        r,
                        "\u{25c6}",
                        AGENT_VIOLET,
                        &run.agent,
                        max_col,
                        theme.foreground,
                    );
                    r += 1;
                }
                for f in &snap.findings {
                    if r >= bottom {
                        break;
                    }
                    if f.severity != FindingSeverity::Failure {
                        continue;
                    }
                    draw_hud_row(
                        raster,
                        painter,
                        metrics,
                        inner_col,
                        r,
                        "\u{2717}",
                        FAILURE,
                        &f.summary,
                        max_col,
                        theme.foreground,
                    );
                    r += 1;
                }
            }
            SectionId::System => {
                // --- SYSTEM (compact) ----------------------------------
                // Single row: "mem ▄▅▆▆▃▁ N/N GB · load X.XX"
                let header_row = r;
                draw_section_header(
                    raster,
                    painter,
                    metrics,
                    inner_col,
                    r,
                    "SYSTEM",
                    label_color,
                    max_col,
                );
                push_section_header_hit(
                    section_hits,
                    raster,
                    metrics,
                    sid,
                    inner_col,
                    header_row,
                    hud_cols,
                );
                r += 1;

                // Single compact row: "mem ▄▅▆▆▃▁ N/N GB · load X.XX"
                if r < bottom {
                    let mem_ratio = mem_usage_ratio().unwrap_or(0.0);
                    let mem_bar = gauge_bar(mem_ratio, 6);
                    let total_gb = total_mem_gb();
                    let used_gb = mem_ratio * total_gb;

                    let load_val = format_load_1m();
                    let load_str = load_val.as_deref().unwrap_or("—");

                    let line = if total_gb > 0.0 {
                        format!(
                            "mem {mem_bar} {:.0}/{:.0} GB \u{00b7} load {load_str}",
                            used_gb, total_gb
                        )
                    } else {
                        format!("mem {mem_bar} \u{00b7} load {load_str}")
                    };
                    draw_text(
                        raster,
                        painter,
                        metrics,
                        inner_col,
                        r,
                        &line,
                        theme.foreground,
                        max_col,
                    );
                    r += 1;
                }
            }
        }
        // Inter-section break — a faint hairline + blank row. Skip after
        // the last visited section so we don't trail a divider into the
        // empty bottom of the HUD.
        r = section_break(raster, metrics, start_col, hud_cols, r, bottom, tones.edge);
    }
}

/// Record a section-header hit rect so the App can route a drag-to-reorder
/// gesture that starts on the header.
fn push_section_header_hit(
    out: &mut Vec<SectionHeaderHit>,
    raster: &Raster,
    metrics: FontMetrics,
    section: SectionId,
    col: usize,
    row: usize,
    hud_cols: usize,
) {
    let cw = metrics.cell_w;
    let ch = metrics.cell_h;
    let rect = PixelRect {
        x: raster.pad_x + col as f64 * cw,
        y: raster.pad_y + row as f64 * ch,
        w: (hud_cols.saturating_sub(2)) as f64 * cw,
        h: ch,
    };
    out.push(SectionHeaderHit { section, rect });
}

/// Section break: draws a faint hairline at `r` (the gap row) and advances
/// to the next row. Use between adjacent sections to give them stronger
/// visual separation than a blank row alone.
#[allow(clippy::too_many_arguments)]
fn section_break(
    raster: &mut Raster,
    metrics: FontMetrics,
    start_col: usize,
    hud_cols: usize,
    r: usize,
    bottom: usize,
    rule_color: [u8; 3],
) -> usize {
    if r < bottom {
        draw_section_rule(raster, metrics, start_col, r, hud_cols, rule_color);
    }
    (r + 1).min(bottom)
}

/// Push a clickable HUD row spanning `width_cells` cells starting at `(col,
/// row)`. The rect is computed in device-pixel space using the raster's
/// padding origin so it lines up with the cell glyphs above it.
#[allow(clippy::too_many_arguments)]
fn push_row_hit(
    hits: &mut Vec<HudHit>,
    raster: &Raster,
    metrics: FontMetrics,
    col: usize,
    row: usize,
    width_cells: usize,
    copy: &str,
    open: &str,
) {
    if copy.is_empty() && open.is_empty() {
        return;
    }
    let cw = metrics.cell_w;
    let ch = metrics.cell_h;
    let rect = PixelRect {
        x: raster.pad_x + col as f64 * cw,
        y: raster.pad_y + row as f64 * ch,
        w: width_cells as f64 * cw,
        h: ch,
    };
    hits.push(HudHit {
        rect,
        copy: copy.to_string(),
        open: open.to_string(),
    });
}

/// "REPO", "GIT" etc. drawn in the supplied label color (theme-dependent,
/// quieter than body text so headers recede on the glass surface). The
/// label is rendered LETTERSPACED — one blank cell inserted between each
/// glyph — to give the headers a "small-caps display" cadence against the
/// dense body text below them.
#[allow(clippy::too_many_arguments)]
fn draw_section_header(
    raster: &mut Raster,
    painter: &mut dyn GlyphPainter,
    metrics: FontMetrics,
    col: usize,
    row: usize,
    label: &str,
    color: [u8; 3],
    max_col: usize,
) {
    // Format: `─ LABEL ────────`. Industrial register-label feel; replaces
    // the older letterspaced `L A B E L` form which read as banner-like at
    // HUD widths and visually dominated short-label sections.
    let mut c = col;
    if c >= max_col {
        return;
    }
    raster.cell_glyph(painter, metrics, c, row, '─' as u32, color);
    c += 1;
    if c >= max_col {
        return;
    }
    c += 1; // gap before label
    for ch in label.chars() {
        if c >= max_col {
            return;
        }
        raster.cell_glyph(painter, metrics, c, row, ch as u32, color);
        c += 1;
    }
    if c >= max_col {
        return;
    }
    c += 1; // gap after label
    while c < max_col {
        raster.cell_glyph(painter, metrics, c, row, '─' as u32, color);
        c += 1;
    }
}

/// A faint horizontal hairline used as a section separator inside the HUD.
/// Drawn between sections in lieu of (or in addition to) a blank row.
fn draw_section_rule(
    raster: &mut Raster,
    metrics: FontMetrics,
    start_col: usize,
    row: usize,
    cols: usize,
    color: [u8; 3],
) {
    let cw = metrics.cell_w;
    let ch = metrics.cell_h;
    // The rule sits on the row's vertical midline so it reads as inset, not
    // as part of either the row above or below.
    let y = raster.pad_y + (row as f64 + 0.5) * ch;
    let x = raster.pad_x + (start_col + 1) as f64 * cw;
    let w = (cols as f64 - 2.0) * cw;
    raster.fill_pixel_rect(x, y, w, 1.0, color);
}

#[allow(clippy::too_many_arguments)]
fn draw_hud_row(
    raster: &mut Raster,
    painter: &mut dyn GlyphPainter,
    metrics: FontMetrics,
    col: usize,
    row: usize,
    glyph_str: &str,
    glyph_color: [u8; 3],
    label: &str,
    max_col: usize,
    text_color: [u8; 3],
) {
    if let Some(g) = glyph_str.chars().next() {
        raster.cell_glyph(painter, metrics, col, row, g as u32, glyph_color);
    }
    draw_text(
        raster,
        painter,
        metrics,
        col + 2,
        row,
        label,
        text_color,
        max_col,
    );
}

/// Display name for the REPO section — the basename of cwd. Future: actual
/// `git rev-parse --show-toplevel`. Falls back to "—".
fn repo_display_name(local: &LocalContext) -> String {
    if local.cwd.is_empty() {
        return "—".to_string();
    }
    let trimmed = local.cwd.trim_end_matches('/');
    let base = trimmed.rsplit('/').next().unwrap_or(trimmed);
    if base.is_empty() {
        "—".to_string()
    } else {
        base.to_string()
    }
}

/// Compact parent-directory display. Truncates with a leading "…/" when too
/// long. Returns None when there's no parent (root or empty).
fn parent_path_compact(cwd: &str, max_chars: usize) -> Option<String> {
    if cwd.is_empty() {
        return None;
    }
    let trimmed = cwd.trim_end_matches('/');
    let last = trimmed.rfind('/')?;
    if last == 0 {
        return None;
    }
    let parent = &trimmed[..last];
    // Replace $HOME with ~ if possible.
    let home_replaced = match std::env::var("HOME") {
        Ok(h) if !h.is_empty() && parent.starts_with(&h) => format!("~{}", &parent[h.len()..]),
        _ => parent.to_string(),
    };
    if home_replaced.chars().count() <= max_chars {
        return Some(home_replaced);
    }
    // Truncate from the left.
    let tail: String = home_replaced
        .chars()
        .rev()
        .take(max_chars.saturating_sub(2))
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    Some(format!("…/{tail}"))
}

// --- Row draw helpers -------------------------------------------------------

/// Draw the header row: bullet U+25CF + "agents" dim label + summary text.
#[allow(clippy::too_many_arguments)]
fn draw_agent_header(
    raster: &mut Raster,
    painter: &mut dyn GlyphPainter,
    metrics: FontMetrics,
    theme: &Theme,
    start_col: usize,
    row: usize,
    bullet_color: [u8; 3],
    summary: &str,
    cols: usize,
) {
    let max_col = start_col + cols - 1;
    // Col+1: bullet U+25CF
    raster.cell_glyph(painter, metrics, start_col + 1, row, 0x25CF, bullet_color);
    // Col+3: "agents" in alloy
    draw_text(
        raster,
        painter,
        metrics,
        start_col + 3,
        row,
        "agents",
        ALLOY,
        max_col,
    );
    // Col+10: summary (3 spaces gap after "agents" which is 6 chars = col+3+6 = col+9, then gap)
    draw_text(
        raster,
        painter,
        metrics,
        start_col + 10,
        row,
        summary,
        theme.foreground,
        max_col,
    );
}

/// Draw a priority row: a glyph + a label.
#[allow(clippy::too_many_arguments)]
fn draw_priority_row(
    raster: &mut Raster,
    painter: &mut dyn GlyphPainter,
    metrics: FontMetrics,
    start_col: usize,
    row: usize,
    glyph_str: &str,
    glyph_color: [u8; 3],
    label: &str,
    cols: usize,
) {
    let max_col = start_col + cols - 1;
    // Col+2: status glyph (1 codepoint)
    draw_text(
        raster,
        painter,
        metrics,
        start_col + 2,
        row,
        glyph_str,
        glyph_color,
        start_col + 4,
    );
    // Col+4: label in alloy
    draw_text(
        raster,
        painter,
        metrics,
        start_col + 4,
        row,
        label,
        ALLOY,
        max_col,
    );
}

/// Draw a horizontal hairline separator at the center of `row`.
fn draw_hairline(
    raster: &mut Raster,
    metrics: FontMetrics,
    theme: &Theme,
    start_col: usize,
    row: usize,
    cols: usize,
) {
    let ch = metrics.cell_h;
    let cw = metrics.cell_w;
    let sep_y = raster.pad_y + (row as f64 + 0.5) * ch;
    let sep_x = raster.pad_x + (start_col + 1) as f64 * cw;
    let sep_w = (cols as f64 - 2.0) * cw;
    raster.fill_pixel_rect(sep_x, sep_y, sep_w, 1.0, theme.border);
}

/// Compact local-context footer packed with useful state.
/// Format: `cwd · branch *N ↑A ↓B · run · HH:MM`
fn draw_local_footer(
    raster: &mut Raster,
    painter: &mut dyn GlyphPainter,
    metrics: FontMetrics,
    start_col: usize,
    row: usize,
    local: &LocalContext,
    cols: usize,
) {
    let max_col = start_col + cols - 1;
    let mut buf = String::with_capacity(96);

    // cwd: keep only the basename to save columns.
    let cwd_short = format_cwd(&local.cwd);
    let tail = match cwd_short.rfind('/') {
        Some(sep) => &cwd_short[sep + 1..],
        None => &cwd_short,
    };
    let _ = write!(buf, "{tail}");

    // git: branch + dirty count + ahead/behind, condensed.
    if local.git != GitState::NoRepo && !local.branch.is_empty() {
        let _ = write!(buf, " \u{00b7} {}", local.branch);
        if local.git_dirty > 0 {
            let _ = write!(buf, " *{}", local.git_dirty);
        }
        let ab = format_ahead_behind(local.git_ahead, local.git_behind);
        if !ab.is_empty() {
            let _ = write!(buf, " {ab}");
        }
    }

    // last run.
    {
        let rtxt = format_run_status(local.run, local.run_exit, local.run_duration_ms);
        let _ = write!(buf, " \u{00b7} {rtxt}");
    }

    // load average (1m) — system pulse at a glance.
    if let Some(la) = format_load_1m() {
        let _ = write!(buf, " \u{00b7} {la}");
    }

    // local clock — HH:MM. Trailing position so it stays out of the way.
    if let Some(hm) = format_local_hm() {
        let _ = write!(buf, " \u{00b7} {hm}");
    }

    draw_text(
        raster,
        painter,
        metrics,
        start_col + 2,
        row,
        &buf,
        ALLOY,
        max_col,
    );
}

/// Local time as `HH:MM` (24h). Returns None when system time is unavailable.
fn format_local_hm() -> Option<String> {
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .ok()?;
    let secs = now.as_secs();
    // Pure conversion: seconds → wall HH:MM in local TZ. We use a fixed
    // offset from the TZ env var to avoid depending on a date crate.
    // Best-effort: if $TZ_OFFSET_SEC is set use that; otherwise treat the
    // system clock as already-local (correct in practice for macOS where
    // SystemTime returns local wall-clock seconds via libc).
    let offset: i64 = std::env::var("TZ_OFFSET_SEC")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(local_offset_seconds);
    let local = secs as i64 + offset;
    let day_secs = local.rem_euclid(86_400);
    let h = day_secs / 3_600;
    let m = (day_secs % 3_600) / 60;
    Some(format!("{:02}:{:02}", h, m))
}

/// 1-minute load average as a compact string e.g. `1.42`. None on failure.
fn format_load_1m() -> Option<String> {
    let mut samples = [0.0_f64; 3];
    // SAFETY: getloadavg writes up to nelem floats into the array.
    let n = unsafe { libc::getloadavg(samples.as_mut_ptr(), 3) };
    if n < 1 {
        return None;
    }
    Some(format!("{:.2}", samples[0]))
}

/// Compute local UTC offset in seconds via libc's `localtime_r`.
fn local_offset_seconds() -> i64 {
    // SAFETY: we read time and pass into thread-safe localtime_r with a
    // local-stack tm struct. No mutation of process state.
    unsafe {
        let mut now: libc::time_t = 0;
        libc::time(&mut now as *mut libc::time_t);
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&now as *const libc::time_t, &mut tm as *mut libc::tm);
        tm.tm_gmtoff as i64
    }
}

// --- Gauge rendering -------------------------------------------------------

/// Block characters U+2581–U+2588 (▁▂▃▄▅▆▇█), indexed 0–7.
const GAUGE_BLOCKS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

/// Render `cells` block-bar glyphs shaded by `ratio` (0.0–1.0).
/// Each cell's fill level is proportional to how far along the bar it sits.
/// Returns a string of exactly `cells` block characters.
pub fn gauge_bar(ratio: f64, cells: usize) -> String {
    if cells == 0 {
        return String::new();
    }
    let ratio = ratio.clamp(0.0, 1.0);
    let mut out = String::with_capacity(cells * 3); // each glyph up to 3 UTF-8 bytes
    for i in 0..cells {
        // Fraction of the total bar that this cell represents.
        let cell_ratio = (i as f64 + 1.0) / cells as f64;
        // How filled this cell is relative to ratio: 0=empty, 1=full.
        let fill = (ratio / cell_ratio).clamp(0.0, 1.0);
        let idx = (fill * 7.0).round() as usize;
        out.push(GAUGE_BLOCKS[idx.min(7)]);
    }
    out
}

// --- Cached system stats ---------------------------------------------------

/// ~1-second cache for the memory usage ratio (0.0–1.0).
static MEM_CACHE: Mutex<Option<(Instant, f64)>> = Mutex::new(None);

const CACHE_TTL: Duration = Duration::from_secs(1);

/// Memory pressure ratio via `host_statistics64`. Returns None on failure.
///
/// Metric: (active + wire + compressor) / total pages.
fn mem_usage_ratio() -> Option<f64> {
    if let Ok(guard) = MEM_CACHE.lock() {
        if let Some((ts, val)) = *guard {
            if ts.elapsed() < CACHE_TTL {
                return Some(val);
            }
        }
    }
    let ratio = mem_usage_ratio_uncached()?;
    if let Ok(mut guard) = MEM_CACHE.lock() {
        *guard = Some((Instant::now(), ratio));
    }
    Some(ratio)
}

fn mem_usage_ratio_uncached() -> Option<f64> {
    // SAFETY: calls macOS host_statistics64 with the correct flavor and a
    // properly-sized output buffer; reads only, no process-state mutation.
    // mach_host_self is deprecated in libc in favour of the mach2 crate; we
    // suppress the lint here rather than adding a new dependency.
    #[allow(deprecated)]
    unsafe {
        let host = libc::mach_host_self();
        let mut stats: libc::vm_statistics64 = std::mem::zeroed();
        let mut count = libc::HOST_VM_INFO64_COUNT;
        let ret = libc::host_statistics64(
            host,
            libc::HOST_VM_INFO64,
            &mut stats as *mut libc::vm_statistics64 as libc::host_info64_t,
            &mut count,
        );
        if ret != libc::KERN_SUCCESS {
            return None;
        }
        let total = stats.free_count as f64
            + stats.active_count as f64
            + stats.inactive_count as f64
            + stats.wire_count as f64
            + stats.compressor_page_count as f64;
        if total == 0.0 {
            return None;
        }
        let used = stats.active_count as f64
            + stats.wire_count as f64
            + stats.compressor_page_count as f64;
        Some((used / total).clamp(0.0, 1.0))
    }
}

/// Total physical memory in GB via `sysctlbyname("hw.memsize")`. Returns 0.0 on failure.
fn total_mem_gb() -> f64 {
    // SAFETY: sysctlbyname reads a kernel variable; no mutation of process state.
    unsafe {
        let name = b"hw.memsize\0";
        let mut size: u64 = 0;
        let mut len: libc::size_t = std::mem::size_of::<u64>();
        let ret = libc::sysctlbyname(
            name.as_ptr() as *const libc::c_char,
            &mut size as *mut u64 as *mut libc::c_void,
            &mut len,
            std::ptr::null_mut(),
            0,
        );
        if ret != 0 {
            0.0
        } else {
            size as f64 / 1_073_741_824.0
        }
    }
}

// --- Shared draw utilities --------------------------------------------------

/// Draw a UTF-8 string from cell `col`, one codepoint per cell, stopping at `max_col`.
#[allow(clippy::too_many_arguments)]
fn draw_text(
    raster: &mut Raster,
    painter: &mut dyn GlyphPainter,
    metrics: FontMetrics,
    col: usize,
    row: usize,
    text: &str,
    color: [u8; 3],
    max_col: usize,
) {
    for (i, cp) in text.chars().enumerate() {
        let cx = col + i;
        if cx >= max_col {
            break;
        }
        raster.cell_glyph(painter, metrics, cx, row, cp as u32, color);
    }
}

// --- Tests ------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raster::PixelRect;

    // Stub painter that records calls.
    #[derive(Default)]
    struct StubPainter {
        pub calls: Vec<(u32, [u8; 3])>,
    }

    impl GlyphPainter for StubPainter {
        #[allow(clippy::too_many_arguments)]
        fn draw_glyph(
            &mut self,
            glyph_id: u32,
            _dest: PixelRect,
            fg: [u8; 3],
            _metrics: FontMetrics,
            _pixels: &mut [u8],
            _bw: usize,
            _bh: usize,
        ) {
            self.calls.push((glyph_id, fg));
        }
    }

    fn metrics() -> FontMetrics {
        FontMetrics {
            cell_w: 10.0,
            cell_h: 20.0,
            descent: 4.0,
        }
    }

    // --- formatDuration ---

    /// formatDuration sub-second
    #[test]
    fn format_duration_sub_second() {
        assert_eq!(format_duration(350), "0.3s");
    }

    /// formatDuration seconds with tenths
    #[test]
    fn format_duration_seconds_with_tenths() {
        assert_eq!(format_duration(1250), "1.2s");
    }

    /// formatDuration large value
    #[test]
    fn format_duration_large_value() {
        assert_eq!(format_duration(72000), "72s");
    }

    /// formatDuration negative clamps to zero
    #[test]
    fn format_duration_negative_clamps() {
        assert_eq!(format_duration(-100), "0s");
    }

    // --- formatRunStatus ---

    /// formatRunStatus ok
    #[test]
    fn format_run_status_ok() {
        let s = format_run_status(RunState::Ok, 0, 1200);
        assert!(
            s.starts_with("ok"),
            "expected to start with 'ok', got '{s}'"
        );
        assert!(s.contains("1.2s"), "expected to contain '1.2s', got '{s}'");
    }

    /// formatRunStatus failed with exit code
    #[test]
    fn format_run_status_failed_with_exit_code() {
        let s = format_run_status(RunState::Failed, 127, 500);
        assert!(s.contains("failed"), "expected 'failed' in '{s}'");
        assert!(s.contains("127"), "expected '127' in '{s}'");
    }

    /// formatRunStatus idle
    #[test]
    fn format_run_status_idle() {
        let s = format_run_status(RunState::Idle, 0, 0);
        assert_eq!(s, "idle");
    }

    // --- formatAheadBehind ---

    /// formatAheadBehind ahead only
    #[test]
    fn format_ahead_behind_ahead_only() {
        let s = format_ahead_behind(2, 0);
        assert!(s.contains('2'), "expected '2' in '{s}'");
    }

    /// formatAheadBehind both
    #[test]
    fn format_ahead_behind_both() {
        let s = format_ahead_behind(3, 1);
        assert!(s.contains('3'), "expected '3' in '{s}'");
        assert!(s.contains('1'), "expected '1' in '{s}'");
    }

    /// formatAheadBehind neither returns empty
    #[test]
    fn format_ahead_behind_neither_returns_empty() {
        assert_eq!(format_ahead_behind(0, 0), "");
    }

    // --- formatCwd ---

    /// formatCwd last two components
    #[test]
    fn format_cwd_last_two_components() {
        let s = format_cwd("/Users/foo/projects/anvil");
        assert!(
            s.contains("projects/anvil"),
            "expected 'projects/anvil' in '{s}'"
        );
    }

    /// formatCwd short path returned as-is
    #[test]
    fn format_cwd_short_path_as_is() {
        let s = format_cwd("/anvil");
        assert_eq!(s, "/anvil");
    }

    /// formatCwd empty
    #[test]
    fn format_cwd_empty() {
        assert_eq!(format_cwd(""), "");
    }

    // --- headerBulletColor ---

    /// headerBulletColor: not_installed is alloy
    #[test]
    fn header_bullet_color_not_installed_is_alloy() {
        let snap = Snapshot {
            connection: Connection::NotInstalled,
            ..Default::default()
        };
        assert_eq!(header_bullet_color(&snap), ALLOY);
    }

    /// headerBulletColor: live with no activity is verified
    #[test]
    fn header_bullet_color_live_no_activity_is_verified() {
        let snap = Snapshot {
            connection: Connection::Live,
            ..Default::default()
        };
        assert_eq!(header_bullet_color(&snap), VERIFIED);
    }

    /// headerBulletColor: live with pending approval is attention
    #[test]
    fn header_bullet_color_live_pending_approval_is_attention() {
        let snap = Snapshot {
            connection: Connection::Live,
            pending_approvals_count: 1,
            ..Default::default()
        };
        assert_eq!(header_bullet_color(&snap), ATTENTION);
    }

    /// headerBulletColor: live with running count is agent_violet
    #[test]
    fn header_bullet_color_live_running_is_agent_violet() {
        let snap = Snapshot {
            connection: Connection::Live,
            running_count: 2,
            ..Default::default()
        };
        assert_eq!(header_bullet_color(&snap), AGENT_VIOLET);
    }

    // --- buildHeaderSummary ---

    /// buildHeaderSummary: not_installed
    #[test]
    fn build_header_summary_not_installed_is_quiet() {
        let snap = Snapshot {
            connection: Connection::NotInstalled,
            ..Default::default()
        };
        // Quiet HUD: no signal states don't shout a diagnostic.
        assert_eq!(build_header_summary(&snap), "");
    }

    #[test]
    fn build_header_summary_live_empty_is_idle() {
        let snap = Snapshot {
            connection: Connection::Live,
            ..Default::default()
        };
        assert_eq!(build_header_summary(&snap), "idle");
    }

    /// buildHeaderSummary: live with running
    #[test]
    fn build_header_summary_live_with_running() {
        let snap = Snapshot {
            connection: Connection::Live,
            running_count: 3,
            ..Default::default()
        };
        let s = build_header_summary(&snap);
        assert!(s.contains('3'), "expected '3' in '{s}'");
        assert!(s.contains("running"), "expected 'running' in '{s}'");
    }

    /// Smoke test: draw does not panic.
    #[test]
    fn draw_no_panic() {
        let m = metrics();
        let mut r = Raster::new(800, 600);
        let mut painter = StubPainter::default();
        let theme = anvil_theme::MINERAL_DARK;
        let snap = Snapshot {
            connection: Connection::Live,
            running_count: 1,
            ..Default::default()
        };
        let local = LocalContext::default();
        let placement = Placement::Floating {
            total_cols: 80,
            total_rows: 30,
            top_offset: 0,
        };
        draw(
            &mut r,
            &mut painter,
            m,
            &theme,
            &snap,
            &local,
            &placement,
            false,
        );
        // Should produce glyph calls for the bullet and text.
        assert!(!painter.calls.is_empty());
    }

    fn make_snap_with_approval() -> Snapshot {
        use anvil_agent::{AgentRunRow, ApprovalRow};
        Snapshot {
            connection: Connection::Live,
            approvals: vec![ApprovalRow {
                approval_id: "a1".to_string(),
                connector: "bash".to_string(),
                pattern: "rm *".to_string(),
                reason: "risky".to_string(),
            }],
            runs: vec![AgentRunRow {
                run_id: "r1".to_string(),
                agent: "codex".to_string(),
                task: "review".to_string(),
                status: RunStatus::Running,
                created_at_unix: 0,
            }],
            findings: vec![anvil_agent::FindingRow {
                severity: FindingSeverity::Failure,
                summary: "test failure".to_string(),
                action: "fix".to_string(),
            }],
            running_count: 1,
            pending_approvals_count: 1,
            ..Default::default()
        }
    }

    /// draw: exercises priority rows (approvals, running, findings).
    #[test]
    fn draw_with_priority_rows_no_panic() {
        let m = metrics();
        let mut r = Raster::new(1200, 800);
        let mut painter = StubPainter::default();
        let theme = anvil_theme::MINERAL_DARK;
        let snap = make_snap_with_approval();
        let local = LocalContext {
            cwd: "/home/user/projects/anvil".to_string(),
            git: GitState::Ok,
            branch: "main".to_string(),
            git_dirty: 2,
            git_ahead: 1,
            git_behind: 0,
            head_short: String::new(),
            head_subject: String::new(),
            run: RunState::Ok,
            run_exit: 0,
            run_duration_ms: 1200,
            ..LocalContext::default()
        };
        let placement = Placement::Floating {
            total_cols: 100,
            total_rows: 40,
            top_offset: 0,
        };
        draw(
            &mut r,
            &mut painter,
            m,
            &theme,
            &snap,
            &local,
            &placement,
            false,
        );
        assert!(!painter.calls.is_empty());
    }

    /// draw: exercises the footer with git branch and run status.
    #[test]
    fn draw_with_local_context_branch_and_run_no_panic() {
        let m = metrics();
        let mut r = Raster::new(1200, 800);
        let mut painter = StubPainter::default();
        let theme = anvil_theme::MINERAL_DARK;
        let snap = Snapshot {
            connection: Connection::Live,
            running_count: 0,
            ..Default::default()
        };
        let local = LocalContext {
            cwd: "/usr/src/anvil".to_string(),
            git: GitState::Dirty,
            branch: "feature/something".to_string(),
            git_dirty: 3,
            git_ahead: 0,
            git_behind: 1,
            head_short: String::new(),
            head_subject: String::new(),
            run: RunState::Failed,
            run_exit: 1,
            run_duration_ms: 500,
            ..LocalContext::default()
        };
        let placement = Placement::Floating {
            total_cols: 100,
            total_rows: 40,
            top_offset: 0,
        };
        draw(
            &mut r,
            &mut painter,
            m,
            &theme,
            &snap,
            &local,
            &placement,
            false,
        );
        assert!(!painter.calls.is_empty());
    }

    /// draw: returns early when too few columns.
    #[test]
    fn draw_returns_early_when_too_few_cols() {
        let m = metrics();
        let mut r = Raster::new(200, 200);
        let mut painter = StubPainter::default();
        let theme = anvil_theme::MINERAL_DARK;
        let snap = Snapshot {
            connection: Connection::Live,
            ..Default::default()
        };
        let local = LocalContext::default();
        // PANEL_COLS is 36; total_cols=10 < PANEL_COLS+2 → returns early.
        let placement = Placement::Floating {
            total_cols: 10,
            total_rows: 30,
            top_offset: 0,
        };
        draw(
            &mut r,
            &mut painter,
            m,
            &theme,
            &snap,
            &local,
            &placement,
            false,
        );
        assert!(painter.calls.is_empty());
    }

    /// draw_local_footer: exercises the "no repo / no branch" path.
    #[test]
    fn draw_with_no_repo_local_context() {
        let m = metrics();
        let mut r = Raster::new(1200, 800);
        let mut painter = StubPainter::default();
        let theme = anvil_theme::MINERAL_DARK;
        let snap = Snapshot {
            connection: Connection::Live,
            ..Default::default()
        };
        let local = LocalContext {
            cwd: "/tmp".to_string(),
            git: GitState::NoRepo,
            branch: String::new(),
            ..LocalContext::default()
        };
        let placement = Placement::Floating {
            total_cols: 100,
            total_rows: 40,
            top_offset: 0,
        };
        draw(
            &mut r,
            &mut painter,
            m,
            &theme,
            &snap,
            &local,
            &placement,
            false,
        );
        assert!(!painter.calls.is_empty());
    }

    /// card_rows: base case (no priority items) returns 4.
    #[test]
    fn card_rows_base_is_4() {
        let snap = Snapshot::default();
        assert_eq!(card_rows(&snap), 4);
    }

    /// card_rows: one running run adds 1 priority row.
    #[test]
    fn card_rows_with_running_run_adds_one() {
        use anvil_agent::AgentRunRow;
        let snap = Snapshot {
            runs: vec![AgentRunRow {
                status: RunStatus::Running,
                ..Default::default()
            }],
            ..Default::default()
        };
        assert_eq!(card_rows(&snap), 5);
    }

    /// card_rows: capped at 3 priority rows → max 7.
    #[test]
    fn card_rows_capped_at_3_priority() {
        use anvil_agent::{AgentRunRow, ApprovalRow, FindingRow};
        let snap = Snapshot {
            approvals: vec![
                ApprovalRow::default(),
                ApprovalRow::default(),
                ApprovalRow::default(),
                ApprovalRow::default(), // 4th should be ignored
            ],
            runs: vec![AgentRunRow {
                status: RunStatus::Running,
                ..Default::default()
            }],
            findings: vec![FindingRow {
                severity: FindingSeverity::Failure,
                ..Default::default()
            }],
            ..Default::default()
        };
        // min(3 + 1 + 1, 3) = 3 priority → 7 total
        assert_eq!(card_rows(&snap), 7);
    }

    // --- glass tones ---------------------------------------------------------

    /// Dark canvas → glass surface is dark, foreground is light.
    #[test]
    fn glass_tones_dark_canvas_returns_dark_surface_and_light_ink() {
        let mut theme = anvil_theme::MINERAL_DARK;
        theme.background = [0x10, 0x12, 0x18]; // explicit dark
        let t = glass_tones_for(&theme);
        let l_surface = 0.2126 * t.surface[0] as f64
            + 0.7152 * t.surface[1] as f64
            + 0.0722 * t.surface[2] as f64;
        let l_fg = 0.2126 * t.foreground[0] as f64
            + 0.7152 * t.foreground[1] as f64
            + 0.0722 * t.foreground[2] as f64;
        assert!(l_surface < 64.0, "expected dark surface, got {l_surface}");
        assert!(l_fg > 180.0, "expected light foreground, got {l_fg}");
    }

    /// Light canvas → glass surface is light, foreground is dark.
    #[test]
    fn glass_tones_light_canvas_returns_light_surface_and_dark_ink() {
        let mut theme = anvil_theme::MINERAL_LIGHT;
        theme.background = [0xee, 0xf1, 0xf2]; // explicit light
        let t = glass_tones_for(&theme);
        let l_surface = 0.2126 * t.surface[0] as f64
            + 0.7152 * t.surface[1] as f64
            + 0.0722 * t.surface[2] as f64;
        let l_fg = 0.2126 * t.foreground[0] as f64
            + 0.7152 * t.foreground[1] as f64
            + 0.0722 * t.foreground[2] as f64;
        assert!(l_surface > 200.0, "expected light surface, got {l_surface}");
        assert!(l_fg < 80.0, "expected dark foreground, got {l_fg}");
    }

    /// Surface alpha is strictly between 0 and 1 for both palettes — the panel
    /// is *frosted*, not opaque and not invisible.
    #[test]
    fn glass_tones_surface_alpha_is_partially_transparent() {
        for theme in [anvil_theme::MINERAL_DARK, anvil_theme::MINERAL_LIGHT] {
            let t = glass_tones_for(&theme);
            assert!(
                t.surface_alpha > 0.5 && t.surface_alpha < 1.0,
                "expected 0.5 < alpha < 1.0, got {}",
                t.surface_alpha
            );
        }
    }

    // --- draw_right_hud smoke -----------------------------------------------

    /// `draw_right_hud` paints glyphs for the standard sections (REPO / GIT /
    /// LAST RUN / AGENTS / SYSTEM) and does not panic on a reasonably-sized
    /// raster.
    #[test]
    fn draw_right_hud_smoke_emits_glyphs() {
        let m = metrics();
        let mut r = Raster::new(1200, 800);
        r.pad_x = 24.0;
        r.pad_y = 24.0;
        let mut painter = StubPainter::default();
        let snap = Snapshot {
            connection: Connection::Live,
            ..Default::default()
        };
        let local = LocalContext {
            cwd: "/Users/p/projects/anvil".to_string(),
            git: GitState::Dirty,
            branch: "main".to_string(),
            git_dirty: 2,
            git_ahead: 1,
            git_behind: 0,
            head_short: "abc1234".to_string(),
            head_subject: "fix: scroll".to_string(),
            run: RunState::Ok,
            run_duration_ms: 1200,
            ..LocalContext::default()
        };
        let surface_rect = PixelRect {
            x: 800.0,
            y: 0.0,
            w: 400.0,
            h: 800.0,
        };
        let mut hits = Vec::new();
        let mut section_hits: Vec<SectionHeaderHit> = Vec::new();
        draw_right_hud(
            &mut r,
            &mut painter,
            m,
            &anvil_theme::MINERAL_DARK,
            &snap,
            &local,
            surface_rect,
            80,
            34,
            1,
            38,
            &mut hits,
            &SectionId::DEFAULT_ORDER,
            &mut section_hits,
        );
        let chars: Vec<char> = painter
            .calls
            .iter()
            .filter_map(|(cp, _)| char::from_u32(*cp))
            .collect();
        // Section headers
        assert!(chars.contains(&'R') && chars.contains(&'E') && chars.contains(&'P'));
        // Branch name "main"
        assert!(chars.contains(&'m') && chars.contains(&'a') && chars.contains(&'i'));
        // Head short sha first char
        assert!(chars.contains(&'a') && chars.contains(&'b') && chars.contains(&'c'));
    }

    /// HUD bails cleanly when given too few columns to render anything useful.
    #[test]
    fn draw_right_hud_returns_early_when_too_narrow() {
        let m = metrics();
        let mut r = Raster::new(400, 200);
        let mut painter = StubPainter::default();
        let snap = Snapshot::default();
        let local = LocalContext::default();
        let surface_rect = PixelRect {
            x: 380.0,
            y: 0.0,
            w: 20.0,
            h: 200.0,
        };
        let mut hits = Vec::new();
        let mut section_hits: Vec<SectionHeaderHit> = Vec::new();
        draw_right_hud(
            &mut r,
            &mut painter,
            m,
            &anvil_theme::MINERAL_DARK,
            &snap,
            &local,
            surface_rect,
            38,
            2, // far too narrow (< 12)
            0,
            10,
            &mut hits,
            &SectionId::DEFAULT_ORDER,
            &mut section_hits,
        );
        assert!(painter.calls.is_empty(), "expected no draws for narrow HUD");
    }

    // --- gauge_bar -----------------------------------------------------------

    /// Pure function: gauge_bar renders exactly `cells` block glyphs, each
    /// proportionally shaded. At ratio=0 every cell is ▁ (index 0); at
    /// ratio=1 every cell is █ (index 7).
    #[test]
    fn gauge_bar_renders_proportional_blocks() {
        // At 0.0, all cells should be ▁ (the lowest block).
        let zero = gauge_bar(0.0, 6);
        assert_eq!(zero.chars().count(), 6);
        assert!(
            zero.chars().all(|c| c == '▁'),
            "expected all ▁ at ratio=0, got '{zero}'"
        );

        // At 1.0, all cells should be █ (the highest block).
        let full = gauge_bar(1.0, 6);
        assert_eq!(full.chars().count(), 6);
        assert!(
            full.chars().all(|c| c == '█'),
            "expected all █ at ratio=1, got '{full}'"
        );

        // At 0.5, the bar should be partially filled — last cells are lighter
        // than first cells (since fill = ratio / cell_ratio decreases as
        // cell_ratio grows).
        let half = gauge_bar(0.5, 6);
        assert_eq!(half.chars().count(), 6);
        // First cell (cell_ratio=1/6) has fill = 0.5/(1/6)=3.0 → clamped 1.0 → █
        assert_eq!(half.chars().next().unwrap(), '█');
        // Last cell (cell_ratio=6/6=1.0) has fill = 0.5/1.0=0.5 → idx ~4 → ▄
        let last = half.chars().last().unwrap();
        assert!(
            GAUGE_BLOCKS.contains(&last),
            "last cell should be a block glyph, got '{last}'"
        );
    }

    // --- CONTEXT section -------------------------------------------------------

    /// CONTEXT section emits cluster and namespace when kube_context is Some.
    #[test]
    fn context_section_emits_glyphs_when_kube_present() {
        use anvil_prompt_core::{EnvKind, KubeCtx};
        let m = metrics();
        let mut r = Raster::new(1200, 800);
        r.pad_x = 24.0;
        r.pad_y = 24.0;
        let mut painter = StubPainter::default();
        let snap = Snapshot {
            connection: Connection::Live,
            ..Default::default()
        };
        let local = LocalContext {
            cwd: "/Users/p/anvil".to_string(),
            kube_context: Some(KubeCtx {
                cluster: "gke-dev".to_string(),
                namespace: "default".to_string(),
                env_kind: EnvKind::Dev,
            }),
            ..LocalContext::default()
        };
        let surface_rect = PixelRect {
            x: 800.0,
            y: 0.0,
            w: 400.0,
            h: 800.0,
        };
        let mut hits = Vec::new();
        let mut section_hits: Vec<SectionHeaderHit> = Vec::new();
        draw_right_hud(
            &mut r,
            &mut painter,
            m,
            &anvil_theme::MINERAL_DARK,
            &snap,
            &local,
            surface_rect,
            80,
            34,
            1,
            38,
            &mut hits,
            &SectionId::DEFAULT_ORDER,
            &mut section_hits,
        );
        let chars: Vec<char> = painter
            .calls
            .iter()
            .filter_map(|(cp, _)| char::from_u32(*cp))
            .collect();
        // "gke-dev" cluster and "default" namespace chars must appear.
        assert!(chars.contains(&'g'), "expected 'g' from cluster name");
        assert!(chars.contains(&'k'), "expected 'k' from cluster name");
        assert!(chars.contains(&'e'), "expected 'e' from cluster name");
    }

    /// CONTEXT section is omitted when kube_context is None.
    #[test]
    fn context_section_omitted_when_no_kube() {
        let m = metrics();
        let mut r = Raster::new(1200, 800);
        r.pad_x = 24.0;
        r.pad_y = 24.0;
        let mut painter = StubPainter::default();
        let snap = Snapshot {
            connection: Connection::Live,
            ..Default::default()
        };
        let local = LocalContext {
            cwd: "/Users/p/anvil".to_string(),
            kube_context: None,
            ..LocalContext::default()
        };
        let surface_rect = PixelRect {
            x: 800.0,
            y: 0.0,
            w: 400.0,
            h: 800.0,
        };
        let mut hits = Vec::new();
        let mut section_hits: Vec<SectionHeaderHit> = Vec::new();
        draw_right_hud(
            &mut r,
            &mut painter,
            m,
            &anvil_theme::MINERAL_DARK,
            &snap,
            &local,
            surface_rect,
            80,
            34,
            1,
            38,
            &mut hits,
            &SectionId::DEFAULT_ORDER,
            &mut section_hits,
        );
        // "CONTEXT" header should not appear.
        let chars: Vec<char> = painter
            .calls
            .iter()
            .filter_map(|(cp, _)| char::from_u32(*cp))
            .collect();
        let context_label: Vec<char> = "CONTEXT".chars().collect();
        let has_context = chars
            .windows(context_label.len())
            .any(|w| w == context_label.as_slice());
        assert!(
            !has_context,
            "expected no CONTEXT header when kube_context is None"
        );
    }

    // --- PORTS section -------------------------------------------------------

    /// PORTS section emits port labels when ports vec is non-empty.
    #[test]
    fn ports_section_emits_glyphs_when_ports_present() {
        let m = metrics();
        let mut r = Raster::new(1200, 800);
        r.pad_x = 24.0;
        r.pad_y = 24.0;
        let mut painter = StubPainter::default();
        let snap = Snapshot {
            connection: Connection::Live,
            ..Default::default()
        };
        let local = LocalContext {
            cwd: "/Users/p/anvil".to_string(),
            ports: vec![3000, 5173],
            ..LocalContext::default()
        };
        let surface_rect = PixelRect {
            x: 800.0,
            y: 0.0,
            w: 400.0,
            h: 800.0,
        };
        let mut hits = Vec::new();
        let mut section_hits: Vec<SectionHeaderHit> = Vec::new();
        draw_right_hud(
            &mut r,
            &mut painter,
            m,
            &anvil_theme::MINERAL_DARK,
            &snap,
            &local,
            surface_rect,
            80,
            34,
            1,
            38,
            &mut hits,
            &SectionId::DEFAULT_ORDER,
            &mut section_hits,
        );
        let chars: Vec<char> = painter
            .calls
            .iter()
            .filter_map(|(cp, _)| char::from_u32(*cp))
            .collect();
        // ':' prefix for ":3000" and ":5173" must appear.
        assert!(chars.contains(&':'), "expected ':' from port labels");
        // '3' and '5' from the port numbers.
        assert!(chars.contains(&'3'), "expected '3' from :3000");
        assert!(chars.contains(&'5'), "expected '5' from :5173");
    }

    /// PORTS section is omitted when ports vec is empty.
    #[test]
    fn ports_section_omitted_when_empty() {
        let m = metrics();
        let mut r = Raster::new(1200, 800);
        r.pad_x = 24.0;
        r.pad_y = 24.0;
        let mut painter_empty = StubPainter::default();
        let snap = Snapshot::default();
        let local = LocalContext {
            cwd: "/Users/p/anvil".to_string(),
            ports: vec![],
            ..LocalContext::default()
        };
        let surface_rect = PixelRect {
            x: 800.0,
            y: 0.0,
            w: 400.0,
            h: 800.0,
        };
        let mut hits = Vec::new();
        let mut section_hits: Vec<SectionHeaderHit> = Vec::new();
        draw_right_hud(
            &mut r,
            &mut painter_empty,
            m,
            &anvil_theme::MINERAL_DARK,
            &snap,
            &local,
            surface_rect,
            80,
            34,
            1,
            38,
            &mut hits,
            &SectionId::DEFAULT_ORDER,
            &mut section_hits,
        );
        // "PORTS" section header: look for 'P','O','R','T','S' consecutive.
        let chars: Vec<char> = painter_empty
            .calls
            .iter()
            .filter_map(|(cp, _)| char::from_u32(*cp))
            .collect();
        let ports_label: Vec<char> = "PORTS".chars().collect();
        let has_ports_header = chars
            .windows(ports_label.len())
            .any(|w| w == ports_label.as_slice());
        assert!(
            !has_ports_header,
            "expected no PORTS header when ports is empty"
        );
    }

    // --- RECENT section -------------------------------------------------------

    /// RECENT section emits basenames when recent_files is non-empty.
    #[test]
    fn recent_section_emits_basenames_when_files_present() {
        let m = metrics();
        let mut r = Raster::new(1200, 800);
        r.pad_x = 24.0;
        r.pad_y = 24.0;
        let mut painter = StubPainter::default();
        let snap = Snapshot {
            connection: Connection::Live,
            ..Default::default()
        };
        let local = LocalContext {
            cwd: "/Users/p/anvil".to_string(),
            recent_files: vec![
                "/Users/p/anvil/src/main.rs".to_string(),
                "/Users/p/anvil/Cargo.toml".to_string(),
            ],
            ..LocalContext::default()
        };
        let surface_rect = PixelRect {
            x: 800.0,
            y: 0.0,
            w: 400.0,
            h: 800.0,
        };
        let mut hits = Vec::new();
        let mut section_hits: Vec<SectionHeaderHit> = Vec::new();
        draw_right_hud(
            &mut r,
            &mut painter,
            m,
            &anvil_theme::MINERAL_DARK,
            &snap,
            &local,
            surface_rect,
            80,
            34,
            1,
            38,
            &mut hits,
            &SectionId::DEFAULT_ORDER,
            &mut section_hits,
        );
        let chars: Vec<char> = painter
            .calls
            .iter()
            .filter_map(|(cp, _)| char::from_u32(*cp))
            .collect();
        // "main.rs" basename: 'm','a','i','n','.','r','s'
        assert!(chars.contains(&'m'), "expected 'm' from 'main.rs'");
        assert!(chars.contains(&'.'), "expected '.' from 'main.rs'");
        // Hits should be populated (one per file).
        assert!(
            hits.len() >= 2,
            "expected at least 2 HudHits for recent files, got {}",
            hits.len()
        );
    }

    /// RECENT section is omitted when recent_files is empty.
    #[test]
    fn recent_section_omitted_when_empty() {
        let m = metrics();
        let mut r = Raster::new(1200, 800);
        r.pad_x = 24.0;
        r.pad_y = 24.0;
        let mut painter = StubPainter::default();
        let snap = Snapshot::default();
        let local = LocalContext {
            cwd: "/Users/p/anvil".to_string(),
            recent_files: vec![],
            ..LocalContext::default()
        };
        let surface_rect = PixelRect {
            x: 800.0,
            y: 0.0,
            w: 400.0,
            h: 800.0,
        };
        let mut hits = Vec::new();
        let mut section_hits: Vec<SectionHeaderHit> = Vec::new();
        draw_right_hud(
            &mut r,
            &mut painter,
            m,
            &anvil_theme::MINERAL_DARK,
            &snap,
            &local,
            surface_rect,
            80,
            34,
            1,
            38,
            &mut hits,
            &SectionId::DEFAULT_ORDER,
            &mut section_hits,
        );
        // "RECENT" header should not appear.
        let chars: Vec<char> = painter
            .calls
            .iter()
            .filter_map(|(cp, _)| char::from_u32(*cp))
            .collect();
        let recent_label: Vec<char> = "RECENT".chars().collect();
        let has_recent = chars
            .windows(recent_label.len())
            .any(|w| w == recent_label.as_slice());
        assert!(
            !has_recent,
            "expected no RECENT header when recent_files is empty"
        );
    }

    // --- system_section_compact_row -----------------------------------------

    /// Smoke test: SYSTEM section emits a single compact row with "mem" and
    /// "load" on one line. No "disk" row in the new design.
    #[test]
    fn system_section_compact_row_has_mem_and_load() {
        let m = metrics();
        let mut r = Raster::new(1200, 800);
        r.pad_x = 24.0;
        r.pad_y = 24.0;
        let mut painter = StubPainter::default();
        let snap = Snapshot {
            connection: Connection::Live,
            ..Default::default()
        };
        let local = LocalContext {
            cwd: "/Users/p/projects/anvil".to_string(),
            ..LocalContext::default()
        };
        let surface_rect = PixelRect {
            x: 800.0,
            y: 0.0,
            w: 400.0,
            h: 800.0,
        };
        let mut hits = Vec::new();
        let mut section_hits: Vec<SectionHeaderHit> = Vec::new();
        draw_right_hud(
            &mut r,
            &mut painter,
            m,
            &anvil_theme::MINERAL_DARK,
            &snap,
            &local,
            surface_rect,
            80,
            34,
            1,
            38,
            &mut hits,
            &SectionId::DEFAULT_ORDER,
            &mut section_hits,
        );
        let chars: Vec<char> = painter
            .calls
            .iter()
            .filter_map(|(cp, _)| char::from_u32(*cp))
            .collect();
        // "mem" label must appear in the compact system row.
        assert!(chars.contains(&'m'), "expected 'm' (from 'mem') in output");
        // "load" label must appear.
        assert!(chars.contains(&'l'), "expected 'l' (from 'load') in output");
        // At least one block glyph (▁–█) should be present from the mem gauge.
        let has_block = chars.iter().any(|c| GAUGE_BLOCKS.contains(c));
        assert!(
            has_block,
            "expected at least one block glyph in SYSTEM section"
        );
        // "disk" should NOT appear — it was removed from SYSTEM.
        let disk_label: Vec<char> = "disk".chars().collect();
        let has_disk = chars
            .windows(disk_label.len())
            .any(|w| w == disk_label.as_slice());
        assert!(!has_disk, "expected no 'disk' row in compact SYSTEM section");
    }
}
