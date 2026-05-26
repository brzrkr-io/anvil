//! Left dock panel — drawn only in Ide mode.
//!
//! Vertical 60/40 split: Explorer (top) and Outline (bottom).
//! v1: top-level dir listing only; no click handling, no scrolling.
//! v2 (item 7): nested directory expansion; `expanded_dirs` keyed by absolute
//!    path; `child_snapshots` holds per-directory listings loaded on demand.
//!
//! Section heights:
//!   explorer_h = rect.h * 0.60   (includes header row)
//!   outline_h  = rect.h * 0.40   (includes header row)

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use anvil_theme::Theme;
use anvil_workspace::layout::Rect;

use crate::raster::{FontMetrics, GlyphPainter, Raster};

#[derive(Debug, Clone, PartialEq)]
pub enum ExplorerHit {
    Header,
    /// Visible row index (0-based, across all rendered rows including nested
    /// children). Look up the absolute path and is_dir flag in
    /// [`LeftDockHits::visible_rows`].
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

/// Hit map returned by [`draw_left_dock_with_scroll`].
///
/// `hits` is the spatial hit-test list; `visible_rows` is a parallel list
/// mapping each `ExplorerHit::Row(i)` → `(absolute_path, is_dir)` so the
/// caller can dispatch open/toggle by path without re-walking the tree.
#[derive(Debug, Clone, Default)]
pub struct LeftDockHits {
    pub hits: Vec<LeftDockHit>,
    /// Parallel to `ExplorerHit::Row(i)` — maps the visible row index to its
    /// absolute path and is_dir flag.  Index 0 = first content row rendered.
    pub visible_rows: Vec<(PathBuf, bool)>,
}

impl LeftDockHits {
    pub fn clear(&mut self) {
        self.hits.clear();
        self.visible_rows.clear();
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
    /// Git status badges per filename (item 10).
    ///
    /// Key: filename (basename only, not full path).
    /// Value: `'M'` modified, `'A'` added, `'?'` untracked, `'D'` deleted.
    /// Empty when not in a git repo or git is unavailable.
    pub git_marks: std::collections::HashMap<String, char>,
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
    /// 0-based buffer line this symbol starts on. Used by outline-click to jump cursor.
    pub line: usize,
}

// ── Row geometry ──────────────────────────────────────────────────────────────

/// Base height of a section header row in points (multiplied by `ui_scale`).
const HEADER_H_BASE: f64 = 32.0;

/// Base height of a content row in points (multiplied by `ui_scale`).
const ROW_H_BASE: f64 = 28.0;

/// Base horizontal padding inside the dock (multiplied by `ui_scale`).
const PAD_X_BASE: f64 = 14.0;

/// Base indent per depth level in device pixels (multiplied by `ui_scale`).
const INDENT_PX_BASE: f64 = 16.0;

/// Derive row geometry from the current UI scale.
struct RowMetrics {
    header_h: f64,
    row_h: f64,
    pad_x: f64,
    indent_px: f64,
}

impl RowMetrics {
    fn from_scale(ui_scale: f64) -> Self {
        Self {
            header_h: (HEADER_H_BASE * ui_scale).round(),
            row_h: (ROW_H_BASE * ui_scale).round(),
            pad_x: (PAD_X_BASE * ui_scale).round(),
            indent_px: (INDENT_PX_BASE * ui_scale).round(),
        }
    }
}

// ── Public entry point ────────────────────────────────────────────────────────

/// Draw the left dock into `rect`.
///
/// - Background: `theme.charcoal`.
/// - Right-edge 1px hairline: `theme.hairline`.
/// - 60/40 vertical split: Explorer (top) / Outline (bottom) with a hairline divider.
/// - `snapshot`: the latest directory listing; `None` means "waiting for cwd".
/// - `outline`: `None` = not yet ready (shows placeholder text); `Some(&[])` = no
///   symbols; `Some(rows)` = symbol list.
///
/// This simplified overload passes empty expansion state and is used by unit
/// tests that only care about the flat listing.
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
        &HashMap::new(),
        0.0,
        1.0,
        None,
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
    expanded_dirs: &HashSet<PathBuf>,
    child_snapshots: &HashMap<PathBuf, DirSnapshot>,
    scroll_indicator_alpha: f32,
    ui_scale: f64,
    // R3: active filter string; `None` = no filter.
    explorer_filter: Option<&str>,
) -> LeftDockHits {
    let mut hits = LeftDockHits::default();
    if rect.w <= 0.0 || rect.h <= 0.0 {
        return hits;
    }

    // G1: icon-only mode when dock is narrow; hide entirely when very narrow.
    let icon_only_threshold = 120.0 * ui_scale;
    let hide_threshold = 60.0 * ui_scale;
    if rect.w < hide_threshold {
        return hits;
    }
    if rect.w < icon_only_threshold {
        return draw_left_dock_icons_only(
            raster,
            painter,
            metrics,
            theme,
            snapshot,
            active_file_path,
            explorer_scroll_offset,
            hovered_row,
            expanded_dirs,
            child_snapshots,
            rect,
            &mut hits,
            ui_scale,
        );
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
    let rm = RowMetrics::from_scale(ui_scale);
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
        child_snapshots,
        scroll_indicator_alpha,
        explorer_rect,
        &mut hits,
        &rm,
        explorer_filter,
    );
    draw_outline_section(
        raster,
        painter,
        ui_metrics,
        theme,
        outline,
        outline_rect,
        &mut hits,
        &rm,
    );
    hits
}

// ── Icons-only mode (G1) ──────────────────────────────────────────────────────

/// Width of the icons-only dock in logical points (before ui_scale).
const ICONS_ONLY_W_BASE: f64 = 48.0;

/// Draw a slim icon-only sidebar: one file/dir icon per row, no labels.
/// Hit map is populated so click-to-open/toggle still works.
#[allow(clippy::too_many_arguments)]
fn draw_left_dock_icons_only(
    raster: &mut Raster,
    painter: &mut dyn GlyphPainter,
    metrics: FontMetrics,
    theme: &Theme,
    snapshot: Option<&DirSnapshot>,
    active_file_path: Option<&Path>,
    scroll_offset: usize,
    hovered_row: Option<usize>,
    expanded_dirs: &HashSet<PathBuf>,
    child_snapshots: &HashMap<PathBuf, DirSnapshot>,
    rect: Rect,
    hits: &mut LeftDockHits,
    ui_scale: f64,
) -> LeftDockHits {
    let rm = RowMetrics::from_scale(ui_scale);
    let slim_w = (ICONS_ONLY_W_BASE * ui_scale).min(rect.w);

    // Background.
    raster.fill_pixel_rect(rect.x, rect.y, slim_w, rect.h, theme.charcoal);
    raster.fill_pixel_rect_alpha(rect.x, rect.y, slim_w, rect.h, theme.accent_ember, 0.02);
    // Right-edge hairline.
    raster.fill_pixel_rect_alpha(
        rect.x + slim_w - 1.0,
        rect.y,
        1.0,
        rect.h,
        theme.accent_bright,
        0.28,
    );

    let snap = match snapshot {
        Some(s) if !s.entries.is_empty() => s,
        _ => return std::mem::take(hits),
    };

    let mut all_rows: Vec<(PathBuf, bool, usize)> = Vec::new();
    collect_visible_rows(
        snap,
        &PathBuf::from(&snap.root),
        0,
        expanded_dirs,
        child_snapshots,
        &mut all_rows,
    );

    // Populate visible_rows (stable index for hit dispatch).
    for (path, is_dir, _) in &all_rows {
        hits.visible_rows.push((path.clone(), *is_dir));
    }

    let row_h = rm.row_h;
    let available_rows = (rect.h / row_h).floor() as usize;
    let total_rows = all_rows.len();
    let first = scroll_offset.min(total_rows.saturating_sub(available_rows));
    let cell_w = metrics.cell_w;
    let cell_h = metrics.cell_h;

    for (slot_i, (path, is_dir, _depth)) in
        all_rows.iter().enumerate().skip(first).take(available_rows)
    {
        let row_i = slot_i - first;
        let row_top = rect.y + row_i as f64 * row_h;

        hits.hits.push(LeftDockHit {
            rect: Rect {
                x: rect.x,
                y: row_top,
                w: slim_w,
                h: row_h.min((rect.y + rect.h - row_top).max(0.0)),
            },
            kind: LeftDockHitKind::Explorer(ExplorerHit::Row(slot_i)),
        });

        let glyph_y = row_top + ((row_h - cell_h) * 0.5 + metrics.descent * 0.5).max(0.0);
        let icon_x = rect.x + (slim_w - cell_w).max(0.0) * 0.5;

        let selected = !is_dir && active_file_path == Some(path.as_path());
        let hovered = hovered_row == Some(slot_i);
        let row_x = rect.x + 2.0;
        let row_w = (slim_w - 4.0).max(0.0);
        if selected {
            raster.fill_pixel_rect(
                row_x,
                row_top + 2.0,
                row_w,
                (row_h - 4.0).max(0.0),
                theme.panel,
            );
            raster.fill_pixel_rect(
                row_x,
                row_top + 2.0,
                2.0,
                (row_h - 4.0).max(0.0),
                theme.accent_primary,
            );
        } else if hovered {
            raster.fill_pixel_rect(
                row_x,
                row_top + 2.0,
                row_w,
                (row_h - 4.0).max(0.0),
                theme.panel,
            );
        }

        let (icon_ch, icon_color) = if *is_dir {
            let ch = if expanded_dirs.contains(path) {
                '▾'
            } else {
                '▸'
            };
            (ch, theme.foreground)
        } else {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            let (ch, col) = file_icon_colored(name, theme);
            (ch, if selected { theme.accent_primary } else { col })
        };

        raster.glyph_at(
            painter,
            metrics,
            icon_x,
            glyph_y,
            icon_ch as u32,
            icon_color,
        );
    }

    std::mem::take(hits)
}

// ── Explorer section ──────────────────────────────────────────────────────────

/// Maximum nesting depth to render. Guards against pathological trees blowing
/// up the render loop.
const MAX_RENDER_DEPTH: usize = 32;

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
    expanded_dirs: &HashSet<PathBuf>,
    child_snapshots: &HashMap<PathBuf, DirSnapshot>,
    scroll_indicator_alpha: f32,
    rect: Rect,
    hits: &mut LeftDockHits,
    rm: &RowMetrics,
    explorer_filter: Option<&str>,
) {
    let cell_w = metrics.cell_w;
    let cell_h = metrics.cell_h;
    let header_h = rm.header_h;
    let row_h = rm.row_h;
    let pad_x = rm.pad_x;
    let indent_px = rm.indent_px;

    // ── Header row ────────────────────────────────────────────────────────────
    hits.hits.push(LeftDockHit {
        rect: Rect {
            x: rect.x,
            y: rect.y,
            w: rect.w,
            h: header_h.min(rect.h),
        },
        kind: LeftDockHitKind::Explorer(ExplorerHit::Header),
    });
    let (header_label, header_meta): (&str, String) = match (explorer_filter, snapshot) {
        (Some(f), _) => ("EXPLORER", format!("[{f}]")),
        (None, Some(snap)) if !snap.root.is_empty() => {
            let basename = snap.root.rsplit('/').next().unwrap_or(&snap.root);
            ("EXPLORER", basename.to_string())
        }
        _ => ("EXPLORER", String::new()),
    };
    // R3: filter chip color — highlight the meta in accent when filter is active.
    let meta_color = if explorer_filter.is_some() {
        theme.accent_bright
    } else {
        theme.text_subtle
    };

    let header_y = rect.y + ((header_h - cell_h) * 0.5 + metrics.descent * 0.5).max(0.0);
    draw_text_run(
        raster,
        painter,
        metrics,
        header_label,
        theme.accent_bright,
        rect.x + pad_x,
        header_y,
        rect.x + rect.w - pad_x,
    );
    if !header_meta.is_empty() {
        let meta_w = header_meta.chars().count() as f64 * cell_w;
        draw_text_run(
            raster,
            painter,
            metrics,
            &header_meta,
            meta_color,
            rect.x + rect.w - pad_x - meta_w,
            header_y,
            rect.x + rect.w - pad_x,
        );
    }
    // Hairline under header.
    raster.fill_pixel_rect(rect.x, rect.y + header_h - 1.0, rect.w, 1.0, theme.hairline);

    // ── Content rows ──────────────────────────────────────────────────────────
    let content_y_start = rect.y + header_h;
    let content_h = rect.h - header_h;
    if content_h <= 0.0 {
        return;
    }

    match snapshot {
        None => {
            // No cwd yet — waiting state.
            let row_y = content_y_start + ((row_h - cell_h) * 0.5 + metrics.descent * 0.5).max(0.0);
            draw_text_run(
                raster,
                painter,
                metrics,
                "Waiting for shell prompt\u{2026}",
                theme.text_muted,
                rect.x + pad_x,
                row_y,
                rect.x + rect.w,
            );
        }
        Some(snap) if snap.entries.is_empty() => {
            let row_y = content_y_start + ((row_h - cell_h) * 0.5 + metrics.descent * 0.5).max(0.0);
            draw_text_run(
                raster,
                painter,
                metrics,
                "(empty)",
                theme.text_muted,
                rect.x + pad_x,
                row_y,
                rect.x + rect.w,
            );
        }
        Some(snap) => {
            // Build the flat ordered list of all visible rows by walking the
            // tree top-down: root entries, then recursively expanded children.
            // Also collect last-sibling flags per row for branch glyph rendering.
            let mut all_rows: Vec<(PathBuf, bool, usize)> = Vec::new(); // (path, is_dir, depth)
            // R3: when a filter is active, expand ALL directories so we can search
            // across the full tree. Use a temporary set that contains every dir path.
            let filter_expanded: HashSet<PathBuf>;
            let effective_expanded: &HashSet<PathBuf> = if explorer_filter.is_some() {
                filter_expanded = collect_all_dirs(snap, child_snapshots);
                &filter_expanded
            } else {
                expanded_dirs
            };
            collect_visible_rows(
                snap,
                &PathBuf::from(&snap.root),
                0,
                effective_expanded,
                child_snapshots,
                &mut all_rows,
            );
            // R3: filter rows — keep file rows whose basename matches and dir rows
            // that are ancestors of at least one matching file.
            if let Some(f) = explorer_filter {
                let f_low = f.to_lowercase();
                // Determine which file rows match.
                let file_matches: HashSet<usize> = all_rows
                    .iter()
                    .enumerate()
                    .filter(|(_, (p, is_dir, _))| {
                        !is_dir
                            && p.file_name()
                                .and_then(|n| n.to_str())
                                .map(|n| n.to_lowercase().contains(&f_low))
                                .unwrap_or(false)
                    })
                    .map(|(i, _)| i)
                    .collect();
                // Build the set of ancestor paths of matching rows.
                let mut keep_paths: HashSet<PathBuf> = HashSet::new();
                for i in &file_matches {
                    let path = &all_rows[*i].0;
                    keep_paths.insert(path.clone());
                    let mut p = path.as_path();
                    while let Some(par) = p.parent() {
                        keep_paths.insert(par.to_path_buf());
                        p = par;
                    }
                }
                all_rows.retain(|(p, _, _)| keep_paths.contains(p));
            }

            // Compute is_last_sibling per row at each depth level.
            // Row i is the last sibling at depth d when no later row at the same
            // depth has an ancestor path that matches row i's ancestor at depth d.
            let row_count = all_rows.len();
            let mut is_last_at_depth: Vec<Vec<bool>> = Vec::with_capacity(row_count);
            for i in 0..row_count {
                let depth_i = all_rows[i].2;
                let path_i = &all_rows[i].0;
                // For each depth level 0..=depth_i, determine is-last-sibling.
                let flags: Vec<bool> = (0..=depth_i)
                    .map(|d| {
                        // Ancestor of path_i at depth d: strip (depth_i - d) trailing components.
                        let strip = depth_i - d;
                        let anc_i: PathBuf = path_i
                            .ancestors()
                            .nth(strip)
                            .unwrap_or(path_i.as_path())
                            .to_path_buf();
                        // is_last: no later row at depth d has anc_i as ancestor.
                        !all_rows[i + 1..].iter().any(|(p, _, dd)| {
                            *dd == d && {
                                let anc_p: PathBuf = p
                                    .ancestors()
                                    .nth(depth_i - d)
                                    .unwrap_or(p.as_path())
                                    .to_path_buf();
                                anc_p == anc_i
                            }
                        })
                    })
                    .collect();
                is_last_at_depth.push(flags);
            }

            let available_rows = (content_h / row_h).floor() as usize;
            let total_rows = all_rows.len();
            let first = scroll_offset.min(total_rows.saturating_sub(available_rows));

            for (slot_i, (path, is_dir, depth)) in
                all_rows.iter().enumerate().skip(first).take(available_rows)
            {
                // slot_i is the absolute index in `all_rows`; row_i is the screen slot.
                let row_i = slot_i - first;
                let row_top = content_y_start + row_i as f64 * row_h;

                // The visible row index reported in the hit is `slot_i` so that
                // `visible_rows[slot_i]` gives back the path.
                hits.hits.push(LeftDockHit {
                    rect: Rect {
                        x: rect.x,
                        y: row_top,
                        w: rect.w,
                        h: row_h.min((content_y_start + content_h - row_top).max(0.0)),
                    },
                    kind: LeftDockHitKind::Explorer(ExplorerHit::Row(slot_i)),
                });

                let glyph_y = row_top + ((row_h - cell_h) * 0.5 + metrics.descent * 0.5).max(0.0);

                let selected = !is_dir && active_file_path == Some(path.as_path());
                let row_x = rect.x + 6.0;
                let row_w = (rect.w - 12.0).max(0.0);
                if selected {
                    // Solid panel fill + 2px accent_primary left rail.
                    raster.fill_pixel_rect(
                        row_x,
                        row_top + 2.0,
                        row_w,
                        (row_h - 4.0).max(0.0),
                        theme.panel,
                    );
                    raster.fill_pixel_rect(
                        row_x,
                        row_top + 2.0,
                        2.0,
                        (row_h - 4.0).max(0.0),
                        theme.accent_primary,
                    );
                } else if hovered_row == Some(slot_i) {
                    // Hover: solid panel fill, no left marker.
                    raster.fill_pixel_rect(
                        row_x,
                        row_top + 2.0,
                        row_w,
                        (row_h - 4.0).max(0.0),
                        theme.panel,
                    );
                }

                // Indent offset for nested entries.
                let indent = *depth as f64 * indent_px;

                // ── Tree branch glyphs (item 2) ───────────────────────────────
                // For each ancestor depth d (0..depth_i): paint │ or nothing.
                // At the row's own depth: paint ├─ or └─.
                let last_flags = &is_last_at_depth[slot_i];
                if *depth > 0 {
                    for d in 0..*depth {
                        let col_x = (rect.x + pad_x + d as f64 * indent_px).floor();
                        let glyph = if d + 1 == *depth {
                            // Row's own connection glyph
                            if last_flags.get(d).copied().unwrap_or(false) {
                                '\u{2514}' // └
                            } else {
                                '\u{251C}' // ├
                            }
                        } else {
                            // Ancestor: vertical bar if NOT last at that depth
                            if last_flags.get(d).copied().unwrap_or(false) {
                                // last sibling at this depth — no vertical line
                                continue;
                            }
                            '\u{2502}' // │
                        };
                        raster.glyph_at(
                            painter,
                            metrics,
                            col_x,
                            glyph_y,
                            glyph as u32,
                            theme.text_subtle,
                        );
                        // Horizontal stub after the connector glyph at own depth.
                        if d + 1 == *depth {
                            let stub_x = col_x + cell_w;
                            raster.glyph_at(
                                painter,
                                metrics,
                                stub_x,
                                glyph_y,
                                '\u{2500}' as u32, // ─
                                theme.text_subtle,
                            );
                        }
                    }
                }

                // File icon column: placed after the tree connector columns.
                let icon_x = rect.x + pad_x + indent;

                // Directory chevron toggles ▸/▾ based on expanded_dirs.
                let (icon_ch, icon_color) = if *is_dir {
                    let chevron = if expanded_dirs.contains(path) {
                        '▾'
                    } else {
                        '▸'
                    };
                    (chevron, theme.foreground)
                } else {
                    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    let (ch, col) = file_icon_colored(name, theme);
                    (ch, if selected { theme.accent_primary } else { col })
                };

                raster.glyph_at(
                    painter,
                    metrics,
                    icon_x,
                    glyph_y,
                    icon_ch as u32,
                    icon_color,
                );

                let label_x = icon_x + cell_w * 2.0;
                let max_x = rect.x + rect.w - pad_x;

                // Name: entry filename only (not full path).
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

                // ── Git status badge (item 10) ────────────────────────────────
                // Look up the filename in the root snapshot's git_marks.
                // Nested children may not have marks; they're derived from the root listing.
                let git_badge: Option<char> = snap.git_marks.get(name).copied().or_else(|| {
                    // Also check child_snapshots for entries inside expanded dirs.
                    path.parent()
                        .and_then(|p| child_snapshots.get(p))
                        .and_then(|cs| cs.git_marks.get(name).copied())
                });
                if let Some(badge) = git_badge {
                    // Badge rendered in the gap between the file icon and the label.
                    // Color: M=attention, A=verified, ?=text_subtle, D=failure.
                    let badge_color = match badge {
                        'M' => theme.attention,
                        'A' => theme.verified,
                        'D' => theme.failure,
                        _ => theme.text_subtle,
                    };
                    let badge_x = label_x - cell_w;
                    let badge_str: &[u8] = &[badge as u8];
                    if let Ok(s) = std::str::from_utf8(badge_str) {
                        draw_text_run(
                            raster,
                            painter,
                            metrics,
                            s,
                            badge_color,
                            badge_x,
                            glyph_y,
                            badge_x + cell_w,
                        );
                    }
                }

                let label_color = theme.foreground;
                let max_chars = ((max_x - label_x) / cell_w).floor().max(0.0) as usize;
                let truncated = truncate_name(name, max_chars);

                draw_text_run(
                    raster,
                    painter,
                    metrics,
                    &truncated,
                    label_color,
                    label_x,
                    glyph_y,
                    max_x,
                );
            }

            // Populate visible_rows — one entry per row in all_rows (not just
            // rendered ones) so that `ExplorerHit::Row(slot_i)` indexes into it.
            // We populate all of them up-front so the index is stable.
            if hits.visible_rows.is_empty() {
                for (path, is_dir, _depth) in &all_rows {
                    hits.visible_rows.push((path.clone(), *is_dir));
                }
            }

            // Item 8: scroll thumb — only when content overflows the dock.
            if total_rows > available_rows && scroll_indicator_alpha > 0.0 {
                let thumb_h = ((available_rows as f64 / total_rows as f64) * content_h)
                    .max(20.0)
                    .min(content_h);
                let max_scroll = (total_rows - available_rows) as f64;
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

/// Recursively collect the flat, ordered list of visible rows for the tree
/// walk. Appends `(absolute_path, is_dir, depth)` for each entry that should
/// be rendered. Respects `expanded_dirs` and `child_snapshots`.
///
/// `snap` is the snapshot for the directory at `dir_path`. Depth is capped at
/// [`MAX_RENDER_DEPTH`] to prevent stack or render blowup on pathological trees.
fn collect_visible_rows(
    snap: &DirSnapshot,
    dir_path: &Path,
    depth: usize,
    expanded_dirs: &HashSet<PathBuf>,
    child_snapshots: &HashMap<PathBuf, DirSnapshot>,
    out: &mut Vec<(PathBuf, bool, usize)>,
) {
    if depth >= MAX_RENDER_DEPTH {
        return;
    }
    for entry in &snap.entries {
        let abs = dir_path.join(&entry.name);
        out.push((abs.clone(), entry.is_dir, depth));
        if entry.is_dir && expanded_dirs.contains(&abs) {
            if let Some(child_snap) = child_snapshots.get(&abs) {
                collect_visible_rows(
                    child_snap,
                    &abs,
                    depth + 1,
                    expanded_dirs,
                    child_snapshots,
                    out,
                );
            }
        }
    }
}

/// R3: collect the absolute paths of all known directories in the tree.
/// Used to build a "expand everything" set for filter mode.
fn collect_all_dirs(
    snap: &DirSnapshot,
    child_snapshots: &HashMap<PathBuf, DirSnapshot>,
) -> HashSet<PathBuf> {
    let mut out = HashSet::new();
    fn recurse(
        snap: &DirSnapshot,
        dir_path: &Path,
        child_snapshots: &HashMap<PathBuf, DirSnapshot>,
        out: &mut HashSet<PathBuf>,
        depth: usize,
    ) {
        if depth >= MAX_RENDER_DEPTH {
            return;
        }
        for entry in &snap.entries {
            if entry.is_dir {
                let abs = dir_path.join(&entry.name);
                out.insert(abs.clone());
                if let Some(child_snap) = child_snapshots.get(&abs) {
                    recurse(child_snap, &abs, child_snapshots, out, depth + 1);
                }
            }
        }
    }
    let root = PathBuf::from(&snap.root);
    recurse(snap, &root, child_snapshots, &mut out, 0);
    out
}

// ── Outline section ───────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn draw_outline_section(
    raster: &mut Raster,
    painter: &mut dyn GlyphPainter,
    metrics: FontMetrics,
    theme: &Theme,
    outline: Option<&[OutlineRow]>,
    rect: Rect,
    hits: &mut LeftDockHits,
    rm: &RowMetrics,
) {
    let cell_h = metrics.cell_h;
    let cell_w = metrics.cell_w;
    let header_h = rm.header_h;
    let row_h = rm.row_h;
    let pad_x = rm.pad_x;

    // ── Header row ────────────────────────────────────────────────────────────
    // Item 10: header color is text_subtle when empty, accent_bright when symbols present.
    let has_symbols = outline.is_some_and(|rows| !rows.is_empty());
    let header_color = if has_symbols {
        theme.accent_bright
    } else {
        theme.text_subtle
    };
    let header_y = rect.y + ((header_h - cell_h) * 0.5 + metrics.descent * 0.5).max(0.0);
    draw_text_run(
        raster,
        painter,
        metrics,
        "OUTLINE",
        header_color,
        rect.x + pad_x,
        header_y,
        rect.x + rect.w,
    );
    raster.fill_pixel_rect(rect.x, rect.y + header_h - 1.0, rect.w, 1.0, theme.hairline);

    let content_y = rect.y + header_h;
    let content_h = rect.h - header_h;
    if content_h <= 0.0 {
        return;
    }

    match outline {
        // Item 10: None and Some(&[]) both collapse to header-only; no body copy.
        None | Some([]) => {
            // Empty state — only the header row is shown (rendered above).
        }
        Some(rows) => {
            let available_rows = (content_h / row_h).floor() as usize;
            for (i, row) in rows.iter().enumerate() {
                if i >= available_rows {
                    break;
                }
                let row_top = content_y + i as f64 * row_h;
                hits.hits.push(LeftDockHit {
                    rect: Rect {
                        x: rect.x,
                        y: row_top,
                        w: rect.w,
                        h: row_h.min((content_y + content_h - row_top).max(0.0)),
                    },
                    kind: LeftDockHitKind::Outline(i),
                });
                let glyph_y = row_top + ((row_h - cell_h) * 0.5 + metrics.descent * 0.5).max(0.0);

                // Indent: 2 cells per depth level.
                let outline_indent_cells = row.depth as usize * 2;
                let outline_indent_px = outline_indent_cells as f64 * cell_w;
                let x_start = rect.x + pad_x + outline_indent_px;
                let x_max = rect.x + rect.w - pad_x;

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

// ── File icon helpers (item 3) ────────────────────────────────────────────────

/// Return the Nerd Font glyph char and tint color for a file based on its name
/// and extension (item 3: colored Nerd Font glyphs from BlexMono Nerd Font Mono).
///
/// Callers that don't have a `Theme` can use [`file_icon`] for plain ASCII.
pub fn file_icon_colored(name: &str, theme: &Theme) -> (char, [u8; 3]) {
    // Exact filename overrides.
    match name {
        "Cargo.toml" => return ('\u{E7A8}', theme.attention), // rust glyph, attention tint
        "README.md" | "README.MD" | "readme.md" => {
            return ('\u{F48A}', theme.accent_bright); // markdown, accent_bright
        }
        _ => {}
    }
    let ext = name.rfind('.').map(|i| &name[i + 1..]).unwrap_or("");
    match ext {
        "rs" => ('\u{E7A8}', theme.attention), // rust icon
        "md" | "markdown" => ('\u{F48A}', theme.text_muted), // markdown
        "toml" | "yaml" | "yml" => ('\u{E6B2}', theme.text_muted), // cog
        "json" => ('\u{E60B}', theme.text_muted),
        "html" | "htm" => ('\u{E736}', theme.text_muted),
        "css" => ('\u{E749}', theme.text_muted),
        "txt" => ('\u{F15C}', theme.text_muted),
        "lock" => ('\u{F023}', theme.text_muted),
        _ => ('\u{25C7}', theme.text_muted), // ◇ default
    }
}

/// Return a single ASCII/Unicode glyph string for a file (legacy; used in tests only).
///
/// These are ASCII/Unicode characters that render in the monospace grid.
/// `.lock` uses "L" rather than 🔒 because emoji rendering through the cell
/// grid atlas is unpredictable on different macOS font configs.
#[cfg(test)]
fn file_icon(name: &str) -> &'static str {
    // Match by extension (case-sensitive; lowercase extensions are the norm).
    let ext = name.rfind('.').map(|i| &name[i + 1..]).unwrap_or("");
    match ext {
        "rs" => "r",
        "md" | "markdown" => "M",
        "toml" => "T",
        "html" | "htm" => "<>",
        "css" => "#",
        "json" => "{}",
        "txt" => "=",
        "lock" => "L",
        _ => "\u{25C7}", // ◇
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
            git_marks: Default::default(),
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
            git_marks: Default::default(),
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
            git_marks: Default::default(),
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
            git_marks: Default::default(),
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
            &HashMap::new(),
            0.0,
            1.0,
            None,
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
            git_marks: Default::default(),
        };
        draw_left_dock(&mut r, &mut p, m, &th, Some(&snap), None, None, dock_rect());

        // File label glyph 'm' must paint in foreground (high contrast).
        let file_m: Vec<_> = p
            .glyphs
            .iter()
            .filter(|(cp, fg)| *cp == 'm' as u32 && *fg == th.foreground)
            .collect();
        assert!(
            !file_m.is_empty(),
            "expected 'm' in foreground for file entry label"
        );

        // File icon for `.rs` is U+E7A8 (Nerd Font rust icon, item 3) and paints in
        // text_muted (attention color from theme) for inactive files.
        let rust_icon_cp = '\u{E7A8}' as u32;
        let file_icon: Vec<_> = p
            .glyphs
            .iter()
            .filter(|(cp, _)| *cp == rust_icon_cp)
            .collect();
        assert!(
            !file_icon.is_empty(),
            "expected Nerd Font rust icon (U+E7A8) for inactive .rs file"
        );

        // Dir chevron ▸ (U+25B8) icon paints in foreground for inactive dir.
        let dir_chevron: Vec<_> = p
            .glyphs
            .iter()
            .filter(|(cp, fg)| *cp == '\u{25B8}' as u32 && *fg == th.foreground)
            .collect();
        assert!(
            !dir_chevron.is_empty(),
            "expected dir chevron ▸ in foreground for inactive dir"
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
            git_marks: Default::default(),
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

        // Selected row signaled by icon color: U+E7A8 (Nerd Font rust, item 3) paints
        // in accent_primary for the active file, theme.attention for inactive.
        // Labels stay foreground for both (selection visible via row bg + left rail).
        let rust_cp = '\u{E7A8}' as u32;
        let selected_icon: Vec<_> = p
            .glyphs
            .iter()
            .filter(|(cp, fg)| *cp == rust_cp && *fg == th.accent_primary)
            .collect();
        assert_eq!(
            selected_icon.len(),
            1,
            "exactly one file icon (the active one) should paint in accent_primary"
        );

        let inactive_icon: Vec<_> = p
            .glyphs
            .iter()
            .filter(|(cp, fg)| *cp == rust_cp && *fg == th.attention)
            .collect();
        assert_eq!(
            inactive_icon.len(),
            1,
            "exactly one inactive file icon should paint in theme.attention (rust tint)"
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
            git_marks: Default::default(),
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
                &HashMap::new(),
                0.0,
                1.0,
                None,
            );
            // Row 0 occupies y=[header_h, header_h+row_h) = [32, 60).
            // The fill rect is row_top+2 .. row_top+row_h-2 = [34, 58).
            // Sample the fill interior: x=50, y=38 (inside fill strip).
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
                &HashMap::new(),
                0.0,
                1.0,
                None,
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
                line: 0,
            },
            OutlineRow {
                name: "MyStruct".to_string(),
                kind: OutlineKind::Struct,
                depth: 0,
                line: 5,
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
            git_marks: Default::default(),
        };

        // At offset=0: first content row (y ≈ header_h + row_h/2 = 32+14 = 46; y=39 is also in [32,60)) → entry 0.
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
                &HashMap::new(),
                0.0,
                1.0,
                None,
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
                &HashMap::new(),
                0.0,
                1.0,
                None,
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

    /// Expanding a directory shows its children indented below it.
    /// visible_rows must contain the child entries after the parent.
    #[test]
    fn expanded_dir_shows_children_in_visible_rows() {
        let m = metrics();
        let th = theme();
        let mut r = Raster::new(800, 800);
        let mut p = StubPainter::default();

        let root_snap = DirSnapshot {
            root: "/project".to_string(),
            entries: vec![
                DirEntry {
                    name: "src".to_string(),
                    is_dir: true,
                },
                DirEntry {
                    name: "Cargo.toml".to_string(),
                    is_dir: false,
                },
            ],
            git_marks: Default::default(),
        };
        let src_snap = DirSnapshot {
            root: "/project/src".to_string(),
            entries: vec![
                DirEntry {
                    name: "main.rs".to_string(),
                    is_dir: false,
                },
                DirEntry {
                    name: "lib.rs".to_string(),
                    is_dir: false,
                },
            ],
            git_marks: Default::default(),
        };
        let src_path = PathBuf::from("/project/src");

        let mut expanded = HashSet::new();
        expanded.insert(src_path.clone());
        let mut children = HashMap::new();
        children.insert(src_path, src_snap);

        let hits = draw_left_dock_with_scroll(
            &mut r,
            &mut p,
            m,
            &th,
            Some(&root_snap),
            None,
            None,
            dock_rect(),
            0,
            None,
            &expanded,
            &children,
            0.0,
            1.0,
            None,
        );

        // visible_rows should be: [/project/src (dir), /project/src/main.rs, /project/src/lib.rs, /project/Cargo.toml]
        assert_eq!(
            hits.visible_rows.len(),
            4,
            "root has 2 entries; expanded src adds 2 children"
        );
        assert_eq!(hits.visible_rows[0], (PathBuf::from("/project/src"), true));
        assert_eq!(
            hits.visible_rows[1],
            (PathBuf::from("/project/src/main.rs"), false)
        );
        assert_eq!(
            hits.visible_rows[2],
            (PathBuf::from("/project/src/lib.rs"), false)
        );
        assert_eq!(
            hits.visible_rows[3],
            (PathBuf::from("/project/Cargo.toml"), false)
        );

        // There should be 4 row hits (+ 1 header hit = 5 total).
        let row_hits: Vec<_> = hits
            .hits
            .iter()
            .filter(|h| matches!(h.kind, LeftDockHitKind::Explorer(ExplorerHit::Row(_))))
            .collect();
        assert_eq!(row_hits.len(), 4, "4 visible rows: dir + 2 children + file");
    }

    /// collect_visible_rows respects depth limit.
    #[test]
    fn collect_visible_rows_depth_cap() {
        // Build a chain 35 levels deep (exceeds MAX_RENDER_DEPTH=32).
        fn make_chain(depth: usize, name: &str) -> DirSnapshot {
            if depth == 0 {
                DirSnapshot {
                    root: name.to_string(),
                    entries: vec![],
                    git_marks: Default::default(),
                }
            } else {
                DirSnapshot {
                    root: name.to_string(),
                    entries: vec![DirEntry {
                        name: "sub".to_string(),
                        is_dir: true,
                    }],
                    git_marks: Default::default(),
                }
            }
        }

        let root_path = PathBuf::from("/r");
        let root_snap = make_chain(1, "/r");
        let mut expanded = HashSet::new();
        let mut children = HashMap::new();

        // Build 35 levels: /r/sub, /r/sub/sub, ...
        let mut cur = root_path.clone();
        for i in 0..35 {
            let child = cur.join("sub");
            expanded.insert(child.clone());
            let child_str = child.to_string_lossy().into_owned();
            let snap = if i < 34 {
                DirSnapshot {
                    root: child_str,
                    entries: vec![DirEntry {
                        name: "sub".to_string(),
                        is_dir: true,
                    }],
                    git_marks: Default::default(),
                }
            } else {
                DirSnapshot {
                    root: child_str,
                    entries: vec![],
                    git_marks: Default::default(),
                }
            };
            children.insert(child.clone(), snap);
            cur = child;
        }

        let mut out = Vec::new();
        collect_visible_rows(&root_snap, &root_path, 0, &expanded, &children, &mut out);

        // Should collect at most MAX_RENDER_DEPTH=32 levels deep; anything beyond is dropped.
        // Root has 1 entry (/r/sub at depth 0), then /r/sub has 1 child at depth 1, etc.
        // At depth 32 the recursion stops, so at most 33 rows (depths 0..=32).
        assert!(
            out.len() <= MAX_RENDER_DEPTH + 1,
            "depth cap should limit rows to at most {}, got {}",
            MAX_RENDER_DEPTH + 1,
            out.len()
        );
    }

    /// A1: top-level rows (depth 0) must not emit any tree connector glyph.
    /// A nested row at depth 1 must emit a connector.
    #[test]
    fn top_level_rows_have_no_connector_glyph() {
        const CONNECTORS: &[u32] = &[
            '\u{2514}' as u32, // └
            '\u{251C}' as u32, // ├
            '\u{2502}' as u32, // │
        ];

        let m = metrics();
        let th = theme();

        // Build: root has one dir (src) and one file (main.rs).
        // Expanding src to show one child (lib.rs) at depth 1.
        let root_snap = DirSnapshot {
            root: "/p".to_string(),
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
            git_marks: Default::default(),
        };
        let src_snap = DirSnapshot {
            root: "/p/src".to_string(),
            entries: vec![DirEntry {
                name: "lib.rs".to_string(),
                is_dir: false,
            }],
            git_marks: Default::default(),
        };
        let src_path = PathBuf::from("/p/src");
        let mut expanded = HashSet::new();
        expanded.insert(src_path.clone());
        let mut children = HashMap::new();
        children.insert(src_path, src_snap);

        // Draw with expansion.
        let mut r = Raster::new(800, 800);
        let mut p = StubPainter::default();
        draw_left_dock_with_scroll(
            &mut r,
            &mut p,
            m,
            &th,
            Some(&root_snap),
            None,
            None,
            dock_rect(),
            0,
            None,
            &expanded,
            &children,
            0.0,
            1.0,
            None,
        );

        // The test rigs glyph positions by recording every (codepoint, color) pair.
        // We check that glyphs recorded before the first nested row's glyph_y contain
        // no connector codepoints.
        //
        // Simpler: count connector glyphs. Top-level rows (slot 0 = "src" at depth 0,
        // slot 2 = "main.rs" at depth 0) must contribute zero connectors.
        // Slot 1 = "lib.rs" at depth 1 MUST have at least one connector.
        //
        // Because StubPainter records all glyphs in order, we can reason about counts:
        // - depth-0 rows add 0 connectors each.
        // - depth-1 row adds ≥1 connector.
        // So total connectors == connectors from depth-1 rows only.
        let connector_count = p
            .glyphs
            .iter()
            .filter(|(cp, _)| CONNECTORS.contains(cp))
            .count();

        // There is exactly 1 depth-1 row (lib.rs), so at least 1 connector expected.
        assert!(
            connector_count >= 1,
            "depth-1 row must emit at least one connector glyph"
        );

        // Now draw without expansion (all rows at depth 0). Zero connectors expected.
        let mut r2 = Raster::new(800, 800);
        let mut p2 = StubPainter::default();
        draw_left_dock(
            &mut r2,
            &mut p2,
            m,
            &th,
            Some(&root_snap),
            None,
            None,
            dock_rect(),
        );
        let connector_count_flat = p2
            .glyphs
            .iter()
            .filter(|(cp, _)| CONNECTORS.contains(cp))
            .count();
        assert_eq!(
            connector_count_flat, 0,
            "top-level rows (depth 0) must emit NO connector glyphs; got {connector_count_flat}"
        );
    }

    /// Item 9: file_icon returns extension-appropriate glyphs.
    #[test]
    fn file_icon_returns_extension_glyphs() {
        assert_eq!(file_icon("main.rs"), "r", ".rs → 'r'");
        assert_eq!(file_icon("README.md"), "M", ".md → 'M'");
        assert_eq!(file_icon("Cargo.toml"), "T", ".toml → 'T'");
        assert_eq!(file_icon("index.html"), "<>", ".html → '<>'");
        assert_eq!(file_icon("style.css"), "#", ".css → '#'");
        assert_eq!(file_icon("config.json"), "{}", ".json returns braces icon");
        assert_eq!(file_icon("notes.txt"), "=", ".txt → '='");
        assert_eq!(file_icon("Cargo.lock"), "L", ".lock → 'L'");
        assert_eq!(file_icon("binary"), "\u{25C7}", "no extension → ◇");
        assert_eq!(file_icon("file.xyz"), "\u{25C7}", "unknown extension → ◇");
    }

    // ── G1: icon-only and hide modes ──────────────────────────────────────────

    /// G1: rect.w < 60pt (hide threshold) returns empty hits, no panic.
    #[test]
    fn very_narrow_dock_hides_entirely() {
        let m = metrics();
        let th = theme();
        let mut r = Raster::new(800, 800);
        let mut p = StubPainter::default();
        let snap = DirSnapshot {
            root: "/p".to_string(),
            entries: vec![DirEntry {
                name: "main.rs".to_string(),
                is_dir: false,
            }],
            git_marks: Default::default(),
        };
        let narrow = Rect {
            x: 0.0,
            y: 0.0,
            w: 40.0,
            h: 800.0,
        };
        let hits = draw_left_dock_with_scroll(
            &mut r,
            &mut p,
            m,
            &th,
            Some(&snap),
            None,
            None,
            narrow,
            0,
            None,
            &HashSet::new(),
            &HashMap::new(),
            0.0,
            1.0,
            None,
        );
        assert!(hits.hits.is_empty(), "dock < 60pt must be fully hidden");
    }

    /// G1: rect.w in [60, 120) returns icon hits only — no EXPLORER header hit.
    #[test]
    fn narrow_dock_returns_icon_only_hits() {
        let m = metrics();
        let th = theme();
        let mut r = Raster::new(800, 800);
        let mut p = StubPainter::default();
        let snap = DirSnapshot {
            root: "/p".to_string(),
            entries: vec![
                DirEntry {
                    name: "a.rs".to_string(),
                    is_dir: false,
                },
                DirEntry {
                    name: "b.rs".to_string(),
                    is_dir: false,
                },
            ],
            git_marks: Default::default(),
        };
        // 80pt wide: between 60 and 120 → icons-only.
        let narrow = Rect {
            x: 0.0,
            y: 0.0,
            w: 80.0,
            h: 800.0,
        };
        let hits = draw_left_dock_with_scroll(
            &mut r,
            &mut p,
            m,
            &th,
            Some(&snap),
            None,
            None,
            narrow,
            0,
            None,
            &HashSet::new(),
            &HashMap::new(),
            0.0,
            1.0,
            None,
        );
        // No EXPLORER header hit in icons-only mode.
        let header_hits: Vec<_> = hits
            .hits
            .iter()
            .filter(|h| h.kind == LeftDockHitKind::Explorer(ExplorerHit::Header))
            .collect();
        assert!(header_hits.is_empty(), "icons-only must have no header hit");
        // Two row hits for the two entries.
        let row_hits: Vec<_> = hits
            .hits
            .iter()
            .filter(|h| matches!(h.kind, LeftDockHitKind::Explorer(ExplorerHit::Row(_))))
            .collect();
        assert_eq!(
            row_hits.len(),
            2,
            "icons-only must have one row hit per entry"
        );
    }

    /// Item 10: git badge 'M' is rendered in attention color for a modified file.
    #[test]
    fn git_badge_modified_renders_in_attention_color() {
        let m = metrics();
        let th = theme();
        let mut r = Raster::new(800, 800);
        let mut p = StubPainter::default();

        let mut git_marks = HashMap::new();
        git_marks.insert("main.rs".to_string(), 'M');
        let snap = DirSnapshot {
            root: "/anvil/src".to_string(),
            entries: vec![DirEntry {
                name: "main.rs".to_string(),
                is_dir: false,
            }],
            git_marks,
        };

        draw_left_dock(&mut r, &mut p, m, &th, Some(&snap), None, None, dock_rect());

        // 'M' glyph in attention color must appear.
        let badge_calls: Vec<_> = p
            .glyphs
            .iter()
            .filter(|(cp, fg)| *cp == 'M' as u32 && *fg == th.attention)
            .collect();
        assert!(
            !badge_calls.is_empty(),
            "modified-file badge 'M' must render in attention color"
        );
    }

    // ── R3: explorer_filter hides non-matching rows ───────────────────────────

    /// When a filter is active, only file rows whose basename case-insensitively
    /// contains the filter string (and their ancestor dirs) should appear in
    /// `visible_rows`. Non-matching files are hidden.
    #[test]
    fn explorer_filter_hides_non_matching_rows() {
        let m = metrics();
        let th = theme();
        let mut r = Raster::new(800, 800);
        let mut p = StubPainter::default();

        let snap = DirSnapshot {
            root: "/project".to_string(),
            entries: vec![
                DirEntry {
                    name: "main.rs".to_string(),
                    is_dir: false,
                },
                DirEntry {
                    name: "lib.rs".to_string(),
                    is_dir: false,
                },
                DirEntry {
                    name: "README.md".to_string(),
                    is_dir: false,
                },
            ],
            git_marks: Default::default(),
        };

        // Filter "main" — only main.rs should appear.
        let hits = draw_left_dock_with_scroll(
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
            &HashMap::new(),
            0.0,
            1.0,
            Some("main"),
        );

        assert_eq!(
            hits.visible_rows.len(),
            1,
            "filter 'main' must leave only main.rs visible; got {:?}",
            hits.visible_rows
        );
        assert!(
            hits.visible_rows[0]
                .0
                .to_str()
                .unwrap_or("")
                .ends_with("main.rs"),
            "the single visible row must be main.rs"
        );
    }

    /// When filter is empty string, all rows should still be visible.
    #[test]
    fn explorer_filter_none_shows_all_rows() {
        let m = metrics();
        let th = theme();
        let mut r = Raster::new(800, 800);
        let mut p = StubPainter::default();

        let snap = DirSnapshot {
            root: "/project".to_string(),
            entries: vec![
                DirEntry {
                    name: "a.rs".to_string(),
                    is_dir: false,
                },
                DirEntry {
                    name: "b.rs".to_string(),
                    is_dir: false,
                },
            ],
            git_marks: Default::default(),
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
            0,
            None,
            &HashSet::new(),
            &HashMap::new(),
            0.0,
            1.0,
            None,
        );

        assert_eq!(
            hits.visible_rows.len(),
            2,
            "no filter: all 2 rows must be visible"
        );
    }
}
