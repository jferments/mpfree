//! Deezer's trained 4-stem checkpoint to the model file, with no
//! TensorFlow or Python: the folder in `4stems.tar.gz` from Spleeter's
//! v1.4.0 release (`model.index` and `model.data-00000-of-00001`, a
//! TensorFlow tensor bundle).
//!
//! The index is a LevelDB-format table (uncompressed data blocks of
//! prefix-compressed keys, an index block, a 48-byte footer); each
//! tensor's entry a `BundleEntryProto` (its type, shape, offset and size
//! in the data file, and a CRC-32C of its bytes, which is checked). The
//! batch norms are folded to a scale and a shift as Spleeter's graph
//! computes them, in f32: scale = gamma / sqrt(variance + 0.001), shift =
//! beta − mean · scale.

use crate::model::write_shaped;
use crate::net::Shape;
use crate::Error;
use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;

/// What packing made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Report {
    /// Kernel weights, and how many of them (not zero) became zero as
    /// halves, being below the smallest normal half (6.1e-5).
    pub kernels: u64,
    pub flushed: u64,
}

/// The model file, to `out`, from Spleeter's 4stems checkpoint in `dir`.
pub fn pack(dir: &Path, out: &mut impl Write) -> Result<Report, Error> {
    let bundle = Bundle::open(dir)?;
    pack_bundle(&bundle, Shape::SPLEETER, out)
}

pub(crate) fn pack_bundle(b: &Bundle, shape: Shape, out: &mut impl Write) -> Result<Report, Error> {
    let layout = shape.stem_layout();
    let dims = dims(shape);
    let mut kernels = 0;
    let flushed = write_shaped(out, shape, |stem, i| {
        let (name, want) = (&tensor_names(stem)[i], &dims[i]);
        let read = |n: &str| -> Result<Vec<f32>, Error> {
            let (got, v) = b.tensor(n).map_err(Error::Model)?;
            if got.iter().map(|&d| d as usize).eq(want.iter().copied()) {
                Ok(v)
            } else {
                Err(Error::Model(format!("{n}: shape {got:?}, not {want:?}")))
            }
        };
        let v = match &name.1 {
            None => read(&name.0)?,
            Some(scale) => {
                let base = &name.0;
                let g = read(&format!("{base}/gamma"))?;
                let var = read(&format!("{base}/moving_variance"))?;
                let sc: Vec<f32> = g
                    .iter()
                    .zip(&var)
                    .map(|(&g, &v)| g / (v + 1e-3f32).sqrt())
                    .collect();
                if *scale {
                    sc
                } else {
                    let beta = read(&format!("{base}/beta"))?;
                    let mean = read(&format!("{base}/moving_mean"))?;
                    beta.iter()
                        .zip(&mean)
                        .zip(&sc)
                        .map(|((&b, &m), &s)| b - m * s)
                        .collect()
                }
            }
        };
        if matches!(layout[i].1, crate::net::Kind::Kernel(_)) {
            kernels += v.len() as u64;
        }
        Ok(v)
    })?;
    Ok(Report { kernels, flushed })
}

/// Each of a stem's tensors in file order: its name in the checkpoint,
/// and for a batch norm's scale (true) or shift (false) the layer's name.
/// Spleeter makes the stems' layers in the order vocals, drums, bass,
/// other, numbering each kind of layer on from the last stem's.
fn tensor_names(stem: usize) -> Vec<(String, Option<bool>)> {
    let name = |base: &str, i: usize| {
        if i == 0 {
            base.to_string()
        } else {
            format!("{base}_{i}")
        }
    };
    let mut out = Vec::new();
    for j in 0..7 {
        let n = name("conv2d", 7 * stem + j);
        out.push((format!("{n}/kernel"), None));
        out.push((format!("{n}/bias"), None));
    }
    for j in 0..6 {
        let n = name("conv2d_transpose", 6 * stem + j);
        out.push((format!("{n}/kernel"), None));
        out.push((format!("{n}/bias"), None));
    }
    for j in 0..12 {
        let n = name("batch_normalization", 12 * stem + j);
        out.push((n.clone(), Some(true)));
        out.push((n, Some(false)));
    }
    out
}

/// Each of a stem's tensors' shape in TensorFlow, in file order.
fn dims(shape: Shape) -> Vec<Vec<usize>> {
    let mut out = Vec::new();
    let mut cin = 2;
    for &f in &shape.filters {
        out.push(vec![5, 5, cin, f]);
        out.push(vec![f]);
        cin = f;
    }
    out.push(vec![4, 4, 1, 2]);
    out.push(vec![2]);
    let (outs, ins) = (shape.outs(), shape.ins());
    for j in 0..6 {
        out.push(vec![5, 5, outs[j], ins[j]]);
        out.push(vec![outs[j]]);
    }
    for j in 0..12 {
        let n = if j < 6 { shape.filters[j] } else { outs[j - 6] };
        out.push(vec![n]);
        out.push(vec![n]);
    }
    out
}

const MAGIC: u64 = 0xdb47_7524_8b80_fb57;

/// A tensor's entry in the index.
struct Entry {
    dtype: u64,
    shape: Vec<i64>,
    shard: u64,
    offset: u64,
    size: u64,
    crc: Option<u32>,
}

fn varint(b: &[u8], at: &mut usize) -> Result<u64, String> {
    let mut v = 0u64;
    for shift in (0..64).step_by(7) {
        let byte = *b.get(*at).ok_or("index: a number cut short")?;
        *at += 1;
        v |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Ok(v);
        }
    }
    Err("index: a number too long".into())
}

/// The block a handle (two varints at `at` in `handle`) points to in
/// `file`, after checking its trailer says uncompressed.
fn block<'a>(file: &'a [u8], handle: &[u8], at: &mut usize) -> Result<&'a [u8], String> {
    let off = varint(handle, at)?;
    let size = varint(handle, at)?;
    let end = off
        .checked_add(size)
        .and_then(|e| usize::try_from(e).ok())
        .filter(|&e| e.checked_add(5).is_some_and(|t| t <= file.len()))
        .ok_or("model.index: a block past the end")?;
    match file[end] {
        0 => Ok(&file[off as usize..end]),
        t => Err(format!(
            "model.index: a compressed block (type {t}); only uncompressed tables are read"
        )),
    }
}

/// A table entry: its key and value.
type Kv<'a> = (Vec<u8>, &'a [u8]);

/// A protobuf field: its number, wire type and the value's bytes.
type Field<'a> = (u64, u64, &'a [u8]);

/// A block's (key, value) entries.
fn entries(b: &[u8]) -> Result<Vec<Kv<'_>>, String> {
    let n = b.len().checked_sub(4).ok_or("index: a block too short")?;
    let restarts = u32::from_le_bytes([b[n], b[n + 1], b[n + 2], b[n + 3]]) as usize;
    let end = restarts
        .checked_mul(4)
        .and_then(|r| n.checked_sub(r))
        .ok_or("index: a bad restart count")?;
    let mut out = Vec::new();
    let mut key: Vec<u8> = Vec::new();
    let mut at = 0;
    while at < end {
        let shared = varint(b, &mut at)? as usize;
        let fresh = varint(b, &mut at)? as usize;
        let vlen = varint(b, &mut at)? as usize;
        if shared > key.len() || at + fresh + vlen > end {
            return Err("index: a bad entry".into());
        }
        key.truncate(shared);
        key.extend_from_slice(&b[at..at + fresh]);
        at += fresh;
        out.push((key.clone(), &b[at..at + vlen]));
        at += vlen;
    }
    Ok(out)
}

/// Protobuf fields: (number, wire type, the value's bytes).
fn fields(b: &[u8]) -> Result<Vec<Field<'_>>, String> {
    let mut out = Vec::new();
    let mut at = 0;
    while at < b.len() {
        let tag = varint(b, &mut at)?;
        let (num, wt) = (tag >> 3, tag & 7);
        let start = at;
        let body = match wt {
            0 => {
                varint(b, &mut at)?;
                &b[start..at]
            }
            1 | 5 => {
                at += if wt == 1 { 8 } else { 4 };
                b.get(start..at).ok_or("entry: cut short")?
            }
            2 => {
                let n = varint(b, &mut at)? as usize;
                let s = at;
                at = at.checked_add(n).ok_or("entry: cut short")?;
                b.get(s..at).ok_or("entry: cut short")?
            }
            _ => return Err(format!("entry: wire type {wt}")),
        };
        out.push((num, wt, body));
    }
    Ok(out)
}

fn number(body: &[u8]) -> Result<u64, String> {
    varint(body, &mut 0)
}

fn entry(v: &[u8]) -> Result<Entry, String> {
    let mut e = Entry {
        dtype: 0,
        shape: Vec::new(),
        shard: 0,
        offset: 0,
        size: 0,
        crc: None,
    };
    for (num, wt, body) in fields(v)? {
        match (num, wt) {
            (1, 0) => e.dtype = number(body)?,
            (2, 2) => {
                for (n2, w2, dim) in fields(body)? {
                    if n2 == 2 && w2 == 2 {
                        let mut size = 0i64;
                        for (n3, w3, x) in fields(dim)? {
                            if n3 == 1 && w3 == 0 {
                                size = number(x)? as i64;
                            }
                        }
                        e.shape.push(size);
                    }
                }
            }
            (3, 0) => e.shard = number(body)?,
            (4, 0) => e.offset = number(body)?,
            (5, 0) => e.size = number(body)?,
            (6, 5) => e.crc = Some(u32::from_le_bytes([body[0], body[1], body[2], body[3]])),
            (7, _) => return Err("a sliced tensor, which isn't read here".into()),
            _ => {}
        }
    }
    Ok(e)
}

/// The tensors an index names, by name.
fn read_index(file: &[u8]) -> Result<BTreeMap<String, Entry>, String> {
    let n = file.len();
    if n < 48 || file[n - 8..] != MAGIC.to_le_bytes() {
        return Err("model.index is not a TensorFlow table".into());
    }
    // The footer: the metaindex's handle (empty in a bundle), then the
    // index's.
    let mut at = n - 48;
    block(file, file, &mut at)?;
    let index = block(file, file, &mut at)?;
    let mut out = BTreeMap::new();
    for (_, handle) in entries(index)? {
        for (key, value) in entries(block(file, handle, &mut 0)?)? {
            if key.is_empty() {
                continue; // the bundle's header
            }
            let name = String::from_utf8(key).map_err(|_| "model.index: a name isn't UTF-8")?;
            let e = entry(value).map_err(|m| format!("model.index: {name}: {m}"))?;
            out.insert(name, e);
        }
    }
    Ok(out)
}

/// CRC-32C (Castagnoli), as TensorFlow checks a tensor's bytes.
pub(crate) fn crc32c(data: &[u8]) -> u32 {
    let mut table = [0u32; 256];
    for (i, t) in table.iter_mut().enumerate() {
        let mut c = i as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 {
                0x82f6_3b78 ^ (c >> 1)
            } else {
                c >> 1
            };
        }
        *t = c;
    }
    let mut crc = !0u32;
    for &b in data {
        crc = table[((crc ^ u32::from(b)) & 0xff) as usize] ^ (crc >> 8);
    }
    !crc
}

/// LevelDB's mask on a stored CRC.
pub(crate) fn mask(crc: u32) -> u32 {
    crc.rotate_right(15).wrapping_add(0xa282_ead8)
}

/// A checkpoint's tensors, each read with its bytes checked.
pub(crate) struct Bundle {
    index: BTreeMap<String, Entry>,
    data: Vec<u8>,
}

impl Bundle {
    pub fn open(dir: &Path) -> Result<Bundle, Error> {
        let read = |name: &str| {
            let p = dir.join(name);
            std::fs::read(&p).map_err(|e| Error::Model(format!("{}: {e}", p.display())))
        };
        let index = read("model.index")?;
        let data = read("model.data-00000-of-00001")?;
        Bundle::from_bytes(&index, data)
    }

    pub fn from_bytes(index: &[u8], data: Vec<u8>) -> Result<Bundle, Error> {
        let index = read_index(index).map_err(Error::Model)?;
        if index.values().any(|e| e.shard != 0) {
            return Err(Error::Model(
                "a checkpoint in more than one data file".into(),
            ));
        }
        Ok(Bundle { index, data })
    }

    /// A float32 tensor's shape and values.
    pub fn tensor(&self, name: &str) -> Result<(Vec<i64>, Vec<f32>), String> {
        let e = self
            .index
            .get(name)
            .ok_or_else(|| format!("the checkpoint has no tensor {name}"))?;
        if e.dtype != 1 {
            return Err(format!("{name}: type {} is not float32", e.dtype));
        }
        let n = e
            .shape
            .iter()
            .try_fold(1u64, |a, &d| {
                u64::try_from(d).ok().and_then(|d| a.checked_mul(d))
            })
            .ok_or_else(|| format!("{name}: a bad shape {:?}", e.shape))?;
        if Some(e.size) != n.checked_mul(4) {
            return Err(format!("{name}: {} bytes for {n} floats", e.size));
        }
        let bytes = e
            .offset
            .checked_add(e.size)
            .and_then(|end| self.data.get(e.offset as usize..end as usize))
            .ok_or_else(|| format!("{name}: past the data file's end"))?;
        if let Some(want) = e.crc {
            let got = crc32c(bytes);
            if want != mask(got) && want != got {
                return Err(format!("{name}: its bytes don't match their CRC"));
            }
        }
        let v = bytes
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect();
        Ok((e.shape.clone(), v))
    }
}

#[cfg(test)]
mod tests;
