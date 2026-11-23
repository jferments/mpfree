//! What each instrument plays in, on the app's side (tunings step T5,
//! [tunings.md](../../../../docs/dev/design/tunings.md)): the song's
//! tuning and each instrument's own ([`crate::tunings::Tunings`]), kept
//! with the project's list of instruments, copied with Duplicate, saved in
//! the tunings part.
//!
//! The Synth tab's Tuning rows set the song's tuning (`synth_pads.rs`).
//! The shared synth plays its own or the song's through the pads'
//! settings ([`App::shared_tuning_follows`]); the audio loop reads
//! [`App::tunings`] each period and hands every other instrument its own
//! (`main.rs`, `TuningsHanded`). An instrument's own tuning gets its rows
//! with the phase 2 screens (T6); until then [`App::set_own_tuning`] sets
//! one.

use mpfree_arp::scale::NOTE_NAMES;
use mpfree_tuning::TuningPick;

use super::{App, Effect};
use crate::instrument_list::{InstrumentId, Kind};
use crate::tunings::Tunings;

impl App {
    /// The song's tuning and each instrument's own.
    pub fn tunings(&self) -> &Tunings {
        &self.tunings
    }

    /// The shared synth's tuning (its own, or the song's) into the pads'
    /// settings, which the synth plays; whether it changed.
    pub(super) fn shared_tuning_follows(&mut self) -> bool {
        let pick = self.tunings.pick(Some(InstrumentId::SHARED_SYNTH));
        let changed = self.synth_pads.tuning != pick;
        self.synth_pads.tuning = pick;
        changed
    }

    /// Gives instrument `id` a tuning of its own, or with None puts it on
    /// Song's again. Says so in the log; an instrument the list hasn't
    /// got, or a change to nothing, does nothing.
    // The phase 2 screens' Tuning rows call it (T6); the tests do now.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn set_own_tuning(&mut self, id: InstrumentId, pick: Option<TuningPick>) -> Vec<Effect> {
        let Some(kind) = self.instrument_list.kind(id) else {
            return Vec::new();
        };
        if !self.tunings.set_own(id, pick) {
            return Vec::new();
        }
        self.project_edit = true;
        let who = instrument_words(kind);
        let line = match pick {
            Some(p) => format!(
                "tunings: {who} plays its own tuning, {}",
                self.pick_label(p)
            ),
            None => format!(
                "tunings: {who} plays the song's tuning, {}",
                self.pick_label(self.tunings.song())
            ),
        };
        let mut fx = vec![Effect::Note(line)];
        if self.shared_tuning_follows() {
            fx.push(Effect::SynthPads(self.synth_pads));
        }
        fx
    }

    /// A tuning as the log names it: "Just (5-limit) on D, A4 432 Hz",
    /// "12-TET", a tuning file by its name (as `TuningPick::label`, with the
    /// files the app knows).
    pub fn pick_label(&self, p: TuningPick) -> String {
        let file = p.file.and_then(|f| self.tuning_files.get(usize::from(f)));
        let (name, has_root) = match file {
            Some((name, true)) => return name.clone(),
            Some((name, false)) => (name.as_str(), true),
            None => (p.builtin.name(), p.builtin.has_root()),
        };
        let mut s = name.to_string();
        if has_root {
            s.push_str(" on ");
            s.push_str(NOTE_NAMES[usize::from(p.root % 12)]);
        }
        if p.a4_hz != 440 {
            s.push_str(&format!(", A4 {} Hz", p.a4_hz));
        }
        s
    }

    /// Track `track`'s own instrument is to be a copy of track `owner`'s
    /// (Duplicate): it takes that one's tuning, now if it has an id, else
    /// once the list gives it one ([`App::tunings_follow_list`]).
    pub(super) fn copy_tuning(&mut self, track: usize, owner: usize) {
        let Some(from) = self.instrument_list.own_of(owner) else {
            return;
        };
        match self.instrument_list.own_of(track) {
            Some(to) => {
                self.tunings.copy(from, to);
                self.tuning_copies[track] = None;
                if self.tunings.own(to).is_some() {
                    self.project_edit = true;
                }
            }
            None => self.tuning_copies[track] = Some(from),
        }
    }

    /// The list of instruments changed: an instrument gone takes its
    /// tuning with it, and a copy that has its id takes its tuning.
    pub(super) fn tunings_follow_list(&mut self) {
        for track in 0..self.tuning_copies.len() {
            let Some(from) = self.tuning_copies[track] else {
                continue;
            };
            if let Some(to) = self.instrument_list.own_of(track) {
                self.tuning_copies[track] = None;
                if to != from {
                    self.tunings.copy(from, to);
                }
            }
        }
        if self.tunings.follow(&self.instrument_list) {
            self.project_edit = true;
        }
        // (The shared synth is always in the list: its tuning stays.)
    }

    /// The project's tunings part ([`Tunings::text`]): just its first line
    /// while no instrument has its own, and then not saved.
    pub fn tunings_part(&self) -> String {
        self.tunings.text(|f| {
            self.tuning_files
                .get(usize::from(f))
                .map(|(n, _)| n.as_str())
        })
    }

    /// A project's tunings part, read after its synth part (which has the
    /// song's tuning) and its list of instruments. None (New project, or
    /// one with no instrument on a tuning of its own): every instrument is
    /// on Song's. A part that doesn't read keeps that too and says why (the
    /// opener refuses such a project first: `project::check`), as does an
    /// instrument the list hasn't got. Returns the lines for the log and
    /// the effect for the shared synth.
    pub fn open_tunings(&mut self, part: Option<&str>) -> (Vec<String>, Vec<Effect>) {
        let mut notes = Vec::new();
        if let Some(text) = part {
            let file_of = |name: &str| {
                self.tuning_files
                    .iter()
                    .position(|(n, _)| n == name)
                    .and_then(|i| u8::try_from(i).ok())
            };
            match Tunings::parse(text, file_of) {
                Ok((t, said)) => {
                    notes.extend(said);
                    let gone: Vec<String> = t
                        .iter_own()
                        .filter(|&(id, _)| self.instrument_list.kind(id).is_none())
                        .map(|(id, _)| id.to_string())
                        .collect();
                    if !gone.is_empty() {
                        notes.push(format!(
                            "tunings: left out the tuning of instrument {}, not in the project's list",
                            gone.join(", ")
                        ));
                    }
                    // The song's is the synth part's, read before.
                    let song = self.tunings.song();
                    self.tunings = t;
                    self.tunings.set_song(song);
                    self.tunings.follow(&self.instrument_list);
                }
                Err(e) => notes.push(format!("tunings: its tunings can't be read ({e})")),
            }
        }
        if self.tunings.owned() > 0 {
            let who: Vec<String> = self
                .tunings
                .iter_own()
                .filter_map(|(id, p)| {
                    let kind = self.instrument_list.kind(id)?;
                    Some(format!("{} {}", instrument_words(kind), self.pick_label(p)))
                })
                .collect();
            notes.push(format!(
                "tunings: the song plays {}; with their own: {}",
                self.pick_label(self.tunings.song()),
                who.join(", ")
            ));
        }
        self.shared_tuning_follows();
        (notes, vec![Effect::SynthPads(self.synth_pads)])
    }
}

/// An instrument as the log names it: "the shared synth", "Track 3's
/// own instrument", "free place 2's synth", "Kit 4".
fn instrument_words(kind: Kind) -> String {
    match kind {
        Kind::SharedSynth => "the shared synth".to_string(),
        Kind::Own { track } => format!("Track {}'s own instrument", track + 1),
        Kind::Free { place } => format!("free place {}'s synth", place + 1),
        Kind::Kit { kit } => format!("Kit {}", kit + 1),
    }
}
