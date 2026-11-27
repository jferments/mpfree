//! Saving the kits (`save::to_text`, their sounds through `wav`) and
//! opening them again: every setting comes back to the bit, each sound is
//! read once, and what can't be used is left out with a note while the
//! rest opens.

use std::collections::HashMap;

use mpfree_sampler::dsp::{Ahdsr, Filter, FilterKind};
use mpfree_sampler::model::*;
use mpfree_sampler::save::{from_text, to_text, FILTERS_VERSION, FORMAT};
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
        drive_db: 0.0,
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
    pad.key_fade = 3;
    pad.vel_fade = 12;
    pad.links = vec![3, 5];
    pad.latch = true;
    pad.vintage = true;
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
        scan_tempo: None,
        sync: Some(TempoSync {
            frames_per_beat: 28_604.3,
            grains_per_beat: Some(8.0),
            size_beats: None,
        }),
        timing: 0.65,
        size_spray: 0.45,
        level_spray: 0.7,
        spray_shape: GrainSprayShape::Peaked,
        lfo: GrainLfo {
            shape: LfoShape::Drift,
            rate_hz: 3.5,
            beats: Some(0.75),
            target: MotionTarget::Position,
            depth: -0.25,
        },
        env: GrainEnv {
            attack_ms: 250.0,
            decay_ms: 4_000.0,
            sustain: 0.3,
            target: MotionTarget::Size,
            depth: 0.6,
        },
        filter: GrainFilter {
            kind: GrainFilterKind::Band,
            cutoff_hz: 1_250.0,
            resonance: 0.6,
            spray: 0.35,
        },
        quality: GrainQuality::Tape,
        align: true,
    };
    g.grain_mode = GrainMode::Repeat;
    g.grain_keys = GrainKeys {
        size: true,
        scan: false,
    };
    g.grain_glide_ms = 250.0;
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
    let notes = ChopSet::from_markers("Notes", &c, ChopMode::Pitch, [6_000, 13_500]);
    p.add_chop_set(notes.unwrap()).unwrap();
    // The Chop tab's slices on the break, kept as last left.
    let kept = ChopSet::from_markers(KEPT_CHOP, &a, ChopMode::Beat, [2_000, 9_000, 15_500]);
    p.keep_chop(kept.unwrap()).unwrap();
    let mut m = *p.master();
    m.mix = 0.35;
    m.grain.window = GrainWindow::Shape { shape: 0.85 };
    m.grain.density = 44.0;
    m.grain.lfo.target = MotionTarget::Cutoff;
    m.grain.filter.kind = GrainFilterKind::High;
    m.grain.quality = GrainQuality::LoFi;
    p.set_master(m, None).unwrap();
    p.set_pads_play(PadsPlay::Levels {
        pad: 5,
        by: Levels::Cutoff,
    })
    .unwrap();
    p.set_keys_play(Some(KeysPlay::Both));
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
    assert_eq!(a.kept_chops(), b.kept_chops());
    assert_eq!(a.master(), b.master());
    assert_eq!(a.pads_play(), b.pads_play());
    assert_eq!(a.keys_play_picked(), b.keys_play_picked());
    assert_eq!(a.takes(), b.takes());
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
    assert!(pa.sources()[0].data().ptr_eq(pb.sources()[0].data()));
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

/// The pad filters' version (the filter gap plan's F2): a save says which
/// filters it was made with, and a kit saved before them whose pads use a
/// filter that changed opens as saved, with a note that its filtered pads
/// may sound different. One without such a pad, or saved now, opens
/// without one. F4's band-pass and F6's Ladder have notes of their own.
#[test]
fn a_kit_saved_before_the_filter_rework_opens_with_a_note() {
    let s = noise(3_000, 2, 4, "Bass");
    let file = wav::file_name(&s);
    let bytes = wav::bytes(&s);
    let old = |kind: &str| {
        format!(
            "{FORMAT}\nkits = 1\nkit.0.name = Old\nkit.0.sources = 1\n\
             kit.0.source.0.file = {file}\nkit.0.pad.0.layers = 1\nkit.0.pad.0.layer.0.source = 0\n\
             kit.0.pad.0.filter.kind = {kind}\nkit.0.pad.0.filter.poles = 4\n\
             kit.0.pad.0.filter.cutoff_hz = 1000\n"
        )
    };
    let open = |text: &str| from_text(text, &mut |_| wav::read("Bass", &bytes)).unwrap();
    let opened = open(&old("low-pass"));
    assert_eq!(opened.notes.len(), 1, "{:?}", opened.notes);
    assert!(opened.notes[0].starts_with("Old: saved before the pad filters were reworked"));
    // The settings are as saved.
    let pad = opened.kits[0].1.pad(0).unwrap();
    assert_eq!(pad.filter.kind, FilterKind::LowPass);
    assert_eq!((pad.filter.poles, pad.filter.cutoff_hz), (4, 1000.0));
    // A Comb plays as it did: no note.
    assert!(open(&old("comb")).notes.is_empty());
    // Saved now, the same kit says its version and opens without a note.
    let (text, files) = save(&[("New", &opened.kits[0].1)], 0);
    assert!(
        text.contains(&format!("\nfilters = {FILTERS_VERSION}\n")),
        "{text}"
    );
    let again = from_text(&text, &mut |f| wav::read("x", &files[f])).unwrap();
    assert!(again.notes.is_empty(), "{:?}", again.notes);

    // Saved with F2's filters (version 2): only a 4-pole band-pass, which
    // F4 changed, gets a note, and it says so.
    let v2 = |kind: &str, poles: u8| {
        old(kind)
            .replace("kits = 1\n", "kits = 1\nfilters = 2\n")
            .replace("filter.poles = 4", &format!("filter.poles = {poles}"))
    };
    let notes = open(&v2("band-pass", 4)).notes;
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(notes[0].starts_with("Old: saved before the band-pass slopes were reworked"));
    for (kind, poles) in [("band-pass", 2), ("low-pass", 4), ("notch", 4)] {
        let notes = open(&v2(kind, poles)).notes;
        assert!(notes.is_empty(), "{kind} {poles}: {notes:?}");
    }
    // Saved before F2, the one note covers it.
    assert_eq!(open(&old("band-pass")).notes.len(), 1);
    assert_eq!(open(&old("ladder")).notes.len(), 1);

    // Saved before F6 (version 2 or 3): a Ladder pad gets a note that the
    // Ladder was rebuilt; the other types none.
    let v3 = |kind: &str| old(kind).replace("kits = 1\n", "kits = 1\nfilters = 3\n");
    for text in [v3("ladder"), v2("ladder", 4)] {
        let notes = open(&text).notes;
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert!(notes[0].starts_with("Old: saved before the Ladder was rebuilt"));
    }
    for kind in ["band-pass", "low-pass", "comb"] {
        let notes = open(&v3(kind)).notes;
        assert!(notes.is_empty(), "{kind}: {notes:?}");
    }
}

/// The slopes and the Peak (the filter gap plan's F4) save and open as
/// set.
#[test]
fn every_slope_and_the_peak_save_and_open() {
    let s = noise(3_000, 2, 4, "Bass");
    let mut p = Project::new();
    p.add_source(s.clone());
    let kinds = [
        (FilterKind::LowPass, 1),
        (FilterKind::LowPass, 3),
        (FilterKind::LowPass, 8),
        (FilterKind::HighPass, 6),
        (FilterKind::BandPass, 8),
        (FilterKind::Peak, 2),
    ];
    for (i, (kind, poles)) in kinds.into_iter().enumerate() {
        let mut pad = Pad::new("p", Layer::new(Region::whole(&s)));
        pad.filter.kind = kind;
        pad.filter.poles = poles;
        pad.filter.resonance = 0.4;
        p.set_pad(i, Some(pad), None).unwrap();
    }
    let (text, files) = save(&[("Slopes", &p)], 0);
    assert!(text.contains("filter.kind = peak"), "{text}");
    let again = from_text(&text, &mut |f| wav::read("x", &files[f])).unwrap();
    assert!(again.notes.is_empty(), "{:?}", again.notes);
    for (i, (kind, poles)) in kinds.into_iter().enumerate() {
        let f = again.kits[0].1.pad(i).unwrap().filter;
        assert_eq!(
            (f.kind, f.poles, f.resonance),
            (kind, poles, 0.4),
            "pad {i}"
        );
    }
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

/// Pad types (the Play, Pads and Sequencer redesign's step 6): an engine's
/// voice and a note out save as their words with their settings, a loop
/// pad's on_launch as a flag, and a kit saved before them opens as Sample
/// pads. The sampler plays only the Sample pads.
#[test]
fn pad_kinds_and_on_launch_save_and_open_and_only_sample_pads_sound() {
    use mpfree_sampler::engine::EngineView;
    let s = noise(3_000, 2, 4, "Sub");
    let mut p = Project::new();
    p.add_source(s.clone());
    let pad = |name: &str, kind: PadKind, on_launch: bool| {
        let mut pd = Pad::new(name, Layer::new(Region::whole(&s)));
        pd.kind = kind;
        pd.on_launch = on_launch;
        pd
    };
    let kinds = [
        pad("Loop", PadKind::Sample, true),
        pad(
            "Sub",
            PadKind::Instrument {
                engine: "poly".into(),
                note: 24,
            },
            false,
        ),
        pad(
            "Volca kick",
            PadKind::MidiOut {
                port: "MIDI Out".into(),
                channel: 10,
                note: 36,
            },
            false,
        ),
    ];
    for (i, k) in kinds.into_iter().enumerate() {
        p.set_pad(i, Some(k), None).unwrap();
    }
    let (text, files) = save(&[("Kinds", &p)], 0);
    for line in [
        "kit.0.pad.0.kind = sample",
        "kit.0.pad.0.on_launch = yes",
        "kit.0.pad.1.kind = instrument",
        "kit.0.pad.1.kind.engine = poly",
        "kit.0.pad.1.kind.note = 24",
        "kit.0.pad.1.on_launch = no",
        "kit.0.pad.2.kind = midi_out",
        "kit.0.pad.2.kind.port = MIDI Out",
        "kit.0.pad.2.kind.channel = 10",
        "kit.0.pad.2.kind.note = 36",
    ] {
        assert!(text.lines().any(|l| l == line), "{line} in\n{text}");
    }
    let read = |text: &str| {
        from_text(text, &mut |f| wav::read("x", &files[f]))
            .unwrap()
            .kits
            .remove(0)
            .1
    };
    assert_same(&read(&text), &p);
    // Out of range, the nearest; without the lines (an older build's), a
    // Sample pad.
    let odd = text
        .replace("pad.2.kind.channel = 10", "pad.2.kind.channel = 40")
        .replace("pad.2.kind.note = 36", "pad.2.kind.note = 200");
    let kind = read(&odd).pad(2).unwrap().kind.clone();
    assert_eq!(
        kind,
        PadKind::MidiOut {
            port: "MIDI Out".into(),
            channel: 16,
            note: 127
        }
    );
    let old: String = text
        .lines()
        .filter(|l| !l.contains(".kind") && !l.contains(".on_launch"))
        .map(|l| format!("{l}\n"))
        .collect();
    let old = read(&old);
    assert!((0..3).all(|i| {
        let pd = old.pad(i).unwrap();
        pd.kind == PadKind::Sample && !pd.on_launch
    }));
    // The sampler plays the Sample pad, not the other two.
    let view = EngineView::build(&p, RATE);
    assert!(view.has_pad(0) && !view.has_pad(1) && !view.has_pad(2));
}

/// A pad's LFO, pitch envelope and velocity on cutoff and pitch (sampler
/// R5 part 4) save and open; a kit saved before them opens with them off.
#[test]
fn lfo_pitch_envelope_and_velocity_routes_save_and_open() {
    let s = noise(3_000, 2, 5, "Pad");
    let mut p = Project::new();
    p.add_source(s.clone());
    let mut pd = Pad::new("Pad", Layer::new(Region::whole(&s)));
    pd.lfo = PadLfo {
        shape: LfoShape::Drift,
        rate_hz: 0.5,
        beats: Some(4.0),
        target: LfoTarget::Pan,
        depth: -0.75,
        retrig: false,
    };
    pd.pitch_env = PitchEnv {
        semitones: -7.0,
        decay_ms: 350.0,
    };
    pd.vel_to_cutoff = 0.5;
    pd.vel_to_pitch = -2.0;
    p.set_pad(3, Some(pd), None).unwrap();
    let (text, files) = save(&[("Moves", &p)], 0);
    for line in [
        "kit.0.pad.3.lfo.shape = drift",
        "kit.0.pad.3.lfo.rate_hz = 0.5",
        "kit.0.pad.3.lfo.beats = 4",
        "kit.0.pad.3.lfo.target = pan",
        "kit.0.pad.3.lfo.depth = -0.75",
        "kit.0.pad.3.lfo.retrig = no",
        "kit.0.pad.3.pitch_env.semitones = -7",
        "kit.0.pad.3.pitch_env.decay_ms = 350",
        "kit.0.pad.3.vel_to_cutoff = 0.5",
        "kit.0.pad.3.vel_to_pitch = -2",
    ] {
        assert!(text.lines().any(|l| l == line), "{line} in\n{text}");
    }
    let read = |text: &str| {
        from_text(text, &mut |f| wav::read("x", &files[f]))
            .unwrap()
            .kits
            .remove(0)
            .1
    };
    assert_same(&read(&text), &p);
    let old: String = text
        .lines()
        .filter(|l| {
            !l.contains(".lfo.")
                && !l.contains(".pitch_env.")
                && !l.contains(".vel_to_c")
                && !l.contains(".vel_to_p")
        })
        .map(|l| format!("{l}\n"))
        .collect();
    let old = read(&old);
    let back = old.pad(3).unwrap();
    assert_eq!(
        (
            back.lfo,
            back.pitch_env,
            back.vel_to_cutoff,
            back.vel_to_pitch
        ),
        (PadLfo::default(), PitchEnv::default(), 0.0, 0.0)
    );
}

/// A pad's Mono, Legato and Glide (sampler R5.5) and Continue (the filter
/// gap plan's F3) save and open; a kit saved before them opens with them
/// off.
#[test]
fn mono_legato_and_glide_save_and_open() {
    let s = noise(3_000, 2, 6, "Bass");
    let mut p = Project::new();
    p.add_source(s.clone());
    let mut pd = Pad::new("Bass", Layer::new(Region::whole(&s)));
    pd.keys = Some(Keys::all(48));
    pd.mono = Mono {
        on: true,
        legato: true,
        glide_ms: 120.0,
        glide_held: true,
        env_continue: true,
    };
    p.set_pad(5, Some(pd), None).unwrap();
    let (text, files) = save(&[("Bass", &p)], 0);
    for line in [
        "kit.0.pad.5.mono.on = yes",
        "kit.0.pad.5.mono.legato = yes",
        "kit.0.pad.5.mono.glide_ms = 120",
        "kit.0.pad.5.mono.glide_held = yes",
        "kit.0.pad.5.mono.env_continue = yes",
    ] {
        assert!(text.lines().any(|l| l == line), "{line} in\n{text}");
    }
    let read = |text: &str| {
        from_text(text, &mut |f| wav::read("x", &files[f]))
            .unwrap()
            .kits
            .remove(0)
            .1
    };
    assert_same(&read(&text), &p);
    let old: String = text
        .lines()
        .filter(|l| !l.contains(".mono."))
        .map(|l| format!("{l}\n"))
        .collect();
    assert_eq!(read(&old).pad(5).unwrap().mono, Mono::default());
}

/// What moves a pad's filter per hit (the filter gap plan's F3) saves and
/// opens: key tracking, velocity on the cutoff (below 0 too), on the
/// envelope's depth and on its attack, and the random on the cutoff and
/// resonance. A kit saved before them opens with none.
#[test]
fn the_filter_per_hit_settings_save_and_open() {
    let s = noise(3_000, 2, 9, "Keys");
    let mut p = Project::new();
    p.add_source(s.clone());
    let mut pd = Pad::new("Keys", Layer::new(Region::whole(&s)));
    pd.filter.kind = FilterKind::LowPass;
    pd.filter.cutoff_hz = 900.0;
    pd.vel_to_cutoff = -0.5;
    pd.key_track = 0.75;
    pd.vel_to_env = 0.25;
    pd.vel_to_attack = 0.5;
    pd.cutoff_random = 0.125;
    pd.res_random = 1.0;
    // Bypassed (F5): saved, every setting kept.
    pd.filter_bypass = true;
    p.set_pad(3, Some(pd), None).unwrap();
    let (text, files) = save(&[("Keys", &p)], 0);
    for line in [
        "kit.0.pad.3.filter_bypass = yes",
        "kit.0.pad.3.vel_to_cutoff = -0.5",
        "kit.0.pad.3.key_track = 0.75",
        "kit.0.pad.3.vel_to_env = 0.25",
        "kit.0.pad.3.vel_to_attack = 0.5",
        "kit.0.pad.3.cutoff_random = 0.125",
        "kit.0.pad.3.res_random = 1",
    ] {
        assert!(text.lines().any(|l| l == line), "{line} in\n{text}");
    }
    let read = |text: &str| {
        from_text(text, &mut |f| wav::read("x", &files[f]))
            .unwrap()
            .kits
            .remove(0)
            .1
    };
    assert_same(&read(&text), &p);
    let new = [
        "filter_bypass",
        "key_track",
        "vel_to_env",
        "vel_to_attack",
        "cutoff_random",
        "res_random",
    ];
    let old: String = text
        .lines()
        .filter(|l| !new.iter().any(|k| l.contains(&format!(".{k} = "))))
        .map(|l| format!("{l}\n"))
        .collect();
    let back = read(&old);
    let b = back.pad(3).unwrap();
    assert!(!b.filter_bypass, "an older kit opens with the filter on");
    assert_eq!(
        (
            b.key_track,
            b.vel_to_env,
            b.vel_to_attack,
            b.cutoff_random,
            b.res_random,
            b.vel_to_cutoff
        ),
        (0.0, 0.0, 0.0, 0.0, 0.0, -0.5)
    );
}

#[test]
fn the_new_filters_and_drive_save_and_open() {
    let s = noise(3_000, 2, 7, "Pad");
    let mut p = Project::new();
    p.add_source(s.clone());
    for (i, (kind, poles, drive_db)) in [
        (FilterKind::Notch, 4, 0.0),
        (FilterKind::Ladder, 4, 12.5),
        (FilterKind::Vowel, 2, 0.0),
        (FilterKind::Comb, 2, 0.0),
    ]
    .into_iter()
    .enumerate()
    {
        let mut pd = Pad::new("Pad", Layer::new(Region::whole(&s)));
        pd.filter = Filter {
            kind,
            poles,
            cutoff_hz: 900.0,
            resonance: 0.6,
            env_octaves: 1.0,
            drive_db,
        };
        p.set_pad(i, Some(pd), None).unwrap();
    }
    let (text, files) = save(&[("Pads", &p)], 0);
    for line in [
        "kit.0.pad.0.filter.kind = notch",
        "kit.0.pad.1.filter.kind = ladder",
        "kit.0.pad.1.filter.drive_db = 12.5",
        "kit.0.pad.2.filter.kind = vowel",
        "kit.0.pad.3.filter.kind = comb",
    ] {
        assert!(text.lines().any(|l| l == line), "{line} in\n{text}");
    }
    let read = |text: &str| {
        from_text(text, &mut |f| wav::read("x", &files[f]))
            .unwrap()
            .kits
            .remove(0)
            .1
    };
    assert_same(&read(&text), &p);
    // An older file has no Drive: none.
    let old: String = text
        .lines()
        .filter(|l| !l.contains(".drive_db"))
        .map(|l| format!("{l}\n"))
        .collect();
    assert_eq!(read(&old).pad(1).unwrap().filter.drive_db, 0.0);
}
