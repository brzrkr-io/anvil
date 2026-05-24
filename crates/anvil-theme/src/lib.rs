//! Anvil theme system: Mineral palette, ANSI-16 mappings, WCAG contrast.
//!
//! This crate ports `src/render/color.zig` and `src/config/theme.zig`.

pub mod color;
pub mod theme;

// Flat re-exports for the most common types.
pub use color::{ClearColor, ColorError, hex_to_clear_color, hex_to_rgb, mix};
pub use theme::{
    AnsiOverrides, EMBER_DARK, EMBER_LIGHT, MINERAL_DARK, MINERAL_LIGHT, Theme, ThemeOverrides,
    by_name, contrast_ratio, resolve,
};
