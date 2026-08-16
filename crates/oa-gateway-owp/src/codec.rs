//! OWP 1.0 text-frame codec. Grammar follows OMSC-SPC-013; no UCI types.
//!
//! A frame is a keyword plus space-or-tab separated fields. JSON payloads
//! (`INIT`, `INFO`, `PUB`) keep their spaces: the first token is the
//! field, the rest of the line is the payload. This module does not
//! compile a schema or convert XML.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Whether `s` is an OWP identifier: `^[A-Za-z0-9_\-.]+$`.
///
/// Empty is not an identifier. Used for `topic`, `sid`, `service_id`,
/// and `group`. `SUB` `message_name` is only checked for non-empty.
#[must_use]
pub fn is_identifier(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-' || b == b'.')
}

/// Protocol error name carried on `-ERR`.
///
/// [`Display`](Self#impl-Display-for-OwpError) and [`FromStr`] use the
/// hyphenated wire tokens (`Unsupported-Version`), not the Rust variant
/// names. This is not a parse failure; those stay as `ParseError`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OwpError {
    UnsupportedVersion,
    UnsupportedSchema,
    UnsupportedService,
    IllegalOperation,
    IllegalArgument,
    IllegalState,
    InternalError,
    InvalidMessage,
}

impl fmt::Display for OwpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::UnsupportedVersion => "Unsupported-Version",
            Self::UnsupportedSchema => "Unsupported-Schema",
            Self::UnsupportedService => "Unsupported-Service",
            Self::IllegalOperation => "Illegal-Operation",
            Self::IllegalArgument => "Illegal-Argument",
            Self::IllegalState => "Illegal-State",
            Self::InternalError => "Internal-Error",
            Self::InvalidMessage => "Invalid-Message",
        })
    }
}

impl FromStr for OwpError {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "Unsupported-Version" => Ok(Self::UnsupportedVersion),
            "Unsupported-Schema" => Ok(Self::UnsupportedSchema),
            "Unsupported-Service" => Ok(Self::UnsupportedService),
            "Illegal-Operation" => Ok(Self::IllegalOperation),
            "Illegal-Argument" => Ok(Self::IllegalArgument),
            "Illegal-State" => Ok(Self::IllegalState),
            "Internal-Error" => Ok(Self::InternalError),
            "Invalid-Message" => Ok(Self::InvalidMessage),
            other => Err(ParseError::UnknownOp(other.to_owned())),
        }
    }
}

/// JSON body of a client `INIT` frame.
///
/// `versions` must be non-empty after parse. `service_id` must be an
/// identifier. `schema` is the protocol version string the server may
/// require to match; it is not a UCI XSD path.
#[derive(Debug, Clone, Deserialize)]
pub struct InitPayload {
    pub versions: Vec<String>,
    pub schema: String,
    pub verbose: Option<bool>,
    pub service_id: String,
}

/// UUID triple sent on `INFO`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Identifiers {
    pub system: String,
    pub service: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subsystem: Option<String>,
}

/// JSON body of a server `INFO` frame.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InfoPayload {
    pub version: String,
    pub server_id: String,
    pub uuids: Identifiers,
    pub system_label: String,
}

/// A parsed client frame.
///
/// `PUB` `payload` is the rest of the line, not a validated JSON
/// object. Call [`type_hint_from_json`] when the root key is needed.
#[derive(Debug, Clone)]
pub enum ClientOp {
    Init(InitPayload),
    /// `PUB <topic> <payload>`. `topic` is an identifier.
    Pub {
        topic: String,
        payload: String,
    },
    /// `SUB <sid> <message_name> <topic> [group]`.
    ///
    /// `message_name` is not identifier-checked. A fourth token is the
    /// optional group; a fifth is rejected as extra fields.
    Sub {
        sid: String,
        message_name: String,
        topic: String,
        group: Option<String>,
    },
    /// `UNSUB <sid>`.
    Unsub {
        sid: String,
    },
}

/// A parsed server frame, or one about to be written.
///
/// [`Display`](Self#impl-Display-for-ServerOp) is the wire form. `INFO`
/// JSON that cannot serialize becomes a `fmt` error, which should not
/// happen for a well-formed [`InfoPayload`].
#[derive(Debug, Clone)]
pub enum ServerOp {
    /// `+OK` with no trailing fields.
    Ok,
    /// `-ERR <name> [details…]`. Details keep their spaces.
    Err {
        error: OwpError,
        details: Option<String>,
    },
    Info(InfoPayload),
    /// `MSG <sid> <payload>`. Payload is the rest of the line.
    Msg {
        sid: String,
        payload: String,
    },
}

impl fmt::Display for ServerOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ok => f.write_str("+OK"),
            Self::Err { error, details } => match details {
                Some(d) => write!(f, "-ERR {error} {d}"),
                None => write!(f, "-ERR {error}"),
            },
            Self::Info(payload) => {
                let json = serde_json::to_string(payload).map_err(|_| fmt::Error)?;
                write!(f, "INFO {json}")
            }
            Self::Msg { sid, payload } => write!(f, "MSG {sid} {payload}"),
        }
    }
}

/// Why a frame could not be parsed.
///
/// Distinct from [`OwpError`], which is a protocol `-ERR` name inside a
/// well-formed frame.
#[derive(Debug)]
pub enum ParseError {
    EmptyFrame,
    UnknownOp(String),
    MissingField {
        op: &'static str,
        field: &'static str,
    },
    ExtraFields {
        op: &'static str,
    },
    InvalidIdentifier {
        op: &'static str,
        field: &'static str,
        value: String,
    },
    InvalidJson {
        op: &'static str,
        message: String,
    },
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyFrame => write!(f, "empty frame"),
            Self::UnknownOp(op) => write!(f, "unknown operation '{op}'"),
            Self::MissingField { op, field } => write!(f, "{op}: missing '{field}'"),
            Self::ExtraFields { op } => write!(f, "{op}: unexpected extra fields"),
            Self::InvalidIdentifier { op, field, value } => {
                write!(
                    f,
                    "{op}: '{field}' value '{value}' is not an OWP identifier"
                )
            }
            Self::InvalidJson { op, message } => write!(f, "{op}: invalid JSON: {message}"),
        }
    }
}

impl std::error::Error for ParseError {}

/// Space or tab. Newlines are not delimiters; a frame is one line.
const fn is_delim(b: u8) -> bool {
    b == b' ' || b == b'\t'
}

/// First non-delimiter run and the remainder, with leading delimiters
/// stripped from the remainder. `None` if `s` is only delimiters.
fn split_first_token(s: &str) -> Option<(&str, &str)> {
    let bytes = s.as_bytes();
    let start = bytes.iter().position(|b| !is_delim(*b))?;
    let end = bytes[start..]
        .iter()
        .position(|b| is_delim(*b))
        .map_or(bytes.len(), |p| start + p);
    let token = std::str::from_utf8(&bytes[start..end]).expect("utf8");
    let rest_start = bytes[end..]
        .iter()
        .position(|b| !is_delim(*b))
        .map_or(bytes.len(), |p| end + p);
    let rest = std::str::from_utf8(&bytes[rest_start..]).expect("utf8");
    Some((token, rest))
}

/// Up to `max` tokens. Leftover text after `max` is dropped, so the
/// caller must treat `len == max` as “maybe more” when extra fields
/// are illegal.
fn tokenize(s: &str, max: usize) -> Vec<&str> {
    let mut tokens = Vec::with_capacity(max);
    let mut remaining = s;
    while tokens.len() < max {
        match split_first_token(remaining) {
            Some((token, rest)) => {
                tokens.push(token);
                remaining = rest;
            }
            None => break,
        }
    }
    tokens
}

/// Parses one client text frame (`INIT`, `PUB`, `SUB`, `UNSUB`).
///
/// Leading and internal space/tab runs are skipped between fields.
/// `PUB` payload is not parsed as JSON here.
///
/// # Errors
///
/// Returns a parse error if the frame is empty, the keyword is
/// unknown, a required field is missing, an identifier is illegal, or
/// `INIT` JSON is unusable.
pub fn parse_client(frame: &str) -> Result<ClientOp, ParseError> {
    let (keyword, rest) = split_first_token(frame).ok_or(ParseError::EmptyFrame)?;
    match keyword {
        "INIT" => parse_init(rest),
        "PUB" => parse_pub(rest),
        "SUB" => parse_sub(rest),
        "UNSUB" => parse_unsub(rest),
        other => Err(ParseError::UnknownOp(other.to_owned())),
    }
}

/// Parses one server text frame (`+OK`, `-ERR`, `INFO`, `MSG`).
///
/// # Errors
///
/// Returns a parse error if the frame is empty, the keyword is
/// unknown, `+OK` has trailing text, or a field is missing or illegal.
pub fn parse_server(frame: &str) -> Result<ServerOp, ParseError> {
    let (keyword, rest) = split_first_token(frame).ok_or(ParseError::EmptyFrame)?;
    match keyword {
        "+OK" => {
            if rest.is_empty() {
                Ok(ServerOp::Ok)
            } else {
                Err(ParseError::ExtraFields { op: "+OK" })
            }
        }
        "-ERR" => {
            let (name, details) = split_first_token(rest).ok_or(ParseError::MissingField {
                op: "-ERR",
                field: "error",
            })?;
            let error = OwpError::from_str(name)?;
            Ok(ServerOp::Err {
                error,
                details: if details.is_empty() {
                    None
                } else {
                    Some(details.to_owned())
                },
            })
        }
        "INFO" => {
            if rest.is_empty() {
                return Err(ParseError::MissingField {
                    op: "INFO",
                    field: "payload",
                });
            }
            let payload: InfoPayload =
                serde_json::from_str(rest).map_err(|e| ParseError::InvalidJson {
                    op: "INFO",
                    message: e.to_string(),
                })?;
            Ok(ServerOp::Info(payload))
        }
        "MSG" => {
            let (sid, payload) = split_first_token(rest).ok_or(ParseError::MissingField {
                op: "MSG",
                field: "sid",
            })?;
            if !is_identifier(sid) {
                return Err(ParseError::InvalidIdentifier {
                    op: "MSG",
                    field: "sid",
                    value: sid.to_owned(),
                });
            }
            if payload.is_empty() {
                return Err(ParseError::MissingField {
                    op: "MSG",
                    field: "payload",
                });
            }
            Ok(ServerOp::Msg {
                sid: sid.to_owned(),
                payload: payload.to_owned(),
            })
        }
        other => Err(ParseError::UnknownOp(other.to_owned())),
    }
}

fn parse_init(rest: &str) -> Result<ClientOp, ParseError> {
    if rest.is_empty() {
        return Err(ParseError::MissingField {
            op: "INIT",
            field: "payload",
        });
    }
    let payload: InitPayload = serde_json::from_str(rest).map_err(|e| ParseError::InvalidJson {
        op: "INIT",
        message: e.to_string(),
    })?;
    if payload.versions.is_empty() {
        return Err(ParseError::MissingField {
            op: "INIT",
            field: "versions",
        });
    }
    if !is_identifier(&payload.service_id) {
        return Err(ParseError::InvalidIdentifier {
            op: "INIT",
            field: "service_id",
            value: payload.service_id,
        });
    }
    Ok(ClientOp::Init(payload))
}

fn parse_pub(rest: &str) -> Result<ClientOp, ParseError> {
    let (topic, payload) = split_first_token(rest).ok_or(ParseError::MissingField {
        op: "PUB",
        field: "topic",
    })?;
    if !is_identifier(topic) {
        return Err(ParseError::InvalidIdentifier {
            op: "PUB",
            field: "topic",
            value: topic.to_owned(),
        });
    }
    if payload.is_empty() {
        return Err(ParseError::MissingField {
            op: "PUB",
            field: "payload",
        });
    }
    Ok(ClientOp::Pub {
        topic: topic.to_owned(),
        payload: payload.to_owned(),
    })
}

fn parse_sub(rest: &str) -> Result<ClientOp, ParseError> {
    let tokens = tokenize(rest, 5);
    if tokens.len() < 3 {
        let missing = match tokens.len() {
            0 => "sid",
            1 => "message_name",
            2 => "topic",
            _ => unreachable!(),
        };
        return Err(ParseError::MissingField {
            op: "SUB",
            field: missing,
        });
    }
    if tokens.len() > 4 {
        return Err(ParseError::ExtraFields { op: "SUB" });
    }
    let sid = tokens[0];
    let message_name = tokens[1];
    let topic = tokens[2];
    let group = tokens.get(3).copied();
    for (field, value) in [("sid", sid), ("topic", topic)] {
        if !is_identifier(value) {
            return Err(ParseError::InvalidIdentifier {
                op: "SUB",
                field,
                value: value.to_owned(),
            });
        }
    }
    if let Some(g) = group {
        if !is_identifier(g) {
            return Err(ParseError::InvalidIdentifier {
                op: "SUB",
                field: "group",
                value: g.to_owned(),
            });
        }
    }
    if message_name.is_empty() {
        return Err(ParseError::MissingField {
            op: "SUB",
            field: "message_name",
        });
    }
    Ok(ClientOp::Sub {
        sid: sid.to_owned(),
        message_name: message_name.to_owned(),
        topic: topic.to_owned(),
        group: group.map(str::to_owned),
    })
}

fn parse_unsub(rest: &str) -> Result<ClientOp, ParseError> {
    let tokens = tokenize(rest, 2);
    if tokens.is_empty() {
        return Err(ParseError::MissingField {
            op: "UNSUB",
            field: "sid",
        });
    }
    if tokens.len() > 1 {
        return Err(ParseError::ExtraFields { op: "UNSUB" });
    }
    let sid = tokens[0];
    if !is_identifier(sid) {
        return Err(ParseError::InvalidIdentifier {
            op: "UNSUB",
            field: "sid",
            value: sid.to_owned(),
        });
    }
    Ok(ClientOp::Unsub {
        sid: sid.to_owned(),
    })
}

/// Returns the single root key of an OMS JSON object.
///
/// That key is the type hint the engine matches on. This does not
/// validate UCI and does not walk nested members.
///
/// # Errors
///
/// Returns a parse error (labelled as `PUB`) if `text` is not an
/// object with exactly one member.
pub fn type_hint_from_json(text: &str) -> Result<String, ParseError> {
    let value: Value = serde_json::from_str(text).map_err(|e| ParseError::InvalidJson {
        op: "PUB",
        message: e.to_string(),
    })?;
    let obj = value.as_object().ok_or(ParseError::InvalidJson {
        op: "PUB",
        message: "root must be a JSON object".into(),
    })?;
    if obj.len() != 1 {
        return Err(ParseError::InvalidJson {
            op: "PUB",
            message: "root object must have exactly one member".into(),
        });
    }
    Ok(obj.keys().next().expect("len == 1").clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_pub_and_type_hint() {
        let op = parse_client(r#"PUB demo.topic {"Ping":{"n":1}}"#).unwrap();
        match op {
            ClientOp::Pub { topic, payload } => {
                assert_eq!(topic, "demo.topic");
                assert_eq!(type_hint_from_json(&payload).unwrap(), "Ping");
            }
            _ => panic!("expected PUB"),
        }
    }

    #[test]
    fn parse_sub_with_group() {
        let op = parse_client("SUB s1 Ping demo.topic workers").unwrap();
        match op {
            ClientOp::Sub {
                sid,
                message_name,
                topic,
                group,
            } => {
                assert_eq!(sid, "s1");
                assert_eq!(message_name, "Ping");
                assert_eq!(topic, "demo.topic");
                assert_eq!(group.as_deref(), Some("workers"));
            }
            _ => panic!("expected SUB"),
        }
    }

    #[test]
    fn server_op_roundtrip_err() {
        let frame = ServerOp::Err {
            error: OwpError::InvalidMessage,
            details: Some("bad root".into()),
        }
        .to_string();
        match parse_server(&frame).unwrap() {
            ServerOp::Err { error, details } => {
                assert_eq!(error, OwpError::InvalidMessage);
                assert_eq!(details.as_deref(), Some("bad root"));
            }
            _ => panic!("expected ERR"),
        }
    }
}
