//! The loops in a project (the format is `mpfree_looper::save`): the
//! looper's part, `parts/looper.txt`, and each layer's audio as a WAV in
//! the project's samples, named by its content, so a layer that was saved
//! before is not written again.
//!
//! A save goes: [`crate::looping::LoopHost::save`] reads the loops as
//! they play, [`write`] (on a worker) puts the layers into the project and
//! gives the text, and the project browser saves the text with the other
//! engines' parts. Opening goes the other way: [`read`] (on a worker) and
//! [`crate::looping::LoopHost::load`]. Nothing here runs on the audio
//! loop: it writes and reads files. The project browser is not wired into
//! the app yet; these wait for it.
#![cfg_attr(not(test), allow(dead_code))]

use std::path::Path;

use mpfree_library::{Library, Loc};
use mpfree_looper::save::{from_text, from_wav, to_text, wav, Saved};

/// The looper's part file's owner.
pub const OWNER: &str = "looper";

/// Put each layer's audio into `project`'s samples (through a WAV in
/// `tmp`, removed after) and name it in its layer; give the part's text.
/// The project must have been saved once (its folder must be there).
pub fn write(
    lib: &Library,
    project: &Loc,
    saved: &mut Saved,
    tmp: &Path,
) -> std::io::Result<String> {
    let path = tmp.join(format!(".mpfree-loop-{}.wav", std::process::id()));
    for (n, f) in &mut saved.loops {
        for (k, y) in f.layers.iter_mut().enumerate() {
            let r =
                std::fs::write(&path, wav(&y.audio[..2 * f.frames], saved.rate)).and_then(|()| {
                    lib.add_sample_file(
                        project,
                        &path,
                        &format!("loop {} layer {}.wav", *n + 1, k + 1),
                    )
                });
            let _ = std::fs::remove_file(&path);
            y.file = r?;
        }
    }
    Ok(to_text(saved))
}

/// The loops from the part's text, each layer's audio read from
/// `project`'s samples.
pub fn read(lib: &Library, project: &Loc, text: &str) -> Result<Saved, String> {
    let mut saved = from_text(text)?;
    for (n, f) in &mut saved.loops {
        for y in &mut f.layers {
            let at = |e: String| format!("loop {}: {}: {e}", *n + 1, y.file);
            let path = lib
                .sample_path(project, &y.file)
                .map_err(|e| at(e.to_string()))?;
            let bytes = std::fs::read(path).map_err(|e| at(e.to_string()))?;
            let (audio, _) = from_wav(&bytes).map_err(at)?;
            if audio.len() < 2 * f.frames {
                return Err(at("shorter than the loop".into()));
            }
            y.audio = audio;
        }
    }
    Ok(saved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::looping::{LoopHost, PERIOD};
    use mpfree_library::{Meta, Place, Snapshot};
    use mpfree_looper::{Command, LayerCommand, Phase};
    use mpfree_transport::{Block, Command as T, Transport};

    /// A host with a one-bar loop of two layers, playing at 120 BPM.
    fn host() -> (LoopHost, Transport, Block) {
        let mut h = LoopHost::new(44_100, 20);
        let mut t = Transport::new(44_100, 120.0).unwrap();
        let mut b = Block::new();
        h.command(Command::SetBars(Some(1)));
        h.command(Command::Record);
        t.command(0, T::Play);
        let mut f = 0u64;
        let mut period = |h: &mut LoopHost, t: &mut Transport, b: &mut Block, hz: f64| {
            assert!(t.process(PERIOD as u32, b));
            let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
            for i in 0..PERIOD {
                l[i] = 0.3
                    * ((f + i as u64) as f64 * hz * std::f64::consts::TAU / 44_100.0).sin() as f32;
                r[i] = -l[i];
            }
            h.take_pads(&l, &r);
            let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
            h.period(b, &mut l, &mut r);
            f += PERIOD as u64;
        };
        while h.looper.status().phase != Phase::Playing {
            period(&mut h, &mut t, &mut b, 441.0);
        }
        h.command(Command::Overdub);
        for _ in 0..200 {
            period(&mut h, &mut t, &mut b, 882.0);
        }
        h.command(Command::Overdub);
        for _ in 0..800 {
            period(&mut h, &mut t, &mut b, 0.0);
        }
        let id = h.looper.layer_info(1).unwrap().id;
        h.looper.layer_command(id, LayerCommand::Level(0.25));
        (h, t, b)
    }

    fn run(h: &mut LoopHost, t: &mut Transport, b: &mut Block, periods: usize) -> Vec<f32> {
        let mut out = Vec::new();
        for _ in 0..periods {
            assert!(t.process(PERIOD as u32, b));
            let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
            h.period(b, &mut l, &mut r);
            out.extend(l.iter().zip(&r).flat_map(|(a, b)| [*a, *b]));
        }
        out
    }

    #[test]
    fn loops_go_into_a_project_and_come_back() {
        let (mut h, mut t, mut b) = host();
        assert_eq!(h.save(), "looper: saving the loops");
        let start = std::time::Instant::now();
        let mut saved = loop {
            run(&mut h, &mut t, &mut b, 1);
            if let Some(s) = h.take_saved() {
                break s;
            }
            std::thread::sleep(std::time::Duration::from_micros(100));
            assert!(start.elapsed().as_secs() < 60, "the save did not finish");
        };
        assert!(h.saving().is_none());
        assert_eq!(saved.loops.len(), 1);
        assert_eq!(saved.loops[0].1.layers.len(), 2);
        assert_eq!(saved.loops[0].1.layers[1].level, 0.25);

        let dir = std::env::temp_dir().join(format!("mpfree-loop-project-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let lib = Library::at(&dir).unwrap();
        let project = Loc::top(Place::Projects).join("Loops").unwrap();
        let mut snap = Snapshot {
            meta: Meta {
                name: "Loops".into(),
                tempo: 120.0,
                created: 1,
                saved: 1,
                app: "test".into(),
                extra: Vec::new(),
            },
            parts: Vec::new(),
        };
        lib.save_project(&project, &snap, 10).unwrap();
        let text = write(&lib, &project, &mut saved, &dir).unwrap();
        // The same layers again are not written twice.
        let again = write(&lib, &project, &mut saved.clone(), &dir).unwrap();
        assert_eq!(text, again);
        snap.parts.push((OWNER.into(), text));
        lib.save_project(&project, &snap, 10).unwrap();

        let open = lib.open_project(&project).unwrap();
        let back = read(&lib, &project, open.part(OWNER).unwrap()).unwrap();
        for ((_, a), (_, b)) in saved.loops.iter().zip(&back.loops) {
            assert_eq!(a.frames, b.frames);
            for (x, y) in a.layers.iter().zip(&b.layers) {
                assert_eq!(x.audio, y.audio);
                assert_eq!((x.level, &x.file), (y.level, &y.file));
            }
        }
        // Into a fresh host: it plays as the first one does, a bar on.
        let (mut h2, mut t2) = (
            LoopHost::new(44_100, 20),
            Transport::new(44_100, 120.0).unwrap(),
        );
        let mut b2 = Block::new();
        t2.command(0, T::Play);
        // Both at the same song frame.
        let skip = (t.frame() / PERIOD as u64) as usize;
        run(&mut h2, &mut t2, &mut b2, skip);
        assert_eq!(t2.frame(), t.frame());
        assert!(h2.load(back).starts_with("looper: loading loops 1"));
        let a = run(&mut h, &mut t, &mut b, 1500);
        let c = run(&mut h2, &mut t2, &mut b2, 1500);
        assert_eq!(h2.looper.status().phase, Phase::Playing);
        let n = a.len();
        let worst = (n - 2 * 88_200..n)
            .map(|i| (a[i] - c[i]).abs())
            .fold(0.0, f32::max);
        assert!(worst < 1e-5, "differs by {worst}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_loop_changing_while_it_is_read_starts_the_save_again() {
        let (mut h, mut t, mut b) = host();
        h.save();
        // Wait for the reading to start, then change a layer's mix.
        let start = std::time::Instant::now();
        while h.saving() == Some(0.0) {
            run(&mut h, &mut t, &mut b, 1);
            std::thread::sleep(std::time::Duration::from_micros(100));
            assert!(start.elapsed().as_secs() < 60);
        }
        let id = h.looper.layer_info(0).unwrap().id;
        h.looper.layer_command(id, LayerCommand::Level(0.5));
        run(&mut h, &mut t, &mut b, 2);
        assert_eq!(h.save_restarts, 1);
        let start = std::time::Instant::now();
        let saved = loop {
            run(&mut h, &mut t, &mut b, 1);
            if let Some(s) = h.take_saved() {
                break s;
            }
            std::thread::sleep(std::time::Duration::from_micros(100));
            assert!(start.elapsed().as_secs() < 60);
        };
        assert_eq!(saved.loops[0].1.layers[0].level, 0.5);
        // A load into a loop that is not empty keeps what it has.
        assert!(h.load(saved).contains("loops 1 were not empty"));
    }
}
