//! STOMP 1.2 frames. Text commands and headers; opaque body bytes.

use std::fmt;

/// One STOMP frame (command + headers + body).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub command: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Frame {
    #[must_use]
    pub fn new(command: impl Into<String>) -> Self {
        Self {
            command: command.into(),
            headers: Vec::new(),
            body: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }

    #[must_use]
    pub fn with_body(mut self, body: impl Into<Vec<u8>>) -> Self {
        self.body = body.into();
        self
    }

    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    /// Encode command, headers, optional `content-length`, body, and trailing NUL.
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

/// Pull one complete frame from `buf`. Incomplete data returns `Ok(None)` and leaves `buf` intact
/// aside from leading heartbeat bytes (`\n` / `\r` / NUL).
pub fn decode_one(buf: &mut Vec<u8>) -> Result<Option<Frame>, CodecError> {
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
        None => return Ok(None),
    };

    let header_bytes = &buf[..header_end];
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
        let end = body_start + len;
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
            None => return Ok(None),
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
