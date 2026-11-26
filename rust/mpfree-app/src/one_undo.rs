//! The one history (audio tracks phase 4d.1): UNDO takes back the newest
//! step anywhere, in the takes (the
//! time engine), a loop or the audio tracks, and SHIFT + UNDO brings back
//! the step taken back last.
//!
//! Each engine keeps its own steps as before, each by its transaction
//! ([`TransactionId`]): the takes ([`TimeHost::take_steps`]), each loop's
//! history (`Looper::loop_steps`: a Clear kept for Undo is a step of its
//! own) and the audio tracks' own steps (`AudioTracks::own_steps`: a song
//! edit's are its take's, and go with it). [`OneUndo`] follows them once a
//! period, in the audio loop beside the engines (main.rs), and lists every
//! step in the order it was made:
//!
//! - a transaction new on an engine's done side is a new step on top, and
//!   the steps the one history could redo go (the engines are asked to
//!   forget theirs, below);
//! - one that moved to its engine's redo side was undone, by the one UNDO
//!   or by the engine's own; one back on the done side was redone;
//! - one gone from both sides is hidden where it is (a loop's steps while
//!   it is cleared, an undo nothing can redo, a project opened) and shows
//!   again if it comes back; with every engine empty the history starts
//!   again;
//! - an engine that let go of steps from its oldest end (full) retires
//!   them and every older step in every engine at once: UNDO stops there
//!   and says older steps were let go.
//!
//! What the one history can no longer reach, the engines are asked to
//! forget ([`OneUndo::tend`], phase 4d.3): with nothing left to redo, the
//! steps each engine's own Redo could bring back (a new step took their
//! place), and the steps made at or before the newest retired, in every
//! engine (`RetireThrough`), so their memory goes back. Asked a period
//! after the step that made them unreachable, and again while an engine
//! still holds them (a busy loop passes the ask over).
//!
//! A loop's step whose layer left memory (a synced layer only UNDO needed,
//! phase 4d.4) waits while its layer is brought back from its file: the
//! audio tracks' worker reads it, and it goes in a piece a period
//! ([`Pending`]); the step is taken back once it is all in, if it is still
//! the next. Another UNDO meanwhile says it is still bringing it back.
//!
//! UNDO sends the newest step back to its own engine ([`press`]): the time
//! engine's UNDO (a song edit's region steps go with it, as
//! `act_with_audio` takes them), the loop's Undo (`LoopHost::undo_loop`,
//! the Looper tab's pick left as it is) or the audio tracks' Undo. A
//! refusal changes nothing: a step moves only when its engine's does.
//!
//! The design page's contract ("Transactions: one UNDO") keeps the one
//! history in the time engine's control half, and commits a step that
//! touches several engines in one message: that is 4d.2. Until then it
//! lives here, beside the looper and the audio tracks.

use std::collections::{HashMap, HashSet};

use mpfree_audiotracks::OwnStep;
use mpfree_looper::{StepTxn, MAX_LOOPS};
use mpfree_time_ui::view::AudioAction;
use mpfree_time_ui::TimeAction;
use mpfree_transport::TransactionId;

use crate::audiotracks::TrackHost;
use crate::looping::LoopHost;
use crate::time::TimeHost;
use crate::{app, mixing, Out};

/// The engine part a step is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Part {
    /// The takes: notes, clips, the song, rows and scenes.
    Time,
    /// The audio tracks' own steps (regions, takes on their lanes).
    Audio,
    /// Loop `n`'s history (its slot, 0 to 7).
    Loop(usize),
    /// Make song (phase 4d.2): the free loops made song loops, the song
    /// taking their tempo, one step across the loops and the song
    /// (`LoopHost::song_steps`).
    Song,
}

/// The parts followed, in the order [`OneUndo`] gathers them.
const PARTS: usize = 3 + MAX_LOOPS;

fn part_at(i: usize) -> Part {
    match i {
        0 => Part::Time,
        1 => Part::Audio,
        n if n < 2 + MAX_LOOPS => Part::Loop(n - 2),
        _ => Part::Song,
    }
}

/// Steps the one history keeps at most; past that the oldest retire.
pub const MOST: usize = 512;

/// Periods before an engine is asked again to forget what it still holds
/// ([`OneUndo::tend`]).
const ASK_AGAIN: u16 = 32;

/// One step: its transaction, its part, and its words for the screen
/// ("Overdub · Layer 2 · Vocal / 1").
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub txn: TransactionId,
    pub part: Part,
    pub words: String,
    /// Gone from its engine for now ([`OneUndo`]'s notes): UNDO passes
    /// over it.
    hidden: bool,
}

/// One part's steps as its engine holds them now, each by its
/// transaction: those Undo takes back, oldest first, those Redo brings
/// back, the next first, and the newest it let go of from its oldest end.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Seen {
    pub done: Vec<TransactionId>,
    pub redo: Vec<TransactionId>,
    pub let_go: TransactionId,
}

/// What UNDO and SHIFT + UNDO would do now, for the screen.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UndoNext {
    pub undo: Option<String>,
    pub redo: Option<String>,
    /// Older steps were let go: UNDO says so when nothing is left.
    pub let_go: bool,
}

/// The steps in the order they were made, and those the one UNDO took
/// back ([`OneUndo`]'s module notes).
#[derive(Debug, Default)]
struct Ledger {
    done: Vec<Entry>,
    /// The next REDO first.
    redo: Vec<Entry>,
    /// The newest transaction retired: steps at or under it that the
    /// engines still hold are older than the one history.
    floor: TransactionId,
    let_go: bool,
    /// Each part's newest step let go, as last seen.
    seen_let_go: [TransactionId; PARTS],
}

/// Where a transaction is now: its part, and whether on the done side.
type Where = HashMap<TransactionId, (Part, bool)>;

impl Ledger {
    /// Follow the engines' parts as they are now (`now`, in [`part_at`]'s
    /// order). `words` names a step seen for the first time.
    fn follow(&mut self, now: &[Seen], mut words: impl FnMut(Part, TransactionId) -> String) {
        // Every engine empty (a project opened or made new): the history
        // starts again.
        if now.iter().all(|s| s.done.is_empty() && s.redo.is_empty()) {
            self.done.clear();
            self.redo.clear();
            self.let_go = false;
            for (i, s) in now.iter().enumerate() {
                self.seen_let_go[i] = s.let_go;
            }
            return;
        }
        let mut at: Where = HashMap::new();
        for (i, s) in now.iter().enumerate() {
            let p = part_at(i);
            at.extend(s.done.iter().map(|&t| (t, (p, true))));
            at.extend(s.redo.iter().map(|&t| (t, (p, false))));
        }
        // Let go from an engine's oldest end: that step and every older
        // one, in every engine, retire.
        for (i, s) in now.iter().enumerate() {
            if s.let_go > self.seen_let_go[i] {
                self.seen_let_go[i] = s.let_go;
                if let Some(k) = self.done.iter().position(|e| e.txn == s.let_go) {
                    self.retire_through(k);
                }
            }
        }
        // Undone, hidden or back.
        let mut undone = Vec::new();
        let mut k = 0;
        while k < self.done.len() {
            match at.get(&self.done[k].txn) {
                Some(&(p, true)) => {
                    let e = &mut self.done[k];
                    e.part = p;
                    e.hidden = false;
                    k += 1;
                }
                Some(&(p, false)) => {
                    let mut e = self.done.remove(k);
                    e.part = p;
                    e.hidden = false;
                    undone.push(e);
                }
                None => {
                    self.done[k].hidden = true;
                    k += 1;
                }
            }
        }
        // Undone in one period: the oldest is redone first.
        self.redo.splice(0..0, undone);
        // Redone, or gone from the redo side.
        let mut k = 0;
        while k < self.redo.len() {
            match at.get(&self.redo[k].txn) {
                Some(&(p, true)) => {
                    let mut e = self.redo.remove(k);
                    e.part = p;
                    self.done.push(e);
                }
                Some(&(p, false)) => {
                    self.redo[k].part = p;
                    k += 1;
                }
                None => {
                    self.redo.remove(k);
                }
            }
        }
        // New steps, in the order they were made; a new step drops what
        // the one history could redo.
        let known: HashSet<TransactionId> =
            self.done.iter().chain(&self.redo).map(|e| e.txn).collect();
        let mut new: Vec<(TransactionId, Part)> = at
            .iter()
            .filter(|&(t, &(_, done))| done && *t > self.floor && !known.contains(t))
            .map(|(&t, &(p, _))| (t, p))
            .collect();
        new.sort_by_key(|&(t, _)| t);
        if !new.is_empty() {
            self.redo.clear();
        }
        for (txn, part) in new {
            self.done.push(Entry {
                txn,
                part,
                words: words(part, txn),
                hidden: false,
            });
        }
        if self.done.len() > MOST {
            self.retire_through(self.done.len() - MOST - 1);
        }
    }

    /// Step `k` and every older one retire.
    fn retire_through(&mut self, k: usize) {
        for e in self.done.drain(..=k) {
            self.floor = self.floor.max(e.txn);
        }
        self.let_go = true;
    }

    /// The step UNDO takes back next, or REDO brings back.
    fn next(&self, redo: bool) -> Option<&Entry> {
        if redo {
            self.redo.first()
        } else {
            self.done.iter().rev().find(|e| !e.hidden)
        }
    }

    fn view(&self) -> UndoNext {
        UndoNext {
            undo: self.next(false).map(|e| e.words.clone()),
            redo: self.next(true).map(|e| e.words.clone()),
            let_go: self.let_go,
        }
    }
}

/// The one history ([`OneUndo`]'s module notes), with room to read the
/// engines' parts each period without allocating once grown.
#[derive(Debug, Default)]
pub struct OneUndo {
    ledger: Ledger,
    /// The parts as gathered this period and the last.
    now: Vec<Seen>,
    last: Vec<Seen>,
    /// Each loop's steps with what they are, and the audio tracks' own.
    loops: Vec<Vec<StepTxn>>,
    audio_done: Vec<OwnStep>,
    audio_redo: Vec<OwnStep>,
    shown: UndoNext,
    /// Periods until each engine (the takes, the audio tracks, the
    /// looper) may be asked again to forget ([`OneUndo::tend`]).
    wait: [u16; 3],
    /// A step waiting for its layer to come back from its file.
    pending: Option<Pending>,
}

/// A loop's step waiting for a layer that left memory (phase 4d.4): the
/// step (its loop, its transaction and its words; None for the Looper
/// tab's history list, whose presses go on by themselves), UNDO or REDO,
/// the loop its layer comes back into, its recording, and its audio once
/// read, with the frames put back so far.
#[derive(Debug)]
struct Pending {
    step: Option<(usize, TransactionId, String)>,
    redo: bool,
    lp: usize,
    rec: u128,
    audio: Option<Vec<f32>>,
    at: usize,
}

/// Frames of a layer coming back put in each period.
const BACK_FRAMES: usize = 16_384;

/// What [`OneUndo::tend`] asks one engine to forget.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Forget {
    /// The steps its Redo could bring back.
    pub redo: bool,
    /// The steps its Undo could take back made at or before this
    /// transaction (none: nothing).
    pub through: TransactionId,
}

impl Forget {
    fn any(&self) -> bool {
        self.redo || self.through.is_some()
    }
}

impl OneUndo {
    /// Follow the engines once a period. Returns what UNDO and SHIFT +
    /// UNDO would do when that changed, for the screen.
    pub fn follow(
        &mut self,
        time: &TimeHost,
        tracks: &TrackHost,
        looper: &LoopHost,
    ) -> Option<UndoNext> {
        let mut now = std::mem::take(&mut self.now);
        now.resize_with(PARTS, Seen::default);
        self.loops.resize_with(MAX_LOOPS, Vec::new);
        {
            let s = &mut now[0];
            s.let_go = time.take_steps(&mut s.done, &mut s.redo);
        }
        {
            let s = &mut now[1];
            s.let_go = tracks
                .host
                .tracks
                .own_steps(&mut self.audio_done, &mut self.audio_redo);
            s.done.clear();
            s.done.extend(self.audio_done.iter().map(|o| o.txn));
            s.redo.clear();
            s.redo.extend(self.audio_redo.iter().map(|o| o.txn));
        }
        for n in 0..MAX_LOOPS {
            let s = &mut now[2 + n];
            let steps = &mut self.loops[n];
            s.let_go = looper.looper.loop_steps(n, steps, &mut s.redo);
            s.done.clear();
            s.done.extend(steps.iter().map(|x| x.txn));
        }
        {
            let s = &mut now[2 + MAX_LOOPS];
            looper.song_steps(&mut s.done, &mut s.redo);
        }
        if now == self.last {
            self.now = now;
            return None;
        }
        let OneUndo {
            ledger,
            loops,
            audio_done,
            ..
        } = self;
        ledger.follow(&now, |part, txn| match part {
            Part::Time => time.take_words(txn).unwrap_or_else(|| "take".to_string()),
            Part::Audio => {
                let o = audio_done.iter().find(|o| o.txn == txn);
                audio_words(o, tracks, time)
            }
            Part::Loop(n) => {
                let step = loops[n].iter().find(|x| x.txn == txn).and_then(|x| x.step);
                loop_words(n, step, looper, time)
            }
            Part::Song => looper.song_words(),
        });
        self.now = std::mem::replace(&mut self.last, now);
        let v = self.ledger.view();
        (v != self.shown).then(|| {
            self.shown = v.clone();
            v
        })
    }

    /// The step UNDO takes back next (`redo` false), or REDO brings back.
    pub fn next(&self, redo: bool) -> Option<&Entry> {
        self.ledger.next(redo)
    }

    /// What each engine (the takes, the audio tracks, the looper) holds
    /// that the one history can no longer reach, as last followed: with
    /// nothing to redo, its redo steps; its steps at or under the newest
    /// retired.
    pub fn to_forget(&self) -> [Forget; 3] {
        let mut f = [Forget::default(); 3];
        if self.last.len() < PARTS {
            return f;
        }
        let (nothing_to_redo, floor) = (self.ledger.redo.is_empty(), self.ledger.floor);
        for (i, s) in self.last.iter().enumerate() {
            let e = &mut f[i.min(2)];
            e.redo |= nothing_to_redo && !s.redo.is_empty();
            if floor.is_some() && s.done.first().is_some_and(|&t| t <= floor) {
                e.through = floor;
            }
        }
        f
    }

    /// Ask the engines to forget what the one history can no longer reach
    /// ([`OneUndo::to_forget`]; audio tracks phase 4d.3). Once a period,
    /// after [`OneUndo::follow`]; an engine still holding it is asked
    /// again [`ASK_AGAIN`] periods later. The looper's and the audio
    /// tracks' forget from their next period.
    pub fn tend(
        &mut self,
        out: &mut Out,
        time: &mut TimeHost,
        tracks: &mut TrackHost,
        looper: &mut LoopHost,
    ) -> Vec<app::Effect> {
        let fx = self.tend_back(out, tracks, looper);
        self.ask_to_forget(out, time, tracks, looper);
        fx
    }

    /// A layer coming back from its file (phase 4d.4): its audio once the
    /// worker has read it, then a piece a period, and once it is all in,
    /// the step that waited for it, if it is still the next; or, for the
    /// Looper tab's history list, whose presses then go on. Once a period
    /// (in [`OneUndo::tend`]).
    pub fn tend_back(
        &mut self,
        out: &mut Out,
        tracks: &mut TrackHost,
        looper: &mut LoopHost,
    ) -> Vec<app::Effect> {
        // The worker's answer: the audio waited for, or a read no step
        // waits for any more, which goes back to the worker.
        let reading = self
            .pending
            .as_ref()
            .filter(|p| p.audio.is_none())
            .map(|p| p.rec);
        if let Some((rec, got)) = tracks.layer_read_back() {
            match (got, reading == Some(rec)) {
                (Ok(audio), true) => {
                    if let Some(p) = self.pending.as_mut() {
                        p.audio = Some(audio);
                    }
                }
                (Err(why), true) => {
                    looper.looper.call_back_off();
                    let lp = self.pending.take().map_or(0, |p| p.lp);
                    return cant_come_back(out, looper, lp, &why);
                }
                (Ok(other), false) => tracks.drop_audio(other),
                (Err(_), false) => {}
            }
        }
        if self.pending.is_none() {
            // The Looper tab's history list waits for a layer away.
            let away = looper
                .waits_for_layer()
                .and_then(|(n, redo)| Some((redo, looper.looper.away_for(n, redo)?)));
            return match away {
                Some((redo, away)) => bring_back(self, out, tracks, looper, None, redo, away),
                None => Vec::new(),
            };
        }
        let Some(p) = self.pending.as_mut() else {
            return Vec::new();
        };
        if !looper.looper.bringing_back() {
            // Called off in the looper: the step was let go, or its loop
            // cleared and forgotten or loaded over. Nothing to take back.
            let p = self.pending.take().expect("pending above");
            if let Some(a) = p.audio {
                tracks.drop_audio(a);
            }
            out.note(&format!(
                "one undo: Loop {}'s layer is no longer needed",
                p.lp + 1
            ));
            return Vec::new();
        }
        let Some(audio) = p.audio.as_ref() else {
            return Vec::new();
        };
        let end = (p.at + 2 * BACK_FRAMES).min(audio.len());
        let whole = looper.looper.put_back(&audio[p.at..end]);
        p.at = end;
        if !whole && p.at < audio.len() {
            return Vec::new();
        }
        let p = self.pending.take().expect("pending above");
        if let Some(a) = p.audio {
            tracks.drop_audio(a);
        }
        if !whole {
            // The file was shorter than the layer: it stays away.
            looper.looper.call_back_off();
            return cant_come_back(out, looper, p.lp, "its file is short");
        }
        out.note(&format!("one undo: Loop {}'s layer is back", p.lp + 1));
        // The history list's presses go on by themselves.
        let Some((n, txn, said)) = p.step else {
            return Vec::new();
        };
        if self.next(p.redo).map(|e| e.txn) != Some(txn) {
            return Vec::new();
        }
        // A step that brings back more than one (a merge's sources): the
        // next one, then the step.
        if let Some(away) = looper.looper.away_for(n, p.redo) {
            let step = Some((n, txn, said));
            return bring_back(self, out, tracks, looper, step, p.redo, away);
        }
        out.note(&looper.undo_loop(n, p.redo));
        vec![app::Effect::Tell(said)]
    }

    fn ask_to_forget(
        &mut self,
        out: &mut Out,
        time: &mut TimeHost,
        tracks: &mut TrackHost,
        looper: &mut LoopHost,
    ) {
        let asks = self.to_forget();
        for (e, f) in asks.iter().enumerate() {
            if self.wait[e] > 0 {
                self.wait[e] -= 1;
                continue;
            }
            if !f.any() {
                continue;
            }
            self.wait[e] = ASK_AGAIN;
            for redo in [true, false] {
                if (redo && !f.redo) || (!redo && !f.through.is_some()) {
                    continue;
                }
                let what = if redo {
                    "what REDO could bring back".to_string()
                } else {
                    format!("steps up to transaction {}", f.through.0)
                };
                match e {
                    0 => {
                        if time.forget_takes(redo, f.through) {
                            out.note(&format!("one undo: the takes forget {what}"));
                        }
                    }
                    1 => match tracks.forget_steps(redo, f.through) {
                        Ok(()) => out.note(&format!("one undo: the audio tracks forget {what}")),
                        Err(why) => out.note(&format!("one undo: audio tracks: {why}")),
                    },
                    _ => {
                        // Make song's step, the loop host's own (4d.2).
                        if looper.forget_song(redo, f.through) {
                            out.note(&format!("one undo: Make song forgets {what}"));
                        }
                        let c = if redo {
                            mpfree_looper::Command::ForgetRedo
                        } else {
                            mpfree_looper::Command::RetireThrough(f.through)
                        };
                        if looper.looper.command(c) {
                            out.note(&format!("one undo: the loops forget {what}"));
                        } else {
                            out.note("one undo: the looper's queue is full");
                        }
                    }
                }
            }
        }
    }
}

/// An audio tracks step as UNDO names it: "region edit · Audio 1", or
/// "region edit · 2 audio tracks".
fn audio_words(o: Option<&OwnStep>, tracks: &TrackHost, time: &TimeHost) -> String {
    match o {
        Some(o) if o.tracks > 1 => format!("region edit · {} audio tracks", o.tracks),
        Some(o) => {
            let name = tracks
                .place(o.track)
                .and_then(|t| time.project().tracks.get(t))
                .map_or_else(
                    || format!("audio track {}", o.track + 1),
                    |t| t.name.clone(),
                );
            format!("region edit · {name}")
        }
        None => "region edit".to_string(),
    }
}

/// A loop's step as UNDO names it, with where the loop is: its cell's
/// track and row ("Overdub · Layer 2 · Vocal / 1") while a cell names it,
/// else the loop ("Overdub · Layer 2 · Loop 3").
fn loop_words(
    n: usize,
    step: Option<mpfree_looper::Step>,
    looper: &LoopHost,
    time: &TimeHost,
) -> String {
    let what = looper.step_words(n, step);
    let id = looper.loop_ids().get(n).copied().unwrap_or(0);
    let p = time.project();
    let cell = (id != 0)
        .then(|| {
            p.cell_clips()
                .find(|(_, _, c)| c.loop_id().is_some_and(|l| l.0 == id))
        })
        .flatten();
    match cell {
        Some((t, row, _)) => {
            let name = p.tracks.get(t).map_or("", |t| t.name.as_str());
            format!("{what} · {name} / {}", row + 1)
        }
        None => format!("{what} · Loop {}", n + 1),
    }
}

/// UNDO, or SHIFT + UNDO (`redo`), from the one history: the newest step
/// goes back to its own engine, or the step taken back last comes back,
/// and the screen says which ("Undid Overdub · Layer 2 · Vocal / 1"; the
/// time engine says its own). Returns the app's effects.
#[allow(clippy::too_many_arguments)]
pub fn press(
    one: &mut OneUndo,
    redo: bool,
    out: &mut Out,
    time: &mut TimeHost,
    tracks: &mut TrackHost,
    looper: &mut LoopHost,
    app: &mut app::App,
    mix: &mut mixing::MixHost,
) -> Vec<app::Effect> {
    if let Some(p) = &one.pending {
        let line = format!("Still bringing back Loop {}'s layer", p.lp + 1);
        out.note(&format!("one undo: {line}"));
        return vec![app::Effect::Tell(line)];
    }
    let Some(e) = one.next(redo) else {
        let line = match (redo, one.ledger.let_go) {
            (true, _) => "Nothing to redo",
            (false, true) => "Nothing more to undo: older steps were let go",
            (false, false) => "Nothing to undo",
        };
        out.note(&format!("one undo: {line}"));
        return vec![app::Effect::Tell(line.to_string())];
    };
    let did = if redo { "Redid" } else { "Undid" };
    let said = format!("{did} {}", e.words);
    match e.part {
        Part::Time => {
            let a = if redo {
                TimeAction::Redo
            } else {
                TimeAction::Undo
            };
            crate::time_action(out, time, tracks, looper, app, mix, a)
        }
        Part::Audio => {
            let a = if redo {
                AudioAction::Redo
            } else {
                AudioAction::Undo
            };
            match tracks.action(a, &time.info(), looper.line.is_open()) {
                Ok(line) => {
                    out.note(&line);
                    vec![app::Effect::Tell(said)]
                }
                Err(why) => {
                    out.note(&format!("audio tracks: {why}"));
                    vec![app::Effect::Tell(format!("Audio tracks: {why}"))]
                }
            }
        }
        Part::Loop(n) => {
            if let Some(away) = looper.looper.away_for(n, redo) {
                let step = Some((n, e.txn, said));
                return bring_back(one, out, tracks, looper, step, redo, away);
            }
            out.note(&looper.undo_loop(n, redo));
            vec![app::Effect::Tell(said)]
        }
        // Taken back (or made again) with the time engine in the next
        // period (`LoopHost::tend_song`), which says if it can't be.
        Part::Song => {
            out.note(&looper.song_undo(redo));
            vec![app::Effect::Tell(said)]
        }
    }
}

/// Start bringing back the layer loop step `step` (its loop, transaction
/// and words; None for the Looper tab's history list) needs for its UNDO
/// or REDO (`redo`): loop `lp`'s slot `slot`, numbered `born` (phase
/// 4d.4). Its memory is taken again and the audio tracks' worker reads
/// its file; [`OneUndo::tend`] puts it in and then takes the step back.
/// Says why not, changing nothing.
fn bring_back(
    one: &mut OneUndo,
    out: &mut Out,
    tracks: &mut TrackHost,
    looper: &mut LoopHost,
    step: Option<(usize, TransactionId, String)>,
    redo: bool,
    (lp, slot, born): (usize, usize, u64),
) -> Vec<app::Effect> {
    let rec = looper.layer_rec(lp, born);
    let begun = match rec {
        0 => Err("it has no file"),
        _ => looper.looper.begin_back(lp, slot).map(|_| ()),
    };
    let begun = begun.and_then(|()| match tracks.read_layer(rec) {
        true => Ok(()),
        false => {
            looper.looper.call_back_off();
            Err("its file can't be read now")
        }
    });
    if let Err(why) = begun {
        return cant_come_back(out, looper, lp, why);
    }
    one.pending = Some(Pending {
        step,
        redo,
        lp,
        rec,
        audio: None,
        at: 0,
    });
    let line = format!("Bringing back Loop {}'s layer", lp + 1);
    out.note(&format!("one undo: {line}"));
    vec![app::Effect::Tell(line)]
}

/// Loop `lp`'s layer can't come back, `why` (phase 4d.4): said, and the
/// Looper tab's history list stops where it is rather than ask again.
fn cant_come_back(out: &mut Out, looper: &mut LoopHost, lp: usize, why: &str) -> Vec<app::Effect> {
    looper.stop_steps();
    let line = format!("Loop {}'s layer can't come back: {why}", lp + 1);
    out.note(&format!("one undo: {line}"));
    vec![app::Effect::Tell(line)]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The parts as the engines would publish them: the takes, the audio
    /// tracks and the loops (none but those given).
    fn parts(
        time: (&[u64], &[u64]),
        audio: (&[u64], &[u64]),
        loops: &[(usize, &[u64], &[u64])],
    ) -> Vec<Seen> {
        let ids = |v: &[u64]| v.iter().map(|&t| TransactionId(t)).collect::<Vec<_>>();
        let mut now = vec![Seen::default(); PARTS];
        now[0].done = ids(time.0);
        now[0].redo = ids(time.1);
        now[1].done = ids(audio.0);
        now[1].redo = ids(audio.1);
        for &(n, d, r) in loops {
            now[2 + n].done = ids(d);
            now[2 + n].redo = ids(r);
        }
        now
    }

    fn named(p: Part, t: TransactionId) -> String {
        format!("{p:?} {}", t.0)
    }

    fn next(l: &Ledger, redo: bool) -> Option<(Part, u64)> {
        l.next(redo).map(|e| (e.part, e.txn.0))
    }

    /// The roadmap's order (a take, an overdub, a region edit, a song edit
    /// over regions), each engine undoing as the one history asks: UNDO
    /// takes them back newest first, wherever each is, and REDO brings
    /// them back in order; a step after UNDOs leaves nothing to redo.
    #[test]
    fn undo_takes_back_the_newest_step_anywhere_and_redo_brings_them_back_in_order() {
        let mut l = Ledger::default();
        l.follow(&parts((&[1], &[]), (&[], &[]), &[]), named);
        l.follow(&parts((&[1], &[]), (&[], &[]), &[(2, &[2], &[])]), named);
        l.follow(&parts((&[1], &[]), (&[3], &[]), &[(2, &[2], &[])]), named);
        // The song edit is a take; its tied region step is its own.
        l.follow(
            &parts((&[1, 4], &[]), (&[3], &[]), &[(2, &[2], &[])]),
            named,
        );
        assert_eq!(l.done.len(), 4);
        assert_eq!(l.done[1].words, "Loop(2) 2");
        let mut stages = [
            parts((&[1], &[4]), (&[3], &[]), &[(2, &[2], &[])]),
            parts((&[1], &[4]), (&[], &[3]), &[(2, &[2], &[])]),
            parts((&[1], &[4]), (&[], &[3]), &[(2, &[], &[2])]),
            parts((&[], &[1, 4]), (&[], &[3]), &[(2, &[], &[2])]),
        ];
        let undone = [
            (Part::Time, 4),
            (Part::Audio, 3),
            (Part::Loop(2), 2),
            (Part::Time, 1),
        ];
        for (s, want) in stages.iter().zip(undone) {
            assert_eq!(next(&l, false), Some(want));
            l.follow(s, named);
        }
        assert_eq!(next(&l, false), None);
        assert!(!l.let_go);
        // REDO: the take first, then the overdub, the region edit and the
        // song edit.
        stages.reverse();
        let mut redone = undone;
        redone.reverse();
        let back = parts((&[1, 4], &[]), (&[3], &[]), &[(2, &[2], &[])]);
        for (k, want) in redone.into_iter().enumerate() {
            assert_eq!(next(&l, true), Some(want));
            let s = stages.get(k + 1).unwrap_or(&back);
            l.follow(s, named);
        }
        assert_eq!(next(&l, true), None);
        assert_eq!(next(&l, false), Some((Part::Time, 4)));
        // Two UNDOs, then a new overdub: nothing left to redo.
        l.follow(&parts((&[1], &[4]), (&[3], &[]), &[(2, &[2], &[])]), named);
        l.follow(&parts((&[1], &[4]), (&[], &[3]), &[(2, &[2], &[])]), named);
        assert_eq!(next(&l, true), Some((Part::Audio, 3)));
        l.follow(
            &parts((&[1], &[4]), (&[], &[3]), &[(2, &[2, 5], &[])]),
            named,
        );
        assert_eq!(next(&l, true), None);
        assert_eq!(next(&l, false), Some((Part::Loop(2), 5)));
    }

    /// What the engines are asked to forget (phase 4d.3): nothing while
    /// the one history can still redo; once a new step leaves nothing to
    /// redo, every engine's redo steps; once a step is let go, every step
    /// at or under it, in every engine, the loops as one looper.
    #[test]
    fn the_engines_forget_what_the_one_history_can_no_longer_reach() {
        let follow = |one: &mut OneUndo, now: Vec<Seen>| {
            one.ledger.follow(&now, named);
            one.last = now;
        };
        let mut one = OneUndo::default();
        assert_eq!(one.to_forget(), [Forget::default(); 3], "nothing seen yet");
        follow(&mut one, parts((&[1], &[]), (&[2], &[]), &[(3, &[3], &[])]));
        assert_eq!(one.to_forget(), [Forget::default(); 3]);
        // Two UNDOs: redo can still bring them back, so nothing goes.
        follow(&mut one, parts((&[1], &[]), (&[], &[2]), &[(3, &[], &[3])]));
        assert_eq!(one.to_forget(), [Forget::default(); 3]);
        // A new take: nothing left to redo, so the audio tracks' and the
        // loop's redo steps go; the takes have none.
        follow(
            &mut one,
            parts((&[1, 4], &[]), (&[], &[2]), &[(3, &[], &[3])]),
        );
        let redo = Forget {
            redo: true,
            through: TransactionId::NONE,
        };
        assert_eq!(one.to_forget(), [Forget::default(), redo, redo]);
        follow(&mut one, parts((&[1, 4], &[]), (&[], &[]), &[]));
        assert_eq!(one.to_forget(), [Forget::default(); 3]);
        // Steps 5 and 6 in a loop and the audio tracks; the takes let go
        // of take 4 (full): 1 and 5 are older steps of other engines.
        follow(
            &mut one,
            parts((&[1, 4], &[]), (&[6], &[]), &[(0, &[5], &[])]),
        );
        let mut let_go = parts((&[7], &[]), (&[6], &[]), &[(0, &[5], &[])]);
        let_go[0].let_go = TransactionId(4);
        follow(&mut one, let_go);
        assert_eq!(one.ledger.floor, TransactionId(4));
        assert_eq!(
            one.to_forget(),
            [Forget::default(); 3],
            "none at or under 4"
        );
        let mut let_go = parts((&[7], &[]), (&[], &[]), &[(0, &[5], &[])]);
        let_go[1].let_go = TransactionId(6);
        follow(&mut one, let_go);
        let through = |t| Forget {
            redo: false,
            through: TransactionId(t),
        };
        assert_eq!(
            one.to_forget(),
            [Forget::default(), Forget::default(), through(6)],
            "the loop's 5 is older than the 6 let go"
        );
    }

    /// An engine that refuses changes nothing: the step stays the next to
    /// undo. One undone by its own engine's UNDO (the Looper tab's own
    /// list, the audio row's Undo) moves as if the one UNDO had asked.
    #[test]
    fn a_refusal_changes_nothing_and_an_engines_own_undo_is_followed() {
        let mut l = Ledger::default();
        let made = parts((&[1], &[]), (&[], &[]), &[(0, &[2], &[])]);
        l.follow(&made, named);
        l.follow(&made, named);
        assert_eq!(next(&l, false), Some((Part::Loop(0), 2)));
        // The take undone in the time engine, under the overdub.
        l.follow(&parts((&[], &[1]), (&[], &[]), &[(0, &[2], &[])]), named);
        assert_eq!(next(&l, false), Some((Part::Loop(0), 2)));
        assert_eq!(next(&l, true), Some((Part::Time, 1)));
    }

    /// A loop cleared: its steps stay where they were, its Clear on top.
    /// UNDO brings the loop back with them, and the Clear, which nothing
    /// redoes, is passed over; the take made between stays in its place.
    /// A project opened (every engine empty) starts again.
    #[test]
    fn a_clear_undone_leaves_the_loops_steps_in_their_places() {
        let mut l = Ledger::default();
        l.follow(&parts((&[], &[]), (&[], &[]), &[(1, &[1, 2], &[])]), named);
        l.follow(&parts((&[3], &[]), (&[], &[]), &[(1, &[1, 2], &[])]), named);
        // Clear: the kept loop's steps, then its Clear.
        l.follow(
            &parts((&[3], &[]), (&[], &[]), &[(1, &[1, 2, 4], &[])]),
            named,
        );
        assert_eq!(next(&l, false), Some((Part::Loop(1), 4)));
        // Cleared again by a new first pass in its slot: all hidden.
        l.follow(&parts((&[3], &[]), (&[], &[]), &[(1, &[], &[])]), named);
        assert_eq!(next(&l, false), Some((Part::Time, 3)));
        // UNDO of the new pass gives the slot back to the kept loop.
        l.follow(
            &parts((&[3], &[]), (&[], &[]), &[(1, &[1, 2, 4], &[])]),
            named,
        );
        assert_eq!(next(&l, false), Some((Part::Loop(1), 4)));
        // The Clear undone: back as it was, the take next to undo.
        l.follow(&parts((&[3], &[]), (&[], &[]), &[(1, &[1, 2], &[])]), named);
        assert_eq!(next(&l, false), Some((Part::Time, 3)));
        assert_eq!(next(&l, true), None);
        l.follow(&parts((&[], &[3]), (&[], &[]), &[(1, &[1, 2], &[])]), named);
        assert_eq!(next(&l, false), Some((Part::Loop(1), 2)));
        // A project opened.
        l.follow(&parts((&[], &[]), (&[], &[]), &[]), named);
        assert!(l.done.is_empty() && l.redo.is_empty());
    }

    /// The audio tracks let go of their oldest step: it and every older
    /// step, the takes' and the loops' too, retire. UNDO stops there and
    /// says so; an older take the time engine still holds isn't taken
    /// back, nor taken for a new step.
    #[test]
    fn a_step_let_go_retires_it_and_every_older_step_in_every_engine() {
        let mut l = Ledger::default();
        l.follow(&parts((&[1], &[]), (&[], &[]), &[]), named);
        l.follow(&parts((&[1], &[]), (&[], &[]), &[(0, &[2], &[])]), named);
        l.follow(&parts((&[1], &[]), (&[3], &[]), &[(0, &[2], &[])]), named);
        l.follow(
            &parts((&[1, 4], &[]), (&[3], &[]), &[(0, &[2], &[])]),
            named,
        );
        l.follow(
            &parts((&[1, 4], &[]), (&[3, 5], &[]), &[(0, &[2], &[])]),
            named,
        );
        // Full: the audio tracks' step 3 goes as 6 comes.
        let mut now = parts((&[1, 4], &[]), (&[5, 6], &[]), &[(0, &[2], &[])]);
        now[1].let_go = TransactionId(3);
        l.follow(&now, named);
        let left: Vec<u64> = l.done.iter().map(|e| e.txn.0).collect();
        assert_eq!(left, [4, 5, 6]);
        assert!(l.let_go && l.view().let_go);
        // The same again changes nothing.
        l.follow(&now, named);
        assert_eq!(l.done.len(), 3);
        // UNDO three times, and nothing is left: the take and the overdub
        // older than the step let go stay.
        let mut s = now.clone();
        s[1].done = vec![TransactionId(5)];
        s[1].redo = vec![TransactionId(6)];
        l.follow(&s, named);
        s[1].done.clear();
        s[1].redo = vec![TransactionId(5), TransactionId(6)];
        l.follow(&s, named);
        s[0].done = vec![TransactionId(1)];
        s[0].redo = vec![TransactionId(4)];
        l.follow(&s, named);
        assert_eq!(next(&l, false), None);
        assert_eq!(l.view().undo, None);
        assert!(l.view().let_go);
    }

    /// A step gone from the middle of its engine (an audio step tied to a
    /// song edit, which its take now takes back) is hidden, nothing
    /// retires; one more than [`MOST`] steps retires the oldest.
    #[test]
    fn a_step_gone_from_the_middle_hides_and_the_oldest_retire_past_the_most() {
        let mut l = Ledger::default();
        l.follow(&parts((&[], &[]), (&[1, 2], &[]), &[]), named);
        l.follow(&parts((&[3], &[]), (&[2], &[]), &[]), named);
        assert!(!l.let_go);
        assert_eq!(next(&l, false), Some((Part::Time, 3)));
        assert_eq!(l.done.iter().filter(|e| e.hidden).count(), 1);
        let many: Vec<u64> = (10..10 + MOST as u64 + 1).collect();
        l.follow(&parts((&many, &[]), (&[2], &[]), &[]), named);
        assert_eq!(l.done.len(), MOST);
        assert!(l.let_go);
    }
}
