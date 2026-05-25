//! Workspace pane layout: PaneTree, split/close/focus model.
//!
//! Modules: `layout`, `pane`, `tab`, `selection`, `palette`, `interact`, `keys`.
//!
//! Shell integration (`setenv`/`getenv`/`ZDOTDIR`, `~/.cache/anvil/shell`
//! scripts) lives in `crates/anvil-platform` — it has platform-specific deps.

pub mod editor_pane;
pub mod editor_search;
pub mod interact;
pub mod keys;
pub mod layout;
pub mod mode;
pub mod palette;
pub mod pane;
pub mod project_search;
pub mod selection;
pub mod tab;
