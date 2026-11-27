//! The render foundation (the audio tracks rethink, phase 6.0;
//! docs/dev/design/renders.md): a render description's fingerprint is
//! fixed and every part changes it; a description that can't be made as
//! it says is refused; provenance goes on a take line and comes back,
//! needs `audio-provenance`, and is kept through a save by the host.

use std::time::{Duration, Instant};

use mpfree_audiotracks::host::AudioHost;
use mpfree_audiotracks::render::{
    Capture, Dep, Kind, Provenance, Purpose, Quality, Range, Read, RenderDesc, Runs, Source, FORMAT,
};
use mpfree_audiotracks::save::{from_text, needs_provenance, to_text};
use mpfree_audiotracks::{Command as A, RecordingId};
use mpfree_transport::{Block, Command as T, Transport};

const REC: RecordingId = RecordingId(0x5b2f_0d9c_41e8_a776_3c0f_12ab_9e4d_8c01);

/// A render of a track before its fader over bars 9 to 17, exclusive, for
/// a lasting recording: every part set to something.
fn bounce() -> RenderDesc {
    RenderDesc {
        source: Source::Track(9),
        range: Range::Song {
            from: 30_720.0,
            to: 61_440.0,
            tempo: 120.0,
            shape: 0,
        },
        capture: Capture::BeforeFader,
        tail: 44_100,
        preroll: 22_050,
        quality: Quality {
            interp: 1,
            stretch: 0,
            stretch_version: 0,
        },
        seed: 7,
        reads: vec![
            Read {
                dep: Dep::Track(9),
                rev: 3,
            },
            Read {
                dep: Dep::Recording(REC),
                rev: 44_100,
            },
            Read {
                dep: Dep::Routes,
                rev: 12,
            },
        ],
        purpose: Purpose::Recording,
        runs: Runs::Exclusive,
    }
}

#[test]
fn a_render_descriptions_fingerprint_is_fixed_and_every_part_changes_it() {
    let d = bounce();
    assert_eq!(d.check(), Ok(()));
    // The same in every run on every machine: pinned.
    assert_eq!(
        d.fingerprint(),
        0x7553_038e_417f_b63f_5d48_3f44_e785_870a_u128
    );
    assert_eq!(
        FORMAT, 1,
        "a new format changes every fingerprint: pin it again"
    );
    // The reads in another order, or how it runs: the same audio.
    let mut same = d.clone();
    same.reads.reverse();
    same.runs = Runs::RealTime;
    assert_eq!(same.fingerprint(), d.fingerprint());
    // Every part changes it.
    let mut changed: Vec<RenderDesc> = Vec::new();
    let mut each = |f: &dyn Fn(&mut RenderDesc)| {
        let mut c = d.clone();
        f(&mut c);
        changed.push(c);
    };
    each(&|c| c.source = Source::Track(10));
    each(&|c| c.source = Source::Tracks(vec![9, 10]));
    each(&|c| c.source = Source::Tracks(vec![10, 9]));
    each(&|c| c.source = Source::Bus(0));
    each(&|c| c.source = Source::Mix);
    each(&|c| c.source = Source::Clip(9));
    each(&|c| c.source = Source::Loop(9));
    each(&|c| {
        c.source = Source::Recording {
            rec: REC,
            frames: 44_100,
        }
    });
    each(&|c| {
        c.range = Range::Song {
            from: 30_720.0,
            to: 61_441.0,
            tempo: 120.0,
            shape: 0,
        }
    });
    each(&|c| {
        c.range = Range::Song {
            from: 30_720.0,
            to: 61_440.0,
            tempo: 121.0,
            shape: 0,
        }
    });
    each(&|c| {
        c.range = Range::Song {
            from: 30_720.0,
            to: 61_440.0,
            tempo: 120.0,
            shape: 5,
        }
    });
    each(&|c| c.range = Range::Cycle { ticks: 30_720 });
    each(&|c| {
        c.range = Range::Frames {
            from: 0,
            len: 30_720,
        }
    });
    each(&|c| c.capture = Capture::AfterFader);
    each(&|c| c.tail = 0);
    each(&|c| c.preroll = 0);
    each(&|c| c.quality.interp = 2);
    each(&|c| c.quality.stretch = 1);
    each(&|c| c.quality.stretch_version = 1);
    each(&|c| c.seed = 8);
    each(&|c| c.reads[0].rev = 4);
    each(&|c| c.reads[1].dep = Dep::Recording(RecordingId(REC.0 + 1)));
    each(&|c| c.reads[2].dep = Dep::Project);
    each(&|c| {
        c.reads.pop();
    });
    each(&|c| c.purpose = Purpose::Freeze);
    each(&|c| c.purpose = Purpose::Export);
    let mut seen = vec![d.fingerprint()];
    for c in &changed {
        let f = c.fingerprint();
        assert!(!seen.contains(&f), "{c:?} gave a fingerprint already seen");
        seen.push(f);
    }
}

#[test]
fn a_description_is_checked_before_anything_is_rendered() {
    let frames = |from, len| RenderDesc::frames(REC, 1_000, from, len, Purpose::Recording);
    assert_eq!(frames(0, 1_000).check(), Ok(()));
    assert_eq!(frames(250, 500).check(), Ok(()));
    assert_eq!(frames(0, 1_000).runs, Runs::Worker);
    let refused: Vec<(RenderDesc, &str)> = vec![
        (frames(0, 1_001), "the frames are not all in the recording"),
        (frames(-1, 10), "the frames are not all in the recording"),
        (frames(10, 0), "the frames are not all in the recording"),
        (
            RenderDesc::frames(RecordingId::NONE, 1_000, 0, 10, Purpose::Recording),
            "a recording with no id or no frames",
        ),
        (
            RenderDesc {
                capture: Capture::BeforeFader,
                ..frames(0, 10)
            },
            "its source has no such capture point",
        ),
        (
            RenderDesc {
                capture: Capture::Alone,
                ..bounce()
            },
            "its source has no such capture point",
        ),
        (
            RenderDesc {
                source: Source::Mix,
                ..bounce()
            },
            "its source has no such capture point",
        ),
        (
            RenderDesc {
                source: Source::Tracks(vec![9]),
                ..bounce()
            },
            "several tracks means two or more, each once",
        ),
        (
            RenderDesc {
                source: Source::Tracks(vec![9, 4, 9]),
                ..bounce()
            },
            "several tracks means two or more, each once",
        ),
        (
            RenderDesc {
                range: Range::Frames { from: 0, len: 10 },
                ..bounce()
            },
            "a range of frames is a recording's",
        ),
        (
            RenderDesc {
                range: Range::Song {
                    from: 10.0,
                    to: 10.0,
                    tempo: 120.0,
                    shape: 0,
                },
                ..bounce()
            },
            "a song range must run forward at a tempo",
        ),
        (
            RenderDesc {
                range: Range::Cycle { ticks: 0 },
                ..bounce()
            },
            "an empty cycle",
        ),
        (
            RenderDesc {
                runs: Runs::Worker,
                ..bounce()
            },
            "only audio as it is renders on the worker",
        ),
        (
            RenderDesc {
                purpose: Purpose::Copy,
                ..bounce()
            },
            "a copy is made from audio alone",
        ),
    ];
    for (d, why) in refused {
        assert_eq!(d.check(), Err(why), "{d:?}");
    }
    // Each purpose's kind of audio.
    assert_eq!(Purpose::Recording.kind(), Some(Kind::Primary));
    assert_eq!(Purpose::Freeze.kind(), Some(Kind::Persistent));
    assert_eq!(Purpose::Copy.kind(), Some(Kind::Disposable));
    assert_eq!(Purpose::Export.kind(), None);
}

/// A part with one take on one track, `words` on its take line.
fn part(words: &str) -> String {
    format!(
        "mpfree-audiotracks 2\nrate=44100\ntrack=0 id=9 input=left monitor=auto\n\
         region start=0 end=3840 take=3 src=0 gain=1 fade_in=0 fade_out=0 into=0 left=0\n\
         take=3 rec={REC} track=0 group=1 pass=0 bpm=120 anchor=0 frames=44100 channels=1{words} file=Bounce.wav\n"
    )
}

#[test]
fn provenance_goes_on_a_take_line_and_comes_back() {
    let d = bounce();
    let p = Provenance::of(&d, 1_764_100_000);
    assert!(p.matches(&d));
    assert_eq!(p.kind(), Kind::Primary);
    assert_eq!(p.sources().collect::<Vec<_>>(), vec![(REC, 44_100)]);
    let words = p.words();
    assert_eq!(
        words,
        format!(
            " made={:032x} for=recording ver=1 at=1764100000 reads=r{REC}.44100,t9.3,routes.12",
            d.fingerprint()
        )
    );
    // Through a part: the same text again, and the project needs it.
    let text = part(&words);
    let saved = from_text(&text).unwrap();
    assert_eq!(saved.takes[0].prov.as_ref(), Some(&p));
    assert_eq!(to_text(&saved), text);
    assert!(saved.needs_provenance() && needs_provenance(&text));
    // A take recorded or brought in has none, and needs nothing.
    let plain = from_text(&part("")).unwrap();
    assert_eq!(plain.takes[0].prov, None);
    assert!(!plain.needs_provenance() && !needs_provenance(&part("")));
    // A freeze, and one that read nothing.
    let freeze = Provenance {
        reads: Vec::new(),
        purpose: Purpose::Freeze,
        ..p.clone()
    };
    let back = from_text(&part(&freeze.words())).unwrap();
    assert_eq!(back.takes[0].prov.as_ref(), Some(&freeze));
    assert_eq!(freeze.kind(), Kind::Persistent);
    // Every read's word.
    let all = Provenance {
        reads: vec![
            Read {
                dep: Dep::Recording(REC),
                rev: 1,
            },
            Read {
                dep: Dep::Track(2),
                rev: 3,
            },
            Read {
                dep: Dep::Clip(4),
                rev: 5,
            },
            Read {
                dep: Dep::Loop(6),
                rev: 7,
            },
            Read {
                dep: Dep::Routes,
                rev: 8,
            },
            Read {
                dep: Dep::Project,
                rev: 9,
            },
        ],
        ..p.clone()
    };
    let back = from_text(&part(&all.words())).unwrap();
    assert_eq!(back.takes[0].prov.as_ref(), Some(&all));

    // Wrong words refuse the part, saying which.
    let made = format!("made={:032x}", d.fingerprint());
    for (words, why) in [
        (" for=recording ver=1".to_string(), "for= with no made="),
        (
            " made=12 for=recording ver=1".into(),
            "made=12 is not 32 hex digits",
        ),
        (format!(" {made} ver=1"), "made= with no for="),
        (
            format!(" {made} for=copy ver=1"),
            "for=copy is not a recording or a freeze",
        ),
        (
            format!(" {made} for=export ver=1"),
            "for=export is not a recording or a freeze",
        ),
        (format!(" {made} for=recording"), "made= with no ver="),
        (format!(" {made} for=recording ver=0"), "made= with no ver="),
        (
            format!(" {made} for=recording ver=1 at=soon"),
            "at=soon is not a time",
        ),
        (
            format!(" {made} for=recording ver=1 reads=t9"),
            "reads: t9 is not a read",
        ),
        (
            format!(" {made} for=recording ver=1 reads=x9.1"),
            "reads: x9.1 is not a read",
        ),
        (
            format!(" {made} for=recording ver=1 reads=r12.1"),
            "reads: r12.1 is not a read",
        ),
    ] {
        let err = from_text(&part(&words)).unwrap_err();
        assert_eq!(
            err,
            format!("line 5: where take 3 came from: {why}"),
            "{words}"
        );
    }
    // A newer format's provenance is kept as it is, and not matched.
    let newer = Provenance {
        format: 2,
        ..p.clone()
    };
    let back = from_text(&part(&newer.words())).unwrap();
    assert_eq!(back.takes[0].prov.as_ref(), Some(&newer));
    assert!(!newer.matches(&d));
}

const RATE: u32 = 22_050;
const P: usize = 256;

/// Runs `h` for one period, a 441 Hz tone coming in while `live`.
fn period(t: &mut Transport, h: &mut AudioHost, b: &mut Block, frame: &mut u64, live: bool) {
    let (mut inp, mut mix) = ([0.0f32; 2 * P], [0.0f32; 2 * P]);
    for j in 0..P {
        let g = *frame + j as u64;
        let s = if live {
            (0.5 * (std::f64::consts::TAU * 441.0 * g as f64 / f64::from(RATE)).sin()) as f32
        } else {
            0.0
        };
        inp[2 * j] = s;
        inp[2 * j + 1] = s;
    }
    t.process(P as u32, b);
    let map = *t.tempo_map();
    h.period(b, &map, &inp, &mut mix);
    *frame += P as u64;
}

/// A save through `h`, run until it comes.
fn save(
    t: &mut Transport,
    h: &mut AudioHost,
    b: &mut Block,
    frame: &mut u64,
) -> mpfree_audiotracks::save::Saved {
    assert!(h.save());
    let t0 = Instant::now();
    loop {
        if let Some(s) = h.take_saved() {
            return s;
        }
        assert!(t0.elapsed() < Duration::from_secs(60), "no save");
        period(t, h, b, frame, false);
        std::thread::sleep(Duration::from_micros(200));
    }
}

#[test]
fn a_take_opened_with_provenance_keeps_it_through_a_save() {
    // Record a bar, and save it.
    let (mut t, mut h, mut b, mut frame) = (
        Transport::new(RATE, 240.0).unwrap(),
        AudioHost::new(RATE, 20),
        Block::new(),
        0u64,
    );
    h.command(A::Arm(0, true));
    t.command(1_000, T::Record);
    t.command(1_000 + u64::from(RATE), T::Stop);
    while frame < 1_000 + u64::from(RATE) + 512 {
        period(&mut t, &mut h, &mut b, &mut frame, true);
    }
    h.command(A::Arm(0, false));
    let mut saved = save(&mut t, &mut h, &mut b, &mut frame);
    assert_eq!(saved.takes.len(), 1);
    assert_eq!(saved.takes[0].prov, None, "a take recorded has none");
    // As if a render had made it: open it in a fresh host with provenance.
    let k = &saved.takes[0];
    let d = RenderDesc::frames(
        k.rec,
        k.frames as u64,
        0,
        k.frames as u64,
        Purpose::Recording,
    );
    let p = Provenance::of(&d, 1_764_100_000);
    saved.takes[0].prov = Some(p.clone());
    let text = to_text(&saved);

    let (mut t2, mut h2, mut b2, mut frame2) = (
        Transport::new(RATE, 240.0).unwrap(),
        AudioHost::new(RATE, 20),
        Block::new(),
        0u64,
    );
    assert!(h2.load(saved).is_ok());
    let t0 = Instant::now();
    while h2.tracks.loading() > 0 {
        assert!(t0.elapsed() < Duration::from_secs(60));
        period(&mut t2, &mut h2, &mut b2, &mut frame2, false);
        std::thread::sleep(Duration::from_micros(200));
    }
    let again = save(&mut t2, &mut h2, &mut b2, &mut frame2);
    assert_eq!(again.takes[0].prov.as_ref(), Some(&p));
    assert_eq!(to_text(&again), text, "the same part, provenance and all");

    // A project opening lets it go: a take of the next project with the
    // same id but none has none.
    h2.clear();
    let mut plain = again.clone();
    plain.takes[0].prov = None;
    for k in &mut plain.takes {
        k.audio = vec![0.0; k.frames * k.channels];
        k.path = None;
    }
    assert!(h2.load(plain).is_ok());
    while h2.tracks.loading() > 0 {
        period(&mut t2, &mut h2, &mut b2, &mut frame2, false);
        std::thread::sleep(Duration::from_micros(200));
    }
    let last = save(&mut t2, &mut h2, &mut b2, &mut frame2);
    assert_eq!(last.takes[0].prov, None);
}

#[test]
fn what_a_render_read_holds_until_it_changes_and_says_what_changed() {
    use mpfree_audiotracks::render::changed;
    use mpfree_audiotracks::AudioTracks;
    const BAR: u64 = 22_050; // 240 BPM
    let mut t = Transport::new(RATE, 240.0).unwrap();
    let mut a = AudioTracks::new(RATE, 6);
    let mut b = Block::new();
    let input = vec![0.25f32; 2 * P];
    let mut out = vec![0.0f32; 2 * P];
    let mut run = |a: &mut AudioTracks, t: &mut Transport, frames: u64| {
        for _ in 0..frames.div_ceil(P as u64) {
            t.process(P as u32, &mut b);
            let map = *t.tempo_map();
            a.process(&b, &map, &input, &mut out);
        }
    };
    a.command(A::Arm(0, true));
    a.command(A::Arm(1, true));
    t.command(0, T::Record);
    t.command(BAR, T::Stop);
    run(&mut a, &mut t, BAR / 2);
    let rec = a.takes().find(|k| k.track == 0).unwrap().rec;
    assert_eq!(a.rec_rev(rec), None, "while it records");
    run(&mut a, &mut t, BAR);
    a.command(A::Arm(0, false));
    a.command(A::Arm(1, false));
    run(&mut a, &mut t, 1);
    let frames = a.rec_rev(rec).expect("stopped");
    assert_eq!(
        frames,
        a.takes().find(|k| k.rec == rec).unwrap().frames as u64
    );
    assert_eq!(a.track_rev(99), None);
    let reads = [
        Read {
            dep: Dep::Recording(rec),
            rev: frames,
        },
        Read {
            dep: Dep::Track(0),
            rev: a.track_rev(0).unwrap(),
        },
        Read {
            dep: Dep::Track(1),
            rev: a.track_rev(1).unwrap(),
        },
    ];
    let now = |a: &AudioTracks| {
        changed(&reads, |d| match d {
            Dep::Recording(r) => a.rec_rev(r),
            Dep::Track(k) => a.track_rev(k as usize),
            _ => None,
        })
    };
    assert_eq!(now(&a), None, "nothing edited");
    let quiet = a.track_rev(2);
    // An edit of track 2's level changes nothing a render of 0 and 1 read.
    a.command(A::SetLevel(2, 0.5));
    run(&mut a, &mut t, 1);
    assert_eq!(now(&a), None);
    // A region's gain on track 1: that track, and only it.
    let before = (a.track_rev(0), a.track_rev(1));
    a.command(A::SetGain {
        track: 1,
        at: 1_920,
        gain: 0.5,
    });
    run(&mut a, &mut t, 1);
    assert_eq!(now(&a), Some(Dep::Track(1)));
    assert_eq!(a.track_rev(0), before.0);
    assert_ne!(a.track_rev(1), before.1);
    // UNDO back to what was read: it holds again.
    a.command(A::Undo);
    run(&mut a, &mut t, 1);
    assert_eq!(now(&a), None, "the state read, after UNDO");
    a.command(A::Redo);
    run(&mut a, &mut t, 1);
    assert_eq!(now(&a), Some(Dep::Track(1)), "REDO");
    a.command(A::Undo);
    run(&mut a, &mut t, 1);
    // A split on track 0.
    a.command(A::Split { track: 0, at: 960 });
    run(&mut a, &mut t, 1);
    assert_eq!(now(&a), Some(Dep::Track(0)));
    a.command(A::Undo);
    run(&mut a, &mut t, 1);
    assert_eq!(now(&a), None);
    assert_eq!(a.track_rev(2), quiet);

    // A project opening: the host's generation.
    let mut h = AudioHost::new(RATE, 20);
    let slot = |id: u32| Some(id as usize);
    let g = h.rev(Dep::Project, slot).unwrap();
    assert!(h.clear());
    assert_eq!(h.rev(Dep::Project, slot), Some(g + 1));
    assert_eq!(h.rev(Dep::Track(0), slot), h.tracks.track_rev(0));
    assert_eq!(
        h.rev(Dep::Track(0), |_| None),
        None,
        "no such track in the project"
    );
    assert_eq!(
        h.rev(Dep::Recording(rec), slot),
        None,
        "not in these tracks"
    );
    for d in [Dep::Clip(1), Dep::Loop(1), Dep::Routes] {
        assert_eq!(h.rev(d, slot), None, "the app's");
    }

    // What changed, in words.
    for (d, words) in [
        (Dep::Recording(rec), "the recording Vocal is gone"),
        (Dep::Track(0), "Vocal was edited while it rendered"),
        (Dep::Clip(0), "the clip Vocal was edited while it rendered"),
        (Dep::Loop(0), "Vocal changed while it rendered"),
        (Dep::Routes, "the mixer's routing changed while it rendered"),
        (Dep::Project, "another project opened while it rendered"),
    ] {
        assert_eq!(d.changed_words("Vocal"), words);
    }
}

// ---------------------------------------------------------------- 6.0.3

use mpfree_audiotracks::files::{recover_into, render_file_name, RenderFile};
use mpfree_audiotracks::host::{RenderFailure, RenderOutcome, RenderRefused};
use mpfree_audiotracks::save::{TakeFile, TrackFile};
use mpfree_audiotracks::{Input, Monitor, Region};
use mpfree_stretch::sched::{Config, Sched};

/// A take of 1.5 bars of a 441 Hz tone at 240 BPM, mono, as a project
/// opened it.
fn tone_take(frames: usize) -> TakeFile {
    TakeFile {
        id: 0,
        rec: REC,
        track: 0,
        group: 1,
        pass: 0,
        bpm: 240.0,
        varied: false,
        anchor: 0.0,
        frames,
        channels: 1,
        file: String::new(),
        path: None,
        audio: (0..frames)
            .map(|g| {
                (0.5 * (std::f64::consts::TAU * 441.0 * g as f64 / f64::from(RATE)).sin()) as f32
            })
            .collect(),
        prov: None,
    }
}

/// A host with the tone on track 0 playing from bar 1, files on in
/// `work`, its renders on a scheduler the test steps.
struct Rr {
    t: Transport,
    h: AudioHost,
    s: Sched,
    b: Block,
    frame: u64,
    work: std::path::PathBuf,
}

const FRAMES: usize = 33_075;

impl Rr {
    fn new(name: &str) -> Rr {
        Rr::with(name, true)
    }

    /// With takes recording to files in its work folder, or not.
    fn with(name: &str, files: bool) -> Rr {
        let work =
            std::env::temp_dir().join(format!("mpfree-render-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&work);
        std::fs::create_dir_all(&work).unwrap();
        let s = Sched::manual(Config::default());
        let mut h = AudioHost::new(RATE, 20);
        h.copies_on(s.clone());
        if files {
            assert!(h.record_to(work.clone(), Default::default()).is_ok());
        }
        assert!(h.tracks.load_take(tone_take(FRAMES)).is_ok());
        let region = Region::of(0, 5_760, 0, (0, 0.0), 240.0);
        assert!(h.tracks.load_track(&TrackFile {
            index: 0,
            id: 9,
            input: Input::Left,
            monitor: Monitor::Auto,
            from: String::new(),
            regions: vec![region],
        }));
        let mut r = Rr {
            t: Transport::new(RATE, 240.0).unwrap(),
            h,
            s,
            b: Block::new(),
            frame: 0,
            work,
        };
        r.h.set_track_id(0, 9);
        r.until(|r| r.h.tracks.loading() == 0);
        r
    }

    fn period(&mut self) -> Vec<f32> {
        let (inp, mut mix) = ([0.0f32; 2 * P], [0.0f32; 2 * P]);
        self.t.process(P as u32, &mut self.b);
        let map = *self.t.tempo_map();
        self.h.period(&self.b, &map, &inp, &mut mix);
        self.frame += P as u64;
        mix.to_vec()
    }

    /// Periods, the scheduler stepped after each, until `done`.
    #[track_caller]
    fn until(&mut self, mut done: impl FnMut(&mut Rr) -> bool) {
        let t0 = Instant::now();
        while !done(self) {
            assert!(t0.elapsed() < Duration::from_secs(60), "timed out");
            self.period();
            self.s.step();
            std::thread::sleep(Duration::from_micros(200));
        }
    }

    /// The song from bar 1 for `frames`.
    fn play(&mut self, frames: u64) -> Vec<f32> {
        self.t.command(self.frame, T::Locate(0));
        self.t.command(self.frame, T::PlayStart);
        let mut out = Vec::new();
        while (out.len() as u64) < 2 * frames {
            out.extend(self.period());
        }
        self.t.command(self.frame, T::Stop);
        self.period();
        out
    }

    /// The outcome, once the render ends.
    fn outcome(&mut self) -> RenderOutcome {
        let mut got = None;
        self.until(|r| {
            got = r.h.render_outcome();
            got.is_some()
        });
        got.unwrap()
    }

    fn names(&self) -> Vec<String> {
        let mut n: Vec<String> = std::fs::read_dir(&self.work)
            .unwrap()
            .filter_map(Result::ok)
            .filter_map(|e| e.file_name().into_string().ok())
            .collect();
        n.sort();
        n
    }

    fn whole(&self) -> RenderDesc {
        RenderDesc::frames(REC, FRAMES as u64, 0, FRAMES as u64, Purpose::Recording)
    }
}

/// A WAV's float samples.
fn samples_of(path: &std::path::Path) -> Vec<f32> {
    let b = std::fs::read(path).unwrap();
    b[44..]
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

#[test]
fn a_render_of_a_recording_commits_one_step_that_plays_the_same() {
    let mut r = Rr::new("commit");
    let before = r.play(2 * u64::from(RATE));
    assert!(r.h.render(r.whole(), 1_764_100_000).is_ok());
    assert!(r.h.rendering());
    let RenderOutcome::Committed { old, new, rec } = r.outcome() else {
        panic!("not committed");
    };
    assert_eq!(old, 0);
    assert!(!r.h.rendering());
    // The step is in from the next period, as every command is.
    r.period();
    // Its file, whole and durable, and no .part.
    let name = render_file_name(rec);
    assert_eq!(r.names(), vec![name.clone()]);
    let mut source = vec![0.0; FRAMES];
    assert_eq!(r.h.tracks.read_take(old, 0, &mut source), Some(FRAMES));
    assert_eq!(samples_of(&r.work.join(&name)), source, "sample for sample");
    let mut made = vec![0.0; FRAMES];
    assert_eq!(r.h.tracks.read_take(new, 0, &mut made), Some(FRAMES));
    assert_eq!(made, source);
    // The song plays it, and sounds the same.
    assert!(r.h.tracks.regions(0).iter().all(|g| g.take == new));
    assert_eq!(r.play(2 * u64::from(RATE)), before);
    // Saved with where it came from.
    assert!(r.h.save());
    let mut saved = None;
    r.until(|r| {
        saved = r.h.take_saved();
        saved.is_some()
    });
    let saved = saved.unwrap();
    let k = saved.takes.iter().find(|k| k.id == new).unwrap();
    assert_eq!(k.file, name);
    let p = k.prov.as_ref().expect("its provenance");
    assert_eq!(p.sources().collect::<Vec<_>>(), vec![(REC, FRAMES as u64)]);
    assert!(p.matches(&r.whole()));
    assert_eq!(p.at, 1_764_100_000);
    assert!(saved.needs_provenance());
    // One UNDO takes it back.
    r.h.command(A::Undo);
    r.period();
    assert!(r.h.tracks.regions(0).iter().all(|g| g.take == old));
}

#[test]
fn a_render_whose_reads_change_under_it_commits_nothing_and_keeps_its_file() {
    let mut r = Rr::new("changed");
    let mut d = r.whole();
    d.reads.push(Read {
        dep: Dep::Track(9),
        rev: r.h.tracks.track_rev(0).unwrap(),
    });
    assert!(r.h.render(d, 0).is_ok());
    // Until its job has begun its file.
    r.until(|r| r.names().iter().any(|n| n.ends_with(".part")));
    r.h.command(A::SetGain {
        track: 0,
        at: 960,
        gain: 0.5,
    });
    assert_eq!(r.outcome(), RenderOutcome::Changed(Dep::Track(9)));
    assert!(
        r.h.tracks.regions(0).iter().all(|g| g.take == 0),
        "nothing committed"
    );
    let names = r.names();
    assert_eq!(names.len(), 1, "{names:?}");
    assert!(
        names[0].starts_with("render_") && names[0].ends_with(".wav"),
        "kept: {names:?}"
    );
}

#[test]
fn cancel_or_a_project_opening_commits_nothing_and_leaves_no_file() {
    let mut r = Rr::new("cancel");
    assert!(r.h.render(r.whole(), 0).is_ok());
    r.until(|r| r.names().iter().any(|n| n.ends_with(".part")));
    r.h.cancel_render();
    assert_eq!(r.h.render_outcome(), Some(RenderOutcome::Cancelled));
    assert!(!r.h.rendering());
    r.until(|r| r.names().is_empty());
    assert!(r.h.tracks.regions(0).iter().all(|g| g.take == 0));
    for _ in 0..20 {
        r.period();
        r.s.step();
    }
    assert!(r.names().is_empty(), "nothing comes later");
    assert_eq!(r.h.render_outcome(), None);

    // A project opening.
    let mut r = Rr::new("opening");
    assert!(r.h.render(r.whole(), 0).is_ok());
    r.until(|r| r.names().iter().any(|n| n.ends_with(".part")));
    assert!(r.h.clear());
    assert_eq!(r.h.render_outcome(), Some(RenderOutcome::Cancelled));
    r.until(|r| r.names().is_empty());
    for _ in 0..20 {
        r.period();
        r.s.step();
    }
    assert!(r.names().is_empty());
}

#[test]
fn a_render_with_nowhere_to_write_fails_and_commits_nothing() {
    let mut r = Rr::with("nowhere", false);
    assert!(r.h.render(r.whole(), 0).is_ok());
    assert_eq!(
        r.outcome(),
        RenderOutcome::Failed(RenderFailure::NoWorkFolder)
    );
    assert!(!r.h.rendering());
    r.period();
    assert!(r.h.tracks.regions(0).iter().all(|g| g.take == 0));
    assert_eq!(r.h.tracks.takes().count(), 1, "no take made");
}

#[test]
fn a_render_is_refused_before_it_starts_when_it_cant_be_made_here() {
    let mut r = Rr::new("refused");
    let whole = r.whole();
    let part = RenderDesc::frames(REC, FRAMES as u64, 0, 100, Purpose::Recording);
    let elsewhere = RenderDesc::frames(
        RecordingId(7),
        FRAMES as u64,
        0,
        FRAMES as u64,
        Purpose::Recording,
    );
    let mut clip = r.whole();
    clip.reads.push(Read {
        dep: Dep::Clip(1),
        rev: 0,
    });
    let mut unknown = r.whole();
    unknown.reads.push(Read {
        dep: Dep::Track(4),
        rev: 0,
    });
    let mut stale = r.whole();
    stale.reads.push(Read {
        dep: Dep::Track(9),
        rev: 1,
    });
    for (d, why) in [
        (bounce(), RenderRefused::NotOnWorker),
        (part, RenderRefused::Part),
        (elsewhere, RenderRefused::NotHere),
        (clip, RenderRefused::Reads),
        (unknown, RenderRefused::Reads),
        (stale, RenderRefused::Changed(Dep::Track(9))),
        (
            RenderDesc::frames(REC, FRAMES as u64, 0, FRAMES as u64 + 1, Purpose::Recording),
            RenderRefused::Bad("the frames are not all in the recording"),
        ),
        (
            RenderDesc::frames(
                REC,
                FRAMES as u64 + 1,
                0,
                FRAMES as u64 + 1,
                Purpose::Recording,
            ),
            RenderRefused::Changed(Dep::Recording(REC)),
        ),
    ] {
        assert_eq!(r.h.render(d, 0), Err(why));
        assert!(!r.h.rendering());
    }
    assert!(r.h.render(whole.clone(), 0).is_ok());
    assert_eq!(r.h.render(whole, 0), Err(RenderRefused::Busy));
    assert!(matches!(r.outcome(), RenderOutcome::Committed { .. }));
}

#[test]
fn a_render_file_is_whole_or_not_there_whatever_the_stick_does() {
    use mpfree_stream::testkit::{Cut, Damage, Names, SharedStick};
    let dir = std::path::Path::new("/work");
    let audio: Vec<f32> = (0..3_000).map(|i| (i as f32 * 0.01).sin()).collect();
    let write = |stick: SharedStick| -> std::io::Result<()> {
        let mut f = RenderFile::begin(stick, dir, REC, 1_500, 2, RATE)?;
        for piece in audio.chunks(700) {
            f.write(piece)?;
        }
        f.finish().map(|_| ())
    };
    // Whole: how many operations it takes.
    let stick = SharedStick::new();
    write(stick.clone()).unwrap();
    let ops = stick.with(|m| m.ops);
    let whole = stick.with(|m| m.bytes(&dir.join(render_file_name(REC))).unwrap().to_vec());
    assert_eq!(whole.len(), 44 + 4 * audio.len());
    // The power going after each operation: what comes back is the whole
    // render under its name, or no such name.
    for k in 0..=ops {
        for names in [Names::Lost, Names::Landed, Names::LastOnly] {
            let stick = SharedStick::new();
            stick.with(|m| m.cut_after = Some(k));
            let _ = write(stick.clone());
            let back = stick.with(|m| m.cut(Cut::Torn(1), names, Damage::default()));
            if let Some(b) = back.bytes(&dir.join(render_file_name(REC))) {
                assert_eq!(b, &whole[..], "cut after {k}, {names:?}");
            }
        }
    }
    // A full stick, or any failed operation: an error, and no .part.
    for k in 1..=ops {
        let stick = SharedStick::new();
        stick.with(|m| {
            m.fail.insert(k);
        });
        let out = write(stick.clone());
        let left = stick.with(|m| m.names());
        let named = left.iter().any(|p| p.ends_with(render_file_name(REC)));
        assert!(out.is_err() || named, "op {k}");
        if out.is_err() {
            assert!(
                !left.iter().any(|p| p.to_string_lossy().ends_with(".part")),
                "op {k}: {left:?}"
            );
        }
    }
    let stick = SharedStick::new();
    stick.with(|m| m.capacity = Some(4_096));
    let e = write(stick.clone()).unwrap_err();
    assert_eq!(e.kind(), std::io::ErrorKind::StorageFull);
    assert!(stick.with(|m| m.names()).is_empty());
}

#[test]
fn no_takes_file_looks_like_a_cut_off_hidden_part() {
    use mpfree_audiotracks::files::{file_name, Name};
    for track in [".part", "..hidden.part", " .x.part", "Vocal", ".", ""] {
        let name = Name::new(track);
        let file = file_name(name.as_str(), REC);
        assert!(
            !file.starts_with('.') && !file.ends_with(".part"),
            "{track:?} gives {file:?}"
        );
    }
}

#[test]
fn the_next_start_removes_what_was_never_whole_and_keeps_renders() {
    let work = std::env::temp_dir().join(format!("mpfree-render-start-{}", std::process::id()));
    let keep = work.with_extension("keep");
    let _ = std::fs::remove_dir_all(&work);
    let _ = std::fs::remove_dir_all(&keep);
    std::fs::create_dir_all(&work).unwrap();
    let render = render_file_name(REC);
    std::fs::write(work.join(format!(".{render}.part")), b"cut off").unwrap();
    std::fs::write(work.join(".take_1.wav.part"), b"cut off").unwrap();
    std::fs::write(
        work.join(&render),
        mpfree_audiotracks::save::wav(&[0.1, 0.2], 1, RATE),
    )
    .unwrap();
    // Never through a link: a hidden `.part` that is a link to a file
    // elsewhere is left, and so is what it points at.
    #[cfg(unix)]
    let outside = work.with_extension("outside");
    #[cfg(unix)]
    {
        std::fs::write(&outside, b"not ours").unwrap();
        std::os::unix::fs::symlink(&outside, work.join(".link.part")).unwrap();
    }
    let found = recover_into(&work, &keep).unwrap();
    #[cfg(unix)]
    {
        assert_eq!(std::fs::read(&outside).unwrap(), b"not ours");
        assert!(std::fs::symlink_metadata(work.join(".link.part")).is_ok());
        std::fs::remove_file(work.join(".link.part")).unwrap();
        std::fs::remove_file(&outside).unwrap();
    }
    assert!(
        std::fs::read_dir(&work).unwrap().next().is_none(),
        "nothing left in the work folder"
    );
    assert!(keep.join(&render).exists());
    assert!(
        found
            .lines
            .iter()
            .any(|l| l == "audio tracks: 2 files cut off before they were whole were removed"),
        "{:?}",
        found.lines
    );
    assert!(
        found
            .lines
            .iter()
            .any(|l| l == "audio tracks: 1 render not saved in a project was kept in Recordings"),
        "{:?}",
        found.lines
    );
}
