//! Unified card-overlay subsystem for Anvil.
//!
//! Provides an `OverlayStack` that manages layered overlays with animation,
//! keyboard/mouse routing, and chrome rendering. Three widget primitives cover
//! the common cases: `Picker`, `TextInput`, and `Tooltip`. Outliers use
//! `Custom`.
//!
//! See the overlay redesign spec (context/2026-05-26-overlay-redesign-spec.md).

pub mod anim;
pub mod chrome;
pub mod input;
pub mod text;
pub mod widgets;

use std::time::Instant;

pub use anim::{AnimState, OverlayAnim};
pub use chrome::{CardGeom, draw_card_chrome};
pub use input::{OverlayClickResult, OverlayInputRouter, OverlayKey, OverlayKeyResult};
pub use text::{MonoPainter, OverlayPainter, Weight};
pub use widgets::picker::PickerOverlay;
pub use widgets::text_input::TextInputOverlay;
pub use widgets::tooltip::TooltipOverlay;

use anvil_theme::Theme;

use crate::raster::{FontMetrics, GlyphPainter, Raster};

// ── Core types ────────────────────────────────────────────────────────────────

/// Stable identifier for each overlay kind.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum OverlayId {
    FilePicker,
    WorkspaceSymbols,
    BufferSymbols,
    ProjectSearch,
    ScmPanel,
    BranchSwitcher,
    GitLog,
    ThemePicker,
    LangPicker,
    Completion,
    CodeActions,
    LspReferences,
    LspRename,
    GotoLine,
    SaveAs,
    OpenFolder,
    ProjectSwitcher,
    Hover,
    BlameTip,
    FileTip,
}

/// Size of a rendered card (device pixels).
#[derive(Clone, Copy, Debug)]
pub struct CardSize {
    pub w: f64,
    pub h: f64,
}

/// Context passed to `CustomOverlay::measure`.
pub struct OverlayMeasureCtx {
    pub metrics: FontMetrics,
    /// Full drawable width in device pixels.
    pub dw: f64,
    /// Full drawable height in device pixels.
    pub dh: f64,
}

/// Context passed to `CustomOverlay::render`.
pub struct OverlayRenderCtx<'a> {
    pub raster: &'a mut Raster,
    pub painter: &'a mut dyn GlyphPainter,
    pub metrics: FontMetrics,
    pub theme: &'a Theme,
    pub geom: CardGeom,
}

/// Escape hatch for overlays that don't fit `Picker`, `TextInput`, or `Tooltip`.
pub trait CustomOverlay {
    fn id(&self) -> OverlayId;
    fn measure(&self, ctx: &OverlayMeasureCtx) -> CardSize;
    fn render(&self, ctx: &mut OverlayRenderCtx<'_>);
    fn handle_key(&mut self, key: OverlayKey) -> OverlayKeyResult;
    fn handle_click(&mut self, x: f64, y: f64) -> OverlayClickResult;
    fn close_on_blur(&self) -> bool {
        true
    }
}

/// The active overlay variant.
pub enum Overlay {
    Picker(PickerOverlay),
    TextInput(TextInputOverlay),
    Tooltip(TooltipOverlay),
    Custom(Box<dyn CustomOverlay>),
}

// ── OverlayEntry ──────────────────────────────────────────────────────────────

pub(crate) struct OverlayEntry {
    pub(crate) overlay: Overlay,
    pub(crate) anim: OverlayAnim,
    #[allow(dead_code)]
    pub(crate) opened_at: Instant,
}

// ── OverlayStack ──────────────────────────────────────────────────────────────

/// Manages a stack of active overlays. Only the top entry receives input;
/// rendering iterates bottom-up so stacked overlays compose.
pub struct OverlayStack {
    entries: Vec<OverlayEntry>,
}

impl OverlayStack {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Push an overlay onto the stack with a fresh animation.
    pub fn push(&mut self, overlay: Overlay) {
        self.entries.push(OverlayEntry {
            overlay,
            anim: OverlayAnim::new(),
            opened_at: Instant::now(),
        });
    }

    /// True when the stack has any entries (including those still animating out).
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// True when any entry is still animating (triggers redraw requests).
    pub fn animating(&self) -> bool {
        self.entries.iter().any(|e| e.anim.is_animating())
    }

    /// Advance animations by `dt_ms`. Returns true if any entry changed state.
    pub fn tick(&mut self, dt_ms: f64) -> bool {
        let mut changed = false;
        for e in &mut self.entries {
            if e.anim.tick(dt_ms) {
                changed = true;
            }
        }
        // GC: drop finished Leaving entries.
        self.gc();
        changed
    }

    /// Remove entries whose animation has finished in the Leaving state.
    pub fn gc(&mut self) {
        self.entries
            .retain(|e| !(matches!(e.anim.state, AnimState::Leaving) && e.anim.finished()));
    }

    /// Begin closing the top overlay (starts fade-out animation).
    pub fn close_top(&mut self) {
        if let Some(top) = self.entries.last_mut() {
            top.anim.begin_close();
        }
    }

    /// Immediately remove the top overlay without animation.
    pub fn pop(&mut self) {
        self.entries.pop();
    }

    /// Render all entries bottom-up.
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        raster: &mut Raster,
        painter: &mut dyn GlyphPainter,
        metrics: FontMetrics,
        theme: &Theme,
        dw: f64,
        dh: f64,
        chrome_top: f64,
        chrome_bot: f64,
    ) {
        for entry in &mut self.entries {
            let alpha = entry.anim.alpha();
            let scale = entry.anim.scale();
            match &entry.overlay {
                Overlay::Picker(p) => {
                    p.render(
                        raster, painter, metrics, theme, dw, dh, chrome_top, chrome_bot, alpha,
                        scale,
                    );
                }
                Overlay::TextInput(_ti) => {
                    // TODO: TextInputOverlay render (Phase 3)
                }
                Overlay::Tooltip(_tt) => {
                    // TODO: TooltipOverlay render (Phase 3)
                }
                Overlay::Custom(c) => {
                    let cw = metrics.cell_w;
                    let ch = metrics.cell_h;
                    let panel_w = (dw * 0.6).min(dw - 4.0 * cw).max(20.0 * cw);
                    let panel_h = dh * 0.5;
                    let geom = CardGeom {
                        x: ((dw - panel_w) * 0.5).max(0.0),
                        y: chrome_top + ch,
                        w: panel_w,
                        h: panel_h,
                        radius: 0.0,
                        padding: 2.0 * cw,
                        anim_scale: scale,
                        anim_alpha: alpha,
                    };
                    draw_card_chrome(raster, theme, geom, true);
                    let mut ctx = OverlayRenderCtx {
                        raster,
                        painter,
                        metrics,
                        theme,
                        geom,
                    };
                    c.render(&mut ctx);
                }
            }
        }
    }

    /// Route a key event to the top overlay. Returns the routing result.
    pub fn dispatch_key(&mut self, key: OverlayKey) -> OverlayKeyResult {
        OverlayInputRouter::dispatch_key(self, key)
    }

    /// Mutable ref to the top overlay entry, if any.
    fn top_mut(&mut self) -> Option<&mut OverlayEntry> {
        self.entries.last_mut()
    }

    /// Update the rows, query, and selection of the top `Picker` overlay.
    /// No-op if the top overlay is not a Picker.
    pub fn update_picker_top(
        &mut self,
        rows: Vec<widgets::picker::PickerRow>,
        query: String,
        selected: usize,
    ) {
        if let Some(entry) = self.entries.last_mut() {
            if let Overlay::Picker(ref mut p) = entry.overlay {
                p.rows = rows;
                p.query = query;
                p.selected = selected;
            }
        }
    }
}

impl Default for OverlayStack {
    fn default() -> Self {
        Self::new()
    }
}

// ── Internal: key dispatch helper used by OverlayInputRouter ─────────────────

impl OverlayStack {
    pub(crate) fn top_entry_mut(&mut self) -> Option<&mut OverlayEntry> {
        self.top_mut()
    }
}

// ── Submission returned on Enter/submit ──────────────────────────────────────

/// Data returned when an overlay submits (Enter pressed with a selection).
#[derive(Debug, Clone, PartialEq)]
pub enum Submission {
    PickerRow { id: OverlayId, index: usize },
    TextValue { id: OverlayId, value: String },
}
