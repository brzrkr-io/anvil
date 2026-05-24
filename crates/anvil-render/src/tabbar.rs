//! The low-profile terminal tab bar — a fixed-height pixel strip, drawn into
//! the raster.

use anvil_workspace::tab::TabManager;

use crate::raster::{FontMetrics, GlyphPainter, PixelRect, Raster};

// Traffic lights (red/yellow/green) span ~78 *points* horizontally on macOS,
// starting ~10pt from the window's left edge. We reserve a generous 80pt
// (clear of even the rightmost green button) and convert to device pixels
// at the actual window scale — 1× / 2× retina / 3× super-retina all work.
const TRAFFIC_LIGHT_RESERVE_PT: f64 = 80.0;

// --- Chrome palette (matches docs/design/layout-mockups.html Option D) ------

/// graphite: chrome row background (#0b0d0e).
const GRAPHITE: [u8; 3] = [0x0b, 0x0d, 0x0e];
/// charcoal: active-tab background (#161a1c). Note: at retina with the
/// macOS title bar's translucent material on top, pure charcoal can blend
/// into graphite — we pick a slightly raised tone for visible contrast.
const CHARCOAL: [u8; 3] = [0x1d, 0x21, 0x29];
/// chrome border: hairline below the chrome row (#23262b).
const CHROME_BORDER: [u8; 3] = [0x23, 0x26, 0x2b];
/// text-muted: inactive-tab + right-side label tone (#a1a4a9).
const TEXT_MUTED: [u8; 3] = [0xa1, 0xa4, 0xa9];
/// mist: active-tab label tone (#d2d8db) — brighter than text-muted.
const MIST: [u8; 3] = [0xd2, 0xd8, 0xdb];
/// ash: separator glyph tone in the right indicators (#374046).
const ASH: [u8; 3] = [0x37, 0x40, 0x46];
/// status.attention: amber unread dot on background tabs (#b07a14).
const ATTENTION: [u8; 3] = [0xb0, 0x7a, 0x14];

/// Hit region for a single element in the chrome row.
#[derive(Clone, Debug)]
pub struct TabBarHit {
    /// Hit rect in device pixels (raster space).
    pub rect: PixelRect,
    pub kind: TabBarHitKind,
}

/// What a tab-bar click means.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TabBarHitKind {
    /// Switch to tab at this index.
    Tab(usize),
    /// Close tab at this index.
    CloseTab(usize),
    /// Open a new tab (+  button).
    AddTab,
}

/// Hit regions populated by [`draw_tab_bar`] and consumed by mouse-down.
#[derive(Default, Debug)]
pub struct TabBarHits {
    pub hits: Vec<TabBarHit>,
}

impl TabBarHits {
    pub fn clear(&mut self) {
        self.hits.clear();
    }
}

/// Draw the chrome+tab row at raster row 0. Always drawn (chrome is always
/// present, even with 0 or 1 tabs).
///
/// Layout left-to-right:
///   1. Traffic-light reserved zone (~78 device-px) — nothing drawn here.
///   2. Basin mark (U+F1396 md-circle_half_full) in theme.accent, immediately after the reserved zone.
///   3. Content-width tabs (label + padding + close ×), then `+` button.
///   4. Right-side indicators (branch `⎇` + name · clock) right-aligned.
///
/// Active tab: theme.surface background + 2px bottom accent rule.
/// Inactive tab: transparent (theme.background), theme.ansi[8] dim text.
/// Unread dot: amber `·` on inactive tabs with PTY output since last focus.
/// Close ×: shown on the active tab only (hover requires mouse tracking).
/// `chrome_top_px` is the FIXED pixel height of the chrome strip — not tied
/// to `cell_h`. Matches Option D's 36pt chrome row, scaled to device pixels
/// by the caller. The terminal viewport starts at y = `chrome_top_px`
/// (i.e. `raster.pad_y` is set to this value upstream).
#[allow(clippy::too_many_arguments)]
pub fn draw_tab_bar(
    raster: &mut Raster,
    painter: &mut dyn GlyphPainter,
    metrics: FontMetrics,
    tabs: &TabManager,
    branch: &str,
    clock: &str,
    window_scale: f64,
    chrome_top_px: f64,
    hits_out: &mut TabBarHits,
) {
    hits_out.clear();

    let cell_w = metrics.cell_w;
    let cell_h = metrics.cell_h;
    let total_w = raster.width as f64;
    if total_w <= 0.0 || chrome_top_px <= 0.0 {
        return;
    }

    // ── Chrome strip background ──────────────────────────────────────────
    // Full-width graphite from y=0 to the 1px hairline. The hairline lives
    // at chrome_top_px - 1; the strip's painted region is [0, chrome_top_px).
    raster.fill_pixel_rect(0.0, 0.0, total_w, chrome_top_px - 1.0, GRAPHITE);

    // Bright accent for basin + active-tab rule.
    const ACCENT_BRIGHT: [u8; 3] = [0x54, 0xb7, 0xc0];

    // Vertical baseline for chrome glyphs: cell rect's top is centred in
    // the strip so the glyph sits visually in the middle of the chrome row.
    let glyph_y = ((chrome_top_px - cell_h) * 0.5).max(0.0);

    // Reserve the traffic-light zone (left side of the chrome row).
    let tl_reserve_px = TRAFFIC_LIGHT_RESERVE_PT * window_scale;
    let basin_x = tl_reserve_px;

    // ── Basin mark ─────────────────────────────────────────────────────────
    // U+25D2 (◒) is absent from BlexMonoNerdFontMono; use U+F1396
    // (md-circle_half_full) which IS present and is visually equivalent.
    const BASIN_MARK: u32 = 0xF1396;
    if basin_x + cell_w < total_w {
        raster.glyph_at(
            painter,
            metrics,
            basin_x,
            glyph_y,
            BASIN_MARK,
            ACCENT_BRIGHT,
        );
    }

    // ── Right indicators (branch · clock) ────────────────────────────────
    let right_str = build_right_str(branch, clock);
    let right_w = right_str.chars().count() as f64 * cell_w;
    let right_pad = 14.0 * window_scale; // D: .right-indicators { padding: 0 14px }
    let right_start_x = (total_w - right_w - right_pad).max(0.0);
    draw_right_indicators(raster, painter, metrics, &right_str, right_start_x, glyph_y);

    // ── Tabs ─────────────────────────────────────────────────────────────
    let n = tabs.count();
    let tabs_start_x = basin_x + 2.0 * cell_w; // 2 cells of breathing after basin
    let tabs_end_x = (right_start_x - 2.0 * cell_w).max(tabs_start_x);
    let avail_tab_w = tabs_end_x - tabs_start_x;

    // Per-tab pixel width: label + 2 left pad + 1 right pad + 1 gap + 1 ×.
    let raw_widths: Vec<f64> = (0..n)
        .map(|t| {
            let label_len = tab_label(tabs, t).chars().count();
            let cells = (label_len + 5).clamp(9, 24) as f64;
            cells * cell_w
        })
        .collect();
    let total_raw_w: f64 = raw_widths.iter().sum();
    let tab_widths: Vec<f64> = if total_raw_w > avail_tab_w && n > 0 {
        let min_each = 8.0 * cell_w;
        if (n as f64) * min_each > avail_tab_w {
            vec![min_each; n]
        } else {
            raw_widths
                .iter()
                .map(|w| (w * avail_tab_w / total_raw_w).max(min_each))
                .collect()
        }
    } else {
        raw_widths
    };

    let mut x = tabs_start_x;
    for t in 0..n {
        let tw = *tab_widths.get(t).unwrap_or(&(8.0 * cell_w));
        let is_active = t == tabs.active;

        // Active tab: charcoal panel covering the FULL chrome strip height
        // (minus the hairline), then a 3px accent rule pinned just above
        // the hairline. Matches D's `.tab.active { background: charcoal }
        // .tab.active::after { left:4px; right:4px; bottom:0; height:2px }`.
        if is_active {
            raster.fill_pixel_rect(x, 0.0, tw, chrome_top_px - 1.0, CHARCOAL);
            let inset = 4.0 * window_scale;
            let rule_y = chrome_top_px - 4.0;
            raster.fill_pixel_rect(
                x + inset,
                rule_y,
                (tw - 2.0 * inset).max(0.0),
                3.0,
                ACCENT_BRIGHT,
            );
        }

        // Label: pixel-positioned, sitting inside the tab with a 2-cell
        // left pad and a 3-cell gap+× on the right.
        let fg = if is_active { MIST } else { TEXT_MUTED };
        let label = tab_label(tabs, t);
        let label_x0 = x + 2.0 * cell_w;
        let label_x_end = x + tw - 3.0 * cell_w;
        for (i, cp) in label.chars().enumerate() {
            let lx = label_x0 + i as f64 * cell_w;
            if lx + cell_w > label_x_end {
                break;
            }
            raster.glyph_at(painter, metrics, lx, glyph_y, cp as u32, fg);
        }

        // Close × on active tab.
        let close_x = x + tw - 2.0 * cell_w;
        if is_active && close_x + cell_w <= total_w {
            raster.glyph_at(painter, metrics, close_x, glyph_y, '×' as u32, TEXT_MUTED);
        }

        // Unread dot on background tabs with new output.
        let tab_has_unread = tabs.tabs.get(t).is_some_and(|tab| tab.has_unread);
        let dot_x = x + tw - cell_w;
        if !is_active && tab_has_unread && dot_x + cell_w <= total_w {
            raster.glyph_at(painter, metrics, dot_x, glyph_y, '·' as u32, ATTENTION);
        }

        // Hit rect spans the full strip vertically (clickable anywhere in
        // the tab's chrome region, not just on the glyph row).
        hits_out.hits.push(TabBarHit {
            rect: PixelRect {
                x,
                y: 0.0,
                w: tw,
                h: chrome_top_px,
            },
            kind: TabBarHitKind::Tab(t),
        });
        if close_x + 2.0 * cell_w <= total_w {
            hits_out.hits.push(TabBarHit {
                rect: PixelRect {
                    x: close_x,
                    y: 0.0,
                    w: 2.0 * cell_w,
                    h: chrome_top_px,
                },
                kind: TabBarHitKind::CloseTab(t),
            });
        }

        x += tw;
    }

    // `+` button: one cell of gap after the last tab.
    let add_x = x + cell_w;
    if add_x + cell_w <= right_start_x {
        raster.glyph_at(painter, metrics, add_x, glyph_y, '+' as u32, TEXT_MUTED);
        hits_out.hits.push(TabBarHit {
            rect: PixelRect {
                x: add_x,
                y: 0.0,
                w: 2.0 * cell_w,
                h: chrome_top_px,
            },
            kind: TabBarHitKind::AddTab,
        });
    }

    // 1px hairline at the bottom of the strip.
    raster.fill_pixel_rect(0.0, chrome_top_px - 1.0, total_w, 1.0, CHROME_BORDER);
}

/// Build the right-side indicator string. Uses the Nerd Font branch glyph
/// (U+E0A0) which IS in BlexMonoNerdFontMono — the previously-used U+2387
/// (⎇) is not in the font and rendered as nothing.
fn build_right_str(branch: &str, clock: &str) -> String {
    if branch.is_empty() {
        clock.to_string()
    } else {
        format!("\u{e0a0} {} · {}", branch, clock)
    }
}

/// Pixel-positioned. Branch glyph in accent, branch name in text-muted,
/// separator `·` in ash, clock in text-muted. Matches D's `.right-indicators`.
fn draw_right_indicators(
    raster: &mut Raster,
    painter: &mut dyn GlyphPainter,
    metrics: FontMetrics,
    right_str: &str,
    start_x: f64,
    glyph_y: f64,
) {
    const ACCENT: [u8; 3] = [0x2f, 0x7f, 0x86];
    let branch_glyph = '\u{e0a0}';
    let sep_glyph = '·';
    for (i, cp) in right_str.chars().enumerate() {
        let color = if cp == branch_glyph && i == 0 {
            ACCENT
        } else if cp == sep_glyph {
            ASH
        } else {
            TEXT_MUTED
        };
        let x = start_x + i as f64 * metrics.cell_w;
        raster.glyph_at(painter, metrics, x, glyph_y, cp as u32, color);
    }
}

/// Derive a display label for tab `t`:
///   - shell title if set,
///   - basename of the focused pane's cwd,
///   - fallback to "shell".
fn tab_label(tabs: &TabManager, t: usize) -> String {
    let tab = match tabs.tabs.get(t) {
        Some(tab) => tab,
        None => return "shell".to_string(),
    };
    let focused_id = tab.tree.focused;
    if let Some(pane) = tab.registry.get(focused_id) {
        let title = pane.terminal().title();
        if !title.is_empty() {
            return title.to_string();
        }
        let cwd = pane.terminal().cwd_path();
        if !cwd.is_empty() {
            return anvil_workspace::tab::basename(cwd).to_string();
        }
    }
    "shell".to_string()
}

// --- Tests ------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raster::{PixelRect, pixel_at};

    // Stub painter.
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

    fn make_hits() -> TabBarHits {
        TabBarHits::default()
    }

    /// Chrome row always renders — even with 0 tabs the basin mark is drawn.
    /// (Previously this was a no-op below 2 tabs; now chrome is always present.)
    #[test]
    fn draw_tab_bar_noop_below_2_tabs() {
        let m = metrics();
        let mut r = Raster::new(200, 80);
        let mut painter = StubPainter::default();
        r.clear([1, 2, 3]);

        let mgr = TabManager::default(); // 0 tabs
        let mut hits = make_hits();

        draw_tab_bar(
            &mut r,
            &mut painter,
            m,
            &mgr,
            "",
            "12:00",
            1.0,
            m.cell_h * 2.0,
            &mut hits,
        );
        // Basin mark must have been drawn (painter received U+F1396).
        let basin_calls: Vec<_> = painter
            .calls
            .iter()
            .filter(|&&(glyph, _)| glyph == 0xF1396)
            .collect();
        assert!(
            !basin_calls.is_empty(),
            "expected basin mark drawn for 0 tabs, painter calls: {:?}",
            painter.calls
        );
    }

    /// draw_tab_bar renders chrome even with 1 tab.
    #[test]
    fn draw_tab_bar_noop_for_one_tab() {
        use anvil_workspace::tab::{Tab, TabManager};
        let m = metrics();
        let mut r = Raster::new(200, 80);
        let mut painter = StubPainter::default();
        r.clear([1, 2, 3]);

        let mut mgr = TabManager::default();
        mgr.push(Tab::new_single_pane(20, 4, 0));
        let mut hits = make_hits();

        draw_tab_bar(
            &mut r,
            &mut painter,
            m,
            &mgr,
            "",
            "12:00",
            1.0,
            m.cell_h * 2.0,
            &mut hits,
        );
        // Chrome is rendered: basin mark present.
        let basin_calls: Vec<_> = painter
            .calls
            .iter()
            .filter(|&&(glyph, _)| glyph == 0xF1396)
            .collect();
        assert!(
            !basin_calls.is_empty(),
            "expected basin mark for 1 tab, painter calls: {:?}",
            painter.calls
        );
    }

    /// Basin mark U+F1396 (md-circle_half_full) is in the painter's call log.
    #[test]
    fn draw_tab_bar_basin_mark_in_painter_calls() {
        use anvil_workspace::tab::{Tab, TabManager};
        let m = metrics();
        let mut r = Raster::new(400, 80);
        let mut painter = StubPainter::default();
        r.clear([0, 0, 0]);

        let mut mgr = TabManager::default();
        mgr.push(Tab::new_single_pane(20, 4, 0));
        let mut hits = make_hits();

        draw_tab_bar(
            &mut r,
            &mut painter,
            m,
            &mgr,
            "main",
            "14:22",
            1.0,
            m.cell_h * 2.0,
            &mut hits,
        );

        const ACCENT_BRIGHT: [u8; 3] = [0x54, 0xb7, 0xc0];
        let basin: Vec<_> = painter
            .calls
            .iter()
            .filter(|&&(glyph, color)| glyph == 0xF1396 && color == ACCENT_BRIGHT)
            .collect();
        assert_eq!(
            basin.len(),
            1,
            "expected exactly one basin mark (U+F1396) in ACCENT_BRIGHT; painter calls: {:?}",
            painter.calls
        );
    }

    /// draw_tab_bar paints the bar with 2 tabs.
    #[test]
    fn draw_tab_bar_paints_with_two_tabs() {
        use anvil_workspace::tab::{Tab, TabManager};
        let m = metrics();
        let mut r = Raster::new(400, 80);
        let mut painter = StubPainter::default();
        r.clear([0, 0, 0]);

        let mut mgr = TabManager::default();
        mgr.push(Tab::new_single_pane(20, 4, 0));
        mgr.push(Tab::new_single_pane(20, 4, 0));
        mgr.active = 0;
        let mut hits = make_hits();

        draw_tab_bar(
            &mut r,
            &mut painter,
            m,
            &mgr,
            "",
            "14:22",
            1.0,
            m.cell_h * 2.0,
            &mut hits,
        );

        // Active tab segment should be painted with CHARCOAL (D's
        // `--charcoal` panel color). pad_x=0, RESERVE=80pt × scale 1.0=80px,
        // cell_w=10 → tl_cols=8. basin col 8, tabs_start_col 10. Active tab 0
        // width=10, at cols 10..20. col 15 → x = 150, y = 10 (mid of cell_h=20).
        let px = pixel_at(&r, 150, 10);
        assert_eq!(px, CHARCOAL, "expected CHARCOAL for active tab, got {px:?}");
    }

    /// When a non-active tab has `has_unread`, the painter receives a `·` glyph
    /// in the attention colour (#b07a14).
    #[test]
    fn draw_tab_bar_unread_dot_on_background_tab() {
        use anvil_workspace::tab::{Tab, TabManager};
        let m = metrics();
        let mut r = Raster::new(400, 80);
        let mut painter = StubPainter::default();
        r.clear([0, 0, 0]);

        let mut mgr = TabManager::default();
        mgr.push(Tab::new_single_pane(20, 4, 0)); // tab 0 — active
        mgr.push(Tab::new_single_pane(20, 4, 0)); // tab 1 — background with unread
        mgr.active = 0;
        mgr.tabs[1].has_unread = true;
        let mut hits = make_hits();

        draw_tab_bar(
            &mut r,
            &mut painter,
            m,
            &mgr,
            "",
            "14:22",
            1.0,
            m.cell_h * 2.0,
            &mut hits,
        );

        const ATTENTION: [u8; 3] = [0xb0, 0x7a, 0x14];
        let dot_calls: Vec<_> = painter
            .calls
            .iter()
            .filter(|&&(glyph, color)| glyph == '·' as u32 && color == ATTENTION)
            .collect();
        assert_eq!(
            dot_calls.len(),
            1,
            "expected exactly one · in attention colour; got painter calls: {:?}",
            painter.calls
        );
    }

    /// Active tab never shows the unread dot even when has_unread is true.
    #[test]
    fn draw_tab_bar_no_unread_dot_on_active_tab() {
        use anvil_workspace::tab::{Tab, TabManager};
        let m = metrics();
        let mut r = Raster::new(400, 80);
        let mut painter = StubPainter::default();
        r.clear([0, 0, 0]);

        let mut mgr = TabManager::default();
        mgr.push(Tab::new_single_pane(20, 4, 0));
        mgr.push(Tab::new_single_pane(20, 4, 0));
        mgr.active = 0;
        mgr.tabs[0].has_unread = true; // active tab — dot must be suppressed
        let mut hits = make_hits();

        draw_tab_bar(
            &mut r,
            &mut painter,
            m,
            &mgr,
            "",
            "14:22",
            1.0,
            m.cell_h * 2.0,
            &mut hits,
        );

        const ATTENTION: [u8; 3] = [0xb0, 0x7a, 0x14];
        let dot_calls: Vec<_> = painter
            .calls
            .iter()
            .filter(|&&(glyph, color)| glyph == '·' as u32 && color == ATTENTION)
            .collect();
        assert!(
            dot_calls.is_empty(),
            "expected no dot on active tab, got: {dot_calls:?}"
        );
    }
}
