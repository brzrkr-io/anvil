//! `anvil-editor` — rope-backed text buffer and nvim RPC bridge.
//!
//! `nvim` submodule: msgpack codec, Unix-socket transport, background polling
//! bridge. Stays as the default editor pane until NE6 ships.
//!
//! `buffer` module: native `Buffer` type built on `ropey`.

pub mod buffer;
pub mod lsp;
pub mod nvim;
pub mod syntax;

// Re-export nvim bridge types for existing call sites in main.rs.
pub use nvim::bridge::{
    ConnectionState, EditorBridge, EditorSnapshot, OutlineState, OutlineSymbol, SymbolKind,
};
pub use nvim::codec::{CodecError, Value, decode_value, encode_request};
pub use nvim::transport::{Endpoint, Transport, TransportError};

// Re-export buffer types. `EditRecord` and `UndoStack` are intentionally kept
// internal to the crate — they live on `Buffer` as private undo state and
// expose no constructor surface worth a public commitment.
pub use buffer::{
    Buffer, BufferId, Cursor, Edit, EditProposal, EncodingError, GhostTextSpan, IoError, Position,
    Range,
};
pub use syntax::{SyntaxLayer, SyntaxRole};

// Re-export LSP types used by App and main.rs (NE9).
pub use lsp::{
    DiagnosticSeverity, DocumentDiagnostic, LspManager, LspState, language_id_for_ext,
    server_id_for_language,
};
