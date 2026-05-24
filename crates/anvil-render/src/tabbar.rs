//! The low-profile terminal tab bar — one text-row tall, drawn into the raster.
//!
//! Ported from `src/render/tabbar.zig`.

use anvil_theme::Theme;
use anvil_workspace::tab::TabManager;

use crate::raster::{FontMetrics, GlyphPainter, PixelRect, Raster};

// Traffic lights (red/yellow/green) span ~78 *points* horizontally on macOS,
// starting ~10pt from the window's left edge. We reserve a generous 80pt
// (clear of even the rightmost green button) and convert to device pixels
// at the actual window scale — 1× / 2× retina / 3× super-retina all work.
const TRAFFIC_LIGHT_RESERVE_PT: f64 = 80.0;

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
///   2. Basin mark `◒` in theme.accent, immediately after the reserved zone.
///   3. Content-width tabs (label + padding + close ×), then `+` button.
///   4. Right-side indicators (branch `⎇` + name · clock) right-aligned.
///
/// Active tab: theme.surface background + 2px bottom accent rule.
/// Inactive tab: transparent (theme.background), theme.ansi[8] dim text.
/// Unread dot: amber `·` on inactive tabs with PTY output since last focus.
/// Close ×: shown on the active tab only (hover requires mouse tracking).
#[allow(clippy::too_many_arguments)]
pub fn draw_tab_bar(
    raster: &mut Raster,
    painter: &mut dyn GlyphPainter,
    metrics: FontMetrics,
    theme: &Theme,
    tabs: &TabManager,
    branch: &str,
    clock: &str,
    // Device-pixels-per-point for the current window (1.0 standard, 2.0
    // retina, 3.0 super-retina). Used to convert the traffic-light reserve
    // from points to device pixels.
    window_scale: f64,
    hits_out: &mut TabBarHits,
) {
    hits_out.clear();

    let cell_w = metrics.cell_w;
    let cell_h = metrics.cell_h;
    let usable_w = raster.width as f64 - 2.0 * raster.pad_x;
    let total_cols = ((usable_w.max(0.0)) / cell_w) as usize;
    if total_cols == 0 {
        return;
    }

    // How many raster columns to skip for the traffic-light zone.
    // pad_x is already the left offset of col 0 from the window edge.
    let tl_reserve_px = TRAFFIC_LIGHT_RESERVE_PT * window_scale;
    let tl_cols = (((tl_reserve_px - raster.pad_x).max(0.0)) / cell_w).ceil() as usize;

    // ── Basin mark ◒ ──────────────────────────────────────────────────────────
    let basin_col = tl_cols;
    if basin_col < total_cols {
        raster.cell_glyph(painter, metrics, basin_col, 0, '◒' as u32, theme.accent);
    }

    // ── Right-side indicators ─────────────────────────────────────────────────
    // Build the right-side string and place it from the right edge.
    let right_str = build_right_str(branch, clock);
    let right_cols = right_str.chars().count();
    // Leave one col gap from the right edge.
    let right_start = if total_cols > right_cols + 1 {
        total_cols - right_cols - 1
    } else {
        0
    };
    draw_right_indicators(
        raster,
        painter,
        metrics,
        theme,
        &right_str,
        branch,
        right_start,
    );

    // ── Tabs ──────────────────────────────────────────────────────────────────
    let n = tabs.count();
    // Tabs start 2 cols after the basin mark.
    let tabs_start_col = basin_col + 2;
    // Reserve space for right indicators (right_cols + 1 gap + 1 padding) and
    // the `+` button (2 cols).
    let add_btn_cols = 2_usize;
    let right_reserved = right_cols + 2 + add_btn_cols;
    let tabs_end_col = if total_cols > right_reserved + tabs_start_col {
        total_cols - right_reserved
    } else {
        tabs_start_col
    };
    let avail_tab_cols = tabs_end_col.saturating_sub(tabs_start_col);

    // Compute per-tab widths (content-width: label + 2 left pad + 1 right pad +
    // 1 close × + 1 gap). Min 8, max 24.
    let tab_widths: Vec<usize> = (0..n)
        .map(|t| {
            let label = tab_label(tabs, t);
            let label_len = label.chars().count();
            // label + 2 left pad + 1 × + 1 right pad
            // label + 2 left pad + 1 right pad + 1 gap + 1 × = +5
            let w = label_len + 5;
            w.clamp(9, 24)
        })
        .collect();
    let total_tab_cols: usize = tab_widths.iter().sum();

    // If there's not enough room, shrink all tabs proportionally to min 8.
    let tab_widths: Vec<usize> = if total_tab_cols > avail_tab_cols && n > 0 {
        let min_total = n * 8;
        if min_total > avail_tab_cols {
            // Not enough room even at minimum — clamp everything to 8.
            vec![8; n]
        } else {
            // Distribute available space fairly, floor to 8.
            tab_widths
                .iter()
                .map(|&w| {
                    let scaled =
                        (w as f64 * avail_tab_cols as f64 / total_tab_cols as f64) as usize;
                    scaled.max(8)
                })
                .collect()
        }
    } else {
        tab_widths
    };

    let mut col = tabs_start_col;
    for t in 0..n {
        let tw = if t < tab_widths.len() {
            tab_widths[t]
        } else {
            8
        };
        let is_active = t == tabs.active;

        // Active tab: filled surface background.
        if is_active {
            for c in col..col + tw {
                if c < total_cols {
                    raster.cell_bg(metrics, c, 0, theme.surface);
                }
            }
            // 2px accent rule along the bottom of the active segment.
            let start_px = raster.pad_x + col as f64 * cell_w;
            let seg_w_px = tw as f64 * cell_w;
            raster.fill_pixel_rect(
                start_px,
                raster.pad_y + cell_h - 2.0,
                seg_w_px,
                2.0,
                theme.accent,
            );
        }

        let fg = if is_active {
            theme.foreground
        } else {
            theme.ansi[8] // dim
        };

        // Label: starts at col + 2 (2-col left pad).
        let label = tab_label(tabs, t);
        let label_end_col = col + tw - 3; // gap + × on the right
        for (i, cp) in label.chars().enumerate() {
            let lc = col + 2 + i;
            if lc >= label_end_col {
                break;
            }
            raster.cell_glyph(painter, metrics, lc, 0, cp as u32, fg);
        }

        // Close × on active tab (at col + tw - 2).
        let close_col = col + tw - 2;
        if is_active && close_col < total_cols {
            raster.cell_glyph(painter, metrics, close_col, 0, '×' as u32, theme.ansi[8]);
        }

        // Unread dot: amber · on background tabs with new output.
        const ATTENTION: [u8; 3] = [0xb0, 0x7a, 0x14]; // status.attention
        let tab_has_unread = tabs.tabs.get(t).is_some_and(|tab| tab.has_unread);
        let dot_col = col + tw - 1;
        if !is_active && tab_has_unread && dot_col < total_cols {
            raster.cell_glyph(painter, metrics, dot_col, 0, '·' as u32, ATTENTION);
        }

        // Hit rects in device pixels.
        let hit_x = raster.pad_x + col as f64 * cell_w;
        let hit_y = raster.pad_y;
        let hit_w = tw as f64 * cell_w;
        let hit_h = cell_h;

        hits_out.hits.push(TabBarHit {
            rect: PixelRect {
                x: hit_x,
                y: hit_y,
                w: hit_w,
                h: hit_h,
            },
            kind: TabBarHitKind::Tab(t),
        });
        // Close × hit: right ~2 cols of the tab.
        if close_col < total_cols {
            let cx = raster.pad_x + close_col as f64 * cell_w;
            hits_out.hits.push(TabBarHit {
                rect: PixelRect {
                    x: cx,
                    y: hit_y,
                    w: 2.0 * cell_w,
                    h: hit_h,
                },
                kind: TabBarHitKind::CloseTab(t),
            });
        }

        col += tw;
    }

    // `+` button: 2 cols after the last tab.
    let add_col = col;
    if add_col + 2 <= total_cols {
        raster.cell_glyph(painter, metrics, add_col, 0, '+' as u32, theme.ansi[8]);
        let ax = raster.pad_x + add_col as f64 * cell_w;
        hits_out.hits.push(TabBarHit {
            rect: PixelRect {
                x: ax,
                y: raster.pad_y,
                w: 2.0 * cell_w,
                h: cell_h,
            },
            kind: TabBarHitKind::AddTab,
        });
    }
}

/// Build the right-side indicator string: `⎇ branch · HH:MM` or just `HH:MM`.
fn build_right_str(branch: &str, clock: &str) -> String {
    if branch.is_empty() {
        clock.to_string()
    } else {
        format!("⎇ {} · {}", branch, clock)
    }
}

/// Draw the right-side indicators. Branch glyph `⎇` in theme.accent;
/// branch name and separator in theme.ansi[8] (dim); clock in theme.ansi[8].
fn draw_right_indicators(
    raster: &mut Raster,
    painter: &mut dyn GlyphPainter,
    metrics: FontMetrics,
    theme: &Theme,
    right_str: &str,
    branch: &str,
    start_col: usize,
) {
    // ⎇ gets accent color; everything else is dim.
    let branch_glyph = '⎇';
    // branch text without the glyph
    let branch_len = branch.chars().count();
    // In the full string "⎇ branch · HH:MM", index of each part:
    //   0: '⎇'
    //   1: ' '
    //   2..2+branch_len: branch name
    //   2+branch_len: ' '
    //   2+branch_len+1: '·'
    //   rest: ' HH:MM'
    // If branch is empty, the whole string is the clock.
    for (i, cp) in right_str.chars().enumerate() {
        let col = start_col + i;
        let color = if !branch.is_empty() && cp == branch_glyph && i == 0 {
            theme.accent
        } else if !branch.is_empty() && i > 0 && i <= 1 + branch_len {
            // branch name chars: still accent for the name, dim for space+separator
            if i >= 2 && i < 2 + branch_len {
                theme.accent
            } else {
                theme.ansi[8]
            }
        } else {
            theme.ansi[8]
        };
        raster.cell_glyph(painter, metrics, col, 0, cp as u32, color);
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
        let theme = anvil_theme::MINERAL_DARK;
        let mut hits = make_hits();

        draw_tab_bar(
            &mut r,
            &mut painter,
            m,
            &theme,
            &mgr,
            "",
            "12:00",
            1.0,
            &mut hits,
        );
        // Basin mark must have been drawn (painter received '◒').
        let basin_calls: Vec<_> = painter
            .calls
            .iter()
            .filter(|&&(glyph, _)| glyph == '◒' as u32)
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
        let theme = anvil_theme::MINERAL_DARK;
        let mut hits = make_hits();

        draw_tab_bar(
            &mut r,
            &mut painter,
            m,
            &theme,
            &mgr,
            "",
            "12:00",
            1.0,
            &mut hits,
        );
        // Chrome is rendered: basin mark present.
        let basin_calls: Vec<_> = painter
            .calls
            .iter()
            .filter(|&&(glyph, _)| glyph == '◒' as u32)
            .collect();
        assert!(
            !basin_calls.is_empty(),
            "expected basin mark for 1 tab, painter calls: {:?}",
            painter.calls
        );
    }

    /// Basin mark U+25D2 (◒) is in the painter's call log.
    #[test]
    fn draw_tab_bar_basin_mark_in_painter_calls() {
        use anvil_workspace::tab::{Tab, TabManager};
        let m = metrics();
        let mut r = Raster::new(400, 80);
        let mut painter = StubPainter::default();
        r.clear([0, 0, 0]);

        let mut mgr = TabManager::default();
        mgr.push(Tab::new_single_pane(20, 4, 0));
        let theme = anvil_theme::MINERAL_DARK;
        let mut hits = make_hits();

        draw_tab_bar(
            &mut r,
            &mut painter,
            m,
            &theme,
            &mgr,
            "main",
            "14:22",
            1.0,
            &mut hits,
        );

        let basin: Vec<_> = painter
            .calls
            .iter()
            .filter(|&&(glyph, color)| glyph == '◒' as u32 && color == theme.accent)
            .collect();
        assert_eq!(
            basin.len(),
            1,
            "expected exactly one ◒ (U+25D2) in accent colour; painter calls: {:?}",
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
        let theme = anvil_theme::MINERAL_DARK;
        let mut hits = make_hits();

        draw_tab_bar(
            &mut r,
            &mut painter,
            m,
            &theme,
            &mgr,
            "",
            "14:22",
            1.0,
            &mut hits,
        );

        // Active tab segment should be painted with theme.surface.
        // pad_x=0, RESERVE=80pt × scale 1.0=80px, cell_w=10 → tl_cols=8.
        // basin col 8, tabs_start_col 10. Active tab 0 width=10, at cols
        // 10..20. col 15 → x = 150, y = 10 (mid of cell_h=20).
        let px = pixel_at(&r, 150, 10);
        assert_eq!(
            px, theme.surface,
            "expected surface for active tab, got {px:?}"
        );
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
        let theme = anvil_theme::MINERAL_DARK;
        let mut hits = make_hits();

        draw_tab_bar(
            &mut r,
            &mut painter,
            m,
            &theme,
            &mgr,
            "",
            "14:22",
            1.0,
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
        let theme = anvil_theme::MINERAL_DARK;
        let mut hits = make_hits();

        draw_tab_bar(
            &mut r,
            &mut painter,
            m,
            &theme,
            &mgr,
            "",
            "14:22",
            1.0,
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
