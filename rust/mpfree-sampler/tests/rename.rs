//! Naming a pad and its sound (`Project::rename`, the Sample tab's text
//! keyboard): one undo step for both, trimmed, never empty.

use mpfree_sampler::model::*;

fn tone(frames: usize, name: &str) -> Source {
    let v = (0..frames)
        .map(|i| ((i % 100) as f32 - 50.0) / 100.0)
        .collect();
    Source::new(name, 44_100, 1, v).unwrap()
}

fn kit() -> (Project, SourceId) {
    let s = tone(20_000, "Break 92");
    let id = s.id();
    let mut p = Project::new();
    p.add_source(s.clone());
    for pad in [0, 1] {
        p.set_pad(
            pad,
            Some(Pad::new("Break 92", Layer::new(Region::whole(&s)))),
            None,
        )
        .unwrap();
    }
    p.clear_history();
    (p, id)
}

fn sound_name(p: &Project, id: SourceId) -> &str {
    &p.source(id).unwrap().name
}

#[test]
fn a_pad_and_its_sound_are_named_in_one_undo_step() {
    let (mut p, id) = kit();
    p.rename(Some((0, "  Amen hit ")), Some((id, "Amen")))
        .unwrap();
    assert_eq!(p.pad(0).unwrap().name, "Amen hit");
    assert_eq!(sound_name(&p, id), "Amen");
    // The other pad keeps its own name; the sound is named for both.
    assert_eq!(p.pad(1).unwrap().name, "Break 92");
    let (done, _) = p.history();
    assert_eq!(done.len(), 1);
    assert_eq!(
        (done[0].pads, done[0].named, done[0].first_pad),
        (1, 1, Some(0))
    );
    assert!(p.undo());
    assert_eq!(p.pad(0).unwrap().name, "Break 92");
    assert_eq!(sound_name(&p, id), "Break 92");
    assert!(p.redo());
    assert_eq!(p.pad(0).unwrap().name, "Amen hit");
    assert_eq!(sound_name(&p, id), "Amen");
}

#[test]
fn one_name_alone_and_a_name_unchanged() {
    let (mut p, id) = kit();
    p.rename(None, Some((id, "Amen"))).unwrap();
    let (done, _) = p.history();
    assert_eq!((done[0].pads, done[0].named), (0, 1));
    // The same name again: no step.
    p.rename(Some((0, "Break 92")), Some((id, "Amen ")))
        .unwrap();
    assert_eq!(p.history().0.len(), 1);
    p.rename(Some((1, "Snare")), None).unwrap();
    assert_eq!(p.pad(1).unwrap().name, "Snare");
    assert_eq!(p.history().0.len(), 2);
}

#[test]
fn an_empty_name_or_a_pad_or_sound_not_there_is_refused() {
    let (mut p, id) = kit();
    let rev = p.revision();
    assert_eq!(p.rename(Some((0, "   ")), None), Err(ModelError::BadName));
    assert_eq!(
        p.rename(Some((0, "Kick")), Some((id, ""))),
        Err(ModelError::BadName)
    );
    assert_eq!(p.rename(Some((5, "Kick")), None), Err(ModelError::BadPad));
    let stray = tone(1_000, "Stray");
    assert_eq!(
        p.rename(None, Some((stray.id(), "Stray"))),
        Err(ModelError::UnknownSource)
    );
    // Nothing changed, not even the pad named before the bad sound.
    assert_eq!(p.pad(0).unwrap().name, "Break 92");
    assert_eq!(p.revision(), rev);
}
