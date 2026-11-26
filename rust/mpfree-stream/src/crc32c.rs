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

/// `crc` followed on by `len` zero bytes, as [`update`] would give for
/// them, in a few dozen steps however long the run: the bytes are never
/// made. (Feeding zeros is linear in the register, so it is a 32 x 32
/// matrix over GF(2), squared up to the run's length, as zlib combines
/// checksums.)
pub fn zeros(crc: u32, mut len: u64) -> u32 {
    fn times(mat: &[u32; 32], mut v: u32) -> u32 {
        let mut sum = 0;
        let mut i = 0;
        while v != 0 {
            if v & 1 != 0 {
                sum ^= mat[i];
            }
            v >>= 1;
            i += 1;
        }
        sum
    }
    fn square(mat: &[u32; 32]) -> [u32; 32] {
        let mut sq = [0u32; 32];
        for (n, s) in sq.iter_mut().enumerate() {
            *s = times(mat, mat[n]);
        }
        sq
    }
    if len == 0 {
        return crc;
    }
    // One zero bit: bit 0 shifts out through the polynomial, the rest down.
    let mut odd = [0u32; 32];
    odd[0] = POLY;
    for (n, o) in odd.iter_mut().enumerate().skip(1) {
        *o = 1 << (n - 1);
    }
    let mut even = square(&odd); // two bits
    odd = square(&even); // four bits
    let mut c = !crc;
    loop {
        even = square(&odd); // eight bits (a byte) the first time
        if len & 1 != 0 {
            c = times(&even, c);
        }
        len >>= 1;
        if len == 0 {
            break;
        }
        odd = square(&even);
        if len & 1 != 0 {
            c = times(&odd, c);
        }
        len >>= 1;
        if len == 0 {
            break;
        }
    }
    !c
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
    fn zeros_match_feeding_them() {
        for start in [0u32, 1, 0xE306_9283, 0xFFFF_FFFF] {
            for len in [0u64, 1, 2, 3, 7, 8, 9, 63, 64, 65, 4096, 65_536, 100_003] {
                assert_eq!(
                    zeros(start, len),
                    update(start, &vec![0u8; len as usize]),
                    "start {start:#x}, {len} zeros"
                );
            }
        }
        assert_eq!(zeros(0, 32), 0x8A91_36AA);
        // A run longer than memory would hold, in pieces.
        let big = 3 * (1u64 << 32) + 12_345;
        let mut c = 0;
        let mut left = big;
        while left > 0 {
            let n = left.min(1 << 31);
            c = zeros(c, n);
            left -= n;
        }
        assert_eq!(zeros(0, big), c);
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
