//! Takes kept on the Sample page, not on a pad yet (the one Sample page's
//! decision 1; mpfree-claude `sampler/onepage/take-slot-check.md`): they
//! have no pad number, so nothing that walks the pads sees them; keeping,
//! editing, putting one on a pad and letting it go are each one undo
//! step; they save and open with the kit.

use mpfree_sampler::model::*;
use mpfree_sampler::save::{from_text, to_text};
use mpfree_sampler::wav;

const RATE: u32 = 44_100;

fn sound(frames: usize, seed: u32, name: &str) -> Source {
    let mut x = seed;
    let data = (0..frames * 2).map(|_| {
        x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (x >> 8) as f32 / (1u32 << 24) as f32 - 0.5
    });
    Source::new(name, RATE, 2, data.collect()).unwrap()
}

fn take_of(s: &Source, name: &str) -> Pad {
    Pad::new(name, Layer::new(Region::whole(s)))
}

#[test]
fn a_take_is_kept_edited_put_on_a_pad_and_undone_a_step_at_a_time() {
    let a = sound(4_000, 1, "Take 1");
    let b = sound(6_000, 2, "Take 2");
    let mut p = Project::new();
    p.add_source(a.clone());
    p.add_source(b.clone());
    p.set_pad(0, Some(take_of(&a, "Kick")), None).unwrap();
    // Kept: no pad changes, and nothing that walks the pads sees it.
    assert_eq!(p.add_take(take_of(&a, "Take 1")), Ok(0));
    assert_eq!(p.add_take(take_of(&b, "Take 2")), Ok(1));
    // A take's sound counts as used by it, and by no pad.
    assert_eq!((p.users_of(b.id()), p.takes_using(b.id())), (vec![], 1));
    assert_eq!((p.users_of(a.id()), p.takes_using(a.id())), (vec![0], 1));
    assert_eq!(p.takes().len(), 2);
    assert_eq!((0..PADS).filter(|&i| p.pad(i).is_some()).count(), 1);
    let last = p.history().0.last().copied().unwrap();
    assert!(last.takes && last.pads == 0, "{last:?}");
    // A run of edits with one gesture is one step.
    for k in 1..=5 {
        p.edit_take(1, Some(7), |t| t.amp.attack_ms = k as f32)
            .unwrap();
    }
    assert_eq!(p.take(1).unwrap().amp.attack_ms, 5.0);
    assert!(p.undo());
    assert_ne!(p.take(1).unwrap().amp.attack_ms, 5.0);
    assert!(p.redo());
    // A bad edit is refused and changes nothing.
    assert_eq!(
        p.edit_take(1, None, |t| t.polyphony = 0),
        Err(ModelError::BadPad)
    );
    assert_eq!(p.edit_take(9, None, |_| {}), Err(ModelError::BadTake));
    // Put on pad A6: the pad has it, the page doesn't; one undo step
    // takes both back.
    p.put_take_on(1, 5).unwrap();
    assert_eq!(p.pad(5).unwrap().name, "Take 2");
    assert_eq!(p.pad(5).unwrap().amp.attack_ms, 5.0);
    assert_eq!(p.takes().len(), 1);
    assert!(p.undo());
    assert!(p.pad(5).is_none());
    assert_eq!(p.takes().len(), 2);
    assert!(p.redo());
    assert_eq!(p.pad(5).unwrap().name, "Take 2");
    // Onto a pad with a sound: it replaces it, and undo brings it back.
    p.put_take_on(0, 0).unwrap();
    assert_eq!(p.pad(0).unwrap().name, "Take 1");
    assert!(p.undo());
    assert_eq!(p.pad(0).unwrap().name, "Kick");
    // Let go, and back.
    p.remove_take(0).unwrap();
    assert!(p.takes().is_empty());
    assert!(p.undo());
    assert_eq!(p.take(0).unwrap().name, "Take 1");
    assert_eq!(p.put_take_on(0, PADS), Err(ModelError::BadPad));
    assert_eq!(p.remove_take(3), Err(ModelError::BadTake));
}

#[test]
fn a_take_needs_its_sound_in_the_kit_and_the_page_keeps_at_most_32() {
    let a = sound(1_000, 3, "Take");
    let mut p = Project::new();
    assert_eq!(
        p.add_take(take_of(&a, "Take")),
        Err(ModelError::UnknownSource)
    );
    p.add_source(a.clone());
    for _ in 0..MAX_TAKES {
        p.add_take(take_of(&a, "Take")).unwrap();
    }
    assert_eq!(p.add_take(take_of(&a, "Take")), Err(ModelError::BadTake));
    assert_eq!(p.takes().len(), MAX_TAKES);
}

#[test]
fn takes_save_and_open_with_the_kit() {
    let a = sound(3_000, 4, "Take 1");
    let mut p = Project::new();
    p.add_source(a.clone());
    let mut t = take_of(&a, "Take 1");
    t.tune_cents = -200;
    p.add_take(t).unwrap();
    let text = to_text(&[("Kit", &p)], 0, &wav::file_name);
    assert!(text.lines().any(|l| l == "kit.0.takes = 1"), "{text}");
    assert!(
        text.lines().any(|l| l == "kit.0.take.0.name = Take 1"),
        "{text}"
    );
    let bytes = wav::bytes(&a);
    let open = |text: &str| {
        from_text(text, &mut |_| wav::read("x", &bytes))
            .unwrap()
            .kits
            .remove(0)
            .1
    };
    let q = open(&text);
    assert_eq!(q.takes(), p.takes());
    assert!(!q.can_undo(), "opening is not a step");
    // A kit saved before takes opens with none.
    let old: String = text
        .lines()
        .filter(|l| !l.contains(".take"))
        .map(|l| format!("{l}\n"))
        .collect();
    assert!(open(&old).takes().is_empty());
}

#[test]
fn a_new_sound_kept_as_a_take_is_one_undo_step() {
    let mut p = Project::new();
    let s = sound(3_000, 9, "Take 4");
    let i = p
        .add_take_of(s.clone(), |name, layer| {
            assert_eq!(name, "Take 4");
            let mut t = Pad::new(name, layer);
            t.level_db = -3.0;
            t
        })
        .unwrap();
    assert_eq!((i, p.sources().len()), (0, 1));
    assert_eq!(p.take(0).unwrap().level_db, -3.0);
    assert_eq!(p.take(0).unwrap().layers[0].region.end, 3_000);
    assert!(p.undo());
    assert!(p.takes().is_empty() && p.sources().is_empty(), "one step");
    assert!(p.redo());
    assert_eq!(p.takes().len(), 1);
    // A take of another sound, or a wrong one, is refused and adds nothing.
    let other = sound(100, 3, "Other");
    let bad = p.add_take_of(sound(200, 4, "Bad"), |n, _| take_of(&other, n));
    assert_eq!(bad, Err(ModelError::UnknownSource));
    let bad = p.add_take_of(sound(200, 5, "Bad"), |n, l| {
        let mut t = Pad::new(n, l);
        t.polyphony = 0;
        t
    });
    assert_eq!(bad, Err(ModelError::BadPad));
    assert_eq!((p.sources().len(), p.takes().len()), (1, 1));
}

/// A take recorded in bars is warped, as a pad would be: its stretched copy
/// is wanted at the tempo like a pad's, so it plays in time on the page.
#[test]
fn a_warped_take_wants_its_stretched_copy() {
    use mpfree_sampler::warp::Warps;
    let s = sound(44_100, 6, "Take 5");
    let mut p = Project::new();
    p.add_take_of(s, |name, mut layer| {
        layer.region.warp = Some(Warp::new(90.0));
        Pad::new(name, layer)
    })
    .unwrap();
    let w = Warps::new(120.0);
    let wanted = w.wanted(&p);
    assert_eq!(wanted.len(), 1);
    assert_eq!(w.missing(&p), wanted);
    let pin_free = Warps::spec(&p, &wanted[0]);
    assert!(pin_free.is_ok());
}

#[test]
fn a_takes_slices_go_on_pads_and_the_take_leaves_the_page_in_one_undo_step() {
    let a = sound(8_000, 3, "Take 1");
    let mut p = Project::new();
    p.add_source(a.clone());
    p.set_pad(2, Some(take_of(&a, "Snare")), None).unwrap();
    let mut t = take_of(&a, "Take 1");
    t.amp.attack_ms = 12.0;
    p.add_take(t).unwrap();
    let steps = p.history().0.len();
    let c = ChopSet::from_markers("Take 1", &a, ChopMode::Grid, [2_000, 4_000, 6_000]).unwrap();
    // The take is each slice's template, and goes from the page.
    let chop = p
        .chop_take(0, |p, take| p.chop_to_pads(c.clone(), 0, 4, 0, Some(take)))
        .unwrap();
    assert_eq!(p.chop_sets.get(chop).map(|c| c.slices()), Some(4));
    assert!(p.takes().is_empty());
    for k in 0..4 {
        let pad = p.pad(k).unwrap();
        assert_eq!(pad.name, format!("Take 1 {}", k + 1));
        assert_eq!(pad.amp.attack_ms, 12.0);
        let r = &pad.layers[0].region;
        assert_eq!((r.start, r.end), (k as u64 * 2_000, (k as u64 + 1) * 2_000));
    }
    // One step: undo takes the slices off (pad 3 had Snare) and brings
    // the take back.
    assert_eq!(p.history().0.len(), steps + 1);
    assert!(p.undo());
    assert_eq!(p.takes().len(), 1);
    assert_eq!(p.take(0).unwrap().name, "Take 1");
    assert!(p.pad(0).is_none() && p.pad(1).is_none() && p.pad(3).is_none());
    assert_eq!(p.pad(2).unwrap().name, "Snare");
    assert!(p.redo());
    assert!(p.takes().is_empty());
    assert_eq!(p.pad(2).unwrap().name, "Take 1 3");
    // A put that fails changes nothing, and the take stays.
    assert!(p.undo());
    let before = p.history().0.len();
    let bad = p.chop_take(0, |p, take| {
        p.chop_to_pads(c.clone(), 0, 4, PADS - 2, Some(take))
    });
    assert_eq!(bad, Err(ModelError::BadSlice));
    assert_eq!((p.takes().len(), p.history().0.len()), (1, before));
    assert_eq!(p.chop_take(5, |_, _| Ok(())), Err(ModelError::BadTake));
}
