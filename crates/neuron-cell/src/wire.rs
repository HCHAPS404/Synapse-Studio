//! Neuron Wire (spec 11.3): the frame codec, CRC-16/CCITT-FALSE, COBS framing for serial
//! links and the duplicate filter. Pure functions, no I/O.
//!
//! Layout (all multi-byte header fields little-endian, like the payload):
//!
//! | Offset | Size | Field |
//! |---|---|---|
//! | 0 | 1 | `ver_flags`: high nibble version (1); bit 0 ack requested, bit 1 timestamped, bit 2 authenticated |
//! | 1 | 1 | `type` |
//! | 2 | 2 | `session` |
//! | 4 | 4 | `seq` |
//! | 8 | 0 or 8 | `ts`, present when timestamped |
//! | next | 2 | `len` |
//! | next | `len` | `payload` |
//! | next | 0 or 8 | `mac`, present when authenticated |
//! | last | 2 | `crc` over every previous byte |
//!
//! The `mac` field is carried, not computed: this module implements no HMAC. A caller
//! that sets `mac` supplies the 8 bytes (and checks them after `decode`); a frame
//! without `mac` has no such field and the authenticated flag is clear.
//!
//! ```
//! use neuron_cell::wire::{decode, encode, Frame, MsgType};
//! let f = Frame::new(MsgType::Data, 7, 1, vec![1, 2, 3]);
//! assert_eq!(decode(&encode(&f).unwrap()).unwrap(), f);
//! ```
use std::collections::BTreeMap;

/// Protocol version carried in the high nibble of `ver_flags`.
pub const VERSION: u8 = 1;
/// Smallest frame: header (8), length (2) and CRC (2).
pub const MIN_FRAME: usize = 12;
/// Largest payload a frame carries (`len` is 16 bits).
pub const MAX_PAYLOAD: usize = u16::MAX as usize;

const FLAG_ACK: u8 = 0x01;
const FLAG_TS: u8 = 0x02;
const FLAG_MAC: u8 = 0x04;
const FLAG_RESERVED: u8 = 0x08;

/// Message types of spec 11.3.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MsgType {
    /// Version, device id, tag table hash, features.
    Hello = 0x01,
    /// One or more tag values.
    Data = 0x02,
    /// Acknowledges a `seq`.
    Ack = 0x03,
    /// State and uptime.
    Heartbeat = 0x04,
    /// Starts a clock-offset measurement.
    TimesyncReq = 0x05,
    /// Answers with receive and send timestamps.
    TimesyncRsp = 0x06,
    /// Fault code and the tag involved.
    Fault = 0x07,
    /// Command from the supervision plane.
    Cmd = 0x08,
    /// Rejected frame with a reason.
    Nack = 0x09,
}

impl MsgType {
    /// The type of a code byte.
    pub fn from_code(code: u8) -> Option<MsgType> {
        Some(match code {
            0x01 => MsgType::Hello,
            0x02 => MsgType::Data,
            0x03 => MsgType::Ack,
            0x04 => MsgType::Heartbeat,
            0x05 => MsgType::TimesyncReq,
            0x06 => MsgType::TimesyncRsp,
            0x07 => MsgType::Fault,
            0x08 => MsgType::Cmd,
            0x09 => MsgType::Nack,
            _ => return None,
        })
    }
}

/// One Neuron Wire message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    /// The sender asks for an ACK.
    pub ack_requested: bool,
    /// Message type.
    pub msg_type: MsgType,
    /// Random at link start; ties frames to one connection.
    pub session: u16,
    /// Per-sender counter, wraps.
    pub seq: u32,
    /// Sender clock in nanoseconds, when timestamped.
    pub ts: Option<u64>,
    /// Little-endian values, no padding.
    pub payload: Vec<u8>,
    /// Truncated HMAC when authenticated (carried, never computed here).
    pub mac: Option<[u8; 8]>,
}

impl Frame {
    /// A plain frame: no ACK request, no timestamp, no MAC.
    pub fn new(msg_type: MsgType, session: u16, seq: u32, payload: Vec<u8>) -> Frame {
        Frame {
            ack_requested: false,
            msg_type,
            session,
            seq,
            ts: None,
            payload,
            mac: None,
        }
    }

    /// A DATA frame carrying one tag value: `tag_id (u16)` then the value bytes.
    pub fn data(session: u16, seq: u32, tag_id: u16, value: &[u8]) -> Frame {
        let mut payload = tag_id.to_le_bytes().to_vec();
        payload.extend_from_slice(value);
        Frame::new(MsgType::Data, session, seq, payload)
    }
}

/// Why a frame was not encoded or accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireError {
    /// The payload does not fit the 16-bit length.
    PayloadTooLong,
    /// Shorter than its own header says, or shorter than a minimal frame.
    Truncated,
    /// Bytes left over after the CRC.
    TrailingBytes,
    /// The CRC does not match.
    BadCrc,
    /// The version nibble is not 1.
    BadVersion,
    /// A reserved flag bit is set.
    BadFlags,
    /// Unknown message type.
    BadType,
    /// A COBS block is malformed.
    BadCobs,
}

impl std::fmt::Display for WireError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            WireError::PayloadTooLong => "payload longer than 65535 bytes",
            WireError::Truncated => "frame is truncated",
            WireError::TrailingBytes => "bytes after the CRC",
            WireError::BadCrc => "CRC mismatch",
            WireError::BadVersion => "unsupported version",
            WireError::BadFlags => "reserved flag bit set",
            WireError::BadType => "unknown message type",
            WireError::BadCobs => "malformed COBS block",
        })
    }
}

impl std::error::Error for WireError {}

/// CRC-16/CCITT-FALSE: polynomial 0x1021, init 0xFFFF, no reflection, no final xor.
///
/// ```
/// assert_eq!(neuron_cell::wire::crc16(b"123456789"), 0x29B1);
/// ```
pub fn crc16(data: &[u8]) -> u16 {
    let mut crc: u16 = 0xFFFF;
    for &b in data {
        crc ^= u16::from(b) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x1021
            } else {
                crc << 1
            };
        }
    }
    crc
}

/// Encodes a frame (without the transport wrapper).
///
/// # Errors
/// `PayloadTooLong` when the payload exceeds 65535 bytes.
pub fn encode(f: &Frame) -> Result<Vec<u8>, WireError> {
    let len = u16::try_from(f.payload.len()).map_err(|_| WireError::PayloadTooLong)?;
    let mut flags = VERSION << 4;
    if f.ack_requested {
        flags |= FLAG_ACK;
    }
    if f.ts.is_some() {
        flags |= FLAG_TS;
    }
    if f.mac.is_some() {
        flags |= FLAG_MAC;
    }
    let mut out = Vec::with_capacity(MIN_FRAME + 16 + f.payload.len());
    out.push(flags);
    out.push(f.msg_type as u8);
    out.extend_from_slice(&f.session.to_le_bytes());
    out.extend_from_slice(&f.seq.to_le_bytes());
    if let Some(ts) = f.ts {
        out.extend_from_slice(&ts.to_le_bytes());
    }
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(&f.payload);
    if let Some(mac) = f.mac {
        out.extend_from_slice(&mac);
    }
    let crc = crc16(&out);
    out.extend_from_slice(&crc.to_le_bytes());
    Ok(out)
}

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], WireError> {
        let end = self.pos.checked_add(n).ok_or(WireError::Truncated)?;
        let s = self.bytes.get(self.pos..end).ok_or(WireError::Truncated)?;
        self.pos = end;
        Ok(s)
    }
    fn u16(&mut self) -> Result<u16, WireError> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }
}

/// Decodes one frame. The CRC is checked first, so any corrupted frame is rejected
/// before its fields are trusted.
///
/// # Errors
/// A [`WireError`] naming the first problem.
pub fn decode(bytes: &[u8]) -> Result<Frame, WireError> {
    if bytes.len() < MIN_FRAME {
        return Err(WireError::Truncated);
    }
    let (body, tail) = bytes.split_at(bytes.len() - 2);
    if crc16(body) != u16::from_le_bytes([tail[0], tail[1]]) {
        return Err(WireError::BadCrc);
    }
    let mut r = Reader {
        bytes: body,
        pos: 0,
    };
    let head = r.take(8)?;
    let flags = head[0];
    if flags >> 4 != VERSION {
        return Err(WireError::BadVersion);
    }
    if flags & FLAG_RESERVED != 0 {
        return Err(WireError::BadFlags);
    }
    let msg_type = MsgType::from_code(head[1]).ok_or(WireError::BadType)?;
    let session = u16::from_le_bytes([head[2], head[3]]);
    let seq = u32::from_le_bytes([head[4], head[5], head[6], head[7]]);
    let ts = if flags & FLAG_TS != 0 {
        let b = r.take(8)?;
        let mut a = [0u8; 8];
        a.copy_from_slice(b);
        Some(u64::from_le_bytes(a))
    } else {
        None
    };
    let len = usize::from(r.u16()?);
    let payload = r.take(len)?.to_vec();
    let mac = if flags & FLAG_MAC != 0 {
        let mut a = [0u8; 8];
        a.copy_from_slice(r.take(8)?);
        Some(a)
    } else {
        None
    };
    if r.pos != body.len() {
        return Err(WireError::TrailingBytes);
    }
    Ok(Frame {
        ack_requested: flags & FLAG_ACK != 0,
        msg_type,
        session,
        seq,
        ts,
        payload,
        mac,
    })
}

/// COBS-encodes `data`: the result contains no `0x00` byte.
pub fn cobs_encode(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + data.len() / 254 + 2);
    out.push(0);
    let mut code_at = 0;
    let mut code: u8 = 1;
    for &b in data {
        if b == 0 {
            out[code_at] = code;
            code_at = out.len();
            out.push(0);
            code = 1;
        } else {
            out.push(b);
            code += 1;
            if code == 0xFF {
                out[code_at] = code;
                code_at = out.len();
                out.push(0);
                code = 1;
            }
        }
    }
    out[code_at] = code;
    out
}

/// Reverses [`cobs_encode`].
///
/// # Errors
/// `BadCobs` for a zero byte inside the data or a block that overruns the input.
pub fn cobs_decode(data: &[u8]) -> Result<Vec<u8>, WireError> {
    let mut out = Vec::with_capacity(data.len());
    let mut i = 0;
    while i < data.len() {
        let code = usize::from(data[i]);
        if code == 0 {
            return Err(WireError::BadCobs);
        }
        let end = i + code;
        if end > data.len() {
            return Err(WireError::BadCobs);
        }
        out.extend_from_slice(&data[i + 1..end]);
        i = end;
        if code < 0xFF && i < data.len() {
            out.push(0);
        }
    }
    Ok(out)
}

/// A frame ready for a serial line: COBS-encoded and ended by the `0x00` delimiter.
///
/// # Errors
/// As [`encode`].
pub fn serial_frame(f: &Frame) -> Result<Vec<u8>, WireError> {
    let mut out = cobs_encode(&encode(f)?);
    out.push(0);
    Ok(out)
}

/// Decodes the bytes between two delimiters of a serial line (the delimiter excluded).
///
/// # Errors
/// A COBS or frame error.
pub fn serial_unframe(block: &[u8]) -> Result<Frame, WireError> {
    decode(&cobs_decode(block)?)
}

/// A TCP frame: a 2-byte little-endian length, then the frame.
///
/// # Errors
/// As [`encode`]; `PayloadTooLong` also when the frame itself exceeds 65535 bytes.
pub fn tcp_frame(f: &Frame) -> Result<Vec<u8>, WireError> {
    let body = encode(f)?;
    let len = u16::try_from(body.len()).map_err(|_| WireError::PayloadTooLong)?;
    let mut out = len.to_le_bytes().to_vec();
    out.extend_from_slice(&body);
    Ok(out)
}

/// What the duplicate filter decided about a `(session, seq)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// First time: apply the frame.
    Apply,
    /// Already applied.
    Duplicate,
    /// Older than the filter window (more than 63 behind the newest): discarded, because
    /// the filter cannot tell whether it was applied.
    Stale,
}

#[derive(Debug, Clone, Copy)]
struct SessionState {
    newest: u32,
    /// Bit `i` is set when `newest - i` was applied.
    seen: u64,
}

/// The receiver's duplicate filter (property WIRE-3): a frame is applied at most once
/// per `(session, seq)`. It keeps a 64-frame window per session, using wrapping
/// sequence arithmetic, so memory is bounded and a wrap of `seq` is handled.
#[derive(Debug, Clone, Default)]
pub struct DedupFilter {
    sessions: BTreeMap<u16, SessionState>,
}

impl DedupFilter {
    /// An empty filter.
    pub fn new() -> DedupFilter {
        DedupFilter::default()
    }

    /// Judges one frame and remembers it when it is applied.
    pub fn check(&mut self, session: u16, seq: u32) -> Verdict {
        let Some(s) = self.sessions.get_mut(&session) else {
            self.sessions.insert(
                session,
                SessionState {
                    newest: seq,
                    seen: 1,
                },
            );
            return Verdict::Apply;
        };
        let ahead = seq.wrapping_sub(s.newest);
        if ahead == 0 {
            return Verdict::Duplicate;
        }
        if ahead < 0x8000_0000 {
            // Newer: slide the window.
            s.seen = if ahead >= 64 { 0 } else { s.seen << ahead };
            s.seen |= 1;
            s.newest = seq;
            return Verdict::Apply;
        }
        let behind = s.newest.wrapping_sub(seq);
        if behind >= 64 {
            return Verdict::Stale;
        }
        let bit = 1u64 << behind;
        if s.seen & bit != 0 {
            Verdict::Duplicate
        } else {
            s.seen |= bit;
            Verdict::Apply
        }
    }
}
