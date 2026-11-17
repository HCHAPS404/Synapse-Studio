//! Intel HEX reader.

use crate::SimError;

/// Parses an Intel HEX file into a flat byte image starting at address 0.
///
/// Records of type 00 (data), 01 (end of file), 02 and 04 (extended addresses) are read; the
/// checksum of every record is verified.
pub fn parse_hex(text: &str) -> Result<Vec<u8>, SimError> {
    let bad = |line: usize, why: &str| SimError::BadHex(format!("line {line}: {why}"));
    let mut image: Vec<u8> = Vec::new();
    let mut base: usize = 0;
    for (n, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let body = line
            .strip_prefix(':')
            .ok_or_else(|| bad(n + 1, "a record starts with `:`"))?;
        if body.len() % 2 != 0 || body.len() < 10 {
            return Err(bad(n + 1, "bad length"));
        }
        let mut bytes = Vec::with_capacity(body.len() / 2);
        for i in (0..body.len()).step_by(2) {
            bytes.push(
                u8::from_str_radix(&body[i..i + 2], 16)
                    .map_err(|_| bad(n + 1, "not hexadecimal"))?,
            );
        }
        let sum: u8 = bytes.iter().fold(0u8, |a, b| a.wrapping_add(*b));
        if sum != 0 {
            return Err(bad(n + 1, "checksum mismatch"));
        }
        let len = usize::from(bytes[0]);
        let addr = (usize::from(bytes[1]) << 8) | usize::from(bytes[2]);
        if bytes.len() != len + 5 {
            return Err(bad(n + 1, "length field does not match the record"));
        }
        let data = &bytes[4..4 + len];
        match bytes[3] {
            0x00 => {
                let start = base + addr;
                if image.len() < start + len {
                    image.resize(start + len, 0xFF);
                }
                image[start..start + len].copy_from_slice(data);
            }
            0x01 => break,
            0x02 if len == 2 => base = ((usize::from(data[0]) << 8) | usize::from(data[1])) << 4,
            0x04 if len == 2 => base = ((usize::from(data[0]) << 8) | usize::from(data[1])) << 16,
            0x03 | 0x05 => {}
            other => {
                return Err(bad(
                    n + 1,
                    &format!("record type {other:#04x} is not supported"),
                ))
            }
        }
    }
    Ok(image)
}
