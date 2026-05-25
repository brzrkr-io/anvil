//! Synchronous Unix-socket msgpack-RPC transport for nvim.
//!
//! Phase 1: `call()` is synchronous — write a request, read responses until
//! the matching msgid arrives. Notifications are silently discarded (phase 2
//! will queue them).

use std::{
    io::{Read, Write},
    os::unix::net::UnixStream,
    path::PathBuf,
    sync::atomic::{AtomicU32, Ordering},
    time::Duration,
};

use thiserror::Error;

use super::codec::{self, CodecError, Value};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Address of a Neovim Unix-socket endpoint.
#[derive(Debug, Clone)]
pub struct Endpoint {
    pub path: PathBuf,
}

/// Errors from the transport layer.
#[derive(Debug, Error)]
pub enum TransportError {
    #[error("connect failed: {0}")]
    ConnectFailed(#[source] std::io::Error),
    #[error("io error: {0}")]
    Io(#[source] std::io::Error),
    #[error("call timed out")]
    Timeout,
    #[error("rpc error from peer: {0:?}")]
    RpcError(Value),
    #[error("codec error: {0}")]
    Codec(#[from] CodecError),
    #[error("unexpected response frame format")]
    BadFrame,
}

// ---------------------------------------------------------------------------
// Transport
// ---------------------------------------------------------------------------

/// A connected msgpack-RPC client over a Unix domain socket.
#[derive(Debug)]
pub struct Transport {
    stream: UnixStream,
    /// Read buffer accumulating bytes from the socket.
    read_buf: Vec<u8>,
    /// Monotonically increasing request ID.
    next_id: AtomicU32,
}

impl Transport {
    /// Connect to the given endpoint.
    pub fn connect(ep: &Endpoint) -> Result<Self, TransportError> {
        let stream = UnixStream::connect(&ep.path).map_err(TransportError::ConnectFailed)?;
        Ok(Self {
            stream,
            read_buf: Vec::with_capacity(4096),
            next_id: AtomicU32::new(1),
        })
    }

    /// Send a request and synchronously wait for the matching response.
    ///
    /// Notifications (`type == 2`) received before the response are discarded.
    /// Returns the `result` field on success; returns `TransportError::RpcError`
    /// if the peer-returned `error` field is non-nil.
    pub fn call(
        &mut self,
        method: &str,
        params: &[Value],
        timeout: Duration,
    ) -> Result<Value, TransportError> {
        let msgid = self.next_id.fetch_add(1, Ordering::Relaxed);

        // Encode and send the request frame.
        let mut frame = Vec::new();
        codec::encode_request(&mut frame, msgid, method, params)?;
        self.stream.write_all(&frame).map_err(TransportError::Io)?;

        // Set read timeout to enforce `timeout`.
        self.stream
            .set_read_timeout(Some(timeout))
            .map_err(TransportError::Io)?;

        // Read and decode frames until we see the response for our msgid.
        loop {
            let value = self.read_one_value()?;
            match parse_frame(value, msgid)? {
                FrameResult::Response(result) => return Ok(result),
                FrameResult::Notification => {
                    // Phase 1: discard notifications.
                    continue;
                }
                FrameResult::WrongMsgid => {
                    // Response for a different msgid — discard (single caller,
                    // so this shouldn't happen in normal use).
                    continue;
                }
            }
        }
    }

    /// Close the transport.
    pub fn close(self) {
        // UnixStream closes on drop; this method exists for explicit clean-up.
        drop(self);
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    /// Read bytes from the socket until we have a complete msgpack value.
    ///
    /// We do this by attempting to decode from `read_buf`; if decoding fails
    /// with `UnexpectedEof`, we read more bytes from the socket and retry.
    fn read_one_value(&mut self) -> Result<Value, TransportError> {
        loop {
            // Try to decode from the current buffer.
            let buf_snapshot = self.read_buf.clone();
            let mut r: &[u8] = &buf_snapshot;
            let start_len = r.len();
            match codec::decode_value(&mut r) {
                Ok(value) => {
                    let consumed = start_len - r.len();
                    self.read_buf.drain(..consumed);
                    return Ok(value);
                }
                Err(CodecError::UnexpectedEof) => {
                    // Need more bytes — read a chunk from the socket.
                    let mut tmp = [0u8; 4096];
                    let n = match self.stream.read(&mut tmp) {
                        Ok(0) => {
                            return Err(TransportError::Io(std::io::Error::new(
                                std::io::ErrorKind::UnexpectedEof,
                                "connection closed",
                            )));
                        }
                        Ok(n) => n,
                        Err(e)
                            if e.kind() == std::io::ErrorKind::WouldBlock
                                || e.kind() == std::io::ErrorKind::TimedOut =>
                        {
                            return Err(TransportError::Timeout);
                        }
                        Err(e) => return Err(TransportError::Io(e)),
                    };
                    self.read_buf.extend_from_slice(&tmp[..n]);
                    // Loop to retry decode.
                }
                Err(other) => return Err(TransportError::Codec(other)),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Frame parsing helpers
// ---------------------------------------------------------------------------

enum FrameResult {
    Response(Value),
    Notification,
    WrongMsgid,
}

/// Parse a decoded msgpack value as a msgpack-RPC frame.
///
/// - `[1, msgid, error, result]` → Response
/// - `[2, method, params]`       → Notification
/// - anything else               → `BadFrame`
fn parse_frame(v: Value, expected_msgid: u32) -> Result<FrameResult, TransportError> {
    let items = match v {
        Value::Array(a) => a,
        _ => return Err(TransportError::BadFrame),
    };

    if items.is_empty() {
        return Err(TransportError::BadFrame);
    }

    let msg_type = match &items[0] {
        Value::Uint(n) => *n,
        Value::Int(n) if *n >= 0 => *n as u64,
        _ => return Err(TransportError::BadFrame),
    };

    match msg_type {
        // Response frame: [1, msgid, error, result]
        1 => {
            if items.len() != 4 {
                return Err(TransportError::BadFrame);
            }
            let frame_msgid = match &items[1] {
                Value::Uint(n) => *n as u32,
                Value::Int(n) if *n >= 0 => *n as u32,
                _ => return Err(TransportError::BadFrame),
            };
            if frame_msgid != expected_msgid {
                return Ok(FrameResult::WrongMsgid);
            }
            let error = items[2].clone();
            let result = items[3].clone();
            if !matches!(error, Value::Nil) {
                return Err(TransportError::RpcError(error));
            }
            Ok(FrameResult::Response(result))
        }
        // Notification frame: [2, method, params]
        2 => Ok(FrameResult::Notification),
        _ => Err(TransportError::BadFrame),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io::Write, os::unix::net::UnixListener, thread};
    use tempfile::tempdir;

    /// Build a response frame `[1, msgid, nil, result]`.
    fn make_response(msgid: u32, result: &Value) -> Vec<u8> {
        let mut buf = Vec::new();
        let frame = Value::Array(vec![
            Value::Uint(1),
            Value::Uint(msgid as u64),
            Value::Nil,
            result.clone(),
        ]);
        // Use encode_value indirectly via encode_request's sibling — we have
        // no public encode_value, so build the frame with encode_request logic.
        // Instead, hand-encode the 4-element array for testing.
        encode_array4(&mut buf, &frame);
        buf
    }

    /// Minimal helper: encode a Value::Array of 4 items for test purposes.
    fn encode_array4(w: &mut Vec<u8>, v: &Value) {
        // Re-use the public encode_request path isn't possible directly, but we
        // can build the bytes ourselves. Use the codec's encode logic via a
        // fresh Vec and encode_request with a shim method.
        //
        // Actually: encode the whole frame as a msgpack array by hand.
        if let Value::Array(items) = v {
            assert_eq!(items.len(), 4);
            w.push(0x94); // fixarray len=4
            for item in items {
                encode_one(w, item);
            }
        }
    }

    fn encode_one(w: &mut Vec<u8>, v: &Value) {
        // Replicate the small encoding subset needed by the test helpers.
        match v {
            Value::Nil => w.push(0xc0),
            Value::Bool(b) => w.push(if *b { 0xc3 } else { 0xc2 }),
            Value::Uint(n) => {
                let n = *n;
                if n <= 0x7f {
                    w.push(n as u8);
                } else if n <= 0xff {
                    w.push(0xcc);
                    w.push(n as u8);
                } else if n <= 0xffff {
                    w.push(0xcd);
                    w.extend_from_slice(&(n as u16).to_be_bytes());
                } else {
                    w.push(0xce);
                    w.extend_from_slice(&(n as u32).to_be_bytes());
                }
            }
            Value::Int(n) => {
                let n = *n;
                if n >= -32 && n <= 127 {
                    w.push(n as i8 as u8);
                } else {
                    w.push(0xd3);
                    w.extend_from_slice(&n.to_be_bytes());
                }
            }
            Value::Str(s) => {
                let bytes = s.as_bytes();
                let len = bytes.len();
                if len <= 31 {
                    w.push(0xa0 | len as u8);
                } else {
                    w.push(0xd9);
                    w.push(len as u8);
                }
                w.extend_from_slice(bytes);
            }
            Value::Array(arr) => {
                let len = arr.len();
                if len <= 15 {
                    w.push(0x90 | len as u8);
                } else {
                    w.push(0xdc);
                    w.extend_from_slice(&(len as u16).to_be_bytes());
                }
                for item in arr {
                    encode_one(w, item);
                }
            }
            Value::Float(f) => {
                w.push(0xcb);
                w.extend_from_slice(&f.to_bits().to_be_bytes());
            }
            Value::Bin(b) => {
                w.push(0xc4);
                w.push(b.len() as u8);
                w.extend_from_slice(b);
            }
            Value::Map(pairs) => {
                let len = pairs.len();
                if len <= 15 {
                    w.push(0x80 | len as u8);
                }
                for (k, v2) in pairs {
                    encode_one(w, k);
                    encode_one(w, v2);
                }
            }
        }
    }

    /// Spawn a fake server on a tmp socket. It reads one request frame then
    /// writes the given response bytes and exits.
    fn spawn_fake_server(
        socket_path: &std::path::Path,
        response: Vec<u8>,
    ) -> thread::JoinHandle<()> {
        let listener = UnixListener::bind(socket_path).expect("bind");
        thread::spawn(move || {
            let (mut conn, _) = listener.accept().expect("accept");
            // Read the request (drain it; we don't validate it here).
            let mut discard = [0u8; 4096];
            // Give the client a moment, then write the response.
            // We do a non-blocking peek: just try a small read.
            conn.set_read_timeout(Some(Duration::from_millis(200))).ok();
            let _ = conn.read(&mut discard);
            conn.write_all(&response).expect("write response");
        })
    }

    #[test]
    fn call_returns_result() {
        let dir = tempdir().unwrap();
        let socket_path = dir.path().join("nvim.sock");

        let expected = Value::Str("hello from nvim".into());
        let response = make_response(1, &expected);

        let _server = spawn_fake_server(&socket_path, response);

        // Give the server thread a moment to bind.
        thread::sleep(Duration::from_millis(50));

        let ep = Endpoint { path: socket_path };
        let mut transport = Transport::connect(&ep).unwrap();
        let result = transport
            .call("nvim_get_current_buf", &[], Duration::from_secs(2))
            .unwrap();
        assert_eq!(result, expected);
    }

    #[test]
    fn call_timeout() {
        let dir = tempdir().unwrap();
        let socket_path = dir.path().join("timeout.sock");

        let listener = UnixListener::bind(&socket_path).unwrap();
        let _server = thread::spawn(move || {
            let (conn, _) = listener.accept().unwrap();
            // Accept but never write a response.
            thread::sleep(Duration::from_secs(5));
            drop(conn);
        });

        thread::sleep(Duration::from_millis(50));

        let ep = Endpoint { path: socket_path };
        let mut transport = Transport::connect(&ep).unwrap();
        let err = transport
            .call("nvim_get_current_buf", &[], Duration::from_millis(200))
            .unwrap_err();
        assert!(
            matches!(err, TransportError::Timeout),
            "expected Timeout, got {err:?}"
        );
    }

    #[test]
    fn connect_nonexistent_path() {
        let ep = Endpoint {
            path: PathBuf::from("/tmp/anvil_editor_nonexistent_8675309.sock"),
        };
        let err = Transport::connect(&ep).unwrap_err();
        assert!(matches!(err, TransportError::ConnectFailed(_)));
    }

    // ── Additional result-type coverage ──────────────────────────────────────

    #[test]
    fn call_returns_bool_result() {
        let dir = tempdir().unwrap();
        let socket_path = dir.path().join("nvim_bool.sock");
        let expected = Value::Bool(true);
        let response = make_response(1, &expected);
        let _server = spawn_fake_server(&socket_path, response);
        thread::sleep(Duration::from_millis(50));
        let ep = Endpoint { path: socket_path };
        let mut transport = Transport::connect(&ep).unwrap();
        let result = transport
            .call("nvim_test", &[], Duration::from_secs(2))
            .unwrap();
        assert_eq!(result, expected);
    }

    #[test]
    fn call_returns_uint_large_result() {
        // Exercises Uint > 0xff and > 0xffff branches in encode_one.
        let dir = tempdir().unwrap();
        let socket_path = dir.path().join("nvim_uint.sock");
        let expected = Value::Uint(0x10000); // > 0xffff
        let response = make_response(1, &expected);
        let _server = spawn_fake_server(&socket_path, response);
        thread::sleep(Duration::from_millis(50));
        let ep = Endpoint { path: socket_path };
        let mut transport = Transport::connect(&ep).unwrap();
        let result = transport
            .call("nvim_test", &[], Duration::from_secs(2))
            .unwrap();
        assert_eq!(result, expected);
    }

    #[test]
    fn call_returns_int_result() {
        let dir = tempdir().unwrap();
        let socket_path = dir.path().join("nvim_int.sock");
        let expected = Value::Int(-100); // exercises Int branch in encode_one
        let response = make_response(1, &expected);
        let _server = spawn_fake_server(&socket_path, response);
        thread::sleep(Duration::from_millis(50));
        let ep = Endpoint { path: socket_path };
        let mut transport = Transport::connect(&ep).unwrap();
        let result = transport
            .call("nvim_test", &[], Duration::from_secs(2))
            .unwrap();
        assert_eq!(result, expected);
    }

    #[test]
    fn call_returns_float_result() {
        let dir = tempdir().unwrap();
        let socket_path = dir.path().join("nvim_float.sock");
        let expected = Value::Float(3.14);
        let response = make_response(1, &expected);
        let _server = spawn_fake_server(&socket_path, response);
        thread::sleep(Duration::from_millis(50));
        let ep = Endpoint { path: socket_path };
        let mut transport = Transport::connect(&ep).unwrap();
        let result = transport
            .call("nvim_test", &[], Duration::from_secs(2))
            .unwrap();
        assert_eq!(result, expected);
    }

    #[test]
    fn call_returns_bin_result() {
        let dir = tempdir().unwrap();
        let socket_path = dir.path().join("nvim_bin.sock");
        let expected = Value::Bin(vec![0xDE, 0xAD, 0xBE, 0xEF]);
        let response = make_response(1, &expected);
        let _server = spawn_fake_server(&socket_path, response);
        thread::sleep(Duration::from_millis(50));
        let ep = Endpoint { path: socket_path };
        let mut transport = Transport::connect(&ep).unwrap();
        let result = transport
            .call("nvim_test", &[], Duration::from_secs(2))
            .unwrap();
        assert_eq!(result, expected);
    }

    #[test]
    fn call_returns_array_result() {
        let dir = tempdir().unwrap();
        let socket_path = dir.path().join("nvim_arr.sock");
        let expected = Value::Array(vec![Value::Uint(1), Value::Uint(2)]);
        let response = make_response(1, &expected);
        let _server = spawn_fake_server(&socket_path, response);
        thread::sleep(Duration::from_millis(50));
        let ep = Endpoint { path: socket_path };
        let mut transport = Transport::connect(&ep).unwrap();
        let result = transport
            .call("nvim_test", &[], Duration::from_secs(2))
            .unwrap();
        assert_eq!(result, expected);
    }

    #[test]
    fn call_returns_map_result() {
        let dir = tempdir().unwrap();
        let socket_path = dir.path().join("nvim_map.sock");
        let expected = Value::Map(vec![(Value::Str("k".into()), Value::Uint(42))]);
        let response = make_response(1, &expected);
        let _server = spawn_fake_server(&socket_path, response);
        thread::sleep(Duration::from_millis(50));
        let ep = Endpoint { path: socket_path };
        let mut transport = Transport::connect(&ep).unwrap();
        let result = transport
            .call("nvim_test", &[], Duration::from_secs(2))
            .unwrap();
        assert_eq!(result, expected);
    }

    // ── Notification frame is discarded, real response follows ────────────────

    #[test]
    fn call_discards_notification_before_response() {
        let dir = tempdir().unwrap();
        let socket_path = dir.path().join("nvim_notify.sock");

        let expected = Value::Uint(99);

        // Build a notification frame [2, "event", []] followed by the real response.
        let mut payload = Vec::new();
        payload.push(0x93); // fixarray len=3
        payload.push(2); // msg_type = 2 (notification)
        payload.push(0xa5); // fixstr len=5
        payload.extend_from_slice(b"event");
        payload.push(0x90); // fixarray len=0 (empty params)

        // Append the real response.
        let real_resp = make_response(1, &expected);
        payload.extend_from_slice(&real_resp);

        let listener = UnixListener::bind(&socket_path).expect("bind");
        let _server = thread::spawn(move || {
            let (mut conn, _) = listener.accept().expect("accept");
            let mut discard = [0u8; 4096];
            conn.set_read_timeout(Some(Duration::from_millis(200))).ok();
            let _ = conn.read(&mut discard);
            conn.write_all(&payload).expect("write");
        });

        thread::sleep(Duration::from_millis(50));
        let ep = Endpoint { path: socket_path };
        let mut transport = Transport::connect(&ep).unwrap();
        let result = transport
            .call("nvim_test", &[], Duration::from_secs(2))
            .unwrap();
        assert_eq!(result, expected);
    }

    // ── close() explicitly drops the transport ────────────────────────────────

    #[test]
    fn close_terminates_transport_cleanly() {
        let dir = tempdir().unwrap();
        let socket_path = dir.path().join("nvim_close.sock");
        let listener = UnixListener::bind(&socket_path).expect("bind");
        let _server = thread::spawn(move || {
            let (_conn, _) = listener.accept().expect("accept");
            // Just accept; the connection will be dropped when transport is closed.
        });
        thread::sleep(Duration::from_millis(50));
        let ep = Endpoint { path: socket_path };
        let transport = Transport::connect(&ep).unwrap();
        transport.close(); // must not panic
    }

    // ── Raw-bytes helpers for frame error tests ───────────────────────────────

    fn spawn_raw_server(socket_path: &std::path::Path, bytes: Vec<u8>) -> thread::JoinHandle<()> {
        let listener = UnixListener::bind(socket_path).expect("bind");
        thread::spawn(move || {
            let (mut conn, _) = listener.accept().expect("accept");
            let mut discard = [0u8; 4096];
            conn.set_read_timeout(Some(Duration::from_millis(200))).ok();
            let _ = conn.read(&mut discard);
            conn.write_all(&bytes).expect("write");
        })
    }

    // ── Server closes connection without writing response (lines 143-146) ─────

    #[test]
    fn call_returns_error_when_server_closes_connection() {
        let dir = tempdir().unwrap();
        let socket_path = dir.path().join("nvim_eof.sock");
        let listener = UnixListener::bind(&socket_path).expect("bind");
        let _server = thread::spawn(move || {
            let (conn, _) = listener.accept().expect("accept");
            // Immediately drop — closes the connection.
            drop(conn);
        });
        thread::sleep(Duration::from_millis(50));
        let ep = Endpoint { path: socket_path };
        let mut transport = Transport::connect(&ep).unwrap();
        let err = transport
            .call("nvim_test", &[], Duration::from_secs(2))
            .unwrap_err();
        // Expect either Io or Timeout (timing-dependent).
        assert!(
            matches!(err, TransportError::Io(_) | TransportError::Timeout),
            "unexpected error: {err:?}"
        );
    }

    // ── Server returns non-array value → BadFrame (line 184) ─────────────────

    #[test]
    fn call_returns_bad_frame_for_non_array_response() {
        let dir = tempdir().unwrap();
        let socket_path = dir.path().join("nvim_badframe1.sock");
        // Nil (0xc0) is a valid msgpack value but not an Array.
        let bytes = vec![0xc0u8];
        let _server = spawn_raw_server(&socket_path, bytes);
        thread::sleep(Duration::from_millis(50));
        let ep = Endpoint { path: socket_path };
        let mut transport = Transport::connect(&ep).unwrap();
        let err = transport
            .call("nvim_test", &[], Duration::from_secs(2))
            .unwrap_err();
        assert!(matches!(err, TransportError::BadFrame), "got: {err:?}");
    }

    // ── Server returns empty array → BadFrame (line 188) ─────────────────────

    #[test]
    fn call_returns_bad_frame_for_empty_array() {
        let dir = tempdir().unwrap();
        let socket_path = dir.path().join("nvim_badframe2.sock");
        // fixarray len=0
        let bytes = vec![0x90u8];
        let _server = spawn_raw_server(&socket_path, bytes);
        thread::sleep(Duration::from_millis(50));
        let ep = Endpoint { path: socket_path };
        let mut transport = Transport::connect(&ep).unwrap();
        let err = transport
            .call("nvim_test", &[], Duration::from_secs(2))
            .unwrap_err();
        assert!(matches!(err, TransportError::BadFrame), "got: {err:?}");
    }

    // ── Unknown msg_type → BadFrame (line 220) ───────────────────────────────

    #[test]
    fn call_returns_bad_frame_for_unknown_msg_type() {
        let dir = tempdir().unwrap();
        let socket_path = dir.path().join("nvim_badframe3.sock");
        // fixarray len=3, msg_type=99, "x", []
        let bytes = vec![0x93u8, 99, 0xa1, b'x', 0x90];
        let _server = spawn_raw_server(&socket_path, bytes);
        thread::sleep(Duration::from_millis(50));
        let ep = Endpoint { path: socket_path };
        let mut transport = Transport::connect(&ep).unwrap();
        let err = transport
            .call("nvim_test", &[], Duration::from_secs(2))
            .unwrap_err();
        assert!(matches!(err, TransportError::BadFrame), "got: {err:?}");
    }

    // ── Response frame with wrong msgid is discarded, then real response ──────

    #[test]
    fn call_discards_wrong_msgid_and_waits_for_correct() {
        let dir = tempdir().unwrap();
        let socket_path = dir.path().join("nvim_wrongid.sock");

        let expected = Value::Uint(77);

        // Frame with wrong msgid=99, then the correct one (msgid=1).
        let mut payload = Vec::new();
        // Wrong: [1, 99, nil, 0]
        payload.push(0x94); // fixarray len=4
        payload.push(1); // msg_type=1 (response)
        payload.push(99); // wrong msgid
        payload.push(0xc0); // nil
        payload.push(0); // result=0

        // Correct: make_response(1, &expected)
        let correct = make_response(1, &expected);
        payload.extend_from_slice(&correct);

        let listener = UnixListener::bind(&socket_path).expect("bind");
        let _server = thread::spawn(move || {
            let (mut conn, _) = listener.accept().expect("accept");
            let mut discard = [0u8; 4096];
            conn.set_read_timeout(Some(Duration::from_millis(200))).ok();
            let _ = conn.read(&mut discard);
            conn.write_all(&payload).expect("write");
        });

        thread::sleep(Duration::from_millis(50));
        let ep = Endpoint { path: socket_path };
        let mut transport = Transport::connect(&ep).unwrap();
        let result = transport
            .call("nvim_test", &[], Duration::from_secs(2))
            .unwrap();
        assert_eq!(result, expected);
    }

    // ── RpcError from server (line 214) ──────────────────────────────────────

    #[test]
    fn call_returns_rpc_error_when_server_sends_error_frame() {
        let dir = tempdir().unwrap();
        let socket_path = dir.path().join("nvim_rpcerr.sock");

        // Response with non-nil error: [1, 1, "oops", nil]
        let mut bytes = vec![0x94u8]; // fixarray len=4
        bytes.push(1); // msg_type=1
        bytes.push(1); // msgid=1
        bytes.push(0xa4); // fixstr len=4
        bytes.extend_from_slice(b"oops");
        bytes.push(0xc0); // result=nil

        let _server = spawn_raw_server(&socket_path, bytes);
        thread::sleep(Duration::from_millis(50));
        let ep = Endpoint { path: socket_path };
        let mut transport = Transport::connect(&ep).unwrap();
        let err = transport
            .call("nvim_test", &[], Duration::from_secs(2))
            .unwrap_err();
        assert!(matches!(err, TransportError::RpcError(_)), "got: {err:?}");
    }

    // ── Response frame with wrong number of items → BadFrame (line 201) ───────

    #[test]
    fn call_returns_bad_frame_for_malformed_response_length() {
        let dir = tempdir().unwrap();
        let socket_path = dir.path().join("nvim_badlen.sock");

        // Response type 1 but only 2 items: [1, 1] (needs 4).
        let bytes = vec![0x92u8, 1, 1];
        let _server = spawn_raw_server(&socket_path, bytes);
        thread::sleep(Duration::from_millis(50));
        let ep = Endpoint { path: socket_path };
        let mut transport = Transport::connect(&ep).unwrap();
        let err = transport
            .call("nvim_test", &[], Duration::from_secs(2))
            .unwrap_err();
        assert!(matches!(err, TransportError::BadFrame), "got: {err:?}");
    }
}
