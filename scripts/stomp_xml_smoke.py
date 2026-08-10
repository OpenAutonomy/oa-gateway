#!/usr/bin/env python3
"""Stdlib STOMP 1.2 smoke client for /topic/PositionReport (no pip deps).

  python3 scripts/stomp_xml_smoke.py send
  python3 scripts/stomp_xml_smoke.py recv

`send` publishes crates/mpg-testing/fixtures/PositionReport.xml.
`recv` prints the next MESSAGE. Together with `cargo run -p mpg -- config/asb.toml`,
SUB on OWP `PositionReport` / `PositionReport` to see broker → mpg → web.
Outbound XML is asserted by
`cargo test -p mpg-testing --test live_activemq -- --ignored`.
"""

from __future__ import annotations

import argparse
import pathlib
import socket
import sys

HOST = "127.0.0.1"
PORT = 61613
DEST = "/topic/PositionReport"
ROOT = pathlib.Path(__file__).resolve().parents[1]
FIXTURE = ROOT / "crates" / "mpg-testing" / "fixtures" / "PositionReport.xml"


def encode(command: str, headers: list[tuple[str, str]], body: bytes = b"") -> bytes:
    lines = [command.encode()]
    for k, v in headers:
        lines.append(f"{k}:{v}".encode())
    lines.append(f"content-length:{len(body)}".encode())
    lines.append(b"")
    return b"\n".join(lines) + b"\n" + body + b"\0"


def decode_one(buf: bytearray) -> dict | None:
    while buf and buf[0] in (0, 10, 13):
        buf.pop(0)
    header_end = buf.find(b"\n\n")
    crlf_end = buf.find(b"\r\n\r\n")
    sep = None
    skip = 0
    if header_end >= 0 and (crlf_end < 0 or header_end <= crlf_end):
        sep, skip = header_end, 2
    elif crlf_end >= 0:
        sep, skip = crlf_end, 4
    if sep is None:
        return None
    header_text = bytes(buf[:sep]).decode()
    lines = [ln.rstrip("\r") for ln in header_text.split("\n")]
    command = lines[0]
    headers: dict[str, str] = {}
    content_length = None
    for line in lines[1:]:
        if not line or ":" not in line:
            continue
        name, value = line.split(":", 1)
        headers[name] = value
        if name == "content-length":
            content_length = int(value)
    body_start = sep + skip
    if content_length is not None:
        end = body_start + content_length
        if len(buf) <= end:
            return None
        if buf[end] != 0:
            raise RuntimeError("STOMP frame missing NUL")
        body = bytes(buf[body_start:end])
        del buf[: end + 1]
    else:
        nul = bytes(buf[body_start:]).find(b"\0")
        if nul < 0:
            return None
        body = bytes(buf[body_start : body_start + nul])
        del buf[: body_start + nul + 1]
    return {"command": command, "headers": headers, "body": body}


class Client:
    def __init__(self) -> None:
        self.sock = socket.create_connection((HOST, PORT), timeout=10)
        self.sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
        self.buf = bytearray()
        self.send_frame(
            "CONNECT",
            [("accept-version", "1.2"), ("host", "/"), ("heart-beat", "0,0")],
        )
        frame = self.recv_frame()
        if frame["command"] != "CONNECTED":
            raise RuntimeError(f"expected CONNECTED, got {frame['command']}: {frame}")

    def send_frame(self, command: str, headers: list[tuple[str, str]], body: bytes = b"") -> None:
        self.sock.sendall(encode(command, headers, body))

    def recv_frame(self) -> dict:
        self.sock.settimeout(10)
        while True:
            frame = decode_one(self.buf)
            if frame is not None:
                return frame
            chunk = self.sock.recv(8192)
            if not chunk:
                raise RuntimeError("broker closed")
            self.buf.extend(chunk)

    def close(self) -> None:
        try:
            self.send_frame("DISCONNECT", [])
        finally:
            self.sock.close()


def cmd_send() -> None:
    body = FIXTURE.read_bytes()
    c = Client()
    c.send_frame(
        "SEND",
        [("destination", DEST), ("content-type", "application/xml")],
        body,
    )
    print(f"sent {len(body)} bytes to {DEST}", file=sys.stderr)
    c.close()


def cmd_recv() -> None:
    c = Client()
    c.send_frame(
        "SUBSCRIBE",
        [("id", "smoke-1"), ("destination", DEST), ("ack", "auto")],
    )
    print(f"subscribed {DEST}, waiting…", file=sys.stderr)
    frame = c.recv_frame()
    print(frame["command"])
    for k, v in frame["headers"].items():
        print(f"{k}: {v}")
    print()
    sys.stdout.buffer.write(frame["body"])
    if not frame["body"].endswith(b"\n"):
        print()
    c.close()


def main() -> None:
    p = argparse.ArgumentParser()
    p.add_argument("action", choices=("send", "recv"))
    args = p.parse_args()
    if args.action == "send":
        cmd_send()
    else:
        cmd_recv()


if __name__ == "__main__":
    main()
