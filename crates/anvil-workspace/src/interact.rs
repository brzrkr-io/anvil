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
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Url,
    Path,
    None,
}

/// Classify `tok` (already suffix-stripped) as a URL, a file path, or neither.
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
}
