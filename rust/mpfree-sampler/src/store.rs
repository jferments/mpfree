//! The sound store (sampler plan, phase 2 step 8): every sound the program
//! holds, once.
//!
//! A [`Source`](crate::model::Source) is a kit's handle on a sound: its name
//! in that kit and the [`Sound`] itself, shared. A sound is its samples, its
//! 128-bit id ([`content_id`]) and the analysis kept with it (its octave
//! copies and its hits). Making a source asks the store first: when a sound
//! with that id is held already (in this kit, another kit, the factory kits
//! or a take kept for undo), the new source shares it, so two kits with one
//! sound hold one copy, and an octave copy or hit list made for one is there
//! for the other. The store only keeps a weak reference: a sound goes when
//! the last source using it goes. One program has one store, and it holds
//! one project at a time.
//!
//! The analysis the screens use (the waveform's peaks, the loudness summary,
//! the hits and the pitch) is kept by the same id, beside the sound, in the
//! app's views (`mpfree-app` `views.rs`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, Weak};

use crate::model::{Pcm, SourceId};

/// One sound, held once ([module docs](self)).
#[derive(Debug)]
pub struct Sound {
    pub(crate) id: SourceId,
    pub(crate) rate: u32,
    pub(crate) channels: u8,
    /// Interleaved when stereo; 16 bits when it can be ([`Pcm::new`]).
    pub(crate) data: Pcm,
    /// Its octave copies, for reading it more than an octave up
    /// ([`crate::bandlimit`]). Made when a pad that can play it that high
    /// asks for them.
    pub(crate) octaves: Arc<crate::bandlimit::OctaveCell>,
    /// Its hits, for a granular Stretch ([`crate::hits`]). Found when a pad
    /// that stretches it asks for them.
    pub(crate) hits: Arc<crate::hits::HitCell>,
    /// The file a project keeps it in, when that isn't named by its id: a
    /// sound an older build saved under its 64-bit id keeps that file
    /// ([`crate::model::Source::keep_old_file`]).
    pub(crate) file: OnceLock<String>,
}

impl Sound {
    /// A sound of its own, not in the store: for a copy that must not be
    /// shared ([`crate::model::Source::held_as_floats`]).
    pub(crate) fn alone(id: SourceId, rate: u32, channels: u8, data: Pcm) -> Sound {
        Sound {
            id,
            rate,
            channels,
            data,
            octaves: Arc::default(),
            hits: Arc::default(),
            file: OnceLock::new(),
        }
    }
}

struct Table {
    sounds: HashMap<SourceId, Weak<Sound>>,
    /// How many were held when the gone ones were last cleared out.
    kept: usize,
}

fn table() -> &'static Mutex<Table> {
    static TABLE: OnceLock<Mutex<Table>> = OnceLock::new();
    TABLE.get_or_init(|| {
        Mutex::new(Table {
            sounds: HashMap::new(),
            kept: 0,
        })
    })
}

fn lock() -> std::sync::MutexGuard<'static, Table> {
    // A panic elsewhere while it was held leaves the table as it was: a
    // map of weak references, never half changed.
    table().lock().unwrap_or_else(|e| e.into_inner())
}

/// The sound with `id` if the store holds it, of this rate, channel count
/// and length.
fn find(t: &Table, id: SourceId, rate: u32, channels: u8, samples: usize) -> Option<Arc<Sound>> {
    t.sounds
        .get(&id)
        .and_then(Weak::upgrade)
        .filter(|s| s.rate == rate && s.channels == channels && s.data.len() == samples)
}

/// The sound with `id`: the one the store holds, or a new one of
/// `make()`'s samples (`samples` long), kept from now on. `make` runs only
/// when the store doesn't hold it, and not under the store's lock, so a
/// long sound being made never holds up another.
pub(crate) fn held(
    id: SourceId,
    rate: u32,
    channels: u8,
    samples: usize,
    make: impl FnOnce() -> Pcm,
) -> Arc<Sound> {
    if let Some(s) = find(&lock(), id, rate, channels, samples) {
        return s;
    }
    let made = Arc::new(Sound::alone(id, rate, channels, make()));
    let mut t = lock();
    // Made at the same moment on another thread: that one is kept.
    if let Some(s) = find(&t, id, rate, channels, samples) {
        return s;
    }
    if let Some(old) = t.sounds.get(&id).and_then(Weak::upgrade) {
        // Two sounds of different shapes with one id, which 128 bits make
        // as good as impossible: the new one isn't shared or kept.
        drop(old);
        return made;
    }
    t.sounds.insert(id, Arc::downgrade(&made));
    if t.sounds.len() > (2 * t.kept).max(64) {
        t.sounds.retain(|_, w| w.strong_count() > 0);
        t.kept = t.sounds.len();
    }
    made
}

/// How many sounds the store holds now, and their samples' bytes: each
/// sound counted once, however many kits and pads use it.
pub fn held_now() -> (usize, u64) {
    let t = lock();
    t.sounds
        .values()
        .filter_map(Weak::upgrade)
        .fold((0, 0), |(n, b), s| (n + 1, b + s.data.bytes() as u64))
}

/// A sound's 128-bit id from its rate, channel count and samples: the same
/// audio always has the same id, and two different sounds (a sample
/// changed, two swapped, one more or less, another rate or channel count)
/// as good as never share one.
///
/// It reads two samples at a time, in stripes of 16 samples through eight
/// running sums, each adding its pair and a 32-by-32-bit product of the
/// pair mixed with a key (XXH3's long-input loop, on samples), stirred
/// after every 256 samples and mixed down to 128 bits at the end. About
/// 5 GB/s on the workstation, six times the 64-bit FNV-1a it replaces,
/// which read one byte at a time; on the MPC's 32-bit cores the product is
/// one multiply, where FNV-1a's 64-bit one was several a byte.
pub fn content_id(rate: u32, channels: u8, data: &[f32]) -> u128 {
    let seed = u64::from(rate) | (u64::from(channels) << 32);
    let mut acc = START.map(|a| a ^ seed);
    let mut blocks = data.chunks_exact(STRIPE * BLOCK);
    for b in &mut blocks {
        for (s, x) in b.chunks_exact(STRIPE).enumerate() {
            stripe(&mut acc, x, s);
        }
        for (a, k) in acc.iter_mut().zip(STIR) {
            *a = (*a ^ (*a >> 47) ^ k).wrapping_mul(0x9E37_79B1);
        }
    }
    let mut stripes = blocks.remainder().chunks_exact(STRIPE);
    let mut s = 0;
    for x in &mut stripes {
        stripe(&mut acc, x, s);
        s += 1;
    }
    let tail = stripes.remainder();
    if !tail.is_empty() {
        // The rest, with silence after it: the length below tells it from
        // a sound that really ends in silence.
        let mut last = [0f32; STRIPE];
        last[..tail.len()].copy_from_slice(tail);
        stripe(&mut acc, &last, s);
    }
    let n = data.len() as u64;
    let lo = merge(&acc, &LOW, n.wrapping_mul(0x9E37_79B1_85EB_CA87) ^ seed);
    let hi = merge(&acc, &HIGH, !n.wrapping_mul(0xC2B2_AE3D_27D4_EB4F) ^ seed);
    (u128::from(hi) << 64) | u128::from(lo)
}

/// Samples a stripe.
const STRIPE: usize = 16;
/// Stripes between stirs.
const BLOCK: usize = 16;
/// The running sums' first values (XXH3's).
const START: [u64; 8] = [
    0xC2B2_AE3D,
    0x9E37_79B1_85EB_CA87,
    0xC2B2_AE3D_27D4_EB4F,
    0x1656_67B1_9E37_79F9,
    0x85EB_CA77_C2B2_AE63,
    0x85EB_CA77,
    0x27D4_EB2F_1656_67C5,
    0x9E37_79B1,
];
/// The keys, fixed random numbers: stripe `s` of a block mixes pair `i`
/// with key `s + i`, so no two stripes of a block use the same keys and
/// swapping two changes the id.
const KEYS: [u64; STRIPE / 2 + BLOCK - 1] = [
    0x91d9_3080_f9b4_39fc,
    0x3cea_af30_8142_c8ea,
    0x6276_55da_a05f_1119,
    0x9dfb_369b_ab13_dfae,
    0x0ff9_bb51_0944_fe0c,
    0x6301_bbfc_a0c5_154e,
    0x047a_8387_cdde_8d51,
    0x6fce_d11d_2bd8_a1e6,
    0xb7c1_dce2_7f76_95f5,
    0x3a12_4c64_7e02_ff05,
    0x5cbd_0ba3_a1a6_bcde,
    0x6608_0f17_4c96_d8e2,
    0x42f2_dc05_873f_9bd0,
    0xb120_a366_f0bb_e9cf,
    0xce4c_da11_b43c_589b,
    0x78b7_d0ac_8477_3ce0,
    0x4866_8d77_e827_062c,
    0x2330_fcf1_562a_e4b8,
    0xe36b_f9b3_ddcb_369d,
    0xc1ac_c990_6424_550a,
    0xf065_a1c2_84cc_4549,
    0x9ad9_b9a2_5e78_454d,
    0xf6a7_30b7_b12e_9eed,
];
/// Mixed into the sums at each stir.
const STIR: [u64; 8] = [
    0x9935_6597_6fba_f662,
    0xc5c2_c2af_3977_a654,
    0x8df0_8078_a22e_a180,
    0x952c_466e_473a_1797,
    0xf319_4a3b_381e_53e7,
    0x2d35_8dcc_aa6c_78a5,
    0x8bb8_4b93_962e_acc9,
    0x4b33_a62e_d433_d4a3,
];
/// The keys the sums are mixed down with, for each half of the id.
const LOW: [u64; 8] = [
    0x4d5a_2da5_1de1_aa47,
    0xa0b3_6f47_c4a8_d0b5,
    0x1d6e_6d1b_b3ae_c2e9,
    0x7f2b_d7d1_9a35_1e6b,
    0xe5b8_6c4f_8a7d_5c33,
    0x3c79_ac49_2ba7_b653,
    0xb4c2_9e0d_5f61_8d17,
    0x6a09_e667_f3bc_c909,
];
const HIGH: [u64; 8] = [
    0xbb67_ae85_84ca_a73b,
    0x3c6e_f372_fe94_f82b,
    0xa54f_f53a_5f1d_36f1,
    0x510e_527f_ade6_82d1,
    0x9b05_688c_2b3e_6c1f,
    0x1f83_d9ab_fb41_bd6b,
    0x5be0_cd19_137e_2179,
    0xcbbb_9d5d_c105_9ed8,
];

/// Adds a stripe (16 samples, stripe `s` of its block) to the sums.
#[inline(always)]
fn stripe(acc: &mut [u64; 8], x: &[f32], s: usize) {
    for i in 0..8 {
        let w = u64::from(x[2 * i].to_bits()) | (u64::from(x[2 * i + 1].to_bits()) << 32);
        let k = w ^ KEYS[s + i];
        acc[i ^ 1] = acc[i ^ 1].wrapping_add(w);
        acc[i] = acc[i].wrapping_add((k & 0xffff_ffff) * (k >> 32));
    }
}

/// The 128-bit product's halves, mixed.
fn fold(a: u64, b: u64) -> u64 {
    let r = u128::from(a) * u128::from(b);
    (r as u64) ^ ((r >> 64) as u64)
}

/// The sums mixed down to 64 bits.
fn merge(acc: &[u64; 8], keys: &[u64; 8], start: u64) -> u64 {
    let mut h = start;
    for i in 0..4 {
        h = h.wrapping_add(fold(
            acc[2 * i] ^ keys[2 * i],
            acc[2 * i + 1] ^ keys[2 * i + 1],
        ));
    }
    h ^= h >> 37;
    h = h.wrapping_mul(0x1656_6791_9E37_79F9);
    h ^ (h >> 32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn noise(n: usize, seed: u32) -> Vec<f32> {
        let mut x = seed.wrapping_mul(2_654_435_761) | 1;
        (0..n)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                (x as f32 / u32::MAX as f32) * 2.0 - 1.0
            })
            .collect()
    }

    #[test]
    fn the_same_audio_has_the_same_id_and_any_change_makes_another() {
        let a = noise(10_000, 1);
        let id = content_id(48_000, 2, &a);
        assert_eq!(content_id(48_000, 2, &a.clone()), id);
        let mut seen = HashSet::from([id]);
        let mut new = |v: u128, what: &str| assert!(seen.insert(v), "{what}");
        new(content_id(44_100, 2, &a), "another rate");
        new(content_id(48_000, 1, &a), "another channel count");
        new(content_id(48_000, 2, &a[..9_999]), "one sample less");
        new(content_id(48_000, 2, &a[1..]), "the first sample gone");
        let mut longer = a.clone();
        longer.push(0.0);
        new(content_id(48_000, 2, &longer), "a silent sample more");
        longer.push(0.0);
        new(content_id(48_000, 2, &longer), "two silent samples more");
        // Every place, in and across stripes and blocks, and the tail.
        for i in [0, 1, 15, 16, 255, 256, 257, 4_095, 9_984, 9_999] {
            let mut b = a.clone();
            b[i] = f32::from_bits(b[i].to_bits() ^ 1);
            new(
                content_id(48_000, 2, &b),
                &format!("sample {i}'s lowest bit"),
            );
            let mut b = a.clone();
            b[i] = -b[i];
            new(
                content_id(48_000, 2, &b),
                &format!("sample {i} turned over"),
            );
        }
        // Two samples, two pairs and two stripes of a block swapped.
        for (i, j) in [
            (0, 1),
            (0, 2),
            (2, 3),
            (16, 32),
            (16, 80),
            (32, 96),
            (0, 256),
        ] {
            let mut b = a.clone();
            b.swap(i, j);
            new(
                content_id(48_000, 2, &b),
                &format!("samples {i} and {j} swapped"),
            );
        }
        let mut b = a.clone();
        let (x, y) = b.split_at_mut(80);
        x[16..32].swap_with_slice(&mut y[..16]);
        new(content_id(48_000, 2, &b), "stripes 1 and 5 swapped");
        // Silence, and the sign of zero (another sample's bits).
        let quiet = vec![0.0f32; 4_096];
        new(content_id(48_000, 2, &quiet), "silence");
        new(content_id(48_000, 2, &quiet[..4_094]), "shorter silence");
        let mut neg = quiet.clone();
        neg[100] = -0.0;
        new(content_id(48_000, 2, &neg), "a zero below zero");
        new(content_id(48_000, 2, &[]), "nothing");
    }

    #[test]
    fn a_hundred_thousand_short_sounds_have_a_hundred_thousand_ids_in_each_half() {
        let (mut lo, mut hi) = (HashSet::new(), HashSet::new());
        for k in 0..100_000u32 {
            // Short sounds that differ only a little: a 16-bit count in
            // one sample, the rest the same.
            let mut x = vec![0.25f32; 37];
            x[(k % 37) as usize] = (k as f32) / 65_536.0;
            let id = content_id(48_000, 1, &x);
            assert!(lo.insert(id as u64), "low half of {k}");
            assert!(hi.insert((id >> 64) as u64), "high half of {k}");
        }
    }

    #[test]
    fn a_sound_made_twice_is_held_once_until_the_last_goes() {
        let a = noise(4_096, 77);
        let id = SourceId(content_id(48_000, 2, &a));
        let mut made = 0;
        let one = held(id, 48_000, 2, a.len(), || {
            made += 1;
            Pcm::new(a.clone())
        });
        let two = held(id, 48_000, 2, a.len(), || {
            made += 1;
            Pcm::new(a.clone())
        });
        assert_eq!(made, 1, "made once");
        assert!(Arc::ptr_eq(&one, &two));
        // Another shape with the same id isn't taken for it.
        let other = held(id, 44_100, 2, a.len(), || Pcm::new(a.clone()));
        assert!(!Arc::ptr_eq(&one, &other));
        drop((one, two));
        let three = held(id, 48_000, 2, a.len(), || {
            made += 1;
            Pcm::new(a.clone())
        });
        assert_eq!(made, 2, "gone with the last one, so made again");
        drop(three);
    }
}
