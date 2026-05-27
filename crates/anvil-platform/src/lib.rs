//! macOS platform layer: AppKit window lifecycle, PTY process I/O,
//! Metal renderer, CoreText font, WKWebView bridge.

pub mod appkit;
pub mod font;
pub mod glyph_atlas;
pub mod metal;
pub mod pty;
pub mod shell_integration;
pub mod system;
pub mod ui_text;
pub mod webview;

pub use appkit::{ContextAction, CursorKind, RightClickZone};
pub use glyph_atlas::{AtlasError, AtlasPainter};
pub use ui_text::UiPainter;
