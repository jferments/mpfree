//! Cloud reverb behaviour: Decay is the time to fall 60 dB at any sample
//! rate, the tail estimate covers the ring-out, Freeze holds the tail and
//! ignores new input, Width 0 is mono and full width is decorrelated, Mix 0
//! is the input exactly, a reset forgets everything, a Character change
//! dips and recovers, Light quality keeps the decay and level and wakes
//! its lines silent, and the presets are in range. The node rules (valid
//! description, any block size, every extreme, no allocation) are checked
//! with the other effects in `fx.rs` and `no_alloc.rs`.

use mpfree_fx::ids::cloud::*;
use mpfree_fx::mpfree_node::{Ctx, Node, NodeEvent, Timed, MAX_BLOCK};
use mpfree_fx::{Cloud, CLOUD_PRESETS};

const RATE: u32 = 48_000;

fn cloud(rate: u32, values: &[(u16, f32)]) -> Cloud {
    let mut c = Cloud::new();
    c.prepare(rate, MAX_BLOCK);
    for &(id, v) in values {
        c.set_param(id, v);
    }
    c.reset();
    c
}

fn run(c: &mut Cloud, rate: u32, l: &mut [f32], r: &mut [f32]) {
    for (i, (a, b)) in l.chunks_mut(128).zip(r.chunks_mut(128)).enumerate() {
        c.process(&Ctx::new(rate, (i * 128) as u64), &[], a, b);
    }
}

fn noise(seed: u64, frames: usize, amp: f32) -> Vec<f32> {
    let mut x = seed.max(1);
    (0..frames)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            amp * ((x >> 40) as f32 / (1u64 << 23) as f32 - 1.0)
        })
        .collect()
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| (v * v) as f64).sum::<f64>() / x.len().max(1) as f64).sqrt() as f32
}

fn peak(x: &[f32]) -> f32 {
    x.iter().fold(0.0f32, |m, v| m.max(v.abs()))
}

fn db(x: f32) -> f32 {
    20.0 * x.max(1e-12).log10()
}

/// The 60 dB decay time from the Schroeder curve's fall from -5 to -35 dB.
fn t60(l: &[f32], r: &[f32], rate: u32) -> f32 {
    let e: Vec<f64> = l
        .iter()
        .zip(r)
        .map(|(a, b)| (*a as f64).powi(2) + (*b as f64).powi(2))
        .collect();
    let mut tail = vec![0.0; e.len() + 1];
    for i in (0..e.len()).rev() {
        tail[i] = tail[i + 1] + e[i];
    }
    let level = |i: usize| 10.0 * (tail[i] / tail[0]).log10();
    let a = (0..e.len()).find(|&i| level(i) < -5.0).unwrap();
    let b = (0..e.len()).find(|&i| level(i) < -35.0).unwrap();
    2.0 * (b - a) as f32 / rate as f32
}

/// The flat, wide-open settings for measuring decay.
const OPEN: [(u16, f32); 6] = [
    (MIX, 100.0),
    (EARLY, 0.0),
    (PRE_DELAY, 0.0),
    (DAMPING, 20_000.0),
    (LOW_CUT, 20.0),
    (HIGH_CUT, 20_000.0),
];

#[test]
fn decay_is_the_time_to_fall_60_db_at_any_rate() {
    // Cloud's long early diffuser adds its own ring to short decays, so it
    // is measured at the long ones it is made for.
    // The last two: big sizes, where long diffuser stages would outlast a
    // short decay without their feedback cap.
    let cases = [
        (0.0, 1000.0, 48_000, 120.0),
        (0.0, 4000.0, 44_100, 120.0),
        (2.0, 1000.0, 96_000, 120.0),
        (2.0, 4000.0, 48_000, 120.0),
        (3.0, 4000.0, 48_000, 120.0),
        (3.0, 8000.0, 44_100, 120.0),
        (3.0, 3000.0, 48_000, 400.0),
        (2.0, 1500.0, 48_000, 600.0),
    ];
    for (character, decay_ms, rate, size) in cases {
        let mut v = OPEN.to_vec();
        v.extend([(CHARACTER, character), (DECAY, decay_ms), (SIZE, size)]);
        let mut c = cloud(rate, &v);
        let frames = (rate as f32 * (decay_ms / 1000.0 * 1.5 + 0.5)) as usize;
        let (mut l, mut r) = (vec![0.0f32; frames], vec![0.0f32; frames]);
        l[0] = 1.0;
        r[0] = 1.0;
        run(&mut c, rate, &mut l, &mut r);
        let t = t60(&l, &r, rate);
        let want = decay_ms / 1000.0;
        println!(
            "character {character}, size {size} ms, decay {want} s at {rate} Hz: T60 {t:.2} s"
        );
        assert!(
            (t / want - 1.0).abs() < 0.2,
            "character {character}, size {size} ms, decay {want} s at {rate} Hz: T60 {t:.2} s"
        );
    }
}

#[test]
fn the_tail_estimate_covers_the_ring_out() {
    for character in 0..5 {
        for decay_ms in [300.0, 2000.0] {
            let mut c = cloud(
                RATE,
                &[
                    (CHARACTER, character as f32),
                    (DECAY, decay_ms),
                    (MIX, 100.0),
                ],
            );
            let tail = c.tail_frames() as usize;
            let frames = tail + RATE as usize / 2;
            let (mut l, mut r) = (vec![0.0f32; frames], vec![0.0f32; frames]);
            l[0] = 1.0;
            r[0] = 1.0;
            run(&mut c, RATE, &mut l, &mut r);
            let all = peak(&l).max(peak(&r));
            let after = peak(&l[tail..]).max(peak(&r[tail..]));
            assert!(
                db(after / all) < -80.0,
                "character {character}, decay {decay_ms} ms: {:.1} dB after {tail} frames",
                db(after / all)
            );
        }
    }
}

#[test]
fn freeze_holds_the_tail_and_ignores_new_input() {
    let settings = [(CHARACTER, 3.0), (DECAY, 1500.0), (MIX, 100.0)];
    let s = RATE as usize;
    let render = |during: &[f32]| {
        let mut c = cloud(RATE, &settings);
        let x = noise(3, s / 2, 0.3);
        let (mut l, mut r) = (x.clone(), x.clone());
        run(&mut c, RATE, &mut l, &mut r);
        c.set_param(FREEZE, 1.0);
        let (mut fl, mut fr) = (during.to_vec(), during.to_vec());
        run(&mut c, RATE, &mut fl, &mut fr);
        (c, fl, fr)
    };
    // Silence while the freeze fades in, then silence or loud noise.
    let quiet = vec![0.0f32; 4 * s];
    let mut loud = noise(9, 4 * s, 0.8);
    loud[..s / 2].fill(0.0);
    let (mut c, ql, qr) = render(&quiet);
    let (_, ll, lr) = render(&loud);
    assert_eq!(c.tail_frames(), u32::MAX, "frozen: an endless tail");
    // Once the freeze has faded in, new input changes nothing.
    assert!(ql == ll && qr == lr, "input ignored while frozen");
    let first = rms(&ql[s / 2..3 * s / 2]);
    let last = rms(&ql[3 * s..]);
    println!(
        "frozen tail: {:.1} dB after 1 s, {:.1} dB after 3.5 s",
        db(first),
        db(last)
    );
    assert!(
        first > 0.01 && db(last / first) > -3.0,
        "the tail holds: {:.1} dB over 2.5 s",
        db(last / first)
    );
    // Unfrozen, it decays again: 4 s at a 1.5 s decay is over 100 dB.
    c.set_param(FREEZE, 0.0);
    let (mut l, mut r) = (vec![0.0f32; 4 * s], vec![0.0f32; 4 * s]);
    run(&mut c, RATE, &mut l, &mut r);
    assert!(
        db(rms(&l[3 * s..]) / first) < -60.0,
        "decays once unfrozen: {:.1} dB",
        db(rms(&l[3 * s..]) / first)
    );
    assert!(c.tail_frames() < u32::MAX);
}

#[test]
fn full_width_is_decorrelated_and_width_zero_is_mono() {
    let x = noise(5, 2 * RATE as usize, 0.3);
    for (width, check) in [(100.0, true), (0.0, false)] {
        let mut c = cloud(RATE, &[(MIX, 100.0), (WIDTH, width)]);
        let (mut l, mut r) = (x.clone(), x.clone());
        run(&mut c, RATE, &mut l, &mut r);
        let (a, b) = (&l[RATE as usize..], &r[RATE as usize..]);
        if check {
            let corr = a
                .iter()
                .zip(b)
                .map(|(p, q)| (*p as f64) * (*q as f64))
                .sum::<f64>()
                / (rms(a) as f64 * rms(b) as f64 * a.len() as f64);
            assert!(
                corr.abs() < 0.2,
                "a mono input comes out wide: correlation {corr:.2}"
            );
        } else {
            assert!(a == b, "width 0 is mono");
        }
    }
}

#[test]
fn mix_zero_is_the_input_exactly() {
    let x = noise(7, RATE as usize, 0.9);
    let y = noise(8, RATE as usize, 0.9);
    let mut c = cloud(RATE, &[(MIX, 0.0), (CHARACTER, 4.0), (DECAY, 10_000.0)]);
    let (mut l, mut r) = (x.clone(), y.clone());
    run(&mut c, RATE, &mut l, &mut r);
    assert!(l == x && r == y);
}

#[test]
fn a_reset_forgets_everything() {
    let settings = [
        (CHARACTER, 3.0),
        (DECAY, 20_000.0),
        (MIX, 100.0),
        (MODULATION, 80.0),
    ];
    let x = noise(11, RATE as usize, 0.5);
    let mut fresh = cloud(RATE, &settings);
    let (mut a, mut b) = (x.clone(), x.clone());
    run(&mut fresh, RATE, &mut a, &mut b);
    // Fill the buffers with something else (and switch character), then
    // reset and play the same input.
    let mut used = cloud(RATE, &settings);
    used.set_param(CHARACTER, 1.0);
    let other = noise(12, 3 * RATE as usize, 0.9);
    let (mut l, mut r) = (other.clone(), other.clone());
    run(&mut used, RATE, &mut l, &mut r);
    used.set_param(CHARACTER, 3.0);
    used.reset();
    let (mut c, mut d) = (x.clone(), x.clone());
    run(&mut used, RATE, &mut c, &mut d);
    assert!(
        a == c && b == d,
        "after a reset the output is a fresh node's"
    );
}

#[test]
fn a_character_change_dips_the_wet_signal_and_recovers() {
    let mut c = cloud(RATE, &[(MIX, 100.0), (CHARACTER, 2.0)]);
    let s = RATE as usize;
    let x = noise(13, 2 * s, 0.3);
    let (mut l, mut r) = (x.clone(), x.clone());
    // The change lands at 1 s; the dip reaches zero 15 ms later.
    let at = s;
    let mut done = 0;
    while done < l.len() {
        let len = 128.min(l.len() - done);
        let ev = [Timed::new(
            at.wrapping_sub(done) as u32,
            NodeEvent::Param {
                id: CHARACTER,
                value: 3.0,
            },
        )];
        let events: &[Timed] = if (done..done + len).contains(&at) {
            &ev
        } else {
            &[]
        };
        c.process(
            &Ctx::new(RATE, done as u64),
            events,
            &mut l[done..done + len],
            &mut r[done..done + len],
        );
        done += len;
    }
    assert_eq!(c.param(CHARACTER), 3.0);
    let before = rms(&l[s - 4800..s]);
    let swap = at + 720;
    let dip = peak(&l[swap - 8..swap + 8]);
    let after = rms(&l[s + 9600..s + 14_400]);
    println!(
        "before {:.1} dB, at the swap {:.1} dB, 200 ms later {:.1} dB",
        db(before),
        db(dip),
        db(after)
    );
    assert!(
        db(dip / before) < -20.0,
        "dipped at the swap: {:.1} dB",
        db(dip / before)
    );
    assert!(
        db(after / before).abs() < 6.0,
        "back within 6 dB: {:.1} dB",
        db(after / before)
    );
}

#[test]
fn light_quality_keeps_the_decay_and_the_level() {
    // Light: half the lines and stages. The decay and the loudness stay
    // (within the same 20 % and 3 dB) for every character.
    for character in 0..5 {
        let decay_ms = 2000.0;
        let mut v = OPEN.to_vec();
        v.extend([(CHARACTER, character as f32), (DECAY, decay_ms)]);
        let render = |light: f32| {
            let mut v = v.clone();
            v.push((QUALITY, light));
            let mut c = cloud(RATE, &v);
            let frames = (RATE as f32 * (decay_ms / 1000.0 * 1.5 + 0.5)) as usize;
            let (mut l, mut r) = (vec![0.0f32; frames], vec![0.0f32; frames]);
            l[..RATE as usize / 10].copy_from_slice(&noise(5, RATE as usize / 10, 0.3));
            r[..RATE as usize / 10].copy_from_slice(&noise(6, RATE as usize / 10, 0.3));
            run(&mut c, RATE, &mut l, &mut r);
            (l, r)
        };
        let (fl, fr) = render(0.0);
        let (ll, lr) = render(1.0);
        let (tf, tl) = (t60(&fl, &fr, RATE), t60(&ll, &lr, RATE));
        let w = RATE as usize / 5..RATE as usize;
        let level = db(rms(&ll[w.clone()]) / rms(&fl[w]));
        println!(
            "character {character}: T60 full {tf:.2} s, light {tl:.2} s; light is {level:+.1} dB"
        );
        assert!(
            (tl / tf - 1.0).abs() < 0.2,
            "character {character}: T60 full {tf:.2} s, light {tl:.2} s"
        );
        assert!(
            level.abs() < 3.0,
            "character {character}: light is {level:+.1} dB"
        );
    }
}

#[test]
fn a_quality_change_dips_and_wakes_its_lines_silent() {
    // Full, then Light for a second (the lines Light drops hold their
    // tail), then Full again with no input: the dropped lines start from
    // silence, so nothing from before comes back.
    let mut c = cloud(RATE, &[(MIX, 100.0), (CHARACTER, 0.0), (DECAY, 60_000.0)]);
    let s = RATE as usize;
    let (mut l, mut r) = (noise(21, s, 0.3), noise(22, s, 0.3));
    run(&mut c, RATE, &mut l, &mut r);
    c.set_param(QUALITY, 1.0);
    c.set_param(FREEZE, 1.0);
    let (mut l, mut r) = (vec![0.0f32; s], vec![0.0f32; s]);
    run(&mut c, RATE, &mut l, &mut r);
    let light = rms(&l[s / 2..]);
    assert!(light > 1e-3, "the frozen light tail plays: {light}");
    c.set_param(QUALITY, 0.0);
    let (mut l, mut r) = (vec![0.0f32; s], vec![0.0f32; s]);
    run(&mut c, RATE, &mut l, &mut r);
    let full = rms(&l[s / 2..]);
    println!(
        "frozen tail: light {:.1} dB, full again {:.1} dB",
        db(light),
        db(full)
    );
    assert!(
        db(full / light) < 1.0,
        "the woken lines add no old tail: {:.1} dB",
        db(full / light)
    );
    assert_eq!(c.param(QUALITY), 0.0);
}

#[test]
fn presets_are_in_range_and_play() {
    let c = Cloud::new();
    let params = c.params();
    for p in CLOUD_PRESETS {
        for &(id, v) in p.values {
            let info = &params[id as usize];
            assert_eq!(
                info.clamp(v),
                v,
                "{}: {} = {v} is in range",
                p.name,
                info.name
            );
        }
        let mut node = cloud(RATE, p.values);
        let x = noise(17, RATE as usize, 0.5);
        let (mut l, mut r) = (x.clone(), x.clone());
        run(&mut node, RATE, &mut l, &mut r);
        assert!(l.iter().chain(&r).all(|v| v.is_finite()), "{}", p.name);
        assert!(rms(&l[RATE as usize / 2..]) > 0.01, "{} is audible", p.name);
    }
}
