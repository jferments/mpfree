//! The Sample tab's FX page shows a pad's effects as pedals
//! ([`mpfree_chain_ui::state::pedals`]): each with its name, its switch,
//! its main knobs and a note of what those don't say.

use mpfree_chain_ui::sample::Song;

#[test]
fn a_pedal_notes_only_what_its_name_and_knobs_dont_say() {
    let pedals = Song::new().pedals();
    let seen: Vec<(&str, &str, bool, usize)> = pedals
        .iter()
        .map(|p| (p.name.as_str(), p.summary.as_str(), p.on, p.knobs.len()))
        .collect();
    assert_eq!(
        seen.iter().map(|s| (s.0, s.1, s.2)).collect::<Vec<_>>(),
        [
            ("Filter · Multi", "Low-pass", true),
            ("Drive · Soft", "", true),
            ("Compressor", "Peak", true),
            ("EQ", "", false),
        ],
    );
    assert!(seen.iter().all(|s| s.3 >= 3), "{seen:?}");
    // A knob's note never repeats a knob: the summary's values are choices.
    for p in &pedals {
        assert!(
            p.knobs.iter().all(|k| k.text() != p.summary),
            "{}: {}",
            p.name,
            p.summary
        );
    }
}
