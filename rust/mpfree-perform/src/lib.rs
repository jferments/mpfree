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
//!   and an empty cell gets no cue, and its mute is set at once, which
//!   sounds the same. With the transport stopped it all happens at once.
//! - [`Perform::started`]: the host passes each [`Started`] the player
//!   reports, before applying it. A scene's mute becomes the track's
//!   `mute`, and the scene's first start is where its follow counts from.
//! - [`Perform::due`]: after each period, the follow actions that come due
//!   are queued as cues, in the bar before their point so they start on
//!   it: `bars` bars after the scene (or the clip) started. A late check
//!   (a locate past the point) starts it on the next bar line instead.
//!   A scene's follow goes first and cues every track, so a clip follow on
//!   a track that already has a cue waits: whatever is cued wins.
//! - Stay keeps playing and rolls again `bars` bars later, so a follow
//!   with a chance between Next and Stay plays on for a random number of
//!   passes. Again starts the same again (a new start to count from).
//!
//! [`morph`]: the scene morph (T10), two rows' stored values on the mod
//! engine's scenes A and B.
//!
//! [`song`]: live to song (T11), what the launch grid plays recorded into
//! the arrangement.
//!
//! Nothing here runs on the audio thread; the player only sees the cues.

#![forbid(unsafe_code)]

pub mod morph;
pub mod song;
pub use morph::{save_scene, Loaded, SceneMorph};
pub use song::{Recorded, SongRecorder};

use mpfree_play::{anchored, Cue, Launch, PlayView, Started};
use mpfree_seq::scenes::follow_target;
use mpfree_seq::{Follow, FollowAction, Project, MAX_TRACKS, SCENES, SLOTS};
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
    /// The tracks a clip follow cued (bit `t` is track `t`).
    pub clips: u16,
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
    clips: [Option<ClipCount>; MAX_TRACKS],
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
            clips: [None; MAX_TRACKS],
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
        let mutes = project
            .scene(row)
            .map(|s| (0..MAX_TRACKS).map(|t| s.mutes_track(t)));
        let mut mute = [None; MAX_TRACKS];
        if let Some(m) = mutes {
            for (t, v) in m.enumerate() {
                mute[t] = v;
            }
        }
        for (t, track) in project.tracks.iter_mut().enumerate() {
            let slot = track.slots[row].as_ref().map(|c| (row, c.length, c.speed));
            let busy = launches.iter().any(|l| l.track == t) || cues.iter().any(|c| c.track == t);
            cues.retain(|c| c.track != t);
            match when {
                When::Cued if slot.is_some() || busy => {
                    cues.push(Cue {
                        track: t,
                        slot: slot.map(|s| s.0),
                    });
                    self.cue_mutes[t] = mute[t];
                }
                When::Cued => {
                    if let Some(m) = mute[t] {
                        track.mute = m;
                    }
                    self.cue_mutes[t] = None;
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

    /// Stop every track (on the next bar line while rolling), and forget
    /// the scene.
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
        self.cued = None;
        self.playing = None;
    }

    /// Take back what is cued on `track` (the host removed its cue).
    pub fn uncue(&mut self, track: usize) {
        if let Some(m) = self.cue_mutes.get_mut(track) {
            *m = None;
        }
    }

    /// Set each cued track's scene mute on a freshly compiled view.
    pub fn dress(&self, view: &mut PlayView) {
        for (t, m) in self.cue_mutes.iter().enumerate() {
            if m.is_some() {
                view.set_cue_mute(t, *m);
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
        if let Some(row) = self.cued.take() {
            self.playing = Some(Playing {
                row,
                start: st.at,
                stays: 0,
            });
        }
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
        // The scene's follow first: it cues every track.
        if let (Some(p), None) = (self.playing, self.cued) {
            if let Some(f) = project.scene(p.row).and_then(|s| s.follow) {
                if now > point(p.start, f, p.stays, bar) - bar {
                    let mut filled = [false; SCENES];
                    for (r, x) in filled.iter_mut().enumerate() {
                        *x = project.row_filled(r);
                    }
                    let action = self.pick(&f);
                    let roll = self.roll();
                    match (action, follow_target(action, p.row, &filled, roll)) {
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
            let Some(f) = track.slots.get(l.slot).and_then(|c| c.as_ref()?.follow) else {
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
            let mut filled = [false; SLOTS];
            for (s, x) in filled.iter_mut().enumerate() {
                *x = track.slots[s].is_some();
            }
            let action = self.pick(&f);
            let roll = self.roll();
            match (action, follow_target(action, l.slot, &filled, roll)) {
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
                    fired.clips |= 1 << l.track;
                }
            }
        }
        fired
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

/// Where a follow from `start` comes due after `stays` Stays.
fn point(start: Tick, f: Follow, stays: u32, bar: Tick) -> Tick {
    start + i64::from(f.bars) * bar * (i64::from(stays) + 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_seq::{Clip, Track};

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
    fn an_idle_track_with_an_empty_cell_gets_its_mute_at_once() {
        let mut p = project();
        p.scene_mut(2).unwrap().mutes = Some(0b10);
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
        assert!(p.tracks[1].mute);
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
        };
        perf.started(&mut p, &st);
        cues.clear();
        // Due at bar 3 (two bars from bar 1): queued once past bar 2.
        for now in [BAR, 2 * BAR] {
            let f = perf.due(&mut p, &mut launches, &mut cues, now, BAR);
            assert_eq!(f, Fired::default(), "{now}");
        }
        let f = perf.due(&mut p, &mut launches, &mut cues, 2 * BAR + 1, BAR);
        assert_eq!(f.scene, Some(Some(1)));
        assert_eq!(cues.len(), 2);
        assert_eq!(perf.cued(), Some(1));
        // Waiting for its bar line, it does not fire again.
        let f = perf.due(&mut p, &mut launches, &mut cues, 2 * BAR + 2, BAR);
        assert_eq!(f, Fired::default());
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
        };
        perf.started(&mut p, &st);
        assert!(p.tracks[1].mute);
    }
}
