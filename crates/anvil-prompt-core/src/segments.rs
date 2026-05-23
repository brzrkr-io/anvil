//! The prompt's segment model. A Segment is one unit on the context line:
//! an icon, a text value, and a state that drives its colour.

use crate::icons::Icon;

/// Drives the segment's colour at render time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Normal,
    Ok,
    Warn,
    Err,
    Run,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    pub icon: Icon,
    pub text: String,
    pub state: State,
}

impl Segment {
    pub fn new(icon: Icon, text: impl Into<String>) -> Self {
        Self {
            icon,
            text: text.into(),
            state: State::Normal,
        }
    }

    pub fn with_state(icon: Icon, text: impl Into<String>, state: State) -> Self {
        Self {
            icon,
            text: text.into(),
            state,
        }
    }
}

/// A fixed-capacity segment list — the prompt never shows more than this many.
pub const MAX_SEGMENTS: usize = 12;

pub struct List {
    items: Vec<Segment>,
}

impl List {
    pub fn new() -> Self {
        Self {
            items: Vec::with_capacity(MAX_SEGMENTS),
        }
    }

    pub fn add(&mut self, seg: Segment) {
        if self.items.len() < MAX_SEGMENTS {
            self.items.push(seg);
        }
    }

    pub fn slice(&self) -> &[Segment] {
        &self.items
    }
}

impl Default for List {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_add_appends_until_capacity() {
        let mut l = List::new();
        assert_eq!(l.slice().len(), 0);
        l.add(Segment::new(Icon::Branch, "main"));
        assert_eq!(l.slice().len(), 1);
        assert_eq!(l.slice()[0].text, "main");
    }

    #[test]
    fn list_add_stops_at_capacity_never_overflows() {
        let mut l = List::new();
        for _ in 0..MAX_SEGMENTS + 5 {
            l.add(Segment::new(Icon::Repo, "x"));
        }
        assert_eq!(l.slice().len(), MAX_SEGMENTS);
    }

    #[test]
    fn list_default_is_empty() {
        let l = List::default();
        assert_eq!(l.slice().len(), 0);
    }
}
