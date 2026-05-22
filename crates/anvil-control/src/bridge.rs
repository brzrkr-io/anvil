//! Typed native<->web IPC message protocol for the embedded webview.
//! JSON strings cross the bridge both directions. This module owns the whole
//! message catalog: encode (native -> web) and decode (web -> native).
//!
//! Wire format is byte-compatible with `src/ipc/bridge.zig`. The discriminator
//! key is `"type"` on every message in both directions.

use serde::{Deserialize, Serialize};
use thiserror::Error;

// --- web -> native -----------------------------------------------------------

/// A message posted by the web surface.
#[derive(Debug, PartialEq)]
pub enum Inbound {
    Ready,
    Invoke(String),
    Dismiss,
}

#[derive(Debug, Error, PartialEq)]
pub enum DecodeError {
    #[error("invalid JSON: {0}")]
    InvalidJson(String),
    #[error("unknown message type")]
    UnknownType,
    #[error("missing required field")]
    MissingField,
}

/// Private wire struct matching the Zig `Wire` struct in `bridge.zig`.
#[derive(Deserialize)]
struct InboundWire {
    #[serde(rename = "type")]
    kind: String,
    id: Option<String>,
}

/// Parse a JSON message from the web surface.
pub fn decode(json: &str) -> Result<Inbound, DecodeError> {
    let wire: InboundWire =
        serde_json::from_str(json).map_err(|e| DecodeError::InvalidJson(e.to_string()))?;
    match wire.kind.as_str() {
        "ready" => Ok(Inbound::Ready),
        "dismiss" => Ok(Inbound::Dismiss),
        "invoke" => {
            let id = wire.id.ok_or(DecodeError::MissingField)?;
            Ok(Inbound::Invoke(id))
        }
        _ => Err(DecodeError::UnknownType),
    }
}

// --- native -> web -----------------------------------------------------------

/// One selectable command shown in the palette.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Command {
    pub id: String,
    pub title: String,
    /// Always serialized (even as `null`) to match the Zig wire format.
    pub subtitle: Option<String>,
}

/// The theme colors the web surface needs to match the terminal. Hex strings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThemeTokens {
    pub background: String,
    pub foreground: String,
    pub accent: String,
}

/// A message sent to the web surface.
#[derive(Debug, PartialEq)]
pub enum Outbound {
    Show {
        commands: Vec<Command>,
        theme: ThemeTokens,
    },
    Hide,
}

/// Private wire structs for outbound encoding.
#[derive(Serialize)]
struct OutboundShowWire<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    commands: &'a [Command],
    theme: &'a ThemeTokens,
}

/// Serialize an outbound message to a JSON string.
pub fn encode(msg: &Outbound) -> Result<String, serde_json::Error> {
    match msg {
        Outbound::Hide => Ok(r#"{"type":"hide"}"#.to_string()),
        Outbound::Show { commands, theme } => {
            let wire = OutboundShowWire {
                kind: "show",
                commands,
                theme,
            };
            serde_json::to_string(&wire)
        }
    }
}

// --- tests -------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_ready() {
        let msg = decode(r#"{"type":"ready"}"#).unwrap();
        assert_eq!(msg, Inbound::Ready);
    }

    #[test]
    fn decode_dismiss() {
        let msg = decode(r#"{"type":"dismiss"}"#).unwrap();
        assert_eq!(msg, Inbound::Dismiss);
    }

    #[test]
    fn decode_invoke_carries_the_command_id() {
        let msg = decode(r#"{"type":"invoke","id":"theme.dark"}"#).unwrap();
        assert_eq!(msg, Inbound::Invoke("theme.dark".to_string()));
    }

    #[test]
    fn decode_ignores_unknown_fields() {
        let msg = decode(r#"{"type":"ready","extra":99}"#).unwrap();
        assert_eq!(msg, Inbound::Ready);
    }

    #[test]
    fn decode_invoke_without_id_fails() {
        let err = decode(r#"{"type":"invoke"}"#).unwrap_err();
        assert_eq!(err, DecodeError::MissingField);
    }

    #[test]
    fn decode_unknown_type_fails() {
        let err = decode(r#"{"type":"banana"}"#).unwrap_err();
        assert_eq!(err, DecodeError::UnknownType);
    }

    #[test]
    fn decode_malformed_json_fails() {
        let err = decode("{not json").unwrap_err();
        assert!(matches!(err, DecodeError::InvalidJson(_)));
    }

    #[test]
    fn encode_hide() {
        let json = encode(&Outbound::Hide).unwrap();
        assert_eq!(json, r#"{"type":"hide"}"#);
    }

    #[test]
    fn encode_show() {
        let msg = Outbound::Show {
            commands: vec![Command {
                id: "x".to_string(),
                title: "X".to_string(),
                subtitle: None,
            }],
            theme: ThemeTokens {
                background: "#000000".to_string(),
                foreground: "#ffffff".to_string(),
                accent: "#2f7f86".to_string(),
            },
        };
        let json = encode(&msg).unwrap();
        assert_eq!(
            json,
            concat!(
                r#"{"type":"show","commands":[{"id":"x","title":"X","subtitle":null}],"#,
                r##""theme":{"background":"#000000","foreground":"#ffffff","accent":"#2f7f86"}}"##,
            )
        );
    }

    #[test]
    fn encode_show_emits_subtitle_when_present() {
        let msg = Outbound::Show {
            commands: vec![Command {
                id: "x".to_string(),
                title: "X".to_string(),
                subtitle: Some("hint".to_string()),
            }],
            theme: ThemeTokens {
                background: "#000000".to_string(),
                foreground: "#ffffff".to_string(),
                accent: "#2f7f86".to_string(),
            },
        };
        let json = encode(&msg).unwrap();
        assert!(json.contains("\"subtitle\":\"hint\""));
    }
}
