use std::fmt;

use crate::{CONTROL_STREAM, HEADER_LEN, MAX_CA_DER, MAX_PAYLOAD};

/// Frame kind byte.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Kind {
    Hello = 0,
    HelloAck = 1,
    Open = 2,
    Data = 3,
    Close = 4,
    Credit = 5,
}

impl Kind {
    pub fn from_u8(b: u8) -> Option<Kind> {
        match b {
            0 => Some(Kind::Hello),
            1 => Some(Kind::HelloAck),
            2 => Some(Kind::Open),
            3 => Some(Kind::Data),
            4 => Some(Kind::Close),
            5 => Some(Kind::Credit),
            _ => None,
        }
    }
}

/// A decoded, syntactically valid frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Frame {
    /// host -> guest, stream 0: protocol version + CA certificate (DER).
    Hello {
        version: u16,
        ca_der: Vec<u8>,
    },
    /// guest -> host, stream 0.
    HelloAck {
        version: u16,
    },
    /// guest -> host: open `stream`.
    Open {
        stream: u32,
    },
    Data {
        stream: u32,
        payload: Vec<u8>,
    },
    /// Closes both directions.
    Close {
        stream: u32,
    },
    /// Window increment for the sender's direction.
    Credit {
        stream: u32,
        increment: u32,
    },
}

/// Every way a peer (or a caller) can violate the protocol.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProtoError {
    UnknownKind(u8),
    /// Declared payload length over the limit for the kind.
    Oversize,
    /// Payload does not have the shape the kind requires.
    BadPayload,
    /// Stream id 0 where a stream is required, or the reverse.
    BadStreamId,
    /// A frame the current state does not allow.
    OutOfOrder,
    UnsupportedVersion(u16),
    /// OPEN with an even, non-increasing or otherwise invalid id.
    BadOpen,
    TooManyStreams,
    UnknownStream,
    /// Data or credit beyond what was granted.
    CreditExceeded,
    CreditOverflow,
    /// The decoder saw an error earlier and refuses to continue.
    Poisoned,
}

impl fmt::Display for ProtoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProtoError::UnknownKind(k) => write!(f, "unknown frame kind {k}"),
            ProtoError::Oversize => f.write_str("frame payload too large"),
            ProtoError::BadPayload => f.write_str("malformed frame payload"),
            ProtoError::BadStreamId => f.write_str("invalid stream id for frame kind"),
            ProtoError::OutOfOrder => f.write_str("frame not allowed in this state"),
            ProtoError::UnsupportedVersion(v) => write!(f, "unsupported protocol version {v}"),
            ProtoError::BadOpen => f.write_str("invalid OPEN"),
            ProtoError::TooManyStreams => f.write_str("too many open streams"),
            ProtoError::UnknownStream => f.write_str("frame for an unknown stream"),
            ProtoError::CreditExceeded => f.write_str("peer exceeded granted credit"),
            ProtoError::CreditOverflow => f.write_str("credit overflow"),
            ProtoError::Poisoned => f.write_str("decoder poisoned by an earlier error"),
        }
    }
}

impl std::error::Error for ProtoError {}

/// Largest payload length allowed for `kind`.
pub(crate) fn max_len(kind: Kind) -> usize {
    match kind {
        Kind::Hello => 2 + MAX_CA_DER,
        Kind::HelloAck => 2,
        Kind::Open | Kind::Close => 0,
        Kind::Data => MAX_PAYLOAD,
        Kind::Credit => 4,
    }
}

/// Validates header fields that do not need the payload.
pub(crate) fn check_header(stream: u32, kind: Kind, len: usize) -> Result<(), ProtoError> {
    if len > MAX_PAYLOAD || len > max_len(kind) {
        return Err(ProtoError::Oversize);
    }
    match kind {
        Kind::Hello | Kind::HelloAck => {
            if stream != CONTROL_STREAM {
                return Err(ProtoError::BadStreamId);
            }
        }
        Kind::Open | Kind::Data | Kind::Close | Kind::Credit => {
            if stream == CONTROL_STREAM {
                return Err(ProtoError::BadStreamId);
            }
        }
    }
    let exact = match kind {
        Kind::HelloAck => Some(2),
        Kind::Open | Kind::Close => Some(0),
        Kind::Credit => Some(4),
        Kind::Hello | Kind::Data => None,
    };
    if let Some(n) = exact {
        if len != n {
            return Err(ProtoError::BadPayload);
        }
    }
    // HELLO carries a version and a non-empty certificate.
    if kind == Kind::Hello && len < 3 {
        return Err(ProtoError::BadPayload);
    }
    Ok(())
}

/// Builds a frame from a validated header and its payload.
pub(crate) fn build(stream: u32, kind: Kind, payload: Vec<u8>) -> Result<Frame, ProtoError> {
    check_header(stream, kind, payload.len())?;
    Ok(match kind {
        Kind::Hello => Frame::Hello {
            version: u16::from_be_bytes([payload[0], payload[1]]),
            ca_der: payload[2..].to_vec(),
        },
        Kind::HelloAck => Frame::HelloAck {
            version: u16::from_be_bytes([payload[0], payload[1]]),
        },
        Kind::Open => Frame::Open { stream },
        Kind::Data => Frame::Data { stream, payload },
        Kind::Close => Frame::Close { stream },
        Kind::Credit => Frame::Credit {
            stream,
            increment: u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]),
        },
    })
}

impl Frame {
    pub fn kind(&self) -> Kind {
        match self {
            Frame::Hello { .. } => Kind::Hello,
            Frame::HelloAck { .. } => Kind::HelloAck,
            Frame::Open { .. } => Kind::Open,
            Frame::Data { .. } => Kind::Data,
            Frame::Close { .. } => Kind::Close,
            Frame::Credit { .. } => Kind::Credit,
        }
    }

    pub fn stream(&self) -> u32 {
        match self {
            Frame::Hello { .. } | Frame::HelloAck { .. } => CONTROL_STREAM,
            Frame::Open { stream }
            | Frame::Data { stream, .. }
            | Frame::Close { stream }
            | Frame::Credit { stream, .. } => *stream,
        }
    }

    /// Appends the wire form to `out`. Fails (leaving `out` unchanged) if
    /// the frame would violate the limits, so a caller bug can never put an
    /// invalid frame on the wire.
    pub fn encode(&self, out: &mut Vec<u8>) -> Result<(), ProtoError> {
        let mut body: Vec<u8>;
        let payload: &[u8] = match self {
            Frame::Hello { version, ca_der } => {
                body = Vec::with_capacity(2 + ca_der.len());
                body.extend_from_slice(&version.to_be_bytes());
                body.extend_from_slice(ca_der);
                &body
            }
            Frame::HelloAck { version } => {
                body = version.to_be_bytes().to_vec();
                &body
            }
            Frame::Open { .. } | Frame::Close { .. } => &[],
            Frame::Data { payload, .. } => payload,
            Frame::Credit { increment, .. } => {
                body = increment.to_be_bytes().to_vec();
                &body
            }
        };
        check_header(self.stream(), self.kind(), payload.len())?;
        out.reserve(HEADER_LEN + payload.len());
        out.extend_from_slice(&self.stream().to_be_bytes());
        out.push(self.kind() as u8);
        // Bounded by MAX_PAYLOAD above, so the cast cannot truncate.
        out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        out.extend_from_slice(payload);
        Ok(())
    }

    /// Wire form as a fresh vector.
    pub fn to_bytes(&self) -> Result<Vec<u8>, ProtoError> {
        let mut v = Vec::new();
        self.encode(&mut v)?;
        Ok(v)
    }
}
