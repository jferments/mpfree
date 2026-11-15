//! Export: one pass over the song makes a stereo mixdown, a stem per mixer
//! channel or per track, and a MIDI file of the notes each track played,
//! all lined up to the sample in one new folder. A track that shares its
//! mixer strip with others (the drum kit's tracks) gets its stem from a
//! replay: the range played again with only that track ([`Replay`]).
//!
//! - [`mode`]: Fast (faster than real time, the speakers quiet: the
//!   default) or Listen (in real time, heard), Prefs > Export, and the
//!   rule that makes an export listen anyway when the song uses something
//!   that only exists in real time ([`choose`], with the reason in words).
//! - [`plan`]: what an export makes: the streams (a mixer capture point
//!   and a WAV file each), the MIDI file, the folder, names a USB stick
//!   takes.
//! - [`start`] makes the folder, the files and a writer thread, and gives
//!   back two halves:
//!   - [`Pass`], the sound thread's: it sets the capture points
//!     ([`Pass::attach`]), feeds their blocks to `mpfree_fx::bounce` after
//!     the mixer each period ([`Pass::period`]), logs the notes the player
//!     sends ([`Pass::note`]), and after the range plays the tail until
//!     everything has been silent for [`pass::TAIL_QUIET_S`] (at most
//!     [`pass::TAIL_MAX_S`]). In fast mode the sound loop waits for room in
//!     the rings before each block ([`Pass::wait_for_room`]); in real time
//!     it never waits. Nothing in it allocates.
//!   - [`Job`], the screen's: progress for the panel, cancel,
//!     [`Job::next`] once a pass is back (the next replay's pass, if there
//!     is one, for the sound thread), and [`Job::finish`] after the last,
//!     which makes every file as long as the first pass's sound (cutting
//!     the tail's silence, never into the range), writes the MIDI file and
//!     says how it went ([`Outcome`]). A cancelled or failed export, in any
//!     pass, takes all its files away.
//!
//! The app's part (which range, the transport, pausing the sound card in
//! fast mode, Prefs, the panel) is in docs/dev/design/export.md.
#![forbid(unsafe_code)]

pub mod job;
pub mod mode;
pub mod pass;
pub mod plan;

pub use job::{clock, start, Done, Job, Next, Outcome, Progress};
pub use mode::{choose, Mode, Rig, Why};
pub use pass::{Failure, Pass, Step};
pub use plan::{file_names, folder_name, safe_name, MidiPlan, Only, Plan, Replay, Stream};

pub use mpfree_fx;
pub use mpfree_midi;
