//! Scenes → song and List → song by playing the rows (the Arrange
//! refactor's 4.1, plan D7): the steps play offline, faster than real
//! time, through a time engine of their own (the real transport, player,
//! Perform and its song list) with Rec song armed, and the song is what
//! Rec song recorded. So whatever the rows do when they play comes out
//! right because it really played: the mutes a scene leaves for the next,
//! one-shots, clips' follow actions and legato. The copy's clips are
//! bare ([`bare`]): only what decides where a block starts and ends.
//! Perform starts from its fixed seed ([`mpfree_perform::Perform`]'s
//! default), so a follow with a chance rolls the same every time and a
//! project converts the same way twice.
//!
//! The copy is the project with an empty song (nothing of the song plays
//! or is recorded over), every track heard (the scenes' mutes carry from
//! none), and the steps as its song list, played once
//! from the first ([`TimeAction::ListPlay`]), from song tick `at`. It
//! stops where the last step ends, where the list stops every track.
//!
//! A linked list's section (the Arrange refactor's 4b.2, [`play_part`])
//! is made the same way from one step, with no scene's mutes.
//!
//! Only ticks are kept, and nothing that decides where a block starts or
//! ends counts frames (cues land on bar lines, follow actions and
//! one-shots on ticks), so the copy plays at [`FAST_BPM`] and
//! [`FAST_RATE`], in periods of [`PERIOD`] frames: under a quarter of a
//! 4/4 bar, so every follow action and list step is cued in the bar
//! before its line, as it is live.
//!
//! Together it runs on the audio thread like every edit, and costs it
//! the time it takes; apart it runs on the control half's thread.

use mpfree_perform::ListPlaying;
use mpfree_play::{Event, Out};
use mpfree_seq::arrange::{scene_length, Arrangement};
use mpfree_seq::mpfree_moves::Moves;
use mpfree_seq::scenes::{ListStep, SongList};
use mpfree_seq::{Clip, Placement, Project};
use mpfree_time_ui::view::TimeAction;
use mpfree_transport::time::{Tick, PPQ};
use mpfree_transport::Command;

use super::{Heard, Pads, TimeHost};

/// The copy's tempo: the fastest the transport plays.
pub const FAST_BPM: f64 = mpfree_transport::time::MAX_BPM;
/// The copy's rate: the lowest the transport takes.
pub const FAST_RATE: u32 = mpfree_transport::transport::MIN_RATE;
/// The copy's period in frames: 594 ticks at [`FAST_BPM`] and
/// [`FAST_RATE`], under a quarter of a 4/4 bar.
pub const PERIOD: u32 = 2048;
/// Whole ticks a period of the copy plays.
const PERIOD_TICKS: Tick =
    (PERIOD as u64 * FAST_BPM as u64 * PPQ as u64 / (60 * FAST_RATE as u64)) as Tick;
/// The longest the rows may play, in bars: about 80 minutes at 120 BPM
/// (a few seconds of work).
pub const MAX_BARS: Tick = 9_999;

/// What the rows played: the blocks Rec song recorded, and where each
/// step started with its row.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Played {
    pub placements: Vec<Placement>,
    pub starts: Vec<(Tick, usize)>,
    /// Where the rows stop playing: `at` and their length.
    pub end: Tick,
}

/// Play `steps` from song tick `at` (on a bar line of `bar` ticks): see
/// the module notes. An error says why it couldn't (too long, the song
/// too full for what they play, or the copy refused something).
pub fn play_rows(
    project: &Project,
    steps: &[ListStep],
    at: Tick,
    bar: Tick,
) -> Result<Played, String> {
    play(project, steps, at, bar, true)
}

/// A linked list's section made from its scene (the Arrange refactor's
/// 4b.2): row `row` played `times` times from song tick `at`, as
/// [`play_rows`] plays one step, but with no scene's mutes, and the
/// blocks it plays. A linked section's blocks are the row's clips
/// whatever played before it: the scenes' mutes are the song's to bring
/// in as each section starts (4b.4), as the list brings them in live,
/// so a section plays the same wherever it is moved.
pub fn play_part(
    project: &Project,
    row: usize,
    times: u16,
    at: Tick,
    bar: Tick,
) -> Result<Vec<Placement>, String> {
    let step = ListStep { row, times };
    Ok(play(project, &[step], at, bar, false)?.placements)
}

/// [`play_rows`], with the scenes' mutes (`mutes`) or none.
fn play(
    project: &Project,
    steps: &[ListStep],
    at: Tick,
    bar: Tick,
    mutes: bool,
) -> Result<Played, String> {
    let bar = bar.max(1);
    let length: Tick = steps
        .iter()
        .map(|s| scene_length(project, s.row, bar) * Tick::from(s.times))
        .sum();
    if length > MAX_BARS * bar {
        return Err(format!(
            "they play {} bars, more than {MAX_BARS}",
            length / bar
        ));
    }
    let end = at + length;
    let mut copy = project.clone();
    copy.arrangement = Arrangement {
        list: SongList {
            steps: steps.to_vec(),
            ..SongList::default()
        },
        ..Arrangement::default()
    };
    for t in &mut copy.tracks {
        t.mute = false;
        t.solo = false;
        for (_, c) in t.slots.iter_mut() {
            bare(c);
        }
    }
    if !mutes {
        for s in &mut copy.scenes {
            s.mutes = None;
        }
    }
    let mut host = TimeHost::with_project(FAST_RATE, FAST_BPM, copy)?;
    if host.info().meter.bar_ticks() != bar {
        return Err("the copy's bars aren't the song's".into());
    }
    let quiet = &mut Quiet;
    host.command(Command::Locate(at));
    host.period(PERIOD, quiet);
    host.act(TimeAction::ListPlay(0))?;
    host.act(TimeAction::RecSong(true))?;
    host.command(Command::Play);
    let mut starts: Vec<(Tick, usize)> = Vec::new();
    let seen = |l: Option<ListPlaying>, starts: &mut Vec<(Tick, usize)>| {
        if let Some((step, start)) = l.and_then(|l| Some((l.step, l.start?))) {
            if starts.last().is_none_or(|&(t, _)| t < start) {
                starts.push((start, steps[step].row));
            }
        }
    };
    seen(host.list_playing(), &mut starts);
    // Twice the periods it takes, and then some: the transport didn't
    // roll.
    let mut left = 2 * (length + bar) / PERIOD_TICKS + 16;
    while host.info().position < end {
        if left == 0 {
            return Err("the copy's transport didn't play".into());
        }
        left -= 1;
        host.period(PERIOD, quiet);
        seen(host.list_playing(), &mut starts);
        if !host.song_rec_armed() {
            return Err("the song would hold too many blocks".into());
        }
    }
    host.act(TimeAction::RecSong(false))?;
    let placements = host.project().arrangement.placements.clone();
    Ok(Played {
        placements,
        starts,
        end,
    })
}

/// A clip as the copy plays it: what decides where its blocks start and
/// end stays (its id, length, speed, launch line, follow action, legato,
/// one-shot and loop cell), and what only sounds goes (its notes, locks,
/// moves and pressure, and the clip it was mutated from), so the copy's
/// periods play its launches and nothing else.
fn bare(c: &mut Clip) {
    c.notes.clear();
    c.locks.clear();
    c.moves = Moves::default();
    c.touch.clear();
    c.original = None;
}

/// Where the copy's clips play: nowhere.
struct Quiet;

impl Pads for Quiet {
    fn hit_at(&mut self, _track: u16, _pad: usize, _velocity: u8, _offset: u32) -> bool {
        true
    }
    fn release_at(&mut self, _track: u16, _pad: usize, _offset: u32) {}
    fn stop_all(&mut self) {}
    fn pressure_at(&mut self, _track: u16, _pad: usize, _value: u8, _offset: u32) {}
    fn out(&mut self, _o: &Out) -> bool {
        true
    }
    fn live(&mut self, o: &Out) -> Heard {
        match o.event {
            Event::NoteOn { .. } | Event::NoteOff { .. } => Heard::pad(o.event),
            _ => Heard::NONE,
        }
    }
}
