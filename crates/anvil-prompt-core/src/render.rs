//! Renders a prompt string. Every escape sequence is wrapped in the shell's
//! zero-width markers (`%{ %}` for zsh, `\001 \002` for bash) — without that
//! the shell miscounts the prompt's visible width and typed input lands in the
//! wrong column.
//!
//! Colors are emitted as indexed ANSI colors (`\x1b[38;5;Nm`) so the terminal
//! re-resolves them through the active theme palette on every frame. A theme
//! switch therefore recolors all prompts in scrollback automatically.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shell {
    Plain,
    Zsh,
    Bash,
}

const RESET: &str = "\x1b[0m";

// Indexed ANSI colors — resolved through the active theme each frame so a
// theme switch recolors all prompts in scrollback automatically.
const ACCENT_BRIGHT: &str = "\x1b[38;5;14m"; // ANSI 14 = bright cyan — input arrow
const ACCENT_ERR: &str = "\x1b[38;5;1m"; // ANSI 1 = red — error state
const DIM: &str = "\x1b[38;5;8m"; // ANSI 8 = dim grey — accent dot, transient prompt
const VERIFIED: &str = "\x1b[38;5;2m"; // ANSI 2 = green — success check

#[derive(Debug, Clone, Copy)]
pub struct Options {
    pub rich: bool,
    /// Last command exited non-zero.
    pub failed: bool,
    pub shell: Shell,
    /// Terminal column width; used to right-align the exit/duration segment.
    /// Zero means no right-aligned segment is emitted.
    pub width: u16,
    /// Previous command duration in milliseconds. `None` means unknown.
    pub duration_ms: Option<u64>,
    /// Number of dirty files in the git working tree. 0 means clean / no repo.
    pub git_dirty: u32,
    /// Exit code of the previous command (0 = success).
    pub exit_code: u8,
}

/// Append a non-printing escape sequence, wrapped in the shell's zero-width
/// markers so the shell counts only the visible glyphs.
fn esc(buf: &mut String, shell: Shell, seq: &str) {
    match shell {
        Shell::Plain => buf.push_str(seq),
        Shell::Zsh => {
            buf.push_str("%{");
            buf.push_str(seq);
            buf.push_str("%}");
        }
        Shell::Bash => {
            buf.push('\x01');
            buf.push_str(seq);
            buf.push('\x02');
        }
    }
}

/// The full single-line prompt.
///
/// Layout (column 0 →):  `◒ [*N ·] ❯ `
///   - `◒` (U+25D2 — CIRCLE WITH LOWER HALF BLACK) is the Anvil "Basin" mark
///     in Unicode form, painted in the mineral accent. It gives the prompt
///     a brand-rooted, distinctive opening glyph instead of a bare chevron.
///   - `*N` appears between the basin and the middot when `opts.git_dirty > 0`,
///     in attention amber. E.g. `◒ *3 · ❯`.
///   - `❯` (U+276F) is the input glyph, in a slightly cooler tone than the
///     mark so the eye lands on the typed input that follows it.
///   - Both glyphs flip to the error red when the previous command exited
///     non-zero, so failures read at a glance from the prompt alone.
///
/// When `opts.width > 0` and there is an exit code or a duration, a
/// right-aligned secondary segment is emitted on the same line, padded with
/// spaces so it lands flush at column `opts.width`:
///   - failure: ` ✗ <code>  <duration>` (or ` ✗ <code>` when no duration)
///   - success: ` ✓ <duration>` (only when duration is present)
///
pub fn full(opts: Options) -> String {
    let mut buf = String::new();
    let sh = opts.shell;

    // Anvil's chrome row already carries cwd + branch + dirty count. The
    // prompt line itself is the bare `❯` with a trailing space — matches
    // Option D's `.prompt-line { color: var(--accent-bright); }`. The
    // right-aligned exit/duration segment still emits after the arrow.

    let arrow_color = if opts.failed {
        ACCENT_ERR
    } else {
        ACCENT_BRIGHT
    };

    let mut left_visible: u16 = 0;

    // Arrow.
    esc(&mut buf, sh, arrow_color);
    buf.push('\u{276f}'); // ❯
    esc(&mut buf, sh, RESET);
    buf.push(' ');
    left_visible += 2;

    // Right-aligned exit code + duration segment.
    // Build the visible text first, measure it, then pad with spaces.
    if let Some(right_text) = build_right_segment(opts) {
        let right_visible = right_text.visible_len as u16;
        // Padding: width - left_visible - right_visible spaces.
        let pad = opts
            .width
            .saturating_sub(left_visible)
            .saturating_sub(right_visible);
        for _ in 0..pad {
            buf.push(' ');
        }
        // Emit the right segment with ANSI escapes.
        emit_right_segment(&mut buf, sh, opts, &right_text);
    }

    esc(&mut buf, sh, "\x1b]133;B\x07");
    buf
}

/// Visible text and length for the right-aligned segment.
struct RightSegment {
    /// The plain-text representation (no ANSI) — used only to measure width.
    visible_len: usize,
    /// Whether this is a failure segment (exit != 0).
    failed: bool,
    /// Exit code (0 when success).
    exit_code: u8,
    /// Formatted duration string, e.g. "0.4s". `None` when no duration.
    duration: Option<String>,
}

/// Format a duration from milliseconds to a human-readable string.
/// Always expressed in seconds with one decimal place (e.g. "0.4s", "12.3s").
fn format_duration(ms: u64) -> String {
    let secs = ms as f64 / 1_000.0;
    format!("{:.1}s", secs)
}

/// Returns `Some(RightSegment)` when there is something to show on the right.
/// Returns `None` when exit == 0 and no duration — nothing to emit.
fn build_right_segment(opts: Options) -> Option<RightSegment> {
    let has_exit = opts.exit_code != 0;
    let duration = opts.duration_ms.map(format_duration);

    if !has_exit && duration.is_none() {
        return None;
    }
    if opts.width == 0 {
        return None;
    }

    // Measure visible length: ` ✗ 127  0.4s` or ` ✓ 0.4s`
    // U+2713 (✓) and U+2717 (✗) are single-column glyphs.
    let visible_len = if has_exit {
        // ` ✗ <code>` — space + glyph + space + digits
        let base = 1 + 1 + 1 + format!("{}", opts.exit_code).len();
        if let Some(d) = &duration {
            // `  <duration>` — two spaces + duration
            base + 2 + d.len()
        } else {
            base
        }
    } else {
        // ` ✓ <duration>`
        1 + 1 + 1 + duration.as_deref().unwrap_or("").len()
    };

    Some(RightSegment {
        visible_len,
        failed: has_exit,
        exit_code: opts.exit_code,
        duration,
    })
}

/// Write the right-aligned segment into `buf` using `esc()` for zero-width wrapping.
fn emit_right_segment(buf: &mut String, sh: Shell, opts: Options, seg: &RightSegment) {
    if seg.failed {
        // ` ✗ <code>` in error red
        buf.push(' ');
        esc(buf, sh, ACCENT_ERR);
        buf.push('\u{2717}'); // ✗
        buf.push(' ');
        buf.push_str(&format!("{}", seg.exit_code));
        esc(buf, sh, RESET);
        // `  <duration>` in dim grey (if present)
        if let Some(d) = &seg.duration {
            buf.push_str("  ");
            esc(buf, sh, DIM);
            buf.push_str(d);
            esc(buf, sh, RESET);
        }
    } else {
        // ` ✓ <duration>` in verified green
        buf.push(' ');
        esc(buf, sh, VERIFIED);
        buf.push('\u{2713}'); // ✓
        buf.push(' ');
        if let Some(d) = &seg.duration {
            buf.push_str(d);
        }
        esc(buf, sh, RESET);
    }
    let _ = opts; // suppress unused warning (opts.shell used via sh)
}

/// The collapsed transient prompt — `◒ · ❯` all in dim grey.
///
/// Same three-note shape as the live prompt, but flattened to one tone so
/// scrollback reads as a quiet echo of the active line, not a louder peer.
pub fn transient(opts: Options) -> String {
    let mut buf = String::new();
    let col = if opts.failed { ACCENT_ERR } else { DIM };
    esc(&mut buf, opts.shell, col);
    buf.push('\u{25d2}'); // ◒
    buf.push(' ');
    buf.push('\u{00b7}'); // ·
    buf.push(' ');
    buf.push('\u{276f}'); // ❯
    esc(&mut buf, opts.shell, RESET);
    buf.push(' ');
    buf
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Baseline options: no right segment, no dirty, no duration.
    fn base_opts(shell: Shell) -> Options {
        Options {
            rich: true,
            failed: false,
            shell,
            width: 0,
            duration_ms: None,
            git_dirty: 0,
            exit_code: 0,
        }
    }

    #[test]
    fn full_is_single_line_chevron_only() {
        // Single-line `❯  ` — no basin, no middot, no newline.
        let out = full(base_opts(Shell::Plain));
        assert!(!out.contains('\n'));
        assert!(out.contains('\u{276f}')); // ❯
        assert!(!out.contains('\u{25d2}')); // no basin
        assert!(!out.contains('\u{00b7}')); // no middot
    }

    #[test]
    fn full_paints_chevron_in_bright_accent_on_success() {
        let out = full(base_opts(Shell::Plain));
        assert!(out.contains(ACCENT_BRIGHT));
    }

    #[test]
    fn transient_carries_all_three_glyphs() {
        let out = transient(base_opts(Shell::Plain));
        assert!(out.contains('\u{25d2}'));
        assert!(out.contains('\u{00b7}'));
        assert!(out.contains('\u{276f}'));
    }

    #[test]
    fn full_uses_indexed_accent_err_colour_on_failure() {
        let ok = full(base_opts(Shell::Plain));
        let bad = full(Options {
            failed: true,
            ..base_opts(Shell::Plain)
        });
        assert!(bad.contains(ACCENT_ERR));
        assert!(!ok.contains(ACCENT_ERR));
    }

    #[test]
    fn full_uses_bright_accent_for_the_chevron() {
        let out = full(base_opts(Shell::Plain));
        assert!(out.contains(ACCENT_BRIGHT));
    }

    #[test]
    fn transient_is_a_single_line() {
        let out = transient(base_opts(Shell::Plain));
        assert!(!out.contains('\n'));
    }

    #[test]
    fn full_emits_the_osc_133b_prompt_end_mark() {
        let out = full(base_opts(Shell::Plain));
        assert!(out.contains("\x1b]133;B"));
    }

    #[test]
    fn full_does_not_contain_a_rule_line() {
        let out = full(base_opts(Shell::Plain));
        // The box-drawing horizontal bar character must not appear in the prompt text.
        assert!(!out.contains('\u{2500}'));
    }

    #[test]
    fn zsh_mode_wraps_escape_sequences_in_zero_width_markers() {
        let out = full(base_opts(Shell::Zsh));
        assert!(out.contains("%{"));
        assert!(out.contains("%}"));
    }

    #[test]
    fn transient_uses_indexed_dim_colour_when_not_failed() {
        let out = transient(base_opts(Shell::Plain));
        assert!(out.contains(DIM));
    }

    #[test]
    fn transient_uses_indexed_accent_err_colour_when_failed() {
        let out = transient(Options {
            failed: true,
            ..base_opts(Shell::Plain)
        });
        assert!(out.contains(ACCENT_ERR));
    }

    #[test]
    fn bash_mode_wraps_escape_sequences_in_rl_markers() {
        let out = full(base_opts(Shell::Bash));
        // Bash zero-width markers are \x01 ... \x02
        assert!(out.contains('\x01'));
        assert!(out.contains('\x02'));
    }

    // ── New tests for Task #11 and Task #12 ───────────────────────────────────

    #[test]
    fn full_does_not_embed_dirty_count_or_branch_pills() {
        // The chrome row carries cwd/branch/dirty count now; the shell prompt
        // is just `❯ ` + optional right-aligned exit/duration. Matches D's
        // `.prompt-line` which is bare.
        let out = full(base_opts(Shell::Plain));
        assert!(
            !out.contains("*3"),
            "dirty count must not appear in the prompt"
        );
        assert!(
            !out.contains("main"),
            "branch must not appear in the prompt"
        );
        assert!(out.contains('\u{276f}'));
    }

    #[test]
    fn full_with_exit_code_right_aligns_failure_indicator() {
        // ` ✗ 127` appears on the same line as the prompt when exit != 0.
        let out = full(Options {
            failed: true,
            exit_code: 127,
            width: 80,
            ..base_opts(Shell::Plain)
        });
        // Failure glyph present.
        assert!(out.contains('\u{2717}')); // ✗
        // Exit code present.
        assert!(out.contains("127"));
        // Error red color used for the right segment.
        assert!(out.contains(ACCENT_ERR));
        // No newline — same line.
        assert!(!out.contains('\n'));
    }

    #[test]
    fn full_with_duration_only_shows_check_in_verified_green() {
        // ` ✓ 0.4s` appears on the right when exit == 0 but duration is set.
        let out = full(Options {
            duration_ms: Some(400),
            width: 80,
            ..base_opts(Shell::Plain)
        });
        // Success glyph present.
        assert!(out.contains('\u{2713}')); // ✓
        // Duration formatted.
        assert!(out.contains("0.4s"));
        // Verified green color used.
        assert!(out.contains(VERIFIED));
        // No failure glyph.
        assert!(!out.contains('\u{2717}'));
    }

    #[test]
    fn full_omits_right_segment_when_no_exit_and_no_duration() {
        // No right segment when exit == 0 and no duration.
        let out = full(base_opts(Shell::Plain));
        // Neither success nor failure glyph.
        assert!(!out.contains('\u{2713}')); // ✓
        assert!(!out.contains('\u{2717}')); // ✗
    }
}
