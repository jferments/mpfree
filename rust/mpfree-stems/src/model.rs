//! The model: four U-Nets (vocals, drums, bass, other), from its file or
//! with seeded weights of the same shapes.
//!
//! The file (`MPFSTEM2`, written by [`crate::pack`]): the eight bytes
//! `MPFSTEM2`, the stems as a little-endian u32 (4), then each stem's 50
//! tensors in Spleeter's order (seven convolutions, six transposed ones,
//! twelve batch norms folded to a scale and a shift), each as its kind
//! (u32: 1 for f32, 2 for half), its count of values (u32) and the values,
//! little-endian. The kernels are halves, the rest f32. 79 MB for
//! Spleeter's 4stems.

use crate::half::is_weight;
use crate::net::{Kind, Shape, Unet, Values};
use crate::Error;
use std::io::{Read, Write};
use std::path::Path;

const MAGIC: &[u8; 8] = b"MPFSTEM2";
const STEMS: u32 = 4;
const F32: u32 = 1;
const F16: u32 = 2;

/// Spleeter's four networks, ready to separate with.
pub struct Model {
    pub(crate) stems: Vec<Unet>,
    pub(crate) shape: Shape,
}

impl Model {
    /// The model file at `path` (see the module's notes for its layout).
    pub fn open(path: &Path) -> Result<Model, Error> {
        let file = std::fs::File::open(path)
            .map_err(|e| Error::Model(format!("{}: {e}", path.display())))?;
        Model::read(std::io::BufReader::new(file))
    }

    /// A model file from `r`, one tensor at a time, so the file is never
    /// held whole beside the packed weights.
    pub fn read(r: impl Read) -> Result<Model, Error> {
        Model::read_shaped(r, Shape::SPLEETER)
    }

    pub(crate) fn read_shaped(mut r: impl Read, shape: Shape) -> Result<Model, Error> {
        let bad = |m: String| Error::Model(m);
        let mut head = [0u8; 12];
        r.read_exact(&mut head)
            .map_err(|_| bad("too short to be a stem model".into()))?;
        if &head[..8] != MAGIC {
            return Err(bad("not a stem model (MPFSTEM2)".into()));
        }
        let stems = u32::from_le_bytes([head[8], head[9], head[10], head[11]]);
        if stems != STEMS {
            return Err(bad(format!("a model of {stems} stems, not {STEMS}")));
        }
        let mut fail: Option<Error> = None;
        let mut at = 0;
        let model = Model::build(shape, |n, kind| {
            at += 1;
            if fail.is_some() {
                return Values::F32(vec![0.0; n]);
            }
            match read_tensor(&mut r, n, kind) {
                Ok(v) => v,
                Err(m) => {
                    fail = Some(bad(format!("tensor {at}: {m}")));
                    Values::F32(vec![0.0; n])
                }
            }
        });
        if let Some(e) = fail {
            return Err(e);
        }
        let mut more = [0u8; 1];
        match r.read(&mut more) {
            Ok(0) => Ok(model),
            Ok(_) => Err(bad("more bytes after the last tensor".into())),
            Err(e) => Err(bad(e.to_string())),
        }
    }

    /// Spleeter's shapes with seeded pseudo-random weights, scaled as a
    /// network is initialised: it costs what the real model costs, for
    /// the benchmark, which needs no model file.
    pub fn synthetic(seed: u32) -> Model {
        Model::synthetic_shaped(Shape::SPLEETER, seed)
    }

    /// A small network of Spleeter's code (narrow layers, 64-frame
    /// segments of 1.5 s) with seeded weights: for the tests of what runs
    /// a separation (the sampler's stems job), which want it quick. Its
    /// stems mean nothing.
    pub fn small(seed: u32) -> Model {
        let shape = Shape {
            filters: [3, 4, 5, 6, 7, 8],
            rows: 64,
        };
        Model::synthetic_shaped(shape, seed)
    }

    pub(crate) fn synthetic_shaped(shape: Shape, seed: u32) -> Model {
        let mut tensors = synthetic_tensors(shape, seed);
        Model::build(shape, |_, _| {
            Values::F32(tensors.next().expect("a tensor per entry"))
        })
    }

    /// The four stems' networks from their tensors in file order:
    /// `next(values, kind)` gives each.
    pub(crate) fn build(shape: Shape, mut next: impl FnMut(usize, Kind) -> Values) -> Model {
        let layout = shape.stem_layout();
        let stems = (0..STEMS)
            .map(|_| {
                let mut i = 0;
                Unet::build(&shape, || {
                    let (n, kind) = layout[i];
                    i += 1;
                    let v = next(n, kind);
                    assert_eq!(v.len(), n, "tensor {i} of a stem");
                    v
                })
            })
            .collect();
        Model { stems, shape }
    }

    /// Bytes the weights take in memory.
    pub fn bytes(&self) -> u64 {
        self.stems.iter().map(|u| u.bytes() as u64).sum()
    }

    /// Floats the widest layer's weights take unpacked.
    pub(crate) fn widest(&self) -> usize {
        self.stems.iter().map(Unet::widest).max().unwrap_or(0)
    }
}

/// Seeded tensors in file order for every stem, as [`Model::synthetic`]
/// makes them.
pub(crate) fn synthetic_tensors(shape: Shape, seed: u32) -> impl Iterator<Item = Vec<f32>> {
    let layout = shape.stem_layout();
    let mut seed = seed;
    let mut rnd = move || {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (seed >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0
    };
    (0..STEMS as usize * layout.len()).map(move |i| {
        let (n, kind) = layout[i % layout.len()];
        match kind {
            Kind::Kernel(fan) => {
                let a = (3.0 / fan as f32).sqrt();
                (0..n).map(|_| rnd() * a).collect()
            }
            Kind::Bias | Kind::Shift => (0..n).map(|_| rnd() * 0.05).collect(),
            Kind::Scale => (0..n).map(|_| 1.0 + rnd() * 0.1).collect(),
        }
    })
}

/// One tensor: a kernel's halves as they are (checked, never widened), the
/// rest f32.
fn read_tensor(r: &mut impl Read, want: usize, kind: Kind) -> Result<Values, String> {
    let mut word = || -> Result<u32, String> {
        let mut b = [0u8; 4];
        r.read_exact(&mut b).map_err(|_| "cut short".to_string())?;
        Ok(u32::from_le_bytes(b))
    };
    let tag = word()?;
    let n = word()? as usize;
    if n != want {
        return Err(format!("{n} values, not {want}"));
    }
    let size = match tag {
        F32 => 4,
        F16 => 2,
        t => return Err(format!("values of unknown kind {t}")),
    };
    let mut raw = vec![0u8; size * n];
    r.read_exact(&mut raw)
        .map_err(|_| "cut short".to_string())?;
    if matches!(kind, Kind::Kernel(_)) != (tag == F16) {
        return Err(if tag == F16 {
            "halves that aren't a kernel's".into()
        } else {
            "a kernel not in halves".into()
        });
    }
    if size == 4 {
        let v: Vec<f32> = raw
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect();
        if v.iter().any(|x| !x.is_finite()) {
            return Err("a value that is not finite".into());
        }
        Ok(Values::F32(v))
    } else {
        // A normal half or a zero is finite, and widens exactly.
        let halves: Vec<u16> = raw
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        if let Some(h) = halves.iter().find(|&&h| !is_weight(h)) {
            return Err(format!(
                "a half ({h:#06x}) that is not a normal number or zero"
            ));
        }
        Ok(Values::Half(halves))
    }
}

/// Writes a model file: every stem's tensors in file order from
/// `next(stem, index)`, the kernels as halves, the rest f32. Returns how
/// many kernel weights were not zero but became zero as halves (below the
/// smallest normal half, 6.1e-5).
pub(crate) fn write_shaped(
    w: &mut impl Write,
    shape: Shape,
    mut next: impl FnMut(usize, usize) -> Result<Vec<f32>, Error>,
) -> Result<u64, Error> {
    let io = |e: std::io::Error| Error::Model(e.to_string());
    w.write_all(MAGIC).map_err(io)?;
    w.write_all(&STEMS.to_le_bytes()).map_err(io)?;
    let layout = shape.stem_layout();
    let mut flushed = 0;
    for stem in 0..STEMS as usize {
        for (i, &(n, kind)) in layout.iter().enumerate() {
            let t = next(stem, i)?;
            if t.len() != n {
                return Err(Error::Model(format!(
                    "stem {stem} tensor {i}: {} values, not {n}",
                    t.len()
                )));
            }
            if let Some(x) = t.iter().find(|x| !x.is_finite()) {
                return Err(Error::Model(format!("stem {stem} tensor {i}: a value {x}")));
            }
            let half = matches!(kind, Kind::Kernel(_));
            let mut bytes = Vec::with_capacity(8 + n * 4);
            bytes.extend_from_slice(&(if half { F16 } else { F32 }).to_le_bytes());
            bytes.extend_from_slice(&(n as u32).to_le_bytes());
            for &x in &t {
                if half {
                    let h = crate::half::to_f16(x);
                    flushed += u64::from(h & 0x7fff == 0 && x != 0.0);
                    bytes.extend_from_slice(&h.to_le_bytes());
                } else {
                    bytes.extend_from_slice(&x.to_le_bytes());
                }
            }
            w.write_all(&bytes).map_err(io)?;
        }
    }
    Ok(flushed)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SMALL: Shape = Shape {
        filters: [2, 3, 4, 5, 6, 7],
        rows: 64,
    };

    fn file(seed: u32) -> Vec<u8> {
        let mut all: Vec<Vec<f32>> = synthetic_tensors(SMALL, seed).collect();
        let per = SMALL.stem_layout().len();
        let mut out = Vec::new();
        write_shaped(&mut out, SMALL, |s, i| {
            Ok(std::mem::take(&mut all[s * per + i]))
        })
        .unwrap();
        out
    }

    fn same(a: &Model, b: &Model) -> bool {
        a.stems.iter().zip(&b.stems).all(|(x, y)| x.same_weights(y))
    }

    #[test]
    fn a_written_model_reads_back_as_the_same_networks() {
        let bytes = file(3);
        let read = Model::read_shaped(&bytes[..], SMALL).unwrap();
        assert!(same(&read, &Model::synthetic_shaped(SMALL, 3)));
        assert!(!same(&read, &Model::synthetic_shaped(SMALL, 4)));
    }

    #[test]
    fn a_damaged_model_file_is_refused_with_a_reason() {
        let good = file(5);
        let refuse = |b: &[u8], why: &str| match Model::read_shaped(b, SMALL) {
            Err(Error::Model(m)) => assert!(m.contains(why), "{m:?} lacks {why:?}"),
            Ok(_) => panic!("read a model that should be refused ({why})"),
            Err(e) => panic!("{e:?}"),
        };
        refuse(&good[..5], "too short");
        refuse(b"MPFSTEM1\x04\0\0\0", "not a stem model");
        let mut five = good.clone();
        five[8] = 5;
        refuse(&five, "5 stems");
        refuse(&good[..good.len() - 1], "cut short");
        let mut longer = good.clone();
        longer.push(0);
        refuse(&longer, "more bytes");
        // The first tensor is a kernel in halves: make its first a NaN.
        let mut nan = good.clone();
        nan[20..22].copy_from_slice(&0x7e00u16.to_le_bytes());
        refuse(&nan, "not a normal number");
        let mut sub = good.clone();
        sub[20..22].copy_from_slice(&0x0001u16.to_le_bytes());
        refuse(&sub, "not a normal number");
        let mut count = good.clone();
        count[16] ^= 1;
        refuse(&count, "values, not");
        let mut kind = good.clone();
        kind[12] = 9;
        refuse(&kind, "unknown kind");
        // The second tensor (the first bias, after the first kernel's 100
        // halves) said to be halves.
        let mut bias = good;
        assert_eq!(bias[220], 1);
        bias[220] = 2;
        refuse(&bias, "aren't a kernel's");
    }

    #[test]
    fn spleeters_model_is_79_mb_and_its_widest_layer_13_mb() {
        let m = Model::synthetic(1);
        let mb = m.bytes() as f64 / 1e6;
        assert!((78.0..80.0).contains(&mb), "{mb} MB");
        assert_eq!(m.widest(), 25 * 256 * 512);
    }
}
