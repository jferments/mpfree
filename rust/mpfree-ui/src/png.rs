//! A minimal PNG writer for screenshots (RGB, 8 bits), with no extra
//! dependency: [`encode_rgb`] stores the pixels without compression (large
//! files, for the host's sample screens); [`encode_rgb_small`] packs them
//! (a Sub filter on every row, then deflate's fixed codes with runs and
//! rows repeated from the row above), which keeps a picture of the screen
//! well under a megabyte for the MPC's screen picture.

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let crc = crc32(&out[start..]);
    out.extend_from_slice(&crc.to_be_bytes());
}

/// Encode `rgb` (width x height x 3 bytes, rows top to bottom) as a PNG file.
pub fn encode_rgb(width: u32, height: u32, rgb: &[u8]) -> Vec<u8> {
    assert_eq!(
        rgb.len(),
        width as usize * height as usize * 3,
        "pixel data does not match the size"
    );
    let mut raw = Vec::with_capacity(rgb.len() + height as usize);
    for row in rgb.chunks(width as usize * 3) {
        raw.push(0); // filter: none
        raw.extend_from_slice(row);
    }
    // zlib stream of stored deflate blocks.
    let mut z = vec![0x78, 0x01];
    let blocks: Vec<&[u8]> = raw.chunks(65_535).collect();
    for (i, b) in blocks.iter().enumerate() {
        z.push(if i + 1 == blocks.len() { 1 } else { 0 });
        z.extend_from_slice(&(b.len() as u16).to_le_bytes());
        z.extend_from_slice(&(!(b.len() as u16)).to_le_bytes());
        z.extend_from_slice(b);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &x in &raw {
        a = (a + x as u32) % 65_521;
        b = (b + a) % 65_521;
    }
    z.extend_from_slice(&(b << 16 | a).to_be_bytes());

    let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]); // 8-bit RGB
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &z);
    chunk(&mut out, b"IEND", &[]);
    out
}

/// Bits written into bytes, least significant first, as deflate wants.
struct Bits {
    out: Vec<u8>,
    acc: u64,
    n: u32,
}

impl Bits {
    fn put(&mut self, value: u32, n: u32) {
        self.acc |= u64::from(value) << self.n;
        self.n += n;
        while self.n >= 8 {
            self.out.push(self.acc as u8);
            self.acc >>= 8;
            self.n -= 8;
        }
    }

    /// A Huffman code, which goes most significant bit first.
    fn code(&mut self, code: u32, n: u32) {
        self.put(code.reverse_bits() >> (32 - n), n);
    }

    fn finish(mut self) -> Vec<u8> {
        if self.n > 0 {
            self.out.push(self.acc as u8);
        }
        self.out
    }
}

/// Deflate's fixed code for literal or length symbol `v` (0 to 287).
fn fixed_lit(b: &mut Bits, v: u32) {
    match v {
        0..=143 => b.code(0x30 + v, 8),
        144..=255 => b.code(0x190 + v - 144, 9),
        256..=279 => b.code(v - 256, 7),
        _ => b.code(0xC0 + v - 280, 8),
    }
}

const LEN_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];
const LEN_EXTRA: [u8; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];
const DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
const DIST_EXTRA: [u8; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];

/// A copy of `len` bytes (3 to 258) from `dist` back (1 to 32768).
fn fixed_match(b: &mut Bits, len: usize, dist: usize) {
    let l = LEN_BASE.partition_point(|&x| usize::from(x) <= len) - 1;
    fixed_lit(b, 257 + l as u32);
    b.put(
        (len - usize::from(LEN_BASE[l])) as u32,
        u32::from(LEN_EXTRA[l]),
    );
    let d = DIST_BASE.partition_point(|&x| usize::from(x) <= dist) - 1;
    b.code(d as u32, 5);
    b.put(
        (dist - usize::from(DIST_BASE[d])) as u32,
        u32::from(DIST_EXTRA[d]),
    );
}

/// How many bytes from `i` repeat those `dist` back, up to 258.
fn run_at(data: &[u8], i: usize, dist: usize) -> usize {
    if dist == 0 || dist > i {
        return 0;
    }
    let max = (data.len() - i).min(258);
    (0..max)
        .position(|k| data[i + k] != data[i + k - dist])
        .unwrap_or(max)
}

/// `data` as one deflate block with the fixed codes: each place copies
/// the byte before it (a run) or the row above (`row` bytes back) when
/// three or more bytes repeat, else is a literal.
fn deflate_fixed(data: &[u8], row: usize) -> Vec<u8> {
    let mut b = Bits {
        out: Vec::with_capacity(data.len() / 4),
        acc: 0,
        n: 0,
    };
    b.put(1, 1); // the last block
    b.put(1, 2); // fixed codes
    let row = if row <= 32_768 { row } else { 0 };
    let mut i = 0;
    while i < data.len() {
        let run = run_at(data, i, 1);
        let up = run_at(data, i, row);
        let (len, dist) = if up > run { (up, row) } else { (run, 1) };
        if len >= 3 {
            fixed_match(&mut b, len, dist);
            i += len;
        } else {
            fixed_lit(&mut b, u32::from(data[i]));
            i += 1;
        }
    }
    fixed_lit(&mut b, 256);
    b.finish()
}

/// Encode `rgb` (width x height x 3 bytes, rows top to bottom) as a packed
/// PNG file: every row Sub-filtered, then deflated with the fixed codes.
/// A screen of flat panels and text packs to a small part of its size.
pub fn encode_rgb_small(width: u32, height: u32, rgb: &[u8]) -> Vec<u8> {
    let stride = width as usize * 3;
    assert_eq!(
        rgb.len(),
        stride * height as usize,
        "pixel data does not match the size"
    );
    let mut raw = Vec::with_capacity(rgb.len() + height as usize);
    for row in rgb.chunks(stride.max(1)) {
        raw.push(1); // filter: Sub
        for (i, &x) in row.iter().enumerate() {
            let left = if i >= 3 { row[i - 3] } else { 0 };
            raw.push(x.wrapping_sub(left));
        }
    }
    let mut z = vec![0x78, 0x01];
    z.extend(deflate_fixed(&raw, stride + 1));
    let (mut a, mut b) = (1u32, 0u32);
    for &x in &raw {
        a = (a + x as u32) % 65_521;
        b = (b + a) % 65_521;
    }
    z.extend_from_slice(&(b << 16 | a).to_be_bytes());
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]); // 8-bit RGB
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &z);
    chunk(&mut out, b"IEND", &[]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_crc_and_layout() {
        assert_eq!(crc32(b"IEND"), 0xAE42_6082);
        let png = encode_rgb(2, 1, &[255, 0, 0, 0, 0, 255]);
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(
            &png[png.len() - 12..],
            &[0, 0, 0, 0, b'I', b'E', b'N', b'D', 0xAE, 0x42, 0x60, 0x82]
        );
    }
}
