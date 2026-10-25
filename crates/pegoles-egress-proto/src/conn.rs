//! Connection state: handshake order, stream table, per-stream credit.
//!
//! Window semantics: each direction of each stream starts at
//! `INITIAL_WINDOW` bytes. A sender may only send while it holds credit; the
//! receiver grants more with CREDIT frames once it has drained data.

use std::collections::HashMap;

use crate::frame::{Frame, ProtoError};
use crate::{INITIAL_WINDOW, MAX_CA_DER, MAX_PAYLOAD, MAX_STREAMS, VERSION};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// Sends HELLO, receives OPEN (never opens streams).
    Host,
    /// Answers HELLO, opens odd, strictly increasing stream ids.
    Guest,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    HostStart,
    HostAwaitAck,
    GuestAwaitHello,
    GuestAckPending,
    Ready,
}

#[derive(Clone, Copy, Debug)]
struct StreamState {
    /// Bytes we may still send.
    send: u32,
    /// Bytes the peer may still send us.
    recv: u32,
}

/// What an accepted incoming frame means for the caller.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// Guest: HELLO accepted (install the CA from the frame, then call
    /// [`Conn::ack_frame`]).
    Hello,
    /// Host: handshake complete.
    HelloAck,
    Opened(u32),
    /// Data accepted on an open stream (its receive window was debited).
    Data(u32),
    /// The peer granted more send credit on an open stream.
    Credit(u32),
    Closed(u32),
    /// A late frame for a stream that is already closed, or a CLOSE for a
    /// stream that is not open. Nothing to do.
    Ignored,
}

/// Protocol state machine for one side of the connection.
pub struct Conn {
    role: Role,
    phase: Phase,
    streams: HashMap<u32, StreamState>,
    /// Highest stream id ever opened (by the peer for `Host`, by us for
    /// `Guest`); ids only grow, so `id <= last_opened` with the right parity
    /// identifies a stream that existed and is now closed.
    last_opened: u32,
}

impl Conn {
    pub fn new(role: Role) -> Conn {
        Conn {
            role,
            phase: match role {
                Role::Host => Phase::HostStart,
                Role::Guest => Phase::GuestAwaitHello,
            },
            streams: HashMap::new(),
            last_opened: 0,
        }
    }

    pub fn role(&self) -> Role {
        self.role
    }

    pub fn is_ready(&self) -> bool {
        self.phase == Phase::Ready
    }

    pub fn open_streams(&self) -> usize {
        self.streams.len()
    }

    pub fn is_open(&self, stream: u32) -> bool {
        self.streams.contains_key(&stream)
    }

    /// Host: the HELLO frame to send first.
    pub fn hello_frame(&mut self, ca_der: Vec<u8>) -> Result<Frame, ProtoError> {
        if self.phase != Phase::HostStart {
            return Err(ProtoError::OutOfOrder);
        }
        if ca_der.is_empty() || ca_der.len() > MAX_CA_DER {
            return Err(ProtoError::BadPayload);
        }
        self.phase = Phase::HostAwaitAck;
        Ok(Frame::Hello {
            version: VERSION,
            ca_der,
        })
    }

    /// Guest: the HELLO_ACK to send after an accepted HELLO.
    pub fn ack_frame(&mut self) -> Result<Frame, ProtoError> {
        if self.phase != Phase::GuestAckPending {
            return Err(ProtoError::OutOfOrder);
        }
        self.phase = Phase::Ready;
        Ok(Frame::HelloAck { version: VERSION })
    }

    /// Guest: allocates the next stream id and returns the OPEN frame.
    pub fn open_stream(&mut self) -> Result<(u32, Frame), ProtoError> {
        if self.role != Role::Guest || self.phase != Phase::Ready {
            return Err(ProtoError::OutOfOrder);
        }
        if self.streams.len() >= MAX_STREAMS {
            return Err(ProtoError::TooManyStreams);
        }
        let id = if self.last_opened == 0 {
            1
        } else {
            self.last_opened.checked_add(2).ok_or(ProtoError::BadOpen)?
        };
        self.last_opened = id;
        self.streams.insert(id, fresh());
        Ok((id, Frame::Open { stream: id }))
    }

    /// Closes locally. Returns the CLOSE frame to send if the stream was
    /// still open (`None`: nothing to send).
    pub fn close_stream(&mut self, stream: u32) -> Option<Frame> {
        self.streams
            .remove(&stream)
            .map(|_| Frame::Close { stream })
    }

    /// Bytes we may send now on `stream` (window only, not frame size).
    pub fn send_window(&self, stream: u32) -> Option<u32> {
        self.streams.get(&stream).map(|s| s.send)
    }

    /// Takes up to `want` bytes (and at most one frame's worth) from the
    /// send window. Returns how many bytes the caller may put in the next
    /// DATA frame (0 when out of credit).
    pub fn reserve_send(&mut self, stream: u32, want: usize) -> Result<usize, ProtoError> {
        let s = self
            .streams
            .get_mut(&stream)
            .ok_or(ProtoError::UnknownStream)?;
        let n = want.min(MAX_PAYLOAD).min(s.send as usize);
        // n <= s.send <= u32::MAX.
        s.send -= n as u32;
        Ok(n)
    }

    /// Grants the peer `increment` more bytes on `stream`; returns the
    /// CREDIT frame to send.
    pub fn grant(&mut self, stream: u32, increment: u32) -> Result<Frame, ProtoError> {
        let s = self
            .streams
            .get_mut(&stream)
            .ok_or(ProtoError::UnknownStream)?;
        s.recv = s
            .recv
            .checked_add(increment)
            .ok_or(ProtoError::CreditOverflow)?;
        Ok(Frame::Credit { stream, increment })
    }

    /// True if `stream` is a valid id of a stream that existed and closed.
    fn was_opened(&self, stream: u32) -> bool {
        stream % 2 == 1 && stream <= self.last_opened && !self.streams.contains_key(&stream)
    }

    /// Applies one decoded incoming frame. Any `Err` means: drop the
    /// connection.
    pub fn on_frame(&mut self, frame: &Frame) -> Result<Event, ProtoError> {
        match self.phase {
            Phase::HostStart | Phase::GuestAckPending => Err(ProtoError::OutOfOrder),
            Phase::HostAwaitAck => match frame {
                Frame::HelloAck { version } if *version == VERSION => {
                    self.phase = Phase::Ready;
                    Ok(Event::HelloAck)
                }
                Frame::HelloAck { version } => Err(ProtoError::UnsupportedVersion(*version)),
                _ => Err(ProtoError::OutOfOrder),
            },
            Phase::GuestAwaitHello => match frame {
                Frame::Hello { version, .. } if *version == VERSION => {
                    self.phase = Phase::GuestAckPending;
                    Ok(Event::Hello)
                }
                Frame::Hello { version, .. } => Err(ProtoError::UnsupportedVersion(*version)),
                _ => Err(ProtoError::OutOfOrder),
            },
            Phase::Ready => self.on_ready(frame),
        }
    }

    fn on_ready(&mut self, frame: &Frame) -> Result<Event, ProtoError> {
        match frame {
            Frame::Hello { .. } | Frame::HelloAck { .. } => Err(ProtoError::OutOfOrder),
            Frame::Open { stream } => {
                if self.role != Role::Host {
                    return Err(ProtoError::BadOpen);
                }
                let id = *stream;
                if id % 2 == 0 || id <= self.last_opened {
                    return Err(ProtoError::BadOpen);
                }
                if self.streams.len() >= MAX_STREAMS {
                    return Err(ProtoError::TooManyStreams);
                }
                self.last_opened = id;
                self.streams.insert(id, fresh());
                Ok(Event::Opened(id))
            }
            Frame::Data { stream, payload } => {
                let closed = self.was_opened(*stream);
                match self.streams.get_mut(stream) {
                    Some(s) => {
                        let n = u32::try_from(payload.len()).map_err(|_| ProtoError::Oversize)?;
                        if n > s.recv {
                            return Err(ProtoError::CreditExceeded);
                        }
                        s.recv -= n;
                        Ok(Event::Data(*stream))
                    }
                    None if closed => Ok(Event::Ignored),
                    None => Err(ProtoError::UnknownStream),
                }
            }
            Frame::Credit { stream, increment } => {
                let closed = self.was_opened(*stream);
                match self.streams.get_mut(stream) {
                    Some(s) => {
                        s.send = s
                            .send
                            .checked_add(*increment)
                            .ok_or(ProtoError::CreditOverflow)?;
                        Ok(Event::Credit(*stream))
                    }
                    None if closed => Ok(Event::Ignored),
                    None => Err(ProtoError::UnknownStream),
                }
            }
            Frame::Close { stream } => match self.streams.remove(stream) {
                Some(_) => Ok(Event::Closed(*stream)),
                None => Ok(Event::Ignored),
            },
        }
    }
}

fn fresh() -> StreamState {
    StreamState {
        send: INITIAL_WINDOW,
        recv: INITIAL_WINDOW,
    }
}
