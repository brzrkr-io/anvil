//! Instance batch for the GPU cell pipeline (Phase B).
//!
//! `CellBatch` collects `CellInstance` records for one frame and exposes them
//! as a raw byte slice for upload to a Metal vertex buffer.  No Metal
//! dependency — this module is pure Rust.

use crate::atlas::GlyphSlot;

/// One cell's worth of per-instance data, matched 1:1 to the MSL
/// `CellInstance` struct in `anvil-platform`'s `metal.rs`.
///
/// The `#[repr(C)]` layout must not change without updating the MSL struct.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct CellInstance {
    /// Top-left corner of the cell in drawable pixels, (x, y).
    pub cell_px_xy: [f32; 2],
    /// Size of the cell in drawable pixels, (w, h).
    pub cell_px_wh: [f32; 2],
    /// Top-left of the glyph rect in atlas pixels, (x, y).
    pub atlas_uv_xy: [u16; 2],
    /// Size of the glyph rect in atlas pixels, (w, h).
    /// `(0, 0)` means bg-only — no glyph sample.
    pub atlas_uv_wh: [u16; 2],
    /// Sub-pixel glyph offset within the cell, (dx, dy) in pixels.
    pub glyph_offset: [i16; 2],
    /// Foreground colour, RGBA.
    pub fg_rgba: [u8; 4],
    /// Background colour, RGBA.
    pub bg_rgba: [u8; 4],
}

// Compile-time size check.  The MSL struct lays out identically:
// [f32;2]=8, [f32;2]=8, [u16;2]=4, [u16;2]=4, [i16;2]=4, [u8;4]=4, [u8;4]=4 → 36 bytes.
const _: () = assert!(std::mem::size_of::<CellInstance>() == 36);

/// Collected instances for one frame, ready to upload to a Metal vertex buffer.
pub struct CellBatch {
    /// One entry per cell that will be drawn this frame.
    pub instances: Vec<CellInstance>,
    /// Drawable size in pixels, (width, height).
    pub viewport_px: [f32; 2],
}

impl CellBatch {
    /// Create an empty batch.  `viewport_px` is initialised to `[0.0, 0.0]`;
    /// call `clear` before filling the batch for each frame.
    pub fn new() -> Self {
        CellBatch {
            instances: Vec::new(),
            viewport_px: [0.0, 0.0],
        }
    }

    /// Reset the batch for a new frame.  Clears `instances` (keeps capacity)
    /// and records the current drawable size.
    pub fn clear(&mut self, viewport_px: [f32; 2]) {
        self.instances.clear();
        self.viewport_px = viewport_px;
    }

    /// Push one cell.
    ///
    /// - `cell_px_xy` — top-left corner of the cell in drawable pixels.
    /// - `cell_px_wh` — width/height of the cell in drawable pixels.
    /// - `slot` — `Some(GlyphSlot)` for cells with a glyph; `None` for
    ///   background-only cells (spaces, empty cells).
    /// - `fg` / `bg` — RGB foreground and background colours.
    pub fn push_cell(
        &mut self,
        cell_px_xy: [f32; 2],
        cell_px_wh: [f32; 2],
        slot: Option<GlyphSlot>,
        fg: [u8; 3],
        bg: [u8; 3],
    ) {
        let (atlas_uv_xy, atlas_uv_wh, glyph_offset) = match slot {
            Some(s) => (
                [s.atlas_x, s.atlas_y],
                [s.w, s.h],
                [s.bearing_x, s.bearing_y],
            ),
            None => ([0u16, 0], [0u16, 0], [0i16, 0]),
        };

        self.instances.push(CellInstance {
            cell_px_xy,
            cell_px_wh,
            atlas_uv_xy,
            atlas_uv_wh,
            glyph_offset,
            fg_rgba: [fg[0], fg[1], fg[2], 255],
            bg_rgba: [bg[0], bg[1], bg[2], 255],
        });
    }

    /// Number of instances in this batch.
    pub fn instance_count(&self) -> usize {
        self.instances.len()
    }

    /// Raw bytes of the instance slice, suitable for memcpy into a Metal buffer.
    pub fn instance_bytes(&self) -> &[u8] {
        // SAFETY: CellInstance is #[repr(C)] with no padding that could be
        // uninitialised; all fields are primitive types.  The resulting byte
        // slice is valid for the lifetime of &self.
        unsafe {
            std::slice::from_raw_parts(
                self.instances.as_ptr() as *const u8,
                self.instances.len() * std::mem::size_of::<CellInstance>(),
            )
        }
    }
}

impl Default for CellBatch {
    fn default() -> Self {
        Self::new()
    }
}

// ── tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::atlas::GlyphSlot;

    fn slot(atlas_x: u16, atlas_y: u16, w: u16, h: u16) -> GlyphSlot {
        GlyphSlot {
            atlas_x,
            atlas_y,
            w,
            h,
            bearing_x: 2,
            bearing_y: 3,
        }
    }

    /// push_cell with a GlyphSlot records correct atlas_uv_* and glyph_offset.
    #[test]
    fn push_cell_with_slot_records_atlas_uv() {
        let mut batch = CellBatch::new();
        let s = slot(16, 32, 10, 20);
        batch.push_cell([0.0, 0.0], [12.0, 24.0], Some(s), [255, 0, 0], [0, 0, 0]);

        assert_eq!(batch.instances.len(), 1);
        let inst = batch.instances[0];
        assert_eq!(inst.atlas_uv_xy, [16, 32]);
        assert_eq!(inst.atlas_uv_wh, [10, 20]);
        assert_eq!(inst.glyph_offset, [2, 3]);
        assert_eq!(inst.fg_rgba, [255, 0, 0, 255]);
        assert_eq!(inst.bg_rgba, [0, 0, 0, 255]);
    }

    /// push_cell without a slot records atlas_uv_wh = (0, 0) for bg-only.
    #[test]
    fn push_cell_without_slot_records_bg_only() {
        let mut batch = CellBatch::new();
        batch.push_cell([4.0, 8.0], [12.0, 24.0], None, [0, 255, 0], [10, 20, 30]);

        let inst = batch.instances[0];
        assert_eq!(inst.atlas_uv_wh, [0, 0]);
        assert_eq!(inst.atlas_uv_xy, [0, 0]);
        assert_eq!(inst.glyph_offset, [0, 0]);
        assert_eq!(inst.cell_px_xy, [4.0, 8.0]);
        assert_eq!(inst.cell_px_wh, [12.0, 24.0]);
    }

    /// clear resets instances length to 0 but keeps capacity.
    #[test]
    fn clear_resets_length_keeps_capacity() {
        let mut batch = CellBatch::new();
        for _ in 0..64 {
            batch.push_cell([0.0, 0.0], [10.0, 20.0], None, [0, 0, 0], [0, 0, 0]);
        }
        assert_eq!(batch.instances.len(), 64);
        let cap_before = batch.instances.capacity();

        batch.clear([800.0, 600.0]);

        assert_eq!(batch.instances.len(), 0);
        assert!(batch.instances.capacity() >= cap_before);
        assert_eq!(batch.viewport_px, [800.0, 600.0]);
    }

    /// instance_bytes returns the correct byte length.
    #[test]
    fn instance_bytes_correct_length() {
        let mut batch = CellBatch::new();
        batch.push_cell([0.0, 0.0], [10.0, 20.0], None, [1, 2, 3], [4, 5, 6]);
        batch.push_cell([10.0, 0.0], [10.0, 20.0], None, [7, 8, 9], [0, 0, 0]);

        let bytes = batch.instance_bytes();
        assert_eq!(bytes.len(), 2 * std::mem::size_of::<CellInstance>());
    }

    /// instance_bytes on an empty batch returns an empty slice.
    #[test]
    fn instance_bytes_empty() {
        let batch = CellBatch::new();
        assert_eq!(batch.instance_bytes().len(), 0);
    }
}
