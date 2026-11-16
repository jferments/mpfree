//! The library's worker thread: every listing, header read and file
//! operation runs here, never on the UI or audio thread. The UI sends a
//! [`Job`] with a number and polls [`Worker::take`] each frame.
//!
//! A job sent in a [`Lane`] ([`Worker::send_in`]) is skipped when a newer
//! job of the same lane was sent before it started: moving through folders
//! or turning the dial over a long list queues one listing and one header
//! read that matter, not one for every step.

use crate::{AudioInfo, Entry, Library, Loc, NewSample, Snapshot, TrashItem, Version};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread;
use std::time::Instant;

/// Jobs where only the newest one sent matters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lane {
    /// The folder's listing and its sizes.
    List = 0,
    /// The selected item's header, picture and facts.
    Detail = 1,
}

const LANES: usize = 2;

/// Work for the library.
#[derive(Clone, Debug, PartialEq)]
pub enum Job {
    List(Loc),
    Info(Loc),
    /// A quick (min, max) picture of a WAV, this many columns wide.
    Overview(Loc, usize),
    NewFolder {
        parent: Loc,
        name: String,
    },
    Rename {
        loc: Loc,
        name: String,
    },
    Copy {
        from: Loc,
        to: Loc,
    },
    Move {
        from: Loc,
        to: Loc,
    },
    Delete(Loc),
    /// Each entry's size and time in a folder [`Library::list`] listed
    /// without them, at most this many.
    Sizes(Loc, usize),
    /// A kit, as a new folder ([`Library::save_kit`]).
    SaveKit {
        folder: Loc,
        name: String,
        text: String,
        sounds: Vec<NewSample>,
    },
    Trash,
    Restore(String),
    EmptyTrash,
    /// A whole file of at most this many bytes.
    Read(Loc, u64),
    SaveProject {
        project: Loc,
        snap: Snapshot,
        keep: usize,
        /// Sounds the parts name, written first where they aren't yet.
        samples: Vec<NewSample>,
    },
    OpenProject(Loc),
    /// The unsaved work, into autosave's project
    /// ([`Library::autosave_project`]), keeping no earlier saves. The
    /// snapshot's meta is marked as autosave's here.
    Autosave {
        snap: Snapshot,
        /// Sounds the parts name, written first where they aren't yet.
        samples: Vec<NewSample>,
    },
    /// A project's earlier saves, newest first.
    Versions(Loc),
    /// One earlier save of a project, by its id.
    OpenVersion(Loc, String),
    /// Names holding this text, anywhere in the library, at most this many.
    Search(String, usize),
    /// A whole new file (a recording, a kit), off the UI thread.
    WriteNew {
        folder: Loc,
        name: String,
        bytes: Vec<u8>,
    },
}

/// What a job gave.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    Listed(Loc, Vec<Entry>),
    /// (name, bytes, modified) for each entry of the folder.
    Sizes(Loc, Vec<(String, u64, u64)>),
    /// A kit saved, here.
    KitSaved(Loc),
    /// A job of a [`Lane`] that a newer one made pointless; not run.
    Skipped,
    Info(Loc, AudioInfo),
    /// None when the file is not a WAV the picture can be read from.
    Overview(Loc, Option<Vec<(f32, f32)>>),
    /// A file or folder made, renamed, copied, moved or restored: where it is now.
    At(Loc),
    Deleted(TrashItem),
    Trash(Vec<TrashItem>),
    Read(Loc, Vec<u8>),
    Saved(Loc),
    /// The unsaved work saved into autosave's project, here.
    Autosaved(Loc),
    Project(Loc, Snapshot),
    Versions(Loc, Vec<Version>),
    /// An earlier save, read: the project, the version's id, its parts.
    Version(Loc, String, Snapshot),
    /// What a search found: the text searched for, then each hit.
    Found(String, Vec<(Loc, Entry)>),
    /// The Trash emptied: bytes freed.
    Emptied(u64),
}

/// A job's number and what came of it (or why it failed, in words).
#[derive(Clone, Debug, PartialEq)]
pub struct Done {
    pub id: u64,
    pub result: Result<Outcome, String>,
    /// How long the job took on the worker, in milliseconds.
    pub took_ms: u64,
}

/// The thread named `mpfree-library`, running jobs one at a time in order.
pub struct Worker {
    tx: Option<Sender<(u64, Option<Lane>, Job)>>,
    rx: Receiver<Done>,
    next: u64,
    /// The newest job's number sent in each lane.
    newest: Arc<[AtomicU64; LANES]>,
}

impl Worker {
    pub fn start(lib: Library) -> Worker {
        let (tx, jobs) = mpsc::channel::<(u64, Option<Lane>, Job)>();
        let (out, rx) = mpsc::channel();
        let newest: Arc<[AtomicU64; LANES]> = Arc::new([AtomicU64::new(0), AtomicU64::new(0)]);
        let seen = newest.clone();
        let spawned = thread::Builder::new()
            .name("mpfree-library".into())
            .spawn(move || {
                for (id, lane, job) in jobs {
                    let stale = lane.is_some_and(|l| seen[l as usize].load(Ordering::Acquire) > id);
                    let t = Instant::now();
                    let result = if stale {
                        Ok(Outcome::Skipped)
                    } else {
                        run(&lib, job).map_err(|e| e.to_string())
                    };
                    let took_ms = t.elapsed().as_millis() as u64;
                    if out
                        .send(Done {
                            id,
                            result,
                            took_ms,
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            });
        Worker {
            tx: spawned.ok().map(|_| tx),
            rx,
            next: 1,
            newest,
        }
    }

    /// Queues a job; its number comes back with the result. 0 if the
    /// worker is not running (nothing will come back).
    pub fn send(&mut self, job: Job) -> u64 {
        self.queue(None, job)
    }

    /// Queues a job in `lane`: it is skipped ([`Outcome::Skipped`]) if
    /// another job is sent in the same lane before it starts.
    pub fn send_in(&mut self, lane: Lane, job: Job) -> u64 {
        self.queue(Some(lane), job)
    }

    fn queue(&mut self, lane: Option<Lane>, job: Job) -> u64 {
        let id = self.next;
        self.next += 1;
        if let Some(l) = lane {
            self.newest[l as usize].store(id, Ordering::Release);
        }
        match &self.tx {
            Some(tx) if tx.send((id, lane, job)).is_ok() => id,
            _ => 0,
        }
    }

    /// What has finished since the last call; never blocks.
    pub fn take(&self) -> Vec<Done> {
        self.rx.try_iter().collect()
    }

    /// Waits for one result (tests and the preview).
    pub fn wait(&self) -> Option<Done> {
        self.rx.recv().ok()
    }
}

/// Runs one job on the calling thread.
pub fn run(lib: &Library, job: Job) -> std::io::Result<Outcome> {
    Ok(match job {
        Job::List(l) => Outcome::Listed(l.clone(), lib.list(&l)?),
        Job::Info(l) => Outcome::Info(l.clone(), lib.audio_info(&l)?),
        Job::Overview(l, cols) => Outcome::Overview(l.clone(), lib.overview(&l, cols)?),
        Job::NewFolder { parent, name } => Outcome::At(lib.new_folder(&parent, &name)?),
        Job::Rename { loc, name } => Outcome::At(lib.rename(&loc, &name)?),
        Job::Copy { from, to } => Outcome::At(lib.copy(&from, &to)?),
        Job::Move { from, to } => Outcome::At(lib.move_to(&from, &to)?),
        Job::Delete(l) => Outcome::Deleted(lib.delete(&l)?),
        Job::Sizes(l, max) => Outcome::Sizes(l.clone(), lib.sizes(&l, max)?),
        Job::SaveKit {
            folder,
            name,
            text,
            sounds,
        } => Outcome::KitSaved(lib.save_kit(&folder, &name, &text, &sounds)?),
        Job::Trash => Outcome::Trash(lib.trash()?),
        Job::Read(l, max) => Outcome::Read(l.clone(), lib.read(&l, max)?),
        Job::SaveProject {
            project,
            snap,
            keep,
            samples,
        } => {
            lib.save_project_with(&project, &snap, keep, &samples)?;
            Outcome::Saved(project)
        }
        Job::Autosave { mut snap, samples } => {
            snap.meta.mark_autosave();
            let loc = lib.autosave_project()?;
            lib.save_project_with(&loc, &snap, 0, &samples)
                .map_err(|e| std::io::Error::new(e.kind(), format!("autosave: {e}")))?;
            Outcome::Autosaved(loc)
        }
        Job::OpenProject(l) => Outcome::Project(l.clone(), lib.open_project(&l)?),
        Job::Versions(l) => Outcome::Versions(l.clone(), lib.versions(&l)?),
        Job::OpenVersion(l, id) => {
            let snap = lib.open_version(&l, &id)?;
            Outcome::Version(l, id, snap)
        }
        Job::Search(q, max) => Outcome::Found(q.clone(), lib.search(&q, max)?),
        Job::Restore(id) => Outcome::At(lib.restore(&id)?),
        Job::EmptyTrash => Outcome::Emptied(lib.empty_trash()?),
        Job::WriteNew {
            folder,
            name,
            bytes,
        } => Outcome::At(lib.write_new(&folder, &name, &bytes)?),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{tests::temp, Place};

    /// An autosave job writes the unsaved work into autosave's project,
    /// marked as autosave's, and says where.
    #[test]
    fn an_autosave_goes_to_autosaves_own_project() {
        let d = temp();
        let lib = Library::at(&d).unwrap();
        let snap = Snapshot {
            meta: crate::Meta {
                name: "Autosave of a new project".into(),
                saved: 9,
                ..Default::default()
            },
            parts: vec![("transport".into(), "bpm = 100\n".into())],
        };
        let at = Loc::top(Place::Projects)
            .join(crate::project::AUTOSAVE)
            .unwrap();
        let out = run(
            &lib,
            Job::Autosave {
                snap,
                samples: Vec::new(),
            },
        )
        .unwrap();
        assert_eq!(out, Outcome::Autosaved(at.clone()));
        let back = lib.open_project(&at).unwrap();
        assert!(back.meta.is_autosave());
        assert_eq!(back.part("transport"), Some("bpm = 100\n"));
    }

    /// Of several jobs queued in one lane before the worker gets to them,
    /// only the newest runs; jobs outside the lane always run.
    #[test]
    fn a_newer_job_in_a_lane_skips_the_older_ones() {
        let d = temp();
        let lib = Library::at(&d).unwrap();
        // Hold the worker on a job that waits, so the others queue up.
        let (go, wait) = mpsc::channel::<()>();
        let wait = std::sync::Mutex::new(wait);
        let gate = NewSample::new(
            "gate.wav",
            Arc::new(move |w: &mut dyn std::io::Write| {
                let _ = wait.lock().map(|r| r.recv());
                w.write_all(b"x")
            }),
        );
        let mut w = Worker::start(lib);
        let kits = Loc::top(Place::Kits);
        w.send(Job::SaveKit {
            folder: kits.clone(),
            name: "Gate".into(),
            text: "t".into(),
            sounds: vec![gate],
        });
        let s = Loc::top(Place::Samples);
        let a = w.send_in(Lane::List, Job::List(s.clone()));
        let b = w.send_in(Lane::List, Job::List(kits.clone()));
        let c = w.send(Job::List(s.clone()));
        go.send(()).unwrap();
        let mut got = Vec::new();
        while got.len() < 4 {
            got.push(w.wait().unwrap());
        }
        assert!(matches!(got[0].result, Ok(Outcome::KitSaved(_))));
        assert_eq!(got[1].id, a);
        assert_eq!(got[1].result, Ok(Outcome::Skipped));
        assert_eq!(got[2].id, b);
        assert!(matches!(&got[2].result, Ok(Outcome::Listed(l, e)) if *l == kits && e.len() == 1));
        assert_eq!(got[3].id, c);
        assert!(matches!(got[3].result, Ok(Outcome::Listed(..))));
    }

    #[test]
    fn jobs_come_back_in_order_with_their_numbers() {
        let d = temp();
        let mut w = Worker::start(Library::at(&d).unwrap());
        let s = Loc::top(Place::Samples);
        let a = w.send(Job::NewFolder {
            parent: s.clone(),
            name: "Drums".into(),
        });
        let b = w.send(Job::List(s.clone()));
        let c = w.send(Job::Rename {
            loc: s.join("Nope").unwrap(),
            name: "x".into(),
        });
        let r1 = w.wait().unwrap();
        assert_eq!(r1.id, a);
        assert_eq!(r1.result, Ok(Outcome::At(s.join("Drums").unwrap())));
        let r2 = w.wait().unwrap();
        assert_eq!(r2.id, b);
        match r2.result {
            Ok(Outcome::Listed(l, e)) => {
                assert_eq!(l, s);
                assert_eq!(e[0].name, "Drums");
            }
            other => panic!("{other:?}"),
        }
        let r3 = w.wait().unwrap();
        assert_eq!(r3.id, c);
        assert!(r3.result.is_err());
        assert!(w.take().is_empty());
    }
}
