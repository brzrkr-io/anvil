//! GPU glyph atlas — pure-Rust infrastructure (Phase A).
//!
//! Provides:
//! - `GlyphSlot`: position + metrics of one glyph in the atlas.
//! - `GlyphRasterizer`: trait that resolves a codepoint to an atlas slot.
//! - `ShelfPacker`: shelf-based 2-D rectangle packer, no platform dependency.
//!
//! Nothing in the running app calls this code yet; Phase B–D wire it in.

/// Position and metrics of a single glyph inside the atlas texture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GlyphSlot {
    /// Left edge of the glyph rect in atlas pixels.
    pub atlas_x: u16,
    /// Top edge of the glyph rect in atlas pixels.
    pub atlas_y: u16,
    /// Width of the glyph rect in atlas pixels.
    pub w: u16,
    /// Height of the glyph rect in atlas pixels.
    pub h: u16,
    /// Horizontal bearing (pixels from cell left to glyph left).
    pub bearing_x: i16,
    /// Vertical bearing (pixels from cell top to glyph top).
    pub bearing_y: i16,
}

/// Trait that resolves a codepoint to an atlas slot.
///
/// `anvil-platform`'s CoreText-backed `AtlasPainter` is the production
/// implementation; tests use a stub.
pub trait GlyphRasterizer {
    /// Returns `None` for missing/empty glyphs (space, NUL, unsupported) and
    /// when the atlas is full (caller falls back to a bg-only cell).
    fn glyph_slot(
        &mut self,
        codepoint: u32,
        metrics: crate::raster::FontMetrics,
    ) -> Option<GlyphSlot>;
}

// ── ShelfPacker ──────────────────────────────────────────────────────────────

/// One shelf in the packer.
struct Shelf {
    /// Next x position to allocate within this shelf.
    cursor_x: u16,
    /// Fixed y of the top of this shelf.
    y: u16,
    /// Height of this shelf (fixed at creation; = height of the first alloc).
    height: u16,
}

/// Shelf-based 2-D rectangle packer.
///
/// Maintains a list of horizontal shelves, each with a current x cursor and a
/// fixed height. New entries pick the shelf with the smallest height waste
/// that still fits. When no existing shelf fits, a new shelf is opened at the
/// current `next_y` cursor. Pure logic; testable without Metal.
pub struct ShelfPacker {
    shelves: Vec<Shelf>,
    atlas_w: u16,
    atlas_h: u16,
    /// Y of the next shelf to open.
    next_y: u16,
}

impl ShelfPacker {
    /// Create a new empty packer for an atlas of `atlas_w` × `atlas_h` pixels.
    pub fn new(atlas_w: u16, atlas_h: u16) -> Self {
        ShelfPacker {
            shelves: Vec::new(),
            atlas_w,
            atlas_h,
            next_y: 0,
        }
    }

    /// Allocate a rect of `w` × `h` pixels.
    ///
    /// Returns the top-left `(x, y)` of the allocated rect in atlas pixels, or
    /// `None` if there is no room.
    pub fn alloc(&mut self, w: u16, h: u16) -> Option<(u16, u16)> {
        if w == 0 || h == 0 || w > self.atlas_w || h > self.atlas_h {
            return None;
        }

        // Find the best existing shelf: the one with the smallest height waste
        // that is still >= h and has enough horizontal space remaining.
        let best = self
            .shelves
            .iter_mut()
            .filter(|s| s.height >= h && self.atlas_w - s.cursor_x >= w)
            .min_by_key(|s| s.height - h);

        if let Some(shelf) = best {
            let x = shelf.cursor_x;
            let y = shelf.y;
            shelf.cursor_x += w;
            return Some((x, y));
        }

        // Open a new shelf.
        if self.next_y > self.atlas_h || self.atlas_h - self.next_y < h {
            return None; // No vertical space.
        }
        // The new shelf needs at least w pixels of width.
        if self.atlas_w < w {
            return None;
        }
        let y = self.next_y;
        self.next_y = self.next_y.checked_add(h)?;
        self.shelves.push(Shelf {
            cursor_x: w,
            y,
            height: h,
        });
        Some((0, y))
    }

    /// Returns the atlas dimensions this packer was created with.
    pub fn capacity(&self) -> (u16, u16) {
        (self.atlas_w, self.atlas_h)
    }
}

// ── tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn metrics(cell_w: f64, cell_h: f64) -> crate::raster::FontMetrics {
        crate::raster::FontMetrics {
            cell_w,
            cell_h,
            descent: 4.0,
        }
    }

    // ── ShelfPacker ──────────────────────────────────────────────────────────

    /// Fresh packer places the first alloc at (0, 0).
    #[test]
    fn single_alloc_at_origin() {
        let mut p = ShelfPacker::new(256, 256);
        assert_eq!(p.alloc(32, 32), Some((0, 0)));
    }

    /// Two same-height allocs land on the same shelf; second is to the right.
    #[test]
    fn two_same_height_on_same_shelf() {
        let mut p = ShelfPacker::new(256, 256);
        let a = p.alloc(32, 32).unwrap();
        let b = p.alloc(32, 32).unwrap();
        // Same row.
        assert_eq!(a.1, b.1);
        // b is to the right of a.
        assert_eq!(b.0, a.0 + 32);
    }

    /// A taller alloc opens a new shelf at the previous shelf's bottom.
    #[test]
    fn taller_alloc_opens_new_shelf() {
        let mut p = ShelfPacker::new(256, 256);
        let a = p.alloc(32, 32).unwrap();
        let b = p.alloc(32, 64).unwrap(); // taller — no existing shelf fits
        assert_ne!(a.1, b.1);
        // b's shelf starts at the bottom of the first shelf.
        assert_eq!(b.1, a.1 + 32);
    }

    /// Filling a shelf's width forces the next alloc onto a new shelf.
    #[test]
    fn full_shelf_width_forces_new_shelf() {
        let mut p = ShelfPacker::new(64, 256);
        // Fill the first shelf exactly.
        p.alloc(32, 32).unwrap();
        p.alloc(32, 32).unwrap();
        // Atlas is 64 wide; shelf is full. Next same-height alloc needs a new shelf.
        let c = p.alloc(1, 32).unwrap();
        // c must be on a new row.
        assert_eq!(c.1, 32);
    }

    /// Allocs that exceed atlas height return None.
    #[test]
    fn alloc_exceeding_atlas_height_returns_none() {
        // 32×32 atlas: one 32-wide shelf exactly fills the height.
        let mut p = ShelfPacker::new(32, 32);
        p.alloc(32, 32).unwrap(); // fills the single shelf; next_y = 32 = atlas_h
        // No vertical space left for another shelf, and the existing shelf is full.
        assert_eq!(p.alloc(1, 32), None);
    }

    /// Allocs that exceed atlas width return None even when there's vertical space.
    #[test]
    fn alloc_exceeding_atlas_width_returns_none() {
        let mut p = ShelfPacker::new(64, 256);
        assert_eq!(p.alloc(128, 32), None);
    }

    /// Best-fit by height waste: given two open shelves of different heights,
    /// a small alloc lands on the smaller (less wasteful) shelf.
    #[test]
    fn best_fit_picks_smaller_shelf() {
        // Atlas: 512×512.
        // Step 1: nearly fill a 64-tall shelf so it has only 12 px of width left.
        let mut p = ShelfPacker::new(512, 512);
        p.alloc(500, 64).unwrap(); // 64-tall shelf at y=0, cursor_x=500, 12 px left.

        // Step 2: alloc something wider than the 64-tall shelf's 12 remaining px.
        // This forces a new 32-tall shelf at y=64.
        p.alloc(100, 32).unwrap(); // 32-tall shelf at y=64, cursor_x=100.

        // Step 3: alloc 10×32. Two candidates:
        //   64-tall shelf: 12 px left >= 10, height=64 >= 32, waste = 64-32 = 32.
        //   32-tall shelf: 412 px left >= 10, height=32 >= 32, waste = 32-32 = 0.
        // Best-fit must pick the 32-tall shelf (y=64).
        let slot = p.alloc(10, 32).unwrap();
        assert_eq!(
            slot.1, 64,
            "should pick the 32-tall shelf at y=64, got y={}",
            slot.1
        );
    }

    // ── GlyphRasterizer stub ─────────────────────────────────────────────────

    /// Smoke test: the trait is object-safe and a stub impl can be used.
    #[test]
    fn trait_is_object_safe() {
        struct Stub;
        impl GlyphRasterizer for Stub {
            fn glyph_slot(
                &mut self,
                _codepoint: u32,
                _metrics: crate::raster::FontMetrics,
            ) -> Option<GlyphSlot> {
                None
            }
        }

        let mut s: Box<dyn GlyphRasterizer> = Box::new(Stub);
        assert!(s.glyph_slot(b'A' as u32, metrics(10.0, 20.0)).is_none());
    }
}
