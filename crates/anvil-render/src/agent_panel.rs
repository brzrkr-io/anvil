//! Agent-panel — the primary developer-context surface for Anvil.
//!
//! Ported from `src/render/agent_panel.zig`.
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

use anvil_agent::{Connection, FindingSeverity, RunStatus, Snapshot};
use anvil_theme::{Theme, mix};

use crate::raster::{FontMetrics, GlyphPainter, Raster};

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

/// Local context: cwd, git, last-run. Used as the footer of the agent panel.
pub struct LocalContext {
    // cwd section
    pub cwd: String,

    // git section
    pub git: GitState,
    pub branch: String,
    pub git_dirty: u32,
    pub git_ahead: u32,
    pub git_behind: u32,

    // last-run section
    pub run: RunState,
    pub run_exit: i32,
    pub run_duration_ms: i64,
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
            run: RunState::Idle,
            run_exit: 0,
            run_duration_ms: 0,
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
}

/// Width of the agent-panel card in terminal columns.
pub const PANEL_COLS: usize = 36;

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
        Connection::NotInstalled => "caldera-local not found".to_string(),
        Connection::NoProject => "no .caldera in this repo".to_string(),
        Connection::Disabled => "caldera disabled for this repo".to_string(),
        Connection::Offline => "caldera-local not running".to_string(),
        Connection::ErrorState => "caldera api error".to_string(),
        Connection::Live => {
            if snap.running_count == 0
                && snap.pending_approvals_count == 0
                && snap.attention_count == 0
            {
                return "no active runs".to_string();
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

    // Minimal card: surface fill + single 1-device-pixel border.
    // Codex/Claude-Desktop style — no halo, no bevel; the surface tone
    // does the lifting and a hairline edge keeps it anchored.
    let border = mix(theme.border, theme.foreground, 0.25);
    raster.fill_pixel_rect(left_px - 1.0, top_px - 1.0, card_w_px + 2.0, 1.0, border);
    raster.fill_pixel_rect(
        left_px - 1.0,
        top_px + card_h_px,
        card_w_px + 2.0,
        1.0,
        border,
    );
    raster.fill_pixel_rect(left_px - 1.0, top_px, 1.0, card_h_px, border);
    raster.fill_pixel_rect(left_px + card_w_px, top_px, 1.0, card_h_px, border);
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

/// Draw the Local footer: one dim row with cwd · branch · last-run.
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

    // Build a compact line: "cwd · branch · run"
    let mut buf = String::with_capacity(80);

    // cwd (last component only to save space)
    let cwd_short = format_cwd(&local.cwd);
    // Use only the last path component for the footer (very compact).
    let tail = match cwd_short.rfind('/') {
        Some(sep) => &cwd_short[sep + 1..],
        None => &cwd_short,
    };
    let _ = write!(buf, "{tail}");

    // · branch (if in a repo)
    if local.git != GitState::NoRepo && !local.branch.is_empty() {
        let _ = write!(buf, " \u{00b7} {}", local.branch);
    }

    // · run state
    {
        let rtxt = format_run_status(local.run, local.run_exit, local.run_duration_ms);
        let _ = write!(buf, " \u{00b7} {rtxt}");
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

    /// Port of "formatDuration sub-second"
    #[test]
    fn format_duration_sub_second() {
        assert_eq!(format_duration(350), "0.3s");
    }

    /// Port of "formatDuration seconds with tenths"
    #[test]
    fn format_duration_seconds_with_tenths() {
        assert_eq!(format_duration(1250), "1.2s");
    }

    /// Port of "formatDuration large value"
    #[test]
    fn format_duration_large_value() {
        assert_eq!(format_duration(72000), "72s");
    }

    /// Port of "formatDuration negative clamps to zero"
    #[test]
    fn format_duration_negative_clamps() {
        assert_eq!(format_duration(-100), "0s");
    }

    // --- formatRunStatus ---

    /// Port of "formatRunStatus ok"
    #[test]
    fn format_run_status_ok() {
        let s = format_run_status(RunState::Ok, 0, 1200);
        assert!(
            s.starts_with("ok"),
            "expected to start with 'ok', got '{s}'"
        );
        assert!(s.contains("1.2s"), "expected to contain '1.2s', got '{s}'");
    }

    /// Port of "formatRunStatus failed with exit code"
    #[test]
    fn format_run_status_failed_with_exit_code() {
        let s = format_run_status(RunState::Failed, 127, 500);
        assert!(s.contains("failed"), "expected 'failed' in '{s}'");
        assert!(s.contains("127"), "expected '127' in '{s}'");
    }

    /// Port of "formatRunStatus idle"
    #[test]
    fn format_run_status_idle() {
        let s = format_run_status(RunState::Idle, 0, 0);
        assert_eq!(s, "idle");
    }

    // --- formatAheadBehind ---

    /// Port of "formatAheadBehind ahead only"
    #[test]
    fn format_ahead_behind_ahead_only() {
        let s = format_ahead_behind(2, 0);
        assert!(s.contains('2'), "expected '2' in '{s}'");
    }

    /// Port of "formatAheadBehind both"
    #[test]
    fn format_ahead_behind_both() {
        let s = format_ahead_behind(3, 1);
        assert!(s.contains('3'), "expected '3' in '{s}'");
        assert!(s.contains('1'), "expected '1' in '{s}'");
    }

    /// Port of "formatAheadBehind neither returns empty"
    #[test]
    fn format_ahead_behind_neither_returns_empty() {
        assert_eq!(format_ahead_behind(0, 0), "");
    }

    // --- formatCwd ---

    /// Port of "formatCwd last two components"
    #[test]
    fn format_cwd_last_two_components() {
        let s = format_cwd("/Users/foo/projects/anvil");
        assert!(
            s.contains("projects/anvil"),
            "expected 'projects/anvil' in '{s}'"
        );
    }

    /// Port of "formatCwd short path returned as-is"
    #[test]
    fn format_cwd_short_path_as_is() {
        let s = format_cwd("/anvil");
        assert_eq!(s, "/anvil");
    }

    /// Port of "formatCwd empty"
    #[test]
    fn format_cwd_empty() {
        assert_eq!(format_cwd(""), "");
    }

    // --- headerBulletColor ---

    /// Port of "headerBulletColor: not_installed is alloy"
    #[test]
    fn header_bullet_color_not_installed_is_alloy() {
        let snap = Snapshot {
            connection: Connection::NotInstalled,
            ..Default::default()
        };
        assert_eq!(header_bullet_color(&snap), ALLOY);
    }

    /// Port of "headerBulletColor: live with no activity is verified"
    #[test]
    fn header_bullet_color_live_no_activity_is_verified() {
        let snap = Snapshot {
            connection: Connection::Live,
            ..Default::default()
        };
        assert_eq!(header_bullet_color(&snap), VERIFIED);
    }

    /// Port of "headerBulletColor: live with pending approval is attention"
    #[test]
    fn header_bullet_color_live_pending_approval_is_attention() {
        let snap = Snapshot {
            connection: Connection::Live,
            pending_approvals_count: 1,
            ..Default::default()
        };
        assert_eq!(header_bullet_color(&snap), ATTENTION);
    }

    /// Port of "headerBulletColor: live with running count is agent_violet"
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

    /// Port of "buildHeaderSummary: not_installed"
    #[test]
    fn build_header_summary_not_installed() {
        let snap = Snapshot {
            connection: Connection::NotInstalled,
            ..Default::default()
        };
        assert_eq!(build_header_summary(&snap), "caldera-local not found");
    }

    /// Port of "buildHeaderSummary: live empty"
    #[test]
    fn build_header_summary_live_empty() {
        let snap = Snapshot {
            connection: Connection::Live,
            ..Default::default()
        };
        assert_eq!(build_header_summary(&snap), "no active runs");
    }

    /// Port of "buildHeaderSummary: live with running"
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
            run: RunState::Ok,
            run_exit: 0,
            run_duration_ms: 1200,
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
            run: RunState::Failed,
            run_exit: 1,
            run_duration_ms: 500,
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
}
