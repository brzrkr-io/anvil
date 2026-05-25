//! GPU rendering pipeline: Metal, CoreText glyph rasterization, frame composition.
//!
//! Phase P4a port: `raster` and `draw` are pure-Rust modules.
//! Phase P4b port: panel renderers — workspace, tabbar, agent_panel, searchbar,
//! cheatsheet.
//! Phase P10a: `atlas` — `GlyphRasterizer` trait + `ShelfPacker` (infrastructure only).
//! Phase P10b: `batch` — `CellBatch` + `CellInstance` for GPU instance submission.

pub mod agent_panel;
pub mod atlas;
pub mod batch;
pub mod cheatsheet;
pub mod context_bar;
pub mod draw;
pub mod left_dock;
pub mod raster;
pub mod searchbar;
pub mod statusbar;
pub mod tabbar;
pub mod workspace;

pub use atlas::{GlyphRasterizer, GlyphSlot, ShelfPacker};
pub use batch::{CellBatch, CellInstance};
pub use context_bar::draw_context_bar;
pub use draw::{
    CursorConfig, CursorParams, CursorStyle, FoldedBlocks, GridPainters, cursor_opacity, draw_cell,
    draw_cursor, draw_viewport, draw_viewport_gpu, resolve_color, rule_row,
};
pub use left_dock::{
    DirEntry as LeftDockEntry, DirSnapshot as LeftDockSnapshot, OutlineKind, OutlineRow,
    draw_left_dock,
};
pub use raster::{FontMetrics, GlyphPainter, PixelRect, Raster};
