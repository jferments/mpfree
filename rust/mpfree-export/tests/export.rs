//! An export from start to finish against a real mixer: the files are the
//! mixer's output, start on the song's first frame and end where the
//! sound does (never inside the range), fast and real time make the same
//! files, the MIDI file holds the notes, and a cancel leaves nothing.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use mpfree_export::mpfree_fx::mpfree_node::{Ctx, Kind, Node, NodeInfo, ParamInfo, Timed};
use mpfree_export::mpfree_fx::{Mixer, MixerConfig, StripId};
use mpfree_export::mpfree_midi::msg::Msg;
use mpfree_export::*;

const RATE: u32 = 48_000;
const LSB: f32 = 1.0 / 8_388_607.0;

/// A pure delay that reports its latency, like a look-ahead effect.
struct Lag {
    n: usize,
    buf: [Vec<f32>; 2],
    w: usize,
}

static LAG: NodeInfo = NodeInfo {
    id: "test.lag",
    name: "Lag",
    kind: Kind::Effect,
};

impl Node for Lag {
    fn info(&self) -> &'static NodeInfo {
        &LAG
    }
    fn params(&self) -> &'static [ParamInfo] {
        &[]
    }
    fn set_param(&mut self, _id: u16, _value: f32) {}
    fn param(&self, _id: u16) -> f32 {
        f32::NAN
    }
    fn prepare(&mut self, _rate: u32, _max_block: usize) {}
    fn reset(&mut self) {
        for b in &mut self.buf {
            b.fill(0.0);
        }
    }
    fn process(&mut self, _ctx: &Ctx, _events: &[Timed], left: &mut [f32], right: &mut [f32]) {
        let size = self.n + 1;
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            self.buf[0][self.w] = *l;
            self.buf[1][self.w] = *r;
            let rd = (self.w + size - self.n) % size;
            *l = self.buf[0][rd];
            *r = self.buf[1][rd];
            self.w = (self.w + 1) % size;
        }
    }
    fn latency(&self) -> u32 {
        self.n as u32
    }
}

/// A new folder name under the system's temporary folder.
fn scratch(name: &str) -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    std::env::temp_dir().join(format!(
        "mpfree-export-test-{name}-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ))
}

/// A 24-bit stereo WAV's left channel (its header checked).
fn read24(b: &[u8]) -> Vec<f32> {
    assert_eq!(&b[0..4], b"RIFF");
    assert_eq!(u16::from_le_bytes([b[34], b[35]]), 24);
    let data = u32::from_le_bytes(b[40..44].try_into().unwrap()) as usize;
    assert_eq!(data + 44, b.len(), "the header's length");
    (0..data / 6)
        .map(|k| {
            let i = 44 + 6 * k;
            (i32::from_le_bytes([0, b[i], b[i + 1], b[i + 2]]) >> 8) as f32 / 8_388_607.0
        })
        .collect()
}

/// The song: Track 1 plays a tone for `sound` frames then dies away over
/// `tail` frames (a reverb, say); Track 2 a tone through 300 frames of
/// look-ahead, `sound` frames only. The range is `range` frames.
/// `endless`: Track 1 never stops.
#[derive(Clone, Copy)]
struct Song {
    range: usize,
    sound: usize,
    tail: usize,
    endless: bool,
}

impl Song {
    fn track(&self, t: usize, i: usize) -> f32 {
        let w = |hz: f32| (std::f32::consts::TAU * hz * i as f32 / RATE as f32).sin();
        match t {
            0 if self.endless || i < self.sound => 0.3 * w(220.0),
            0 if i < self.sound + self.tail => {
                let k = (i - self.sound) as f32 / self.tail as f32;
                0.3 * (-9.5 * k).exp() * w(220.0)
            }
            1 if i < self.sound => 0.2 * w(330.0),
            _ => 0.0,
        }
    }
}

struct Run {
    outcome: Outcome,
    out: Vec<f32>,
    most: u32,
    ended_at: u64,
    played: u64,
}

fn plan(folder: &Path, mode: Mode, song: Song) -> Plan {
    Plan {
        folder: folder.to_path_buf(),
        song: "Night drive".into(),
        streams: vec![
            Stream::mixdown(),
            Stream::stem("Drums", StripId::Track(0)),
            Stream::stem("Synth", StripId::Track(1)),
            // A return nothing is sent to: silent, so left out.
            Stream::stem("Delay", StripId::Return(0)),
        ],
        midi: Some(MidiPlan {
            tracks: vec!["Drums".into(), "Synth".into()],
            tempos: vec![(0, 120.0)],
            meters: vec![(0, 4, 4)],
            end: song.range as u64 / 25,
            room: 64,
        }),
        mode,
        rate: RATE,
        length: Some(song.range as u64),
        first_slot: 3,
    }
}

/// Plays an export through the mixer in blocks from `sizes`, as the sound
/// loop does in fast mode: waiting for room before each block. (A pass in
/// real time is paced by the sound card and never waits; this test runs
/// far faster than real time in both modes, so it waits in both.)
fn run(folder: &Path, mode: Mode, song: Song, sizes: &[usize], cancel_at: Option<u64>) -> Run {
    let cfg = MixerConfig {
        inputs: 0,
        pads: 0,
        tracks: 2,
        slots: 2,
        max_latency_ms: 50.0,
    };
    let mut m = Mixer::new(cfg, RATE, 512);
    m.chain_mut(StripId::Track(1)).unwrap().push(Box::new(Lag {
        n: 300,
        buf: [vec![0.0; 301], vec![0.0; 301]],
        w: 0,
    }));
    // The app's mixer has always played before an export starts; here,
    // one silent block lines its paths up.
    let mut z = (vec![0.0f32; 64], vec![0.0f32; 64]);
    m.process(&Ctx::new(RATE, 0), &mut z.0, &mut z.1);

    let (mut pass, job) = start(plan(folder, mode, song)).unwrap();
    assert_eq!(pass.mode(), mode);
    assert!(pass.attach(&mut m));
    let most = m.capture_latency(3).unwrap();
    for s in 1..4 {
        assert!(m.capture_latency(3 + s).unwrap() <= most);
    }
    let mut out = Vec::new();
    let mut at = 0usize;
    let mut ended_at = None;
    let mut k = 0;
    let step = loop {
        let len = sizes[k % sizes.len()];
        k += 1;
        if cancel_at.is_some_and(|c| at as u64 >= c) {
            job.cancel();
        }
        assert!(pass.wait_for_room(len, Duration::from_secs(5)));
        for t in 0..2 {
            let (il, ir) = m.strip_input(StripId::Track(t)).unwrap();
            for i in 0..len {
                il[i] = song.track(t, at + i);
                ir[i] = -song.track(t, at + i);
            }
        }
        // The notes the player sends: a kick on Drums, a held pad on Synth.
        for i in at..at + len {
            let tick = i as u64 / 25;
            if i == 0 {
                pass.note(
                    tick,
                    0,
                    Msg::NoteOn {
                        ch: 9,
                        note: 36,
                        vel: 100,
                    },
                );
            }
            if i == 6000 {
                pass.note(
                    tick,
                    0,
                    Msg::NoteOff {
                        ch: 9,
                        note: 36,
                        vel: 0,
                    },
                );
            }
            if i == 1000 {
                pass.note(
                    tick,
                    1,
                    Msg::NoteOn {
                        ch: 0,
                        note: 60,
                        vel: 90,
                    },
                );
            }
        }
        let mut l = vec![0.0f32; len];
        let mut r = vec![0.0f32; len];
        m.process(&Ctx::new(RATE, at as u64), &mut l, &mut r);
        out.extend_from_slice(&l);
        at += len;
        if at >= song.range && ended_at.is_none() {
            ended_at = Some(at as u64);
        }
        match pass.period(&m, len, at >= song.range) {
            Step::Go => assert!(at < 60 * RATE as usize, "runs away"),
            s => break s,
        }
    };
    pass.detach(&mut m);
    assert!(m.capture(3).is_none(), "the capture points are free again");
    let progress = job.progress();
    if step == Step::Done {
        assert_eq!(progress.played, at as u64);
        assert!(progress.tail);
    }
    let outcome = job.finish(pass);
    match step {
        Step::Done => assert!(matches!(outcome, Outcome::Done(_)), "{outcome:?}"),
        Step::Cancelled => assert_eq!(outcome, Outcome::Cancelled),
        s => panic!("{s:?}: {outcome:?}"),
    }
    Run {
        outcome,
        out,
        most,
        ended_at: ended_at.unwrap_or(0),
        played: at as u64,
    }
}

fn done(r: &Run) -> &Done {
    match &r.outcome {
        Outcome::Done(d) => d,
        o => panic!("{o:?}"),
    }
}

#[test]
fn the_mixdown_is_the_output_and_ends_where_the_sound_does() {
    let folder = scratch("mixdown");
    let song = Song {
        range: 24_000,
        sound: 24_000,
        tail: 14_400,
        endless: false,
    };
    let r = run(&folder, Mode::Fast, song, &[256, 97, 512, 1, 333], None);
    let d = done(&r);
    let most = r.most as usize;
    assert!(most >= 300, "{most}");
    let names: Vec<String> = d
        .wavs
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["Mixdown.wav", "Drums.wav", "Synth.wav"]);
    assert_eq!(d.silent, ["Delay"]);
    assert!(
        !folder.join("Delay.wav").exists(),
        "a silent stem is left out"
    );
    // The files reach past the range, into the tail, and stop where it
    // has died away (below -90 dBFS), not 2 s of silence later.
    let frames = d.frames as usize;
    assert!(frames > song.range + song.tail * 3 / 4, "{frames}");
    assert!(frames <= song.range + song.tail, "{frames}");
    let mix = read24(&std::fs::read(&d.wavs[0]).unwrap());
    assert_eq!(mix.len(), frames);
    for (i, v) in mix.iter().enumerate() {
        assert!((v - r.out[most + i]).abs() <= 0.5 * LSB + 1e-9, "frame {i}");
    }
    // The song starts on the first frame.
    assert!(mix[1].abs() > 1e-3);
    // Each stem is its track alone, lined up to the song.
    let synth = read24(&std::fs::read(&d.wavs[2]).unwrap());
    assert_eq!(synth.len(), frames);
    for i in [1usize, 500, 12_345, song.range - 1] {
        assert!((synth[i] - song.track(1, i)).abs() < 1e-5, "frame {i}");
    }
    assert!(synth[song.range + 10].abs() < 1e-6);
    // The MIDI file: the conductor and one track for each that played.
    let midi = d.midi.as_ref().unwrap();
    assert_eq!(midi.file_name().unwrap(), "Night drive.mid");
    let b = std::fs::read(midi).unwrap();
    assert_eq!(&b[0..4], b"MThd");
    assert_eq!(u16::from_be_bytes([b[8], b[9]]), 1);
    assert_eq!(u16::from_be_bytes([b[10], b[11]]), 3);
    assert_eq!(u16::from_be_bytes([b[12], b[13]]), 960);
    let held = b.windows(3).filter(|w| *w == [0x80, 60, 64]).count();
    assert_eq!(held, 1, "the held note is let go at the end");
    assert_eq!(d.notes_dropped, 0);
    assert!(r
        .outcome
        .words()
        .starts_with("Exported 0:00 to mpfree-export-test-mixdown"));
    assert!(r
        .outcome
        .words()
        .ends_with(": 3 WAV files and a MIDI file (Delay was silent, so left out)"));
    std::fs::remove_dir_all(&folder).unwrap();
}

#[test]
fn either_mode_and_any_block_size_make_the_same_files() {
    let song = Song {
        range: 12_000,
        sound: 12_000,
        tail: 4_800,
        endless: false,
    };
    let (fa, fb) = (scratch("fast"), scratch("listen"));
    let a = run(&fa, Mode::Fast, song, &[128], None);
    let b = run(&fb, Mode::Listen, song, &[1, 7, 64, 300, 511], None);
    let (da, db) = (done(&a), done(&b));
    assert_eq!(da.frames, db.frames);
    for (x, y) in da.wavs.iter().zip(&db.wavs) {
        assert_eq!(std::fs::read(x).unwrap(), std::fs::read(y).unwrap());
    }
    assert_eq!(
        std::fs::read(da.midi.as_ref().unwrap()).unwrap(),
        std::fs::read(db.midi.as_ref().unwrap()).unwrap()
    );
    std::fs::remove_dir_all(&fa).unwrap();
    std::fs::remove_dir_all(&fb).unwrap();
}

#[test]
fn a_silent_end_keeps_the_whole_range() {
    let folder = scratch("silent-end");
    // The sound stops long before the range does.
    let song = Song {
        range: 3 * RATE as usize,
        sound: RATE as usize / 2,
        tail: 0,
        endless: false,
    };
    let r = run(&folder, Mode::Fast, song, &[480], None);
    let d = done(&r);
    assert_eq!(d.frames, r.ended_at, "the range, to its last frame");
    // It didn't wait 2 s past the range: the end was already quiet.
    assert!(r.played <= r.ended_at + u64::from(r.most) + 480);
    std::fs::remove_dir_all(&folder).unwrap();
}

#[test]
fn a_tail_that_never_dies_stops_after_ten_seconds() {
    let folder = scratch("endless");
    let song = Song {
        range: 4_800,
        sound: 4_800,
        tail: 0,
        endless: true,
    };
    let r = run(&folder, Mode::Fast, song, &[512], None);
    let d = done(&r);
    let max = (pass_tail_max() * f64::from(RATE)) as u64;
    assert!(r.played >= r.ended_at + max && r.played < r.ended_at + max + 512);
    assert_eq!(d.frames + u64::from(r.most), r.played);
    std::fs::remove_dir_all(&folder).unwrap();
}

fn pass_tail_max() -> f64 {
    mpfree_export::pass::TAIL_MAX_S
}

#[test]
fn a_cancel_leaves_nothing_behind() {
    let folder = scratch("cancel");
    let song = Song {
        range: 48_000,
        sound: 48_000,
        tail: 0,
        endless: false,
    };
    let r = run(&folder, Mode::Listen, song, &[256], Some(10_000));
    assert_eq!(r.outcome, Outcome::Cancelled);
    assert!(!folder.exists(), "the folder export made is gone too");
    assert_eq!(r.outcome.words(), "Export cancelled, and its files removed");
}

#[test]
fn a_folder_with_files_in_it_is_refused() {
    let folder = scratch("taken");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("keep.txt"), b"mine").unwrap();
    let song = Song {
        range: 100,
        sound: 100,
        tail: 0,
        endless: false,
    };
    let e = start(plan(&folder, Mode::Fast, song)).err().unwrap();
    assert_eq!(e.kind(), std::io::ErrorKind::AlreadyExists);
    assert_eq!(std::fs::read(folder.join("keep.txt")).unwrap(), b"mine");
    let mut nothing = plan(&scratch("nothing"), Mode::Fast, song);
    nothing.streams.clear();
    nothing.midi = None;
    assert!(start(nothing).is_err());
    std::fs::remove_dir_all(&folder).unwrap();
}

#[test]
fn a_mixer_without_the_strips_fails_at_once() {
    let folder = scratch("no-strip");
    let mut m = Mixer::new(
        MixerConfig {
            inputs: 0,
            pads: 0,
            tracks: 1,
            slots: 1,
            max_latency_ms: 10.0,
        },
        RATE,
        64,
    );
    let song = Song {
        range: 100,
        sound: 100,
        tail: 0,
        endless: false,
    };
    // The plan asks for a second track this mixer doesn't have.
    let (mut pass, job) = start(plan(&folder, Mode::Fast, song)).unwrap();
    assert!(!pass.attach(&mut m));
    assert!(m.capture(3).is_none() && m.capture(4).is_none());
    assert_eq!(pass.period(&m, 64, false), Step::Failed(Failure::Capture));
    let o = job.finish(pass);
    assert!(matches!(o, Outcome::Failed(_)), "{o:?}");
    assert!(!folder.exists());
}

/// A two-track mixer that has played a block, as the app's always has.
fn played_mixer() -> Mixer {
    let mut m = Mixer::new(
        MixerConfig {
            inputs: 0,
            pads: 0,
            tracks: 2,
            slots: 2,
            max_latency_ms: 50.0,
        },
        RATE,
        64,
    );
    let mut z = (vec![0.0f32; 64], vec![0.0f32; 64]);
    m.process(&Ctx::new(RATE, 0), &mut z.0, &mut z.1);
    m
}

#[test]
fn look_ahead_put_in_during_the_pass_fails_it() {
    let folder = scratch("latency");
    let mut m = played_mixer();
    let song = Song {
        range: 48_000,
        sound: 48_000,
        tail: 0,
        endless: false,
    };
    let (mut pass, job) = start(plan(&folder, Mode::Listen, song)).unwrap();
    assert!(pass.attach(&mut m));
    let mut z = (vec![0.0f32; 64], vec![0.0f32; 64]);
    m.process(&Ctx::new(RATE, 0), &mut z.0, &mut z.1);
    assert_eq!(pass.period(&m, 64, false), Step::Go);
    m.chain_mut(StripId::Track(1)).unwrap().push(Box::new(Lag {
        n: 300,
        buf: [vec![0.0; 301], vec![0.0; 301]],
        w: 0,
    }));
    m.process(&Ctx::new(RATE, 64), &mut z.0, &mut z.1);
    assert_eq!(pass.period(&m, 64, false), Step::Failed(Failure::Latency));
    pass.detach(&mut m);
    let o = job.finish(pass);
    assert!(o.words().contains("would no longer line up"), "{o:?}");
    assert!(!folder.exists());
}

#[test]
fn stop_on_the_sound_thread_cancels() {
    let folder = scratch("stop");
    let mut m = played_mixer();
    let song = Song {
        range: 48_000,
        sound: 48_000,
        tail: 0,
        endless: false,
    };
    let (mut pass, job) = start(plan(&folder, Mode::Listen, song)).unwrap();
    assert!(pass.attach(&mut m));
    let mut z = (vec![0.0f32; 64], vec![0.0f32; 64]);
    m.process(&Ctx::new(RATE, 0), &mut z.0, &mut z.1);
    assert_eq!(pass.period(&m, 64, false), Step::Go);
    pass.cancel();
    assert_eq!(pass.period(&m, 64, false), Step::Cancelled);
    pass.detach(&mut m);
    assert_eq!(job.finish(pass), Outcome::Cancelled);
    assert!(!folder.exists());
}

#[test]
fn progress_reads_as_time() {
    let p = Progress {
        played: 72 * 48_000,
        written: 0,
        length: Some(230 * 48_000),
        tail: false,
        rate: 48_000,
        mode: Mode::Fast,
    };
    assert_eq!(p.words(), "1:12 of 3:50");
    assert_eq!(Progress { length: None, ..p }.words(), "1:12");
    assert_eq!(Progress { tail: true, ..p }.words(), "1:12 · the tail");
    assert_eq!(clock(59 * 48_000 + 47_999, 48_000), "0:59");
}
