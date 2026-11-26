//! mpfree-looper: the audio looper.
//!
//! Up to [`MAX_LOOPS`] stereo loops on the transport, recorded from the
//! line input, sharing one memory:
//!
//! - **The first pass** starts on the next bar line after Record, and ends
//!   after the set number of bars, or (free length) on the bar line after
//!   Record is pressed again. Its length in frames is what was recorded;
//!   its place in the song is a whole number of bars from `origin`.
//! - **Playback** follows the song position: the loop's position is the
//!   song position's place in the loop, so it stays locked to the bars
//!   through stops, locates and transport loops. At the tempo it was
//!   recorded at it plays sample for sample; at another tempo it plays
//!   faster or slower, like tape, until a print for that tempo is in.
//! - **Prints: another tempo, the same pitch.** A print ([`Print`]) is one
//!   pass of the loop's layers as heard, stretched to the loop's length at
//!   another tempo with its pitch kept ([`print`] makes one from a
//!   [`Looper::bounce_print`], on a worker). Handed in with
//!   [`Looper::print`], it plays in place of the layers while the loop is
//!   as it was when bounced ([`Status::version`]), crossfading in and out
//!   over [`FADE`] frames. Any change to the layers (an overdub, an undo,
//!   an edit, a layer's level) stops it until a new one comes.
//! - **Fit.** Overdub (or Record) while a print plays first fits the loop
//!   to the tempo: the print becomes the loop's one layer, at its length,
//!   and the overdub then starts, sample for sample at the new tempo. It
//!   copies at [`FIT_SPEED`], so the overdub waits about a 64th of the
//!   loop. Undo takes the fit back (the loop dips over [`FADE`] frames and
//!   comes back with its layers as they were). [`Command::Fit`] fits
//!   without overdubbing.
//! - **Overdubs** are layers on top of the first pass, recorded from the
//!   moment Overdub is pressed until it is pressed again, over as many
//!   passes as you like. Undo fades out and removes the newest layer; Redo
//!   brings it back, until a new overdub starts.
//! - **Each layer has its own level, pan, mute and solo**
//!   ([`Looper::layer_command`]), and can be deleted. A delete is undone
//!   and redone like an overdub: Undo and Redo walk one history of both.
//!   A layer keeps its id ([`LayerInfo::id`]) while it exists, whatever
//!   comes and goes around it.
//! - **Several loops.** [`Looper::select_loop`] picks the loop the commands
//!   after it act on; every loop plays. One loop records at a time:
//!   starting to record in one ends the recording in the others (an
//!   overdub punches out, a first pass ends on the next bar line, an armed
//!   loop disarms).
//! - **One memory.** Loops take memory in blocks of [`BLOCK`] frames from
//!   one pool, as they need it, and give it back when a layer can no
//!   longer come back. Nothing is ever moved, so the memory never
//!   fragments. [`Looper::memory`] is the gauge.
//! - **No clicks:** the first pass keeps what was heard just before it
//!   ([`KEEP_BEFORE`]) and after it ([`KEEP_AFTER`]), and the loop point
//!   is a seam the loop plays, never written into the audio: the end
//!   fades into what came just before the start, so it flows into the
//!   start, over the loop's seam fade ([`Edit::SeamFade`]). Punch in and
//!   out, start, stop, mute, undo, delete, clear and every layer's level,
//!   pan, mute and solo fade over [`FADE`] frames, and a jump in the song
//!   position (a locate, a transport loop that is not a whole number of
//!   loops, a restart) crossfades from the old place to the new one.
//!   Commands that would cut a fade short wait for it to end (at most
//!   [`FADE`] frames).
//! - **Stopped:** the loops are silent while the transport is stopped or
//!   counting in. Once a loop has faded out, its position waits at the
//!   song position's place, so the playhead stops with the transport and
//!   shows where it will start again. Overdub pressed while stopped waits
//!   for the transport to play; stopping ends an overdub (with a fade).
//! - **Latency:** the player hears the loop late (the output buffer) and
//!   their playing reaches the input late (the input buffer). Input that
//!   arrives `latency` frames after a moment belongs to that moment, so it
//!   is written that much earlier in the loop.
//! - **Saved with the project.** A playing loop's settings, layer mixes
//!   and layers (as they play, through its edits) go into a project
//!   ([`save`]: [`Looper::loop_file`], [`Looper::read_layer`]), and come
//!   back into an empty loop with [`Looper::load`]: the audio copies in at
//!   [`LOAD_SPEED`] while the loop stays silent, then the loop fades in,
//!   in time with the song.
//! - **Layers between loops.** [`Looper::copy_layer`] copies a layer, as
//!   it plays, onto another loop of the same length in bars (in time with
//!   the song, at that loop's length in frames), or into an empty loop,
//!   which takes the layer's loop's length and start. It copies at
//!   [`COPY_SPEED`] while both loops play on, then the layer fades in. A
//!   move then fades it out of its own loop as a delete (a loop's last
//!   layer stays). Undo in either loop takes a move back in both while
//!   each is the other's last step; undoing a layer that made a loop of
//!   its own clears that loop.
//! - **Free loops** ([`Clock::Free`]): a loop can keep time with the
//!   looper's free clock ([`FreeClock`]) instead of the song's. It plays
//!   on when the song stops, keeps its place when the song locates and its
//!   tempo when the song's changes, and records with the song stopped, at
//!   once or in your own time. Every free loop keeps time with the one free
//!   clock, which the first sets: a copy of the song's clock from the bar
//!   line it starts on, or the tempo its length reads as. STOP with the
//!   song stopped stops them ([`Command::FreeStop`] while it plays), and
//!   PLAY starts them again from the free clock's bar 1
//!   ([`Command::FreePlay`]).
//!
//! The memory is allocated once, in [`Looper::new`]. [`Looper::command`],
//! [`Looper::process`] and [`Looper::process_each`] never allocate, lock or
//! panic, and give the same output for any period size. `process_each`
//! keeps each loop's sound apart ([`LoopOuts`]), for a host that gives
//! each loop a channel of its own; their sum is `process`'s mix, bit for
//! bit.

#![forbid(unsafe_code)]

mod clock;
pub mod close;
mod command;
pub mod ends;
pub mod find;
mod free_time;
pub mod heard;
pub mod hits;
mod looper;
mod loops;
mod on_track;
mod outs;
pub mod print;
pub mod reading;
pub mod save;
mod smart;
mod status;
mod store;
mod view;

pub use close::Confidence;
pub use ends::{Candidate, Event, EventLog, InnerBeat, NoteStart, Verdict};
pub use heard::{Take, HEARD_SECONDS};
pub use on_track::{PlayRun, MAX_PLAY_RUNS};
pub use outs::LoopOuts;
pub use reading::{
    read_bars, read_bars_with, Reading, Sure, TempoHint, TempoKind, LOOP_MAX_BPM, LOOP_MIN_BPM,
};
pub use save::{LayerFile, LoopFile, RawLayer, RawShape};

// The engine itself, which was all in this file once: what is `pub` in
// these parts is the crate's, at its root as before, and each part sees
// the rest through `use super::*`.
pub use clock::*;
pub use command::*;
pub use free_time::*;
pub use looper::*;
use loops::*;
pub use smart::*;
pub use status::*;
pub use store::*;

use ends::NoteLog;
use find::FindJob;
use heard::Heard;
use hits::{HitLog, HitTrack};
use on_track::{OnTrack, Told};

use view::{Cursor, View};

use std::ops::Index;

use mpfree_transport::time::{ticks_in, Tick, PPQ};
use mpfree_transport::transport::MAX_PERIOD;
use mpfree_transport::{Block, Mark, Meter, Span, State, Swing, TransactionId};

/// Frames of every fade: the loop point, punch in and out, start, stop,
/// mute, undo and clear (about 5.8 ms at 44.1 kHz).
pub const FADE: usize = 256;
/// Layer slots in one loop. A loop plays at most one less (the first pass
/// and 14 overdubs), so that a merge always has a slot to render into.
pub const MAX_LAYERS: usize = 16;
/// Loops at most.
pub const MAX_LOOPS: usize = 8;
/// Frames in a block of memory (about 23 ms at 44.1 kHz). A layer takes
/// whole blocks.
pub const BLOCK: usize = 1024;
/// Commands waiting for the next period.
const QUEUE: usize = 32;
/// The most frames the loops run in one go (a period is cut into runs at
/// bar lines, markers and this many frames).
const RUN: usize = 128;
/// Overdubs, deletes and edits Undo can take back, per loop.
pub const HISTORY: usize = 64;
/// Edits Undo can take back, per loop (each keeps the loop's shape before
/// it).
const EDITS: usize = 16;
/// The longest seam fade, in frames (about 186 ms at 44.1 kHz).
pub const MAX_SEAM: usize = 8192;
/// Frames a first pass keeps of what was heard before its first frame:
/// a seam's worth at the longest, so the loop point fades into what was
/// really played before the start, at any seam fade.
pub const KEEP_BEFORE: usize = MAX_SEAM;
/// Frames a first pass keeps of what it hears after its end (while the
/// loop plays on): room for the end to move later, and a seam after it.
pub const KEEP_AFTER: usize = 2 * MAX_SEAM;
/// A 1/16 note, in ticks: the nudge step and the shortest loop an edit
/// makes.
const SIXTEENTH: Tick = PPQ as Tick / 4;
/// A merge renders this many frames of the merged layer for each frame
/// that plays, so it is done in half the loop's length.
pub const MERGE_SPEED: usize = 2;
/// A print's bounce ([`Looper::bounce_print`]) renders this many frames
/// for each frame that plays.
pub const PRINT_SPEED: usize = 8;
/// A fit copies the print into the loop at this many frames for each
/// frame that plays (a plain copy).
pub const FIT_SPEED: usize = 64;
/// A loaded loop ([`Looper::load`]) copies in at this many frames (of all
/// its layers together) for each frame that plays.
pub const LOAD_SPEED: usize = 64;
/// A layer copied from another loop ([`Looper::copy_layer`]) copies in at
/// this many frames for each frame that plays.
pub const COPY_SPEED: usize = 64;
const STEP: f32 = 1.0 / FADE as f32;
/// The fade-in of an overdub that starts where the first pass ends: the
/// input runs on without a break there, so a short fade is enough, and the
/// downbeat's attack is kept (about 0.7 ms at 44.1 kHz).
pub const HANDOFF_FADE: usize = 32;
/// The most loop frames the overdub writer moves on from one frame to the
/// next while the song plays on: the song at its fastest (400 BPM) over a
/// loop recorded at its slowest (20 BPM) moves 20. Further than that, or
/// back, the song jumped ([`Loop::od_jump`]).
const MAX_STEP: usize = 32;
/// A table entry with no block.
const NONE: u16 = u16::MAX;
/// An overdub whose input never went over this (-60 dB) took silence, and
/// leaves no layer ([`Overdubs`]).
pub const SILENT: f32 = 0.001;
/// A loop waiting for you to play ([`Starts::Play`]) by ear starts on the
/// first input sample at or over this (-40 dB).
pub const PLAY_LEVEL: f32 = 0.01;

/// A loop armed on the bar with the song playing starts on the first bar
/// line at least this many beats after the tap, so a tap just before a
/// bar line gives time to get ready instead of starting at once.
pub const LEAD_BEATS: i64 = 1;
static SILENCE: f32 = 0.0;

#[cfg(test)]
mod tests {
    use super::{bytes_for, n_blocks, rem_ticks, written_to, Looper, BLOCK, NONE};
    use crate::save::{LayerFile, LoopFile, RawLayer, RawShape};
    use crate::{After, Starts, FADE};

    /// `bytes_for` is what `Looper::new` really sets aside, and the
    /// longest Prefs size (20 minutes) fits in the block numbers at both
    /// rates, so it is never cut short without saying.
    #[test]
    fn bytes_for_is_the_memory_the_looper_takes() {
        for seconds in [0, 1, 7, 60] {
            let l = Looper::new(44_100, seconds);
            let s = &l.store;
            let real = s.pool.len() * 4 + s.tables.len() * 2 + s.free.len() * 2 + l.heard.bytes();
            assert_eq!(bytes_for(44_100, seconds), real, "{seconds} s");
            assert!(l.memory().capacity >= 44_100 * seconds as usize);
        }
        for rate in [44_100, 48_000] {
            let n = n_blocks(rate, 20 * 60);
            assert!(n < NONE as usize && n * BLOCK >= rate as usize * 1200);
        }
        assert_eq!(n_blocks(44_100, 3600), NONE as usize);
        // Five minutes: about 109 MB, a fifth of it per minute, and 21.2 MB
        // for the 60 seconds Capture takes from.
        assert_eq!(bytes_for(44_100, 300) / 1_000_000, 130);
    }

    /// Bit for bit what `rem_euclid` gives, over song positions and loop
    /// lengths of every size, whole multiples and the places either side.
    #[test]
    fn rem_ticks_is_rem_euclid() {
        let mut seed = 0x2545_F491_4F6C_DD1Du64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let mut checked = 0;
        for _ in 0..200_000 {
            let y = (next() % 2_000_000 + 1) as f64;
            let scale = [1.0, 10.0, 1e3, 1e5, 1e7, 1.9e9][(next() % 6) as usize];
            let unit = (next() % 1_000_000) as f64 / 1_000_000.0;
            let sign = if next() % 2 == 0 { 1.0 } else { -1.0 };
            let x = sign * unit * scale;
            let k = (next() % 1000) as f64 * sign;
            for x in [
                x,
                k * y,
                f64::from_bits((k * y).to_bits() + 1),
                f64::from_bits((k * y).to_bits().saturating_sub(1)),
                x / 7.0,
            ] {
                let (a, b) = (rem_ticks(x, y), x.rem_euclid(y));
                assert_eq!(a.to_bits(), b.to_bits(), "{x} rem {y}: {a} against {b}");
                checked += 1;
            }
        }
        for (x, y) in [
            (0.0, 3840.0),
            (-0.0, 3840.0),
            (-1e-300, 7680.0),
            (f64::from_bits(3840.0f64.to_bits() - 1), 3840.0),
            (2.5e9, 3840.0),
            (-2.5e9, 3.0),
        ] {
            assert_eq!(
                rem_ticks(x, y).to_bits(),
                x.rem_euclid(y).to_bits(),
                "{x} rem {y}"
            );
        }
        assert_eq!(checked, 1_000_000);
    }

    /// A layer kept as held takes memory up to its last frame written:
    /// all of it once it went round.
    #[test]
    fn written_to_is_the_last_frame_written() {
        assert_eq!(written_to(100, (0, 40)), 40);
        assert_eq!(written_to(100, (30, 40)), 70);
        assert_eq!(written_to(100, (70, 40)), 100);
        assert_eq!(written_to(100, (0, 100)), 100);
        assert_eq!(written_to(100, (5, 300)), 100);
    }

    /// Opened as held, a muted Replace covers nothing from its first
    /// frame, and a heard one all it wrote: they don't move there.
    #[test]
    fn a_loaded_replace_starts_where_its_mute_leaves_it() {
        let frames = 4 * BLOCK;
        let layer = |muted: bool, replace: Option<(u32, u32)>| LayerFile {
            level: 1.0,
            pan: 0.0,
            muted,
            solo: false,
            faded: 1.0,
            file: String::new(),
            audio: vec![0.25; 2 * frames],
            raw: Some(RawLayer {
                size: frames,
                valid: (BLOCK, BLOCK),
                pieces: vec![(0, frames as u32, false)],
                replace,
                rec: 0,
                from: Vec::new(),
            }),
        };
        for (muted, duck) in [(true, 0.0), (false, 1.0)] {
            let mut l = Looper::new(44_100, 4);
            let f = LoopFile {
                frames,
                ticks: 960,
                origin: 0,
                level: 1.0,
                muted: false,
                bars: None,
                after: After::Play,
                starts: Starts::Bar,
                fade_old: 1.0,
                speed: 0,
                rate: 0,
                id: 0,
                clock: crate::Clock::Song,
                layers: vec![layer(false, None), layer(muted, Some((64, 64)))],
                raw: Some(RawShape {
                    seam: FADE,
                    region: None,
                    captured: (0, 0),
                    cut: frames,
                }),
            };
            assert!(l.load(0, f).is_ok());
            let s = &l.loops[0].slots;
            assert_eq!((s[0].duck, s[1].duck), (1.0, duck), "muted {muted}");
            // Memory up to the last frame each wrote.
            assert_eq!(l.store.held[0][0], 2);
        }
    }
}
