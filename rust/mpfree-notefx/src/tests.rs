use super::*;

const RATE: u32 = 48_000;
const BPM: f64 = 120.0;

/// Runs `input` (in time order) through `chain` in periods of `block`
/// frames until `until`, as the sound loop does; gives what came out.
fn run(chain: &mut Chain, input: &[Ev], block: u32, until: u64) -> Vec<Ev> {
    run_with(chain, input, block, until, |_, _| {})
}

/// [`run`], calling `between(chain, period start)` before each period.
fn run_with(
    chain: &mut Chain,
    input: &[Ev],
    block: u32,
    until: u64,
    mut between: impl FnMut(&mut Chain, u64),
) -> Vec<Ev> {
    let mut got = Vec::new();
    let mut start = 0u64;
    let mut next = 0;
    let mut out = Box::new(Notes::new());
    while start < until {
        between(chain, start);
        let p = Period {
            start,
            frames: block,
            rate: RATE,
            bpm: BPM,
        };
        let end = p.end();
        let first = next;
        while next < input.len() && input[next].at < end {
            next += 1;
        }
        out.clear();
        chain.process(&p, &input[first..next], &mut out);
        for e in out.as_slice() {
            assert!(
                e.at >= start && e.at < end,
                "{e:?} left outside its period {start}..{end}"
            );
        }
        got.extend_from_slice(out.as_slice());
        start = end;
    }
    got
}

fn chain_of(fx: &[Fx]) -> Box<Chain> {
    let mut c = Box::new(Chain::new());
    for (i, f) in fx.iter().enumerate() {
        c.set(i, Some(Slot { fx: *f, on: true }));
    }
    c
}

fn ons(got: &[Ev]) -> Vec<(u64, u8, u8)> {
    got.iter()
        .filter_map(|e| match e.kind {
            Kind::On { note, velocity } => Some((e.at, note, velocity)),
            Kind::Off { .. } => None,
        })
        .collect()
}

fn offs(got: &[Ev]) -> Vec<(u64, u8)> {
    got.iter()
        .filter_map(|e| match e.kind {
            Kind::Off { note } => Some((e.at, note)),
            Kind::On { .. } => None,
        })
        .collect()
}

/// Every note that started ended, after it started, and nothing ended a
/// note that wasn't on.
fn balanced(got: &[Ev]) {
    let mut on = [0i32; 128];
    for e in got {
        match e.kind {
            Kind::On { note, .. } => on[usize::from(note)] += 1,
            Kind::Off { note } => {
                on[usize::from(note)] -= 1;
                assert!(on[usize::from(note)] >= 0, "an off before its on: {e:?}");
            }
        }
    }
    let left: Vec<usize> = (0..128).filter(|&n| on[n] != 0).collect();
    assert!(left.is_empty(), "notes left on: {left:?}");
}

fn c_major() -> Scale {
    Scale::new(0, mpfree_scale::MAJOR)
}

#[test]
fn transpose_moves_each_note_and_a_change_while_held_ends_it_where_it_went() {
    let mut c = chain_of(&[Fx::Transpose { semitones: 12 }]);
    let got = run_with(
        &mut c,
        &[
            Ev::on(10, 48, 100),
            Ev::off(2000, 48),
            Ev::on(3000, 48, 90),
            Ev::off(4000, 48),
        ],
        512,
        5000,
        |c, start| {
            if start == 1024 {
                c.set(
                    0,
                    Some(Slot {
                        fx: Fx::Transpose { semitones: 7 },
                        on: true,
                    }),
                );
            }
        },
    );
    assert_eq!(ons(&got), [(10, 60, 100), (3000, 55, 90)]);
    assert_eq!(offs(&got), [(2000, 60), (4000, 55)]);
    // Off the top: not played, and its off sends nothing.
    let mut c = chain_of(&[Fx::Transpose { semitones: 48 }]);
    let got = run(&mut c, &[Ev::on(0, 100, 100), Ev::off(10, 100)], 64, 64);
    assert!(got.is_empty(), "{got:?}");
}

#[test]
fn scale_snaps_notes_into_key_and_skip_leaves_them_out() {
    let input = [
        Ev::on(0, 49, 100),
        Ev::off(100, 49),
        Ev::on(200, 50, 80),
        Ev::off(300, 50),
    ];
    for (snap, want) in [
        (Snap::Down, vec![48, 50]),
        (Snap::Up, vec![50, 50]),
        (Snap::Skip, vec![50]),
    ] {
        let mut c = chain_of(&[Fx::Scale {
            scale: c_major(),
            snap,
        }]);
        let got = run(&mut c, &input, 128, 512);
        let notes: Vec<u8> = ons(&got).iter().map(|o| o.1).collect();
        assert_eq!(notes, want, "{snap:?}");
        balanced(&got);
    }
}

#[test]
fn a_chord_from_c3_is_c3_eb3_g3_bb3_and_its_key_ends_it() {
    // Board 7: m7, close, its added notes at 90 %.
    let chord = Chord {
        shape: Shape::Minor7,
        velocity: 90,
        ..Chord::default()
    };
    let mut c = chain_of(&[Fx::Chord(chord)]);
    let got = run(&mut c, &[Ev::on(5, 48, 100), Ev::off(900, 48)], 256, 1024);
    assert_eq!(
        ons(&got),
        [(5, 48, 100), (5, 51, 90), (5, 55, 90), (5, 58, 90)]
    );
    assert_eq!(offs(&got), [(900, 48), (900, 51), (900, 55), (900, 58)]);
}

#[test]
fn voicing_inversion_and_keep_below_move_the_chords_notes() {
    let notes = |c: Chord, played: u8| {
        let (n, len, at) = c.notes(played);
        (n[..len].to_vec(), at)
    };
    let major = Chord::default();
    assert_eq!(notes(major, 48), (vec![48, 52, 55], Some(0)));
    let spread = Chord {
        voicing: Voicing::Spread,
        ..major
    };
    assert_eq!(notes(spread, 48), (vec![48, 55, 64], Some(0)));
    let drop2 = Chord {
        voicing: Voicing::Drop2,
        ..major
    };
    assert_eq!(notes(drop2, 48), (vec![40, 48, 55], Some(1)));
    let first = Chord {
        inversion: 1,
        ..major
    };
    assert_eq!(notes(first, 48), (vec![52, 55, 60], Some(2)));
    let below = Chord {
        shape: Shape::Minor7,
        keep_below: Some(57),
        ..major
    };
    assert_eq!(notes(below, 48), (vec![46, 48, 51, 55], Some(1)));
    // In key: thirds in the scale, from the note played.
    let in_key = Chord {
        shape: Shape::InKey(3),
        scale: Some(c_major()),
        ..major
    };
    assert_eq!(notes(in_key, 50), (vec![50, 53, 57], Some(0)));
    let four = Chord {
        shape: Shape::InKey(4),
        ..in_key
    };
    assert_eq!(notes(four, 55), (vec![55, 59, 62, 65], Some(0)));
    // Custom: a fifth and an octave; the top of the range is left out.
    let custom = Chord {
        shape: Shape::Custom((1 << 6) | (1 << 11)),
        ..major
    };
    assert_eq!(notes(custom, 60), (vec![60, 67, 72], Some(0)));
    assert_eq!(notes(custom, 120), (vec![120, 127], Some(0)));
}

#[test]
fn strum_starts_each_note_after_the_one_below_and_keeps_each_length() {
    let chord = Chord {
        strum_ms: 10,
        ..Chord::default()
    };
    let mut c = chain_of(&[Fx::Chord(chord)]);
    let got = run(&mut c, &[Ev::on(0, 48, 100), Ev::off(4000, 48)], 256, 6000);
    assert_eq!(ons(&got), [(0, 48, 100), (480, 52, 100), (960, 55, 100)]);
    assert_eq!(offs(&got), [(4000, 48), (4480, 52), (4960, 55)]);
}

#[test]
fn velocity_is_fixed_or_on_a_curve_between_its_lowest_and_highest() {
    let fixed = Velocity {
        fixed: Some(100),
        ..Velocity::default()
    };
    assert_eq!(fixed.apply(20), 100);
    let straight = Velocity::default();
    assert_eq!(straight.apply(127), 127);
    assert_eq!(straight.apply(64), 64);
    let range = Velocity {
        min: 40,
        max: 100,
        ..Velocity::default()
    };
    assert_eq!(range.apply(127), 100);
    assert_eq!(range.apply(1), 40);
    let louder = Velocity {
        curve: 50,
        ..Velocity::default()
    };
    assert_eq!(louder.apply(32), 64);
    let softer = Velocity {
        curve: -50,
        ..Velocity::default()
    };
    assert!(softer.apply(64) < 40);
    let mut c = chain_of(&[Fx::Velocity(range)]);
    let got = run(&mut c, &[Ev::on(0, 60, 127), Ev::off(10, 60)], 64, 64);
    assert_eq!(ons(&got), [(0, 60, 100)]);
}

#[test]
fn note_length_ends_each_note_on_time_however_long_it_is_held() {
    // A 1/16 at 120 BPM: 6000 frames.
    let mut c = chain_of(&[Fx::Length {
        time: Time::Ticks(240),
    }]);
    let got = run(
        &mut c,
        &[
            Ev::on(100, 60, 100),
            Ev::off(50_000, 60),
            Ev::on(60_000, 62, 100),
            Ev::off(60_010, 62),
        ],
        1024,
        70_000,
    );
    assert_eq!(ons(&got), [(100, 60, 100), (60_000, 62, 100)]);
    assert_eq!(offs(&got), [(6100, 60), (66_000, 62)]);
    assert_eq!(Time::Ms(250).frames(RATE, BPM), 12_000);
    assert_eq!(Time::Ticks(480).label(), "1/8");
}

#[test]
fn echo_plays_each_note_again_softer_and_its_key_ends_the_repeats() {
    let echo = Echo {
        time: Time::Ticks(480),
        repeats: 3,
        feedback: 50,
        pitch: 0,
    };
    let mut c = chain_of(&[Fx::Echo(echo)]);
    let got = run(
        &mut c,
        &[Ev::on(0, 60, 100), Ev::off(1000, 60)],
        4096,
        48_000,
    );
    assert_eq!(
        ons(&got),
        [
            (0, 60, 100),
            (12_000, 60, 50),
            (24_000, 60, 25),
            (36_000, 60, 13)
        ]
    );
    assert_eq!(
        offs(&got),
        [(1000, 60), (13_000, 60), (25_000, 60), (37_000, 60)]
    );
    // Climbing repeats, cut where they would leave the keyboard.
    let up = Echo {
        pitch: 12,
        feedback: 100,
        ..echo
    };
    let mut c = chain_of(&[Fx::Echo(up)]);
    let got = run(
        &mut c,
        &[Ev::on(0, 100, 100), Ev::off(1000, 100)],
        4096,
        48_000,
    );
    let notes: Vec<u8> = ons(&got).iter().map(|o| o.1).collect();
    assert_eq!(notes, [100, 112, 124]);
    balanced(&got);
}

#[test]
fn humanize_is_late_by_at_most_its_time_and_a_note_keeps_its_length() {
    let mut c = chain_of(&[Fx::Humanize {
        time_ms: 10,
        velocity: 8,
    }]);
    let mut input = Vec::new();
    for k in 0..200u64 {
        input.push(Ev::on(k * 2000, 60, 100));
        input.push(Ev::off(k * 2000 + 700, 60));
    }
    let got = run(&mut c, &input, 512, 200 * 2000 + 2000);
    let on = ons(&got);
    let off = offs(&got);
    assert_eq!(on.len(), 200);
    let mut late = std::collections::BTreeSet::new();
    for (k, (o, f)) in on.iter().zip(&off).enumerate() {
        let d = o.0 - k as u64 * 2000;
        assert!(d <= 480, "{d} frames late");
        assert_eq!(f.0 - o.0, 700, "the note keeps its length");
        assert!((92..=108).contains(&o.2), "velocity {}", o.2);
        late.insert(d);
    }
    assert!(late.len() > 50, "not the same each time");
    // The same seed plays the same notes.
    let mut again = chain_of(&[Fx::Humanize {
        time_ms: 10,
        velocity: 8,
    }]);
    assert_eq!(run(&mut again, &input, 333, 200 * 2000 + 2000), got);
}

#[test]
fn chance_plays_about_its_share_and_a_note_not_played_sends_no_off() {
    let mut c = chain_of(&[Fx::Chance { percent: 25 }]);
    let mut input = Vec::new();
    for k in 0..1000u64 {
        input.push(Ev::on(k * 100, 60, 100));
        input.push(Ev::off(k * 100 + 50, 60));
    }
    let got = run(&mut c, &input, 1024, 100_000);
    let n = ons(&got).len();
    assert!((180..=320).contains(&n), "{n} of 1000 played");
    assert_eq!(offs(&got).len(), n);
    balanced(&got);
}

#[test]
fn effects_run_top_to_bottom() {
    let scale = Fx::Scale {
        scale: c_major(),
        snap: Snap::Skip,
    };
    let chord = Fx::Chord(Chord::default());
    let input = [Ev::on(0, 50, 100), Ev::off(100, 50)];
    // Chord, then Scale: D major's F# isn't in C major.
    let mut c = chain_of(&[chord, scale]);
    let notes: Vec<u8> = ons(&run(&mut c, &input, 128, 128))
        .iter()
        .map(|o| o.1)
        .collect();
    assert_eq!(notes, [50, 57]);
    // Scale, then Chord: the chord is made after.
    let mut c = chain_of(&[scale, chord]);
    let notes: Vec<u8> = ons(&run(&mut c, &input, 128, 128))
        .iter()
        .map(|o| o.1)
        .collect();
    assert_eq!(notes, [50, 54, 57]);
    // Switched off, an effect passes notes as they come.
    c.set(
        1,
        Some(Slot {
            fx: chord,
            on: false,
        }),
    );
    let notes: Vec<u8> = ons(&run(&mut c, &input, 128, 128))
        .iter()
        .map(|o| o.1)
        .collect();
    assert_eq!(notes, [50]);
    // Up to a place and from it, as around a track's arp.
    let mut c = chain_of(&[Fx::Transpose { semitones: 12 }, chord]);
    let p = Period {
        start: 0,
        frames: 128,
        rate: RATE,
        bpm: BPM,
    };
    let mut out = Box::new(Notes::new());
    c.process_range(0, 1, &p, &input, &mut out);
    assert_eq!(out.as_slice(), [Ev::on(0, 62, 100), Ev::off(100, 62)]);
    out.clear();
    c.process_range(1, MAX_FX, &p, &input, &mut out);
    assert_eq!(out.len(), 6);
}

#[test]
fn an_effect_taken_out_while_a_key_is_down_still_ends_its_notes() {
    let mut c = chain_of(&[Fx::Chord(Chord::default())]);
    let got = run_with(
        &mut c,
        &[
            Ev::on(0, 48, 100),
            Ev::off(1000, 48),
            Ev::on(1100, 48, 100),
            Ev::off(1200, 48),
        ],
        256,
        2048,
        |c, start| {
            if start == 512 {
                c.set(0, None);
            }
        },
    );
    assert_eq!(offs(&got), [(1000, 48), (1000, 52), (1000, 55), (1200, 48)]);
    assert_eq!(ons(&got).len(), 4);
    // A key down before the effect came ends as it was played.
    let mut c = Box::new(Chain::new());
    let got = run_with(
        &mut c,
        &[Ev::on(0, 48, 100), Ev::off(1000, 48)],
        256,
        2048,
        |c, start| {
            if start == 512 {
                c.set(
                    0,
                    Some(Slot {
                        fx: Fx::Chord(Chord::default()),
                        on: true,
                    }),
                );
            }
        },
    );
    assert_eq!(got, [Ev::on(0, 48, 100), Ev::off(1000, 48)]);
    // A panic forgets what was held and waiting.
    let mut c = chain_of(&[Fx::Echo(Echo::default())]);
    let _ = run(&mut c, &[Ev::on(0, 60, 100)], 256, 512);
    assert!(c.busy());
    c.reset();
    c.set(0, None);
    assert!(!c.busy());
}

#[test]
fn a_key_played_twice_while_held_ends_twice() {
    let chord = Fx::Chord(Chord::default());
    let mut c = chain_of(&[chord, Fx::Echo(Echo::default())]);
    let got = run(
        &mut c,
        &[
            Ev::on(0, 48, 100),
            Ev::on(500, 48, 80),
            Ev::off(800, 48),
            Ev::off(900, 48),
        ],
        256,
        96_000,
    );
    balanced(&got);
}

/// The notes don't depend on how the periods are cut.
#[test]
fn period_size_changes_nothing() {
    let fx = [
        Fx::Chord(Chord {
            strum_ms: 7,
            shape: Shape::Seventh,
            ..Chord::default()
        }),
        Fx::Humanize {
            time_ms: 12,
            velocity: 10,
        },
        Fx::Echo(Echo {
            time: Time::Ticks(320),
            ..Echo::default()
        }),
        Fx::Length { time: Time::Ms(90) },
    ];
    let mut input = Vec::new();
    for k in 0..40u64 {
        let note = 40 + (k * 7 % 30) as u8;
        input.push(Ev::on(k * 1500, note, 60 + k as u8));
        input.push(Ev::off(k * 1500 + 900 + k * 13, note));
    }
    let until = 40 * 1500 + 96_000;
    let mut first = None;
    for block in [64, 127, 1000, 4096] {
        let mut c = chain_of(&fx);
        let got = run(&mut c, &input, block, until);
        assert_eq!(c.dropped(), 0, "periods of {block}");
        balanced(&got);
        match &first {
            None => first = Some(got),
            Some(f) => assert_eq!(&got, f, "periods of {block}"),
        }
    }
}

/// Whatever is played and changed, every note that starts ends.
#[test]
fn a_cut_drops_what_waits_and_ends_what_sounds() {
    let echo = Echo {
        time: Time::Ticks(480),
        repeats: 3,
        feedback: 50,
        pitch: 0,
    };
    let fx = [
        Fx::Chord(Chord {
            shape: Shape::Fifth,
            strum_ms: 10,
            ..Chord::default()
        }),
        Fx::Echo(echo),
    ];
    // C3 held from 0 to 30 000; the cut lands at 20 000, after the first
    // repeat (12 000), before the second (24 000). D3 comes after it.
    let input = [
        Ev::on(0, 48, 100),
        Ev::on(25_000, 50, 100),
        Ev::off(26_000, 50),
        Ev::off(30_000, 48),
    ];
    let mut c = chain_of(&fx);
    let got = run_with(&mut c, &input, 512, 96_000, |c, start| {
        if start == 19_968 {
            c.cut(20_000);
        }
    });
    let before: Vec<_> = ons(&got).into_iter().filter(|o| o.0 < 20_000).collect();
    assert_eq!(
        before,
        [
            (0, 48, 100),
            (480, 55, 100),
            (12_000, 48, 50),
            (12_480, 55, 50)
        ]
    );
    // What sounded ends at the cut; the held key's off ends nothing more.
    let at_cut: Vec<_> = offs(&got).into_iter().filter(|o| o.0 == 20_000).collect();
    assert_eq!(
        at_cut,
        [(20_000, 48), (20_000, 48), (20_000, 55), (20_000, 55)]
    );
    assert!(offs(&got)
        .iter()
        .all(|o| o.0 <= 20_000 || o.1 == 50 || o.1 == 57));
    // D3 after the cut plays with its fifth and echoes.
    let after: Vec<_> = ons(&got).into_iter().filter(|o| o.0 > 20_000).collect();
    assert_eq!(after[..2], [(25_000, 50, 100), (25_480, 57, 100)]);
    assert_eq!(after.len(), 8);
    balanced(&got);
    assert!(!c.busy() || c.slots().count() > 0);
}

#[test]
fn every_note_that_starts_ends_whatever_is_played_and_changed() {
    let mut seed = 0x1234_5678u32;
    let mut rnd = move |n: u32| {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed % n
    };
    let choices = [
        Fx::Transpose { semitones: 5 },
        Fx::Transpose { semitones: -12 },
        Fx::Scale {
            scale: c_major(),
            snap: Snap::Skip,
        },
        Fx::Scale {
            scale: Scale::new(9, mpfree_scale::PENTATONIC_MINOR),
            snap: Snap::Nearest,
        },
        Fx::Chord(Chord::default()),
        Fx::Chord(Chord {
            shape: Shape::Add9,
            voicing: Voicing::Drop2,
            inversion: 2,
            strum_ms: 15,
            keep_below: Some(72),
            ..Chord::default()
        }),
        Fx::Velocity(Velocity::default()),
        Fx::Length {
            time: Time::Ticks(160),
        },
        Fx::Echo(Echo {
            repeats: 5,
            pitch: 7,
            ..Echo::default()
        }),
        Fx::Humanize {
            time_ms: 20,
            velocity: 20,
        },
        Fx::Chance { percent: 50 },
    ];
    for round in 0..20 {
        let mut c = Box::new(Chain::new());
        for i in 0..4 {
            let fx = choices[rnd(choices.len() as u32) as usize];
            c.set(i, Some(Slot { fx, on: true }));
        }
        let mut input = Vec::new();
        let mut down = [false; 128];
        let mut t = 0u64;
        for _ in 0..300 {
            t += u64::from(rnd(3000));
            let note = 36 + rnd(48) as u8;
            if down[usize::from(note)] {
                input.push(Ev::off(t, note));
            } else {
                input.push(Ev::on(t, note, 1 + rnd(127) as u8));
            }
            down[usize::from(note)] = !down[usize::from(note)];
        }
        for n in 0..128u8 {
            if down[usize::from(n)] {
                input.push(Ev::off(t + 1, n));
            }
        }
        let got = run_with(&mut c, &input, 256, t + 400_000, |c, start| {
            if rnd(100) == 0 {
                // A mute landing somewhere in the coming period.
                c.cut(start + u64::from(rnd(256)));
            }
            if rnd(40) == 0 {
                let i = rnd(4) as usize;
                let slot = match rnd(3) {
                    0 => None,
                    1 => Some(Slot {
                        fx: choices[rnd(choices.len() as u32) as usize],
                        on: true,
                    }),
                    _ => c.slot(i).map(|s| Slot { on: !s.on, ..s }),
                };
                c.set(i, slot);
            }
        });
        assert_eq!(c.dropped(), 0, "round {round}");
        balanced(&got);
    }
}

#[test]
fn a_flood_drops_new_notes_but_never_an_off() {
    // Echo with 16 repeats on every key: far more than a list holds.
    let mut c = chain_of(&[Fx::Echo(Echo {
        repeats: 16,
        feedback: 100,
        time: Time::Ms(500),
        pitch: 0,
    })]);
    let mut input = Vec::new();
    for k in 0..128u8 {
        input.push(Ev::on(u64::from(k), k, 100));
    }
    for k in 0..128u8 {
        input.push(Ev::off(200 + u64::from(k), k));
    }
    let got = run(&mut c, &input, 512, 48_000 * 10);
    balanced(&got);
    assert!(ons(&got).len() < 128 * 17);
}

#[test]
fn every_effect_has_a_name_and_says_what_it_does() {
    let said: Vec<String> = Fx::all()
        .iter()
        .map(|f| format!("{}: {}", f.name(), f.summary()))
        .collect();
    assert_eq!(
        said,
        [
            "Scale: C Major · down",
            "Chord: Major · close",
            "Transpose: +12 semitones",
            "Velocity: 1 to 127 · curve +0",
            "Note length: 1/16",
            "Echo: 1/8 · 3 repeats",
            "Humanize: up to 6 ms late · ±8 vel",
            "Chance: 75 %",
        ]
    );
}
