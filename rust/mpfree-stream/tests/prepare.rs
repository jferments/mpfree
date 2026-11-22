//! Prepared copies, snapshots and readiness (the later phases' P3, step
//! 4).

use std::f64::consts::TAU;
use std::fs;
use std::path::{Path, PathBuf};

use mpfree_stream::format::{decode, layout, Layout, Sample};
use mpfree_stream::fsio::{Fs, StdFs};
use mpfree_stream::prepare::{self, make, target, Recipe, Target, DITHER, RESAMPLER};
use mpfree_stream::publish::{hash_file, hex, publish, Sink};
use mpfree_stream::ready::{level, Evidence, Level, Passed, Workload};
use mpfree_stream::resample::Rate;
use mpfree_stream::snapshot::{
    self, prepare as prepare_set, Kind, Manifest, Prep, Problem, Refused, Wanted,
};
use mpfree_stream::testkit::{float_wav, folder, int24_wav, wav, Cut, ModelStick, Names, Store};

fn sine(rate: u32, hz: f64, frames: usize, channels: usize, level: f64) -> Vec<f32> {
    (0..frames * channels)
        .map(|i| {
            let f = (i / channels) as f64;
            ((TAU * hz * f / f64::from(rate)).sin() * level) as f32
        })
        .collect()
}

fn layout_of(path: &Path) -> Layout {
    layout(&mut fs::File::open(path).expect("opens")).expect("a sound file")
}

/// A file's audio as floats.
fn audio(path: &Path) -> (Layout, Vec<f32>) {
    let l = layout_of(path);
    let bytes = fs::read(path).expect("reads");
    let data = &bytes[l.data_at as usize..][..l.frames as usize * l.block_align()];
    let mut out = vec![0.0; l.frames as usize * usize::from(l.channels)];
    decode(l.sample, l.endian, data, &mut out);
    (l, out)
}

fn hash(path: &Path) -> [u8; 32] {
    hash_file(&mut StdFs::new(), path).expect("hashes").0
}

#[test]
fn copies_are_made_only_when_needed_as_the_table_says() {
    let dir = folder("prep-table");
    let cases = [
        (44_100, Store::Pcm16, false, Target::Original),
        (44_100, Store::Float, false, Target::Original),
        (48_000, Store::Pcm24, false, Target::Int24 { rate: 44_100 }),
        (48_000, Store::Pcm16, true, Target::Float { rate: 44_100 }),
        (96_000, Store::Float, false, Target::Float { rate: 44_100 }),
    ];
    for (i, (rate, store, float, want)) in cases.into_iter().enumerate() {
        let p = dir.join(format!("{i}.wav"));
        wav(&p, store, 2, 100, 1);
        let mut l = layout_of(&p);
        l.rate = rate;
        assert_eq!(target(&l, 44_100, float), want, "case {i}");
    }
}

#[test]
fn a_copy_is_named_by_what_made_it() {
    let dir = folder("prep-name");
    let p = dir.join("a.wav");
    int24_wav(&p, 48_000, 2, &sine(48_000, 440.0, 1_000, 2, 0.5));
    let l = layout_of(&p);
    let r = Recipe {
        original: hash(&p),
        from: l,
        target: Target::Int24 { rate: 44_100 },
    };
    let name = r.name().expect("a copy");
    assert!(name.starts_with(&hex(&r.original)[..32]), "{name}");
    for part in ["48000i24x2", "to-44100i24", RESAMPLER, DITHER] {
        assert!(name.contains(part), "{name} lacks {part}");
    }
    let float = Recipe {
        target: Target::Float { rate: 44_100 },
        ..r
    };
    assert_ne!(float.name(), r.name());
    let other = Recipe {
        original: [7; 32],
        ..r
    };
    assert_ne!(other.name(), r.name());
    assert_eq!(
        Recipe {
            target: Target::Original,
            ..r
        }
        .name(),
        None
    );
}

#[test]
fn a_24_bit_copy_is_dithered_the_same_every_time_and_a_float_one_is_exact() {
    let dir = folder("prep-make");
    let out = dir.join("managed");
    fs::create_dir_all(&out).expect("dir");
    let input = sine(48_000, 1_000.0, 30_000, 2, 0.5);
    let p = dir.join("a.wav");
    int24_wav(&p, 48_000, 2, &input);
    let (l, original) = audio(&p);
    let rate = Rate::new(48_000, 44_100, l.frames).expect("converts");
    let mut want = vec![0.0f32; rate.frames() as usize * 2];
    rate.run(2, &original, 0, 0, &mut want);
    let r = Recipe {
        original: hash(&p),
        from: l,
        target: Target::Int24 { rate: 44_100 },
    };
    let made = make(&mut StdFs::new(), &p, &mut StdFs::new(), &r, &out, 1).expect("makes");
    assert_eq!(made.sample, Sample::I24);
    assert!(!made.file.existed);
    let peak = made.peak.expect("measured");
    assert!(peak > 0.49 && peak < 0.51, "{peak}");
    let (cl, copy) = audio(&made.file.path);
    assert_eq!(
        (cl.rate, cl.sample, cl.frames),
        (44_100, Sample::I24, rate.frames())
    );
    // Within the dither's one step and the rounding's half.
    let step = 1.0 / 8_388_608.0;
    let worst = copy
        .iter()
        .zip(&want)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0f32, f32::max);
    assert!(worst <= 1.5 * step + 1e-9, "{worst}");
    // Dithered, not just rounded: some samples moved past the nearest step.
    assert!(copy
        .iter()
        .zip(&want)
        .any(|(a, b)| (a - b).abs() > 0.5 * step));
    // The same recipe makes the same file: found, not made again.
    let again = make(&mut StdFs::new(), &p, &mut StdFs::new(), &r, &out, 2).expect("finds");
    assert!(again.file.existed);
    assert_eq!(again.file.hash, made.file.hash);
    assert_eq!((again.sample, again.peak), (Sample::I24, None));
    // Made afresh elsewhere, it comes out bit for bit the same.
    let elsewhere = dir.join("elsewhere");
    fs::create_dir_all(&elsewhere).expect("dir");
    let fresh = make(&mut StdFs::new(), &p, &mut StdFs::new(), &r, &elsewhere, 3).expect("makes");
    assert_eq!(fresh.file.hash, made.file.hash);
    // A float original makes a float copy, exactly the resampler's output.
    let f = dir.join("f.wav");
    float_wav(&f, 48_000, 2, &input);
    let fr = Recipe {
        original: hash(&f),
        from: layout_of(&f),
        target: Target::Float { rate: 44_100 },
    };
    let made = make(&mut StdFs::new(), &f, &mut StdFs::new(), &fr, &out, 4).expect("makes");
    assert_eq!(made.sample, Sample::F32);
    let (_, copy) = audio(&made.file.path);
    let mut want = vec![0.0f32; rate.frames() as usize * 2];
    rate.run(2, &input, 0, 0, &mut want);
    assert_eq!(copy, want);
    assert!(prepare::copy_bytes(&fr) >= made.file.size);
}

#[test]
fn a_copy_whose_peaks_would_pass_full_scale_is_made_as_float_never_clipped() {
    let dir = folder("prep-peak");
    // A full-scale square wave: resampled, it rings past full scale.
    let input: Vec<f32> = (0..20_000)
        .flat_map(|f| {
            let x = if (f / 24) % 2 == 0 { 1.0 } else { -1.0 };
            [x, x]
        })
        .collect();
    let p = dir.join("square.wav");
    int24_wav(&p, 48_000, 2, &input);
    let r = Recipe {
        original: hash(&p),
        from: layout_of(&p),
        target: Target::Int24 { rate: 44_100 },
    };
    let made = make(&mut StdFs::new(), &p, &mut StdFs::new(), &r, &dir, 1).expect("makes");
    let peak = made.peak.expect("measured");
    assert!(peak > 1.0, "{peak}");
    assert_eq!(made.sample, Sample::F32);
    let (_, copy) = audio(&made.file.path);
    let top = copy.iter().fold(0.0f32, |m, x| m.max(x.abs()));
    assert_eq!(top, peak, "kept whole");
    assert!(prepare::copy_bytes(&r) >= made.file.size);
}

#[test]
fn a_published_file_appears_whole_or_not_at_all_after_any_cut() {
    let content: Vec<u8> = (0..300_000u32).map(|i| (i * 31 % 251) as u8).collect();
    let dir = PathBuf::from("/m");
    let run = |stick: &mut ModelStick| {
        let mut write = |sink: &mut Sink| {
            for chunk in content.chunks(70_000) {
                sink(chunk)?;
            }
            Ok(())
        };
        publish(stick, &dir, "copy.wav", 9, &mut write)
    };
    let mut dry = ModelStick::new();
    let done = run(&mut dry).expect("publishes");
    assert_eq!(dry.bytes(&dir.join("copy.wav")), Some(&content[..]));
    assert!(!dry.exists(&dir.join("copy.wav.9.part")));
    for n in 0..=dry.ops {
        let mut stick = ModelStick::new();
        stick.cut_after = Some(n);
        let _ = run(&mut stick);
        for how in [Cut::Lost, Cut::Landed, Cut::Torn(0), Cut::Torn(3)] {
            for names in [Names::Lost, Names::Landed, Names::LastOnly] {
                let after = stick.cut(how, names, Default::default());
                if let Some(b) = after.bytes(&dir.join("copy.wav")) {
                    assert_eq!(b, &content[..], "cut after op {n}, {how:?}, {names:?}");
                }
            }
        }
    }
    // Never over another file: the same content is kept, other content
    // refused.
    let mut stick = dry.clone();
    let same = run(&mut stick).expect("same content");
    assert!(same.existed && same.hash == done.hash);
    let mut stick = dry.clone();
    let mut other = |sink: &mut Sink| sink(b"other");
    assert!(publish(&mut stick, &dir, "copy.wav", 10, &mut other).is_err());
    assert_eq!(stick.bytes(&dir.join("copy.wav")), Some(&content[..]));
    assert!(!stick.exists(&dir.join("copy.wav.10.part")));
    assert!(publish(&mut stick, &dir, "../x", 11, &mut other).is_err());
}

struct Set {
    dir: PathBuf,
    managed: PathBuf,
    take: PathBuf,
    loop_: PathBuf,
    stem48: PathBuf,
}

fn set(name: &str) -> Set {
    let dir = folder(name);
    let managed = dir.join("managed");
    fs::create_dir_all(&managed).expect("dir");
    let take = dir.join("take.wav");
    let loop_ = dir.join("loop.wav");
    let stem48 = dir.join("stem48.wav");
    wav(&take, Store::Pcm24, 2, 20_000, 1);
    wav(&loop_, Store::Pcm16, 2, 10_000, 2);
    int24_wav(&stem48, 48_000, 2, &sine(48_000, 220.0, 24_000, 2, 0.4));
    Set {
        dir,
        managed,
        take,
        loop_,
        stem48,
    }
}

fn wanted(s: &Set) -> Vec<Wanted> {
    vec![
        Wanted {
            name: "Take 1".into(),
            path: s.take.clone(),
            published: true,
        },
        Wanted {
            name: "Loop".into(),
            path: s.loop_.clone(),
            published: false,
        },
        Wanted {
            name: "Stem at 48k".into(),
            path: s.stem48.clone(),
            published: false,
        },
    ]
}

fn run_prepare(s: &Set, free: u64, keep: u64) -> Result<Manifest, Refused> {
    let (mut src, mut dst) = (StdFs::new(), StdFs::new());
    let mut lay = |p: &Path| layout(&mut fs::File::open(p)?);
    let mut meta = snapshot::meta;
    let mut p = Prep {
        src: &mut src,
        fs: &mut dst,
        managed: &s.managed,
        engine: 44_100,
        float: false,
        free,
        keep,
        job: 1,
        layout: &mut lay,
        meta: &mut meta,
    };
    prepare_set(&mut p, "Saturday", &wanted(s), 1_700_000_000)
}

#[test]
fn preparing_a_set_stages_what_could_change_and_copies_what_needs_it() {
    let s = set("snap");
    // Not room beside what is kept for recording: refused, nothing written.
    match run_prepare(&s, 300_000, 200_000) {
        Err(Refused::Space { need, free, keep }) => {
            assert!(
                need > 100_000 && free == 300_000 && keep == 200_000,
                "{need}"
            );
            let msg = Refused::Space { need, free, keep }.to_string();
            assert!(msg.contains("kept for recording and saves"), "{msg}");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(fs::read_dir(&s.managed).expect("dir").count(), 0);
    let mut m = run_prepare(&s, 100_000_000, 200_000).expect("prepares");
    let kinds: Vec<Kind> = m.entries.iter().map(|e| e.kind).collect();
    assert_eq!(kinds, [Kind::Published, Kind::Staged, Kind::Prepared]);
    assert_eq!(
        m.entries[0].path, s.take,
        "a published file is used where it is"
    );
    assert!(m.entries[1].path.starts_with(&s.managed));
    assert_eq!(m.entries[1].hash, hash(&s.loop_));
    assert_eq!(m.entries[2].from, Some(hash(&s.stem48)));
    assert_eq!(layout_of(&m.entries[2].path).rate, 44_100);
    // Written and read back as text.
    let back = Manifest::from_text(&m.to_text()).expect("reads back");
    assert_eq!(back, m);
    assert_eq!(Manifest::from_text("not a snapshot"), None);
    // Opening checks sizes and change times: all fine.
    assert_eq!(m.quick(&mut snapshot::meta), vec![]);
    assert_eq!(m.full(&mut StdFs::new(), 1_700_000_100), vec![]);
    assert_eq!(m.checked, Some(1_700_000_100));
    // The loop is edited in place: the set plays its staged copy, unmoved.
    wav(&s.loop_, Store::Pcm16, 2, 10_000, 99);
    assert_eq!(m.quick(&mut snapshot::meta), vec![]);
    // Preparing again reuses the copies already there.
    let again = run_prepare(&s, 100_000_000, 200_000).expect("prepares");
    assert_eq!(again.entries[2].path, m.entries[2].path);
    // Another program changes the take: the quick check sees it.
    wav(&s.take, Store::Pcm24, 2, 20_001, 1);
    assert_eq!(
        m.quick(&mut snapshot::meta),
        vec![Problem::Changed("Take 1".into())]
    );
    // A staged copy damaged in place, its size and time kept: only the
    // full check sees it, and it isn't marked checked.
    let staged = m.entries[1].path.clone();
    let before = snapshot::meta(&staged).expect("meta");
    let mut bytes = fs::read(&staged).expect("reads");
    bytes[200] ^= 0xFF;
    fs::write(&staged, &bytes).expect("writes");
    let mut fake = |p: &Path| {
        if p == staged {
            Ok(before)
        } else {
            snapshot::meta(p)
        }
    };
    let quick = m.quick(&mut fake);
    assert!(!quick.contains(&Problem::Changed("Loop".into())));
    let full = m.full(&mut StdFs::new(), 1_700_000_200);
    assert!(full.contains(&Problem::Changed("Loop".into())), "{full:?}");
    assert_eq!(m.checked, Some(1_700_000_100), "the last pass stays");
    // A missing file is named.
    fs::remove_file(&m.entries[2].path).expect("removes");
    assert!(m
        .quick(&mut snapshot::meta)
        .contains(&Problem::Missing("Stem at 48k".into())));
    assert_eq!(
        Problem::Missing("Stem at 48k".into()).to_string(),
        "Stem at 48k: missing"
    );
    let _ = s.dir;
}

#[test]
fn readiness_has_three_levels_each_from_its_own_evidence() {
    let set = Workload {
        streams: 12,
        files: 14,
        bytes: 900_000_000,
        largest: 90_000_000,
        recordings: 1,
        voices: 16,
        cache: 128 << 20,
        stick: "SanDisk 64G #3".into(),
        version: "0.9".into(),
        ..Workload::default()
    };
    let mut e = Evidence::default();
    assert_eq!(level(&set, &e), Level::NotReady);
    e.files = true;
    assert_eq!(level(&set, &e), Level::NotReady, "starts not in memory yet");
    e.starts = true;
    assert_eq!(level(&set, &e), Level::FilesReady);
    assert_eq!(level(&set, &e).words(), "All files ready.");
    // A read test on another stick says nothing; on this one, enough.
    let read = |stick: &str, streams| Passed {
        ran: Workload {
            streams,
            stick: stick.into(),
            ..Workload::default()
        },
        on: "21 Nov".into(),
    };
    e.read_tests.push(read("Another stick", 32));
    assert_eq!(level(&set, &e), Level::FilesReady);
    e.read_tests.push(read("SanDisk 64G #3", 16));
    assert_eq!(level(&set, &e), Level::ReadTested);
    assert_eq!(
        level(&set, &e).words(),
        "Read test passed on this stick. Recording not tested."
    );
    // A show test that didn't record doesn't cover a set that records.
    let mut ran = Workload {
        streams: 16,
        recordings: 0,
        ..set.clone()
    };
    e.show_tests.push(Passed {
        ran: ran.clone(),
        on: "20 Nov".into(),
    });
    assert_eq!(level(&set, &e), Level::ReadTested);
    // One that did, with more of everything: Show-tested, with its date.
    ran.recordings = 2;
    e.show_tests.push(Passed {
        ran: ran.clone(),
        on: "22 Nov".into(),
    });
    assert_eq!(
        level(&set, &e).words(),
        "Show test passed on this stick, 22 Nov."
    );
    // A smaller cache than the test's, a FLAC stream, a newer MPFree: not
    // covered.
    for changed in [
        Workload {
            cache: 64 << 20,
            ..set.clone()
        },
        Workload {
            flac: 1,
            ..set.clone()
        },
        Workload {
            version: "1.0".into(),
            ..set.clone()
        },
    ] {
        assert_ne!(
            level(&changed, &e),
            Level::ShowTested {
                on: "22 Nov".into()
            },
            "{changed:?}"
        );
    }
    // Files changed since: not ready, whatever was tested.
    e.files = false;
    assert_eq!(level(&set, &e), Level::NotReady);
}
