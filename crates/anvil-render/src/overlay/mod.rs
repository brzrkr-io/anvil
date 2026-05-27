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
    /// Override card position. When `Some((x, y))`, the stack places the card
    /// at that pixel origin instead of centering it. Used by anchored overlays
    /// (completion, hover) that must sit near the cursor.
    fn card_origin(
        &self,
        _size: CardSize,
        _dw: f64,
        _dh: f64,
        _chrome_top: f64,
    ) -> Option<(f64, f64)> {
        None
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
                Overlay::TextInput(ti) => {
                    ti.render(
                        raster, painter, metrics, theme, dw, dh, chrome_top, alpha, scale,
                    );
                }
                Overlay::Tooltip(tt) => {
                    tt.render(raster, painter, metrics, theme, dw, dh, alpha, scale);
                }
                Overlay::Custom(c) => {
                    let cw = metrics.cell_w;
                    let ch = metrics.cell_h;
                    let size = c.measure(&OverlayMeasureCtx { metrics, dw, dh });
                    let (panel_x, panel_y) =
                        if let Some(origin) = c.card_origin(size, dw, dh, chrome_top) {
                            origin
                        } else {
                            let panel_w = size.w.max(20.0 * cw);
                            (((dw - panel_w) * 0.5).max(0.0), chrome_top + ch)
                        };
                    let geom = CardGeom {
                        x: panel_x,
                        y: panel_y,
                        w: size.w,
                        h: size.h,
                        radius: 0.0,
                        padding: 2.0 * cw,
                        anim_scale: scale,
                        anim_alpha: alpha,
                    };
                    draw_card_chrome(raster, theme, geom, c.close_on_blur());
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

    /// The `OverlayId` of the topmost entry, if any.
    pub fn top_id(&self) -> Option<OverlayId> {
        self.entries.last().map(|e| match &e.overlay {
            Overlay::Picker(p) => p.id,
            Overlay::TextInput(ti) => ti.id,
            Overlay::Tooltip(tt) => tt.id,
            Overlay::Custom(c) => c.id(),
        })
    }

    /// Mutable ref to the top overlay entry, if any.
    fn top_mut(&mut self) -> Option<&mut OverlayEntry> {
        self.entries.last_mut()
    }

    /// Remove any entry whose id matches `id` (immediate, no animation).
    /// Used to replace a custom overlay whose state has changed (e.g. SCM refresh).
    pub fn remove_by_id(&mut self, id: OverlayId) {
        self.entries.retain(|e| {
            let eid = match &e.overlay {
                Overlay::Picker(p) => p.id,
                Overlay::TextInput(ti) => ti.id,
                Overlay::Tooltip(tt) => tt.id,
                Overlay::Custom(c) => c.id(),
            };
            eid != id
        });
    }

    /// True if any entry with the given id is present in the stack.
    pub fn contains_id(&self, id: OverlayId) -> bool {
        self.entries.iter().any(|e| {
            let eid = match &e.overlay {
                Overlay::Picker(p) => p.id,
                Overlay::TextInput(ti) => ti.id,
                Overlay::Tooltip(tt) => tt.id,
                Overlay::Custom(c) => c.id(),
            };
            eid == id
        })
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
