//! Live to song (roadmap T11): what is played on the launch grid goes
//! into the arrangement, so a jam becomes a song to edit.
//!
//! [`SongRecorder`] is control-side code. While it is armed and the
//! transport plays, it follows what each track plays from the session
//! (the launch grid):
//!
//! - **Launches and stops.** A clip that starts on a track (a launch, a
//!   scene, a follow action) becomes a placement linked to its slot from
//!   that tick, at the right place in the clip ([`Placement::offset`]), and
//!   it ends where the track switches clip or stops.
//! - **Mutes.** A muted stretch is left out: the placement ends where the
//!   track is muted, and a new one starts where it is heard again, in
//!   phase with the clip.
//! - **The morph.** Moves of the morph crossfader become the song's
//!   morph lane (`Arrangement::lanes`, on `MORPH_LANE`), and the two rows
//!   it morphs between are kept with them (`Arrangement::morphs`). A pass
//!   with no move leaves the lane as it was.
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
//! The host calls [`SongRecorder::period`] after each period with the
//! transport's block and the cues the player started in it. In the pad
//! player that is `TimeHost`'s period, which runs in the audio loop until
//! edits move to a control thread. A period where nothing starts or stops
//! allocates nothing (`tests/song_no_alloc.rs`). A clip starting or
//! stopping, a pass beginning or ending, and [`SongRecorder::finish`]
//! allocate, like any edit there today.

use mpfree_play::{Launch, Started};
use mpfree_seq::arrange::MorphRows;
use mpfree_seq::mpfree_moves::Kind;
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
}

/// What [`SongRecorder::finish`] put in the song.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Recorded {
    pub passes: usize,
    pub placements: usize,
    pub morph_points: usize,
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
    pass: Option<Pass>,
    done: Vec<Pass>,
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
            pass: None,
            done: Vec::new(),
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
                *m = project.tracks.get(t).is_some_and(|t| t.mute);
            }
            self.morph = morph;
            self.rows = rows;
        }
        if !on {
            if let Some(at) = self.next {
                self.end_pass(at);
            }
            self.next = None;
        }
        self.armed = on;
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
                continue;
            }
            match self.next {
                Some(at) if at != ticks.start => {
                    // A locate or a loop jump: a new pass where it lands.
                    self.end_pass(at);
                    self.begin_pass(ticks.start);
                }
                None => self.begin_pass(ticks.start),
                Some(_) => {}
            }
            for st in started.iter().filter(|s| ticks.contains(&s.at)) {
                self.switch(st);
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
                let mut pl = Placement::linked(track, slot, from, to - from);
                if let Some(clip) = project.clip(track, slot) {
                    let at = clip.speed.to_clip((from - origin) as f64).round() as Tick;
                    pl.offset = at.rem_euclid(clip.length.max(1));
                }
                song.place(pl)?;
                out.placements += 1;
            }
            if !p.morph.is_empty() {
                song.put_morph(&p.morph, &p.rows, p.from, p.to)?;
                out.morph_points += p.morph.len();
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
