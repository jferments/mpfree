//! CV outputs as LFOs and envelopes ([`mpfree_midi::cv_mod`]), through
//! the CV engine as the app drives it: the song's spans, then the
//! period's notes in any order, then the period's end.

use mpfree_midi::cv::{CvAssign, CvChange, CvEngine};
use mpfree_midi::cv_mod::{Env, Lfo, Notes, Rate, Shape, MOD_STEP_MS};
use mpfree_midi::Msg;

const RATE: u32 = 48_000;
/// 120 BPM: quarter notes a frame.
const PER_FRAME: f64 = 2.0 / RATE as f64;
const STEP: u64 = (MOD_STEP_MS as u64) * RATE as u64 / 1000;

/// What happens at an absolute frame.
#[derive(Clone, Copy)]
enum Do {
    On(u8, u8),
    Off(u8, u8),
    Set(usize, CvAssign),
    /// The song jumps to this beat (a locate) and plays on from it.
    Locate(f64),
    Stop,
}

/// Runs `frames` frames in periods of `period`, the song playing from beat
/// 0 at 120 BPM unless stopped; each period's song spans are given first
/// (as `CvOut::clock` does), then its notes, then its end. The changes, by
/// absolute frame.
fn run(
    outs: &[(usize, CvAssign)],
    script: &[(u64, Do)],
    frames: u64,
    period: u32,
) -> Vec<(u64, u8, f32)> {
    let mut cv = CvEngine::new(RATE);
    let mut got = Vec::new();
    for &(o, a) in outs {
        cv.set_assign(o, a, 0, &mut |c: CvChange| {
            got.push((u64::from(c.offset), c.out, c.volts))
        });
    }
    let (mut beat0, mut from, mut rolling) = (0.0, 0u64, true);
    let mut base = 0u64;
    while base < frames {
        let len = u64::from(period).min(frames - base);
        let mut emit = |c: CvChange| got.push((base + u64::from(c.offset), c.out, c.volts));
        // The song: a locate or a stop splits the period.
        let mut cuts: Vec<(u64, Do)> = script
            .iter()
            .filter(|(f, d)| {
                (base..base + len).contains(f) && matches!(d, Do::Locate(_) | Do::Stop)
            })
            .copied()
            .collect();
        cuts.push((base + len, Do::Stop));
        let mut at = base;
        for (f, d) in cuts {
            if f > at {
                let beat = beat0 + (at - from) as f64 * PER_FRAME;
                cv.song(
                    (at - base) as u32,
                    (f - at) as u32,
                    beat,
                    PER_FRAME,
                    rolling,
                );
            }
            if f < base + len {
                match d {
                    Do::Locate(b) => (beat0, from, rolling) = (b, f, true),
                    Do::Stop => {
                        beat0 += (f - from) as f64 * PER_FRAME;
                        from = f;
                        rolling = false;
                    }
                    _ => {}
                }
            }
            at = f;
        }
        if !rolling {
            // Stopped: the song stays where it is.
            from = base + len;
        }
        for &(f, d) in script {
            if !(base..base + len).contains(&f) {
                continue;
            }
            let offset = (f - base) as u32;
            match d {
                Do::On(t, n) => cv.track_msg(
                    t,
                    Msg::NoteOn {
                        ch: 0,
                        note: n,
                        vel: 100,
                    },
                    offset,
                    &mut emit,
                ),
                Do::Off(t, n) => cv.track_msg(
                    t,
                    Msg::NoteOff {
                        ch: 0,
                        note: n,
                        vel: 0,
                    },
                    offset,
                    &mut emit,
                ),
                Do::Set(o, a) => cv.set_assign(o, a, offset, &mut emit),
                _ => {}
            }
        }
        cv.end_period(len as u32, &mut emit);
        base += len;
    }
    got.sort_by_key(|c| c.0);
    got
}

fn of(got: &[(u64, u8, f32)], out: u8) -> Vec<(u64, f32)> {
    got.iter()
        .filter(|c| c.1 == out)
        .map(|c| (c.0, c.2))
        .collect()
}

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-3
}

#[test]
fn an_lfo_on_the_song_starts_its_cycle_on_the_beat_every_four_ms() {
    let lfo = CvAssign::Lfo(Lfo {
        shape: Shape::Sine,
        rate: Rate::Beats(1.0),
        low_volts: 1.0,
        high_volts: 3.0,
        ..Lfo::default()
    });
    let got = run(&[(0, lfo)], &[], RATE as u64, 256);
    let ch = of(&got, 0);
    // Half a second is a beat at 120: the low at each beat, the top between.
    for (f, v) in &ch {
        let beat = *f as f64 * PER_FRAME;
        let want = 1.0 + 2.0 * (1.0 - (std::f64::consts::TAU * beat.fract()).cos()) / 2.0;
        assert!(near(*v, want as f32), "frame {f}: {v} not {want}");
        assert_eq!(f % STEP, 0, "on the grid");
    }
    // Every step changes it except where it turns round.
    assert!(ch.len() as u64 > RATE as u64 / STEP - 8, "{}", ch.len());
    // The same voltages at the same frames for any period size.
    for period in [64, 100, 1024] {
        assert_eq!(
            run(&[(0, lfo)], &[], RATE as u64, period),
            got,
            "period {period}"
        );
    }
}

#[test]
fn a_tempo_lfo_keeps_the_pace_when_stopped_and_jumps_to_the_song_when_it_plays() {
    let lfo = CvAssign::Lfo(Lfo {
        shape: Shape::RampUp,
        rate: Rate::Beats(4.0),
        low_volts: 0.0,
        high_volts: 4.0,
        ..Lfo::default()
    });
    // Stopped at 0.25 s (beat 0.5), then played from beat 2 at 1 s.
    let script = [(12_000, Do::Stop), (48_000, Do::Locate(2.0))];
    let ch = of(&run(&[(0, lfo)], &script, 60_000, 256), 0);
    let at = |f: u64| ch.iter().rev().find(|c| c.0 <= f).unwrap().1;
    // A bar's ramp: 4 V over 2 s at 120 BPM.
    assert!(near(at(11_904), 4.0 * 11_904.0 / 96_000.0));
    // Stopped, it keeps going at the tempo.
    assert!(near(at(47_808), 4.0 * 47_808.0 / 96_000.0));
    // Playing from beat 2: halfway up the bar's ramp.
    assert!(near(at(48_000), 2.0), "{}", at(48_000));
    assert!(near(at(59_904), 2.0 + 4.0 * 11_904.0 / 96_000.0));
}

#[test]
fn a_free_lfo_restarts_on_its_tracks_notes_only() {
    let lfo = CvAssign::Lfo(Lfo {
        shape: Shape::RampUp,
        rate: Rate::Hz(1.0),
        low_volts: 0.0,
        high_volts: 5.0,
        restart: Some(2),
        ..Lfo::default()
    });
    let script = [
        (10_000, Do::On(1, 60)),
        (24_050, Do::On(2, 60)),
        (24_100, Do::Off(2, 60)),
    ];
    let ch = of(&run(&[(0, lfo)], &script, 30_000, 512), 0);
    let at = |f: u64| ch.iter().rev().find(|c| c.0 <= f).unwrap().1;
    // Another track's note leaves it alone.
    assert!(near(at(23_999), 5.0 * 23_808.0 / 48_000.0));
    // Its own track's note restarts it there and then, not on the grid.
    assert!(ch.contains(&(24_050, 0.0)), "{:?}", &ch[ch.len() - 10..]);
    assert!(near(at(29_952), 5.0 * (29_952.0 - 24_050.0) / 48_000.0));
}

#[test]
fn random_levels_hold_a_cycle_and_come_back_with_the_song() {
    let lfo = CvAssign::Lfo(Lfo {
        shape: Shape::Random,
        rate: Rate::Beats(1.0),
        ..Lfo::default()
    });
    // Four beats, then back to the top.
    let ch = of(
        &run(&[(0, lfo)], &[(96_000, Do::Locate(0.0))], 192_000, 256),
        0,
    );
    let at = |f: u64| ch.iter().rev().find(|c| c.0 <= f).unwrap().1;
    let first: Vec<f32> = (0..4).map(|b| at(b * 24_000 + 12_000)).collect();
    let again: Vec<f32> = (0..4).map(|b| at(96_000 + b * 24_000 + 12_000)).collect();
    assert_eq!(first, again);
    assert!(first.windows(2).any(|w| w[0] != w[1]), "{first:?}");
    // Held for each beat: one change a beat.
    assert!(ch.len() <= 9, "{ch:?}");
}

#[test]
fn an_lfo_that_snaps_to_notes_plays_only_the_scales_notes() {
    // A minor pentatonic from A: A C D E G.
    let notes = Notes {
        mask: 0b0100_1010_1001,
        root: 9,
    };
    let lfo = CvAssign::Lfo(Lfo {
        shape: Shape::Drift,
        rate: Rate::Hz(4.0),
        low_volts: 0.0,
        high_volts: 2.0,
        notes,
        ..Lfo::default()
    });
    let ch = of(&run(&[(0, lfo)], &[], 4 * RATE as u64, 512), 0);
    assert!(ch.len() > 16, "a melody: {}", ch.len());
    for (_, v) in &ch {
        let semis = v * 12.0;
        assert!((semis - semis.round()).abs() < 1e-4, "{v}");
        let pc = (semis.round() as i32).rem_euclid(12);
        assert!([9, 0, 2, 4, 7].contains(&pc), "{v} is pitch class {pc}");
        assert!((0.0..=2.0).contains(v));
    }
    // Only a new note is sent.
    assert!(ch.windows(2).all(|w| w[0].1 != w[1].1));
}

#[test]
fn an_envelope_follows_its_tracks_keys_and_a_turned_setting_keeps_it_going() {
    let env = Env {
        attack_ms: 10.0,
        decay_ms: 50.0,
        sustain: 0.5,
        release_ms: 100.0,
        volts: 4.0,
        ..Env::new(0)
    };
    let turned = CvAssign::Envelope(Env {
        sustain: 0.75,
        ..env
    });
    let script = [
        (1_000, Do::On(0, 60)),
        (1_100, Do::On(0, 64)), // a second key: the attack again, from where it is
        (1_200, Do::Off(0, 60)), // a key still held: no release
        (9_600, Do::Set(0, turned)),
        (19_200, Do::Off(0, 64)),
        (30_000, Do::On(3, 60)), // another track
    ];
    let got = run(&[(0, CvAssign::Envelope(env))], &script, 33_600, 512);
    let ch = of(&got, 0);
    let at = |f: u64| ch.iter().rev().find(|c| c.0 <= f).unwrap().1;
    assert_eq!(at(999), 0.0);
    // Up to the top within 10 ms (480 frames) of the first key, and not
    // from 0 again for the second: the top is reached at 1480, and the
    // step after it is already on the way down.
    let top = ch.iter().map(|c| c.1).fold(0.0, f32::max);
    assert!(top > 3.7 && top <= 4.0, "{top}");
    let first_top = ch.iter().find(|c| c.1 == top).unwrap().0;
    assert_eq!(first_top, 1_536);
    // Down to the sustain, which follows its knob without starting again.
    assert!(near(at(9_599), 2.0), "{}", at(9_599));
    assert!(near(at(19_199), 3.0), "{}", at(19_199));
    assert!(ch
        .iter()
        .filter(|c| (9_600..19_200).contains(&c.0))
        .all(|c| c.1 <= 3.0 + 1e-3));
    // Let go: 0 V within its 100 ms.
    assert!(at(19_200 + 4_800 + STEP) < 0.05);
    assert_eq!(at(33_599), 0.0);
    // The same for any period size.
    for period in [64, 333, 4096] {
        assert_eq!(
            run(&[(0, CvAssign::Envelope(env))], &script, 33_600, period),
            got,
            "period {period}"
        );
    }
}

/// A new job starts from its start; the same job with a turned setting
/// keeps its place.
#[test]
fn a_new_job_starts_fresh_and_a_turned_lfo_keeps_its_place() {
    let slow = Lfo {
        shape: Shape::RampUp,
        rate: Rate::Hz(1.0),
        ..Lfo::default()
    };
    let wider = Lfo {
        high_volts: 4.0,
        low_volts: 0.0,
        ..slow
    };
    let script = [(24_000, Do::Set(0, CvAssign::Lfo(wider)))];
    let ch = of(&run(&[(0, CvAssign::Lfo(slow))], &script, 30_000, 512), 0);
    let at = |f: u64| ch.iter().rev().find(|c| c.0 <= f).unwrap().1;
    // Halfway up its ramp before and after: 2.5 V of 5, then 2 V of 4.
    assert!(near(at(23_999), 5.0 * 23_808.0 / 48_000.0));
    assert!(
        near(at(24_192), 4.0 * 24_192.0 / 48_000.0),
        "{}",
        at(24_192)
    );
    // An envelope given to an output mid-LFO starts at 0 V.
    let script = [(24_000, Do::Set(0, CvAssign::Envelope(Env::new(0))))];
    let ch = of(&run(&[(0, CvAssign::Lfo(slow))], &script, 30_000, 512), 0);
    assert_eq!(ch.last().unwrap(), &(24_000, 0.0));
}
