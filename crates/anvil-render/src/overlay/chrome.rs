//! Card chrome: scrim, shadow, panel fill, highlight, border.
//!
//! `draw_card_chrome` is the single entry point. Rounded corners are deferred
//! to Phase 4 when `raster::fill_rounded_rect` ships.
//!
//! Paint order (spec § 3):
//! 1. Scrim (#000 α 0.28 × anim_alpha) — full window.
//! 2. Three shadow rects (offsets +2/+4/+8, α 0.10/0.06/0.03 × anim_alpha).
//! 3. Panel fill (theme.panel × anim_alpha).
//! 4. 1px top inner highlight (theme.surface α 0.3).
//! 5. 1px border (theme.hairline).
//!
//! `anim_scale` is applied around the card center (unused in pixel-fill path;
//! stored for future GPU transform).

use anvil_theme::Theme;

use crate::raster::Raster;

/// Geometry and animation state for a rendered card.
#[derive(Clone, Copy, Debug)]
pub struct CardGeom {
    /// Left edge of the card in device pixels.
    pub x: f64,
    /// Top edge of the card in device pixels.
    pub y: f64,
    /// Card width in device pixels.
    pub w: f64,
    /// Card height in device pixels.
    pub h: f64,
    /// Corner radius in device pixels. Currently unused (TODO(radius)).
    pub radius: f64,
    /// Horizontal/vertical inner padding in device pixels.
    pub padding: f64,
    /// Scale factor from animation (0.96 → 1.00). Used for layout hints.
    pub anim_scale: f64,
    /// Alpha from animation (0 → 1). Multiplies all painted alphas.
    pub anim_alpha: f64,
}

impl CardGeom {
    /// Inner content x (left edge + padding).
    pub fn content_x(&self) -> f64 {
        self.x + self.padding
    }
    /// Inner content y (top edge + padding).
    pub fn content_y(&self) -> f64 {
        self.y + self.padding
    }
    /// Inner content width.
    pub fn content_w(&self) -> f64 {
        (self.w - self.padding * 2.0).max(0.0)
    }
    /// Inner content height.
    pub fn content_h(&self) -> f64 {
        (self.h - self.padding * 2.0).max(0.0)
    }
}

/// Draw the chrome for a card overlay.
///
/// `show_scrim` — when true, paints a full-window dark veil before the card.
/// Pass `true` for modal overlays; `false` for non-modal tooltips.
pub fn draw_card_chrome(raster: &mut Raster, theme: &Theme, geom: CardGeom, show_scrim: bool) {
    let a = geom.anim_alpha;

    // 1. Full-window scrim.
    if show_scrim {
        let dw = raster.width as f64;
        let dh = raster.height as f64;
        raster.fill_pixel_rect_alpha(0.0, 0.0, dw, dh, [0, 0, 0], 0.28 * a);
    }

    // 2. Shadow rects: three concentric expansions.
    // Spec says offsets +2/+4/+8 with α 0.10/0.06/0.03.
    let shadow_steps: &[(f64, f64)] = &[(2.0, 0.10), (4.0, 0.06), (8.0, 0.03)];
    for &(expand, base_alpha) in shadow_steps {
        raster.fill_pixel_rect_alpha(
            geom.x - expand,
            geom.y - expand,
            geom.w + expand * 2.0,
            geom.h + expand * 2.0,
            [0, 0, 0],
            base_alpha * a,
        );
    }

    // 3. Panel fill.
    // TODO(radius): replace with fill_rounded_rect when Phase 4 lands.
    raster.fill_pixel_rect_alpha(geom.x, geom.y, geom.w, geom.h, theme.panel, a);

    // 4. 1px top inner highlight.
    raster.fill_pixel_rect_alpha(geom.x, geom.y, geom.w, 1.0, theme.surface, 0.3 * a);

    // 5. 1px border (full-alpha; hairline is already a subtle color).
    raster.fill_pixel_rect(geom.x, geom.y, geom.w, 1.0, theme.hairline); // top
    raster.fill_pixel_rect(geom.x, geom.y + geom.h - 1.0, geom.w, 1.0, theme.hairline); // bot
    raster.fill_pixel_rect(geom.x, geom.y, 1.0, geom.h, theme.hairline); // left
    raster.fill_pixel_rect(geom.x + geom.w - 1.0, geom.y, 1.0, geom.h, theme.hairline); // right
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raster::pixel_at;
    use anvil_theme::MINERAL_DARK;

    /// Paint order: scrim pixel must be darker than the virgin background,
    /// panel must be present, border must be at hairline color.
    #[test]
    fn chrome_paints_shadow_then_panel_then_border() {
        let theme = MINERAL_DARK;
        let mut raster = Raster::new(200, 200);
        // Start with a white background so paint effects are visible.
        raster.clear([255, 255, 255]);

        let geom = CardGeom {
            x: 20.0,
            y: 20.0,
            w: 100.0,
            h: 80.0,
            radius: 0.0,
            padding: 8.0,
            anim_scale: 1.0,
            anim_alpha: 1.0,
        };

        draw_card_chrome(&mut raster, &theme, geom, true);

        // The pixel at (0,0) should be darkened by the scrim (no longer pure white).
        let corner = pixel_at(&raster, 0, 0);
        assert!(
            corner[0] < 255 || corner[1] < 255 || corner[2] < 255,
            "scrim should darken corner pixel, got {:?}",
            corner
        );

        // The center of the card should be the panel color (not white).
        let cx = (geom.x + geom.w / 2.0) as usize;
        let cy = (geom.y + geom.h / 2.0) as usize;
        let center = pixel_at(&raster, cx, cy);
        // Panel is dark in mineral_dark; certainly not white [255,255,255].
        assert!(
            center[0] != 255 || center[1] != 255 || center[2] != 255,
            "panel center should not be white, got {:?}",
            center
        );

        // Top border pixel (top edge of card) should be the hairline color.
        let border_x = (geom.x + geom.w / 2.0) as usize;
        let border_y = geom.y as usize;
        let border = pixel_at(&raster, border_x, border_y);
        assert_eq!(
            border, theme.hairline,
            "top border pixel should be hairline color"
        );
    }
}
