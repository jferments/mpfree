//! The chop kept for a sound: the Chop tab's slices as last left, one set
//! a sound, outside undo, moved with its sound to another rate.

use mpfree_sampler::model::*;

fn tone(rate: u32, frames: usize, name: &str) -> Source {
    let v = (0..frames)
        .map(|i| ((i % 100) as f32 - 50.0) / 100.0)
        .collect();
    Source::new(name, rate, 1, v).unwrap()
}

#[test]
fn a_sound_keeps_one_chop_outside_undo() {
    let a = tone(44_100, 40_000, "Break");
    let b = tone(44_100, 20_000, "Vox");
    let mut p = Project::new();
    p.add_source(a.clone());
    p.add_source(b.clone());
    p.set_pad(
        0,
        Some(Pad::new("Break", Layer::new(Region::whole(&a)))),
        None,
    )
    .unwrap();
    let steps = |p: &Project| {
        let mut p = p.clone();
        let mut n = 0;
        while p.undo() {
            n += 1;
        }
        n
    };
    let before = steps(&p);
    assert!(p.kept_chop(a.id()).is_none());
    let first = ChopSet::from_markers(KEPT_CHOP, &a, ChopMode::Transient, [5_000, 12_000]).unwrap();
    p.keep_chop(first).unwrap();
    let vox = ChopSet::grid(KEPT_CHOP, &b, 4).unwrap();
    p.keep_chop(vox.clone()).unwrap();
    // Keeping again replaces the sound's chop, not the other sound's.
    let second =
        ChopSet::from_markers(KEPT_CHOP, &a, ChopMode::Manual, [5_000, 8_000, 12_000]).unwrap();
    p.keep_chop(second.clone()).unwrap();
    assert_eq!(p.kept_chops().len(), 2);
    assert_eq!(p.kept_chop(a.id()), Some(&second));
    assert_eq!(p.kept_chop(b.id()), Some(&vox));
    // Not an undo step, and undo leaves it be.
    assert_eq!(steps(&p), before);
    assert!(p.undo());
    assert_eq!(p.kept_chop(a.id()), Some(&second));
    // A chop for a sound the kit doesn't have is refused.
    let stray = tone(44_100, 10_000, "Stray");
    let c = ChopSet::grid(KEPT_CHOP, &stray, 2).unwrap();
    assert_eq!(p.keep_chop(c), Err(ModelError::UnknownSource));
}

#[test]
fn a_kept_chop_moves_with_its_sound_to_another_rate() {
    let s = tone(48_000, 48_000, "Break");
    let mut p = Project::new();
    p.add_source(s.clone());
    p.set_pad(
        0,
        Some(Pad::new("Break", Layer::new(Region::whole(&s)))),
        None,
    )
    .unwrap();
    let c =
        ChopSet::from_markers(KEPT_CHOP, &s, ChopMode::Manual, [12_000, 24_000, 36_000]).unwrap();
    p.keep_chop(c).unwrap();
    let moved = p.convert_to_rate(44_100);
    assert_eq!(moved.len(), 1);
    let to = p.pad(0).unwrap().layers[0].region.source;
    assert!(p.kept_chop(s.id()).is_none(), "the old sound has none");
    let kept = p.kept_chop(to).expect("kept on the converted sound");
    assert_eq!(kept.markers(), &[11_025, 22_050, 33_075]);
}
