//! Git blame + gutter result polling, split out of the `App` god object.
//!
//! These methods consume results from the blame/gutter worker threads (spawned
//! in `services`) and drive the hover-dwell blame tooltip. They are inherent
//! `App` methods living in a child module so they can still touch `App`'s
//! private fields directly.

use std::path::PathBuf;
use std::time::Instant;

use anvil_editor::BufferId;
use anvil_workspace::layout::PaneId;

use crate::App;
use crate::BlamePopup;
use crate::services::{BlameRequest, GutterRequest, GutterResult, blame_popup_text};

impl App {
    pub(crate) fn poll_gutter_results(&mut self) {
        while let Ok(GutterResult { buffer_id, gutter }) = self.gutter_rx.try_recv() {
            let mut found = false;
            for tab in self.tabs.tabs.iter_mut() {
                if let Some(buf) = tab.editor_panes.get_buffer_mut(buffer_id) {
                    buf.git_gutter = Some(gutter);
                    found = true;
                    break;
                }
            }
            if found {
                self.dirty = true;
            }
        }
    }

    /// Send a gutter recompute request for `buffer_id` at `path` with the
    /// current text snapshot.  Called after a file open or save.
    pub(crate) fn request_gutter_recompute(
        &self,
        buffer_id: BufferId,
        path: PathBuf,
        text: String,
    ) {
        let _ = self.gutter_tx.try_send(GutterRequest {
            buffer_id,
            path,
            text,
        });
    }

    // ── T2: blame hover logic ─────────────────────────────────────────────────

    /// Track cursor-line dwell.  Called each tick.  After 800ms on the same
    /// line in a native editor, fires a blame request (if not already cached).
    pub(crate) fn tick_blame_hover(&mut self) {
        const BLAME_DWELL: std::time::Duration = std::time::Duration::from_millis(800);
        const BLAME_TTL: std::time::Duration = std::time::Duration::from_secs(60);

        let now = Instant::now();

        // Resolve current (pane, line) for the focused native editor.
        let current_target: Option<(PaneId, usize, PathBuf)> = (|| {
            let tab = self.tabs.current()?;
            let pane_id = tab.focused_id();
            let ep = tab.editor_panes.get_pane(pane_id)?;
            let buf = tab.editor_panes.get_buffer(ep.buffer_id)?;
            let path = buf.tracked_path()?.to_path_buf();
            let line = ep.primary_cursor().pos.line;
            Some((pane_id, line, path))
        })();

        let Some((pane_id, cur_line, path)) = current_target else {
            // Not in a native editor — clear blame state.
            self.blame_hover = None;
            self.blame_popup = None;
            return;
        };

        // Update or reset the dwell tracker.
        match self.blame_hover {
            Some((pid, line, _)) if pid == pane_id && line == cur_line => {
                // Same line — check if dwell threshold passed.
                let dwell_since = self.blame_hover.unwrap().2;
                if now.duration_since(dwell_since) >= BLAME_DWELL {
                    // Check cache first.
                    let cache_key = (path.clone(), cur_line);
                    let cached = self.blame_cache.get(&cache_key).and_then(|(entry, ts)| {
                        if now.duration_since(*ts) < BLAME_TTL {
                            Some(entry)
                        } else {
                            None
                        }
                    });
                    if let Some(entry) = cached {
                        // Cache hit — show popup from cache.
                        if let Some(text) = blame_popup_text(entry) {
                            if self.blame_popup.as_ref().map(|p| p.anchor_line) != Some(cur_line) {
                                if let Some(pos) = self.hover_mouse_pos {
                                    self.blame_popup = Some(BlamePopup {
                                        text,
                                        anchor_line: cur_line,
                                        anchor_x: pos.0,
                                        anchor_y: pos.1,
                                    });
                                    self.dirty = true;
                                }
                            }
                        } else if self.blame_popup.take().is_some() {
                            self.dirty = true;
                        }
                    } else if self.blame_popup.is_none() {
                        // No cache hit and no popup yet — fire request (once).
                        let _ = self.blame_tx.try_send(BlameRequest {
                            path,
                            line: cur_line,
                        });
                        // Advance blame_hover time so we don't spam requests.
                        self.blame_hover = Some((pane_id, cur_line, now));
                    }
                }
            }
            _ => {
                // Line changed — reset dwell, clear popup.
                self.blame_hover = Some((pane_id, cur_line, now));
                self.blame_popup = None;
            }
        }
    }

    /// Poll for a blame result, cache it, and update `blame_popup` if the cursor
    /// is still on the same line.
    pub(crate) fn poll_blame_result(&mut self) {
        while let Ok(result) = self.blame_rx.try_recv() {
            let now = Instant::now();
            let cache_key = (result.path.clone(), result.line);
            self.blame_cache.insert(cache_key, (result.entry, now));
            // Update popup if the cursor is still on this line.
            let current_line: Option<(usize, (f64, f64))> = (|| {
                let tab = self.tabs.current()?;
                let pane_id = tab.focused_id();
                let ep = tab.editor_panes.get_pane(pane_id)?;
                let buf = tab.editor_panes.get_buffer(ep.buffer_id)?;
                let path = buf.tracked_path()?;
                if path != result.path {
                    return None;
                }
                let line = ep.primary_cursor().pos.line;
                let pos = self.hover_mouse_pos?;
                Some((line, pos))
            })();
            if let Some((cur_line, pos)) = current_line {
                if cur_line == result.line {
                    let entry_opt = self.blame_cache.get(&(result.path, result.line));
                    if let Some((entry, _)) = entry_opt {
                        if let Some(text) = blame_popup_text(entry) {
                            self.blame_popup = Some(BlamePopup {
                                text,
                                anchor_line: cur_line,
                                anchor_x: pos.0,
                                anchor_y: pos.1,
                            });
                            self.dirty = true;
                        } else if self.blame_popup.take().is_some() {
                            self.dirty = true;
                        }
                    }
                }
            }
        }
    }
}
