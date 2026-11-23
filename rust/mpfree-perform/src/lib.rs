//! Scenes and follow actions (roadmap T9) and the scene morph (T10), on
//! the control thread.
//!
//! The model is in `mpfree_seq::scenes`: scene `k` is row `k` of the
//! launch grid, with its mutes and follow action; a clip can have a follow
//! action too. [`Perform`] carries them out with the launches and cues the
//! host already keeps:
//!
//! - [`Perform::launch_scene`]: every track plays its clip in the row from
//!   the next bar line (a cue each, so they start together), and a track
//!   playing something with an empty cell stops there. The scene's mutes
//!   switch on the same bar line: each cue carries its track's mute
//!   ([`mpfree_play::PlayView::set_cue_mute`], set again after every
//!   compile by [`Perform::dress`]). A track with nothing playing or cued
//!   and an empty cell gets no cue: its mute waits for the same line in
//!   the host's mute queue ([`Perform::take_line_mutes`]), as TRACK
//!   MUTE's do, since in song mode such a track still plays the song.
//!   With the transport stopped, or Launch at once, it all happens at
//!   once. A scene's **Keep playing** tracks (the Arrange refactor's
//!   5.2, [`mpfree_seq::Scene::keep`]) are left out where their cell is
//!   empty: no cue, so what they play, and a cue already waiting, carry
//!   on through the scene change; their mutes still wait for the line.
//! - A scene's program changes (sequencer S5,
//!   [`mpfree_seq::Scene::programs`]) go out on the same line, every track
//!   with one, cued or not, so a track whose gear plays its own patterns
//!   switches with the scene too ([`mpfree_play::SceneProgram`], set on
//!   every compile by [`Perform::dress`]). Where the clip a track plays
//!   then has a program change of its own, the clip's goes out instead. A
//!   scene launched while stopped sends them where the music starts; after
//!   a stop or a jump they go out again, as a clip's do. The next launch
//!   replaces them, and Stop all forgets them.
//! - [`Perform::started`]: the host passes each [`Started`] the player
//!   reports, before applying it. A scene's mute becomes the track's
//!   `mute`, and the scene's first start is where its follow counts from.
//! - [`Perform::due`]: after each period, the follow actions that come due
//!   are queued as cues, in the bar before their point so they start on
//!   it: `bars` bars after the scene (or the clip) started. A late check
//!   (a locate past the point) starts it on the next bar line instead.
//!   A scene's follow goes first and cues every track, so a clip follow on
//!   a track that already has a cue waits: whatever is cued wins.
//! - A one-shot clip ([`mpfree_seq::Clip::once`]) has its track's stop
//!   cued the same way, in the bar before the end of the pass it started
//!   in, on that tick exactly ([`once_end`], `LaunchGrid::At`); its own
//!   follow waits.
//! - Stay keeps playing and rolls again `bars` bars later, so a follow
//!   with a chance between Next and Stay plays on for a random number of
//!   passes. Again starts the same again (a new start to count from).
//! - [`Perform::play_list`]: the song list (sequencer S4) from a step: its
//!   scene launches as above, and [`Perform::due`] launches the next
//!   step's scene in the bar before the step's span ends (its scene's
//!   length times its times), on the bar line where it ends, or stops
//!   every track after the last step (the first again when the list goes
//!   round). While the list plays, the scenes' own follow actions wait;
//!   the host ends it ([`Perform::end_list`]) when a scene is launched by
//!   hand or everything stops.
//! - [`Perform::section_starts`]: a song linked to its list (the Arrange
//!   refactor's 4b.4) plays its sections' blocks, made with no scene's
//!   mutes; as each section starts, the host brings its scene's mutes
//!   and program changes in here, as a launch brings them for the tracks
//!   it gives no cue (the host's `sections` module says when).
//!
//! [`morph`]: the scene morph (T10), two rows' stored values on the mod
//! engine's scenes A and B.
//!
//! [`song`]: live to song (T11), what the launch grid plays recorded into
//! the arrangement.
//!
//! None of this runs in the player: the player only sees the cues. In the
//! pad player, `TimeHost` calls it from its period, as it does its other
//! edits, until edits move to a control thread.

#![forbid(unsafe_code)]

pub mod morph;
pub mod song;
pub use morph::{save_scene, Loaded, SceneMorph};
pub use song::{Recorded, SongRecorder};

use mpfree_play::{anchored, Cue, Launch, PlayView, SceneProgram, Started};
use mpfree_seq::arrange::scene_length;
use mpfree_seq::scenes::follow_target;
use mpfree_seq::{
    bank, Clip, Follow, FollowAction, LaunchGrid, Project, MAX_TRACKS, SCENES, SLOTS,
};
use mpfree_transport::Tick;

/// When a launch happens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum When {
    /// The transport rolls: on the next bar line the playhead plays.
    Cued,
    /// It is stopped: at once, as if started on this bar line (the line of
    /// the bar the position is in).
    Now(Tick),
}

/// What [`Perform::due`] did this time.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Fired {
    /// A scene follow: `Some(Some(row))` launched that row, `Some(None)`
    /// stopped every track.
    pub scene: Option<Option<usize>>,
    /// The tracks a clip follow cued (bit `t` is track `t`, all 128).
    pub clips: u128,
    /// A follow rolled Stay: nothing changed now, it rolls again later.
    pub stayed: bool,
}

impl Fired {
    /// Whether the cues changed (the host compiles again).
    pub fn changed(&self) -> bool {
        self.scene.is_some() || self.clips != 0
    }
}

/// The scene playing: its row, the bar line it started on, and how many
/// times its follow rolled Stay since.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Playing {
    pub row: usize,
    pub start: Tick,
    pub stays: u32,
}

/// The song list playing: its step, and the bar line that step's scene
/// starts on (None while it waits for a line not yet known); while it
/// waits, the step playing out and the line it started on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ListPlaying {
    pub step: usize,
    pub start: Option<Tick>,
    pub out: Option<(usize, Tick)>,
}

impl ListPlaying {
    /// The step sounding at song tick `now`, and the line it started on.
    fn sounding(&self, now: Tick) -> Option<(usize, Tick)> {
        match self.start {
            Some(s) if s <= now => Some((self.step, s)),
            _ => self.out,
        }
    }
}

/// A track's clip follow: the launch it counts for (slot and start) and
/// its Stays since.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ClipCount {
    slot: usize,
    start: Tick,
    stays: u32,
}

/// Scenes and follow actions: see the module notes.
#[derive(Clone, Debug)]
pub struct Perform {
    playing: Option<Playing>,
    /// The row launched and waiting for its bar line.
    cued: Option<usize>,
    /// Each track's mute when its cue starts (a scene's), for
    /// [`Perform::dress`].
    cue_mutes: [Option<bool>; MAX_TRACKS],
    /// Each track's cue line when it isn't the cued clip's own launch
    /// quantise (S4): a scene's cues land together on the project's, and a
    /// follow's on the next bar line, its point.
    cue_lines: [Option<LaunchGrid>; MAX_TRACKS],
    /// Each track's program change from the scene launched last (S5), for
    /// [`Perform::dress`].
    programs: [Option<SceneProgram>; MAX_TRACKS],
    /// The id of the last launch's program changes.
    program_id: u32,
    /// The mutes the last scene launch set for its line on tracks it gave
    /// no cue, until the host takes them ([`Perform::take_line_mutes`]).
    line_mutes: [Option<bool>; MAX_TRACKS],
    /// The line they land on: the scene's cues' line.
    mute_line: LaunchGrid,
    clips: [Option<ClipCount>; MAX_TRACKS],
    /// The song list, while it plays.
    list: Option<ListPlaying>,
    rng: u64,
}

impl Default for Perform {
    fn default() -> Perform {
        Perform::new(0x9E37_79B9_7F4A_7C15)
    }
}

impl Perform {
    /// `seed` picks the rolls for chances, Any and Other.
    pub fn new(seed: u64) -> Perform {
        Perform {
            playing: None,
            cued: None,
            cue_mutes: [None; MAX_TRACKS],
            cue_lines: [None; MAX_TRACKS],
            programs: [None; MAX_TRACKS],
            program_id: 0,
            line_mutes: [None; MAX_TRACKS],
            mute_line: LaunchGrid::Bars(1),
            clips: [None; MAX_TRACKS],
            list: None,
            rng: seed | 1,
        }
    }

    /// The scene playing, if one was launched and has started.
    pub fn playing(&self) -> Option<Playing> {
        self.playing
    }

    /// The row launched and waiting for its bar line.
    pub fn cued(&self) -> Option<usize> {
        self.cued
    }

    /// The song list's step playing, or waiting for its line.
    pub fn list(&self) -> Option<ListPlaying> {
        self.list
    }

    /// Play the song list from step `step`: its scene launches as
    /// [`Perform::launch_scene`] launches it, and the list goes on by
    /// itself (see the module notes). False (nothing done) past the
    /// list's end.
    pub fn play_list(
        &mut self,
        project: &mut Project,
        step: usize,
        when: When,
        launches: &mut Vec<Launch>,
        cues: &mut Vec<Cue>,
    ) -> bool {
        let Some(row) = project.arrangement.list.steps.get(step).map(|s| s.row) else {
            return false;
        };
        if !self.launch_scene(project, row, when, launches, cues) {
            return false;
        }
        // The step playing until now plays out until the line.
        let out = self.list.and_then(|l| match l.start {
            Some(s) => Some((l.step, s)),
            None => l.out,
        });
        self.list = Some(match when {
            When::Now(bar) => ListPlaying {
                step,
                start: Some(bar),
                out: None,
            },
            When::Cued => ListPlaying {
                step,
                start: None,
                out,
            },
        });
        true
    }

    /// Step `at` was put in the list (`inserted`) or taken out: the step
    /// playing keeps its place. Taken out while it plays, the list stops
    /// going on (what plays plays on).
    pub fn list_edited(&mut self, at: usize, inserted: bool) {
        let Some(l) = self.list.as_mut() else {
            return;
        };
        let shift = |k: &mut usize| -> bool {
            if inserted && *k >= at {
                *k += 1;
            } else if !inserted && *k > at {
                *k -= 1;
            } else if !inserted && *k == at {
                return false;
            }
            true
        };
        if !shift(&mut l.step) {
            self.list = None;
            return;
        }
        if let Some((k, s)) = l.out {
            let mut k = k;
            l.out = shift(&mut k).then_some((k, s));
        }
    }

    /// Step `from` moved to `to` in the list: the step playing keeps
    /// playing, and the list goes on from where it is now.
    pub fn list_moved(&mut self, from: usize, to: usize) {
        let Some(l) = self.list.as_mut() else {
            return;
        };
        let moved = |k: usize| {
            if k == from {
                to
            } else if from < k && k <= to {
                k - 1
            } else if to <= k && k < from {
                k + 1
            } else {
                k
            }
        };
        l.step = moved(l.step);
        if let Some((k, s)) = l.out {
            l.out = Some((moved(k), s));
        }
    }

    /// The list stops going on by itself (a scene launched by hand, Stop
    /// all, Back to song); what plays plays on.
    pub fn end_list(&mut self) {
        self.list = None;
    }

    /// Where the list is at song tick `now`, with bars of `bar` ticks:
    /// the step sounding, the pass playing (from 1) and how far through
    /// the step it is (0 to 1). None while it doesn't play, or its first
    /// step waits for its line.
    pub fn list_pass(&self, project: &Project, now: Tick, bar: Tick) -> Option<(usize, u16, f32)> {
        let (step, start) = self.list?.sounding(now)?;
        let s = project.arrangement.list.steps.get(step)?;
        let pass_len = scene_length(project, s.row, bar).max(1);
        let span = pass_len * Tick::from(s.times);
        let into = (now - start).clamp(0, span - 1);
        let pass = (into / pass_len) as u16 + 1;
        Some((step, pass.min(s.times), into as f32 / span as f32))
    }

    /// The list's step waiting for its line at song tick `now`, if one
    /// does.
    pub fn list_waiting(&self, now: Tick) -> Option<usize> {
        let l = self.list?;
        l.start.is_none_or(|s| s > now).then_some(l.step)
    }

    /// What the list plays next and where, for NEXT: the step after the
    /// one playing (None: it stops) and the bar line its step ends on.
    /// None while the list doesn't play or its step waits for its line.
    pub fn list_next(&self, project: &Project, bar: Tick) -> Option<(Option<usize>, Tick)> {
        let l = self.list.filter(|_| self.cued.is_none())?;
        let start = l.start?;
        let list = &project.arrangement.list;
        let s = list.steps.get(l.step)?;
        let end = start + scene_length(project, s.row, bar) * Tick::from(s.times);
        Some((list.after(l.step), end))
    }

    /// Launch row `row`: see the module notes. False (and nothing done)
    /// past the last row.
    pub fn launch_scene(
        &mut self,
        project: &mut Project,
        row: usize,
        when: When,
        launches: &mut Vec<Launch>,
        cues: &mut Vec<Cue>,
    ) -> bool {
        if row >= SCENES {
            return false;
        }
        // The scene's tracks start together, on the project's launch line.
        let line = project.launch;
        // Its mutes for tracks it gives no cue wait for that line too (a
        // launch before the host took the last ones replaces them).
        self.line_mutes = [None; MAX_TRACKS];
        self.mute_line = line;
        let mutes = project
            .scene(row)
            .map(|s| (0..MAX_TRACKS).map(|t| s.mutes_track(t)));
        let mut mute = [None; MAX_TRACKS];
        if let Some(m) = mutes {
            for (t, v) in m.enumerate() {
                mute[t] = v;
            }
        }
        // Its program changes, on the same line (stopped: where the music
        // starts), with a new id.
        self.program_id = self.program_id.wrapping_add(1);
        let scene = project.scene(row);
        for (t, p) in self.programs.iter_mut().enumerate() {
            *p = scene
                .and_then(|s| s.program(t))
                .map(|program| SceneProgram {
                    program,
                    line: matches!(when, When::Cued).then_some(line),
                    id: self.program_id,
                });
        }
        // Its Keep playing tracks (the Arrange refactor's 5.2): an empty
        // cell there leaves the track as it is.
        let keep = scene.map_or(0, |s| s.keep);
        for (t, track) in project.tracks.iter_mut().enumerate() {
            let slot = track.slots[row].as_ref().map(|c| (row, c.length, c.speed));
            if slot.is_none() && t < MAX_TRACKS && keep & (1 << t) != 0 {
                // Keep playing: no cue, and what plays or waits carries
                // on; the scene's mute still comes in on its line.
                match (when, mute[t]) {
                    (When::Now(_), Some(m)) => track.mute = m,
                    (When::Now(_), None) => {}
                    (When::Cued, Some(m)) if line == LaunchGrid::Now => track.mute = m,
                    (When::Cued, m) => self.line_mutes[t] = m,
                }
                continue;
            }
            let busy = launches.iter().any(|l| l.track == t) || cues.iter().any(|c| c.track == t);
            cues.retain(|c| c.track != t);
            match when {
                When::Cued if slot.is_some() || busy => {
                    cues.push(Cue {
                        track: t,
                        slot: slot.map(|s| s.0),
                    });
                    self.cue_mutes[t] = mute[t];
                    self.cue_lines[t] = Some(line);
                }
                When::Cued => {
                    // Nothing to cue, but in song mode the track plays the
                    // song: its mute waits for the line, unless the line
                    // is at once.
                    match mute[t] {
                        Some(m) if line == LaunchGrid::Now => track.mute = m,
                        m => self.line_mutes[t] = m,
                    }
                    self.cue_mutes[t] = None;
                    self.cue_lines[t] = None;
                }
                When::Now(bar) => {
                    launches.retain(|l| l.track != t);
                    if let Some((slot, length, speed)) = slot {
                        launches.push(Launch {
                            track: t,
                            slot,
                            origin: anchored(bar, length, speed),
                            start: bar,
                        });
                    }
                    if let Some(m) = mute[t] {
                        track.mute = m;
                    }
                    self.cue_mutes[t] = None;
                    self.cue_lines[t] = None;
                }
            }
        }
        match when {
            When::Cued => self.cued = Some(row),
            When::Now(bar) => {
                self.cued = None;
                self.playing = Some(Playing {
                    row,
                    start: bar,
                    stays: 0,
                });
            }
        }
        true
    }

    /// A section of a song linked to its list, made from row `row`,
    /// starts on `line` (the Arrange refactor's 4b.4; None: at once, the
    /// transport stopped or the playhead jumped into it). Its scene's
    /// mutes and program changes come in there as a launch of the scene
    /// brings them for the tracks it gives no cue: the mutes wait for the
    /// line ([`Perform::take_line_mutes`]; at once they are set here), and
    /// the program changes go out on it (at once: where the music plays
    /// from), replacing the last ones. Nothing is cued: the song plays the
    /// section's blocks. False (nothing done) past the last row.
    pub fn section_starts(
        &mut self,
        project: &mut Project,
        row: usize,
        line: Option<LaunchGrid>,
    ) -> bool {
        if row >= SCENES {
            return false;
        }
        let scene = project.scene(row);
        let mut mute = [None; MAX_TRACKS];
        if let Some(s) = scene {
            for (t, m) in mute.iter_mut().enumerate() {
                *m = s.mutes_track(t);
            }
        }
        self.program_id = self.program_id.wrapping_add(1);
        for (t, p) in self.programs.iter_mut().enumerate() {
            *p = scene
                .and_then(|s| s.program(t))
                .map(|program| SceneProgram {
                    program,
                    line,
                    id: self.program_id,
                });
        }
        self.line_mutes = [None; MAX_TRACKS];
        match line {
            Some(line) => {
                self.mute_line = line;
                for (t, m) in mute.iter().enumerate().take(project.tracks.len()) {
                    self.line_mutes[t] = *m;
                }
            }
            None => {
                for (track, m) in project.tracks.iter_mut().zip(mute) {
                    if let Some(m) = m {
                        track.mute = m;
                    }
                }
            }
        }
        true
    }

    /// Stop every track (on the next bar line while rolling), and forget
    /// the scene and the song list.
    pub fn stop_all(&mut self, when: When, launches: &mut Vec<Launch>, cues: &mut Vec<Cue>) {
        let mut busy = [false; MAX_TRACKS];
        for t in launches
            .iter()
            .map(|l| l.track)
            .chain(cues.iter().map(|c| c.track))
        {
            if t < MAX_TRACKS {
                busy[t] = true;
            }
        }
        cues.clear();
        match when {
            When::Cued => {
                for (t, _) in busy.iter().enumerate().filter(|b| *b.1) {
                    cues.push(Cue {
                        track: t,
                        slot: None,
                    });
                }
            }
            When::Now(_) => launches.clear(),
        }
        self.cue_mutes = [None; MAX_TRACKS];
        self.cue_lines = [None; MAX_TRACKS];
        self.programs = [None; MAX_TRACKS];
        self.cued = None;
        self.playing = None;
        self.list = None;
    }

    /// The program change the scene launched last sends `track` (S5).
    pub fn program(&self, track: usize) -> Option<SceneProgram> {
        self.programs.get(track).copied().flatten()
    }

    /// The program changes still waiting for their line, and the mutes
    /// for tracks with no cue, land on `line` (a follow's or the song
    /// list's point).
    fn programs_on(&mut self, line: LaunchGrid) {
        for p in self.programs.iter_mut().flatten() {
            if p.line.is_some() {
                p.line = Some(line);
            }
        }
        self.mute_line = line;
    }

    /// The mutes the last scene launch set on tracks it gave no cue (bit
    /// for bit as the scene's: `Some(true)` mutes, `Some(false)` lets the
    /// track be heard), and the line they land on with the scene's cues,
    /// taken: the host puts them in its mute queue, so they land on that
    /// line as TRACK MUTE's do. None when there are none. Call it after
    /// each launch, play of the song list and [`Perform::due`].
    pub fn take_line_mutes(&mut self) -> Option<(LaunchGrid, [Option<bool>; MAX_TRACKS])> {
        if self.line_mutes.iter().all(Option::is_none) {
            return None;
        }
        let mutes = std::mem::replace(&mut self.line_mutes, [None; MAX_TRACKS]);
        Some((self.mute_line, mutes))
    }

    /// The mute a scene's cue on `track` sets when it starts, if any.
    pub fn cue_mute(&self, track: usize) -> Option<bool> {
        self.cue_mutes.get(track).copied().flatten()
    }

    /// The line the cue on `track` lands on where a scene, a follow or a
    /// change of Launch set it; None for the clip's own or the project's.
    pub fn cue_line(&self, track: usize) -> Option<LaunchGrid> {
        self.cue_lines.get(track).copied().flatten()
    }

    /// The cue waiting on `track` keeps landing on `line`, unless a scene
    /// or a follow already set its line (the launch quantise changed while
    /// it waited).
    pub fn hold_line(&mut self, track: usize, line: LaunchGrid) {
        if let Some(g @ None) = self.cue_lines.get_mut(track) {
            *g = Some(line);
        }
    }

    /// A new tap on `track` replaces its cue: that one lands on its own
    /// line (the clip's launch quantise or the project's), not a line the
    /// cue it replaces kept. Returns the line it had, for an undo.
    pub fn forget_line(&mut self, track: usize) -> Option<LaunchGrid> {
        self.cue_lines.get_mut(track).and_then(Option::take)
    }

    /// Put back a line [`Perform::forget_line`] took (the host's change
    /// failed).
    pub fn restore_line(&mut self, track: usize, line: Option<LaunchGrid>) {
        if let Some(g) = self.cue_lines.get_mut(track) {
            *g = line;
        }
    }

    /// Take back what is cued on `track` (the host removed its cue), with
    /// the scene's program change for it.
    pub fn uncue(&mut self, track: usize) {
        if let Some(m) = self.cue_mutes.get_mut(track) {
            *m = None;
        }
        if let Some(g) = self.cue_lines.get_mut(track) {
            *g = None;
        }
        if let Some(p) = self.programs.get_mut(track) {
            *p = None;
        }
    }

    /// Set each cued track's scene mute, the line a scene's or a follow's
    /// cue lands on, and the scene's program changes, on a freshly
    /// compiled view.
    pub fn dress(&self, view: &mut PlayView) {
        for (t, m) in self.cue_mutes.iter().enumerate() {
            if m.is_some() {
                view.set_cue_mute(t, *m);
            }
        }
        for (t, g) in self.cue_lines.iter().enumerate() {
            if let Some(g) = g {
                view.set_cue_line(t, *g);
            }
        }
        for (t, p) in self.programs.iter().enumerate() {
            if p.is_some() {
                view.set_scene_program(t, *p);
            }
        }
    }

    /// A cue the player started: call it for each [`Started`], before
    /// applying it to the launches and cues.
    pub fn started(&mut self, project: &mut Project, st: &Started) {
        if let (Some(m), Some(track)) = (st.mute, project.tracks.get_mut(st.track)) {
            track.mute = m;
        }
        if let Some(m) = self.cue_mutes.get_mut(st.track) {
            *m = None;
        }
        if let Some(g) = self.cue_lines.get_mut(st.track) {
            *g = None;
        }
        if let Some(row) = self.cued.take() {
            self.playing = Some(Playing {
                row,
                start: st.at,
                stays: 0,
            });
            // The list's step counts from where its scene started.
            if let Some(l) = self.list.as_mut() {
                l.start = Some(st.at);
            }
        }
    }

    /// The scene playing's follow action while no scene is cued, for
    /// Perform's NEXT (sequencer S4): its row, its point (the bar line it
    /// lands on, with bars of `bar` ticks) and the follow. It is cued a
    /// bar before the point ([`Perform::due`]), and the cue shows from then.
    pub fn scene_follow(&self, project: &Project, bar: Tick) -> Option<(usize, Tick, Follow)> {
        let p = self
            .playing
            .filter(|_| self.cued.is_none() && self.list.is_none())?;
        let f = project.scene(p.row)?.follow?;
        Some((p.row, point(p.start, f, p.stays, bar.max(1)), f))
    }

    /// The follow actions due at song tick `now` (the next tick the
    /// playhead plays), with bars of `bar` ticks: see the module notes.
    /// Call it after each period while the transport rolls; the host
    /// compiles again when [`Fired::changed`].
    pub fn due(
        &mut self,
        project: &mut Project,
        launches: &mut Vec<Launch>,
        cues: &mut Vec<Cue>,
        now: Tick,
        bar: Tick,
    ) -> Fired {
        let mut fired = Fired::default();
        let bar = bar.max(1);
        // The song list in place of the scenes' follow actions.
        if self.list.is_some() {
            self.list_due(project, launches, cues, now, bar, &mut fired);
        }
        // The scene's follow first: it cues every track.
        if let (Some(p), None, None) = (self.playing, self.cued, self.list) {
            if let Some(f) = project.scene(p.row).and_then(|s| s.follow) {
                if now > point(p.start, f, p.stays, bar) - bar {
                    // The rows of its bank (Arrange 1.2: the grid's 16
                    // until scene banks come).
                    let rows = bank(p.row);
                    let mut filled = [false; SLOTS];
                    for (r, x) in filled.iter_mut().enumerate() {
                        *x = project.row_filled(rows.start + r);
                    }
                    let action = self.pick(&f);
                    let roll = self.roll();
                    let at = p.row - rows.start;
                    let target = follow_target(action, at, &filled, roll).map(|r| rows.start + r);
                    match (action, target) {
                        (FollowAction::Stay, _) => {
                            self.playing = Some(Playing {
                                stays: p.stays + 1,
                                ..p
                            });
                            fired.stayed = true;
                        }
                        (_, Some(row)) => {
                            self.launch_scene(project, row, When::Cued, launches, cues);
                            fired.scene = Some(Some(row));
                        }
                        (_, None) => {
                            self.stop_all(When::Cued, launches, cues);
                            fired.scene = Some(None);
                        }
                    }
                    // A follow lands on its point, the next bar line,
                    // whatever the launch quantise.
                    if fired.scene.is_some() {
                        for c in cues.iter() {
                            if let Some(g) = self.cue_lines.get_mut(c.track) {
                                *g = Some(LaunchGrid::Bars(1));
                            }
                        }
                        self.programs_on(LaunchGrid::Bars(1));
                    }
                }
            }
        }
        for i in 0..launches.len() {
            let l = launches[i];
            if l.track >= MAX_TRACKS || cues.iter().any(|c| c.track == l.track) {
                continue;
            }
            let Some(track) = project.tracks.get(l.track) else {
                continue;
            };
            let Some(clip) = track.slots.get(l.slot).and_then(Option::as_ref) else {
                self.clips[l.track] = None;
                continue;
            };
            // A one-shot: its track stops where the pass it started in
            // ends, cued in the bar before as a follow is; its follow
            // waits.
            if clip.once {
                self.clips[l.track] = None;
                let end = once_end(&l, clip);
                if now > end - bar {
                    cues.push(Cue {
                        track: l.track,
                        slot: None,
                    });
                    self.cue_lines[l.track] = Some(LaunchGrid::At(end));
                    fired.clips |= 1u128 << l.track;
                }
                continue;
            }
            let Some(f) = clip.follow else {
                self.clips[l.track] = None;
                continue;
            };
            let count = match self.clips[l.track] {
                Some(c) if c.slot == l.slot && c.start == l.start => c,
                _ => ClipCount {
                    slot: l.slot,
                    start: l.start,
                    stays: 0,
                },
            };
            self.clips[l.track] = Some(count);
            if now <= point(l.start, f, count.stays, bar) - bar {
                continue;
            }
            // The clips of its bank (Arrange 1.2).
            let rows = bank(l.slot);
            let mut filled = [false; SLOTS];
            for (s, x) in filled.iter_mut().enumerate() {
                *x = track.slots[rows.start + s].is_some();
            }
            let action = self.pick(&f);
            let roll = self.roll();
            let at = l.slot - rows.start;
            let target = follow_target(action, at, &filled, roll).map(|s| rows.start + s);
            match (action, target) {
                (FollowAction::Stay, _) => {
                    self.clips[l.track] = Some(ClipCount {
                        stays: count.stays + 1,
                        ..count
                    });
                    fired.stayed = true;
                }
                (_, slot) => {
                    cues.push(Cue {
                        track: l.track,
                        slot,
                    });
                    self.cue_lines[l.track] = Some(LaunchGrid::Bars(1));
                    fired.clips |= 1u128 << l.track;
                }
            }
        }
        fired
    }

    /// The song list's next step, in the bar before the one playing ends:
    /// its scene cued for that bar line (the next bar line, if the check
    /// comes late), or every track stopped after the last step.
    fn list_due(
        &mut self,
        project: &mut Project,
        launches: &mut Vec<Launch>,
        cues: &mut Vec<Cue>,
        now: Tick,
        bar: Tick,
        fired: &mut Fired,
    ) {
        let Some(l) = self.list else {
            return;
        };
        let (Some(start), None) = (l.start, self.cued) else {
            return;
        };
        let Some(s) = project.arrangement.list.steps.get(l.step).copied() else {
            // The list lost the step (an edit): it ends.
            self.list = None;
            return;
        };
        let end = start + scene_length(project, s.row, bar) * Tick::from(s.times);
        if now <= end - bar {
            return;
        }
        // Late (a locate past it): the next bar line.
        let line = if now <= end {
            end
        } else {
            (now + bar - 1) / bar * bar
        };
        match project.arrangement.list.after(l.step) {
            Some(next) => {
                let row = project.arrangement.list.steps[next].row;
                self.launch_scene(project, row, When::Cued, launches, cues);
                self.list = Some(ListPlaying {
                    step: next,
                    start: Some(line),
                    out: Some((l.step, start)),
                });
                // An empty row with nothing playing has no cue to start:
                // it is a silent step from its line.
                if cues.is_empty() {
                    self.cued = None;
                    self.playing = Some(Playing {
                        row,
                        start: line,
                        stays: 0,
                    });
                }
                fired.scene = Some(Some(row));
            }
            None => {
                self.stop_all(When::Cued, launches, cues);
                self.list = None;
                fired.scene = Some(None);
            }
        }
        // It lands on its line, the next bar line, whatever the launch
        // quantise.
        for c in cues.iter() {
            if let Some(g) = self.cue_lines.get_mut(c.track) {
                *g = Some(LaunchGrid::Bars(1));
            }
        }
        self.programs_on(LaunchGrid::Bars(1));
    }

    /// `f.action` with a chance of `f.chance` %, else `f.other`.
    fn pick(&mut self, f: &Follow) -> FollowAction {
        if f.chance >= 100 || self.roll() % 100 < u64::from(f.chance) {
            f.action
        } else {
            f.other
        }
    }

    /// xorshift64*.
    fn roll(&mut self) -> u64 {
        let mut x = self.rng;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.rng = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11
    }
}

/// Where a one-shot launch stops ([`mpfree_seq::Clip::once`]): the end of
/// the pass of `clip` that its start is in, its passes counted from its
/// origin (a legato launch's is earlier than its start).
pub fn once_end(l: &Launch, clip: &Clip) -> Tick {
    let pass = clip.speed.to_song(clip.length as f64).max(1.0);
    let k = ((l.start - l.origin) as f64 / pass).floor().max(0.0);
    l.origin + ((k + 1.0) * pass).round() as Tick
}

/// Where a follow from `start` comes due after `stays` Stays.
fn point(start: Tick, f: Follow, stays: u32, bar: Tick) -> Tick {
    start + i64::from(f.bars) * bar * (i64::from(stays) + 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_seq::Track;

    const BAR: Tick = 3840;

    /// Two tracks, rows 0 and 1 full, row 2 only on track 0.
    fn project() -> Project {
        let mut p = Project::new();
        for name in ["drums", "bass"] {
            let mut t = Track::new(name, 0);
            t.slots[0] = Some(Clip::new("a", BAR));
            t.slots[1] = Some(Clip::new("b", 2 * BAR));
            p.tracks.push(t);
        }
        p.tracks[0].slots[2] = Some(Clip::new("c", BAR));
        p
    }

    /// A launch's program changes: on the project's launch line while
    /// rolling, with none while stopped; the next launch's replace them
    /// with a new id; a follow's land on the next bar line; Stop all and
    /// an uncue forget them.
    #[test]
    fn a_scenes_program_changes_go_with_its_launch() {
        let mut p = project();
        let pc = |program| {
            Some(mpfree_seq::ClipProgram {
                program,
                bank: None,
            })
        };
        p.scene_mut(1).unwrap().set_program(1, pc(7)).unwrap();
        p.scene_mut(1).unwrap().set_program(5, pc(9)).unwrap();
        p.scene_mut(1).unwrap().follow = Some(Follow::after(1, FollowAction::Next));
        p.scene_mut(2).unwrap().set_program(0, pc(3)).unwrap();
        p.launch = LaunchGrid::Bars(2);
        let mut perf = Perform::default();
        let (mut launches, mut cues) = (vec![], vec![]);
        assert!(perf.launch_scene(&mut p, 1, When::Now(0), &mut launches, &mut cues));
        let now = perf.program(1).unwrap();
        assert_eq!((now.program, now.line), (pc(7).unwrap(), None));
        assert_eq!(
            perf.program(5).map(|x| x.program),
            pc(9),
            "past the tracks too"
        );
        assert_eq!(perf.program(0), None);
        assert!(perf.launch_scene(&mut p, 1, When::Cued, &mut launches, &mut cues));
        let cued = perf.program(1).unwrap();
        assert_eq!(cued.line, Some(LaunchGrid::Bars(2)));
        assert!(cued.id > now.id);
        // The view of a track that exists gets it.
        let mut v = PlayView::compile_cued(&p, &launches, &cues).unwrap();
        perf.dress(&mut v);
        assert_eq!(v.scene_program(1), Some(cued));
        assert_eq!(v.scene_program(0), None);
        // Scene 1's follow launches scene 2 on the next bar line.
        let st = Started {
            track: 0,
            slot: Some(1),
            at: 2 * BAR,
            origin: 2 * BAR,
            mute: None,
            from: 2 * BAR,
            rev: 0,
        };
        perf.started(&mut p, &st);
        st.apply(&mut launches, &mut cues);
        let fired = perf.due(&mut p, &mut launches, &mut cues, 3 * BAR - 10, BAR);
        assert_eq!(fired.scene, Some(Some(2)));
        assert_eq!(perf.program(1), None, "scene 2 has none for track 1");
        let f = perf.program(0).unwrap();
        assert_eq!(
            (f.program, f.line),
            (pc(3).unwrap(), Some(LaunchGrid::Bars(1)))
        );
        perf.uncue(0);
        assert_eq!(perf.program(0), None);
        assert!(perf.launch_scene(&mut p, 2, When::Cued, &mut launches, &mut cues));
        assert!(perf.program(0).is_some());
        perf.stop_all(When::Cued, &mut launches, &mut cues);
        assert_eq!(perf.program(0), None);
    }

    #[test]
    fn a_scene_cues_every_track_and_stops_a_playing_one_with_an_empty_cell() {
        let mut p = project();
        let mut perf = Perform::default();
        let mut launches = vec![
            Launch {
                track: 0,
                slot: 0,
                origin: 0,
                start: 0,
            },
            Launch {
                track: 1,
                slot: 0,
                origin: 0,
                start: 0,
            },
        ];
        let mut cues = vec![];
        assert!(perf.launch_scene(&mut p, 2, When::Cued, &mut launches, &mut cues));
        assert_eq!(
            cues,
            vec![
                Cue {
                    track: 0,
                    slot: Some(2)
                },
                Cue {
                    track: 1,
                    slot: None
                }
            ]
        );
        assert_eq!(perf.cued(), Some(2));
        assert!(!perf.launch_scene(&mut p, SCENES, When::Cued, &mut launches, &mut cues));
    }

    /// The Arrange refactor's 5.2 (D4's empty cells): a scene's Keep
    /// playing track with an empty cell gets no cue, so what it plays,
    /// and a cue already waiting on it, carry on through the scene
    /// change; its mute still waits for the scene's line. A cell with a
    /// clip launches as ever, and a Stop track still stops.
    #[test]
    fn a_keep_playing_track_runs_on_through_a_scene_change() {
        let mut p = project();
        let sc = p.scene_mut(2).unwrap();
        sc.set_keep(1, true).unwrap();
        sc.mutes = Some(0b10);
        let mut perf = Perform::default();
        let playing = |track| Launch {
            track,
            slot: 0,
            origin: 0,
            start: 0,
        };
        let mut launches = vec![playing(0), playing(1)];
        let mut cues = vec![];
        assert!(perf.launch_scene(&mut p, 2, When::Cued, &mut launches, &mut cues));
        assert_eq!(
            cues,
            vec![Cue {
                track: 0,
                slot: Some(2)
            }],
            "track 1 keeps playing: no stop cue"
        );
        assert_eq!(launches, vec![playing(0), playing(1)]);
        let (_, mutes) = perf.take_line_mutes().unwrap();
        assert_eq!(mutes[1], Some(true), "its mute on the line");
        // A cue already waiting on a Keep playing track stays.
        let mut cues = vec![Cue {
            track: 1,
            slot: Some(1),
        }];
        perf.launch_scene(&mut p, 2, When::Cued, &mut launches, &mut cues);
        assert!(cues.contains(&Cue {
            track: 1,
            slot: Some(1)
        }));
        // Stopped, at once: the track's launch stays, and its mute is set.
        let mut cues = vec![];
        perf.launch_scene(&mut p, 2, When::Now(BAR), &mut launches, &mut cues);
        assert!(launches.contains(&playing(1)));
        assert!(launches.iter().any(|l| l.track == 0 && l.slot == 2));
        assert!(p.tracks[1].mute);
        // Back to Stop: the empty cell stops it, as before.
        p.scene_mut(2).unwrap().set_keep(1, false).unwrap();
        let mut launches = vec![playing(0), playing(1)];
        let mut cues = vec![];
        perf.launch_scene(&mut p, 2, When::Cued, &mut launches, &mut cues);
        assert!(cues.contains(&Cue {
            track: 1,
            slot: None
        }));
    }

    #[test]
    fn stopped_a_scene_launches_at_once_with_its_mutes() {
        let mut p = project();
        p.scene_mut(1).unwrap().mutes = Some(0b10);
        let mut perf = Perform::default();
        let (mut launches, mut cues) = (vec![], vec![]);
        perf.launch_scene(&mut p, 1, When::Now(BAR), &mut launches, &mut cues);
        assert_eq!(launches.len(), 2);
        assert!(launches.iter().all(|l| l.slot == 1 && l.start == BAR));
        assert!(cues.is_empty());
        assert_eq!((p.tracks[0].mute, p.tracks[1].mute), (false, true));
        assert_eq!(
            perf.playing(),
            Some(Playing {
                row: 1,
                start: BAR,
                stays: 0
            })
        );
    }

    #[test]
    fn a_scene_mutes_tracks_past_16_up_to_the_128th() {
        let mut p = project();
        while p.tracks.len() < MAX_TRACKS {
            let t = p.tracks.len();
            p.tracks.push(mpfree_seq::Track::new("t", t as u16));
        }
        p.scene_mut(1).unwrap().mutes = Some(1 << 127 | 1 << 40);
        let mut perf = Perform::default();
        let (mut launches, mut cues) = (vec![], vec![]);
        perf.launch_scene(&mut p, 1, When::Now(BAR), &mut launches, &mut cues);
        let muted: Vec<usize> = (0..MAX_TRACKS).filter(|&t| p.tracks[t].mute).collect();
        assert_eq!(muted, vec![40, 127]);
    }

    /// A track the scene gives no cue (nothing playing or cued, an empty
    /// cell) still plays the song in song mode: its mute waits for the
    /// scene's line, for the host's mute queue, instead of landing at the
    /// tap. A follow's waits for the next bar line; Launch at once and a
    /// launch while stopped set it at once.
    #[test]
    fn an_idle_tracks_mute_waits_for_the_scenes_line() {
        let mut p = project();
        p.scene_mut(2).unwrap().mutes = Some(0b10);
        p.launch = LaunchGrid::Bars(2);
        let mut perf = Perform::default();
        let (mut launches, mut cues) = (vec![], vec![]);
        perf.launch_scene(&mut p, 2, When::Cued, &mut launches, &mut cues);
        assert_eq!(
            cues,
            vec![Cue {
                track: 0,
                slot: Some(2)
            }]
        );
        assert!(!p.tracks[1].mute, "not at the tap");
        let (line, mutes) = perf.take_line_mutes().unwrap();
        assert_eq!(line, LaunchGrid::Bars(2), "the scene's line");
        assert_eq!((mutes[0], mutes[1]), (None, Some(true)), "track 0 is cued");
        assert!(mutes[2..].iter().all(Option::is_none));
        assert_eq!(perf.take_line_mutes(), None, "taken once");
        // A second launch before the host takes them replaces them.
        p.scene_mut(1).unwrap().mutes = Some(0);
        p.scene_mut(3).unwrap().mutes = Some(0b100);
        p.tracks.push(mpfree_seq::Track::new("t", 2));
        p.tracks[0].slots[3] = Some(Clip::new("d", BAR));
        let (mut launches, mut cues) = (vec![], vec![]);
        perf.launch_scene(&mut p, 2, When::Cued, &mut launches, &mut cues);
        perf.launch_scene(&mut p, 3, When::Cued, &mut launches, &mut cues);
        let (_, mutes) = perf.take_line_mutes().unwrap();
        assert_eq!((mutes[1], mutes[2]), (Some(false), Some(true)));
        // A scene follow's land on its point, the next bar line.
        p.scene_mut(2).unwrap().follow = Some(Follow::after(1, FollowAction::Next));
        let (mut launches, mut cues) = (vec![], vec![]);
        perf.launch_scene(&mut p, 2, When::Now(0), &mut launches, &mut cues);
        assert_eq!(perf.take_line_mutes(), None, "stopped: at once");
        assert!(p.tracks[1].mute);
        let fired = perf.due(&mut p, &mut launches, &mut cues, BAR - 10, BAR);
        assert_eq!(fired.scene, Some(Some(3)));
        let (line, mutes) = perf.take_line_mutes().unwrap();
        assert_eq!(
            (line, mutes[1], mutes[2]),
            (LaunchGrid::Bars(1), Some(false), Some(true))
        );
        // Launch at once: the cues start at once, and so do the mutes.
        (p.tracks[1].mute, p.tracks[2].mute) = (false, true);
        p.launch = LaunchGrid::Now;
        perf.launch_scene(&mut p, 2, When::Cued, &mut launches, &mut cues);
        assert_eq!(perf.take_line_mutes(), None);
        assert!(p.tracks[1].mute && !p.tracks[2].mute);
    }

    /// A linked song's section starting (the Arrange refactor's 4b.4):
    /// its scene's mutes wait for its line for every track, cued or not
    /// (nothing is cued: the song plays), its program changes go out on
    /// the line and replace the last; with no line the mutes land at once.
    #[test]
    fn a_sections_mutes_and_programs_come_in_on_its_line() {
        use mpfree_seq::{ClipProgram, TrackProgram};
        let mut p = project();
        p.tracks[0].mute = true;
        let s = p.scene_mut(1).unwrap();
        s.mutes = Some(0b10);
        s.programs = vec![TrackProgram {
            track: 1,
            program: ClipProgram {
                program: 3,
                bank: None,
            },
        }];
        let mut perf = Perform::default();
        let line = LaunchGrid::At(4 * BAR);
        assert!(perf.section_starts(&mut p, 1, Some(line)));
        let (at, mutes) = perf.take_line_mutes().unwrap();
        assert_eq!((at, mutes[0], mutes[1]), (line, Some(false), Some(true)));
        assert!(mutes[p.tracks.len()..].iter().all(Option::is_none));
        assert!(p.tracks[0].mute && !p.tracks[1].mute, "not yet");
        assert_eq!(perf.programs[0], None);
        let sent = perf.programs[1].unwrap();
        assert_eq!((sent.program.program, sent.line), (3, Some(line)));
        assert_eq!((perf.cued, perf.playing.is_none()), (None, true));
        // Row 1 has no mutes or programs: the mutes stay, the programs go.
        assert!(perf.section_starts(&mut p, 0, None));
        assert_eq!(perf.take_line_mutes(), None);
        assert!(p.tracks[0].mute && !p.tracks[1].mute);
        assert_eq!(perf.programs[1], None);
        // Stopped: row 2's mutes at once.
        assert!(perf.section_starts(&mut p, 1, None));
        assert!(!p.tracks[0].mute && p.tracks[1].mute);
        assert!(perf.programs[1].unwrap().id > sent.id);
        assert!(!perf.section_starts(&mut p, SCENES, None));
    }

    #[test]
    fn a_scene_follow_fires_in_the_bar_before_its_point() {
        let mut p = project();
        p.scene_mut(0).unwrap().follow = Some(Follow::after(2, FollowAction::Next));
        let mut perf = Perform::default();
        let (mut launches, mut cues) = (vec![], vec![]);
        perf.launch_scene(&mut p, 0, When::Cued, &mut launches, &mut cues);
        let st = Started {
            track: 0,
            slot: Some(0),
            at: BAR,
            origin: BAR,
            mute: None,
            from: BAR,
            rev: 0,
        };
        perf.started(&mut p, &st);
        cues.clear();
        // NEXT's reading (S4): the scene's follow lands on bar 3.
        let next = perf.scene_follow(&p, BAR);
        assert_eq!(
            next,
            Some((0, 3 * BAR, Follow::after(2, FollowAction::Next)))
        );
        // Due at bar 3 (two bars from bar 1): queued once past bar 2.
        for now in [BAR, 2 * BAR] {
            let f = perf.due(&mut p, &mut launches, &mut cues, now, BAR);
            assert_eq!(f, Fired::default(), "{now}");
        }
        let f = perf.due(&mut p, &mut launches, &mut cues, 2 * BAR + 1, BAR);
        assert_eq!(f.scene, Some(Some(1)));
        assert_eq!(cues.len(), 2);
        assert_eq!(perf.cued(), Some(1));
        // Cued, the cue is what comes next.
        assert_eq!(perf.scene_follow(&p, BAR), None);
        // Waiting for its bar line, it does not fire again.
        let f = perf.due(&mut p, &mut launches, &mut cues, 2 * BAR + 2, BAR);
        assert_eq!(f, Fired::default());
    }

    /// The song list (S4): a step plays its scene its times, the next
    /// step's scene is cued in the bar before it ends, a scene can come
    /// twice, the scenes' own follows wait while it plays, and after the
    /// last step every track stops (or the first plays again).
    #[test]
    fn the_song_list_plays_its_steps_in_order_then_stops() {
        use mpfree_seq::ListStep;
        let mut p = project();
        // Row 0 is a bar long, row 1 two bars; row 0's own follow waits.
        p.scene_mut(0).unwrap().follow = Some(Follow::after(1, FollowAction::Last));
        p.arrangement.list.steps = vec![
            ListStep { row: 0, times: 2 },
            ListStep { row: 1, times: 1 },
            ListStep { row: 0, times: 1 },
        ];
        let mut perf = Perform::default();
        let (mut launches, mut cues) = (vec![], vec![]);
        assert!(!perf.play_list(&mut p, 3, When::Cued, &mut launches, &mut cues));
        assert!(perf.play_list(&mut p, 0, When::Cued, &mut launches, &mut cues));
        assert_eq!(
            perf.list(),
            Some(ListPlaying {
                step: 0,
                start: None,
                out: None
            })
        );
        assert_eq!(perf.list_pass(&p, 0, BAR), None, "waiting for its line");
        assert_eq!(perf.list_waiting(0), Some(0));
        let started = |perf: &mut Perform, p: &mut Project, at: Tick, slot: usize| {
            perf.started(
                p,
                &Started {
                    track: 0,
                    slot: Some(slot),
                    at,
                    origin: at,
                    mute: None,
                    from: at,
                    rev: 0,
                },
            );
        };
        started(&mut perf, &mut p, BAR, 0);
        cues.clear();
        assert_eq!(perf.scene_follow(&p, BAR), None, "the list in its place");
        assert_eq!(perf.list_next(&p, BAR), Some((Some(1), 3 * BAR)));
        assert_eq!(
            perf.list_pass(&p, 2 * BAR + BAR / 2, BAR),
            Some((0, 2, 0.75))
        );
        // Two passes of a bar from bar 1: step 2 is cued in bar 2.
        for now in [BAR, 2 * BAR] {
            let f = perf.due(&mut p, &mut launches, &mut cues, now, BAR);
            assert_eq!(f, Fired::default(), "{now}");
        }
        let f = perf.due(&mut p, &mut launches, &mut cues, 2 * BAR + 1, BAR);
        assert_eq!(f.scene, Some(Some(1)));
        assert_eq!(perf.cued(), Some(1));
        assert_eq!(perf.cue_line(0), Some(LaunchGrid::Bars(1)));
        assert_eq!(perf.list().unwrap().step, 1);
        // Step 1 plays out its last pass while step 2 waits for bar 3.
        assert_eq!(perf.list_pass(&p, 2 * BAR + 1, BAR).map(|x| x.0), Some(0));
        assert_eq!(perf.list_waiting(2 * BAR + 1), Some(1));
        started(&mut perf, &mut p, 3 * BAR, 1);
        assert_eq!(perf.list_waiting(3 * BAR), None);
        assert_eq!(perf.list_pass(&p, 3 * BAR, BAR), Some((1, 1, 0.0)));
        cues.clear();
        // Row 1 (two bars) from bar 3, then row 0 again from bar 5.
        assert_eq!(
            perf.due(&mut p, &mut launches, &mut cues, 4 * BAR, BAR),
            Fired::default()
        );
        let f = perf.due(&mut p, &mut launches, &mut cues, 4 * BAR + 1, BAR);
        assert_eq!(f.scene, Some(Some(0)));
        started(&mut perf, &mut p, 5 * BAR, 0);
        cues.clear();
        assert_eq!(perf.list_next(&p, BAR), Some((None, 6 * BAR)));
        // After the last: every track stops on bar 6, and the list ends.
        launches = vec![Launch {
            track: 0,
            slot: 0,
            origin: 5 * BAR,
            start: 5 * BAR,
        }];
        let f = perf.due(&mut p, &mut launches, &mut cues, 5 * BAR + 1, BAR);
        assert_eq!(f.scene, Some(None));
        assert_eq!(
            cues,
            vec![Cue {
                track: 0,
                slot: None
            }]
        );
        assert_eq!(perf.list(), None);
        // Round: the first step again; and end_list leaves the scene.
        p.arrangement.list.looped = true;
        cues.clear();
        perf.play_list(&mut p, 2, When::Now(0), &mut launches, &mut cues);
        let f = perf.due(&mut p, &mut launches, &mut cues, 1, BAR);
        assert_eq!(f.scene, Some(Some(0)));
        assert_eq!(perf.list().unwrap().step, 0);
        // A step moved (its grip, Arrange 4b): the one playing keeps
        // playing wherever it goes, and one moved past it moves it.
        for (from, to, now) in [(0, 2, 2), (1, 0, 2), (2, 0, 0), (1, 0, 1), (1, 0, 0)] {
            perf.list_moved(from, to);
            assert_eq!(perf.list().unwrap().step, now, "{from} to {to}");
        }
        // A step put in before the one playing moves it on; the one
        // playing taken out ends the list.
        perf.list_edited(0, true);
        assert_eq!(perf.list().unwrap().step, 1);
        perf.list_edited(0, false);
        assert_eq!(perf.list().unwrap().step, 0);
        perf.list_edited(0, false);
        assert_eq!(perf.list(), None);
        perf.play_list(&mut p, 1, When::Now(0), &mut launches, &mut cues);
        perf.end_list();
        assert_eq!(perf.list(), None);
    }

    /// Launch quantise (S4): a scene's cues land together on the
    /// project's line, a follow's on its point (the next bar line), and a
    /// line kept when Launch changes stays only where none was set.
    #[test]
    fn scene_cues_take_the_projects_line_and_follows_the_bar() {
        let mut p = project();
        p.launch = LaunchGrid::Bars(4);
        p.tracks[1].slots[0].as_mut().unwrap().launch = Some(LaunchGrid::Now);
        p.scene_mut(0).unwrap().follow = Some(Follow::after(2, FollowAction::Next));
        let mut perf = Perform::default();
        let (mut launches, mut cues) = (vec![], vec![]);
        perf.launch_scene(&mut p, 0, When::Cued, &mut launches, &mut cues);
        // Both tracks, the one whose clip has its own line too.
        assert_eq!(perf.cue_line(0), Some(LaunchGrid::Bars(4)));
        assert_eq!(perf.cue_line(1), Some(LaunchGrid::Bars(4)));
        perf.hold_line(1, LaunchGrid::Ticks(240));
        assert_eq!(perf.cue_line(1), Some(LaunchGrid::Bars(4)), "set already");
        // Started: the lines go.
        for track in 0..2 {
            let st = Started {
                track,
                slot: Some(0),
                at: 4 * BAR,
                origin: 4 * BAR,
                mute: None,
                from: 4 * BAR,
                rev: 0,
            };
            perf.started(&mut p, &st);
            st.apply(&mut launches, &mut cues);
        }
        assert_eq!(perf.cue_line(0), None);
        perf.hold_line(0, LaunchGrid::Ticks(240));
        assert_eq!(perf.cue_line(0), Some(LaunchGrid::Ticks(240)));
        perf.uncue(0);
        assert_eq!(perf.cue_line(0), None);
        // The scene's follow, two bars on: its cues land on the bar.
        let f = perf.due(&mut p, &mut launches, &mut cues, 5 * BAR + 1, BAR);
        assert_eq!(f.scene, Some(Some(1)));
        assert_eq!(perf.cue_line(0), Some(LaunchGrid::Bars(1)));
        assert_eq!(perf.cue_line(1), Some(LaunchGrid::Bars(1)));
    }

    #[test]
    fn a_clip_follow_cues_its_track_and_a_cue_waiting_wins() {
        let mut p = project();
        p.tracks[0].slots[0].as_mut().unwrap().follow = Some(Follow::after(1, FollowAction::Next));
        let mut perf = Perform::default();
        let mut launches = vec![Launch {
            track: 0,
            slot: 0,
            origin: 0,
            start: 0,
        }];
        let mut cues = vec![Cue {
            track: 0,
            slot: None,
        }];
        let f = perf.due(&mut p, &mut launches, &mut cues, 10, BAR);
        assert_eq!(f, Fired::default(), "the stop cued wins");
        cues.clear();
        let f = perf.due(&mut p, &mut launches, &mut cues, 10, BAR);
        assert_eq!(f.clips, 1);
        assert_eq!(
            cues,
            vec![Cue {
                track: 0,
                slot: Some(1)
            }]
        );
    }

    /// A one-shot clip (S4): its stop is cued in the bar before the end
    /// of the pass it started in, on that tick; a legato launch's pass
    /// counts from its origin; its follow waits; a cue waiting wins.
    #[test]
    fn a_one_shot_clip_cues_its_stop_at_the_end_of_its_pass() {
        let mut p = project();
        {
            let c = p.tracks[1].slots[1].as_mut().unwrap();
            c.once = true;
            c.follow = Some(Follow::after(1, FollowAction::Next));
        }
        let mut perf = Perform::default();
        let launch = |origin, start| Launch {
            track: 1,
            slot: 1,
            origin,
            start,
        };
        let mut launches = vec![launch(BAR, BAR)];
        let mut cues = vec![];
        // Two bars long from bar 2: nothing in its first bar.
        let f = perf.due(&mut p, &mut launches, &mut cues, 2 * BAR, BAR);
        assert_eq!(f, Fired::default(), "its follow waits");
        let f = perf.due(&mut p, &mut launches, &mut cues, 2 * BAR + 1, BAR);
        assert_eq!(f.clips, 0b10);
        assert_eq!(
            cues,
            vec![Cue {
                track: 1,
                slot: None
            }]
        );
        assert_eq!(perf.cue_line(1), Some(LaunchGrid::At(3 * BAR)));
        assert_eq!(LaunchGrid::At(3 * BAR).next(2 * BAR + 1, BAR), 3 * BAR);
        assert_eq!(LaunchGrid::At(3 * BAR).next(4 * BAR, BAR), 4 * BAR);
        // The stop waiting wins: nothing more.
        let f = perf.due(&mut p, &mut launches, &mut cues, 2 * BAR + 2, BAR);
        assert!(!f.changed());
        // Legato, taking over a pass at its second bar: it ends with that
        // pass.
        let l = launch(0, BAR);
        assert_eq!(once_end(&l, p.clip(1, 1).unwrap()), 2 * BAR);
        // At half speed a pass is twice as long.
        p.tracks[1].slots[1].as_mut().unwrap().speed = mpfree_seq::Speed::new(1, 2).unwrap();
        assert_eq!(once_end(&launch(0, 0), p.clip(1, 1).unwrap()), 4 * BAR);
    }

    #[test]
    fn stay_rolls_again_a_follow_later() {
        let mut p = project();
        p.tracks[1].slots[1].as_mut().unwrap().follow = Some(Follow::after(2, FollowAction::Stay));
        let mut perf = Perform::default();
        let mut launches = vec![Launch {
            track: 1,
            slot: 1,
            origin: 0,
            start: 0,
        }];
        let mut cues = vec![];
        let f = perf.due(&mut p, &mut launches, &mut cues, BAR + 1, BAR);
        assert!(f.stayed && !f.changed());
        let f = perf.due(&mut p, &mut launches, &mut cues, BAR + 2, BAR);
        assert!(!f.stayed, "the next roll is two bars on");
        let f = perf.due(&mut p, &mut launches, &mut cues, 3 * BAR + 1, BAR);
        assert!(f.stayed);
        // A new start counts afresh.
        launches[0].start = 4 * BAR;
        let f = perf.due(&mut p, &mut launches, &mut cues, 5 * BAR, BAR);
        assert!(!f.stayed);
    }

    #[test]
    fn chances_pick_both_ways_and_stop_all_stops_everything() {
        let mut p = project();
        p.tracks[0].slots[0].as_mut().unwrap().follow = Some(Follow {
            bars: 1,
            action: FollowAction::Next,
            other: FollowAction::Stop,
            chance: 50,
        });
        let mut perf = Perform::new(7);
        let (mut next, mut stop) = (0, 0);
        for _ in 0..200 {
            let mut launches = vec![Launch {
                track: 0,
                slot: 0,
                origin: 0,
                start: 0,
            }];
            let mut cues = vec![];
            perf.due(&mut p, &mut launches, &mut cues, 1, BAR);
            match cues[0].slot {
                Some(1) => next += 1,
                None => stop += 1,
                s => panic!("{s:?}"),
            }
        }
        assert!(next > 60 && stop > 60, "{next} {stop}");
        let mut launches = vec![Launch {
            track: 1,
            slot: 0,
            origin: 0,
            start: 0,
        }];
        let mut cues = vec![Cue {
            track: 0,
            slot: Some(2),
        }];
        perf.stop_all(When::Cued, &mut launches, &mut cues);
        assert_eq!(cues.len(), 2);
        assert!(cues.iter().all(|c| c.slot.is_none()));
        perf.stop_all(When::Now(0), &mut launches, &mut cues);
        assert!(launches.is_empty() && cues.is_empty());
    }

    #[test]
    fn a_scene_mute_is_set_on_the_track_when_it_starts() {
        let mut p = project();
        let mut perf = Perform::default();
        let st = Started {
            track: 1,
            slot: Some(0),
            at: BAR,
            origin: BAR,
            mute: Some(true),
            from: BAR,
            rev: 0,
        };
        perf.started(&mut p, &st);
        assert!(p.tracks[1].mute);
    }
}
