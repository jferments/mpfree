//! Robustness: a damaged pattern or library file (a bad stick, a hand edit)
//! is refused with a reason or read into something that writes back to the
//! same thing, and a pattern read from one plays in the engine without a
//! panic. Never a panic in the reader.

#[path = "../../test-support/mutate.rs"]
mod mutate;

use mpfree_arp::{Engine, Events, Library, Mode, Pattern, Settings, BUILT_IN};
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
