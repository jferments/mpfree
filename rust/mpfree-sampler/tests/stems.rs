//! A sound's stems put on the kit (`Project::put_stems`, the stems plan's
//! step 4): on four pads, as one pad's layers or as takes, the sounds added
//! with them, all one undo step; and what it refuses, changing nothing.

use mpfree_sampler::model::*;

const RATE: u32 = 44_100;

/// Stereo noise, `frames` long, different for each `seed`.
fn noise(name: &str, frames: usize, seed: u32) -> Source {
    let mut x = seed;
    let data = (0..frames * 2).map(|_| {
        x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (x >> 8) as f32 / (1u32 << 24) as f32 - 0.5
    });
    Source::new(name, RATE, 2, data.collect()).unwrap()
}

/// A kit with pad 0 playing a loop, and its four stems (not in the kit).
fn kit() -> (Project, Vec<Source>) {
    let mut p = Project::new();
    let s = noise("Soul loop", 4410, 1);
    p.add_source(s.clone());
    p.set_pad(
        0,
        Some(Pad::new("Soul loop", Layer::new(Region::whole(&s)))),
        None,
    )
    .unwrap();
    let stems = ["vocals", "drums", "bass", "other"]
        .iter()
        .zip(10..)
        .map(|(n, seed)| noise(&format!("Soul loop {n}"), 4410, seed))
        .collect();
    (p, stems)
}

fn pad_of(s: &Source) -> Pad {
    Pad::new(&s.name, Layer::new(Region::whole(s)))
}

#[test]
fn on_four_pads_the_stems_and_their_sounds_are_one_undo_step() {
    let (mut p, stems) = kit();
    let before = (p.sources().len(), p.history().0.len());
    let pads = stems
        .iter()
        .enumerate()
        .map(|(k, s)| (5 + k, pad_of(s)))
        .collect();
    p.put_stems(stems.clone(), StemsTo::Pads(pads)).unwrap();
    for (k, s) in stems.iter().enumerate() {
        let pad = p.pad(5 + k).expect("a stem's pad");
        assert_eq!(pad.layers[0].region.source, s.id());
        assert!(p.source(s.id()).is_some());
    }
    assert_eq!(p.sources().len(), before.0 + 4);
    assert_eq!(p.history().0.len(), before.1 + 1, "one undo step");
    // UNDO takes the pads and the sounds back; REDO puts them again.
    assert!(p.undo());
    assert!((5..9).all(|i| p.pad(i).is_none()));
    assert!(stems.iter().all(|s| p.source(s.id()).is_none()));
    assert_eq!(p.sources().len(), before.0);
    assert!(p.redo());
    assert!(stems.iter().all(|s| p.source(s.id()).is_some()));
    assert_eq!(p.pad(8).unwrap().layers[0].region.source, stems[3].id());
}

#[test]
fn a_pad_tapped_has_its_sound_replaced_and_undo_puts_it_back() {
    let (mut p, stems) = kit();
    let was = p.pad(0).unwrap().clone();
    let pads = stems
        .iter()
        .enumerate()
        .map(|(k, s)| (k, pad_of(s)))
        .collect();
    p.put_stems(stems.clone(), StemsTo::Pads(pads)).unwrap();
    assert_eq!(p.pad(0).unwrap().layers[0].region.source, stems[0].id());
    p.undo();
    assert_eq!(p.pad(0), Some(&was));
}

#[test]
fn as_layers_the_pad_plays_the_four_together_in_place_of_the_sound() {
    let (mut p, stems) = kit();
    let was = p.pad(0).unwrap().clone();
    let mut to = was.clone();
    to.layers = stems.iter().map(|s| Layer::new(Region::whole(s))).collect();
    to.select = LayerSelect::All;
    p.put_stems(stems.clone(), StemsTo::Layers(0, Box::new(to)))
        .unwrap();
    let pad = p.pad(0).unwrap();
    assert_eq!(pad.layers.len(), 4);
    assert!(pad.layers.iter().all(|l| (l.vel_lo, l.vel_hi) == (1, 127)));
    assert_eq!(pad.name, was.name);
    p.undo();
    assert_eq!(p.pad(0), Some(&was));
    assert!(stems.iter().all(|s| p.source(s.id()).is_none()));
}

#[test]
fn as_takes_they_join_the_page() {
    let (mut p, stems) = kit();
    let takes = stems.iter().map(pad_of).collect();
    p.put_stems(stems.clone(), StemsTo::Takes(takes)).unwrap();
    assert_eq!(p.takes().len(), 4);
    assert_eq!(p.take(2).unwrap().layers[0].region.source, stems[2].id());
    p.undo();
    assert!(p.takes().is_empty());
}

#[test]
fn a_stem_switched_off_is_just_not_there() {
    let (mut p, mut stems) = kit();
    stems.remove(1);
    let pads = stems
        .iter()
        .enumerate()
        .map(|(k, s)| (5 + k, pad_of(s)))
        .collect();
    p.put_stems(stems.clone(), StemsTo::Pads(pads)).unwrap();
    assert!(p.pad(7).is_some() && p.pad(8).is_none());
}

#[test]
fn what_it_refuses_changes_nothing() {
    let (mut p, stems) = kit();
    let (sources, rev) = (p.sources().len(), p.revision());
    let refuse = |p: &mut Project, to: StemsTo| {
        assert!(p.put_stems(stems.clone(), to).is_err());
        assert_eq!((p.sources().len(), p.revision()), (sources, rev));
    };
    // A pad past the last, or one named twice.
    refuse(&mut p, StemsTo::Pads(vec![(PADS, pad_of(&stems[0]))]));
    refuse(
        &mut p,
        StemsTo::Pads(vec![(5, pad_of(&stems[0])), (5, pad_of(&stems[1]))]),
    );
    // Layers on an empty pad.
    refuse(&mut p, StemsTo::Layers(9, Box::new(pad_of(&stems[0]))));
    // A layer playing a sound neither given nor in the kit.
    let stranger = noise("stranger", 100, 99);
    refuse(&mut p, StemsTo::Pads(vec![(5, pad_of(&stranger))]));
    // More takes than the page keeps.
    let many = (0..MAX_TAKES + 1).map(|_| pad_of(&stems[0])).collect();
    refuse(&mut p, StemsTo::Takes(many));
    assert!(p.pad(5).is_none());
}
