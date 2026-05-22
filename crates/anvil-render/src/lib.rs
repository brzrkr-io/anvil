//! GPU rendering pipeline: Metal, CoreText glyph rasterization, frame composition.
//!
//! Phase P4a port: `raster` and `draw` are pure-Rust modules.
//! Phase P4b port: panel renderers — workspace, tabbar, agent_panel, searchbar,
//! filetree, cheatsheet.

pub mod agent_panel;
pub mod cheatsheet;
pub mod draw;
pub mod filetree;
pub mod raster;
pub mod searchbar;
pub mod tabbar;
pub mod workspace;

pub use draw::{
    CursorConfig, CursorParams, CursorStyle, cursor_opacity, draw_cell, draw_cursor, draw_viewport,
    resolve_color, rule_row,
};
pub use raster::{FontMetrics, GlyphPainter, PixelRect, Raster};
