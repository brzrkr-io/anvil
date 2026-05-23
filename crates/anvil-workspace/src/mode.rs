//! Layout mode: Terminal (default) vs IDE.

/// The two top-level layout arrangements.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum LayoutMode {
    /// Terminal mode: pane tree only; file tree and agent panel are optional
    /// and user-toggled.
    #[default]
    Terminal,
    /// IDE mode: file tree always shown left, agent panel docked full-height
    /// right, status bar bottom.
    Ide,
}

/// Cycle to the next layout mode.
pub fn next(m: LayoutMode) -> LayoutMode {
    match m {
        LayoutMode::Terminal => LayoutMode::Ide,
        LayoutMode::Ide => LayoutMode::Terminal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_terminal_gives_ide() {
        assert_eq!(next(LayoutMode::Terminal), LayoutMode::Ide);
    }

    #[test]
    fn next_ide_gives_terminal() {
        assert_eq!(next(LayoutMode::Ide), LayoutMode::Terminal);
    }

    #[test]
    fn next_cycles_back_to_start() {
        let m = LayoutMode::Terminal;
        assert_eq!(next(next(m)), m);
    }

    #[test]
    fn default_is_terminal() {
        assert_eq!(LayoutMode::default(), LayoutMode::Terminal);
    }
}
