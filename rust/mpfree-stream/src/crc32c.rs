//! CRC-32C (Castagnoli), the checksum the recording files use: the same
//! polynomial as ext4, iSCSI and SSE 4.2, worked out eight bytes at a time
//! from tables built at compile time.

const POLY: u32 = 0x82F6_3B78; // 0x1EDC6F41 reflected

const fn tables() -> [[u32; 256]; 8] {
    let mut t = [[0u32; 256]; 8];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 { (c >> 1) ^ POLY } else { c >> 1 };
            k += 1;
        }
        t[0][i] = c;
        i += 1;
    }
    let mut s = 1;
    while s < 8 {
        let mut i = 0;
        while i < 256 {
            let prev = t[s - 1][i];
            t[s][i] = (prev >> 8) ^ t[0][(prev & 0xFF) as usize];
            i += 1;
        }
        s += 1;
    }
    t
}

static T: [[u32; 256]; 8] = tables();

/// The CRC-32C of `bytes` following on from `crc` (0 to start), so a long
/// run can be checked a piece at a time: `update(update(0, a), b)` is the
/// checksum of `a` then `b`.
pub fn update(crc: u32, bytes: &[u8]) -> u32 {
    let mut c = !crc;
    let mut chunks = bytes.chunks_exact(8);
    for b in &mut chunks {
        let lo = c ^ u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
        c = T[7][(lo & 0xFF) as usize]
            ^ T[6][((lo >> 8) & 0xFF) as usize]
            ^ T[5][((lo >> 16) & 0xFF) as usize]
            ^ T[4][(lo >> 24) as usize]
            ^ T[3][b[4] as usize]
            ^ T[2][b[5] as usize]
            ^ T[1][b[6] as usize]
            ^ T[0][b[7] as usize];
    }
    for &b in chunks.remainder() {
        c = (c >> 8) ^ T[0][((c ^ u32::from(b)) & 0xFF) as usize];
    }
    !c
}

/// The CRC-32C of `bytes`.
pub fn of(bytes: &[u8]) -> u32 {
    update(0, bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_values() {
        assert_eq!(of(b""), 0);
        assert_eq!(of(b"123456789"), 0xE306_9283);
        assert_eq!(of(&[0u8; 32]), 0x8A91_36AA);
        assert_eq!(of(&[0xFFu8; 32]), 0x62A8_AB43);
    }

    #[test]
    fn pieces_match_the_whole() {
        let data: Vec<u8> = (0..1000u32).map(|i| (i * 7 + i / 3) as u8).collect();
        let whole = of(&data);
        for cut in [0, 1, 7, 8, 9, 500, 999, 1000] {
            assert_eq!(
                update(of(&data[..cut]), &data[cut..]),
                whole,
                "cut at {cut}"
            );
        }
        // Against the plain bitwise definition.
        let mut c = !0u32;
        for &b in &data {
            c ^= u32::from(b);
            for _ in 0..8 {
                c = if c & 1 != 0 { (c >> 1) ^ POLY } else { c >> 1 };
            }
        }
        assert_eq!(!c, whole);
    }
}
