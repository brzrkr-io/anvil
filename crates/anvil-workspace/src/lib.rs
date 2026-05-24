//! Workspace pane layout: PaneTree, split/close/focus model.
//!
//! Modules: `layout`, `pane`, `tab`, `selection`, `palette`, `interact`, `keys`.
//!
//! Shell integration (`setenv`/`getenv`/`ZDOTDIR`, `~/.cache/anvil/shell`
//! scripts) lives in `crates/anvil-platform` — it has platform-specific deps.

pub mod interact;
pub mod keys;
pub mod layout;
pub mod palette;
pub mod pane;
pub mod selection;
pub mod tab;
