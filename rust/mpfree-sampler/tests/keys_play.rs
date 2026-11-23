//! What keys play on a kit (its Keys play: Pads, Keygroups or Both; live
//! shows decision 6, sampler step 15): by what the kit holds until one is
//! picked, one undo step, saved only once picked, and resolved with the
//! kit's view ([`KitKeys`]) so the audio thread decides each note in one
//! step.

use mpfree_sampler::engine::{EngineView, KitKeys};
use mpfree_sampler::model::*;
use mpfree_sampler::save::{from_text, to_text};
use mpfree_sampler::wav;

const RATE: u32 = 44_100;

fn sine(name: &str) -> Source {
    let v = (0..4_410)
        .map(|i| (0.5 * (std::f64::consts::TAU * 220.0 * i as f64 / f64::from(RATE)).sin()) as f32)
        .collect();
    Source::new(name, RATE, 1, v).unwrap()
}

/// A drum pad on pad `pad`.
fn drum(p: &mut Project, pad: usize) {
    let s = sine("Kick");
    p.add_source(s.clone());
    p.set_pad(
        pad,
        Some(Pad::new("Kick", Layer::new(Region::whole(&s)))),
        None,
    )
    .unwrap();
}

/// A keygroup on pad `pad` holding notes `lo` to `hi`.
fn keygroup(p: &mut Project, pad: usize, lo: u8, hi: u8) {
    let s = sine("Bass");
    p.add_source(s.clone());
    let mut pad_ = Pad::new("Bass", Layer::new(Region::whole(&s)));
    pad_.keys = Some(Keys { lo, hi, root: lo });
    p.set_pad(pad, Some(pad_), None).unwrap();
}

#[test]
fn keys_play_by_what_the_kit_holds_until_one_is_picked() {
    let mut p = Project::new();
    assert_eq!(p.keys_play(), KeysPlay::Pads, "an empty kit");
    drum(&mut p, 0);
    assert_eq!(p.keys_play(), KeysPlay::Pads, "only pads");
    let mut keys = Project::new();
    keygroup(&mut keys, 3, 36, 59);
    assert_eq!(keys.keys_play(), KeysPlay::Keygroups, "only keygroups");
    keygroup(&mut p, 3, 36, 59);
    assert_eq!(p.keys_play(), KeysPlay::Pads, "both stays on Pads");
    assert_eq!(p.keys_play_picked(), None);
    // Picked: one undo step, and undo goes back to what the kit holds.
    p.set_keys_play(Some(KeysPlay::Both));
    assert_eq!(p.keys_play(), KeysPlay::Both);
    assert!(p.history().0.last().is_some_and(|i| i.keys_play));
    p.undo();
    assert_eq!(p.keys_play_picked(), None);
    assert_eq!(p.keys_play(), KeysPlay::Pads);
    p.redo();
    assert_eq!(p.keys_play_picked(), Some(KeysPlay::Both));
    // The same pick again is no step.
    let at = p.revision();
    p.set_keys_play(Some(KeysPlay::Both));
    assert_eq!(p.revision(), at);
    for k in [KeysPlay::Pads, KeysPlay::Keygroups, KeysPlay::Both] {
        assert_eq!(KeysPlay::from_word(k.word()), Some(k));
    }
    assert_eq!(KeysPlay::from_word("drums"), None);
}

#[test]
fn the_kits_view_decides_each_note_in_one_step() {
    // Drums on pads 1 to 2, a bass keygroup over C2 to B3 on pad 4.
    let mut p = Project::new();
    drum(&mut p, 0);
    drum(&mut p, 1);
    keygroup(&mut p, 3, 36, 59);
    let keys = |p: &Project| EngineView::build(p, RATE).keys();
    let k = keys(&p);
    assert_eq!(k.play, KeysPlay::Pads);
    assert_eq!(k.held, (36..=59).fold(0u128, |b, n| b | 1 << n));
    assert!(!k.to_keygroups(40), "Pads: every note its pad");
    assert!(!k.takes_notes());
    p.set_keys_play(Some(KeysPlay::Both));
    let k = keys(&p);
    assert!(
        k.to_keygroups(36) && k.to_keygroups(59),
        "a keygroup holds it"
    );
    assert!(
        !k.to_keygroups(35) && !k.to_keygroups(60),
        "its pad elsewhere"
    );
    assert!(!k.takes_notes(), "Both keeps the pads' hits");
    p.set_keys_play(Some(KeysPlay::Keygroups));
    let k = keys(&p);
    assert!(
        k.to_keygroups(20) && k.to_keygroups(40),
        "every note, held or not"
    );
    assert!(!k.takes_notes(), "the pads play their own sounds");
    // With the pads playing notes from C2, the kit is played as notes.
    p.set_pads_play(PadsPlay::Keys { from: 36 }).unwrap();
    let k = keys(&p);
    assert!(k.takes_notes());
    assert_eq!(k.pad_of_note(36), Some(0));
    assert_eq!(k.pad_of_note(59), Some(23));
    assert_eq!(k.pad_of_note(35), None, "below pad 1's note");
    assert_eq!(k.note_of_pad(0), Some(36));
    assert_eq!(k.note_of_pad(91), Some(127));
    assert_eq!(k.note_of_pad(92), None, "past the last note");
    // A kit that never had the setting plays pads.
    assert_eq!(EngineView::empty().keys(), KitKeys::PADS);
}

#[test]
fn keys_play_is_saved_once_picked_and_an_unknown_one_is_said() {
    let mut picked = Project::new();
    keygroup(&mut picked, 0, 48, 72);
    drum(&mut picked, 1);
    picked.set_keys_play(Some(KeysPlay::Both));
    let mut plain = Project::new();
    keygroup(&mut plain, 0, 48, 72);
    let text = to_text(
        &[("Picked", &picked), ("Plain", &plain)],
        0,
        &wav::file_name,
    );
    assert!(text.contains("kit.0.keys_play = both"), "{text}");
    assert!(!text.contains("kit.1.keys_play"), "not picked, not written");
    let files: std::collections::HashMap<String, Vec<u8>> = [&picked, &plain]
        .iter()
        .flat_map(|p| p.sources())
        .map(|s| (wav::file_name(s), wav::bytes(s)))
        .collect();
    let mut read = |f: &str| wav::read("file", files.get(f).ok_or("missing")?);
    let opened = from_text(&text, &mut read).unwrap();
    assert!(opened.notes.is_empty(), "{:?}", opened.notes);
    assert_eq!(opened.kits[0].1.keys_play_picked(), Some(KeysPlay::Both));
    assert_eq!(opened.kits[1].1.keys_play_picked(), None);
    assert_eq!(opened.kits[1].1.keys_play(), KeysPlay::Keygroups);
    // A word this build doesn't know: said, and by what the kit holds.
    let odd = text.replace("keys_play = both", "keys_play = drums");
    let opened = from_text(&odd, &mut read).unwrap();
    assert_eq!(opened.kits[0].1.keys_play_picked(), None);
    assert_eq!(opened.kits[0].1.keys_play(), KeysPlay::Pads);
    assert!(
        opened
            .notes
            .iter()
            .any(|n| n.contains("keys play by what the kit holds")),
        "{:?}",
        opened.notes
    );
}
