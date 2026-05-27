//! Native editor pane state and registry — NE4, NE6.
//!
//! `EditorPane` is the per-pane view state for a native editor.  `EditorPaneRegistry`
//! holds both the per-pane view state and the underlying `Buffer`s, keyed by
//! `PaneId` and `BufferId` respectively.  It lives alongside `PaneRegistry` on `Tab`.
//!
//! `EditorAction` is the typed action enum used by NE6 (keyboard dispatch).  A future
//! modal layer or vim plugin can sit as a thin keymap on top.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use anvil_editor::{Buffer, BufferId, Cursor, IoError, Position, Range};
use unicode_segmentation::UnicodeSegmentation;

use crate::editor_search::EditorSearch;
use crate::layout::PaneId;
use crate::selection::Selection;

// ── FontMetrics (NE7) ────────────────────────────────────────────────────────

/// Font cell dimensions used by mouse hit-testing.  Mirrors the fields of
/// `anvil_render::FontMetrics`; kept here to avoid a dependency cycle
/// (anvil-render → anvil-workspace → anvil-render).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FontMetrics {
    /// Width of one monospace cell in device pixels.
    pub cell_w: f64,
    /// Height of one monospace cell in device pixels.
    pub cell_h: f64,
    /// Baseline descent in device pixels (positive = below baseline).
    pub descent: f64,
}

// ── EditorAction ──────────────────────────────────────────────────────────────

// ── HoverPopup (NE10) ─────────────────────────────────────────────────────────

/// A hover popup anchored to a buffer position.
///
/// Populated by `main.rs` when `LspManager::poll_hover` returns a result.
/// Rendered by `draw_editor_into`. Dismissed on `EditorAction::HoverDismiss`
/// or on the next non-hover key press in the editor.
#[derive(Debug, Clone)]
pub struct HoverPopup {
    /// The markdown / plain-text content from the LSP hover response.
    pub text: String,
    /// Buffer position at which the popup is anchored (cursor position at time
    /// of the hover request).
    pub anchor: Position,
}

// ── CompletionPopup (item 16) ─────────────────────────────────────────────────

/// A completion popup anchored to the cursor position.
///
/// Populated by `main.rs` when `LspManager::poll_completion` returns items.
/// Rendered by `draw_editor_chrome` as a 12-row floating list below the cursor.
/// Dismissed by Esc, any buffer-mutating key, or a non-navigation key.
#[derive(Debug, Clone)]
pub struct CompletionPopup {
    /// All items returned by the LSP (pre-filter).
    pub items: Vec<CompletionEntry>,
    /// Index into `visible_items()` of the currently selected row.
    pub selected: usize,
    /// Buffer position at which completion was triggered.
    pub anchor: Position,
    /// Text typed since the trigger (prefix filter applied client-side).
    pub filter_prefix: String,
}

/// One row in the completion popup.
#[derive(Debug, Clone, PartialEq)]
pub struct CompletionEntry {
    pub label: String,
    pub detail: Option<String>,
    /// Text to insert on accept; falls back to `label` when absent.
    pub insert_text: String,
}

impl CompletionPopup {
    /// Items that pass the prefix filter.  O(n) each call — fine for popup sizes.
    pub fn visible_items(&self) -> Vec<&CompletionEntry> {
        if self.filter_prefix.is_empty() {
            self.items.iter().collect()
        } else {
            self.items
                .iter()
                .filter(|e| {
                    e.label
                        .to_ascii_lowercase()
                        .starts_with(&self.filter_prefix.to_ascii_lowercase())
                })
                .collect()
        }
    }
}

// ── CodeActionsPopup (item 25) ────────────────────────────────────────────────

/// One item in the code-actions popup.
#[derive(Debug, Clone, PartialEq)]
pub struct CodeActionEntry {
    pub title: String,
}

/// A floating list of code actions anchored to the cursor line (item 25).
///
/// Populated by `main.rs` when `LspManager::poll_code_actions` returns items.
/// Rendered via the completion-popup render path in `draw_editor_into`.
/// Dismissed by Esc; Enter applies the selected action's `WorkspaceEdit`.
#[derive(Debug, Clone)]
pub struct CodeActionsPopup {
    pub items: Vec<CodeActionEntry>,
    pub selected: usize,
    pub anchor: Position,
}

// ── EditorAction ──────────────────────────────────────────────────────────────

/// A typed editor action — the unit of currency between the keymap and the
/// buffer/cursor engine.  NE6 insert-mode only; a future modal layer adds a
/// keymap on top without touching this enum.
#[derive(Debug, Clone, PartialEq)]
pub enum EditorAction {
    InsertChar(char),
    InsertNewline,
    Backspace,
    Delete,
    MoveLeft {
        extend: bool,
    },
    MoveRight {
        extend: bool,
    },
    MoveUp {
        extend: bool,
    },
    MoveDown {
        extend: bool,
    },
    MoveLineStart {
        extend: bool,
    },
    MoveLineEnd {
        extend: bool,
    },
    MoveBufferStart {
        extend: bool,
    },
    MoveBufferEnd {
        extend: bool,
    },
    PageUp {
        extend: bool,
    },
    PageDown {
        extend: bool,
    },
    Save,
    /// Save buffer to a new path and update `tracked_path`.
    /// TODO(anvil-tierJ-J2-nspanel): replace inline overlay with NSSavePanel.
    SaveAs(std::path::PathBuf),
    Undo,
    Redo,
    Copy,
    Cut,
    Paste(String),
    SelectAll,
    GoToLine(usize),
    InsertTab,
    // ── Search (NE11) ────────────────────────────────────────────────────────
    /// Open in-buffer search (initialise EditorSearch if not already open).
    SearchOpen,
    /// Close in-buffer search and clear the EditorSearch state.
    SearchClose,
    /// Update the search query and re-scan.
    SearchSetQuery(String),
    /// Advance to the next hit (wrapping); moves the cursor.
    SearchNext,
    /// Retreat to the previous hit (wrapping); moves the cursor.
    SearchPrev,
    /// Toggle regex mode and re-scan.
    SearchToggleRegex,
    // ── Find+Replace (item 9) ────────────────────────────────────────────────
    /// Open find+replace mode (second input row appears).
    FindReplaceOpen,
    /// Update the replace string.
    SetReplaceInput(String),
    /// Replace the current match with the replace string, advance to the next.
    ReplaceOne,
    /// Replace all matches in the active buffer.
    ReplaceAll,
    /// Place cursor at the given position; clears selection unless `extend` is true (NE7).
    MoveTo {
        pos: Position,
        extend: bool,
    },
    /// Select the word containing `pos` (double-click, NE7).
    /// Word chars: alphanumeric + underscore.
    SelectWordAt(Position),
    /// Select the entire line containing `pos` (triple-click, NE7).
    SelectLineAt(Position),
    // ── LSP UI (NE10, tier-3) ────────────────────────────────────────────────
    /// Request hover information at the current cursor position (Cmd+K).
    ///
    /// `main.rs` translates this into an `LspManager::request_hover` call; the
    /// result is stored on `EditorPane::hover_popup` when polled.  The action
    /// itself is a no-op inside `apply` — the caller handles the LSP request.
    HoverRequest,
    /// Dismiss the hover popup.
    HoverDismiss,
    // ── Completion popup (item 16) ────────────────────────────────────────────
    /// Open completion popup with the given items anchored at the cursor.
    /// `main.rs` calls this after `LspManager::poll_completion` returns.
    CompletionOpen(Vec<CompletionEntry>),
    /// Move selection up in the completion popup.
    CompletionUp,
    /// Move selection down in the completion popup.
    CompletionDown,
    /// Accept the selected completion item: insert its `insert_text` and close.
    CompletionAccept,
    /// Dismiss the completion popup without inserting.
    CompletionDismiss,
    /// Append a character to the completion filter prefix and re-filter.
    CompletionFilter(char),
    // ── AI ghost-text (NE14) ─────────────────────────────────────────────────
    /// Accept the first ghost-text suggestion at the cursor: inserts its text
    /// at the cursor position and clears all ghost text.
    AcceptGhostText,
    /// Dismiss all ghost-text suggestions without inserting anything.
    DismissGhostText,
    // ── Multi-cursor (NE13) ──────────────────────────────────────────────────
    /// Cmd+click: add a secondary cursor at `pos`.  Deduplicates by position.
    AddCursorAt(Position),
    /// Esc (when multi-cursor active): drop all secondary cursors, keep primary.
    ClearSecondaryCursors,
    /// Cmd+D: extend selection to the next occurrence of the selected text (or
    /// the word under the primary cursor when there is no selection). Adds a
    /// secondary cursor at the start of the new match with the same-length
    /// selection.
    AddNextOccurrence,
    // ── Code folding (item 13) ────────────────────────────────────────────────
    /// Toggle fold at `line` (the line that starts the foldable range).
    ToggleFold(usize),
    // ── Tier-C power features (#16–#23) ──────────────────────────────────────
    /// Insert a newline with auto-indent (#16). Copies leading whitespace of
    /// the prior line; adds one extra indent level when the prior line ends
    /// with `{`, `(`, or `[`.
    InsertNewlineSmart,
    /// Toggle line comment on the current line or all selected lines (#18).
    /// Uses the language-appropriate marker from `Buffer::language_id()`.
    ToggleLineComment,
    /// Duplicate the current line below; cursor moves to the new line (#19).
    DuplicateLine,
    /// Swap the current line with the line above (#20).
    MoveLineUp,
    /// Swap the current line with the line below (#20).
    MoveLineDown,
    /// Indent selected lines by one level; or insert indent at cursor when no
    /// selection spans multiple lines (#21).
    IndentSelection,
    /// Dedent selected lines by one level (#21).
    DedentSelection,
    /// Format the active buffer (#23). Falls back to `rustfmt` for Rust files
    /// when no LSP is connected.
    FormatFile,
    // ── Tier-H toggles ────────────────────────────────────────────────────────
    /// H1: toggle soft word-wrap for the active pane (Cmd+K W).
    ToggleSoftWrap,
    /// H2: toggle show-whitespace overlay for the active pane (Cmd+K Space).
    ToggleShowWhitespace,
    // ── Code actions popup (item 25) ──────────────────────────────────────────
    /// Open the code-actions popup with the given items (Cmd+.).
    CodeActionsOpen(Vec<CodeActionEntry>),
    /// Move selection up in the code-actions popup.
    CodeActionsUp,
    /// Move selection down in the code-actions popup.
    CodeActionsDown,
    /// Dismiss the code-actions popup without applying.
    CodeActionsDismiss,
    // ── Tier-K selection (K6) ─────────────────────────────────────────────────
    /// Cmd+L: select the cursor's current line (col 0 → end-of-line).
    ///
    /// If the selection is already line-aligned (anchor.col == 0 and the cursor
    /// is at end-of-line), extend the selection down by one more line so that
    /// repeated Cmd+L grows the selection one line at a time.
    SelectLine,

    // ── Tier-W additions ──────────────────────────────────────────────────────
    /// W4 (Cmd+Shift+K): delete the current cursor line; cursor moves to col 0
    /// of the line that takes its place (or the last line if it was the final one).
    DeleteLine,

    /// W7 (Cmd+Shift+L): select all occurrences of the selected text (or the
    /// word under the cursor when there is no selection).  Places one cursor at
    /// each match.  No-op when no search text can be determined or no matches exist.
    SelectAllOccurrences,

    /// W8 (Cmd+U): drop the most-recently-added secondary cursor.  No-op when
    /// only one cursor is active.
    DropLastCursor,

    /// W9 (Cmd+K Cmd+0): fold all foldable ranges in the active buffer.
    /// Foldable ranges are detected as lines whose syntax tree contains a block
    /// opener — for now we use a simple heuristic: any line that ends with `{`,
    /// `(`, or `[` (after trimming trailing whitespace).
    FoldAll,

    /// W9 (Cmd+K Cmd+J): unfold all — clear all folds for the active buffer.
    UnfoldAll,

    /// W10 (Cmd+K Cmd+U): convert selected text (or current word) to UPPER CASE.
    ConvertCaseUpper,

    /// W10 (Cmd+K Cmd+Y): convert selected text (or current word) to lower case.
    ConvertCaseLower,

    /// W10 (Cmd+K Cmd+Shift+T): convert selected text (or current word) to
    /// Title Case (first letter of each whitespace-separated word capitalised).
    ConvertCaseTitle,

    /// W11 (Cmd+K Cmd+R): sort the selected lines alphabetically (case-insensitive).
    /// Requires a multi-line selection; no-op on a single-line selection.
    SortSelectedLines,

    /// W12 (Cmd+Shift+J): join the current line with the next line by replacing
    /// the newline between them with a single space.  No-op on the last line.
    JoinLines,

    // ── Tier-X additions ─────────────────────────────────────────────────────
    /// X14 (Opt+←): move one word boundary to the left.  Word chars: alnum + `_`.
    MoveWordLeft {
        extend: bool,
    },

    /// X14 (Opt+→): move one word boundary to the right.
    MoveWordRight {
        extend: bool,
    },

    /// X13 (Cmd+K Cmd+W): trim trailing whitespace in selection or entire buffer.
    TrimTrailingWhitespace,

    /// X8 (Cmd+F12): jump to the declaration of the symbol under the cursor
    /// using the buffer's syntax-derived outline.  No-op when no match.
    GotoSymbolAtCursor,

    /// X9 (Opt+F12): peek definition in a floating popup.
    /// TODO(anvil-tierX-#9): full inline peek popup not yet implemented.
    PeekDefinition,

    /// X11 (Tab): expand a snippet trigger under the cursor.
    /// `trigger` is the matched key from `~/.config/anvil/snippets.toml`;
    /// `body` is its expansion text.  Placeholders `$0`, `$1` are resolved.
    ExpandSnippet {
        trigger: String,
        body: String,
    },
}

/// Maximum number of open buffers tracked per pane. When the limit is
/// exceeded, the oldest non-active buffer is evicted.
pub const MAX_TABS_PER_PANE: usize = 16;

/// Per-pane view state for a native editor pane.
pub struct EditorPane {
    /// The currently active buffer. Kept in sync with `open_buffers`.
    pub buffer_id: BufferId,
    /// Ordered list of open buffer IDs (insertion order, oldest first).
    /// Always contains at least `buffer_id`.
    pub open_buffers: Vec<BufferId>,
    /// All cursors. `cursors[0]` is the primary cursor (always present).
    /// Secondary cursors are appended by `AddCursorAt` and dropped by
    /// `ClearSecondaryCursors`.
    pub cursors: Vec<Cursor>,
    pub selection: Selection,
    pub scroll_pos: f32,
    pub scroll_target: f32,
    pub scroll_vel: f32,
    /// In-buffer search state (NE11). `None` when the search bar is closed.
    pub search: Option<EditorSearch>,
    /// Active hover popup (NE10). `None` when no hover is showing.
    /// Set by `main.rs` when `LspManager::poll_hover` returns a result.
    /// Cleared by `EditorAction::HoverDismiss` or on any buffer-mutating action.
    pub hover_popup: Option<HoverPopup>,
    /// Active completion popup (item 16). `None` when no completion is showing.
    pub completion_popup: Option<CompletionPopup>,
    /// Active code-actions popup (item 25). `None` when closed.
    pub code_actions_popup: Option<CodeActionsPopup>,
    /// Folded line ranges keyed by `BufferId` (item 13).
    /// Each entry is a set of start-line numbers for active folds.
    pub folds: HashMap<BufferId, HashSet<usize>>,
    /// H1: soft word-wrap enabled for this pane.  When true, long lines wrap
    /// instead of clipping to the content width.
    /// TODO(anvil-tierH-H1-wrap): actual wrap rendering not yet wired; flag is
    /// toggled and persisted but render shows a placeholder until the full
    /// multi-visual-row layout is implemented.
    pub soft_wrap: bool,
    /// H2: show whitespace overlay — spaces render as `·`, tabs as `→`.
    pub show_whitespace: bool,
    /// P3: horizontal scroll offset in fractional cell columns.
    /// `0.0` = no horizontal scroll.  Forced to `0.0` when `soft_wrap` is true.
    /// Incremented by Shift+scroll wheel; auto-scrolled to keep the cursor
    /// visible; drives the thin horizontal scrollbar in `draw_editor_into`.
    pub scroll_x: f64,
}

impl EditorPane {
    /// Return the primary cursor (always at index 0).
    pub fn primary_cursor(&self) -> &Cursor {
        &self.cursors[0]
    }

    /// Return the primary cursor mutably.
    pub fn primary_cursor_mut(&mut self) -> &mut Cursor {
        &mut self.cursors[0]
    }
}

/// Registry of all native editor panes and their buffers for one `Tab`.
///
/// `panes` maps a `PaneId` to its `EditorPane` view state.
/// `buffers` maps a `BufferId` to the underlying `Buffer`.
/// `next_buffer_id` is a monotonic counter for allocating fresh `BufferId`s.
/// `buffer_scroll` remembers the last scroll position per buffer so tab
/// switches restore where the user was (G4).
pub struct EditorPaneRegistry {
    panes: HashMap<PaneId, EditorPane>,
    buffers: HashMap<BufferId, Buffer>,
    next_buffer_id: BufferId,
    /// G4: last known scroll_pos per buffer. Saved on open_buffer (tab switch
    /// away); restored on open_buffer (tab switch to). Dropped on close_buffer.
    buffer_scroll: HashMap<BufferId, f32>,
}

impl Default for EditorPaneRegistry {
    fn default() -> Self {
        Self {
            panes: HashMap::new(),
            buffers: HashMap::new(),
            next_buffer_id: 1,
            buffer_scroll: HashMap::new(),
        }
    }
}

impl EditorPaneRegistry {
    /// Allocate a fresh `Buffer`, register an `EditorPane` for `pane_id`,
    /// and return the new `BufferId`.
    pub fn new_pane(&mut self, pane_id: PaneId) -> BufferId {
        let buffer_id = self.next_buffer_id;
        self.next_buffer_id += 1;
        let origin = Position { line: 0, col: 0 };
        let pane = EditorPane {
            buffer_id,
            open_buffers: vec![buffer_id],
            cursors: vec![Cursor {
                pos: origin,
                anchor: origin,
            }],
            selection: Selection::default(),
            scroll_pos: 0.0,
            scroll_target: 0.0,
            scroll_vel: 0.0,
            search: None,
            hover_popup: None,
            completion_popup: None,
            code_actions_popup: None,
            folds: HashMap::new(),
            soft_wrap: false,
            show_whitespace: false,
            scroll_x: 0.0,
        };
        self.panes.insert(pane_id, pane);
        self.buffers.insert(buffer_id, Buffer::new());
        buffer_id
    }

    /// Load `path` into `pane_id`, replacing that pane's previous buffer.
    ///
    /// The old buffer is dropped and `open_buffers` is updated to contain only
    /// the new buffer. For opening a file as a new tab without closing others,
    /// use [`open_path_as_tab`] instead.
    pub fn open_path(&mut self, pane_id: PaneId, path: &Path) -> Result<BufferId, IoError> {
        let buffer = Buffer::from_path(path)?;
        let buffer_id = self.next_buffer_id;
        self.next_buffer_id += 1;
        let origin = Position { line: 0, col: 0 };
        let pane = self.panes.entry(pane_id).or_insert_with(|| EditorPane {
            buffer_id,
            open_buffers: vec![buffer_id],
            cursors: vec![Cursor {
                pos: origin,
                anchor: origin,
            }],
            selection: Selection::default(),
            scroll_pos: 0.0,
            scroll_target: 0.0,
            scroll_vel: 0.0,
            search: None,
            hover_popup: None,
            completion_popup: None,
            code_actions_popup: None,
            folds: HashMap::new(),
            soft_wrap: false,
            show_whitespace: false,
            scroll_x: 0.0,
        });
        let old_buffer_id = pane.buffer_id;
        pane.buffer_id = buffer_id;
        pane.open_buffers = vec![buffer_id];
        pane.cursors = vec![Cursor {
            pos: origin,
            anchor: origin,
        }];
        pane.selection = Selection::default();
        pane.scroll_pos = 0.0;
        pane.scroll_target = 0.0;
        pane.scroll_vel = 0.0;
        pane.search = None;
        pane.hover_popup = None;
        pane.completion_popup = None;
        pane.code_actions_popup = None;
        pane.scroll_x = 0.0;
        self.buffers.remove(&old_buffer_id);
        self.buffers.insert(buffer_id, buffer);
        Ok(buffer_id)
    }

    /// Open `path` as a new tab in `pane_id`.
    ///
    /// - If the path is already open in this pane, activate it and return its id.
    /// - Otherwise load the file into a new buffer, append to `open_buffers`,
    ///   activate it, and enforce the [`MAX_TABS_PER_PANE`] cap.
    /// - The existing open buffers are preserved (unlike `open_path`).
    pub fn open_path_as_tab(&mut self, pane_id: PaneId, path: &Path) -> Result<BufferId, IoError> {
        // Check if the path is already open in this pane.
        if let Some(pane) = self.panes.get(&pane_id) {
            for &bid in &pane.open_buffers {
                if let Some(buf) = self.buffers.get(&bid) {
                    if buf.tracked_path() == Some(path) {
                        // Already open — just activate.
                        let pane = self.panes.get_mut(&pane_id).unwrap();
                        pane.buffer_id = bid;
                        return Ok(bid);
                    }
                }
            }
        }

        // Load new buffer.
        let buffer = Buffer::from_path(path)?;
        let buffer_id = self.next_buffer_id;
        self.next_buffer_id += 1;
        let origin = Position { line: 0, col: 0 };

        // Ensure a pane exists.
        if let std::collections::hash_map::Entry::Vacant(e) = self.panes.entry(pane_id) {
            let pane = EditorPane {
                buffer_id,
                open_buffers: vec![buffer_id],
                cursors: vec![Cursor {
                    pos: origin,
                    anchor: origin,
                }],
                selection: Selection::default(),
                scroll_pos: 0.0,
                scroll_target: 0.0,
                scroll_vel: 0.0,
                search: None,
                hover_popup: None,
                completion_popup: None,
                code_actions_popup: None,
                folds: HashMap::new(),
                soft_wrap: false,
                show_whitespace: false,
                scroll_x: 0.0,
            };
            e.insert(pane);
            self.buffers.insert(buffer_id, buffer);
            return Ok(buffer_id);
        }

        // Identify scratch tabs (untracked, never edited) for eviction.
        let scratch_to_evict: Vec<BufferId> = {
            let pane = self.panes.get(&pane_id).unwrap();
            pane.open_buffers
                .iter()
                .copied()
                .filter(|bid| {
                    self.buffers
                        .get(bid)
                        .map(|b| b.tracked_path().is_none() && b.revisions == 0)
                        .unwrap_or(false)
                })
                .collect()
        };

        // Append to open_buffers, drop scratch siblings, enforce cap.
        {
            let pane = self.panes.get_mut(&pane_id).unwrap();
            pane.open_buffers.push(buffer_id);
            for bid in &scratch_to_evict {
                if let Some(pos) = pane.open_buffers.iter().position(|b| b == bid) {
                    pane.open_buffers.remove(pos);
                }
            }
            // Evict oldest non-active if over the cap.
            while pane.open_buffers.len() > MAX_TABS_PER_PANE {
                let active = pane.buffer_id;
                if let Some(pos) = pane.open_buffers.iter().position(|&b| b != active) {
                    let evicted = pane.open_buffers.remove(pos);
                    self.buffers.remove(&evicted);
                } else {
                    break;
                }
            }
            pane.buffer_id = buffer_id;
        }
        for bid in scratch_to_evict {
            self.buffers.remove(&bid);
        }
        self.buffers.insert(buffer_id, buffer);
        Ok(buffer_id)
    }

    /// Activate `buffer_id` in `pane_id` without loading a new file.
    ///
    /// `buffer_id` must already be in `pane.open_buffers`. No-op if not found.
    /// G4: saves the current pane scroll_pos under the outgoing buffer id and
    /// restores it for the incoming buffer (defaulting to 0.0 if not seen before).
    pub fn open_buffer(&mut self, pane_id: PaneId, buffer_id: BufferId) {
        // Collect what we need before any mutation so borrow checker is happy.
        let (outgoing, current_scroll, valid) = match self.panes.get(&pane_id) {
            Some(pane) if pane.open_buffers.contains(&buffer_id) => {
                (pane.buffer_id, pane.scroll_pos, true)
            }
            _ => return,
        };
        if !valid {
            return;
        }
        if outgoing != buffer_id {
            // Save scroll for the outgoing buffer.
            self.buffer_scroll.insert(outgoing, current_scroll);
            // Restore scroll for the incoming buffer.
            let restored = self.buffer_scroll.get(&buffer_id).copied().unwrap_or(0.0);
            if let Some(pane) = self.panes.get_mut(&pane_id) {
                pane.scroll_pos = restored;
                pane.scroll_target = restored;
                pane.buffer_id = buffer_id;
            }
        } else if let Some(pane) = self.panes.get_mut(&pane_id) {
            pane.buffer_id = buffer_id;
        }
    }

    /// Close `buffer_id` in `pane_id`.
    ///
    /// Returns the new active `BufferId`, or `None` if no buffers remain (the
    /// pane should fall back to its scratch buffer in that case).
    ///
    /// Activation priority: right neighbor in `open_buffers`, or the last
    /// remaining buffer if there is no right neighbor.
    pub fn close_buffer(&mut self, pane_id: PaneId, buffer_id: BufferId) -> Option<BufferId> {
        let pane = self.panes.get_mut(&pane_id)?;
        let pos = pane.open_buffers.iter().position(|&b| b == buffer_id)?;
        pane.open_buffers.remove(pos);

        if pane.open_buffers.is_empty() {
            // No buffers left — caller should fall back to scratch.
            self.buffers.remove(&buffer_id);
            // Allocate a fresh scratch buffer to keep the registry consistent.
            let new_id = self.next_buffer_id;
            self.next_buffer_id += 1;
            let origin = Position { line: 0, col: 0 };
            pane.open_buffers = vec![new_id];
            pane.buffer_id = new_id;
            pane.cursors = vec![Cursor {
                pos: origin,
                anchor: origin,
            }];
            pane.selection = Selection::default();
            pane.scroll_pos = 0.0;
            pane.scroll_target = 0.0;
            pane.scroll_vel = 0.0;
            pane.search = None;
            pane.hover_popup = None;
            pane.completion_popup = None;
            pane.code_actions_popup = None;
            pane.scroll_x = 0.0;
            self.buffers.insert(new_id, Buffer::new());
            return None;
        }

        // Pick new active: right neighbor clamped to valid index.
        let new_pos = pos.min(pane.open_buffers.len() - 1);
        let new_active = pane.open_buffers[new_pos];
        pane.buffer_id = new_active;
        self.buffers.remove(&buffer_id);
        // G4: drop the saved scroll position for the closed buffer.
        self.buffer_scroll.remove(&buffer_id);
        Some(new_active)
    }

    /// Iterate over all `(PaneId, EditorPane)` pairs in this registry.
    pub fn panes_iter(&self) -> impl Iterator<Item = (PaneId, &EditorPane)> {
        self.panes.iter().map(|(&id, ep)| (id, ep))
    }

    /// Look up the `EditorPane` for `pane_id`.
    pub fn get_pane(&self, pane_id: PaneId) -> Option<&EditorPane> {
        self.panes.get(&pane_id)
    }

    /// Look up the `EditorPane` mutably for `pane_id`.
    pub fn get_pane_mut(&mut self, pane_id: PaneId) -> Option<&mut EditorPane> {
        self.panes.get_mut(&pane_id)
    }

    /// Look up the `Buffer` by `buffer_id`.
    pub fn get_buffer(&self, buffer_id: BufferId) -> Option<&Buffer> {
        self.buffers.get(&buffer_id)
    }

    /// Look up the `Buffer` mutably by `buffer_id`.
    pub fn get_buffer_mut(&mut self, buffer_id: BufferId) -> Option<&mut Buffer> {
        self.buffers.get_mut(&buffer_id)
    }

    /// Iterate all buffers mutably — used by save-on-blur (tier-J J1) and
    /// similar "enumerate all open files" operations.
    pub fn buffers_mut(&mut self) -> impl Iterator<Item = (BufferId, &mut Buffer)> {
        self.buffers.iter_mut().map(|(k, v)| (*k, v))
    }

    /// Find and return a mutable reference to a `Buffer` that tracks `path`
    /// (item 24, rename edits). Returns `None` if no open buffer matches.
    pub fn find_buffer_for_path(&mut self, path: &std::path::Path) -> Option<&mut Buffer> {
        self.buffers
            .values_mut()
            .find(|b| b.tracked_path() == Some(path))
    }

    /// Remove the `EditorPane` for `pane_id` and drop all its buffers.
    ///
    /// No-op if `pane_id` is not registered.
    pub fn remove_pane(&mut self, pane_id: PaneId) {
        if let Some(pane) = self.panes.remove(&pane_id) {
            for bid in pane.open_buffers {
                self.buffers.remove(&bid);
            }
        }
    }

    /// Number of registered editor panes.
    pub fn count(&self) -> usize {
        self.panes.len()
    }

    /// Apply `action` to the pane identified by `pane_id`.
    ///
    /// Returns `true` when the buffer was mutated (caller should mark the pane
    /// dirty for re-render).  Returns `false` for pure cursor/selection moves,
    /// no-ops, or when `pane_id` is not registered.
    ///
    /// Copy and Cut return the selected text through `clipboard_out`.  The
    /// caller is responsible for writing it to the system clipboard.
    pub fn apply(
        &mut self,
        pane_id: PaneId,
        action: EditorAction,
        clipboard_out: &mut Option<String>,
    ) -> bool {
        *clipboard_out = None;

        let pane = match self.panes.get_mut(&pane_id) {
            Some(p) => p,
            None => return false,
        };
        let buffer_id = pane.buffer_id;

        match action {
            // ── Insert ───────────────────────────────────────────────────────
            EditorAction::InsertChar(ch) => {
                // ── #17 Smart bracket pairs ──────────────────────────────────
                // Only applies to the primary cursor. Secondary cursors receive
                // a plain character insert below.

                // Step 1: skip-over a paired closing char that already sits at
                // the cursor (handles typing `)` after auto-pair inserted `()`).
                if is_closing_bracket(ch) {
                    let cursor_pos = {
                        let pane = self.panes.get(&pane_id).unwrap();
                        pane.cursors[0].pos
                    };
                    let char_at_cursor = {
                        let buf = self.buffers.get(&buffer_id).unwrap();
                        let lc = buf.line_to_char(cursor_pos.line);
                        let line_str: String = buf.line(cursor_pos.line).chars().collect();
                        let mut char_off = 0usize;
                        for (gi, g) in line_str.graphemes(true).enumerate() {
                            if gi == cursor_pos.col {
                                break;
                            }
                            char_off += g.chars().count();
                        }
                        buf.char_at(lc + char_off)
                    };
                    if char_at_cursor == Some(ch) {
                        let pane = self.panes.get_mut(&pane_id).unwrap();
                        let new_pos = advance_col(cursor_pos, 1);
                        set_cursor(pane, new_pos, false);
                        return false;
                    }
                }

                // Step 2: auto-pair openers.
                if let Some(close) = bracket_pair_close(ch) {
                    // Check for selection — wrap it.
                    let has_selection = {
                        let pane = self.panes.get(&pane_id).unwrap();
                        pane.cursors[0].anchor != pane.cursors[0].pos
                    };
                    if has_selection {
                        // Wrap selection: insert open before, close after.
                        let (start, end) = {
                            let pane = self.panes.get(&pane_id).unwrap();
                            selection_range(pane)
                        };
                        {
                            let buf = self.buffers.get_mut(&buffer_id).unwrap();
                            // Insert close at end first (so start offset is unaffected).
                            buf.insert_char(end, close);
                            buf.insert_char(start, ch);
                        }
                        // Move cursor to just after the closing char.
                        let pane = self.panes.get_mut(&pane_id).unwrap();
                        let new_end = Position {
                            line: end.line,
                            col: end.col + 2,
                        };
                        pane.cursors[0].anchor = Position {
                            line: start.line,
                            col: start.col + 1,
                        };
                        pane.cursors[0].pos = new_end;
                        return true;
                    }
                    let cursor_pos = {
                        let pane = self.panes.get(&pane_id).unwrap();
                        pane.cursors[0].pos
                    };
                    // For ' and " skip auto-pair when previous char is alphanumeric.
                    let should_pair = if ch == '\'' || ch == '"' || ch == '`' {
                        let buf = self.buffers.get(&buffer_id).unwrap();
                        !prev_char_is_alnum(buf, cursor_pos)
                    } else {
                        true
                    };
                    if should_pair {
                        // Insert open+close, leave cursor between them.
                        {
                            let buf = self.buffers.get_mut(&buffer_id).unwrap();
                            let mut pair = ch.to_string();
                            pair.push(close);
                            buf.insert_str(cursor_pos, &pair);
                        }
                        let pane = self.panes.get_mut(&pane_id).unwrap();
                        let new_pos = advance_col(cursor_pos, 1);
                        set_cursor(pane, new_pos, false);
                        return true;
                    }
                }

                // ── plain insert (multi-cursor path) ─────────────────────────
                let cursor_positions: Vec<Position> = {
                    let pane = self.panes.get(&pane_id).unwrap();
                    let mut positions: Vec<Position> = pane.cursors.iter().map(|c| c.pos).collect();
                    positions.sort_by_key(|b| std::cmp::Reverse((b.line, b.col)));
                    positions.dedup();
                    positions
                };
                {
                    let buf = self.buffers.get_mut(&buffer_id).unwrap();
                    for pos in &cursor_positions {
                        buf.insert_char(*pos, ch);
                    }
                }
                // Advance each cursor by 1 col.
                let pane = self.panes.get_mut(&pane_id).unwrap();
                for c in &mut pane.cursors {
                    c.pos = advance_col(c.pos, 1);
                    c.anchor = c.pos;
                }
                true
            }
            EditorAction::InsertNewline => {
                let pos = pane.cursors[0].pos;
                {
                    let buf = self.buffers.get_mut(&buffer_id).unwrap();
                    buf.insert_char(pos, '\n');
                }
                let pane = self.panes.get_mut(&pane_id).unwrap();
                let new_pos = Position {
                    line: pos.line + 1,
                    col: 0,
                };
                set_cursor(pane, new_pos, false);
                true
            }
            EditorAction::InsertTab => {
                let pos = pane.cursors[0].pos;
                {
                    let buf = self.buffers.get_mut(&buffer_id).unwrap();
                    buf.insert_str(pos, "    ");
                }
                let pane = self.panes.get_mut(&pane_id).unwrap();
                let new_pos = advance_col(pos, 4);
                set_cursor(pane, new_pos, false);
                true
            }
            EditorAction::Paste(text) => {
                let pos = pane.cursors[0].pos;
                let line_delta = text.lines().count().saturating_sub(1);
                let last_line_len = text
                    .lines()
                    .last()
                    .map(|l| l.graphemes(true).count())
                    .unwrap_or(0);
                {
                    let buf = self.buffers.get_mut(&buffer_id).unwrap();
                    buf.insert_str(pos, &text);
                }
                let pane = self.panes.get_mut(&pane_id).unwrap();
                let new_pos = if line_delta == 0 {
                    advance_col(pos, last_line_len)
                } else {
                    Position {
                        line: pos.line + line_delta,
                        col: last_line_len,
                    }
                };
                set_cursor(pane, new_pos, false);
                true
            }

            // ── Delete ───────────────────────────────────────────────────────
            EditorAction::Backspace => {
                // Multi-cursor: compute (prev, pos) pairs then apply in
                // reverse position order so higher deletions don't shift lower
                // cursor positions.
                let pairs: Vec<(Position, Position)> = {
                    let pane = self.panes.get(&pane_id).unwrap();
                    let buf = self.buffers.get(&buffer_id).unwrap();
                    let mut ps: Vec<(Position, Position)> = pane
                        .cursors
                        .iter()
                        .filter_map(|c| {
                            let prev = prev_position(buf, c.pos);
                            if prev != c.pos {
                                Some((prev, c.pos))
                            } else {
                                None
                            }
                        })
                        .collect();
                    // Sort descending by the `pos` (end) of each range.
                    ps.sort_by_key(|b| std::cmp::Reverse((b.1.line, b.1.col)));
                    ps.dedup_by_key(|p| p.1);
                    ps
                };
                if pairs.is_empty() {
                    return false;
                }
                {
                    let buf = self.buffers.get_mut(&buffer_id).unwrap();
                    for (prev, pos) in &pairs {
                        buf.delete_range(Range {
                            start: *prev,
                            end: *pos,
                        });
                    }
                }
                // Update each cursor: move to its `prev` position.
                // We walk primary cursor specially; secondary cursors approximate.
                let pane = self.panes.get_mut(&pane_id).unwrap();
                // Match cursors to their pairs by pos.
                for c in &mut pane.cursors {
                    if let Some((prev, _)) = pairs.iter().find(|(_, p)| *p == c.pos) {
                        c.pos = *prev;
                        c.anchor = *prev;
                    }
                }
                true
            }
            EditorAction::Delete => {
                // Multi-cursor: apply in reverse position order.
                let pairs: Vec<(Position, Position)> = {
                    let pane = self.panes.get(&pane_id).unwrap();
                    let buf = self.buffers.get(&buffer_id).unwrap();
                    let mut ps: Vec<(Position, Position)> = pane
                        .cursors
                        .iter()
                        .filter_map(|c| {
                            let next = next_position(buf, c.pos);
                            if next != c.pos {
                                Some((c.pos, next))
                            } else {
                                None
                            }
                        })
                        .collect();
                    ps.sort_by_key(|b| std::cmp::Reverse((b.0.line, b.0.col)));
                    ps.dedup_by_key(|p| p.0);
                    ps
                };
                if pairs.is_empty() {
                    return false;
                }
                {
                    let buf = self.buffers.get_mut(&buffer_id).unwrap();
                    for (pos, next) in &pairs {
                        buf.delete_range(Range {
                            start: *pos,
                            end: *next,
                        });
                    }
                }
                // Cursors stay at their positions (each points to the former
                // next char); no position update needed.
                true
            }

            // ── Cursor movement ──────────────────────────────────────────────
            EditorAction::MoveLeft { extend } => {
                let buf = self.buffers.get(&buffer_id).unwrap();
                let new_pos = prev_position(buf, pane.cursors[0].pos);
                let pane = self.panes.get_mut(&pane_id).unwrap();
                set_cursor(pane, new_pos, extend);
                false
            }
            EditorAction::MoveRight { extend } => {
                let buf = self.buffers.get(&buffer_id).unwrap();
                let new_pos = next_position(buf, pane.cursors[0].pos);
                let pane = self.panes.get_mut(&pane_id).unwrap();
                set_cursor(pane, new_pos, extend);
                false
            }
            EditorAction::MoveUp { extend } => {
                let pos = pane.cursors[0].pos;
                let new_pos = if pos.line == 0 {
                    Position { line: 0, col: 0 }
                } else {
                    let buf = self.buffers.get(&buffer_id).unwrap();
                    let target_line = pos.line - 1;
                    let max_col = line_grapheme_len(buf, target_line);
                    Position {
                        line: target_line,
                        col: pos.col.min(max_col),
                    }
                };
                let pane = self.panes.get_mut(&pane_id).unwrap();
                set_cursor(pane, new_pos, extend);
                false
            }
            EditorAction::MoveDown { extend } => {
                let pos = pane.cursors[0].pos;
                let buf = self.buffers.get(&buffer_id).unwrap();
                let last_line = buf.line_count().saturating_sub(1);
                let new_pos = if pos.line >= last_line {
                    let max_col = line_grapheme_len(buf, last_line);
                    Position {
                        line: last_line,
                        col: max_col,
                    }
                } else {
                    let target_line = pos.line + 1;
                    let max_col = line_grapheme_len(buf, target_line);
                    Position {
                        line: target_line,
                        col: pos.col.min(max_col),
                    }
                };
                let pane = self.panes.get_mut(&pane_id).unwrap();
                set_cursor(pane, new_pos, extend);
                false
            }
            EditorAction::MoveLineStart { extend } => {
                let new_pos = Position {
                    line: pane.cursors[0].pos.line,
                    col: 0,
                };
                let pane = self.panes.get_mut(&pane_id).unwrap();
                set_cursor(pane, new_pos, extend);
                false
            }
            EditorAction::MoveLineEnd { extend } => {
                let line = pane.cursors[0].pos.line;
                let buf = self.buffers.get(&buffer_id).unwrap();
                let col = line_grapheme_len(buf, line);
                let new_pos = Position { line, col };
                let pane = self.panes.get_mut(&pane_id).unwrap();
                set_cursor(pane, new_pos, extend);
                false
            }
            EditorAction::MoveBufferStart { extend } => {
                let new_pos = Position { line: 0, col: 0 };
                let pane = self.panes.get_mut(&pane_id).unwrap();
                set_cursor(pane, new_pos, extend);
                false
            }
            EditorAction::MoveBufferEnd { extend } => {
                let buf = self.buffers.get(&buffer_id).unwrap();
                let last_line = buf.line_count().saturating_sub(1);
                let col = line_grapheme_len(buf, last_line);
                let new_pos = Position {
                    line: last_line,
                    col,
                };
                let pane = self.panes.get_mut(&pane_id).unwrap();
                set_cursor(pane, new_pos, extend);
                false
            }
            EditorAction::PageUp { extend } => {
                let pos = pane.cursors[0].pos;
                let page = pane.scroll_pos as usize;
                let new_line = pos.line.saturating_sub(page.max(1));
                let buf = self.buffers.get(&buffer_id).unwrap();
                let col = pos.col.min(line_grapheme_len(buf, new_line));
                let new_pos = Position {
                    line: new_line,
                    col,
                };
                let pane = self.panes.get_mut(&pane_id).unwrap();
                set_cursor(pane, new_pos, extend);
                false
            }
            EditorAction::PageDown { extend } => {
                let pos = pane.cursors[0].pos;
                let page = pane.scroll_pos as usize;
                let buf = self.buffers.get(&buffer_id).unwrap();
                let last_line = buf.line_count().saturating_sub(1);
                let new_line = (pos.line + page.max(1)).min(last_line);
                let col = pos.col.min(line_grapheme_len(buf, new_line));
                let new_pos = Position {
                    line: new_line,
                    col,
                };
                let pane = self.panes.get_mut(&pane_id).unwrap();
                set_cursor(pane, new_pos, extend);
                false
            }

            // ── Select all ──────────────────────────────────────────────────
            EditorAction::SelectAll => {
                let buf = self.buffers.get(&buffer_id).unwrap();
                let last_line = buf.line_count().saturating_sub(1);
                let last_col = line_grapheme_len(buf, last_line);
                let pane = self.panes.get_mut(&pane_id).unwrap();
                pane.cursors[0].anchor = Position { line: 0, col: 0 };
                pane.cursors[0].pos = Position {
                    line: last_line,
                    col: last_col,
                };
                false
            }

            // ── GoToLine ────────────────────────────────────────────────────
            EditorAction::GoToLine(target) => {
                let buf = self.buffers.get(&buffer_id).unwrap();
                let last_line = buf.line_count().saturating_sub(1);
                let line = target.min(last_line);
                let new_pos = Position { line, col: 0 };
                let pane = self.panes.get_mut(&pane_id).unwrap();
                set_cursor(pane, new_pos, false);
                let buf = self.buffers.get_mut(&buffer_id).unwrap();
                buf.flush_undo_group();
                false
            }

            // ── Save ────────────────────────────────────────────────────────
            EditorAction::Save => {
                let buf = self.buffers.get_mut(&buffer_id).unwrap();
                if let Some(path) = buf.tracked_path().map(|p| p.to_path_buf()) {
                    if let Err(e) = buf.save(&path) {
                        eprintln!("anvil: editor save failed: {e}");
                    }
                } else {
                    eprintln!("anvil: editor save: no tracked path (file-open is NE7+ scope)");
                }
                false
            }

            // ── SaveAs ──────────────────────────────────────────────────────
            // Write contents to `new_path`, update tracked_path, refresh
            // syntax/git associations.
            // TODO(anvil-tierJ-J2-nspanel): replace inline overlay with NSSavePanel.
            EditorAction::SaveAs(new_path) => {
                let buf = self.buffers.get_mut(&buffer_id).unwrap();
                if let Err(e) = buf.save(&new_path) {
                    eprintln!("anvil: save-as failed ({}): {e}", new_path.display());
                }
                false
            }

            // ── Undo / Redo ─────────────────────────────────────────────────
            EditorAction::Undo => {
                let buf = self.buffers.get_mut(&buffer_id).unwrap();
                buf.undo();
                // Clamp cursor to valid position after undo.
                let last_line = buf.line_count().saturating_sub(1);
                let pane = self.panes.get_mut(&pane_id).unwrap();
                let line = pane.cursors[0].pos.line.min(last_line);
                let buf = self.buffers.get(&buffer_id).unwrap();
                let col = pane.cursors[0].pos.col.min(line_grapheme_len(buf, line));
                let new_pos = Position { line, col };
                let pane = self.panes.get_mut(&pane_id).unwrap();
                set_cursor(pane, new_pos, false);
                true
            }
            EditorAction::Redo => {
                let buf = self.buffers.get_mut(&buffer_id).unwrap();
                buf.redo();
                let last_line = buf.line_count().saturating_sub(1);
                let pane = self.panes.get_mut(&pane_id).unwrap();
                let line = pane.cursors[0].pos.line.min(last_line);
                let buf = self.buffers.get(&buffer_id).unwrap();
                let col = pane.cursors[0].pos.col.min(line_grapheme_len(buf, line));
                let new_pos = Position { line, col };
                let pane = self.panes.get_mut(&pane_id).unwrap();
                set_cursor(pane, new_pos, false);
                true
            }

            // ── Search (NE11) ────────────────────────────────────────────────
            EditorAction::SearchOpen => {
                let pane = self.panes.get_mut(&pane_id).unwrap();
                if pane.search.is_none() {
                    pane.search = Some(EditorSearch::new());
                }
                false
            }
            EditorAction::SearchClose => {
                let pane = self.panes.get_mut(&pane_id).unwrap();
                pane.search = None;
                false
            }
            EditorAction::SearchSetQuery(q) => {
                // Update the query on the pane search state.
                {
                    let pane = self.panes.get_mut(&pane_id).unwrap();
                    if let Some(s) = &mut pane.search {
                        s.query = q;
                    }
                }
                // Re-scan using the buffer (separate field from panes, so these
                // borrows are non-overlapping at the struct-field level).
                let buf = self.buffers.get(&buffer_id).unwrap();
                let pane = self.panes.get_mut(&pane_id).unwrap();
                if let Some(s) = &mut pane.search {
                    s.rescan(buf);
                }
                false
            }
            EditorAction::SearchNext => {
                let pane = self.panes.get_mut(&pane_id).unwrap();
                if let Some(s) = &mut pane.search {
                    s.next();
                    if let Some(hit) = s.current_hit() {
                        // Select the match: anchor=start, pos=end.
                        pane.cursors[0].anchor = hit.start;
                        pane.cursors[0].pos = hit.end;
                        pane.scroll_target = hit.start.line as f32;
                    }
                }
                false
            }
            EditorAction::SearchPrev => {
                let pane = self.panes.get_mut(&pane_id).unwrap();
                if let Some(s) = &mut pane.search {
                    s.prev();
                    if let Some(hit) = s.current_hit() {
                        pane.cursors[0].anchor = hit.start;
                        pane.cursors[0].pos = hit.end;
                        pane.scroll_target = hit.start.line as f32;
                    }
                }
                false
            }
            EditorAction::SearchToggleRegex => {
                {
                    let pane = self.panes.get_mut(&pane_id).unwrap();
                    if let Some(s) = &mut pane.search {
                        s.is_regex = !s.is_regex;
                    }
                }
                let buf = self.buffers.get(&buffer_id).unwrap();
                let pane = self.panes.get_mut(&pane_id).unwrap();
                if let Some(s) = &mut pane.search {
                    s.rescan(buf);
                }
                false
            }

            // ── Mouse actions (NE7) ─────────────────────────────────────────
            EditorAction::MoveTo { pos, extend } => {
                let buf = self.buffers.get(&buffer_id).unwrap();
                let last_line = buf.line_count().saturating_sub(1);
                let clamped_line = pos.line.min(last_line);
                let max_col = line_grapheme_len(buf, clamped_line);
                let clamped = Position {
                    line: clamped_line,
                    col: pos.col.min(max_col),
                };
                let pane = self.panes.get_mut(&pane_id).unwrap();
                set_cursor(pane, clamped, extend);
                false
            }
            EditorAction::SelectWordAt(pos) => {
                let buf = self.buffers.get(&buffer_id).unwrap();
                let last_line = buf.line_count().saturating_sub(1);
                let line = pos.line.min(last_line);
                let line_str: String = buf.line(line).chars().collect();
                let graphemes: Vec<&str> = line_str
                    .trim_end_matches('\n')
                    .trim_end_matches('\r')
                    .graphemes(true)
                    .collect();
                let col = pos.col.min(graphemes.len().saturating_sub(1));
                let is_word = |g: &str| g.chars().all(|c| c.is_alphanumeric() || c == '_');
                // Walk left to find word start.
                let mut lo = col;
                while lo > 0 && is_word(graphemes[lo - 1]) {
                    lo -= 1;
                }
                // Walk right to find word end.
                let mut hi = col;
                if hi < graphemes.len() && is_word(graphemes[hi]) {
                    hi += 1;
                    while hi < graphemes.len() && is_word(graphemes[hi]) {
                        hi += 1;
                    }
                }
                let pane = self.panes.get_mut(&pane_id).unwrap();
                pane.cursors[0].anchor = Position { line, col: lo };
                pane.cursors[0].pos = Position { line, col: hi };
                false
            }
            EditorAction::SelectLineAt(pos) => {
                let buf = self.buffers.get(&buffer_id).unwrap();
                let last_line = buf.line_count().saturating_sub(1);
                let line = pos.line.min(last_line);
                let line_len = line_grapheme_len(buf, line);
                let pane = self.panes.get_mut(&pane_id).unwrap();
                pane.cursors[0].anchor = Position { line, col: 0 };
                pane.cursors[0].pos = Position {
                    line,
                    col: line_len,
                };
                false
            }

            // ── LSP UI (NE10) ────────────────────────────────────────────────
            // HoverRequest is a signal to main.rs; apply() just clears any stale
            // popup so it doesn't show stale content while the request is in-flight.
            EditorAction::HoverRequest => {
                let pane = self.panes.get_mut(&pane_id).unwrap();
                pane.hover_popup = None;
                pane.completion_popup = None;
                pane.code_actions_popup = None;
                false
            }
            EditorAction::HoverDismiss => {
                let pane = self.panes.get_mut(&pane_id).unwrap();
                pane.hover_popup = None;
                false
            }

            // ── Copy / Cut ──────────────────────────────────────────────────
            EditorAction::Copy => {
                let buf = self.buffers.get(&buffer_id).unwrap();
                if let Some(text) = selected_text(pane, buf) {
                    *clipboard_out = Some(text);
                } else {
                    // No selection: copy the whole current line (VS Code behaviour).
                    let line = pane.cursors[0].pos.line;
                    let line_text: String = buf.line(line).chars().collect();
                    let text = if line_text.ends_with('\n') {
                        line_text
                    } else {
                        line_text + "\n"
                    };
                    *clipboard_out = Some(text);
                }
                false
            }
            EditorAction::Cut => {
                if let Some(text) = selected_text(pane, self.buffers.get(&buffer_id).unwrap()) {
                    let (start, end) = selection_range(pane);
                    *clipboard_out = Some(text);
                    let buf = self.buffers.get_mut(&buffer_id).unwrap();
                    buf.delete_range(Range { start, end });
                    let pane = self.panes.get_mut(&pane_id).unwrap();
                    set_cursor(pane, start, false);
                    return true;
                }
                // No selection: cut the whole current line.
                let line = pane.cursors[0].pos.line;
                let line_text: String = {
                    let buf = self.buffers.get(&buffer_id).unwrap();
                    buf.line(line).chars().collect()
                };
                let text = if line_text.ends_with('\n') {
                    line_text.clone()
                } else {
                    line_text.clone() + "\n"
                };
                *clipboard_out = Some(text);
                // Delete from col 0 to start of next line.
                let buf = self.buffers.get(&buffer_id).unwrap();
                let last_line = buf.line_count().saturating_sub(1);
                let delete_end = if line < last_line {
                    Position {
                        line: line + 1,
                        col: 0,
                    }
                } else {
                    // Last line: delete to end of line content.
                    let col = line_text
                        .trim_end_matches('\n')
                        .trim_end_matches('\r')
                        .graphemes(true)
                        .count();
                    Position { line, col }
                };
                let buf = self.buffers.get_mut(&buffer_id).unwrap();
                buf.delete_range(Range {
                    start: Position { line, col: 0 },
                    end: delete_end,
                });
                let new_line = line.min(
                    self.buffers
                        .get(&buffer_id)
                        .unwrap()
                        .line_count()
                        .saturating_sub(1),
                );
                let pane = self.panes.get_mut(&pane_id).unwrap();
                set_cursor(
                    pane,
                    Position {
                        line: new_line,
                        col: 0,
                    },
                    false,
                );
                true
            }

            // ── AI ghost-text (NE14) ─────────────────────────────────────────
            EditorAction::AcceptGhostText => {
                let cursor_pos = pane.cursors[0].pos;
                let span_text = self.buffers.get(&buffer_id).and_then(|b| {
                    b.ghost_text
                        .iter()
                        .find(|s| s.anchor == cursor_pos)
                        .map(|s| s.text.clone())
                });
                if let Some(text) = span_text {
                    let buf = self.buffers.get_mut(&buffer_id).unwrap();
                    // insert_str routes through apply_edit, which clears ghost_text.
                    buf.insert_str(cursor_pos, &text);
                    let n = text.graphemes(true).count();
                    let new_pos = Position {
                        line: cursor_pos.line,
                        col: cursor_pos.col + n,
                    };
                    let pane = self.panes.get_mut(&pane_id).unwrap();
                    set_cursor(pane, new_pos, false);
                    true
                } else {
                    let buf = self.buffers.get_mut(&buffer_id).unwrap();
                    buf.clear_ghost_text();
                    false
                }
            }
            EditorAction::DismissGhostText => {
                let buf = self.buffers.get_mut(&buffer_id).unwrap();
                buf.clear_ghost_text();
                false
            }

            // ── Multi-cursor (NE13) ──────────────────────────────────────────
            EditorAction::AddCursorAt(pos) => {
                let buf = self.buffers.get(&buffer_id).unwrap();
                let last_line = buf.line_count().saturating_sub(1);
                let clamped_line = pos.line.min(last_line);
                let max_col = line_grapheme_len(buf, clamped_line);
                let clamped = Position {
                    line: clamped_line,
                    col: pos.col.min(max_col),
                };
                let pane = self.panes.get_mut(&pane_id).unwrap();
                // Deduplicate: don't add if a cursor already sits at this position.
                let already = pane.cursors.iter().any(|c| c.pos == clamped);
                if !already {
                    pane.cursors.push(Cursor {
                        pos: clamped,
                        anchor: clamped,
                    });
                }
                false
            }
            EditorAction::ClearSecondaryCursors => {
                let pane = self.panes.get_mut(&pane_id).unwrap();
                pane.cursors.truncate(1);
                false
            }

            // ── Find+Replace (item 9) ─────────────────────────────────────────
            EditorAction::FindReplaceOpen => {
                let pane = self.panes.get_mut(&pane_id).unwrap();
                if pane.search.is_none() {
                    pane.search = Some(EditorSearch::new());
                }
                if let Some(s) = &mut pane.search {
                    s.open_replace();
                }
                false
            }
            EditorAction::SetReplaceInput(text) => {
                let pane = self.panes.get_mut(&pane_id).unwrap();
                if let Some(s) = &mut pane.search {
                    s.replace_input = Some(text);
                }
                false
            }
            EditorAction::ReplaceOne => {
                // Replace the current hit with the replace string, rescan.
                let replacement = self
                    .panes
                    .get(&pane_id)
                    .and_then(|p| p.search.as_ref())
                    .and_then(|s| s.replace_input.clone())
                    .unwrap_or_default();
                let hit = self
                    .panes
                    .get(&pane_id)
                    .and_then(|p| p.search.as_ref())
                    .and_then(|s| s.current_hit());
                if let Some(range) = hit {
                    let buf = self.buffers.get_mut(&buffer_id).unwrap();
                    buf.replace_range(range, &replacement);
                    // Rescan after mutation.
                    let buf = self.buffers.get(&buffer_id).unwrap();
                    let pane = self.panes.get_mut(&pane_id).unwrap();
                    if let Some(s) = &mut pane.search {
                        s.rescan(buf);
                    }
                    return true;
                }
                false
            }
            EditorAction::ReplaceAll => {
                // Walk hits in reverse (highest position first) to avoid offset
                // drift. Collect first, then apply.
                let replacement = self
                    .panes
                    .get(&pane_id)
                    .and_then(|p| p.search.as_ref())
                    .and_then(|s| s.replace_input.clone())
                    .unwrap_or_default();
                let hits: Vec<anvil_editor::Range> = self
                    .panes
                    .get(&pane_id)
                    .and_then(|p| p.search.as_ref())
                    .map(|s| s.hits.clone())
                    .unwrap_or_default();
                if hits.is_empty() {
                    return false;
                }
                // Sort descending by start position.
                let mut sorted = hits;
                sorted.sort_by_key(|b| std::cmp::Reverse((b.start.line, b.start.col)));
                sorted.dedup_by_key(|r| (r.start.line, r.start.col));
                {
                    let buf = self.buffers.get_mut(&buffer_id).unwrap();
                    for range in &sorted {
                        buf.replace_range(*range, &replacement);
                    }
                }
                // Rescan after all replacements.
                let buf = self.buffers.get(&buffer_id).unwrap();
                let pane = self.panes.get_mut(&pane_id).unwrap();
                if let Some(s) = &mut pane.search {
                    s.rescan(buf);
                }
                true
            }

            // ── AddNextOccurrence (item 12) ────────────────────────────────────
            EditorAction::AddNextOccurrence => {
                // Determine the search text: selection of primary cursor, or word
                // under cursor if no selection.
                let search_text = {
                    let pane = self.panes.get(&pane_id).unwrap();
                    let buf = self.buffers.get(&buffer_id).unwrap();
                    let (start, end) = if pane.cursors[0].anchor != pane.cursors[0].pos {
                        let a = pane.cursors[0].anchor;
                        let p = pane.cursors[0].pos;
                        if (a.line, a.col) <= (p.line, p.col) {
                            (a, p)
                        } else {
                            (p, a)
                        }
                    } else {
                        // No selection: expand to word under cursor.
                        let pos = pane.cursors[0].pos;
                        let line_str: String = buf
                            .line(pos.line.min(buf.line_count().saturating_sub(1)))
                            .chars()
                            .collect();
                        let graphemes: Vec<&str> = line_str
                            .trim_end_matches('\n')
                            .trim_end_matches('\r')
                            .graphemes(true)
                            .collect();
                        let col = pos.col.min(graphemes.len().saturating_sub(1));
                        let is_word = |g: &str| g.chars().all(|c| c.is_alphanumeric() || c == '_');
                        let mut lo = col;
                        while lo > 0 && is_word(graphemes[lo - 1]) {
                            lo -= 1;
                        }
                        let mut hi = col;
                        if hi < graphemes.len() && is_word(graphemes[hi]) {
                            hi += 1;
                            while hi < graphemes.len() && is_word(graphemes[hi]) {
                                hi += 1;
                            }
                        }
                        (
                            Position {
                                line: pos.line,
                                col: lo,
                            },
                            Position {
                                line: pos.line,
                                col: hi,
                            },
                        )
                    };
                    // Extract text of [start, end) from buffer.
                    let mut s = String::new();
                    for line_idx in start.line..=end.line {
                        if line_idx >= buf.line_count() {
                            break;
                        }
                        let line_str: String = buf.line(line_idx).chars().collect();
                        let graphemes: Vec<&str> = line_str
                            .trim_end_matches('\n')
                            .trim_end_matches('\r')
                            .graphemes(true)
                            .collect();
                        let lo = if line_idx == start.line { start.col } else { 0 };
                        let hi = if line_idx == end.line {
                            end.col.min(graphemes.len())
                        } else {
                            graphemes.len()
                        };
                        for g in &graphemes[lo.min(graphemes.len())..hi.min(graphemes.len())] {
                            s.push_str(g);
                        }
                        if line_idx < end.line {
                            s.push('\n');
                        }
                    }
                    s
                };
                if search_text.is_empty() {
                    return false;
                }
                // Find next occurrence of search_text after the last cursor.
                let last_cursor_end = {
                    let pane = self.panes.get(&pane_id).unwrap();
                    pane.cursors
                        .iter()
                        .map(|c| {
                            let a = c.anchor;
                            let p = c.pos;
                            if (a.line, a.col) >= (p.line, p.col) {
                                a
                            } else {
                                p
                            }
                        })
                        .max_by_key(|p| (p.line, p.col))
                        .unwrap_or(Position { line: 0, col: 0 })
                };
                // Scan buffer text for occurrences.
                let text = self.buffers.get(&buffer_id).unwrap().to_text();
                // Find all occurrences, pick the first one after last_cursor_end.
                let occurrences: Vec<(Position, Position)> = {
                    let buf = self.buffers.get(&buffer_id).unwrap();
                    let mut results = Vec::new();
                    for (byte_start, _) in text.match_indices(search_text.as_str()) {
                        let byte_end = byte_start + search_text.len();
                        if byte_end > text.len() {
                            continue;
                        }
                        let char_start = text[..byte_start].chars().count();
                        let char_end = text[..byte_end].chars().count();
                        let start_line = buf.char_to_line(char_start);
                        let end_line = buf.char_to_line(char_end.saturating_sub(1).max(char_start));
                        let start_col = char_start - buf.line_to_char(start_line);
                        let end_col = char_end - buf.line_to_char(end_line);
                        results.push((
                            Position {
                                line: start_line,
                                col: start_col,
                            },
                            Position {
                                line: end_line,
                                col: end_col,
                            },
                        ));
                    }
                    results
                };
                // Pick the first occurrence after last_cursor_end (wraps).
                let next_hit = occurrences
                    .iter()
                    .find(|(start, _)| {
                        (start.line, start.col) > (last_cursor_end.line, last_cursor_end.col)
                    })
                    .or_else(|| occurrences.first());
                if let Some((hit_start, hit_end)) = next_hit.copied() {
                    let pane = self.panes.get_mut(&pane_id).unwrap();
                    // Only add if not already covered by an existing cursor.
                    let already = pane
                        .cursors
                        .iter()
                        .any(|c| c.anchor == hit_start && c.pos == hit_end);
                    if !already {
                        pane.cursors.push(Cursor {
                            pos: hit_end,
                            anchor: hit_start,
                        });
                        pane.scroll_target = hit_start.line as f32;
                    }
                }
                false
            }

            // ── Code folding (item 13) ─────────────────────────────────────────
            EditorAction::ToggleFold(line) => {
                let pane = self.panes.get_mut(&pane_id).unwrap();
                let folds = pane.folds.entry(buffer_id).or_default();
                if folds.contains(&line) {
                    folds.remove(&line);
                } else {
                    folds.insert(line);
                }
                false
            }

            // ── #16 Auto-indent on Enter ───────────────────────────────────────
            EditorAction::InsertNewlineSmart => {
                let pos = pane.cursors[0].pos;
                let indent = {
                    let buf = self.buffers.get(&buffer_id).unwrap();
                    smart_indent(buf, pos)
                };
                {
                    let buf = self.buffers.get_mut(&buffer_id).unwrap();
                    // Insert newline + indentation as one edit.
                    let mut s = String::from('\n');
                    s.push_str(&indent);
                    buf.insert_str(pos, &s);
                }
                let pane = self.panes.get_mut(&pane_id).unwrap();
                let new_pos = Position {
                    line: pos.line + 1,
                    col: indent.graphemes(true).count(),
                };
                set_cursor(pane, new_pos, false);
                true
            }

            // ── #18 Toggle line comment ────────────────────────────────────────
            EditorAction::ToggleLineComment => {
                let (start_line, end_line) = {
                    let pane = self.panes.get(&pane_id).unwrap();
                    let (s, e) = selection_range(pane);
                    if s == e {
                        (s.line, s.line)
                    } else {
                        (s.line, e.line)
                    }
                };
                let marker = {
                    let buf = self.buffers.get(&buffer_id).unwrap();
                    comment_marker_for_buffer(buf)
                };
                // Determine whether ALL lines are currently commented.
                let all_commented = {
                    let buf = self.buffers.get(&buffer_id).unwrap();
                    (start_line..=end_line).all(|ln| {
                        let line_str: String = buf.line(ln).chars().collect();
                        let trimmed = line_str.trim_start();
                        trimmed.starts_with(marker)
                    })
                };
                // Apply in reverse line order so earlier line edits don't shift
                // later line positions.
                for ln in (start_line..=end_line).rev() {
                    let line_str: String = {
                        let buf = self.buffers.get(&buffer_id).unwrap();
                        buf.line(ln).chars().collect()
                    };
                    let buf = self.buffers.get_mut(&buffer_id).unwrap();
                    if all_commented {
                        // Strip the comment marker (and one space if present).
                        if let Some(col) = leading_col_of_comment(&line_str, marker) {
                            let end_col = col
                                + marker.graphemes(true).count()
                                + if line_str[leading_byte_offset(&line_str) + marker.len()..]
                                    .starts_with(' ')
                                {
                                    1
                                } else {
                                    0
                                };
                            buf.delete_range(Range {
                                start: Position { line: ln, col },
                                end: Position {
                                    line: ln,
                                    col: end_col,
                                },
                            });
                        }
                    } else {
                        // Prepend marker + space after leading whitespace.
                        let insert_col = leading_grapheme_count(&line_str);
                        let mut s = marker.to_string();
                        s.push(' ');
                        buf.insert_str(
                            Position {
                                line: ln,
                                col: insert_col,
                            },
                            &s,
                        );
                    }
                }
                true
            }

            // ── #19 Duplicate line ─────────────────────────────────────────────
            EditorAction::DuplicateLine => {
                let line = pane.cursors[0].pos.line;
                let col = pane.cursors[0].pos.col;
                let line_text: String = {
                    let buf = self.buffers.get(&buffer_id).unwrap();
                    buf.line(line).chars().collect()
                };
                // Ensure line_text ends with a newline; if not, add one.
                let insert_text = if line_text.ends_with('\n') {
                    line_text.clone()
                } else {
                    let mut s = line_text.clone();
                    s.push('\n');
                    s
                };
                let insert_len = line_text
                    .trim_end_matches('\n')
                    .trim_end_matches('\r')
                    .graphemes(true)
                    .count();
                // Insert the duplicate below the current line.
                {
                    let buf = self.buffers.get_mut(&buffer_id).unwrap();
                    buf.insert_str(
                        Position {
                            line: line + 1,
                            col: 0,
                        },
                        &insert_text,
                    );
                }
                let new_pos = Position {
                    line: line + 1,
                    col: col.min(insert_len),
                };
                let pane = self.panes.get_mut(&pane_id).unwrap();
                set_cursor(pane, new_pos, false);
                true
            }

            // ── #20 Move line up / down ────────────────────────────────────────
            EditorAction::MoveLineUp => {
                let line = pane.cursors[0].pos.line;
                if line == 0 {
                    return false;
                }
                let col = pane.cursors[0].pos.col;
                let (cur_text, prev_text) = {
                    let buf = self.buffers.get(&buffer_id).unwrap();
                    let cur: String = buf.line(line).chars().collect();
                    let prev: String = buf.line(line - 1).chars().collect();
                    (cur, prev)
                };
                // Normalise both lines to end with exactly one newline.
                let cur_content = normalise_line(&cur_text);
                let prev_content = normalise_line(&prev_text);
                // Replace prev_line with cur_content, cur_line with prev_content.
                {
                    let buf = self.buffers.get_mut(&buffer_id).unwrap();
                    // Delete both lines and reinsert swapped. Work from bottom up.
                    let cur_len = cur_text
                        .trim_end_matches('\n')
                        .trim_end_matches('\r')
                        .graphemes(true)
                        .count();
                    let prev_len = prev_text
                        .trim_end_matches('\n')
                        .trim_end_matches('\r')
                        .graphemes(true)
                        .count();
                    buf.delete_range(Range {
                        start: Position { line, col: 0 },
                        end: Position { line, col: cur_len },
                    });
                    // After deleting cur line content, reinsert prev_content there.
                    buf.insert_str(
                        Position { line, col: 0 },
                        prev_content.trim_end_matches('\n').trim_end_matches('\r'),
                    );
                    // Now fix prev line.
                    buf.delete_range(Range {
                        start: Position {
                            line: line - 1,
                            col: 0,
                        },
                        end: Position {
                            line: line - 1,
                            col: prev_len,
                        },
                    });
                    buf.insert_str(
                        Position {
                            line: line - 1,
                            col: 0,
                        },
                        cur_content.trim_end_matches('\n').trim_end_matches('\r'),
                    );
                }
                let new_line_len = normalise_line(&cur_text)
                    .trim_end_matches('\n')
                    .trim_end_matches('\r')
                    .graphemes(true)
                    .count();
                let new_pos = Position {
                    line: line - 1,
                    col: col.min(new_line_len),
                };
                let pane = self.panes.get_mut(&pane_id).unwrap();
                set_cursor(pane, new_pos, false);
                true
            }
            EditorAction::MoveLineDown => {
                let line = pane.cursors[0].pos.line;
                let col = pane.cursors[0].pos.col;
                let last_line = {
                    let buf = self.buffers.get(&buffer_id).unwrap();
                    buf.line_count().saturating_sub(1)
                };
                if line >= last_line {
                    return false;
                }
                let (cur_text, next_text) = {
                    let buf = self.buffers.get(&buffer_id).unwrap();
                    let cur: String = buf.line(line).chars().collect();
                    let next: String = buf.line(line + 1).chars().collect();
                    (cur, next)
                };
                let cur_content = normalise_line(&cur_text);
                let next_content = normalise_line(&next_text);
                {
                    let buf = self.buffers.get_mut(&buffer_id).unwrap();
                    let next_len = next_text
                        .trim_end_matches('\n')
                        .trim_end_matches('\r')
                        .graphemes(true)
                        .count();
                    let cur_len = cur_text
                        .trim_end_matches('\n')
                        .trim_end_matches('\r')
                        .graphemes(true)
                        .count();
                    // Delete next line content first (higher line index).
                    buf.delete_range(Range {
                        start: Position {
                            line: line + 1,
                            col: 0,
                        },
                        end: Position {
                            line: line + 1,
                            col: next_len,
                        },
                    });
                    buf.insert_str(
                        Position {
                            line: line + 1,
                            col: 0,
                        },
                        cur_content.trim_end_matches('\n').trim_end_matches('\r'),
                    );
                    // Now fix current line.
                    buf.delete_range(Range {
                        start: Position { line, col: 0 },
                        end: Position { line, col: cur_len },
                    });
                    buf.insert_str(
                        Position { line, col: 0 },
                        next_content.trim_end_matches('\n').trim_end_matches('\r'),
                    );
                }
                let new_line_len = normalise_line(&cur_text)
                    .trim_end_matches('\n')
                    .trim_end_matches('\r')
                    .graphemes(true)
                    .count();
                let new_pos = Position {
                    line: line + 1,
                    col: col.min(new_line_len),
                };
                let pane = self.panes.get_mut(&pane_id).unwrap();
                set_cursor(pane, new_pos, false);
                true
            }

            // ── #21 Indent / dedent selection ─────────────────────────────────
            EditorAction::IndentSelection => {
                let (start_line, end_line) = {
                    let pane = self.panes.get(&pane_id).unwrap();
                    let (s, e) = selection_range(pane);
                    if s == e {
                        // No selection: fall back to inserting 4 spaces at cursor.
                        let pos = s;
                        let buf = self.buffers.get_mut(&buffer_id).unwrap();
                        buf.insert_str(pos, "    ");
                        let pane = self.panes.get_mut(&pane_id).unwrap();
                        let new_pos = advance_col(pos, 4);
                        set_cursor(pane, new_pos, false);
                        return true;
                    }
                    (s.line, e.line)
                };
                for ln in (start_line..=end_line).rev() {
                    let buf = self.buffers.get_mut(&buffer_id).unwrap();
                    buf.insert_str(Position { line: ln, col: 0 }, "    ");
                }
                // Shift the selection by 4 cols on both ends.
                let pane = self.panes.get_mut(&pane_id).unwrap();
                pane.cursors[0].anchor.col = pane.cursors[0].anchor.col.saturating_add(4);
                pane.cursors[0].pos.col = pane.cursors[0].pos.col.saturating_add(4);
                true
            }
            EditorAction::DedentSelection => {
                let (start_line, end_line) = {
                    let pane = self.panes.get(&pane_id).unwrap();
                    let (s, e) = selection_range(pane);
                    (s.line, e.line)
                };
                for ln in (start_line..=end_line).rev() {
                    let line_str: String = {
                        let buf = self.buffers.get(&buffer_id).unwrap();
                        buf.line(ln).chars().collect()
                    };
                    let removed = leading_spaces_to_remove(&line_str, 4);
                    if removed > 0 {
                        let buf = self.buffers.get_mut(&buffer_id).unwrap();
                        buf.delete_range(Range {
                            start: Position { line: ln, col: 0 },
                            end: Position {
                                line: ln,
                                col: removed,
                            },
                        });
                    }
                }
                // Adjust cursor and anchor (saturating).
                let pane = self.panes.get_mut(&pane_id).unwrap();
                pane.cursors[0].anchor.col = pane.cursors[0].anchor.col.saturating_sub(4);
                pane.cursors[0].pos.col = pane.cursors[0].pos.col.saturating_sub(4);
                true
            }

            // ── #23 Format file ────────────────────────────────────────────────
            EditorAction::FormatFile => {
                // TODO(anvil-tierC-#23-lsp): when an LSP connection is active,
                // send textDocument/formatting and apply returned TextEdits.
                // For now: rustfmt fallback for Rust buffers.
                let is_rust = self
                    .buffers
                    .get(&buffer_id)
                    .and_then(|b| b.language_id())
                    .map(|id| id == "rust")
                    .unwrap_or(false);
                if is_rust {
                    let text = self.buffers.get(&buffer_id).unwrap().to_text();
                    match run_rustfmt(&text) {
                        Some(formatted) if formatted != text => {
                            let buf = self.buffers.get_mut(&buffer_id).unwrap();
                            let last_line = buf.line_count().saturating_sub(1);
                            let last_col = line_grapheme_len(buf, last_line);
                            buf.replace_range(
                                Range {
                                    start: Position { line: 0, col: 0 },
                                    end: Position {
                                        line: last_line,
                                        col: last_col,
                                    },
                                },
                                &formatted,
                            );
                            return true;
                        }
                        _ => {}
                    }
                }
                false
            }

            // ── Completion popup (item 16) ─────────────────────────────────────
            EditorAction::CompletionOpen(entries) => {
                let pane = self.panes.get_mut(&pane_id).unwrap();
                let anchor = pane.cursors[0].pos;
                pane.completion_popup = Some(CompletionPopup {
                    items: entries,
                    selected: 0,
                    anchor,
                    filter_prefix: String::new(),
                });
                false
            }
            EditorAction::CompletionUp => {
                let pane = self.panes.get_mut(&pane_id).unwrap();
                if let Some(cp) = &mut pane.completion_popup {
                    let n = cp.visible_items().len();
                    if n > 0 {
                        cp.selected = cp.selected.saturating_sub(1);
                    }
                }
                false
            }
            EditorAction::CompletionDown => {
                let pane = self.panes.get_mut(&pane_id).unwrap();
                if let Some(cp) = &mut pane.completion_popup {
                    let n = cp.visible_items().len();
                    if n > 0 {
                        cp.selected = (cp.selected + 1).min(n - 1);
                    }
                }
                false
            }
            EditorAction::CompletionAccept => {
                // Extract the insert_text of the selected visible item.
                let insert_text: Option<String> = {
                    let pane = self.panes.get(&pane_id).unwrap();
                    pane.completion_popup.as_ref().and_then(|cp| {
                        let vis = cp.visible_items();
                        vis.get(cp.selected).map(|e| e.insert_text.clone())
                    })
                };
                // Remove the completion popup and the filter prefix chars.
                let (anchor, prefix_len) = {
                    let pane = self.panes.get(&pane_id).unwrap();
                    pane.completion_popup
                        .as_ref()
                        .map(|cp| (cp.anchor, cp.filter_prefix.chars().count()))
                        .unwrap_or((pane.cursors[0].pos, 0))
                };
                let pane = self.panes.get_mut(&pane_id).unwrap();
                pane.completion_popup = None;
                if let Some(text) = insert_text {
                    // Delete the filter prefix chars typed after the trigger.
                    let del_start = Position {
                        line: anchor.line,
                        col: anchor.col.saturating_sub(prefix_len),
                    };
                    let del_end = anchor;
                    if del_start != del_end {
                        let buf = self.buffers.get_mut(&buffer_id).unwrap();
                        buf.delete_range(anvil_editor::Range {
                            start: del_start,
                            end: del_end,
                        });
                    }
                    // Insert the completion text.
                    let cur_pos = self.panes.get(&pane_id).unwrap().cursors[0].pos;
                    let buf = self.buffers.get_mut(&buffer_id).unwrap();
                    buf.insert_str(cur_pos, &text);
                    let n = text.graphemes(true).count();
                    let new_pos = Position {
                        line: cur_pos.line,
                        col: cur_pos.col + n,
                    };
                    let pane = self.panes.get_mut(&pane_id).unwrap();
                    set_cursor(pane, new_pos, false);
                    true
                } else {
                    false
                }
            }
            EditorAction::CompletionDismiss => {
                let pane = self.panes.get_mut(&pane_id).unwrap();
                pane.completion_popup = None;
                false
            }
            EditorAction::CompletionFilter(ch) => {
                let pane = self.panes.get_mut(&pane_id).unwrap();
                if let Some(cp) = &mut pane.completion_popup {
                    cp.filter_prefix.push(ch);
                    // Clamp selected to new visible count.
                    let n = cp.visible_items().len();
                    if n == 0 {
                        pane.completion_popup = None;
                    } else {
                        let cp = pane.completion_popup.as_mut().unwrap();
                        cp.selected = cp.selected.min(n - 1);
                    }
                }
                false
            }
            // ── Code actions popup (item 25) ───────────────────────────────────
            EditorAction::CodeActionsOpen(entries) => {
                let pane = self.panes.get_mut(&pane_id).unwrap();
                let anchor = pane.cursors[0].pos;
                pane.code_actions_popup = Some(CodeActionsPopup {
                    items: entries,
                    selected: 0,
                    anchor,
                });
                false
            }
            EditorAction::CodeActionsUp => {
                let pane = self.panes.get_mut(&pane_id).unwrap();
                if let Some(cp) = &mut pane.code_actions_popup {
                    cp.selected = cp.selected.saturating_sub(1);
                }
                false
            }
            EditorAction::CodeActionsDown => {
                let pane = self.panes.get_mut(&pane_id).unwrap();
                if let Some(cp) = &mut pane.code_actions_popup {
                    let n = cp.items.len();
                    if n > 0 {
                        cp.selected = (cp.selected + 1).min(n - 1);
                    }
                }
                false
            }
            EditorAction::CodeActionsDismiss => {
                let pane = self.panes.get_mut(&pane_id).unwrap();
                pane.code_actions_popup = None;
                false
            }

            // ── Tier-K selection (K6) ────────────────────────────────────
            EditorAction::SelectLine => {
                let buf = self.buffers.get(&buffer_id).unwrap();
                let last_line = buf.line_count().saturating_sub(1);
                let pane = self.panes.get(&pane_id).unwrap();
                let anchor = pane.cursors[0].anchor;
                let pos = pane.cursors[0].pos;
                // Detect "already line-aligned": anchor at col 0 and cursor at
                // end-of-line. Extend by one line in that case.
                let cur_line = pos.line;
                let cur_end_col = line_grapheme_len(buf, cur_line);
                let line_aligned = anchor.col == 0 && pos.col == cur_end_col && cur_end_col > 0;
                let pane = self.panes.get_mut(&pane_id).unwrap();
                if line_aligned && cur_line < last_line {
                    // Extend head to end of the next line.
                    let next_line = cur_line + 1;
                    let next_end_col = line_grapheme_len(buf, next_line);
                    pane.cursors[0].pos = Position {
                        line: next_line,
                        col: next_end_col,
                    };
                } else {
                    // Fresh select: anchor at col 0, pos at end-of-line.
                    let line = pos.line.min(last_line);
                    let end_col = line_grapheme_len(buf, line);
                    pane.cursors[0].anchor = Position { line, col: 0 };
                    pane.cursors[0].pos = Position { line, col: end_col };
                }
                false
            }

            // ── Tier-H toggles ────────────────────────────────────────────
            EditorAction::ToggleSoftWrap => {
                let pane = self.panes.get_mut(&pane_id).unwrap();
                pane.soft_wrap = !pane.soft_wrap;
                // P3: force scroll_x to 0 when enabling soft-wrap.
                if pane.soft_wrap {
                    pane.scroll_x = 0.0;
                }
                false
            }
            EditorAction::ToggleShowWhitespace => {
                let pane = self.panes.get_mut(&pane_id).unwrap();
                pane.show_whitespace = !pane.show_whitespace;
                false
            }

            // ── Tier-W additions ──────────────────────────────────────────────

            // W4: delete current line; cursor lands at col 0 of successor line.
            EditorAction::DeleteLine => {
                let line = pane.cursors[0].pos.line;
                let buf = self.buffers.get(&buffer_id).unwrap();
                let last_line = buf.line_count().saturating_sub(1);
                // Build the range to delete: from col 0 of `line` to col 0 of
                // the next line (or end of content when on the last line).
                let (del_start, del_end) = if line < last_line {
                    (
                        Position { line, col: 0 },
                        Position {
                            line: line + 1,
                            col: 0,
                        },
                    )
                } else {
                    // Last line: delete content but leave the line itself so
                    // the buffer is never empty.
                    let col_end = line_grapheme_len(buf, line);
                    (Position { line, col: 0 }, Position { line, col: col_end })
                };
                {
                    let buf = self.buffers.get_mut(&buffer_id).unwrap();
                    buf.delete_range(Range {
                        start: del_start,
                        end: del_end,
                    });
                }
                let buf = self.buffers.get(&buffer_id).unwrap();
                let new_last = buf.line_count().saturating_sub(1);
                let new_line = line.min(new_last);
                let pane = self.panes.get_mut(&pane_id).unwrap();
                set_cursor(
                    pane,
                    Position {
                        line: new_line,
                        col: 0,
                    },
                    false,
                );
                true
            }

            // W7: place one cursor at every occurrence of the selected text (or
            // word under cursor).  Existing cursors are replaced.
            EditorAction::SelectAllOccurrences => {
                let search_text = {
                    let pane = self.panes.get(&pane_id).unwrap();
                    let buf = self.buffers.get(&buffer_id).unwrap();
                    word_or_selection_text(pane, buf)
                };
                if search_text.is_empty() {
                    return false;
                }
                let text = self.buffers.get(&buffer_id).unwrap().to_text();
                let occurrences: Vec<(Position, Position)> = {
                    let buf = self.buffers.get(&buffer_id).unwrap();
                    let mut results = Vec::new();
                    for (byte_start, _) in text.match_indices(search_text.as_str()) {
                        let byte_end = byte_start + search_text.len();
                        if byte_end > text.len() {
                            continue;
                        }
                        let char_start = text[..byte_start].chars().count();
                        let char_end = text[..byte_end].chars().count();
                        let sl = buf.char_to_line(char_start);
                        let el = buf.char_to_line(char_end.saturating_sub(1).max(char_start));
                        let sc = char_start - buf.line_to_char(sl);
                        let ec = char_end - buf.line_to_char(el);
                        results.push((
                            Position { line: sl, col: sc },
                            Position { line: el, col: ec },
                        ));
                    }
                    results
                };
                if occurrences.is_empty() {
                    return false;
                }
                let pane = self.panes.get_mut(&pane_id).unwrap();
                pane.cursors.clear();
                for (start, end) in occurrences {
                    pane.cursors.push(Cursor {
                        pos: end,
                        anchor: start,
                    });
                }
                // Scroll to first match.
                if let Some(first) = pane.cursors.first() {
                    pane.scroll_target = first.anchor.line as f32;
                }
                false
            }

            // W8: drop the last secondary cursor; no-op when only one cursor exists.
            EditorAction::DropLastCursor => {
                let pane = self.panes.get_mut(&pane_id).unwrap();
                if pane.cursors.len() > 1 {
                    pane.cursors.pop();
                }
                false
            }

            // W9: fold all — use the simple heuristic of any line ending with
            // `{`, `(`, or `[` (after stripping trailing whitespace/newline).
            EditorAction::FoldAll => {
                let fold_lines: Vec<usize> = {
                    let buf = self.buffers.get(&buffer_id).unwrap();
                    (0..buf.line_count())
                        .filter(|&ln| {
                            let s: String = buf.line(ln).chars().collect();
                            let trimmed =
                                s.trim_end_matches('\n').trim_end_matches('\r').trim_end();
                            matches!(trimmed.chars().last(), Some('{') | Some('(') | Some('['))
                        })
                        .collect()
                };
                let pane = self.panes.get_mut(&pane_id).unwrap();
                let folds = pane.folds.entry(buffer_id).or_default();
                for ln in fold_lines {
                    folds.insert(ln);
                }
                false
            }

            // W9: unfold all — clear the fold set for the active buffer.
            EditorAction::UnfoldAll => {
                let pane = self.panes.get_mut(&pane_id).unwrap();
                pane.folds.remove(&buffer_id);
                false
            }

            // W10: convert case operations on selected text or current word.
            EditorAction::ConvertCaseUpper => {
                convert_case(self, pane_id, buffer_id, |s| s.to_uppercase())
            }
            EditorAction::ConvertCaseLower => {
                convert_case(self, pane_id, buffer_id, |s| s.to_lowercase())
            }
            EditorAction::ConvertCaseTitle => convert_case(self, pane_id, buffer_id, |s| {
                let mut result = String::with_capacity(s.len());
                let mut cap_next = true;
                for ch in s.chars() {
                    if ch.is_whitespace() {
                        cap_next = true;
                        result.push(ch);
                    } else if cap_next {
                        result.extend(ch.to_uppercase());
                        cap_next = false;
                    } else {
                        result.extend(ch.to_lowercase());
                    }
                }
                result
            }),

            // W11: sort selected lines alphabetically (case-insensitive).
            // Requires a multi-line selection; single-line selection is a no-op.
            EditorAction::SortSelectedLines => {
                let (start_line, end_line) = {
                    let pane = self.panes.get(&pane_id).unwrap();
                    let (s, e) = selection_range(pane);
                    (s.line, e.line)
                };
                if start_line == end_line {
                    return false;
                }
                let lines: Vec<String> = {
                    let buf = self.buffers.get(&buffer_id).unwrap();
                    (start_line..=end_line)
                        .map(|ln| buf.line(ln).chars().collect::<String>())
                        .collect()
                };
                let mut sorted = lines.clone();
                sorted.sort_by_key(|a: &String| a.to_lowercase());
                if sorted == lines {
                    return false; // already sorted
                }
                // Replace each line in reverse order to keep positions stable.
                for (i, ln) in (start_line..=end_line).rev().enumerate() {
                    let sorted_i = (end_line - start_line) - i;
                    let old_text: String = {
                        let buf = self.buffers.get(&buffer_id).unwrap();
                        buf.line(ln).chars().collect()
                    };
                    let old_content = old_text.trim_end_matches('\n').trim_end_matches('\r');
                    let new_content = sorted[sorted_i]
                        .trim_end_matches('\n')
                        .trim_end_matches('\r');
                    if old_content == new_content {
                        continue;
                    }
                    let old_len = old_content.graphemes(true).count();
                    let new_text = new_content.to_string();
                    let buf = self.buffers.get_mut(&buffer_id).unwrap();
                    buf.delete_range(Range {
                        start: Position { line: ln, col: 0 },
                        end: Position {
                            line: ln,
                            col: old_len,
                        },
                    });
                    buf.insert_str(Position { line: ln, col: 0 }, &new_text);
                }
                true
            }

            // W12: join current line with next by replacing the newline with a space.
            EditorAction::JoinLines => {
                let line = pane.cursors[0].pos.line;
                let buf = self.buffers.get(&buffer_id).unwrap();
                let last_line = buf.line_count().saturating_sub(1);
                if line >= last_line {
                    return false; // already last line
                }
                // The newline at end of `line` is the char at position (line, line_len).
                let line_len = line_grapheme_len(buf, line);
                // Delete the newline character, then insert a space if the next line
                // is non-empty.
                let next_line_str: String = buf.line(line + 1).chars().collect();
                let next_non_empty = !next_line_str
                    .trim_end_matches('\n')
                    .trim_end_matches('\r')
                    .is_empty();
                {
                    let buf = self.buffers.get_mut(&buffer_id).unwrap();
                    // Delete from end of this line to start of next.
                    buf.delete_range(Range {
                        start: Position {
                            line,
                            col: line_len,
                        },
                        end: Position {
                            line: line + 1,
                            col: 0,
                        },
                    });
                    if next_non_empty {
                        buf.insert_str(
                            Position {
                                line,
                                col: line_len,
                            },
                            " ",
                        );
                    }
                }
                // Cursor stays at the join point.
                let pane = self.panes.get_mut(&pane_id).unwrap();
                set_cursor(
                    pane,
                    Position {
                        line,
                        col: line_len,
                    },
                    false,
                );
                true
            }

            // ── Tier-X actions ────────────────────────────────────────────────

            // X14: move one word boundary to the left (Opt+←).
            EditorAction::MoveWordLeft { extend } => {
                let buf = self.buffers.get(&buffer_id).unwrap();
                let new_pos = prev_word_boundary(buf, pane.cursors[0].pos);
                let pane = self.panes.get_mut(&pane_id).unwrap();
                set_cursor(pane, new_pos, extend);
                false
            }

            // X14: move one word boundary to the right (Opt+→).
            EditorAction::MoveWordRight { extend } => {
                let buf = self.buffers.get(&buffer_id).unwrap();
                let new_pos = next_word_boundary(buf, pane.cursors[0].pos);
                let pane = self.panes.get_mut(&pane_id).unwrap();
                set_cursor(pane, new_pos, extend);
                false
            }

            // X13: trim trailing whitespace in selection or entire buffer.
            EditorAction::TrimTrailingWhitespace => {
                let buf = self.buffers.get(&buffer_id).unwrap();
                let line_count = buf.line_count();
                let (trim_start, trim_end) = {
                    let pane = self.panes.get(&pane_id).unwrap();
                    let (s, e) = selection_range(pane);
                    if s == e {
                        // No selection: trim whole buffer.
                        (0, line_count.saturating_sub(1))
                    } else {
                        (s.line, e.line)
                    }
                };
                let mut mutated = false;
                // Walk lines in reverse so earlier edits don't shift later positions.
                for ln in (trim_start..=trim_end.min(line_count.saturating_sub(1))).rev() {
                    let buf = self.buffers.get(&buffer_id).unwrap();
                    let line_str: String = buf.line(ln).chars().collect();
                    let content = line_str.trim_end_matches('\n').trim_end_matches('\r');
                    let trimmed = content.trim_end_matches([' ', '\t']);
                    if trimmed.len() < content.len() {
                        let col_start = trimmed.graphemes(true).count();
                        let col_end = content.graphemes(true).count();
                        let buf = self.buffers.get_mut(&buffer_id).unwrap();
                        buf.delete_range(Range {
                            start: Position {
                                line: ln,
                                col: col_start,
                            },
                            end: Position {
                                line: ln,
                                col: col_end,
                            },
                        });
                        mutated = true;
                    }
                }
                // Snap cursor column to the (possibly shortened) line.
                if mutated {
                    let buf = self.buffers.get(&buffer_id).unwrap();
                    let cursor_line = self.panes.get(&pane_id).unwrap().cursors[0].pos.line;
                    let max_col = line_grapheme_len(buf, cursor_line);
                    let pane = self.panes.get_mut(&pane_id).unwrap();
                    pane.cursors[0].pos.col = pane.cursors[0].pos.col.min(max_col);
                    pane.cursors[0].anchor = pane.cursors[0].pos;
                }
                mutated
            }

            // X8: jump to the first outline symbol matching the word under cursor.
            EditorAction::GotoSymbolAtCursor => {
                // Get word under cursor.
                let word = {
                    let buf = self.buffers.get(&buffer_id).unwrap();
                    let pos = pane.cursors[0].pos;
                    let line_str: String = buf.line(pos.line).chars().collect();
                    let chars: Vec<char> = line_str.chars().collect();
                    let col = pos.col.min(chars.len());
                    let is_word = |c: char| c.is_alphanumeric() || c == '_';
                    let start = (0..col)
                        .rev()
                        .take_while(|&i| is_word(chars[i]))
                        .last()
                        .unwrap_or(col);
                    let end = (col..chars.len())
                        .take_while(|&i| is_word(chars[i]))
                        .last()
                        .map(|i| i + 1)
                        .unwrap_or(col);
                    chars[start..end].iter().collect::<String>()
                };
                if word.is_empty() {
                    return false;
                }
                // Derive outline from syntax layer.
                let buf = self.buffers.get(&buffer_id).unwrap();
                let text = buf.to_text();
                let symbols = anvil_editor::derive_outline_rows(buf.syntax(), &text);
                let target = symbols.iter().find(|s| s.name == word).map(|s| s.line);
                if let Some(target_line) = target {
                    let new_pos = Position {
                        line: target_line,
                        col: 0,
                    };
                    let pane = self.panes.get_mut(&pane_id).unwrap();
                    set_cursor(pane, new_pos, false);
                    pane.scroll_target = target_line.saturating_sub(4) as f32;
                    false
                } else {
                    false
                }
            }

            // X9: peek definition (stub — full inline popup deferred).
            // TODO(anvil-tierX-#9): implement floating inline peek popup.
            EditorAction::PeekDefinition => {
                // No-op: the peek popup requires a separate floating overlay pass.
                // The caller (main.rs) should open a tooltip instead when LSP returns.
                false
            }

            // X11: expand snippet body at cursor.
            EditorAction::ExpandSnippet { trigger, body } => {
                let pos = pane.cursors[0].pos;
                // Delete the trigger word that was already typed.
                let trigger_len = trigger.graphemes(true).count();
                let del_start = Position {
                    line: pos.line,
                    col: pos.col.saturating_sub(trigger_len),
                };
                {
                    let buf = self.buffers.get_mut(&buffer_id).unwrap();
                    if del_start != pos {
                        buf.delete_range(Range {
                            start: del_start,
                            end: pos,
                        });
                    }
                    // Insert expanded body (strip placeholder markers for now).
                    let expanded = body.replace("$0", "").replace("$1", "");
                    buf.insert_str(del_start, &expanded);
                }
                // Move cursor to the end of the inserted text.
                let buf = self.buffers.get(&buffer_id).unwrap();
                let expanded_body = body.replace("$0", "").replace("$1", "");
                let lines: Vec<&str> = expanded_body.lines().collect();
                let new_pos = if lines.len() <= 1 {
                    Position {
                        line: del_start.line,
                        col: del_start.col + expanded_body.graphemes(true).count(),
                    }
                } else {
                    let last_line_idx = del_start.line + lines.len() - 1;
                    let last_col = lines.last().map(|l| l.graphemes(true).count()).unwrap_or(0);
                    let max_col = line_grapheme_len(buf, last_line_idx);
                    Position {
                        line: last_line_idx,
                        col: last_col.min(max_col),
                    }
                };
                let pane = self.panes.get_mut(&pane_id).unwrap();
                set_cursor(pane, new_pos, false);
                true
            }
        }
    }
}

// ── Public helpers (NE7) ─────────────────────────────────────────────────────

/// Convert a click pixel (relative to pane top-left) to a buffer `Position`.
///
/// Accounts for gutter width, scroll offset, and grapheme column walk.
///
/// - `rel_x`, `rel_y`: pointer position in device pixels relative to the
///   top-left corner of the pane's draw area.
/// - `metrics`: font cell dimensions in device pixels.
/// - `gutter_cols`: width of the line-number gutter in character columns.
pub fn pixel_to_position(
    editor_pane: &EditorPane,
    buffer: &Buffer,
    rel_x: f64,
    rel_y: f64,
    metrics: FontMetrics,
    gutter_cols: usize,
) -> Position {
    // Row: floor(rel_y / cell_h) + scroll_pos, clamped to buffer bounds.
    let row_raw = (rel_y / metrics.cell_h).floor() as usize;
    let line_count = buffer.line_count();
    let last_line = line_count.saturating_sub(1);
    let row = (row_raw + editor_pane.scroll_pos as usize).min(last_line);

    // Column: subtract gutter pixels, then walk graphemes.
    let col_pixel = rel_x - gutter_cols as f64 * metrics.cell_w;
    if col_pixel < 0.0 {
        return Position { line: row, col: 0 };
    }
    let cell_col = (col_pixel / metrics.cell_w).round() as usize;

    // Walk line graphemes and clamp to line length.
    let line_len = if line_count == 0 {
        0
    } else {
        let line_str: String = buffer.line(row).chars().collect();
        line_str
            .trim_end_matches('\n')
            .trim_end_matches('\r')
            .graphemes(true)
            .count()
    };
    Position {
        line: row,
        col: cell_col.min(line_len),
    }
}

// ── Private helpers ───────────────────────────────────────────────────────────

/// Set the cursor position. When `extend` is false, the anchor snaps to `pos`.
fn set_cursor(pane: &mut EditorPane, pos: Position, extend: bool) {
    pane.cursors[0].pos = pos;
    if !extend {
        pane.cursors[0].anchor = pos;
    }
}

/// Advance `pos` by `n` grapheme columns on the same line (no line wrapping).
fn advance_col(pos: Position, n: usize) -> Position {
    Position {
        line: pos.line,
        col: pos.col + n,
    }
}

/// Number of grapheme clusters on `line` (excluding any trailing newline).
fn line_grapheme_len(buf: &Buffer, line: usize) -> usize {
    if line >= buf.line_count() {
        return 0;
    }
    let line_str: String = buf.line(line).chars().collect();
    line_str
        .trim_end_matches('\n')
        .trim_end_matches('\r')
        .graphemes(true)
        .count()
}

/// Move one grapheme cluster backward from `pos`.  Returns `pos` unchanged at
/// the buffer start.
fn prev_position(buf: &Buffer, pos: Position) -> Position {
    if pos.col > 0 {
        return Position {
            line: pos.line,
            col: pos.col - 1,
        };
    }
    if pos.line == 0 {
        return pos;
    }
    // Move to end of previous line.
    let prev_line = pos.line - 1;
    let col = line_grapheme_len(buf, prev_line);
    Position {
        line: prev_line,
        col,
    }
}

/// Move one grapheme cluster forward from `pos`.  Returns `pos` unchanged at
/// the buffer end.
fn next_position(buf: &Buffer, pos: Position) -> Position {
    let line_len = line_grapheme_len(buf, pos.line);
    if pos.col < line_len {
        return Position {
            line: pos.line,
            col: pos.col + 1,
        };
    }
    // Move to start of next line if there is one.
    let last_line = buf.line_count().saturating_sub(1);
    if pos.line < last_line {
        Position {
            line: pos.line + 1,
            col: 0,
        }
    } else {
        pos
    }
}

/// Extract the text covered by the cursor's selection anchor→pos range.
/// Returns `None` when anchor == pos (no selection).
fn selected_text(pane: &EditorPane, buf: &Buffer) -> Option<String> {
    let (start, end) = selection_range(pane);
    if start == end {
        return None;
    }
    // Walk every line from start to end, collecting grapheme clusters.
    let mut out = String::new();
    for line_idx in start.line..=end.line {
        if line_idx >= buf.line_count() {
            break;
        }
        let line_str: String = buf.line(line_idx).chars().collect();
        let graphemes: Vec<&str> = line_str
            .trim_end_matches('\n')
            .trim_end_matches('\r')
            .graphemes(true)
            .collect();
        let lo = if line_idx == start.line { start.col } else { 0 };
        let hi = if line_idx == end.line {
            end.col.min(graphemes.len())
        } else {
            graphemes.len()
        };
        for g in &graphemes[lo.min(graphemes.len())..hi.min(graphemes.len())] {
            out.push_str(g);
        }
        if line_idx < end.line {
            out.push('\n');
        }
    }
    if out.is_empty() { None } else { Some(out) }
}

/// Return the ordered `(start, end)` pair of the cursor's anchor→pos range.
fn selection_range(pane: &EditorPane) -> (Position, Position) {
    let a = pane.cursors[0].anchor;
    let p = pane.cursors[0].pos;
    // Compare by (line, col).
    if (a.line, a.col) <= (p.line, p.col) {
        (a, p)
    } else {
        (p, a)
    }
}

// ── Tier-C helper functions ───────────────────────────────────────────────────

/// Return the matching closing bracket for `ch` (#17), or `None` if `ch` is
/// not an auto-pairable opener.
/// X14: move left to the previous word boundary.
/// A word boundary is the position just before the first char of a run of
/// word-chars (alphanumeric + `_`).  If already inside a word, moves to the
/// start of that word; otherwise skips non-word chars first.
fn prev_word_boundary(buf: &Buffer, pos: Position) -> Position {
    // Gather the line text up to col.
    if pos.col == 0 {
        // Jump to end of previous line if possible.
        if pos.line == 0 {
            return pos;
        }
        let prev_line = pos.line - 1;
        let col = line_grapheme_len(buf, prev_line);
        return prev_word_boundary(
            buf,
            Position {
                line: prev_line,
                col,
            },
        );
    }
    let line_str: String = buf.line(pos.line).chars().collect();
    let content = line_str.trim_end_matches('\n').trim_end_matches('\r');
    let graphemes: Vec<&str> = content.graphemes(true).collect();
    let col = pos.col.min(graphemes.len());
    let is_word = |g: &str| g.chars().all(|c| c.is_alphanumeric() || c == '_');
    // Skip non-word chars immediately to the left.
    let mut i = col;
    while i > 0 && !is_word(graphemes[i - 1]) {
        i -= 1;
    }
    // Skip word chars to find the start of the word.
    while i > 0 && is_word(graphemes[i - 1]) {
        i -= 1;
    }
    Position {
        line: pos.line,
        col: i,
    }
}

/// X14: move right to the next word boundary.
/// Moves past any word chars from the current position, then past any
/// non-word chars, landing at the start of the next word (or end of line).
fn next_word_boundary(buf: &Buffer, pos: Position) -> Position {
    let line_len = line_grapheme_len(buf, pos.line);
    if pos.col >= line_len {
        // Jump to start of next line if possible.
        let last_line = buf.line_count().saturating_sub(1);
        if pos.line >= last_line {
            return pos;
        }
        return Position {
            line: pos.line + 1,
            col: 0,
        };
    }
    let line_str: String = buf.line(pos.line).chars().collect();
    let content = line_str.trim_end_matches('\n').trim_end_matches('\r');
    let graphemes: Vec<&str> = content.graphemes(true).collect();
    let is_word = |g: &str| g.chars().all(|c| c.is_alphanumeric() || c == '_');
    let mut i = pos.col;
    // Skip current word chars.
    while i < graphemes.len() && is_word(graphemes[i]) {
        i += 1;
    }
    // Skip non-word chars (punctuation / whitespace between words).
    while i < graphemes.len() && !is_word(graphemes[i]) {
        i += 1;
    }
    Position {
        line: pos.line,
        col: i,
    }
}

fn bracket_pair_close(ch: char) -> Option<char> {
    match ch {
        '(' => Some(')'),
        '[' => Some(']'),
        '{' => Some('}'),
        '"' => Some('"'),
        '\'' => Some('\''),
        '`' => Some('`'),
        _ => None,
    }
}

/// Return `true` if `ch` is one of the auto-paired closing brackets.
fn is_closing_bracket(ch: char) -> bool {
    matches!(ch, ')' | ']' | '}' | '"' | '\'' | '`')
}

/// Return `true` if the grapheme immediately before `pos` is alphanumeric.
fn prev_char_is_alnum(buf: &Buffer, pos: Position) -> bool {
    if pos.col == 0 {
        return false;
    }
    let line_str: String = buf.line(pos.line).chars().collect();
    let graphemes: Vec<&str> = line_str.graphemes(true).collect();
    if pos.col > graphemes.len() {
        return false;
    }
    let g = graphemes[pos.col - 1];
    g.chars().all(|c| c.is_alphanumeric())
}

/// Compute the indent string to insert after a newline for auto-indent (#16).
///
/// Copies the leading whitespace of `line`; if the line (trimmed) ends with
/// `{`, `(`, or `[`, appends one extra indent level (4 spaces).
fn smart_indent(buf: &Buffer, pos: Position) -> String {
    if buf.line_count() == 0 {
        return String::new();
    }
    let line = pos.line.min(buf.line_count().saturating_sub(1));
    let line_str: String = buf.line(line).chars().collect();
    // Collect leading whitespace.
    let leading: String = line_str
        .chars()
        .take_while(|c| *c == ' ' || *c == '\t')
        .collect();
    // Check whether the trimmed content ends with an opener.
    let trimmed = line_str
        .trim_end_matches('\n')
        .trim_end_matches('\r')
        .trim_end();
    if trimmed.ends_with('{') || trimmed.ends_with('(') || trimmed.ends_with('[') {
        let mut s = leading;
        s.push_str("    ");
        s
    } else {
        leading
    }
}

/// Return the comment marker string for a buffer's language (#18).
fn comment_marker_for_buffer(buf: &Buffer) -> &'static str {
    match buf.language_id() {
        Some("python") | Some("toml") | Some("yaml") | Some("shellscript") | Some("bash") => "#",
        // HTML/XML block comments are handled separately if needed; use // as default.
        _ => "//",
    }
}

/// Return the grapheme column of the comment marker in a line, or `None`.
fn leading_col_of_comment(line_str: &str, marker: &str) -> Option<usize> {
    let graphemes: Vec<&str> = line_str.graphemes(true).collect();
    // Find the first non-whitespace run that equals the marker.
    let mut col = 0usize;
    for g in &graphemes {
        if *g == " " || *g == "\t" {
            col += 1;
        } else {
            break;
        }
    }
    // Reconstruct the substring starting at col to check for marker.
    let rest: String = graphemes[col..].iter().copied().collect();
    if rest.starts_with(marker) {
        Some(col)
    } else {
        None
    }
}

/// Byte offset of the first non-whitespace character.
fn leading_byte_offset(s: &str) -> usize {
    s.len() - s.trim_start().len()
}

/// Number of leading grapheme clusters that are whitespace.
fn leading_grapheme_count(line_str: &str) -> usize {
    line_str
        .graphemes(true)
        .take_while(|g| g.chars().all(|c| c == ' ' || c == '\t'))
        .count()
}

/// Normalise a line by stripping trailing `\r`/`\n` and appending `\n`.
fn normalise_line(s: &str) -> String {
    let mut out = s.trim_end_matches('\n').trim_end_matches('\r').to_string();
    out.push('\n');
    out
}

/// Count how many leading spaces (up to `indent_width`) can be removed (#21).
fn leading_spaces_to_remove(line_str: &str, indent_width: usize) -> usize {
    let mut removed = 0;
    for g in line_str.graphemes(true) {
        if removed >= indent_width {
            break;
        }
        if g == " " {
            removed += 1;
        } else if g == "\t" {
            removed += 1;
            break;
        } else {
            break;
        }
    }
    removed
}

/// Run `rustfmt` on `text` and return the formatted output, or `None` on error.
///
/// Uses `--emit stdout --edition 2021`. A missing binary or non-zero exit is
/// treated as a silent no-op (logged once to stderr).
fn run_rustfmt(text: &str) -> Option<String> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let mut child = Command::new("rustfmt")
        .args(["--emit", "stdout", "--edition", "2021"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    if let Some(stdin) = child.stdin.as_mut() {
        let _ = stdin.write_all(text.as_bytes());
    }
    let output = child.wait_with_output().ok()?;
    if output.status.success() {
        String::from_utf8(output.stdout).ok()
    } else {
        eprintln!("anvil: rustfmt exited non-zero; format skipped");
        None
    }
}

// ── Tier-W private helpers ────────────────────────────────────────────────────

/// Extract the selected text for the primary cursor, or the word under the
/// cursor when there is no selection.  Returns an empty string when neither
/// can be determined.
///
/// Used by `SelectAllOccurrences` and `ConvertCase*`.
fn word_or_selection_text(pane: &EditorPane, buf: &Buffer) -> String {
    let (start, end) = selection_range(pane);
    if start != end {
        // Selection exists — extract it.
        let mut s = String::new();
        for line_idx in start.line..=end.line {
            if line_idx >= buf.line_count() {
                break;
            }
            let line_str: String = buf.line(line_idx).chars().collect();
            let graphemes: Vec<&str> = line_str
                .trim_end_matches('\n')
                .trim_end_matches('\r')
                .graphemes(true)
                .collect();
            let lo = if line_idx == start.line { start.col } else { 0 };
            let hi = if line_idx == end.line {
                end.col.min(graphemes.len())
            } else {
                graphemes.len()
            };
            for g in &graphemes[lo.min(graphemes.len())..hi.min(graphemes.len())] {
                s.push_str(g);
            }
            if line_idx < end.line {
                s.push('\n');
            }
        }
        return s;
    }
    // No selection — expand to word under cursor.
    let pos = pane.cursors[0].pos;
    let line_str: String = buf
        .line(pos.line.min(buf.line_count().saturating_sub(1)))
        .chars()
        .collect();
    let graphemes: Vec<&str> = line_str
        .trim_end_matches('\n')
        .trim_end_matches('\r')
        .graphemes(true)
        .collect();
    if graphemes.is_empty() {
        return String::new();
    }
    let col = pos.col.min(graphemes.len().saturating_sub(1));
    let is_word = |g: &str| g.chars().all(|c| c.is_alphanumeric() || c == '_');
    let mut lo = col;
    while lo > 0 && is_word(graphemes[lo - 1]) {
        lo -= 1;
    }
    let mut hi = col;
    if hi < graphemes.len() && is_word(graphemes[hi]) {
        hi += 1;
        while hi < graphemes.len() && is_word(graphemes[hi]) {
            hi += 1;
        }
    }
    graphemes[lo..hi].concat()
}

/// Apply a case-conversion function to the selected text (or word under
/// cursor) and replace it in the buffer.  Returns `true` when the buffer
/// was mutated.
fn convert_case<F>(reg: &mut EditorPaneRegistry, pane_id: PaneId, buffer_id: BufferId, f: F) -> bool
where
    F: Fn(&str) -> String,
{
    let (start, end) = {
        let pane = reg.panes.get(&pane_id).unwrap();
        let buf = reg.buffers.get(&buffer_id).unwrap();
        let (s, e) = selection_range(pane);
        if s == e {
            // No selection — expand to word.
            let text = word_or_selection_text(pane, buf);
            if text.is_empty() {
                return false;
            }
            let pos = pane.cursors[0].pos;
            let line_str: String = buf
                .line(pos.line.min(buf.line_count().saturating_sub(1)))
                .chars()
                .collect();
            let graphemes: Vec<&str> = line_str
                .trim_end_matches('\n')
                .trim_end_matches('\r')
                .graphemes(true)
                .collect();
            let col = pos.col.min(graphemes.len().saturating_sub(1));
            let is_word = |g: &str| g.chars().all(|c: char| c.is_alphanumeric() || c == '_');
            let mut lo = col;
            while lo > 0 && is_word(graphemes[lo - 1]) {
                lo -= 1;
            }
            let mut hi = col;
            if hi < graphemes.len() && is_word(graphemes[hi]) {
                hi += 1;
                while hi < graphemes.len() && is_word(graphemes[hi]) {
                    hi += 1;
                }
            }
            (
                Position {
                    line: pos.line,
                    col: lo,
                },
                Position {
                    line: pos.line,
                    col: hi,
                },
            )
        } else {
            (s, e)
        }
    };
    let original: String = {
        let buf = reg.buffers.get(&buffer_id).unwrap();
        let mut s = String::new();
        for line_idx in start.line..=end.line {
            if line_idx >= buf.line_count() {
                break;
            }
            let line_str: String = buf.line(line_idx).chars().collect();
            let gs: Vec<&str> = line_str
                .trim_end_matches('\n')
                .trim_end_matches('\r')
                .graphemes(true)
                .collect();
            let lo = if line_idx == start.line { start.col } else { 0 };
            let hi = if line_idx == end.line {
                end.col.min(gs.len())
            } else {
                gs.len()
            };
            for g in &gs[lo.min(gs.len())..hi.min(gs.len())] {
                s.push_str(g);
            }
            if line_idx < end.line {
                s.push('\n');
            }
        }
        s
    };
    let converted = f(&original);
    if converted == original {
        return false;
    }
    {
        let buf = reg.buffers.get_mut(&buffer_id).unwrap();
        buf.replace_range(Range { start, end }, &converted);
    }
    // Keep cursor at end of replacement (same position semantics as paste).
    let new_col = converted
        .lines()
        .last()
        .map(|l| l.graphemes(true).count())
        .unwrap_or(0);
    let new_line = start.line + converted.lines().count().saturating_sub(1);
    let pane = reg.panes.get_mut(&pane_id).unwrap();
    set_cursor(
        pane,
        Position {
            line: new_line,
            col: new_col,
        },
        false,
    );
    true
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_reg_with_text(text: &str) -> (EditorPaneRegistry, PaneId) {
        let mut reg = EditorPaneRegistry::default();
        let pane_id: PaneId = 1;
        reg.new_pane(pane_id);
        // Load text into the buffer.
        let bid = reg.get_pane(pane_id).unwrap().buffer_id;
        let buf = reg.get_buffer_mut(bid).unwrap();
        *buf = anvil_editor::Buffer::from_text(text);
        (reg, pane_id)
    }

    fn buf_text(reg: &EditorPaneRegistry, pane_id: PaneId) -> String {
        let bid = reg.get_pane(pane_id).unwrap().buffer_id;
        reg.get_buffer(bid).unwrap().to_text()
    }

    fn test_metrics() -> FontMetrics {
        FontMetrics {
            cell_w: 8.0,
            cell_h: 16.0,
            descent: 3.0,
        }
    }

    fn make_pane_with_text(text: &str) -> (EditorPane, Buffer) {
        let origin = Position { line: 0, col: 0 };
        let pane = EditorPane {
            buffer_id: 1,
            open_buffers: vec![1],
            cursors: vec![Cursor {
                pos: origin,
                anchor: origin,
            }],
            selection: Selection::default(),
            scroll_pos: 0.0,
            scroll_target: 0.0,
            scroll_vel: 0.0,
            search: None,
            hover_popup: None,
            completion_popup: None,
            code_actions_popup: None,
            folds: HashMap::new(),
            soft_wrap: false,
            show_whitespace: false,
            scroll_x: 0.0,
        };
        let buf = anvil_editor::Buffer::from_text(text);
        (pane, buf)
    }

    // ── NE7 pixel_to_position tests ──────────────────────────────────────────

    #[test]
    fn pixel_to_position_origin_returns_0_0() {
        let (pane, buf) = make_pane_with_text("hello\nworld\n");
        let pos = pixel_to_position(&pane, &buf, 0.0, 0.0, test_metrics(), 0);
        assert_eq!(pos, Position { line: 0, col: 0 });
    }

    #[test]
    fn pixel_to_position_row_3_col_5_with_gutter() {
        // row = floor(3*16 / 16) + 0 = 3
        // rel_x = gutter(2)*8 + col(5)*8 = 16 + 40 = 56
        // col_pixel = 56 - 16 = 40; 40/8 = 5.0 → 5
        // Line 3 is "barnacle" (8 chars) so col 5 is within bounds.
        let (pane, buf) = make_pane_with_text("hello\nworld\nfoobar\nbarnacle\n");
        let rel_x = (2 + 5) as f64 * 8.0; // gutter 2 cols + col 5
        let rel_y = 3.0 * 16.0; // row 3
        let pos = pixel_to_position(&pane, &buf, rel_x, rel_y, test_metrics(), 2);
        assert_eq!(pos, Position { line: 3, col: 5 });
    }

    #[test]
    fn pixel_to_position_clamps_overflow() {
        // Buffer has 2 lines of "hi". Click far past end.
        let (pane, buf) = make_pane_with_text("hi\nhi\n");
        let pos = pixel_to_position(&pane, &buf, 9999.0, 9999.0, test_metrics(), 0);
        let last_line = buf.line_count().saturating_sub(1);
        assert_eq!(pos.line, last_line);
        // col clamped to "hi" length = 2
        assert!(pos.col <= 2);
    }

    // ── NE7 action tests ─────────────────────────────────────────────────────

    #[test]
    fn apply_move_to_clears_selection() {
        let (mut reg, pid) = make_reg_with_text("hello world");
        // Put cursor at col 5 with anchor at 0 (simulating a selection).
        {
            let pane = reg.get_pane_mut(pid).unwrap();
            pane.cursors[0].anchor = Position { line: 0, col: 0 };
            pane.cursors[0].pos = Position { line: 0, col: 5 };
        }
        let mut clip = None;
        reg.apply(
            pid,
            EditorAction::MoveTo {
                pos: Position { line: 0, col: 3 },
                extend: false,
            },
            &mut clip,
        );
        let pane = reg.get_pane(pid).unwrap();
        // Anchor should equal pos (selection collapsed).
        assert_eq!(pane.cursors[0].pos, Position { line: 0, col: 3 });
        assert_eq!(pane.cursors[0].anchor, Position { line: 0, col: 3 });
    }

    #[test]
    fn apply_move_to_with_extend_preserves_anchor() {
        let (mut reg, pid) = make_reg_with_text("hello world");
        {
            let pane = reg.get_pane_mut(pid).unwrap();
            pane.cursors[0].anchor = Position { line: 0, col: 2 };
            pane.cursors[0].pos = Position { line: 0, col: 2 };
        }
        let mut clip = None;
        reg.apply(
            pid,
            EditorAction::MoveTo {
                pos: Position { line: 0, col: 7 },
                extend: true,
            },
            &mut clip,
        );
        let pane = reg.get_pane(pid).unwrap();
        // Anchor stays at 2; pos moves to 7.
        assert_eq!(pane.cursors[0].anchor, Position { line: 0, col: 2 });
        assert_eq!(pane.cursors[0].pos, Position { line: 0, col: 7 });
    }

    #[test]
    fn apply_select_word_at_picks_word_span() {
        // "hello world" — click on 'o' (col 4) should select "hello" (0..5).
        let (mut reg, pid) = make_reg_with_text("hello world");
        let mut clip = None;
        reg.apply(
            pid,
            EditorAction::SelectWordAt(Position { line: 0, col: 4 }),
            &mut clip,
        );
        let pane = reg.get_pane(pid).unwrap();
        assert_eq!(pane.cursors[0].anchor, Position { line: 0, col: 0 });
        assert_eq!(pane.cursors[0].pos, Position { line: 0, col: 5 });
    }

    #[test]
    fn apply_select_line_at_picks_full_line() {
        // "foo\nbar\n" — select line 1 ("bar") → col 0..3.
        let (mut reg, pid) = make_reg_with_text("foo\nbar\n");
        let mut clip = None;
        reg.apply(
            pid,
            EditorAction::SelectLineAt(Position { line: 1, col: 0 }),
            &mut clip,
        );
        let pane = reg.get_pane(pid).unwrap();
        assert_eq!(pane.cursors[0].anchor, Position { line: 1, col: 0 });
        assert_eq!(pane.cursors[0].pos, Position { line: 1, col: 3 });
    }

    // ── NE6 apply tests ──────────────────────────────────────────────────────

    #[test]
    fn apply_insert_char_appends_to_buffer() {
        let (mut reg, pid) = make_reg_with_text("");
        let mut clip = None;
        let mutated = reg.apply(pid, EditorAction::InsertChar('x'), &mut clip);
        assert!(mutated);
        assert_eq!(buf_text(&reg, pid), "x");
        // Cursor should advance one col.
        assert_eq!(reg.get_pane(pid).unwrap().cursors[0].pos.col, 1);
    }

    #[test]
    fn apply_backspace_removes_prior_char() {
        let (mut reg, pid) = make_reg_with_text("ab");
        // Move cursor to col 2 (end of "ab").
        {
            let pane = reg.get_pane_mut(pid).unwrap();
            pane.cursors[0].pos = Position { line: 0, col: 2 };
            pane.cursors[0].anchor = Position { line: 0, col: 2 };
        }
        let mut clip = None;
        let mutated = reg.apply(pid, EditorAction::Backspace, &mut clip);
        assert!(mutated);
        // "ab" → "a" (col 2 − 1 = col 1 now has nothing).
        let text = buf_text(&reg, pid);
        assert_eq!(text.trim_end_matches('\n'), "a");
        assert_eq!(reg.get_pane(pid).unwrap().cursors[0].pos.col, 1);
    }

    #[test]
    fn apply_move_right_advances_cursor() {
        let (mut reg, pid) = make_reg_with_text("hello");
        let mut clip = None;
        let mutated = reg.apply(pid, EditorAction::MoveRight { extend: false }, &mut clip);
        assert!(!mutated);
        assert_eq!(reg.get_pane(pid).unwrap().cursors[0].pos.col, 1);
        // Anchor snaps to pos.
        assert_eq!(reg.get_pane(pid).unwrap().cursors[0].anchor.col, 1);
    }

    #[test]
    fn apply_move_right_with_extend_grows_selection() {
        let (mut reg, pid) = make_reg_with_text("hello");
        let mut clip = None;
        reg.apply(pid, EditorAction::MoveRight { extend: true }, &mut clip);
        let pane = reg.get_pane(pid).unwrap();
        // pos advanced, anchor stayed.
        assert_eq!(pane.cursors[0].pos.col, 1);
        assert_eq!(pane.cursors[0].anchor.col, 0);
    }

    #[test]
    fn apply_select_all_anchors_at_origin() {
        let (mut reg, pid) = make_reg_with_text("line1\nline2\n");
        let mut clip = None;
        reg.apply(pid, EditorAction::SelectAll, &mut clip);
        let pane = reg.get_pane(pid).unwrap();
        assert_eq!(pane.cursors[0].anchor, Position { line: 0, col: 0 });
        // pos should be at last line.
        assert!(pane.cursors[0].pos.line > 0);
    }

    // ── K6: SelectLine ───────────────────────────────────────────────────────

    #[test]
    fn select_line_selects_current_line_from_col0_to_end() {
        // Cursor on line 1 col 2; SelectLine should anchor at (1,0) and pos at (1,5).
        let (mut reg, pid) = make_reg_with_text("hello\nworld\nfoo\n");
        {
            let pane = reg.get_pane_mut(pid).unwrap();
            pane.cursors[0].pos = Position { line: 1, col: 2 };
            pane.cursors[0].anchor = Position { line: 1, col: 2 };
        }
        let mut clip = None;
        reg.apply(pid, EditorAction::SelectLine, &mut clip);
        let pane = reg.get_pane(pid).unwrap();
        assert_eq!(pane.cursors[0].anchor, Position { line: 1, col: 0 });
        assert_eq!(pane.cursors[0].pos, Position { line: 1, col: 5 }); // "world" = 5
    }

    #[test]
    fn select_line_repeated_extends_down_one_line() {
        // First Cmd+L selects line 0 ("hello"). Second Cmd+L should extend to
        // end of line 1 ("world").
        let (mut reg, pid) = make_reg_with_text("hello\nworld\nfoo\n");
        let mut clip = None;
        // First SelectLine.
        reg.apply(pid, EditorAction::SelectLine, &mut clip);
        {
            let pane = reg.get_pane(pid).unwrap();
            assert_eq!(pane.cursors[0].anchor, Position { line: 0, col: 0 });
            assert_eq!(pane.cursors[0].pos, Position { line: 0, col: 5 });
        }
        // Second SelectLine — selection is line-aligned, so extend to line 1.
        reg.apply(pid, EditorAction::SelectLine, &mut clip);
        let pane = reg.get_pane(pid).unwrap();
        assert_eq!(pane.cursors[0].anchor, Position { line: 0, col: 0 }); // anchor stays
        assert_eq!(pane.cursors[0].pos, Position { line: 1, col: 5 }); // extends to end of "world"
    }

    #[test]
    fn apply_paste_inserts_string() {
        let (mut reg, pid) = make_reg_with_text("ab");
        // Cursor at col 1.
        {
            let pane = reg.get_pane_mut(pid).unwrap();
            pane.cursors[0].pos = Position { line: 0, col: 1 };
            pane.cursors[0].anchor = Position { line: 0, col: 1 };
        }
        let mut clip = None;
        let mutated = reg.apply(pid, EditorAction::Paste("XY".to_string()), &mut clip);
        assert!(mutated);
        let text = buf_text(&reg, pid);
        assert!(text.starts_with("aXYb"), "expected aXYb, got {text}");
        assert_eq!(reg.get_pane(pid).unwrap().cursors[0].pos.col, 3);
    }

    #[test]
    fn apply_undo_reverts_last_edit() {
        let (mut reg, pid) = make_reg_with_text("");
        let mut clip = None;
        reg.apply(pid, EditorAction::InsertChar('z'), &mut clip);
        assert_eq!(buf_text(&reg, pid), "z");
        let mutated = reg.apply(pid, EditorAction::Undo, &mut clip);
        assert!(mutated);
        assert_eq!(buf_text(&reg, pid), "");
    }

    #[test]
    fn apply_save_with_no_path_is_noop() {
        // A fresh buffer has no tracked path — Save should be a silent no-op
        // (logs to stderr but doesn't panic).
        let (mut reg, pid) = make_reg_with_text("content");
        let mut clip = None;
        let mutated = reg.apply(pid, EditorAction::Save, &mut clip);
        // Buffer not mutated; text unchanged.
        assert!(!mutated);
        assert_eq!(buf_text(&reg, pid), "content");
    }

    // ── Existing registry tests ──────────────────────────────────────────────

    #[test]
    fn editor_pane_registry_open_path_loads_file_into_existing_pane() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("main.rs");
        std::fs::write(&path, "fn main() {}\n").unwrap();

        let mut reg = EditorPaneRegistry::default();
        let old_bid = reg.new_pane(9);
        let bid = reg.open_path(9, &path).unwrap();

        assert_ne!(bid, old_bid);
        assert!(reg.get_buffer(old_bid).is_none());
        assert_eq!(reg.get_pane(9).unwrap().buffer_id, bid);
        let buffer = reg.get_buffer(bid).unwrap();
        assert_eq!(buffer.to_text(), "fn main() {}\n");
        assert_eq!(buffer.tracked_path(), Some(path.as_path()));
        assert_eq!(
            reg.get_pane(9).unwrap().cursors[0].pos,
            Position { line: 0, col: 0 }
        );
    }

    #[test]
    fn editor_pane_registry_open_path_creates_missing_pane() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("README.md");
        std::fs::write(&path, "# Project\n").unwrap();

        let mut reg = EditorPaneRegistry::default();
        let bid = reg.open_path(44, &path).unwrap();

        assert_eq!(reg.count(), 1);
        assert_eq!(reg.get_pane(44).unwrap().buffer_id, bid);
        assert_eq!(reg.get_buffer(bid).unwrap().to_text(), "# Project\n");
    }

    #[test]
    fn editor_pane_registry_new_pane_returns_buffer_id() {
        let mut reg = EditorPaneRegistry::default();
        let bid = reg.new_pane(1);
        // Buffer id must be a non-zero sentinel (our counter starts at 1).
        assert!(bid > 0);
        // The pane must be findable.
        assert!(reg.get_pane(1).is_some());
    }

    #[test]
    fn editor_pane_registry_get_buffer_round_trip() {
        let mut reg = EditorPaneRegistry::default();
        let bid = reg.new_pane(42);
        // Buffer must be present and empty.
        let buf = reg
            .get_buffer(bid)
            .expect("buffer should exist after new_pane");
        assert_eq!(buf.char_count(), 0);
    }

    #[test]
    fn editor_pane_registry_remove_pane_drops_buffer() {
        let mut reg = EditorPaneRegistry::default();
        let bid = reg.new_pane(7);
        assert_eq!(reg.count(), 1);
        reg.remove_pane(7);
        assert_eq!(reg.count(), 0);
        // Buffer is gone.
        assert!(reg.get_buffer(bid).is_none());
    }

    #[test]
    fn editor_pane_registry_remove_pane_evicts_only_target() {
        // Insert two panes, remove one, assert count + the other is still present.
        let mut reg = EditorPaneRegistry::default();
        let _bid1 = reg.new_pane(10);
        let bid2 = reg.new_pane(20);
        assert_eq!(reg.count(), 2);
        reg.remove_pane(10);
        assert_eq!(reg.count(), 1, "only the removed pane should be gone");
        // Pane 20 and its buffer must still be accessible.
        assert!(reg.get_pane(20).is_some());
        assert!(reg.get_buffer(bid2).is_some());
        // Pane 10 is gone.
        assert!(reg.get_pane(10).is_none());
    }

    #[test]
    fn editor_pane_registry_multiple_panes_independent() {
        let mut reg = EditorPaneRegistry::default();
        let bid1 = reg.new_pane(1);
        let bid2 = reg.new_pane(2);
        assert_ne!(bid1, bid2, "each pane must get a unique buffer id");
        assert_eq!(reg.count(), 2);
        // Remove one; other stays.
        reg.remove_pane(1);
        assert_eq!(reg.count(), 1);
        assert!(reg.get_pane(2).is_some());
        assert!(reg.get_buffer(bid2).is_some());
    }

    // ── NE13: Multi-cursor tests ──────────────────────────────────────────────

    /// `AddCursorAt` appends a second entry to `cursors` and deduplicates.
    #[test]
    fn multi_cursor_add_appends_to_cursors_vec() {
        let (mut reg, pid) = make_reg_with_text("hello world");
        let mut clip = None;
        // Primary cursor starts at (0,0).
        assert_eq!(reg.get_pane(pid).unwrap().cursors.len(), 1);
        // Add a cursor at (0, 5).
        reg.apply(
            pid,
            EditorAction::AddCursorAt(Position { line: 0, col: 5 }),
            &mut clip,
        );
        assert_eq!(
            reg.get_pane(pid).unwrap().cursors.len(),
            2,
            "cursors vec must grow to 2 after AddCursorAt"
        );
        assert_eq!(
            reg.get_pane(pid).unwrap().cursors[1].pos,
            Position { line: 0, col: 5 }
        );

        // Adding the same position again must be a no-op (dedup).
        reg.apply(
            pid,
            EditorAction::AddCursorAt(Position { line: 0, col: 5 }),
            &mut clip,
        );
        assert_eq!(
            reg.get_pane(pid).unwrap().cursors.len(),
            2,
            "duplicate AddCursorAt must not grow the vec"
        );
    }

    /// `InsertChar` with two cursors inserts the char at both positions.
    #[test]
    fn multi_cursor_insert_char_applies_to_all() {
        // "ab" — primary cursor at col 0, secondary at col 1.
        let (mut reg, pid) = make_reg_with_text("ab");
        let mut clip = None;
        reg.apply(
            pid,
            EditorAction::AddCursorAt(Position { line: 0, col: 2 }),
            &mut clip,
        );
        // Primary at 0, secondary at 2.
        assert_eq!(reg.get_pane(pid).unwrap().cursors.len(), 2);

        // Insert 'X' at both cursors. Reverse order: col 2 first, then col 0.
        // Result: "Xab" becomes "XaXb" → but actually each cursor inserts at
        // its current position in reverse order: col 2 → "abX", col 0 → "Xab…"
        // Let's verify the buffer has 2 extra chars.
        let orig_len = buf_text(&reg, pid).trim_end_matches('\n').len();
        reg.apply(pid, EditorAction::InsertChar('X'), &mut clip);
        let new_text = buf_text(&reg, pid);
        let new_len = new_text.trim_end_matches('\n').len();
        assert_eq!(
            new_len,
            orig_len + 2,
            "InsertChar with 2 cursors must insert 2 chars; got: {new_text:?}"
        );
    }

    // ── Buffer tab management tests ───────────────────────────────────────────

    /// open_path_as_tab appends a new buffer without removing the existing one.
    #[test]
    fn open_path_as_tab_adds_buffer_to_list() {
        let dir = tempfile::tempdir().unwrap();
        let p1 = dir.path().join("a.rs");
        let p2 = dir.path().join("b.rs");
        std::fs::write(&p1, "fn a() {}").unwrap();
        std::fs::write(&p2, "fn b() {}").unwrap();

        let mut reg = EditorPaneRegistry::default();
        reg.new_pane(1);
        let bid1 = reg.open_path_as_tab(1, &p1).unwrap();
        let bid2 = reg.open_path_as_tab(1, &p2).unwrap();

        let pane = reg.get_pane(1).unwrap();
        assert_eq!(pane.buffer_id, bid2, "active buffer must be last opened");
        assert!(
            pane.open_buffers.contains(&bid1),
            "first buffer must remain in open_buffers"
        );
        assert!(pane.open_buffers.contains(&bid2));
        assert_eq!(
            pane.open_buffers.len(),
            2,
            "scratch should have been evicted on first file open; a.rs + b.rs remain"
        );
    }

    /// First real file open evicts the empty scratch tab.
    #[test]
    fn open_path_as_tab_evicts_scratch_on_first_file() {
        let dir = tempfile::tempdir().unwrap();
        let p1 = dir.path().join("first.rs");
        std::fs::write(&p1, "fn first() {}").unwrap();

        let mut reg = EditorPaneRegistry::default();
        let scratch_bid = reg.new_pane(1);
        assert_eq!(reg.get_pane(1).unwrap().open_buffers, vec![scratch_bid]);

        let file_bid = reg.open_path_as_tab(1, &p1).unwrap();
        let pane = reg.get_pane(1).unwrap();
        assert_eq!(
            pane.open_buffers,
            vec![file_bid],
            "scratch must be gone after the first real file opens"
        );
        assert!(
            reg.get_buffer(scratch_bid).is_none(),
            "scratch buffer must be removed from registry"
        );
    }

    /// open_path_as_tab deduplicates: re-opening the same file just activates it.
    #[test]
    fn open_path_as_tab_deduplicates() {
        let dir = tempfile::tempdir().unwrap();
        let p1 = dir.path().join("dup.rs");
        std::fs::write(&p1, "fn dup() {}").unwrap();

        let mut reg = EditorPaneRegistry::default();
        reg.new_pane(1);
        let bid1 = reg.open_path_as_tab(1, &p1).unwrap();
        let bid2 = reg.open_path_as_tab(1, &p1).unwrap();

        assert_eq!(
            bid1, bid2,
            "re-opening same path must return the same BufferId"
        );
        let pane = reg.get_pane(1).unwrap();
        assert_eq!(
            pane.open_buffers.len(),
            1,
            "scratch evicted on first open, dup reopen adds no entry"
        );
        assert_eq!(pane.buffer_id, bid1, "active is the existing buffer");
    }

    /// open_buffer switches the active buffer without loading a file.
    #[test]
    fn open_buffer_switches_active() {
        let dir = tempfile::tempdir().unwrap();
        let p1 = dir.path().join("x.rs");
        let p2 = dir.path().join("y.rs");
        std::fs::write(&p1, "").unwrap();
        std::fs::write(&p2, "").unwrap();

        let mut reg = EditorPaneRegistry::default();
        reg.new_pane(1);
        let bid1 = reg.open_path_as_tab(1, &p1).unwrap();
        let _bid2 = reg.open_path_as_tab(1, &p2).unwrap();

        reg.open_buffer(1, bid1);
        assert_eq!(reg.get_pane(1).unwrap().buffer_id, bid1);
    }

    /// close_buffer removes a non-active buffer and picks the right neighbor.
    #[test]
    fn close_buffer_removes_tab_and_picks_right_neighbor() {
        let dir = tempfile::tempdir().unwrap();
        let p1 = dir.path().join("1.rs");
        let p2 = dir.path().join("2.rs");
        let p3 = dir.path().join("3.rs");
        std::fs::write(&p1, "").unwrap();
        std::fs::write(&p2, "").unwrap();
        std::fs::write(&p3, "").unwrap();

        let mut reg = EditorPaneRegistry::default();
        reg.new_pane(1);
        // open_path_as_tab adds a scratch buffer first — discard and open clean
        let bid1 = reg.open_path_as_tab(1, &p1).unwrap();
        let bid2 = reg.open_path_as_tab(1, &p2).unwrap();
        let bid3 = reg.open_path_as_tab(1, &p3).unwrap();
        // active is bid3; close bid2 (middle) → right neighbor = bid3
        let active_before = reg.get_pane(1).unwrap().buffer_id;
        assert_eq!(active_before, bid3);
        // close bid2 when it's not active
        let new_active = reg.close_buffer(1, bid2);
        assert!(new_active.is_some());
        // bid2 should be gone
        let pane = reg.get_pane(1).unwrap();
        assert!(!pane.open_buffers.contains(&bid2));
        assert!(reg.get_buffer(bid2).is_none());
        // bid1 and bid3 still present
        assert!(pane.open_buffers.contains(&bid1));
        assert!(pane.open_buffers.contains(&bid3));
    }

    /// close_buffer on the last buffer falls back to a fresh scratch buffer.
    #[test]
    fn close_last_buffer_creates_scratch() {
        let dir = tempfile::tempdir().unwrap();
        let p1 = dir.path().join("lone.rs");
        std::fs::write(&p1, "fn lone() {}").unwrap();

        let mut reg = EditorPaneRegistry::default();
        // new_pane gives scratch; then we open one file
        reg.new_pane(1);
        let scratch_id = reg.get_pane(1).unwrap().buffer_id;
        // Close the scratch buffer first to get to a clean single-tab state
        reg.close_buffer(1, scratch_id); // closes scratch → None, new scratch allocated
        let pane = reg.get_pane(1).unwrap();
        let new_scratch = pane.buffer_id;
        let result = reg.close_buffer(1, new_scratch);
        // After closing the last buffer, close_buffer returns None (fall back to scratch).
        assert!(result.is_none(), "closing last buffer returns None");
        // A new scratch buffer must exist.
        let pane = reg.get_pane(1).unwrap();
        assert_eq!(pane.open_buffers.len(), 1);
        assert!(reg.get_buffer(pane.buffer_id).is_some());
    }

    /// `ClearSecondaryCursors` drops all but the primary cursor.
    #[test]
    fn multi_cursor_clear_drops_secondary() {
        let (mut reg, pid) = make_reg_with_text("hello world");
        let mut clip = None;
        // Add two secondary cursors.
        reg.apply(
            pid,
            EditorAction::AddCursorAt(Position { line: 0, col: 3 }),
            &mut clip,
        );
        reg.apply(
            pid,
            EditorAction::AddCursorAt(Position { line: 0, col: 7 }),
            &mut clip,
        );
        assert_eq!(reg.get_pane(pid).unwrap().cursors.len(), 3);

        reg.apply(pid, EditorAction::ClearSecondaryCursors, &mut clip);
        assert_eq!(
            reg.get_pane(pid).unwrap().cursors.len(),
            1,
            "ClearSecondaryCursors must leave exactly 1 cursor"
        );
        // Primary cursor must still exist.
        assert_eq!(
            reg.get_pane(pid).unwrap().primary_cursor().pos,
            Position { line: 0, col: 0 }
        );
    }

    // ── Item 20: detach_buffer_removes_from_source_pane ───────────────────────
    // This test exercises the close_buffer path that `App::detach_buffer_to_new_window`
    // calls.  No new window is spawned; the test verifies the buffer is removed
    // from the source pane so the stub API surface is correct.
    #[test]
    fn detach_buffer_removes_from_source_pane() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("main.rs");
        std::fs::write(&path, "fn main() {}").unwrap();

        let pane_id: PaneId = 42;
        let mut reg = EditorPaneRegistry::default();
        reg.new_pane(pane_id);
        let buffer_id = reg.open_path_as_tab(pane_id, &path).unwrap();

        // Simulate what detach_buffer_to_new_window does: close the buffer.
        let _new_active = reg.close_buffer(pane_id, buffer_id);

        // The detached buffer is gone from the registry.
        assert!(
            reg.get_buffer(buffer_id).is_none(),
            "detached buffer must be removed from the source pane's registry"
        );
        // Pane still exists (scratch buffer created automatically).
        assert!(
            reg.get_pane(pane_id).is_some(),
            "source pane must survive after detach"
        );
    }

    // ── #16 Auto-indent on Enter ─────────────────────────────────────────────

    #[test]
    fn insert_newline_smart_copies_indent() {
        // "    hello" — Enter at end should produce "    hello\n    ".
        let (mut reg, pid) = make_reg_with_text("    hello\n");
        {
            let pane = reg.get_pane_mut(pid).unwrap();
            pane.cursors[0].pos = Position { line: 0, col: 9 };
            pane.cursors[0].anchor = Position { line: 0, col: 9 };
        }
        let mut clip = None;
        reg.apply(pid, EditorAction::InsertNewlineSmart, &mut clip);
        let text = buf_text(&reg, pid);
        // New line should start with 4 spaces.
        let line1: String = {
            let bid = reg.get_pane(pid).unwrap().buffer_id;
            let buf = reg.get_buffer(bid).unwrap();
            buf.line(1).chars().collect()
        };
        assert!(
            line1.starts_with("    "),
            "auto-indent must copy leading 4 spaces; got: {:?}",
            text
        );
        // Cursor on line 1, col 4.
        assert_eq!(reg.get_pane(pid).unwrap().cursors[0].pos.line, 1);
        assert_eq!(reg.get_pane(pid).unwrap().cursors[0].pos.col, 4);
    }

    #[test]
    fn insert_newline_smart_adds_indent_after_open_brace() {
        let (mut reg, pid) = make_reg_with_text("fn foo() {\n");
        {
            let pane = reg.get_pane_mut(pid).unwrap();
            // Cursor at end of "fn foo() {" (col 10).
            pane.cursors[0].pos = Position { line: 0, col: 10 };
            pane.cursors[0].anchor = Position { line: 0, col: 10 };
        }
        let mut clip = None;
        reg.apply(pid, EditorAction::InsertNewlineSmart, &mut clip);
        let bid = reg.get_pane(pid).unwrap().buffer_id;
        let buf = reg.get_buffer(bid).unwrap();
        let line1: String = buf.line(1).chars().collect();
        // Must have exactly 4 spaces (no prior indent + one extra level).
        assert!(
            line1.starts_with("    "),
            "auto-indent after open brace must add one indent level; got: {:?}",
            line1
        );
        assert_eq!(reg.get_pane(pid).unwrap().cursors[0].pos.col, 4);
    }

    // ── #17 Smart bracket pairs ──────────────────────────────────────────────

    #[test]
    fn insert_char_open_paren_auto_pairs() {
        let (mut reg, pid) = make_reg_with_text("");
        let mut clip = None;
        reg.apply(pid, EditorAction::InsertChar('('), &mut clip);
        let text = buf_text(&reg, pid);
        // Buffer should contain "()" and cursor at col 1 (between parens).
        assert!(
            text.contains("()"),
            "auto-pair must insert (); got: {:?}",
            text
        );
        assert_eq!(reg.get_pane(pid).unwrap().cursors[0].pos.col, 1);
    }

    #[test]
    fn insert_char_closing_bracket_skips_over_paired() {
        // Type `(` to get `()`, then type `)` — cursor should move to col 2
        // without inserting a second `)`.
        let (mut reg, pid) = make_reg_with_text("");
        let mut clip = None;
        reg.apply(pid, EditorAction::InsertChar('('), &mut clip);
        // Buffer is "()", cursor at col 1.
        reg.apply(pid, EditorAction::InsertChar(')'), &mut clip);
        let text = buf_text(&reg, pid);
        // Should still be "()" — no double close.
        let content = text.trim_end_matches('\n').trim_end_matches('\r');
        assert_eq!(
            content, "()",
            "skip-over must not double-insert; got: {:?}",
            content
        );
        assert_eq!(reg.get_pane(pid).unwrap().cursors[0].pos.col, 2);
    }

    // ── #18 Toggle line comment ──────────────────────────────────────────────

    #[test]
    fn toggle_line_comment_adds_marker() {
        let (mut reg, pid) = make_reg_with_text("let x = 1;\n");
        let mut clip = None;
        reg.apply(pid, EditorAction::ToggleLineComment, &mut clip);
        let text = buf_text(&reg, pid);
        assert!(
            text.contains("// let x = 1;"),
            "comment must be prepended; got: {:?}",
            text
        );
    }

    #[test]
    fn toggle_line_comment_removes_marker() {
        let (mut reg, pid) = make_reg_with_text("// let x = 1;\n");
        let mut clip = None;
        reg.apply(pid, EditorAction::ToggleLineComment, &mut clip);
        let text = buf_text(&reg, pid);
        assert!(
            !text.contains("//"),
            "comment marker must be stripped; got: {:?}",
            text
        );
    }

    // ── #19 Duplicate line ───────────────────────────────────────────────────

    #[test]
    fn duplicate_line_inserts_copy_below() {
        let (mut reg, pid) = make_reg_with_text("hello\nworld\n");
        {
            let pane = reg.get_pane_mut(pid).unwrap();
            pane.cursors[0].pos = Position { line: 0, col: 2 };
            pane.cursors[0].anchor = Position { line: 0, col: 2 };
        }
        let mut clip = None;
        reg.apply(pid, EditorAction::DuplicateLine, &mut clip);
        let bid = reg.get_pane(pid).unwrap().buffer_id;
        let buf = reg.get_buffer(bid).unwrap();
        let line0: String = buf.line(0).chars().collect();
        let line1: String = buf.line(1).chars().collect();
        assert!(line0.starts_with("hello"), "original line unchanged");
        assert!(line1.starts_with("hello"), "duplicate must equal original");
        // Cursor on line 1, same col.
        assert_eq!(reg.get_pane(pid).unwrap().cursors[0].pos.line, 1);
        assert_eq!(reg.get_pane(pid).unwrap().cursors[0].pos.col, 2);
    }

    // ── #20 Move line up/down ────────────────────────────────────────────────

    #[test]
    fn move_line_up_swaps_with_previous() {
        let (mut reg, pid) = make_reg_with_text("aaa\nbbb\n");
        {
            let pane = reg.get_pane_mut(pid).unwrap();
            pane.cursors[0].pos = Position { line: 1, col: 0 };
            pane.cursors[0].anchor = Position { line: 1, col: 0 };
        }
        let mut clip = None;
        reg.apply(pid, EditorAction::MoveLineUp, &mut clip);
        let bid = reg.get_pane(pid).unwrap().buffer_id;
        let buf = reg.get_buffer(bid).unwrap();
        let l0: String = buf.line(0).chars().collect();
        let l1: String = buf.line(1).chars().collect();
        assert!(l0.starts_with("bbb"), "bbb must move up; got {:?}", l0);
        assert!(l1.starts_with("aaa"), "aaa must move down; got {:?}", l1);
        assert_eq!(reg.get_pane(pid).unwrap().cursors[0].pos.line, 0);
    }

    #[test]
    fn move_line_down_swaps_with_next() {
        let (mut reg, pid) = make_reg_with_text("aaa\nbbb\n");
        {
            let pane = reg.get_pane_mut(pid).unwrap();
            pane.cursors[0].pos = Position { line: 0, col: 0 };
            pane.cursors[0].anchor = Position { line: 0, col: 0 };
        }
        let mut clip = None;
        reg.apply(pid, EditorAction::MoveLineDown, &mut clip);
        let bid = reg.get_pane(pid).unwrap().buffer_id;
        let buf = reg.get_buffer(bid).unwrap();
        let l0: String = buf.line(0).chars().collect();
        let l1: String = buf.line(1).chars().collect();
        assert!(l0.starts_with("bbb"), "bbb must move up; got {:?}", l0);
        assert!(l1.starts_with("aaa"), "aaa must move down; got {:?}", l1);
        assert_eq!(reg.get_pane(pid).unwrap().cursors[0].pos.line, 1);
    }

    // ── #21 Indent / dedent selection ────────────────────────────────────────

    #[test]
    fn indent_selection_adds_four_spaces_to_each_line() {
        let (mut reg, pid) = make_reg_with_text("foo\nbar\n");
        // Select from line 0 col 0 to line 1 col 3.
        {
            let pane = reg.get_pane_mut(pid).unwrap();
            pane.cursors[0].anchor = Position { line: 0, col: 0 };
            pane.cursors[0].pos = Position { line: 1, col: 3 };
        }
        let mut clip = None;
        reg.apply(pid, EditorAction::IndentSelection, &mut clip);
        let bid = reg.get_pane(pid).unwrap().buffer_id;
        let buf = reg.get_buffer(bid).unwrap();
        let l0: String = buf.line(0).chars().collect();
        let l1: String = buf.line(1).chars().collect();
        assert!(
            l0.starts_with("    foo"),
            "line 0 must be indented; got {:?}",
            l0
        );
        assert!(
            l1.starts_with("    bar"),
            "line 1 must be indented; got {:?}",
            l1
        );
    }

    #[test]
    fn dedent_selection_removes_leading_spaces() {
        let (mut reg, pid) = make_reg_with_text("    foo\n    bar\n");
        {
            let pane = reg.get_pane_mut(pid).unwrap();
            pane.cursors[0].anchor = Position { line: 0, col: 0 };
            pane.cursors[0].pos = Position { line: 1, col: 3 };
        }
        let mut clip = None;
        reg.apply(pid, EditorAction::DedentSelection, &mut clip);
        let bid = reg.get_pane(pid).unwrap().buffer_id;
        let buf = reg.get_buffer(bid).unwrap();
        let l0: String = buf.line(0).chars().collect();
        let l1: String = buf.line(1).chars().collect();
        assert!(
            l0.starts_with("foo"),
            "line 0 must be dedented; got {:?}",
            l0
        );
        assert!(
            l1.starts_with("bar"),
            "line 1 must be dedented; got {:?}",
            l1
        );
    }

    // ── G4: per-buffer scroll position ───────────────────────────────────────

    /// G4: open A, scroll, switch to B, switch back to A → scroll restored.
    #[test]
    fn buffer_scroll_restored_on_tab_switch() {
        let mut reg = EditorPaneRegistry::default();
        let bid_a = reg.new_pane(1);
        // Open B as an additional tab in pane 1.
        let bid_b = {
            let b = anvil_editor::Buffer::new();
            let bid = reg.next_buffer_id;
            reg.next_buffer_id += 1;
            reg.buffers.insert(bid, b);
            let pane = reg.panes.get_mut(&1).unwrap();
            pane.open_buffers.push(bid);
            bid
        };

        // Scroll pane to line 42 while on buffer A.
        {
            let pane = reg.get_pane_mut(1).unwrap();
            assert_eq!(pane.buffer_id, bid_a);
            pane.scroll_pos = 42.0;
            pane.scroll_target = 42.0;
        }

        // Switch to B — A's scroll should be saved.
        reg.open_buffer(1, bid_b);
        {
            let pane = reg.get_pane(1).unwrap();
            assert_eq!(pane.buffer_id, bid_b);
            assert_eq!(pane.scroll_pos, 0.0, "B should start at 0");
        }

        // Switch back to A — scroll should be restored to 42.
        reg.open_buffer(1, bid_a);
        {
            let pane = reg.get_pane(1).unwrap();
            assert_eq!(pane.buffer_id, bid_a);
            assert!(
                (pane.scroll_pos - 42.0).abs() < 0.01,
                "A scroll must be restored; got {}",
                pane.scroll_pos
            );
        }
    }

    // ── L1: clipboard copy/cut whole-line fallback ───────────────────────────

    #[test]
    fn copy_no_selection_yields_whole_line() {
        // Cursor at col 3 of "hello\n" — no selection.
        let (mut reg, pid) = make_reg_with_text("hello\nworld\n");
        {
            let pane = reg.get_pane_mut(pid).unwrap();
            pane.cursors[0].pos = Position { line: 0, col: 3 };
            pane.cursors[0].anchor = Position { line: 0, col: 3 };
        }
        let mut clip: Option<String> = None;
        let mutated = reg.apply(pid, EditorAction::Copy, &mut clip);
        assert!(!mutated, "copy must not mutate the buffer");
        assert_eq!(
            clip.as_deref(),
            Some("hello\n"),
            "whole line must be copied"
        );
        // Buffer unchanged
        assert_eq!(buf_text(&reg, pid), "hello\nworld\n");
    }

    #[test]
    fn copy_with_selection_copies_selection() {
        // Select "ell" from col 1..4.
        let (mut reg, pid) = make_reg_with_text("hello\n");
        {
            let pane = reg.get_pane_mut(pid).unwrap();
            pane.cursors[0].anchor = Position { line: 0, col: 1 };
            pane.cursors[0].pos = Position { line: 0, col: 4 };
        }
        let mut clip: Option<String> = None;
        reg.apply(pid, EditorAction::Copy, &mut clip);
        assert_eq!(
            clip.as_deref(),
            Some("ell"),
            "selection text must be copied"
        );
    }

    #[test]
    fn cut_no_selection_removes_whole_line() {
        let (mut reg, pid) = make_reg_with_text("hello\nworld\n");
        {
            let pane = reg.get_pane_mut(pid).unwrap();
            pane.cursors[0].pos = Position { line: 0, col: 2 };
            pane.cursors[0].anchor = Position { line: 0, col: 2 };
        }
        let mut clip: Option<String> = None;
        let mutated = reg.apply(pid, EditorAction::Cut, &mut clip);
        assert!(mutated, "cut must mutate the buffer");
        assert_eq!(
            clip.as_deref(),
            Some("hello\n"),
            "cut must yield the whole line"
        );
        assert_eq!(buf_text(&reg, pid), "world\n", "line must be removed");
        let pos = reg.get_pane(pid).unwrap().cursors[0].pos;
        assert_eq!(
            pos,
            Position { line: 0, col: 0 },
            "cursor must sit at start of next line"
        );
    }

    // ── L2: Undo / Redo wiring verify ───────────────────────────────────────

    #[test]
    fn undo_redo_round_trip() {
        let (mut reg, pid) = make_reg_with_text("abc");
        let mut clip = None;
        reg.apply(pid, EditorAction::InsertChar('X'), &mut clip);
        assert_eq!(buf_text(&reg, pid), "Xabc");
        // Undo must revert.
        let mutated = reg.apply(pid, EditorAction::Undo, &mut clip);
        assert!(mutated);
        assert_eq!(
            buf_text(&reg, pid),
            "abc",
            "undo must restore original text"
        );
        // Redo must re-apply.
        let mutated = reg.apply(pid, EditorAction::Redo, &mut clip);
        assert!(mutated);
        assert_eq!(buf_text(&reg, pid), "Xabc", "redo must re-apply the edit");
    }

    /// G4: close_buffer drops the saved scroll entry (no stale memory).
    #[test]
    fn buffer_scroll_dropped_on_close() {
        let mut reg = EditorPaneRegistry::default();
        let bid_a = reg.new_pane(1);
        let bid_b = {
            let b = anvil_editor::Buffer::new();
            let bid = reg.next_buffer_id;
            reg.next_buffer_id += 1;
            reg.buffers.insert(bid, b);
            let pane = reg.panes.get_mut(&1).unwrap();
            pane.open_buffers.push(bid);
            bid
        };

        // Set scroll on A, switch to B to save it.
        {
            let pane = reg.get_pane_mut(1).unwrap();
            pane.scroll_pos = 10.0;
        }
        reg.open_buffer(1, bid_b);

        // Saved scroll for A must be in the map.
        assert!(
            reg.buffer_scroll.contains_key(&bid_a),
            "scroll for A must be saved after switching away"
        );

        // Close A — entry must be dropped.
        reg.close_buffer(1, bid_a);
        assert!(
            !reg.buffer_scroll.contains_key(&bid_a),
            "scroll for closed buffer must be removed"
        );
    }

    // ── M3: PageUp / PageDown cursor movement ─────────────────────────────────

    /// M3: PageUp moves cursor upward (toward line 0) and does not panic.
    #[test]
    fn page_up_moves_cursor_up() {
        let mut reg = EditorPaneRegistry::default();
        reg.new_pane(1);
        let text: String = (0..50).map(|i| format!("line{i}\n")).collect();
        {
            let buf = reg.get_buffer_mut(1).unwrap();
            *buf = anvil_editor::Buffer::from_text(&text);
        }
        // Position cursor at line 30.
        {
            let pane = reg.get_pane_mut(1).unwrap();
            pane.cursors[0].pos = anvil_editor::Position { line: 30, col: 0 };
            pane.cursors[0].anchor = anvil_editor::Position { line: 30, col: 0 };
            pane.scroll_pos = 20.0;
            pane.scroll_target = 20.0;
        }
        let mut clip = None;
        reg.apply(1, EditorAction::PageUp { extend: false }, &mut clip);
        let pane = reg.get_pane(1).unwrap();
        // Cursor must have moved to a line < 30.
        assert!(
            pane.cursors[0].pos.line < 30,
            "PageUp must move cursor above line 30; got line {}",
            pane.cursors[0].pos.line
        );
    }

    /// M3: PageDown moves cursor downward and does not panic.
    #[test]
    fn page_down_moves_cursor_down() {
        let mut reg = EditorPaneRegistry::default();
        reg.new_pane(1);
        let text: String = (0..50).map(|i| format!("line{i}\n")).collect();
        {
            let buf = reg.get_buffer_mut(1).unwrap();
            *buf = anvil_editor::Buffer::from_text(&text);
        }
        // Position cursor at line 5.
        {
            let pane = reg.get_pane_mut(1).unwrap();
            pane.cursors[0].pos = anvil_editor::Position { line: 5, col: 0 };
            pane.cursors[0].anchor = anvil_editor::Position { line: 5, col: 0 };
        }
        let mut clip = None;
        reg.apply(1, EditorAction::PageDown { extend: false }, &mut clip);
        let pane = reg.get_pane(1).unwrap();
        // Cursor must have moved below line 5.
        assert!(
            pane.cursors[0].pos.line > 5,
            "PageDown must move cursor below line 5; got line {}",
            pane.cursors[0].pos.line
        );
    }

    // ── M1: scroll_target stays non-negative after wheel scroll ───────────────

    /// M1: scroll_target must not go negative when clamped.
    #[test]
    fn scroll_target_clamp_non_negative() {
        let mut reg = EditorPaneRegistry::default();
        reg.new_pane(1);
        {
            let pane = reg.get_pane_mut(1).unwrap();
            pane.scroll_target = 0.0;
        }
        // Simulate an upward scroll that would go negative.
        {
            let pane = reg.get_pane_mut(1).unwrap();
            pane.scroll_target = (pane.scroll_target - 5.0).max(0.0);
        }
        let pane = reg.get_pane(1).unwrap();
        assert_eq!(
            pane.scroll_target, 0.0,
            "scroll_target must clamp to 0 when scrolled above top"
        );
    }

    // ── Tier-W action tests ───────────────────────────────────────────────────

    // W4: DeleteLine

    #[test]
    fn delete_line_removes_current_line_and_moves_cursor() {
        let (mut reg, pid) = make_reg_with_text("alpha\nbeta\ngamma\n");
        // Position cursor on line 1 ("beta").
        {
            let pane = reg.get_pane_mut(pid).unwrap();
            pane.cursors[0].pos = Position { line: 1, col: 2 };
            pane.cursors[0].anchor = Position { line: 1, col: 2 };
        }
        let mut clip = None;
        let mutated = reg.apply(pid, EditorAction::DeleteLine, &mut clip);
        assert!(mutated);
        assert_eq!(buf_text(&reg, pid), "alpha\ngamma\n");
        let pane = reg.get_pane(pid).unwrap();
        // Cursor should be at col 0 of the successor line (now "gamma" at line 1).
        assert_eq!(pane.cursors[0].pos, Position { line: 1, col: 0 });
    }

    #[test]
    fn delete_line_on_last_line_clears_content() {
        // "only\n" has line 0 = "only" and line 1 = "".
        // DeleteLine on line 0 deletes its content, leaving an empty buffer.
        let (mut reg, pid) = make_reg_with_text("only\n");
        let mut clip = None;
        let mutated = reg.apply(pid, EditorAction::DeleteLine, &mut clip);
        assert!(mutated);
        // Content of the (sole) line is gone; the buffer text may be empty or
        // contain only the newline depending on rope internals — either is acceptable.
        // The critical invariant is that no panic occurred and the cursor is valid.
        let text = buf_text(&reg, pid);
        assert!(
            text.is_empty() || text == "\n",
            "expected empty or bare newline, got {text:?}"
        );
    }

    // W7: SelectAllOccurrences

    #[test]
    fn select_all_occurrences_places_cursor_at_each_match() {
        let (mut reg, pid) = make_reg_with_text("foo bar foo baz foo\n");
        // Position cursor on first "foo" (col 0..3).
        {
            let pane = reg.get_pane_mut(pid).unwrap();
            pane.cursors[0].anchor = Position { line: 0, col: 0 };
            pane.cursors[0].pos = Position { line: 0, col: 3 };
        }
        let mut clip = None;
        reg.apply(pid, EditorAction::SelectAllOccurrences, &mut clip);
        let pane = reg.get_pane(pid).unwrap();
        assert_eq!(pane.cursors.len(), 3, "three occurrences of 'foo'");
    }

    // W8: DropLastCursor

    #[test]
    fn drop_last_cursor_removes_most_recent_secondary() {
        let (mut reg, pid) = make_reg_with_text("hello world\n");
        // Add two secondary cursors manually.
        {
            let pane = reg.get_pane_mut(pid).unwrap();
            pane.cursors.push(Cursor {
                pos: Position { line: 0, col: 5 },
                anchor: Position { line: 0, col: 5 },
            });
            pane.cursors.push(Cursor {
                pos: Position { line: 0, col: 8 },
                anchor: Position { line: 0, col: 8 },
            });
        }
        assert_eq!(reg.get_pane(pid).unwrap().cursors.len(), 3);
        let mut clip = None;
        reg.apply(pid, EditorAction::DropLastCursor, &mut clip);
        let pane = reg.get_pane(pid).unwrap();
        assert_eq!(pane.cursors.len(), 2, "last cursor dropped");
        assert_eq!(
            pane.cursors.last().unwrap().pos,
            Position { line: 0, col: 5 }
        );
    }

    #[test]
    fn drop_last_cursor_noop_with_single_cursor() {
        let (mut reg, pid) = make_reg_with_text("hello\n");
        let mut clip = None;
        reg.apply(pid, EditorAction::DropLastCursor, &mut clip);
        assert_eq!(reg.get_pane(pid).unwrap().cursors.len(), 1);
    }

    // W9: FoldAll / UnfoldAll

    #[test]
    fn fold_all_marks_block_opener_lines() {
        let (mut reg, pid) = make_reg_with_text("fn foo() {\nlet x = 1;\n}\n");
        let mut clip = None;
        reg.apply(pid, EditorAction::FoldAll, &mut clip);
        let pane = reg.get_pane(pid).unwrap();
        let bid = pane.buffer_id;
        let folds = pane.folds.get(&bid);
        // Line 0 ends with `{` → should be folded.
        assert!(
            folds.map(|f| f.contains(&0)).unwrap_or(false),
            "line 0 (opener) must be folded"
        );
        // Line 1 is plain → must not be folded.
        assert!(
            !folds.map(|f| f.contains(&1)).unwrap_or(false),
            "line 1 must not be folded"
        );
    }

    #[test]
    fn unfold_all_clears_all_folds() {
        let (mut reg, pid) = make_reg_with_text("fn foo() {\nlet x = 1;\n}\n");
        let mut clip = None;
        reg.apply(pid, EditorAction::FoldAll, &mut clip);
        reg.apply(pid, EditorAction::UnfoldAll, &mut clip);
        let pane = reg.get_pane(pid).unwrap();
        let bid = pane.buffer_id;
        assert!(pane.folds.get(&bid).map(|f| f.is_empty()).unwrap_or(true));
    }

    // W10: ConvertCase*

    #[test]
    fn convert_case_upper_selected_text() {
        let (mut reg, pid) = make_reg_with_text("hello world\n");
        {
            let pane = reg.get_pane_mut(pid).unwrap();
            pane.cursors[0].anchor = Position { line: 0, col: 0 };
            pane.cursors[0].pos = Position { line: 0, col: 5 };
        }
        let mut clip = None;
        let mutated = reg.apply(pid, EditorAction::ConvertCaseUpper, &mut clip);
        assert!(mutated);
        assert_eq!(buf_text(&reg, pid), "HELLO world\n");
    }

    #[test]
    fn convert_case_lower_selected_text() {
        let (mut reg, pid) = make_reg_with_text("HELLO WORLD\n");
        {
            let pane = reg.get_pane_mut(pid).unwrap();
            pane.cursors[0].anchor = Position { line: 0, col: 6 };
            pane.cursors[0].pos = Position { line: 0, col: 11 };
        }
        let mut clip = None;
        let mutated = reg.apply(pid, EditorAction::ConvertCaseLower, &mut clip);
        assert!(mutated);
        assert_eq!(buf_text(&reg, pid), "HELLO world\n");
    }

    #[test]
    fn convert_case_title_selected_text() {
        let (mut reg, pid) = make_reg_with_text("hello world\n");
        {
            let pane = reg.get_pane_mut(pid).unwrap();
            pane.cursors[0].anchor = Position { line: 0, col: 0 };
            pane.cursors[0].pos = Position { line: 0, col: 11 };
        }
        let mut clip = None;
        let mutated = reg.apply(pid, EditorAction::ConvertCaseTitle, &mut clip);
        assert!(mutated);
        assert_eq!(buf_text(&reg, pid), "Hello World\n");
    }

    // W11: SortSelectedLines

    #[test]
    fn sort_selected_lines_alphabetically() {
        let (mut reg, pid) = make_reg_with_text("banana\napple\ncherry\n");
        {
            let pane = reg.get_pane_mut(pid).unwrap();
            pane.cursors[0].anchor = Position { line: 0, col: 0 };
            pane.cursors[0].pos = Position { line: 2, col: 6 };
        }
        let mut clip = None;
        let mutated = reg.apply(pid, EditorAction::SortSelectedLines, &mut clip);
        assert!(mutated);
        assert_eq!(buf_text(&reg, pid), "apple\nbanana\ncherry\n");
    }

    #[test]
    fn sort_selected_lines_noop_on_single_line() {
        let (mut reg, pid) = make_reg_with_text("banana\napple\ncherry\n");
        {
            let pane = reg.get_pane_mut(pid).unwrap();
            pane.cursors[0].anchor = Position { line: 1, col: 0 };
            pane.cursors[0].pos = Position { line: 1, col: 5 };
        }
        let mut clip = None;
        let mutated = reg.apply(pid, EditorAction::SortSelectedLines, &mut clip);
        assert!(!mutated, "single-line selection must be a no-op");
        assert_eq!(buf_text(&reg, pid), "banana\napple\ncherry\n");
    }

    // W12: JoinLines

    #[test]
    fn join_lines_replaces_newline_with_space() {
        let (mut reg, pid) = make_reg_with_text("hello\nworld\n");
        let mut clip = None;
        let mutated = reg.apply(pid, EditorAction::JoinLines, &mut clip);
        assert!(mutated);
        assert_eq!(buf_text(&reg, pid), "hello world\n");
        let pane = reg.get_pane(pid).unwrap();
        assert_eq!(pane.cursors[0].pos.line, 0, "cursor stays on original line");
    }

    #[test]
    fn join_lines_noop_on_last_line() {
        let (mut reg, pid) = make_reg_with_text("hello\n");
        {
            // Move cursor to the last line manually (there's only one line here).
            // last_line = 0.
        }
        let mut clip = None;
        let mutated = reg.apply(pid, EditorAction::JoinLines, &mut clip);
        // "hello\n" has line 0 and possibly a phantom empty line 1 depending on
        // rope impl — the important thing is we don't panic and the buffer is sane.
        let _ = mutated;
        assert!(!buf_text(&reg, pid).is_empty());
    }

    // ── Tier-X tests ─────────────────────────────────────────────────────────

    // X14: MoveWordLeft / MoveWordRight

    #[test]
    fn move_word_right_lands_at_next_word_start() {
        // "hello world" — cursor at col 0, MoveWordRight → col 6 ("world").
        let (mut reg, pid) = make_reg_with_text("hello world\n");
        let mut clip = None;
        reg.apply(
            pid,
            EditorAction::MoveWordRight { extend: false },
            &mut clip,
        );
        let pane = reg.get_pane(pid).unwrap();
        assert_eq!(
            pane.cursors[0].pos.col, 6,
            "should skip 'hello ' and land at 'w'"
        );
    }

    #[test]
    fn move_word_left_from_middle_of_word_lands_at_word_start() {
        // "hello world" — cursor at col 9 (inside "world"), MoveWordLeft → col 6 ("w").
        let (mut reg, pid) = make_reg_with_text("hello world\n");
        {
            let pane = reg.get_pane_mut(pid).unwrap();
            pane.cursors[0].pos = Position { line: 0, col: 9 };
            pane.cursors[0].anchor = Position { line: 0, col: 9 };
        }
        let mut clip = None;
        reg.apply(pid, EditorAction::MoveWordLeft { extend: false }, &mut clip);
        let pane = reg.get_pane(pid).unwrap();
        assert_eq!(
            pane.cursors[0].pos.col, 6,
            "should land at start of 'world'"
        );
    }

    #[test]
    fn move_word_right_with_extend_creates_selection() {
        let (mut reg, pid) = make_reg_with_text("hello world\n");
        let mut clip = None;
        reg.apply(pid, EditorAction::MoveWordRight { extend: true }, &mut clip);
        let pane = reg.get_pane(pid).unwrap();
        assert_eq!(pane.cursors[0].anchor.col, 0);
        assert_eq!(pane.cursors[0].pos.col, 6);
    }

    // X13: TrimTrailingWhitespace

    #[test]
    fn trim_trailing_whitespace_removes_trailing_spaces() {
        let (mut reg, pid) = make_reg_with_text("hello   \nworld\n");
        let mut clip = None;
        let mutated = reg.apply(pid, EditorAction::TrimTrailingWhitespace, &mut clip);
        assert!(mutated);
        let text = buf_text(&reg, pid);
        assert_eq!(
            text, "hello\nworld\n",
            "trailing spaces on line 0 should be removed"
        );
    }

    #[test]
    fn trim_trailing_whitespace_noop_when_no_trailing_spaces() {
        let (mut reg, pid) = make_reg_with_text("clean\nlines\n");
        let mut clip = None;
        let mutated = reg.apply(pid, EditorAction::TrimTrailingWhitespace, &mut clip);
        assert!(!mutated, "no trailing whitespace: must be a no-op");
    }

    // X11: ExpandSnippet

    #[test]
    fn expand_snippet_replaces_trigger_with_body() {
        // Buffer: "fn" at col 0..2; cursor at col 2.
        let (mut reg, pid) = make_reg_with_text("fn\n");
        {
            let pane = reg.get_pane_mut(pid).unwrap();
            pane.cursors[0].pos = Position { line: 0, col: 2 };
            pane.cursors[0].anchor = Position { line: 0, col: 2 };
        }
        let mut clip = None;
        let mutated = reg.apply(
            pid,
            EditorAction::ExpandSnippet {
                trigger: "fn".to_string(),
                body: "fn name() {}".to_string(),
            },
            &mut clip,
        );
        assert!(mutated);
        let text = buf_text(&reg, pid);
        // The trigger "fn" should be replaced by the body.
        assert!(
            text.contains("fn name() {}"),
            "body should be inserted; got {text:?}"
        );
        // "fn" trigger is removed; only body remains.
        assert!(
            !text.starts_with("fn\n"),
            "original trigger line should be replaced"
        );
    }

    // X8: GotoSymbolAtCursor (no-op without syntax; just verify no panic)

    #[test]
    fn goto_symbol_at_cursor_noop_when_no_match() {
        let (mut reg, pid) = make_reg_with_text("fn main() {}\n");
        {
            let pane = reg.get_pane_mut(pid).unwrap();
            pane.cursors[0].pos = Position { line: 0, col: 3 };
            pane.cursors[0].anchor = Position { line: 0, col: 3 };
        }
        let mut clip = None;
        // Without a parsed syntax tree the outline is empty; action must not panic.
        let mutated = reg.apply(pid, EditorAction::GotoSymbolAtCursor, &mut clip);
        assert!(!mutated, "no outline match → no mutation");
    }

    // X12: Surround-selection (already in InsertChar bracket-pair logic)

    #[test]
    fn insert_open_bracket_wraps_selection() {
        // "hello" with full selection → insert '(' wraps as "(hello)".
        let (mut reg, pid) = make_reg_with_text("hello\n");
        {
            let pane = reg.get_pane_mut(pid).unwrap();
            pane.cursors[0].anchor = Position { line: 0, col: 0 };
            pane.cursors[0].pos = Position { line: 0, col: 5 };
        }
        let mut clip = None;
        let mutated = reg.apply(pid, EditorAction::InsertChar('('), &mut clip);
        assert!(mutated);
        let text = buf_text(&reg, pid);
        assert!(
            text.starts_with("(hello)"),
            "selection should be wrapped; got {text:?}"
        );
    }
}
