//! Overlay animation state machine.
//!
//! Each overlay entry carries an `OverlayAnim` that drives a scale + alpha
//! transition. `AppShell` calls `overlays.tick(dt)` every frame; if any entry
//! is still animating, it requests a redraw.
//!
//! States:
//! - `Entering` → t goes 0→1, scale 0.96→1.00, alpha 0→1.
//! - `Visible`  → t = 1, fully opaque.
//! - `Leaving`  → t goes 1→0, mirrors Entering.

/// Animation lifecycle state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnimState {
    Entering,
    Visible,
    Leaving,
}

/// Per-overlay animation state machine.
pub struct OverlayAnim {
    pub state: AnimState,
    /// Normalized progress: 0.0 (start) → 1.0 (fully visible).
    pub t: f64,
    /// Total transition duration in milliseconds.
    pub duration_ms: f64,
}

impl OverlayAnim {
    /// Create a new animation in the Entering state.
    pub fn new() -> Self {
        Self {
            state: AnimState::Entering,
            t: 0.0,
            duration_ms: 100.0,
        }
    }

    /// Advance by `dt_ms`. Returns `true` if the state changed.
    pub fn tick(&mut self, dt_ms: f64) -> bool {
        match self.state {
            AnimState::Entering => {
                self.t += dt_ms / self.duration_ms;
                if self.t >= 1.0 {
                    self.t = 1.0;
                    self.state = AnimState::Visible;
                    return true;
                }
                true
            }
            AnimState::Visible => false,
            AnimState::Leaving => {
                self.t -= dt_ms / self.duration_ms;
                if self.t <= 0.0 {
                    self.t = 0.0;
                    return true; // caller should gc this entry
                }
                true
            }
        }
    }

    /// Current alpha: ease_out_cubic of t.
    pub fn alpha(&self) -> f64 {
        ease_out_cubic(self.t)
    }

    /// Current scale: 0.96 + 0.04 × ease_out_cubic(t).
    pub fn scale(&self) -> f64 {
        0.96 + 0.04 * ease_out_cubic(self.t)
    }

    /// Begin the closing transition.
    pub fn begin_close(&mut self) {
        if self.state != AnimState::Leaving {
            self.state = AnimState::Leaving;
            // If we weren't fully visible yet, start from the current t.
        }
    }

    /// True when the animation has completed (for gc after Leaving).
    pub fn finished(&self) -> bool {
        match self.state {
            AnimState::Leaving => self.t <= 0.0,
            AnimState::Visible => false,
            AnimState::Entering => false,
        }
    }

    /// True when the animation is still in motion (Entering or Leaving).
    pub fn is_animating(&self) -> bool {
        matches!(self.state, AnimState::Entering | AnimState::Leaving)
    }
}

impl Default for OverlayAnim {
    fn default() -> Self {
        Self::new()
    }
}

/// Cubic ease-out: `1 - (1 - t)^3`.
fn ease_out_cubic(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anim_alpha_zero_at_entering_start_one_at_visible() {
        let mut a = OverlayAnim::new();
        // At creation (t=0, Entering) alpha should be 0.
        assert!(
            a.alpha() < 0.001,
            "alpha at Entering t=0 should be ~0, got {}",
            a.alpha()
        );
        // Tick all the way to Visible.
        a.tick(1000.0);
        assert_eq!(a.state, AnimState::Visible);
        assert!(
            (a.alpha() - 1.0).abs() < 0.001,
            "alpha at Visible should be ~1.0, got {}",
            a.alpha()
        );
    }

    #[test]
    fn anim_begin_close_transitions_to_leaving() {
        let mut a = OverlayAnim::new();
        a.tick(1000.0); // reach Visible
        assert_eq!(a.state, AnimState::Visible);
        a.begin_close();
        assert_eq!(a.state, AnimState::Leaving);
    }

    #[test]
    fn anim_finished_after_leave() {
        let mut a = OverlayAnim::new();
        a.tick(1000.0); // reach Visible
        a.begin_close();
        a.tick(1000.0); // fully left
        assert!(a.finished(), "should be finished after full leave");
    }

    #[test]
    fn ease_out_cubic_monotone() {
        // alpha must be monotonically non-decreasing as t goes 0→1.
        let mut prev = ease_out_cubic(0.0);
        for i in 1..=100 {
            let t = i as f64 / 100.0;
            let cur = ease_out_cubic(t);
            assert!(cur >= prev, "not monotone at t={t}: prev={prev} cur={cur}");
            prev = cur;
        }
    }
}
