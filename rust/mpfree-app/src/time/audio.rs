//! The time engine's audio half: the transport, the player and the
//! recorder, the play view, the click, and what goes out each period (the
//! pads' notes, the locks and lanes for the mod engine, the pad hits for
//! the Play page). It runs once a period on the audio thread, allocates
//! and frees nothing, and hears from the control half only through its
//! queue (see `link`).
//!
//! It makes each recorded change to the view it plays in place, numbers
//! it and keeps it in `Recent`, then sends it on, so the control half
//! makes it to the project in the same order. A view the control half
//! compiled before the newest changes gets them before it plays
//! (`Recent::catch_up`); one that cannot goes back with a request to
//! compile again, and the view that holds everything plays on.

use mpfree_notefx::{Chain, Slot, TrackFx, MAX_FX};
use mpfree_play::{
    is_gear_control, Ahead, AheadCounts, Event, Out, PlayView, Player, Rec, Recent, Recorder,
    TrackArps,
};
use mpfree_play::{MOVE_POINTS, TOUCH_POINTS};
use mpfree_seq::mpfree_moves::{Input, Kind};
use mpfree_seq::{LaunchGrid, TouchPoint, MAX_TRACKS};
use mpfree_time_ui::view::TransportInfo;
use mpfree_transport::time::{TempoMap, Tick};
use mpfree_transport::{Block, Command, Mark, State, Transport};

use super::follow::{Follow, FollowCounts, Follower};
use super::link::{
    Counts, CueCounts, InPlace, PeriodEnd, Rx, StripCounts, ToAudio, ToControl, Tracks, Tx,
    MAX_IN_PLACE, POINTS_CHUNK, TOUCH_CHUNK,
};
use super::pad_wait::{DuePads, PadFrom, PadWait, PadWaits};
use super::strips::{Ev, Sent, StripLocks};
use super::timing::{next_timeline, Allowed, Answer, Answers, Pending, TimingChange};
use super::{
    Click, Heard, HitInfo, LaneOut, LockOut, Pads, Table, HEARD_NOTES, MAX_HITS, MAX_LANES,
    MAX_LOCKS,
};

/// Views kept here while the queue back to the control half is full,
/// until they can go back to be freed there.
const RETURNS: usize = 8;
/// Why a start on a note was refused when the transport's queue is full.
const QUEUE_FULL: &str = "the transport's queue is full";
/// Why a timing change was refused when the commands due in its period
/// would leave some of its own for the next one.
const PERIOD_FULL: &str = "too many commands land in its period";

/// Why a transport command moves the song's timeline, if it does: a
/// timing change made before it is cancelled for that reason.
fn moves_time(cmd: Command) -> Option<&'static str> {
    match cmd {
        Command::Play | Command::PlayStart => Some("the song started"),
        Command::Stop | Command::StopAndRewind => Some("the song stopped"),
        Command::Record => Some("recording started or stopped"),
        Command::Locate(_) => Some("the song moved"),
        Command::SetTempo(_) | Command::TempoNow(_) => Some("the tempo changed"),
        Command::ToggleOverdub
        | Command::SetSwing(_)
        | Command::SetLoop(_)
        | Command::SetCountIn(_) => None,
    }
}

/// Timing changes committed, cancelled and refused so far.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TimingCounts {
    pub committed: u64,
    pub cancelled: u64,
    pub refused: u64,
}

/// Notes started that one period keeps for the looper
/// ([`TimeAudio::take_notes`]).
pub const NOTES: usize = 64;

/// MIDI effect settings the mod engine can hold at once on one track.
pub const NOTE_MODS: usize = 16;

/// A value the mod engine holds on one MIDI effect setting: the effect's
/// id on the track (0: none), the setting and its value.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct NoteMod {
    id: u16,
    k: u8,
    value: i32,
}

pub struct TimeAudio {
    // Boxed: the fixed tables are large.
    transport: Box<Transport>,
    player: Box<Player>,
    recorder: Box<Recorder>,
    /// The other armed tracks' recorders, notes only
    /// ([`Recorder::also`]): recorder `k` records the view's `k`th other
    /// armed track ([`PlayView::also_track`]), up to [`super::MAX_ALSO`].
    also: Vec<Recorder>,
    /// How many of `also` recorded last period, so one whose track let go
    /// is run once more to drop what it held.
    also_live: usize,
    /// Routed keys for another armed track past [`super::MAX_ALSO`]: not
    /// recorded.
    also_dropped: u64,
    /// Notes a take couldn't record in its clip's kind (step 5c): pads as
    /// pressed into a clip of notes that came without their notes, or
    /// routed keys for another armed track playing a clip of notes.
    /// Counted with the notes dropped.
    kind_refused: u64,
    /// The notes each pad started in a take of notes, for its pressure
    /// ([`TimeAudio::pad_pressure`]): (notes, how many).
    pad_notes: [([u8; HEARD_NOTES], u8); 128],
    /// Pressure from routed keys on another armed track while the
    /// transport records: not recorded (that recorder keeps notes only),
    /// so counted with the pressure values lost ([`Counts::touch_dropped`]).
    also_pressure: u64,
    block: Box<Block>,
    /// The period's events, let in by its room so no ending is ever lost
    /// (refactor A2): the player's own, or what the arps pass on.
    pub(super) sink: Box<Table>,
    pub(super) view: Box<PlayView>,
    /// The last recorded changes, for a view that comes in late.
    pub(super) recent: Box<Recent>,
    /// The tracks' settings, as the control half last sent them.
    tracks: Tracks,
    /// The tracks' MIDI effects, as the control half last sent them.
    note_fx: Box<[TrackFx; MAX_TRACKS]>,
    /// What the mod engine holds on the tracks' MIDI effect settings (a
    /// macro, an LFO, a step's lock), on top of `note_fx` until it lets
    /// go ([`TimeAudio::modulate_note_fx`]).
    note_mods: Box<[[NoteMod; NOTE_MODS]; MAX_TRACKS]>,
    note_mods_dropped: u64,
    /// `note_fx` changed since [`TimeAudio::take_note_fx_moved`].
    note_fx_moved: bool,
    /// Chains for the tracks' MIDI effects the control half sent, until
    /// [`TimeAudio::dress_arps`] hands them to the arps.
    chains_in: [Option<Box<Chain>>; MAX_TRACKS],
    /// Another device's clock, when the transport follows one, and its
    /// counts as last sent.
    follower: Follower,
    follow_sent: FollowCounts,
    /// The timing contract's counts as last sent (Arrange 2.5).
    cues_sent: CueCounts,
    click: Click,
    /// Click while counting in and recording.
    pub metronome: bool,
    /// [`TimeAudio::advance`] ran the transport for the coming period (and
    /// whether it could).
    advanced: Option<bool>,
    /// A start or jump after the first span of the last period: the next
    /// arp clock says it jumped.
    #[allow(dead_code)] // read by arp_clock only
    jump_pending: bool,
    /// Pad hits this period, for the screen.
    hits: [HitInfo; MAX_HITS],
    n_hits: usize,
    /// Notes started in the coming period, played live on any track (as
    /// the arps give them) or by a route, for the looper's smart loop end
    /// ([`TimeAudio::take_notes`]): the frame, the track, note and
    /// velocity. Ones past [`NOTES`] are lost, and the newest frame of
    /// one lost is kept, so the looper knows; ones nobody took are
    /// forgotten when the next period starts.
    notes: [(u64, u16, u8, u8); NOTES],
    n_notes: usize,
    notes_lost_to: Option<u64>,
    /// Each pad's pressure last sent to CAPTURE's history, None after a
    /// press: a value that did not change is not sent again.
    pressure_sent: [Option<u8>; 128],
    /// The clips' parameter locks and lanes' values this period, for the
    /// mixer's mod engine.
    locks: Vec<LockOut>,
    lanes: Vec<LaneOut>,
    locks_dropped: u64,
    /// Unlocks with no place in `locks`: a bug (it holds every one the
    /// period's table lets in), which must stay 0.
    locks_lost: u64,
    /// The arps' new notes left out of busy moments, and their endings
    /// lost, so far (`TrackArps::refused`, `TrackArps::lost`).
    arps_busy: u64,
    arps_lost: u64,
    /// The locks on "this track's strip", with where each went: the strip
    /// its track's instrument sounds on ([`super::strips`]).
    strips: Box<StripLocks>,
    /// Their counts as last sent to the control half.
    strips_sent: StripCounts,
    tx: Tx<ToControl>,
    rx: Rx<ToAudio>,
    /// Views waiting to go back.
    returns: [Option<Box<PlayView>>; RETURNS],
    lost: u64,
    behind: u64,
    kept: u64,
    refused: u64,
    /// Step edits the view could not take in place.
    missed: u64,
    /// A step edit missed and asked for a compile, which brings it: the
    /// next view taken ends it, so a run of misses asks once.
    asked: bool,
    /// The number of the view it plays ([`ToAudio::View`]): a step edit
    /// made on another one isn't made to it.
    pub(super) view_n: u32,
    /// The parts of the step edit coming ([`ToAudio::EditPart`]), kept
    /// aside until it is whole: its id, its changes, how many came.
    staged: [InPlace; MAX_IN_PLACE],
    staged_id: u32,
    parts: usize,
    /// The tempo last asked for (a SetTempo, or [`TimeAudio::start_at`]'s)
    /// and the frame it is due on, until the period that applies it has
    /// run: [`TimeAudio::follow_tempo`] compares the app's tempo with it,
    /// so it doesn't ask again for what is on its way.
    tempo_asked: Option<(u64, f64)>,
    /// The song's tempo map (song mode with tempo points: the view's),
    /// queued and not in the transport yet, so
    /// [`TimeAudio::follow_tempo`] doesn't queue it again. A tempo queued
    /// after it replaces it, so that forgets it.
    map_asked: Option<TempoMap>,
    /// Starts on a note refused ([`TimeAudio::start_at`]).
    starts_refused: u64,
    /// The song's timeline (step 0C): a fresh revision whenever something
    /// moves time, and why the last one came. A timing change made against
    /// an older one is cancelled for that reason.
    timeline: u64,
    timeline_why: &'static str,
    /// The view's own timeline revision, as last seen
    /// ([`mpfree_play::Includes::timeline`]).
    view_timeline: u64,
    /// The timing change waiting for its frame, and the last answers.
    pending: Option<Pending>,
    answers: Answers,
    /// Timing changes committed, cancelled and refused so far.
    timing_counts: TimingCounts,
    /// The look-ahead for tracks on slow gear (sequencer S5).
    ahead: Ahead,
    /// Live pad hits waiting for the launch line (sampler R5.3b).
    waits: PadWaits,
}

impl TimeAudio {
    /// Playing `view` (the control half sends the first real one), with
    /// the tracks' settings `tracks`.
    pub fn new(
        transport: Box<Transport>,
        rate: u32,
        tracks: Tracks,
        tx: Tx<ToControl>,
        rx: Rx<ToAudio>,
    ) -> TimeAudio {
        let ahead = Ahead::new(&transport);
        TimeAudio {
            transport,
            player: Box::new(Player::new()),
            recorder: Box::new(Recorder::new()),
            also: (0..super::MAX_ALSO).map(Recorder::also).collect(),
            also_live: 0,
            also_dropped: 0,
            kind_refused: 0,
            pad_notes: [([0; HEARD_NOTES], 0); 128],
            also_pressure: 0,
            block: Box::new(Block::new()),
            sink: Box::new(Table::new()),
            view: Box::new(PlayView::empty()),
            recent: Box::new(Recent::new()),
            tracks,
            note_fx: Box::new([TrackFx::default(); MAX_TRACKS]),
            note_mods: Box::new([[NoteMod::default(); NOTE_MODS]; MAX_TRACKS]),
            note_mods_dropped: 0,
            note_fx_moved: false,
            chains_in: std::array::from_fn(|_| None),
            follower: Follower::new(rate),
            follow_sent: FollowCounts::default(),
            cues_sent: CueCounts::default(),
            click: Click::new(rate),
            metronome: false,
            advanced: None,
            jump_pending: false,
            hits: [HitInfo::default(); MAX_HITS],
            n_hits: 0,
            notes: [(0, 0, 0, 0); NOTES],
            n_notes: 0,
            notes_lost_to: None,
            pressure_sent: [None; 128],
            locks: Vec::with_capacity(MAX_LOCKS),
            lanes: Vec::with_capacity(MAX_LANES),
            locks_dropped: 0,
            locks_lost: 0,
            arps_busy: 0,
            arps_lost: 0,
            strips: Box::new(StripLocks::new()),
            strips_sent: StripCounts::default(),
            tx,
            rx,
            returns: std::array::from_fn(|_| None),
            lost: 0,
            behind: 0,
            kept: 0,
            refused: 0,
            missed: 0,
            asked: false,
            view_n: 0,
            staged: [InPlace::EraseNote(mpfree_seq::Note::new(0, 1, 0, 1)); MAX_IN_PLACE],
            staged_id: 0,
            parts: 0,
            tempo_asked: None,
            map_asked: None,
            starts_refused: 0,
            timeline: next_timeline(),
            timeline_why: "the song changed",
            view_timeline: 0,
            pending: None,
            answers: Answers::new(),
            timing_counts: TimingCounts::default(),
            ahead,
            waits: PadWaits::default(),
        }
    }

    /// Each track's ahead in frames (sequencer S5, `MidiOut::ahead`): how
    /// far before its frame its gear needs a note handed over, past the
    /// sound's lead. A track whose clips play through its arp plays none.
    pub fn set_ahead(&mut self, frames: &[u32]) {
        self.ahead.set(frames);
        for t in 0..self.tracks.n {
            if self.tracks.set[t].through_arp {
                self.ahead.clear(t);
            }
        }
    }

    /// What the look-ahead did, for the log.
    pub fn ahead_counts(&self) -> AheadCounts {
        self.ahead.counts()
    }

    /// Take what the control half sent: views, transport commands and the
    /// tracks' settings. At the start of each period, before the
    /// transport runs; the one-thread host also calls it after each call
    /// on the control half.
    pub fn receive(&mut self) {
        self.send_returns();
        while let Some(m) = self.rx.recv() {
            match m {
                ToAudio::View(v, n) => self.take_view(v, n),
                ToAudio::Command(cmd) => {
                    if !self.command(cmd) {
                        self.refused += 1;
                    }
                }
                ToAudio::Tracks { n, armed } => {
                    self.tracks.n = n.min(MAX_TRACKS);
                    self.tracks.armed = armed;
                }
                ToAudio::Track(t, s) => {
                    if let Some(set) = self.tracks.set.get_mut(t) {
                        *set = s;
                    }
                }
                ToAudio::NoteFx {
                    track,
                    place,
                    slot,
                    id,
                } => self.note_place(usize::from(track), usize::from(place), slot, id),
                ToAudio::NoteArp { track, at } => {
                    if let Some(fx) = self.note_fx.get_mut(usize::from(track)) {
                        fx.arp_at = at;
                        self.note_fx_moved = true;
                    }
                }
                ToAudio::Chain(t, c) => match self.chains_in.get_mut(usize::from(t)) {
                    Some(slot @ None) => *slot = Some(c),
                    _ => self.free_chain(c),
                },
                ToAudio::EditPart { id, change } => {
                    if id != self.staged_id {
                        (self.staged_id, self.parts) = (id, 0);
                    }
                    if let Some(c) = self.staged.get_mut(self.parts) {
                        *c = change;
                    }
                    self.parts += 1;
                }
                ToAudio::EditEnd {
                    id,
                    track,
                    slot,
                    view,
                    n,
                } => self.edit(id, (usize::from(track), usize::from(slot)), view, n),
            }
        }
    }

    /// Step edit `id`, whole: its `n` changes, made on view `view`, to
    /// clip `slot` of `track` in this view in place where a compile would
    /// put them, all at once or none, without allocating (refactor A3).
    /// One with a part missing, made on another view, or with a change
    /// the view cannot take (no room left, or what it erases is not
    /// there) asks for a compile, which brings it and the edits after it:
    /// once, until a new view comes.
    fn edit(&mut self, id: u32, (track, slot): (usize, usize), view: u32, n: u8) {
        let n = usize::from(n);
        let whole = id == self.staged_id && self.parts == n && n <= MAX_IN_PLACE;
        self.parts = 0;
        let done =
            whole && view == self.view_n && self.view.edit_in_place(track, slot, &self.staged[..n]);
        if !done {
            self.missed += 1;
            if !std::mem::replace(&mut self.asked, true) {
                self.send(ToControl::Compile);
            }
        }
    }

    /// A new view: it plays from now on, with the recorded changes made
    /// since it was compiled. One that cannot catch up goes back.
    fn take_view(&mut self, mut v: Box<PlayView>, n: u32) {
        let after = v.includes().map_or(self.recent.last(), |i| i.recorded);
        match self.recent.catch_up(&mut v, after) {
            Ok(()) => {
                let old = std::mem::replace(&mut self.view, v);
                self.give_back(old);
                self.asked = false;
                self.view_n = n;
                // A song edit that moved time. The first view says only
                // where its timeline starts: the engine's own is new.
                let t = self.view.includes().map_or(0, |i| i.timeline);
                if t != 0 && t != self.view_timeline {
                    if self.view_timeline != 0 {
                        self.moved("the song was edited");
                    }
                    self.view_timeline = t;
                }
            }
            Err(_) => {
                self.behind += 1;
                self.send(ToControl::Compile);
                self.give_back(v);
            }
        }
    }

    /// A view to free on the control half. While the queue is full it
    /// waits here; past [`RETURNS`] of them, it is kept for good rather
    /// than freed on this thread (counted: it should never happen).
    fn give_back(&mut self, v: Box<PlayView>) {
        if let Err(ToControl::Done(v)) = self.tx.send(ToControl::Done(v)) {
            match self.returns.iter_mut().find(|r| r.is_none()) {
                Some(r) => *r = Some(v),
                None => {
                    self.kept += 1;
                    std::mem::forget(v);
                }
            }
        }
    }

    /// A chain to free on the control half; while the queue is full it is
    /// kept for good rather than freed on this thread (counted: it should
    /// never happen, the control half sends one chain a track).
    fn free_chain(&mut self, c: Box<Chain>) {
        if let Err(ToControl::Chain(c)) = self.tx.send(ToControl::Chain(c)) {
            self.kept += 1;
            std::mem::forget(c);
        }
    }

    fn send_returns(&mut self) {
        for r in &mut self.returns {
            if let Some(v) = r.take() {
                if let Err(ToControl::Done(v)) = self.tx.send(ToControl::Done(v)) {
                    *r = Some(v);
                    return;
                }
            }
        }
    }

    /// Send a message that holds nothing to free; a full queue loses it
    /// (counted).
    fn send(&mut self, m: ToControl) {
        if self.tx.send(m).is_err() {
            self.lost += 1;
        }
    }

    pub fn hits(&self) -> &[HitInfo] {
        &self.hits[..self.n_hits]
    }

    /// Every note started in the period just run, live on a track or by
    /// a route (as the pad that plays its note there), with its frame on
    /// the transport's clock, its track, note and velocity; then forgets
    /// them. The looper weighs them for smart loop end. Call it after the
    /// period runs. The newest frame of a note the period had no room
    /// for, if one was lost ([`NOTES`]).
    pub fn take_notes(&mut self, mut f: impl FnMut(u64, u16, u8, u8)) -> Option<u64> {
        for &(frame, track, key, velocity) in &self.notes[..self.n_notes] {
            f(frame, track, key, velocity);
        }
        self.n_notes = 0;
        self.notes_lost_to.take()
    }

    /// A note started on `frame`, for the looper.
    fn note_started(&mut self, frame: u64, track: u16, event: Event) {
        let Event::NoteOn { note, velocity } = event else {
            return;
        };
        if self.n_notes == NOTES {
            self.notes_lost_to = Some(self.notes_lost_to.map_or(frame, |f| f.max(frame)));
            return;
        }
        self.notes[self.n_notes] = (frame, track, note, velocity);
        self.n_notes += 1;
    }

    pub fn clear_hits(&mut self) {
        self.n_hits = 0;
    }

    pub fn locks(&self) -> &[LockOut] {
        &self.locks
    }

    pub fn lanes(&self) -> &[LaneOut] {
        &self.lanes
    }

    pub fn clear_locks(&mut self) {
        self.locks.clear();
        self.lanes.clear();
    }

    /// A clip's lock, move value or their end on its way to the mixer's
    /// mod engine (`MixHost::lock`, `MixHost::lane`). One on "this
    /// track's strip" goes to the strip the track's instrument sounds on
    /// ([`Pads::sounds_on`]), ends where it began, and waits while a track
    /// before it in line holds the same control ([`super::strips`]). A
    /// move sends the old strip's end first; the list has room for it
    /// ([`super::MAX_LOCKS`]), and past that a lock is dropped and an
    /// unlock lost, as in [`Self::lock_out`].
    fn send_to_mixer(
        &mut self,
        pads: &impl Pads,
        o: &Out,
        addr: mpfree_play::mpfree_param::ParamAddr,
        ev: Ev,
    ) {
        let (locks, lanes) = (&mut self.locks, &mut self.lanes);
        let (dropped, lost) = (&mut self.locks_dropped, &mut self.locks_lost);
        let set = &self.tracks.set;
        let rank = |t: u8| set.get(usize::from(t)).map_or(0, |s| s.id);
        let offset = o.offset;
        let mut send = |s: Sent| match s {
            Sent::Lock(a, v) => Self::lock_out(locks, (&mut *dropped, &mut *lost), offset, a, v),
            Sent::Lane(a, v) => {
                if lanes.len() < lanes.capacity() {
                    lanes.push((offset, a, v));
                } else {
                    *dropped += 1;
                }
            }
        };
        let sounds_on = pads.sounds_on(o.track);
        if !self
            .strips
            .take(o.track, addr, ev, sounds_on, rank, &mut send)
        {
            send(match ev {
                Ev::Lock(v) => Sent::Lock(addr, Some(v)),
                Ev::Lane(v) => Sent::Lane(addr, v),
                Ev::End => Sent::Lock(addr, None),
            });
        }
    }

    /// A lock (`Some`) or unlock (`None`) for the mixer's mod engine. The
    /// list has a place for every one the period's table lets in; past
    /// that a lock is dropped, an unlock lost (a bug, counted apart).
    fn lock_out(
        locks: &mut Vec<LockOut>,
        (dropped, lost): (&mut u64, &mut u64),
        offset: u32,
        addr: mpfree_play::mpfree_param::ParamAddr,
        value: Option<f32>,
    ) {
        if locks.len() < locks.capacity() {
            locks.push((offset, addr, value));
        } else if value.is_some() {
            *dropped += 1;
        } else {
            *lost += 1;
        }
    }

    /// A transport command, from the next period on. False if the
    /// transport's queue is full.
    pub fn command(&mut self, cmd: Command) -> bool {
        // Following another device's position, a loop waits for that to
        // stop (follow.rs).
        if let Command::SetLoop(range) = cmd {
            if self.follower.keep_loop(range) {
                return true;
            }
        }
        let at = self.transport.frame();
        if let Command::SetTempo(bpm) | Command::TempoNow(bpm) = cmd {
            self.tempo_asked = Some((at, bpm));
            self.map_asked = None;
        }
        if let Some(why) = moves_time(cmd) {
            self.moved(why);
        }
        self.transport.command(at, cmd)
    }

    /// Anchors the clips' chance rolls to the frame the transport next
    /// starts on (`mpfree_play::Player::anchor_dice`), or not.
    pub fn anchor_dice(&mut self, on: bool) {
        self.player.anchor_dice(on);
    }

    /// A transport command on frame `at` of the transport's clock, or at
    /// the next period's start if that is later. False if the queue is
    /// full.
    pub fn command_at(&mut self, at: u64, cmd: Command) -> bool {
        let at = at.max(self.transport.frame());
        if let Command::SetTempo(bpm) | Command::TempoNow(bpm) = cmd {
            self.tempo_asked = Some((at, bpm));
            self.map_asked = None;
        }
        if let Some(why) = moves_time(cmd) {
            self.moved(why);
        }
        self.transport.command(at, cmd)
    }

    /// The app's tempo `bpm`, unless the transport follows another
    /// device's clock that is arriving. While song mode plays the song's
    /// tempo points, the song's tempo map instead (the view's, which has
    /// the app's tempo before the first point): it takes over as a live
    /// tempo does, on the next 1/16 while rolling. Once song mode stops
    /// playing them, the app's tempo throughout again. A tempo or map
    /// asked for and not applied yet counts as the transport's.
    pub fn follow_tempo(&mut self, bpm: f64) {
        if self.follower.bpm().is_some() {
            // The follower's tempo replaces a map waiting.
            self.map_asked = None;
            return;
        }
        if let Some(&map) = self.view.tempo_map() {
            let have = self
                .map_asked
                .as_ref()
                .unwrap_or(self.transport.tempo_map());
            if *have != map {
                if self.transport.change_tempo_map(self.transport.frame(), map) {
                    self.map_asked = Some(map);
                    self.tempo_asked = None;
                    self.moved("the tempo changed");
                } else {
                    self.refused += 1;
                }
            }
            return;
        }
        let now = self.tempo_asked.map_or(self.transport.bpm(), |(_, b)| b);
        // A song's map still in the transport, or on its way, goes too.
        let flat = self.tempo_asked.is_some()
            || (self.map_asked.is_none() && self.transport.tempo_map().segments().len() == 1);
        if (now - bpm).abs() > 1e-9 || !flat {
            self.command(Command::SetTempo(bpm));
        }
    }

    /// Song mode plays the song's tempo points (the view carries their
    /// map).
    pub fn song_tempo(&self) -> bool {
        self.view.tempo_map().is_some()
    }

    /// Start the song on frame `at` of the transport's clock (the next
    /// period's first frame or later): the looper's Starts: When I play,
    /// whose first note starts the song. Only while it is stopped. On
    /// `at`, in this order: back to the start of the bar it stopped in
    /// (bar 1 after STOP twice), `into` ticks after it (a loop smart loop
    /// end's Auto closed on an end before `at`: the song starts as far in
    /// as the loop is), the tempo `bpm` if given (before PLAY, so
    /// the whole song takes it), then PLAY, with no count-in. With a take
    /// ready (`take`: adding, true, or replacing), it records from there
    /// too, as a followed clock's start does: the overdub mode first, then
    /// PLAY and REC on the one frame, a punch-in with no count-in.
    ///
    /// Refused, and counted ([`TimeAudio::starts_refused`]), while the song
    /// plays or counts in, while another device's clock leads (its tempo
    /// arriving, or PLAY waiting for its Start), or when the transport's
    /// queue hasn't room for every command: all go in or none, so a
    /// refused start leaves no Locate or tempo behind. Returns why.
    pub fn start_at(
        &mut self,
        at: u64,
        bpm: Option<f64>,
        take: Option<bool>,
        into: Tick,
    ) -> Result<(), &'static str> {
        // While song mode plays the song's tempo points, they lead.
        let bpm = bpm.filter(|_| !self.song_tempo());
        if let Some(why) = self.start_refusal(bpm, take, into) {
            self.starts_refused += 1;
            if why == QUEUE_FULL {
                self.refused += 1;
            }
            return Err(why);
        }
        // A timing change's stopped case, landing at once (there is
        // nothing to wait for: the song is stopped and nothing else moves
        // it before `at`). Back to the start of the bar it stopped in.
        let (_, start) = self.bar_start();
        let c = TimingChange {
            at: at.max(self.transport.frame()),
            tempo: bpm,
            position: Some(start + into.max(0)),
            play: true,
            take,
            made_against: self.timeline,
        };
        // start_refusal counted the room on the coming period's first
        // frame, so this fails only when other commands are queued between
        // it and `at`; counted if it does.
        self.commit(&c)
            .map(|_| ())
            .inspect_err(|_| self.starts_refused += 1)
    }

    /// Why [`TimeAudio::start_at`] would refuse a start with this tempo,
    /// take and ticks into the bar now, or None. Changes nothing and
    /// counts nothing.
    pub fn start_refusal(
        &self,
        bpm: Option<f64>,
        take: Option<bool>,
        into: Tick,
    ) -> Option<&'static str> {
        let bpm = bpm.filter(|_| !self.song_tempo());
        let why = match self.transport.state() {
            _ if self.follower.bpm().is_some() => Some("the song follows another device's clock"),
            _ if self.follower.waits() => Some("PLAY waits for the leader's Start"),
            State::CountIn => Some("the song is counting in"),
            State::Stopped => None,
            _ => Some("the song is playing already"),
        };
        why.or_else(|| {
            let (pos, start) = self.bar_start();
            let commands = usize::from(start + into.max(0) != pos)
                + usize::from(bpm.is_some())
                + usize::from(take.is_some_and(|add| add != self.transport.overdub()))
                + 1
                + usize::from(take.is_some());
            if self.transport.queue_room() < commands {
                Some(QUEUE_FULL)
            } else {
                (self.transport.room_on(self.transport.frame()) < commands).then_some(PERIOD_FULL)
            }
        })
    }

    /// Room left in the transport's queue (tests).
    #[cfg(test)]
    pub fn queue_room(&self) -> usize {
        self.transport.queue_room()
    }

    /// The song's timeline as it is now: a timing change is made against
    /// it, and anything that moves time (a locate, a tempo or tempo map,
    /// PLAY, STOP or REC, a song edit that moves time, a timing change
    /// landing) gives it a new revision. Another project's engine never
    /// has the same one ([`super::timing::next_timeline`]).
    pub fn timeline(&self) -> u64 {
        self.timeline
    }

    /// Something moved time: a new timeline, and a timing change waiting
    /// is cancelled for this reason.
    fn moved(&mut self, why: &'static str) {
        self.timeline = next_timeline();
        self.timeline_why = why;
        if let Some(p) = self.pending.take() {
            self.answers.put(p.ticket, Answer::Cancelled(why));
            self.timing_counts.cancelled += 1;
        }
    }

    /// Why timing change `c` can't be made now, or None.
    fn timing_refusal(&self, c: &TimingChange) -> Option<&'static str> {
        let rolling = self.transport.state().rolling();
        let why = match self.transport.state() {
            _ if self.follower.bpm().is_some() => Some("the song follows another device's clock"),
            _ if self.follower.waits() => Some("PLAY waits for the leader's Start"),
            State::CountIn => Some("the song is counting in"),
            State::Recording => Some("the song is recording"),
            _ => None,
        };
        why.or_else(|| {
            if c.at < self.transport.frame() {
                Some("its frame has played")
            } else if rolling && c.tempo.is_some() && self.song_tempo() {
                Some("the song's tempo points lead")
            } else if rolling && c.take.is_some() {
                Some("a take starts only with the song")
            } else {
                None
            }
        })
    }

    /// Whether timing change `c` may be offered now: Refused (and why)
    /// while the song records or counts in, while another device's clock
    /// leads, while the song's tempo points lead a tempo change, or when
    /// its frame has played; Ask when it changes the tempo and `audio`
    /// (recorded audio playing at its own speed, which the app knows:
    /// audio regions, other loops) would play out of step; else Safe.
    /// Changes nothing.
    pub fn allowed(&self, c: &TimingChange, audio: bool) -> Allowed {
        if let Some(why) = self.timing_refusal(c) {
            return Allowed::Refused(why);
        }
        let now = self.tempo_asked.map_or(self.transport.bpm(), |(_, b)| b);
        let new_tempo = c
            .tempo
            .filter(|_| !self.song_tempo())
            .is_some_and(|b| (b - now).abs() > 1e-9);
        if new_tempo && audio {
            Allowed::Ask("recorded audio would play at its old tempo")
        } else {
            Allowed::Safe
        }
    }

    /// Timing change `c`: it waits for its frame (a newer one cancels it,
    /// and so does anything that moves time first), then lands as one
    /// piece. Its ticket for [`TimeAudio::timing_answer`], or why it was
    /// refused (counted): see [`TimeAudio::allowed`], or the timeline it
    /// was made against has moved on (that reason). Real-time safe.
    pub fn timing_change(&mut self, c: TimingChange) -> Result<u32, &'static str> {
        if let Some(why) = self.timing_refusal(&c) {
            self.timing_counts.refused += 1;
            return Err(why);
        }
        if c.made_against != self.timeline {
            self.timing_counts.cancelled += 1;
            return Err(self.timeline_why);
        }
        if let Some(p) = self.pending.take() {
            self.answers
                .put(p.ticket, Answer::Cancelled("a newer change came"));
            self.timing_counts.cancelled += 1;
        }
        let ticket = self.answers.open();
        self.pending = Some(Pending { ticket, change: c });
        Ok(ticket)
    }

    /// Take back the timing change with `ticket` if it still waits (its
    /// maker's half went: the loop it was for changed). True if it did.
    pub fn cancel_timing(&mut self, ticket: u32, why: &'static str) -> bool {
        if self.pending.is_none_or(|p| p.ticket != ticket) {
            return false;
        }
        self.pending = None;
        self.answers.put(ticket, Answer::Cancelled(why));
        self.timing_counts.cancelled += 1;
        true
    }

    /// What became of the timing change with `ticket`, if it is one of
    /// the last few this engine had. None for one it never had: one made
    /// on the engine of a project since closed went with it, unapplied.
    pub fn timing_answer(&self, ticket: u32) -> Option<Answer> {
        self.answers.get(ticket)
    }

    /// Timing changes committed, cancelled and refused so far.
    #[allow(dead_code)] // the status lines, with smart loop end
    pub fn timing_counts(&self) -> TimingCounts {
        self.timing_counts
    }

    /// The change waiting, if its frame falls in the coming period of
    /// `frames`: it lands, or is refused, before the transport runs it.
    fn commit_due(&mut self, frames: u32) {
        let Some(p) = self.pending else {
            return;
        };
        if p.change.at >= self.transport.frame() + u64::from(frames) {
            return;
        }
        self.pending = None;
        let a = if p.change.made_against != self.timeline {
            self.timing_counts.cancelled += 1;
            Answer::Cancelled(self.timeline_why)
        } else {
            match self.commit(&p.change) {
                Ok(at) => {
                    self.timing_counts.committed += 1;
                    Answer::Committed(at)
                }
                Err(why) => {
                    self.timing_counts.refused += 1;
                    Answer::Refused(why)
                }
            }
        };
        self.answers.put(p.ticket, a);
    }

    /// Timing change `c` into the transport's queue on its frame, every
    /// command or none, and all applied in one period
    /// ([`Transport::room_on`]). Playing: the tempo on that frame
    /// (`Command::TempoNow`, no jump), then a jump to the position if one
    /// was given. Stopped: the position, the tempo (before PLAY, so the
    /// whole song takes it), the overdub mode for a take, PLAY, then REC
    /// for a take. The frame it landed on, or why not.
    fn commit(&mut self, c: &TimingChange) -> Result<u64, &'static str> {
        if let Some(why) = self.timing_refusal(c) {
            return Err(why);
        }
        let rolling = self.transport.state().rolling();
        let at = c.at.max(self.transport.frame());
        // While song mode plays the song's tempo points, a start leaves
        // its tempo out (they lead; playing, a tempo was refused above).
        let tempo = c.tempo.filter(|_| !self.song_tempo());
        let position = c
            .position
            .filter(|&p| rolling || p != self.transport.position());
        let take = c.take.filter(|_| !rolling);
        let toggle = take.is_some_and(|add| add != self.transport.overdub());
        let play = c.play && !rolling;
        let n = usize::from(tempo.is_some())
            + usize::from(position.is_some())
            + usize::from(toggle)
            + usize::from(play)
            + usize::from(take.is_some());
        if self.transport.queue_room() < n {
            self.refused += 1;
            return Err(QUEUE_FULL);
        }
        if self.transport.room_on(at) < n {
            self.refused += 1;
            return Err(PERIOD_FULL);
        }
        let mut sent = true;
        if let Some(bpm) = tempo {
            self.tempo_asked = Some((at, bpm));
            self.map_asked = None;
        }
        if rolling {
            if let Some(bpm) = tempo {
                sent &= self.transport.command(at, Command::TempoNow(bpm));
            }
            if let Some(p) = position {
                sent &= self.transport.command(at, Command::Locate(p));
            }
        } else {
            if let Some(p) = position {
                sent &= self.transport.command(at, Command::Locate(p));
            }
            if let Some(bpm) = tempo {
                sent &= self.transport.command(at, Command::SetTempo(bpm));
            }
            if toggle {
                sent &= self.transport.command(at, Command::ToggleOverdub);
            }
            if play {
                sent &= self.transport.command(at, Command::Play);
            }
            if take.is_some() {
                sent &= self.transport.command(at, Command::Record);
            }
        }
        // The room was counted, so this can't happen; counted if it does.
        if !sent {
            self.refused += 1;
            return Err(QUEUE_FULL);
        }
        if n > 0 {
            self.moved("a timing change landed");
        }
        Ok(at)
    }

    /// The song position and the start of the bar it is in.
    fn bar_start(&self) -> (Tick, Tick) {
        let pos = self.transport.position();
        let bar = self.transport.meter().bar_ticks().max(1);
        (pos, pos - pos.rem_euclid(bar))
    }

    /// Starts on a note refused so far ([`TimeAudio::start_at`]).
    #[allow(dead_code)]
    pub fn starts_refused(&self) -> u64 {
        self.starts_refused
    }

    /// Whether a live hit on a pad that starts on the launch line (a Loop
    /// pad, sampler R5.3b) waits for it now: the transport plays and the
    /// project's launch quantise isn't "at once".
    pub fn pads_wait(&self) -> bool {
        self.transport.state().rolling() && self.view.launch() != LaunchGrid::Now
    }

    /// Whether a hit or let-go played on `from` waits for its line: what
    /// is played there next waits too, to keep their order.
    pub fn waits_on(&self, from: PadFrom) -> bool {
        self.waits.waiting(from)
    }

    /// `w` waits for its launch line ([`PadWaits::wait`]). False when too
    /// many wait: it plays at once.
    pub fn wait_pad(&mut self, w: PadWait) -> bool {
        self.waits.wait(w)
    }

    /// Whether `w` lets go of a pad whose hit STOP dropped while it was
    /// held: it plays nothing ([`PadWaits::lets_go_of_dropped`]).
    pub fn lets_go_of_dropped(&mut self, w: PadWait) -> bool {
        self.waits.lets_go_of_dropped(w)
    }

    /// The pads of bank `page` (bit `p` for its pad `p`) whose hit waits
    /// for its line: they blink until it sounds.
    pub fn hits_waiting(&self, page: usize) -> u16 {
        self.waits.hits_waiting(page)
    }

    /// The waiting hits and let-gos whose line falls in the period
    /// [`TimeAudio::advance`] ran, each with its frame in the period, in
    /// order, in `due`. Each line the playhead plays is a line, wherever
    /// it jumps, but not a count-in's: a hit made then waits for the first
    /// line after it. Once the launch quantise is at once, everything left
    /// plays where it changed, a frame apart; once the transport stops,
    /// the hits still waiting are dropped with their let-gos and the
    /// let-gos of hits that sounded play where it stopped. Returns how
    /// many hits STOP dropped. Never allocates.
    pub fn due_pads(&mut self, due: &mut DuePads) -> usize {
        due.clear();
        if self.waits.is_empty() {
            return 0;
        }
        let grid = self.view.launch();
        let last = self.block.frames.saturating_sub(1);
        for s in self.block.spans() {
            // A frame apart, in order, so a pad's hit and its let-go land
            // as they were made (the arp ends a note before it starts one
            // on the same frame).
            let mut at = s.offset.min(last);
            let push = |w| {
                due.push(w, at);
                at = (at + 1).min(last);
            };
            if !s.state.rolling() {
                return self.waits.stop(push);
            }
            if grid == LaunchGrid::Now {
                self.waits.all(push);
                return 0;
            }
            if s.state == State::CountIn {
                continue;
            }
            for (_, offset) in s.lines(grid.line(s.meter.bar_ticks())) {
                self.waits.line(|w| due.push(w, offset));
            }
        }
        0
    }

    pub fn pad_pressed(&mut self, offset: u32, h: &Heard) {
        let Some(event) = h.pad else {
            return;
        };
        if self.tracks.records_pressed() {
            if self.takes_notes() {
                // A clip of notes takes the pad's notes as pressed (step
                // 5d); a pad with none to give is counted, not recorded.
                if h.notes().is_empty() && matches!(event, Event::NoteOn { .. }) {
                    self.kind_refused += 1;
                }
                for &e in h.notes() {
                    self.recorder.input(offset, e);
                }
                self.keep_pad_notes(h);
            } else {
                self.recorder.input(offset, event);
            }
            self.played(offset, event);
        }
    }

    /// Which notes a pad started, for its pressure on a take of notes.
    fn keep_pad_notes(&mut self, h: &Heard) {
        let Some(Event::NoteOn { note, .. }) = h.pad else {
            return;
        };
        if let Some(p) = self.pad_notes.get_mut(usize::from(note)) {
            *p = ([0; HEARD_NOTES], 0);
            for e in h.notes() {
                if let Event::NoteOn { note: n, .. } = *e {
                    p.0[usize::from(p.1)] = n;
                    p.1 += 1;
                }
            }
        }
    }

    pub fn pad(&mut self, offset: u32, event: Event) {
        if !self.tracks.records_pressed() {
            self.recorder.input(offset, event);
            self.played(offset, event);
        }
        self.hit_info(offset, event);
    }

    /// The armed track's clip holds notes (step 5c), so a take records
    /// notes, not pads.
    fn takes_notes(&self) -> bool {
        self.tracks
            .armed
            .and_then(|t| self.now_playing(t))
            .is_some_and(|n| n.holds_notes)
    }

    /// A live note heard on the armed track ([`Pads::live`]): a take of
    /// pad hits records its pad, as [`TimeAudio::pad`]; a take of notes
    /// its notes, and keeps which notes the pad started for its pressure.
    /// CAPTURE's history and the Play page's timing take the pad either
    /// way.
    fn heard(&mut self, offset: u32, h: &Heard) {
        if !self.takes_notes() {
            if let Some(ev) = h.pad {
                self.pad(offset, ev);
            }
            return;
        }
        if !self.tracks.records_pressed() {
            for &e in h.notes() {
                self.recorder.input(offset, e);
            }
        }
        if let Some(ev) = h.pad {
            // Pads as pressed keep their own notes (`pad_pressed`).
            if !self.tracks.records_pressed() {
                self.keep_pad_notes(h);
                self.played(offset, ev);
            }
            self.hit_info(offset, ev);
        }
    }

    /// A pad's hit while the transport rolls, for the Play page's timing.
    fn hit_info(&mut self, offset: u32, event: Event) {
        let Event::NoteOn { note, velocity } = event else {
            return;
        };
        if self.n_hits >= MAX_HITS {
            return;
        }
        let span = self
            .block
            .spans()
            .iter()
            .find(|s| offset >= s.offset && offset < s.offset + s.len.max(1));
        if let Some(s) = span.filter(|s| s.state.rolling()) {
            self.hits[self.n_hits] = HitInfo {
                pad: note,
                velocity,
                tick: s.tick_at(s.frame + u64::from(offset - s.offset)),
                bpm: s.bpm,
                bar_ticks: s.meter.bar_ticks(),
            };
            self.n_hits += 1;
        }
    }

    pub fn pad_pressure(&mut self, offset: u32, pad: u8, value: u8) {
        if pad >= 128 {
            return;
        }
        let event = Event::Pressure { note: pad, value };
        if self.tracks.records_touch() {
            if self.takes_notes() {
                // On each note the pad started (step 5c).
                let (notes, n) = self.pad_notes[usize::from(pad)];
                for &note in &notes[..usize::from(n)] {
                    self.recorder.input(offset, Event::Pressure { note, value });
                }
            } else {
                self.recorder.input(offset, event);
            }
        }
        // CAPTURE's history keeps it always, as it keeps the notes (its
        // keep leaves it out for a track that doesn't record pressure).
        let sent = &mut self.pressure_sent[usize::from(pad)];
        if *sent != Some(value) {
            *sent = Some(value);
            self.send(ToControl::Played(offset, event));
        }
    }

    /// A key a route played on `track`, not the armed one, as the pad
    /// that plays its note there (a note, its end or its pressure), or
    /// `AllOff` for every note held there ending at once, for CAPTURE's
    /// history; and, on another armed track ([`PlayView::set_also`]), for
    /// its recorder while the transport records (its notes, not its
    /// pressure: that is the armed track's alone for now, and counted).
    pub fn routed(&mut self, track: u8, offset: u32, event: Event) {
        let f = self.transport.frame() + u64::from(offset);
        self.note_started(f, u16::from(track), event);
        // A routed key comes as the pad that plays its note: no note for a
        // clip of notes on that track (step 5c), so counted, not recorded.
        let mut rank = self.view.also_rank(usize::from(track));
        if rank.is_some()
            && self
                .now_playing(usize::from(track))
                .is_some_and(|n| n.holds_notes)
        {
            if matches!(event, Event::NoteOn { .. }) {
                self.kind_refused += 1;
            }
            rank = None;
        }
        if let Some(k) = rank {
            match event {
                Event::NoteOn { .. } | Event::NoteOff { .. } | Event::AllOff => {
                    match self.also.get_mut(k) {
                        Some(r) => r.input(offset, event),
                        None => self.also_dropped += 1,
                    }
                }
                Event::Pressure { .. } if self.transport.state() == State::Recording => {
                    self.also_pressure += 1
                }
                _ => {}
            }
        }
        if matches!(
            event,
            Event::NoteOn { .. } | Event::NoteOff { .. } | Event::Pressure { .. } | Event::AllOff
        ) {
            self.send(ToControl::PlayedOn(track, offset, event));
        }
    }

    /// A pad note for CAPTURE's history; a press sends its pad's next
    /// pressure value even when it is the last one sent.
    fn played(&mut self, offset: u32, event: Event) {
        if let Event::NoteOn { note, .. } = event {
            if let Some(p) = self.pressure_sent.get_mut(usize::from(note)) {
                *p = None;
            }
        }
        self.send(ToControl::Played(offset, event));
    }

    pub fn touch_dropped(&self) -> u64 {
        self.recorder.touch_dropped() + self.also_pressure
    }

    /// Live erase: sound `note` erased on the armed track while the
    /// transport records (`on`), or not any more.
    pub fn erase_sound(&mut self, note: u8, on: bool) {
        let Some(bit) = 1u128.checked_shl(u32::from(note)) else {
            return;
        };
        let now = self.player.erasing();
        self.player
            .set_erasing(if on { now | bit } else { now & !bit });
    }

    /// Live erase off for every sound.
    pub fn erase_none(&mut self) {
        self.player.set_erasing(0);
    }

    /// The sounds live erase erases, a bit each.
    pub fn erasing(&self) -> u128 {
        self.player.erasing()
    }

    pub fn knob(&mut self, offset: u32, addr: mpfree_play::mpfree_param::ParamAddr, value: f32) {
        self.recorder.input(offset, Event::Lock { addr, value });
    }

    pub fn gesture(&mut self, offset: u32, input: Input) {
        self.send(ToControl::Moved(offset, input));
        self.recorder.input_move(offset, input);
    }

    pub fn set_fill(&mut self, on: bool, offset: u32) {
        self.player.set_fill(on, offset);
    }

    pub fn fill(&self) -> bool {
        self.player.fill()
    }

    /// Follow another device's clock with these options (the project's
    /// MIDI setup), or not (None).
    pub fn follow(&mut self, follow: Option<Follow>) {
        self.follower.set(follow);
    }

    /// The followed port's bytes, read at `frame` on the transport's frame
    /// clock ([`TimeAudio::next_frame`] for bytes read at the top of the
    /// period), before [`TimeAudio::advance`].
    pub fn clock_in(&mut self, frame: u64, bytes: &[u8]) {
        self.follower.bytes(&mut self.transport, frame, bytes);
    }

    /// The first frame of the coming period, on the transport's frame
    /// clock.
    pub fn next_frame(&self) -> u64 {
        self.transport.frame()
    }

    /// The followed clock's tempo while one arrives (for the tempo chip).
    pub fn followed_bpm(&self) -> Option<f64> {
        self.follower.bpm()
    }

    /// Following another device's Start and Stop while it is stopped:
    /// PLAY waits for it.
    pub fn clock_waits(&self) -> bool {
        self.follower.waits()
    }

    /// A take the app has ready, adding (true) or replacing: the leader's
    /// start records it.
    pub fn take_waits(&mut self, take: Option<bool>) {
        self.follower.take_waits(take);
    }

    /// True once after the leader's start began a take.
    pub fn took(&mut self) -> bool {
        self.follower.took()
    }

    /// The transport runs the coming period, after the follower's turn.
    fn process(&mut self, frames: u32) -> bool {
        // A timing change due in this period lands first (step 0C).
        self.commit_due(frames);
        self.follower.period(&mut self.transport);
        let ok = self.transport.process(frames, &mut self.block);
        // A tempo asked for is in once the transport plays at it. A change
        // while rolling waits for the next 1/16 line, so until then it
        // still counts as asked; stopped, its period having run is enough
        // (a tempo the transport kept in its range never matches).
        if self.tempo_asked.is_some_and(|(due, bpm)| {
            due < self.transport.frame()
                && ((self.transport.bpm() - bpm).abs() <= 1e-9 || !self.transport.state().rolling())
        }) {
            self.tempo_asked = None;
        }
        // The song's map is in once the transport has it.
        if self.map_asked.as_ref() == Some(self.transport.tempo_map()) {
            self.map_asked = None;
        }
        ok
    }

    pub fn advance(&mut self, frames: u32) -> bool {
        // Notes of a period gone that nobody took.
        if self.n_notes > 0 {
            let now = self.transport.frame();
            let mut k = 0;
            for i in 0..self.n_notes {
                if self.notes[i].0 >= now {
                    self.notes[k] = self.notes[i];
                    k += 1;
                }
            }
            self.n_notes = k;
        }
        self.notes_lost_to = self.notes_lost_to.filter(|&f| f >= self.transport.frame());
        self.receive();
        let ok = self.process(frames);
        self.advanced = Some(ok);
        ok
    }

    #[allow(dead_code)]
    pub fn arp_clock(&mut self) -> Option<mpfree_arp::Clock> {
        let jump = |m: &Mark| {
            matches!(
                m,
                Mark::Started { .. }
                    | Mark::CountIn { .. }
                    | Mark::Located { .. }
                    | Mark::Looped { .. }
            )
        };
        let first = self.block.spans().first().copied();
        let first_offset = first.map_or(0, |s| s.offset);
        let mut jumped = std::mem::take(&mut self.jump_pending);
        for m in self.block.markers() {
            if jump(&m.mark) {
                if m.offset <= first_offset {
                    jumped = true;
                } else {
                    self.jump_pending = true;
                }
            }
        }
        let span = first?;
        if !span.state.rolling() {
            return None;
        }
        Some(mpfree_arp::Clock {
            anchor_tick: span.anchor_tick,
            frames_since_anchor: span.frame.saturating_sub(span.anchor_frame),
            bpm: span.bpm,
            playing: true,
            jumped,
            swing: (span.swing.amount > 50).then_some((span.swing.grid, span.swing.amount)),
            bar_ticks: u32::try_from(span.meter.bar_ticks()).unwrap_or(4 * mpfree_arp::PPQ),
        })
    }

    /// Each track's live transpose (sequencer S4), as the control half
    /// last sent it; as written past the last track.
    pub fn transposes(&self) -> [mpfree_seq::Transpose; mpfree_seq::MAX_TRACKS] {
        std::array::from_fn(|t| {
            if t < self.tracks.n {
                self.tracks.set[t].transpose
            } else {
                mpfree_seq::Transpose::NONE
            }
        })
    }

    /// The per-track arps take each track's switch, destination and MIDI
    /// effects, and a chain to run the effects in once the control half
    /// has sent one.
    pub fn dress_arps(&mut self, arps: &mut TrackArps) {
        for t in 0..self.tracks.n {
            arps.set_through(t, self.tracks.set[t].through_arp);
            arps.set_dest(t, self.tracks.set[t].dest);
            if let Some(c) = self.chains_in[t].take() {
                if let Some(back) = arps.give_chain(t, c) {
                    self.free_chain(back);
                }
            }
            arps.set_notes(t, &self.note_fx_now(t));
        }
    }

    /// Track `t`'s MIDI effects as they play now: the track's own, with
    /// the mod engine's values on top; none past the last track.
    pub fn note_fx_now(&self, t: usize) -> TrackFx {
        let (Some(own), Some(mods)) = (self.note_fx.get(t), self.note_mods.get(t)) else {
            return TrackFx::default();
        };
        let mut fx = *own;
        for m in mods.iter().filter(|m| m.id != 0) {
            fx.set_value(m.id, usize::from(m.k), m.value);
        }
        fx
    }

    /// One place of a track's MIDI effects, as the control half sent it.
    /// What the mod engine held on the effect that was there goes when
    /// another effect (or another id) takes the place.
    fn note_place(&mut self, t: usize, place: usize, slot: Option<Slot>, id: u16) {
        let (Some(fx), Some(mods)) = (self.note_fx.get_mut(t), self.note_mods.get_mut(t)) else {
            return;
        };
        if place >= MAX_FX {
            return;
        }
        let kind = |s: Option<Slot>| s.map(|s| std::mem::discriminant(&s.fx));
        let (old, old_id) = (fx.slots[place], fx.ids[place]);
        if kind(old) != kind(slot) || old_id != id {
            for m in mods
                .iter_mut()
                .filter(|m| m.id != 0 && (m.id == old_id || m.id == id))
            {
                *m = NoteMod::default();
            }
        }
        fx.slots[place] = slot;
        fx.ids[place] = id;
        self.note_fx_moved = true;
    }

    /// The mod engine's value for setting `k` of the effect with id `id` on
    /// `track` (`Some`), or its end (`None`: the track's own value comes
    /// back). It applies from the next period. Never allocates: one more
    /// than [`NOTE_MODS`] on a track is counted and dropped.
    pub fn modulate_note_fx(&mut self, track: usize, id: u16, k: u8, value: Option<i32>) {
        let (Some(fx), Some(mods)) = (self.note_fx.get(track), self.note_mods.get_mut(track))
        else {
            return;
        };
        let at = mods.iter().position(|m| m.id == id && m.k == k);
        match (value, at) {
            (None, Some(i)) => mods[i] = NoteMod::default(),
            (None, None) => {}
            (Some(value), Some(i)) => mods[i].value = value,
            (Some(value), None) => {
                if id == 0 || fx.value(id, usize::from(k)).is_none() {
                    return;
                }
                match mods.iter().position(|m| m.id == 0) {
                    Some(i) => mods[i] = NoteMod { id, k, value },
                    None => self.note_mods_dropped += 1,
                }
            }
        }
    }

    /// The tracks' MIDI effects as this half has them (with their ids).
    pub fn note_fx(&self) -> &[TrackFx; MAX_TRACKS] {
        &self.note_fx
    }

    /// Whether the tracks' MIDI effects changed since the last call.
    pub fn take_note_fx_moved(&mut self) -> bool {
        std::mem::take(&mut self.note_fx_moved)
    }

    /// MIDI effect values the mod engine sent with no room for them.
    pub fn note_mods_dropped(&self) -> u64 {
        self.note_mods_dropped
    }

    /// One period: the transport (unless [`TimeAudio::advance`] ran it),
    /// the clips into `pads` (through `arps` if given), recording into the
    /// view in place, the click; then everything the control half needs
    /// from it, ending with [`ToControl::Period`].
    /// The click's level, as a gain.
    pub fn set_click_gain(&mut self, gain: f32) {
        self.click.gain = gain;
    }

    pub fn run(&mut self, frames: u32, pads: &mut impl Pads, arps: Option<&mut TrackArps>) {
        let ok = match self.advanced.take() {
            Some(ok) => ok,
            None => {
                self.receive();
                self.process(frames)
            }
        };
        if !ok {
            return;
        }
        self.sink.clear();
        self.locks.clear();
        self.lanes.clear();
        match arps {
            // The player's notes go through the per-track arps (a through
            // track's into its arp, the rest as they are), then to the pads,
            // with what the arps played (the live pads among it) marked.
            Some(arps) => {
                self.player
                    .process(&self.view, &self.block, &mut arps.through_sink());
                // A track going silent on its line (the mute queue): what
                // its MIDI effects hold or have waiting ends there.
                for l in self.player.mutes_landed() {
                    if l.silent {
                        let offset = self
                            .block
                            .spans()
                            .iter()
                            .find_map(|s| s.offset_of(l.at as f64))
                            .unwrap_or(0);
                        arps.cut(l.track, offset);
                    }
                }
                let sink = &mut *self.sink;
                arps.finish_with(&self.block, |o, arp| {
                    sink.store(*o, arp);
                });
                (self.arps_busy, self.arps_lost) = (arps.refused(), arps.lost());
            }
            None => self
                .player
                .process(&self.view, &self.block, &mut *self.sink),
        }
        for i in 0..self.sink.len() {
            let Some((o, arp)) = self.sink.get(i) else {
                break;
            };
            if arp {
                // Live playing: the pad player's own treatment, and what it
                // says to record goes to the armed track as heard (or, on a
                // track whose clips play through its arp, the pads as
                // pressed do, through pad_pressed).
                let heard = pads.live(&o);
                let f = self.block.start_frame + u64::from(o.offset);
                self.note_started(f, o.track, o.event);
                if self.tracks.armed == Some(usize::from(o.track)) {
                    self.heard(o.offset, &heard);
                }
                continue;
            }
            // A track that plays ahead (S5) had this event go out to its
            // gear already, if the forecast had it: the rest of what
            // `Pads::out` does happens now, MIDI out's part doesn't again.
            if self.ahead.sent_already(&o) {
                pads.out_sent(&o);
                continue;
            }
            // A lock or lane on the controls of a track that plays ahead
            // that didn't go ahead (the forecast missed it, or MIDI out
            // refused it then): MIDI out sends it now if it takes it, so
            // the control stays with the look-ahead; else the mod engine
            // has it, as for any track.
            if is_gear_control(&o) && self.ahead.plays_ahead(o.track) && pads.out_ahead(&o) {
                pads.out_sent(&o);
                continue;
            }
            // A track that plays elsewhere (MIDI OUT) takes its clips'
            // events first.
            if pads.out(&o) {
                continue;
            }
            match o.event {
                Event::NoteOn { note, velocity } => {
                    pads.hit_at(o.track, usize::from(note), velocity, o.offset);
                }
                Event::NoteOff { note } => pads.release_at(o.track, usize::from(note), o.offset),
                Event::AllOff => pads.stop_all(),
                Event::Pressure { note, value } => {
                    pads.pressure_at(o.track, usize::from(note), value, o.offset)
                }
                // Parameter locks: a kit pad's sound and an instrument
                // take theirs on the frame (`Pads::lock_at`); the rest go
                // to the mod engine in the mixer (`MixHost::lock`), which
                // the audio loop hands them to.
                Event::Lock { addr, value } => {
                    if !pads.lock_at(addr, Some(value), o.offset) {
                        self.send_to_mixer(&*pads, &o, addr, Ev::Lock(value));
                    }
                }
                Event::Unlock { addr } => {
                    if !pads.lock_at(addr, None, o.offset) {
                        self.send_to_mixer(&*pads, &o, addr, Ev::End);
                    }
                }
                // A move lane's value goes to the mod engine too, where any
                // hand on the control beats it.
                Event::Lane { addr, value } => {
                    self.send_to_mixer(&*pads, &o, addr, Ev::Lane(value))
                }
                // A clip's program change goes to outside gear alone
                // (`Pads::out`, S5): the pads have no programs.
                Event::Program { .. } => {}
            }
        }
        // The look-ahead (S5): a note-on sent ahead that this pass didn't
        // play gets its note-off now, a control's lock its unlock; then the coming periods' events of
        // the tracks that play ahead go out.
        let start = self.transport.frame().saturating_sub(u64::from(frames));
        self.ahead.check(self.transport.frame(), |o| {
            let o = Out {
                offset: 0,
                frame: start,
                ..*o
            };
            // A control's unlock goes where its lock went.
            if is_gear_control(&o) {
                pads.out_ahead(&o);
            } else {
                pads.out(&o);
            }
        });
        self.ahead
            .forecast(&self.transport, &self.player, &self.view, frames, |o| {
                pads.out_ahead(o)
            });
        let take = self.recorder.take();
        self.recorder.process(&self.view, &self.block);
        // The other armed tracks, numbering the take as the armed one's.
        let live = (self.view.also().count_ones() as usize).min(self.also.len());
        let ran = live.max(self.also_live);
        for r in &mut self.also[..ran] {
            r.set_take(take);
            r.process(&self.view, &self.block);
        }
        self.also_live = live;
        // Cues that started on a bar line: the player switched already;
        // the control half makes them the tracks' launches.
        for i in 0..self.player.started().len() {
            let st = self.player.started()[i];
            self.send(ToControl::Started(st));
        }
        // Mute cues that landed on their line: the control half applies
        // the queue.
        for i in 0..self.player.mutes_landed().len() {
            let l = self.player.mutes_landed()[i];
            self.send(ToControl::MuteLanded(l));
        }
        // Recorded and erased notes, locks and gestures go into the view
        // in place (unless it has no room left), numbered in this order,
        // and on to the control half, which makes them to the clips.
        let mut no_room = false;
        let mut made = Made {
            view: &mut self.view,
            recent: &mut self.recent,
            tx: &mut self.tx,
            lost: &mut self.lost,
            no_room: &mut no_room,
        };
        for e in self.player.erased() {
            made.make(Rec::Erase(*e), &[], &[]);
        }
        for e in self.player.erased_locks() {
            made.make(Rec::EraseLock(*e), &[], &[]);
        }
        for k in self.recorder.taken() {
            made.make(Rec::Note(*k), self.recorder.touch_points(&k.note), &[]);
        }
        for r in &self.also[..ran] {
            for k in r.taken() {
                made.make(Rec::Note(*k), &[], &[]);
            }
        }
        for k in self.recorder.taken_locks() {
            made.make(Rec::Lock(*k), &[], &[]);
        }
        for k in self.recorder.taken_moves() {
            made.make(Rec::Move(*k), &[], self.recorder.move_points(k));
        }
        if no_room {
            self.send(ToControl::Compile);
        }
        if self.metronome {
            for span in self.block.spans() {
                if matches!(span.state, State::CountIn | State::Recording) {
                    let bar = span.meter.bar_ticks();
                    for (tick, offset) in span.lines(span.meter.beat_ticks()) {
                        self.click.schedule(offset, tick.rem_euclid(bar) == 0);
                    }
                }
            }
        }
        for i in 0..self.block.spans().len() {
            let s = self.block.spans()[i];
            self.send(ToControl::Span(s));
        }
        let follow = self.follower.counts();
        if follow != self.follow_sent && self.tx.send(ToControl::Follow(follow)).is_ok() {
            self.follow_sent = follow;
        }
        let cues = self.cue_counts();
        if cues != self.cues_sent && self.tx.send(ToControl::Cues(cues)).is_ok() {
            self.cues_sent = cues;
        }
        let strips = self.strips.counts();
        if strips != self.strips_sent && self.tx.send(ToControl::Strips(strips)).is_ok() {
            self.strips_sent = strips;
        }
        let end = PeriodEnd {
            start_frame: self.block.start_frame,
            frames: self.block.frames,
            info: self.info(),
            counts: self.counts(),
        };
        self.send(ToControl::Period(end));
    }

    /// The armed track as the audio half has it: its recorder takes live
    /// notes for it alone.
    #[cfg(test)]
    pub(super) fn armed(&self) -> Option<usize> {
        self.tracks.armed
    }

    pub fn counts(&self) -> Counts {
        Counts {
            dropped: self.recorder.dropped()
                + self.also.iter().map(Recorder::dropped).sum::<u64>()
                + self.also_dropped
                + self.kind_refused
                + self.player.stats().dropped
                + self.locks_dropped,
            touch_dropped: self.touch_dropped(),
            lost: self.lost,
            behind: super::link::rare(self.behind),
            kept: super::link::rare(self.kept),
            refused: super::link::rare(self.refused),
            missed: super::link::rare(self.missed),
            busy: self.sink.refused() + self.player.stats().busy + self.arps_busy,
            ends_lost: super::link::rare(self.sink.lost() + self.locks_lost + self.arps_lost),
        }
    }

    /// What the timing contract counted (Arrange 2.5): the player's cues
    /// that came late, were dropped as stale, or lost a switch's clip.
    pub fn cue_counts(&self) -> CueCounts {
        let s = self.player.stats();
        CueCounts {
            late: super::link::rare(s.late_cues),
            stale: super::link::rare(s.stale_cues),
            lost: super::link::rare(s.lost_switches),
        }
    }

    pub fn mix_click(&mut self, left: &mut [f32], right: &mut [f32]) -> f32 {
        self.click.mix(left, right)
    }

    pub fn block(&self) -> &Block {
        &self.block
    }

    pub fn tempo_map(&self) -> &mpfree_transport::TempoMap {
        self.transport.tempo_map()
    }

    /// What `track` plays now ([`mpfree_play::Player::now_playing`]): for
    /// a frozen mixer strip that follows the track's loop (takes.rs).
    pub fn now_playing(&self, track: usize) -> Option<mpfree_play::NowPlaying> {
        self.player
            .now_playing(&self.view, track, self.transport.position())
    }

    pub fn position_text(&self) -> String {
        let t = &self.transport;
        t.meter().position(t.position()).to_string()
    }

    /// The transport for the screens, with the followed clock's tempo
    /// rather than the nudged one while following.
    pub fn info(&self) -> TransportInfo {
        let mut info = TransportInfo::of(&self.transport);
        if let Some(bpm) = self.follower.bpm() {
            info.bpm = bpm;
        }
        if let Some(held) = self.follower.held_loop() {
            info.follows_position = true;
            info.loop_range = held;
        }
        info
    }
}

/// Where a period's recorded changes go: the view in place, the ring of
/// recent changes, and the queue to the control half.
struct Made<'a> {
    view: &'a mut PlayView,
    recent: &'a mut Recent,
    tx: &'a mut Tx<ToControl>,
    lost: &'a mut u64,
    /// The view had no room in place for one.
    no_room: &'a mut bool,
}

impl Made<'_> {
    fn make(&mut self, rec: Rec, touch: &[TouchPoint], moves: &[(f64, f32, Kind)]) {
        let touch = &touch[..touch.len().min(TOUCH_POINTS)];
        let moves = &moves[..moves.len().min(MOVE_POINTS)];
        let number = self.recent.push(rec, touch, moves);
        *self.no_room |= !self.view.record(&rec, touch, moves);
        // Its points first, then the change.
        for c in touch.chunks(TOUCH_CHUNK) {
            let mut a = [TouchPoint { at: 0, value: 0 }; TOUCH_CHUNK];
            a[..c.len()].copy_from_slice(c);
            self.send(ToControl::Touch(a, c.len() as u8));
        }
        for c in moves.chunks(POINTS_CHUNK) {
            let mut a = [(0.0, 0.0, Kind::Step); POINTS_CHUNK];
            a[..c.len()].copy_from_slice(c);
            self.send(ToControl::Points(a, c.len() as u8));
        }
        self.send(ToControl::Rec(number, rec));
    }

    fn send(&mut self, m: ToControl) {
        if self.tx.send(m).is_err() {
            *self.lost += 1;
        }
    }
}
