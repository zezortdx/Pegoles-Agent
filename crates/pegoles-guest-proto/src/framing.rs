//! Stream framing for the guest protocol: JSON Lines with a hard cap.
//!
//! Guarantees:
//! - partial reads accumulate until a full line arrives;
//! - several messages in one read all surface;
//! - a frame over `MAX_FRAME_BYTES` is rejected (buffer reset, caller
//!   should drop the connection);
//! - invalid UTF-8 is rejected (bytes are untrusted guest input);
//! - a peer disconnecting mid-frame simply leaves an incomplete buffer
//!   behind (nothing is emitted, nothing panics).

/// Maximum bytes for one frame (line content, excluding `\n`).
pub const MAX_FRAME_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameError {
    /// A line exceeded the cap without terminating. Buffer was reset.
    Oversized,
    /// A complete line is not valid UTF-8. The offending line was dropped.
    InvalidUtf8,
}

#[derive(Debug, Default)]
pub struct Framer {
    buf: Vec<u8>,
}

impl Framer {
    pub fn new() -> Self {
        Self { buf: Vec::new() }
    }

    /// Feed stream bytes; returns each complete line (without `\n`/`\r\n`).
    /// Incomplete trailing bytes stay buffered for the next `push`.
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<String>, FrameError> {
        if self.buf.len() + bytes.len() > MAX_FRAME_BYTES + 4096 {
            // Fast path: unbounded growth without any newline at all.
            self.buf.clear();
            return Err(FrameError::Oversized);
        }
        self.buf.extend_from_slice(bytes);
        let mut out = Vec::new();
        while let Some(pos) = self.buf.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.buf.drain(..=pos).collect();
            let mut line = &line[..line.len() - 1]; // strip \n
            if line.last() == Some(&b'\r') {
                line = &line[..line.len() - 1];
            }
            if line.len() > MAX_FRAME_BYTES {
                self.buf.clear();
                return Err(FrameError::Oversized);
            }
            match std::str::from_utf8(line) {
                Ok(s) => out.push(s.to_string()),
                Err(_) => return Err(FrameError::InvalidUtf8),
            }
        }
        if self.buf.len() > MAX_FRAME_BYTES {
            self.buf.clear();
            return Err(FrameError::Oversized);
        }
        Ok(out)
    }

    pub fn pending_bytes(&self) -> usize {
        self.buf.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_complete_frame() {
        let mut f = Framer::new();
        assert_eq!(f.push(b"{\"a\":1}\n").unwrap(), vec!["{\"a\":1}"]);
    }

    #[test]
    fn partial_frame_across_reads() {
        let mut f = Framer::new();
        assert!(f.push(b"{\"a\":").unwrap().is_empty());
        assert!(f.push(b"1}").unwrap().is_empty());
        assert_eq!(f.push(b"\n").unwrap(), vec!["{\"a\":1}"]);
    }

    #[test]
    fn multiple_frames_one_read() {
        let mut f = Framer::new();
        assert_eq!(
            f.push(b"one\ntwo\nthree\n").unwrap(),
            vec!["one", "two", "three"]
        );
    }

    #[test]
    fn crlf_stripped_and_empty_lines_kept() {
        let mut f = Framer::new();
        assert_eq!(f.push(b"a\r\n\r\nb\n").unwrap(), vec!["a", "", "b"]);
    }

    #[test]
    fn oversized_frame_rejected_and_recovers() {
        let mut f = Framer::new();
        let big = vec![b'x'; MAX_FRAME_BYTES + 1];
        assert_eq!(f.push(&big), Err(FrameError::Oversized));
        // Framer usable again afterwards.
        assert_eq!(f.push(b"ok\n").unwrap(), vec!["ok"]);
    }

    #[test]
    fn oversized_without_newline_rejected() {
        let mut f = Framer::new();
        let big = vec![b'y'; MAX_FRAME_BYTES + 8192];
        assert_eq!(f.push(&big), Err(FrameError::Oversized));
    }

    #[test]
    fn invalid_utf8_rejected() {
        let mut f = Framer::new();
        assert_eq!(f.push(b"\xff\xfe\n"), Err(FrameError::InvalidUtf8));
    }

    #[test]
    fn disconnect_mid_frame_emits_nothing() {
        let mut f = Framer::new();
        assert!(f.push(b"{\"half\":").unwrap().is_empty());
        assert_eq!(f.pending_bytes(), 8);
        // Drop without completing: nothing ever surfaces. No panic.
    }

    #[test]
    fn max_size_frame_accepted() {
        let mut f = Framer::new();
        let mut big = vec![b'z'; MAX_FRAME_BYTES];
        big.push(b'\n');
        assert_eq!(f.push(&big).unwrap().len(), 1);
    }
}
