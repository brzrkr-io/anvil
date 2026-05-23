//! Workspace pane layout: PaneTree, split/close/focus model.
//!
//! ## Modules ported from Zig
//!
//! | Rust module  | Zig source                     |
//! |--------------|-------------------------------|
//! | `layout`     | `src/workspace/layout.zig`    |
//! | `pane`       | `src/workspace/pane.zig`      |
//! | `tab`        | `src/app/tab.zig`             |
//! | `selection`  | `src/app/selection.zig`       |
//! | `palette`    | `src/app/palette.zig`         |
//! | `interact`   | `src/app/interact.zig`        |
//! | `keys`       | `src/app/keys.zig`            |
//!
//! ## Deferred (platform-bound)
//!
//! - `src/app/shell_integration.zig` — calls `setenv`/`getenv`/`ZDOTDIR`,
//!   writes files to `~/.cache/anvil/shell`, and embeds zsh/bash script
//!   sources.  This belongs in `crates/anvil-platform`.

pub mod interact;
pub mod keys;
pub mod layout;
pub mod palette;
pub mod pane;
pub mod selection;
pub mod tab;
