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
        // Markdown (tree-sitter-md) uses "text.*" capture names.
        // Map the sub-segments to the closest existing roles so that headers,
        // code spans, and links get syntax colour in the editor.
        "text" => match name {
            "text.title" => SyntaxRole::Keyword, // accent_bright — matches `^#+` headings
            "text.literal" => SyntaxRole::String, // code spans / fenced code blocks
            "text.uri" => SyntaxRole::Function,  // link destinations / URLs
            "text.reference" => SyntaxRole::Type, // link labels
            _ => SyntaxRole::Plain,
        },
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

    /// True when this layer has a parsed tree and a highlight query.
    pub fn has_highlights(&self) -> bool {
        self.tree.is_some() && self.query.is_some()
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

// ── Outline derivation (NE9 / item 19) ───────────────────────────────────────

/// Symbol kind as returned by `derive_outline_rows`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutlineSymbolKind {
    Function,
    Impl,
    Struct,
    Enum,
    Trait,
    Other,
}

/// A single symbol entry produced by [`derive_outline_rows`].
#[derive(Debug, Clone)]
pub struct OutlineSymbol {
    pub kind: OutlineSymbolKind,
    /// Display name (identifier token, or best-effort text).
    pub name: String,
    /// 0-based line number in the buffer.
    pub line: usize,
}

/// Walk the syntax tree for the top-level named declarations in `layer` and
/// return them as a flat list.  Falls back to a line-regex scan when no tree
/// is available (e.g. non-Rust buffers where the tree-sitter grammar isn't
/// loaded).
///
/// Capped at 500 nodes for performance.
pub fn derive_outline_rows(layer: &SyntaxLayer, text: &str) -> Vec<OutlineSymbol> {
    const NODE_CAP: usize = 500;

    // Tree-sitter path: walk the tree looking for named declaration nodes.
    if let Some(tree) = &layer.tree {
        let mut results = Vec::new();
        let mut stack: Vec<tree_sitter::Node<'_>> = vec![tree.root_node()];
        let mut visited = 0usize;

        while let Some(node) = stack.pop() {
            visited += 1;
            if visited > NODE_CAP {
                break;
            }

            let kind = node.kind();
            let sym_kind = match kind {
                "function_item" => Some(OutlineSymbolKind::Function),
                "impl_item" => Some(OutlineSymbolKind::Impl),
                "struct_item" => Some(OutlineSymbolKind::Struct),
                "enum_item" => Some(OutlineSymbolKind::Enum),
                "trait_item" => Some(OutlineSymbolKind::Trait),
                _ => None,
            };

            if let Some(sk) = sym_kind {
                // Extract the name from the first "name" or "type_identifier" child.
                let name = node
                    .child_by_field_name("name")
                    .and_then(|n| text.get(n.byte_range()))
                    .unwrap_or(kind)
                    .to_string();
                let line = node.start_position().row;
                results.push(OutlineSymbol {
                    kind: sk,
                    name,
                    line,
                });
                // Don't recurse into declaration bodies — top-level only.
                continue;
            }

            // Push children in reverse so we visit left-to-right.
            for i in (0..node.child_count()).rev() {
                if let Some(child) = node.child(i) {
                    stack.push(child);
                }
            }
        }

        if !results.is_empty() {
            return results;
        }
    }

    // Regex-fallback path: scan text lines for fn/struct/impl/enum/trait.
    let mut results = Vec::new();
    for (line_idx, line) in text.lines().enumerate() {
        if results.len() >= NODE_CAP {
            break;
        }
        let trimmed = line.trim_start();
        let (sym_kind, prefix) = if trimmed.starts_with("pub fn ")
            || trimmed.starts_with("fn ")
            || trimmed.starts_with("async fn ")
            || trimmed.starts_with("pub async fn ")
        {
            let prefix = if trimmed.starts_with("pub async fn ") {
                "pub async fn "
            } else if trimmed.starts_with("async fn ") {
                "async fn "
            } else if trimmed.starts_with("pub fn ") {
                "pub fn "
            } else {
                "fn "
            };
            (OutlineSymbolKind::Function, prefix)
        } else if trimmed.starts_with("pub struct ") || trimmed.starts_with("struct ") {
            let prefix = if trimmed.starts_with("pub struct ") {
                "pub struct "
            } else {
                "struct "
            };
            (OutlineSymbolKind::Struct, prefix)
        } else if trimmed.starts_with("impl ") {
            (OutlineSymbolKind::Impl, "impl ")
        } else if trimmed.starts_with("pub enum ") || trimmed.starts_with("enum ") {
            let prefix = if trimmed.starts_with("pub enum ") {
                "pub enum "
            } else {
                "enum "
            };
            (OutlineSymbolKind::Enum, prefix)
        } else if trimmed.starts_with("pub trait ") || trimmed.starts_with("trait ") {
            let prefix = if trimmed.starts_with("pub trait ") {
                "pub trait "
            } else {
                "trait "
            };
            (OutlineSymbolKind::Trait, prefix)
        } else {
            continue;
        };

        // Take the identifier up to the first whitespace or '{' or '<' or '('.
        let rest = &trimmed[prefix.len()..];
        let name: String = rest
            .chars()
            .take_while(|&c| c != ' ' && c != '{' && c != '<' && c != '(' && c != '\n')
            .collect();
        if name.is_empty() {
            continue;
        }
        results.push(OutlineSymbol {
            kind: sym_kind,
            name,
            line: line_idx,
        });
    }
    results
}

// ── Fold range derivation (item 13) ───────────────────────────────────────────

/// A foldable range `(start_line, end_line)` where both are 0-indexed.
///
/// The fold covers lines `start_line+1..=end_line` — the start line stays
/// visible; lines after it are hidden when the fold is active.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FoldRange {
    /// The line that starts the foldable block (e.g. `fn foo() {`).
    pub start: usize,
    /// The last line of the block body (the line containing the closing `}`).
    pub end: usize,
}

/// Walk the syntax tree to find foldable ranges for Rust files.
///
/// Foldable node kinds: `function_item`, `impl_item`, `struct_item`,
/// `enum_item`, `trait_item`, `mod_item`, `block`. For non-Rust buffers (or
/// when no tree is available), returns an empty `Vec`.
///
/// Capped at 500 nodes.
pub fn derive_fold_ranges(layer: &SyntaxLayer) -> Vec<FoldRange> {
    const NODE_CAP: usize = 500;

    let tree = match &layer.tree {
        Some(t) => t,
        None => return Vec::new(),
    };

    let mut results = Vec::new();
    let mut stack: Vec<tree_sitter::Node<'_>> = vec![tree.root_node()];
    let mut visited = 0usize;

    while let Some(node) = stack.pop() {
        visited += 1;
        if visited > NODE_CAP {
            break;
        }

        let kind = node.kind();
        let is_foldable = matches!(
            kind,
            "function_item"
                | "impl_item"
                | "struct_item"
                | "enum_item"
                | "trait_item"
                | "mod_item"
                | "block"
        );

        if is_foldable {
            let start = node.start_position().row;
            let end = node.end_position().row;
            if end > start {
                results.push(FoldRange { start, end });
            }
        }

        // Always recurse to collect nested foldable ranges.
        for i in (0..node.child_count()).rev() {
            if let Some(child) = node.child(i) {
                stack.push(child);
            }
        }
    }

    results
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

    // ── Item 19: outline derivation ───────────────────────────────────────────

    #[test]
    fn derive_outline_rows_tree_sitter_finds_rust_fn() {
        let src = "pub fn hello() {}\nfn world() {}\n";
        let mut layer = SyntaxLayer::new();
        layer.set_language_from_path(&path("lib.rs"));
        layer.parse(src);

        let rows = super::derive_outline_rows(&layer, src);
        let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
        assert!(
            names.contains(&"hello"),
            "outline must contain 'hello'; got {names:?}"
        );
        assert!(
            names.contains(&"world"),
            "outline must contain 'world'; got {names:?}"
        );
    }

    #[test]
    fn derive_outline_rows_fallback_regex_finds_struct() {
        // No tree (no set_language_from_path call) → regex path.
        let src = "struct Foo {}\nimpl Foo {}\n";
        let layer = SyntaxLayer::new();
        let rows = super::derive_outline_rows(&layer, src);
        let kinds: Vec<super::OutlineSymbolKind> = rows.iter().map(|r| r.kind).collect();
        assert!(
            kinds.contains(&super::OutlineSymbolKind::Struct),
            "fallback must find Struct; got {kinds:?}"
        );
        assert!(
            kinds.contains(&super::OutlineSymbolKind::Impl),
            "fallback must find Impl; got {kinds:?}"
        );
    }

    #[test]
    fn derive_outline_rows_records_line_numbers() {
        let src = "fn first() {}\n\nfn second() {}\n";
        let mut layer = SyntaxLayer::new();
        layer.set_language_from_path(&path("a.rs"));
        layer.parse(src);
        let rows = super::derive_outline_rows(&layer, src);
        assert_eq!(rows.len(), 2, "expected 2 symbols; got {rows:?}");
        assert_eq!(rows[0].line, 0, "first fn is on line 0");
        assert_eq!(rows[1].line, 2, "second fn is on line 2");
    }

    // ── G3: markdown syntax highlighting ─────────────────────────────────────

    /// Markdown headings must resolve to SyntaxRole::Keyword (accent_bright).
    /// The tree-sitter-md grammar emits "text.title" for `(atx_heading (inline))`.
    #[test]
    fn syntax_markdown_heading_resolves_to_keyword() {
        let src = "# Hello World\n\nsome body text\n";
        let mut layer = SyntaxLayer::new();
        layer.set_language_from_path(&path("README.md"));
        layer.parse(src);

        let spans = layer.highlights_for_range(0, src.len(), src);
        let has_heading = spans.iter().any(|(_, role)| *role == SyntaxRole::Keyword);
        assert!(
            has_heading,
            "expected a Keyword role for markdown heading; got: {spans:?}"
        );
    }

    /// Markdown fenced code blocks must resolve to SyntaxRole::String.
    #[test]
    fn syntax_markdown_code_block_resolves_to_string() {
        let src = "```\nlet x = 1;\n```\n";
        let mut layer = SyntaxLayer::new();
        layer.set_language_from_path(&path("notes.md"));
        layer.parse(src);

        let spans = layer.highlights_for_range(0, src.len(), src);
        let has_literal = spans.iter().any(|(_, role)| *role == SyntaxRole::String);
        assert!(
            has_literal,
            "expected a String role for markdown code block; got: {spans:?}"
        );
    }
}
