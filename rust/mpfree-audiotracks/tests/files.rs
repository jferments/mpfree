//! Recording to files (phase 3a): each take goes to its own file as it
//! records, through the stream layer's writer, and the file holds exactly
//! what went in, frame for frame; a save asked for as recording stops
//! waits for the files to finish and copies them whole; a project save
//! moves the takes over from the work folder; a track whose files can't
//! be made doesn't arm and says why; a take cut off (the host gone mid
//! take, as when the app is killed) is recovered at the next start up to
//! its last checkpoint, and moved where the musician finds it.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use mpfree_audiotracks::files::{
    layer_file_name, recover_into, FileEvent, TakeState, WriterConfig,
};
use mpfree_audiotracks::host::AudioHost;
use mpfree_audiotracks::save::from_wav;
use mpfree_audiotracks::{Command as A, Input, RecordingId};
use mpfree_transport::{Block, Command as T, Transport};

const RATE: u32 = 22_050;
const BAR: u64 = 22_050; // at 240 BPM
const P: u64 = 256;

/// The line input: left and right differ, and no two frames near each
/// other match, so a frame out of place shows.
fn sig(g: u64, c: usize) -> f32 {
    let v = ((g * 7919 + c as u64 * 104_729) % 20_011) as f32 / 20_011.0 - 0.5;
    if c == 0 {
        v
    } else {
        -v * 0.75
    }
}

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("mpfree-files-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

/// Quick rounds, so the tests see checkpoints in well under a second.
fn quick() -> WriterConfig {
    WriterConfig {
        interval_s: 0.05,
        max_interval_s: 0.2,
        commit_s: 0.01,
        ..WriterConfig::default()
    }
}

struct Rig {
    t: Transport,
    h: AudioHost,
    b: Block,
    frame: u64,
    live: bool,
}

impl Rig {
    fn new(dir: &Path) -> Rig {
        let mut h = AudioHost::new(RATE, 30);
        assert!(h.record_to(dir.to_path_buf(), quick()).is_ok());
        Rig {
            t: Transport::new(RATE, 240.0).unwrap(),
            h,
            b: Block::new(),
            frame: 0,
            live: true,
        }
    }

    fn period(&mut self) {
        let n = P as usize;
        let (mut inp, mut mix) = ([0.0f32; 2 * P as usize], [0.0f32; 2 * P as usize]);
        for j in 0..n {
            let g = self.frame + j as u64;
            if self.live {
                inp[2 * j] = sig(g, 0);
                inp[2 * j + 1] = sig(g, 1);
            }
        }
        self.t.process(n as u32, &mut self.b);
        let map = *self.t.tempo_map();
        self.h.period(&self.b, &map, &inp, &mut mix);
        self.frame += P;
    }

    fn run_to(&mut self, frame: u64) {
        while self.frame < frame {
            self.period();
        }
    }

    /// Periods until `done`, giving the worker and the writer time.
    #[track_caller]
    fn run_until(&mut self, mut done: impl FnMut(&mut Rig) -> bool) {
        let t0 = Instant::now();
        while !done(self) {
            assert!(t0.elapsed() < Duration::from_secs(30), "timed out");
            self.period();
            std::thread::sleep(Duration::from_micros(300));
        }
    }

    fn events(&mut self) -> Vec<FileEvent> {
        std::iter::from_fn(|| self.h.take_event()).collect()
    }
}

/// The frames of a WAV file.
fn wav_at(path: &Path) -> (Vec<f32>, usize) {
    let (audio, ch, rate) = from_wav(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(rate, RATE);
    (audio, ch)
}

#[test]
fn each_take_goes_to_its_own_file_frame_for_frame_and_a_save_copies_it_whole() {
    let dir = scratch("frames");
    let mut r = Rig::new(&dir);
    r.h.set_name(0, "Vocal");
    r.h.set_name(1, "Keys: L/R");
    r.h.command(A::Arm(0, true));
    r.h.command(A::Arm(1, true));
    r.h.command(A::SetInput(1, Input::Stereo));
    // The files are made as the tracks arm, before anything records.
    r.run_until(|r| r.h.armed_file(0) == Some(true) && r.h.armed_file(1) == Some(true));
    let (rec0, _) = r.h.tracks.next_recording(0).unwrap();
    let (rec1, ch1) = r.h.tracks.next_recording(1).unwrap();
    assert_eq!(ch1, 2);
    assert!(dir.join(format!("Vocal_{rec0}.wav.recording")).exists());
    assert!(dir.join(format!("Keys- L-R_{rec1}.wav.recording")).exists());
    // Two bars and a bit, then stop and save at once.
    let start = r.frame + 100;
    let stop = start + 2 * BAR + 777;
    r.t.command(start, T::Record);
    r.t.command(stop, T::Stop);
    r.run_until(|r| r.frame > start + 100 && r.h.file(0).is_some());
    assert_eq!(r.h.file(0).unwrap().state, TakeState::Recording);
    r.run_to(stop + 200);
    r.h.command(A::Arm(0, false));
    r.h.command(A::Arm(1, false));
    assert!(r.h.save());
    r.live = false;
    let mut saved = None;
    r.run_until(|r| {
        saved = r.h.take_saved();
        saved.is_some()
    });
    let saved = saved.unwrap();
    let takes: Vec<_> = r.h.tracks.takes().collect();
    assert_eq!(takes.len(), 2);
    assert_eq!(
        (takes[0].rec, takes[1].rec),
        (rec0, rec1),
        "the ids the files were made for"
    );
    for k in &takes {
        assert_eq!(
            r.h.file(k.id).unwrap().state,
            TakeState::Saved,
            "take {}",
            k.id
        );
        // Its file holds the take, frame for frame, and the take what
        // went in.
        let f = saved.takes.iter().find(|f| f.id == k.id).unwrap();
        let path = f
            .path
            .as_ref()
            .expect("copied whole from its file, not memory");
        assert!(f.audio.is_empty());
        assert_eq!(path.file_name().unwrap().to_str().unwrap(), f.file);
        assert!(f.file.ends_with(&format!("_{}.wav", k.rec)));
        let (audio, ch) = wav_at(path);
        assert_eq!(ch, k.channels);
        assert_eq!(audio.len(), k.frames * ch);
        let mut mem = vec![0.0; audio.len()];
        assert_eq!(r.h.tracks.read_take(k.id, 0, &mut mem), Some(k.frames));
        assert!(audio == mem, "take {}: the file is the take", k.id);
        // No latency: its frame 0 went in at the punch in.
        for (i, v) in audio.chunks_exact(ch).enumerate() {
            let g = start + i as u64;
            if ch == 1 {
                assert_eq!(v[0], sig(g, 0), "mono frame {i}");
            } else {
                assert_eq!((v[0], v[1]), (sig(g, 0), sig(g, 1)), "stereo frame {i}");
            }
        }
        // Past the punch out by the crossfade only.
        assert!(k.frames as u64 >= stop - start, "{} frames", k.frames);
    }
    assert!(r.events().is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_project_save_takes_the_files_over_from_the_work_folder() {
    let dir = scratch("over");
    let project = scratch("over-project");
    std::fs::create_dir_all(&project).unwrap();
    let mut r = Rig::new(&dir);
    r.h.set_name(0, "Bass");
    r.h.command(A::Arm(0, true));
    r.run_until(|r| r.h.armed_file(0) == Some(true));
    let start = r.frame + 100;
    r.t.command(start, T::Record);
    r.t.command(start + BAR, T::Stop);
    r.run_to(start + BAR + 200);
    r.h.command(A::Arm(0, false));
    r.run_until(|r| r.h.file(0).is_some_and(|f| f.state == TakeState::Saved));
    assert!(r.h.save());
    let mut saved = None;
    r.run_until(|r| {
        saved = r.h.take_saved();
        saved.is_some()
    });
    let k = saved.unwrap().takes.remove(0);
    let work = k.path.clone().unwrap();
    assert_eq!(work.parent(), Some(dir.as_path()));
    // The library copies it into the project's samples/, then says so.
    std::fs::copy(&work, project.join(&k.file)).unwrap();
    r.h.saved_into(project.clone());
    r.run_until(|_| !work.exists());
    // The next save copies it from the project.
    assert!(r.h.save());
    let mut again = None;
    r.run_until(|r| {
        again = r.h.take_saved();
        again.is_some()
    });
    let k2 = again.unwrap().takes.remove(0);
    assert_eq!(k2.file, k.file);
    assert_eq!(k2.path.unwrap(), project.join(&k.file));
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&project);
}

#[test]
fn a_track_whose_files_cant_be_made_doesnt_arm_and_says_why() {
    let dir = scratch("gone");
    let mut r = Rig::new(&dir);
    r.run_until(|r| r.frame > 10 * P && dir.exists());
    // The stick is pulled.
    std::fs::remove_dir_all(&dir).unwrap();
    r.h.command(A::Arm(2, true));
    r.run_until(|r| !r.h.tracks.armed(2) && r.frame > 20 * P);
    let ev = r.events();
    assert!(
        matches!(&ev[..], [FileEvent::ArmFailed { track: 2, why }] if why.contains("No such file")),
        "{ev:?}"
    );
    assert_eq!(r.h.armed_file(2), None);
    // Recording now records nothing on it.
    r.t.command(r.frame + 10, T::Record);
    r.run_to(r.frame + BAR);
    assert_eq!(r.h.tracks.takes().count(), 0);
}

/// Step 5d: the drive the takes record to is looked at on a thread of its
/// own; when it goes, System status hears Stick missing, and when it is
/// back, that it ended. Each look also goes where the page's stick meter
/// reads it (System status phase 3).
#[test]
fn the_drive_going_and_coming_back_is_told_to_system_status() {
    use mpfree_audiotracks::files::{Problem, Room};
    use mpfree_status::{kinds, Report};
    fn probe(dir: &Path) -> Option<Room> {
        dir.exists().then_some(Room {
            writable: true,
            free: 1 << 40,
            total: 1 << 41,
        })
    }
    let dir = scratch("drive");
    let mut r = Rig::new(&dir);
    let seen = r.h.drive_seen();
    assert_eq!(seen.get(), None, "not looked at before the watch starts");
    r.run_until(|r| r.frame > 10 * P && dir.exists());
    // The stick is pulled, then the drive is looked at.
    std::fs::remove_dir_all(&dir).unwrap();
    r.h.watch_drive(probe);
    let mut got = Vec::new();
    r.run_until(|r| {
        got.extend(std::iter::from_fn(|| r.h.take_problem()));
        !got.is_empty()
    });
    let missing = Problem {
        report: Report::began(&kinds::STICK_MISSING),
        track: None,
    };
    assert_eq!(got, [missing]);
    assert_eq!(seen.get(), Some(None), "the meter hears it's gone");
    // Back: the next look (within three seconds) ends it.
    std::fs::create_dir_all(&dir).unwrap();
    let mut back = Vec::new();
    r.run_until(|r| {
        back.extend(std::iter::from_fn(|| r.h.take_problem()));
        !back.is_empty()
    });
    assert_eq!(
        back,
        [Problem {
            report: Report::ended(&kinds::STICK_MISSING),
            track: None,
        }]
    );
    assert_eq!(
        seen.get(),
        Some(Some(Room {
            writable: true,
            free: 1 << 40,
            total: 1 << 41,
        }))
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_take_cut_off_is_recovered_at_the_next_start_up_to_its_last_checkpoint() {
    let dir = scratch("cut");
    let keep = scratch("cut-recordings");
    let mut r = Rig::new(&dir);
    r.h.set_name(0, "Gtr");
    r.h.command(A::Arm(0, true));
    r.run_until(|r| r.h.armed_file(0) == Some(true));
    let start = r.frame + 100;
    r.t.command(start, T::Record);
    // Record about as fast as it plays, so the writer commits rounds as
    // it goes.
    while r.frame < start + 3 * BAR {
        r.period();
        std::thread::sleep(Duration::from_micros(2_000));
    }
    let rec = r.h.tracks.take(0).unwrap().rec;
    // The app is killed: the host goes mid take, nothing stopped.
    drop(r);
    std::thread::sleep(Duration::from_millis(300));
    assert!(dir.join(format!("Gtr_{rec}.wav.recording")).exists());
    let found = recover_into(&dir, &keep).unwrap();
    let lines = &found.lines;
    assert!(
        lines[0].contains("cut off while it recorded; recovered up to 0:0"),
        "{lines:?}"
    );
    assert!(
        lines[1].contains("1 take not saved in a project was kept in Recordings"),
        "{lines:?}"
    );
    // System status hears it was recovered; its whole check matched, so
    // nothing is damaged and no found take needs a name.
    assert_eq!(
        found.reports,
        [mpfree_status::Report::happened(
            &mpfree_status::kinds::TAKE_RECOVERED,
            1
        )]
    );
    assert!(found.found.is_empty());
    let got = keep.join(format!("Gtr_{rec}.wav"));
    let (audio, ch) = wav_at(&got);
    assert_eq!(ch, 1);
    assert!(
        audio.len() > BAR as usize / 2,
        "{} frames recovered",
        audio.len()
    );
    for (i, v) in audio.iter().enumerate() {
        assert_eq!(*v, sig(start + i as u64, 0), "frame {i}");
    }
    // Nothing is left in the work folder but what may still be checked.
    let left: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.file_name().into_string().unwrap())
        .filter(|n| !n.ends_with(".unverified.wav"))
        .collect();
    assert!(left.is_empty(), "{left:?}");
    // A second start finds nothing more.
    assert_eq!(recover_into(&dir, &keep).unwrap(), Default::default());
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&keep);
}

impl Rig {
    /// Give `audio` as recording `rec`'s loop layer, a piece at a time as
    /// buffers come back, ending it if `end`.
    fn give_layer(&mut self, rec: RecordingId, audio: &[f32], end: bool) {
        assert!(self.h.layer_begin(rec, audio.len() / 2, 2));
        let mut at = 0;
        let t0 = Instant::now();
        while at < audio.len() {
            let Some(mut buf) = self.h.layer_buffer() else {
                assert!(t0.elapsed() < Duration::from_secs(30), "no buffer back");
                self.period();
                std::thread::sleep(Duration::from_micros(300));
                continue;
            };
            let n = buf.len().min(audio.len() - at);
            buf[..n].copy_from_slice(&audio[at..at + n]);
            self.h.layer_piece(rec, buf, n);
            at += n;
        }
        if end {
            self.h.layer_end(rec);
        }
    }

    #[track_caller]
    fn layer_said(&mut self) -> (RecordingId, Option<String>) {
        let mut said = None;
        self.run_until(|r| {
            said = r.h.layer_written();
            said.is_some()
        });
        said.unwrap()
    }
}

fn names_in(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(Result::ok)
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    v.sort();
    v
}

/// Audio tracks phase 4c.3: a loop's closed layer, given a piece at a
/// time, is written whole as `loop_ID.wav` in the work folder (no part
/// file left) and said once; a project save that holds a copy takes it
/// over; one dropped part-way, or short of its audio, leaves nothing; and
/// one a cut-off run left is found at the next start.
#[test]
fn a_loops_layer_goes_to_a_file_of_its_own_and_is_found_after_a_cut_off_run() {
    let dir = scratch("layers");
    let project = scratch("layers-project");
    let keep = scratch("layers-keep");
    std::fs::create_dir_all(&project).unwrap();
    let mut r = Rig::new(&dir);
    assert!(r.h.layer_files());
    let audio: Vec<f32> = (0..2 * 50_000u64)
        .map(|i| sig(i / 2, (i % 2) as usize))
        .collect();
    let rec = RecordingId(0xfeed_0001);
    r.give_layer(rec, &audio, true);
    assert_eq!(r.layer_said(), (rec, None));
    let path = dir.join(layer_file_name(rec));
    assert_eq!(wav_at(&path), (audio.clone(), 2));
    assert!(names_in(&dir).iter().all(|n| !n.ends_with(".part")));
    // A save that holds a copy takes it over.
    std::fs::copy(&path, project.join(layer_file_name(rec))).unwrap();
    r.h.saved_into(project.clone());
    r.run_until(|_| !path.exists());
    // Dropped part-way: nothing of it is kept.
    let rec2 = RecordingId(0xfeed_0002);
    r.give_layer(rec2, &audio, false);
    r.h.layer_drop(rec2);
    // Short of its audio: refused, nothing kept.
    let rec3 = RecordingId(0xfeed_0003);
    assert!(r.h.layer_begin(rec3, 1_000, 2));
    let buf = loop {
        if let Some(b) = r.h.layer_buffer() {
            break b;
        }
        r.period();
    };
    r.h.layer_piece(rec3, buf, 500);
    r.h.layer_end(rec3);
    let (said, why) = r.layer_said();
    assert_eq!(said, rec3);
    assert!(why.unwrap().contains("500 of its 2000 samples"));
    assert!(names_in(&dir)
        .iter()
        .all(|n| !n.contains("feed0002") && !n.contains("feed0003")));
    // Written, then the run cut off before a save: the next start keeps
    // it in Recordings, said as a loop layer.
    let rec4 = RecordingId(0xfeed_0004);
    r.give_layer(rec4, &audio, true);
    assert_eq!(r.layer_said(), (rec4, None));
    drop(r);
    let found = recover_into(&dir, &keep).unwrap();
    assert_eq!(
        found.lines,
        ["audio tracks: 1 loop layer not saved in a project was kept in Recordings"]
    );
    assert_eq!(wav_at(&keep.join(layer_file_name(rec4))), (audio, 2));
    for d in [&dir, &project, &keep] {
        let _ = std::fs::remove_dir_all(d);
    }
}

/// Record a bar on the armed track from a little after now, and run a
/// period past the stop: the new take's id.
fn record_a_bar(r: &mut Rig) -> u16 {
    let start = r.frame + 100;
    r.t.command(start, T::Record);
    r.t.command(start + BAR, T::Stop);
    r.run_until(|r| r.frame > start + 100 && r.h.tracks.takes().count() > 0);
    let id = r.h.tracks.takes().map(|k| k.id).max().unwrap();
    r.run_to(start + BAR + P);
    id
}

/// SD card plan, S2b: recording moves to another folder (the card's,
/// when it opens). An armed track's files for its next take go from the
/// folder before and are made again in the new one, and the next take
/// records there; takes saved before stay where they are, and a project
/// save takes the files over from both folders. Refused, the folder
/// given back, while a take is still being saved, while a save runs,
/// and while a loop layer's file is open.
#[test]
fn recording_moves_to_another_folder_only_with_nothing_in_flight() {
    let a = scratch("move-a");
    let b = scratch("move-b");
    let project = scratch("move-project");
    std::fs::create_dir_all(&project).unwrap();
    let mut r = Rig::new(&a);
    r.h.set_name(0, "Vocal");
    r.h.command(A::Arm(0, true));
    r.run_until(|r| r.h.armed_file(0) == Some(true));
    // A take in the first folder. Just stopped, the writer hasn't said
    // it is saved: refused.
    let first = record_a_bar(&mut r);
    assert_eq!(r.h.record_to(b.clone(), quick()), Err(b.clone()));
    r.run_until(|r| r.h.file(first).is_some_and(|f| f.state == TakeState::Saved));
    let (before, _) = r.h.tracks.next_recording(0).unwrap();
    let armed_before = a.join(format!("Vocal_{before}.wav.recording"));
    r.run_until(|r| r.h.armed_file(0) == Some(true));
    assert!(armed_before.exists());
    // A loop layer's file open: refused; dropped, no longer.
    let layer = RecordingId(0xfeed_0101);
    assert!(r.h.layer_begin(layer, 1_000, 2));
    assert_eq!(r.h.record_to(b.clone(), quick()), Err(b.clone()));
    r.h.layer_drop(layer);
    // A save running: refused.
    assert!(r.h.save());
    assert_eq!(r.h.record_to(b.clone(), quick()), Err(b.clone()));
    let mut saved = None;
    r.run_until(|r| {
        saved = r.h.take_saved();
        saved.is_some()
    });
    let in_a = saved.unwrap().takes.remove(0).path.unwrap();
    assert_eq!(in_a.parent(), Some(a.as_path()));
    // Nothing in flight: it moves. The armed track's files are made
    // again in the new folder, under a new id, and the old ones go.
    assert!(r.h.record_to(b.clone(), quick()).is_ok());
    r.run_until(|r| {
        r.h.armed_file(0) == Some(true)
            && r.h
                .tracks
                .next_recording(0)
                .is_some_and(|(n, _)| n != before)
    });
    let (after, _) = r.h.tracks.next_recording(0).unwrap();
    assert!(b.join(format!("Vocal_{after}.wav.recording")).exists());
    assert!(
        !armed_before.exists(),
        "the files made in the old folder go"
    );
    // The next take records into the new folder, frame for frame; the
    // one before stays where it was.
    let second = record_a_bar(&mut r);
    r.h.command(A::Arm(0, false));
    r.run_until(|r| {
        r.h.file(second)
            .is_some_and(|f| f.state == TakeState::Saved)
    });
    assert!(in_a.exists());
    assert!(r.events().is_empty());
    assert!(r.h.save());
    let mut saved = None;
    r.run_until(|r| {
        saved = r.h.take_saved();
        saved.is_some()
    });
    let saved = saved.unwrap();
    let k = saved.takes.iter().find(|k| k.id == second).unwrap();
    let in_b = k.path.clone().expect("copied whole from its file");
    assert_eq!(in_b, b.join(format!("Vocal_{after}.wav")));
    let take = r.h.tracks.take(second).unwrap();
    let (audio, ch) = wav_at(&in_b);
    assert_eq!(audio.len(), take.frames * ch);
    let mut mem = vec![0.0; audio.len()];
    assert_eq!(r.h.tracks.read_take(second, 0, &mut mem), Some(take.frames));
    assert!(audio == mem, "the file in the new folder is the take");
    // A project save that holds both takes takes them over from both
    // folders.
    for k in &saved.takes {
        std::fs::copy(k.path.as_ref().unwrap(), project.join(&k.file)).unwrap();
    }
    r.h.saved_into(project.clone());
    r.run_until(|_| !in_a.exists() && !in_b.exists());
    for d in [&a, &b, &project] {
        let _ = std::fs::remove_dir_all(d);
    }
}

/// A take the writer can never finish (its folder gone, so the rename
/// fails and it stays At risk: the writer retries for ever) doesn't keep
/// recording in the folder before. Recording moves, the armed track's
/// files are made in the new folder, and the take is still saved, from
/// memory.
#[test]
fn a_take_that_cant_be_finished_doesnt_hold_recording_in_the_old_folder() {
    let a = scratch("stuck-a");
    let b = scratch("stuck-b");
    let mut r = Rig::new(&a);
    r.h.set_name(0, "Vocal");
    r.h.command(A::Arm(0, true));
    r.run_until(|r| r.h.armed_file(0) == Some(true));
    let start = r.frame + 100;
    r.t.command(start, T::Record);
    r.run_until(|r| r.frame > start + BAR / 2 && r.h.tracks.takes().count() > 0);
    let id = r.h.tracks.takes().map(|k| k.id).max().unwrap();
    std::fs::remove_dir_all(&a).unwrap();
    let stop = r.frame + P;
    r.t.command(stop, T::Stop);
    r.run_to(stop + P);
    r.run_until(|r| {
        r.h.file(id)
            .is_some_and(|f| matches!(f.state, TakeState::AtRisk(_)))
    });
    let mut dir = Some(b.clone());
    r.run_until(|r| match r.h.record_to(dir.take().unwrap(), quick()) {
        Ok(()) => true,
        Err(d) => {
            dir = Some(d);
            false
        }
    });
    r.run_until(|r| r.h.armed_file(0) == Some(true));
    let (next, _) = r.h.tracks.next_recording(0).unwrap();
    assert!(b.join(format!("Vocal_{next}.wav.recording")).exists());
    r.h.command(A::Arm(0, false));
    assert!(r.h.save());
    let mut saved = None;
    r.run_until(|r| {
        saved = r.h.take_saved();
        saved.is_some()
    });
    let k = saved.unwrap().takes.remove(0);
    let take = r.h.tracks.take(id).unwrap();
    assert!(k.path.is_none(), "not a finished file");
    assert_eq!(k.audio.len(), take.frames * take.channels);
    let _ = std::fs::remove_dir_all(&b);
}

/// A loop layer begun twice (the worker replaces the first write) holds
/// a move to another folder only until its one answer comes.
#[test]
fn a_layer_begun_twice_holds_a_move_only_until_its_one_answer() {
    let a = scratch("twice-a");
    let b = scratch("twice-b");
    let mut r = Rig::new(&a);
    let audio: Vec<f32> = (0..2 * 1_000u64)
        .map(|i| sig(i / 2, (i % 2) as usize))
        .collect();
    let rec = RecordingId(0xfeed_0201);
    assert!(r.h.layer_begin(rec, 1_000, 2));
    r.give_layer(rec, &audio, true);
    assert_eq!(r.layer_said(), (rec, None));
    assert!(r.h.record_to(b.clone(), quick()).is_ok());
    for d in [&a, &b] {
        let _ = std::fs::remove_dir_all(d);
    }
}
