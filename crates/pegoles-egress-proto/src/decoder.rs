use crate::frame::{build, check_header, Frame, Kind, ProtoError};
use crate::HEADER_LEN;

#[derive(Clone, Copy)]
struct Header {
    stream: u32,
    kind: Kind,
    len: usize,
}

/// Incremental frame decoder. Holds at most one header and one payload;
/// the payload buffer is sized from a header that was already validated, so
/// memory is bounded by `MAX_PAYLOAD` no matter what the peer declares.
pub struct Decoder {
    head: [u8; HEADER_LEN],
    head_len: usize,
    current: Option<Header>,
    payload: Vec<u8>,
    failed: bool,
}

impl Default for Decoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Decoder {
    pub fn new() -> Self {
        Decoder {
            head: [0; HEADER_LEN],
            head_len: 0,
            current: None,
            payload: Vec::new(),
            failed: false,
        }
    }

    /// Capacity currently reserved for a payload (for bound checks).
    pub fn reserved(&self) -> usize {
        self.payload.capacity()
    }

    /// Consumes bytes from the front of `input` (advancing the slice) until
    /// one frame is complete or the input is used up. Call again with the
    /// remaining input until it returns `Ok(None)` with an empty slice.
    pub fn decode(&mut self, input: &mut &[u8]) -> Result<Option<Frame>, ProtoError> {
        if self.failed {
            return Err(ProtoError::Poisoned);
        }
        match self.step(input) {
            Ok(f) => Ok(f),
            Err(e) => {
                self.failed = true;
                self.payload = Vec::new();
                Err(e)
            }
        }
    }

    fn step(&mut self, input: &mut &[u8]) -> Result<Option<Frame>, ProtoError> {
        if self.current.is_none() {
            let take = (HEADER_LEN - self.head_len).min(input.len());
            self.head[self.head_len..self.head_len + take].copy_from_slice(&input[..take]);
            self.head_len += take;
            *input = &input[take..];
            if self.head_len < HEADER_LEN {
                return Ok(None);
            }
            let h = &self.head;
            let stream = u32::from_be_bytes([h[0], h[1], h[2], h[3]]);
            let kind = Kind::from_u8(h[4]).ok_or(ProtoError::UnknownKind(h[4]))?;
            let declared = u32::from_be_bytes([h[5], h[6], h[7], h[8]]);
            // Reject before reserving anything.
            let len = usize::try_from(declared).map_err(|_| ProtoError::Oversize)?;
            check_header(stream, kind, len)?;
            self.head_len = 0;
            self.payload = Vec::with_capacity(len);
            self.current = Some(Header { stream, kind, len });
        }
        let Some(h) = self.current else {
            return Ok(None);
        };
        let need = h.len - self.payload.len();
        let take = need.min(input.len());
        self.payload.extend_from_slice(&input[..take]);
        *input = &input[take..];
        if self.payload.len() < h.len {
            return Ok(None);
        }
        self.current = None;
        let payload = std::mem::take(&mut self.payload);
        build(h.stream, h.kind, payload).map(Some)
    }

    /// Decodes every complete frame in `input` into `out`; stops at the
    /// first error. Convenience for tests and simple callers.
    pub fn decode_all(&mut self, mut input: &[u8], out: &mut Vec<Frame>) -> Result<(), ProtoError> {
        loop {
            match self.decode(&mut input)? {
                Some(f) => out.push(f),
                None if input.is_empty() => return Ok(()),
                None => {}
            }
        }
    }
}
