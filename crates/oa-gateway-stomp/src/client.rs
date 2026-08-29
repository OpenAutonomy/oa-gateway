//! TCP STOMP session: handshake, framed read/write.
//!
//! Heartbeats are advertised as `0,0` (none). This crate does not send
//! or expect them. `ack:auto` means the broker does not wait for ACK.

use oa_gateway_adapter::tls::MaybeTlsStream;
use tokio::io::{AsyncReadExt, AsyncWriteExt, ReadHalf, WriteHalf};
use tokio::net::TcpStream;
use tokio::time::timeout;

use crate::codec::{decode_one_with_limit, CodecError, Frame};
use crate::config::StompConfig;

/// The socket a STOMP session runs over, TLS or not.
type Stream = MaybeTlsStream<TcpStream>;

/// Write half of a connected STOMP socket.
pub struct FrameWriter {
    write: WriteHalf<Stream>,
}

impl FrameWriter {
    /// Encodes `frame` and flushes it.
    ///
    /// # Errors
    ///
    /// Returns [`CodecError::Io`] if the write or flush fails.
    pub async fn send(&mut self, frame: &Frame) -> Result<(), CodecError> {
        self.write.write_all(&frame.encode()).await?;
        self.write.flush().await?;
        Ok(())
    }
}

/// Read half of a connected STOMP socket.
///
/// Bytes accumulate in an internal buffer until
/// [`decode_one_with_limit`] can take one frame. The cap is
/// [`StompConfig::max_frame_size`].
pub struct FrameReader {
    read: ReadHalf<Stream>,
    buf: Vec<u8>,
    max_frame_size: usize,
}

impl FrameReader {
    /// Next complete frame, or `None` if the broker closed the socket.
    ///
    /// # Errors
    ///
    /// Returns a codec error if the buffer exceeds the size cap or a
    /// frame is malformed.
    pub async fn recv(&mut self) -> Result<Option<Frame>, CodecError> {
        loop {
            if let Some(frame) = decode_one_with_limit(&mut self.buf, self.max_frame_size)? {
                return Ok(Some(frame));
            }
            let mut tmp = [0u8; 8192];
            let n = self.read.read(&mut tmp).await?;
            if n == 0 {
                return Ok(None);
            }
            self.buf.extend_from_slice(&tmp[..n]);
        }
    }
}

/// Opens TCP, optionally negotiates TLS, sends CONNECT, and waits for
/// CONNECTED.
///
/// TCP, the TLS handshake when [`StompConfig::tls`] is set, and CONNECTED
/// each use [`StompConfig::connect_timeout`]. `login` is omitted when
/// unset; `passcode` is sent only when `login` is set. Nagle is disabled
/// when the socket allows it.
///
/// # Errors
///
/// Returns [`CodecError::Io`] on timeout, a refused connect, a failed TLS
/// handshake, a broker ERROR, a close before CONNECTED, or any other
/// first frame.
pub async fn connect(config: &StompConfig) -> Result<(FrameReader, FrameWriter), CodecError> {
    let stream = timeout(config.connect_timeout, TcpStream::connect(config.broker))
        .await
        .map_err(|_| CodecError::Io(format!("connect timed out ({})", config.broker)))?
        .map_err(CodecError::from)?;
    stream.set_nodelay(true).ok();

    let stream: Stream = match &config.tls {
        Some(tls) => timeout(config.connect_timeout, tls.connect(stream))
            .await
            .map_err(|_| CodecError::Io(format!("tls handshake timed out ({})", config.broker)))?
            .map_err(|err| {
                CodecError::Io(format!("tls handshake failed ({}): {err}", config.broker))
            })?,
        None => MaybeTlsStream::Plain(stream),
    };

    let (read, write) = tokio::io::split(stream);
    let mut reader = FrameReader {
        read,
        buf: Vec::new(),
        max_frame_size: config.max_frame_size,
    };
    let mut writer = FrameWriter { write };

    let mut connect = Frame::new("CONNECT")
        .with_header("accept-version", "1.2")
        .with_header("host", config.host.clone())
        .with_header("heart-beat", "0,0");
    if let Some(login) = &config.login {
        connect = connect.with_header("login", login);
        if let Some(passcode) = &config.passcode {
            connect = connect.with_header("passcode", passcode.expose());
        }
    }
    writer.send(&connect).await?;

    let frame = timeout(config.connect_timeout, reader.recv())
        .await
        .map_err(|_| CodecError::Io("STOMP CONNECTED timed out".into()))??;
    match frame {
        Some(f) if f.command == "CONNECTED" => Ok((reader, writer)),
        Some(f) if f.command == "ERROR" => {
            let msg = f
                .header("message")
                .unwrap_or_else(|| std::str::from_utf8(&f.body).unwrap_or("ERROR"));
            Err(CodecError::Io(format!("STOMP ERROR: {msg}")))
        }
        Some(f) => Err(CodecError::Io(format!(
            "expected CONNECTED, got {}",
            f.command
        ))),
        None => Err(CodecError::Io("broker closed during CONNECT".into())),
    }
}

/// SUBSCRIBE with `ack:auto`. The broker will not wait for ACK.
pub fn subscribe_frame(id: &str, destination: &str) -> Frame {
    Frame::new("SUBSCRIBE")
        .with_header("id", id)
        .with_header("destination", destination)
        .with_header("ack", "auto")
}

/// SEND to `destination`. Extra `headers` are appended as-is.
pub fn send_frame(destination: &str, headers: Vec<(String, String)>, body: Vec<u8>) -> Frame {
    let mut frame = Frame::new("SEND").with_header("destination", destination);
    frame.headers.extend(headers);
    frame.body = body;
    frame
}

/// DISCONNECT with no `receipt`. The writer does not wait for a reply.
pub fn disconnect_frame() -> Frame {
    Frame::new("DISCONNECT")
}
