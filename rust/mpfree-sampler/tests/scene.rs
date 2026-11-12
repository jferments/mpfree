//! Scenes A and B (`scene`): stored with the kit and saved with it, the
//! crossfader's values heard on the pads, and Return putting the pads back
//! as one undo step.

use mpfree_sampler::engine::{Engine, EngineView, Event, PadField};
use mpfree_sampler::model::*;
use mpfree_sampler::save::{from_text, to_text};
use mpfree_sampler::scene::{self, Scene};
use mpfree_sampler::wav;

const RATE: u32 = 48_000;

fn tone(name: &str) -> Source {
    let data = (0..20_000).map(|i| (i as f32 * 0.05).sin() * 0.5).collect();
    Source::new(name, RATE, 1, data).unwrap()
}

/// Two pads, as scene A: pad 1 at -6 dB, pad 2 tuned up.
fn kit() -> (Project, Source) {
    let s = tone("Tone");
    let mut p = Project::new();
    p.add_source(s.clone());
    let mut a = Pad::new("Low", Layer::new(Region::whole(&s)));
    a.level_db = -6.0;
    p.set_pad(0, Some(a), None).unwrap();
    let mut b = Pad::new("High", Layer::new(Region::whole(&s)));
    b.tune_cents = 700;
    p.set_pad(1, Some(b), None).unwrap();
    (p, s)
}

fn peak(e: &mut Engine, frames: usize) -> f32 {
    let (mut l, mut r) = (vec![0.0; 128], vec![0.0; 128]);
    let mut peak = 0f32;
    for _ in 0..frames / 128 {
        e.render(&mut l, &mut r);
        peak = l.iter().fold(peak, |m, x| m.max(x.abs()));
    }
    peak
}

#[test]
fn return_puts_the_mangled_pads_back_in_one_undo_step() {
    let (mut p, s) = kit();
    let a = Scene::of(&p);
    // Mangled: pad 1 louder, pad 2 cleared, pad 5 new.
    p.edit_pad(0, None, |q| q.level_db = 3.0).unwrap();
    p.set_pad(1, None, None).unwrap();
    p.set_pad(
        4,
        Some(Pad::new("New", Layer::new(Region::whole(&s)))),
        None,
    )
    .unwrap();
    let mangled: Vec<Option<Pad>> = (0..5).map(|i| p.pad(i).cloned()).collect();
    assert_eq!(p.return_to(&a), (3, 0));
    assert_eq!(Scene::of(&p), a);
    // Again: nothing to do, no undo step.
    assert_eq!(p.return_to(&a), (0, 0));
    assert!(p.undo());
    let back: Vec<Option<Pad>> = (0..5).map(|i| p.pad(i).cloned()).collect();
    assert_eq!(back, mangled, "one undo takes the whole return back");
    // A stored pad whose sound the kit no longer has stays as it is.
    let mut q = Project::new();
    // (A sound is known by what it holds, so this one is other samples.)
    q.add_source(Source::new("Other", RATE, 1, vec![0.25; 1_000]).unwrap());
    assert_eq!(q.return_to(&a), (0, 2));
    assert!(q.undo(), "the sound's adding");
    assert!(!q.can_undo(), "and no step for the return");
}

#[test]
fn the_crossfader_moves_the_pads_from_a_to_b_and_clear_lets_go() {
    let (mut p, _) = kit();
    let a = Scene::of(&p);
    p.edit_pad(0, None, |q| q.level_db = -26.0).unwrap();
    let b = Scene::of(&p);
    // The engine plays the kit as B is; the crossfader at A plays pad 1
    // 20 dB louder, half way 10 dB.
    let mut e = Engine::new(RATE, 8);
    e.set_view(EngineView::build(&p, RATE));
    let hit = |e: &mut Engine, values: &[(u8, PadField, f32)]| {
        for &(pad, field, value) in values {
            e.apply(Event::PadLive { pad, field, value });
        }
        e.apply(Event::NoteOn {
            pad: 0,
            velocity: 127,
        });
        let level = peak(e, 2_048);
        e.apply(Event::AllOff);
        peak(e, 4_096);
        level
    };
    let own = hit(&mut e, &[]);
    let at_a = hit(&mut e, &scene::morph(&a, &b, 0.0));
    let half = hit(&mut e, &scene::morph(&a, &b, 0.5));
    let at_b = hit(&mut e, &scene::morph(&a, &b, 1.0));
    let db = |x: f32, y: f32| 20.0 * (x / y).log10();
    assert!((db(at_a, own) - 20.0).abs() < 0.5, "{}", db(at_a, own));
    assert!((db(half, own) - 10.0).abs() < 0.5, "{}", db(half, own));
    assert!(db(at_b, own).abs() < 0.5);
    // Cleared, the pad plays its own again.
    let back = hit(&mut e, &scene::morph(&a, &b, 0.0));
    assert!(back > own * 5.0);
    let cleared = hit(&mut e, &scene::clear(&a, &b));
    assert!(db(cleared, own).abs() < 0.5);
    // Only what differs moves: pad 2 is the same in both.
    assert!(scene::morph(&a, &b, 0.3).iter().all(|v| v.0 == 0));
}

#[test]
fn scenes_save_with_the_kit_and_open_again() {
    let (mut p, s) = kit();
    p.scenes[0] = Some(Scene::of(&p));
    p.edit_pad(1, None, |q| q.pan = -0.5).unwrap();
    p.scenes[1] = Some(Scene::of(&p));
    let text = to_text(&[("Kit", &p)], 0, &wav::file_name);
    assert!(text.contains("kit.0.scene.0 = stored"));
    let file = wav::file_name(&s);
    let bytes = wav::bytes(&s);
    let o = from_text(&text, &mut |name| {
        assert_eq!(name, file);
        wav::read(name, &bytes)
    })
    .unwrap();
    assert!(o.notes.is_empty(), "{:?}", o.notes);
    let q = &o.kits[0].1;
    assert_eq!(q.scenes, p.scenes);
    // No scenes, none back; a scene whose sound is gone says so.
    let mut plain = p.clone();
    plain.scenes = [None, None];
    let text = to_text(&[("Kit", &plain)], 0, &wav::file_name);
    let o = from_text(&text, &mut |n| wav::read(n, &bytes)).unwrap();
    assert_eq!(o.kits[0].1.scenes, [None, None]);
    let text = to_text(&[("Kit", &p)], 0, &wav::file_name);
    let o = from_text(&text, &mut |_| Err("gone".into())).unwrap();
    assert!(o
        .notes
        .iter()
        .any(|n| n == "Kit: scene A lost 2 pad(s) (their sounds are missing)"));
}
