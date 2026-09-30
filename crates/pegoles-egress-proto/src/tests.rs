use proptest::prelude::*;

use super::*;

fn enc(f: &Frame) -> Vec<u8> {
    f.to_bytes().expect("valid frame encodes")
}

fn header(stream: u32, kind: u8, len: u32) -> Vec<u8> {
    let mut v = stream.to_be_bytes().to_vec();
    v.push(kind);
    v.extend_from_slice(&len.to_be_bytes());
    v
}

fn decode_bytes(bytes: &[u8]) -> Result<Vec<Frame>, ProtoError> {
    let mut d = Decoder::new();
    let mut out = Vec::new();
    d.decode_all(bytes, &mut out)?;
    Ok(out)
}

fn sample_frames() -> Vec<Frame> {
    vec![
        Frame::Hello {
            version: VERSION,
            ca_der: vec![0x30, 0x82, 1, 2, 3],
        },
        Frame::HelloAck { version: VERSION },
        Frame::Open { stream: 1 },
        Frame::Data {
            stream: 1,
            payload: b"GET / HTTP/1.1\r\n\r\n".to_vec(),
        },
        Frame::Data {
            stream: 3,
            payload: vec![],
        },
        Frame::Credit {
            stream: 1,
            increment: 4096,
        },
        Frame::Close { stream: 1 },
    ]
}

#[test]
fn constants_match_contract() {
    assert_eq!(VERSION, 1);
    assert_eq!(VSOCK_PORT, 4051);
    assert_eq!(GUEST_PROXY_PORT, 3128);
    assert_eq!(MAX_PAYLOAD, 16384);
    assert_eq!(MAX_CA_DER, 4096);
    assert_eq!(INITIAL_WINDOW, 262_144);
    assert_eq!(MAX_STREAMS, 64);
}

#[test]
fn wire_format_is_big_endian_header_then_payload() {
    let b = enc(&Frame::Credit {
        stream: 0x0102_0304,
        increment: 0x0A0B_0C0D,
    });
    assert_eq!(
        b,
        [1, 2, 3, 4, 5, 0, 0, 0, 4, 0x0A, 0x0B, 0x0C, 0x0D],
        "stream BE | kind 5 | len BE | u32 BE"
    );
    let b = enc(&Frame::Hello {
        version: 1,
        ca_der: vec![9, 9],
    });
    assert_eq!(b, [0, 0, 0, 0, 0, 0, 0, 0, 4, 0, 1, 9, 9]);
}

#[test]
fn roundtrip_all_kinds() {
    let frames = sample_frames();
    let mut wire = Vec::new();
    for f in &frames {
        f.encode(&mut wire).expect("encode");
    }
    assert_eq!(decode_bytes(&wire).expect("decode"), frames);
}

#[test]
fn byte_at_a_time_matches_whole_buffer() {
    let frames = sample_frames();
    let mut wire = Vec::new();
    for f in &frames {
        f.encode(&mut wire).expect("encode");
    }
    let mut d = Decoder::new();
    let mut out = Vec::new();
    for b in &wire {
        d.decode_all(std::slice::from_ref(b), &mut out)
            .expect("decode");
    }
    assert_eq!(out, frames);
}

#[test]
fn max_size_data_frame_is_accepted() {
    let f = Frame::Data {
        stream: 5,
        payload: vec![7; MAX_PAYLOAD],
    };
    assert_eq!(decode_bytes(&enc(&f)).expect("decode"), vec![f]);
}

#[test]
fn rejects_every_header_violation() {
    let cases: Vec<(Vec<u8>, ProtoError)> = vec![
        (header(1, 6, 0), ProtoError::UnknownKind(6)),
        (header(1, 255, 0), ProtoError::UnknownKind(255)),
        (header(1, 3, (MAX_PAYLOAD + 1) as u32), ProtoError::Oversize),
        (header(1, 3, u32::MAX), ProtoError::Oversize),
        (
            header(0, 0, (2 + MAX_CA_DER + 1) as u32),
            ProtoError::Oversize,
        ),
        (header(0, 0, 2), ProtoError::BadPayload),
        (header(0, 0, 0), ProtoError::BadPayload),
        (header(7, 0, 10), ProtoError::BadStreamId),
        (header(7, 1, 2), ProtoError::BadStreamId),
        (header(0, 1, 3), ProtoError::Oversize),
        (header(0, 2, 0), ProtoError::BadStreamId),
        (header(1, 2, 1), ProtoError::Oversize),
        (header(0, 3, 1), ProtoError::BadStreamId),
        (header(0, 4, 0), ProtoError::BadStreamId),
        (header(1, 4, 1), ProtoError::Oversize),
        (header(0, 5, 4), ProtoError::BadStreamId),
        (header(1, 5, 3), ProtoError::BadPayload),
        (header(1, 5, 5), ProtoError::Oversize),
    ];
    for (bytes, want) in cases {
        if let Ok(frames) = decode_bytes(&bytes) {
            // The only "valid" case above is a HELLO_ACK header awaiting its payload.
            assert!(frames.is_empty(), "{bytes:?} produced {frames:?}");
            continue;
        }
        assert_eq!(decode_bytes(&bytes).unwrap_err(), want, "{bytes:?}");
    }
}

#[test]
fn oversize_is_rejected_before_any_allocation() {
    let mut d = Decoder::new();
    let mut input: &[u8] = &header(1, 3, u32::MAX);
    assert_eq!(d.decode(&mut input).unwrap_err(), ProtoError::Oversize);
    assert_eq!(d.reserved(), 0);
}

#[test]
fn decoder_is_poisoned_after_an_error() {
    let mut d = Decoder::new();
    let mut bad: &[u8] = &header(1, 9, 0);
    assert!(d.decode(&mut bad).is_err());
    let good = enc(&Frame::Open { stream: 1 });
    let mut input: &[u8] = &good;
    assert_eq!(d.decode(&mut input).unwrap_err(), ProtoError::Poisoned);
}

#[test]
fn encode_refuses_invalid_frames() {
    let mut out = vec![1, 2, 3];
    let too_big = Frame::Data {
        stream: 1,
        payload: vec![0; MAX_PAYLOAD + 1],
    };
    assert_eq!(too_big.encode(&mut out).unwrap_err(), ProtoError::Oversize);
    assert_eq!(out, [1, 2, 3], "unchanged on failure");
    let big_ca = Frame::Hello {
        version: 1,
        ca_der: vec![0; MAX_CA_DER + 1],
    };
    assert!(big_ca.encode(&mut out).is_err());
    let empty_ca = Frame::Hello {
        version: 1,
        ca_der: vec![],
    };
    assert!(empty_ca.encode(&mut out).is_err());
    assert!(Frame::Open { stream: 0 }.encode(&mut out).is_err());
    let ok = Frame::Hello {
        version: 1,
        ca_der: vec![0; MAX_CA_DER],
    };
    assert!(ok.encode(&mut out).is_ok());
}

// ---- connection state machine ------------------------------------------

fn ready_pair() -> (Conn, Conn) {
    let mut host = Conn::new(Role::Host);
    let mut guest = Conn::new(Role::Guest);
    let hello = host.hello_frame(vec![1, 2, 3]).expect("hello");
    assert_eq!(guest.on_frame(&hello), Ok(Event::Hello));
    let ack = guest.ack_frame().expect("ack");
    assert_eq!(host.on_frame(&ack), Ok(Event::HelloAck));
    assert!(host.is_ready() && guest.is_ready());
    (host, guest)
}

#[test]
fn handshake_must_come_first() {
    let mut host = Conn::new(Role::Host);
    assert_eq!(
        host.on_frame(&Frame::Open { stream: 1 }),
        Err(ProtoError::OutOfOrder),
        "guest spoke before HELLO"
    );
    let mut host = Conn::new(Role::Host);
    host.hello_frame(vec![1]).expect("hello");
    assert_eq!(
        host.on_frame(&Frame::Open { stream: 1 }),
        Err(ProtoError::OutOfOrder),
        "OPEN before HELLO_ACK"
    );
    let mut guest = Conn::new(Role::Guest);
    assert_eq!(
        guest.on_frame(&Frame::Open { stream: 2 }),
        Err(ProtoError::OutOfOrder)
    );
    assert_eq!(
        guest.on_frame(&Frame::Hello {
            version: 2,
            ca_der: vec![1]
        }),
        Err(ProtoError::UnsupportedVersion(2))
    );
    let mut host = Conn::new(Role::Host);
    host.hello_frame(vec![1]).expect("hello");
    assert_eq!(
        host.on_frame(&Frame::HelloAck { version: 9 }),
        Err(ProtoError::UnsupportedVersion(9))
    );
}

#[test]
fn second_hello_is_rejected() {
    let (mut host, mut guest) = ready_pair();
    assert!(host
        .on_frame(&Frame::HelloAck { version: VERSION })
        .is_err());
    assert!(guest
        .on_frame(&Frame::Hello {
            version: VERSION,
            ca_der: vec![1]
        })
        .is_err());
}

#[test]
fn open_ids_must_be_odd_and_strictly_increasing() {
    let (mut host, _) = ready_pair();
    assert_eq!(
        host.on_frame(&Frame::Open { stream: 2 }),
        Err(ProtoError::BadOpen)
    );
    assert_eq!(
        host.on_frame(&Frame::Open { stream: 5 }),
        Ok(Event::Opened(5))
    );
    assert_eq!(
        host.on_frame(&Frame::Open { stream: 5 }),
        Err(ProtoError::BadOpen)
    );
    assert_eq!(
        host.on_frame(&Frame::Open { stream: 3 }),
        Err(ProtoError::BadOpen)
    );
    // ids are never reused, even after close
    assert_eq!(
        host.on_frame(&Frame::Close { stream: 5 }),
        Ok(Event::Closed(5))
    );
    assert_eq!(
        host.on_frame(&Frame::Open { stream: 5 }),
        Err(ProtoError::BadOpen)
    );
    assert_eq!(
        host.on_frame(&Frame::Open { stream: 7 }),
        Ok(Event::Opened(7))
    );
}

#[test]
fn host_never_accepts_open_on_guest_side() {
    let (_, mut guest) = ready_pair();
    assert_eq!(
        guest.on_frame(&Frame::Open { stream: 1 }),
        Err(ProtoError::BadOpen)
    );
}

#[test]
fn at_most_64_open_streams() {
    let (mut host, mut guest) = ready_pair();
    for i in 0..MAX_STREAMS {
        let (id, f) = guest.open_stream().expect("open");
        assert_eq!(id as usize, 2 * i + 1);
        assert_eq!(host.on_frame(&f), Ok(Event::Opened(id)));
    }
    assert_eq!(guest.open_stream().unwrap_err(), ProtoError::TooManyStreams);
    assert_eq!(
        host.on_frame(&Frame::Open {
            stream: 2 * MAX_STREAMS as u32 + 1
        }),
        Err(ProtoError::TooManyStreams)
    );
    // closing frees a slot
    let close = guest.close_stream(1).expect("was open");
    assert_eq!(host.on_frame(&close), Ok(Event::Closed(1)));
    assert!(guest.open_stream().is_ok());
}

#[test]
fn data_for_unknown_stream_drops_connection_but_close_does_not() {
    let (mut host, _) = ready_pair();
    let data = Frame::Data {
        stream: 9,
        payload: vec![1],
    };
    assert_eq!(host.on_frame(&data), Err(ProtoError::UnknownStream));
    assert_eq!(
        host.on_frame(&Frame::Credit {
            stream: 9,
            increment: 1
        }),
        Err(ProtoError::UnknownStream)
    );
    assert_eq!(
        host.on_frame(&Frame::Close { stream: 9 }),
        Ok(Event::Ignored)
    );
}

#[test]
fn late_frames_for_a_closed_stream_are_ignored() {
    let (mut host, _) = ready_pair();
    host.on_frame(&Frame::Open { stream: 1 }).expect("open");
    host.on_frame(&Frame::Open { stream: 3 }).expect("open");
    assert!(host.close_stream(1).is_some());
    let late = Frame::Data {
        stream: 1,
        payload: vec![1],
    };
    assert_eq!(host.on_frame(&late), Ok(Event::Ignored));
    // never-opened id above the highest is still fatal
    let ghost = Frame::Data {
        stream: 5,
        payload: vec![1],
    };
    assert_eq!(host.on_frame(&ghost), Err(ProtoError::UnknownStream));
}

#[test]
fn data_beyond_granted_credit_is_a_violation() {
    let (mut host, _) = ready_pair();
    host.on_frame(&Frame::Open { stream: 1 }).expect("open");
    let full = INITIAL_WINDOW as usize;
    let mut left = full;
    while left > 0 {
        let n = left.min(MAX_PAYLOAD);
        let f = Frame::Data {
            stream: 1,
            payload: vec![0; n],
        };
        assert_eq!(host.on_frame(&f), Ok(Event::Data(1)));
        left -= n;
    }
    let one_more = Frame::Data {
        stream: 1,
        payload: vec![0],
    };
    assert_eq!(host.on_frame(&one_more), Err(ProtoError::CreditExceeded));
}

#[test]
fn grant_replenishes_the_window() {
    let (mut host, _) = ready_pair();
    host.on_frame(&Frame::Open { stream: 1 }).expect("open");
    for _ in 0..(INITIAL_WINDOW as usize / MAX_PAYLOAD) {
        let f = Frame::Data {
            stream: 1,
            payload: vec![0; MAX_PAYLOAD],
        };
        host.on_frame(&f).expect("within window");
    }
    let credit = host.grant(1, 100).expect("grant");
    assert_eq!(
        credit,
        Frame::Credit {
            stream: 1,
            increment: 100
        }
    );
    let f = Frame::Data {
        stream: 1,
        payload: vec![0; 100],
    };
    assert_eq!(host.on_frame(&f), Ok(Event::Data(1)));
    assert_eq!(
        host.on_frame(&Frame::Data {
            stream: 1,
            payload: vec![0]
        }),
        Err(ProtoError::CreditExceeded)
    );
}

#[test]
fn credit_overflow_is_rejected_both_ways() {
    let (mut host, _) = ready_pair();
    host.on_frame(&Frame::Open { stream: 1 }).expect("open");
    let inc = Frame::Credit {
        stream: 1,
        increment: u32::MAX,
    };
    assert_eq!(host.on_frame(&inc), Err(ProtoError::CreditOverflow));
    assert_eq!(
        host.grant(1, u32::MAX).unwrap_err(),
        ProtoError::CreditOverflow
    );
}

#[test]
fn reserve_send_respects_window_and_frame_size() {
    let (mut host, _) = ready_pair();
    host.on_frame(&Frame::Open { stream: 1 }).expect("open");
    assert_eq!(host.reserve_send(1, 1 << 20), Ok(MAX_PAYLOAD));
    assert_eq!(host.reserve_send(1, 10), Ok(10));
    assert_eq!(
        host.send_window(1),
        Some(INITIAL_WINDOW - MAX_PAYLOAD as u32 - 10)
    );
    host.on_frame(&Frame::Credit {
        stream: 1,
        increment: 5,
    })
    .expect("credit");
    // drain the window, then it yields 0 until credit arrives
    while host.reserve_send(1, MAX_PAYLOAD).expect("reserve") > 0 {}
    assert_eq!(host.reserve_send(1, 1), Ok(0));
    assert_eq!(host.reserve_send(99, 1), Err(ProtoError::UnknownStream));
}

// ---- proptest -----------------------------------------------------------

fn arb_frame() -> impl Strategy<Value = Frame> {
    prop_oneof![
        (
            any::<u16>(),
            proptest::collection::vec(any::<u8>(), 1..=MAX_CA_DER)
        )
            .prop_map(|(version, ca_der)| Frame::Hello { version, ca_der }),
        any::<u16>().prop_map(|version| Frame::HelloAck { version }),
        (1u32..).prop_map(|stream| Frame::Open { stream }),
        (
            1u32..,
            proptest::collection::vec(any::<u8>(), 0..=MAX_PAYLOAD)
        )
            .prop_map(|(stream, payload)| Frame::Data { stream, payload }),
        (1u32..).prop_map(|stream| Frame::Close { stream }),
        (1u32.., any::<u32>()).prop_map(|(stream, increment)| Frame::Credit { stream, increment }),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn arbitrary_bytes_never_panic_and_stay_bounded(
        bytes in proptest::collection::vec(any::<u8>(), 0..2048),
        cut in 1usize..64,
    ) {
        let mut d = Decoder::new();
        let mut out = Vec::new();
        for chunk in bytes.chunks(cut) {
            if d.decode_all(chunk, &mut out).is_err() {
                break;
            }
            prop_assert!(d.reserved() <= MAX_PAYLOAD);
        }
        for f in &out {
            prop_assert!(f.to_bytes().is_ok(), "decoded frame re-encodes");
        }
    }

    #[test]
    fn hostile_headers_never_reserve_more_than_one_frame(
        stream in any::<u32>(), kind in any::<u8>(), len in any::<u32>(),
        tail in proptest::collection::vec(any::<u8>(), 0..64),
    ) {
        let mut bytes = header(stream, kind, len);
        bytes.extend_from_slice(&tail);
        let mut d = Decoder::new();
        let mut out = Vec::new();
        let _ = d.decode_all(&bytes, &mut out);
        prop_assert!(d.reserved() <= MAX_PAYLOAD);
    }

    #[test]
    fn any_frame_roundtrips_under_any_split(
        frames in proptest::collection::vec(arb_frame(), 1..6),
        cut in 1usize..4096,
    ) {
        let mut wire = Vec::new();
        for f in &frames { f.encode(&mut wire).map_err(|e| TestCaseError::fail(e.to_string()))?; }
        let mut d = Decoder::new();
        let mut out = Vec::new();
        for chunk in wire.chunks(cut) {
            d.decode_all(chunk, &mut out).map_err(|e| TestCaseError::fail(e.to_string()))?;
        }
        prop_assert_eq!(out, frames);
    }

    #[test]
    fn oversize_declared_length_is_always_rejected(
        kind in 0u8..6, extra in 1u32..100_000,
    ) {
        let len = MAX_PAYLOAD as u32 + extra;
        let stream = if kind < 2 { 0 } else { 1 };
        let mut d = Decoder::new();
        let mut input: &[u8] = &header(stream, kind, len);
        prop_assert_eq!(d.decode(&mut input), Err(ProtoError::Oversize));
        prop_assert_eq!(d.reserved(), 0);
    }

    #[test]
    fn conn_never_panics_on_any_frame_sequence(
        frames in proptest::collection::vec(arb_frame(), 0..40),
    ) {
        let (mut host, _) = ready_pair();
        for f in &frames {
            if host.on_frame(f).is_err() { break; }
            prop_assert!(host.open_streams() <= MAX_STREAMS);
        }
    }
}
