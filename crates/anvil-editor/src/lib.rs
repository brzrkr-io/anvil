//! `anvil-editor` — rope-backed native text buffer, syntax, LSP, and git
//! gutter. The nvim RPC bridge was retired at NE15; this crate is now fully
//! native.
//!
//! `buffer` module: native `Buffer` type built on `ropey`.

pub mod buffer;
pub mod git;
pub mod lsp;
pub mod syntax;

// Re-export buffer types. `EditRecord` and `UndoStack` are intentionally kept
// internal to the crate — they live on `Buffer` as private undo state and
// expose no constructor surface worth a public commitment.
pub use buffer::{
    AgentRevision, Buffer, BufferId, Cursor, Edit, EditProposal, EncodingError, GhostTextSpan,
    IoError, Position, ProposalError, ProposalStatus, Range,
};
pub use git::{GitChange, GitGutter};
pub use syntax::{
    FoldRange, OutlineSymbol, OutlineSymbolKind, SyntaxLayer, SyntaxRole, derive_fold_ranges,
    derive_outline_rows,
};

// Re-export LSP types used by App and main.rs (NE9, NE10, tier-3).
pub use lsp::{
    CompletionItem, DefinitionLocation, DiagnosticSeverity, DocumentDiagnostic, HoverResult,
    LspManager, LspState, language_id_for_ext, server_id_for_language,
};
