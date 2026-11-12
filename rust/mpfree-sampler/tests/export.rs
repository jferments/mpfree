//! A pad exported as files (`export::pad`) and read back from them
//! (`export::read_sidecar`, `Project::put_pad_with`): the sounds, their
//! loop and slice points for other software, and the whole pad for MPFree.

use std::collections::HashMap;

use mpfree_sampler::export::{self, Export, SIDECAR};
use mpfree_sampler::model::*;
use mpfree_sampler::wav;

const RATE: u32 = 44_100;

fn noise(frames: usize, channels: u8, seed: u32, name: &str) -> Source {
    let mut x = seed;
    let data = (0..frames * usize::from(channels)).map(|_| {
        x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (x >> 8) as f32 / (1u32 << 24) as f32 - 0.5
    });
    Source::new(name, RATE, channels, data.collect()).unwrap()
}

/// A kit with pad 4 looped, keyed and on two layers of two sounds.
fn kit() -> (Project, Source, Source) {
    let a = noise(8_000, 2, 1, "Break 92");
    let b = noise(3_000, 1, 2, "Ghost");
    let mut p = Project::new();
    p.add_source(a.clone());
    p.add_source(b.clone());
    let mut r = Region::whole(&a);
    r.start = 100;
    r.end = 7_900;
    r.sustain_loop = Some(Loop {
        start: 1_000,
        end: 6_000,
        crossfade: 0,
        mode: LoopMode::PingPong,
    });
    r.mode = PlayMode::Loop;
    r.tune_cents = -200;
    let mut soft = Layer::new(r);
    soft.vel_hi = 80;
    let mut hard = Layer::new(Region::whole(&b));
    hard.vel_lo = 81;
    let mut pad = Pad::new("Break/92", soft);
    pad.layers.push(hard);
    pad.chokes = vec![5];
    pad.keys = Some(Keys {
        lo: 40,
        hi: 60,
        root: 48,
    });
    pad.amp.attack_ms = 12.5;
    p.set_pad(3, Some(pad), None).unwrap();
    (p, a, b)
}

fn file<'a>(e: &'a Export, name: &str) -> &'a [u8] {
    &e.files
        .iter()
        .find(|f| f.0 == name)
        .unwrap_or_else(|| panic!("{name}"))
        .1
}

#[test]
fn a_pad_becomes_its_sounds_a_sidecar_and_its_slices() {
    let (p, a, b) = kit();
    // A cut outside the region and one twice are dropped.
    let e = export::pad(&p, 3, 0, &[3_000, 50, 1_500, 3_000]).unwrap();
    assert_eq!(e.name, "Break 92");
    let names: Vec<&str> = e.files.iter().map(|f| f.0.as_str()).collect();
    assert_eq!(
        names,
        [
            "Break 92.wav",
            "Break 92 layer 2.wav",
            "Break 92.mpfree",
            "Break 92 01.wav",
            "Break 92 02.wav",
            "Break 92 03.wav"
        ]
    );
    assert_eq!(e.slices, 3);
    // The sounds, whole and bit for bit, the chopped one with its loop,
    // root note and slice points for other software.
    let main = file(&e, "Break 92.wav");
    assert_eq!(wav::read("x", main).unwrap().id(), a.id());
    assert_eq!(
        wav::read("x", file(&e, "Break 92 layer 2.wav"))
            .unwrap()
            .id(),
        b.id()
    );
    let m = wav::marks(main);
    assert_eq!(m.root, Some(48));
    assert_eq!(m.loop_, Some((1_000, 6_000, true)));
    assert_eq!(
        m.cues,
        [
            (1_500, "Slice 2".to_string()),
            (3_000, "Slice 3".to_string())
        ]
    );
    assert_eq!(
        wav::marks(file(&e, "Break 92 layer 2.wav")),
        wav::Marks::default()
    );
    // Each slice is its frames of the sound as they are.
    let s2 = wav::read("x", file(&e, "Break 92 02.wav")).unwrap();
    assert_eq!((s2.channels(), s2.rate(), s2.frames()), (2, RATE, 1_500));
    assert_eq!(&s2.data()[..], &a.data()[1_500 * 2..3_000 * 2]);
    let s3 = wav::read("x", file(&e, "Break 92 03.wav")).unwrap();
    assert_eq!(s3.frames(), 7_900 - 3_000);
    // Unchopped: the sounds and the sidecar only.
    let e = export::pad(&p, 3, 0, &[]).unwrap();
    assert_eq!((e.files.len(), e.slices), (3, 0));
    assert!(wav::marks(file(&e, "Break 92.wav")).cues.is_empty());
    assert_eq!(
        export::pad(&p, 2, 0, &[]).unwrap_err(),
        "the pad has no sound"
    );
    assert_eq!(
        export::pad(&p, 3, 2, &[]).unwrap_err(),
        "the pad has no such layer"
    );
}

#[test]
fn the_sidecar_brings_the_pad_back_as_it_was_in_one_undo_step() {
    let (p, a, b) = kit();
    let e = export::pad(&p, 3, 0, &[1_500, 3_000]).unwrap();
    let files: HashMap<String, Vec<u8>> = e.files.iter().cloned().collect();
    let text = String::from_utf8(files[&format!("Break 92.{SIDECAR}")].clone()).unwrap();
    let mut reads = Vec::new();
    let o = export::read_sidecar(&text, &mut |name| {
        reads.push(name.to_string());
        let b = files.get(name).ok_or("missing")?;
        wav::read(name, b)
    })
    .unwrap();
    assert_eq!(reads, ["Break 92.wav", "Break 92 layer 2.wav"]);
    assert!(o.notes.is_empty(), "{:?}", o.notes);
    let mut want = p.pad(3).unwrap().clone();
    want.chokes.clear();
    assert_eq!(o.pad, want, "every setting, but the pads it stopped");
    let ids: Vec<SourceId> = o.sources.iter().map(Source::id).collect();
    assert_eq!(ids, [a.id(), b.id()]);
    assert_eq!(o.chops.len(), 1);
    assert_eq!(o.chops[0].markers(), [1_500, 3_000]);
    assert_eq!(o.chops[0].source, a.id());
    // Onto another kit's pad 8: one undo takes it all away again.
    let mut k = Project::new();
    k.put_pad_with(7, o.sources.clone(), o.pad.clone(), o.chops.clone())
        .unwrap();
    assert_eq!(k.pad(7), Some(&want));
    assert_eq!((k.sources().len(), k.chop_sets.len()), (2, 1));
    assert!(k.undo());
    assert!(k.pad(7).is_none());
    assert_eq!((k.sources().len(), k.chop_sets.len()), (0, 0));
    assert!(!k.can_undo());
    // A sound the kit has already is used as it is.
    let mut k = Project::new();
    k.add_source(a.clone());
    k.put_pad_with(0, o.sources.clone(), o.pad.clone(), Vec::new())
        .unwrap();
    assert_eq!(k.sources().len(), 2);
    // A pad that doesn't fit changes nothing.
    let mut bad = o.pad.clone();
    bad.polyphony = 0;
    let mut k = Project::new();
    assert!(k
        .put_pad_with(0, o.sources.clone(), bad, Vec::new())
        .is_err());
    assert!(k
        .put_pad_with(PADS, o.sources.clone(), o.pad.clone(), Vec::new())
        .is_err());
    let orphan = ChopSet::grid("x", &noise(100, 1, 9, "other"), 2).unwrap();
    assert!(k
        .put_pad_with(0, o.sources.clone(), o.pad.clone(), vec![orphan])
        .is_err());
    assert_eq!(k.sources().len(), 0);
    assert!(!k.can_undo());
}

#[test]
fn a_sidecar_without_its_sounds_or_not_one_says_why() {
    let (p, _, _) = kit();
    let e = export::pad(&p, 3, 0, &[]).unwrap();
    let text = String::from_utf8(e.files[2].1.clone()).unwrap();
    // The second layer's sound gone: the pad comes back with one layer.
    let o = export::read_sidecar(&text, &mut |name| match name {
        "Break 92.wav" => wav::read(name, &e.files[0].1),
        _ => Err("not found".into()),
    })
    .unwrap();
    assert_eq!(o.pad.layers.len(), 1);
    assert_eq!(o.notes.len(), 2, "{:?}", o.notes);
    assert!(o.notes[0].contains("Break 92 layer 2.wav left out (not found)"));
    // No sounds at all: no pad.
    let err = export::read_sidecar(&text, &mut |_| Err("not found".into())).unwrap_err();
    assert!(err.contains("left out"), "{err}");
    assert!(export::read_sidecar("hello", &mut |_| Err("no".into())).is_err());
}
