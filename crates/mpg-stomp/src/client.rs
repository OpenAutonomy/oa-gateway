//! TCP STOMP session: handshake, framed read/write.

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::TcpStream;
use tokio::time::timeout;

use crate::codec::{decode_one, CodecError, Frame};
use crate::config::StompConfig;

pub struct FrameWriter {
    write: OwnedWriteHalf,
}

impl FrameWriter {
    pub async fn send(&mut self, frame: &Frame) -> Result<(), CodecError> {
        self.write.write_all(&frame.encode()).await?;
        self.write.flush().await?;
        Ok(())
    }
}

pub struct FrameReader {
    read: OwnedReadHalf,
    buf: Vec<u8>,
}

impl FrameReader {
    /// Next frame, or `None` on EOF.
    pub async fn recv(&mut self) -> Result<Option<Frame>, CodecError> {
        loop {
            if let Some(frame) = decode_one(&mut self.buf)? {
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

pub async fn connect(config: &StompConfig) -> Result<(FrameReader, FrameWriter), CodecError> {
    let stream = timeout(config.connect_timeout, TcpStream::connect(config.broker))
        .await
        .map_err(|_| CodecError::Io(format!("connect timed out ({})", config.broker)))?
        .map_err(CodecError::from)?;
    stream.set_nodelay(true).ok();

    let (read, write) = stream.into_split();
    let mut reader = FrameReader {
        read,
        buf: Vec::new(),
    };
    let mut writer = FrameWriter { write };

    let mut connect = Frame::new("CONNECT")
        .with_header("accept-version", "1.2")
        .with_header("host", config.host.clone())
        .with_header("heart-beat", "0,0");
    if let Some(login) = &config.login {
        connect = connect.with_header("login", login);
        if let Some(passcode) = &config.passcode {
            connect = connect.with_header("passcode", passcode);
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

pub fn subscribe_frame(id: &str, destination: &str) -> Frame {
    Frame::new("SUBSCRIBE")
        .with_header("id", id)
        .with_header("destination", destination)
        .with_header("ack", "auto")
}

pub fn send_frame(destination: &str, headers: Vec<(String, String)>, body: Vec<u8>) -> Frame {
    let mut frame = Frame::new("SEND").with_header("destination", destination);
    frame.headers.extend(headers);
    frame.body = body;
    frame
}

pub fn disconnect_frame() -> Frame {
    Frame::new("DISCONNECT")
}
