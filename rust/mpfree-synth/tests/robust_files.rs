//! Robustness: the synth's file readers answer every damaged file with an
//! error or something safe to play, never a panic.
//!
//! - DX7 sysex banks and single voices (`dx7::parse`): damaged anywhere,
//!   with the checksum fixed up afterwards so the damage reaches the voice
//!   data. Every voice read has known parameters with finite values, and
//!   loaded into the FM engine it plays finite audio with every parameter
//!   inside its range. (A damaged voice can carry a byte past the DX7's
//!   range, such as a rate of 128; the engine clamps it on load.)
//! - Wavetable WAVs (`wavetable::import::table_from_wav`): every sample of a
//!   table read is finite.

#[path = "../../test-support/mutate.rs"]
mod mutate;

use mpfree_node::{Ctx, Node, NodeEvent, Timed};
use mpfree_synth::dx7::{self, Dx7Voice};
use mpfree_synth::fm::Fm;
use mpfree_synth::fm_params::{info, PARAMS};
use mpfree_synth::wavetable::import::table_from_wav;
use mutate::Mutator;

fn banks() -> Vec<Vec<u8>> {
    let voices: Vec<Dx7Voice> = Fm::presets().iter().map(Dx7Voice::from_preset).collect();
    let bank = dx7::bank(&voices);
    // One voice: the unpacked format, built from the bank's first voice.
    let first = dx7::unpack(bank[6..6 + 128].try_into().unwrap());
    let mut voice = vec![0xF0, 0x43, 0x00, 0x00, 0x01, 0x1B];
    voice.extend_from_slice(&first);
    voice.push(dx7::checksum(&first));
    voice.push(0xF7);
    vec![bank, voice]
}

/// Puts back the checksum over the data (bank or voice), so a check past it runs.
fn fix_checksum(b: &mut [u8]) {
    let n = match b.len() {
        4104 => 4096,
        163 => 155,
        _ => return,
    };
    let sum = dx7::checksum(&b[6..6 + n]);
    b[6 + n] = sum;
}

fn check_voice(case: usize, fm: &mut Fm, v: &Dx7Voice) {
    for &(id, x) in &v.values {
        assert!(
            info(id).is_some(),
            "case {case}: {}: unknown id {id}",
            v.name
        );
        assert!(x.is_finite(), "case {case}: {}: id {id} = {x}", v.name);
    }
    fm.load_preset(&v.preset());
    for p in PARAMS.iter().filter(|p| !p.readonly) {
        let x = fm.param(p.id);
        assert!(
            x.is_finite() && x >= p.min && x <= p.max,
            "case {case}: {}: {} = {x} after loading, outside {}..{}",
            v.name,
            p.name,
            p.min,
            p.max
        );
    }
    // A chord for a few blocks: every sample finite.
    fm.reset();
    let (mut l, mut r) = ([0f32; 128], [0f32; 128]);
    let on: Vec<Timed> = [48u8, 60, 67, 127]
        .iter()
        .map(|&note| {
            Timed::new(
                0,
                NodeEvent::NoteOn {
                    note,
                    velocity: 127,
                },
            )
        })
        .collect();
    for b in 0..8u64 {
        l.fill(0.0);
        r.fill(0.0);
        let ev: &[Timed] = if b == 0 { &on } else { &[] };
        fm.process(&Ctx::new(48_000, b * 128), ev, &mut l, &mut r);
        assert!(
            l.iter().chain(&r).all(|x| x.is_finite()),
            "case {case}: {} plays non-finite audio",
            v.name
        );
    }
}

#[test]
fn the_seeds_read_back() {
    for (i, b) in banks().iter().enumerate() {
        let v = dx7::parse(b).unwrap_or_else(|e| panic!("seed {i}: {e}"));
        assert!(!v.is_empty());
    }
}

#[test]
fn damaged_dx7_sysex_is_refused_or_in_range() {
    let seeds = banks();
    let mut m = Mutator::new(3);
    let mut fm = Fm::new(48_000);
    fm.prepare(48_000, 128);
    let (mut ok, mut refused) = (0, 0);
    for case in 0..1_500 {
        let mut input = m.mutate(&seeds[case % seeds.len()]);
        if case % 4 != 0 {
            fix_checksum(&mut input);
        }
        match dx7::parse(&input) {
            Ok(voices) => {
                ok += 1;
                // A bank's damage is in one or two voices: check those,
                // and a few others.
                let seed = dx7::parse(&seeds[case % seeds.len()]).unwrap();
                for (k, v) in voices.iter().enumerate() {
                    if seed.get(k) != Some(v) || k % 8 == case % 8 {
                        check_voice(case, &mut fm, v);
                    }
                }
            }
            Err(_) => refused += 1,
        }
    }
    assert!(ok > 100 && refused > 100, "ok {ok}, refused {refused}");
}

fn wav(tag: u16, bits: u16, channels: u16, samples: usize, clm: bool) -> Vec<u8> {
    let rate = 48_000u32;
    let align = channels * bits / 8;
    let mut data = Vec::new();
    for i in 0..samples * usize::from(channels) {
        let x = 1.0 - 2.0 * (i % 256) as f64 / 256.0;
        match (tag, bits) {
            (3, 32) => data.extend((x as f32).to_le_bytes()),
            (3, 64) => data.extend(x.to_le_bytes()),
            (1, 8) => data.push((x * 127.0 + 128.0) as u8),
            (1, 16) => data.extend(((x * 32767.0) as i16).to_le_bytes()),
            (1, 24) => data.extend(&((x * 8_388_607.0) as i32).to_le_bytes()[..3]),
            _ => data.extend(((x * 2_147_483_647.0) as i32).to_le_bytes()),
        }
    }
    let mut d = b"RIFF\0\0\0\0WAVEfmt ".to_vec();
    d.extend(16u32.to_le_bytes());
    d.extend(tag.to_le_bytes());
    d.extend(channels.to_le_bytes());
    d.extend(rate.to_le_bytes());
    d.extend((rate * u32::from(align)).to_le_bytes());
    d.extend(align.to_le_bytes());
    d.extend(bits.to_le_bytes());
    if clm {
        d.extend(b"clm ");
        d.extend(8u32.to_le_bytes());
        d.extend(b"<!>256 x");
    }
    d.extend(b"data");
    d.extend((data.len() as u32).to_le_bytes());
    d.extend(&data);
    let n = (d.len() - 8) as u32;
    d[4..8].copy_from_slice(&n.to_le_bytes());
    d
}

fn wavs() -> Vec<Vec<u8>> {
    vec![
        wav(3, 32, 1, 2048, false),
        wav(3, 64, 1, 512, false),
        wav(1, 8, 2, 256, false),
        wav(1, 16, 1, 1024, true),
        wav(1, 24, 2, 512, true),
        wav(1, 32, 1, 256, false),
    ]
}

#[test]
fn the_wav_seeds_make_tables() {
    for (i, w) in wavs().iter().enumerate() {
        table_from_wav("seed", w).unwrap_or_else(|e| panic!("seed {i}: {e:?}"));
    }
}

#[test]
fn damaged_wavetable_wavs_are_refused_or_finite() {
    let seeds = wavs();
    let mut m = Mutator::new(4);
    let (mut ok, mut refused) = (0, 0);
    for case in 0..1_500 {
        let input = m.mutate(&seeds[case % seeds.len()]);
        match table_from_wav("fuzz", &input) {
            Ok(t) => {
                ok += 1;
                assert!(t.frames() > 0, "case {case}");
                for (k, lv) in t.levels().iter().enumerate() {
                    for f in [0.0, 0.37, 1.0] {
                        for ph in [0.0, 0.25, 0.999] {
                            let x =
                                mpfree_synth::wavetable::table::read_level(lv, t.frames(), f, ph);
                            assert!(x.is_finite(), "case {case}, level {k}: {x}");
                        }
                    }
                }
                for fr in 0..t.frames() {
                    assert!(t.preview(fr).iter().all(|x| x.is_finite()), "case {case}");
                }
            }
            Err(_) => refused += 1,
        }
    }
    assert!(ok > 100 && refused > 50, "ok {ok}, refused {refused}");
}
