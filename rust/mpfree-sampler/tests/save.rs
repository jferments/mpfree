//! Saving the kits (`save::to_text`, their sounds through `wav`) and
//! opening them again: every setting comes back to the bit, each sound is
//! read once, and what can't be used is left out with a note while the
//! rest opens.

use std::collections::HashMap;

use mpfree_sampler::dsp::{Ahdsr, Filter, FilterKind};
use mpfree_sampler::model::*;
use mpfree_sampler::save::{from_text, to_text, FORMAT};
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

/// A 16-bit sound, as a line-input take or an import is.
fn pcm(frames: usize, name: &str) -> Source {
    let data = (0..frames * 2).map(|i| ((i * 37 % 2000) as f32 - 1000.0) / 32_768.0);
    Source::new(name, RATE, 2, data.collect()).unwrap()
}

/// A kit that sets every setting away from its default somewhere.
fn busy_kit() -> Project {
    let a = noise(40_000, 2, 1, "Break 92");
    let b = noise(12_000, 1, 2, "Mono hit");
    let c = pcm(20_000, "Take 3");
    let mut p = Project::new();
    for s in [&a, &b, &c] {
        p.add_source(s.clone());
    }
    // A looped, warped, faded region on two velocity layers.
    let mut r = Region::whole(&a);
    r.start = 1_000;
    r.end = 30_000;
    r.sustain_loop = Some(Loop {
        start: 4_000,
        end: 20_000,
        crossfade: 500,
        mode: LoopMode::PingPong,
    });
    r.fade_in = Fade {
        frames: 300,
        curve: FadeCurve::Log,
    };
    r.fade_out = Fade {
        frames: 700,
        curve: FadeCurve::Exp,
    };
    r.gain_db = -3.25;
    r.reverse = true;
    r.channels = ChannelPick::Left;
    r.tune_cents = -150;
    r.mode = PlayMode::Loop;
    r.warp = Some(Warp {
        bpm: 92.5,
        method: WarpMethod::Repitch,
        pins: vec![
            Pin {
                frame: 8_000,
                beat: 0.25,
            },
            Pin {
                frame: 15_000,
                beat: 0.515625,
            },
        ],
    });
    let mut soft = Layer::new(r.clone());
    soft.vel_hi = 63;
    soft.level_db = -6.0;
    soft.pan = -0.3;
    soft.tune_cents = 12;
    let mut hard = Layer::new(Region::whole(&c));
    hard.vel_lo = 64;
    let mut pad = Pad::new("Break", soft);
    pad.layers.push(hard);
    pad.select = LayerSelect::Cycle;
    pad.polyphony = 3;
    pad.mute_group = Some(4);
    pad.chokes = vec![2, 7];
    pad.amp = Ahdsr {
        attack_ms: 2.5,
        hold_ms: 10.0,
        decay_ms: 120.0,
        sustain: 0.7,
        release_ms: 333.3,
    };
    pad.filter = Filter {
        kind: FilterKind::BandPass,
        poles: 4,
        cutoff_hz: 1234.5,
        resonance: 0.45,
        env_octaves: -1.5,
    };
    pad.filter_env = Ahdsr {
        attack_ms: 1.0,
        hold_ms: 0.0,
        decay_ms: 80.0,
        sustain: 0.2,
        release_ms: 50.0,
    };
    pad.vel_to_level = 0.5;
    pad.vel_to_start = 0.25;
    pad.level_db = -1.5;
    pad.pan = 0.1;
    pad.tune_cents = 700;
    pad.pressure = PressureRoute {
        target: PressureTarget::Filter,
        depth: -0.6,
    };
    pad.keys = Some(Keys {
        lo: 36,
        hi: 59,
        root: 48,
    });
    pad.wow = 0.3;
    p.set_pad(0, Some(pad), None).unwrap();
    // A granular pad with every grain setting, synced.
    let mut g = Region::whole(&b);
    g.mode = PlayMode::Granular;
    g.grain = GrainParams {
        position: 0.4,
        spray: 0.2,
        size_ms: 55.5,
        density: 33.0,
        pitch: -7.0,
        pitch_spray: 0.3,
        scale: 0b1010_1101_0101,
        scan: 0.125,
        window: GrainWindow::Tukey { taper: 0.35 },
        reverse: 0.5,
        spread: 0.8,
        gain: 0.9,
        sync: Some(TempoSync {
            frames_per_beat: 28_604.3,
            grains_per_beat: Some(8.0),
            size_beats: None,
        }),
    };
    p.set_pad(5, Some(Pad::new("Cloud", Layer::new(g))), None)
        .unwrap();
    // A keygroup far up the pad list, a toggle and a gate pad.
    let mut t = Region::whole(&c);
    t.mode = PlayMode::Toggle;
    p.set_pad(100, Some(Pad::new("Toggle", Layer::new(t))), None)
        .unwrap();
    let mut gate = Region::whole(&b);
    gate.mode = PlayMode::Gate;
    gate.channels = ChannelPick::MonoSum;
    let mut gp = Pad::new("Gate", Layer::new(gate));
    gp.filter.kind = FilterKind::HighPass;
    p.set_pad(15, Some(gp), None).unwrap();
    // Chop sets, the master effect, 16 Levels.
    let chop = ChopSet::from_markers("Break chops", &a, ChopMode::Transient, [900, 5_000, 21_000]);
    p.add_chop_set(chop.unwrap()).unwrap();
    p.add_chop_set(ChopSet::grid("Even", &c, 4).unwrap())
        .unwrap();
    let mut m = *p.master();
    m.mix = 0.35;
    m.grain.window = GrainWindow::Exponential { decay: 6.0 };
    m.grain.density = 44.0;
    p.set_master(m, None).unwrap();
    p.set_pads_play(PadsPlay::Levels {
        pad: 5,
        by: Levels::Cutoff,
    })
    .unwrap();
    p
}

/// Saves the kits into "files" (name to bytes), as a project's samples/.
fn save(kits: &[(&str, &Project)], kit: usize) -> (String, HashMap<String, Vec<u8>>) {
    let text = to_text(kits, kit, &wav::file_name);
    let mut files = HashMap::new();
    for (_, p) in kits {
        for s in p.sources() {
            files.insert(wav::file_name(s), wav::bytes(s));
        }
    }
    (text, files)
}

/// Every setting of two projects, compared.
fn assert_same(a: &Project, b: &Project) {
    for i in 0..PADS {
        assert_eq!(a.pad(i), b.pad(i), "pad {i}");
    }
    assert_eq!(a.chop_sets, b.chop_sets);
    assert_eq!(a.master(), b.master());
    assert_eq!(a.pads_play(), b.pads_play());
    let ids = |p: &Project| p.sources().iter().map(Source::id).collect::<Vec<_>>();
    assert_eq!(ids(a), ids(b));
    let names = |p: &Project| {
        p.sources()
            .iter()
            .map(|s| s.name.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(names(a), names(b));
}

#[test]
fn a_saved_kit_opens_with_every_setting_to_the_bit() {
    let busy = busy_kit();
    let plain = {
        let s = noise(5_000, 2, 9, "Kick");
        let mut p = Project::new();
        p.add_source(s.clone());
        p.set_pad(
            0,
            Some(Pad::new("Kick", Layer::new(Region::whole(&s)))),
            None,
        )
        .unwrap();
        p
    };
    let (text, files) = save(&[("Boom bap", &busy), ("Kick only", &plain)], 1);
    assert!(text.starts_with(FORMAT));
    let mut reads = Vec::new();
    let opened = from_text(&text, &mut |f| {
        reads.push(f.to_string());
        wav::read("file", files.get(f).ok_or("missing")?)
    })
    .unwrap();
    assert!(opened.notes.is_empty(), "{:?}", opened.notes);
    assert_eq!(opened.kit, 1);
    assert_eq!(opened.kits.len(), 2);
    assert_eq!(opened.kits[0].0, "Boom bap");
    assert_eq!(opened.kits[1].0, "Kick only");
    assert_same(&opened.kits[0].1, &busy);
    assert_same(&opened.kits[1].1, &plain);
    assert!(!opened.kits[0].1.can_undo(), "no undo history");
    // Each file was read once.
    let mut once = reads.clone();
    once.dedup();
    assert_eq!(once.len(), reads.len());
    assert_eq!(reads.len(), 4);
    // Saved again, it is the same text.
    let kits: Vec<(&str, &Project)> = opened.kits.iter().map(|(n, p)| (n.as_str(), p)).collect();
    assert_eq!(to_text(&kits, opened.kit, &wav::file_name), text);
}

#[test]
fn the_same_sound_in_two_kits_is_one_file_read_once() {
    let s = pcm(8_000, "Shared");
    let mut a = Project::new();
    a.put_source(0, s.clone()).unwrap();
    let mut b = Project::new();
    b.put_source(3, s.clone()).unwrap();
    let (text, files) = save(&[("A", &a), ("B", &b)], 0);
    assert_eq!(files.len(), 1);
    // A 16-bit sound is kept as 16-bit.
    assert_eq!(files.values().next().unwrap().len(), 44 + 8_000 * 4);
    let mut reads = 0;
    let opened = from_text(&text, &mut |f| {
        reads += 1;
        wav::read("x", &files[f])
    })
    .unwrap();
    assert_eq!(reads, 1);
    let (pa, pb) = (&opened.kits[0].1, &opened.kits[1].1);
    assert_eq!(pa.pad(0).unwrap().name, "Shared");
    assert_eq!(pb.pad(3).unwrap().layers[0].region.source, s.id());
    // They share the one copy in memory.
    assert!(std::sync::Arc::ptr_eq(
        pa.sources()[0].data(),
        pb.sources()[0].data()
    ));
}

#[test]
fn what_cannot_be_used_is_left_out_and_the_rest_opens() {
    let busy = busy_kit();
    let (text, mut files) = save(&[("Boom bap", &busy)], 0);
    // The mono sound's file is gone, and a pad has no voices.
    let mono = busy.sources().iter().find(|s| s.channels() == 1).unwrap();
    files.remove(&wav::file_name(mono));
    let text = text.replace("kit.0.pad.100.polyphony = 8", "kit.0.pad.100.polyphony = 0");
    let opened = from_text(&text, &mut |f| {
        wav::read("x", files.get(f).ok_or("the file is missing")?)
    })
    .unwrap();
    let p = &opened.kits[0].1;
    let notes = opened.notes.join("\n");
    assert!(notes.contains("the file is missing"), "{notes}");
    // The pads on the missing sound (the granular and the gate pad) are
    // left out, saying so; the pad the model refuses too.
    assert!(p.pad(5).is_none() && p.pad(15).is_none());
    assert!(notes.contains("pad 6 left out"), "{notes}");
    assert!(notes.contains("pad 101 (Toggle) left out"), "{notes}");
    assert!(p.pad(100).is_none());
    // The rest is there as saved.
    assert_eq!(p.pad(0), busy.pad(0));
    assert_eq!(p.chop_sets, busy.chop_sets);
    assert_eq!(p.pads_play(), busy.pads_play());
}

#[test]
fn an_older_or_newer_builds_text_still_opens() {
    let s = noise(3_000, 2, 4, "Snare");
    let file = wav::file_name(&s);
    let bytes = wav::bytes(&s);
    // An older build's: hardly anything but the sound; a newer build's:
    // keys this one doesn't know.
    let text = format!(
        "{FORMAT}\n# a comment\nkits = 1\nkit.0.name = Old\nkit.0.sources = 1\n\
         kit.0.source.0.file = {file}\nkit.0.pad.2.layers = 1\nkit.0.pad.2.layer.0.source = 0\n\
         kit.0.pad.2.name = Snare\nkit.0.pad.2.sparkle = 11\nkit.0.pad.2.layer.0.region.mode = teleport\n\
         kit.0.future.thing = yes\n"
    );
    let opened = from_text(&text, &mut |_| wav::read("Snare", &bytes)).unwrap();
    assert!(opened.notes.is_empty(), "{:?}", opened.notes);
    let p = &opened.kits[0].1;
    let pad = p.pad(2).unwrap();
    // Missing settings are the defaults; the unknown mode word keeps one.
    let want = Pad::new("Snare", Layer::new(Region::whole(&s)));
    assert_eq!(pad, &want);
    // Not a sampler part at all.
    assert!(from_text("mpfree-synth 1\nx = 1", &mut |_| Err("no".into())).is_err());
    assert!(from_text("", &mut |_| Err("no".into())).is_err());
    // A count that the text can't hold doesn't run away.
    let opened = from_text(&format!("{FORMAT}\nkits = 99999999999"), &mut |_| {
        Err("no".into())
    })
    .unwrap();
    assert!(opened.kits.len() <= 2);
}

#[test]
fn names_with_line_breaks_stay_on_their_line() {
    let s = pcm(1_000, "Line\nbreak\t= in it");
    let mut p = Project::new();
    p.put_source(0, s.clone()).unwrap();
    let (text, files) = save(&[("Kit\r\nname", &p)], 0);
    let opened = from_text(&text, &mut |f| wav::read("x", &files[f])).unwrap();
    assert_eq!(opened.kits[0].0, "Kit  name");
    assert_eq!(opened.kits[0].1.pad(0).unwrap().name, "Line break = in it");
}
