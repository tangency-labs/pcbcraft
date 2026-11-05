//! Bounded line framing and authenticated local transport for Craft applications.

use std::io::{self, BufRead, Write};

#[cfg(not(target_arch = "wasm32"))]
pub mod loopback;

/// The result of reading one newline-delimited frame.
#[derive(Debug, PartialEq, Eq)]
pub enum Frame {
    /// The original bytes, without the terminating newline.
    Message(Vec<u8>),
    /// The frame exceeded the caller's limit. The reader is positioned at the next frame.
    Oversized,
}

/// Read one frame, buffering at most `limit` bytes. Drain oversized frames so a stdio
/// server can reject the request and continue. A final line at EOF may omit its newline.
pub fn read_frame(input: &mut impl BufRead, limit: usize) -> io::Result<Option<Frame>> {
    let mut line = Vec::new();
    let mut oversized = false;
    loop {
        let chunk = input.fill_buf()?;
        if chunk.is_empty() {
            return if oversized { Ok(Some(Frame::Oversized)) } else { Ok((!line.is_empty()).then_some(Frame::Message(line))) };
        }
        let newline = chunk.iter().position(|&byte| byte == b'\n');
        let content_len = newline.unwrap_or(chunk.len());
        if !oversized {
            oversized = line.len().saturating_add(content_len) > limit;
            if oversized {
                line.clear();
            } else {
                line.extend_from_slice(&chunk[..content_len]);
            }
        }
        input.consume(content_len + usize::from(newline.is_some()));
        if newline.is_some() {
            return Ok(Some(if oversized { Frame::Oversized } else { Frame::Message(line) }));
        }
    }
}

/// Write a frame and flush it so a request or reply can reach a waiting peer.
pub fn write_frame(output: &mut impl Write, message: &[u8]) -> io::Result<()> {
    output.write_all(message)?;
    output.write_all(b"\n")?;
    output.flush()
}
