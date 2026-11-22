//! Task D3.1: Neuron Wire (spec 11.3), properties WIRE-1, WIRE-2 and WIRE-3.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use neuron_cell::wire::{
    cobs_decode, cobs_encode, crc16, decode, encode, serial_frame, serial_unframe, tcp_frame,
    DedupFilter, Frame, MsgType, Verdict, WireError,
};
use proptest::prelude::*;
use std::collections::BTreeMap;

fn msg_type() -> impl Strategy<Value = MsgType> {
    prop::sample::select(vec![
        MsgType::Hello,
        MsgType::Data,
        MsgType::Ack,
        MsgType::Heartbeat,
        MsgType::TimesyncReq,
        MsgType::TimesyncRsp,
        MsgType::Fault,
        MsgType::Cmd,
        MsgType::Nack,
    ])
}

fn frame() -> impl Strategy<Value = Frame> {
    (
        any::<bool>(),
        msg_type(),
        any::<u16>(),
        any::<u32>(),
        proptest::option::of(any::<u64>()),
        proptest::collection::vec(any::<u8>(), 0..300),
        proptest::option::of(any::<[u8; 8]>()),
    )
        .prop_map(
            |(ack_requested, msg_type, session, seq, ts, payload, mac)| Frame {
                ack_requested,
                msg_type,
                session,
                seq,
                ts,
                payload,
                mac,
            },
        )
}

#[test]
fn crc_matches_the_ccitt_false_check_value() {
    assert_eq!(crc16(b"123456789"), 0x29B1);
    assert_eq!(crc16(b""), 0xFFFF);
}

#[test]
fn a_known_frame_has_the_documented_layout() {
    let f = Frame::data(0x1234, 5, 0x0001, &[0xAA, 0xBB]);
    let bytes = encode(&f).unwrap();
    // ver_flags, type, session, seq, len, tag id and value, crc.
    assert_eq!(&bytes[..8], &[0x10, 0x02, 0x34, 0x12, 5, 0, 0, 0]);
    assert_eq!(&bytes[8..10], &[4, 0]);
    assert_eq!(&bytes[10..14], &[1, 0, 0xAA, 0xBB]);
    assert_eq!(bytes.len(), 16);
    let crc = crc16(&bytes[..14]);
    assert_eq!(&bytes[14..], &crc.to_le_bytes());
    // A timestamp and a MAC field add 8 bytes each and set their flags.
    let mut g = f.clone();
    g.ts = Some(7);
    g.mac = Some([0; 8]);
    g.ack_requested = true;
    let b = encode(&g).unwrap();
    assert_eq!(b[0], 0x10 | 0x01 | 0x02 | 0x04);
    assert_eq!(b.len(), 32);
    assert_eq!(decode(&b).unwrap(), g);
}

#[test]
fn malformed_frames_are_refused() {
    let good = encode(&Frame::new(MsgType::Ack, 1, 2, vec![])).unwrap();
    assert_eq!(decode(&good[..5]), Err(WireError::Truncated));
    let mut extra = good.clone();
    extra.push(0);
    assert!(decode(&extra).is_err());
    // A well-formed CRC over a bad version, bad type or reserved flag is refused for
    // its content.
    let reseal = |mut b: Vec<u8>| {
        let n = b.len() - 2;
        let c = crc16(&b[..n]).to_le_bytes();
        b[n..].copy_from_slice(&c);
        b
    };
    let mut v = good.clone();
    v[0] = 0x20;
    assert_eq!(decode(&reseal(v)), Err(WireError::BadVersion));
    let mut t = good.clone();
    t[1] = 0x7F;
    assert_eq!(decode(&reseal(t)), Err(WireError::BadType));
    let mut r = good;
    r[0] |= 0x08;
    assert_eq!(decode(&reseal(r)), Err(WireError::BadFlags));
    let long = Frame::new(MsgType::Data, 1, 1, vec![0; 70_000]);
    assert_eq!(encode(&long), Err(WireError::PayloadTooLong));
}

#[test]
fn cobs_known_vectors() {
    assert_eq!(cobs_encode(&[0x00]), [0x01, 0x01]);
    assert_eq!(
        cobs_encode(&[0x11, 0x22, 0x00, 0x33]),
        [0x03, 0x11, 0x22, 0x02, 0x33]
    );
    assert_eq!(
        cobs_encode(&[0x11, 0x00, 0x00, 0x00]),
        [0x02, 0x11, 0x01, 0x01, 0x01]
    );
    assert_eq!(cobs_encode(&[]), [0x01]);
    assert_eq!(cobs_decode(&[0x00, 0x01]), Err(WireError::BadCobs));
    assert_eq!(cobs_decode(&[0x05, 0x01]), Err(WireError::BadCobs));
}

#[test]
fn serial_and_tcp_wrappers() {
    let f = Frame::data(1, 2, 3, &[0, 0, 0, 0]);
    let s = serial_frame(&f).unwrap();
    assert_eq!(s.last(), Some(&0));
    assert!(!s[..s.len() - 1].contains(&0), "COBS removes every zero");
    assert_eq!(serial_unframe(&s[..s.len() - 1]).unwrap(), f);
    let t = tcp_frame(&f).unwrap();
    assert_eq!(usize::from(u16::from_le_bytes([t[0], t[1]])), t.len() - 2);
    assert_eq!(decode(&t[2..]).unwrap(), f);
}

proptest! {
    /// WIRE-1: `decode(encode(m)) == m`, also through COBS.
    #[test]
    fn wire_1_round_trip(f in frame()) {
        let bytes = encode(&f).unwrap();
        prop_assert_eq!(decode(&bytes).unwrap(), f.clone());
        let s = serial_frame(&f).unwrap();
        prop_assert_eq!(serial_unframe(&s[..s.len() - 1]).unwrap(), f);
    }

    /// WIRE-2: flipping any single bit of an encoded frame is rejected.
    #[test]
    fn wire_2_single_bit_corruption_is_rejected(f in frame(), pick in any::<prop::sample::Index>()) {
        let mut bytes = encode(&f).unwrap();
        let bit = pick.index(bytes.len() * 8);
        bytes[bit / 8] ^= 1 << (bit % 8);
        prop_assert!(decode(&bytes).is_err());
    }

    /// COBS round trip, and no zero in the output.
    #[test]
    fn cobs_round_trip(data in proptest::collection::vec(any::<u8>(), 0..600)) {
        let e = cobs_encode(&data);
        prop_assert!(!e.contains(&0));
        prop_assert_eq!(cobs_decode(&e).unwrap(), data);
    }

    /// Decoding arbitrary bytes never panics.
    #[test]
    fn decode_never_panics(data in proptest::collection::vec(any::<u8>(), 0..200)) {
        let _ = decode(&data);
        let _ = cobs_decode(&data);
    }

    /// WIRE-3: whatever order frames arrive in, even repeated and across the wrap of
    /// `seq`, a `(session, seq)` is applied at most once.
    #[test]
    fn wire_3_at_most_once(
        arrivals in proptest::collection::vec((0u16..3, 0u32..24), 0..200),
        base in prop::sample::select(vec![0u32, 1000, u32::MAX - 10, u32::MAX - 100]),
    ) {
        let mut filter = DedupFilter::new();
        let mut applied: BTreeMap<(u16, u32), u32> = BTreeMap::new();
        for (session, k) in arrivals {
            let seq = base.wrapping_add(k);
            if filter.check(session, seq) == Verdict::Apply {
                *applied.entry((session, seq)).or_default() += 1;
            }
        }
        prop_assert!(applied.values().all(|n| *n == 1));
    }
}

#[test]
fn the_filter_judges_new_repeated_and_stale_frames() {
    let mut f = DedupFilter::new();
    assert_eq!(f.check(1, 10), Verdict::Apply);
    assert_eq!(f.check(1, 10), Verdict::Duplicate);
    assert_eq!(f.check(1, 12), Verdict::Apply);
    assert_eq!(
        f.check(1, 11),
        Verdict::Apply,
        "late but new, inside the window"
    );
    assert_eq!(f.check(1, 11), Verdict::Duplicate);
    assert_eq!(f.check(2, 10), Verdict::Apply, "another session");
    assert_eq!(f.check(1, 200), Verdict::Apply);
    assert_eq!(f.check(1, 12), Verdict::Stale, "older than the window");
    // The sequence number wraps.
    let mut w = DedupFilter::new();
    assert_eq!(w.check(0, u32::MAX), Verdict::Apply);
    assert_eq!(w.check(0, 0), Verdict::Apply);
    assert_eq!(w.check(0, u32::MAX), Verdict::Duplicate);
}
