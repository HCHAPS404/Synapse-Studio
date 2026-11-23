//! UF2 files: the format the RP2040 and RP2350 boot ROMs accept on their USB drive.
//!
//! A UF2 file is a list of 512-byte blocks, each carrying 256 bytes of payload and the address
//! to write them to (format published by Microsoft under the MIT licence).

/// Family id of the RP2040.
pub const FAMILY_RP2040: u32 = 0xe48b_ff56;
/// Family id of the RP2350 running Arm code in secure mode.
pub const FAMILY_RP2350_ARM_S: u32 = 0xe48b_ff59;
/// Family id of the Adafruit nRF52840 UF2 bootloader (Arduino Nano 33 BLE).
pub const FAMILY_NRF52840: u32 = 0xada5_2840;

const MAGIC0: u32 = 0x0a32_4655;
const MAGIC1: u32 = 0x9e5d_5157;
const MAGIC_END: u32 = 0x0ab1_6f30;
const FLAG_FAMILY: u32 = 0x0000_2000;
const PAYLOAD: usize = 256;

/// The UF2 image of `data` loaded at `address`.
///
/// ```
/// let f = neuron_codegen_c::uf2::uf2(&[1, 2, 3], 0x2000_0000, neuron_codegen_c::uf2::FAMILY_RP2040);
/// assert_eq!(f.len(), 512);
/// assert_eq!(&f[0..4], &0x0a32_4655u32.to_le_bytes());
/// ```
pub fn uf2(data: &[u8], address: u32, family: u32) -> Vec<u8> {
    let chunks: Vec<&[u8]> = if data.is_empty() {
        vec![&[][..]]
    } else {
        data.chunks(PAYLOAD).collect()
    };
    let total = chunks.len() as u32;
    let mut out = Vec::with_capacity(chunks.len() * 512);
    for (i, chunk) in chunks.iter().enumerate() {
        let mut block = [0u8; 512];
        let words = [
            MAGIC0,
            MAGIC1,
            FLAG_FAMILY,
            address + (i * PAYLOAD) as u32,
            PAYLOAD as u32,
            i as u32,
            total,
            family,
        ];
        for (k, w) in words.iter().enumerate() {
            block[4 * k..4 * k + 4].copy_from_slice(&w.to_le_bytes());
        }
        block[32..32 + chunk.len()].copy_from_slice(chunk);
        block[508..512].copy_from_slice(&MAGIC_END.to_le_bytes());
        out.extend_from_slice(&block);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(b: &[u8], at: usize) -> u32 {
        u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
    }

    #[test]
    fn blocks_carry_their_address_number_and_family() {
        let data: Vec<u8> = (0..600u32).map(|i| i as u8).collect();
        let f = uf2(&data, 0x2000_0000, FAMILY_RP2040);
        assert_eq!(f.len(), 3 * 512);
        for (i, b) in f.chunks(512).enumerate() {
            assert_eq!(word(b, 0), MAGIC0);
            assert_eq!(word(b, 4), MAGIC1);
            assert_eq!(word(b, 12), 0x2000_0000 + 256 * i as u32);
            assert_eq!(word(b, 20), i as u32);
            assert_eq!(word(b, 24), 3);
            assert_eq!(word(b, 28), FAMILY_RP2040);
            assert_eq!(word(b, 508), MAGIC_END);
        }
        assert_eq!(f[32], 0);
        assert_eq!(f[512 + 32], 0); // byte 256 of the data is 256 as u8 = 0
        assert_eq!(f[2 * 512 + 32 + 87], 87); // byte 599 is 599 mod 256
    }
}
