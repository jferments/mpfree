use super::*;

/// The direct read gives what the clamped one does, bit for bit, at
/// every kind of position: whole and fractional frames, the region's
/// first and last frames and just outside them, for mono and stereo
/// and every channel pick.
#[test]
fn the_direct_read_matches_the_clamped_one() {
    let frames = 64usize;
    let stereo: Vec<f32> = (0..frames * 2)
        .map(|i| ((i * 7919) % 1000) as f32 / 500.0 - 1.0)
        .collect();
    let mono: Vec<f32> = (0..frames)
        .map(|i| ((i * 104_729) % 977) as f32 / 488.5 - 1.0)
        .collect();
    let fracs = [0u64, 1, 1 << 31, 0x1234_5678, u64::from(u32::MAX)];
    for (data, is_stereo) in [(&stereo, true), (&mono, false)] {
        for pick in [
            ChannelPick::Stereo,
            ChannelPick::MonoSum,
            ChannelPick::Left,
            ChannelPick::Right,
        ] {
            let (pick_l, pick_r) = picks(is_stereo, pick);
            for (vintage, step) in [(false, ONE), (false, ONE / 3), (true, ONE / 3)] {
                let v = Voice {
                    stereo: is_stereo,
                    pick_l,
                    pick_r,
                    start: 5,
                    end: 40,
                    vintage,
                    step,
                    ..Voice::default()
                };
                for i in 0..frames as i64 {
                    for &t in &fracs {
                        let pos = (i << 32) | t as i64;
                        let (l, r) = v.read2(data, pos);
                        let (wl, wr) = (v.read(data, pos, pick_l), v.read(data, pos, pick_r));
                        assert_eq!(
                        (l.to_bits(), r.to_bits()),
                        (wl.to_bits(), wr.to_bits()),
                        "{pick:?} stereo {is_stereo} vintage {vintage} step {step:#x} at {i} + {t:#x}"
                    );
                    }
                }
            }
        }
    }
}

/// A sound held as 16 bits reads as its floats do, bit for bit: the
/// direct read and the clamped one, mono and stereo, every pick, at
/// whole and fractional frames and the region's edges.
#[test]
fn a_16_bit_sound_reads_as_its_floats_do() {
    let frames = 64usize;
    let ints: Vec<i16> = (0..frames * 2)
        .map(|i| (((i * 7919) % 1000) as i32 * 65 - 32_500) as i16)
        .collect();
    let floats: Vec<f32> = ints.iter().map(|&k| f32::from(k) / 32_768.0).collect();
    let fracs = [0u64, 1, 1 << 31, 0x1234_5678, u64::from(u32::MAX)];
    for is_stereo in [true, false] {
        let n = if is_stereo { frames * 2 } else { frames };
        let (ints, floats) = (&ints[..n], &floats[..n]);
        for pick in [
            ChannelPick::Stereo,
            ChannelPick::MonoSum,
            ChannelPick::Left,
            ChannelPick::Right,
        ] {
            let (pick_l, pick_r) = picks(is_stereo, pick);
            for (vintage, step) in [(false, ONE), (false, ONE / 3), (true, ONE / 3)] {
                let v = Voice {
                    stereo: is_stereo,
                    pick_l,
                    pick_r,
                    start: 5,
                    end: 40,
                    vintage,
                    step,
                    ..Voice::default()
                };
                for i in 0..frames as i64 {
                    for &t in &fracs {
                        let pos = (i << 32) | t as i64;
                        let (a, b) = (v.read2(ints, pos), v.read2(floats, pos));
                        assert_eq!(
                        (a.0.to_bits(), a.1.to_bits()),
                        (b.0.to_bits(), b.1.to_bits()),
                        "{pick:?} stereo {is_stereo} vintage {vintage} step {step:#x} at {i} + {t:#x}"
                    );
                    }
                }
            }
        }
    }
}

/// Pressure let go lands on zero: its smoothing never creeps toward
/// zero through the denormals, which cost far more than a normal
/// number (7 times, for 16 pads held after pressure).
#[test]
fn pressure_let_go_lands_on_exact_zero() {
    let mut v = Voice {
        route: PressureRoute {
            target: PressureTarget::Level,
            depth: 0.5,
        },
        press: 1.0,
        press_target: 0.0,
        press_coef: 1.0 - (-1.0 / (PRESSURE_SMOOTH_S * 44_100.0)).exp(),
        press_gain: 1.0,
        ..Voice::default()
    };
    for frame in 0..44_100 {
        follow_pressure(&mut v);
        assert!(
            v.press == 0.0 || v.press.abs() >= f32::MIN_POSITIVE,
            "a denormal at frame {frame}"
        );
    }
    assert_eq!(v.press, 0.0);
}

/// A plain voice gives the same samples, bit for bit, from
/// `render_plain` as from the full loop: stereo and mono, from the
/// start and from part way in, at full and at a held sustain level,
/// running out within a block and exactly at its end; and it ends
/// (or plays on) on the same frame.
#[test]
fn a_plain_voice_renders_as_the_full_loop_does() {
    let rate = 44_100.0;
    let frames = 300usize;
    let stereo: Arc<[f32]> = (0..frames * 2)
        .map(|i| ((i * 7919) % 1000) as f32 / 500.0 - 1.0)
        .collect::<Vec<_>>()
        .into();
    let mono: Arc<[f32]> = (0..frames)
        .map(|i| ((i * 104_729) % 977) as f32 / 488.5 - 1.0)
        .collect::<Vec<_>>()
        .into();
    // And the same held as 16 bits.
    let stereo16: Arc<[i16]> = (0..frames * 2)
        .map(|i| (((i * 7919) % 1000) as i32 * 65 - 32_500) as i16)
        .collect::<Vec<_>>()
        .into();
    let mono16: Arc<[i16]> = (0..frames)
        .map(|i| (((i * 104_729) % 977) as i32 * 67 - 32_700) as i16)
        .collect::<Vec<_>>()
        .into();
    let sounds = [
        (Pcm::F32(stereo), true),
        (Pcm::F32(mono), false),
        (Pcm::I16(stereo16), true),
        (Pcm::I16(mono16), false),
    ];
    for (data, is_stereo) in &sounds {
        let is_stereo = *is_stereo;
        for sustain in [1.0f32, 0.7] {
            for (start, end, from) in [(0i64, 300i64, 0i64), (10, 256, 30), (3, 131, 3)] {
                let mut amp = Envelope::start(
                    &Ahdsr {
                        sustain,
                        ..Ahdsr::OPEN
                    },
                    rate,
                );
                for _ in 0..4 {
                    amp.next();
                }
                assert!(amp.is_steady());
                let (pick_l, pick_r) = picks(is_stereo, ChannelPick::Stereo);
                let v = Voice {
                    active: true,
                    data: Some(data.clone()),
                    stereo: is_stereo,
                    pick_l,
                    pick_r,
                    pos: from << 32,
                    step: ONE,
                    start,
                    end,
                    gain_l: 0.83,
                    gain_r: -0.41,
                    amp: Some(amp),
                    ..Voice::default()
                };
                assert!(plain_now(&v));
                let (mut a, mut b) = (v.clone(), v);
                for block in 0..4 {
                    let (mut al, mut ar) = ([0.25f32; 128], [-0.5f32; 128]);
                    let (mut bl, mut br) = (al, ar);
                    render_plain(&mut a, &mut al, &mut ar);
                    render_voice_as::<false, false, false>(&mut b, rate, &mut bl, &mut br);
                    let what = format!("stereo {is_stereo}, sustain {sustain}, {start}..{end} from {from}, block {block}");
                    assert_eq!(al.map(f32::to_bits), bl.map(f32::to_bits), "{what}");
                    assert_eq!(ar.map(f32::to_bits), br.map(f32::to_bits), "{what}");
                    assert_eq!((a.active, a.pos), (b.active, b.pos), "{what}");
                    assert_eq!(
                        (a.last_l.to_bits(), a.last_r.to_bits()),
                        (b.last_l.to_bits(), b.last_r.to_bits()),
                        "{what}"
                    );
                    if !a.active {
                        break;
                    }
                }
            }
        }
    }
}

/// What isn't plain goes through the full loop: a filter, a fade, a
/// loop, pitch, a blend between frames, a kill fade, a stolen voice's
/// tail, an envelope still moving, a channel pick.
#[test]
fn only_plain_voices_take_the_plain_path() {
    let mut amp = Envelope::start(&Ahdsr::OPEN, 44_100.0);
    for _ in 0..4 {
        amp.next();
    }
    let plain = Voice {
        active: true,
        stereo: true,
        pick_l: 0,
        pick_r: 1,
        step: ONE,
        end: 100,
        amp: Some(amp),
        ..Voice::default()
    };
    assert!(plain_now(&plain));
    type Change = (&'static str, fn(&mut Voice));
    let not: [Change; 12] = [
        ("filter", |v| v.filter = Some(OPEN_FILTER)),
        ("fade", |v| {
            v.fade_out = Some(Fade {
                frames: 10,
                ..Fade::NONE
            })
        }),
        ("loop", |v| v.looping = true),
        ("pitch", |v| v.step = ONE + 1),
        ("blend", |v| v.pos = 1),
        ("kill", |v| v.kill_left = 3),
        ("tail", |v| v.tail_left = 3),
        ("attack", |v| {
            v.amp = Some(Envelope::start(
                &Ahdsr {
                    attack_ms: 5.0,
                    ..Ahdsr::OPEN
                },
                44_100.0,
            ))
        }),
        ("release", |v| v.amp.as_mut().unwrap().release()),
        ("left only", |v| v.pick_r = 0),
        ("heard the 8-point read", |v| {
            v.last_read = Some(Heard::Near)
        }),
        ("heard the band-limited read", |v| {
            v.last_read = Some(Heard::Band)
        }),
    ];
    for (what, change) in not {
        let mut v = plain.clone();
        change(&mut v);
        assert!(!plain_now(&v), "{what}");
    }
}

#[test]
fn the_fraction_is_the_low_half_over_two_to_the_32() {
    for pos in [
        0i64,
        1,
        (7 << 32) | 0x8000_0000,
        -1,
        (3 << 32) | 0xFFFF_FFFF,
        i64::MIN + 12345,
    ] {
        assert_eq!(frac(pos), (pos & (ONE - 1)) as f32 / ONE as f32, "{pos:#x}");
    }
}

/// A mono sine of `hz` at 44.1 kHz played by a voice that starts at
/// `step0` on a whole frame and changes to `step1` at frame `at` (a block
/// boundary), rendered in 32-frame blocks. Returns, frame by frame, the
/// voice's level (dB) and phase (degrees) against the ideal tone at the
/// positions it read: the output times the ideal tone's conjugate,
/// averaged over a 2 ms Hann window.
fn level_and_phase(hz: f64, step0: i64, step1: i64, at: usize, frames: usize) -> Vec<(f64, f64)> {
    const RATE: f64 = 44_100.0;
    let len = 60_000usize;
    let sound: Vec<f32> = (0..len)
        .map(|k| (0.5 * (std::f64::consts::TAU * hz * k as f64 / RATE).sin()) as f32)
        .collect();
    let mut amp = Envelope::start(&Ahdsr::OPEN, RATE as f32);
    for _ in 0..4 {
        amp.next();
    }
    let (pick_l, pick_r) = picks(false, ChannelPick::Stereo);
    let mut v = Voice {
        active: true,
        data: Some(Pcm::F32(sound.into())),
        stereo: false,
        pick_l,
        pick_r,
        pos: 1000 << 32,
        step: step0,
        start: 0,
        end: len as i64,
        gain_l: 1.0,
        gain_r: 1.0,
        amp: Some(amp),
        bl_above: BlAbove(CLEAN_ABOVE),
        ..Voice::default()
    };
    let (mut out, mut phase) = (Vec::new(), Vec::new());
    while out.len() < frames {
        if out.len() >= at {
            v.step = step1;
        }
        let (pos0, step) = (v.pos, v.step);
        let (mut l, mut r) = ([0.0f32; 32], [0.0f32; 32]);
        render_voice(&mut v, RATE as f32, &mut l, &mut r);
        for (j, &y) in l.iter().enumerate() {
            let p = (pos0 + j as i64 * step) as f64 / ONE as f64;
            out.push(f64::from(y));
            phase.push(std::f64::consts::TAU * hz * p / RATE);
        }
    }
    let w = 88usize;
    let hann: Vec<f64> = (0..w)
        .map(|k| 0.5 - 0.5 * (std::f64::consts::TAU * (k as f64 + 0.5) / w as f64).cos())
        .collect();
    let norm: f64 = hann.iter().sum();
    (0..frames - w)
        .map(|n| {
            let (mut re, mut im) = (0.0, 0.0);
            for (k, h) in hann.iter().enumerate() {
                re += h * out[n + k] * phase[n + k].cos();
                im -= h * out[n + k] * phase[n + k].sin();
            }
            // A sine is (e^jx - e^-jx) / 2j: times e^-jx, its average is
            // 1 / 2j of its amplitude, so the gain is 2j z / 0.5.
            let (gr, gi) = (-2.0 * im / norm / 0.5, 2.0 * re / norm / 0.5);
            (20.0 * gr.hypot(gi).log10(), gi.atan2(gr).to_degrees())
        })
        .collect()
}

/// The most a voice's level (dB) and phase (degrees) move in any 1 ms
/// between frames `from` and `to`.
fn most_moved(lp: &[(f64, f64)], from: usize, to: usize) -> (f64, f64) {
    let d = 44;
    (from..to.min(lp.len() - d)).fold((0.0f64, 0.0f64), |(ml, mp), n| {
        let dl = (lp[n + d].0 - lp[n].0).abs();
        let mut dp = (lp[n + d].1 - lp[n].1).abs() % 360.0;
        if dp > 180.0 {
            dp = 360.0 - dp;
        }
        (ml.max(dl), mp.max(dp))
    })
}

/// A voice changing read mid-note fades from one read to the other: its
/// level and phase move smoothly, not in a step, as it leaves the plain
/// copy (a bend starting from its own pitch), as it crosses into the
/// band-limited read at two semitones up, and as it jumps from its own
/// pitch straight past it. Measured at 1 to 20 kHz: in any 1 ms around
/// the change, the level moves at most 0.6 dB and the phase 3 degrees
/// more than the new read moves by itself at that pitch (near the top a
/// read's level and phase move with the place between frames). Without
/// the fade, the step at 18 kHz (the 8-point read sits 2.5 dB under the
/// plain copy on a whole frame) shows as over 1.2 dB in 1 ms. The third
/// review's sweep (DSP quality step 1.4).
#[test]
fn a_voice_changing_read_moves_smoothly() {
    let st = |s: f64| (2f64.powf(s / 12.0) * ONE as f64).round() as i64;
    let at = 2048;
    let cases: [(&str, i64, i64, &[f64]); 3] = [
        (
            "plain to 8-point, a cent up",
            ONE,
            st(0.01),
            &[1e3, 5e3, 10e3, 15e3, 18e3, 20e3],
        ),
        (
            "8-point to band-limited",
            st(1.9),
            st(2.1),
            &[1e3, 5e3, 10e3, 15e3, 18e3],
        ),
        (
            "plain to band-limited",
            ONE,
            st(3.0),
            &[1e3, 5e3, 10e3, 15e3],
        ),
    ];
    for (what, step0, step1, tones) in cases {
        for &hz in tones {
            let lp = level_and_phase(hz, step0, step1, at, 4096);
            let (dl, dp) = most_moved(&lp, at - 400, at + 1200);
            let steady = level_and_phase(hz, step1, step1, at, 4096);
            let (sl, sp) = most_moved(&steady, at - 400, at + 1200);
            assert!(
                dl <= sl + 0.6 && dp <= sp + 3.0,
                "{what} at {hz} Hz: level moved {dl:.2} dB, phase {dp:.1} degrees in 1 ms (by itself {sl:.2}, {sp:.1})"
            );
        }
    }
}

/// A voice back at its sound's own speed on a whole frame after another
/// read (half speed keeps the fraction at zero at each block's start)
/// fades that read out before it turns plain.
#[test]
fn a_voice_back_at_its_own_speed_fades_before_it_turns_plain() {
    const RATE: f32 = 44_100.0;
    let sound: Vec<f32> = (0..20_000)
        .map(|k| (std::f32::consts::TAU * 15_000.0 * k as f32 / RATE).sin())
        .collect();
    let mut amp = Envelope::start(&Ahdsr::OPEN, RATE);
    for _ in 0..4 {
        amp.next();
    }
    let (pick_l, pick_r) = picks(false, ChannelPick::Stereo);
    let mut v = Voice {
        active: true,
        data: Some(Pcm::F32(sound.clone().into())),
        pick_l,
        pick_r,
        pos: 1000 << 32,
        step: ONE / 2,
        end: sound.len() as i64,
        gain_l: 1.0,
        gain_r: 1.0,
        amp: Some(amp),
        bl_above: BlAbove(CLEAN_ABOVE),
        ..Voice::default()
    };
    let block = |v: &mut Voice| {
        let (mut l, mut r) = ([0.0f32; 32], [0.0f32; 32]);
        render_voice(v, RATE, &mut l, &mut r);
        l
    };
    for _ in 0..(2 * XF_FRAMES as usize / 32) {
        block(&mut v);
    }
    assert_eq!(v.last_read, Some(Heard::Near));
    v.step = ONE;
    assert_eq!(v.pos & (ONE - 1), 0);
    assert!(!plain_now(&v));
    let at = (v.pos >> 32) as usize;
    let first = block(&mut v);
    assert_eq!(v.last_read, Some(Heard::Exact));
    assert_eq!(v.xf_left, XF_FRAMES - 32, "the fade is under way");
    assert_ne!(first[0], sound[at], "the first frame is still the old read");
    for _ in 1..(XF_FRAMES as usize / 32) {
        block(&mut v);
    }
    assert_eq!(v.xf_left, 0);
    assert!(plain_now(&v), "plain once the fade is done");
    let at = (v.pos >> 32) as usize;
    assert_eq!(block(&mut v)[..], sound[at..at + 32]);
}

/// How a voice moves through its region, for the boundary tests.
#[derive(Clone, Copy, Debug)]
enum Moves {
    Forward,
    Reverse,
    Loop,
    LoopCrossfade,
    PingPong,
}

/// A voice over `region` of a sound whose frames outside the region are
/// `outside`'s noise, moving as `moves` at `step`, Vintage or not, read
/// band-limited past two semitones up. Rendered `blocks` blocks of 64,
/// switching to `step2` (a change of read) after the first.
#[allow(clippy::too_many_arguments)]
fn render_region(
    stereo: bool,
    sixteen: bool,
    region: (i64, i64),
    outside: u64,
    moves: Moves,
    step: i64,
    step2: i64,
    vintage: bool,
    blocks: usize,
) -> Vec<f32> {
    let frames = 256usize;
    let ch = if stereo { 2 } else { 1 };
    let mut seed = outside;
    let mut noise = || {
        seed = seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        ((seed >> 40) as f32 / (1u64 << 24) as f32) * 2.0 - 1.0
    };
    let (start, end) = region;
    let floats: Vec<f32> = (0..frames * ch)
        .map(|n| {
            let f = (n / ch) as i64;
            let inside = ((n * 7919) % 1000) as f32 / 1000.0 - 0.5;
            let other = noise();
            if (start..end).contains(&f) {
                inside
            } else {
                other
            }
        })
        .collect();
    // 16-bit: the same values rounded to 16 bits.
    let data = if sixteen {
        Pcm::I16(
            floats
                .iter()
                .map(|&x| (x * 32_767.0) as i16)
                .collect::<Vec<_>>()
                .into(),
        )
    } else {
        Pcm::F32(floats.into())
    };
    let mut amp = Envelope::start(&Ahdsr::OPEN, 44_100.0);
    for _ in 0..4 {
        amp.next();
    }
    let (pick_l, pick_r) = picks(stereo, ChannelPick::Stereo);
    let len = end - start;
    let backwards = matches!(moves, Moves::Reverse);
    let mut v = Voice {
        active: true,
        data: Some(data),
        stereo,
        pick_l,
        pick_r,
        pos: if backwards {
            (end - 1) << 32
        } else {
            start << 32
        },
        step,
        start,
        end,
        backwards,
        reverse: backwards,
        looping: matches!(moves, Moves::Loop | Moves::LoopCrossfade | Moves::PingPong),
        ping_pong: matches!(moves, Moves::PingPong),
        loop_start: start + len / 4,
        loop_end: end - len / 4,
        crossfade: if matches!(moves, Moves::LoopCrossfade) {
            (len / 8).max(1)
        } else {
            0
        },
        gain_l: 1.0,
        gain_r: 1.0,
        amp: Some(amp),
        vintage,
        bl_above: BlAbove(if vintage { i64::MAX } else { CLEAN_ABOVE }),
        ..Voice::default()
    };
    if v.loop_end <= v.loop_start {
        (v.loop_start, v.loop_end) = (start, end);
    }
    let mut out = Vec::new();
    for b in 0..blocks {
        if b == 1 {
            v.step = step2;
        }
        let (mut l, mut r) = ([0.0f32; 64], [0.0f32; 64]);
        render_voice(&mut v, 44_100.0, &mut l, &mut r);
        out.extend_from_slice(&l);
        out.extend_from_slice(&r);
        if !v.active {
            break;
        }
    }
    out
}

/// A read never looks outside the region it plays: changing every frame
/// outside it changes nothing, bit for bit, for each read (the 8-point
/// read, Vintage's Hermite, the band-limited read, a crossfade between
/// reads), playing forwards, in reverse, looping, looping with a
/// crossfade and ping-pong, mono and stereo, floats and 16 bits, and for
/// regions down to a single frame. Frames past the region's edge read as
/// the edge frame (DSP quality step 1.4, third review).
#[test]
fn a_read_never_looks_outside_its_region() {
    let st = |s: f64| (2f64.powf(s / 12.0) * ONE as f64).round() as i64;
    let reads: [(&str, i64, i64, bool); 5] = [
        ("8-point", st(-5.0), st(-5.0), false),
        ("Vintage", st(-5.0), st(-5.0), true),
        ("band-limited", st(3.0), st(3.0), false),
        ("plain to 8-point", ONE, st(0.3), false),
        ("8-point to band-limited", st(1.5), st(2.5), false),
    ];
    let moves = [
        Moves::Forward,
        Moves::Reverse,
        Moves::Loop,
        Moves::LoopCrossfade,
        Moves::PingPong,
    ];
    for (stereo, sixteen) in [(false, false), (true, false), (false, true), (true, true)] {
        for region in [(40i64, 200i64), (100, 107), (100, 101)] {
            for &m in &moves {
                for &(what, step, step2, vintage) in &reads {
                    let a = render_region(stereo, sixteen, region, 1, m, step, step2, vintage, 12);
                    let b = render_region(stereo, sixteen, region, 2, m, step, step2, vintage, 12);
                    assert!(a.iter().all(|x| x.is_finite()), "{what} {m:?} {region:?}");
                    assert_eq!(
                        a.iter().map(|x| x.to_bits()).collect::<Vec<_>>(),
                        b.iter().map(|x| x.to_bits()).collect::<Vec<_>>(),
                        "{what}, {m:?}, region {region:?}, stereo {stereo}, 16-bit {sixteen}"
                    );
                }
            }
        }
    }
}

/// A one-frame region reads as that frame at any speed and place: the
/// 8-point read's weights add up to one.
#[test]
fn a_one_frame_region_reads_as_its_frame() {
    let data: Vec<f32> = (0..64).map(|i| i as f32 * 0.01 - 0.3).collect();
    for step in [ONE / 3, ONE + 12_345, ONE * 3 / 2] {
        let v = Voice {
            start: 20,
            end: 21,
            step,
            ..Voice::default()
        };
        for t in [0u32, 1, 1 << 31, 0x9876_5432] {
            let pos = (20i64 << 32) | i64::from(t);
            let (l, _) = v.read2(&data[..], pos);
            assert!(
                (l - data[20]).abs() < 1e-6,
                "step {step:#x} at {t:#x}: {l} for {}",
                data[20]
            );
        }
    }
}

/// A seamless loop (its sound repeating with the loop's length, the
/// frames past its end continuing it) plays pitched exactly as the same
/// sound unrolled does, bit for bit, across every wrap: the read sees the
/// same frames either side of the seam.
#[test]
fn a_seamless_loop_reads_as_the_unrolled_sound() {
    let period: Vec<f32> = (0..50)
        .map(|k| (k as f32 * 0.37).sin() * 0.5 + (k as f32 * 1.91).cos() * 0.2)
        .collect();
    let sound: Vec<f32> = period.iter().cycle().take(50 * 400).copied().collect();
    let pcm = Pcm::F32(sound.into());
    let mut amp = Envelope::start(&Ahdsr::OPEN, 44_100.0);
    for _ in 0..4 {
        amp.next();
    }
    let st = |s: f64| (2f64.powf(s / 12.0) * ONE as f64).round() as i64;
    for step in [st(-7.0), st(-0.5), st(1.0)] {
        let base = Voice {
            active: true,
            data: Some(pcm.clone()),
            pos: 100 << 32,
            step,
            start: 0,
            end: 50 * 400,
            gain_l: 1.0,
            gain_r: 1.0,
            amp: Some(amp),
            bl_above: BlAbove(CLEAN_ABOVE),
            ..Voice::default()
        };
        let mut looped = Voice {
            looping: true,
            loop_start: 100,
            loop_end: 250,
            ..base.clone()
        };
        let mut unrolled = base;
        for block in 0..40 {
            let (mut al, mut ar) = ([0.0f32; 64], [0.0f32; 64]);
            let (mut bl, mut br) = (al, ar);
            render_voice(&mut looped, 44_100.0, &mut al, &mut ar);
            render_voice(&mut unrolled, 44_100.0, &mut bl, &mut br);
            assert_eq!(
                al.map(f32::to_bits),
                bl.map(f32::to_bits),
                "step {step:#x}, block {block}"
            );
        }
    }
}
