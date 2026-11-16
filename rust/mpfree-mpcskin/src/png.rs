//! PNG decoding, for skin images: every colour type and bit depth, Adam7
//! interlacing, palette and tRNS transparency. Gamma and colour profiles are
//! ignored (the MPC draws the pixels as they are).

use crate::inflate::{crc32_update, inflate_zlib};

/// Pictures larger than this many pixels are refused (the biggest film
/// strip in the catalog is about 10 million).
pub const MAX_PIXELS: u64 = 24_000_000;

/// A decoded picture: one premultiplied ARGB word per pixel (alpha in the
/// top byte), rows top to bottom.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Picture {
    pub w: u32,
    pub h: u32,
    pub px: Vec<u32>,
}

impl Picture {
    pub fn new(w: u32, h: u32) -> Picture {
        Picture {
            w,
            h,
            px: vec![0; (w as usize) * (h as usize)],
        }
    }

    /// Bytes this picture holds.
    pub fn bytes(&self) -> usize {
        self.px.len() * 4
    }
}

/// Premultiplies straight RGBA into an ARGB word.
pub fn premultiply(r: u8, g: u8, b: u8, a: u8) -> u32 {
    if a == 255 {
        return 0xFF00_0000 | (r as u32) << 16 | (g as u32) << 8 | b as u32;
    }
    let m = |c: u8| (c as u32 * a as u32 + 127) / 255;
    (a as u32) << 24 | m(r) << 16 | m(g) << 8 | m(b)
}

struct Header {
    w: u32,
    h: u32,
    depth: u8,
    ctype: u8,
    interlace: bool,
}

impl Header {
    fn channels(&self) -> usize {
        match self.ctype {
            0 | 3 => 1,
            2 => 3,
            4 => 2,
            _ => 4,
        }
    }

    /// Bits per pixel.
    fn bpp_bits(&self) -> usize {
        self.channels() * self.depth as usize
    }

    /// Bytes per row of a (sub)image `w` pixels wide, without the filter byte.
    fn row_bytes(&self, w: u32) -> usize {
        (w as usize * self.bpp_bits()).div_ceil(8)
    }
}

/// Decodes a PNG file.
pub fn decode(data: &[u8]) -> Result<Picture, String> {
    if data.len() < 8 || &data[..8] != b"\x89PNG\r\n\x1a\n" {
        return Err("not a PNG file".into());
    }
    let mut pos = 8;
    let mut header: Option<Header> = None;
    let mut palette: Vec<[u8; 4]> = Vec::new();
    let mut trns: Vec<u8> = Vec::new();
    let mut idat: Vec<u8> = Vec::new();
    loop {
        let len = data
            .get(pos..pos + 4)
            .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize)
            .ok_or("the file ends before IEND")?;
        let end = pos
            .checked_add(12)
            .and_then(|p| p.checked_add(len))
            .filter(|&e| e <= data.len())
            .ok_or("a chunk runs past the end of the file")?;
        let kind = &data[pos + 4..pos + 8];
        let body = &data[pos + 8..pos + 8 + len];
        let crc = u32::from_be_bytes([data[end - 4], data[end - 3], data[end - 2], data[end - 1]]);
        if crc32_update(crc32_update(0, kind), body) != crc {
            return Err(format!(
                "chunk {} fails its CRC",
                String::from_utf8_lossy(kind)
            ));
        }
        match kind {
            b"IHDR" => {
                if body.len() != 13 {
                    return Err("a bad IHDR".into());
                }
                let w = u32::from_be_bytes([body[0], body[1], body[2], body[3]]);
                let h = u32::from_be_bytes([body[4], body[5], body[6], body[7]]);
                let (depth, ctype) = (body[8], body[9]);
                let ok = matches!(
                    (ctype, depth),
                    (0, 1 | 2 | 4 | 8 | 16) | (2 | 4 | 6, 8 | 16) | (3, 1 | 2 | 4 | 8)
                );
                if !ok || body[10] != 0 || body[11] != 0 || body[12] > 1 {
                    return Err(format!("an unsupported PNG (type {ctype}, depth {depth})"));
                }
                if w == 0 || h == 0 || w as u64 * h as u64 > MAX_PIXELS {
                    return Err(format!("a {w}x{h} picture is too big"));
                }
                header = Some(Header {
                    w,
                    h,
                    depth,
                    ctype,
                    interlace: body[12] == 1,
                });
            }
            b"PLTE" => {
                palette = body
                    .chunks_exact(3)
                    .map(|c| [c[0], c[1], c[2], 255])
                    .collect();
            }
            b"tRNS" => trns = body.to_vec(),
            b"IDAT" => idat.extend_from_slice(body),
            b"IEND" => break,
            _ => {}
        }
        pos = end;
    }
    let h = header.ok_or("no IHDR")?;
    if h.ctype == 3 {
        if palette.is_empty() {
            return Err("a palette picture without a palette".into());
        }
        for (i, &a) in trns.iter().enumerate() {
            if let Some(p) = palette.get_mut(i) {
                p[3] = a;
            }
        }
    }
    // The raw size: every pass's rows, each with its filter byte.
    let passes = passes(&h);
    let raw_len: usize = passes
        .iter()
        .map(|&(pw, ph, ..)| {
            if pw == 0 || ph == 0 {
                0
            } else {
                (h.row_bytes(pw) + 1) * ph as usize
            }
        })
        .sum();
    let raw = inflate_zlib(&idat, raw_len)?;
    if raw.len() < raw_len {
        return Err("the image data is short".into());
    }
    let mut pic = Picture::new(h.w, h.h);
    let mut at = 0;
    for &(pw, ph, x0, y0, dx, dy) in &passes {
        if pw == 0 || ph == 0 {
            continue;
        }
        let rb = h.row_bytes(pw);
        let stride = rb + 1;
        let mut rows = raw[at..at + stride * ph as usize].to_vec();
        at += stride * ph as usize;
        unfilter(
            &mut rows,
            rb,
            (h.bpp_bits().div_ceil(8)).max(1),
            ph as usize,
        )?;
        for y in 0..ph as usize {
            let row = &rows[y * stride + 1..(y + 1) * stride];
            let py = y0 + y as u32 * dy;
            for x in 0..pw {
                let px = x0 + x * dx;
                pic.px[(py * h.w + px) as usize] = pixel(&h, row, x as usize, &palette, &trns);
            }
        }
    }
    Ok(pic)
}

/// Each pass's (width, height, x0, y0, dx, dy): one pass for a plain
/// picture, Adam7's seven for an interlaced one.
fn passes(h: &Header) -> Vec<(u32, u32, u32, u32, u32, u32)> {
    if !h.interlace {
        return vec![(h.w, h.h, 0, 0, 1, 1)];
    }
    const A7: [(u32, u32, u32, u32); 7] = [
        (0, 0, 8, 8),
        (4, 0, 8, 8),
        (0, 4, 4, 8),
        (2, 0, 4, 4),
        (0, 2, 2, 4),
        (1, 0, 2, 2),
        (0, 1, 1, 2),
    ];
    A7.iter()
        .map(|&(x0, y0, dx, dy)| {
            let pw = (h.w + dx - 1 - x0) / dx;
            let ph = (h.h + dy - 1 - y0) / dy;
            let pw = if h.w > x0 { pw } else { 0 };
            let ph = if h.h > y0 { ph } else { 0 };
            (pw, ph, x0, y0, dx, dy)
        })
        .collect()
}

fn unfilter(rows: &mut [u8], rb: usize, bpp: usize, n: usize) -> Result<(), String> {
    let stride = rb + 1;
    for y in 0..n {
        let (above, here) = rows.split_at_mut(y * stride);
        let prev: Option<&[u8]> = if y == 0 {
            None
        } else {
            Some(&above[(y - 1) * stride + 1..])
        };
        let f = here[0];
        let cur = &mut here[1..stride];
        let up = |i: usize| prev.map_or(0, |p| p[i]);
        match f {
            0 => {}
            1 => {
                for i in bpp..rb {
                    cur[i] = cur[i].wrapping_add(cur[i - bpp]);
                }
            }
            2 => {
                if let Some(p) = prev {
                    for i in 0..rb {
                        cur[i] = cur[i].wrapping_add(p[i]);
                    }
                }
            }
            3 => {
                for i in 0..rb {
                    let left = if i >= bpp { cur[i - bpp] } else { 0 };
                    cur[i] = cur[i].wrapping_add(((left as u16 + up(i) as u16) / 2) as u8);
                }
            }
            4 => {
                for i in 0..rb {
                    let a = if i >= bpp { cur[i - bpp] as i16 } else { 0 };
                    let b = up(i) as i16;
                    let c = if i >= bpp {
                        prev.map_or(0, |p| p[i - bpp]) as i16
                    } else {
                        0
                    };
                    let p = a + b - c;
                    let (pa, pb, pc) = ((p - a).abs(), (p - b).abs(), (p - c).abs());
                    let pred = if pa <= pb && pa <= pc {
                        a
                    } else if pb <= pc {
                        b
                    } else {
                        c
                    };
                    cur[i] = cur[i].wrapping_add(pred as u8);
                }
            }
            _ => return Err(format!("a bad row filter {f}")),
        }
    }
    Ok(())
}

/// Sample `i` (0-based) of `row` at the header's bit depth, as 8 bits (and
/// its raw value for tRNS checks).
fn sample(h: &Header, row: &[u8], i: usize) -> (u8, u16) {
    match h.depth {
        16 => {
            let v = u16::from_be_bytes([row[i * 2], row[i * 2 + 1]]);
            ((v >> 8) as u8, v)
        }
        8 => (row[i], row[i] as u16),
        d => {
            let per = 8 / d as usize;
            let byte = row[i / per];
            let shift = 8 - d as usize * (i % per + 1);
            let v = (byte >> shift) & ((1u8 << d) - 1);
            let scale = match d {
                1 => 255,
                2 => 85,
                _ => 17,
            };
            (v * scale, v as u16)
        }
    }
}

fn pixel(h: &Header, row: &[u8], x: usize, palette: &[[u8; 4]], trns: &[u8]) -> u32 {
    let c = h.channels();
    let trns16 = |k: usize| -> Option<u16> {
        trns.get(k * 2..k * 2 + 2)
            .map(|b| u16::from_be_bytes([b[0], b[1]]))
    };
    match h.ctype {
        0 => {
            let (g, raw) = sample(h, row, x);
            let a = if trns16(0) == Some(raw) { 0 } else { 255 };
            premultiply(g, g, g, a)
        }
        2 => {
            let (r, rr) = sample(h, row, x * 3);
            let (g, rg) = sample(h, row, x * 3 + 1);
            let (b, rb) = sample(h, row, x * 3 + 2);
            let a = match (trns16(0), trns16(1), trns16(2)) {
                (Some(tr), Some(tg), Some(tb)) if (tr, tg, tb) == (rr, rg, rb) => 0,
                _ => 255,
            };
            premultiply(r, g, b, a)
        }
        3 => {
            let (_, idx) = sample(h, row, x);
            let p = palette.get(idx as usize).copied().unwrap_or([0, 0, 0, 255]);
            premultiply(p[0], p[1], p[2], p[3])
        }
        4 => {
            let (g, _) = sample(h, row, x * c);
            let (a, _) = sample(h, row, x * c + 1);
            premultiply(g, g, g, a)
        }
        _ => {
            let (r, _) = sample(h, row, x * 4);
            let (g, _) = sample(h, row, x * 4 + 1);
            let (b, _) = sample(h, row, x * 4 + 2);
            let (a, _) = sample(h, row, x * 4 + 3);
            premultiply(r, g, b, a)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A PNG of `w` x `h` with the given colour type, depth and raw rows
    /// (filter bytes included), stored without compression.
    fn png(
        w: u32,
        h: u32,
        ctype: u8,
        depth: u8,
        interlace: u8,
        raw: &[u8],
        extra: &[(&[u8; 4], Vec<u8>)],
    ) -> Vec<u8> {
        fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], body: &[u8]) {
            out.extend_from_slice(&(body.len() as u32).to_be_bytes());
            out.extend_from_slice(kind);
            out.extend_from_slice(body);
            out.extend_from_slice(&crc32_update(crc32_update(0, kind), body).to_be_bytes());
        }
        let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&w.to_be_bytes());
        ihdr.extend_from_slice(&h.to_be_bytes());
        ihdr.extend_from_slice(&[depth, ctype, 0, 0, interlace]);
        chunk(&mut out, b"IHDR", &ihdr);
        for (k, b) in extra {
            chunk(&mut out, k, b);
        }
        // zlib with stored blocks.
        let mut z = vec![0x78, 0x01];
        let blocks: Vec<&[u8]> = raw.chunks(65535).collect();
        for (i, b) in blocks.iter().enumerate() {
            z.push(if i + 1 == blocks.len() { 1 } else { 0 });
            z.extend_from_slice(&(b.len() as u16).to_le_bytes());
            z.extend_from_slice(&(!(b.len() as u16)).to_le_bytes());
            z.extend_from_slice(b);
        }
        z.extend_from_slice(&crate::inflate::adler32(raw).to_be_bytes());
        chunk(&mut out, b"IDAT", &z);
        chunk(&mut out, b"IEND", &[]);
        out
    }

    #[test]
    fn rgba_with_every_filter() {
        // 2x5 RGBA, one row per filter type, each decoding to the same two
        // pixels: (10,20,30,255) then (40,50,60,128).
        let want = [10u8, 20, 30, 255, 40, 50, 60, 128];
        let mut raw = Vec::new();
        let mut prev = [0u8; 8];
        for f in 0..5u8 {
            raw.push(f);
            for i in 0..8 {
                let a = if i >= 4 { want[i - 4] } else { 0 } as i16;
                let b = prev[i] as i16;
                let c = if i >= 4 { prev[i - 4] } else { 0 } as i16;
                let pred = match f {
                    0 => 0,
                    1 => a,
                    2 => b,
                    3 => (a + b) / 2,
                    _ => {
                        let p = a + b - c;
                        let (pa, pb, pc) = ((p - a).abs(), (p - b).abs(), (p - c).abs());
                        if pa <= pb && pa <= pc {
                            a
                        } else if pb <= pc {
                            b
                        } else {
                            c
                        }
                    }
                };
                raw.push((want[i] as i16 - pred) as u8);
            }
            prev = want;
        }
        let p = decode(&png(2, 5, 6, 8, 0, &raw, &[])).unwrap();
        assert_eq!((p.w, p.h), (2, 5));
        for y in 0..5 {
            assert_eq!(p.px[y * 2], 0xFF0A_141E);
            assert_eq!(p.px[y * 2 + 1], premultiply(40, 50, 60, 128));
        }
    }

    #[test]
    fn palette_gray_and_transparency() {
        // 4x1 palette at 2 bits: indices 0,1,2,3; index 1 half transparent.
        let plte = vec![255, 0, 0, 0, 255, 0, 0, 0, 255, 9, 9, 9];
        let p = decode(&png(
            4,
            1,
            3,
            2,
            0,
            &[0, 0b0001_1011],
            &[(b"PLTE", plte), (b"tRNS", vec![255, 128])],
        ))
        .unwrap();
        assert_eq!(p.px[0], 0xFFFF_0000);
        assert_eq!(p.px[1], premultiply(0, 255, 0, 128));
        assert_eq!(p.px[2], 0xFF00_00FF);
        assert_eq!(p.px[3], 0xFF09_0909);
        // 16-bit gray with tRNS on the value 0x1234.
        let p = decode(&png(
            2,
            1,
            0,
            16,
            0,
            &[0, 0x12, 0x34, 0xFF, 0xFF],
            &[(b"tRNS", vec![0x12, 0x34])],
        ))
        .unwrap();
        assert_eq!(p.px, [0, 0xFFFF_FFFF]);
        // Gray + alpha.
        let p = decode(&png(1, 1, 4, 8, 0, &[0, 200, 255], &[])).unwrap();
        assert_eq!(p.px, [0xFFC8_C8C8]);
    }

    #[test]
    fn adam7_interlaced() {
        // 3x3 RGB, pixel (x, y) = (x*10, y*10, 7).
        let h = Header {
            w: 3,
            h: 3,
            depth: 8,
            ctype: 2,
            interlace: true,
        };
        let mut raw = Vec::new();
        for (pw, ph, x0, y0, dx, dy) in passes(&h) {
            if pw == 0 {
                continue; // an empty pass has no rows at all
            }
            for y in 0..ph {
                raw.push(0);
                for x in 0..pw {
                    let (px, py) = (x0 + x * dx, y0 + y * dy);
                    raw.extend_from_slice(&[(px * 10) as u8, (py * 10) as u8, 7]);
                }
            }
        }
        let p = decode(&png(3, 3, 2, 8, 1, &raw, &[])).unwrap();
        for y in 0..3u32 {
            for x in 0..3u32 {
                assert_eq!(
                    p.px[(y * 3 + x) as usize],
                    premultiply((x * 10) as u8, (y * 10) as u8, 7, 255)
                );
            }
        }
    }

    #[test]
    fn refuses_broken_files() {
        let good = png(1, 1, 2, 8, 0, &[0, 1, 2, 3], &[]);
        assert!(decode(&good).is_ok());
        let mut bad = good.clone();
        bad[20] ^= 1; // inside IHDR: its CRC fails
        assert!(decode(&bad).is_err());
        assert!(decode(&good[..good.len() - 13]).is_err(), "no IEND");
        assert!(decode(b"GIF89a").is_err());
        assert!(
            decode(&png(1, 1, 2, 8, 0, &[0, 1, 2], &[])).is_err(),
            "short data"
        );
        assert!(
            decode(&png(1, 1, 2, 8, 0, &[9, 1, 2, 3], &[])).is_err(),
            "bad filter"
        );
        assert!(
            decode(&png(100_000, 100_000, 2, 8, 0, &[0], &[])).is_err(),
            "too big"
        );
    }
}
