//! What each audio track plays, for the audio engine (the audio tracks
//! rethink's phase 2, step 2b): [`crate::Player::audio`].
//!
//! The player decides everything the engine needs, so the engine decides
//! nothing about time itself:
//!
//! - **What plays at the period's start** ([`AudioAnswer::start`]):
//!   nothing, the song (song mode with no clip and no block: the track's
//!   own regions, which the engine keeps), or an audio clip.
//! - **Each change in the period, on its exact frame** ([`AudioChange`]):
//!   a launch, a stop (a once clip's own end too: Perform cues it in the
//!   bar before, as any cue), a legato switch, a scene's mute or the mute
//!   queue landing, a song block's edge, and what plays after a loop jump
//!   or a locate. The frame is the one a note on that tick lands on, so an
//!   audio clip never starts a frame away from the note clip beside it.
//! - **The track's mute rides along**: at the start and with each change,
//!   as the player has it (a scene's mutes, the mute queue). The audio
//!   panel's own Mute and Solo are the engine's.
//! - **A ring** ([`AudioChange::ring`]): a stop made by a once clip's own
//!   end, or a song block ending at its once clip's end, lets what sounds
//!   play on to the clip's last frame. Any later change ends it.
//! - **More than [`MAX_AUDIO_CHANGES`] changes** on a track in one period
//!   (only with Launch at Now or a tiny grid, or across a loop jump): the
//!   last ones are kept at their frames and the earlier ones are dropped
//!   as superseded, counted ([`AudioAnswer::superseded`],
//!   [`crate::Stats::audio_superseded`]). Nothing is ever moved earlier.
//!
//! An audio clip's regions are compiled into its clip's view and lent by
//! [`AudioPlays::Clip`], so a switch and the regions it plays always come
//! from the same view: the engine copies them when a voice starts, and
//! reads the stamp each period to take an edit (new regions at the next
//! pass, gain at once). Where a region starts in a pass, and where a pass
//! wraps, the engine finds on the period's spans
//! (`mpfree_transport::Span::offset_of`), as the player places notes.

use mpfree_seq::{ClipId, ClipRegion, MAX_TRACKS};
use mpfree_transport::time::Tick;

use crate::view::{pass_into, ClipView, PlayView};

/// The most changes [`crate::Player::audio`] gives a track in one period.
pub const MAX_AUDIO_CHANGES: usize = 4;

/// What an audio track plays.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum AudioPlays<'a> {
    /// Stopped, or no clip with song mode off.
    #[default]
    Nothing,
    /// Song mode, no clip launched and no block here: the track's own
    /// regions (the audio tracks part's).
    Song,
    /// An audio clip, launched or a song block's: its regions on its own
    /// ticks, pass 0 starting on song tick `origin` and one every `len`
    /// song ticks (an audio clip plays at normal speed).
    Clip {
        regions: &'a [ClipRegion],
        origin: f64,
        len: Tick,
        /// Where its last region's sound ends: what a ring plays on to.
        last_frame: i64,
        /// The clip's own gain, on top of each region's.
        gain: f32,
        clip: ClipId,
        /// Changes with any edit to what it plays (its regions, gain,
        /// last frame, length), not with where it plays.
        stamp: u64,
    },
}

/// A change on an audio track in the period.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AudioChange<'a> {
    /// The frame in the period it lands on.
    pub frame: u32,
    /// The song tick it lands on.
    pub tick: f64,
    /// What plays from it on.
    pub plays: AudioPlays<'a>,
    /// The track's mute from it on.
    pub muted: bool,
    /// What sounded plays on to its clip's last frame (a once clip's own
    /// end, or a block ending at its once clip's end) instead of ending.
    pub ring: bool,
}

/// What an audio track played in the last period
/// ([`crate::Player::audio`]).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AudioAnswer<'a> {
    pub start: AudioPlays<'a>,
    /// The track's mute at the start.
    pub muted: bool,
    changes: [AudioChange<'a>; MAX_AUDIO_CHANGES],
    n: u8,
    /// Changes dropped as more than [`MAX_AUDIO_CHANGES`]: the ones kept
    /// are the last.
    pub superseded: u32,
}

impl<'a> AudioAnswer<'a> {
    /// The changes in the period, in frame order.
    pub fn changes(&self) -> &[AudioChange<'a>] {
        &self.changes[..usize::from(self.n)]
    }
}

/// Where the clip a track plays is in the view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Source {
    /// The track's launched clip.
    Launched,
    /// The clip of a cue the player carried out that the view doesn't
    /// show as launched yet.
    Cued,
    /// A song clip ([`PlayView::song_clips`]), by its place, for a block.
    Placed(usize),
}

/// What an audio track plays, as the player keeps it until it is asked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum What {
    Nothing,
    Song,
    /// A clip, its origin, and the tick its pass counts from (a block's
    /// start for a block): a once stop at a pass's end rings only after it.
    Clip {
        at: Source,
        origin: Tick,
        from: Tick,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Mark {
    pub offset: u32,
    pub tick: Tick,
    pub what: What,
    pub muted: bool,
    pub ring: bool,
}

const NO_MARK: Mark = Mark {
    offset: 0,
    tick: 0,
    what: What::Nothing,
    muted: false,
    ring: false,
};

/// Each audio track's start and changes in the period.
#[derive(Clone, Debug)]
pub(crate) struct Marks {
    start: [Mark; MAX_TRACKS],
    changes: [[Mark; MAX_AUDIO_CHANGES]; MAX_TRACKS],
    n: [u8; MAX_TRACKS],
    superseded: [u32; MAX_TRACKS],
}

impl Marks {
    pub fn new() -> Marks {
        Marks {
            start: [NO_MARK; MAX_TRACKS],
            changes: [[NO_MARK; MAX_AUDIO_CHANGES]; MAX_TRACKS],
            n: [0; MAX_TRACKS],
            superseded: [0; MAX_TRACKS],
        }
    }

    /// Track `i` starts the period playing `what`, muted or not.
    pub fn begin(&mut self, i: usize, tick: Tick, what: What, muted: bool) {
        self.start[i] = Mark {
            tick,
            what,
            muted,
            ..NO_MARK
        };
        self.n[i] = 0;
        self.superseded[i] = 0;
    }

    /// What track `i` plays now: its last change, else its start.
    pub fn last(&self, i: usize) -> Mark {
        match self.n[i] {
            0 => self.start[i],
            n => self.changes[i][usize::from(n) - 1],
        }
    }

    /// Add a change on track `i`; the oldest goes when it is full. True
    /// when one was superseded.
    pub fn push(&mut self, i: usize, m: Mark) -> bool {
        let n = usize::from(self.n[i]);
        let row = &mut self.changes[i];
        if n == MAX_AUDIO_CHANGES {
            row.copy_within(1.., 0);
            row[MAX_AUDIO_CHANGES - 1] = m;
            self.superseded[i] += 1;
            true
        } else {
            row[n] = m;
            self.n[i] += 1;
            false
        }
    }

    /// The answer for track `i`, its clips lent from `view`.
    pub fn answer<'a>(&self, view: &'a PlayView, i: usize) -> AudioAnswer<'a> {
        let mut a = AudioAnswer {
            start: plays(view, i, self.start[i].what),
            muted: self.start[i].muted,
            n: self.n[i],
            superseded: self.superseded[i],
            ..AudioAnswer::default()
        };
        for (c, m) in a
            .changes
            .iter_mut()
            .zip(&self.changes[i][..usize::from(self.n[i])])
        {
            *c = AudioChange {
                frame: m.offset,
                tick: m.tick as f64,
                plays: plays(view, i, m.what),
                muted: m.muted,
                ring: m.ring,
            };
        }
        a
    }
}

/// The clip `what` names on track `i` in `view`.
pub(crate) fn clip_of(view: &PlayView, i: usize, what: What) -> Option<&ClipView> {
    let What::Clip { at, .. } = what else {
        return None;
    };
    let t = view.tracks.get(i)?;
    match at {
        Source::Launched => t.clip.as_ref(),
        Source::Cued => t.cue.as_ref()?.clip.as_ref(),
        Source::Placed(k) => view.song_clips.get(k),
    }
}

fn plays(view: &PlayView, i: usize, what: What) -> AudioPlays<'_> {
    match what {
        What::Nothing => AudioPlays::Nothing,
        What::Song => AudioPlays::Song,
        What::Clip { origin, .. } => match clip_of(view, i, what) {
            Some(c) => match &c.audio {
                Some(a) => AudioPlays::Clip {
                    regions: &a.regions,
                    origin: origin as f64,
                    len: c.length,
                    last_frame: a.last_frame,
                    gain: a.gain,
                    clip: a.id,
                    stamp: c.stamp,
                },
                // A note clip has nothing for the audio engine.
                None => AudioPlays::Nothing,
            },
            None => AudioPlays::Nothing,
        },
    }
}

/// Whether a stop on `line` from `was` rings: `was` a once audio clip,
/// `line` the end of one of its passes after the one it started in.
pub(crate) fn rings(view: &PlayView, i: usize, was: What, line: Tick) -> bool {
    let What::Clip { origin, from, .. } = was else {
        return false;
    };
    let Some(c) = clip_of(view, i, was) else {
        return false;
    };
    c.audio.as_ref().is_some_and(|a| a.once)
        && line > from
        && pass_into(line, origin, c.length, c.speed) == 0
}

#[cfg(test)]
mod tests {
    use mpfree_seq::{AudioClip, Clip, ClipId, Project, RecId, Track, ROWS};
    use mpfree_transport::{Block, Command, Transport};

    use super::*;
    use crate::view::{view_of, RegionView};
    use crate::{Out, Player, Sink};

    const BAR: Tick = 3840;
    /// 120 BPM at 48 kHz: 25 frames a tick.
    const TICK: u64 = 25;

    struct Quiet;
    impl Sink for Quiet {
        fn send(&mut self, _: Out) {}
    }

    /// A project with one audio track, and a view of it in song mode
    /// whose track 0 holds blocks `(start, end)` of an audio clip one bar
    /// long, `once` or not, each in step with its own start. Song blocks
    /// of audio clips come with step 2d; the player plays them already.
    fn song(blocks: &[(Tick, Tick)], once: bool) -> PlayView {
        let mut p = Project::new();
        p.tracks.push(Track::new_audio("vocal", 1));
        let mut c = Clip::new("X", BAR);
        c.id = ClipId(4);
        c.once = once;
        c.audio = Some(AudioClip::of(RecId(1), 0, 96_000, BAR, 120.0));
        let mut v = PlayView::compile(&p, &[]).unwrap();
        v.song = true;
        v.song_clips = vec![view_of(&c, None, ROWS + 4, 0, 0)];
        v.tracks[0].regions = blocks
            .iter()
            .map(|&(start, end)| RegionView {
                start,
                end,
                origin: start,
                clip: 0,
            })
            .collect();
        v
    }

    /// Play `view` from bar 1 for `frames`, in periods of `period`, with
    /// `commands`: each period's start and changes, frames counted from 0.
    fn story(
        view: &PlayView,
        frames: u64,
        period: u32,
        commands: &[Command],
    ) -> (Vec<(u64, Option<Tick>, bool)>, Player) {
        let mut t = Transport::new(48_000, 120.0).unwrap();
        for &c in commands {
            assert!(t.command(0, c));
        }
        let (mut player, mut block) = (Player::new(), Block::new());
        let mut out = vec![];
        let origin = |p: AudioPlays| match p {
            AudioPlays::Clip { origin, .. } => Some(origin as Tick),
            _ => None,
        };
        while t.frame() < frames {
            let f = t.frame();
            assert!(t.process(period, &mut block));
            player.process(view, &block, &mut Quiet);
            let a = player.audio(view, 0);
            // A jump between periods: the start differs from the last
            // period's end, a change on the period's first frame.
            if out
                .last()
                .is_none_or(|l: &(u64, Option<Tick>, bool)| l.1 != origin(a.start))
            {
                out.push((f, origin(a.start), false));
            }
            for c in a.changes() {
                out.push((f + u64::from(c.frame), origin(c.plays), c.ring));
            }
        }
        (out, player)
    }

    #[test]
    fn each_block_edge_is_a_change_and_a_block_ending_on_its_once_clips_end_rings() {
        // Two blocks end to end (each its own pass), a gap, then one that
        // ends where its clip does; and one cut short of its clip's end.
        let blocks = [(BAR, 2 * BAR), (2 * BAR, 3 * BAR), (4 * BAR, 5 * BAR)];
        let at = |t: Tick| t as u64 * TICK;
        for period in [1, 128, 1000] {
            let (s, _) = story(&song(&blocks, true), at(6 * BAR), period, &[Command::Play]);
            assert_eq!(
                s,
                [
                    (0, None, false),
                    (at(BAR), Some(BAR), false),
                    (at(2 * BAR), Some(2 * BAR), false),
                    (at(3 * BAR), None, true),
                    (at(4 * BAR), Some(4 * BAR), false),
                    (at(5 * BAR), None, true),
                ],
                "period {period}"
            );
        }
        let (s, _) = story(
            &song(&[(BAR, BAR + BAR / 2)], true),
            at(3 * BAR),
            128,
            &[Command::Play],
        );
        assert_eq!(s[2], (at(BAR + BAR / 2), None, false), "cut short");
        // Not a once clip: it never rings.
        let (s, _) = story(&song(&blocks, false), at(6 * BAR), 128, &[Command::Play]);
        assert!(s.iter().all(|c| !c.2));
    }

    #[test]
    fn past_four_changes_in_a_period_the_last_four_stay_on_their_frames() {
        // Six 10-tick blocks 10 ticks apart: twelve edges in a 4,096-frame
        // period (163 ticks).
        let blocks: Vec<(Tick, Tick)> = (0..6).map(|k| (10 + 20 * k, 20 + 20 * k)).collect();
        let view = song(&blocks, false);
        let mut t = Transport::new(48_000, 120.0).unwrap();
        assert!(t.command(0, Command::Play));
        let (mut player, mut block) = (Player::new(), Block::new());
        assert!(t.process(4096, &mut block));
        player.process(&view, &block, &mut Quiet);
        let a = player.audio(&view, 0);
        assert_eq!(a.start, AudioPlays::Song);
        assert_eq!(a.superseded, 8);
        assert_eq!(player.stats().audio_superseded, 8);
        let kept: Vec<(u32, bool)> = a
            .changes()
            .iter()
            .map(|c| (c.frame, c.plays == AudioPlays::Song))
            .collect();
        // The last two blocks' edges: 90, 100, 110 and 120 ticks.
        assert_eq!(
            kept,
            [
                (90 * 25, false),
                (100 * 25, true),
                (110 * 25, false),
                (120 * 25, true)
            ]
        );
        // The next period starts where the last change left it.
        assert!(t.process(4096, &mut block));
        player.process(&view, &block, &mut Quiet);
        let b = player.audio(&view, 0);
        assert_eq!(
            (b.start, b.superseded, b.changes().len()),
            (AudioPlays::Song, 0, 0)
        );
    }

    #[test]
    fn a_loop_jump_says_what_plays_on_the_far_side() {
        // Looping bars 1 and 2, the block on bar 2: back on bar 1 the song
        // plays again, from the jump's frame.
        let at = |t: Tick| t as u64 * TICK;
        let view = song(&[(BAR, 2 * BAR)], false);
        for period in [100, 1000, 4096] {
            let (s, _) = story(
                &view,
                at(4 * BAR + BAR / 2),
                period,
                &[Command::SetLoop(Some((0, 2 * BAR))), Command::Play],
            );
            assert_eq!(
                s,
                [
                    (0, None, false),
                    (at(BAR), Some(BAR), false),
                    (at(2 * BAR), None, false),
                    (at(3 * BAR), Some(BAR), false),
                    (at(4 * BAR), None, false),
                ],
                "period {period}"
            );
        }
    }

    #[test]
    fn a_note_clip_or_a_track_not_audio_gives_the_engine_nothing() {
        let mut p = Project::new();
        p.tracks.push(Track::new("drums", 1));
        let mut c = Clip::new("beat", BAR);
        c.id = ClipId(2);
        p.tracks[0].slots[0] = Some(c);
        let launch = crate::Launch {
            track: 0,
            slot: 0,
            origin: 0,
            start: 0,
        };
        let v = PlayView::compile(&p, &[launch]).unwrap();
        assert_eq!(v.audio, 0);
        let mut t = Transport::new(48_000, 120.0).unwrap();
        assert!(t.command(0, Command::Play));
        let (mut player, mut block) = (Player::new(), Block::new());
        assert!(t.process(512, &mut block));
        player.process(&v, &block, &mut Quiet);
        assert_eq!(player.audio(&v, 0), AudioAnswer::default());
        // Its clip view has no audio, so it has no audio share of a stamp.
        assert!(v.tracks[0].clip.as_ref().unwrap().audio.is_none());
    }
}
