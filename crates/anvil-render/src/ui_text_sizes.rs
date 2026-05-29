//! Per-surface UI font size constants (logical pt).
//!
//! All values are logical pt; `UiPainter` multiplies by `backing_scale`
//! before rasterizing.  See the per-surface table in the proportional font
//! spec (`context/2026-05-26-proportional-font-spec.md § Per-surface size table`).

/// Explorer file/directory row label — Regular weight.
pub const EXPLORER_ROW_PT: f64 = 11.0;

/// Explorer section header (e.g. "OPEN EDITORS") — Regular weight.
pub const EXPLORER_HEADER_PT: f64 = 10.0;

/// Editor tab label (active and inactive) — Medium (active) / Regular (inactive).
pub const TAB_LABEL_PT: f64 = 11.0;

/// Context bar / breadcrumb segment — Regular weight.
pub const CONTEXT_BAR_PT: f64 = 10.0;

/// Status bar text — Regular weight.
pub const STATUS_PT: f64 = 10.5;

/// Overlay card title — Semibold weight.
pub const OVERLAY_TITLE_PT: f64 = 14.0;

/// Overlay card body — Regular weight.
pub const OVERLAY_BODY_PT: f64 = 13.0;

/// Toast notification title — Medium weight.
pub const TOAST_TITLE_PT: f64 = 13.0;

/// Toast notification body — Regular weight.
pub const TOAST_BODY_PT: f64 = 12.0;

/// Tooltip — Regular weight.
pub const TOOLTIP_PT: f64 = 11.0;
