//! A minimal client-side WebSocket, sufficient for the Chrome DevTools
//! Protocol and nothing more.
//!
//! Deliberately hand-rolled rather than pulled from a crate: the surface CDP
//! needs is small (text frames, no extensions, no TLS), and a tool people
//! install should not drag an async runtime behind it.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;
use sha1::{Digest, Sha1};

use crate::error::{Error, Result};

/// RFC 6455 section 1.3. Verified against the specification's own test vector
/// in `tests::rfc6455_accept_vector` — do not edit this from memory.
const GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

const OP_CONTINUATION: u8 = 0x0;
const OP_TEXT: u8 = 0x1;
const OP_BINARY: u8 = 0x2;
const OP_CLOSE: u8 = 0x8;
const OP_PING: u8 = 0x9;
const OP_PONG: u8 = 0xA;

/// Refuse to buffer a single message larger than this. CDP screenshot
/// responses are base64 and can be many megabytes, so the ceiling is
/// generous, but unbounded growth from a confused peer is not acceptable.
const MAX_MESSAGE: usize = 256 * 1024 * 1024;

/// Compute the `Sec-WebSocket-Accept` value for a given client key.
fn accept_for(key: &str) -> String {
    let mut h = Sha1::new();
    h.update(key.as_bytes());
    h.update(GUID.as_bytes());
    B64.encode(h.finalize())
}

fn random_bytes<const N: usize>() -> Result<[u8; N]> {
    let mut b = [0u8; N];
    getrandom::getrandom(&mut b).map_err(|e| Error::Protocol(format!("rng unavailable: {e}")))?;
    Ok(b)
}

/// Split a `ws://host:port/path` URL into its parts.
fn split_url(url: &str) -> Result<(String, u16, String)> {
    let rest = url
        .strip_prefix("ws://")
        .ok_or_else(|| Error::Protocol(format!("only ws:// is supported, got {url}")))?;
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) => (
            h.to_string(),
            p.parse::<u16>()
                .map_err(|_| Error::Protocol(format!("bad port in {url}")))?,
        ),
        None => (authority.to_string(), 80u16),
    };
    Ok((host, port, path.to_string()))
}

/// A connected client-side WebSocket.
#[derive(Debug)]
pub struct Ws {
    stream: TcpStream,
    buf: Vec<u8>,
}

impl Ws {
    /// Perform the HTTP upgrade and return a connected socket.
    pub fn connect(url: &str, timeout: Duration) -> Result<Ws> {
        let (host, port, path) = split_url(url)?;
        let stream = TcpStream::connect((host.as_str(), port))?;
        stream.set_read_timeout(Some(timeout))?;
        stream.set_write_timeout(Some(timeout))?;
        stream.set_nodelay(true)?;

        let key = B64.encode(random_bytes::<16>()?);
        let req = format!(
            "GET {path} HTTP/1.1\r\n\
             Host: {host}:{port}\r\n\
             Upgrade: websocket\r\n\
             Connection: Upgrade\r\n\
             Sec-WebSocket-Key: {key}\r\n\
             Sec-WebSocket-Version: 13\r\n\r\n"
        );
        let mut ws = Ws {
            stream,
            buf: Vec::with_capacity(8192),
        };
        ws.stream.write_all(req.as_bytes())?;
        ws.stream.flush()?;

        // Read until end of headers, keeping any frame bytes that spilled over.
        let head_end = loop {
            if let Some(i) = find_subslice(&ws.buf, b"\r\n\r\n") {
                break i;
            }
            if ws.buf.len() > 64 * 1024 {
                return Err(Error::Protocol("handshake headers too large".into()));
            }
            ws.fill()?;
        };
        let head = String::from_utf8_lossy(&ws.buf[..head_end]).to_string();
        ws.buf.drain(..head_end + 4);

        let mut lines = head.split("\r\n");
        let status = lines.next().unwrap_or_default();
        if !status.contains("101") {
            return Err(Error::Protocol(format!("upgrade refused: {status}")));
        }
        let got = lines
            .filter_map(|l| l.split_once(':'))
            .find(|(k, _)| k.trim().eq_ignore_ascii_case("sec-websocket-accept"))
            .map(|(_, v)| v.trim().to_string())
            .ok_or_else(|| Error::Protocol("no Sec-WebSocket-Accept header".into()))?;
        let want = accept_for(&key);
        if got != want {
            return Err(Error::Protocol(format!(
                "accept mismatch: got {got}, want {want}"
            )));
        }
        Ok(ws)
    }

    fn fill(&mut self) -> Result<()> {
        let mut chunk = [0u8; 65536];
        let n = self.stream.read(&mut chunk)?;
        if n == 0 {
            return Err(Error::Protocol("connection closed by peer".into()));
        }
        self.buf.extend_from_slice(&chunk[..n]);
        Ok(())
    }

    fn need(&mut self, n: usize) -> Result<()> {
        while self.buf.len() < n {
            self.fill()?;
        }
        Ok(())
    }

    /// Send a masked text frame.
    pub fn send_text(&mut self, text: &str) -> Result<()> {
        self.send_frame(OP_TEXT, text.as_bytes())
    }

    fn send_frame(&mut self, opcode: u8, payload: &[u8]) -> Result<()> {
        let mut out = Vec::with_capacity(payload.len() + 14);
        out.push(0x80 | opcode); // FIN set; we never fragment outbound
        let n = payload.len();
        if n < 126 {
            out.push(0x80 | n as u8);
        } else if n <= u16::MAX as usize {
            out.push(0x80 | 126);
            out.extend_from_slice(&(n as u16).to_be_bytes());
        } else {
            out.push(0x80 | 127);
            out.extend_from_slice(&(n as u64).to_be_bytes());
        }
        let mask = random_bytes::<4>()?;
        out.extend_from_slice(&mask);
        out.extend(payload.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
        self.stream.write_all(&out)?;
        self.stream.flush()?;
        Ok(())
    }

    /// Read the next complete text message, reassembling fragments and
    /// transparently answering pings.
    pub fn recv_text(&mut self) -> Result<String> {
        let mut parts: Vec<u8> = Vec::new();
        let mut assembling = false;
        loop {
            let (fin, opcode, data) = self.read_frame()?;
            match opcode {
                OP_CLOSE => return Err(Error::Protocol("peer sent close".into())),
                OP_PING => {
                    self.send_frame(OP_PONG, &data)?;
                }
                OP_PONG => {}
                OP_TEXT | OP_BINARY => {
                    parts = data;
                    assembling = true;
                    if fin {
                        return finish(parts);
                    }
                }
                OP_CONTINUATION => {
                    if !assembling {
                        return Err(Error::Protocol("continuation without start".into()));
                    }
                    if parts.len() + data.len() > MAX_MESSAGE {
                        return Err(Error::Protocol("message exceeds limit".into()));
                    }
                    parts.extend_from_slice(&data);
                    if fin {
                        return finish(parts);
                    }
                }
                other => {
                    return Err(Error::Protocol(format!("unknown opcode {other:#x}")));
                }
            }
        }
    }

    fn read_frame(&mut self) -> Result<(bool, u8, Vec<u8>)> {
        self.need(2)?;
        let b0 = self.buf[0];
        let b1 = self.buf[1];
        let fin = b0 & 0x80 != 0;
        let opcode = b0 & 0x0F;
        let masked = b1 & 0x80 != 0;
        let mut off = 2usize;
        let mut len = (b1 & 0x7F) as usize;
        if len == 126 {
            self.need(off + 2)?;
            len = u16::from_be_bytes([self.buf[off], self.buf[off + 1]]) as usize;
            off += 2;
        } else if len == 127 {
            self.need(off + 8)?;
            let mut a = [0u8; 8];
            a.copy_from_slice(&self.buf[off..off + 8]);
            len = u64::from_be_bytes(a) as usize;
            off += 8;
        }
        if len > MAX_MESSAGE {
            return Err(Error::Protocol(format!(
                "frame of {len} bytes exceeds limit"
            )));
        }
        let mask = if masked {
            self.need(off + 4)?;
            let m = [
                self.buf[off],
                self.buf[off + 1],
                self.buf[off + 2],
                self.buf[off + 3],
            ];
            off += 4;
            Some(m)
        } else {
            None
        };
        self.need(off + len)?;
        let mut data = self.buf[off..off + len].to_vec();
        if let Some(m) = mask {
            for (i, b) in data.iter_mut().enumerate() {
                *b ^= m[i % 4];
            }
        }
        self.buf.drain(..off + len);
        Ok((fin, opcode, data))
    }

    /// Send a close frame; errors are ignored because we are tearing down.
    pub fn close(&mut self) {
        let _ = self.send_frame(OP_CLOSE, &[]);
    }
}

fn finish(parts: Vec<u8>) -> Result<String> {
    String::from_utf8(parts).map_err(|e| Error::Protocol(format!("invalid utf-8: {e}")))
}

fn find_subslice(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() {
        return None;
    }
    hay.windows(needle.len()).position(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The exact worked example from RFC 6455 section 1.3.
    ///
    /// This test exists because the magic GUID was transcribed from memory
    /// twice during development and was wrong both times. The handshake
    /// failed with an opaque mismatch; this vector located it immediately.
    #[test]
    fn rfc6455_accept_vector() {
        assert_eq!(
            accept_for("dGhlIHNhbXBsZSBub25jZQ=="),
            "s3pPLMBiTxaQ9kYGzzhZRbK+xOo="
        );
    }

    #[test]
    fn url_splitting() {
        let (h, p, path) = split_url("ws://127.0.0.1:9222/devtools/page/AB12").unwrap();
        assert_eq!(h, "127.0.0.1");
        assert_eq!(p, 9222);
        assert_eq!(path, "/devtools/page/AB12");
    }

    #[test]
    fn url_without_path_defaults_to_root() {
        let (h, p, path) = split_url("ws://localhost:1234").unwrap();
        assert_eq!((h.as_str(), p, path.as_str()), ("localhost", 1234, "/"));
    }

    #[test]
    fn non_ws_scheme_is_rejected() {
        assert!(split_url("http://example.com/").is_err());
    }

    #[test]
    fn subslice_search() {
        assert_eq!(find_subslice(b"abc\r\n\r\ndef", b"\r\n\r\n"), Some(3));
        assert_eq!(find_subslice(b"abcdef", b"\r\n\r\n"), None);
        assert_eq!(find_subslice(b"", b"x"), None);
    }
}
