//! Robustness: a damaged pattern or library file (a bad stick, a hand edit)
//! is refused with a reason or read into something that writes back to the
//! same thing, and a pattern read from one plays in the engine without a
//! panic. Never a panic in the reader.

#[path = "../../test-support/mutate.rs"]
mod mutate;

use mpfree_arp::{
    scale, Engine, EventKind, Events, Follow, Library, Mode, Pattern, Phrase, Scale, Settings,
    Speed, BUILT_IN, STARTER_PHRASES,
};
use mutate::Mutator;

/// Plays `p` on a held chord for a few hundred periods.
fn play(case: usize, engine: &mut Engine, p: Pattern) {
    engine.set_pattern(p);
    for note in [0u8, 4, 7, 15] {
        engine.press(note, 100);
    }
    let mut ev = Events::new();
    for _ in 0..300 {
        engine.process(128, &mut ev);
        for e in ev.as_slice() {
            assert!(e.offset < 128, "case {case}: {e:?}");
        }
    }
    for note in [0u8, 4, 7, 15] {
        engine.release(note);
    }
}

#[test]
fn damaged_patterns_are_refused_or_round_trip_and_play() {
    let mut m = Mutator::new(12);
    let mut engine = Engine::new(44_100, 128.0);
    engine.set_settings(Settings {
        mode: Mode::Pattern,
        ..Settings::default()
    });
    let (mut ok, mut refused) = (0, 0);
    for case in 0..6_000 {
        let text = m.mutate_text(BUILT_IN[case % BUILT_IN.len()].1);
        match Pattern::parse(&text) {
            Ok(p) => {
                ok += 1;
                let again = Pattern::parse(&p.to_text()).unwrap_or_else(|e| {
                    panic!("case {case}: {:?} does not read back: {e:?}", p.to_text())
                });
                assert_eq!(again, p, "case {case}: {text:?}");
                if case % 10 == 0 {
                    play(case, &mut engine, p);
                }
            }
            Err(_) => refused += 1,
        }
    }
    assert!(ok > 500 && refused > 500, "ok {ok}, refused {refused}");
}

/// Plays `p` from a held note, moves it to another, and lets go: every
/// note that started has ended, and none sounds twice at once. The gate,
/// the octaves, swing, the speed and the key change from case to case.
fn play_phrase(case: usize, engine: &mut Engine, p: Phrase) {
    engine.set_settings(Settings {
        mode: Mode::Phrase,
        gate: [50, 100, 20, 75][case % 4],
        octaves: [1, 3][case / 4 % 2],
        swing: [50, 66][case / 8 % 2],
        speed: [Speed::AsWritten, Speed::Double, Speed::Half][case / 16 % 3],
        follow: [Follow::InKey, Follow::Exact][case / 48 % 2],
        scale: [
            None,
            Some(Scale::new(0, scale::MINOR)),
            Some(Scale::new(3, scale::PENTATONIC_MINOR)),
        ][case / 96 % 3],
        ..Settings::default()
    });
    engine.set_phrase(p);
    let mut on = [0i32; 128];
    let mut played = 0;
    let mut ev = Events::new();
    let mut periods = |engine: &mut Engine, n: usize| {
        for _ in 0..n {
            engine.process(128, &mut ev);
            for e in ev.as_slice() {
                assert!(e.offset < 128, "case {case}: {e:?}");
                match e.kind {
                    EventKind::On { note, .. } => {
                        played += 1;
                        on[usize::from(note)] += 1;
                        assert!(
                            on[usize::from(note)] == 1,
                            "case {case}: note {note} twice at once"
                        );
                    }
                    EventKind::Off { note } => {
                        on[usize::from(note)] = (on[usize::from(note)] - 1).max(0);
                    }
                }
            }
        }
    };
    // Two bars and more at 128 bpm from each note.
    engine.press(48, 100);
    periods(engine, 1_400);
    engine.press(55, 90);
    periods(engine, 1_400);
    engine.release(55);
    periods(engine, 300);
    engine.release(48);
    periods(engine, 20);
    assert!(
        on.iter().all(|&n| n == 0),
        "case {case}: a note is left sounding"
    );
    assert_eq!(engine.overflow, 0, "case {case}");
    // A starter phrase as written is heard (the first 96 cases have no
    // scale to leave a held note out).
    if case < 96 {
        assert!(played >= 4, "case {case}: {played} notes");
    }
}

/// A damaged phrase is refused or reads into one that writes back to the
/// same text, and plays in the engine with every note ended.
#[test]
fn damaged_phrases_are_refused_or_round_trip_and_play() {
    let seeds: Vec<&str> = STARTER_PHRASES
        .lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .filter_map(|l| l.split_once('=').map(|(_, body)| body.trim()))
        .collect();
    assert_eq!(seeds.len(), 64);
    let mut m = Mutator::new(14);
    let mut engine = Engine::new(44_100, 128.0);
    engine.set_settings(Settings {
        mode: Mode::Phrase,
        ..Settings::default()
    });
    let (mut ok, mut refused) = (0, 0);
    for case in 0..6_000 {
        let seed = seeds[case % seeds.len()];
        // Every phrase as written with each gate, then damaged ones.
        let text = if case < 4 * seeds.len() {
            seed.to_string()
        } else {
            m.mutate_text(seed)
        };
        match Phrase::parse(&text) {
            Ok(p) => {
                ok += 1;
                let written = p.to_text();
                let again = Phrase::parse(&written).unwrap_or_else(|e| {
                    panic!("case {case}: {written:?} does not read back: {e:?}")
                });
                assert_eq!(again.to_text(), written, "case {case}: {text:?}");
                if case < 4 * seeds.len() || case % 10 == 0 {
                    play_phrase(case, &mut engine, p);
                }
            }
            Err(_) => refused += 1,
        }
    }
    assert!(ok > 500 && refused > 500, "ok {ok}, refused {refused}");
}

#[test]
fn damaged_libraries_are_refused_or_round_trip() {
    let seed = Library::built_in().to_text();
    let mut m = Mutator::new(13);
    let (mut ok, mut refused) = (0, 0);
    for case in 0..1_500 {
        let text = m.mutate_text(&seed);
        match Library::parse(&text) {
            Ok(lib) => {
                ok += 1;
                let written = lib.to_text();
                let again = Library::parse(&written)
                    .unwrap_or_else(|e| panic!("case {case}: does not read back: {e:?}"));
                assert_eq!(again.to_text(), written, "case {case}");
            }
            Err(errors) => {
                refused += 1;
                assert!(!errors.is_empty(), "case {case}");
            }
        }
    }
    assert!(ok > 100 && refused > 100, "ok {ok}, refused {refused}");
}
