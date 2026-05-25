//! Tree-sitter syntax layer — NE8.
//!
//! `SyntaxLayer` holds a single tree-sitter `Parser` + `Tree` for one buffer.
//! It exposes:
//! - `set_language_from_path` — detect grammar from file extension.
//! - `parse` — full (re)parse.
//! - `edit` — incremental reparse after a buffer mutation.
//! - `highlights_for_range` — query capture spans for a byte range; results
//!   are cached by `(start_byte, end_byte)` and invalidated by `invalidate`.
//!
//! The caller (`Buffer::apply_edit`) calls `invalidate()` on every edit so the
//! cache is never stale.

use std::cell::RefCell;
use std::ops::Range;
use std::path::Path;

use streaming_iterator::StreamingIterator;
use tree_sitter::{InputEdit, Language, Parser, Query, QueryCursor, Tree};

// ── SyntaxRole ───────────────────────────────────────────────────────────────

/// Semantic highlight role for a byte span.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyntaxRole {
    Plain,
    Keyword,
    String,
    Number,
    Comment,
    Function,
    Type,
    Variable,
    Operator,
    Punctuation,
}

// ── capture name → SyntaxRole mapping ────────────────────────────────────────

fn capture_name_to_role(name: &str) -> SyntaxRole {
    // Match on the first dot-segment so that sub-captures like
    // "keyword.control" or "string.special" resolve correctly.
    let base = name.split('.').next().unwrap_or(name);
    match base {
        "keyword" => SyntaxRole::Keyword,
        "string" => SyntaxRole::String,
        "number" | "float" | "integer" | "constant" => SyntaxRole::Number,
        "comment" => SyntaxRole::Comment,
        "function" | "method" => SyntaxRole::Function,
        "type" | "constructor" | "class" => SyntaxRole::Type,
        "variable" | "parameter" | "property" => SyntaxRole::Variable,
        "operator" => SyntaxRole::Operator,
        "punctuation" | "delimiter" | "bracket" => SyntaxRole::Punctuation,
        _ => SyntaxRole::Plain,
    }
}

// ── SyntaxLayer ──────────────────────────────────────────────────────────────

/// A cached result from one `highlights_for_range` call.
/// Key is `(start_byte, end_byte)`; value is the highlight spans.
type HighlightCache = Option<((usize, usize), Vec<(Range<usize>, SyntaxRole)>)>;

/// Per-buffer syntax state.
pub struct SyntaxLayer {
    parser: Parser,
    tree: Option<Tree>,
    /// Active tree-sitter language, if any.
    language: Option<Language>,
    /// Pre-compiled highlight query for the active language.
    query: Option<Query>,
    /// Cache: `(start_byte, end_byte)` → highlight spans.
    ///
    /// Wrapped in `RefCell` so `highlights_for_range` can take `&self` (the
    /// cache write is an interior mutation; callers holding `&Buffer` never
    /// need `&mut Buffer` just to query highlights).
    ///
    /// Invalidated by `invalidate()` which is called from `Buffer::apply_edit`.
    visible_cache: RefCell<HighlightCache>,
}

impl SyntaxLayer {
    pub fn new() -> Self {
        SyntaxLayer {
            parser: Parser::new(),
            tree: None,
            language: None,
            query: None,
            visible_cache: RefCell::new(None),
        }
    }

    // ── Language detection ────────────────────────────────────────────────────

    /// Select the grammar based on `path`'s extension. Silently no-ops if the
    /// extension is unrecognized; `highlights_for_range` will return empty.
    pub fn set_language_from_path(&mut self, path: &Path) {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();

        let (lang, query_src): (Language, &'static str) = match ext.as_str() {
            "rs" => (
                tree_sitter_rust::LANGUAGE.into(),
                tree_sitter_rust::HIGHLIGHTS_QUERY,
            ),
            "ts" | "tsx" => (
                tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
                tree_sitter_typescript::HIGHLIGHTS_QUERY,
            ),
            "py" => (
                tree_sitter_python::LANGUAGE.into(),
                tree_sitter_python::HIGHLIGHTS_QUERY,
            ),
            "toml" => (
                tree_sitter_toml_ng::LANGUAGE.into(),
                tree_sitter_toml_ng::HIGHLIGHTS_QUERY,
            ),
            "json" => (
                tree_sitter_json::LANGUAGE.into(),
                tree_sitter_json::HIGHLIGHTS_QUERY,
            ),
            "md" | "markdown" => (
                tree_sitter_md::LANGUAGE.into(),
                tree_sitter_md::HIGHLIGHT_QUERY_BLOCK,
            ),
            _ => return,
        };

        if self.parser.set_language(&lang).is_err() {
            return;
        }
        // Build the highlight query; ignore grammars whose bundled query fails
        // to compile (shouldn't happen with the official crates).
        let q = match Query::new(&lang, query_src) {
            Ok(q) => q,
            Err(_) => return,
        };
        self.language = Some(lang);
        self.query = Some(q);
        self.tree = None;
        *self.visible_cache.borrow_mut() = None;
    }

    // ── Parse / edit ─────────────────────────────────────────────────────────

    /// Full parse. Call after loading a buffer from disk.
    pub fn parse(&mut self, text: &str) {
        if self.language.is_none() {
            return;
        }
        self.tree = self.parser.parse(text, None);
        *self.visible_cache.borrow_mut() = None;
    }

    /// Incremental reparse after a buffer edit.
    ///
    /// `edit` describes the byte positions that changed. After calling this,
    /// the tree is updated in place (tree-sitter only reparses changed nodes).
    pub fn edit(&mut self, edit: InputEdit, text: &str) {
        if let Some(tree) = &mut self.tree {
            tree.edit(&edit);
            self.tree = self.parser.parse(text, self.tree.as_ref());
        }
        *self.visible_cache.borrow_mut() = None;
    }

    /// Invalidate the highlights cache. Called by `Buffer::apply_edit`.
    pub fn invalidate(&mut self) {
        *self.visible_cache.borrow_mut() = None;
    }

    // ── Highlights query ──────────────────────────────────────────────────────

    /// Return highlight spans overlapping `[start_byte, end_byte)`.
    ///
    /// Results are cached: if the same range is requested again (and no
    /// invalidation happened), the cached `Vec` is cloned and returned without
    /// re-querying tree-sitter.
    ///
    /// Takes `&self` (not `&mut self`) so callers that hold an immutable
    /// `&Buffer` can query highlights.  The cache write uses `RefCell` interior
    /// mutability.
    pub fn highlights_for_range(
        &self,
        start_byte: usize,
        end_byte: usize,
        text: &str,
    ) -> Vec<(Range<usize>, SyntaxRole)> {
        // Cache hit? Clone the stored Vec to avoid holding the borrow across
        // the tree-sitter query below.
        {
            let cached = self.visible_cache.borrow();
            if let Some(((cs, ce), ref spans)) = *cached {
                if cs == start_byte && ce == end_byte {
                    return spans.clone();
                }
            }
        }

        let (Some(tree), Some(query)) = (&self.tree, &self.query) else {
            *self.visible_cache.borrow_mut() = Some(((start_byte, end_byte), Vec::new()));
            return Vec::new();
        };

        let mut cursor = QueryCursor::new();
        cursor.set_byte_range(start_byte..end_byte);

        let mut spans: Vec<(Range<usize>, SyntaxRole)> = Vec::new();
        let capture_names = query.capture_names();
        let mut matches = cursor.captures(query, tree.root_node(), text.as_bytes());

        while let Some((m, cap_idx)) = matches.next() {
            let cap = &m.captures[*cap_idx];
            let name = capture_names[cap.index as usize];
            let role = capture_name_to_role(name);
            if role != SyntaxRole::Plain {
                spans.push((cap.node.byte_range(), role));
            }
        }

        *self.visible_cache.borrow_mut() = Some(((start_byte, end_byte), spans.clone()));
        spans
    }
}

impl Default for SyntaxLayer {
    fn default() -> Self {
        Self::new()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tree_sitter::Point;

    fn path(name: &str) -> PathBuf {
        PathBuf::from(name)
    }

    // ── NE8-T1: language selection from path extension ────────────────────────

    #[test]
    fn syntax_set_language_from_rs_path() {
        let mut layer = SyntaxLayer::new();
        layer.set_language_from_path(&path("main.rs"));
        assert!(
            layer.language.is_some(),
            "Rust path should activate a language"
        );
        assert!(layer.query.is_some(), "Rust path should build a query");
    }

    #[test]
    fn syntax_no_language_for_unknown_extension() {
        let mut layer = SyntaxLayer::new();
        layer.set_language_from_path(&path("binary.exe"));
        assert!(layer.language.is_none());
    }

    // ── NE8-T2: parse Rust fn → keyword capture ───────────────────────────────

    #[test]
    fn syntax_parse_rust_fn_extracts_keyword() {
        let src = "fn hello() {}";
        let mut layer = SyntaxLayer::new();
        layer.set_language_from_path(&path("hello.rs"));
        layer.parse(src);

        let spans = layer.highlights_for_range(0, src.len(), src);
        let has_keyword = spans
            .iter()
            .any(|(r, role)| *role == SyntaxRole::Keyword && &src[r.clone()] == "fn");
        assert!(
            has_keyword,
            "expected 'fn' to be tagged as Keyword; got: {spans:?}"
        );
    }

    // ── NE8-T3: parse Python → string role ───────────────────────────────────

    #[test]
    fn syntax_parse_python_string_extracts_string_role() {
        let src = "x = \"hello\"\n";
        let mut layer = SyntaxLayer::new();
        layer.set_language_from_path(&path("script.py"));
        layer.parse(src);

        let spans = layer.highlights_for_range(0, src.len(), src);
        let has_string = spans.iter().any(|(_, role)| *role == SyntaxRole::String);
        assert!(
            has_string,
            "expected a String role in Python source; got: {spans:?}"
        );
    }

    // ── NE8-T4: incremental edit preserves tree ───────────────────────────────

    #[test]
    fn syntax_incremental_edit_preserves_tree() {
        let src = "fn hello() {}";
        let mut layer = SyntaxLayer::new();
        layer.set_language_from_path(&path("hello.rs"));
        layer.parse(src);
        assert!(layer.tree.is_some());

        // Insert a space before `{` — a minimal edit.
        let new_src = "fn hello() { }";
        // InputEdit: we inserted one byte at position 12.
        let edit = InputEdit {
            start_byte: 12,
            old_end_byte: 12,
            new_end_byte: 13,
            start_position: Point::new(0, 12),
            old_end_position: Point::new(0, 12),
            new_end_position: Point::new(0, 13),
        };
        layer.edit(edit, new_src);
        assert!(
            layer.tree.is_some(),
            "tree should survive an incremental edit"
        );

        // After edit, the keyword 'fn' should still parse.
        let spans = layer.highlights_for_range(0, new_src.len(), new_src);
        let has_keyword = spans
            .iter()
            .any(|(r, role)| *role == SyntaxRole::Keyword && &new_src[r.clone()] == "fn");
        assert!(
            has_keyword,
            "incremental edit must preserve keyword highlight"
        );
    }

    // ── NE8-T5: no language → empty highlights ────────────────────────────────

    #[test]
    fn syntax_no_language_returns_empty_highlights() {
        let src = "hello world";
        let mut layer = SyntaxLayer::new();
        // No set_language_from_path call.
        layer.parse(src);
        let spans = layer.highlights_for_range(0, src.len(), src);
        assert!(spans.is_empty(), "expected no spans without a language");
    }

    // ── NE8-T6: cache hit avoids re-query ────────────────────────────────────

    #[test]
    fn syntax_cache_hit_on_same_range() {
        let src = "fn main() {}";
        let mut layer = SyntaxLayer::new();
        layer.set_language_from_path(&path("main.rs"));
        layer.parse(src);

        let s1 = layer.highlights_for_range(0, src.len(), src).to_vec();
        let s2 = layer.highlights_for_range(0, src.len(), src).to_vec();
        assert_eq!(s1, s2, "cache hit must return the same spans");
    }

    // ── NE8-T7: invalidate clears cache ──────────────────────────────────────

    #[test]
    fn syntax_invalidate_clears_cache() {
        let src = "fn main() {}";
        let mut layer = SyntaxLayer::new();
        layer.set_language_from_path(&path("main.rs"));
        layer.parse(src);

        // Populate cache.
        let _ = layer.highlights_for_range(0, src.len(), src);
        assert!(layer.visible_cache.borrow().is_some());

        // Invalidate.
        layer.invalidate();
        assert!(
            layer.visible_cache.borrow().is_none(),
            "invalidate must clear the cache"
        );
    }
}
