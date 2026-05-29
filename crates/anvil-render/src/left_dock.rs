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

use crate::raster::{FontMetrics, GlyphPainter, Raster, UiTextPainter, UiWeight};
use crate::ui_text_sizes::{EXPLORER_HEADER_PT, EXPLORER_ROW_PT};

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
    /// Click on the OUTLINE section header (for collapse toggle, #9).
    OutlineHeader,
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
    /// True when the filesystem entry is a symbolic link (Y3).
    pub is_symlink: bool,
}

/// Snapshot of a directory's top-level entries.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
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
const HEADER_H_BASE: f64 = 22.0;

/// Base height of a content row in points (multiplied by `ui_scale`).
const ROW_H_BASE: f64 = 22.0;

/// Base horizontal padding inside the dock (multiplied by `ui_scale`).
const PAD_X_BASE: f64 = 8.0;

/// Base indent per depth level in device pixels (multiplied by `ui_scale`).
const INDENT_PX_BASE: f64 = 12.0;

/// Derive row geometry from the current UI scale and actual chrome glyph size.
struct RowMetrics {
    header_h: f64,
    row_h: f64,
    pad_x: f64,
    indent_px: f64,
}

impl RowMetrics {
    fn from_scale(ui_scale: f64, metrics: FontMetrics) -> Self {
        let scale = ui_scale.max(0.5);
        let vertical_pad = (6.0 * scale).round().max(6.0);
        let min_text_h = (metrics.cell_h + vertical_pad).ceil();
        Self {
            header_h: (HEADER_H_BASE * scale).round().max(min_text_h),
            row_h: (ROW_H_BASE * scale).round().max(min_text_h),
            pad_x: (PAD_X_BASE * scale).round(),
            indent_px: (INDENT_PX_BASE * scale).round(),
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
    ui_painter: &mut dyn UiTextPainter,
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
        ui_painter,
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
        false,
        false,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn draw_left_dock_with_scroll(
    raster: &mut Raster,
    painter: &mut dyn GlyphPainter,
    ui_painter: &mut dyn UiTextPainter,
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
    // #9: whether each section is collapsed (header-only when true).
    explorer_collapsed: bool,
    outline_collapsed: bool,
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
    // Glass-style sidebar: a dark base plus translucent lifted surface, with a
    // single edge hairline. Accent is reserved for selected rows.
    raster.fill_pixel_rect(rect.x, rect.y, rect.w, rect.h, theme.graphite);
    raster.fill_pixel_rect_alpha(rect.x, rect.y, rect.w, rect.h, theme.surface, 0.46);
    raster.fill_pixel_rect_alpha(rect.x, rect.y, rect.w, 1.0, theme.foreground, 0.035);

    // Right-edge 1px hairline.
    raster.fill_pixel_rect_alpha(
        rect.x + rect.w - 1.0,
        rect.y,
        1.0,
        rect.h,
        theme.hairline,
        0.72,
    );

    // ── Section height: #9 collapse shrinks a section to header-only ─────────
    let rm = RowMetrics::from_scale(ui_scale, metrics);
    // When both are collapsed the remaining space goes to explorer.
    let explorer_h = if explorer_collapsed {
        rm.header_h
    } else if outline_collapsed {
        rect.h - rm.header_h
    } else {
        (rect.h * 0.60).floor()
    };
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
    raster.fill_pixel_rect_alpha(
        rect.x,
        rect.y + explorer_h,
        rect.w - 1.0,
        1.0,
        theme.hairline,
        0.70,
    );

    let ui_metrics = metrics;
    draw_explorer_section(
        raster,
        painter,
        ui_painter,
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
        explorer_collapsed,
    );
    draw_outline_section(
        raster,
        painter,
        ui_painter,
        ui_metrics,
        theme,
        outline,
        outline_rect,
        &mut hits,
        &rm,
        outline_collapsed,
    );
    hits
}

// ── Icons-only mode (G1) ──────────────────────────────────────────────────────

/// Width of the icons-only dock in logical points (before ui_scale).
const ICONS_ONLY_W_BASE: f64 = 48.0;

const ICON_FOLDER: char = '\u{f07b}'; // nf-fa-folder
const ICON_FOLDER_OPEN: char = '\u{f07c}'; // nf-fa-folder_open
const ICON_FILE: char = '\u{f15b}'; // nf-fa-file
const ICON_FILE_TEXT: char = '\u{f0f6}'; // nf-fa-file_text_o
const ICON_FILE_CODE: char = '\u{f1c9}'; // nf-fa-file_code_o
const ICON_FILE_IMAGE: char = '\u{f1c5}'; // nf-fa-file_image_o
const ICON_FILE_ARCHIVE: char = '\u{f1c6}'; // nf-fa-file_archive_o
const ICON_COG: char = '\u{f013}'; // nf-fa-cog
const ICON_LOCK: char = '\u{f023}'; // nf-fa-lock

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
    let rm = RowMetrics::from_scale(ui_scale, metrics);
    let slim_w = (ICONS_ONLY_W_BASE * ui_scale).min(rect.w);

    // Background.
    raster.fill_pixel_rect(rect.x, rect.y, slim_w, rect.h, theme.graphite);
    raster.fill_pixel_rect_alpha(rect.x, rect.y, slim_w, rect.h, theme.surface, 0.46);
    // Right-edge hairline.
    raster.fill_pixel_rect_alpha(
        rect.x + slim_w - 1.0,
        rect.y,
        1.0,
        rect.h,
        theme.hairline,
        0.72,
    );

    let snap = match snapshot {
        Some(s) if !s.entries.is_empty() => s,
        _ => return std::mem::take(hits),
    };

    let mut all_rows: Vec<(PathBuf, bool, usize, bool)> = Vec::new();
    collect_visible_rows(
        snap,
        &PathBuf::from(&snap.root),
        0,
        expanded_dirs,
        child_snapshots,
        &mut all_rows,
    );

    // Populate visible_rows (stable index for hit dispatch).
    for (path, is_dir, _, _) in &all_rows {
        hits.visible_rows.push((path.clone(), *is_dir));
    }

    let row_h = rm.row_h;
    let available_rows = (rect.h / row_h).floor() as usize;
    let total_rows = all_rows.len();
    let first = scroll_offset.min(total_rows.saturating_sub(available_rows));
    let cell_w = metrics.cell_w;
    let cell_h = metrics.cell_h;

    for (slot_i, (path, is_dir, _depth, _sym)) in
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
            raster.fill_pixel_rect_alpha(
                row_x,
                row_top + 1.0,
                row_w,
                (row_h - 2.0).max(0.0),
                theme.accent_primary,
                0.13,
            );
            raster.fill_pixel_rect(
                row_x,
                row_top + 1.0,
                2.0,
                (row_h - 2.0).max(0.0),
                theme.accent_primary,
            );
        } else if hovered {
            raster.fill_pixel_rect_alpha(
                row_x,
                row_top + 1.0,
                row_w,
                (row_h - 2.0).max(0.0),
                theme.surface_alt,
                0.32,
            );
        }

        let (icon_ch, icon_color) = if *is_dir {
            let ch = if expanded_dirs.contains(path) {
                ICON_FOLDER_OPEN
            } else {
                ICON_FOLDER
            };
            (ch, theme.text_muted)
        } else {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            let (ch, _col) = file_icon_colored(name, theme);
            (
                ch,
                if selected {
                    theme.accent_primary
                } else {
                    theme.text_subtle
                },
            )
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
    ui_painter: &mut dyn UiTextPainter,
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
    collapsed: bool,
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

    let header_icon_top = rect.y + ((header_h - cell_h) * 0.5 + metrics.descent * 0.5).max(0.0);
    let header_baseline = header_icon_top + (cell_h - metrics.descent);
    // #9: collapse chevron — ▾ when expanded, ▸ when collapsed. Drawn at pad_x.
    // Chevrons are Nerd Font glyphs; keep on mono path.
    let chevron = if collapsed { '▸' } else { '▾' };
    raster.glyph_at(
        painter,
        metrics,
        rect.x + pad_x,
        header_icon_top,
        chevron as u32,
        theme.text_subtle,
    );
    // Option A: section labels are quiet chrome, not alerts.
    raster.ui_line(
        ui_painter,
        header_label,
        rect.x + pad_x + cell_w * 1.5,
        header_baseline,
        EXPLORER_HEADER_PT,
        UiWeight::Regular,
        theme.text_muted,
    );
    if !header_meta.is_empty() {
        let meta_w = raster.ui_measure(
            ui_painter,
            &header_meta,
            EXPLORER_HEADER_PT,
            UiWeight::Regular,
        );
        raster.ui_line(
            ui_painter,
            &header_meta,
            (rect.x + rect.w - pad_x - meta_w).max(rect.x + pad_x + cell_w * 1.5),
            header_baseline,
            EXPLORER_HEADER_PT,
            UiWeight::Regular,
            meta_color,
        );
    }
    // Hairline under header.
    raster.fill_pixel_rect_alpha(
        rect.x,
        rect.y + header_h - 1.0,
        rect.w,
        1.0,
        theme.hairline,
        0.70,
    );

    // #9: section is collapsed — only the header row is shown.
    if collapsed {
        return;
    }

    // ── Content rows ──────────────────────────────────────────────────────────
    let content_y_start = rect.y + header_h;
    let content_h = rect.h - header_h;
    if content_h <= 0.0 {
        return;
    }

    match snapshot {
        None => {
            // No cwd yet — waiting state.
            let row_icon_top =
                content_y_start + ((row_h - cell_h) * 0.5 + metrics.descent * 0.5).max(0.0);
            let row_baseline = row_icon_top + (cell_h - metrics.descent);
            raster.ui_line(
                ui_painter,
                "Waiting for shell prompt\u{2026}",
                rect.x + pad_x,
                row_baseline,
                EXPLORER_ROW_PT,
                UiWeight::Regular,
                theme.text_muted,
            );
        }
        Some(snap) if snap.entries.is_empty() => {
            let row_icon_top =
                content_y_start + ((row_h - cell_h) * 0.5 + metrics.descent * 0.5).max(0.0);
            let row_baseline = row_icon_top + (cell_h - metrics.descent);
            raster.ui_line(
                ui_painter,
                "(empty)",
                rect.x + pad_x,
                row_baseline,
                EXPLORER_ROW_PT,
                UiWeight::Regular,
                theme.text_muted,
            );
        }
        Some(snap) => {
            // Build the flat ordered list of all visible rows by walking the
            // tree top-down: root entries, then recursively expanded children.
            // Also collect last-sibling flags per row for branch glyph rendering.
            let mut all_rows: Vec<(PathBuf, bool, usize, bool)> = Vec::new(); // (path, is_dir, depth, is_symlink)
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
                    .filter(|(_, (p, is_dir, _, _))| {
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
                all_rows.retain(|(p, _, _, _)| keep_paths.contains(p));
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
                        !all_rows[i + 1..].iter().any(|(p, _, dd, _)| {
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

            for (slot_i, (path, is_dir, depth, is_symlink)) in
                all_rows.iter().enumerate().skip(first).take(available_rows)
            {
                // slot_i is the absolute index in `all_rows`; row_i is the screen slot.
                let row_i = slot_i - first;
                let row_top = content_y_start + row_i as f64 * row_h;

                // Y4: empty-dir sentinel rows use path ending in "\x00empty".
                let is_empty_sentinel = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n == "\x00empty")
                    .unwrap_or(false);

                // The visible row index reported in the hit is `slot_i` so that
                // `visible_rows[slot_i]` gives back the path.
                if !is_empty_sentinel {
                    hits.hits.push(LeftDockHit {
                        rect: Rect {
                            x: rect.x,
                            y: row_top,
                            w: rect.w,
                            h: row_h.min((content_y_start + content_h - row_top).max(0.0)),
                        },
                        kind: LeftDockHitKind::Explorer(ExplorerHit::Row(slot_i)),
                    });
                }

                let icon_top = row_top + ((row_h - cell_h) * 0.5 + metrics.descent * 0.5).max(0.0);
                let baseline_y = icon_top + (cell_h - metrics.descent);

                // Y4: render italic "(empty)" placeholder for empty expanded dirs.
                if is_empty_sentinel {
                    let indent = *depth as f64 * indent_px;
                    let text_x = rect.x + pad_x + indent + cell_w * 2.0;
                    raster.ui_line(
                        ui_painter,
                        "(empty)",
                        text_x,
                        baseline_y,
                        EXPLORER_ROW_PT,
                        UiWeight::Regular,
                        theme.text_subtle,
                    );
                    continue;
                }

                let selected = !is_dir && active_file_path == Some(path.as_path());
                let row_x = rect.x + 6.0;
                let row_w = (rect.w - 12.0).max(0.0);
                if selected {
                    raster.fill_pixel_rect_alpha(
                        row_x,
                        row_top + 1.0,
                        row_w,
                        (row_h - 2.0).max(0.0),
                        theme.accent_primary,
                        0.13,
                    );
                    raster.fill_pixel_rect(
                        row_x,
                        row_top + 1.0,
                        2.0,
                        (row_h - 2.0).max(0.0),
                        theme.accent_primary,
                    );
                } else if hovered_row == Some(slot_i) {
                    raster.fill_pixel_rect_alpha(
                        row_x,
                        row_top + 1.0,
                        row_w,
                        (row_h - 2.0).max(0.0),
                        theme.surface_alt,
                        0.32,
                    );
                }

                // Indent offset for nested entries.
                let indent = *depth as f64 * indent_px;

                // ── Tree indent guides (pixel lines, not Unicode glyphs) ───────
                // Each ancestor level that is NOT the last sibling at that depth
                // gets a 1px vertical guide line running the full row height.
                // This is cleaner than Unicode box-drawing at every font size.
                let last_flags = &is_last_at_depth[slot_i];
                if *depth > 0 {
                    for d in 0..*depth {
                        // Skip the guide line if this ancestor is the last sibling
                        // (no children follow, so no vertical continuation needed).
                        if last_flags.get(d).copied().unwrap_or(false) {
                            continue;
                        }
                        // Place the guide at the center of the indent column for depth d.
                        let guide_x =
                            (rect.x + pad_x + d as f64 * indent_px + indent_px * 0.5).floor();
                        raster.fill_pixel_rect_alpha(
                            guide_x,
                            row_top,
                            1.0,
                            row_h,
                            theme.text_subtle,
                            0.16,
                        );
                    }
                }

                // Tree rows use stable lanes: chevron, file/folder icon, git
                // badge, then label. Keeping these separate avoids collisions
                // like "Mcrates" while preserving a compact explorer.
                let chevron_x = rect.x + pad_x + indent;
                let file_icon_x = chevron_x + (cell_w * 1.45).ceil();
                let badge_x = file_icon_x + (cell_w * 1.55).ceil();
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

                if *is_dir {
                    let chevron = if expanded_dirs.contains(path) {
                        '▾'
                    } else {
                        '▸'
                    };
                    raster.glyph_at(
                        painter,
                        metrics,
                        chevron_x,
                        icon_top,
                        chevron as u32,
                        theme.text_subtle,
                    );
                    let folder_icon = if expanded_dirs.contains(path) {
                        ICON_FOLDER_OPEN
                    } else {
                        ICON_FOLDER
                    };
                    raster.glyph_at(
                        painter,
                        metrics,
                        file_icon_x,
                        icon_top,
                        folder_icon as u32,
                        theme.text_muted,
                    );
                } else {
                    let (file_icon, file_icon_color) = file_icon_colored(name, theme);
                    raster.glyph_at(
                        painter,
                        metrics,
                        file_icon_x,
                        icon_top,
                        file_icon as u32,
                        if selected {
                            theme.accent_primary
                        } else {
                            file_icon_color
                        },
                    );
                }

                // Badge rows reserve a full lane after the type icon so status
                // glyphs cannot crash into names.
                let label_x = if git_badge.is_some() {
                    badge_x + (cell_w * 1.55).ceil()
                } else {
                    file_icon_x + (cell_w * 1.7).ceil()
                };
                let max_x = rect.x + rect.w - pad_x;

                if let Some(badge) = git_badge {
                    // Badge rendered in its own lane between the tree marker and label.
                    // Color: M=attention, A=verified, ?=text_subtle, D=failure.
                    // Single ASCII char — keep on mono path.
                    let badge_color = match badge {
                        'M' => theme.attention,
                        'A' => theme.verified,
                        'D' => theme.failure,
                        _ => theme.text_subtle,
                    };
                    raster.glyph_at(
                        painter,
                        metrics,
                        badge_x,
                        icon_top,
                        badge as u32,
                        badge_color,
                    );
                }

                // P4: visual hierarchy — dirs are "heavier" anchors, files are
                // quieter leaf nodes. Selected rows use foreground text on the
                // accent rail so the type icon can carry the accent.
                let label_color = if selected {
                    theme.foreground
                } else {
                    theme.text_muted
                };
                // Y5: count badge "(N)" for collapsed dirs whose children are cached.
                // Appended after the name for dirs that are NOT expanded.
                let child_count_suffix: Option<String> = if *is_dir && !expanded_dirs.contains(path)
                {
                    child_snapshots
                        .get(path)
                        .map(|cs| format!(" ({})", cs.entries.len()))
                } else {
                    None
                };

                let avail_w = (max_x - label_x).max(0.0);
                let full_name = if let Some(ref suffix) = child_count_suffix {
                    format!("{name}{suffix}")
                } else {
                    name.to_string()
                };
                let display_name = ui_truncate(
                    ui_painter,
                    &full_name,
                    avail_w,
                    EXPLORER_ROW_PT,
                    UiWeight::Regular,
                );

                raster.ui_line(
                    ui_painter,
                    &display_name,
                    label_x,
                    baseline_y,
                    EXPLORER_ROW_PT,
                    UiWeight::Regular,
                    label_color,
                );

                // Y3: symlink indicator — render → after the name in text_subtle.
                // Single Unicode glyph — keep on mono path.
                if *is_symlink {
                    let label_w = raster.ui_measure(
                        ui_painter,
                        &display_name,
                        EXPLORER_ROW_PT,
                        UiWeight::Regular,
                    );
                    let arrow_x = label_x + label_w + cell_w * 0.5;
                    if arrow_x + cell_w < max_x {
                        raster.glyph_at(
                            painter,
                            metrics,
                            arrow_x,
                            icon_top,
                            '\u{2192}' as u32,
                            theme.text_subtle,
                        );
                    }
                }
            }

            // Populate visible_rows — one entry per row in all_rows (not just
            // rendered ones) so that `ExplorerHit::Row(slot_i)` indexes into it.
            // We populate all of them up-front so the index is stable.
            if hits.visible_rows.is_empty() {
                for (path, is_dir, _depth, _sym) in &all_rows {
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
/// walk. Appends `(absolute_path, is_dir, depth, is_symlink)` for each entry
/// that should be rendered. Respects `expanded_dirs` and `child_snapshots`.
///
/// `snap` is the snapshot for the directory at `dir_path`. Depth is capped at
/// [`MAX_RENDER_DEPTH`] to prevent stack or render blowup on pathological trees.
fn collect_visible_rows(
    snap: &DirSnapshot,
    dir_path: &Path,
    depth: usize,
    expanded_dirs: &HashSet<PathBuf>,
    child_snapshots: &HashMap<PathBuf, DirSnapshot>,
    out: &mut Vec<(PathBuf, bool, usize, bool)>,
) {
    if depth >= MAX_RENDER_DEPTH {
        return;
    }
    for entry in &snap.entries {
        let abs = dir_path.join(&entry.name);
        out.push((abs.clone(), entry.is_dir, depth, entry.is_symlink));
        if entry.is_dir && expanded_dirs.contains(&abs) {
            if let Some(child_snap) = child_snapshots.get(&abs) {
                let child_depth = depth + 1;
                collect_visible_rows(
                    child_snap,
                    &abs,
                    child_depth,
                    expanded_dirs,
                    child_snapshots,
                    out,
                );
                // Y4: empty dir indicator — if the expanded dir has no children and
                // we are within the depth limit, push a sentinel placeholder row.
                if child_snap.entries.is_empty() && child_depth < MAX_RENDER_DEPTH {
                    out.push((abs.join("\x00empty"), false, child_depth, false));
                }
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
    ui_painter: &mut dyn UiTextPainter,
    metrics: FontMetrics,
    theme: &Theme,
    outline: Option<&[OutlineRow]>,
    rect: Rect,
    hits: &mut LeftDockHits,
    rm: &RowMetrics,
    collapsed: bool,
) {
    let cell_h = metrics.cell_h;
    let cell_w = metrics.cell_w;
    let header_h = rm.header_h;
    let row_h = rm.row_h;
    let pad_x = rm.pad_x;

    // ── Header row ────────────────────────────────────────────────────────────
    // #9: add a hit for the outline header so clicks can toggle collapse.
    hits.hits.push(LeftDockHit {
        rect: Rect {
            x: rect.x,
            y: rect.y,
            w: rect.w,
            h: header_h.min(rect.h),
        },
        kind: LeftDockHitKind::OutlineHeader,
    });
    // Item 1: "OUTLINE" header always at text_muted (same as EXPLORER), regular.
    // Item 10 accent_bright was removed; text_muted keeps both headers visually
    // consistent and readable without competing with content.
    let header_color = theme.text_muted;
    let outline_header_icon_top =
        rect.y + ((header_h - cell_h) * 0.5 + metrics.descent * 0.5).max(0.0);
    let outline_header_baseline = outline_header_icon_top + (cell_h - metrics.descent);
    // #9: collapse chevron — keep on mono path (Nerd Font / Unicode glyph).
    let chevron = if collapsed { '▸' } else { '▾' };
    raster.glyph_at(
        painter,
        metrics,
        rect.x + pad_x,
        outline_header_icon_top,
        chevron as u32,
        theme.text_subtle,
    );
    raster.ui_line(
        ui_painter,
        "OUTLINE",
        rect.x + pad_x + cell_w * 1.5,
        outline_header_baseline,
        EXPLORER_HEADER_PT,
        UiWeight::Regular,
        header_color,
    );
    raster.fill_pixel_rect_alpha(
        rect.x,
        rect.y + header_h - 1.0,
        rect.w,
        1.0,
        theme.hairline,
        0.70,
    );

    // #9: collapsed — only header shown.
    if collapsed {
        return;
    }

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
                let outline_icon_top =
                    row_top + ((row_h - cell_h) * 0.5 + metrics.descent * 0.5).max(0.0);
                let outline_baseline = outline_icon_top + (cell_h - metrics.descent);

                // Indent: 2 cells per depth level.
                let outline_indent_cells = row.depth as usize * 2;
                let outline_indent_px = outline_indent_cells as f64 * cell_w;
                let x_start = rect.x + pad_x + outline_indent_px;
                let x_max = rect.x + rect.w - pad_x;

                // Kind glyph — special Unicode/Nerd Font char, keep on mono path.
                let glyph_ch = outline_kind_glyph(row.kind);
                if let Some(ch) = glyph_ch.chars().next() {
                    raster.glyph_at(
                        painter,
                        metrics,
                        x_start,
                        outline_icon_top,
                        ch as u32,
                        theme.accent_primary,
                    );
                }

                // Name: one cell after the glyph + one space gap.
                let name_x = x_start + cell_w * 2.0;
                let avail_name_w = (x_max - name_x).max(0.0);
                let truncated = ui_truncate(
                    ui_painter,
                    &row.name,
                    avail_name_w,
                    EXPLORER_ROW_PT,
                    UiWeight::Regular,
                );
                raster.ui_line(
                    ui_painter,
                    &truncated,
                    name_x,
                    outline_baseline,
                    EXPLORER_ROW_PT,
                    UiWeight::Regular,
                    theme.text_muted,
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
    match name {
        "Cargo.toml" => return (ICON_COG, theme.attention),
        "Cargo.lock" => return (ICON_LOCK, theme.text_muted),
        "README.md" | "README.MD" | "readme.md" => {
            return (ICON_FILE_TEXT, theme.accent_bright);
        }
        _ => {}
    }
    let ext = name.rfind('.').map(|i| &name[i + 1..]).unwrap_or("");
    match ext {
        "rs" => (ICON_FILE_CODE, theme.attention),
        "md" | "markdown" | "txt" => (ICON_FILE_TEXT, theme.text_muted),
        "toml" | "yaml" | "yml" => (ICON_COG, theme.text_muted),
        "json" | "html" | "htm" | "css" | "js" | "jsx" | "ts" | "tsx" | "py" | "go" | "zig"
        | "c" | "h" | "cpp" | "hpp" | "swift" | "sh" | "bash" | "zsh" => {
            (ICON_FILE_CODE, theme.text_muted)
        }
        "lock" => (ICON_LOCK, theme.text_muted),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "ico" => {
            (ICON_FILE_IMAGE, theme.text_muted)
        }
        "zip" | "gz" | "tgz" | "tar" | "bz2" | "xz" | "7z" => (ICON_FILE_ARCHIVE, theme.text_muted),
        _ => (ICON_FILE, theme.text_muted),
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

/// Truncate `name` to fit within `max_px` device pixels using the UI text path,
/// appending `…` if clipped.  Returns the display string unchanged when it fits.
fn ui_truncate(
    ui_painter: &mut dyn UiTextPainter,
    name: &str,
    max_px: f64,
    size_pt: f64,
    weight: UiWeight,
) -> String {
    if name.is_empty() || max_px <= 0.0 {
        return String::new();
    }
    let full_w = ui_painter.measure(name, size_pt, weight);
    if full_w <= max_px {
        return name.to_string();
    }
    // Reserve space for the ellipsis.
    let ellipsis_w = ui_painter.measure("\u{2026}", size_pt, weight);
    let budget = (max_px - ellipsis_w).max(0.0);
    let mut acc = 0.0;
    let mut cut = 0;
    for ch in name.chars() {
        let cw = ui_painter.measure(&ch.to_string(), size_pt, weight);
        if acc + cw > budget {
            break;
        }
        acc += cw;
        cut += ch.len_utf8();
    }
    format!("{}\u{2026}", &name[..cut])
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raster::{PixelRect, UiWeight, pixel_at};

    #[derive(Default)]
    struct StubPainter {
        pub glyphs: Vec<(u32, [u8; 3])>,
        pub glyph_positions: Vec<(u32, PixelRect)>,
    }

    impl GlyphPainter for StubPainter {
        #[allow(clippy::too_many_arguments)]
        fn draw_glyph(
            &mut self,
            glyph_id: u32,
            dest: PixelRect,
            fg: [u8; 3],
            _metrics: FontMetrics,
            _pixels: &mut [u8],
            _bw: usize,
            _bh: usize,
        ) {
            self.glyphs.push((glyph_id, fg));
            self.glyph_positions.push((glyph_id, dest));
        }
    }

    /// Stub UI text painter — records (text, color) of every draw_line call.
    #[derive(Default)]
    struct StubUiPainter {
        pub draws: Vec<(String, [u8; 3])>,
        pub attrs: Vec<(String, f64, UiWeight, [u8; 3])>,
        pub positions: Vec<(String, f64, f64)>,
    }

    impl UiTextPainter for StubUiPainter {
        fn measure(&mut self, text: &str, _size_pt: f64, _weight: UiWeight) -> f64 {
            // Return a fixed 8px per char so width math is predictable.
            text.chars().count() as f64 * 8.0
        }

        #[allow(clippy::too_many_arguments)]
        fn draw_line(
            &mut self,
            text: &str,
            x_px: f64,
            baseline_y_px: f64,
            size_pt: f64,
            weight: UiWeight,
            fg: [u8; 3],
            _pixels: &mut [u8],
            _bitmap_w: usize,
            _bitmap_h: usize,
        ) {
            if !text.is_empty() {
                self.draws.push((text.to_string(), fg));
                self.attrs.push((text.to_string(), size_pt, weight, fg));
                self.positions.push((text.to_string(), x_px, baseline_y_px));
            }
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

    #[test]
    fn explorer_row_metrics_reserve_glyph_breathing_room() {
        let m = FontMetrics {
            cell_w: 11.0,
            cell_h: 28.0,
            descent: 6.0,
        };
        let rm = RowMetrics::from_scale(1.0, m);

        assert!(
            rm.header_h >= m.cell_h + 6.0,
            "header_h={} must fit a {}px glyph plus breathing room",
            rm.header_h,
            m.cell_h
        );
        assert!(
            rm.row_h >= m.cell_h + 6.0,
            "row_h={} must fit a {}px glyph plus breathing room",
            rm.row_h,
            m.cell_h
        );
    }

    #[test]
    fn explorer_row_metrics_default_to_dense_twenty_two_px_rows() {
        let rm = RowMetrics::from_scale(1.0, metrics());

        assert_eq!(rm.header_h, 22.0);
        assert_eq!(rm.row_h, 22.0);
        assert_eq!(rm.pad_x, 8.0);
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
        let mut up = StubUiPainter::default();
        let zero = Rect {
            x: 0.0,
            y: 0.0,
            w: 0.0,
            h: 0.0,
        };
        draw_left_dock(&mut r, &mut p, &mut up, m, &th, None, None, None, zero);
        // No panic = pass.
    }

    /// No snapshot → "Waiting" text painted in text_muted.
    #[test]
    fn no_snapshot_waiting_text_painted() {
        let m = metrics();
        let th = theme();
        let mut r = Raster::new(800, 800);
        let mut p = StubPainter::default();
        let mut up = StubUiPainter::default();

        draw_left_dock(
            &mut r,
            &mut p,
            &mut up,
            m,
            &th,
            None,
            None,
            None,
            dock_rect(),
        );

        // "Waiting for shell prompt…" now rendered via UiTextPainter.
        let waiting: Vec<_> = up
            .draws
            .iter()
            .filter(|(text, fg)| text.contains("Waiting") && *fg == th.text_muted)
            .collect();
        assert!(
            !waiting.is_empty(),
            "expected Waiting text in text_muted via ui_painter"
        );
    }

    /// Empty snapshot → "(empty)" row painted.
    #[test]
    fn empty_snapshot_empty_row_painted() {
        let m = metrics();
        let th = theme();
        let mut r = Raster::new(800, 800);
        let mut p = StubPainter::default();
        let mut up = StubUiPainter::default();

        let snap = DirSnapshot {
            root: "/anvil".to_string(),
            entries: vec![],
            git_marks: Default::default(),
        };
        draw_left_dock(
            &mut r,
            &mut p,
            &mut up,
            m,
            &th,
            Some(&snap),
            None,
            None,
            dock_rect(),
        );

        // "(empty)" now rendered via UiTextPainter.
        let empty_draw: Vec<_> = up
            .draws
            .iter()
            .filter(|(text, fg)| text.contains("(empty)") && *fg == th.text_muted)
            .collect();
        assert!(
            !empty_draw.is_empty(),
            "expected '(empty)' in text_muted via ui_painter for empty state"
        );
    }

    /// Snapshot with entries → one header hit + one row hit per visible entry.
    #[test]
    fn explorer_rows_return_click_hits() {
        let m = metrics();
        let th = theme();
        let mut r = Raster::new(800, 800);
        let mut p = StubPainter::default();
        let mut up = StubUiPainter::default();

        let snap = DirSnapshot {
            root: "/anvil".to_string(),
            entries: vec![
                DirEntry {
                    name: "src".to_string(),
                    is_dir: true,
                    is_symlink: false,
                },
                DirEntry {
                    name: "main.rs".to_string(),
                    is_dir: false,
                    is_symlink: false,
                },
            ],
            git_marks: Default::default(),
        };

        let hits = draw_left_dock(
            &mut r,
            &mut p,
            &mut up,
            m,
            &th,
            Some(&snap),
            None,
            None,
            dock_rect(),
        );

        assert_eq!(
            hits.at(12.0, 18.0),
            Some(&LeftDockHitKind::Explorer(ExplorerHit::Header))
        );
        assert_eq!(
            hits.at(12.0, 36.0),
            Some(&LeftDockHitKind::Explorer(ExplorerHit::Row(0)))
        );
        assert_eq!(
            hits.at(12.0, 56.0),
            Some(&LeftDockHitKind::Explorer(ExplorerHit::Row(1)))
        );
    }

    #[test]
    fn explorer_rows_have_mouse_sized_full_width_targets() {
        let m = metrics();
        let th = theme();
        let mut r = Raster::new(800, 800);
        let mut p = StubPainter::default();
        let mut up = StubUiPainter::default();

        let snap = DirSnapshot {
            root: "/anvil".to_string(),
            entries: vec![DirEntry {
                name: "main.rs".to_string(),
                is_dir: false,
                is_symlink: false,
            }],
            git_marks: Default::default(),
        };

        let hits = draw_left_dock(
            &mut r,
            &mut p,
            &mut up,
            m,
            &th,
            Some(&snap),
            None,
            None,
            dock_rect(),
        );
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

    #[test]
    fn explorer_rows_are_compact_but_still_clickable() {
        let m = metrics();
        let th = anvil_theme::MINERAL_DARK;
        let mut r = Raster::new(800, 800);
        let mut p = StubPainter::default();
        let mut up = StubUiPainter::default();

        let snap = DirSnapshot {
            root: "/anvil".to_string(),
            entries: vec![DirEntry {
                name: "main.rs".to_string(),
                is_dir: false,
                is_symlink: false,
            }],
            git_marks: Default::default(),
        };

        let hits = draw_left_dock(
            &mut r,
            &mut p,
            &mut up,
            m,
            &th,
            Some(&snap),
            None,
            None,
            dock_rect(),
        );
        let row_hit = hits
            .hits
            .iter()
            .find(|hit| hit.kind == LeftDockHitKind::Explorer(ExplorerHit::Row(0)))
            .expect("row hit region should be emitted for visible explorer row");

        assert!(
            row_hit.rect.h <= 22.0,
            "explorer rows should be compact like an editor file tree, got height {}",
            row_hit.rect.h
        );
        assert!(
            row_hit.rect.h >= 20.0,
            "compact explorer rows still need reliable mouse targets"
        );
    }

    /// Snapshot with entries → file names appear in text_muted, dirs in text_subtle.
    #[test]
    fn explorer_scroll_offset_preserves_original_row_indices() {
        let m = metrics();
        let th = theme();
        let mut r = Raster::new(800, 800);
        let mut p = StubPainter::default();
        let mut up = StubUiPainter::default();

        // Use 25 entries: with ROW_H=22 and explorer_h=480px, available_rows=20,
        // so 25 entries give 5 overflow rows and scroll_offset=1 is not clamped.
        // Entry[1] appears in visible slot 0 at y=[28, 50). Hit at y=36 → Row(1).
        let snap = DirSnapshot {
            root: "/anvil".to_string(),
            entries: (0..25)
                .map(|i| DirEntry {
                    name: format!("file-{i}.rs"),
                    is_dir: false,
                    is_symlink: false,
                })
                .collect(),
            git_marks: Default::default(),
        };

        let hits = draw_left_dock_with_scroll(
            &mut r,
            &mut p,
            &mut up,
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
            false,
            false,
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
        let mut up = StubUiPainter::default();

        let snap = DirSnapshot {
            root: "/anvil".to_string(),
            entries: vec![
                DirEntry {
                    name: "src".to_string(),
                    is_dir: true,
                    is_symlink: false,
                },
                DirEntry {
                    name: "main.rs".to_string(),
                    is_dir: false,
                    is_symlink: false,
                },
            ],
            git_marks: Default::default(),
        };
        draw_left_dock(
            &mut r,
            &mut p,
            &mut up,
            m,
            &th,
            Some(&snap),
            None,
            None,
            dock_rect(),
        );

        // P4: file label "main.rs" rendered via UiTextPainter in text_muted
        // (quieter leaf-node style; selected files use accent_primary).
        let file_label: Vec<_> = up
            .draws
            .iter()
            .filter(|(text, fg)| text.contains("main.rs") && *fg == th.text_muted)
            .collect();
        assert!(
            !file_label.is_empty(),
            "expected 'main.rs' in text_muted via ui_painter for file entry label"
        );

        // Plain file rows render a compact file icon before the label. Git
        // badges use a separate lane so icons never crash into names.
        let file_icon_cp = ICON_FILE_CODE as u32;
        let file_icon: Vec<_> = p
            .glyphs
            .iter()
            .filter(|(cp, fg)| *cp == file_icon_cp && *fg == th.attention)
            .collect();
        assert!(
            !file_icon.is_empty(),
            "expected Rust file row to paint a compact file icon before the label"
        );

        // Inactive directory chevrons stay muted; selection owns the accent.
        let dir_chevron: Vec<_> = p
            .glyphs
            .iter()
            .filter(|(cp, fg)| *cp == '\u{25B8}' as u32 && *fg == th.text_subtle)
            .collect();
        assert!(
            !dir_chevron.is_empty(),
            "expected inactive dir chevron ▸ in text_subtle"
        );
    }

    #[test]
    fn active_file_path_marks_matching_explorer_row_selected() {
        let m = metrics();
        let th = theme();
        let mut r = Raster::new(800, 800);
        let mut p = StubPainter::default();
        let mut up = StubUiPainter::default();

        let snap = DirSnapshot {
            root: "/anvil/src".to_string(),
            entries: vec![
                DirEntry {
                    name: "editor.rs".to_string(),
                    is_dir: false,
                    is_symlink: false,
                },
                DirEntry {
                    name: "main.rs".to_string(),
                    is_dir: false,
                    is_symlink: false,
                },
            ],
            git_marks: Default::default(),
        };

        draw_left_dock(
            &mut r,
            &mut p,
            &mut up,
            m,
            &th,
            Some(&snap),
            Some(Path::new("/anvil/src/main.rs")),
            None,
            dock_rect(),
        );

        // Selected row is signaled through subtle row chrome and foreground text;
        // inactive files remain muted.
        let selected_label: Vec<_> = up
            .draws
            .iter()
            .filter(|(text, fg)| text == "main.rs" && *fg == th.foreground)
            .collect();
        assert_eq!(
            selected_label.len(),
            1,
            "active file label should paint once in foreground"
        );

        let inactive_label: Vec<_> = up
            .draws
            .iter()
            .filter(|(text, fg)| text == "editor.rs" && *fg == th.text_muted)
            .collect();
        assert_eq!(
            inactive_label.len(),
            1,
            "inactive file label should paint once in text_muted"
        );
    }

    /// Hover paints a translucent lift only when the row is not selected.
    /// Selected row suppresses hover rendering and keeps the accent rail.
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
                    is_symlink: false,
                },
                DirEntry {
                    name: "bar.rs".to_string(),
                    is_dir: false,
                    is_symlink: false,
                },
            ],
            git_marks: Default::default(),
        };

        // Row 0 is hovered, not selected — should lift from the base glass
        // without becoming a solid surface_alt block.
        {
            let mut base = Raster::new(800, 800);
            base.clear(th.charcoal);
            let mut base_p = StubPainter::default();
            let mut base_up = StubUiPainter::default();
            draw_left_dock_with_scroll(
                &mut base,
                &mut base_p,
                &mut base_up,
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
                false,
                false,
            );

            let mut r = Raster::new(800, 800);
            r.clear(th.charcoal);
            let mut p = StubPainter::default();
            let mut up = StubUiPainter::default();
            draw_left_dock_with_scroll(
                &mut r,
                &mut p,
                &mut up,
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
                false,
                false,
            );
            // Sample the fill interior.
            let base_px = pixel_at(&base, 50, 38);
            let px = pixel_at(&r, 50, 38);
            assert_ne!(
                px, base_px,
                "hovered non-selected row should visibly lift from the base glass"
            );
            assert_ne!(
                px, th.surface_alt,
                "hovered non-selected row should not be a solid surface_alt block, got {px:?}"
            );
        }

        // Row 0 is both hovered AND selected — selected wins, no plain panel hover.
        // The left-rail pixel (x=6+1=7) should be accent_primary, not just panel.
        {
            let mut r = Raster::new(800, 800);
            r.clear(th.charcoal);
            let mut p = StubPainter::default();
            let mut up = StubUiPainter::default();
            draw_left_dock_with_scroll(
                &mut r,
                &mut p,
                &mut up,
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
                false,
                false,
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

    #[test]
    fn explorer_row_chrome_is_translucent_not_blocky() {
        let m = metrics();
        let th = anvil_theme::MINERAL_DARK;

        let snap = DirSnapshot {
            root: "/anvil/src".to_string(),
            entries: vec![DirEntry {
                name: "foo.rs".to_string(),
                is_dir: false,
                is_symlink: false,
            }],
            git_marks: Default::default(),
        };

        let mut hovered = Raster::new(800, 800);
        hovered.clear(th.charcoal);
        let mut hovered_painter = StubPainter::default();
        let mut hovered_ui = StubUiPainter::default();
        draw_left_dock_with_scroll(
            &mut hovered,
            &mut hovered_painter,
            &mut hovered_ui,
            m,
            &th,
            Some(&snap),
            None,
            None,
            dock_rect(),
            0,
            Some(0),
            &HashSet::new(),
            &HashMap::new(),
            0.0,
            1.0,
            None,
            false,
            false,
        );
        let hover_px = pixel_at(&hovered, 50, 30);
        assert_ne!(
            hover_px, th.surface_alt,
            "hover row should be a translucent lift, not a solid surface_alt block"
        );

        let mut selected = Raster::new(800, 800);
        selected.clear(th.charcoal);
        let mut selected_painter = StubPainter::default();
        let mut selected_ui = StubUiPainter::default();
        draw_left_dock_with_scroll(
            &mut selected,
            &mut selected_painter,
            &mut selected_ui,
            m,
            &th,
            Some(&snap),
            Some(Path::new("/anvil/src/foo.rs")),
            None,
            dock_rect(),
            0,
            Some(0),
            &HashSet::new(),
            &HashMap::new(),
            0.0,
            1.0,
            None,
            false,
            false,
        );
        let selected_px = pixel_at(&selected, 50, 30);
        assert_ne!(
            selected_px, th.panel,
            "selected row should not be a solid panel block"
        );
        assert_eq!(
            pixel_at(&selected, 6, 30),
            th.accent_primary,
            "selected row should retain a crisp accent rail"
        );
    }

    /// Outline section with `None` shows only the header row (no body copy).
    #[test]
    fn outline_unavailable_always_shown() {
        let m = metrics();
        let th = theme();
        let mut r = Raster::new(800, 800);
        let mut p = StubPainter::default();
        let mut up = StubUiPainter::default();

        draw_left_dock(
            &mut r,
            &mut p,
            &mut up,
            m,
            &th,
            None,
            None,
            None,
            dock_rect(),
        );

        // Item 1: "OUTLINE" header now rendered via UiTextPainter in text_muted.
        let outline_muted: Vec<_> = up
            .draws
            .iter()
            .filter(|(text, fg)| *text == "OUTLINE" && *fg == th.text_muted)
            .collect();
        assert!(
            !outline_muted.is_empty(),
            "expected OUTLINE header in text_muted via ui_painter in empty state"
        );

        // Item 10: body copy removed — no draw call containing "source" should exist.
        let source_body: Vec<_> = up
            .draws
            .iter()
            .filter(|(text, _)| text.contains("source"))
            .collect();
        assert!(
            source_body.is_empty(),
            "body copy 'Open a source file' must not render when outline is None"
        );
    }

    /// Background is a neutral glass-style Mineral panel, not an Ember block.
    #[test]
    fn background_is_quiet_mineral_sidebar() {
        let m = metrics();
        let th = anvil_theme::MINERAL_DARK;
        let mut r = Raster::new(800, 800);
        r.clear([0, 0, 0]);
        let mut p = StubPainter::default();
        let mut up = StubUiPainter::default();

        draw_left_dock(
            &mut r,
            &mut p,
            &mut up,
            m,
            &th,
            None,
            None,
            None,
            dock_rect(),
        );

        let px = pixel_at(&r, 50, 400); // middle of dock
        assert_ne!(
            px,
            [27, 18, 13],
            "sidebar must not regress to red/brown block"
        );
        assert!(
            px[0] > th.graphite[0] && px[0] < th.surface[0],
            "glass fill should blend between graphite and surface: {px:?}"
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
        let mut up = StubUiPainter::default();

        draw_left_dock(
            &mut r,
            &mut p,
            &mut up,
            m,
            &th,
            None,
            None,
            Some(&[]),
            dock_rect(),
        );

        // Item 1: "OUTLINE" header now rendered via UiTextPainter in text_muted.
        let outline_muted: Vec<_> = up
            .draws
            .iter()
            .filter(|(text, fg)| *text == "OUTLINE" && *fg == th.text_muted)
            .collect();
        assert!(
            !outline_muted.is_empty(),
            "expected OUTLINE header in text_muted via ui_painter when Some(&[])"
        );

        // No body copy — no draw call containing "symbols" should exist.
        let symbols_body: Vec<_> = up
            .draws
            .iter()
            .filter(|(text, _)| text.contains("symbols"))
            .collect();
        assert!(
            symbols_body.is_empty(),
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
        let mut up = StubUiPainter::default();

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
        draw_left_dock(
            &mut r,
            &mut p,
            &mut up,
            m,
            &th,
            None,
            None,
            Some(&rows),
            dock_rect(),
        );

        // "my_fn" symbol name now rendered via UiTextPainter in text_muted.
        let fn_label: Vec<_> = up
            .draws
            .iter()
            .filter(|(text, fg)| text.contains("my_fn") && *fg == th.text_muted)
            .collect();
        assert!(
            !fn_label.is_empty(),
            "expected 'my_fn' in text_muted via ui_painter for function symbol name"
        );

        // ƒ glyph (0x0192) stays on mono path in accent_primary.
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

    /// Item 1: OUTLINE header uses text_muted at all times.
    /// Uses 'U' which appears only in "OUTLINE" (not in "EXPLORER" or waiting text).
    #[test]
    fn outline_empty_header_uses_text_muted() {
        let m = metrics();
        let th = theme();
        let mut r = Raster::new(800, 800);
        let mut p = StubPainter::default();
        let mut up = StubUiPainter::default();

        draw_left_dock(
            &mut r,
            &mut p,
            &mut up,
            m,
            &th,
            None,
            None,
            None,
            dock_rect(),
        );

        // Item 1: "OUTLINE" header rendered via UiTextPainter in text_muted.
        let outline_muted: Vec<_> = up
            .draws
            .iter()
            .filter(|(text, fg)| *text == "OUTLINE" && *fg == th.text_muted)
            .collect();
        assert!(
            !outline_muted.is_empty(),
            "OUTLINE header must be in text_muted via ui_painter"
        );

        // "OUTLINE" must NOT appear in accent_bright.
        let outline_bright: Vec<_> = up
            .draws
            .iter()
            .filter(|(text, fg)| *text == "OUTLINE" && *fg == th.accent_bright)
            .collect();
        assert!(
            outline_bright.is_empty(),
            "OUTLINE header must NOT use accent_bright"
        );
    }

    #[test]
    fn section_headers_use_regular_quiet_chrome_weight() {
        let m = metrics();
        let th = theme();
        let mut r = Raster::new(800, 800);
        let mut p = StubPainter::default();
        let mut up = StubUiPainter::default();

        draw_left_dock(
            &mut r,
            &mut p,
            &mut up,
            m,
            &th,
            None,
            None,
            None,
            dock_rect(),
        );

        let explorer = up
            .attrs
            .iter()
            .find(|(text, _, _, _)| text == "EXPLORER")
            .expect("EXPLORER header should render");
        let outline = up
            .attrs
            .iter()
            .find(|(text, _, _, _)| text == "OUTLINE")
            .expect("OUTLINE header should render");

        assert_eq!(explorer.2, UiWeight::Regular);
        assert_eq!(outline.2, UiWeight::Regular);
        assert_eq!(explorer.3, th.text_muted);
        assert_eq!(outline.3, th.text_muted);
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
                    is_symlink: false,
                })
                .collect(),
            git_marks: Default::default(),
        };

        // At offset=0: first content row (y ≈ header_h + row_h/2 = 32+14 = 46; y=39 is also in [32,60)) → entry 0.
        let hits_at_0 = {
            let mut r = Raster::new(800, 800);
            let mut p = StubPainter::default();
            let mut up = StubUiPainter::default();
            draw_left_dock_with_scroll(
                &mut r,
                &mut p,
                &mut up,
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
                false,
                false,
            )
        };

        // At offset=5: same pixel maps to entry 5.
        let hits_at_5 = {
            let mut r = Raster::new(800, 800);
            let mut p = StubPainter::default();
            let mut up = StubUiPainter::default();
            draw_left_dock_with_scroll(
                &mut r,
                &mut p,
                &mut up,
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
                false,
                false,
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
        let mut up = StubUiPainter::default();

        let root_snap = DirSnapshot {
            root: "/project".to_string(),
            entries: vec![
                DirEntry {
                    name: "src".to_string(),
                    is_dir: true,
                    is_symlink: false,
                },
                DirEntry {
                    name: "Cargo.toml".to_string(),
                    is_dir: false,
                    is_symlink: false,
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
                    is_symlink: false,
                },
                DirEntry {
                    name: "lib.rs".to_string(),
                    is_dir: false,
                    is_symlink: false,
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
            &mut up,
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
            false,
            false,
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
                        is_symlink: false,
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
                        is_symlink: false,
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

    /// A1: no Unicode box-drawing connector glyphs emitted at any depth.
    /// Indent guides are now 1px pixel rects (not glyphs), so the glyph
    /// painter should see zero connector codepoints regardless of nesting.
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
                    is_symlink: false,
                },
                DirEntry {
                    name: "main.rs".to_string(),
                    is_dir: false,
                    is_symlink: false,
                },
            ],
            git_marks: Default::default(),
        };
        let src_snap = DirSnapshot {
            root: "/p/src".to_string(),
            entries: vec![DirEntry {
                name: "lib.rs".to_string(),
                is_dir: false,
                is_symlink: false,
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
        let mut up = StubUiPainter::default();
        draw_left_dock_with_scroll(
            &mut r,
            &mut p,
            &mut up,
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
            false,
            false,
        );

        // Indent guides are now pixel rects, not glyphs. Neither depth-0 nor
        // depth-1 rows should emit any Unicode box-drawing connector codepoints.
        let connector_count = p
            .glyphs
            .iter()
            .filter(|(cp, _)| CONNECTORS.contains(cp))
            .count();
        assert_eq!(
            connector_count, 0,
            "indent guides are pixel rects; no connector glyphs expected (depth-1 row included)"
        );

        // Same for flat (all depth-0) layout.
        let mut r2 = Raster::new(800, 800);
        let mut p2 = StubPainter::default();
        let mut up2 = StubUiPainter::default();
        draw_left_dock(
            &mut r2,
            &mut p2,
            &mut up2,
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

    #[test]
    fn file_icon_colored_returns_nerd_font_glyphs() {
        let th = theme();

        assert_eq!(
            file_icon_colored("main.rs", &th),
            (ICON_FILE_CODE, th.attention)
        );
        assert_eq!(
            file_icon_colored("README.md", &th),
            (ICON_FILE_TEXT, th.accent_bright)
        );
        assert_eq!(
            file_icon_colored("Cargo.toml", &th),
            (ICON_COG, th.attention)
        );
        assert_eq!(
            file_icon_colored("archive.tar", &th),
            (ICON_FILE_ARCHIVE, th.text_muted)
        );
        assert_eq!(file_icon_colored("binary", &th), (ICON_FILE, th.text_muted));
    }

    // ── G1: icon-only and hide modes ──────────────────────────────────────────

    /// G1: rect.w < 60pt (hide threshold) returns empty hits, no panic.
    #[test]
    fn very_narrow_dock_hides_entirely() {
        let m = metrics();
        let th = theme();
        let mut r = Raster::new(800, 800);
        let mut p = StubPainter::default();
        let mut up = StubUiPainter::default();
        let snap = DirSnapshot {
            root: "/p".to_string(),
            entries: vec![DirEntry {
                name: "main.rs".to_string(),
                is_dir: false,
                is_symlink: false,
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
            &mut up,
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
            false,
            false,
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
        let mut up = StubUiPainter::default();
        let snap = DirSnapshot {
            root: "/p".to_string(),
            entries: vec![
                DirEntry {
                    name: "a.rs".to_string(),
                    is_dir: false,
                    is_symlink: false,
                },
                DirEntry {
                    name: "b.rs".to_string(),
                    is_dir: false,
                    is_symlink: false,
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
            &mut up,
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
            false,
            false,
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
        let mut up = StubUiPainter::default();

        let mut git_marks = HashMap::new();
        git_marks.insert("main.rs".to_string(), 'M');
        let snap = DirSnapshot {
            root: "/anvil/src".to_string(),
            entries: vec![DirEntry {
                name: "main.rs".to_string(),
                is_dir: false,
                is_symlink: false,
            }],
            git_marks,
        };

        draw_left_dock(
            &mut r,
            &mut p,
            &mut up,
            m,
            &th,
            Some(&snap),
            None,
            None,
            dock_rect(),
        );

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

    #[test]
    fn git_badge_keeps_clear_gap_before_label() {
        let m = metrics();
        let th = theme();
        let mut r = Raster::new(800, 800);
        let mut p = StubPainter::default();
        let mut up = StubUiPainter::default();

        let mut git_marks = HashMap::new();
        git_marks.insert("crates".to_string(), 'M');
        let snap = DirSnapshot {
            root: "/anvil".to_string(),
            entries: vec![DirEntry {
                name: "crates".to_string(),
                is_dir: true,
                is_symlink: false,
            }],
            git_marks,
        };

        draw_left_dock(
            &mut r,
            &mut p,
            &mut up,
            m,
            &th,
            Some(&snap),
            None,
            None,
            dock_rect(),
        );

        let badge_rect = p
            .glyph_positions
            .iter()
            .find_map(|(cp, rect)| (*cp == 'M' as u32).then_some(*rect))
            .expect("modified badge must render");
        let label_x = up
            .positions
            .iter()
            .find_map(|(text, x, _)| (text == "crates").then_some(*x))
            .expect("entry label must render");

        assert!(
            badge_rect.x + badge_rect.w + 3.0 <= label_x,
            "badge at x={} w={} must not collide with label_x={label_x}",
            badge_rect.x,
            badge_rect.w
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
        let mut up = StubUiPainter::default();

        let snap = DirSnapshot {
            root: "/project".to_string(),
            entries: vec![
                DirEntry {
                    name: "main.rs".to_string(),
                    is_dir: false,
                    is_symlink: false,
                },
                DirEntry {
                    name: "lib.rs".to_string(),
                    is_dir: false,
                    is_symlink: false,
                },
                DirEntry {
                    name: "README.md".to_string(),
                    is_dir: false,
                    is_symlink: false,
                },
            ],
            git_marks: Default::default(),
        };

        // Filter "main" — only main.rs should appear.
        let hits = draw_left_dock_with_scroll(
            &mut r,
            &mut p,
            &mut up,
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
            false,
            false,
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
        let mut up = StubUiPainter::default();

        let snap = DirSnapshot {
            root: "/project".to_string(),
            entries: vec![
                DirEntry {
                    name: "a.rs".to_string(),
                    is_dir: false,
                    is_symlink: false,
                },
                DirEntry {
                    name: "b.rs".to_string(),
                    is_dir: false,
                    is_symlink: false,
                },
            ],
            git_marks: Default::default(),
        };

        let hits = draw_left_dock_with_scroll(
            &mut r,
            &mut p,
            &mut up,
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
            false,
            false,
        );

        assert_eq!(
            hits.visible_rows.len(),
            2,
            "no filter: all 2 rows must be visible"
        );
    }

    // ── Y3: symlink indicator ─────────────────────────────────────────────────

    /// Y3: a symlink file entry must render the → glyph (U+2192) in text_subtle.
    #[test]
    fn symlink_entry_renders_arrow_glyph() {
        let m = metrics();
        let th = theme();
        let mut r = Raster::new(800, 800);
        let mut p = StubPainter::default();
        let mut up = StubUiPainter::default();

        let snap = DirSnapshot {
            root: "/project".to_string(),
            entries: vec![DirEntry {
                name: "link_to_file.rs".to_string(),
                is_dir: false,
                is_symlink: true,
            }],
            git_marks: Default::default(),
        };

        draw_left_dock(
            &mut r,
            &mut p,
            &mut up,
            m,
            &th,
            Some(&snap),
            None,
            None,
            dock_rect(),
        );

        // Arrow glyph → (U+2192) must appear in text_subtle.
        let arrow: Vec<_> = p
            .glyphs
            .iter()
            .filter(|(cp, fg)| *cp == '\u{2192}' as u32 && *fg == th.text_subtle)
            .collect();
        assert!(
            !arrow.is_empty(),
            "symlink entry must render → glyph (U+2192) in text_subtle (Y3)"
        );
    }

    // ── Y4: empty dir indicator ───────────────────────────────────────────────

    /// Y4: an expanded dir with no children must render "(empty)" in text_subtle.
    #[test]
    fn expanded_empty_dir_shows_empty_placeholder() {
        let m = metrics();
        let th = theme();
        let mut r = Raster::new(800, 800);
        let mut p = StubPainter::default();
        let mut up = StubUiPainter::default();

        let root_snap = DirSnapshot {
            root: "/project".to_string(),
            entries: vec![DirEntry {
                name: "empty_dir".to_string(),
                is_dir: true,
                is_symlink: false,
            }],
            git_marks: Default::default(),
        };
        let empty_child = DirSnapshot {
            root: "/project/empty_dir".to_string(),
            entries: vec![],
            git_marks: Default::default(),
        };
        let empty_dir_path = PathBuf::from("/project/empty_dir");

        let mut expanded = HashSet::new();
        expanded.insert(empty_dir_path.clone());
        let mut children = HashMap::new();
        children.insert(empty_dir_path, empty_child);

        draw_left_dock_with_scroll(
            &mut r,
            &mut p,
            &mut up,
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
            false,
            false,
        );

        // "(empty)" placeholder now rendered via UiTextPainter in text_subtle.
        let empty_draw: Vec<_> = up
            .draws
            .iter()
            .filter(|(text, fg)| text.contains("(empty)") && *fg == th.text_subtle)
            .collect();
        assert!(
            !empty_draw.is_empty(),
            "expanded empty dir must render '(empty)' in text_subtle via ui_painter (Y4)"
        );
    }

    // ── Y5: child count badge ─────────────────────────────────────────────────

    /// Y5: a collapsed dir whose children are cached shows "(N)" after the name.
    #[test]
    fn collapsed_dir_with_cached_children_shows_count_badge() {
        let m = metrics();
        let th = theme();
        let mut r = Raster::new(800, 800);
        let mut p = StubPainter::default();
        let mut up = StubUiPainter::default();

        let root_snap = DirSnapshot {
            root: "/project".to_string(),
            entries: vec![DirEntry {
                name: "src".to_string(),
                is_dir: true,
                is_symlink: false,
            }],
            git_marks: Default::default(),
        };
        let src_snap = DirSnapshot {
            root: "/project/src".to_string(),
            entries: vec![
                DirEntry {
                    name: "main.rs".to_string(),
                    is_dir: false,
                    is_symlink: false,
                },
                DirEntry {
                    name: "lib.rs".to_string(),
                    is_dir: false,
                    is_symlink: false,
                },
            ],
            git_marks: Default::default(),
        };
        let src_path = PathBuf::from("/project/src");

        // src is NOT expanded but IS cached (children known).
        let expanded = HashSet::new(); // empty — not expanded
        let mut children = HashMap::new();
        children.insert(src_path, src_snap);

        draw_left_dock_with_scroll(
            &mut r,
            &mut p,
            &mut up,
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
            false,
            false,
        );

        // Dir label "src (2)" now rendered via UiTextPainter in text_muted.
        let badge_draw: Vec<_> = up
            .draws
            .iter()
            .filter(|(text, fg)| {
                text.contains("src") && text.contains("(2)") && *fg == th.text_muted
            })
            .collect();
        assert!(
            !badge_draw.is_empty(),
            "collapsed dir with 2 cached children must render 'src (2)' in text_muted via ui_painter"
        );
    }
}
