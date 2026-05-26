//! Left dock panel — drawn only in Ide mode.
//!
//! Vertical 60/40 split: Explorer (top) and Outline (bottom).
//! v1: top-level dir listing only; no click handling, no scrolling.
//!
//! Section heights:
//!   explorer_h = rect.h * 0.60   (includes header row)
//!   outline_h  = rect.h * 0.40   (includes header row)

use std::collections::HashSet;
use std::path::Path;

use anvil_theme::Theme;
use anvil_workspace::layout::Rect;

use crate::raster::{FontMetrics, GlyphPainter, Raster};

#[derive(Debug, Clone, PartialEq)]
pub enum ExplorerHit {
    Header,
    Row(usize),
}

#[derive(Debug, Clone, PartialEq)]
pub enum LeftDockHitKind {
    Explorer(ExplorerHit),
    Outline(usize),
}

#[derive(Debug, Clone, PartialEq)]
pub struct LeftDockHit {
    pub rect: Rect,
    pub kind: LeftDockHitKind,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct LeftDockHits {
    pub hits: Vec<LeftDockHit>,
}

impl LeftDockHits {
    pub fn clear(&mut self) {
        self.hits.clear();
    }

    pub fn at(&self, x: f64, y: f64) -> Option<&LeftDockHitKind> {
        self.hits
            .iter()
            .find(|hit| {
                let r = hit.rect;
                x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h
            })
            .map(|hit| &hit.kind)
    }
}

/// A single directory entry as produced by `crates/anvil/src/fs_worker.rs`.
///
/// Duplicated here so `anvil-render` stays independent of the `anvil` binary
/// crate (crate graph constraint). The binary maps its own `DirEntry` →
/// this type by value (identical shape).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirEntry {
    pub name: String,
    pub is_dir: bool,
}

/// Snapshot of a directory's top-level entries.
#[derive(Debug, Clone, Default)]
pub struct DirSnapshot {
    /// The directory that was listed. Empty means "no cwd yet".
    pub root: String,
    pub entries: Vec<DirEntry>,
}

/// Render-side outline symbol kind.
///
/// Duplicated so `anvil-render` stays independent of `anvil-editor`.  The
/// nvim bridge that originally supplied this data was retired at NE15; the
/// caller now populates it from the native editor's LSP layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutlineKind {
    Function,
    Method,
    Class,
    Struct,
    Enum,
    Interface,
    Module,
    Property,
    Constant,
    Variable,
    Other,
}

/// A single row in the outline panel (render-side representation).
/// `anvil-render` keeps its own type so it does not depend on `anvil-editor`.
#[derive(Debug, Clone)]
pub struct OutlineRow {
    pub name: String,
    pub kind: OutlineKind,
    /// Nesting depth (0 = top-level). Used to compute left indent.
    pub depth: u8,
}

// ── Row geometry ──────────────────────────────────────────────────────────────

/// Height of a section header row in pixels (fixed; chrome font sized).
const HEADER_H: f64 = 28.0;

/// Height of a content row in pixels.
///
/// 22px matches the IDE redesign spec (§5, 2026-05-24-ide-redesign.md),
/// VS Code, and Zed — compact operational layout.
const ROW_H: f64 = 22.0;

/// Horizontal padding inside the dock.
const PAD_X: f64 = 10.0;

// ── Public entry point ────────────────────────────────────────────────────────

/// Draw the left dock into `rect`.
///
/// - Background: `theme.charcoal`.
/// - Right-edge 1px hairline: `theme.hairline`.
/// - 60/40 vertical split: Explorer (top) / Outline (bottom) with a hairline divider.
/// - `snapshot`: the latest directory listing; `None` means "waiting for cwd".
/// - `outline`: `None` = not yet ready (shows placeholder text); `Some(&[])` = no
///   symbols; `Some(rows)` = symbol list.
#[allow(clippy::too_many_arguments)]
pub fn draw_left_dock(
    raster: &mut Raster,
    painter: &mut dyn GlyphPainter,
    metrics: FontMetrics,
    theme: &Theme,
    snapshot: Option<&DirSnapshot>,
    active_file_path: Option<&Path>,
    outline: Option<&[OutlineRow]>,
    rect: Rect,
) -> LeftDockHits {
    draw_left_dock_with_scroll(
        raster,
        painter,
        metrics,
        theme,
        snapshot,
        active_file_path,
        outline,
        rect,
        0,
        None,
        &HashSet::new(),
        0.0,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn draw_left_dock_with_scroll(
    raster: &mut Raster,
    painter: &mut dyn GlyphPainter,
    metrics: FontMetrics,
    theme: &Theme,
    snapshot: Option<&DirSnapshot>,
    active_file_path: Option<&Path>,
    outline: Option<&[OutlineRow]>,
    rect: Rect,
    explorer_scroll_offset: usize,
    hovered_row: Option<usize>,
    expanded_dirs: &HashSet<usize>,
    scroll_indicator_alpha: f32,
) -> LeftDockHits {
    let mut hits = LeftDockHits::default();
    if rect.w <= 0.0 || rect.h <= 0.0 {
        return hits;
    }

    // ── Background ────────────────────────────────────────────────────────────
    // Direction A sidebar: a quiet Mineral panel, not a red/brown block. The
    // Ember wash is only a trace so Explorer reads as a tool rail beside the
    // editor instead of competing with the file contents.
    raster.fill_pixel_rect(rect.x, rect.y, rect.w, rect.h, theme.charcoal);
    raster.fill_pixel_rect_alpha(rect.x, rect.y, rect.w, rect.h, theme.accent_ember, 0.02);

    // Right-edge 1px warm hairline.
    raster.fill_pixel_rect_alpha(
        rect.x + rect.w - 1.0,
        rect.y,
        1.0,
        rect.h,
        theme.accent_bright,
        0.28,
    );

    // ── 60/40 split ───────────────────────────────────────────────────────────
    let explorer_h = (rect.h * 0.60).floor();
    let outline_h = rect.h - explorer_h;

    let explorer_rect = Rect {
        x: rect.x,
        y: rect.y,
        w: rect.w - 1.0,
        h: explorer_h,
    };
    let outline_rect = Rect {
        x: rect.x,
        y: rect.y + explorer_h,
        w: rect.w - 1.0,
        h: outline_h,
    };

    // Divider between sections.
    raster.fill_pixel_rect(
        rect.x,
        rect.y + explorer_h,
        rect.w - 1.0,
        1.0,
        theme.hairline,
    );

    let ui_metrics = metrics;
    draw_explorer_section(
        raster,
        painter,
        ui_metrics,
        theme,
        snapshot,
        active_file_path,
        explorer_scroll_offset,
        hovered_row,
        expanded_dirs,
        scroll_indicator_alpha,
        explorer_rect,
        &mut hits,
    );
    draw_outline_section(
        raster,
        painter,
        ui_metrics,
        theme,
        outline,
        outline_rect,
        &mut hits,
    );
    hits
}

// ── Explorer section ──────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn draw_explorer_section(
    raster: &mut Raster,
    painter: &mut dyn GlyphPainter,
    metrics: FontMetrics,
    theme: &Theme,
    snapshot: Option<&DirSnapshot>,
    active_file_path: Option<&Path>,
    scroll_offset: usize,
    hovered_row: Option<usize>,
    expanded_dirs: &HashSet<usize>,
    scroll_indicator_alpha: f32,
    rect: Rect,
    hits: &mut LeftDockHits,
) {
    let cell_w = metrics.cell_w;
    let cell_h = metrics.cell_h;

    // ── Header row ────────────────────────────────────────────────────────────
    hits.hits.push(LeftDockHit {
        rect: Rect {
            x: rect.x,
            y: rect.y,
            w: rect.w,
            h: HEADER_H.min(rect.h),
        },
        kind: LeftDockHitKind::Explorer(ExplorerHit::Header),
    });
    let (header_label, header_meta): (&str, String) = match snapshot {
        Some(snap) if !snap.root.is_empty() => {
            let basename = snap.root.rsplit('/').next().unwrap_or(&snap.root);
            ("EXPLORER", basename.to_string())
        }
        _ => ("EXPLORER", String::new()),
    };

    let header_y = rect.y + ((HEADER_H - cell_h) * 0.5 + metrics.descent * 0.5).max(0.0);
    draw_text_run(
        raster,
        painter,
        metrics,
        header_label,
        theme.accent_bright,
        rect.x + PAD_X,
        header_y,
        rect.x + rect.w - PAD_X,
    );
    if !header_meta.is_empty() {
        let meta_w = header_meta.chars().count() as f64 * cell_w;
        draw_text_run(
            raster,
            painter,
            metrics,
            &header_meta,
            theme.text_subtle,
            rect.x + rect.w - PAD_X - meta_w,
            header_y,
            rect.x + rect.w - PAD_X,
        );
    }
    // Hairline under header.
    raster.fill_pixel_rect(rect.x, rect.y + HEADER_H - 1.0, rect.w, 1.0, theme.hairline);

    // ── Content rows ──────────────────────────────────────────────────────────
    let content_y_start = rect.y + HEADER_H;
    let content_h = rect.h - HEADER_H;
    if content_h <= 0.0 {
        return;
    }

    match snapshot {
        None => {
            // No cwd yet — waiting state.
            let row_y = content_y_start + ((ROW_H - cell_h) * 0.5 + metrics.descent * 0.5).max(0.0);
            draw_text_run(
                raster,
                painter,
                metrics,
                "Waiting for shell prompt\u{2026}",
                theme.text_muted,
                rect.x + PAD_X,
                row_y,
                rect.x + rect.w,
            );
        }
        Some(snap) if snap.entries.is_empty() => {
            let row_y = content_y_start + ((ROW_H - cell_h) * 0.5 + metrics.descent * 0.5).max(0.0);
            draw_text_run(
                raster,
                painter,
                metrics,
                "(empty)",
                theme.text_muted,
                rect.x + PAD_X,
                row_y,
                rect.x + rect.w,
            );
        }
        Some(snap) => {
            let available_rows = (content_h / ROW_H).floor() as usize;
            let total_entries = snap.entries.len();
            let first = scroll_offset.min(total_entries.saturating_sub(available_rows));
            for (visible_i, entry) in snap
                .entries
                .iter()
                .enumerate()
                .skip(first)
                .take(available_rows)
            {
                let row_i = visible_i - first;
                let row_top = content_y_start + row_i as f64 * ROW_H;
                hits.hits.push(LeftDockHit {
                    rect: Rect {
                        x: rect.x,
                        y: row_top,
                        w: rect.w,
                        h: ROW_H.min((content_y_start + content_h - row_top).max(0.0)),
                    },
                    kind: LeftDockHitKind::Explorer(ExplorerHit::Row(visible_i)),
                });
                let glyph_y = row_top + ((ROW_H - cell_h) * 0.5 + metrics.descent * 0.5).max(0.0);

                let selected = !entry.is_dir && is_active_entry(snap, entry, active_file_path);
                let row_x = rect.x + 6.0;
                let row_w = (rect.w - 12.0).max(0.0);
                if selected {
                    // Solid panel fill + 2px accent_primary left rail.
                    raster.fill_pixel_rect(
                        row_x,
                        row_top + 2.0,
                        row_w,
                        (ROW_H - 4.0).max(0.0),
                        theme.panel,
                    );
                    raster.fill_pixel_rect(
                        row_x,
                        row_top + 2.0,
                        2.0,
                        (ROW_H - 4.0).max(0.0),
                        theme.accent_primary,
                    );
                } else if hovered_row == Some(visible_i) {
                    // Hover: solid panel fill, no left marker.
                    raster.fill_pixel_rect(
                        row_x,
                        row_top + 2.0,
                        row_w,
                        (ROW_H - 4.0).max(0.0),
                        theme.panel,
                    );
                }

                // Item 7: directory chevron toggles ▸/▾ based on expanded_dirs.
                let (icon, label, color) = if entry.is_dir {
                    let chevron = if expanded_dirs.contains(&visible_i) {
                        "▾"
                    } else {
                        "▸"
                    };
                    (chevron, entry.name.clone(), theme.text_subtle)
                } else {
                    (
                        "◇",
                        entry.name.clone(),
                        if selected {
                            theme.foreground
                        } else {
                            theme.text_muted
                        },
                    )
                };

                draw_text_run(
                    raster,
                    painter,
                    metrics,
                    icon,
                    if selected {
                        theme.accent_primary
                    } else if entry.is_dir {
                        theme.text_subtle
                    } else {
                        theme.hairline
                    },
                    rect.x + PAD_X,
                    glyph_y,
                    rect.x + PAD_X + cell_w,
                );

                // Truncate label to fit available width.
                let max_chars = ((rect.w - PAD_X * 2.0 - cell_w * 2.0) / cell_w).floor() as usize;
                let truncated = truncate_name(&label, max_chars);

                draw_text_run(
                    raster,
                    painter,
                    metrics,
                    &truncated,
                    color,
                    rect.x + PAD_X + cell_w * 2.0,
                    glyph_y,
                    rect.x + rect.w - PAD_X,
                );
            }

            // Item 8: scroll thumb — only when content overflows the dock.
            if total_entries > available_rows && scroll_indicator_alpha > 0.0 {
                let thumb_h = ((available_rows as f64 / total_entries as f64) * content_h)
                    .max(20.0)
                    .min(content_h);
                let max_scroll = (total_entries - available_rows) as f64;
                let thumb_top =
                    content_y_start + (first as f64 / max_scroll) * (content_h - thumb_h);
                let thumb_x = rect.x + rect.w - 3.0;
                raster.fill_pixel_rect_alpha(
                    thumb_x,
                    thumb_top,
                    3.0,
                    thumb_h,
                    theme.text_subtle,
                    (scroll_indicator_alpha * 0.6) as f64,
                );
            }
        }
    }
}

fn is_active_entry(snap: &DirSnapshot, entry: &DirEntry, active_file_path: Option<&Path>) -> bool {
    let Some(active) = active_file_path else {
        return false;
    };
    if active.file_name().and_then(|name| name.to_str()) != Some(entry.name.as_str()) {
        return false;
    }
    active.parent() == Some(Path::new(&snap.root))
}

// ── Outline section ───────────────────────────────────────────────────────────

fn draw_outline_section(
    raster: &mut Raster,
    painter: &mut dyn GlyphPainter,
    metrics: FontMetrics,
    theme: &Theme,
    outline: Option<&[OutlineRow]>,
    rect: Rect,
    hits: &mut LeftDockHits,
) {
    let cell_h = metrics.cell_h;
    let cell_w = metrics.cell_w;

    // ── Header row ────────────────────────────────────────────────────────────
    // Item 10: header color is text_subtle when empty, accent_bright when symbols present.
    let has_symbols = outline.is_some_and(|rows| !rows.is_empty());
    let header_color = if has_symbols {
        theme.accent_bright
    } else {
        theme.text_subtle
    };
    let header_y = rect.y + ((HEADER_H - cell_h) * 0.5 + metrics.descent * 0.5).max(0.0);
    draw_text_run(
        raster,
        painter,
        metrics,
        "OUTLINE",
        header_color,
        rect.x + PAD_X,
        header_y,
        rect.x + rect.w,
    );
    raster.fill_pixel_rect(rect.x, rect.y + HEADER_H - 1.0, rect.w, 1.0, theme.hairline);

    let content_y = rect.y + HEADER_H;
    let content_h = rect.h - HEADER_H;
    if content_h <= 0.0 {
        return;
    }

    match outline {
        // Item 10: None and Some(&[]) both collapse to header-only; no body copy.
        None | Some([]) => {
            // Empty state — only the header row is shown (rendered above).
        }
        Some(rows) => {
            let available_rows = (content_h / ROW_H).floor() as usize;
            for (i, row) in rows.iter().enumerate() {
                if i >= available_rows {
                    break;
                }
                let row_top = content_y + i as f64 * ROW_H;
                hits.hits.push(LeftDockHit {
                    rect: Rect {
                        x: rect.x,
                        y: row_top,
                        w: rect.w,
                        h: ROW_H.min((content_y + content_h - row_top).max(0.0)),
                    },
                    kind: LeftDockHitKind::Outline(i),
                });
                let glyph_y = row_top + ((ROW_H - cell_h) * 0.5 + metrics.descent * 0.5).max(0.0);

                // Indent: 2 cells per depth level.
                let indent_cells = row.depth as usize * 2;
                let indent_px = indent_cells as f64 * cell_w;
                let x_start = rect.x + PAD_X + indent_px;
                let x_max = rect.x + rect.w - PAD_X;

                // Kind glyph.
                let glyph = outline_kind_glyph(row.kind);
                draw_text_run(
                    raster,
                    painter,
                    metrics,
                    glyph,
                    theme.accent_primary,
                    x_start,
                    glyph_y,
                    x_max,
                );

                // Name: one cell after the glyph + one space gap.
                let name_x = x_start + cell_w * 2.0;
                let max_name_chars = ((x_max - name_x) / cell_w).floor().max(0.0) as usize;
                let truncated = truncate_name(&row.name, max_name_chars);
                draw_text_run(
                    raster,
                    painter,
                    metrics,
                    &truncated,
                    theme.text_muted,
                    name_x,
                    glyph_y,
                    x_max,
                );
            }
        }
    }
}

/// Return the single-character glyph string for a symbol kind.
fn outline_kind_glyph(kind: OutlineKind) -> &'static str {
    match kind {
        OutlineKind::Function | OutlineKind::Method => "\u{0192}", // ƒ
        OutlineKind::Class | OutlineKind::Struct | OutlineKind::Enum => "\u{25a2}", // ▢
        OutlineKind::Module => "\u{2699}",                         // ⚙
        _ => "\u{00b7}",                                           // ·
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Draw a string run clipped to `max_x`. Returns the x position after the last glyph.
#[allow(clippy::too_many_arguments)]
fn draw_text_run(
    raster: &mut Raster,
    painter: &mut dyn GlyphPainter,
    metrics: FontMetrics,
    text: &str,
    color: [u8; 3],
    x_start: f64,
    y: f64,
    max_x: f64,
) {
    let mut x = x_start;
    for ch in text.chars() {
        if x + metrics.cell_w > max_x {
            break;
        }
        raster.glyph_at(painter, metrics, x, y, ch as u32, color);
        x += metrics.cell_w;
    }
}

/// Truncate `name` to at most `max_chars` characters, appending `…` if clipped.
fn truncate_name(name: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let chars: Vec<char> = name.chars().collect();
    if chars.len() <= max_chars {
        name.to_string()
    } else {
        let cut = max_chars.saturating_sub(1);
        let s: String = chars[..cut].iter().collect();
        format!("{s}\u{2026}")
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raster::{PixelRect, pixel_at};

    #[derive(Default)]
    struct StubPainter {
        pub glyphs: Vec<(u32, [u8; 3])>,
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
            self.glyphs.push((glyph_id, fg));
        }
    }

    fn metrics() -> FontMetrics {
        FontMetrics {
            cell_w: 8.0,
            cell_h: 16.0,
            descent: 3.0,
        }
    }

    fn theme() -> Theme {
        anvil_theme::EMBER_DARK
    }

    fn dock_rect() -> Rect {
        Rect {
            x: 0.0,
            y: 0.0,
            w: 260.0,
            h: 800.0,
        }
    }

    /// Zero-size rect must not panic.
    #[test]
    fn zero_rect_no_panic() {
        let m = metrics();
        let th = theme();
        let mut r = Raster::new(800, 800);
        let mut p = StubPainter::default();
        let zero = Rect {
            x: 0.0,
            y: 0.0,
            w: 0.0,
            h: 0.0,
        };
        draw_left_dock(&mut r, &mut p, m, &th, None, None, None, zero);
        // No panic = pass.
    }

    /// No snapshot → "Waiting" text painted in text_muted.
    #[test]
    fn no_snapshot_waiting_text_painted() {
        let m = metrics();
        let th = theme();
        let mut r = Raster::new(800, 800);
        let mut p = StubPainter::default();

        draw_left_dock(&mut r, &mut p, m, &th, None, None, None, dock_rect());

        // "Waiting" → 'W' codepoint 87
        let waiting_w: Vec<_> = p
            .glyphs
            .iter()
            .filter(|(cp, fg)| *cp == 'W' as u32 && *fg == th.text_muted)
            .collect();
        assert!(
            !waiting_w.is_empty(),
            "expected 'W' in text_muted for Waiting state"
        );
    }

    /// Empty snapshot → "(empty)" row painted.
    #[test]
    fn empty_snapshot_empty_row_painted() {
        let m = metrics();
        let th = theme();
        let mut r = Raster::new(800, 800);
        let mut p = StubPainter::default();

        let snap = DirSnapshot {
            root: "/anvil".to_string(),
            entries: vec![],
        };
        draw_left_dock(&mut r, &mut p, m, &th, Some(&snap), None, None, dock_rect());

        // "(empty)" → '(' codepoint 40
        let paren: Vec<_> = p
            .glyphs
            .iter()
            .filter(|(cp, fg)| *cp == '(' as u32 && *fg == th.text_muted)
            .collect();
        assert!(
            !paren.is_empty(),
            "expected '(' in text_muted for empty state"
        );
    }

    /// Snapshot with entries → one header hit + one row hit per visible entry.
    #[test]
    fn explorer_rows_return_click_hits() {
        let m = metrics();
        let th = theme();
        let mut r = Raster::new(800, 800);
        let mut p = StubPainter::default();

        let snap = DirSnapshot {
            root: "/anvil".to_string(),
            entries: vec![
                DirEntry {
                    name: "src".to_string(),
                    is_dir: true,
                },
                DirEntry {
                    name: "main.rs".to_string(),
                    is_dir: false,
                },
            ],
        };

        let hits = draw_left_dock(&mut r, &mut p, m, &th, Some(&snap), None, None, dock_rect());

        assert_eq!(
            hits.at(12.0, 18.0),
            Some(&LeftDockHitKind::Explorer(ExplorerHit::Header))
        );
        assert_eq!(
            hits.at(12.0, 36.0),
            Some(&LeftDockHitKind::Explorer(ExplorerHit::Row(0)))
        );
        assert_eq!(
            hits.at(12.0, 68.0),
            Some(&LeftDockHitKind::Explorer(ExplorerHit::Row(1)))
        );
    }

    #[test]
    fn explorer_rows_have_mouse_sized_full_width_targets() {
        let m = metrics();
        let th = theme();
        let mut r = Raster::new(800, 800);
        let mut p = StubPainter::default();

        let snap = DirSnapshot {
            root: "/anvil".to_string(),
            entries: vec![DirEntry {
                name: "main.rs".to_string(),
                is_dir: false,
            }],
        };

        let hits = draw_left_dock(&mut r, &mut p, m, &th, Some(&snap), None, None, dock_rect());
        let row_hit = hits
            .hits
            .iter()
            .find(|hit| hit.kind == LeftDockHitKind::Explorer(ExplorerHit::Row(0)))
            .expect("row hit region should be emitted for visible explorer row");

        assert!(
            row_hit.rect.h >= 20.0,
            "explorer rows must be easy mouse targets, got height {}",
            row_hit.rect.h
        );
        assert!(
            row_hit.rect.w >= dock_rect().w - 1.0,
            "explorer row hit should span the full dock width"
        );
        assert_eq!(
            hits.at(
                row_hit.rect.x + row_hit.rect.w - 2.0,
                row_hit.rect.y + row_hit.rect.h / 2.0
            ),
            Some(&LeftDockHitKind::Explorer(ExplorerHit::Row(0))),
            "right side of row should be clickable, not just the label"
        );
    }

    /// Snapshot with entries → file names appear in text_muted, dirs in text_subtle.
    #[test]
    fn explorer_scroll_offset_preserves_original_row_indices() {
        let m = metrics();
        let th = theme();
        let mut r = Raster::new(800, 800);
        let mut p = StubPainter::default();

        // Use 25 entries: with ROW_H=22 and explorer_h=480px, available_rows=20,
        // so 25 entries give 5 overflow rows and scroll_offset=1 is not clamped.
        // Entry[1] appears in visible slot 0 at y=[28, 50). Hit at y=36 → Row(1).
        let snap = DirSnapshot {
            root: "/anvil".to_string(),
            entries: (0..25)
                .map(|i| DirEntry {
                    name: format!("file-{i}.rs"),
                    is_dir: false,
                })
                .collect(),
        };

        let hits = draw_left_dock_with_scroll(
            &mut r,
            &mut p,
            m,
            &th,
            Some(&snap),
            None,
            None,
            dock_rect(),
            1,
            None,
            &HashSet::new(),
            0.0,
        );

        assert_eq!(
            hits.at(12.0, 36.0),
            Some(&LeftDockHitKind::Explorer(ExplorerHit::Row(1)))
        );
    }

    #[test]
    fn entries_rendered_with_correct_colors() {
        let m = metrics();
        let th = theme();
        let mut r = Raster::new(800, 800);
        let mut p = StubPainter::default();

        let snap = DirSnapshot {
            root: "/anvil".to_string(),
            entries: vec![
                DirEntry {
                    name: "src".to_string(),
                    is_dir: true,
                },
                DirEntry {
                    name: "main.rs".to_string(),
                    is_dir: false,
                },
            ],
        };
        draw_left_dock(&mut r, &mut p, m, &th, Some(&snap), None, None, dock_rect());

        // File entry 'm' should appear in text_muted.
        let file_m: Vec<_> = p
            .glyphs
            .iter()
            .filter(|(cp, fg)| *cp == 'm' as u32 && *fg == th.text_muted)
            .collect();
        assert!(
            !file_m.is_empty(),
            "expected 'm' in text_muted for file entry"
        );

        // Dir entry chevron ▸ (U+25B8) should appear in text_subtle.
        let dir_chevron: Vec<_> = p
            .glyphs
            .iter()
            .filter(|(cp, fg)| *cp == '\u{25B8}' as u32 && *fg == th.text_subtle)
            .collect();
        assert!(
            !dir_chevron.is_empty(),
            "expected dir chevron in text_subtle for dir entry"
        );
    }

    #[test]
    fn active_file_path_marks_matching_explorer_row_selected() {
        let m = metrics();
        let th = theme();
        let mut r = Raster::new(800, 800);
        let mut p = StubPainter::default();

        let snap = DirSnapshot {
            root: "/anvil/src".to_string(),
            entries: vec![
                DirEntry {
                    name: "editor.rs".to_string(),
                    is_dir: false,
                },
                DirEntry {
                    name: "main.rs".to_string(),
                    is_dir: false,
                },
            ],
        };

        draw_left_dock(
            &mut r,
            &mut p,
            m,
            &th,
            Some(&snap),
            Some(Path::new("/anvil/src/main.rs")),
            None,
            dock_rect(),
        );

        let main_m_selected: Vec<_> = p
            .glyphs
            .iter()
            .filter(|(cp, fg)| *cp == 'm' as u32 && *fg == th.foreground)
            .collect();
        assert!(
            !main_m_selected.is_empty(),
            "expected active file label to use foreground selected color"
        );

        let editor_e_selected: Vec<_> = p
            .glyphs
            .iter()
            .filter(|(cp, fg)| *cp == 'e' as u32 && *fg == th.foreground)
            .collect();
        assert!(
            editor_e_selected.is_empty(),
            "selection must come from active_file_path, not a hardcoded editor.rs row"
        );
    }

    /// Hover paints solid `panel` only when the row is not selected.
    /// Selected row suppresses hover rendering.
    #[test]
    fn hover_paints_panel_only_when_not_selected() {
        let m = metrics();
        let th = theme();

        let snap = DirSnapshot {
            root: "/anvil/src".to_string(),
            entries: vec![
                DirEntry {
                    name: "foo.rs".to_string(),
                    is_dir: false,
                },
                DirEntry {
                    name: "bar.rs".to_string(),
                    is_dir: false,
                },
            ],
        };

        // Row 0 is hovered, not selected — should get panel fill.
        {
            let mut r = Raster::new(800, 800);
            r.clear(th.charcoal);
            let mut p = StubPainter::default();
            draw_left_dock_with_scroll(
                &mut r,
                &mut p,
                m,
                &th,
                Some(&snap),
                None, // no active file
                None,
                dock_rect(),
                0,
                Some(0), // hover row 0
                &HashSet::new(),
                0.0,
            );
            // Row 0 occupies y=[HEADER_H, HEADER_H+ROW_H) = [28, 50).
            // The fill rect is row_top+2 .. row_top+ROW_H-2 = [30, 48).
            // Sample the fill interior: x=50, y=38 (middle of fill strip).
            let px = pixel_at(&r, 50, 38);
            assert_eq!(
                px, th.panel,
                "hovered non-selected row must be filled with panel, got {px:?}"
            );
        }

        // Row 0 is both hovered AND selected — selected wins, no plain panel hover.
        // Selected fill is also panel, but with a 2px accent_primary left rail.
        // The left-rail pixel (x=6+1=7) should be accent_primary, not just panel.
        {
            let mut r = Raster::new(800, 800);
            r.clear(th.charcoal);
            let mut p = StubPainter::default();
            draw_left_dock_with_scroll(
                &mut r,
                &mut p,
                m,
                &th,
                Some(&snap),
                Some(Path::new("/anvil/src/foo.rs")), // foo.rs is selected (row 0)
                None,
                dock_rect(),
                0,
                Some(0), // hover row 0 as well
                &HashSet::new(),
                0.0,
            );
            // Left-rail pixel (x=rect.x+6=6, inside 2px rail) should be accent_primary.
            // Row 0 fill starts at row_top+2 = 28+2 = 30. Sample y=38.
            let rail_px = pixel_at(&r, 6, 38);
            assert_eq!(
                rail_px, th.accent_primary,
                "selected row must show accent_primary left rail even when also hovered, got {rail_px:?}"
            );
        }
    }

    /// Outline section with `None` shows only the header row (no body copy).
    #[test]
    fn outline_unavailable_always_shown() {
        let m = metrics();
        let th = theme();
        let mut r = Raster::new(800, 800);
        let mut p = StubPainter::default();

        draw_left_dock(&mut r, &mut p, m, &th, None, None, None, dock_rect());

        // "OUTLINE" header 'N' rendered in text_subtle (Item 10: empty → text_subtle).
        let n_subtle: Vec<_> = p
            .glyphs
            .iter()
            .filter(|(cp, fg)| *cp == 'N' as u32 && *fg == th.text_subtle)
            .collect();
        assert!(
            !n_subtle.is_empty(),
            "expected 'N' in text_subtle for outline header in empty state"
        );

        // Item 10: body copy removed. 'c' from "sour[c]e" (in "Open a source file") must
        // not appear — 'c' is absent from all other rendered strings ("OUTLINE", "EXPLORER",
        // "Waiting for shell prompt…").
        let c_body: Vec<_> = p
            .glyphs
            .iter()
            .filter(|(cp, _)| *cp == 'c' as u32)
            .collect();
        assert!(
            c_body.is_empty(),
            "body copy 'Open a source file' must not render when outline is None"
        );
    }

    /// Background is a neutral Mineral panel with only a trace Ember wash.
    #[test]
    fn background_is_quiet_mineral_sidebar() {
        let m = metrics();
        let th = theme();
        let mut r = Raster::new(800, 800);
        r.clear([0, 0, 0]);
        let mut p = StubPainter::default();

        draw_left_dock(&mut r, &mut p, m, &th, None, None, None, dock_rect());

        let px = pixel_at(&r, 50, 400); // middle of dock
        assert_ne!(
            px,
            [27, 18, 13],
            "sidebar must not regress to red/brown block"
        );
        assert!(
            px[0] >= th.charcoal[0],
            "trace wash should lift charcoal slightly"
        );
        assert!(
            px[0].saturating_sub(px[2]) <= 12,
            "sidebar should stay neutral, not heavily red-biased: {px:?}"
        );
    }

    /// `Some(&[])` → only the header row, no body copy.
    #[test]
    fn left_dock_renders_outline_no_symbols() {
        let m = metrics();
        let th = theme();
        let mut r = Raster::new(800, 800);
        let mut p = StubPainter::default();

        draw_left_dock(&mut r, &mut p, m, &th, None, None, Some(&[]), dock_rect());

        // "OUTLINE" header 'N' rendered in text_subtle (Item 10: empty → text_subtle).
        let n_subtle: Vec<_> = p
            .glyphs
            .iter()
            .filter(|(cp, fg)| *cp == 'N' as u32 && *fg == th.text_subtle)
            .collect();
        assert!(
            !n_subtle.is_empty(),
            "expected 'N' in text_subtle for outline header when Some(&[])"
        );

        // No body copy: 's' from "No symbols" must not appear.
        // 's' appears in "src" (Explorer) but not in OUTLINE body when empty.
        // More precisely, 'y' only appears in "No symbols" and "No symbols yet".
        let y_body: Vec<_> = p
            .glyphs
            .iter()
            .filter(|(cp, _)| *cp == 'y' as u32)
            .collect();
        assert!(
            y_body.is_empty(),
            "body copy must not render when outline is Some(&[])"
        );
    }

    /// `Some(rows)` → symbol names painted in text_muted.
    #[test]
    fn left_dock_renders_outline_with_rows() {
        let m = metrics();
        let th = theme();
        let mut r = Raster::new(800, 800);
        let mut p = StubPainter::default();

        let rows = vec![
            OutlineRow {
                name: "my_fn".to_string(),
                kind: OutlineKind::Function,
                depth: 0,
            },
            OutlineRow {
                name: "MyStruct".to_string(),
                kind: OutlineKind::Struct,
                depth: 0,
            },
        ];
        draw_left_dock(&mut r, &mut p, m, &th, None, None, Some(&rows), dock_rect());

        // 'm' from "my_fn" should appear in text_muted.
        let m_muted: Vec<_> = p
            .glyphs
            .iter()
            .filter(|(cp, fg)| *cp == 'm' as u32 && *fg == th.text_muted)
            .collect();
        assert!(
            !m_muted.is_empty(),
            "expected 'm' in text_muted for function symbol name"
        );

        // ƒ glyph (0x0192) should appear in accent_primary.
        let f_accent: Vec<_> = p
            .glyphs
            .iter()
            .filter(|(cp, fg)| *cp == '\u{0192}' as u32 && *fg == th.accent_primary)
            .collect();
        assert!(
            !f_accent.is_empty(),
            "expected ƒ glyph in accent_primary for function kind"
        );
    }

    /// Item 10: OUTLINE header uses text_subtle (not accent_bright) when outline is None.
    /// Uses 'U' which appears only in "OUTLINE" (not in "EXPLORER" or waiting text).
    #[test]
    fn outline_empty_header_uses_text_subtle() {
        let m = metrics();
        let th = theme();
        let mut r = Raster::new(800, 800);
        let mut p = StubPainter::default();

        draw_left_dock(&mut r, &mut p, m, &th, None, None, None, dock_rect());

        // 'U' from "OUTLINE" — unique to that header — must appear in text_subtle.
        let u_subtle: Vec<_> = p
            .glyphs
            .iter()
            .filter(|(cp, fg)| *cp == 'U' as u32 && *fg == th.text_subtle)
            .collect();
        assert!(
            !u_subtle.is_empty(),
            "OUTLINE header 'U' must be in text_subtle when outline is None"
        );

        // 'U' must NOT appear in accent_bright (that would mean the wrong color was used).
        let u_bright: Vec<_> = p
            .glyphs
            .iter()
            .filter(|(cp, fg)| *cp == 'U' as u32 && *fg == th.accent_bright)
            .collect();
        assert!(
            u_bright.is_empty(),
            "OUTLINE header must NOT use accent_bright when outline is None"
        );
    }

    /// Item 9: overflow smoke test — scroll offset changes which entries are rendered.
    ///
    /// Uses hit-target indices as the proxy for "which entry is at the first visible row".
    /// At offset=0 the first content row maps to entry 0; at offset=5 it maps to entry 5.
    #[test]
    fn overflow_scroll_changes_rendered_entries() {
        let m = metrics();
        let th = theme();
        // dock_rect height 800px: explorer_h = 480px, HEADER_H=28px, content_h = 452px.
        // available_rows = floor(452/22) = 20.
        // Use 30 entries to guarantee overflow at this height.
        let snap = DirSnapshot {
            root: "/anvil".to_string(),
            entries: (0..30)
                .map(|i| DirEntry {
                    name: format!("f{i}.rs"),
                    is_dir: false,
                })
                .collect(),
        };

        // At offset=0: first content row (y ≈ HEADER_H + ROW_H/2 = 28+11 = 39) → entry 0.
        let hits_at_0 = {
            let mut r = Raster::new(800, 800);
            let mut p = StubPainter::default();
            draw_left_dock_with_scroll(
                &mut r,
                &mut p,
                m,
                &th,
                Some(&snap),
                None,
                None,
                dock_rect(),
                0,
                None,
                &HashSet::new(),
                0.0,
            )
        };

        // At offset=5: same pixel maps to entry 5.
        let hits_at_5 = {
            let mut r = Raster::new(800, 800);
            let mut p = StubPainter::default();
            draw_left_dock_with_scroll(
                &mut r,
                &mut p,
                m,
                &th,
                Some(&snap),
                None,
                None,
                dock_rect(),
                5,
                None,
                &HashSet::new(),
                0.0,
            )
        };

        // First content row: y = HEADER_H + ROW_H/2 ≈ 28 + 11 = 39.
        let first_row_y = 39.0_f64;
        let hit_at_0 = hits_at_0.at(50.0, first_row_y);
        let hit_at_5 = hits_at_5.at(50.0, first_row_y);

        assert_eq!(
            hit_at_0,
            Some(&LeftDockHitKind::Explorer(ExplorerHit::Row(0))),
            "at scroll_offset=0 the first content row must map to entry 0"
        );
        assert_eq!(
            hit_at_5,
            Some(&LeftDockHitKind::Explorer(ExplorerHit::Row(5))),
            "at scroll_offset=5 the first content row must map to entry 5"
        );

        // The two hit results must differ — scroll actually changed what is rendered.
        assert_ne!(
            hit_at_0, hit_at_5,
            "scroll offset must change which entry is rendered at a given pixel row"
        );
    }
}
