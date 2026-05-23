//! GPU rendering pipeline: Metal, CoreText glyph rasterization, frame composition.
//!
//! Phase P4a port: `raster` and `draw` are pure-Rust modules.
//! Phase P4b port: panel renderers — workspace, tabbar, agent_panel, searchbar,
//! filetree, cheatsheet.
//! Phase P10a: `atlas` — `GlyphRasterizer` trait + `ShelfPacker` (infrastructure only).
//! Phase P10b: `batch` — `CellBatch` + `CellInstance` for GPU instance submission.

pub mod agent_panel;
pub mod atlas;
pub mod batch;
pub mod cheatsheet;
pub mod draw;
pub mod filetree;
pub mod raster;
pub mod searchbar;
pub mod statusbar;
pub mod tabbar;
pub mod workspace;

pub use atlas::{GlyphRasterizer, GlyphSlot, ShelfPacker};
pub use batch::{CellBatch, CellInstance};
pub use draw::{
    CursorConfig, CursorParams, CursorStyle, FoldedBlocks, cursor_opacity, draw_cell, draw_cursor,
    draw_viewport, draw_viewport_gpu, resolve_color, rule_row,
};
pub use raster::{FontMetrics, GlyphPainter, PixelRect, Raster};
pub use statusbar::STATUS_BAR_ROWS;
