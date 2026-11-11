//! The library's worker thread: every listing, header read and file
//! operation runs here, never on the UI or audio thread. The UI sends a
//! [`Job`] with a number and polls [`Worker::take`] each frame.

use crate::{AudioInfo, Entry, Library, Loc, Snapshot, TrashItem};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

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
    Trash,
    Restore(String),
    EmptyTrash,
    /// A whole file of at most this many bytes.
    Read(Loc, u64),
    SaveProject {
        project: Loc,
        snap: Snapshot,
        keep: usize,
    },
    OpenProject(Loc),
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
    Info(Loc, AudioInfo),
    /// None when the file is not a WAV the picture can be read from.
    Overview(Loc, Option<Vec<(f32, f32)>>),
    /// A file or folder made, renamed, copied, moved or restored: where it is now.
    At(Loc),
    Deleted(TrashItem),
    Trash(Vec<TrashItem>),
    Read(Loc, Vec<u8>),
    Saved(Loc),
    Project(Loc, Snapshot),
    /// The Trash emptied: bytes freed.
    Emptied(u64),
}

/// A job's number and what came of it (or why it failed, in words).
#[derive(Clone, Debug, PartialEq)]
pub struct Done {
    pub id: u64,
    pub result: Result<Outcome, String>,
}

/// The thread named `mpfree-library`, running jobs one at a time in order.
pub struct Worker {
    tx: Option<Sender<(u64, Job)>>,
    rx: Receiver<Done>,
    next: u64,
}

impl Worker {
    pub fn start(lib: Library) -> Worker {
        let (tx, jobs) = mpsc::channel::<(u64, Job)>();
        let (out, rx) = mpsc::channel();
        let spawned = thread::Builder::new()
            .name("mpfree-library".into())
            .spawn(move || {
                for (id, job) in jobs {
                    let result = run(&lib, job).map_err(|e| e.to_string());
                    if out.send(Done { id, result }).is_err() {
                        break;
                    }
                }
            });
        Worker {
            tx: spawned.ok().map(|_| tx),
            rx,
            next: 1,
        }
    }

    /// Queues a job; its number comes back with the result. 0 if the
    /// worker is not running (nothing will come back).
    pub fn send(&mut self, job: Job) -> u64 {
        let id = self.next;
        self.next += 1;
        match &self.tx {
            Some(tx) if tx.send((id, job)).is_ok() => id,
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
        Job::Trash => Outcome::Trash(lib.trash()?),
        Job::Read(l, max) => Outcome::Read(l.clone(), lib.read(&l, max)?),
        Job::SaveProject {
            project,
            snap,
            keep,
        } => {
            lib.save_project(&project, &snap, keep)?;
            Outcome::Saved(project)
        }
        Job::OpenProject(l) => Outcome::Project(l.clone(), lib.open_project(&l)?),
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
