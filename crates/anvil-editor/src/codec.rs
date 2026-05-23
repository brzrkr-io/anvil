//! Hand-rolled msgpack encoder/decoder for the nvim msgpack-RPC subset.
//!
//! Supported types on encode: nil, bool, i64, u64, f64, str, bin, array, map.
//! Supported types on decode: all of the above plus float32; ext tags return
//! `CodecError::UnsupportedType`.

use thiserror::Error;

/// Maximum recursion depth allowed when decoding nested arrays/maps.
const MAX_DEPTH: usize = 32;
/// Maximum number of elements in a single array or map (protects against OOM
/// from a hostile length field).
const MAX_COLLECTION_LEN: usize = 256 * 1024;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A msgpack value.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Nil,
    Bool(bool),
    Int(i64),
    Uint(u64),
    Float(f64),
    Str(String),
    Bin(Vec<u8>),
    Array(Vec<Value>),
    Map(Vec<(Value, Value)>),
}

/// Errors produced by the codec.
#[derive(Debug, Error)]
pub enum CodecError {
    #[error("unexpected end of input")]
    UnexpectedEof,
    #[error("recursion depth exceeded (max {MAX_DEPTH})")]
    DepthExceeded,
    #[error("collection length {0} exceeds limit ({MAX_COLLECTION_LEN})")]
    Oversize(usize),
    #[error("unsupported msgpack type tag: 0x{0:02x}")]
    UnsupportedType(u8),
    #[error("invalid UTF-8 in str field")]
    InvalidUtf8,
}

// ---------------------------------------------------------------------------
// Encode
// ---------------------------------------------------------------------------

/// Encode a msgpack-RPC request frame `[0, msgid, method, params]` and append
/// it to `w`.
pub fn encode_request(
    w: &mut Vec<u8>,
    msgid: u32,
    method: &str,
    params: &[Value],
) -> Result<(), CodecError> {
    // The frame is a 4-element fixarray.
    w.push(0x94); // fixarray len=4
    encode_value(w, &Value::Uint(0))?; // type = Request
    encode_value(w, &Value::Uint(msgid as u64))?;
    encode_value(w, &Value::Str(method.to_string()))?;
    // params is always an Array
    encode_value(w, &Value::Array(params.to_vec()))?;
    Ok(())
}

fn encode_value(w: &mut Vec<u8>, v: &Value) -> Result<(), CodecError> {
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
            } else if n <= 0xffff_ffff {
                w.push(0xce);
                w.extend_from_slice(&(n as u32).to_be_bytes());
            } else {
                w.push(0xcf);
                w.extend_from_slice(&n.to_be_bytes());
            }
        }

        Value::Int(n) => {
            let n = *n;
            if (-32..=127).contains(&n) {
                // positive fixint or negative fixint
                w.push(n as i8 as u8);
            } else if n >= i8::MIN as i64 && n <= i8::MAX as i64 {
                w.push(0xd0);
                w.push(n as i8 as u8);
            } else if n >= i16::MIN as i64 && n <= i16::MAX as i64 {
                w.push(0xd1);
                w.extend_from_slice(&(n as i16).to_be_bytes());
            } else if n >= i32::MIN as i64 && n <= i32::MAX as i64 {
                w.push(0xd2);
                w.extend_from_slice(&(n as i32).to_be_bytes());
            } else {
                w.push(0xd3);
                w.extend_from_slice(&n.to_be_bytes());
            }
        }

        Value::Float(f) => {
            // Always encode as float64.
            w.push(0xcb);
            w.extend_from_slice(&f.to_bits().to_be_bytes());
        }

        Value::Str(s) => {
            let bytes = s.as_bytes();
            let len = bytes.len();
            if len <= 31 {
                w.push(0xa0 | len as u8);
            } else if len <= 0xff {
                w.push(0xd9);
                w.push(len as u8);
            } else if len <= 0xffff {
                w.push(0xda);
                w.extend_from_slice(&(len as u16).to_be_bytes());
            } else {
                w.push(0xdb);
                w.extend_from_slice(&(len as u32).to_be_bytes());
            }
            w.extend_from_slice(bytes);
        }

        Value::Bin(b) => {
            let len = b.len();
            if len <= 0xff {
                w.push(0xc4);
                w.push(len as u8);
            } else if len <= 0xffff {
                w.push(0xc5);
                w.extend_from_slice(&(len as u16).to_be_bytes());
            } else {
                w.push(0xc6);
                w.extend_from_slice(&(len as u32).to_be_bytes());
            }
            w.extend_from_slice(b);
        }

        Value::Array(arr) => {
            let len = arr.len();
            if len <= 15 {
                w.push(0x90 | len as u8);
            } else if len <= 0xffff {
                w.push(0xdc);
                w.extend_from_slice(&(len as u16).to_be_bytes());
            } else {
                w.push(0xdd);
                w.extend_from_slice(&(len as u32).to_be_bytes());
            }
            for item in arr {
                encode_value(w, item)?;
            }
        }

        Value::Map(pairs) => {
            let len = pairs.len();
            if len <= 15 {
                w.push(0x80 | len as u8);
            } else if len <= 0xffff {
                w.push(0xde);
                w.extend_from_slice(&(len as u16).to_be_bytes());
            } else {
                w.push(0xdf);
                w.extend_from_slice(&(len as u32).to_be_bytes());
            }
            for (k, v) in pairs {
                encode_value(w, k)?;
                encode_value(w, v)?;
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Decode
// ---------------------------------------------------------------------------

/// Decode one msgpack value from `r`, advancing the slice past consumed bytes.
///
/// Enforces a hard depth cap of [`MAX_DEPTH`] and a max collection length of
/// [`MAX_COLLECTION_LEN`].
pub fn decode_value(r: &mut &[u8]) -> Result<Value, CodecError> {
    decode_value_depth(r, 0)
}

fn read_byte(r: &mut &[u8]) -> Result<u8, CodecError> {
    if r.is_empty() {
        return Err(CodecError::UnexpectedEof);
    }
    let b = r[0];
    *r = &r[1..];
    Ok(b)
}

fn read_bytes<'a>(r: &mut &'a [u8], n: usize) -> Result<&'a [u8], CodecError> {
    if r.len() < n {
        return Err(CodecError::UnexpectedEof);
    }
    let out = &r[..n];
    *r = &r[n..];
    Ok(out)
}

fn read_u8(r: &mut &[u8]) -> Result<u8, CodecError> {
    read_byte(r)
}
fn read_u16(r: &mut &[u8]) -> Result<u16, CodecError> {
    let b = read_bytes(r, 2)?;
    Ok(u16::from_be_bytes([b[0], b[1]]))
}
fn read_u32(r: &mut &[u8]) -> Result<u32, CodecError> {
    let b = read_bytes(r, 4)?;
    Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}
fn read_u64(r: &mut &[u8]) -> Result<u64, CodecError> {
    let b = read_bytes(r, 8)?;
    Ok(u64::from_be_bytes(b.try_into().unwrap()))
}
fn read_i8(r: &mut &[u8]) -> Result<i8, CodecError> {
    Ok(read_byte(r)? as i8)
}
fn read_i16(r: &mut &[u8]) -> Result<i16, CodecError> {
    let b = read_bytes(r, 2)?;
    Ok(i16::from_be_bytes([b[0], b[1]]))
}
fn read_i32(r: &mut &[u8]) -> Result<i32, CodecError> {
    let b = read_bytes(r, 4)?;
    Ok(i32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}
fn read_i64(r: &mut &[u8]) -> Result<i64, CodecError> {
    let b = read_bytes(r, 8)?;
    Ok(i64::from_be_bytes(b.try_into().unwrap()))
}

fn decode_value_depth(r: &mut &[u8], depth: usize) -> Result<Value, CodecError> {
    if depth > MAX_DEPTH {
        return Err(CodecError::DepthExceeded);
    }

    let tag = read_byte(r)?;

    match tag {
        // nil
        0xc0 => Ok(Value::Nil),
        // false / true
        0xc2 => Ok(Value::Bool(false)),
        0xc3 => Ok(Value::Bool(true)),
        // positive fixint 0x00..=0x7f
        0x00..=0x7f => Ok(Value::Uint(tag as u64)),
        // negative fixint 0xe0..=0xff
        0xe0..=0xff => Ok(Value::Int(tag as i8 as i64)),
        // uint8
        0xcc => Ok(Value::Uint(read_u8(r)? as u64)),
        // uint16
        0xcd => Ok(Value::Uint(read_u16(r)? as u64)),
        // uint32
        0xce => Ok(Value::Uint(read_u32(r)? as u64)),
        // uint64
        0xcf => Ok(Value::Uint(read_u64(r)?)),
        // int8
        0xd0 => Ok(Value::Int(read_i8(r)? as i64)),
        // int16
        0xd1 => Ok(Value::Int(read_i16(r)? as i64)),
        // int32
        0xd2 => Ok(Value::Int(read_i32(r)? as i64)),
        // int64
        0xd3 => Ok(Value::Int(read_i64(r)?)),
        // float32
        0xca => {
            let b = read_bytes(r, 4)?;
            let bits = u32::from_be_bytes([b[0], b[1], b[2], b[3]]);
            Ok(Value::Float(f32::from_bits(bits) as f64))
        }
        // float64
        0xcb => {
            let bits = read_u64(r)?;
            Ok(Value::Float(f64::from_bits(bits)))
        }
        // fixstr 0xa0..=0xbf
        0xa0..=0xbf => {
            let len = (tag & 0x1f) as usize;
            decode_str(r, len)
        }
        // str8
        0xd9 => {
            let len = read_u8(r)? as usize;
            decode_str(r, len)
        }
        // str16
        0xda => {
            let len = read_u16(r)? as usize;
            decode_str(r, len)
        }
        // str32
        0xdb => {
            let len = read_u32(r)? as usize;
            decode_str(r, len)
        }
        // bin8
        0xc4 => {
            let len = read_u8(r)? as usize;
            let data = read_bytes(r, len)?;
            Ok(Value::Bin(data.to_vec()))
        }
        // bin16
        0xc5 => {
            let len = read_u16(r)? as usize;
            let data = read_bytes(r, len)?;
            Ok(Value::Bin(data.to_vec()))
        }
        // bin32
        0xc6 => {
            let len = read_u32(r)? as usize;
            let data = read_bytes(r, len)?;
            Ok(Value::Bin(data.to_vec()))
        }
        // fixarray 0x90..=0x9f
        0x90..=0x9f => {
            let len = (tag & 0x0f) as usize;
            decode_array(r, len, depth)
        }
        // array16
        0xdc => {
            let len = read_u16(r)? as usize;
            decode_array(r, len, depth)
        }
        // array32
        0xdd => {
            let len = read_u32(r)? as usize;
            decode_array(r, len, depth)
        }
        // fixmap 0x80..=0x8f
        0x80..=0x8f => {
            let len = (tag & 0x0f) as usize;
            decode_map(r, len, depth)
        }
        // map16
        0xde => {
            let len = read_u16(r)? as usize;
            decode_map(r, len, depth)
        }
        // map32
        0xdf => {
            let len = read_u32(r)? as usize;
            decode_map(r, len, depth)
        }
        // ext types — fixext1/2/4/8/16, ext8/16/32 — not supported
        0xd4..=0xd8 | 0xc7..=0xc9 => Err(CodecError::UnsupportedType(tag)),
        // everything else
        other => Err(CodecError::UnsupportedType(other)),
    }
}

fn decode_str(r: &mut &[u8], len: usize) -> Result<Value, CodecError> {
    let bytes = read_bytes(r, len)?;
    let s = std::str::from_utf8(bytes).map_err(|_| CodecError::InvalidUtf8)?;
    Ok(Value::Str(s.to_string()))
}

fn decode_array(r: &mut &[u8], len: usize, depth: usize) -> Result<Value, CodecError> {
    if len > MAX_COLLECTION_LEN {
        return Err(CodecError::Oversize(len));
    }
    let mut arr = Vec::with_capacity(len.min(64));
    for _ in 0..len {
        arr.push(decode_value_depth(r, depth + 1)?);
    }
    Ok(Value::Array(arr))
}

fn decode_map(r: &mut &[u8], len: usize, depth: usize) -> Result<Value, CodecError> {
    if len > MAX_COLLECTION_LEN {
        return Err(CodecError::Oversize(len));
    }
    let mut pairs = Vec::with_capacity(len.min(64));
    for _ in 0..len {
        let k = decode_value_depth(r, depth + 1)?;
        let v = decode_value_depth(r, depth + 1)?;
        pairs.push((k, v));
    }
    Ok(Value::Map(pairs))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(v: &Value) -> Value {
        let mut buf = Vec::new();
        encode_value(&mut buf, v).unwrap();
        let mut r: &[u8] = &buf;
        decode_value(&mut r).unwrap()
    }

    #[test]
    fn nil_roundtrip() {
        assert_eq!(roundtrip(&Value::Nil), Value::Nil);
    }

    #[test]
    fn bool_roundtrip() {
        assert_eq!(roundtrip(&Value::Bool(true)), Value::Bool(true));
        assert_eq!(roundtrip(&Value::Bool(false)), Value::Bool(false));
    }

    #[test]
    fn positive_fixint_roundtrip() {
        for n in [0u64, 1, 42, 127] {
            assert_eq!(roundtrip(&Value::Uint(n)), Value::Uint(n));
        }
    }

    #[test]
    fn negative_fixint_roundtrip() {
        for n in [-1i64, -16, -32] {
            assert_eq!(roundtrip(&Value::Int(n)), Value::Int(n));
        }
    }

    #[test]
    fn uint_all_widths() {
        let cases = [
            (128u64, Value::Uint(128)),           // uint8
            (256u64, Value::Uint(256)),           // uint16
            (0x1_0000u64, Value::Uint(0x1_0000)), // uint32
            (u64::MAX, Value::Uint(u64::MAX)),    // uint64
        ];
        for (_, v) in &cases {
            assert_eq!(roundtrip(v), *v);
        }
    }

    #[test]
    fn int_all_widths() {
        let cases = [
            Value::Int(i8::MIN as i64),
            Value::Int(i16::MIN as i64),
            Value::Int(i32::MIN as i64),
            Value::Int(i64::MIN),
        ];
        for v in &cases {
            assert_eq!(roundtrip(v), *v);
        }
    }

    #[test]
    fn float64_roundtrip() {
        let v = Value::Float(std::f64::consts::PI);
        assert_eq!(roundtrip(&v), v);
    }

    #[test]
    fn float32_decode() {
        // Encode a raw float32 tag + bits, then decode.
        let f: f32 = 1.5;
        let mut buf = vec![0xcau8];
        buf.extend_from_slice(&f.to_bits().to_be_bytes());
        let mut r: &[u8] = &buf;
        let v = decode_value(&mut r).unwrap();
        match v {
            Value::Float(d) => assert!((d - 1.5f64).abs() < 1e-6),
            other => panic!("expected Float, got {other:?}"),
        }
    }

    #[test]
    fn fixstr_roundtrip() {
        let v = Value::Str("hello".to_string());
        assert_eq!(roundtrip(&v), v);
    }

    #[test]
    fn str8_roundtrip() {
        let s = "x".repeat(32); // len=32, needs str8
        let v = Value::Str(s);
        assert_eq!(roundtrip(&v), v);
    }

    #[test]
    fn str16_roundtrip() {
        let s = "y".repeat(256); // needs str16
        let v = Value::Str(s);
        assert_eq!(roundtrip(&v), v);
    }

    #[test]
    fn bin_roundtrip() {
        let v = Value::Bin(vec![0x01, 0x02, 0x03]);
        assert_eq!(roundtrip(&v), v);
    }

    #[test]
    fn fixarray_roundtrip() {
        let v = Value::Array(vec![Value::Uint(1), Value::Uint(2), Value::Uint(3)]);
        assert_eq!(roundtrip(&v), v);
    }

    #[test]
    fn array16_roundtrip() {
        let arr = (0u64..16).map(Value::Uint).collect();
        let v = Value::Array(arr);
        assert_eq!(roundtrip(&v), v);
    }

    #[test]
    fn fixmap_roundtrip() {
        let v = Value::Map(vec![(Value::Str("k".into()), Value::Uint(1))]);
        assert_eq!(roundtrip(&v), v);
    }

    #[test]
    fn map16_roundtrip() {
        let pairs: Vec<(Value, Value)> = (0u64..16)
            .map(|i| (Value::Uint(i), Value::Uint(i * 2)))
            .collect();
        let v = Value::Map(pairs);
        assert_eq!(roundtrip(&v), v);
    }

    #[test]
    fn nested_array_roundtrip() {
        let inner = Value::Array(vec![Value::Uint(1), Value::Bool(true)]);
        let outer = Value::Array(vec![inner, Value::Nil]);
        assert_eq!(roundtrip(&outer), outer);
    }

    #[test]
    fn nested_map_roundtrip() {
        let inner = Value::Map(vec![(Value::Str("x".into()), Value::Float(1.0))]);
        let outer = Value::Map(vec![(Value::Str("inner".into()), inner)]);
        assert_eq!(roundtrip(&outer), outer);
    }

    // --- error cases ---

    #[test]
    fn truncated_input_eof() {
        let mut r: &[u8] = &[];
        let err = decode_value(&mut r).unwrap_err();
        assert!(matches!(err, CodecError::UnexpectedEof));
    }

    #[test]
    fn truncated_uint16() {
        let mut r: &[u8] = &[0xcd, 0x01]; // uint16 needs 2 bytes, only 1 present
        let err = decode_value(&mut r).unwrap_err();
        assert!(matches!(err, CodecError::UnexpectedEof));
    }

    #[test]
    fn depth_exceeded() {
        // Build a 33-deep nested fixarray (each wraps the next).
        let mut buf = Vec::new();
        for _ in 0..33 {
            buf.push(0x91); // fixarray len=1
        }
        buf.push(0xc0); // nil at the bottom
        let mut r: &[u8] = &buf;
        let err = decode_value(&mut r).unwrap_err();
        assert!(matches!(err, CodecError::DepthExceeded));
    }

    #[test]
    fn oversize_array() {
        // array32 with len = MAX_COLLECTION_LEN + 1
        let too_many = (MAX_COLLECTION_LEN + 1) as u32;
        let mut buf = vec![0xdd];
        buf.extend_from_slice(&too_many.to_be_bytes());
        let mut r: &[u8] = &buf;
        let err = decode_value(&mut r).unwrap_err();
        assert!(matches!(err, CodecError::Oversize(_)));
    }

    #[test]
    fn oversize_map() {
        let too_many = (MAX_COLLECTION_LEN + 1) as u32;
        let mut buf = vec![0xdf];
        buf.extend_from_slice(&too_many.to_be_bytes());
        let mut r: &[u8] = &buf;
        let err = decode_value(&mut r).unwrap_err();
        assert!(matches!(err, CodecError::Oversize(_)));
    }

    #[test]
    fn unsupported_ext_type() {
        let mut r: &[u8] = &[0xd4]; // fixext1
        let err = decode_value(&mut r).unwrap_err();
        assert!(matches!(err, CodecError::UnsupportedType(0xd4)));
    }

    // --- request frame ---

    #[test]
    fn encode_decode_request_frame() {
        let params = vec![Value::Str("buf".into()), Value::Uint(42)];
        let mut buf = Vec::new();
        encode_request(&mut buf, 7, "nvim_get_current_buf", &params).unwrap();

        let mut r: &[u8] = &buf;
        let frame = decode_value(&mut r).unwrap();
        match frame {
            Value::Array(items) => {
                assert_eq!(items.len(), 4);
                assert_eq!(items[0], Value::Uint(0));
                assert_eq!(items[1], Value::Uint(7));
                assert_eq!(items[2], Value::Str("nvim_get_current_buf".into()));
                assert_eq!(
                    items[3],
                    Value::Array(vec![Value::Str("buf".into()), Value::Uint(42)])
                );
            }
            other => panic!("expected Array, got {other:?}"),
        }
    }

    // ── bin16 and bin32 decode paths ─────────────────────────────────────────

    #[test]
    fn bin16_roundtrip() {
        // bin16 requires len 256–65535.
        let b = vec![0xCDu8; 256];
        let v = Value::Bin(b);
        assert_eq!(roundtrip(&v), v);
    }

    #[test]
    fn decode_bin16_from_raw_bytes() {
        // Hand-craft a bin16 (0xc5) with len=3.
        let buf = vec![0xc5, 0x00, 0x03, 0x01, 0x02, 0x03];
        let mut r: &[u8] = &buf;
        let v = decode_value(&mut r).unwrap();
        assert_eq!(v, Value::Bin(vec![1, 2, 3]));
    }

    #[test]
    fn decode_bin32_from_raw_bytes() {
        // Hand-craft a bin32 (0xc6) with len=2.
        let buf = vec![0xc6, 0x00, 0x00, 0x00, 0x02, 0xAA, 0xBB];
        let mut r: &[u8] = &buf;
        let v = decode_value(&mut r).unwrap();
        assert_eq!(v, Value::Bin(vec![0xAA, 0xBB]));
    }

    #[test]
    fn decode_unsupported_type_other_branch() {
        // 0xc1 is reserved and unsupported.
        let buf = [0xc1u8];
        let mut r: &[u8] = &buf;
        let err = decode_value(&mut r).unwrap_err();
        assert!(matches!(err, CodecError::UnsupportedType(0xc1)));
    }
}
