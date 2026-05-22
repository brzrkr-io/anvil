//! GPU rendering pipeline: Metal, CoreText glyph rasterization, frame composition.
//!
//! Phase P4a port: `raster` and `draw` are pure-Rust modules.
//! Panel renderers (workspace, tabbar, agent_panel, searchbar, filetree,
//! cheatsheet) are deferred to the next phase.

pub mod draw;
pub mod raster;

pub use draw::{
    CursorConfig, CursorParams, CursorStyle, cursor_opacity, draw_cell, draw_cursor, draw_viewport,
    resolve_color, rule_row,
};
pub use raster::{FontMetrics, GlyphPainter, PixelRect, Raster};
