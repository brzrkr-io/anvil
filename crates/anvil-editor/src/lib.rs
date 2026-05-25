//! `anvil-editor` — rope-backed text buffer and nvim RPC bridge.
//!
//! `nvim` submodule: msgpack codec, Unix-socket transport, background polling
//! bridge. Stays as the default editor pane until NE6 ships.
//!
//! `buffer` module: native `Buffer` type built on `ropey`.

pub mod buffer;
pub mod nvim;

// Re-export nvim bridge types for existing call sites in main.rs.
pub use nvim::bridge::{
    ConnectionState, EditorBridge, EditorSnapshot, OutlineState, OutlineSymbol, SymbolKind,
};
pub use nvim::codec::{CodecError, Value, decode_value, encode_request};
pub use nvim::transport::{Endpoint, Transport, TransportError};

// Re-export buffer types.
pub use buffer::{
    Buffer, BufferId, Cursor, Edit, EditProposal, GhostTextSpan, Position, Range, RevisionTag,
};
