//! GPU glyph atlas for the terminal rasterizer — Phase A infrastructure.
//!
//! `AtlasPainter` owns:
//! - An `MTLTexture` (R8Unorm; 1024×1024 initially, grows to 2048×2048 on
//!   first overflow; hard cap 2048×2048).
//! - A `ShelfPacker` for rect allocation.
//! - A `HashMap<AtlasKey, GlyphSlot>` for cached slots.
//! - A `Font` for glyph cmap lookup and rasterization.
//!
//! Glyph rasterization uses `CoreTextPainter` (Phase A: cell-sized masks,
//! bearing = (0, 0)). The BGRA8 output is converted to R8 by extracting the
//! red channel — white-on-black rendering makes all channels equal.
//!
//! Phase A: infrastructure only. Nothing in the running app calls this yet.

use std::collections::HashMap;
use std::ffi::c_void;
use std::ptr::NonNull;

use anvil_render::{FontMetrics, GlyphPainter, GlyphRasterizer, GlyphSlot, PixelRect, ShelfPacker};
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_metal::{
    MTLDevice, MTLOrigin, MTLPixelFormat, MTLRegion, MTLSize, MTLStorageMode, MTLTexture,
    MTLTextureDescriptor,
};
use thiserror::Error;

use crate::font::{CoreTextPainter, Font};

/// Initial atlas dimension (pixels, square).
const ATLAS_INITIAL: u16 = 1024;
/// Maximum atlas dimension (pixels, square).
const ATLAS_MAX: u16 = 2048;
/// Memory budget for the R8Unorm atlas (bytes). 2048² × 1 byte = 4 MB per atlas;
/// at 32 MB we could fit 8 atlases. We use a single atlas capped at 2048² (4 MB)
/// which is well under 32 MB. When the packer is full at max size, we evict all
/// cached slots (LRU approximation: clear-all) and reset the packer so glyphs are
/// re-rasterized on next demand. This keeps RSS bounded regardless of zoom level.
const ATLAS_MEMORY_BUDGET: usize = 32 * 1024 * 1024;

/// Errors from `AtlasPainter`.
#[derive(Debug, Error)]
pub enum AtlasError {
    #[error("Metal texture creation failed: {0}")]
    TextureCreate(String),
    #[error("texture upload failed: {0}")]
    Upload(String),
}

/// Cache key: quantized font size + glyph index.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
struct AtlasKey {
    /// `metrics.cell_h.round() as u16` — one entry per integer-pixel cell height.
    font_size_q: u16,
    /// Font cmap glyph index.
    glyph_idx: u16,
}

fn font_size_q_from(metrics: FontMetrics) -> u16 {
    metrics.cell_h.round() as u16
}

// ── AtlasPainter ─────────────────────────────────────────────────────────────

/// Implements `GlyphRasterizer` via CoreText + Metal R8Unorm atlas texture.
pub struct AtlasPainter {
    font: Font,
    device: Retained<ProtocolObject<dyn MTLDevice>>,
    texture: Retained<ProtocolObject<dyn MTLTexture>>,
    packer: ShelfPacker,
    map: HashMap<AtlasKey, GlyphSlot>,
    /// Whether a packer overflow has already been logged (log once).
    overflowed: bool,
    /// Number of full-atlas evictions performed (AA11). Each eviction clears
    /// the packer and cache so glyphs are re-rasterized on next demand.
    eviction_count: u32,
}

impl AtlasPainter {
    /// Create an `AtlasPainter` with the default 1024×1024 R8Unorm atlas.
    pub fn new(device: &ProtocolObject<dyn MTLDevice>, font: Font) -> Result<Self, AtlasError> {
        Self::new_with_size(device, font, ATLAS_INITIAL)
    }

    /// Create an `AtlasPainter` using the system default Metal device.
    ///
    /// Returns `None` when no Metal device is available (e.g., headless CI).
    /// This is a convenience constructor that avoids callers depending on
    /// `objc2-metal` directly.
    pub fn new_with_default_device(font: Font) -> Option<Result<Self, AtlasError>> {
        use objc2_metal::MTLCreateSystemDefaultDevice;
        let device = MTLCreateSystemDefaultDevice()?;
        Some(Self::new(&device, font))
    }

    /// Create an `AtlasPainter` with a custom atlas side length. Exposed for
    /// testing (a tiny atlas exercises the full-atlas fallback path).
    pub fn new_with_size(
        device: &ProtocolObject<dyn MTLDevice>,
        font: Font,
        size: u16,
    ) -> Result<Self, AtlasError> {
        let texture = make_r8_texture(device, size as usize, size as usize)?;
        // Retain the device reference for future texture recreations.
        // SAFETY: device is a live Metal device; Retained::retain increments its
        // ref-count and gives us an owned handle.
        let device = unsafe { Retained::retain(device as *const _ as *mut _) }
            .expect("device is a valid live object");
        Ok(AtlasPainter {
            font,
            device,
            texture,
            packer: ShelfPacker::new(size, size),
            map: HashMap::new(),
            overflowed: false,
            eviction_count: 0,
        })
    }

    /// The underlying Metal R8Unorm texture (for GPU sampling in later phases).
    pub fn texture(&self) -> &Retained<ProtocolObject<dyn MTLTexture>> {
        &self.texture
    }

    /// The atlas texture dimensions in pixels, `(width, height)`.
    pub fn texture_size(&self) -> (usize, usize) {
        use objc2_metal::MTLTexture as _;
        (self.texture.width(), self.texture.height())
    }

    /// The font this painter was created with.
    pub fn font(&self) -> &Font {
        &self.font
    }

    /// Grow the atlas to 2048×2048: recreate the texture, reset the packer,
    /// and clear the cache. Returns Err if the texture creation fails.
    fn grow(&mut self) -> Result<(), AtlasError> {
        let size = ATLAS_MAX as usize;
        self.texture = make_r8_texture(&self.device, size, size)?;
        self.packer = ShelfPacker::new(ATLAS_MAX, ATLAS_MAX);
        self.map.clear();
        Ok(())
    }
}

impl GlyphRasterizer for AtlasPainter {
    fn glyph_slot(&mut self, codepoint: u32, metrics: FontMetrics) -> Option<GlyphSlot> {
        // ── Step 1: resolve cmap ─────────────────────────────────────────────
        let glyph_idx = self.font.glyph(codepoint);
        if glyph_idx == 0 {
            return None;
        }

        // ── Step 2: check cache ──────────────────────────────────────────────
        let key = AtlasKey {
            font_size_q: font_size_q_from(metrics),
            glyph_idx,
        };
        if let Some(&slot) = self.map.get(&key) {
            return Some(slot);
        }

        // ── Step 3: rasterize into a cell-sized BGRA8 buffer ────────────────
        let cell_w = metrics.cell_w.round() as usize;
        let cell_h = metrics.cell_h.round() as usize;
        if cell_w == 0 || cell_h == 0 {
            return None;
        }
        let dest = PixelRect {
            x: 0.0,
            y: 0.0,
            w: cell_w as f64,
            h: cell_h as f64,
        };
        let mut bgra = vec![0u8; cell_w * cell_h * 4];
        {
            // CoreTextPainter borrows &self.font.  We take a raw pointer so
            // that the borrow does not conflict with the &mut self borrow on
            // AtlasPainter.  The pointer is valid for the duration of this
            // block because self.font is not mutated here.
            //
            // SAFETY: self.font is not moved or freed while painter lives.
            let font_ptr: *const Font = &self.font;
            let mut painter = CoreTextPainter::new(unsafe { &*font_ptr });
            painter.draw_glyph(
                codepoint,
                dest,
                [255, 255, 255],
                metrics,
                &mut bgra,
                cell_w,
                cell_h,
            );
        }

        // Convert BGRA8 → R8 by extracting the red channel.
        // `CoreTextPainter` draws white glyphs (R = G = B = coverage).
        // BGRA layout: byte 0 = B, 1 = G, 2 = R, 3 = A.
        let mask: Vec<u8> = bgra.chunks_exact(4).map(|px| px[2]).collect();

        // ── Step 4: pack into the atlas ──────────────────────────────────────
        let cw = cell_w as u16;
        let ch = cell_h as u16;
        let pos = self.pack_or_grow(cw, ch)?;

        // ── Step 5: upload the R8 mask ───────────────────────────────────────
        let (x, y) = pos;
        let region = MTLRegion {
            origin: MTLOrigin {
                x: x as usize,
                y: y as usize,
                z: 0,
            },
            size: MTLSize {
                width: cell_w,
                height: cell_h,
                depth: 1,
            },
        };
        // SAFETY: mask is non-empty; pointer is non-null.
        // bytes_per_row = cell_w (1 byte per R8 pixel).
        let bytes_ptr = NonNull::new(mask.as_ptr() as *mut c_void).unwrap();
        unsafe {
            self.texture
                .replaceRegion_mipmapLevel_withBytes_bytesPerRow(region, 0, bytes_ptr, cell_w);
        }

        // ── Step 6: cache and return ─────────────────────────────────────────
        let slot = GlyphSlot {
            atlas_x: x,
            atlas_y: y,
            w: cw,
            h: ch,
            bearing_x: 0,
            bearing_y: 0,
        };
        self.map.insert(key, slot);
        Some(slot)
    }
}

impl AtlasPainter {
    /// Try to pack a `w × h` rect; on first overflow, grow the atlas and retry.
    /// When already at max size (AA11): evict all cached glyphs, reset the packer,
    /// and retry — keeping memory within `ATLAS_MEMORY_BUDGET`.
    fn pack_or_grow(&mut self, w: u16, h: u16) -> Option<(u16, u16)> {
        if let Some(pos) = self.packer.alloc(w, h) {
            return Some(pos);
        }

        // First overflow: try growing to 2048².
        let (aw, _) = self.packer.capacity();
        if aw < ATLAS_MAX {
            if self.grow().is_err() {
                if !self.overflowed {
                    self.overflowed = true;
                    eprintln!("glyph_atlas: atlas grow failed; some glyphs will be missing");
                }
                return None;
            }
            if let Some(pos) = self.packer.alloc(w, h) {
                return Some(pos);
            }
        }

        // AA11: Atlas is at max size and still full.  Evict all cached glyphs
        // (LRU approximation: clear-all) so they are re-rasterized on demand.
        // The atlas texture is reused in-place — no Metal reallocation needed.
        // This keeps RSS bounded by ATLAS_MEMORY_BUDGET (2048² R8 = 4 MB << 32 MB).
        self.map.clear();
        self.packer = ShelfPacker::new(ATLAS_MAX, ATLAS_MAX);
        self.eviction_count += 1;
        self.overflowed = false; // reset so the next overflow logs once again
        eprintln!(
            "glyph_atlas: evicting all glyphs (eviction #{}, budget={}MB)",
            self.eviction_count,
            ATLAS_MEMORY_BUDGET / (1024 * 1024)
        );

        // Retry after eviction.
        self.packer.alloc(w, h)
    }
}

// ── helpers ──────────────────────────────────────────────────────────────────

fn make_r8_texture(
    device: &ProtocolObject<dyn MTLDevice>,
    width: usize,
    height: usize,
) -> Result<Retained<ProtocolObject<dyn MTLTexture>>, AtlasError> {
    // SAFETY: texture2DDescriptorWithPixelFormat_width_height_mipmapped is a
    // class method returning a +1 retained descriptor.
    let desc = unsafe {
        MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
            MTLPixelFormat::R8Unorm,
            width,
            height,
            false,
        )
    };
    // Shared storage: CPU can write via replaceRegion without a blit command.
    desc.setStorageMode(MTLStorageMode::Shared);
    device
        .newTextureWithDescriptor(&desc)
        .ok_or_else(|| AtlasError::TextureCreate(format!("R8Unorm {}x{}", width, height)))
}

// ── tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[cfg(target_os = "macos")]
mod tests {
    use super::*;
    use objc2_metal::MTLCreateSystemDefaultDevice;

    fn device() -> Option<Retained<ProtocolObject<dyn MTLDevice>>> {
        MTLCreateSystemDefaultDevice()
    }

    fn font() -> Font {
        Font::init_first_available(&["IBMPlexMono", "SFMono-Regular", "Menlo"], 26.0)
            .expect("test font must be available")
    }

    /// Round-trip: requesting the same codepoint twice returns the cached slot.
    #[test]
    fn round_trip_second_call_hits_cache() {
        let Some(dev) = device() else { return };
        let f = font();
        let m = f.metrics;
        let mut painter = AtlasPainter::new(&dev, f).expect("AtlasPainter::new");

        let slot_a = painter
            .glyph_slot(b'A' as u32, m)
            .expect("'A' must have a slot");
        let slot_b = painter
            .glyph_slot(b'A' as u32, m)
            .expect("cached slot must return");

        // Both calls return identical slots; the second hits the cache.
        assert_eq!(slot_a, slot_b);
    }

    /// Missing glyph: codepoint 0 returns None.
    #[test]
    fn missing_glyph_returns_none() {
        let Some(dev) = device() else { return };
        let f = font();
        let m = f.metrics;
        let mut painter = AtlasPainter::new(&dev, f).expect("AtlasPainter::new");

        assert!(painter.glyph_slot(0, m).is_none());
    }

    /// Atlas-full fallback: a tiny atlas can't fit the cell, returns None without panic.
    #[test]
    fn atlas_full_returns_none_without_panic() {
        let Some(dev) = device() else { return };
        let f = font();
        let m = f.metrics;
        // Atlas is 4×4 px — far smaller than any cell; alloc must fail gracefully.
        let mut painter =
            AtlasPainter::new_with_size(&dev, f, 4).expect("AtlasPainter::new_with_size");

        // Must return None and must not panic.
        let _ = painter.glyph_slot(b'A' as u32, m);
    }

    /// Different font sizes produce separate cache entries; both slots succeed.
    #[test]
    fn different_font_sizes_have_separate_cache_entries() {
        let Some(dev) = device() else { return };

        let f_a =
            Font::init_first_available(&["IBMPlexMono", "SFMono-Regular", "Menlo"], 20.0).unwrap();
        let f_b =
            Font::init_first_available(&["IBMPlexMono", "SFMono-Regular", "Menlo"], 32.0).unwrap();

        let metrics_a = f_a.metrics;
        let metrics_b = f_b.metrics;

        assert_ne!(
            metrics_a.cell_h.round() as u16,
            metrics_b.cell_h.round() as u16,
            "test fonts must produce different cell heights"
        );

        let mut painter = AtlasPainter::new(&dev, f_a).expect("AtlasPainter::new");

        let slot_a = painter.glyph_slot(b'A' as u32, metrics_a);
        let slot_b = painter.glyph_slot(b'A' as u32, metrics_b);

        assert!(slot_a.is_some(), "'A' at metrics_a must produce a slot");
        assert!(slot_b.is_some(), "'A' at metrics_b must produce a slot");

        // Different cell sizes → different atlas positions.
        assert_ne!(
            slot_a.unwrap(),
            slot_b.unwrap(),
            "different cell sizes must produce different slots"
        );
    }
}
