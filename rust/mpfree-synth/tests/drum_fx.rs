//! Drums' per-drum effects and the kit's sends ([`mpfree_synth::drum_fx`])
//! through the mpfree-node interface.

use mpfree_node::preset::Preset;
use mpfree_node::{check_params, Ctx, Node, NodeEvent, Timed, MAX_BLOCK};
use mpfree_synth::drum_fx::{self, field as fx, fx_id, kit, ALL, BASE, END, N_FX};
use mpfree_synth::drum_params::{self, field, model, pad_id, LEVEL, PADS};
use mpfree_synth::Drums;

const RATE: u32 = 48_000;

fn render(
    d: &mut Drums,
    frames: usize,
    events: &[(u64, NodeEvent)],
    mut cut: impl FnMut(usize) -> usize,
) -> (Vec<f32>, Vec<f32>) {
    let mut l = vec![0.0f32; frames];
    let mut r = vec![0.0f32; frames];
    let (mut at, mut n) = (0usize, 0usize);
    let mut buf = Vec::new();
    while at < frames {
        let len = cut(n).clamp(1, MAX_BLOCK).min(frames - at);
        n += 1;
        buf.clear();
        for &(f, ev) in events {
            if f >= at as u64 && f < (at + len) as u64 {
                buf.push(Timed::new((f - at as u64) as u32, ev));
            }
        }
        d.process(
            &Ctx::new(RATE, at as u64),
            &buf,
            &mut l[at..at + len],
            &mut r[at..at + len],
        );
        at += len;
    }
    (l, r)
}

fn hit(pad: u8, velocity: u8) -> NodeEvent {
    NodeEvent::NoteOn {
        note: 36 + pad,
        velocity,
    }
}
fn set(id: u16, value: f32) -> NodeEvent {
    NodeEvent::Param { id, value }
}
fn peak(x: &[f32]) -> f32 {
    x.iter().fold(0.0, |a, &v| a.max(v.abs()))
}
fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
}
fn db(x: f32) -> f32 {
    20.0 * x.max(1e-12).log10()
}
fn ms(t: f32) -> usize {
    (t * 0.001 * RATE as f32) as usize
}

/// A drum alone: `pad` with its effects set as `fx`, hit once at full
/// velocity, `frames` long (left channel).
fn one(pad: usize, fx: &[(u16, f32)], frames: usize) -> Vec<f32> {
    let mut d = Drums::new(RATE);
    for &(f, v) in fx {
        d.set_param(fx_id(pad, f), v);
    }
    render(&mut d, frames, &[(0, hit(pad as u8, 127))], |_| 128).0
}

#[test]
fn the_effects_settings_are_valid_and_sit_in_their_range() {
    check_params(&ALL).unwrap();
    assert_eq!(ALL.len(), drum_params::N_PARAMS + N_FX);
    assert_eq!(N_FX, PADS * fx::COUNT + kit::COUNT);
    let d = Drums::new(RATE);
    assert_eq!(d.params().len(), ALL.len());
    for p in &drum_fx::PARAMS {
        assert!(
            drum_fx::owns(p.id) && p.id >= BASE && p.id < END,
            "{}",
            p.id
        );
        assert_eq!(d.param(p.id), p.default, "{} {}", p.group, p.name);
    }
    // The voices' ids are below the range (their owner keeps 0 to 1023).
    for p in &drum_params::PARAMS {
        assert!(p.id < BASE);
    }
    // Every drum's effects have one group and the same names.
    for pad in 0..PADS {
        let names: Vec<_> = drum_fx::PARAMS
            .iter()
            .filter(|p| p.group == format!("Pad {} effects", pad + 1))
            .map(|p| p.name)
            .collect();
        assert_eq!(names.len(), fx::COUNT);
        assert_eq!(names[0], "Punch");
        assert_eq!(names[fx::REVERB_SEND as usize], "Reverb send");
    }
    // No two settings on one drum's page share a name.
    let one: Vec<_> = drum_fx::PARAMS[..fx::COUNT]
        .iter()
        .map(|p| p.name)
        .collect();
    for (i, n) in one.iter().enumerate() {
        assert!(!one[..i].contains(n), "{n} twice");
    }
}

#[test]
fn settings_clamp_and_unknown_ids_are_ignored() {
    let mut d = Drums::new(RATE);
    d.set_param(fx_id(3, fx::DISTORTION), 99.0);
    assert_eq!(d.param(fx_id(3, fx::DISTORTION)), 48.0);
    d.set_param(kit::DELAY_FEEDBACK, 500.0);
    assert_eq!(d.param(kit::DELAY_FEEDBACK), 90.0);
    // A gap in a drum's range and a gap after the kit's.
    d.set_param(fx_id(0, 40), 1.0);
    assert!(d.param(fx_id(0, 40)).is_nan());
    assert!(d.param(kit::REVERB_LEVEL + 1).is_nan());
    d.set_param(fx_id(2, fx::LOW), f32::NAN);
    assert!(d.param(fx_id(2, fx::LOW)).is_finite());
}

/// Every effect and send at its default: the drums play bit for bit as
/// with no effects, straight to the output.
#[test]
fn nothing_up_is_bit_exact_and_engages_nothing() {
    let ev: Vec<_> = (0..40u64)
        .map(|k| (k * 997, hit((k * 5 % 16) as u8, 30 + (k * 11 % 97) as u8)))
        .collect();
    let (rl, rr) = render(&mut Drums::new(RATE), 48_000, &ev, |_| 128);
    // Each setting set to its default, and one turned up and back down
    // before any hit.
    let mut d = Drums::new(RATE);
    for p in &drum_fx::PARAMS {
        d.set_param(p.id, p.default);
    }
    d.set_param(fx_id(4, fx::PUNCH), 50.0);
    d.set_param(fx_id(4, fx::PUNCH), 0.0);
    let (l, r) = render(&mut d, 48_000, &ev, |_| 128);
    assert!(l == rl && r == rr);
    assert!(!d.fx().engaged());
    // Distortion up but its mix at 0 % changes nothing either.
    let mut d = Drums::new(RATE);
    d.set_param(fx_id(0, fx::DIST_MIX), 0.0);
    d.set_param(fx_id(0, fx::DISTORTION), 30.0);
    let (l, r) = render(&mut d, 48_000, &ev, |_| 128);
    assert!(l == rl && r == rr);
}

#[test]
fn same_output_for_any_block_size_with_effects_and_sends() {
    let mut ev = Vec::new();
    for k in 0..70u64 {
        ev.push((
            k * 743 + 5,
            hit((k * 7 % 16) as u8, 40 + (k * 13 % 88) as u8),
        ));
    }
    // Every effect up somewhere, both sends, changes mid-stream.
    let setup = [
        (fx_id(0, fx::PUNCH), 60.0),
        (fx_id(0, fx::DISTORTION), 18.0),
        (fx_id(0, fx::RATIO), 4.0),
        (fx_id(1, fx::SUSTAIN), -40.0),
        (fx_id(1, fx::REVERB_SEND), 40.0),
        (fx_id(1, fx::DIST_TONE), 60.0),
        (fx_id(2, fx::DELAY_SEND), 50.0),
        (fx_id(3, fx::HIGH), 6.0),
        (fx_id(3, fx::DIST_TONE), -50.0),
        (fx_id(3, fx::DISTORTION), 10.0),
        (fx_id(5, fx::MID), -8.0),
        (fx_id(5, fx::LOW), 4.0),
        (fx_id(7, fx::OUTPUT), -6.0),
        (fx_id(9, fx::THRESHOLD), -30.0),
        (fx_id(9, fx::RATIO), 8.0),
        (kit::REVERB_GATE, 200.0),
    ];
    for &(id, v) in &setup {
        ev.push((0, set(id, v)));
    }
    ev.push((9_000, set(fx_id(0, fx::DISTORTION), 30.0)));
    ev.push((12_000, set(fx_id(5, fx::MID_FREQ), 3000.0)));
    ev.push((15_000, set(fx_id(1, fx::REVERB_SEND), 0.0)));
    ev.push((18_000, set(fx_id(0, fx::PUNCH), 0.0)));
    ev.push((20_000, set(LEVEL, -6.0)));
    ev.push((24_000, set(kit::DELAY_TIME, 2.0)));
    ev.push((26_000, set(fx_id(9, fx::RATIO), 1.0)));
    ev.push((40_000, NodeEvent::AllOff));
    ev.sort_by_key(|e| e.0);
    let frames = 70_000;
    let (rl, rr) = render(&mut Drums::new(RATE), frames, &ev, |_| 128);
    assert!(peak(&rl) > 0.05);
    for size in [1usize, 7, 16, 17, 64, 129, 1000, 8192] {
        let (l, r) = render(&mut Drums::new(RATE), frames, &ev, |_| size);
        let worst = l
            .iter()
            .zip(&rl)
            .chain(r.iter().zip(&rr))
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        assert!(l == rl && r == rr, "block size {size} differs by {worst:e}");
    }
    let mut seed = 9u64;
    let (l, r) = render(&mut Drums::new(RATE), frames, &ev, |_| {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        1 + (seed >> 33) as usize % 700
    });
    assert!(l == rl && r == rr, "random block sizes differ");
}

#[test]
fn punch_and_sustain_shape_the_hit_at_any_level() {
    let n = ms(400.0);
    let front = |x: &[f32]| rms(&x[..ms(8.0)]);
    let tail = |x: &[f32]| rms(&x[ms(120.0)..ms(300.0)]);
    for level in [0.0, -12.0] {
        let lv = [(0u16, 0.0)];
        let mut base = Drums::new(RATE);
        base.set_param(pad_id(0, field::LEVEL), level);
        let dry = render(&mut base, n, &[(0, hit(0, 127))], |_| 128).0;
        let with = |fx: &[(u16, f32)]| {
            let mut d = Drums::new(RATE);
            d.set_param(pad_id(0, field::LEVEL), level);
            for &(f, v) in fx.iter().chain(&lv) {
                if f != 0 || v != 0.0 {
                    d.set_param(fx_id(0, f), v);
                }
            }
            render(&mut d, n, &[(0, hit(0, 127))], |_| 128).0
        };
        let up = with(&[(fx::PUNCH, 100.0)]);
        let down = with(&[(fx::PUNCH, -100.0)]);
        let long = with(&[(fx::SUSTAIN, 100.0)]);
        let short = with(&[(fx::SUSTAIN, -100.0)]);
        let rise = db(front(&up)) - db(front(&dry));
        let cut = db(front(&down)) - db(front(&dry));
        assert!(rise > 3.0, "level {level}: Punch +100 adds {rise:.1} dB");
        assert!(cut < -3.0, "level {level}: Punch -100 takes {cut:.1} dB");
        assert!(
            db(tail(&long)) > db(tail(&dry)) + 3.0,
            "level {level}: Sustain +100"
        );
        assert!(
            db(tail(&short)) < db(tail(&dry)) - 3.0,
            "level {level}: Sustain -100"
        );
    }
    // Level doesn't matter, only the shape: the same gain at 0 and -12 dB.
    let gain = |level: f32| {
        let mut a = Drums::new(RATE);
        a.set_param(pad_id(0, field::LEVEL), level);
        let dry = render(&mut a, n, &[(0, hit(0, 127))], |_| 128).0;
        let mut b = Drums::new(RATE);
        b.set_param(pad_id(0, field::LEVEL), level);
        b.set_param(fx_id(0, fx::PUNCH), 60.0);
        let wet = render(&mut b, n, &[(0, hit(0, 127))], |_| 128).0;
        db(front(&wet)) - db(front(&dry))
    };
    assert!((gain(0.0) - gain(-12.0)).abs() < 1.0);
}

#[test]
fn distortion_adds_grit_without_getting_louder() {
    // The snare: noise and a body, peaking near -9 dBFS.
    let n = ms(300.0);
    let dry = one(1, &[], n);
    for amount in [12.0, 24.0, 48.0] {
        let wet = one(1, &[(fx::DISTORTION, amount)], n);
        let (pd, pw) = (db(peak(&dry)), db(peak(&wet)));
        assert!(pw < pd + 3.0, "{amount} dB: peak {pd:.1} -> {pw:.1}");
        // Denser: the tail comes up against the peak.
        let crest = |x: &[f32]| db(peak(x)) - db(rms(&x[ms(30.0)..]));
        assert!(
            crest(&wet) < crest(&dry) - 2.0,
            "{amount} dB: crest {:.1} -> {:.1}",
            crest(&dry),
            crest(&wet)
        );
    }
    // Half the mix sits between dry and full.
    let full = one(1, &[(fx::DISTORTION, 36.0)], n);
    let half = one(1, &[(fx::DISTORTION, 36.0), (fx::DIST_MIX, 50.0)], n);
    let tail = |x: &[f32]| rms(&x[ms(60.0)..]);
    assert!(tail(&half) > tail(&dry) && tail(&half) < tail(&full));
    // Dark takes highs away, bright adds them (against the plain
    // distortion), seen as the first difference's energy.
    let hi = |x: &[f32]| rms(&x.windows(2).map(|w| w[1] - w[0]).collect::<Vec<_>>());
    let dark = one(1, &[(fx::DISTORTION, 24.0), (fx::DIST_TONE, -100.0)], n);
    let bright = one(1, &[(fx::DISTORTION, 24.0), (fx::DIST_TONE, 100.0)], n);
    let plain = one(1, &[(fx::DISTORTION, 24.0)], n);
    assert!(hi(&dark) < hi(&plain) * 0.7);
    assert!(hi(&bright) > hi(&plain));
}

#[test]
fn the_eq_moves_its_bands() {
    // The long kick (pad 14, a 1.5 s 50 Hz body after its sweep).
    let n = ms(600.0);
    let body = |x: &[f32]| rms(&x[ms(200.0)..]);
    let dry = one(13, &[], n);
    let low = one(13, &[(fx::LOW, 12.0)], n);
    let cut = one(13, &[(fx::LOW, -12.0)], n);
    let up = db(body(&low)) - db(body(&dry));
    let down = db(body(&cut)) - db(body(&dry));
    assert!((8.0..13.0).contains(&up), "Low +12 at 47 Hz: {up:.1} dB");
    assert!(
        (-13.0..-8.0).contains(&down),
        "Low -12 at 47 Hz: {down:.1} dB"
    );
    // The bell at the body's pitch, and far from it.
    let bell = one(13, &[(fx::MID, 12.0), (fx::MID_FREQ, 100.0)], n);
    let far = one(13, &[(fx::MID, 12.0), (fx::MID_FREQ, 8000.0)], n);
    assert!(db(body(&bell)) > db(body(&dry)) + 3.0);
    assert!((db(body(&far)) - db(body(&dry))).abs() < 0.5);
    // High lifts a hat.
    let hat = one(3, &[], ms(80.0));
    let bright = one(3, &[(fx::HIGH, 12.0)], ms(80.0));
    assert!(db(rms(&bright)) > db(rms(&hat)) + 4.0);
}

#[test]
fn the_compressor_holds_the_peak_down_and_lifts_the_tail() {
    // The snare: a fast attack holds its crack down, a short release lets
    // its tail come back up.
    let n = ms(300.0);
    let dry = one(1, &[], n);
    let wet = one(
        1,
        &[
            (fx::THRESHOLD, -36.0),
            (fx::RATIO, 8.0),
            (fx::COMP_ATTACK, 0.5),
            (fx::COMP_RELEASE, 40.0),
        ],
        n,
    );
    // The front's level against the tail's (the first frames' peak passes
    // before the attack, as on any compressor).
    let crest = |x: &[f32]| db(rms(&x[..ms(10.0)])) - db(rms(&x[ms(40.0)..ms(120.0)]));
    assert!(
        crest(&wet) < crest(&dry) - 3.0,
        "crest {:.1} -> {:.1}",
        crest(&dry),
        crest(&wet)
    );
    // It only turns down: the crack held under the dry peak.
    assert!(db(peak(&wet)) < db(peak(&dry)));
    // Ratio 1:1 is off: bit for bit dry, whatever the threshold.
    let off = one(1, &[(fx::THRESHOLD, -48.0), (fx::RATIO, 1.0)], n);
    assert_eq!(off, dry);
}

#[test]
fn output_sets_the_drums_level_after_its_effects() {
    let n = ms(200.0);
    let dry = one(1, &[], n);
    let quiet = one(1, &[(fx::OUTPUT, -12.0)], n);
    let d = db(peak(&quiet)) - db(peak(&dry));
    assert!((d + 12.0).abs() < 0.2, "{d}");
}

#[test]
fn pan_stays_where_the_hit_put_it() {
    // A drum panned hard left keeps its right side silent through its
    // effects.
    let mut d = Drums::new(RATE);
    d.set_param(pad_id(1, field::PAN), -100.0);
    d.set_param(fx_id(1, fx::DISTORTION), 20.0);
    let (l, r) = render(&mut d, ms(200.0), &[(0, hit(1, 127))], |_| 128);
    assert!(peak(&l) > 0.05);
    assert!(peak(&r) < 1e-6, "{}", peak(&r));
}

#[test]
fn the_reverb_send_rings_after_the_drum_and_its_level_sets_it() {
    // The rim (pad 9) is over in 60 ms; the room rings on after it.
    let n = ms(1500.0);
    let after = |x: &[f32]| rms(&x[ms(200.0)..ms(600.0)]);
    let base = one(8, &[], n);
    let wet = one(8, &[(fx::REVERB_SEND, 80.0)], n);
    assert!(after(&base) < 1e-5, "{}", after(&base));
    assert!(after(&wet) > 1e-3, "{}", after(&wet));
    let mut d = Drums::new(RATE);
    d.set_param(fx_id(8, fx::REVERB_SEND), 80.0);
    d.set_param(kit::REVERB_LEVEL, 0.0);
    let (l, _) = render(&mut d, n, &[(0, hit(8, 127))], |_| 128);
    assert!(after(&l) < 1e-5);
    // The gate cuts a long tail short.
    let long = |gate: f32| {
        let mut g = Drums::new(RATE);
        g.set_param(fx_id(8, fx::REVERB_SEND), 80.0);
        g.set_param(kit::REVERB_DECAY, 3000.0);
        g.set_param(kit::REVERB_GATE, gate);
        render(&mut g, n, &[(0, hit(8, 127))], |_| 128).0
    };
    let late = |x: &[f32]| rms(&x[ms(500.0)..ms(900.0)]);
    assert!(late(&long(100.0)) < late(&long(0.0)) * 0.1);
}

#[test]
fn the_delay_send_echoes_on_the_beat() {
    // 1/8 at 120 BPM is 250 ms; the echo arrives there, on the right
    // first (ping-pong), the rim long gone.
    let n = ms(700.0);
    let mut d = Drums::new(RATE);
    d.set_param(fx_id(8, fx::DELAY_SEND), 100.0);
    assert_eq!(d.param(kit::DELAY_TIME), 5.0);
    let (l, r) = render(&mut d, n, &[(0, hit(8, 127))], |_| 128);
    let around = |x: &[f32], t: f32| peak(&x[ms(t - 20.0)..ms(t + 40.0)]);
    let echo = around(&l, 250.0).max(around(&r, 250.0));
    assert!(echo > 0.01, "{echo}");
    assert!(peak(&l[ms(120.0)..ms(225.0)]) < echo * 0.1);
}

#[test]
fn quiet_drums_and_sends_fall_asleep_and_wake_on_a_hit() {
    let mut d = Drums::new(RATE);
    d.set_param(fx_id(0, fx::PUNCH), 40.0);
    d.set_param(fx_id(1, fx::RATIO), 4.0);
    d.set_param(fx_id(2, fx::REVERB_SEND), 50.0);
    d.set_param(fx_id(3, fx::DELAY_SEND), 50.0);
    let ev = [
        (0, hit(0, 127)),
        (0, hit(1, 127)),
        (0, hit(2, 127)),
        (0, hit(3, 127)),
    ];
    render(&mut d, ms(100.0), &ev, |_| 128);
    assert!(d.fx().running() >= 4);
    assert_eq!(d.fx().sends_running(), (true, true));
    // Long after: every bus and both sends asleep, and nothing runs.
    let (l, _) = render(&mut d, ms(12_000.0), &[], |_| 128);
    assert!(peak(&l[ms(11_000.0)..]) == 0.0);
    assert_eq!(d.fx().running(), 0);
    assert_eq!(d.fx().sends_running(), (false, false));
    let s = d.fx().stats();
    assert!(s.sleeps >= 4 && s.send_sleeps >= 2, "{s:?}");
    // A drum with its effects still up stays on its bus, ready.
    assert_eq!(d.fx().routed(), 4);
    let (l, _) = render(&mut d, ms(100.0), &[(0, hit(0, 127))], |_| 128);
    assert!(peak(&l) > 0.05);
    assert_eq!(d.fx().running(), 1);
}

#[test]
fn reset_silences_every_tail() {
    let mut d = Drums::new(RATE);
    d.set_param(fx_id(1, fx::REVERB_SEND), 100.0);
    d.set_param(fx_id(1, fx::DELAY_SEND), 100.0);
    d.set_param(kit::DELAY_FEEDBACK, 90.0);
    render(&mut d, ms(300.0), &[(0, hit(1, 127))], |_| 128);
    d.reset();
    let (l, r) = render(&mut d, ms(200.0), &[], |_| 128);
    assert_eq!(peak(&l).max(peak(&r)), 0.0);
}

#[test]
fn the_tail_covers_the_sends() {
    let mut d = Drums::new(RATE);
    let dry = d.tail_frames();
    d.set_param(fx_id(1, fx::REVERB_SEND), 50.0);
    d.set_param(kit::REVERB_DECAY, 5000.0);
    assert!(d.tail_frames() > dry + RATE * 4);
}

#[test]
fn extreme_effect_settings_stay_finite() {
    for p in drum_fx::PARAMS.iter() {
        for v in [p.min, p.max, f32::NAN, f32::INFINITY] {
            let mut d = Drums::new(RATE);
            // Everything loud on the way in, so each stage is pushed.
            for pad in 0..PADS {
                d.set_param(pad_id(pad, field::LEVEL), 6.0);
                d.set_param(pad_id(pad, field::DRIVE), 100.0);
            }
            d.set_param(LEVEL, 6.0);
            d.set_param(p.id, v);
            let ev: Vec<_> = (0..16u8).map(|k| (k as u64 * 10, hit(k, 127))).collect();
            let (l, r) = render(&mut d, 6000, &ev, |_| 333);
            assert!(
                l.iter().chain(&r).all(|x| x.is_finite()),
                "{} {} = {v}",
                p.group,
                p.name
            );
            assert!(
                peak(&l) < 40.0,
                "{} {} = {v}: {}",
                p.group,
                p.name,
                peak(&l)
            );
        }
    }
    // Every effect at its most on every drum at once.
    let mut d = Drums::new(RATE);
    for pad in 0..PADS {
        for (f, v) in [
            (fx::PUNCH, 100.0),
            (fx::SUSTAIN, 100.0),
            (fx::DISTORTION, 48.0),
            (fx::DIST_TONE, 100.0),
            (fx::LOW, 15.0),
            (fx::MID, 15.0),
            (fx::HIGH, 15.0),
            (fx::THRESHOLD, -48.0),
            (fx::RATIO, 20.0),
            (fx::COMP_ATTACK, 0.1),
            (fx::OUTPUT, 12.0),
            (fx::DELAY_SEND, 100.0),
            (fx::REVERB_SEND, 100.0),
        ] {
            d.set_param(fx_id(pad, f), v);
        }
    }
    d.set_param(kit::DELAY_FEEDBACK, 90.0);
    let ev: Vec<_> = (0..64u64)
        .map(|k| (k * 300, hit((k % 16) as u8, 127)))
        .collect();
    let (l, r) = render(&mut d, 48_000, &ev, |_| 128);
    assert!(l.iter().chain(&r).all(|x| x.is_finite()));
}

#[test]
fn a_kit_with_effects_saves_and_loads_to_the_bit() {
    let mut d = Drums::new(RATE);
    d.set_param(pad_id(0, field::MODEL), f32::from(model::KICK));
    d.set_param(fx_id(0, fx::PUNCH), 35.0);
    d.set_param(fx_id(0, fx::DISTORTION), 9.5);
    d.set_param(fx_id(5, fx::MID_FREQ), 2345.6);
    d.set_param(kit::REVERB_SIZE, 70.0);
    d.set_param(kit::DELAY_TIME, 8.0);
    let p = Preset::capture(&d, "Fx Kit", &[]);
    let text = p.to_text(d.params());
    assert!(text.contains("pad-1-effects.punch = 35"), "{text}");
    assert!(text.contains("sends.delay-time = 1/4"), "{text}");
    let back = Preset::parse(&text, d.info(), d.params()).unwrap();
    let mut e = Drums::new(RATE);
    back.apply(&mut e).unwrap();
    for q in ALL.iter().filter(|q| !q.readonly) {
        assert_eq!(
            e.param(q.id).to_bits(),
            d.param(q.id).to_bits(),
            "{}",
            q.name
        );
    }
    // Loading a kit puts every effect back to its default first.
    let init = &Drums::presets()[0];
    e.load_preset(init);
    assert_eq!(e.param(fx_id(0, fx::PUNCH)), 0.0);
    assert_eq!(e.param(kit::REVERB_SIZE), 45.0);
}

/// A 5 kHz tone at `db` dBFS through drum 1's Distortion at `drive` dB,
/// fed straight into its bus: the power off the tone's harmonics (aliases
/// folded back from above Nyquist), against the tone's, dB. One second of
/// 5 kHz at 44.1 kHz is a whole number of cycles of every harmonic and
/// every alias, so projecting on the harmonics separates them exactly.
fn dist_alias_db(drive: f32, db_in: f32, hz: f64) -> f32 {
    let rate = 44_100u32;
    let mut fx = drum_fx::KitFx::new(rate);
    fx.prepare(rate);
    fx.set_param(fx_id(0, fx::DISTORTION), drive);
    let n = rate as usize;
    let settle = rate as usize / 4;
    let amp = 10f64.powf(f64::from(db_in) / 20.0);
    let mut out = vec![0f32; n + settle];
    let mut right = vec![0f32; n + settle];
    let mut at = 0;
    while at < n + settle {
        let end = (at + drum_fx::CHUNK).min(n + settle);
        fx.begin_chunk(end - at);
        let bus = fx
            .bus(0)
            .expect("a drum with Distortion up runs through its bus");
        for i in at..end {
            let t = i as f64 / f64::from(rate);
            bus[i - at] = (amp * (std::f64::consts::TAU * hz * t).sin()) as f32;
        }
        fx.end_chunk(at, &mut out[at..end], &mut right[at..end]);
        at = end;
    }
    let y: Vec<f64> = out[settle..].iter().map(|&v| f64::from(v)).collect();
    let total: f64 = y.iter().map(|v| v * v).sum::<f64>() / n as f64;
    let mean = y.iter().sum::<f64>() / n as f64;
    let mut harmonics = mean * mean;
    let mut tone = 0.0;
    let mut k = 1;
    while k as f64 * hz < f64::from(rate) / 2.0 {
        let w = std::f64::consts::TAU * k as f64 * hz / f64::from(rate);
        let (mut c, mut s) = (0.0, 0.0);
        for (i, v) in y.iter().enumerate() {
            c += v * (w * i as f64).cos();
            s += v * (w * i as f64).sin();
        }
        let p = 2.0 * (c * c + s * s) / (n as f64 * n as f64);
        if k == 1 {
            tone = p;
        }
        harmonics += p;
        k += 1;
    }
    (10.0 * ((total - harmonics).max(1e-30) / tone).log10()) as f32
}

/// The Distortion runs at the host's rate with first-order antialiasing
/// only: its knee is smooth (the curve's slope reaches 0 where it flattens),
/// so a low drum's harmonics fall away fast and fold back little even at
/// full drive, while a high tone driven hard folds back as fizz. These
/// bounds hold today's numbers (a dB or two of room), so a change that
/// makes them worse shows. Measured 19 Nov: Distortion 12 dB, 997 Hz
/// -77.7 dB, 4999 Hz -40.3; 24 dB, 101 Hz -98.0, 997 Hz -52.8; 48 dB,
/// 101 Hz -46.1, 997 Hz -26.7 (all at -6 dBFS).
#[test]
fn distortion_folds_back_little_on_low_drums() {
    for (drive, hz, want) in [
        (6.0, 4999.0, -120.0),
        (12.0, 997.0, -75.0),
        (12.0, 4999.0, -38.0),
        (24.0, 101.0, -95.0),
        (24.0, 997.0, -50.0),
        (48.0, 101.0, -44.0),
        (48.0, 997.0, -25.0),
    ] {
        let a = dist_alias_db(drive, -6.0, hz);
        assert!(
            a < want,
            "Distortion {drive} dB, {hz} Hz at -6 dBFS: aliases {a:.1} dB, want under {want}"
        );
    }
}
