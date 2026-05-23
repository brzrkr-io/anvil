//! `anvil-editor` — msgpack codec and Unix-socket transport for nvim RPC.
//!
//! Phase 1 public surface: codec types and a synchronous Transport.

pub mod codec;
pub mod transport;

pub use codec::{CodecError, Value, decode_value, encode_request};
pub use transport::{Endpoint, Transport, TransportError};
