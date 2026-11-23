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
            let v = Voice {
                stereo: is_stereo,
                pick_l,
                pick_r,
                start: 5,
                end: 40,
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
                        "{pick:?} stereo {is_stereo} at {i} + {t:#x}"
                    );
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
            let v = Voice {
                stereo: is_stereo,
                pick_l,
                pick_r,
                start: 5,
                end: 40,
                ..Voice::default()
            };
            for i in 0..frames as i64 {
                for &t in &fracs {
                    let pos = (i << 32) | t as i64;
                    let (a, b) = (v.read2(ints, pos), v.read2(floats, pos));
                    assert_eq!(
                        (a.0.to_bits(), a.1.to_bits()),
                        (b.0.to_bits(), b.1.to_bits()),
                        "{pick:?} stereo {is_stereo} at {i} + {t:#x}"
                    );
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
                    render_voice_as::<false, false>(&mut b, rate, &mut bl, &mut br);
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
    let not: [Change; 10] = [
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
