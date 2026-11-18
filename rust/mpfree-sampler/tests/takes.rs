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
