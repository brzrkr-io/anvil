#[cfg(test)]
use std::path::PathBuf;

#[cfg(test)]
use anvil_render::{ExplorerHit, LeftDockSnapshot};

use crate::{EXPLORER_SCROLL_ROWS_PER_WHEEL, LEFT_DOCK_DEFAULT_PT};

/// callers can fall back to "(not implemented)" rather than panicking.
#[cfg(target_os = "macos")]
pub(crate) fn current_rss_bytes() -> u64 {
    use std::mem;
    let pid = unsafe { libc::getpid() };
    let mut info: libc::proc_taskinfo = unsafe { mem::zeroed() };
    let size = mem::size_of::<libc::proc_taskinfo>() as libc::c_int;
    let ret = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDTASKINFO,
            0,
            &mut info as *mut _ as *mut libc::c_void,
            size,
        )
    };
    if ret == size {
        info.pti_resident_size
    } else {
        0
    }
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn current_rss_bytes() -> u64 {
    0
}

/// Format a byte count as a human-readable string: "12.4 KB", "3.2 MB", etc.
pub(crate) fn humanize_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}

/// Format a UNIX timestamp (seconds) as a relative time string.
/// Uses the current UNIX time from `std::time::SystemTime`.
pub(crate) fn relative_time(mtime_secs: u64) -> String {
    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if mtime_secs > now_secs {
        return "just now".to_string();
    }
    let delta = now_secs - mtime_secs;
    if delta < 60 {
        "just now".to_string()
    } else if delta < 3600 {
        let m = delta / 60;
        format!("{m} minute{} ago", if m == 1 { "" } else { "s" })
    } else if delta < 86400 {
        let h = delta / 3600;
        format!("{h} hour{} ago", if h == 1 { "" } else { "s" })
    } else {
        let d = delta / 86400;
        format!("{d} day{} ago", if d == 1 { "" } else { "s" })
    }
}

// ── X6: URL detection helper ─────────────────────────────────────────────────

/// Return the URL that contains grapheme column `col` on `line_str`, or `None`.
///
/// A URL is any token starting with `http://` or `https://` and extending to
/// the first whitespace or `"` or `'` or `)` or `>` character.
pub(crate) fn url_at_col(line_str: &str, col: usize) -> Option<String> {
    // Find all URL spans in the line.
    let mut spans: Vec<(usize, usize, String)> = Vec::new(); // (start_col, end_col, url)
    let line = line_str.trim_end_matches('\n').trim_end_matches('\r');
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        // Look for http:// or https://
        let rest: String = chars[i..].iter().collect();
        if rest.starts_with("http://") || rest.starts_with("https://") {
            let start = i;
            let end = chars[i..]
                .iter()
                .position(|&c| c.is_whitespace() || matches!(c, '"' | '\'' | ')' | '>' | '<'))
                .map(|off| i + off)
                .unwrap_or(chars.len());
            let url: String = chars[start..end].iter().collect();
            spans.push((start, end, url));
            i = end;
        } else {
            i += 1;
        }
    }
    spans
        .into_iter()
        .find(|(start, end, _)| col >= *start && col < *end)
        .map(|(_, _, url)| url)
}

// ── App helpers ───────────────────────────────────────────────────────────────

#[cfg(test)]
pub(crate) fn explorer_path_for_hit(
    snapshot: &LeftDockSnapshot,
    hit: ExplorerHit,
) -> Option<PathBuf> {
    match hit {
        ExplorerHit::Header => Some(PathBuf::from(&snapshot.root)),
        ExplorerHit::Row(idx) => snapshot
            .entries
            .get(idx)
            .map(|entry| PathBuf::from(&snapshot.root).join(&entry.name)),
    }
}

pub(crate) fn editor_gutter_width_for_buffer(buffer: &anvil_editor::Buffer, cell_w: f64) -> f64 {
    anvil_render::editor_gutter_width(buffer.line_count(), buffer.git_gutter.is_some(), cell_w)
}

pub(crate) fn next_explorer_scroll_offset(
    current: usize,
    dy: f64,
    row_count: usize,
    visible_rows: usize,
) -> usize {
    let max_offset = row_count.saturating_sub(visible_rows.max(1));
    if row_count == 0 || dy == 0.0 {
        return current.min(max_offset);
    }
    if dy > 0.0 {
        current
            .saturating_add(EXPLORER_SCROLL_ROWS_PER_WHEEL)
            .min(max_offset)
    } else {
        current.saturating_sub(EXPLORER_SCROLL_ROWS_PER_WHEEL)
    }
}

pub(crate) fn toggle_left_dock_instant(visible: bool) -> (bool, f64, f64) {
    (!visible, LEFT_DOCK_DEFAULT_PT, LEFT_DOCK_DEFAULT_PT)
}

pub(crate) fn should_animate_cursor_blink(
    focused_is_terminal: bool,
    app_focused: bool,
    pane_blink: Option<bool>,
    app_blink: bool,
) -> bool {
    focused_is_terminal && app_focused && pane_blink.unwrap_or(app_blink)
}
