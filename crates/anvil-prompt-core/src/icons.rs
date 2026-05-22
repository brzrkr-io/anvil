//! Prompt icon glyphs. `rich` glyphs are Nerd Font v3 codepoints, carried by
//! the bundled BlexMono Nerd Font Mono; `ascii` fallbacks render anywhere. The
//! two-form table is the single swap point for icon rendering.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Repo,
    Branch,
    Dirty,
    Ahead,
    Behind,
    Toolchain,
    Container,
    Cluster,
    Ok,
    Err,
    Clock,
}

/// The glyph for `icon`. When `rich` is false, returns a plain-ASCII fallback.
pub fn glyph(icon: Icon, rich: bool) -> &'static str {
    match icon {
        Icon::Repo => {
            if rich { "\u{f07b}" } else { "#" } // nf-fa-folder
        }
        Icon::Branch => {
            if rich { "\u{e0a0}" } else { "@" } // nf-pl-branch
        }
        Icon::Dirty => {
            if rich { "\u{f111}" } else { "*" } // nf-fa-circle
        }
        Icon::Ahead => {
            if rich { "\u{f062}" } else { "^" } // nf-fa-arrow_up
        }
        Icon::Behind => {
            if rich { "\u{f063}" } else { "v" } // nf-fa-arrow_down
        }
        Icon::Toolchain => {
            if rich { "\u{f085}" } else { "=" } // nf-fa-cogs
        }
        Icon::Container => {
            if rich { "\u{f308}" } else { "[]" } // nf-linux-docker
        }
        Icon::Cluster => {
            if rich { "\u{f10fe}" } else { "{}" } // nf-md-kubernetes
        }
        Icon::Ok => {
            if rich { "\u{f00c}" } else { "ok" } // nf-fa-check
        }
        Icon::Err => {
            if rich { "\u{f00d}" } else { "x" } // nf-fa-close
        }
        Icon::Clock => {
            if rich { "\u{f017}" } else { "@" } // nf-fa-clock_o
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rich_glyphs_differ_from_ascii_fallbacks() {
        assert_ne!(glyph(Icon::Branch, true), glyph(Icon::Branch, false));
        assert_ne!(glyph(Icon::Ok, true), glyph(Icon::Ok, false));
    }

    #[test]
    fn every_icon_has_a_non_empty_glyph_in_both_modes() {
        let all = [
            Icon::Repo,
            Icon::Branch,
            Icon::Dirty,
            Icon::Ahead,
            Icon::Behind,
            Icon::Toolchain,
            Icon::Container,
            Icon::Cluster,
            Icon::Ok,
            Icon::Err,
            Icon::Clock,
        ];
        for icon in all {
            assert!(!glyph(icon, true).is_empty());
            assert!(!glyph(icon, false).is_empty());
        }
    }
}
