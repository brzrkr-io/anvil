//! macOS platform layer: AppKit window lifecycle, PTY process I/O,
//! Metal renderer, CoreText font, WKWebView bridge.

pub mod appkit;
pub mod font;
pub mod metal;
pub mod pty;
pub mod shell_integration;
pub mod webview;
