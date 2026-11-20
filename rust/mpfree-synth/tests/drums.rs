//! Behaviour tests for the drum synth through the mpfree-node interface.

use mpfree_node::{check_params, Ctx, Node, NodeEvent, Timed, MAX_BLOCK};
use mpfree_synth::drum::hit_shape;
use mpfree_synth::drum_params::{
    field, model, pad_id, BASE_NOTE, CURRENT, LEVEL, METER_VOICES, MODELS, PADS, PARAMS, VOICES,
};
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

/// A factory kit by name: its pads play the Current voices.
fn kit(name: &str) -> Drums {
    let mut d = Drums::new(RATE);
    let k = Drums::presets()
        .iter()
        .find(|k| k.name == name)
        .unwrap_or_else(|| panic!("no kit {name}"));
    d.load_preset(k);
    d
}

fn hit(pad: u8, velocity: u8) -> NodeEvent {
    NodeEvent::NoteOn {
        note: 36 + pad,
        velocity,
    }
}
fn peak(x: &[f32]) -> f32 {
    x.iter().fold(0.0, |a, &v| a.max(v.abs()))
}
fn db(x: f32) -> f32 {
    20.0 * x.max(1e-12).log10()
}
fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
}

#[test]
fn parameters_and_kits_are_valid() {
    check_params(&PARAMS).unwrap();
    // Each pad's fields, then the kit's Level, Base note, Voices and the
    // voices meter.
    assert_eq!(PARAMS.len(), PADS * (field::COUNT + field::MORE) + 4);
    for kit in Drums::presets() {
        for &(id, v) in kit.values {
            // The voices' settings, or the drums' effects and sends.
            let p = mpfree_synth::drum_fx::ALL
                .iter()
                .find(|p| p.id == id)
                .unwrap_or_else(|| panic!("{}: no param {id}", kit.name));
            assert!(
                v >= p.min && v <= p.max,
                "{}: {} = {v} out of range",
                kit.name,
                p.name
            );
        }
    }
    // Every model is in some factory kit, and every factory kit plays
    // the Current voices.
    let kits: Vec<Drums> = Drums::presets().iter().map(|k| kit(k.name)).collect();
    for (m, name) in MODELS.iter().enumerate() {
        assert!(
            kits.iter()
                .any(|d| (0..PADS).any(|p| d.param(pad_id(p, field::MODEL)) as usize == m)),
            "{name} in no factory kit"
        );
    }
    assert!(kits.iter().all(|d| d.param(VOICES) == CURRENT));
    // A kit saved before the Voices setting (and a new engine) plays the
    // Classic voices, as it did.
    assert_eq!(Drums::new(RATE).param(VOICES), 0.0);
}

#[test]
fn same_output_for_any_block_size() {
    let mut ev = Vec::new();
    for k in 0..60u64 {
        ev.push((
            k * 811 + 3,
            hit((k * 7 % 16) as u8, 40 + (k * 13 % 88) as u8),
        ));
    }
    ev.push((
        9_000,
        NodeEvent::Param {
            id: pad_id(0, field::DECAY),
            value: 900.0,
        },
    ));
    ev.push((
        20_000,
        NodeEvent::Param {
            id: LEVEL,
            value: -6.0,
        },
    ));
    ev.push((30_000, NodeEvent::AllOff));
    ev.sort_by_key(|e| e.0);
    let frames = 60_000;
    let (rl, rr) = render(&mut Drums::new(RATE), frames, &ev, |_| 128);
    assert!(peak(&rl) > 0.05);
    for size in [1usize, 7, 16, 17, 64, 1000, 8192] {
        let (l, r) = render(&mut Drums::new(RATE), frames, &ev, |_| size);
        assert!(l == rl && r == rr, "block size {size} differs");
    }
    let mut seed = 5u64;
    let (l, r) = render(&mut Drums::new(RATE), frames, &ev, |_| {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        1 + (seed >> 33) as usize % 700
    });
    assert!(l == rl && r == rr, "random block sizes differ");
}

#[test]
fn every_pad_and_kit_plays_at_a_sane_level_and_ends() {
    for kit in Drums::presets() {
        for pad in 0..PADS as u8 {
            let mut d = Drums::new(RATE);
            d.load_preset(kit);
            let (l, r) = render(&mut d, RATE as usize * 6, &[(0, hit(pad, 127))], |_| 256);
            let pk = db(peak(&l).max(peak(&r)));
            assert!(
                (-26.0..=-2.5).contains(&pk),
                "{} pad {}: {pk:.1} dBFS",
                kit.name,
                pad + 1
            );
            assert_eq!(
                d.active_voices(),
                0,
                "{} pad {} still sounding",
                kit.name,
                pad + 1
            );
            let tail = &l[l.len() - 4800..];
            assert!(peak(tail) < 1e-4);
        }
    }
}

#[test]
fn instruments_add_to_the_buffer() {
    let mut d = Drums::new(RATE);
    let mut l = vec![0.5f32; 64];
    let mut r = vec![0.5f32; 64];
    d.process(&Ctx::new(RATE, 0), &[], &mut l, &mut r);
    assert!(l.iter().chain(&r).all(|&x| x == 0.5));
}

#[test]
fn the_kick_sweeps_down_to_its_tuned_pitch() {
    let mut d = Drums::new(RATE);
    let (l, _) = render(&mut d, RATE as usize / 2, &[(0, hit(0, 127))], |_| 128);
    // Zero crossings between 150 and 350 ms: about 50 Hz.
    let s = &l[RATE as usize * 15 / 100..RATE as usize * 35 / 100];
    let z = s
        .windows(2)
        .filter(|w| (w[0] < 0.0) != (w[1] < 0.0))
        .count() as f32
        / 2.0
        / 0.2;
    assert!((44.0..=58.0).contains(&z), "kick at {z} Hz");
    // And its first 10 ms is well above that (the sweep).
    let s = &l[..RATE as usize / 100];
    let z0 = s
        .windows(2)
        .filter(|w| (w[0] < 0.0) != (w[1] < 0.0))
        .count() as f32
        / 2.0
        / 0.01;
    assert!(z0 > 120.0, "sweep start {z0} Hz");
}

#[test]
fn tune_moves_the_pitch() {
    let mut a = Drums::new(RATE);
    a.set_param(pad_id(6, field::SWEEP), 0.0);
    let mut b = Drums::new(RATE);
    b.set_param(pad_id(6, field::SWEEP), 0.0);
    b.set_param(pad_id(6, field::TUNE), 12.0);
    let cross = |x: &[f32]| {
        x.windows(2)
            .filter(|w| (w[0] < 0.0) != (w[1] < 0.0))
            .count() as f32
    };
    let (la, _) = render(&mut a, 9600, &[(0, hit(6, 127))], |_| 128);
    let (lb, _) = render(&mut b, 9600, &[(0, hit(6, 127))], |_| 128);
    let ratio = cross(&lb[480..]) / cross(&la[480..]);
    assert!((1.85..=2.15).contains(&ratio), "an octave up gave {ratio}");
}

#[test]
fn a_closed_hat_chokes_the_open_hat() {
    let open = model::OPEN_HAT;
    let mut d = Drums::new(RATE);
    assert_eq!(d.param(pad_id(4, field::MODEL)) as u8, open);
    // The open hat alone rings past 200 ms.
    let (alone, _) = render(&mut d, RATE as usize / 2, &[(0, hit(4, 127))], |_| 128);
    let w = RATE as usize / 5;
    assert!(rms(&alone[w..w + 2400]) > 1e-3);
    // Choked by the closed hat at 100 ms: by 200 ms only the closed hat's
    // short tail is left, and both are gone soon after.
    let mut d = Drums::new(RATE);
    d.set_param(pad_id(3, field::DECAY), 20.0);
    let (l, _) = render(
        &mut d,
        RATE as usize / 2,
        &[(0, hit(4, 127)), (4800, hit(3, 127))],
        |_| 128,
    );
    assert!(
        rms(&l[w..w + 2400]) < 1e-4,
        "open hat still ringing: {}",
        rms(&l[w..w + 2400])
    );
    // With choke Off the open hat rings on.
    let mut d = Drums::new(RATE);
    d.set_param(pad_id(3, field::CHOKE), 0.0);
    d.set_param(pad_id(3, field::DECAY), 20.0);
    let (l, _) = render(
        &mut d,
        RATE as usize / 2,
        &[(0, hit(4, 127)), (4800, hit(3, 127))],
        |_| 128,
    );
    assert!(rms(&l[w..w + 2400]) > 1e-3);
}

#[test]
fn a_retrigger_fades_the_last_hit_without_piling_up() {
    let mut d = Drums::new(RATE);
    let ev: Vec<_> = (0..40u64).map(|k| (k * 600, hit(13, 127))).collect();
    let (l, _) = render(&mut d, 24_100, &ev, |_| 64);
    assert!(d.active_voices() <= 2, "{} voices", d.active_voices());
    assert!(d.param(METER_VOICES) <= 2.0);
    // Fading, not cutting: no sample jumps by more than a hit's own swing.
    let jump = l
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0, f32::max);
    assert!(jump < 0.3, "jump {jump}");
}

#[test]
fn notes_play_pads_from_the_base_note() {
    let mut d = Drums::new(RATE);
    assert_eq!(d.pad_of(36), Some(0));
    assert_eq!(d.pad_of(51), Some(15));
    assert_eq!(d.pad_of(35), None);
    assert_eq!(d.pad_of(52), None);
    let (l, _) = render(
        &mut d,
        4800,
        &[(
            0,
            NodeEvent::NoteOn {
                note: 60,
                velocity: 127,
            },
        )],
        |_| 128,
    );
    assert_eq!(peak(&l), 0.0);
    d.set_param(BASE_NOTE, 60.0);
    assert_eq!(d.pad_of(60), Some(0));
    assert_eq!(d.pad_name(0), "Kick");
    let (l, _) = render(
        &mut d,
        4800,
        &[(
            0,
            NodeEvent::NoteOn {
                note: 60,
                velocity: 127,
            },
        )],
        |_| 128,
    );
    assert!(peak(&l) > 0.1);
}

#[test]
fn velocity_sets_level_and_tone() {
    let level = |v: u8| {
        let (l, _) = render(&mut Drums::new(RATE), 9600, &[(0, hit(1, v))], |_| 128);
        peak(&l)
    };
    let (hi, lo) = (level(127), level(40));
    assert!(db(hi) - db(lo) > 6.0, "{} vs {}", db(hi), db(lo));
    // Vel > level at 0: every velocity is as loud.
    let mut d = Drums::new(RATE);
    d.set_param(pad_id(0, field::VEL_LEVEL), 0.0);
    d.set_param(pad_id(0, field::VEL_TONE), 0.0);
    let (a, _) = render(&mut d, 4800, &[(0, hit(0, 127))], |_| 128);
    let mut d = Drums::new(RATE);
    d.set_param(pad_id(0, field::VEL_LEVEL), 0.0);
    d.set_param(pad_id(0, field::VEL_TONE), 0.0);
    let (b, _) = render(&mut d, 4800, &[(0, hit(0, 20))], |_| 128);
    assert!((peak(&a) - peak(&b)).abs() < 1e-3);
}

#[test]
fn all_off_fades_every_hit() {
    let mut d = Drums::new(RATE);
    let ev: Vec<_> = (0..16u8)
        .map(|p| (0u64, hit(p, 127)))
        .chain([(4800, NodeEvent::AllOff)])
        .collect();
    let (l, _) = render(&mut d, 9600, &ev, |_| 128);
    assert!(peak(&l[4800 + 1500..]) < 1e-6);
    assert_eq!(d.active_voices(), 0);
}

#[test]
fn extreme_settings_stay_finite() {
    for p in PARAMS.iter().filter(|p| !p.readonly) {
        for v in [p.min, p.max, f32::NAN, f32::INFINITY] {
            let mut d = Drums::new(RATE);
            d.set_param(p.id, v);
            let ev: Vec<_> = (0..16u8).map(|k| (k as u64 * 10, hit(k, 127))).collect();
            let (l, r) = render(&mut d, 6000, &ev, |_| 333);
            assert!(
                l.iter().chain(&r).all(|x| x.is_finite()),
                "{} {} = {v}",
                p.group,
                p.name
            );
            assert!(peak(&l) < 4.0, "{} {} = {v}: {}", p.group, p.name, peak(&l));
        }
    }
    // Every pad on every model at its loudest.
    for (m, name) in MODELS.iter().enumerate() {
        let mut d = Drums::new(RATE);
        for p in 0..PADS {
            d.set_param(pad_id(p, field::MODEL), m as f32);
            for (f, v) in [
                (field::DRIVE, 100.0),
                (field::LEVEL, 6.0),
                (field::TONE, 100.0),
                (field::SNAP, 100.0),
                (field::NOISE, 100.0),
                (field::TUNE, 24.0),
                (field::SWEEP, 48.0),
            ] {
                d.set_param(pad_id(p, f), v);
            }
        }
        d.set_param(LEVEL, 6.0);
        let ev: Vec<_> = (0..16u8).map(|k| (0, hit(k, 127))).collect();
        let (l, _) = render(&mut d, 6000, &ev, |_| 128);
        assert!(l.iter().all(|x| x.is_finite()), "{name}");
    }
}

#[test]
fn the_hit_shape_drawn_is_the_hit_played() {
    let mut d = Drums::new(RATE);
    // No drive, so the body's envelope is the sound's.
    d.set_param(pad_id(0, field::DRIVE), 0.0);
    let shape = hit_shape(|id| d.param(id), 0, RATE);
    assert_eq!(shape.model, "Kick");
    assert!((shape.hz - 50.0).abs() < 0.01, "{shape:?}");
    assert!(shape.body.0 > 0.0 && shape.sweep.0 > 0.0 && shape.bursts.is_none());
    let (l, _) = render(&mut d, RATE as usize, &[(0, hit(0, 127))], |_| 128);
    // The body's decay: past the click and the noise, its level falls as
    // drawn.
    let win = |t: f32| rms(&l[(t * RATE as f32) as usize..][..RATE as usize / 25]);
    let got = db(win(0.1)) - db(win(0.3));
    let want = -db(shape.body.1.powf(0.2 * RATE as f32));
    assert!(
        (got - want).abs() < 1.5,
        "falls {got:.1} dB, drawn {want:.1}"
    );
    // The sweep: the pitch 20 to 40 ms in is the drawn one, within 10 %.
    let pitch_at =
        |t: f32| shape.hz * (shape.sweep.0 * shape.sweep.1.powf(t * RATE as f32) / 12.0).exp2();
    let drawn = (0..20)
        .map(|i| pitch_at(0.02 + i as f32 * 0.001))
        .sum::<f32>()
        / 20.0;
    let s = &l[RATE as usize / 50..RATE as usize / 25];
    let crossings = s
        .windows(2)
        .filter(|w| (w[0] < 0.0) != (w[1] < 0.0))
        .count() as f32;
    let played = crossings / 2.0 / 0.02;
    assert!(
        (played - drawn).abs() < 0.1 * drawn + 25.0,
        "{played} Hz played, {drawn} Hz drawn"
    );
    // Knobs move it: twice the decay, half the fall per frame in dB.
    d.set_param(pad_id(0, field::DECAY), 900.0);
    let longer = hit_shape(|id| d.param(id), 0, RATE);
    assert!((longer.body.1.ln() * 2.0 - shape.body.1.ln()).abs() < 1e-6);
    // Hats have no body to bend; a clap has its bursts.
    let hat = hit_shape(|id| d.param(id), 3, RATE);
    assert_eq!((hat.model, hat.hz, hat.body.0), ("Closed hat", 0.0, 0.0));
    let clap = hit_shape(|id| d.param(id), 2, RATE);
    assert_eq!(clap.bursts.map(|b| b.count), Some(4));
}

#[test]
fn current_voices_are_the_same_for_any_block_size() {
    // Every pad of a Current kit, gated pads let go, a choke, a knob
    // turned and all notes off: the same samples in any blocks.
    let mut ev = Vec::new();
    for k in 0..64u64 {
        let pad = (k * 5 % 16) as u8;
        ev.push((k * 701 + 5, hit(pad, 30 + (k * 29 % 98) as u8)));
        ev.push((k * 701 + 300, NodeEvent::NoteOff { note: 36 + pad }));
    }
    ev.push((
        12_000,
        NodeEvent::Param {
            id: pad_id(13, field::DECAY),
            value: 400.0,
        },
    ));
    ev.push((40_000, NodeEvent::AllOff));
    ev.sort_by_key(|e| e.0);
    let frames = 50_000;
    let make = || {
        let mut d = kit("Init Kit");
        // Pad 5 plays only while held.
        d.set_param(pad_id(4, field::PLAYS), 1.0);
        d
    };
    let (rl, rr) = render(&mut make(), frames, &ev, |_| 128);
    assert!(peak(&rl) > 0.05);
    for size in [1usize, 16, 17, 333, 8192] {
        let (l, r) = render(&mut make(), frames, &ev, |_| size);
        assert!(l == rl && r == rr, "block size {size} differs");
    }
}

#[test]
fn a_current_choke_is_quick() {
    // The open hat choked at 100 ms by a silent pad in its group (the
    // closed hat at no level): gone within the choke's 5 ms fade. (How
    // clean the fade is, the lab's D6 row measures on a ringing kick.)
    let mut d = kit("Init Kit");
    d.set_param(pad_id(3, field::LEVEL), -60.0);
    let (l, _) = render(
        &mut d,
        RATE as usize / 2,
        &[(0, hit(4, 127)), (4800, hit(3, 127))],
        |_| 64,
    );
    assert!(rms(&l[4000..4800]) > 1e-3);
    let after = rms(&l[4800 + 300..4800 + 2400]);
    assert!(after < 1e-6, "{after}");
}

#[test]
fn a_hard_click_ends_with_its_hit() {
    // A click louder than 1 (Click at 100, a full-velocity hit with its
    // accent) once kept its hit alive for ever, at a level under hearing:
    // every Current drum with a click ends with its sound.
    for m in [
        model::TOM,
        model::RIM,
        model::CONGA,
        model::KICK,
        model::WOODBLOCK,
    ] {
        let mut d = kit("Init Kit");
        d.set_param(pad_id(6, field::MODEL), f32::from(m));
        d.set_param(pad_id(6, field::DECAY), 300.0);
        d.set_param(pad_id(6, field::SNAP), 100.0);
        d.set_param(pad_id(6, field::ACCENT), 100.0);
        render(&mut d, RATE as usize * 2, &[(0, hit(6, 127))], |_| 256);
        assert_eq!(d.active_voices(), 0, "{}", MODELS[usize::from(m)]);
    }
}

#[test]
fn a_gated_pad_stops_when_let_go() {
    let ring = |plays: f32| {
        let mut d = kit("Init Kit");
        // The cymbal (pad 11), long, let go at 100 ms.
        d.set_param(pad_id(10, field::PLAYS), plays);
        let (l, _) = render(
            &mut d,
            RATE as usize,
            &[(0, hit(10, 127)), (4800, NodeEvent::NoteOff { note: 46 })],
            |_| 128,
        );
        (
            rms(&l[RATE as usize / 5..RATE as usize / 4]),
            d.active_voices(),
        )
    };
    let (held, voices) = ring(1.0);
    assert!(held < 1e-5, "still ringing at {held}");
    assert_eq!(voices, 0);
    // One-shot rings on.
    assert!(ring(0.0).0 > 1e-3);
}

#[test]
fn hits_vary_unless_variation_is_off() {
    let two = |variation: f32| {
        let mut d = kit("Init Kit");
        d.set_param(pad_id(0, field::VARIATION), variation);
        let gap = RATE as u64;
        let (l, _) = render(
            &mut d,
            2 * gap as usize,
            &[(0, hit(0, 100)), (gap, hit(0, 100))],
            |_| 128,
        );
        let (a, b) = l.split_at(gap as usize);
        (db(rms(&a[..4800])) - db(rms(&b[..4800]))).abs()
    };
    let step = two(50.0);
    assert!((0.1..=1.2).contains(&step), "{step} dB");
    assert!(two(0.0) < 1e-3);
}

#[test]
fn a_new_drum_in_a_classic_kit_plays() {
    // The 808 kick on pad 1 of the Classic Init kit.
    let mut d = Drums::new(RATE);
    d.set_param(pad_id(0, field::MODEL), f32::from(model::KICK_808));
    let (l, _) = render(&mut d, RATE as usize, &[(0, hit(0, 127))], |_| 128);
    assert!(peak(&l) > 0.1);
    assert_eq!(d.pad_name(0), "808 kick");
}

#[test]
fn current_retriggers_fade_without_piling_up() {
    let mut d = kit("Boom Kit");
    let ev: Vec<_> = (0..40u64).map(|k| (k * 600, hit(13, 127))).collect();
    let (l, _) = render(&mut d, 24_100, &ev, |_| 64);
    assert!(d.active_voices() <= 2, "{} voices", d.active_voices());
    let jump = l
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0, f32::max);
    assert!(jump < 0.5, "jump {jump}");
}
