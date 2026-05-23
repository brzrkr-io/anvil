//! Renders a segment list to an ANSI prompt string. Every escape sequence is
//! wrapped in the shell's zero-width markers (`%{ %}` for zsh, `\001 \002` for
//! bash) — without that the shell miscounts the prompt's visible width and
//! typed input lands in the wrong column.
//!
//! Colors are emitted as indexed ANSI colors (`\x1b[38;5;Nm`) so the terminal
//! re-resolves them through the active theme palette on every frame. A theme
//! switch therefore recolors all prompts in scrollback automatically.

use crate::icons::Icon;
use crate::icons::glyph;
use crate::segments::{Segment, State};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shell {
    Plain,
    Zsh,
    Bash,
}

const RESET: &str = "\x1b[0m";
const EDGE: &str = "\u{258e}"; // ▎

// Indexed ANSI colors — resolved through the active theme each frame.
const ANCHOR: &str = "\x1b[39m"; // default foreground — cwd anchor; flips with theme
const ACCENT: &str = "\x1b[38;5;6m"; // ANSI 6 = mineral/cyan — edge glyph, prompt glyph
const ACCENT_ERR: &str = "\x1b[38;5;1m"; // ANSI 1 = red — error state
const DIM: &str = "\x1b[38;5;8m"; // ANSI 8 = readable dim grey
const GIT_COLOR: &str = "\x1b[38;5;6m"; // ANSI 6 = teal/mineral
const TOOL_COLOR: &str = "\x1b[38;5;5m"; // ANSI 5 = magenta — toolchain
const INFRA_COLOR: &str = "\x1b[38;5;4m"; // ANSI 4 = blue — container/cluster
const WARN_COLOR: &str = "\x1b[38;5;3m"; // ANSI 3 = yellow/amber — attention
const OK_COLOR: &str = "\x1b[38;5;2m"; // ANSI 2 = green

/// A segment's colour: an attention state (dirty / failed) wins; otherwise the
/// colour is keyed to the segment's type.
fn seg_color(s: &Segment) -> &'static str {
    match s.state {
        State::Warn => return WARN_COLOR,
        State::Err => return ACCENT_ERR,
        State::Ok => return OK_COLOR,
        State::Run => return ACCENT,
        State::Normal => {}
    }
    match s.icon {
        Icon::Repo => ANCHOR,
        Icon::Branch => GIT_COLOR,
        Icon::Toolchain => TOOL_COLOR,
        Icon::Container | Icon::Cluster => INFRA_COLOR,
        _ => DIM,
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Options {
    pub rich: bool,
    /// Last command exited non-zero.
    pub failed: bool,
    pub shell: Shell,
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

/// The full two-line prompt block.
pub fn full(segments: &[Segment], opts: Options) -> String {
    let mut buf = String::new();
    let sh = opts.shell;

    let edge_color = if opts.failed { ACCENT_ERR } else { ACCENT };
    // Line 1: edge + segments.
    esc(&mut buf, sh, edge_color);
    buf.push_str(EDGE);
    esc(&mut buf, sh, RESET);
    buf.push_str("  ");
    for (idx, s) in segments.iter().enumerate() {
        if idx != 0 {
            // Dim middot separator gives the line real structure instead of
            // reading as a row of unrelated word gaps.
            buf.push_str("  ");
            esc(&mut buf, sh, DIM);
            buf.push('\u{00b7}'); // ·
            esc(&mut buf, sh, RESET);
            buf.push_str("  ");
        }
        esc(&mut buf, sh, seg_color(s));
        if opts.rich {
            buf.push_str(glyph(s.icon, true));
            buf.push(' ');
        }
        buf.push_str(&s.text);
        esc(&mut buf, sh, RESET);
    }
    buf.push('\n');
    // Line 2: edge + prompt glyph, aligned under the segments.
    esc(&mut buf, sh, edge_color);
    buf.push_str(EDGE);
    esc(&mut buf, sh, RESET);
    buf.push_str("  ");
    esc(&mut buf, sh, edge_color);
    buf.push('\u{276f}'); // ❯ — heavier than U+203A, the modern prompt glyph
    esc(&mut buf, sh, RESET);
    buf.push(' ');
    esc(&mut buf, sh, "\x1b]133;B\x07");
    buf
}

/// The collapsed transient prompt — just the glyph.
pub fn transient(opts: Options) -> String {
    let mut buf = String::new();
    let col = if opts.failed { ACCENT_ERR } else { DIM };
    esc(&mut buf, opts.shell, col);
    buf.push('\u{276f}'); // ❯ — matches the full prompt's heavier glyph
    esc(&mut buf, opts.shell, RESET);
    buf.push(' ');
    buf
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::segments::Segment;

    fn sample_segs() -> Vec<Segment> {
        vec![
            Segment::new(Icon::Repo, "anvil"),
            Segment::with_state(Icon::Branch, "main", State::Warn),
        ]
    }

    #[test]
    fn full_renders_two_lines_with_the_accent_edge() {
        let segs = sample_segs();
        let out = full(
            &segs,
            Options {
                rich: true,
                failed: false,
                shell: Shell::Plain,
            },
        );
        assert!(out.contains('\n'));
        assert!(out.contains(EDGE));
        assert!(out.contains("anvil"));
        assert!(out.contains("main"));
    }

    #[test]
    fn full_uses_indexed_accent_err_colour_on_failure() {
        let segs = sample_segs();
        let ok = full(
            &segs,
            Options {
                rich: true,
                failed: false,
                shell: Shell::Plain,
            },
        );
        let bad = full(
            &segs,
            Options {
                rich: true,
                failed: true,
                shell: Shell::Plain,
            },
        );
        assert!(bad.contains(ACCENT_ERR));
        assert!(!ok.contains(ACCENT_ERR));
    }

    #[test]
    fn the_cwd_anchor_uses_the_default_fg_indexed_color() {
        let segs = sample_segs();
        let out = full(
            &segs,
            Options {
                rich: true,
                failed: false,
                shell: Shell::Plain,
            },
        );
        assert!(out.contains(ANCHOR));
    }

    #[test]
    fn transient_is_a_single_line() {
        let out = transient(Options {
            rich: true,
            failed: false,
            shell: Shell::Plain,
        });
        assert!(!out.contains('\n'));
    }

    #[test]
    fn full_emits_the_osc_133b_prompt_end_mark() {
        let segs = sample_segs();
        let out = full(
            &segs,
            Options {
                rich: true,
                failed: false,
                shell: Shell::Plain,
            },
        );
        assert!(out.contains("\x1b]133;B"));
    }

    #[test]
    fn full_does_not_contain_a_rule_line() {
        let segs = sample_segs();
        let out = full(
            &segs,
            Options {
                rich: true,
                failed: false,
                shell: Shell::Plain,
            },
        );
        // The box-drawing horizontal bar character must not appear in the prompt text.
        assert!(!out.contains('\u{2500}'));
    }

    #[test]
    fn zsh_mode_wraps_escape_sequences_in_zero_width_markers() {
        let segs = sample_segs();
        let out = full(
            &segs,
            Options {
                rich: true,
                failed: false,
                shell: Shell::Zsh,
            },
        );
        assert!(out.contains("%{"));
        assert!(out.contains("%}"));
    }

    #[test]
    fn transient_uses_indexed_dim_colour_when_not_failed() {
        let out = transient(Options {
            rich: false,
            failed: false,
            shell: Shell::Plain,
        });
        assert!(out.contains(DIM));
    }

    #[test]
    fn transient_uses_indexed_accent_err_colour_when_failed() {
        let out = transient(Options {
            rich: false,
            failed: true,
            shell: Shell::Plain,
        });
        assert!(out.contains(ACCENT_ERR));
    }

    #[test]
    fn seg_color_state_err_returns_accent_err() {
        let s = Segment::with_state(Icon::Repo, "x", State::Err);
        assert_eq!(seg_color(&s), ACCENT_ERR);
    }

    #[test]
    fn seg_color_state_ok_returns_ok_color() {
        let s = Segment::with_state(Icon::Repo, "x", State::Ok);
        assert_eq!(seg_color(&s), OK_COLOR);
    }

    #[test]
    fn seg_color_icon_branch_returns_git_color() {
        // State::Normal → falls through to icon match → Branch → GIT_COLOR
        let s = Segment::new(Icon::Branch, "x");
        assert_eq!(seg_color(&s), GIT_COLOR);
    }

    #[test]
    fn seg_color_icon_toolchain_returns_tool_color() {
        let s = Segment::new(Icon::Toolchain, "x");
        assert_eq!(seg_color(&s), TOOL_COLOR);
    }

    #[test]
    fn seg_color_state_run_returns_accent() {
        let s = Segment::with_state(Icon::Repo, "x", State::Run);
        assert_eq!(seg_color(&s), ACCENT);
    }

    #[test]
    fn seg_color_state_normal_uses_icon_color() {
        // State::Normal falls through to the icon match.
        // Icon::Container => INFRA_COLOR
        let s = Segment::with_state(Icon::Container, "x", State::Normal);
        assert_eq!(seg_color(&s), INFRA_COLOR);
    }

    #[test]
    fn seg_color_icon_cluster_returns_infra_color() {
        let s = Segment::new(Icon::Cluster, "x");
        assert_eq!(seg_color(&s), INFRA_COLOR);
    }

    #[test]
    fn seg_color_catch_all_icon_returns_dim() {
        // Icon::Dirty is not Repo/Branch/Toolchain/Container/Cluster => DIM
        let s = Segment::new(Icon::Dirty, "x");
        assert_eq!(seg_color(&s), DIM);
    }

    #[test]
    fn bash_mode_wraps_escape_sequences_in_rl_markers() {
        let segs = sample_segs();
        let out = full(
            &segs,
            Options {
                rich: false,
                failed: false,
                shell: Shell::Bash,
            },
        );
        // Bash zero-width markers are \x01 ... \x02
        assert!(out.contains('\x01'));
        assert!(out.contains('\x02'));
    }
}
