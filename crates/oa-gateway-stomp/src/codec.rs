//! STOMP 1.2 frames. Text commands and headers; opaque body bytes.
//!
//! A frame is `COMMAND\nheaders\n\nbody\0`. Headers may be LF or CRLF.
//! Leading `\n`, `\r`, and NUL are heartbeats and are discarded.

use std::fmt;

/// Largest single frame (header block plus body) accepted from a peer.
///
/// Both bounds this implies matter: a peer that never terminates a frame would
/// otherwise grow the read buffer without limit, and a peer-supplied
/// `content-length` must never be trusted as an unbounded buffer offset.
pub const DEFAULT_MAX_FRAME_SIZE: usize = 16 * 1024 * 1024;

/// One STOMP frame (command + headers + body).
///
/// Headers are a list, not a map: duplicates are kept, and
/// [`Self::header`] returns the first match. The body is not UTF-8
/// checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub command: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Frame {
    /// An empty frame with this command and no headers or body.
    #[must_use]
    pub fn new(command: impl Into<String>) -> Self {
        Self {
            command: command.into(),
            headers: Vec::new(),
            body: Vec::new(),
        }
    }

    /// Appends a header. Does not replace an existing name.
    #[must_use]
    pub fn with_header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }

    /// Replaces the body. Does not set `content-length`; [`Self::encode`]
    /// adds that if the headers do not already name it.
    #[must_use]
    pub fn with_body(mut self, body: impl Into<Vec<u8>>) -> Self {
        self.body = body.into();
        self
    }

    /// First header value whose name equals `name`. Case-sensitive.
    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    /// Encodes command, headers, optional `content-length`, body, and a
    /// trailing NUL.
    ///
    /// Header names and values are escaped (`\\`, `\\c`, `\\n`, `\\r`).
    /// `content-length` is added only when the headers do not already
    /// include it, so a caller can lie about the length if it wants to.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out =
            Vec::with_capacity(self.command.len() + self.body.len() + self.headers.len() * 16 + 32);
        out.extend_from_slice(self.command.as_bytes());
        out.push(b'\n');
        let mut has_len = false;
        for (k, v) in &self.headers {
            if k == "content-length" {
                has_len = true;
            }
            out.extend_from_slice(escape_header(k).as_bytes());
            out.push(b':');
            out.extend_from_slice(escape_header(v).as_bytes());
            out.push(b'\n');
        }
        if !has_len {
            out.extend_from_slice(b"content-length:");
            out.extend_from_slice(self.body.len().to_string().as_bytes());
            out.push(b'\n');
        }
        out.push(b'\n');
        out.extend_from_slice(&self.body);
        out.push(0);
        out
    }
}

/// Why a frame could not be decoded, or why a write failed.
///
/// [`Self::Io`] also covers handshake failures (timeout, unexpected
/// command), not only `std::io::Error`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CodecError {
    #[error("STOMP frame is not valid UTF-8 in command or headers")]
    NotUtf8,
    #[error("STOMP frame missing command")]
    MissingCommand,
    #[error("STOMP header is missing ':'")]
    BadHeader,
    #[error("STOMP header has invalid escape")]
    BadEscape,
    #[error("STOMP content-length is not a number")]
    BadContentLength,
    /// A NUL in the header block would be valid UTF-8 and would make
    /// the body offset ambiguous.
    #[error("STOMP header block contains a NUL byte")]
    NulInHeaders,
    #[error("STOMP frame exceeds the {max} byte limit")]
    FrameTooLarge { max: usize },
    #[error("STOMP frame missing NUL terminator")]
    MissingNull,
    #[error("I/O error: {0}")]
    Io(String),
}

impl From<std::io::Error> for CodecError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err.to_string())
    }
}

/// Pulls one complete frame from `buf`, rejecting frames larger than
/// [`DEFAULT_MAX_FRAME_SIZE`].
///
/// # Errors
///
/// Same as [`decode_one_with_limit`].
pub fn decode_one(buf: &mut Vec<u8>) -> Result<Option<Frame>, CodecError> {
    decode_one_with_limit(buf, DEFAULT_MAX_FRAME_SIZE)
}

/// Pulls one complete frame from `buf`.
///
/// Incomplete data returns `Ok(None)` and leaves `buf` intact aside from
/// leading heartbeat bytes (`\n` / `\r` / NUL). A complete frame is
/// drained from the front so two frames in one buffer can be read in
/// two calls.
///
/// Everything here is peer-controlled. `content-length` is bounded by
/// `max_frame_size` and added with checked arithmetic, so it can
/// neither wrap into a bogus offset nor address memory past the frame.
/// An unterminated header block or body is refused once it passes the
/// limit rather than buffered forever.
///
/// # Errors
///
/// Returns [`CodecError`] if the command or headers are not UTF-8, a
/// header is malformed, a NUL appears in the header block,
/// `content-length` is unusable, the NUL terminator is missing, or the
/// frame exceeds `max_frame_size`.
pub fn decode_one_with_limit(
    buf: &mut Vec<u8>,
    max_frame_size: usize,
) -> Result<Option<Frame>, CodecError> {
    let too_large = || CodecError::FrameTooLarge {
        max: max_frame_size,
    };

    let start = match buf.iter().position(|&b| b != b'\n' && b != b'\r' && b != 0) {
        Some(i) => i,
        None => {
            buf.clear();
            return Ok(None);
        }
    };
    if start > 0 {
        buf.drain(..start);
    }

    let header_end = match find_header_end(buf) {
        Some(end) => end,
        None => {
            if buf.len() > max_frame_size {
                return Err(too_large());
            }
            return Ok(None);
        }
    };
    if header_end > max_frame_size {
        return Err(too_large());
    }

    let header_bytes = &buf[..header_end];
    // A NUL is valid UTF-8, so it would otherwise survive header parsing and make the
    // frame boundary ambiguous.
    if header_bytes.contains(&0) {
        return Err(CodecError::NulInHeaders);
    }
    let header_text = std::str::from_utf8(header_bytes).map_err(|_| CodecError::NotUtf8)?;
    let mut lines = header_text.split('\n').map(|l| l.trim_end_matches('\r'));
    let command = lines.next().ok_or(CodecError::MissingCommand)?;
    if command.is_empty() {
        return Err(CodecError::MissingCommand);
    }
    let command = command.to_owned();

    let mut headers = Vec::new();
    let mut content_length: Option<usize> = None;
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let (raw_name, raw_value) = line.split_once(':').ok_or(CodecError::BadHeader)?;
        let name = unescape_header(raw_name)?;
        let value = unescape_header(raw_value)?;
        if name == "content-length" {
            content_length = Some(value.parse().map_err(|_| CodecError::BadContentLength)?);
        }
        headers.push((name, value));
    }

    let body_start = header_end;
    let (body_end, frame_end) = if let Some(len) = content_length {
        if len > max_frame_size {
            return Err(too_large());
        }
        let end = body_start.checked_add(len).ok_or_else(too_large)?;
        if buf.len() <= end {
            return Ok(None);
        }
        if buf[end] != 0 {
            return Err(CodecError::MissingNull);
        }
        (end, end + 1)
    } else {
        match buf[body_start..].iter().position(|&b| b == 0) {
            Some(rel) => (body_start + rel, body_start + rel + 1),
            None => {
                if buf.len() > max_frame_size {
                    return Err(too_large());
                }
                return Ok(None);
            }
        }
    };

    let body = buf[body_start..body_end].to_vec();
    buf.drain(..frame_end);

    Ok(Some(Frame {
        command,
        headers,
        body,
    }))
}

/// Byte offset after the blank line that ends the header block (`\n\n`
/// or `\r\n\r\n`).
fn find_header_end(buf: &[u8]) -> Option<usize> {
    let mut i = 0;
    while i + 1 < buf.len() {
        if buf[i] == b'\n' && buf[i + 1] == b'\n' {
            return Some(i + 2);
        }
        if i + 3 < buf.len()
            && buf[i] == b'\r'
            && buf[i + 1] == b'\n'
            && buf[i + 2] == b'\r'
            && buf[i + 3] == b'\n'
        {
            return Some(i + 4);
        }
        i += 1;
    }
    None
}

/// STOMP 1.2 header escapes: `\\`, `\\c` (`:`), `\\n`, `\\r`.
fn escape_header(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            ':' => out.push_str("\\c"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            other => out.push(other),
        }
    }
    out
}

/// Inverse of [`escape_header`].
///
/// # Errors
///
/// Returns [`CodecError::BadEscape`] if a `\\` is not followed by
/// `\\`, `c`, `n`, or `r`.
fn unescape_header(s: &str) -> Result<String, CodecError> {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('\\') => out.push('\\'),
            Some('c') => out.push(':'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            _ => return Err(CodecError::BadEscape),
        }
    }
    Ok(out)
}

/// The command only. Use [`Frame::encode`] for the wire form.
impl fmt::Display for Frame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.command)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_send_with_body() {
        let frame = Frame::new("SEND")
            .with_header("destination", "/topic/demo")
            .with_header("content-type", "application/json")
            .with_body(br#"{"Ping":{"n":1}}"#.to_vec());
        let mut buf = frame.encode();
        let decoded = decode_one(&mut buf).unwrap().unwrap();
        assert!(buf.is_empty());
        assert_eq!(decoded.command, "SEND");
        assert_eq!(decoded.header("destination"), Some("/topic/demo"));
        assert_eq!(decoded.body, br#"{"Ping":{"n":1}}"#);
    }

    #[test]
    fn header_escaping_colon_and_backslash() {
        let frame = Frame::new("SEND")
            .with_header("destination", "/topic/demo")
            .with_header("note", "a:b\\c");
        let mut buf = frame.encode();
        let decoded = decode_one(&mut buf).unwrap().unwrap();
        assert_eq!(decoded.header("note"), Some("a:b\\c"));
    }

    #[test]
    fn crlf_headers_and_no_content_length() {
        let raw =
            b"MESSAGE\r\ndestination:/topic/demo\r\nsubscription:s1\r\nmessage-id:1\r\n\r\nhello\0";
        let mut buf = raw.to_vec();
        let decoded = decode_one(&mut buf).unwrap().unwrap();
        assert_eq!(decoded.command, "MESSAGE");
        assert_eq!(decoded.header("destination"), Some("/topic/demo"));
        assert_eq!(decoded.body, b"hello");
        assert!(buf.is_empty());
    }

    #[test]
    fn leading_heartbeats_are_skipped() {
        let frame = Frame::new("CONNECTED").with_header("version", "1.2");
        let mut buf = vec![b'\n', b'\n', 0];
        buf.extend(frame.encode());
        let decoded = decode_one(&mut buf).unwrap().unwrap();
        assert_eq!(decoded.command, "CONNECTED");
    }

    #[test]
    fn incomplete_frame_waits() {
        let frame = Frame::new("SEND")
            .with_header("destination", "/topic/x")
            .with_body(vec![1, 2, 3, 4, 5]);
        let encoded = frame.encode();
        let mut buf = encoded[..encoded.len() - 3].to_vec();
        assert_eq!(decode_one(&mut buf).unwrap(), None);
        buf.extend_from_slice(&encoded[encoded.len() - 3..]);
        let decoded = decode_one(&mut buf).unwrap().unwrap();
        assert_eq!(decoded.body, vec![1, 2, 3, 4, 5]);
    }

    #[test]
    fn absurd_content_length_is_refused_not_added() {
        let mut buf =
            b"MESSAGE\ndestination:/topic/demo\ncontent-length:18446744073709551615\n\n".to_vec();
        assert_eq!(
            decode_one(&mut buf),
            Err(CodecError::FrameTooLarge {
                max: DEFAULT_MAX_FRAME_SIZE
            })
        );
    }

    #[test]
    fn nul_in_header_value_is_refused() {
        // A NUL is valid UTF-8, so without this check it reaches the body-offset
        // arithmetic and can point body_end before body_start.
        let mut buf =
            b"MESSAGE\ndestination:/topic/demo\nx:\0\ncontent-length:4\n\nbody\0".to_vec();
        assert_eq!(decode_one(&mut buf), Err(CodecError::NulInHeaders));
    }

    #[test]
    fn content_length_beyond_limit_is_refused() {
        let mut buf = b"MESSAGE\ndestination:/topic/demo\ncontent-length:5000\n\n".to_vec();
        assert_eq!(
            decode_one_with_limit(&mut buf, 1024),
            Err(CodecError::FrameTooLarge { max: 1024 })
        );
    }

    #[test]
    fn unterminated_header_block_stops_buffering() {
        let mut buf = b"MESSAGE\n".to_vec();
        buf.extend(std::iter::repeat_n(b'a', 200));
        assert_eq!(
            decode_one_with_limit(&mut buf, 64),
            Err(CodecError::FrameTooLarge { max: 64 })
        );
    }

    #[test]
    fn unterminated_body_stops_buffering() {
        let mut buf = b"SEND\ndestination:/topic/demo\n\n".to_vec();
        buf.extend(std::iter::repeat_n(b'a', 200));
        assert_eq!(
            decode_one_with_limit(&mut buf, 64),
            Err(CodecError::FrameTooLarge { max: 64 })
        );
    }

    #[test]
    fn frame_at_the_limit_still_decodes() {
        let frame = Frame::new("SEND")
            .with_header("destination", "/topic/demo")
            .with_body(b"hello".to_vec());
        let mut buf = frame.encode();
        let limit = buf.len();
        let decoded = decode_one_with_limit(&mut buf, limit).unwrap().unwrap();
        assert_eq!(decoded.body, b"hello");
        assert!(buf.is_empty());
    }

    #[test]
    fn two_frames_in_one_buffer() {
        let a = Frame::new("CONNECTED").with_header("version", "1.2");
        let b = Frame::new("MESSAGE")
            .with_header("destination", "/topic/demo")
            .with_body(b"x".to_vec());
        let mut buf = a.encode();
        buf.extend(b.encode());
        assert_eq!(decode_one(&mut buf).unwrap().unwrap().command, "CONNECTED");
        assert_eq!(decode_one(&mut buf).unwrap().unwrap().body, b"x");
        assert!(buf.is_empty());
    }
}
