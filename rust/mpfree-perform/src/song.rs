//! Live to song (roadmap T11): what is played on the launch grid goes
//! into the arrangement, so a jam becomes a song to edit.
//!
//! [`SongRecorder`] is control-side code. While it is armed and the
//! transport plays, it follows what each track plays from the session
//! (the launch grid):
//!
//! - **Launches and stops.** A clip that starts on a track (a launch, a
//!   scene, a follow action) becomes a placement naming the clip by id from
//!   that tick, at the right place in the clip ([`Placement::offset`]), and
//!   it ends where the track switches clip or stops.
//! - **Mutes.** A muted stretch is left out: the placement ends where the
//!   track is muted, and a new one starts where it is heard again, in
//!   phase with the clip.
//! - **The morph.** Moves of the morph crossfader become the song's
//!   morph lane (`Arrangement::lanes`, on `MORPH_LANE`), and the two rows
//!   it morphs between are kept with them (`Arrangement::morphs`). A pass
//!   with no move leaves the lane as it was.
//! - **The XY pad's moves** (the XY controller plan, phase 3): a setting
//!   the pad moves ([`SongRecorder::moved`]: the master, a bus, a return,
//!   another track's, or with Rec song alone any of them) becomes a move
//!   in the song's lane for it (`Target::Fixed`), over what the lane had
//!   only where it moved, as a knob's move records into a clip. A gesture
//!   still moving at a jump goes on in the next pass from where the
//!   transport lands; a stop ends it. A move that finds no room (the
//!   song's 64 lanes, or a lane's points) is left out and counted
//!   ([`Recorded::moves_lost`]), never failing the rest.
//!
//! A track playing the song (song mode, nothing launched on it) is not
//! recorded: the song stays as it is there. Where the session played a
//! track, what the song had there is replaced, as when recording over a
//! tape. A pass ends where the transport stops or jumps (a locate, a loop
//! jump), and a new one starts where it lands, so each loop pass records
//! over the last. Nothing goes into the project until
//! [`SongRecorder::finish`] (the host calls it when the transport stops or
//! recording is turned off), so the song does not change under the player
//! while recording.
//!
//! **Room.** The song holds at most [`MAX_PLACEMENTS`] blocks, and
//! [`SongRecorder::finish`] refuses a recording that would not fit, so
//! the recorder keeps within it as it goes. The host says how many blocks
//! the song has ([`SongRecorder::song_blocks`]); [`SongRecorder::worst`]
//! counts the most the song would hold if the recording went in now: the
//! song's blocks, each block heard (and the one each track has open), and
//! one more for each stretch it records over, where a block of the song
//! may be cut in two. Before the cues of a line switch anything, and
//! before a new pass after a jump, it counts what they could add; if that
//! wouldn't fit, recording stops on every track at that line (the pass
//! ends there and it disarms), so it never ends halfway through a scene
//! change, and what it recorded always goes in. The host takes the stop
//! once ([`SongRecorder::take_full`]) to say so, and a warning the first
//! time the count reaches 90 % ([`SongRecorder::take_warning`]).
//!
//! The host calls [`SongRecorder::period`] after each period with the
//! transport's block and the cues the player started in it. In the pad
//! player that is `TimeHost`'s period, which runs in the audio loop until
//! edits move to a control thread. A period where nothing starts or stops
//! allocates nothing (`tests/song_no_alloc.rs`). A clip starting or
//! stopping, a pass beginning or ending, and [`SongRecorder::finish`]
//! allocate, like any edit there today.

use mpfree_play::{Launch, Started};
use mpfree_seq::arrange::{MorphRows, MAX_PLACEMENTS, SONG_LANE_LEN};
use mpfree_seq::mpfree_moves::{Input, Kind, Map, Target};
use mpfree_seq::mpfree_param::ParamAddr;
use mpfree_seq::{ModelError, Placement, Project, MAX_TRACKS};
use mpfree_transport::{Block, State, Tick};

/// A pause in the crossfader's moves this long or longer (a 16th note, in
/// ticks) is held in the song's morph lane rather than ramped across.
pub const MORPH_HOLD: f64 = 240.0;

/// The clip a track plays from the session: its slot and origin.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Session {
    slot: usize,
    origin: Tick,
}

/// One pass: from where recording started (or the transport landed) to
/// where it stopped or jumped.
#[derive(Clone, Debug, Default, PartialEq)]
struct Pass {
    from: Tick,
    to: Tick,
    /// (track, from, to): where the session played the track; the song is
    /// replaced there.
    owned: Vec<(usize, Tick, Tick)>,
    /// (track, slot, origin, from, to): what was heard.
    heard: Vec<(usize, usize, Tick, Tick, Tick)>,
    /// The morph position's points, and the rows at each change.
    morph: Vec<(f64, f32, Kind)>,
    rows: Vec<MorphRows>,
    /// The XY pad's gestures, each a setting's points on song ticks.
    moves: Vec<SongMove>,
}

/// A setting's gesture in a pass, for the song's lane for it.
#[derive(Clone, Debug, PartialEq)]
struct SongMove {
    addr: ParamAddr,
    map: Map,
    points: Vec<(f64, f32, Kind)>,
}

/// What [`SongRecorder::finish`] put in the song.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Recorded {
    pub passes: usize,
    pub placements: usize,
    pub morph_points: usize,
    /// The XY pad's gestures written into the song's lanes, and those
    /// that found no room.
    pub moves: usize,
    pub moves_lost: usize,
}

/// Records the session into the song: see the module notes.
#[derive(Clone, Debug)]
pub struct SongRecorder {
    armed: bool,
    /// The next tick a pass expects (where the last span ended).
    next: Option<Tick>,
    playing: [Option<Session>; MAX_TRACKS],
    muted: [bool; MAX_TRACKS],
    /// Per track: where the session took it over, and where the stretch
    /// heard now began.
    owned_from: [Option<Tick>; MAX_TRACKS],
    heard_from: [Option<Tick>; MAX_TRACKS],
    morph: f32,
    rows: (usize, usize),
    /// The morph moved in this pass.
    morph_moved: bool,
    /// The XY pad's gestures under way: the setting, its map, its last
    /// knob place and its gesture in the pass.
    moving: Vec<(ParamAddr, Map, f32, usize)>,
    pass: Option<Pass>,
    done: Vec<Pass>,
    /// The most blocks the song may hold ([`MAX_PLACEMENTS`]).
    limit: usize,
    /// The song's blocks, as the host last said.
    song: usize,
    /// Where recording stopped for room, until the host takes it.
    full: Option<Tick>,
    /// The count that reached 90 %, until the host takes it; and whether
    /// it did since the recorder was armed.
    warning: Option<usize>,
    warned: bool,
}

impl Default for SongRecorder {
    fn default() -> SongRecorder {
        SongRecorder {
            armed: false,
            next: None,
            playing: [None; MAX_TRACKS],
            muted: [false; MAX_TRACKS],
            owned_from: [None; MAX_TRACKS],
            heard_from: [None; MAX_TRACKS],
            morph: 0.0,
            rows: (0, 1),
            morph_moved: false,
            moving: Vec::new(),
            pass: None,
            done: Vec::new(),
            limit: MAX_PLACEMENTS,
            song: 0,
            full: None,
            warning: None,
            warned: false,
        }
    }
}

impl SongRecorder {
    pub fn armed(&self) -> bool {
        self.armed
    }

    /// Recording now (armed, and a pass under way).
    pub fn recording(&self) -> bool {
        self.pass.is_some()
    }

    /// Passes ended and waiting for [`SongRecorder::finish`].
    pub fn waiting(&self) -> bool {
        !self.done.is_empty()
    }

    /// Arm it, with what the session plays now (`launches`) and the
    /// tracks' mutes, the morph's position and rows. It records from the
    /// next tick the transport plays. Disarming ends the pass: call
    /// [`SongRecorder::finish`] after.
    pub fn arm(
        &mut self,
        on: bool,
        project: &Project,
        launches: &[Launch],
        morph: f32,
        rows: (usize, usize),
    ) {
        if on && !self.armed {
            self.playing = [None; MAX_TRACKS];
            for l in launches {
                if let Some(p) = self.playing.get_mut(l.track) {
                    *p = Some(Session {
                        slot: l.slot,
                        origin: l.origin,
                    });
                }
            }
            for (t, m) in self.muted.iter_mut().enumerate() {
                *m = project.silent(t);
            }
            self.morph = morph;
            self.rows = rows;
            self.song = project.arrangement.placements.len();
            self.full = None;
            self.warned = false;
        }
        if !on {
            if let Some(at) = self.next {
                self.end_pass(at);
            }
            self.next = None;
            self.moving.clear();
        }
        self.armed = on;
    }

    /// How many blocks the song has now: the host says before each period
    /// (an edit in the Song view can change it while recording).
    pub fn song_blocks(&mut self, n: usize) {
        self.song = n;
    }

    /// The most blocks the song would hold if what is recorded so far went
    /// in now: see the module notes.
    pub fn worst(&self) -> usize {
        let kept: usize = self
            .done
            .iter()
            .chain(self.pass.as_ref())
            .map(|p| p.heard.len() + p.owned.len())
            .sum();
        let open = self.heard_from.iter().filter(|h| h.is_some()).count()
            + self.owned_from.iter().filter(|o| o.is_some()).count();
        self.song + kept + open
    }

    /// The most blocks the song may hold.
    pub fn limit(&self) -> usize {
        self.limit
    }

    /// Where recording stopped because the song would have been full
    /// (every track at that line), once.
    pub fn take_full(&mut self) -> Option<Tick> {
        self.full.take()
    }

    /// The count of blocks, once, the first time it reaches 90 % of the
    /// limit while armed.
    pub fn take_warning(&mut self) -> Option<usize> {
        self.warning.take()
    }

    /// Whether `more` blocks fit; if not, recording stops at `at` on every
    /// track. Warns the first time the count with them reaches 90 %.
    fn room(&mut self, more: usize, at: Tick) -> bool {
        let need = self.worst() + more;
        if need > self.limit {
            self.end_pass(at);
            self.next = None;
            self.moving.clear();
            self.armed = false;
            self.full = Some(at);
            return false;
        }
        if !self.warned && need * 10 >= self.limit * 9 {
            self.warned = true;
            self.warning = Some(need);
        }
        true
    }

    /// What a new pass from here could add: each track playing takes its
    /// stretch over (a block of the song may be cut) and, heard, a block.
    fn pass_needs(&self) -> usize {
        (0..MAX_TRACKS)
            .filter(|&t| self.playing[t].is_some())
            .map(|t| 1 + usize::from(!self.muted[t]))
            .sum()
    }

    /// What the cues starting at `at` could add: a block for each clip
    /// started, and a stretch taken over where the track had none.
    fn line_needs(&self, started: &[Started], at: Tick) -> usize {
        started
            .iter()
            .filter(|s| s.at == at && s.slot.is_some() && s.track < MAX_TRACKS)
            .map(|s| 1 + usize::from(self.owned_from[s.track].is_none()))
            .sum()
    }

    /// After each period: the transport's `block` and the cues the player
    /// started in it ([`mpfree_play::Player::started`]), before or after
    /// they are applied.
    pub fn period(&mut self, block: &Block, started: &[Started]) {
        if !self.armed {
            return;
        }
        for span in block.spans() {
            let ticks = span.ticks();
            let plays = matches!(span.state, State::Playing | State::Recording);
            if !plays {
                if let Some(at) = self.next.take() {
                    self.end_pass(at);
                }
                // A stop ends the XY pad's gestures.
                self.moving.clear();
                continue;
            }
            match self.next {
                Some(at) if at != ticks.start => {
                    // A locate or a loop jump: a new pass where it lands.
                    self.end_pass(at);
                    if !self.room(self.pass_needs(), at) {
                        return;
                    }
                    self.begin_pass(ticks.start);
                }
                None => {
                    if !self.room(self.pass_needs(), ticks.start) {
                        return;
                    }
                    self.begin_pass(ticks.start);
                }
                Some(_) => {}
            }
            // Line by line, in time order: all of a line's cues, or (no
            // room) none of them.
            let mut from = ticks.start;
            while let Some(at) = started
                .iter()
                .map(|s| s.at)
                .filter(|&at| at >= from && at < ticks.end)
                .min()
            {
                if !self.room(self.line_needs(started, at), at) {
                    return;
                }
                for st in started.iter().filter(|s| s.at == at) {
                    self.switch(st);
                }
                from = at + 1;
            }
            self.next = Some(ticks.end);
        }
    }

    /// The morph crossfader moved to `pos` at song tick `at` (between
    /// rows `rows`). After a pause of [`MORPH_HOLD`] or more, the lane holds
    /// the last place up to the move.
    pub fn morph(&mut self, at: Tick, pos: f32, rows: (usize, usize)) {
        let pos = pos.clamp(0.0, 1.0);
        let changed_rows = rows != self.rows;
        self.morph = pos;
        self.rows = rows;
        let Some(p) = self.pass.as_mut() else {
            return;
        };
        let at = at.max(p.from);
        if !self.morph_moved {
            self.morph_moved = true;
        }
        if changed_rows || p.rows.is_empty() {
            p.rows.push(MorphRows {
                at,
                a: rows.0,
                b: rows.1,
            });
        }
        // A crossfader left still holds its place until it moves again, so
        // a move after a pause starts from there instead of ramping across
        // the pause. A drag sends its moves closer together than
        // `MORPH_HOLD`, so it stays one smooth ramp.
        if let Some(&(t, u, _)) = p.morph.last() {
            let hold = at as f64 - 1.0;
            if hold - t >= MORPH_HOLD {
                p.morph.push((hold, u, Kind::Ramp));
            }
        }
        p.morph.push((at as f64, pos, Kind::Ramp));
    }

    /// A setting the XY pad moves, or lets go of, at song tick `at`
    /// (`Input::Move`, the first value starting a gesture, or
    /// `Input::End`): see the module notes. Nothing happens unless a pass
    /// records.
    pub fn moved(&mut self, at: Tick, input: Input) {
        let Some(p) = self.pass.as_mut() else {
            return;
        };
        let at = at.max(p.from) as f64;
        match input {
            Input::Move { addr, value, map } => {
                let u = map.unit(value);
                let kind = if map.stepped() {
                    Kind::Step
                } else {
                    Kind::Ramp
                };
                match self.moving.iter_mut().find(|m| m.0 == addr) {
                    Some(m) => {
                        m.2 = u;
                        if let Some(g) = p.moves.get_mut(m.3) {
                            let at = g.points.last().map_or(at, |l| at.max(l.0));
                            g.points.push((at, u, kind));
                        }
                    }
                    None => {
                        self.moving.push((addr, map, u, p.moves.len()));
                        p.moves.push(SongMove {
                            addr,
                            map,
                            points: vec![(at, u, kind)],
                        });
                    }
                }
            }
            Input::End { addr } => {
                let Some(k) = self.moving.iter().position(|m| m.0 == addr) else {
                    return;
                };
                let (_, _, _, g) = self.moving.swap_remove(k);
                if let Some(g) = p.moves.get_mut(g) {
                    let at = g.points.last().map_or(at, |l| at.max(l.0));
                    g.points.push((at, 0.0, Kind::End));
                }
            }
        }
    }

    /// Track `track` muted or heard again from song tick `at`, other than
    /// by a scene's cue (which comes with its [`Started`]).
    pub fn mute(&mut self, track: usize, on: bool, at: Tick) {
        if track >= MAX_TRACKS || self.muted[track] == on {
            return;
        }
        self.muted[track] = on;
        if self.pass.is_none() {
            return;
        }
        if on {
            self.close_heard(track, at);
        } else if self.playing[track].is_some() {
            self.heard_from[track] = Some(at);
        }
    }

    /// Put the passes recorded since the last call into the song. The
    /// passes go in in the order they were played, each replacing what
    /// the song had where the session played. On an error (a full song)
    /// it changes nothing and keeps the passes.
    pub fn finish(&mut self, project: &mut Project) -> Result<Recorded, ModelError> {
        if let Some(at) = self.next.filter(|_| self.pass.is_some()) {
            // Still rolling: what is recorded so far goes in, and the
            // pass goes on from here.
            self.end_pass(at);
            self.begin_pass(at);
        }
        let mut song = project.arrangement.clone();
        let mut out = Recorded::default();
        for p in &self.done {
            for &(track, from, to) in &p.owned {
                song.clear(&project.tracks, track, from, to)?;
            }
            for &(track, slot, origin, from, to) in &p.heard {
                // A block of the clip heard, by its id. A clip deleted
                // since (its cell empty) leaves nothing to name.
                let Some(clip) = project.clip(track, slot) else {
                    continue;
                };
                let mut pl = Placement::new(track, clip.id, from, to - from);
                let at = clip.speed.to_clip((from - origin) as f64).round() as Tick;
                pl.offset = at.rem_euclid(clip.length.max(1));
                song.place(pl)?;
                out.placements += 1;
            }
            if !p.morph.is_empty() {
                song.put_morph(&p.morph, &p.rows, p.from, p.to)?;
                out.morph_points += p.morph.len();
            }
            for m in &p.moves {
                // Song ticks never run past the song's lane (a lane wraps
                // a gesture longer than it).
                let last = f64::from(SONG_LANE_LEN - 1);
                if m.points.first().is_none_or(|f| f.0 > last) {
                    continue;
                }
                let mut points: Vec<(f64, f32, Kind)> =
                    m.points.iter().copied().filter(|q| q.0 <= last).collect();
                if m.points.len() > points.len() && points.last().is_some_and(|q| q.2 != Kind::End)
                {
                    points.push((last, 0.0, Kind::End));
                }
                match song
                    .lanes
                    .put(Target::Fixed(m.addr), m.map, &points, SONG_LANE_LEN)
                {
                    Ok(()) => out.moves += 1,
                    Err(_) => out.moves_lost += 1,
                }
            }
            out.passes += 1;
        }
        song.check(&project.tracks)?;
        project.arrangement = song;
        self.done.clear();
        Ok(out)
    }

    fn begin_pass(&mut self, at: Tick) {
        self.pass = Some(Pass {
            from: at,
            to: at,
            ..Pass::default()
        });
        self.morph_moved = false;
        for t in 0..MAX_TRACKS {
            let on = self.playing[t].is_some();
            self.owned_from[t] = on.then_some(at);
            self.heard_from[t] = (on && !self.muted[t]).then_some(at);
        }
        // The morph's place and rows where the pass starts, so a move in
        // it starts from there.
        let (pos, rows) = (self.morph, self.rows);
        if let Some(p) = self.pass.as_mut() {
            p.morph.push((at as f64, pos, Kind::Ramp));
            p.rows.push(MorphRows {
                at,
                a: rows.0,
                b: rows.1,
            });
            // The XY pad's gestures still moving (after a jump) go on from
            // where the transport landed.
            for m in &mut self.moving {
                let kind = if m.1.stepped() {
                    Kind::Step
                } else {
                    Kind::Ramp
                };
                m.3 = p.moves.len();
                p.moves.push(SongMove {
                    addr: m.0,
                    map: m.1,
                    points: vec![(at as f64, m.2, kind)],
                });
            }
        }
    }

    fn end_pass(&mut self, at: Tick) {
        if self.pass.is_none() {
            return;
        }
        for t in 0..MAX_TRACKS {
            self.close_heard(t, at);
            self.close_owned(t, at);
        }
        if let Some(mut p) = self.pass.take() {
            p.to = at;
            // Gestures still moving end where the pass does (a jump goes on
            // in the next one, `begin_pass`).
            for m in &self.moving {
                if let Some(g) = p.moves.get_mut(m.3) {
                    let at = g.points.last().map_or(at as f64, |l| (at as f64).max(l.0));
                    g.points.push((at, 0.0, Kind::End));
                }
            }
            if !self.morph_moved {
                p.morph.clear();
                p.rows.clear();
            } else {
                p.morph.push((at as f64, self.morph, Kind::Ramp));
            }
            if at > p.from {
                self.done.push(p);
            }
        }
    }

    fn switch(&mut self, st: &Started) {
        let t = st.track;
        if t >= MAX_TRACKS {
            return;
        }
        if let Some(m) = st.mute {
            self.muted[t] = m;
        }
        self.close_heard(t, st.at);
        match st.slot {
            Some(slot) => {
                self.playing[t] = Some(Session {
                    slot,
                    origin: st.origin,
                });
                if self.owned_from[t].is_none() {
                    self.owned_from[t] = Some(st.at);
                }
                if !self.muted[t] {
                    self.heard_from[t] = Some(st.at);
                }
            }
            None => {
                self.close_owned(t, st.at);
                self.playing[t] = None;
            }
        }
    }

    fn close_heard(&mut self, t: usize, at: Tick) {
        let (Some(from), Some(s)) = (self.heard_from[t].take(), self.playing[t]) else {
            return;
        };
        if let Some(p) = self.pass.as_mut().filter(|_| at > from) {
            p.heard.push((t, s.slot, s.origin, from, at));
        }
    }

    fn close_owned(&mut self, t: usize, at: Tick) {
        let Some(from) = self.owned_from[t].take() else {
            return;
        };
        if let Some(p) = self.pass.as_mut().filter(|_| at > from) {
            p.owned.push((t, from, at));
        }
    }
}
