//! The Song view's Chords lane in the time engine (composing step 6, board
//! C): the chords of each row the song's sections play, heard in turn on a
//! thread of their own while the Song view shows, each heard again only
//! when its clips or chords change (as the Clips grid's strip hears its
//! one row: `adding.rs`). The screens lay them out along the song,
//! section by section (`TimeState::song_chords`).

use std::collections::BTreeMap;
use std::sync::mpsc::TryRecvError;
use std::sync::Arc;

use mpfree_compose::{AddPartJob, RowChords};
use mpfree_seq::Project;

use super::adding::{fingerprint, hear_job, Hearing};
use super::compose::Worker;

/// Each row's chords as the song's sections play them, for the screens.
pub(super) type SongChords = Arc<[(usize, Arc<RowChords>)]>;

pub(super) struct SongHearing {
    worker: Option<Worker<AddPartJob, RowChords>>,
    requests: u32,
    /// The hearing under way: its request, row and fingerprint.
    waiting: Option<(u32, usize, u64)>,
    /// Each row heard, with the fingerprint it was heard at.
    heard: BTreeMap<usize, (u64, Arc<RowChords>)>,
    /// `heard` as the screens take it.
    shown: SongChords,
}

impl SongHearing {
    pub(super) fn new() -> SongHearing {
        SongHearing {
            worker: None,
            requests: 0,
            waiting: None,
            heard: BTreeMap::new(),
            shown: Arc::from(Vec::new()),
        }
    }

    /// The rows heard so far, for the screens.
    pub(super) fn shown(&self) -> SongChords {
        Arc::clone(&self.shown)
    }

    /// Hear the first of `rows` whose clips or chords changed since it was
    /// heard, unless a hearing is under way (one at a time; the next comes
    /// when it answers). Rows no longer wanted are forgotten. True if what
    /// the screens show changed.
    pub(super) fn hear(
        &mut self,
        project: &Project,
        rows: Vec<(usize, Hearing)>,
    ) -> Result<bool, String> {
        let before = self.heard.len();
        self.heard.retain(|r, _| rows.iter().any(|(w, _)| w == r));
        let forgot = self.heard.len() != before;
        if forgot {
            self.reshow();
        }
        if self.waiting.is_some() {
            return Ok(forgot);
        }
        for (row, hearing) in rows {
            let print = fingerprint(project, row, &hearing.parts);
            if self.heard.get(&row).is_some_and(|h| h.0 == print) {
                continue;
            }
            let job = hear_job(project, row, hearing);
            if self.worker.is_none() {
                self.worker = Some(Worker::start("song chords", AddPartJob::hear)?);
            }
            self.requests = self.requests.wrapping_add(1);
            let n = self.requests;
            let worker = self.worker.as_ref().expect("started above");
            if worker.jobs.send((n, Box::new(job))).is_err() {
                self.worker = None;
                return Err("song chords: the composer stopped".into());
            }
            self.waiting = Some((n, row, print));
            break;
        }
        Ok(forgot)
    }

    /// Take the thread's answers. True if a row's chords came.
    pub(super) fn answers(&mut self) -> bool {
        let Some(w) = &self.worker else {
            return false;
        };
        let mut changed = false;
        loop {
            match w.done.try_recv() {
                Ok((n, line)) => {
                    if let Some((m, row, print)) = self.waiting {
                        if m == n {
                            self.waiting = None;
                            if let Some(line) = line {
                                self.heard.insert(row, (print, Arc::new(*line)));
                                changed = true;
                            }
                        }
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.worker = None;
                    self.waiting = None;
                    break;
                }
            }
        }
        if changed {
            self.reshow();
        }
        changed
    }

    fn reshow(&mut self) {
        self.shown = self
            .heard
            .iter()
            .map(|(&r, (_, c))| (r, Arc::clone(c)))
            .collect();
    }
}
