//! Pure helpers for interactive terminal actions: token extraction at a column,
//! line-suffix stripping, and URL-vs-path classification.  No platform I/O
//! beyond an optional `access(2)` existence check.

use std::path::Path;

/// Extract the whitespace-delimited token from `line` that spans byte column
/// `col`.  Returns a sub-slice of `line`, or `""` if `col` is out of range or
/// the character at `col` is ASCII whitespace.
pub fn token_at_col(line: &str, col: usize) -> &str {
    let bytes = line.as_bytes();
    if col >= bytes.len() {
        return "";
    }
    if bytes[col].is_ascii_whitespace() {
        return "";
    }

    // Scan left for start.
    let mut start = col;
    while start > 0 && !bytes[start - 1].is_ascii_whitespace() {
        start -= 1;
    }

    // Scan right for end.
    let mut end = col + 1;
    while end < bytes.len() && !bytes[end].is_ascii_whitespace() {
        end += 1;
    }

    &line[start..end]
}

/// Strip a trailing `:line` or `:line:col` numeric suffix from `tok`.
/// E.g. `"src/main.zig:412"` → `"src/main.zig"`.
/// Returns a sub-slice of `tok` (no allocation).
pub fn strip_line_suffix(tok: &str) -> &str {
    let mut s = tok;
    for _ in 0..2 {
        let colon = match s.rfind(':') {
            Some(i) if i > 0 => i,
            _ => break,
        };
        let after = &s[colon + 1..];
        if after.is_empty() {
            break;
        }
        if !after.bytes().all(|b| b.is_ascii_digit()) {
            break;
        }
        s = &s[..colon];
    }
    s
}

/// Classification of a terminal token for ⌘-click handling.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Kind {
    Url,
    Path,
    /// A path with a trailing `:line` or `:line:col` suffix — the click
    /// handler should open the file at that location instead of the top.
    PathWithLine {
        path: String,
        line: u32,
        col: Option<u32>,
    },
    None,
}

/// Parse a `foo.rs:42` or `foo.rs:42:7` token into its parts.
///
/// Rules:
///   - The suffix must be all-digit segments separated by colons.
///   - Line is required and must be > 0 (editors are 1-based).
///   - Col is optional. A trailing `:0` for col is allowed and surfaced.
///   - Returns `None` if no recognizable line suffix.
pub fn parse_path_with_line(tok: &str) -> Option<(String, u32, Option<u32>)> {
    // Right-to-left: peel up to two `:N` segments. A non-numeric tail or
    // missing colon stops the scan (preserves whatever's already been
    // collected); we don't bail because the path itself may legitimately
    // contain colons earlier (e.g. `/some/path:weird`).
    let mut s = tok;
    let mut nums: Vec<u32> = Vec::new();
    for _ in 0..2 {
        let Some(colon) = s.rfind(':') else {
            break;
        };
        if colon == 0 {
            break;
        }
        let after = &s[colon + 1..];
        if after.is_empty() || !after.bytes().all(|b| b.is_ascii_digit()) {
            break;
        }
        let Ok(n) = after.parse() else {
            break;
        };
        nums.push(n);
        s = &s[..colon];
    }
    let path = s.to_string();
    match nums.as_slice() {
        [line] if *line > 0 => Some((path, *line, None)),
        [col, line] if *line > 0 => Some((path, *line, Some(*col))),
        _ => None,
    }
}

/// Classify `tok` (raw, may contain `:line[:col]` suffix) as a URL, a path
/// with optional line, a plain path, or neither.
///
/// `cwd` is used to probe for file existence when other heuristics are
/// inconclusive.  Pass `""` to skip the existence check.
pub fn classify(tok: &str, cwd: &str) -> Kind {
    if tok.is_empty() {
        return Kind::None;
    }

    if tok.starts_with("http://") || tok.starts_with("https://") {
        return Kind::Url;
    }

    // Path with `:line[:col]` suffix → upgrade Path → PathWithLine when the
    // base classifies as a path.
    if let Some((path, line, col)) = parse_path_with_line(tok) {
        if classify_plain(&path, cwd) == Kind::Path {
            return Kind::PathWithLine { path, line, col };
        }
    }

    classify_plain(tok, cwd)
}

/// Classify without considering the `:line[:col]` suffix. Kept separate so
/// `classify` can re-use it for both the suffixed and bare-path paths.
fn classify_plain(tok: &str, cwd: &str) -> Kind {
    if tok.starts_with('/') {
        return Kind::Path;
    }

    let has_slash = tok.contains('/');
    let has_ext = tok
        .rfind('.')
        .map(|dot| dot > 0 && dot + 1 < tok.len())
        .unwrap_or(false);

    if has_slash || has_ext {
        return Kind::Path;
    }

    if !cwd.is_empty() && file_exists_relative(cwd, tok) {
        return Kind::Path;
    }

    Kind::None
}

fn file_exists_relative(dir: &str, name: &str) -> bool {
    Path::new(dir).join(name).exists()
}

// ---------------------------------------------------------------------------
// Tests  (13 Zig tests → 13 Rust tests)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_at_col_basic_extraction() {
        assert_eq!(token_at_col("hello world", 0), "hello");
        assert_eq!(token_at_col("hello world", 3), "hello");
        assert_eq!(token_at_col("hello world", 4), "hello");
        assert_eq!(token_at_col("hello world", 6), "world");
        assert_eq!(token_at_col("hello world", 10), "world");
    }

    #[test]
    fn token_at_col_whitespace_at_col_returns_empty() {
        assert_eq!(token_at_col("hello world", 5), "");
    }

    #[test]
    fn token_at_col_col_out_of_range_returns_empty() {
        assert_eq!(token_at_col("hi", 99), "");
    }

    #[test]
    fn token_at_col_single_token() {
        assert_eq!(token_at_col("src/main.zig:412", 5), "src/main.zig:412");
    }

    #[test]
    fn token_at_col_leading_whitespace() {
        assert_eq!(token_at_col("  foo  bar", 3), "foo");
        assert_eq!(token_at_col("  foo  bar", 7), "bar");
    }

    #[test]
    fn strip_line_suffix_strips_line() {
        assert_eq!(strip_line_suffix("src/main.zig:412"), "src/main.zig");
    }

    #[test]
    fn strip_line_suffix_strips_line_col() {
        assert_eq!(strip_line_suffix("src/main.zig:412:3"), "src/main.zig");
    }

    #[test]
    fn strip_line_suffix_no_suffix_unchanged() {
        assert_eq!(strip_line_suffix("src/main.zig"), "src/main.zig");
    }

    #[test]
    fn strip_line_suffix_non_digit_suffix_unchanged() {
        assert_eq!(strip_line_suffix("foo:bar"), "foo:bar");
    }

    #[test]
    fn strip_line_suffix_trailing_colon_unchanged() {
        assert_eq!(strip_line_suffix("foo:"), "foo:");
    }

    #[test]
    fn classify_urls() {
        assert_eq!(classify("http://example.com", ""), Kind::Url);
        assert_eq!(classify("https://example.com/path", ""), Kind::Url);
    }

    #[test]
    fn classify_absolute_path() {
        assert_eq!(classify("/usr/local/bin", ""), Kind::Path);
    }

    #[test]
    fn classify_relative_path_with_slash() {
        assert_eq!(classify("src/main.zig", ""), Kind::Path);
    }

    #[test]
    fn classify_relative_path_with_extension() {
        assert_eq!(classify("main.zig", ""), Kind::Path);
    }

    #[test]
    fn classify_bare_word_no_heuristics() {
        assert_eq!(classify("hello", ""), Kind::None);
    }

    #[test]
    fn classify_empty_token() {
        assert_eq!(classify("", ""), Kind::None);
    }

    #[test]
    fn classify_bare_word_file_exists_relative_to_cwd() {
        // Create a temp file without any slash or extension so heuristics
        // won't classify it as a path, but file_exists_relative will.
        let dir = std::env::temp_dir().join("anvil_interact_test");
        let _ = std::fs::create_dir_all(&dir);
        let fname = "Makefile"; // no slash, no extension
        let fpath = dir.join(fname);
        std::fs::write(&fpath, b"").expect("write tmp file");
        let cwd = dir.to_string_lossy().into_owned();
        assert_eq!(classify(fname, &cwd), Kind::Path);
        let _ = std::fs::remove_file(&fpath);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn classify_bare_word_file_not_found_returns_none() {
        // A bare word that doesn't exist on disk with a valid cwd → Kind::None.
        assert_eq!(classify("nonexistent_bare_word_xyz", "/tmp"), Kind::None);
    }

    #[test]
    fn parse_path_with_line_recognizes_line_only() {
        let (p, l, c) = parse_path_with_line("foo.rs:42").unwrap();
        assert_eq!(p, "foo.rs");
        assert_eq!(l, 42);
        assert_eq!(c, None);
    }

    #[test]
    fn parse_path_with_line_recognizes_line_and_col() {
        let (p, l, c) = parse_path_with_line("foo.rs:42:7").unwrap();
        assert_eq!(p, "foo.rs");
        assert_eq!(l, 42);
        assert_eq!(c, Some(7));
    }

    #[test]
    fn parse_path_with_line_handles_absolute_path() {
        let (p, l, c) = parse_path_with_line("/abs/path.rs:10").unwrap();
        assert_eq!(p, "/abs/path.rs");
        assert_eq!(l, 10);
        assert_eq!(c, None);
    }

    #[test]
    fn parse_path_with_line_no_suffix_returns_none() {
        assert!(parse_path_with_line("foo.rs").is_none());
    }

    #[test]
    fn parse_path_with_line_zero_line_rejected() {
        // Editors are 1-based — 0 is meaningless.
        assert!(parse_path_with_line("foo.rs:0").is_none());
    }

    #[test]
    fn classify_path_with_line_returns_pathwithline() {
        match classify("src/main.rs:42:7", "") {
            Kind::PathWithLine { path, line, col } => {
                assert_eq!(path, "src/main.rs");
                assert_eq!(line, 42);
                assert_eq!(col, Some(7));
            }
            other => panic!("expected PathWithLine, got {other:?}"),
        }
    }
}
