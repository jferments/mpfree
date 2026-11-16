//! DEFLATE (RFC 1951) and its zlib wrapper (RFC 1950), for the PNG files
//! skins are drawn from (and plugin release zips). Decoding only. CRC-32 for
//! PNG chunks and zip entries is here too.
//!
//! Every call takes a limit on the output, so a small file can't make it
//! allocate without bound.

/// Codes up to this many bits are decoded with one table lookup.
const FAST: u32 = 10;

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
/// The order the code length code's lengths are stored in.
const CL_ORDER: [usize; 19] = [
    16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
];

struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
    buf: u64,
    count: u32,
    /// Zero bits added past the end of `data`; consuming any is an error.
    padded: u32,
}

impl<'a> Bits<'a> {
    fn new(data: &'a [u8]) -> Bits<'a> {
        Bits {
            data,
            pos: 0,
            buf: 0,
            count: 0,
            padded: 0,
        }
    }

    fn refill(&mut self) {
        while self.count <= 56 {
            if let Some(&b) = self.data.get(self.pos) {
                self.buf |= (b as u64) << self.count;
                self.pos += 1;
            } else {
                self.padded += 8;
            }
            self.count += 8;
        }
    }

    fn take(&mut self, n: u32) -> u32 {
        if n == 0 {
            return 0;
        }
        if self.count < n {
            self.refill();
        }
        let v = (self.buf & ((1u64 << n) - 1)) as u32;
        self.buf >>= n;
        self.count -= n;
        v
    }

    /// True if more bits were used than the input holds.
    fn overran(&self) -> bool {
        self.count < self.padded
    }

    /// Drops the bits up to the next byte boundary.
    fn align(&mut self) {
        let r = self.count % 8;
        self.take(r);
    }
}

struct Huffman {
    /// For each FAST-bit pattern (read least significant bit first): the
    /// symbol << 4 | its length, or 0 when the code is longer.
    fast: Vec<u16>,
    counts: [u16; 16],
    /// Symbols ordered by code length, then value (canonical order).
    symbols: Vec<u16>,
}

impl Huffman {
    fn new(lengths: &[u8]) -> Result<Huffman, String> {
        let mut counts = [0u16; 16];
        for &l in lengths {
            counts[l as usize] += 1;
        }
        counts[0] = 0;
        let mut left: i32 = 1;
        for &c in &counts[1..] {
            left = (left << 1) - c as i32;
            if left < 0 {
                return Err("an over-subscribed Huffman code".into());
            }
        }
        let mut offs = [0u16; 16];
        for l in 1..15 {
            offs[l + 1] = offs[l] + counts[l];
        }
        let mut symbols = vec![0u16; lengths.len()];
        for (s, &l) in lengths.iter().enumerate() {
            if l != 0 {
                symbols[offs[l as usize] as usize] = s as u16;
                offs[l as usize] += 1;
            }
        }
        let mut fast = vec![0u16; 1 << FAST];
        let mut code: u32 = 0;
        let mut next = [0u32; 16];
        for l in 1..16 {
            code = (code + counts[l - 1] as u32) << 1;
            next[l] = code;
        }
        for (s, &l) in lengths.iter().enumerate() {
            let l = l as u32;
            if l == 0 || l > FAST {
                continue;
            }
            let c = next[l as usize];
            next[l as usize] += 1;
            let rev = c.reverse_bits() >> (32 - l);
            let mut i = rev;
            while i < (1 << FAST) {
                fast[i as usize] = ((s as u16) << 4) | l as u16;
                i += 1 << l;
            }
        }
        Ok(Huffman {
            fast,
            counts,
            symbols,
        })
    }

    fn decode(&self, b: &mut Bits) -> Result<u16, String> {
        if b.count < 15 {
            b.refill();
        }
        let e = self.fast[(b.buf & ((1 << FAST) - 1)) as usize];
        if e != 0 {
            let l = (e & 15) as u32;
            b.buf >>= l;
            b.count -= l;
            return Ok(e >> 4);
        }
        // A long code: canonical decoding one bit at a time.
        let (mut code, mut first, mut index) = (0i32, 0i32, 0i32);
        for l in 1..16 {
            code |= b.take(1) as i32;
            let count = self.counts[l] as i32;
            if code - first < count {
                return Ok(self.symbols[(index + code - first) as usize]);
            }
            index += count;
            first = (first + count) << 1;
            code <<= 1;
        }
        Err("a Huffman code not in its table".into())
    }
}

fn fixed() -> (Huffman, Huffman) {
    let mut lit = [0u8; 288];
    lit[..144].fill(8);
    lit[144..256].fill(9);
    lit[256..280].fill(7);
    lit[280..].fill(8);
    let dist = [5u8; 30];
    (
        Huffman::new(&lit).expect("fixed code"),
        Huffman::new(&dist).expect("fixed code"),
    )
}

fn dynamic(b: &mut Bits) -> Result<(Huffman, Huffman), String> {
    let hlit = b.take(5) as usize + 257;
    let hdist = b.take(5) as usize + 1;
    let hclen = b.take(4) as usize + 4;
    if hlit > 286 || hdist > 30 {
        return Err("too many codes in a block header".into());
    }
    let mut cl = [0u8; 19];
    for &i in &CL_ORDER[..hclen] {
        cl[i] = b.take(3) as u8;
    }
    let clh = Huffman::new(&cl)?;
    let mut lens = vec![0u8; hlit + hdist];
    let mut i = 0;
    while i < lens.len() {
        let sym = clh.decode(b)?;
        let (val, n) = match sym {
            0..=15 => (sym as u8, 1),
            16 => {
                if i == 0 {
                    return Err("a repeat with nothing to repeat".into());
                }
                (lens[i - 1], 3 + b.take(2) as usize)
            }
            17 => (0, 3 + b.take(3) as usize),
            18 => (0, 11 + b.take(7) as usize),
            _ => return Err("a bad code length code".into()),
        };
        if i + n > lens.len() {
            return Err("code lengths run past the end".into());
        }
        lens[i..i + n].fill(val);
        i += n;
        if b.overran() {
            return Err("the data ends in a block header".into());
        }
    }
    if lens[256] == 0 {
        return Err("a block with no end code".into());
    }
    Ok((Huffman::new(&lens[..hlit])?, Huffman::new(&lens[hlit..])?))
}

/// Decodes raw DEFLATE data. Fails on bad data, on data that stops early,
/// and when the output would pass `limit` bytes. Returns the output and how
/// many input bytes were used.
pub fn inflate_raw(data: &[u8], limit: usize) -> Result<(Vec<u8>, usize), String> {
    let mut out: Vec<u8> = Vec::with_capacity(limit.min(data.len().saturating_mul(4)));
    let mut b = Bits::new(data);
    loop {
        let last = b.take(1) == 1;
        match b.take(2) {
            0 => {
                b.align();
                let len = b.take(16) as usize;
                let nlen = b.take(16) as usize;
                if len != !nlen & 0xFFFF {
                    return Err("a stored block's length check fails".into());
                }
                if out.len() + len > limit {
                    return Err(format!("more than {limit} bytes"));
                }
                // Bytes still in the bit buffer first, then straight from
                // the input.
                let mut left = len;
                while left > 0 && b.count >= 8 {
                    out.push(b.take(8) as u8);
                    left -= 1;
                }
                if b.overran() {
                    return Err("the data ends in a stored block".into());
                }
                let end = b.pos + left;
                if end > data.len() {
                    return Err("the data ends in a stored block".into());
                }
                out.extend_from_slice(&data[b.pos..end]);
                b.pos = end;
            }
            t @ (1 | 2) => {
                let (lit, dist) = if t == 1 { fixed() } else { dynamic(&mut b)? };
                loop {
                    let sym = lit.decode(&mut b)?;
                    if sym < 256 {
                        if out.len() >= limit {
                            return Err(format!("more than {limit} bytes"));
                        }
                        out.push(sym as u8);
                        continue;
                    }
                    if sym == 256 {
                        break;
                    }
                    let li = (sym - 257) as usize;
                    if li >= 29 {
                        return Err("a bad length code".into());
                    }
                    let len = LEN_BASE[li] as usize + b.take(LEN_EXTRA[li] as u32) as usize;
                    let di = dist.decode(&mut b)? as usize;
                    if di >= 30 {
                        return Err("a bad distance code".into());
                    }
                    let d = DIST_BASE[di] as usize + b.take(DIST_EXTRA[di] as u32) as usize;
                    if d > out.len() {
                        return Err("a distance back past the start".into());
                    }
                    if out.len() + len > limit {
                        return Err(format!("more than {limit} bytes"));
                    }
                    let start = out.len() - d;
                    if d >= len {
                        out.extend_from_within(start..start + len);
                    } else {
                        for k in 0..len {
                            let v = out[start + k];
                            out.push(v);
                        }
                    }
                    if b.overran() {
                        return Err("the data ends in the middle of a block".into());
                    }
                }
            }
            _ => return Err("a block of the reserved type".into()),
        }
        if b.overran() {
            return Err("the data ends in the middle of a block".into());
        }
        if last {
            break;
        }
    }
    // Whole bytes still buffered were not used.
    let used = b.pos - (b.count.saturating_sub(b.padded) / 8) as usize;
    Ok((out, used))
}

/// Decodes zlib data (a PNG's image data), checking its header and Adler-32.
pub fn inflate_zlib(data: &[u8], limit: usize) -> Result<Vec<u8>, String> {
    if data.len() < 6 {
        return Err("zlib data too short".into());
    }
    let (cmf, flg) = (data[0], data[1]);
    if cmf & 15 != 8 || cmf >> 4 > 7 || !((cmf as u16) << 8 | flg as u16).is_multiple_of(31) {
        return Err("not zlib data".into());
    }
    if flg & 0x20 != 0 {
        return Err("zlib data with a preset dictionary".into());
    }
    let (out, used) = inflate_raw(&data[2..], limit)?;
    let at = 2 + used;
    let want = data
        .get(at..at + 4)
        .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]));
    match want {
        Some(w) if w == adler32(&out) => Ok(out),
        Some(_) => Err("the zlib checksum doesn't match".into()),
        None => Err("zlib data without its checksum".into()),
    }
}

pub fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for chunk in data.chunks(5552) {
        for &x in chunk {
            a += x as u32;
            b += a;
        }
        a %= 65521;
        b %= 65521;
    }
    b << 16 | a
}

const fn crc_table() -> [u32; 256] {
    let mut t = [0u32; 256];
    let mut n = 0;
    while n < 256 {
        let mut c = n as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
            k += 1;
        }
        t[n] = c;
        n += 1;
    }
    t
}

static CRC: [u32; 256] = crc_table();

/// CRC-32 (the one PNG and zip use), continued from `crc` (0 to start).
pub fn crc32_update(crc: u32, data: &[u8]) -> u32 {
    let mut c = !crc;
    for &b in data {
        c = CRC[((c ^ b as u32) & 0xFF) as usize] ^ (c >> 8);
    }
    !c
}

pub fn crc32(data: &[u8]) -> u32 {
    crc32_update(0, data)
}

#[cfg(test)]
mod tests {
    use super::*;

    // zlib.compress(b"hello hello hello hello", 9) from Python.
    const HELLO: &[u8] = &[
        0x78, 0xda, 0xcb, 0x48, 0xcd, 0xc9, 0xc9, 0x57, 0xc8, 0x40, 0x27, 0x01, 0x68, 0x03, 0x08,
        0xb1,
    ];

    #[test]
    fn fixed_huffman_with_a_back_reference() {
        assert_eq!(
            inflate_zlib(HELLO, 1000).unwrap(),
            b"hello hello hello hello"
        );
    }

    #[test]
    fn stored_blocks() {
        // One stored block holding "abc", then the zlib trailer.
        let mut z = vec![0x78, 0x01, 0x01, 3, 0, 0xfc, 0xff, b'a', b'b', b'c'];
        z.extend_from_slice(&adler32(b"abc").to_be_bytes());
        assert_eq!(inflate_zlib(&z, 10).unwrap(), b"abc");
        // The limit holds.
        assert!(inflate_zlib(&z, 2).is_err());
        // A cut-off stored block.
        assert!(inflate_raw(&[0x01, 3, 0, 0xfc, 0xff, b'a'], 10).is_err());
    }

    #[test]
    fn dynamic_huffman() {
        // Python: 400 words picked by the LCG below, joined by spaces,
        // zlib.compress(text, 9): one dynamic block with many matches.
        let z = include_bytes!("../testdata/words.zlib");
        let words = [
            "skin",
            "knob",
            "filmstrip",
            "label",
            "parameter",
            "value",
            "focus",
            "tab",
        ];
        let mut x: u64 = 1;
        let mut want = Vec::new();
        for _ in 0..400 {
            x = (x * 1_103_515_245 + 12_345) % (1 << 31);
            want.push(words[((x >> 16) % 8) as usize]);
        }
        let want = want.join(" ");
        assert_eq!(z[2] >> 1 & 3, 2, "a dynamic block");
        assert_eq!(inflate_zlib(z, 1 << 20).unwrap(), want.as_bytes());
        assert!(inflate_zlib(z, want.len() - 1).is_err(), "the limit holds");
    }

    #[test]
    fn refuses_bad_data() {
        assert!(inflate_zlib(&HELLO[..HELLO.len() - 1], 100).is_err());
        let mut bad = HELLO.to_vec();
        bad[5] ^= 0x40;
        assert!(inflate_zlib(&bad, 100).is_err());
        assert!(inflate_raw(&[0x07], 100).is_err(), "reserved block type");
        assert!(inflate_zlib(&[0, 0, 0, 0, 0, 0], 100).is_err(), "not zlib");
    }

    #[test]
    fn checksums() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32_update(crc32(b"1234"), b"56789"), 0xCBF4_3926);
        assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
    }
}
