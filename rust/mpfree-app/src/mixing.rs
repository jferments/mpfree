//! The mixer in the pad player: mpfree-fx's [`Mixer`] is the one place the
//! sounds are summed.
//!
//! - The playing kit's 16 pads each have a pad channel (Pad Mix), and all
//!   16 go to Track 1 "Drums" (the sampler renders each pad into its own
//!   channel: `render_routed`; the clips play the kit too). Track 2
//!   "Synth" is Poly, Track 3 "Loop" is the audio looper's playback, and
//!   Track 4 "Audio" is the audio tracks on the Song view
//!   (`audiotracks.rs`). The tracks go to the master.
//! - Input 1 "Line in" is the line input, as the stereo pair, going to the
//!   master. It is heard while the looper's Monitor is on (the strip's
//!   Monitor follows it), and it is off while the line input is closed. The
//!   looper records the line input from its own copy, as before.
//! - The master fader is the pad player's Level (Q-Link 1 or the slider on
//!   the Play page), never above `--gain-db`. Master safety (the safe
//!   limiter at -0.3 dBFS) takes the place of the clip at full scale.
//! - The metronome goes on after the mixer, as before: never recorded or
//!   limited (it keeps itself within ±1).
//! - What the looper records from the pads ([`MixHost::instrument_strip`],
//!   [`MixHost::loop_take`]): the channel of the instrument the pads
//!   play, after its effects and fader, so a pattern on another
//!   instrument stays out. It comes a period late (the looper runs before
//!   the mixer), and the looper makes that up. When it can't be had, the
//!   looper records silence and says why; never another sound.
//!   [`MixHost::pads_wet`] (the master's input without the Loop track,
//!   the audio tracks' channels and the line input) is the pads as they
//!   were before.
//!   The master is refused while the Loop track's sound is anywhere else
//!   that plays into it: a submix, a send, or the echoes of either still
//!   ringing ([`MixHost::loop_echo`]).
//!
//! The Play page's effects are inserts from the start ([`PlayFx`]): a DJ
//! filter on the Drums and Loop tracks (the Filter Q-Links), a 1/8 dotted
//! delay on Return A (the Delay send Q-Links send to it), and Perform FX
//! (`mpfree_fx::Perform`) on the master, before Master safety: the throws
//! hold its pads, the XY pad plays its DJ filter and echo, and the FX mix
//! Q-Link is its Mix. Each rests (costs next to nothing) until it is used.
//!
//! Parameter locks: the clips' locks on strip controls and inserts go
//! through the mod engine ([`MixHost::lock`], `modmix.rs`), and once a
//! control has been locked the host's own turns of it go through the
//! engine too ([`MixHost::set`]), so the end of a lock puts back the last
//! turn.
//!
//! The screen: while the mixer panel shows ([`MixHost::show`]), the audio
//! loop sends a [`MixState`] every [`STATE_EVERY`] (it takes the meters),
//! and applies what the panel sends ([`MixHost::action`]).
//!
//! Nothing in [`MixHost::period`] allocates. Its time is in the "mix" stage
//! of CPU by engine; the state for the screen allocates, a few times a
//! second while the panel shows, as the other pages' states do.

use std::sync::Arc;
use std::time::{Duration, Instant};

use mpfree_fx::ids::mixer::output;
use mpfree_fx::ids::mixer::{self as mx, monitor, source};
use mpfree_fx::ids::{delay, dj_filter};
use mpfree_fx::{Delay, DjFilter, Edit, Insert, Mixer, MixerConfig, StripId, CAPTURES};
use mpfree_mixer_ui::{default_name, MixAction, MixState, MixView};
use mpfree_node::{Ctx, Held, HeldStep, Node, NodeEvent, Timed};
use mpfree_play::mpfree_param::{Owner, ParamAddr};
use mpfree_sampler::engine::PadOut;

use crate::instruments::{self, Instruments};
use crate::plugins::Plugins;
use crate::PERIOD;
use mpfree_fx::ids::perform as perf;

/// The tracks: what each engine plays into.
pub const DRUMS: usize = 0;
pub const SYNTH: usize = 1;
pub const LOOP: usize = 2;
/// The audio tracks (`audiotracks.rs`), summed after each track's own
/// level, pan, mute and solo.
pub const AUDIO: usize = 3;
pub const TRACKS: usize = 4;
// The parameter addresses count the engines' tracks the same way.
const _: () = assert!(mpfree_play::mpfree_param::ENGINE_STRIPS as usize == TRACKS);

/// Sequencer track `t`'s mixer channel (0 is track 1), where its own
/// instrument and a kit homed on it play, and what its clips' "this
/// track's strip" locks reach (`mpfree_param::track_strip`, the same
/// numbers): track 1's is the Drums strip (sampler R3 part 4b), so a
/// track has one strip whatever it plays; every other track's is its own,
/// past the engines' ([`crate::instruments::first_track`]` + t`). The
/// mixer track that was track 1's own (`TRACKS`) is no track's, but for a
/// project saved with settings on it ([`MixHost::old_first`]).
pub const fn track_channel(t: usize) -> usize {
    if t == 0 {
        DRUMS
    } else {
        crate::instruments::first_track() + t
    }
}

/// The sequencer track whose own channel mixer track `ch` is, the other
/// way from [`track_channel`]: none for the engines' tracks, track 1's old
/// own channel and the free places'. (Track 1's is Drums, which isn't
/// only track 1's.)
pub const fn channel_track(ch: usize) -> Option<usize> {
    if ch > crate::instruments::first_track() && ch < FREE_FIRST {
        Some(ch - crate::instruments::first_track())
    } else {
        None
    }
}

/// The first free place's strip: free places' instruments (the
/// instrument redesign's step 6, `instruments::FREE`) sound on strips of
/// their own, past every sequencer track's.
pub const FREE_FIRST: usize = crate::instruments::first_track() + crate::instruments::TRACKS;

/// Free place `n`'s mixer strip (from 0): [`FREE_FIRST`]` + n`.
pub const fn free_channel(n: usize) -> usize {
    FREE_FIRST + n
}

/// A track channel's name while the track's own instrument plays there
/// (the instrument redesign's step 4d, Jay's rule: name by instrument):
/// the instrument first, then the track, "FM · Track 3", so a name cut
/// short on a strip still says what plays. Track 1's is the Drums strip,
/// "Poly · Track 1" while its own instrument plays there; kit 1's pads,
/// from whichever track plays them, still sound there too (the manual's
/// mixer page says so), and the strip's name has no room for both.
pub fn instrument_channel_name(instrument: &str, track: &str) -> String {
    format!("{instrument} \u{b7} {track}")
}
/// The name of the shared Audio strip, which every audio track played
/// through before each had its own channel (the audio tracks rethink,
/// phase 1b). Nothing plays through it now; it shows only while it still
/// holds settings a project an older mpfree saved gave it, so they aren't
/// hidden ([`MixHost::old_audio`]).
pub const OLD_AUDIO: &str = "Audio (old)";
/// The name of track 1's old own channel, in a project saved with
/// settings on it ([`MixHost::old_first`]).
pub const OLD_TRACK_1: &str = "Track 1 (old)";
/// Pad channels: the playing kit's pads.
pub const PADS: usize = 16;
/// [`Pads::played`]'s bit for kit 1's pads past [`PADS`], which play
/// straight into the Drums track.
const DRUMS_DIRECT: u128 = 1 << PADS;
/// Pad channels lent to any kit's pad past kit 1's A1-A16 (R3 part 4a):
/// a pad gets one when it is given effects of its own, and gives it back
/// once it is plain again ([`PadChannels`]).
pub const LENT: usize = 48;
/// Every pad channel the mixer has: kit 1's A1-A16, then the lent ones.
pub const PAD_STRIPS: usize = PADS + LENT;
/// Kits a lent channel can name (the engine's most).
const KITS: usize = mpfree_sampler::engine::MAX_KITS;
/// Pads a kit has.
const KIT_PADS: usize = 128;
/// [`Pads::played`]'s first bit for the lent channels.
const LENT_BIT: usize = PADS + 1;

/// The pad channels lent past kit 1's A1-A16: which kit's pad holds each,
/// and, for the audio thread, each pad's lent channel. Fixed size, made
/// before the sound starts: lending and giving back never allocate.
pub struct PadChannels {
    /// Lent channel `i` (the mixer's pad channel `PADS + i`): the kit and
    /// pad holding it, or None while free.
    holder: [Option<(u8, u8)>; LENT],
    /// Each kit's pads' lent channel, plus 1 (0: none).
    of: Box<[[u8; KIT_PADS]; KITS]>,
}

impl Default for PadChannels {
    fn default() -> Self {
        PadChannels {
            holder: [None; LENT],
            of: Box::new([[0; KIT_PADS]; KITS]),
        }
    }
}

impl PadChannels {
    /// Kit `kit`'s pad `pad`'s own channel: kit 1's A1-A16 always have
    /// theirs, any other pad once one is lent to it.
    pub fn channel(&self, kit: usize, pad: usize) -> Option<StripId> {
        if kit == 0 && pad < PADS {
            return Some(StripId::Pad(pad));
        }
        let i = *self.of.get(kit)?.get(pad)?;
        (i > 0).then(|| StripId::Pad(PADS + usize::from(i) - 1))
    }

    /// The kit and pad whose channel `s` is: kit 1's for the first 16, the
    /// holder's for a lent one.
    pub fn holder(&self, s: StripId) -> Option<(usize, usize)> {
        match s {
            StripId::Pad(p) if p < PADS => Some((0, p)),
            StripId::Pad(p) => self
                .holder
                .get(p - PADS)
                .copied()
                .flatten()
                .map(|(k, q)| (usize::from(k), usize::from(q))),
            _ => None,
        }
    }

    /// The lent channels: each one's strip, kit and pad.
    pub fn lent(&self) -> impl Iterator<Item = (StripId, usize, usize)> + '_ {
        self.holder.iter().enumerate().filter_map(|(i, h)| {
            h.map(|(k, p)| (StripId::Pad(PADS + i), usize::from(k), usize::from(p)))
        })
    }

    /// Lends kit `kit`'s pad `pad` a channel: the one it has, or the first
    /// free one (true: new). None when all are lent.
    fn lend(&mut self, kit: usize, pad: usize) -> Option<(StripId, bool)> {
        if let Some(s) = self.channel(kit, pad) {
            return Some((s, false));
        }
        if kit >= KITS || pad >= KIT_PADS {
            return None;
        }
        let i = self.holder.iter().position(Option::is_none)?;
        self.holder[i] = Some((kit as u8, pad as u8));
        self.of[kit][pad] = i as u8 + 1;
        Some((StripId::Pad(PADS + i), true))
    }

    /// Lent channel `s` held by kit `kit`'s pad `pad`, as a project saved
    /// it: left out if it isn't a lent channel, is held already, or the
    /// pad has one.
    fn hold(&mut self, s: StripId, kit: u8, pad: u8) {
        let StripId::Pad(p) = s else { return };
        let Some(i) = p.checked_sub(PADS).filter(|&i| i < LENT) else {
            return;
        };
        let (k, q) = (usize::from(kit), usize::from(pad));
        if k >= KITS || q >= KIT_PADS || self.holder[i].is_some() || self.channel(k, q).is_some() {
            return;
        }
        self.holder[i] = Some((kit, pad));
        self.of[k][q] = i as u8 + 1;
    }

    /// Channel `s` free again (a lent one).
    fn give_back(&mut self, s: StripId) {
        let StripId::Pad(p) = s else { return };
        let Some(i) = p.checked_sub(PADS).filter(|&i| i < LENT) else {
            return;
        };
        if let Some((k, q)) = self.holder[i].take() {
            self.of[usize::from(k)][usize::from(q)] = 0;
        }
    }

    fn clear(&mut self) {
        for i in 0..LENT {
            self.give_back(StripId::Pad(PADS + i));
        }
    }
}

/// A pad's name on its lent channel's strip: "B5 · Kit 2".
pub fn lent_name(kit: usize, pad: usize) -> String {
    format!(
        "{}{} · Kit {}",
        (b'A' + (pad / 16) as u8) as char,
        pad % 16 + 1,
        kit + 1
    )
}
/// The line input's strip.
pub const LINE: StripId = StripId::Input(0);
/// Insert slots each strip has room for.
pub const SLOTS: usize = 8;
/// How often the panel gets a new state (meters) while it shows.
pub const STATE_EVERY: Duration = Duration::from_millis(40);
/// The Play page's inserts: the DJ filters' slot on the Drums and Loop
/// tracks, the delay's return, and the throws' slot on the master.
const FILTER_SLOT: usize = 0;
/// [`MixHost::poll_built`]'s line when the builder's queue was full and an
/// effect taken out of a chain was freed on the audio thread.
const DROPPED_HERE: &str =
    "mixer: an effect taken out was freed on the audio thread (the builder's queue was full)";
pub const DELAY_RETURN: usize = 0;
const PERFORM_SLOT: usize = 0;
/// A send this low is off.
pub const SEND_OFF_DB: f32 = -60.0;
/// Why the looper can't record the master while the Loop track sends to
/// return `r`, directly or through the submix it goes to
/// ([`MixHost::loop_take`]).
const LOOP_SENDS_HEARD: [&str; mpfree_fx::RETURNS] = [
    "the Loop track sends to Return A, so the loop would record its own echoes: turn that send down",
    "the Loop track sends to Return B, so the loop would record its own echoes: turn that send down",
    "the Loop track sends to Return C, so the loop would record its own echoes: turn that send down",
    "the Loop track sends to Return D, so the loop would record its own echoes: turn that send down",
];
/// Why the looper can't record the master yet after the Loop track's send
/// to return `r` came down: its echoes still ring there
/// ([`MixHost::loop_echo`]).
const LOOP_ECHOES_RING: [&str; mpfree_fx::RETURNS] = [
    "Return A still rings with the loop's echoes: the master records once they die away",
    "Return B still rings with the loop's echoes: the master records once they die away",
    "Return C still rings with the loop's echoes: the master records once they die away",
    "Return D still rings with the loop's echoes: the master records once they die away",
];
/// Why the looper can't record the master while the Loop track goes to a
/// submix, or the submix's effects still ring with it.
const LOOP_IN_SUBMIX: &str =
    "the Loop track goes to a submix, so the master would record the loop: send it to the master";
const LOOP_SUBMIX_RINGS: &str =
    "the submix the Loop track went to still rings with the loop: the master records once it dies away";
/// Frames a tail that never ends is counted as (a frozen delay): held
/// until it ends.
const RINGS_ON: u64 = u64::MAX;
/// The capture point the sampler resamples from (the recorder's Source):
/// the last of the mixer's [`CAPTURES`], so the others stay free for
/// print, stems and the looper.
pub const SAMPLER_CAPTURE: usize = CAPTURES - 1;
/// The looper's capture points, [`LOOP_CAPTURE`] and the three after it,
/// below Freeze and Print's: the master's input, then the strips taken out
/// of it ([`MixHost::pads_wet`]).
pub const LOOP_CAPTURE: usize = SAMPLER_CAPTURE - 5;
/// The audio tracks' capture points, [`AUDIO_CAPTURE`] + track, below the
/// looper's: each audio track recording from inside the mix (the
/// instrument redesign's step 9, [`MixHost::audio_take`]).
pub const AUDIO_CAPTURE: usize = LOOP_CAPTURE - mpfree_audiotracks::MAX_TRACKS;
/// The audio tracks' own channels after their faders, one capture point an
/// engine track from here: what the looper's pads leave out of the mix
/// for them ([`MixHost::pads_wet`], [`MixHost::point_audio_out`]).
pub const AUDIO_OUT_CAPTURE: usize = AUDIO_CAPTURE - mpfree_audiotracks::MAX_TRACKS;
/// The strips the looper's pads leave out besides the audio tracks: the
/// loops and the line input, in the order of their capture points after
/// [`LOOP_CAPTURE`].
const LEFT_OUT: [StripId; 2] = [StripId::Track(LOOP), LINE];

/// What the Play page's Q-Links and speakers show, read from the mixer (so
/// the mixer panel's changes show there too).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlayLevels {
    /// The Drums and Loop tracks' sends to the delay, dB.
    pub drums_send_db: f32,
    pub loop_send_db: f32,
    /// The Loop track's and the line input's faders, dB.
    pub loop_db: f32,
    pub line_db: f32,
    /// The DJ filters, -100 (low-pass) to 100 (high-pass); 0 is off.
    pub drums_filter: f32,
    pub loop_filter: f32,
    /// The XY pad's FX mix, 0 to 1.
    pub fx_mix: f32,
}

/// A change from the Play page for the mixer. The page says what it wants
/// (a filter's place, a throw held, where the XY pad is); which engine and
/// which parameter make it is the mixer's business ([`MixHost::play`]), so
/// another performance engine swaps in here without the page changing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PlayFx {
    /// A strip's parameter (a fader, a send).
    Strip { strip: StripId, id: u16, value: f32 },
    /// A track's DJ filter: -100 (low-pass) to 100 (high-pass), 0 off.
    Filter { strip: StripId, position: f32 },
    /// Kit 1's pad channel `pad` (A1-A16) muted or let play in the Mix
    /// tab: the pad's sound on every track. A pad mute on a track is the
    /// time engine's (the instrument redesign's step 10).
    PadMute { pad: usize, mute: bool },
    /// Drop drums: these pads (a bit each) muted while held, as well.
    Drop(u16),
    /// A throw held or let go: as the Perform FX pad's own settings say
    /// (`how` None, Play's throw cards), or as a Perform pad's say.
    Throw {
        fx: ThrowFx,
        down: bool,
        how: Option<ThrowHow>,
    },
    /// Macro `card` (0 to 7) held at `value` (0 to 1) by a Perform pad, or
    /// let go (None): back to where it was.
    Macro { card: u8, value: Option<f32> },
    /// The XY pad: where the finger is (0 to 1 each way), and whether it
    /// is down.
    Xy { x: f32, y: f32, down: bool },
    /// What the XY pad plays, picked from its chip's list (Jay, 18 Nov
    /// 2026 07:34Z): None, the Perform FX's DJ filter and echo (as
    /// before); `Some(k)`, effect `k` of the XY FX
    /// (`mpfree_fx::xyfx::EFFECTS`), put on the master after the Perform
    /// FX the first time it is picked.
    XyEffect(Option<usize>),
}

/// The XY FX's node id: what the XY pad plays when an effect is picked.
pub const XY_FX: &str = "mpfree.fx.xy";

/// What the master's XY FX starts with when the pad puts it there: the
/// effect picked, closed (Active off) until a finger lands.
static XY_START: [[(u16, f32); 2]; 9] = {
    use mpfree_fx::xyfx::ids as xy;
    let mut out = [[(xy::EFFECT, 0.0), (xy::ACTIVE, 0.0)]; 9];
    let mut k = 0;
    while k < 9 {
        out[k][0].1 = k as f32;
        k += 1;
    }
    out
};

/// How a Perform pad's throw plays: the Perform FX pad's Starts (1 at
/// once, 2 on the next 1/16, 3 on the next beat; 0 its own Punch in), Let
/// go (cut, else ring out) and Plays (for a bar, else while held), set
/// for the pad's hit and put back after it, so Play's throw cards and the
/// Perform FX page keep the pad's own settings; and the strip it plays on
/// (`on`: the master, a track or a submix), whose Perform FX it holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThrowHow {
    pub starts: u8,
    pub cut: bool,
    pub bar: bool,
    pub on: StripId,
}

impl Default for ThrowHow {
    fn default() -> ThrowHow {
        ThrowHow {
            starts: 0,
            cut: false,
            bar: false,
            on: StripId::Master,
        }
    }
}

/// The Perform FX's id: on the master from the start, and put on a track
/// or a submix the first time a Perform pad's throw plays there.
pub const PERFORM_FX: &str = "mpfree.fx.perform";

/// Throws held while the Perform FX they play on is being put on their
/// strip, at most: each plays when it lands.
const WAITING: usize = 4;

/// A Perform pad's throw held while its strip's Perform FX is being put
/// on ([`MixHost::play`]).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Waiting {
    fx: ThrowFx,
    how: ThrowHow,
}

/// The strips a Perform pad's throw can play on, named as the Mix tab
/// names them: the master, the engines' tracks, each track's own channel
/// in use (named after its track, as the stems are), and the submixes.
pub fn throw_strips(
    mixer: &Mixer,
    in_use: u128,
    tracks: &[String],
    own: &dyn Fn(StripId) -> Option<(usize, &'static str)>,
) -> Vec<(String, StripId)> {
    let mut out = vec![(name(StripId::Master), StripId::Master)];
    let mut subs = Vec::new();
    for s in mixer.strips() {
        match s {
            StripId::Track(t) if t < TRACKS => out.push((track_strip_name(s, tracks, own), s)),
            // A free place's strip, while an instrument plays there.
            StripId::Track(t) if t >= FREE_FIRST => {
                if own(s).is_some() {
                    out.push((track_strip_name(s, tracks, own), s));
                }
            }
            StripId::Track(t) if in_use & (1 << (t - TRACKS)) != 0 => {
                out.push((track_strip_name(s, tracks, own), s));
            }
            StripId::Bus(_) => subs.push((name(s), s)),
            _ => {}
        }
    }
    out.extend(subs);
    out
}

/// A track strip's name in the lists other screens pick from (Perform's
/// On, the Looper's Records from, export's stems): a track's own
/// instrument's channel as [`instrument_channel_name`] has it, with the
/// track's own name when it has one; another track's channel by that
/// name; the engines' strips as [`name`] has them.
pub fn track_strip_name(
    s: StripId,
    tracks: &[String],
    own: &dyn Fn(StripId) -> Option<(usize, &'static str)>,
) -> String {
    let track_name = |t: usize| {
        tracks
            .get(t)
            .filter(|n| !n.trim().is_empty())
            .cloned()
            .unwrap_or_else(|| own_track_label(s, t))
    };
    match (own(s), s) {
        (Some((t, instrument)), _) => instrument_channel_name(instrument, &track_name(t)),
        (None, StripId::Track(t)) if t >= TRACKS => track_name(t - TRACKS),
        (None, s) => name(s),
    }
}

/// Track `t`'s part of its own instrument's channel name: "Track 3", and
/// track 1's old own channel ([`MixHost::old_first`]) as [`OLD_TRACK_1`].
/// A free place's instrument is named for who plays it
/// ([`MixHost::own_instrument_on`]): the pads ([`instruments::THE_PADS`],
/// "Pads"), else the first track that does.
fn own_track_label(s: StripId, t: usize) -> String {
    if s == StripId::Track(TRACKS) {
        OLD_TRACK_1.into()
    } else if t == instruments::THE_PADS {
        "Pads".into()
    } else {
        format!("Track {}", t + 1)
    }
}

/// The performance effects a throw holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThrowFx {
    Delay,
    Reverb,
    Stutter,
    TapeStop,
    FilterDip,
    BeatRepeat,
    VinylBrake,
}

impl ThrowFx {
    /// The Perform FX pad (0 to 15) that plays it, in the default bank:
    /// Stutter 1/16, Tape stop 1/4, Vinyl brake 1/2, Echo throw 1/8 dotted,
    /// LP sweep 1 bar, Beat repeat 1/4 and Wash throw.
    pub fn pad(self) -> usize {
        match self {
            ThrowFx::Stutter => 0,
            ThrowFx::TapeStop => 2,
            ThrowFx::VinylBrake => 3,
            ThrowFx::Delay => 9,
            ThrowFx::FilterDip => 10,
            ThrowFx::BeatRepeat => 14,
            ThrowFx::Reverb => 15,
        }
    }
}

/// A throw's own settings on the Perform FX of the strip it plays on
/// ([`MixHost::throw_knobs`]): what the Perform view's Q-Links turn. They
/// are the Perform FX pad's, so every throw of that effect on that strip
/// shares them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ThrowKnobs {
    pub on: StripId,
    pub fx: ThrowFx,
    /// The Perform FX insert's slot id (`Chain::uid`); `None` while the
    /// strip has none.
    pub uid: Option<u16>,
    /// The pad's Effect (an index into `mpfree_fx::PERFORM_EFFECTS`), its
    /// Length (an index into `mpfree_fx::PERFORM_LENGTHS`) and Amount (0
    /// to 100).
    pub effect: u8,
    pub length: f32,
    pub amount: f32,
}

impl ThrowKnobs {
    /// Parameter `o` of the pad (`perf::LENGTH`, `perf::AMOUNT`) for the
    /// mod engine, while the strip has its Perform FX.
    pub fn addr(&self, o: u16) -> Option<ParamAddr> {
        let strip = crate::modmix::strip_of(self.on)?;
        Some(ParamAddr::new(
            Owner::Insert(strip, self.uid?),
            perf::slot(self.fx.pad()) + o,
        ))
    }
}

/// A period of stereo sound.
pub type Stereo<'a> = (&'a [f32; PERIOD], &'a [f32; PERIOD]);

/// A strip's name, a lent pad channel by its pad and kit ("B5 · Kit 2"),
/// any other as [`name`] has it.
pub fn strip_name(ch: &PadChannels, s: StripId) -> String {
    match s {
        StripId::Pad(p) if p >= PADS => match ch.holder(s) {
            Some((k, q)) => lent_name(k, q),
            None => format!("Pad channel {}", p + 1),
        },
        s => name(s),
    }
}

/// The strips' names on the panel.
pub fn name(s: StripId) -> String {
    match s {
        StripId::Track(DRUMS) => "Drums".into(),
        StripId::Track(SYNTH) => "Synth".into(),
        StripId::Track(LOOP) => "Loop".into(),
        StripId::Track(AUDIO) => OLD_AUDIO.into(),
        StripId::Track(TRACKS) => OLD_TRACK_1.into(),
        StripId::Track(t) if t >= FREE_FIRST => format!("Free {}", t - FREE_FIRST + 1),
        StripId::Track(t) if t >= TRACKS => format!("Track {}", t - TRACKS + 1),
        LINE => "Line in".into(),
        s => default_name(s),
    }
}

/// A delay or reverb put on a send return starts fully wet (Jay, 16 Nov):
/// the tracks' own sound already reaches the master through their strips,
/// so a return only has to add the effect, and each track's send sets how
/// much. At an insert's Mix (25 to 30 %) most of what came back would be a
/// second dry copy. On any other strip, and for any other effect, `node`
/// keeps its own default. The Play page's delay on Return A is set wet
/// where it is made.
pub fn wet_on_a_return(strip: StripId, node: &mut dyn Node) {
    if !matches!(strip, StripId::Return(_)) {
        return;
    }
    let delay_or_reverb = mpfree_fx::families::family(node.info().id)
        .is_some_and(|f| matches!(f.name, "Delay" | "Reverb"));
    let mix = node.params().iter().find(|p| p.name == "Mix").map(|p| p.id);
    if let (true, Some(mix)) = (delay_or_reverb, mix) {
        node.set_param(mix, 100.0);
    }
}

pub struct MixHost {
    pub mixer: Mixer,
    /// The mod engine between the clips' locks and the mixer.
    pub modu: crate::modmix::ModMix,
    /// The highest the master fader goes (`--gain-db`).
    max_db: f32,
    monitor: bool,
    line_open: bool,
    /// Master safety's count of frames over the ceiling, at the last period.
    overs: u64,
    /// The panel's view while it shows, and the strip selected on it.
    shown: Option<MixView>,
    /// The view shown last, for the Mix tab.
    last: MixView,
    selected: Option<StripId>,
    state_at: Option<Instant>,
    /// Kit 1's pad channels muted in the Mix tab, and by Drop drums.
    pad_mutes: u16,
    dropped: u16,
    /// An old project's pad mutes and solos, for the app to give to each
    /// track on kit 1 ([`MixHost::take_old_pad_mutes`]).
    old_pads: Option<(u16, u16)>,
    /// What the XY pad plays ([`PlayFx::XyEffect`]).
    xy_effect: Option<usize>,
    /// Who holds the XY pad (`mpfree_xy::Gesture`): the first touch until
    /// it ends, or a newer gesture from the other controller.
    xy_gesture: mpfree_xy::Gesture,
    /// Each controller's own gesture on the pad is down, the screen's
    /// ([`MixHost::play_screen`]) and a learned control's
    /// ([`MixHost::play`]), whether or not it holds the pad; the pad's
    /// state on the screen ([`MixHost::screen_xy`]) can end the screen's.
    xy_on: [bool; 2],
    /// Counts the pad's inputs, for the gesture's order.
    xy_seq: u32,
    /// The pad was pressed since the last period ran.
    xy_new: bool,
    /// A lift that came before the period its press plays in (a tap
    /// quicker than a period): done once that period has played.
    xy_late: Option<(f32, f32)>,
    /// Lifts the pad's state on the screen did, its lift action not having
    /// come (counted for the status line).
    xy_found_up: u64,
    /// A layout of yours on the XY pad (`xy_play.rs`): while one plays,
    /// the pad's gestures play it through the hand layer.
    xy_play: crate::xy_play::XyPlay,
    /// Frames until a learned control's gesture on a layout of yours ends
    /// (`HAND_S` after its last move; 0: none going).
    xy_midi_left: u32,
    /// A track records (`main.rs` says so each loop): the XY pad's hand
    /// beats a step's lock, and Stay keeps no value.
    recording: bool,
    /// Perform pads' throws held while their strip's Perform FX is being
    /// put on: each plays when it lands ([`MixHost::poll_built`]).
    waiting: [Option<Waiting>; WAITING],
    /// What the Perform set's throws on tracks and submixes said, for the
    /// log and the screen: a Perform FX put on or taken off, or why a
    /// throw couldn't play.
    throw_said: Vec<String>,
    /// The tracks and submixes the Perform set's throws play on
    /// ([`MixHost::perform_strips`]), the ones whose Perform FX the set put
    /// on this session, and the ones being put on now.
    perform_want: Vec<StripId>,
    perform_added: Vec<StripId>,
    perform_building: Vec<StripId>,
    /// Where [`LOOP_CAPTURE`] listens now ([`MixHost::loop_take`]): the
    /// master's input for the pads and the master, a strip after its
    /// fader for a mix.
    loop_point: (StripId, mpfree_fx::CapturePoint),
    /// Where each audio track's capture point listens now
    /// ([`MixHost::audio_take`]), None while it is free.
    audio_points: [Option<(StripId, mpfree_fx::CapturePoint)>; mpfree_audiotracks::MAX_TRACKS],
    /// Each engine audio track's channel as [`AUDIO_OUT_CAPTURE`] + track
    /// takes it ([`MixHost::point_audio_out`]).
    audio_outs: [Option<usize>; mpfree_audiotracks::MAX_TRACKS],
    /// Frames each return still rings with the Loop track's sound, and the
    /// submix it went to with how long that rings ([`MixHost::loop_echo`]).
    loop_echoes: [u64; mpfree_fx::RETURNS],
    loop_bus_echo: Option<(StripId, u64)>,
    /// The pad strips' mutes as `apply_mutes` last set them (a bit each):
    /// a strip's parameter reads its old value until the next period, so
    /// two changes in one period compare with what was sent.
    mutes_sent: u16,
    /// Builds the chain editor's effects, and drops the ones edits take
    /// out, off the audio thread (control.rs). None: here (the tests).
    builder: Option<crate::control::Builder>,
    /// A built effect the mixer had no room for yet (its edits were full),
    /// put in on a later period.
    landing: Option<crate::control::Built>,
    /// The CLAP plugins found (plugins.rs): the picker's plugin cards and
    /// what the builder makes `clap:` effects with.
    plugins: Arc<Plugins>,
    /// A project's mixer being swapped in ([`MixHost::open`]).
    opening: Option<Box<Open>>,
    /// Each track's own instrument (instruments.rs), rendered into its
    /// mixer track before the mixer runs.
    pub instruments: Instruments,
    /// Each synth-voice pad's own synth (pad_synths.rs), rendered into its
    /// pad's strip after the tracks' instruments.
    pub pad_synths: crate::pad_synths::PadSynths,
    /// What the last synth-voice pad's copy the builder made came to, for
    /// the log ([`MixHost::take_pad_landed`]).
    pad_landed: Option<PadLanded>,
    /// The tracks whose channels a kit plays into (a kit other than kit
    /// 1, on the first track that plays it), a bit each, as the last
    /// period's [`MixHost::pads_homed`] had them.
    kit_tracks: u128,
    /// The tracks that are audio tracks with an engine track, a bit each
    /// (`audiotracks::TrackHost::play_into` sets it every period): each
    /// plays into its own channel (the audio tracks rethink, phase 1b).
    pub audio_tracks: u128,
    /// Each kit's home as the last period's [`MixHost::pads_homed`] had it
    /// (0: the pads and Drums; t: track t - 1's channel).
    homes: [u8; KITS],
    /// The pad channels lent to pads past kit 1's A1-A16 (R3 part 4a).
    pub pad_channels: PadChannels,
    /// How long the tracks' own instruments took in the last period (0
    /// when no track has one), for load shedding's cost a voice (shed.rs).
    pub instrument_time: Duration,
    /// Projects opened whose track 1 kept its old own channel
    /// ([`MixHost::old_first`]).
    pub old_first_opens: u64,
    /// Instruments made without a builder (the tests), waiting to go in.
    made: Vec<crate::control::MadeInstrument>,
    /// Each plugin synth's own presets, once listed (or why not), by its
    /// node id; and the ones asked for and not back yet.
    plugin_presets: Vec<(&'static str, Result<Vec<mpfree_plughost::Preset>, String>)>,
    presets_asked: Vec<&'static str>,
    /// Preset news made without a builder (the tests).
    preset_news: Vec<crate::control::PresetNews>,
    /// The effect picker's favourites and recents (`files/fx/`), and
    /// whether they changed since they were last saved.
    pub shelf: mpfree_fx::picker::Shelf,
    pub shelf_dirty: bool,
    /// Effects' modes (fxmodes.rs): the values each insert had in the
    /// engines it left, and the switches across engines on their way.
    pub modes: crate::fxmodes::Modes,
    /// Freeze and Print (takes.rs).
    pub takes: crate::takes::Takes,
    /// A take's memory made without a builder (the tests).
    here: crate::takes::Here,
    /// An insert's memory on its way to a pad ([`MixHost::capture`]), and
    /// one that is done and not yet sent back.
    capturing: Option<Capturing>,
    captured: Option<(Box<Held>, Captured)>,
}

/// An insert's memory being copied out, a piece a period
/// ([`CAPTURE_STEP`] frames), so a long one never costs a period much.
struct Capturing {
    strip: StripId,
    uid: u16,
    held: Box<Held>,
}

/// Frames of an insert's memory copied a period: 8,192 stereo frames are
/// 64 KB, a small part of a period's time, and 8 s of Grains memory takes
/// 44 periods (about 130 ms).
pub const CAPTURE_STEP: usize = 8192;

/// How a capture of an insert's memory ended ([`MixHost::capture`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Captured {
    /// All of it is in the box.
    Done,
    /// The effect holds nothing yet.
    Nothing,
    /// Its memory was cleared or overwritten while it was copied.
    Lost,
    /// The effect was taken out, or moved off its strip.
    Gone,
    /// Another capture is on its way.
    Busy,
}

impl Captured {
    /// Why a capture made nothing, for the hint line.
    pub fn why(self) -> &'static str {
        match self {
            Captured::Done => "done",
            Captured::Nothing => "the effect hasn't heard anything yet",
            Captured::Lost => "its memory changed while it was copied; try again",
            Captured::Gone => "the effect was taken out",
            Captured::Busy => "another capture is on its way",
        }
    }
}

/// The take host's memory: made and dropped on the builder thread, or
/// here without one (the tests).
struct TakeMem<'a> {
    builder: Option<&'a mut crate::control::Builder>,
    here: &'a mut crate::takes::Here,
}

impl crate::takes::Memory for TakeMem<'_> {
    fn ask(&mut self, rate: u32, seconds: f32) -> bool {
        match self.builder.as_deref_mut() {
            Some(b) => b.print(rate, seconds),
            None => self.here.ask(rate, seconds),
        }
    }
    fn made(&mut self) -> Option<mpfree_fx::Print> {
        match self.builder.as_deref_mut() {
            Some(b) => b.take_print(),
            None => self.here.made(),
        }
    }
    fn drop_frozen(&mut self, f: mpfree_fx::Frozen) {
        if let Some(b) = self.builder.as_deref_mut() {
            b.drop_frozen_later(f);
        }
    }
    fn drop_print(&mut self, p: mpfree_fx::Print) {
        if let Some(b) = self.builder.as_deref_mut() {
            b.drop_print_later(p);
        }
    }
}

/// What became of a synth-voice pad's copy the builder made
/// ([`MixHost::poll_instruments`]), for the log.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PadLanded {
    /// The pad plays its own `name` now.
    Playing {
        pad: crate::pad_synths::PadFor,
        name: &'static str,
    },
    /// It couldn't be made: the pad plays through the Synth channel.
    Failed { pad: crate::pad_synths::PadFor },
    /// The pad wasn't waiting for it any more (it changed meanwhile), or
    /// no slot was free: dropped.
    Unwanted { pad: crate::pad_synths::PadFor },
}

/// What [`MixHost::poll_instruments`] put in (or couldn't) for the log.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Landed {
    /// Track `track` now plays `name`.
    Playing { track: usize, name: &'static str },
    /// Instrument `id` couldn't be made for track `track`; it keeps what
    /// it had.
    Failed { track: usize, id: &'static str },
    /// Instrument `name` was made for track `track`, but
    /// [`instruments::SLOTS`] other tracks already have their own; it keeps
    /// what it had.
    Full { track: usize, name: &'static str },
}

/// A project's mixer going in over a few periods: the old inserts out,
/// then the new ones in, each as an edit the mixer makes while the strip
/// has ducked out; then the keys, the strips' controls and the mod engine.
pub struct Open {
    load: Box<crate::project::MixLoad>,
    stage: OpenStage,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OpenStage {
    /// Taking every strip's inserts out.
    Clear,
    /// Putting the project's in.
    Fill,
    /// Waiting for the last edits, then the rest.
    Finish,
}

impl MixHost {
    /// Made before the sound starts (it allocates). `level_db` is the
    /// Level now, `max_db` the most it goes to.
    pub fn new(rate: u32, level_db: f32, max_db: f32) -> MixHost {
        let cfg = MixerConfig {
            inputs: 1,
            pads: PAD_STRIPS,
            // The engines' tracks, then one for each sequencer track's own
            // instrument, then one for each free place's (step 6; unused
            // ones cost a few flag checks).
            tracks: TRACKS + instruments::HOLDERS,
            slots: SLOTS,
            max_latency_ms: 50.0,
        };
        let mut mixer = Mixer::new(cfg, rate, PERIOD);
        for p in 0..PADS {
            mixer.set_now(StripId::Pad(p), mx::OUTPUT, output::TRACK);
            mixer.set_now(StripId::Pad(p), mx::OUTPUT_TRACK, DRUMS as f32);
        }
        mixer.set_now(StripId::Master, mx::VOLUME, level_db.min(max_db));
        mixer.set_now(LINE, mx::SOURCE, source::OFF);
        mixer.set_now(LINE, mx::MONITOR, monitor::OFF);
        // The Play page's effects.
        let ret = StripId::Return(DELAY_RETURN);
        let mut echo = Delay::new();
        echo.set_param(delay::MIX, 100.0);
        let inserts: [(StripId, Box<dyn Node>); 4] = [
            (StripId::Track(DRUMS), Box::new(DjFilter::new())),
            (StripId::Track(LOOP), Box::new(DjFilter::new())),
            (ret, Box::new(echo)),
            (StripId::Master, Box::new(mpfree_fx::Perform::new())),
        ];
        for (strip, node) in inserts {
            let insert = Insert::new(node, rate, PERIOD);
            // Room for these is made above (SLOTS), so this never fails.
            let _ = mixer.edit(Edit::Insert {
                strip,
                at: 0,
                insert,
            });
        }
        for t in [DRUMS, LOOP] {
            mixer.set_now(StripId::Track(t), mx::send(DELAY_RETURN), SEND_OFF_DB);
        }
        // What the looper records the pads from, wet (pads_wet).
        let master_in = (StripId::Master, mpfree_fx::CapturePoint::BeforeInserts);
        let _ = mixer.set_capture(LOOP_CAPTURE, Some(master_in));
        for (i, s) in LEFT_OUT.into_iter().enumerate() {
            let _ = mixer.set_capture(
                LOOP_CAPTURE + 1 + i,
                Some((s, mpfree_fx::CapturePoint::AfterFader)),
            );
        }
        MixHost {
            mixer,
            modu: crate::modmix::ModMix::new(rate),
            max_db,
            monitor: false,
            line_open: false,
            overs: 0,
            shown: None,
            last: MixView::Tracks,
            selected: None,
            state_at: None,
            pad_mutes: 0,
            dropped: 0,
            old_pads: None,
            mutes_sent: 0,
            loop_point: master_in,
            audio_points: [None; mpfree_audiotracks::MAX_TRACKS],
            audio_outs: [None; mpfree_audiotracks::MAX_TRACKS],
            loop_echoes: [0; mpfree_fx::RETURNS],
            loop_bus_echo: None,
            xy_effect: None,
            xy_gesture: mpfree_xy::Gesture::default(),
            xy_on: [false; 2],
            xy_seq: 0,
            xy_new: false,
            xy_late: None,
            xy_found_up: 0,
            xy_play: crate::xy_play::XyPlay::new(rate),
            xy_midi_left: 0,
            recording: false,
            waiting: [None; WAITING],
            throw_said: Vec::new(),
            perform_want: Vec::new(),
            perform_added: Vec::new(),
            perform_building: Vec::new(),
            builder: None,
            landing: None,
            plugins: Arc::new(Plugins::none()),
            opening: None,
            instruments: Instruments::new(rate, PERIOD),
            pad_synths: crate::pad_synths::PadSynths::new(rate, PERIOD),
            pad_landed: None,
            kit_tracks: 0,
            audio_tracks: 0,
            homes: [0; KITS],
            pad_channels: PadChannels::default(),
            instrument_time: Duration::ZERO,
            old_first_opens: 0,
            made: Vec::new(),
            plugin_presets: Vec::new(),
            presets_asked: Vec::new(),
            preset_news: Vec::new(),
            shelf: mpfree_fx::picker::Shelf::default(),
            shelf_dirty: false,
            modes: crate::fxmodes::Modes::default(),
            takes: crate::takes::Takes::new(rate),
            here: crate::takes::Here::default(),
            capturing: None,
            captured: None,
        }
    }

    /// Asks for a track's own instrument (`job`): made on the builder
    /// thread and put in by [`MixHost::poll_instruments`]; without a
    /// builder (the tests), made here. False if the builder is busy.
    pub fn build_instrument(&mut self, job: crate::control::InstrumentJob) -> bool {
        let rate = self.mixer.rate();
        if let Some(b) = self.builder.as_mut() {
            return b.instrument(Box::new(job), rate, PERIOD);
        }
        let made = crate::control::make_instrument(job, &self.plugins, rate, PERIOD);
        self.made.push(made);
        true
    }

    /// Once a period: puts in an instrument the builder made, if one is
    /// ready (the old one rings out), and drops retired ones off the audio
    /// thread. Never waits or allocates.
    pub fn poll_instruments(&mut self) -> Option<Landed> {
        while let Some(old) = self.instruments.pop_retired() {
            self.drop_instrument(old);
        }
        while let Some(old) = self.pad_synths.pop_retired() {
            self.drop_instrument(old);
        }
        let m = match self.builder.as_mut() {
            Some(b) => b.take_instrument()?,
            None => self.made.pop()?,
        };
        // A synth-voice pad's copy goes to the pads' synths.
        if let Some(pad) = m.pad {
            self.pad_landed = Some(match m.made {
                None => {
                    self.pad_synths.failed(pad);
                    PadLanded::Failed { pad }
                }
                Some((inst, held)) => match self.pad_synths.put(pad, inst, &m.tuning) {
                    Ok(()) => PadLanded::Playing {
                        pad,
                        name: held.name,
                    },
                    Err(back) => {
                        self.drop_instrument(back);
                        PadLanded::Unwanted { pad }
                    }
                },
            });
            return None;
        }
        let Some((inst, held)) = m.made else {
            return Some(Landed::Failed {
                track: m.track,
                id: m.id,
            });
        };
        match self
            .instruments
            .put(m.track, (inst, held), m.pads, &m.tuning)
        {
            Ok(()) => Some(Landed::Playing {
                track: m.track,
                name: held.name,
            }),
            Err((why, back)) => {
                if let Some(i) = back {
                    self.drop_instrument(i);
                }
                let track = m.track;
                Some(match why {
                    instruments::Refused::Full => Landed::Full {
                        track,
                        name: held.name,
                    },
                    _ => Landed::Failed { track, id: m.id },
                })
            }
        }
    }

    /// What became of the last synth-voice pad's copy the builder made,
    /// once.
    pub fn take_pad_landed(&mut self) -> Option<PadLanded> {
        self.pad_landed.take()
    }

    /// Where pad `pad` of kit `kit` sounds, as the sampler's pads do
    /// ([`Pads`]): a channel lent to it, a pad channel by its place for a
    /// kit no track is home to, else its kit's track
    /// ([`MixHost::kit_track`]). Never allocates.
    pub fn pad_dest(&self, kit: usize, pad: usize) -> crate::pad_synths::Dest {
        use crate::pad_synths::Dest;
        if kit > 0 || pad >= PADS {
            if let Some(StripId::Pad(s)) = self.pad_channels.channel(kit, pad) {
                return Dest::Pad(s);
            }
        }
        if self.homes.get(kit).copied().unwrap_or(0) == 0 && pad < PADS {
            return Dest::Pad(pad);
        }
        Dest::Track(self.kit_track(kit))
    }

    /// Plugin synth `id`'s own presets, once listed (None until then, or if
    /// they couldn't be).
    pub fn plugin_presets(&self, id: &str) -> Option<&[mpfree_plughost::Preset]> {
        self.plugin_presets
            .iter()
            .find(|(i, _)| *i == id)
            .and_then(|(_, l)| l.as_deref().ok())
    }

    /// Asks for plugin synth `id`'s presets, once: listed on the builder
    /// thread (the first time in a process of their own, which can take
    /// seconds) and kept when [`MixHost::poll_presets`] hands them back.
    pub fn ask_presets(&mut self, id: &'static str) {
        if !Plugins::is_plugin(id)
            || self.presets_asked.contains(&id)
            || self.plugin_presets.iter().any(|(i, _)| *i == id)
        {
            return;
        }
        let asked = match self.builder.as_mut() {
            Some(b) => b.presets(id),
            None => {
                let list = self.plugins.presets(id);
                self.preset_news
                    .push(crate::control::PresetNews::Listed { id, list });
                true
            }
        };
        if asked {
            self.presets_asked.push(id);
        }
    }

    /// Asks for preset `index` of track `track`'s plugin synth to be
    /// loaded (on the builder thread); [`MixHost::poll_presets`] says how
    /// it went. False if the track has no plugin, its presets aren't
    /// listed, there's no such preset, or the builder is busy.
    pub fn load_plugin_preset(&mut self, track: usize, index: usize) -> bool {
        let Some(held) = self.instruments.held(track) else {
            return false;
        };
        let Some(preset) = self
            .plugin_presets(held.id)
            .and_then(|l| l.get(index))
            .cloned()
        else {
            return false;
        };
        let job = Box::new(crate::control::PresetJob {
            track,
            instance: held.instance,
            index,
            preset,
        });
        match self.builder.as_mut() {
            Some(b) => b.load_preset(job),
            None => {
                let news = job.run(&self.plugins);
                self.preset_news.push(news);
                true
            }
        }
    }

    /// Asks for track `track`'s plugin's values and texts, for its skin (on
    /// the builder thread); [`MixHost::poll_presets`] brings them. False if
    /// the track has no plugin or the builder is busy.
    pub fn ask_values(&mut self, track: usize) -> bool {
        let Some(held) = self.instruments.held(track) else {
            return false;
        };
        if !Plugins::is_plugin(held.id) {
            return false;
        }
        let instance = held.instance;
        match self.builder.as_mut() {
            Some(b) => b.values(track, instance),
            None => {
                let values = self.plugins.values(instance);
                self.preset_news.push(crate::control::PresetNews::Values {
                    track,
                    instance,
                    values,
                });
                true
            }
        }
    }

    /// Once a period: presets listed or loaded on the builder thread, if
    /// any are back (a listed plugin's are kept for
    /// [`MixHost::plugin_presets`]). Never waits.
    pub fn poll_presets(&mut self) -> Option<crate::control::PresetNews> {
        let news = match self.builder.as_mut() {
            Some(b) => b.take_preset_news(),
            None => (!self.preset_news.is_empty()).then(|| self.preset_news.remove(0)),
        }?;
        if let crate::control::PresetNews::Listed { id, list } = &news {
            self.presets_asked.retain(|i| i != id);
            self.plugin_presets.push((id, list.clone()));
        }
        Some(news)
    }

    fn drop_instrument(&mut self, inst: mpfree_track::Instrument) {
        match self.builder.as_mut() {
            Some(b) => {
                b.drop_instrument_later(inst);
            }
            None => drop(inst),
        }
    }

    /// Starts swapping in a project's mixer (built on the screen thread:
    /// `project::mixer_load`). [`MixHost::tend_open`] does it, a few edits
    /// a period; until then the old mixer plays on. A second open replaces
    /// one under way (its effects are dropped off this thread).
    pub fn open(&mut self, load: Box<crate::project::MixLoad>) {
        // Frozen strips are not saved: everything thaws.
        let mut mem = TakeMem {
            builder: self.builder.as_mut(),
            here: &mut self.here,
        };
        self.takes.clear(&mut self.mixer, &mut mem);
        if let Some(mut old) = self.opening.take() {
            self.drop_load(&mut old.load);
        }
        // Its inserts may reuse the old ones' slot ids.
        self.modes = crate::fxmodes::Modes::default();
        // Nothing the last project held stays held: every hand, step lock
        // and move lets go, and the XY pad with them.
        self.modu.let_go_all();
        self.xy_gesture.clear();
        self.xy_on = [false; 2];
        self.xy_late = None;
        self.xy_midi_left = 0;
        let mut reach = crate::xy_play::ModReach::new(&mut self.modu, &self.mixer, false);
        self.xy_play.cancel(&mut reach);
        // An old project's pad mutes, for the tracks to take as it opens.
        self.old_pads = Some((load.old_pad_mutes, load.old_pad_solos)).filter(|&(m, s)| m | s != 0);
        self.opening = Some(Box::new(Open {
            load,
            stage: OpenStage::Clear,
        }));
    }

    /// Whether a project's mixer is still going in.
    pub fn opening(&self) -> bool {
        self.opening.is_some()
    }

    /// The effects of a load not used, dropped off the audio thread.
    fn drop_load(&mut self, load: &mut crate::project::MixLoad) {
        for (_, chain) in load.chains.drain(..) {
            for (ins, _) in chain {
                let _ = self.drop_off_thread(ins);
            }
        }
        // The rest is small (a few vectors): dropped here.
    }

    /// Once a period while [`MixHost::opening`]: the next edits of the
    /// swap. Returns a line for the log when it is done. Edits never wait:
    /// one the mixer has no room for is made on a later period. Allocates
    /// only at the end, when the mod engine reads its part.
    pub fn tend_open(&mut self) -> Option<String> {
        let mut open = self.opening.take()?;
        if self.mixer.edits_pending() > 0 {
            self.opening = Some(open);
            return None;
        }
        match open.stage {
            OpenStage::Clear => {
                let mut left = false;
                for k in 0..self.mixer.strips().count() {
                    let st = self.strip_at(k);
                    let n = self.mixer.chain(st).map_or(0, |c| c.len());
                    for _ in 0..n {
                        if self.mixer.edit(Edit::Remove { strip: st, at: 0 }).is_err() {
                            left = true;
                            break;
                        }
                    }
                }
                if !left {
                    open.stage = OpenStage::Fill;
                }
            }
            OpenStage::Fill => {
                let mut left = false;
                let load = &mut *open.load;
                for (st, chain) in load.chains.iter_mut() {
                    while !chain.is_empty() {
                        let at = self.mixer.chain(*st).map_or(0, |c| c.len());
                        let (ins, key) = chain.remove(0);
                        match self.mixer.edit(Edit::Insert {
                            strip: *st,
                            at,
                            insert: ins,
                        }) {
                            Ok(()) => {
                                // Its key once it is in (Finish).
                                if let Some(k) = key {
                                    load.keys.push((*st, at, k));
                                }
                            }
                            Err(Edit::Insert { insert, .. }) => {
                                chain.insert(0, (insert, key));
                                left = true;
                                break;
                            }
                            Err(_) => {}
                        }
                    }
                    if left {
                        break;
                    }
                }
                if !left {
                    open.stage = OpenStage::Finish;
                }
            }
            OpenStage::Finish => {
                let load = &open.load;
                for &(st, ref values) in &load.strips {
                    if self.mixer.chain(st).is_none() && !matches!(st, StripId::Master) {
                        // A strip this mixer hasn't got.
                        if self.mixer.strips().all(|s| s != st) {
                            continue;
                        }
                    }
                    for &(id, v) in values {
                        self.mixer.set_now(st, id, v);
                    }
                }
                // A track's own channel or a lent pad channel the project
                // leaves out has nothing set (`project::untouched_channel`):
                // back to its defaults, whatever the last project had there.
                let left_out = (TRACKS..self.mixer.tracks())
                    .map(StripId::Track)
                    .chain((PADS..PAD_STRIPS).map(StripId::Pad));
                for st in left_out {
                    if load.strips.iter().any(|(s, _)| *s == st) {
                        continue;
                    }
                    for p in mpfree_fx::strip_params() {
                        if crate::project::saved_control(st, p.id)
                            && self.mixer.param(st, p.id) != p.default
                        {
                            self.mixer.set_now(st, p.id, p.default);
                        }
                    }
                }
                // A sidechain the mixer refuses is counted and said: the
                // effect then says "Nothing picked" in red, if it listens.
                let lost = load
                    .keys
                    .iter()
                    .filter(|&&(st, slot, k)| !self.mixer.set_key(st, slot, Some(k)))
                    .count();
                // The pads the project lent channels to.
                self.pad_channels.clear();
                for &(st, kit, pad) in &load.lent {
                    self.pad_channels.hold(st, kit, pad);
                }
                self.route_lent();
                self.pad_mutes = load.pad_mutes;
                self.apply_mutes();
                let effects: usize = self
                    .mixer
                    .strips()
                    .map(|s| self.mixer.chain(s).map_or(0, |c| c.len()))
                    .sum();
                if let Some(fx) = &load.note_fx {
                    self.modu.set_note_fx(fx);
                }
                let skipped = match &load.modu {
                    Some(text) => self.modu.load(&self.mixer, text),
                    None => 0,
                };
                // Track 1 plays where the project says (its `track1`
                // line). A project saved before Drums was track 1's strip
                // keeps track 1 on its old own channel when its instrument
                // played there or the channel has settings, so the project
                // sounds as it did (counted, said, and shown as
                // OLD_TRACK_1); otherwise nothing played there, and track 1
                // moves to Drums.
                let settings =
                    !crate::project::untouched_channel(&self.mixer, StripId::Track(TRACKS));
                let old = match load.track1 {
                    Some(st) => st == StripId::Track(TRACKS),
                    None => settings || load.track1_own,
                };
                self.instruments
                    .set_first_channel(if old { TRACKS } else { track_channel(0) });
                let mut line = format!("mixer: the project's mixer is in ({effects} effects)");
                if old {
                    self.old_first_opens += 1;
                    let why = match (load.track1, load.track1_own) {
                        (Some(_), _) => "saved so",
                        (None, true) => "its instrument played there",
                        (None, false) => "the project has settings on it",
                    };
                    line += &format!(
                        "; track 1 keeps its old own channel (\"{OLD_TRACK_1}\": {why}), not Drums"
                    );
                }
                if skipped > 0 {
                    line += &format!("; {skipped} mod lines skipped");
                }
                if lost > 0 {
                    line += &format!("; {lost} sidechains not restored");
                }
                return Some(line);
            }
        }
        self.opening = Some(open);
        None
    }

    fn strip_at(&self, k: usize) -> StripId {
        self.mixer.strips().nth(k).unwrap_or(StripId::Master)
    }

    /// One period of silence through the mixer (tests: edits land).
    #[cfg(test)]
    pub fn run_silent_period(&mut self) {
        let (mut l, mut r) = ([0f32; PERIOD], [0f32; PERIOD]);
        let ctx = Ctx::new(self.mixer.rate(), 0);
        self.mixer.process(&ctx, &mut l, &mut r);
        while let Some(old) = self.mixer.pop_retired() {
            drop(old);
        }
    }

    /// The plugins the chain editor offers and the builder makes. Before
    /// [`MixHost::build_off_thread`], which hands them to the builder.
    pub fn set_plugins(&mut self, plugins: Arc<Plugins>) {
        self.plugins = plugins;
    }

    pub fn plugins(&self) -> &Arc<Plugins> {
        &self.plugins
    }

    /// From now on effects added in the chain editor are built on the
    /// `mpfree-build` thread, and the effects edits take out are dropped
    /// there (control.rs). Allocates and starts the thread: call it before
    /// the loop.
    pub fn build_off_thread(&mut self) -> std::io::Result<()> {
        self.builder = Some(crate::control::Builder::start(Arc::clone(&self.plugins))?);
        Ok(())
    }

    /// [`MixHost::add_insert_with`] at the effect's defaults (the tests).
    #[cfg(test)]
    pub fn add_insert(&mut self, strip: StripId, id: &'static str) -> Result<(), &'static str> {
        self.add_insert_with(strip, id, &[])
    }

    /// Adds effect `id` at the end of `strip`'s chain, with the values in
    /// `preset` set before it goes in (a factory preset from the picker;
    /// empty for its defaults). With a builder it is built there and goes
    /// in a few periods later ([`MixHost::poll_built`]); without one, here.
    /// Err: the mixer is busy (too many edits or builds under way), or no
    /// effect has that id.
    pub fn add_insert_with(
        &mut self,
        strip: StripId,
        id: &'static str,
        preset: &'static [(u16, f32)],
    ) -> Result<(), &'static str> {
        let rate = self.mixer.rate();
        if let Some(b) = self.builder.as_mut() {
            return if b.build_with(strip, id, preset, rate, PERIOD) {
                Ok(())
            } else {
                Err("The mixer is busy")
            };
        }
        let mut node = if Plugins::is_plugin(id) {
            self.plugins
                .make(id)
                .map_err(|_| "The plugin would not start")?
        } else {
            mpfree_fx_ui::make(id).ok_or("No such effect")?
        };
        for &(p, v) in preset {
            node.set_param(p, v);
        }
        wet_on_a_return(strip, node.as_mut());
        let at = self.mixer.chain(strip).map_or(0, |c| c.len());
        self.mixer
            .edit(Edit::Insert {
                strip,
                at,
                insert: Insert::new(node, rate, PERIOD),
            })
            .map_err(|_| "The mixer is busy")
    }

    /// Switches insert `uid` on `strip` to engine `id` across engines (a
    /// family's mode, `mpfree_fx::families`): built like an added effect,
    /// set up by `switch`, and swapped in place with the same slot id
    /// ([`MixHost::poll_built`]). Err: the mixer is busy, or no effect has
    /// that id.
    pub fn replace_insert(
        &mut self,
        strip: StripId,
        uid: u16,
        id: &'static str,
        switch: mpfree_fx::families::Switch,
    ) -> Result<(), &'static str> {
        let rate = self.mixer.rate();
        if let Some(b) = self.builder.as_mut() {
            return if b.build_switch(strip, uid, id, switch, rate, PERIOD) {
                Ok(())
            } else {
                Err("The mixer is busy")
            };
        }
        let mut node = mpfree_fx_ui::make(id).ok_or("No such effect")?;
        switch.apply(node.as_mut());
        let at = self
            .mixer
            .chain(strip)
            .and_then(|c| c.slot_of(uid))
            .ok_or("That effect has gone")?;
        let insert = self.keep_bypass(strip, at, Insert::new(node, rate, PERIOD).with_uid(uid));
        self.mixer
            .edit(Edit::Replace { strip, at, insert })
            .map_err(|_| "The mixer is busy")
    }

    /// An insert taking the place of slot `at` of `strip`, bypassed if the
    /// one there is.
    fn keep_bypass(&self, strip: StripId, at: usize, insert: Insert) -> Insert {
        match self.mixer.chain(strip) {
            Some(c) if !c.is_on(at) => insert.off(),
            _ => insert,
        }
    }

    /// Effects asked for and not in the mixer's edits yet.
    pub fn inserts_building(&self) -> usize {
        self.builder.as_ref().map_or(0, |b| b.pending()) + usize::from(self.landing.is_some())
    }

    /// Once a period: puts the effects the builder made into their chains
    /// (at the end, or in the place of the insert a switch of mode
    /// replaces), and hands the ones edits took out back to be dropped
    /// off the audio thread; then a switch's macros and rows follow it
    /// (`fxmodes::land`). Returns a line for the log when an effect
    /// could not be made. No allocation while nothing is edited; never
    /// waits.
    pub fn poll_built(&mut self) -> Option<&'static str> {
        let mut said = None;
        loop {
            let built = match self.landing.take() {
                Some(b) => b,
                None => match self.builder.as_mut().and_then(|b| b.take()) {
                    Some(b) => b,
                    None => break,
                },
            };
            let Some(insert) = built.insert else {
                said = Some("mixer: the chain editor asked for an effect that has no maker");
                continue;
            };
            // A switch of mode takes its insert's slot, and its slot id;
            // an added effect goes at the end.
            let at = self
                .mixer
                .chain(built.strip)
                .and_then(|c| match built.replace {
                    Some(uid) => c.slot_of(uid),
                    None => Some(c.len()),
                });
            let Some(at) = at else {
                said = Some(match built.replace {
                    Some(_) => "mixer: an effect was switched after it was taken out",
                    None => "mixer: an effect was built for a strip the mixer does not have",
                });
                if !self.drop_off_thread(insert) {
                    said = Some(DROPPED_HERE);
                }
                continue;
            };
            let edit = match built.replace {
                Some(uid) => Edit::Replace {
                    strip: built.strip,
                    at,
                    insert: self.keep_bypass(built.strip, at, insert.with_uid(uid)),
                },
                None => Edit::Insert {
                    strip: built.strip,
                    at,
                    insert,
                },
            };
            if let Err(Edit::Insert { insert, .. } | Edit::Replace { insert, .. }) =
                self.mixer.edit(edit)
            {
                // The mixer's edits are full: it goes in on a later period.
                self.landing = Some(crate::control::Built {
                    strip: built.strip,
                    id: built.id,
                    insert: Some(insert),
                    replace: built.replace,
                });
                break;
            }
        }
        while let Some(old) = self.mixer.pop_retired() {
            if !self.drop_off_thread(old) {
                said = Some(DROPPED_HERE);
            }
        }
        crate::fxmodes::land(self);
        let unwanted = self
            .perform_added
            .iter()
            .any(|s| !self.perform_want.contains(s));
        if !self.perform_building.is_empty() || unwanted {
            self.play_waiting();
            self.take_off_unwanted();
        }
        said
    }

    /// Drops `insert` on the builder's thread, or here without one (the
    /// tests). False if a builder's queue was full and it was dropped here.
    fn drop_off_thread(&mut self, insert: Insert) -> bool {
        match self.builder.as_mut() {
            Some(b) => b.drop_later(insert),
            None => {
                drop(insert);
                true
            }
        }
    }

    /// A change from the Play page.
    pub fn play(&mut self, fx: PlayFx) {
        match fx {
            PlayFx::Strip { strip, id, value } => {
                self.set(strip, id, value);
            }
            PlayFx::Filter { strip, position } => {
                self.insert(strip, FILTER_SLOT, dj_filter::POSITION, position);
            }
            PlayFx::Throw { fx, down, how } => {
                let on = how.map_or(StripId::Master, |h| h.on);
                match self.perform_slot(on) {
                    Some(slot) => self.throw(slot, fx, down, how),
                    None => self.throw_waits(fx, down, how.unwrap_or_default()),
                }
            }
            PlayFx::Macro { card, value } => {
                let addr = ParamAddr::macro_value(card.min(7));
                self.modu.lock(&self.mixer, 0, addr, value);
            }
            // A learned control's (the screen's comes through
            // `play_screen`).
            PlayFx::Xy { x, y, down } => self.xy(mpfree_xy::Device::Midi, x, y, down),
            PlayFx::XyEffect(pick) => {
                use mpfree_fx::xyfx::ids as xy;
                if self.xy_play.playing() {
                    self.xy_yours(None);
                }
                let pick = pick.filter(|&k| k < XY_START.len());
                self.xy_effect = pick;
                match (pick, self.xy_slot()) {
                    (Some(k), Some(slot)) => {
                        self.insert(StripId::Master, slot, xy::EFFECT, k as f32);
                        self.insert(StripId::Master, slot, xy::ACTIVE, 0.0);
                    }
                    (Some(k), None) => {
                        // Busy: the next pick or start tries again.
                        let _ = self.add_insert_with(StripId::Master, XY_FX, &XY_START[k]);
                    }
                    // Back to the DJ filter and echo: the XY FX closes, on
                    // its Filter, an insert that doesn't run while closed
                    // (a closed echo or wash keeps running to ring out).
                    (None, Some(slot)) => {
                        self.insert(StripId::Master, slot, xy::ACTIVE, 0.0);
                        self.insert(StripId::Master, slot, xy::EFFECT, 0.0);
                    }
                    (None, None) => {}
                }
            }
            PlayFx::PadMute { pad, mute } if pad < PADS => {
                if mute {
                    self.pad_mutes |= 1 << pad;
                } else {
                    self.pad_mutes &= !(1 << pad);
                }
                self.apply_mutes();
            }
            PlayFx::PadMute { .. } => {}
            PlayFx::Drop(pads) => {
                self.dropped = pads;
                self.apply_mutes();
            }
        }
    }

    /// `device` (the screen, or a learned control's XY across and up) put
    /// the XY pad at `x`, `y`, held or not. Its own gesture begins with the
    /// first held move and ends with the first not held; the gesture that
    /// owns the pad plays (`mpfree_xy::Gesture`). A move not held outside
    /// a gesture is the pad gliding home after a lift: it plays while no
    /// gesture holds the pad.
    fn xy(&mut self, device: mpfree_xy::Device, x: f32, y: f32, down: bool) {
        use mpfree_xy::{Phase, Step, XyInput};
        if self.xy_play.playing() {
            self.xy_on_yours(device, x, y, down);
            return;
        }
        let d = usize::from(device != mpfree_xy::Device::Screen);
        let was = self.xy_on[d];
        if !down && !was {
            if self.xy_gesture.held() {
                return;
            }
            if self.xy_new {
                // A lift waits for its press's period: this takes its place.
                self.xy_late = Some((x, y));
                return;
            }
            self.xy_late = None;
            self.play_xy(x, y, false);
            return;
        }
        self.xy_on[d] = down;
        let phase = match (was, down) {
            (false, true) => Phase::Begin,
            (true, true) => Phase::Move,
            _ => Phase::End,
        };
        self.xy_seq = self.xy_seq.wrapping_add(1);
        let input = XyInput::new(device, 0, self.xy_seq, 0, phase, x, y);
        match self.xy_gesture.input(&input) {
            // The other controller's gesture holds the pad.
            None => {}
            Some(Step::Begin { x, y }) => {
                self.xy_late = None;
                self.xy_new = true;
                self.play_xy(x, y, true);
            }
            Some(Step::Move { x, y }) => {
                // Anything newer takes a waiting lift's place.
                self.xy_late = None;
                self.play_xy(x, y, true);
            }
            Some(Step::End { x, y }) if self.xy_new => {
                // Pressed and lifted before a period played it (a tap
                // quicker than a period): it plays for one period and
                // lets go after it.
                self.xy_late = Some((x, y));
            }
            Some(Step::End { x, y }) => {
                self.xy_late = None;
                self.play_xy(x, y, false);
            }
        }
    }

    /// The XY pad on a layout of yours (`xy_play.rs`): `device` put it at
    /// `x`, `y`, held or not. A learned control's moves each hold the pad,
    /// and its gesture ends `HAND_S` after its last move (it has no lift
    /// of its own); the screen's lift is its own. The gesture that owns
    /// the pad plays, as for the ready-made layouts.
    fn xy_on_yours(&mut self, device: mpfree_xy::Device, x: f32, y: f32, down: bool) {
        use mpfree_xy::{Phase, Step, XyInput};
        let midi = device != mpfree_xy::Device::Screen;
        let down = if midi && down {
            let rate = self.modu.engine().rate();
            self.xy_midi_left = crate::xy_play::frames(rate, crate::locks::HAND_S * 1000.0);
            true
        } else {
            down
        };
        let d = usize::from(midi);
        let was = self.xy_on[d];
        if !down && !was {
            // Not held outside a gesture (a glide home): a layout of yours
            // has none.
            return;
        }
        self.xy_on[d] = down;
        let phase = match (was, down) {
            (false, true) => Phase::Begin,
            (true, true) => Phase::Move,
            _ => Phase::End,
        };
        self.xy_seq = self.xy_seq.wrapping_add(1);
        let input = XyInput::new(device, 0, self.xy_seq, 0, phase, x, y);
        let step = self.xy_gesture.input(&input);
        let recording = self.recording;
        let mut reach = crate::xy_play::ModReach::new(&mut self.modu, &self.mixer, recording);
        match step {
            None => {}
            Some(Step::Begin { x, y }) => {
                // The other controller's gesture lost the pad: this one
                // takes it, from where it is.
                if self.xy_play.down() {
                    self.xy_play.end(&mut reach, recording);
                }
                self.xy_play.begin(&mut reach, x, y, |a| a)
            }
            Some(Step::Move { x, y }) => self.xy_play.moved(&mut reach, x, y),
            Some(Step::End { x, y }) => {
                self.xy_play.moved(&mut reach, x, y);
                self.xy_play.end(&mut reach, recording);
            }
        }
    }

    /// A learned control's gesture on a layout of yours ends: still for
    /// `HAND_S` (it has no lift of its own).
    fn xy_midi_ends(&mut self) {
        use mpfree_xy::{Phase, Step, XyInput};
        self.xy_on[1] = false;
        self.xy_seq = self.xy_seq.wrapping_add(1);
        let (x, y) = self.xy_play.position();
        let input = XyInput::new(mpfree_xy::Device::Midi, 0, self.xy_seq, 0, Phase::End, x, y);
        if let Some(Step::End { .. }) = self.xy_gesture.input(&input) {
            let recording = self.recording;
            let mut reach = crate::xy_play::ModReach::new(&mut self.modu, &self.mixer, recording);
            self.xy_play.end(&mut reach, recording);
        }
    }

    /// The XY pad plays `layout`, a layout of yours (None: back to the
    /// ready-made one picked). Whatever the pad held lets go first, at
    /// once: a ready-made layout's finger lifts (an XY FX closes), and a
    /// layout of yours lets go of everything, even with Hold on.
    pub fn xy_yours(&mut self, layout: Option<mpfree_xy::Compiled>) {
        if !self.xy_play.playing() && self.xy_gesture.held() {
            let (x, y) = self.xy_at();
            self.play_xy(x, y, false);
        }
        self.xy_gesture.clear();
        self.xy_on = [false; 2];
        self.xy_late = None;
        self.xy_midi_left = 0;
        let mut reach = crate::xy_play::ModReach::new(&mut self.modu, &self.mixer, self.recording);
        self.xy_play.load(&mut reach, layout);
    }

    /// Hold on the XY pad (a layout of yours): while on, a lift leaves the
    /// pad in charge of its settings; turned off, the pad lets go as its
    /// layout says.
    // The XY page's Hold and Stay's UNDO (the screens' pull request).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn xy_hold(&mut self, on: bool) {
        let recording = self.recording;
        let mut reach = crate::xy_play::ModReach::new(&mut self.modu, &self.mixer, recording);
        self.xy_play.set_hold(&mut reach, on, recording);
    }

    /// Whether a track records: the XY pad's hand beats a step's lock
    /// then, and Stay keeps no value (`main.rs`, each loop).
    pub fn set_recording(&mut self, recording: bool) {
        self.recording = recording;
    }

    /// Stay's turns on a layout of yours since the last call, as one UNDO
    /// step: each setting, its own value before and after.
    // The XY page's Hold and Stay's UNDO (the screens' pull request).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn take_xy_kept(&mut self) -> std::vec::Drain<'_, (ParamAddr, f32, f32)> {
        self.xy_play.take_kept()
    }

    /// The XY pad on a layout of yours as the sound loop has it (the
    /// tests').
    #[cfg(test)]
    pub fn xy_play(&self) -> &crate::xy_play::XyPlay {
        &self.xy_play
    }

    /// The XY pad's layout as it plays now (`xy_layouts.rs`): the XY FX
    /// picked, or the Perform FX's DJ filter and echo, on its insert's
    /// slot id. `None` while the XY FX is still being put on.
    fn xy_layout(&self) -> Option<mpfree_xy::Compiled> {
        let slot = match self.xy_effect {
            Some(_) => self.xy_slot()?,
            None => PERFORM_SLOT,
        };
        let uid = self.mixer.chain(StripId::Master)?.uid(slot)?;
        Some(crate::xy_layouts::compiled(self.xy_effect, uid))
    }

    /// The XY pad at `x`, `y`, held or not: its layout's settings set (a
    /// pad still being put on plays from the next move).
    fn play_xy(&mut self, x: f32, y: f32, down: bool) {
        let Some(layout) = self.xy_layout() else {
            return;
        };
        layout.values(x, y, down, |addr, value| self.set_insert(addr, value));
    }

    /// An insert's setting set as a turn sets it, found by its slot id.
    fn set_insert(&mut self, addr: ParamAddr, value: f32) {
        let Owner::Insert(s, uid) = addr.owner else {
            return;
        };
        let sid = crate::modmix::strip_id(s);
        if let Some(slot) = self.mixer.chain(sid).and_then(|c| c.slot_of(uid)) {
            self.insert(sid, slot, addr.param, value);
        }
    }

    /// An insert's setting as it is now, found by its slot id.
    fn insert_param(&self, addr: ParamAddr) -> Option<f32> {
        let Owner::Insert(s, uid) = addr.owner else {
            return None;
        };
        let c = self.mixer.chain(crate::modmix::strip_id(s))?;
        Some(c.node(c.slot_of(uid)?)?.param(addr.param))
    }

    /// A change from the screen's Play page: as [`MixHost::play`], with
    /// the XY pad's moves the screen's.
    pub fn play_screen(&mut self, fx: PlayFx) {
        match fx {
            PlayFx::Xy { x, y, down } => self.xy(mpfree_xy::Device::Screen, x, y, down),
            fx => self.play(fx),
        }
    }

    /// The screen's XY pad as it stands (`xy_slot`), read once a period
    /// after the screen's actions: if the mixer holds the pad for the
    /// screen but the finger is up there, the pad lets go now, its lift
    /// action not having come. True when it did.
    pub fn screen_xy(&mut self, s: crate::xy_slot::XyState) -> bool {
        if !self.xy_on[0] || s.down {
            return false;
        }
        self.xy_found_up += 1;
        self.xy(mpfree_xy::Device::Screen, s.x, s.y, false);
        true
    }

    /// Lifts the screen's pad state did for its lost lift actions.
    pub fn xy_found_up(&self) -> u64 {
        self.xy_found_up
    }

    /// The XY pad's line for the status: gestures one controller's new
    /// gesture took the pad from, inputs ignored (another touch held it),
    /// inputs out of order, and lifts found from the screen's state.
    pub fn describe_xy(&self) -> String {
        let g = &self.xy_gesture;
        format!(
            "XY pad: {} gestures taken by the other controller's, {} moves ignored (another touch held it), \
             {} out of order, {} lifts found from the screen's state; {}",
            g.taken,
            g.ignored,
            g.stale,
            self.xy_found_up,
            self.xy_play.describe()
        )
    }

    fn insert(&mut self, strip: StripId, slot: usize, id: u16, value: f32) {
        let addr = crate::modmix::strip_of(strip).and_then(|s| {
            let uid = self.mixer.chain(strip)?.uid(slot)?;
            Some(ParamAddr::new(Owner::Insert(s, uid), id))
        });
        if addr.is_some_and(|a| self.modu.set(a, value)) {
            return;
        }
        self.mixer
            .queue_insert(strip, slot, Timed::new(0, NodeEvent::Param { id, value }));
    }

    /// Whether throw `fx` holds its Perform FX pad on the master now (the
    /// tests').
    #[cfg(test)]
    pub fn throw_held(&self, fx: ThrowFx) -> bool {
        self.mixer
            .chain(StripId::Master)
            .and_then(|c| c.node(PERFORM_SLOT))
            .is_some_and(|n| n.param(perf::hold(fx.pad())) > 0.5)
    }

    /// A parameter of the master's Perform FX as it is now.
    #[cfg(test)]
    fn perform_param(&self, id: u16) -> Option<f32> {
        self.perform_param_on(StripId::Master, id)
    }

    /// A parameter of strip `on`'s Perform FX as it is now.
    fn perform_param_on(&self, on: StripId, id: u16) -> Option<f32> {
        let slot = self.perform_slot(on)?;
        self.mixer
            .chain(on)
            .and_then(|c| c.node(slot))
            .map(|n| n.param(id))
    }

    /// Where the XY pad stands, 0 to 1 across and up, as its effect has
    /// it: the XY FX's X and Y while the pad plays one (where it starts
    /// while it is being put on), else the Perform FX's DJ filter across
    /// and echo up. For MIDI Learn's XY across and up, which move one way
    /// and leave the other where it is.
    pub fn xy_at(&self) -> (f32, f32) {
        if self.xy_play.playing() {
            return self.xy_play.position();
        }
        let rest = crate::xy_layouts::pad(self.xy_effect).rest();
        let Some(layout) = self.xy_layout() else {
            return rest;
        };
        let (x, y) = layout.position(|a| self.insert_param(a));
        (x.unwrap_or(rest.0), y.unwrap_or(rest.1))
    }

    /// Throw `fx`'s settings on strip `on`'s Perform FX, for the Perform
    /// view's Q-Links: the pad that plays it there, its Effect, Length and
    /// Amount, and the insert's slot id (`None`, with zeros, while the
    /// strip has no Perform FX: still going in, or taken off on the Mix
    /// tab).
    pub fn throw_knobs(&self, on: StripId, fx: ThrowFx) -> ThrowKnobs {
        let pad = fx.pad();
        let uid = self
            .perform_slot(on)
            .and_then(|slot| self.mixer.chain(on)?.uid(slot));
        let v = |o: u16| match uid {
            Some(_) => self
                .perform_param_on(on, perf::slot(pad) + o)
                .unwrap_or(0.0),
            None => 0.0,
        };
        ThrowKnobs {
            on,
            fx,
            uid,
            effect: v(perf::EFFECT).round().max(0.0) as u8,
            length: v(perf::LENGTH),
            amount: v(perf::AMOUNT),
        }
    }

    /// Strip `on`'s Perform FX: the first on its chain (the master's own
    /// is its first insert), if it has one.
    pub fn perform_slot(&self, on: StripId) -> Option<usize> {
        let c = self.mixer.chain(on)?;
        (0..c.len()).find(|&i| c.node(i).is_some_and(|n| n.info().id == PERFORM_FX))
    }

    /// Throw `fx` held (`down`) or let go on the Perform FX in `slot` of
    /// its strip (`how.on`; the master's for Play's cards). It holds the
    /// Perform FX pad the FX thread laid out for it in the default bank.
    /// A Perform pad's settings go on the pad for the hit (the node reads
    /// Starts and Plays on the press, Let go on the release, in the order
    /// they come) and the pad's own come back after.
    fn throw(&mut self, slot: usize, fx: ThrowFx, down: bool, how: Option<ThrowHow>) {
        let on = how.map_or(StripId::Master, |h| h.on);
        let k = fx.pad();
        let own = how.map(|h| {
            let set = [
                (perf::starts(k), f32::from(h.starts)),
                (perf::let_go(k), if h.cut { 1.0 } else { 0.0 }),
                (perf::plays(k), if h.bar { 1.0 } else { 0.0 }),
            ];
            let own = set.map(|(id, _)| (id, self.perform_param_on(on, id).unwrap_or(0.0)));
            for (id, v) in set {
                self.insert(on, slot, id, v);
            }
            own
        });
        self.insert(on, slot, perf::hold(k), if down { 1.0 } else { 0.0 });
        for (id, v) in own.into_iter().flatten() {
            self.insert(on, slot, id, v);
        }
    }

    /// A throw on a track or a submix with no Perform FX there: while the
    /// one its On put on is still being built, it waits and plays when it
    /// lands (let go before that, a held throw doesn't play at all; a
    /// bar's still does). With none coming, it doesn't play: a throw never
    /// changes the mix, picking On does ([`MixHost::perform_strips`]).
    fn throw_waits(&mut self, fx: ThrowFx, down: bool, how: ThrowHow) {
        let on = how.on;
        let same = |w: &Option<Waiting>| w.is_some_and(|w| w.how.on == on && w.fx == fx);
        if !down {
            if !how.bar {
                for w in self.waiting.iter_mut().filter(|w| same(w)) {
                    *w = None;
                }
            }
            return;
        }
        if !self.perform_building.contains(&on) {
            self.throw_said.push(format!(
                "perform: {} has no Perform FX for the throw: pick the pad's On again on the \
                 Pads tab",
                name(on)
            ));
            return;
        }
        let held = Waiting { fx, how };
        match self.waiting.iter_mut().find(|w| same(w) || w.is_none()) {
            Some(w) => *w = Some(held),
            None => {
                self.waiting.rotate_left(1);
                self.waiting[WAITING - 1] = Some(held);
            }
        }
    }

    /// The tracks and submixes the Perform set's throws play on (`want`),
    /// after the set changed: each gets a Perform FX at the end of its
    /// chain if it has none (it shows on the Mix tab, is saved with the
    /// project and can be taken off there), and one this set put on that
    /// no throw plays on any more comes off again, so stepping a pad's On
    /// through the strips leaves nothing behind. One that was there
    /// already, or that a project brought, stays. `opened`: a project's
    /// set, whose mixer brings its own: nothing is put on or taken off.
    pub fn perform_strips(&mut self, want: &[StripId], opened: bool) {
        self.perform_want = want.to_vec();
        if opened {
            self.perform_added.clear();
            self.perform_building.clear();
            return;
        }
        for &on in want {
            if self.perform_slot(on).is_some() || self.perform_building.contains(&on) {
                continue;
            }
            if self.mixer.chain(on).map_or(SLOTS, |c| c.len()) >= SLOTS {
                self.throw_said.push(format!(
                    "perform: {} has no free effect slot for the throw's Perform FX",
                    name(on)
                ));
                continue;
            }
            match self.add_insert_with(on, PERFORM_FX, &[]) {
                Ok(()) => {
                    self.perform_building.push(on);
                    self.perform_added.push(on);
                    self.throw_said.push(format!(
                        "perform: Perform FX put on {} for the Perform pads' throws (the Mix \
                         tab shows it)",
                        name(on)
                    ));
                }
                Err(e) => self
                    .throw_said
                    .push(format!("perform: {e}: no Perform FX on {}", name(on))),
            }
        }
        self.take_off_unwanted();
    }

    /// The Perform FX the set put on that no throw plays on now, taken off
    /// once they are in (one still being built comes off when it lands).
    /// Allocates only to say what came off.
    fn take_off_unwanted(&mut self) {
        let mut i = 0;
        while i < self.perform_building.len() {
            if self.perform_slot(self.perform_building[i]).is_some() {
                self.perform_building.swap_remove(i);
            } else {
                i += 1;
            }
        }
        let mut i = 0;
        while i < self.perform_added.len() {
            let on = self.perform_added[i];
            if self.perform_want.contains(&on) || self.perform_building.contains(&on) {
                i += 1;
                continue;
            }
            let Some(at) = self.perform_slot(on) else {
                self.perform_added.swap_remove(i);
                continue;
            };
            if self.mixer.edit(Edit::Remove { strip: on, at }).is_err() {
                // The mixer's edits are full: the next poll tries again.
                i += 1;
                continue;
            }
            self.perform_added.swap_remove(i);
            for w in self.waiting.iter_mut() {
                if w.is_some_and(|w| w.how.on == on) {
                    *w = None;
                }
            }
            self.throw_said.push(format!(
                "perform: Perform FX taken off {} (no Perform pad throws there now)",
                name(on)
            ));
        }
    }

    /// The throws held while their Perform FX was being put on, played on
    /// it once it is there.
    fn play_waiting(&mut self) {
        for i in 0..WAITING {
            let Some(w) = self.waiting[i] else {
                continue;
            };
            if let Some(slot) = self.perform_slot(w.how.on) {
                self.waiting[i] = None;
                self.throw(slot, w.fx, true, Some(w.how));
            }
        }
    }

    /// What the Perform set's throws on tracks and submixes said since
    /// last asked.
    pub fn take_throw_said(&mut self) -> Vec<String> {
        std::mem::take(&mut self.throw_said)
    }

    /// The master's XY FX, the first on its chain, if it has one.
    fn xy_slot(&self) -> Option<usize> {
        let c = self.mixer.chain(StripId::Master)?;
        (0..c.len()).find(|&i| c.node(i).is_some_and(|n| n.info().id == XY_FX))
    }

    /// A strip control as the mixer panel set it, for UNDO: a pad's mute
    /// is its own, not Drop drums' held over it.
    pub fn value(&self, strip: StripId, id: u16) -> f32 {
        match strip {
            StripId::Pad(p) if id == mx::MUTE && p < PADS => {
                f32::from(u8::from(self.pad_mutes & (1 << p) != 0))
            }
            _ => self.mixer.param(strip, id),
        }
    }

    /// A strip control as the next period will have it: the last value
    /// given to [`MixHost::set`] since the last period, else as it stands
    /// ([`MixHost::value`]). Steps that come between two periods add up
    /// from it.
    pub fn wanted(&self, strip: StripId, id: u16) -> f32 {
        match strip {
            StripId::Pad(_) => self.value(strip, id),
            _ => self.mixer.wanted(strip, id),
        }
    }

    /// Sets a strip control from the start of the next period: through the
    /// mod engine once a lock has made it a target, else straight to the
    /// mixer.
    pub fn set(&mut self, strip: StripId, id: u16, value: f32) {
        let addr = crate::modmix::strip_of(strip).map(|s| ParamAddr::new(Owner::Strip(s), id));
        if !addr.is_some_and(|a| self.modu.set(a, value)) {
            self.mixer.set(strip, id, value);
        }
    }

    /// A clip's parameter lock (`value`) or its end (`None`), `offset`
    /// frames into the coming period.
    pub fn lock(&mut self, offset: u32, addr: ParamAddr, value: Option<f32>) {
        self.modu.lock(&self.mixer, offset, addr, value);
    }

    /// A clip's move lane value (an FX move), `offset` frames into the
    /// coming period.
    pub fn lane(&mut self, offset: u32, addr: ParamAddr, value: f32) {
        self.modu.lane(&self.mixer, offset, addr, value);
    }

    fn apply_mutes(&mut self) {
        let silent = self.pad_mutes | self.dropped;
        for p in 0..PADS {
            let bit = 1 << p;
            let want = if silent & bit != 0 { 1.0 } else { 0.0 };
            let sent = (self.mutes_sent & bit != 0) == (silent & bit != 0);
            if !sent || self.mixer.param(StripId::Pad(p), mx::MUTE) != want {
                self.set(StripId::Pad(p), mx::MUTE, want);
            }
        }
        self.mutes_sent = silent;
    }

    /// Master safety's limiting now, dB (0, or a positive number of dB
    /// taken off), for the master meter.
    pub fn gain_reduction_db(&self) -> f32 {
        use mpfree_node::Node as _;
        -self
            .mixer
            .safety()
            .param(mpfree_fx::ids::master::REDUCTION)
            .min(0.0)
    }

    /// Kit 1's pad channels muted in the Mix tab (a bit each; Drop
    /// drums' are not counted).
    pub fn pad_mutes(&self) -> u16 {
        self.pad_mutes
    }

    /// The pad mutes and solos of the project just opened, if it was saved
    /// before they were a track's (step 10): kit 1's A1-A16, a bit each,
    /// for each track on kit 1. Taken once.
    pub fn take_old_pad_mutes(&mut self) -> Option<(u16, u16)> {
        self.old_pads.take()
    }

    /// What the Play page shows. Cheap: a few reads.
    pub fn play_levels(&self) -> PlayLevels {
        let m = &self.mixer;
        let insert = |s: StripId, slot: usize, id: u16| {
            m.chain(s)
                .and_then(|c| c.node(slot))
                .map_or(0.0, |n| n.param(id))
        };
        PlayLevels {
            drums_send_db: m.param(StripId::Track(DRUMS), mx::send(DELAY_RETURN)),
            loop_send_db: m.param(StripId::Track(LOOP), mx::send(DELAY_RETURN)),
            loop_db: m.param(StripId::Track(LOOP), mx::VOLUME),
            line_db: m.param(LINE, mx::VOLUME),
            drums_filter: insert(StripId::Track(DRUMS), FILTER_SLOT, dj_filter::POSITION),
            loop_filter: insert(StripId::Track(LOOP), FILTER_SLOT, dj_filter::POSITION),
            fx_mix: insert(StripId::Master, PERFORM_SLOT, perf::MIX) / 100.0,
        }
    }

    /// What the looper records from the pads, wet, into `left` and `right`:
    /// the master's input as the last period left it, without the Loop
    /// track, the audio tracks (each on its own channel) and the line input.
    /// That is every track you play after its effects and fader, with the
    /// echoes and reverbs the send returns add (the master's own effects and
    /// fader come after it, as they do for the loop when it plays). Returns
    /// its latency in frames after the period it is late; or why it can't be
    /// had, when one of those, sounding, goes to a submix or is out of step
    /// with the master's input (a plugin's latency). Cheap: a subtraction a
    /// frame for each that sounds.
    pub fn pads_wet(
        &self,
        left: &mut [f32; PERIOD],
        right: &mut [f32; PERIOD],
    ) -> Result<u32, &'static str> {
        self.mix_less(2, left, right)
    }

    /// Points engine audio track `t`'s capture ([`AUDIO_OUT_CAPTURE`] + `t`)
    /// at its channel `ch`, after the fader, or frees it (None), so
    /// [`MixHost::pads_wet`] leaves it out. Only a change moves the point.
    /// Real-time safe.
    pub fn point_audio_out(&mut self, t: usize, ch: Option<usize>) {
        use mpfree_fx::CapturePoint;
        let Some(was) = self.audio_outs.get(t).copied() else {
            return;
        };
        if was == ch {
            return;
        }
        let point = ch.map(|c| (StripId::Track(c), CapturePoint::AfterFader));
        let held = self.mixer.set_capture(AUDIO_OUT_CAPTURE + t, point);
        self.audio_outs[t] = if held { ch } else { None };
    }

    /// [`LOOP_CAPTURE`] as the last period left it, without: nothing (`n`
    /// 0), the Loop track (1), or the Loop track, the audio tracks and the
    /// line input (2, [`MixHost::pads_wet`]).
    fn mix_less(
        &self,
        n: usize,
        left: &mut [f32; PERIOD],
        right: &mut [f32; PERIOD],
    ) -> Result<u32, &'static str> {
        let m = &self.mixer;
        let latency = m.capture_latency(LOOP_CAPTURE).unwrap_or(0);
        match m.capture(LOOP_CAPTURE) {
            Some((l, r)) if l.len() == PERIOD => {
                left.copy_from_slice(l);
                right.copy_from_slice(r);
            }
            // Before the mixer's first period.
            _ => {
                left.fill(0.0);
                right.fill(0.0);
                return Ok(latency);
            }
        }
        const LOOP_SUB: &str = "the Loop track goes to a submix";
        const AUDIO_SUB: &str = "an audio track goes to a submix";
        const LINE_SUB: &str = "the line input goes to a submix";
        let loops = [(LOOP_CAPTURE + 1, LEFT_OUT[0], LOOP_SUB)];
        let audio =
            self.audio_outs.iter().enumerate().filter_map(|(t, c)| {
                c.map(|c| (AUDIO_OUT_CAPTURE + t, StripId::Track(c), AUDIO_SUB))
            });
        let line = [(LOOP_CAPTURE + 2, LEFT_OUT[1], LINE_SUB)];
        let rest = audio.chain(line).filter(|_| n >= 2);
        for (slot, s, why) in loops.into_iter().filter(|_| n >= 1).chain(rest) {
            let Some((l, r)) = m.capture(slot).filter(|(l, _)| l.len() == PERIOD) else {
                continue;
            };
            if l.iter().chain(r).all(|&x| x == 0.0) {
                continue;
            }
            if m.destination(s) != Some(StripId::Master) {
                return Err(why);
            }
            if m.capture_latency(slot) != Some(latency) {
                return Err("a plugin's latency puts the mix out of step");
            }
            for f in 0..PERIOD {
                left[f] -= l[f];
                right[f] -= r[f];
            }
        }
        Ok(latency)
    }

    /// The channel instrument `i` plays into, for the looper's Records
    /// from: Pads, so it records what the pads play and not
    /// the whole mix (Jay, 20 Nov: a pattern on another instrument stays
    /// out). The app says which instrument (`main.rs`, `pads_instrument`);
    /// this only says where it sounds. A track's own instrument: that
    /// track's channel. Kit 1, and a kit no track homes: Drums (through the
    /// pad channels); another kit: the channel of the first track that
    /// plays it (`homes`, [`crate::sampler::TrackKits::homes`]), as
    /// [`Pads`] and export's stems find them. The shared synth: Synth.
    /// None for MIDI out, which sounds outside mpfree. Clips that play the
    /// same instrument share its channel, so they are heard there too.
    pub fn instrument_strip(&self, i: crate::looping::Instrument, homes: &[u8]) -> Option<StripId> {
        use crate::looping::Instrument;
        Some(StripId::Track(match i {
            // Where its instrument's slot feeds (step 4d); the track's
            // channel while it has none yet.
            Instrument::Own(t) => self
                .instruments
                .strip(t)
                .unwrap_or_else(|| self.track_channel(t)),
            Instrument::Synth => SYNTH,
            Instrument::Kit(k) => match homes.get(k).copied().unwrap_or(0) {
                0 => DRUMS,
                h => self.track_channel(usize::from(h) - 1),
            },
            Instrument::MidiOut => return None,
        }))
    }

    /// How long strip `s`'s effects ring after its input stops: the tails
    /// of the inserts that are on, one after another, with their latency
    /// ([`RINGS_ON`] for one that never ends). Allocates nothing.
    fn rings(&self, s: StripId) -> u64 {
        let Some(c) = self.mixer.chain(s) else {
            return 0;
        };
        let mut frames = 0u64;
        for slot in (0..c.len()).filter(|&k| c.is_on(k)) {
            let Some(n) = c.node(slot) else { continue };
            if n.tail_frames() == u32::MAX {
                return RINGS_ON;
            }
            frames += u64::from(n.tail_frames()) + u64::from(n.latency());
        }
        frames
    }

    /// Where the Loop track's sound still is, besides the master, each
    /// period (the design review of 20 Nov: an echo rings on after its
    /// send comes down, and so do a submix's effects): a return it sends
    /// to, directly or through the submix it goes to, rings with it for as
    /// long as that return's effects ring after the send comes down
    /// ([`MixHost::rings`]); the submix for as long as its own do, after
    /// the Loop track is back on the master. Records from: Master is
    /// refused until then ([`MixHost::loop_take`]).
    fn watch_loop_echoes(&mut self) {
        let lp = StripId::Track(LOOP);
        let n = PERIOD as u64;
        let down = |left: u64| {
            if left == RINGS_ON {
                RINGS_ON
            } else {
                left.saturating_sub(n)
            }
        };
        let bus = match self.mixer.destination(lp) {
            Some(b @ StripId::Bus(_)) => Some(b),
            _ => None,
        };
        let bus_rings = bus.map_or(0, |b| self.rings(b));
        self.loop_bus_echo = match (bus, self.loop_bus_echo) {
            (Some(b), _) => Some((b, bus_rings)),
            (None, Some((b, left))) => Some((b, down(left))).filter(|e| e.1 > 0),
            (None, None) => None,
        };
        for r in 0..mpfree_fx::RETURNS {
            let send = |s: StripId| self.mixer.param(s, mx::send(r)) > SEND_OFF_DB;
            let through = bus.filter(|&b| send(b));
            self.loop_echoes[r] = if send(lp) || through.is_some() {
                let ring = self.rings(StripId::Return(r));
                let extra = if through.is_some() { bus_rings } else { 0 };
                if ring == RINGS_ON || extra == RINGS_ON {
                    RINGS_ON
                } else {
                    ring + extra
                }
            } else {
                down(self.loop_echoes[r])
            };
        }
    }

    /// Why the master can't be recorded now without the loop in it: the
    /// Loop track goes to a submix or sends to a return (directly or
    /// through that submix), or one still rings with it
    /// ([`MixHost::watch_loop_echoes`]); None when the master's input
    /// holds the Loop track only on its own way in, which the looper
    /// takes out.
    pub fn loop_echo(&self) -> Option<&'static str> {
        let lp = StripId::Track(LOOP);
        let bus = match self.mixer.destination(lp) {
            Some(b @ StripId::Bus(_)) => Some(b),
            _ => None,
        };
        let sends = |s: StripId, r: usize| self.mixer.param(s, mx::send(r)) > SEND_OFF_DB;
        if let Some(r) = (0..mpfree_fx::RETURNS).find(|&r| sends(lp, r)) {
            return Some(LOOP_SENDS_HEARD[r]);
        }
        if bus.is_some() {
            return Some(LOOP_IN_SUBMIX);
        }
        if let Some(r) = (0..mpfree_fx::RETURNS).find(|&r| self.loop_echoes[r] > 0) {
            return Some(LOOP_ECHOES_RING[r]);
        }
        self.loop_bus_echo
            .filter(|e| e.1 > 0)
            .map(|_| LOOP_SUBMIX_RINGS)
    }

    /// What the looper records into `left` and `right` this period, and its
    /// latency, as [`MixHost::pads_wet`] says them: `from` None is the
    /// master's input less the loops ([`MixHost::pads_wet`], where the
    /// capture point rests with Line in picked; the pads are their strip,
    /// [`MixHost::pads_strip`]); the master is all its input but the
    /// Loop track (the audio tracks and the line input stay in, before
    /// the master's effects and fader, as the pads are); any other strip,
    /// a track or a submix, is what it sends on, after its fader. Never the
    /// Loop track, nor a submix the Loop track goes to (it would record
    /// itself). When `from` changes, the capture point moves and this
    /// period is silence (the mixer's next block takes from the new
    /// point). Cheap: a copy, and at most three subtractions a frame.
    pub fn loop_take(
        &mut self,
        from: Option<StripId>,
        left: &mut [f32; PERIOD],
        right: &mut [f32; PERIOD],
    ) -> Result<u32, &'static str> {
        use mpfree_fx::CapturePoint;
        let point = match from {
            None | Some(StripId::Master) => (StripId::Master, CapturePoint::BeforeInserts),
            Some(s) => (s, CapturePoint::AfterFader),
        };
        if from == Some(StripId::Track(LOOP)) {
            left.fill(0.0);
            right.fill(0.0);
            return Err("the Loop track can't record itself");
        }
        if let Some(s @ StripId::Bus(_)) = from {
            if self.mixer.destination(StripId::Track(LOOP)) == Some(s) {
                left.fill(0.0);
                right.fill(0.0);
                return Err("the Loop track goes to that submix");
            }
        }
        // The master hears the send returns (they always play into it)
        // and the submixes: the Loop track's sound in them would be
        // recorded, its own echoes, and each overdub would add them again.
        // Refused (silence, said) while the Loop track goes to a submix or
        // sends to a return, and until their effects stop ringing with it.
        if from == Some(StripId::Master) {
            if let Some(why) = self.loop_echo() {
                left.fill(0.0);
                right.fill(0.0);
                return Err(why);
            }
        }
        let moved = point != self.loop_point;
        // A strip is found again each period (cheap, and it allocates
        // nothing), so one that moves in the mixer stays the one taken.
        if moved || point.0 != StripId::Master {
            if !self.mixer.set_capture(LOOP_CAPTURE, Some(point)) {
                let master_in = (StripId::Master, CapturePoint::BeforeInserts);
                let _ = self.mixer.set_capture(LOOP_CAPTURE, Some(master_in));
                self.loop_point = master_in;
                left.fill(0.0);
                right.fill(0.0);
                return Err("that strip isn't in the mix");
            }
            self.loop_point = point;
        }
        if moved {
            left.fill(0.0);
            right.fill(0.0);
            return Ok(self.mixer.capture_latency(LOOP_CAPTURE).unwrap_or(0));
        }
        match from {
            None => self.pads_wet(left, right),
            Some(StripId::Master) => self.mix_less(1, left, right),
            Some(_) => self.mix_less(0, left, right),
        }
    }

    /// What audio track `t` records from inside the mix (the instrument
    /// redesign's step 9): `from`'s strip as the last period left it,
    /// before or after its effects (before its fader either way, so its
    /// level never changes the take), into `into` interleaved stereo, and
    /// the capture's latency (frames between the block's track inputs and
    /// the point). `from` None frees the track's capture point. The period
    /// the point moves is silence, as the looper's. Silence and None while
    /// the strip isn't in the mix (a strip is found again each period, so
    /// one that moves in the mixer stays the one taken). Real-time safe: a
    /// copy.
    pub fn audio_take(
        &mut self,
        t: usize,
        from: Option<crate::audiotracks::Inside>,
        into: &mut [f32],
    ) -> Option<u32> {
        use mpfree_fx::CapturePoint;
        let slot = AUDIO_CAPTURE + t;
        let was = *self.audio_points.get(t)?;
        let point = from.map(|f| {
            let p = if f.after_effects {
                CapturePoint::BeforeFader
            } else {
                CapturePoint::BeforeInserts
            };
            (f.strip, p)
        });
        let Some(point) = point else {
            if was.is_some() {
                self.mixer.set_capture(slot, None);
                self.audio_points[t] = None;
            }
            return None;
        };
        if !self.mixer.set_capture(slot, Some(point)) {
            self.mixer.set_capture(slot, None);
            self.audio_points[t] = None;
            into.fill(0.0);
            return None;
        }
        self.audio_points[t] = Some(point);
        let latency = self.mixer.capture_latency(slot);
        match self.mixer.capture(slot) {
            Some((l, r)) if was == Some(point) => {
                let n = l.len().min(into.len() / 2);
                for f in 0..n {
                    into[2 * f] = l[f];
                    into[2 * f + 1] = r[f];
                }
                into[2 * n..].fill(0.0);
            }
            _ => into.fill(0.0),
        }
        latency
    }

    /// What an audio track can record from inside the mix (step 9), by
    /// name, as the mixer panel names the strips: each instrument's
    /// channel (a track's own or a free place's instrument by its name,
    /// Drums, Synth, a kit's channel on a track), the Loop track, each
    /// submix, then the master. Never an audio track's own channel (none
    /// is an instrument's or a kit's), the old shared Audio strip, the
    /// line input (the panel's L, R, L+R and ST) or a pad's channel.
    /// Allocates: for the panel's taps.
    pub fn inside_sources(&self) -> Vec<(String, StripId)> {
        let kit_channels: Vec<usize> = (0..mpfree_seq::MAX_TRACKS)
            .filter(|&t| self.kit_tracks >> t & 1 == 1)
            .map(|t| self.track_channel(t))
            .collect();
        self.mixer
            .strips()
            .filter(|&s| match s {
                StripId::Track(c) => {
                    matches!(c, DRUMS | SYNTH | LOOP)
                        || self.instruments.on_strip(c).is_some()
                        || kit_channels.contains(&c)
                }
                StripId::Bus(_) | StripId::Master => true,
                _ => false,
            })
            .map(|s| (self.strip_name(s), s))
            .collect()
    }

    /// Points the sampler's capture point at `src`, or frees it for the
    /// line input. Takes effect from the next block; false if the mixer
    /// has no such strip (the capture point is then free).
    pub fn set_rec_source(&mut self, src: crate::sampler::RecSource) -> bool {
        match src {
            crate::sampler::RecSource::Line => {
                self.mixer.set_capture(SAMPLER_CAPTURE, None);
                true
            }
            crate::sampler::RecSource::Mix(strip, point) => {
                let ok = self
                    .mixer
                    .set_capture(SAMPLER_CAPTURE, Some((strip, point)));
                if !ok {
                    self.mixer.set_capture(SAMPLER_CAPTURE, None);
                }
                ok
            }
        }
    }

    /// What the sampler's capture point took in the last period, and how
    /// many frames late it hears the transport's block; None while it is
    /// free.
    pub fn rec_capture(&self) -> Option<(&[f32], &[f32], u32)> {
        let (l, r) = self.mixer.capture(SAMPLER_CAPTURE)?;
        let latency = self.mixer.capture_latency(SAMPLER_CAPTURE)?;
        Some((l, r, latency))
    }

    /// The Level (Q-Link 1, the slider): the master fader.
    pub fn set_level(&mut self, db: f32) {
        self.set(StripId::Master, mx::VOLUME, db.min(self.max_db));
    }

    /// The master fader now, dB.
    #[cfg(test)]
    pub fn level(&self) -> f32 {
        self.mixer.param(StripId::Master, mx::VOLUME)
    }

    /// How loud Monitor plays the line input: its strip's fader, dB (the
    /// Sample page's recorder shows and turns it; one-page step 2d).
    pub fn line_level(&self) -> f32 {
        self.mixer.param(LINE, mx::VOLUME)
    }

    /// Follows the looper: whether the line input is open, and whether it
    /// is heard (Monitor). Cheap to call every period.
    pub fn follow(&mut self, line_open: bool, monitor_on: bool) {
        if line_open != self.line_open {
            self.line_open = line_open;
            self.set(
                LINE,
                mx::SOURCE,
                if line_open {
                    source::STEREO
                } else {
                    source::OFF
                },
            );
        }
        if monitor_on != self.monitor {
            self.monitor = monitor_on;
            self.set(
                LINE,
                mx::MONITOR,
                if monitor_on {
                    monitor::ON
                } else {
                    monitor::OFF
                },
            );
        }
    }

    /// The pad channels for the sampler's `render_routed` this period,
    /// every kit on them (the tests).
    #[cfg(test)]
    pub fn pads(&mut self) -> Pads<'_> {
        self.pads_homed(&[])
    }

    /// The pad channels and the kits' tracks for the sampler's
    /// `render_routed` this period: kit `k` plays into track
    /// `homes[k] - 1`'s channel, or the pad channels where that is 0 or
    /// past `homes` (`sampler::TrackKits::homes`). Those tracks' channels
    /// show while it is so. Never allocates.
    pub fn pads_homed<'a>(&'a mut self, homes: &'a [u8]) -> Pads<'a> {
        self.kit_tracks = homes
            .iter()
            .filter(|&&h| h > 0)
            .fold(0, |m, &h| m | 1 << (h - 1));
        if self.homes.get(..homes.len()) != Some(homes)
            || self.homes[homes.len().min(KITS)..].iter().any(|&h| h != 0)
        {
            let n = homes.len().min(KITS);
            self.homes[..n].copy_from_slice(&homes[..n]);
            self.homes[n..].fill(0);
            self.route_lent();
        }
        Pads {
            first: self.instruments.channel(0),
            mixer: &mut self.mixer,
            channels: &self.pad_channels,
            played: 0,
            homes,
        }
    }

    /// The track kit `kit`'s sound plays into: Drums for kit 1 and a kit
    /// no track has, else its home track's own channel.
    pub fn kit_track(&self, kit: usize) -> usize {
        match self.homes.get(kit).copied().unwrap_or(0) {
            0 => DRUMS,
            h => self.track_channel(usize::from(h) - 1),
        }
    }

    /// Each lent channel's output to its kit's track (after a kit's home
    /// changes, or a lend).
    fn route_lent(&mut self) {
        for i in 0..LENT {
            let s = StripId::Pad(PADS + i);
            let Some((kit, _)) = self.pad_channels.holder(s) else {
                continue;
            };
            // One sent elsewhere (a submix) stays there.
            let t = self.kit_track(kit) as f32;
            if self.mixer.param(s, mx::OUTPUT) == output::TRACK
                && self.mixer.param(s, mx::OUTPUT_TRACK) != t
            {
                self.mixer.set_now(s, mx::OUTPUT_TRACK, t);
            }
        }
    }

    /// Kit `kit`'s pad `pad`'s own channel, lending it one if it has none
    /// (R3 part 4a): a lent channel starts plain, playing into the kit's
    /// track. None when every lent channel is taken. Never allocates.
    pub fn lend_pad(&mut self, kit: usize, pad: usize) -> Option<StripId> {
        let (s, new) = self.pad_channels.lend(kit, pad)?;
        if new {
            for p in mpfree_fx::strip_params() {
                if crate::project::saved_control(s, p.id) && self.mixer.param(s, p.id) != p.default
                {
                    self.mixer.set_now(s, p.id, p.default);
                }
            }
            self.mixer.set_now(s, mx::OUTPUT, output::TRACK);
            self.route_lent();
        }
        Some(s)
    }

    /// Gives back every lent channel that is plain again: no effects, and
    /// every control but its output at its default. Called when the chain
    /// editor lets a pad go, and before a save.
    pub fn give_back_plain(&mut self) {
        for i in 0..LENT {
            let s = StripId::Pad(PADS + i);
            if self.pad_channels.holder(s).is_some() && self.plain(s) {
                self.pad_channels.give_back(s);
                for id in [mx::OUTPUT, mx::OUTPUT_TRACK] {
                    let d = mpfree_fx::strip_params()
                        .iter()
                        .find(|p| p.id == id)
                        .map_or(0.0, |p| p.default);
                    self.mixer.set_now(s, id, d);
                }
            }
        }
    }

    /// Whether channel `s` has nothing set: no effects, playing into a
    /// track, every other saved control at its default.
    fn plain(&self, s: StripId) -> bool {
        self.mixer.chain(s).is_none_or(|c| c.is_empty())
            && self.mixer.param(s, mx::OUTPUT) == output::TRACK
            && mpfree_fx::strip_params()
                .iter()
                .filter(|p| {
                    crate::project::saved_control(s, p.id)
                        && p.id != mx::OUTPUT
                        && p.id != mx::OUTPUT_TRACK
                })
                .all(|p| self.mixer.param(s, p.id) == p.default)
    }

    /// A strip's name on the panel: a lent pad channel by its pad and kit,
    /// a track's own instrument's channel by the instrument
    /// ([`instrument_channel_name`]).
    pub fn strip_name(&self, s: StripId) -> String {
        match self.own_instrument_on(s) {
            Some((t, instrument)) => instrument_channel_name(instrument, &own_track_label(s, t)),
            None => strip_name(&self.pad_channels, s),
        }
    }

    /// The track whose own instrument plays on strip `s`, and the
    /// instrument's name (step 4d), from where the instrument's slot feeds
    /// ([`Instruments::on_strip`]): today track 1's on the Drums strip (or
    /// its old own channel), any other's on its own channel. None for a
    /// strip no track's own instrument plays on (a kit's, the engines'
    /// others). A free place's instrument (step 6) is named for who plays
    /// it: the pads ([`instruments::THE_PADS`]) while they do, else the
    /// first track that plays it, so two free places' strips never share
    /// a name ("FM · Pads", "FM · Track 2").
    pub fn own_instrument_on(&self, s: StripId) -> Option<(usize, &'static str)> {
        let StripId::Track(c) = s else {
            return None;
        };
        let (holder, held) = self.instruments.on_strip(c)?;
        if holder < instruments::TRACKS {
            return Some((holder, held.name));
        }
        let senders = self.instruments.senders(holder);
        let by = if self.instruments.pads_play_at() == Some(holder) || senders == 0 {
            instruments::THE_PADS
        } else {
            senders.trailing_zeros() as usize
        };
        Some((by, held.name))
    }

    /// The project's mixer part (`project::mixer_text_lent`), once the
    /// lent pad channels that are plain again have been given back.
    pub fn mixer_text(&mut self) -> String {
        self.give_back_plain();
        let lent: Vec<(StripId, usize, usize)> = self.pad_channels.lent().collect();
        let track1 = StripId::Track(self.track_channel(0));
        crate::project::mixer_text_lent(&self.mixer, self.pad_mutes, &lent, track1)
    }

    /// The tracks whose own channels are in use, a bit each: playing their
    /// own instrument (or ringing out an old one), or a kit
    /// ([`MixHost::pads_homed`]).
    pub fn channels_in_use(&self) -> u128 {
        self.instruments.in_use() | self.kit_tracks | self.audio_tracks
    }

    /// Sequencer track `t`'s mixer channel: [`track_channel`], but track
    /// 1's old own one in a project saved with settings there
    /// ([`MixHost::old_first`]).
    pub fn track_channel(&self, t: usize) -> usize {
        self.instruments.channel(t)
    }

    /// Whether track 1 plays into its old own channel (a project saved
    /// before Drums was track 1's strip, with settings on that channel)
    /// rather than Drums: shown as [`OLD_TRACK_1`] while in use, said in
    /// the log when the project opens and counted
    /// ([`MixHost::old_first_opens`]).
    pub fn old_first(&self) -> bool {
        self.instruments.channel(0) != DRUMS
    }

    /// The own channels in use, a bit each by their place past the
    /// engines' tracks (bit `k`: mixer track `TRACKS + k`), as the Tracks
    /// view, the stems and the throws list them: [`MixHost::channels_in_use`]
    /// but track 1's, which plays into Drums ([`track_channel`]) unless it
    /// keeps its old channel.
    pub fn own_channels_in_use(&self) -> u128 {
        let used = self.channels_in_use();
        if self.old_first() {
            used
        } else {
            used & !1
        }
    }

    /// The drums as the sampler made them: the pads that played (`played`,
    /// from [`Pads::played`]) summed from their channels' inputs (and the
    /// Drums track's, for kit 1's pads past 16, and the lent channels
    /// playing into Drums) into `left` and `right`, for the looper.
    /// Silence when none played.
    pub fn drums(&mut self, played: u128, left: &mut [f32; PERIOD], right: &mut [f32; PERIOD]) {
        left.fill(0.0);
        right.fill(0.0);
        let mut add = |input: Option<(&mut [f32], &mut [f32])>| {
            if let Some((l, r)) = input {
                for f in 0..PERIOD {
                    left[f] += l[f];
                    right[f] += r[f];
                }
            }
        };
        for p in (0..PADS).filter(|p| played & (1 << p) != 0) {
            add(self.mixer.pad_input(p));
        }
        if played & DRUMS_DIRECT != 0 {
            add(self.mixer.track_input(DRUMS));
        }
        for i in (0..LENT).filter(|i| played & (1 << (LENT_BIT + i)) != 0) {
            let s = StripId::Pad(PADS + i);
            let drums = self
                .pad_channels
                .holder(s)
                .is_some_and(|(k, _)| self.homes.get(k).copied().unwrap_or(0) == 0);
            if drums {
                add(self.mixer.pad_input(PADS + i));
            }
        }
    }

    /// One period: the synth and the loop into their tracks, the line input
    /// into the input (the pads are in already: [`MixHost::pads`]), the
    /// master out into `left` and `right`. Returns the output's peak and
    /// whether master safety caught anything.
    // The sounds come in as they are made; a struct of them would only be
    // taken apart again here.
    #[allow(clippy::too_many_arguments)]
    pub fn period(
        &mut self,
        ctx: &Ctx,
        synth: Stereo,
        looped: Stereo,
        line: Stereo,
        left: &mut [f32; PERIOD],
        right: &mut [f32; PERIOD],
    ) -> (f32, bool) {
        // A tap on the XY pad quicker than a period played for the last
        // one: it lets go now.
        if !self.xy_new {
            if let Some((x, y)) = self.xy_late.take() {
                self.play_xy(x, y, false);
            }
        }
        self.xy_new = false;
        // A layout of yours: a learned control's gesture ends HAND_S after
        // its last move, and pushes glide away.
        if self.xy_play.playing() {
            if self.xy_midi_left > 0 {
                self.xy_midi_left = self.xy_midi_left.saturating_sub(PERIOD as u32);
                if self.xy_midi_left == 0 && self.xy_on[1] {
                    self.xy_midi_ends();
                }
            }
            let mut reach =
                crate::xy_play::ModReach::new(&mut self.modu, &self.mixer, self.recording);
            self.xy_play.tick(&mut reach, PERIOD as u32);
        }
        // The audio tracks are in already, each on its own channel
        // (`audiotracks::TrackHost::play_into`): the Audio strip hears
        // nothing since phase 1b.
        for (t, (l, r)) in [(SYNTH, synth), (LOOP, looped)] {
            if let Some((dl, dr)) = self.mixer.track_input(t) {
                dl[..PERIOD].copy_from_slice(l);
                dr[..PERIOD].copy_from_slice(r);
            }
        }
        // Each track's own instrument into its mixer track, timed while
        // any track has one.
        if self.instruments.in_use() != 0 {
            let t = Instant::now();
            self.instruments.render(ctx, PERIOD, &mut self.mixer);
            self.instrument_time = t.elapsed();
        } else {
            self.instruments.render(ctx, PERIOD, &mut self.mixer);
            self.instrument_time = Duration::ZERO;
        }
        // Each synth-voice pad's own synth into its pad's strip, timed
        // with the instruments, while any has one or rings out.
        if self.pad_synths.count() > 0 || self.pad_synths.busy() {
            let mut dest = [None; crate::pad_synths::HOST_SLOTS];
            for (s, d) in dest.iter_mut().enumerate() {
                *d = self.pad_synths.pad_of(s).map(|(k, p)| self.pad_dest(k, p));
            }
            let t = Instant::now();
            self.pad_synths.render(ctx, PERIOD, &mut self.mixer, &dest);
            self.instrument_time += t.elapsed();
        }
        if self.line_open {
            let (hl, hr) = self.mixer.hw_input();
            hl[..PERIOD].copy_from_slice(line.0);
            hr[..PERIOD].copy_from_slice(line.1);
        }
        self.modu.process(ctx, PERIOD, &mut self.mixer);
        self.mixer.process(ctx, left, right);
        self.step_capture();
        self.watch_loop_echoes();
        let peak = left
            .iter()
            .chain(right.iter())
            .fold(0f32, |m, s| m.max(s.abs()));
        let overs = self.mixer.safety().stats().overs;
        let caught = overs != self.overs;
        self.overs = overs;
        (peak, caught)
    }

    /// Starts copying what insert `slot` of `strip` holds (the Grains
    /// effect's memory) into `held`, made off the audio thread, a piece
    /// each period from now on; [`MixHost::take_captured`] gives it back
    /// when done. The box may have held another copy (it is emptied
    /// first). While another is on its way, or with no insert there, the
    /// box comes straight back with why.
    pub fn capture(
        &mut self,
        strip: StripId,
        slot: usize,
        held: Box<Held>,
    ) -> Result<(), (Box<Held>, Captured)> {
        if self.capturing.is_some() || self.captured.is_some() {
            return Err((held, Captured::Busy));
        }
        let Some(uid) = self.mixer.chain(strip).and_then(|c| c.uid(slot)) else {
            return Err((held, Captured::Gone));
        };
        let mut held = held;
        held.clear();
        self.capturing = Some(Capturing { strip, uid, held });
        Ok(())
    }

    /// The period's piece of the capture: the insert found again by its
    /// id (a move keeps it), its memory copied on. Never allocates.
    fn step_capture(&mut self) {
        let Some(c) = self.capturing.as_mut() else {
            return;
        };
        let node = self
            .mixer
            .chain_mut(c.strip)
            .and_then(|ch| ch.slot_of(c.uid).and_then(|i| ch.node_mut(i)));
        let end = match node {
            None => Captured::Gone,
            Some(n) => match n.copy_held(&mut c.held, CAPTURE_STEP) {
                HeldStep::More => return,
                HeldStep::Done => {
                    c.held.keep_values(&*n);
                    Captured::Done
                }
                HeldStep::Nothing => Captured::Nothing,
                HeldStep::Lost => Captured::Lost,
            },
        };
        if let Some(c) = self.capturing.take() {
            self.captured = Some((c.held, end));
        }
    }

    /// A capture that has ended, to send to the screen thread (which frees
    /// it). Give it back with [`MixHost::keep_captured`] if it found no
    /// room, so the audio thread never frees it.
    pub fn take_captured(&mut self) -> Option<(Box<Held>, Captured)> {
        self.captured.take()
    }

    /// A capture [`MixHost::take_captured`] gave that couldn't be sent yet.
    pub fn keep_captured(&mut self, held: Box<Held>, end: Captured) {
        self.captured = Some((held, end));
    }

    /// The mixer panel opened on `view`, or closed (`None`).
    pub fn show(&mut self, view: Option<MixView>) {
        self.shown = view;
        if let Some(v) = view {
            self.last = v;
        }
        self.state_at = None;
    }

    /// Something the panel shows changed outside the mixer (an audio
    /// track's mute or solo): its next state goes at once.
    pub fn state_now(&mut self) {
        self.state_at = None;
    }

    /// The view the panel showed last (Track Mix at first), where the Mix
    /// tab opens it again.
    pub fn last(&self) -> MixView {
        self.last
    }

    /// The mixer view the panel shows, if it is open.
    pub fn shown(&self) -> Option<MixView> {
        self.shown
    }

    /// What the panel sent. Returns the master fader's new level when it
    /// moved, so the Play page's Level follows.
    pub fn action(&mut self, a: &MixAction) -> Option<f32> {
        match *a {
            MixAction::Set {
                strip: StripId::Master,
                id: mx::VOLUME,
                value,
            } => {
                let db = value.min(self.max_db);
                self.set(StripId::Master, mx::VOLUME, db);
                self.state_at = None;
                Some(db)
            }
            MixAction::Set {
                strip: StripId::Pad(p),
                id: mx::MUTE,
                value,
            } if p < PADS => {
                // Kept with Drop drums' over it (not a track's pad mute:
                // the Play page's speakers are those, step 10).
                self.play(PlayFx::PadMute {
                    pad: p,
                    mute: value >= 0.5,
                });
                None
            }
            MixAction::Set { strip, id, value } => {
                self.set(strip, id, value);
                None
            }
            MixAction::Select(s) => {
                self.selected = Some(s);
                self.state_at = None;
                None
            }
            MixAction::View(v) => {
                self.show(Some(v));
                None
            }
            // The insert pages come with the FX pages; the look is the
            // screen's.
            MixAction::OpenInsert { .. }
            | MixAction::OpenSidechain { .. }
            | MixAction::AddInsert { .. }
            | MixAction::PickInsert { .. }
            | MixAction::MoveInsert { .. }
            | MixAction::Unpick
            | MixAction::AddEq { .. }
            | MixAction::FocusInsert { .. }
            | MixAction::SetInsert { .. }
            | MixAction::ReleaseInsert { .. }
            | MixAction::Look(_) => None,
            // Freeze, Thaw and Print need the song: `MixHost::take_action`.
            MixAction::Freeze(_)
            | MixAction::Thaw(_)
            | MixAction::Print(_)
            | MixAction::CancelTake(_) => None,
        }
    }

    /// The panel's next state, while it shows and [`STATE_EVERY`] has
    /// passed (at once after a change).
    /// The tracks' strips as the mixer's Tracks view shows them: the
    /// engines' tracks, then each track's own instrument's while in use.
    pub fn track_strips(&self) -> Vec<StripId> {
        (0..self.mixer.tracks())
            .filter(|&t| self.track_strip_shown(t))
            .map(StripId::Track)
            .collect()
    }

    /// Whether mixer track `t` shows on the Tracks view: the engines'
    /// always, a track's own channel while in use
    /// ([`MixHost::own_channels_in_use`]), a free place's while its
    /// instrument plays or rings out there (step 6).
    fn track_strip_shown(&self, t: usize) -> bool {
        if t == AUDIO {
            self.old_audio()
        } else if t < TRACKS {
            true
        } else if t >= FREE_FIRST {
            let n = t - FREE_FIRST;
            n < instruments::FREE && self.instruments.free_in_use() >> n & 1 == 1
        } else {
            self.own_channels_in_use() >> (t - TRACKS) & 1 == 1
        }
    }

    /// The shared Audio strip still holds settings (an effect, a send, a
    /// fader or pan moved, a mute): a project's an older mpfree saved.
    /// Audio tracks play through their own channels, so nothing is heard
    /// through it; it shows, as "Audio (old)", to say what is there.
    pub fn old_audio(&self) -> bool {
        let s = StripId::Track(AUDIO);
        self.mixer.chain(s).is_some_and(|c| !c.is_empty())
            || mpfree_fx::strip_params()
                .iter()
                .filter(|p| crate::project::saved_control(s, p.id))
                .any(|p| self.mixer.param(s, p.id) != p.default)
    }

    pub fn poll_state(&mut self, now: Instant) -> Option<MixState> {
        let view = self.shown?;
        if self
            .state_at
            .is_some_and(|t| now.duration_since(t) < STATE_EVERY)
        {
            return None;
        }
        self.state_at = Some(now);
        // Tracks' own channels show only while in use (an instrument or a
        // kit plays there); the others aren't read. A track's own
        // instrument's channel is named for it, track 1's on Drums too.
        let used = self.own_channels_in_use();
        let own: Vec<(StripId, String)> = self
            .mixer
            .strips()
            .filter(|s| matches!(s, StripId::Track(_)))
            .filter_map(|s| self.own_instrument_on(s).map(|_| (s, self.strip_name(s))))
            .collect();
        let ch = &self.pad_channels;
        // Lent pad channels show while lent.
        let free_used = self.instruments.free_in_use();
        // The shared Audio strip only while it still holds settings.
        let old_audio = self.old_audio();
        let shown = |s: StripId| match s {
            StripId::Track(AUDIO) => old_audio,
            StripId::Track(t) if t >= FREE_FIRST => {
                t - FREE_FIRST < instruments::FREE && free_used >> (t - FREE_FIRST) & 1 == 1
            }
            StripId::Track(t) if t >= TRACKS => used & (1 << (t - TRACKS)) != 0,
            StripId::Pad(p) if p >= PADS => ch.holder(s).is_some(),
            _ => true,
        };
        let named = |s: StripId| match own.iter().find(|(o, _)| *o == s) {
            Some((_, n)) => n.clone(),
            None => strip_name(ch, s),
        };
        let mut st = MixState::from_mixer_where(&mut self.mixer, view, &named, &shown);
        st.counts[0] = TRACKS - usize::from(!old_audio)
            + used.count_ones() as usize
            + free_used.count_ones() as usize;
        st.counts[1] = PADS + ch.lent().count();
        st.tracks = TRACKS;
        st.selected = self.selected;
        st.takes = true;
        for v in st.strips.iter_mut().chain(std::iter::once(&mut st.master)) {
            v.freezable = crate::takes::freezable(v.id);
            v.take = self.takes.view(v.id).map(|t| mpfree_mixer_ui::TakeView {
                print: t.print,
                done: t.done,
            });
        }
        Some(st)
    }

    /// The song as a take sees it: `tracks` in the project, at `bpm`, each
    /// playing what `now` says.
    fn song<'a>(
        instruments: &'a Instruments,
        tracks: usize,
        bpm: f64,
        now: &'a dyn Fn(usize) -> Option<mpfree_play::NowPlaying>,
        instrument: &'a dyn Fn(usize) -> u64,
    ) -> crate::takes::Song<'a> {
        crate::takes::Song {
            tracks,
            own: instruments.in_use(),
            bpm,
            now,
            instrument,
        }
    }

    /// The strip menu's Freeze, Thaw, Print and Cancel, with the song's
    /// `tracks` at `bpm` playing what `now` says. Returns the line for
    /// the screen (None for any other action).
    pub fn take_action(
        &mut self,
        a: &MixAction,
        tracks: usize,
        bpm: f64,
        now: &dyn Fn(usize) -> Option<mpfree_play::NowPlaying>,
    ) -> Option<String> {
        let inst = &self.instruments;
        let instrument = |k: usize| {
            inst.node(k)
                .map_or(0, |n| crate::takes::id_number(n.info().id))
        };
        let song = Self::song(inst, tracks, bpm, now, &instrument);
        let mut mem = TakeMem {
            builder: self.builder.as_mut(),
            here: &mut self.here,
        };
        let (kind, s) = match *a {
            MixAction::Freeze(s) => (crate::takes::Kind::Freeze, s),
            MixAction::Print(s) => (crate::takes::Kind::Print, s),
            MixAction::Thaw(s) => return Some(self.takes.thaw(s, &name(s), &mut self.mixer)),
            MixAction::CancelTake(s) => return self.takes.cancel(s, &mut self.mixer, &mut mem),
            _ => return None,
        };
        self.state_at = None;
        Some(
            self.takes
                .ask(kind, s, &name(s), &song, &self.mixer, &mut mem),
        )
    }

    /// A print the screen couldn't take, dropped off the audio thread.
    pub fn drop_printed(&mut self, p: Box<mpfree_fx::Printed>) {
        if let Some(b) = self.builder.as_mut() {
            b.drop_printed_later(p);
        }
    }

    /// Once a period, after [`MixHost::period`] with the same `ctx`: the
    /// take under way and the frozen strips (takes.rs), with the song's
    /// `tracks` playing what `now` says.
    pub fn takes_period(
        &mut self,
        ctx: &Ctx,
        tracks: usize,
        now: &dyn Fn(usize) -> Option<mpfree_play::NowPlaying>,
    ) -> Option<crate::takes::Said> {
        let inst = &self.instruments;
        let instrument = |k: usize| {
            inst.node(k)
                .map_or(0, |n| crate::takes::id_number(n.info().id))
        };
        let song = Self::song(inst, tracks, ctx.bpm, now, &instrument);
        let mut mem = TakeMem {
            builder: self.builder.as_mut(),
            here: &mut self.here,
        };
        let said = self.takes.period(ctx, &song, &mut self.mixer, &mut mem);
        if said.is_some() {
            self.state_at = None;
        }
        said
    }
}

/// The pad channels as the sampler sees them (`PadOut`): pad `p` of kit 1
/// adds into pad channel `p`, and every pad of another kit into its
/// track's channel (`homes`, [`MixHost::pads_homed`]), or into pad
/// channel `p` if no track has it. It notes which pad channels played.
pub struct Pads<'a> {
    /// Track 1's channel ([`MixHost::track_channel`]).
    first: usize,
    mixer: &'a mut Mixer,
    channels: &'a PadChannels,
    played: u128,
    homes: &'a [u8],
}

impl Pads<'_> {
    /// The pads that played this period, a bit each (and bit [`PADS`] for
    /// kit 1's pads past them, then a bit for each lent channel).
    pub fn played(&self) -> u128 {
        self.played
    }
}

impl PadOut for Pads<'_> {
    fn pad(&mut self, kit: usize, pad: usize) -> Option<(&mut [f32], &mut [f32])> {
        // A pad lent a channel of its own (R3 part 4a).
        if kit > 0 || pad >= PADS {
            if let Some(StripId::Pad(s)) = self.channels.channel(kit, pad) {
                self.played |= 1 << (LENT_BIT + s - PADS);
                return self.mixer.pad_input(s);
            }
        }
        match self.homes.get(kit).copied().unwrap_or(0) {
            0 if pad < PADS => {
                self.played |= 1 << pad;
                self.mixer.pad_input(pad)
            }
            // Kit 1's pads past its first 16 (PAD BANK B to H) have no
            // channel of their own: straight into the Drums track.
            0 => {
                self.played |= DRUMS_DIRECT;
                self.mixer.track_input(DRUMS)
            }
            1 => self.mixer.track_input(self.first),
            home => self.mixer.track_input(track_channel(usize::from(home) - 1)),
        }
    }
}

/// The mixer's context for a period: the transport's tempo, position
/// (beats) and whether it rolls, from the period's first span.
pub fn ctx(rate: u32, frame: u64, bpm: f64, block: &mpfree_transport::Block) -> Ctx<'static> {
    let mut c = Ctx::new(rate, frame);
    c.bpm = bpm;
    if let Some(s) = block.spans().first() {
        c.bpm = s.bpm;
        c.playing = s.state.rolling();
        c.beats = s.tick_at(s.frame) / f64::from(mpfree_transport::PPQ);
    }
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 44_100;

    fn tone(f: f32, amp: f32, at: usize) -> [f32; PERIOD] {
        std::array::from_fn(|i| {
            amp * (std::f32::consts::TAU * f * (at + i) as f32 / RATE as f32).sin()
        })
    }

    /// Pad 1 plays `drums` (as the sampler would, into its channel), the
    /// line input `line`.
    fn run(h: &mut MixHost, periods: usize, drums: f32, line: f32) -> (f32, bool) {
        let z = [0f32; PERIOD];
        let (mut l, mut r) = ([0f32; PERIOD], [0f32; PERIOD]);
        let (mut peak, mut caught) = (0f32, false);
        for p in 0..periods {
            let d = tone(110.0, drums, p * PERIOD);
            let i = tone(220.0, line, p * PERIOD);
            if drums > 0.0 {
                let mut pads = h.pads();
                let (pl, pr) = pads.pad(0, 0).unwrap();
                pl[..PERIOD].iter_mut().zip(&d).for_each(|(o, x)| *o += x);
                pr[..PERIOD].iter_mut().zip(&d).for_each(|(o, x)| *o += x);
            }
            let ctx = Ctx::new(RATE, (p * PERIOD) as u64);
            let (pk, c) = h.period(&ctx, (&z, &z), (&z, &z), (&i, &i), &mut l, &mut r);
            // The loudest of the last half: after the glides.
            if p >= periods / 2 {
                peak = peak.max(pk);
            }
            caught |= c;
        }
        (peak, caught)
    }

    /// UNDO on the mixer sets a pad's mute back to the pad's own, not to
    /// Drop drums' held over it.
    #[test]
    fn a_pads_mute_reads_as_its_own_under_drop_drums() {
        let mut h = MixHost::new(RATE, -6.0, -6.0);
        h.play(PlayFx::PadMute { pad: 2, mute: true });
        h.play(PlayFx::Drop(0b11));
        assert_eq!(h.value(StripId::Pad(2), mx::MUTE), 1.0);
        assert_eq!(
            h.value(StripId::Pad(0), mx::MUTE),
            0.0,
            "dropped, not muted"
        );
    }

    /// Step 10: a pad mute is a track's. The mixer keeps only kit 1's pad
    /// channels muted in the Mix tab (`chanmutes`); a project saved before
    /// kept kit 1's pad mutes and solos in the mixer part (`padmutes`,
    /// `padsolos`): they open as the old ones, for the tracks to take
    /// once, and mute no channel.
    #[test]
    fn old_pad_mutes_open_for_the_tracks_and_the_mix_tabs_mutes_stay() {
        let open = |text: &str| {
            let mut h = MixHost::new(RATE, -6.0, 0.0);
            let load =
                crate::project::mixer_load(text, None, RATE, &crate::plugins::Plugins::none())
                    .unwrap();
            h.open(Box::new(load));
            for _ in 0..400 {
                h.tend_open();
                h.run_silent_period();
            }
            h
        };
        let muted = |h: &MixHost| -> u16 {
            (0..PADS)
                .filter(|&p| h.mixer.param(StripId::Pad(p), mx::MUTE) >= 0.5)
                .fold(0, |m, p| m | 1 << p)
        };
        let text = crate::project::mixer_text(&MixHost::new(RATE, -6.0, 0.0).mixer, 0b100);
        assert!(text.contains("\nchanmutes 4\n"), "{text}");
        let mut h = open(&text);
        assert_eq!((h.pad_mutes(), muted(&h)), (0b100, 0b100));
        assert_eq!(h.take_old_pad_mutes(), None);
        let mut h = open(&text.replace("chanmutes 4", "padmutes 5\npadsolos 1"));
        assert_eq!((h.pad_mutes(), muted(&h)), (0, 0), "no channel muted");
        assert_eq!(h.take_old_pad_mutes(), Some((5, 1)));
        assert_eq!(h.take_old_pad_mutes(), None, "taken once");
    }

    /// Jay, 16 Nov: a delay or reverb put on a send return starts fully
    /// wet; everywhere else, and every other effect, keeps its default.
    #[test]
    fn a_delay_or_reverb_on_a_return_starts_wet() {
        let mix = |n: &dyn Node| {
            let id = n.params().iter().find(|p| p.name == "Mix").map(|p| p.id);
            n.param(id.unwrap_or_else(|| panic!("{} has a Mix", n.info().id)))
        };
        let fresh = |id: &str, fixed: &[(u16, f32)]| {
            let mut n = mpfree_fx_ui::make(id).unwrap_or_else(|| panic!("{id}"));
            for &(p, v) in fixed {
                n.set_param(p, v);
            }
            n
        };
        let mut checked = 0;
        for f in mpfree_fx::families::FAMILIES {
            if !matches!(f.name, "Delay" | "Reverb") {
                continue;
            }
            for m in f.modes {
                let mut on_return = fresh(m.id, m.fixed);
                let default = mix(on_return.as_ref());
                assert!(default < 100.0, "{} {} starts partly dry", f.name, m.label);
                wet_on_a_return(StripId::Return(1), on_return.as_mut());
                assert_eq!(mix(on_return.as_ref()), 100.0, "{} on a return", m.label);
                for strip in [StripId::Track(0), StripId::Bus(0), StripId::Master] {
                    let mut n = fresh(m.id, m.fixed);
                    wet_on_a_return(strip, n.as_mut());
                    assert_eq!(mix(n.as_ref()), default, "{} on {strip:?}", m.label);
                }
                checked += 1;
            }
        }
        assert_eq!(
            checked, 9,
            "Room, Plate, Spring, Hall, Cloud, Echoes; three delays"
        );
        // Other effects with a Mix keep it, on a return too.
        for id in [
            "mpfree.fx.granulator",
            "mpfree.fx.drive",
            "mpfree.fx.chorus",
        ] {
            let mut n = fresh(id, &[]);
            let default = mix(n.as_ref());
            wet_on_a_return(StripId::Return(0), n.as_mut());
            assert_eq!(mix(n.as_ref()), default, "{id}");
        }
    }

    #[test]
    fn the_engines_reach_the_master_at_the_level() {
        let mut h = MixHost::new(RATE, -6.0, -6.0);
        let (peak, caught) = run(&mut h, 200, 0.5, 0.0);
        // 0.5 at -6 dB (about 0.25): pad 1 and the drums track at 0 dB, the master at the Level.
        assert!((0.24..0.26).contains(&peak), "{peak}");
        assert!(!caught);
        // The Level never goes above --gain-db.
        h.set_level(0.0);
        run(&mut h, 2, 0.5, 0.0);
        assert_eq!(h.level(), -6.0);
        h.set_level(-12.0);
        let (peak, _) = run(&mut h, 200, 0.5, 0.0);
        assert!((0.12..0.13).contains(&peak), "{peak}");
    }

    /// Kit 1's pads go into their pad channels; another kit's into the
    /// channel of the first track that plays it (`homes`), which then
    /// shows in the Tracks view; a kit no track has goes onto the pads.
    #[test]
    fn a_kit_plays_into_its_tracks_channel() {
        let mut h = MixHost::new(RATE, -6.0, -6.0);
        // Kit 2 on track 3; kit 3 on no track.
        let homes = [0u8, 3, 0];
        let mut pads = h.pads_homed(&homes);
        pads.pad(0, 5).unwrap().0[0] = 0.25;
        pads.pad(1, 5).unwrap().0[0] = 0.5;
        pads.pad(1, 9).unwrap().0[1] = 0.125;
        pads.pad(2, 7).unwrap().0[0] = 1.0;
        assert!(
            pads.pad(1, PADS).is_some(),
            "every pad of kit 2 reaches its track"
        );
        assert_eq!(
            pads.played(),
            1 << 5 | 1 << 7,
            "the pad channels kit 1 and kit 3 played"
        );
        let track = crate::instruments::first_track() + 2;
        let (l, _) = h.mixer.track_input(track).unwrap();
        assert_eq!((l[0], l[1]), (0.5, 0.125), "both of kit 2's pads, added");
        assert_eq!(h.mixer.pad_input(5).unwrap().0[0], 0.25);
        assert_eq!(h.mixer.pad_input(7).unwrap().0[0], 1.0);
        assert_eq!(h.channels_in_use(), 1 << 2);
        assert!(h.track_strips().contains(&StripId::Track(track)));
        // Kit 2 on no track again: its channel goes.
        let _ = h.pads_homed(&[0, 0]);
        assert_eq!(h.channels_in_use(), 0);
        assert!(!h.track_strips().contains(&StripId::Track(track)));
    }

    /// A track's channel (R3 part 4b): Drums for track 1, its own past
    /// the engines' for the rest, the same numbers the clips' "this
    /// track's strip" locks reach.
    #[test]
    fn track_1s_channel_is_drums_and_the_rest_their_own() {
        use mpfree_play::mpfree_param::{strip_track, track_strip, Strip};
        assert_eq!(track_channel(0), DRUMS);
        assert_eq!(track_channel(1), TRACKS + 1);
        for t in 0..crate::instruments::TRACKS {
            let ch = track_channel(t);
            assert_eq!(track_strip(t as u8), Strip::Track(ch as u8), "track {t}");
            assert_eq!(strip_track(Strip::Track(ch as u8)), Some(t as u8));
        }
        // A kit homed on track 1 plays into Drums, and no own channel
        // shows for it.
        let mut h = MixHost::new(RATE, -6.0, -6.0);
        let mut pads = h.pads_homed(&[0, 1]);
        pads.pad(1, 5).unwrap().0[0] = 0.5;
        assert_eq!(h.mixer.track_input(DRUMS).unwrap().0[0], 0.5);
        assert_eq!(h.kit_track(1), DRUMS);
        assert_eq!(h.channels_in_use(), 1);
        assert_eq!(h.own_channels_in_use(), 0);
        assert!(!h.track_strips().contains(&StripId::Track(TRACKS)));
        assert!(!h.old_first());
    }

    /// A track's own instrument's channel is named for the instrument
    /// (step 4d), track 1's on the Drums strip too: on the Mix tab, in the
    /// chain editor and in the lists other screens pick from.
    #[test]
    fn an_instruments_channel_is_named_for_it() {
        use crate::synth::Engine;
        let synth = crate::synth::Synth::new(RATE);
        let mut h = MixHost::new(RATE, -6.0, -6.0);
        for (t, e) in [(0, Engine::Poly), (2, Engine::Fm)] {
            assert!(h.build_instrument(crate::instrument_job(&synth, t, e.node_id(), None)));
            assert!(h.poll_instruments().is_some());
        }
        let fm = StripId::Track(track_channel(2));
        // Where each sounds is its slot's: the holder's channel when lent.
        assert_eq!(h.instruments.strip(0), Some(DRUMS));
        assert_eq!(h.instruments.strip(2), Some(track_channel(2)));
        assert_eq!(h.instruments.strip(1), None);
        let looper = |h: &MixHost, i| h.instrument_strip(i, &[]);
        assert_eq!(looper(&h, crate::looping::Instrument::Own(2)), Some(fm));
        assert_eq!(
            h.own_instrument_on(StripId::Track(DRUMS)),
            Some((0, "Poly"))
        );
        assert_eq!(h.own_instrument_on(fm), Some((2, "FM")));
        assert_eq!(h.own_instrument_on(StripId::Track(SYNTH)), None);
        assert_eq!(h.strip_name(StripId::Track(DRUMS)), "Poly \u{b7} Track 1");
        assert_eq!(h.strip_name(fm), "FM \u{b7} Track 3");
        assert_eq!(h.strip_name(StripId::Track(SYNTH)), "Synth");
        h.show(Some(mpfree_mixer_ui::MixView::Tracks));
        let st = h.poll_state(Instant::now()).unwrap();
        let shown: Vec<&str> = st.strips.iter().map(|v| v.name.as_str()).collect();
        assert_eq!(
            shown,
            ["Poly \u{b7} Track 1", "Synth", "Loop", "FM \u{b7} Track 3"]
        );
        // Perform's On and the Looper's Records from: the track's own name
        // when it has one.
        let names = [String::new(), String::new(), "Bass".to_string()];
        let own = |s| h.own_instrument_on(s);
        let strips = throw_strips(&h.mixer, h.own_channels_in_use(), &names, &own);
        let named: Vec<&str> = strips.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(
            named[..6],
            [
                "Master",
                "Poly \u{b7} Track 1",
                "Synth",
                "Loop",
                "Audio (old)",
                "FM \u{b7} Bass"
            ]
        );
        // A new sound in track 3 keeps its strip, named for the new one.
        let poly = crate::instrument_job(&synth, 2, Engine::Poly.node_id(), None);
        assert!(h.build_instrument(poly));
        assert!(h.poll_instruments().is_some());
        assert_eq!(h.instruments.strip(2), Some(track_channel(2)));
        assert_eq!(h.strip_name(fm), "Poly \u{b7} Track 3");
    }

    /// A free place's instrument (step 6, a synth picked for the pads)
    /// sounds on a strip of its own past every track's, named for who
    /// plays it: "FM · Pads" while the pads do, else the first track that
    /// plays it. It shows on the Tracks view and in the lists while it is
    /// there.
    #[test]
    fn a_free_places_instrument_has_its_own_strip_named_for_who_plays_it() {
        use crate::instruments::free;
        use crate::synth::Engine;
        let synth = crate::synth::Synth::new(RATE);
        let mut h = MixHost::new(RATE, -6.0, -6.0);
        assert!(h.mixer.tracks() > free_channel(instruments::FREE - 1));
        let job = crate::instrument_job(&synth, free(1), Engine::Fm.node_id(), None);
        assert!(h.build_instrument(job));
        assert!(h.poll_instruments().is_some());
        let strip = StripId::Track(free_channel(1));
        assert_eq!(h.instruments.strip(free(1)), Some(free_channel(1)));
        assert_eq!(h.strip_name(strip), "FM \u{b7} Pads", "nobody plays it yet");
        h.instruments.set_send(4, Some(free(1))).unwrap();
        assert_eq!(h.strip_name(strip), "FM \u{b7} Track 5");
        assert!(h.instruments.pads_play(Some(free(1))));
        assert_eq!(h.strip_name(strip), "FM \u{b7} Pads");
        assert!(h.track_strips().contains(&strip));
        h.show(Some(mpfree_mixer_ui::MixView::Tracks));
        let st = h.poll_state(Instant::now()).unwrap();
        assert_eq!(
            st.strips.last().map(|v| v.name.as_str()),
            Some("FM \u{b7} Pads")
        );
        let own = |s| h.own_instrument_on(s);
        let strips = throw_strips(&h.mixer, h.own_channels_in_use(), &[], &own);
        assert!(strips
            .iter()
            .any(|(n, s)| *s == strip && n == "FM \u{b7} Pads"));
        // Taken out: the strip goes from the Tracks view.
        assert!(h.instruments.pads_play(None));
        h.instruments.set_send(4, None).unwrap();
        assert!(h.instruments.take_out(free(1)));
        assert_eq!(name(strip), "Free 2");
    }

    /// A project saved with settings on track 1's old own channel opens
    /// with track 1 kept there, so it sounds as it did: said in the log,
    /// counted, and the strip named so while in use. One without them
    /// opens with track 1 on Drums.
    #[test]
    fn an_old_project_keeps_track_1_on_its_old_channel() {
        let open = |text: &str, own: bool| {
            let mut h = MixHost::new(RATE, -6.0, 0.0);
            let mut load =
                crate::project::mixer_load(text, None, RATE, &crate::plugins::Plugins::none())
                    .unwrap();
            load.track1_own = own;
            let notes = load.notes.join("; ");
            h.open(Box::new(load));
            let mut said = None;
            for _ in 0..400 {
                if let Some(l) = h.tend_open() {
                    said = Some(l);
                }
                h.run_silent_period();
            }
            (h, said.unwrap_or_default() + &notes)
        };
        // A project saved before Drums was track 1's strip has no track1
        // line.
        let before = |text: String| -> String {
            assert!(text.contains("\ntrack1 t0\n"), "{text}");
            text.lines()
                .filter(|l| !l.starts_with("track1 "))
                .map(|l| format!("{l}\n"))
                .collect()
        };
        // Settings on its old own channel: track 1 keeps it.
        let mut a = MixHost::new(RATE, -6.0, 0.0);
        a.mixer.set_now(StripId::Track(TRACKS), mx::VOLUME, -9.0);
        let (h, said) = open(&before(a.mixer_text()), false);
        assert!(h.old_first());
        assert_eq!(h.track_channel(0), TRACKS);
        assert_eq!(h.track_channel(2), TRACKS + 2);
        assert_eq!(h.old_first_opens, 1);
        assert!(
            said.contains("track 1 keeps its old own channel (\"Track 1 (old)\": the project has settings on it)"),
            "{said}"
        );
        assert_eq!(name(StripId::Track(TRACKS)), OLD_TRACK_1);
        // A kit homed on track 1 plays there, and it shows.
        let mut h = h;
        let mut pads = h.pads_homed(&[0, 1]);
        pads.pad(1, 5).unwrap().0[0] = 0.5;
        assert_eq!(h.mixer.track_input(TRACKS).unwrap().0[0], 0.5);
        assert_eq!(h.own_channels_in_use(), 1);
        assert!(h.track_strips().contains(&StripId::Track(TRACKS)));
        // Saved again, it says so, and opens the same.
        let text = h.mixer_text();
        assert!(text.contains("\ntrack1 t4\n"), "{text}");
        let (h, said) = open(&text, false);
        assert_eq!(h.track_channel(0), TRACKS);
        assert!(said.contains("(\"Track 1 (old)\": saved so)"), "{said}");
        // Its own instrument played there, with nothing set: it keeps it
        // too, so it sounds through no effects of Drums'.
        let plain = MixHost::new(RATE, -6.0, 0.0).mixer_text();
        let (h, said) = open(&before(plain.clone()), true);
        assert_eq!(h.track_channel(0), TRACKS);
        assert!(said.contains("its instrument played there"), "{said}");
        // Nothing played there: Drums.
        let (h, said) = open(&before(plain.clone()), false);
        assert!(!h.old_first());
        assert!(!said.contains("old own channel"), "{said}");
        // A project saved since: where it says, whatever track 1 plays.
        let (h, said) = open(&plain, true);
        assert_eq!(h.track_channel(0), DRUMS);
        assert_eq!(h.old_first_opens, 0);
        assert!(!said.contains("old own channel"), "{said}");
        // A strip track 1 can't be: Drums, and said.
        let (h, said) = open(&plain.replace("track1 t0", "track1 t9"), true);
        assert_eq!(h.track_channel(0), DRUMS);
        assert!(
            said.contains("track 1 can't play into t9; it plays into Drums"),
            "{said}"
        );
    }

    /// Kit 1's pads past its first 16 (PAD BANK B to H) have no pad
    /// channel: they play straight into the Drums track, and the looper's
    /// dry drums take them in.
    #[test]
    fn kit_ones_pads_past_16_play_into_the_drums_track() {
        let mut h = MixHost::new(RATE, -6.0, -6.0);
        let mut pads = h.pads_homed(&[0]);
        pads.pad(0, 20).expect("pad B5 is heard").0[0] = 0.5;
        pads.pad(0, 127).unwrap().0[0] += 0.25;
        pads.pad(0, 3).unwrap().0[0] = 0.125;
        assert_eq!(pads.played(), 1 << 3 | DRUMS_DIRECT);
        let played = pads.played();
        assert_eq!(h.mixer.track_input(DRUMS).unwrap().0[0], 0.75);
        let (mut l, mut r) = ([0f32; PERIOD], [0f32; PERIOD]);
        h.drums(played, &mut l, &mut r);
        assert_eq!(l[0], 0.875, "pad 4 and B5 and H16");
        let z = [0f32; PERIOD];
        let (mut ol, mut or) = ([0f32; PERIOD], [0f32; PERIOD]);
        for p in 0..8 {
            let d = tone(110.0, 0.5, p * PERIOD);
            let mut pads = h.pads_homed(&[0]);
            let (pl, pr) = pads.pad(0, 20).unwrap();
            pl[..PERIOD].iter_mut().zip(&d).for_each(|(o, x)| *o += x);
            pr[..PERIOD].iter_mut().zip(&d).for_each(|(o, x)| *o += x);
            let ctx = Ctx::new(RATE, (p * PERIOD) as u64);
            h.period(&ctx, (&z, &z), (&z, &z), (&z, &z), &mut ol, &mut or);
        }
        assert!(ol.iter().any(|s| s.abs() > 0.05), "heard through Drums");
    }

    /// Any kit's pad past kit 1's A1-A16 is lent a channel of its own when
    /// it asks (R3 part 4a): named by its pad and kit, playing into its
    /// kit's track (and following it to another), its sound through it;
    /// kept while something is set on it, given back once it is plain.
    /// With every channel lent, a pad gets none.
    #[test]
    fn a_pad_is_lent_a_channel_into_its_kits_track_and_gives_it_back_plain() {
        use mpfree_fx::ids::mixer::output;
        let first = crate::instruments::first_track();
        let mut h = MixHost::new(RATE, -6.0, -6.0);
        let _ = h.pads_homed(&[0, 2]);
        assert_eq!(h.pad_channels.channel(1, 20), None);
        assert_eq!(
            h.lend_pad(0, 3),
            Some(StripId::Pad(3)),
            "kit 1's A4 has its own"
        );
        let s = h.lend_pad(1, 20).expect("a channel for kit 2's B5");
        assert_eq!(s, StripId::Pad(PADS));
        assert_eq!(h.lend_pad(1, 20), Some(s), "the same one again");
        assert_eq!(h.strip_name(s), "B5 · Kit 2");
        assert_eq!(h.pad_channels.holder(s), Some((1, 20)));
        assert_eq!(h.mixer.param(s, mx::OUTPUT), output::TRACK);
        assert_eq!(h.mixer.param(s, mx::OUTPUT_TRACK), (first + 1) as f32);
        // Its sound goes in through it.
        let mut pads = h.pads_homed(&[0, 2]);
        pads.pad(1, 20).expect("B5 is heard").0[0] = 0.5;
        pads.pad(1, 21).unwrap().0[0] = 0.25;
        assert_eq!(pads.played(), 1 << LENT_BIT);
        assert_eq!(h.mixer.pad_input(PADS).unwrap().0[0], 0.5);
        assert_eq!(
            h.mixer.track_input(first + 1).unwrap().0[0],
            0.25,
            "B6 has none"
        );
        // The mixer's Pads view: kit 1's 16, then the lent one by its name.
        h.show(Some(MixView::Pads));
        let st = h.poll_state(Instant::now()).unwrap();
        assert_eq!(st.counts[1], PADS + 1);
        assert_eq!(st.strips.len(), PADS + 1);
        assert_eq!(st.strips[PADS].name, "B5 · Kit 2");
        h.show(None);
        // Kit 2 to another track: the channel follows.
        let _ = h.pads_homed(&[0, 3]);
        assert_eq!(h.mixer.param(s, mx::OUTPUT_TRACK), (first + 2) as f32);
        // Something set keeps it, and the save names its pad.
        h.mixer.set_now(s, mx::VOLUME, -4.0);
        h.give_back_plain();
        assert_eq!(h.pad_channels.channel(1, 20), Some(s), "kept");
        let text = h.mixer_text();
        assert!(
            text.contains(&format!("lent {} 1 20", crate::project::strip_text(s))),
            "{text}"
        );
        // Plain again: given back, at its defaults.
        h.mixer.set_now(s, mx::VOLUME, 0.0);
        h.give_back_plain();
        assert_eq!(h.pad_channels.channel(1, 20), None, "given back");
        assert_eq!(h.mixer.param(s, mx::OUTPUT), 0.0);
        assert!(!h.mixer_text().contains("lent "));
        // Every channel lent: none for the next pad.
        for pad in 0..LENT {
            assert!(h.lend_pad(2, pad).is_some());
        }
        assert_eq!(h.lend_pad(2, LENT), None);
    }

    /// The looper's pads, wet: the master's input without the loops, the
    /// same whether a loop plays or not, with the Drums' echo from the
    /// delay on Return A in it; and the reason it can't be had once the
    /// Loop track, playing, goes to a submix.
    #[test]
    fn the_pads_wet_are_the_mix_without_the_loops() {
        use mpfree_fx::ids::mixer::output;
        // Pad 1 a burst at the start, the synth a tone, the loop (if
        // `looping`) a tone of its own; the Drums send to the echo.
        let go = |looping: bool, to_bus: bool| {
            let mut h = MixHost::new(RATE, -6.0, -6.0);
            h.play(PlayFx::Strip {
                strip: StripId::Track(DRUMS),
                id: mx::send(DELAY_RETURN),
                value: 0.0,
            });
            // The Play page's "off" send is -60 dB; truly off here, so the
            // loop is left out to the last bit.
            h.mixer.set_now(
                StripId::Track(LOOP),
                mx::send(DELAY_RETURN),
                mpfree_fx::SILENT_DB,
            );
            if to_bus {
                h.mixer
                    .set_now(StripId::Track(LOOP), mx::OUTPUT, output::bus(0));
            }
            let z = [0f32; PERIOD];
            let (mut wet, mut why) = (Vec::new(), None);
            for p in 0..400 {
                let at = p * PERIOD;
                if p < 40 {
                    let d = tone(110.0, 0.5, at);
                    let mut pads = h.pads();
                    let (pl, pr) = pads.pad(0, 0).unwrap();
                    pl[..PERIOD].copy_from_slice(&d);
                    pr[..PERIOD].copy_from_slice(&d);
                }
                let s = tone(330.0, 0.3, at);
                let lp = if looping { tone(550.0, 0.4, at) } else { z };
                // What the looper takes, from the last period's mix.
                let (mut wl, mut wr) = ([0f32; PERIOD], [0f32; PERIOD]);
                match h.pads_wet(&mut wl, &mut wr) {
                    Ok(latency) => assert_eq!(latency, 0),
                    Err(e) => why = Some(e),
                }
                wet.extend(wl.iter().zip(&wr).flat_map(|(a, b)| [*a, *b]));
                let ctx = Ctx::new(RATE, at as u64);
                let (mut l, mut r) = ([0f32; PERIOD], [0f32; PERIOD]);
                h.period(&ctx, (&s, &s), (&lp, &lp), (&z, &z), &mut l, &mut r);
            }
            (wet, why)
        };
        let (with, why) = go(true, false);
        assert_eq!(why, None);
        let (without, _) = go(false, false);
        let worst = with
            .iter()
            .zip(&without)
            .fold(0f32, |w, (a, b)| w.max((a - b).abs()));
        assert!(worst < 1e-6, "the loop is left out: {worst}");
        // The echo: the burst is over after 40 periods (0.12 s); the 1/8
        // dotted delay brings it back 0.375 s later, on top of the synth.
        let rms = |x: &[f32]| (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt();
        let frame = |s: f64| 2 * (s * f64::from(RATE)) as usize;
        let before = rms(&without[frame(0.2)..frame(0.35)]);
        let echo = rms(&without[frame(0.4)..frame(0.5)]);
        assert!(echo > 1.2 * before, "the echo is in it: {echo} vs {before}");
        // The Loop track on a submix: it can't be taken out.
        let (_, why) = go(true, true);
        assert_eq!(why, Some("the Loop track goes to a submix"));
    }

    /// An audio track plays into its own channel (the audio tracks
    /// rethink, phase 1b): the looper's pads, wet, leave it out as they
    /// leave out the loops, and say why they can't once it goes to a
    /// submix. Freed, its capture point takes nothing out.
    #[test]
    fn the_pads_wet_leave_out_each_audio_track_s_own_channel() {
        use mpfree_fx::ids::mixer::output;
        let go = |audio: bool, to_bus: bool, point: bool| {
            let mut h = MixHost::new(RATE, -6.0, -6.0);
            let ch = h.track_channel(2);
            h.audio_tracks = 1 << 2;
            if point {
                h.point_audio_out(0, Some(ch));
                // Once is enough: the same channel again moves nothing.
                h.point_audio_out(0, Some(ch));
            }
            if to_bus {
                h.mixer
                    .set_now(StripId::Track(ch), mx::OUTPUT, output::bus(0));
            }
            let z = [0f32; PERIOD];
            let (mut wet, mut why) = (Vec::new(), None);
            for p in 0..60 {
                let at = p * PERIOD;
                let (mut wl, mut wr) = ([0f32; PERIOD], [0f32; PERIOD]);
                if let Err(e) = h.pads_wet(&mut wl, &mut wr) {
                    why = Some(e);
                }
                wet.extend(wl.iter().zip(&wr).flat_map(|(a, b)| [*a, *b]));
                let s = tone(330.0, 0.3, at);
                if audio {
                    let a = tone(550.0, 0.4, at);
                    let (l, r) = h.mixer.track_input(ch).unwrap();
                    l[..PERIOD].copy_from_slice(&a);
                    r[..PERIOD].copy_from_slice(&a);
                }
                let ctx = Ctx::new(RATE, at as u64);
                let (mut l, mut r) = (z, z);
                h.period(&ctx, (&s, &s), (&z, &z), (&z, &z), &mut l, &mut r);
            }
            (wet, why)
        };
        let (with, why) = go(true, false, true);
        assert_eq!(why, None);
        let (without, _) = go(false, false, true);
        let worst = with
            .iter()
            .zip(&without)
            .fold(0f32, |w, (a, b)| w.max((a - b).abs()));
        assert!(worst < 1e-6, "the audio track is left out: {worst}");
        // Not pointed at it: it is in the pads.
        let (heard, _) = go(true, false, false);
        let most = heard
            .iter()
            .zip(&without)
            .fold(0f32, |w, (a, b)| w.max((a - b).abs()));
        assert!(most > 0.1, "{most}");
        let (_, why) = go(true, true, true);
        assert_eq!(why, Some("an audio track goes to a submix"));
    }

    /// The master is refused while the Loop track's sound is anywhere
    /// else that plays into it (the design review of 20 Nov: tails and
    /// indirect routes): a send to a return, a submix, a submix's send, and
    /// after each comes down, for as long as the effects there ring with it
    /// (Return A's delay from the start). Silence while refused, never a
    /// sound in its place; a track or a submix is still fine.
    #[test]
    fn the_master_is_refused_while_the_loop_is_heard_anywhere_else_and_its_echoes_ring() {
        use mpfree_fx::ids::mixer::output;
        let mut h = MixHost::new(RATE, -6.0, -6.0);
        let lp = StripId::Track(LOOP);
        // A silent period: the mixer lands its inserts (Return A's delay)
        // and the host watches where the loop is (`watch_loop_echoes`), as
        // each period does.
        let mut at = 0u64;
        let mut period = |h: &mut MixHost| {
            let z = [0f32; PERIOD];
            let (mut l, mut r) = (z, z);
            h.period(
                &Ctx::new(RATE, at),
                (&z, &z),
                (&z, &z),
                (&z, &z),
                &mut l,
                &mut r,
            );
            at += PERIOD as u64;
        };
        // The inserts land once their strips have ducked out.
        for _ in 0..100 {
            if h.mixer.edits_pending() == 0 {
                break;
            }
            period(&mut h);
        }
        assert_eq!(h.mixer.edits_pending(), 0);
        let take = |h: &mut MixHost| {
            let (mut l, mut r) = ([1f32; PERIOD], [1f32; PERIOD]);
            let got = h.loop_take(Some(StripId::Master), &mut l, &mut r);
            if got.is_err() {
                assert!(l.iter().chain(&r).all(|&x| x == 0.0), "silence");
            }
            got.err()
        };
        let send = mx::send(DELAY_RETURN);
        assert_eq!(h.mixer.param(lp, send), SEND_OFF_DB);
        assert_eq!(take(&mut h), None);
        // A send up: refused.
        h.mixer.set_now(lp, send, -12.0);
        period(&mut h);
        assert_eq!(take(&mut h), Some(LOOP_SENDS_HEARD[DELAY_RETURN]));
        assert!(LOOP_SENDS_HEARD[DELAY_RETURN].contains("Return A"));
        // Down again: refused while Return A's delay rings, then fine.
        let ring = h.rings(StripId::Return(DELAY_RETURN));
        assert!(
            ring > RATE as u64 / 4 && ring < RINGS_ON,
            "the delay rings: {ring}"
        );
        h.mixer.set_now(lp, send, SEND_OFF_DB);
        let periods = ring.div_ceil(PERIOD as u64);
        for _ in 0..periods - 1 {
            period(&mut h);
            assert_eq!(take(&mut h), Some(LOOP_ECHOES_RING[DELAY_RETURN]));
        }
        period(&mut h);
        assert_eq!(
            take(&mut h),
            None,
            "after {periods} periods, its echoes died away"
        );
        // A track and a submix stay fine while the master is refused.
        h.mixer.set_now(lp, send, -12.0);
        let (mut l, mut r) = ([1f32; PERIOD], [1f32; PERIOD]);
        assert!(h
            .loop_take(Some(StripId::Track(SYNTH)), &mut l, &mut r)
            .is_ok());
        assert!(h.loop_take(Some(StripId::Bus(1)), &mut l, &mut r).is_ok());
        h.mixer.set_now(lp, send, SEND_OFF_DB);
        h.loop_echoes = [0; mpfree_fx::RETURNS];
        // Through a submix: refused while the Loop track goes there, its
        // send to Return A counts as the loop's, and both ring after.
        h.mixer.set_now(lp, mx::OUTPUT, output::bus(0));
        period(&mut h);
        assert_eq!(take(&mut h), Some(LOOP_IN_SUBMIX));
        h.mixer.set_now(StripId::Bus(0), send, -12.0);
        period(&mut h);
        h.mixer.set_now(lp, mx::OUTPUT, output::MASTER);
        h.mixer.set_now(StripId::Bus(0), send, SEND_OFF_DB);
        period(&mut h);
        assert_eq!(take(&mut h), Some(LOOP_ECHOES_RING[DELAY_RETURN]));
        for _ in 0..periods {
            period(&mut h);
        }
        assert_eq!(take(&mut h), None);
        // A submix with effects rings after the Loop track leaves it.
        h.mixer.set_now(lp, mx::OUTPUT, output::bus(0));
        period(&mut h);
        h.loop_bus_echo = Some((StripId::Bus(0), 3 * PERIOD as u64));
        h.mixer.set_now(lp, mx::OUTPUT, output::MASTER);
        period(&mut h);
        assert_eq!(take(&mut h), Some(LOOP_SUBMIX_RINGS));
        period(&mut h);
        period(&mut h);
        assert_eq!(take(&mut h), None);
    }

    /// The looper's Pads: the channel each instrument plays
    /// into, whichever track the pads were on.
    #[test]
    fn the_instrument_strip_is_where_the_pads_instrument_sounds() {
        use crate::looping::Instrument;
        let h = MixHost::new(RATE, -6.0, -6.0);
        // Kit 1 (and a kit no track homes) through the pad channels to
        // Drums; the shared synth on Synth.
        let drums = Some(StripId::Track(DRUMS));
        assert_eq!(h.instrument_strip(Instrument::Kit(0), &[0, 0]), drums);
        assert_eq!(h.instrument_strip(Instrument::Kit(1), &[0, 0]), drums);
        let synth = Some(StripId::Track(SYNTH));
        assert_eq!(h.instrument_strip(Instrument::Synth, &[0, 0]), synth);
        // Kit 2 homed on track 3: its channel.
        let home = Some(StripId::Track(h.track_channel(2)));
        assert_eq!(h.instrument_strip(Instrument::Kit(1), &[0, 3]), home);
        // A track's own instrument: that track's channel.
        let own = StripId::Track(h.track_channel(4));
        assert_eq!(h.instrument_strip(Instrument::Own(4), &[0, 0]), Some(own));
        assert_eq!(name(own), "Track 5");
        // MIDI out: nothing in the mix.
        assert_eq!(h.instrument_strip(Instrument::MidiOut, &[0, 0]), None);
    }

    /// The looper's mix choices: a track after its fader (the loop and
    /// the drums left out), the master without the loop (the same as the
    /// pads here, with no audio track or line input playing), never the
    /// Loop track or a submix it goes to, and a strip not in the mix said.
    #[test]
    fn the_looper_records_a_mix_the_master_or_one_strip() {
        use mpfree_fx::ids::mixer::output;
        let go = |from: Option<StripId>, looping: bool, drums: bool, loop_bus: bool| {
            let mut h = MixHost::new(RATE, -6.0, -6.0);
            // The loop's echo (its send at -60 dB) stays in the master;
            // truly off here, so the loop is left out to the last bit.
            h.mixer.set_now(
                StripId::Track(LOOP),
                mx::send(DELAY_RETURN),
                mpfree_fx::SILENT_DB,
            );
            if loop_bus {
                h.mixer
                    .set_now(StripId::Track(LOOP), mx::OUTPUT, output::bus(0));
            }
            let z = [0f32; PERIOD];
            let (mut took, mut why) = (Vec::new(), None);
            for p in 0..60 {
                let at = p * PERIOD;
                if drums {
                    let d = tone(110.0, 0.5, at);
                    let mut pads = h.pads();
                    let (pl, pr) = pads.pad(0, 0).unwrap();
                    pl[..PERIOD].copy_from_slice(&d);
                    pr[..PERIOD].copy_from_slice(&d);
                }
                let s = tone(330.0, 0.3, at);
                let lp = if looping { tone(550.0, 0.4, at) } else { z };
                let (mut wl, mut wr) = ([1f32; PERIOD], [1f32; PERIOD]);
                match h.loop_take(from, &mut wl, &mut wr) {
                    Ok(_) => {}
                    Err(e) => why = Some(e),
                }
                took.extend(wl.iter().zip(&wr).flat_map(|(a, b)| [*a, *b]));
                let ctx = Ctx::new(RATE, at as u64);
                let (mut l, mut r) = ([0f32; PERIOD], [0f32; PERIOD]);
                h.period(&ctx, (&s, &s), (&lp, &lp), (&z, &z), &mut l, &mut r);
            }
            (took, why)
        };
        let same =
            |a: &[f32], b: &[f32]| a.iter().zip(b).fold(0f32, |w, (x, y)| w.max((x - y).abs()));
        let loud = |x: &[f32]| x.iter().fold(0f32, |m, v| m.max(v.abs()));
        // The Synth track: its own sound only.
        let synth = Some(StripId::Track(SYNTH));
        let (all, why) = go(synth, true, true, false);
        assert_eq!(why, None);
        assert!(
            all[..2 * PERIOD].iter().all(|&v| v == 0.0),
            "the period it moves is silence"
        );
        let (alone, _) = go(synth, false, false, false);
        assert!(same(&all, &alone) < 1e-6, "no loop and no drums in it");
        assert!(loud(&alone) > 0.05, "the synth is in it");
        // The master: all but the loop; here, what the pads take.
        let (master, why) = go(Some(StripId::Master), true, true, false);
        assert_eq!(why, None);
        let (pads, _) = go(None, false, true, false);
        assert!(same(&master, &pads) < 1e-6, "the master without the loop");
        // Never the Loop track, nor a submix it plays into.
        let (took, why) = go(Some(StripId::Track(LOOP)), true, true, false);
        assert_eq!(why, Some("the Loop track can't record itself"));
        assert_eq!(loud(&took), 0.0);
        let (_, why) = go(Some(StripId::Bus(0)), true, true, true);
        assert_eq!(why, Some("the Loop track goes to that submix"));
        let (_, why) = go(Some(StripId::Bus(1)), true, true, true);
        assert_eq!(why, None, "another submix is fine");
        let (_, why) = go(Some(StripId::Bus(250)), true, true, false);
        assert_eq!(why, Some("that strip isn't in the mix"));
    }

    #[test]
    fn the_samplers_capture_point_hears_the_strip_picked() {
        use crate::sampler::RecSource;
        use mpfree_fx::CapturePoint;
        let mut h = MixHost::new(RATE, -6.0, -6.0);
        assert!(h.rec_capture().is_none(), "free until a source is picked");
        let master = RecSource::Mix(StripId::Master, CapturePoint::AfterSafety);
        let pad1 = RecSource::Mix(StripId::Pad(0), CapturePoint::AfterFader);
        // One period of pad 1 playing 0.5; what came out, and what the
        // capture point took.
        let z = [0f32; PERIOD];
        let mut at = 0;
        let mut once = |h: &mut MixHost| {
            let d = tone(110.0, 0.5, at);
            {
                let mut pads = h.pads();
                let (pl, pr) = pads.pad(0, 0).unwrap();
                pl[..PERIOD].copy_from_slice(&d);
                pr[..PERIOD].copy_from_slice(&d);
            }
            let ctx = Ctx::new(RATE, at as u64);
            let (mut l, mut r) = ([0f32; PERIOD], [0f32; PERIOD]);
            h.period(&ctx, (&z, &z), (&z, &z), (&z, &z), &mut l, &mut r);
            at += PERIOD;
            let peak = |x: &[f32]| x.iter().fold(0f32, |m, v| m.max(v.abs()));
            let cap = h.rec_capture().map(|(cl, cr, lat)| {
                assert_eq!((cl.len(), cr.len()), (PERIOD, PERIOD));
                (cl == l.as_slice() && cr == r.as_slice(), peak(cl), lat)
            });
            (peak(&l), cap)
        };
        assert!(h.set_rec_source(master));
        for _ in 0..200 {
            once(&mut h);
        }
        let (out, cap) = once(&mut h);
        let (same, _, lat) = cap.unwrap();
        assert!(same, "the master as heard is what comes out");
        assert!((0.24..0.26).contains(&out), "{out}");
        assert!(lat < RATE / 20, "{lat} frames late");
        // Pad 1's channel after its fader: before the Level.
        assert!(h.set_rec_source(pad1));
        for _ in 0..50 {
            once(&mut h);
        }
        let (_, cap) = once(&mut h);
        let (same, peak, _) = cap.unwrap();
        assert!(!same && (0.49..0.51).contains(&peak), "{peak}");
        // Back to the line input: the capture point is free again.
        assert!(h.set_rec_source(RecSource::Line));
        once(&mut h);
        assert!(h.rec_capture().is_none());
        // A strip the mixer doesn't have.
        let none = RecSource::Mix(StripId::Pad(PAD_STRIPS), CapturePoint::AfterFader);
        assert!(!h.set_rec_source(none));
        assert!(h.rec_capture().is_none());
    }

    /// Step 9: an audio track takes a strip's sound from its own capture
    /// point, before its fader either way (its level never changes the
    /// take), silent the period the point moves; a strip not in the mix
    /// gives silence and None; None frees the point. The sampler's point
    /// is left alone.
    #[test]
    fn an_audio_track_takes_a_strip_before_its_fader() {
        use crate::audiotracks::Inside;
        let mut h = MixHost::new(RATE, -6.0, -6.0);
        h.set(StripId::Pad(0), mx::VOLUME, -6.0);
        let z = [0f32; PERIOD];
        let mut at = 0;
        let mut once = |h: &mut MixHost| {
            let d = tone(110.0, 0.5, at);
            {
                let mut pads = h.pads();
                let (pl, pr) = pads.pad(0, 0).unwrap();
                pl[..PERIOD].copy_from_slice(&d);
                pr[..PERIOD].copy_from_slice(&d);
            }
            let ctx = Ctx::new(RATE, at as u64);
            let (mut l, mut r) = ([0f32; PERIOD], [0f32; PERIOD]);
            h.period(&ctx, (&z, &z), (&z, &z), (&z, &z), &mut l, &mut r);
            at += PERIOD;
        };
        let peak = |x: &[f32]| x.iter().fold(0f32, |m, v| m.max(v.abs()));
        for _ in 0..50 {
            once(&mut h);
        }
        let mut buf = [1.0f32; 2 * PERIOD];
        let after = Inside {
            strip: StripId::Pad(0),
            after_effects: true,
        };
        assert!(h.audio_take(2, Some(after), &mut buf).is_some());
        assert_eq!(peak(&buf), 0.0, "silent the period it moves");
        once(&mut h);
        let lat = h.audio_take(2, Some(after), &mut buf).unwrap();
        assert!(lat < RATE / 20, "{lat}");
        let (left, right) = (
            peak(&buf[0..].iter().step_by(2).copied().collect::<Vec<_>>()),
            peak(&buf[1..].iter().step_by(2).copied().collect::<Vec<_>>()),
        );
        assert!((0.45..0.51).contains(&left), "before the fader: {left}");
        assert!((0.45..0.51).contains(&right), "{right}");
        // Before its effects: still the pad's sound (no effects on it).
        let before = Inside {
            after_effects: false,
            ..after
        };
        h.audio_take(2, Some(before), &mut buf).unwrap();
        assert_eq!(peak(&buf), 0.0, "moved again");
        once(&mut h);
        h.audio_take(2, Some(before), &mut buf).unwrap();
        assert!((0.45..0.51).contains(&peak(&buf)), "before: {}", peak(&buf));
        // A strip the mixer doesn't have.
        let gone = Inside {
            strip: StripId::Pad(PAD_STRIPS),
            after_effects: true,
        };
        assert_eq!(h.audio_take(2, Some(gone), &mut buf), None);
        assert_eq!(peak(&buf), 0.0);
        assert!(h.mixer.capture(AUDIO_CAPTURE + 2).is_none());
        // Freed; the sampler's point never moved.
        h.audio_take(2, Some(after), &mut buf).unwrap();
        assert!(h.mixer.capture(AUDIO_CAPTURE + 2).is_some());
        assert_eq!(h.audio_take(2, None, &mut buf), None);
        assert!(h.mixer.capture(AUDIO_CAPTURE + 2).is_none());
        assert!(h.rec_capture().is_none());
        assert_eq!(
            h.audio_take(mpfree_audiotracks::MAX_TRACKS, Some(after), &mut buf),
            None
        );
    }

    /// What an audio track can record from inside the mix: the
    /// instruments' channels, the submixes and the master, never the audio
    /// tracks themselves, the line input or a pad's channel.
    #[test]
    fn the_mix_offers_instruments_submixes_and_the_master() {
        let h = MixHost::new(RATE, -6.0, -6.0);
        let v = h.inside_sources();
        let strips: Vec<StripId> = v.iter().map(|x| x.1).collect();
        for s in [
            StripId::Track(DRUMS),
            StripId::Track(SYNTH),
            StripId::Track(LOOP),
        ] {
            assert!(strips.contains(&s), "{s:?}");
        }
        assert!(strips.contains(&StripId::Bus(0)));
        assert_eq!(strips.last(), Some(&StripId::Master));
        assert!(!strips.contains(&StripId::Track(AUDIO)));
        assert!(!strips.contains(&LINE));
        assert!(!strips
            .iter()
            .any(|s| matches!(s, StripId::Pad(_) | StripId::Return(_))));
        let names: Vec<&str> = v.iter().map(|x| x.0.as_str()).collect();
        assert!(
            names.contains(&"Drums") && names.contains(&"Master"),
            "{names:?}"
        );
    }

    #[test]
    fn the_line_input_is_heard_only_while_open_and_monitored() {
        let mut h = MixHost::new(RATE, 0.0, 0.0);
        assert_eq!(run(&mut h, 100, 0.0, 0.5).0, 0.0, "closed");
        h.follow(true, false);
        assert_eq!(run(&mut h, 100, 0.0, 0.5).0, 0.0, "open, not monitored");
        h.follow(true, true);
        let (peak, _) = run(&mut h, 200, 0.0, 0.5);
        assert!((0.49..0.51).contains(&peak), "{peak}");
        h.follow(false, true);
        assert!(run(&mut h, 200, 0.0, 0.5).0 < 1e-6, "closed again");
    }

    #[test]
    fn master_safety_catches_what_would_clip() {
        let mut h = MixHost::new(RATE, 0.0, 0.0);
        let (peak, caught) = run(&mut h, 400, 1.6, 0.0);
        assert!(caught);
        assert!(peak <= 1.0, "{peak}");
    }

    /// The audio tracks rethink, phase 1b: an audio track's own channel
    /// shows while its track is an audio track, and the old shared Audio
    /// strip, as "Audio (old)", only while it holds an older project's
    /// settings; it gets no sound either way.
    #[test]
    fn audio_tracks_show_their_own_channels_and_the_old_strip_only_with_settings() {
        let mut h = MixHost::new(RATE, -6.0, -6.0);
        h.audio_tracks = 1 << 2;
        h.show(Some(MixView::Tracks));
        let t0 = Instant::now();
        let st = h.poll_state(t0).unwrap();
        let ids: Vec<StripId> = st.strips.iter().map(|s| s.id).collect();
        assert!(ids.contains(&StripId::Track(h.track_channel(2))), "{ids:?}");
        assert!(!ids.contains(&StripId::Track(AUDIO)), "{ids:?}");
        assert!(!h.old_audio());
        h.mixer.set_now(StripId::Track(AUDIO), mx::VOLUME, -4.0);
        assert!(h.old_audio());
        h.state_now();
        let st = h.poll_state(t0).unwrap();
        let old = st
            .strip(StripId::Track(AUDIO))
            .expect("shown with settings");
        assert_eq!(old.name, OLD_AUDIO);
        // No longer an audio track: its channel goes.
        h.audio_tracks = 0;
        h.state_now();
        let st = h.poll_state(t0).unwrap();
        assert!(st.strip(StripId::Track(h.track_channel(2))).is_none());
    }

    #[test]
    fn the_panel_gets_states_while_it_shows_and_drives_the_mixer() {
        let mut h = MixHost::new(RATE, -6.0, -6.0);
        let t0 = Instant::now();
        assert!(h.poll_state(t0).is_none(), "closed");
        h.show(Some(MixView::Tracks));
        run(&mut h, 20, 0.5, 0.0);
        let st = h.poll_state(t0).expect("at once when opened");
        assert_eq!(
            st.strips
                .iter()
                .map(|s| s.name.as_str())
                .collect::<Vec<_>>(),
            ["Drums", "Synth", "Loop"],
            "the old shared Audio strip only with an older project's settings"
        );
        assert!(st.strips[0].peak[0] > 0.1);
        assert_eq!(st.master.volume_db, -6.0);
        assert!(
            h.poll_state(t0 + STATE_EVERY / 2).is_none(),
            "not again so soon"
        );
        assert!(h.poll_state(t0 + STATE_EVERY).is_some());
        // A fader on the panel; the master fader is the Level.
        assert_eq!(
            h.action(&MixAction::Set {
                strip: StripId::Track(DRUMS),
                id: mx::MUTE,
                value: 1.0
            }),
            None
        );
        assert_eq!(
            h.action(&MixAction::Set {
                strip: StripId::Master,
                id: mx::VOLUME,
                value: 3.0
            }),
            Some(-6.0)
        );
        assert_eq!(h.action(&MixAction::Select(StripId::Track(SYNTH))), None);
        run(&mut h, 200, 0.5, 0.0);
        let st = h.poll_state(t0 + STATE_EVERY * 3).unwrap();
        assert!(st.strips[0].mute);
        assert_eq!(st.selected, Some(StripId::Track(SYNTH)));
        // The meter held the mute's fade until that read; now it is silent.
        run(&mut h, 20, 0.5, 0.0);
        let st = h.poll_state(t0 + STATE_EVERY * 4).unwrap();
        assert_eq!(
            st.strip(StripId::Track(DRUMS)).unwrap().peak,
            [0.0; 2],
            "muted"
        );
        h.action(&MixAction::View(MixView::Inputs));
        let st = h.poll_state(t0 + STATE_EVERY * 4).unwrap();
        assert_eq!(
            (st.view, st.strips[0].name.as_str()),
            (MixView::Inputs, "Line in")
        );
        h.show(None);
        assert!(h.poll_state(t0 + STATE_EVERY * 10).is_none());
        assert_eq!(h.level(), -6.0);
    }

    #[test]
    fn the_pads_go_through_their_channels_into_the_drums_track() {
        let mut h = MixHost::new(RATE, 0.0, 0.0);
        // The sampler's pads 1 and 3 (kit 1's pads past 16 play into the
        // Drums track: `kit_ones_pads_past_16_play_into_the_drums_track`).
        let mut pads = h.pads();
        pads.pad(0, 0).unwrap().0[..PERIOD].fill(0.25);
        pads.pad(0, 2).unwrap().0[..PERIOD].fill(0.5);
        let played = pads.played();
        assert_eq!(played, 0b101);
        let (mut l, mut r) = ([0f32; PERIOD], [0f32; PERIOD]);
        h.drums(played, &mut l, &mut r);
        assert_eq!(
            (l[0], l[PERIOD - 1], r[0]),
            (0.75, 0.75, 0.0),
            "what the looper records"
        );
        // Pad 1 muted on Pad Mix: the rest still reach the master through Drums.
        h.action(&MixAction::Set {
            strip: StripId::Pad(0),
            id: mx::MUTE,
            value: 1.0,
        });
        h.show(Some(MixView::Pads));
        let st = h.poll_state(Instant::now()).unwrap();
        assert_eq!(st.strips.len(), PADS);
        assert_eq!(
            (st.strips[0].name.as_str(), st.strips[0].output_label()),
            ("Pad 1", "Track 1".to_string())
        );
        // (Master safety's DC blocker is still forgetting the first block's
        // DC: about -100 dBFS.)
        let muted = run(&mut h, 200, 0.5, 0.0).0;
        assert!(muted < 1e-4, "pad 1 muted: {muted}");
        h.action(&MixAction::Set {
            strip: StripId::Pad(0),
            id: mx::MUTE,
            value: 0.0,
        });
        let (peak, _) = run(&mut h, 200, 0.5, 0.0);
        assert!((0.49..0.51).contains(&peak), "{peak}");
    }

    #[test]
    fn the_throws_and_the_xy_pad_play_perform_fx() {
        let mut h = MixHost::new(RATE, 0.0, 0.0);
        let perform = |h: &MixHost, id: u16| {
            h.mixer
                .chain(StripId::Master)
                .and_then(|c| c.node(PERFORM_SLOT))
                .map(|n| n.param(id))
        };
        // The inserts go in once each strip has ducked out (about 10 ms).
        run(&mut h, 10, 0.0, 0.0);
        assert_eq!(h.mixer.edits_pending(), 0);
        assert_eq!(perform(&h, perf::MIX), Some(100.0));
        assert_eq!(h.play_levels().fx_mix, 1.0);
        h.play(PlayFx::Throw {
            fx: ThrowFx::Delay,
            down: true,
            how: None,
        });
        h.play(PlayFx::Xy {
            x: 0.0,
            y: 1.0,
            down: true,
        });
        run(&mut h, 2, 0.0, 0.0);
        assert_eq!(
            perform(&h, perf::hold(9)),
            Some(1.0),
            "the echo throw's pad"
        );
        assert_eq!(perform(&h, perf::DJ_FILTER), Some(-100.0), "low-pass");
        assert_eq!(perform(&h, perf::ECHO), Some(100.0));
        assert_eq!(h.xy_at(), (0.0, 1.0), "where Learn reads the pad");
        h.play(PlayFx::Throw {
            fx: ThrowFx::Delay,
            down: false,
            how: None,
        });
        h.play(PlayFx::Xy {
            x: 0.5,
            y: 0.0,
            down: false,
        });
        run(&mut h, 2, 0.0, 0.0);
        assert_eq!(perform(&h, perf::hold(9)), Some(0.0));
        assert_eq!(perform(&h, perf::DJ_FILTER), Some(0.0), "off in the middle");
        assert_eq!(h.xy_at(), (0.5, 0.0));
        // Each throw holds a pad of its own.
        let all = [
            ThrowFx::Delay,
            ThrowFx::Reverb,
            ThrowFx::Stutter,
            ThrowFx::TapeStop,
            ThrowFx::FilterDip,
            ThrowFx::BeatRepeat,
            ThrowFx::VinylBrake,
        ];
        let mut pads: Vec<usize> = all.iter().map(|t| t.pad()).collect();
        pads.sort_unstable();
        pads.dedup();
        assert_eq!(pads.len(), all.len());
    }

    /// The XY pad's chip picks what it plays: an XY FX effect goes on the
    /// master after the Perform FX, closed until a finger lands; the pad
    /// then plays X, Y and Active; Filter + echo plays the Perform FX
    /// again and the XY FX closes.
    #[test]
    fn the_xy_pads_pick_puts_an_xy_fx_on_the_master_and_plays_it() {
        use mpfree_fx::xyfx::ids as xy;
        let mut h = MixHost::new(RATE, 0.0, 0.0);
        run(&mut h, 10, 0.0, 0.0);
        assert_eq!(h.xy_slot(), None, "none until picked");
        h.play(PlayFx::XyEffect(Some(3)));
        run(&mut h, 10, 0.0, 0.0);
        let slot = h.xy_slot().expect("the XY FX on the master");
        assert!(slot > PERFORM_SLOT, "after the Perform FX");
        let xyfx = |h: &MixHost, id: u16| {
            h.mixer
                .chain(StripId::Master)
                .and_then(|c| c.node(slot))
                .map(|n| n.param(id))
        };
        assert_eq!(xyfx(&h, xy::EFFECT), Some(3.0), "Wash");
        assert_eq!(xyfx(&h, xy::ACTIVE), Some(0.0), "closed");
        h.play(PlayFx::Xy {
            x: 0.25,
            y: 1.0,
            down: true,
        });
        run(&mut h, 2, 0.0, 0.0);
        assert_eq!(xyfx(&h, xy::X), Some(25.0));
        assert_eq!(xyfx(&h, xy::Y), Some(100.0));
        assert_eq!(xyfx(&h, xy::ACTIVE), Some(1.0), "open while held");
        assert_eq!(h.xy_at(), (0.25, 1.0), "the XY FX's X and Y");
        let dj = |h: &MixHost| {
            h.mixer
                .chain(StripId::Master)
                .and_then(|c| c.node(PERFORM_SLOT))
                .map(|n| n.param(perf::DJ_FILTER))
        };
        assert_eq!(dj(&h), Some(0.0), "the DJ filter is left alone");
        h.play(PlayFx::Xy {
            x: 0.25,
            y: 1.0,
            down: false,
        });
        run(&mut h, 2, 0.0, 0.0);
        assert_eq!(xyfx(&h, xy::ACTIVE), Some(0.0), "lifting lets it go");
        // Another effect: the same insert, no second one.
        h.play(PlayFx::XyEffect(Some(1)));
        run(&mut h, 2, 0.0, 0.0);
        assert_eq!(xyfx(&h, xy::EFFECT), Some(1.0));
        let n = h.mixer.chain(StripId::Master).map_or(0, |c| c.len());
        // Filter + echo: the Perform FX again, the XY FX closed.
        h.play(PlayFx::XyEffect(None));
        h.play(PlayFx::Xy {
            x: 0.0,
            y: 1.0,
            down: true,
        });
        run(&mut h, 2, 0.0, 0.0);
        assert_eq!(dj(&h), Some(-100.0), "low-pass");
        assert_eq!(xyfx(&h, xy::ACTIVE), Some(0.0));
        assert_eq!(xyfx(&h, xy::EFFECT), Some(0.0), "on the Filter, idle");
        assert_eq!(h.mixer.chain(StripId::Master).map_or(0, |c| c.len()), n);
    }

    /// The XY FX's Active switch, with Wash picked and put on the master.
    fn xy_fx_on(h: &mut MixHost) -> impl Fn(&MixHost) -> Option<f32> {
        use mpfree_fx::xyfx::ids as xy;
        h.play(PlayFx::XyEffect(Some(3)));
        run(h, 10, 0.0, 0.0);
        let slot = h.xy_slot().expect("the XY FX on the master");
        move |h: &MixHost| {
            h.mixer
                .chain(StripId::Master)
                .and_then(|c| c.node(slot))
                .map(|n| n.param(xy::ACTIVE))
        }
    }

    fn xy(x: f32, y: f32, down: bool) -> PlayFx {
        PlayFx::Xy { x, y, down }
    }

    /// The ten ready-made XY layouts (Filter + echo and the nine XY FX)
    /// send what they sent before the XY engine (`mpfree-xy`) took them
    /// over, frame by frame: a scripted touch on the Play page's pad (a
    /// press, moves, off the edge, a lift, the glide home), each frame's
    /// action from the screen and the settings' values after its period,
    /// against golden/xy-ready-made.txt, recorded from the code before.
    /// MPFREE_GOLDEN_BLESS=1 writes it again.
    #[test]
    fn the_ten_ready_made_xy_layouts_send_what_they_sent_before() {
        use crate::play_page::PlayPage;
        use mpfree_fx::ids::perform as pf;
        use mpfree_fx::xyfx::ids as xy;
        enum S {
            Down(f32, f32),
            Up,
            Tick(f32),
        }
        let mut script = vec![S::Down(0.2, 0.7)];
        for k in 1..=10 {
            let t = k as f32 / 10.0;
            script.push(S::Down(0.2 + 0.6 * t, 0.7 - 0.4 * t));
            script.push(S::Tick(16.667));
        }
        script.extend([S::Down(0.95, 0.99), S::Down(1.3, -0.2), S::Up]);
        script.extend((0..40).map(|_| S::Tick(16.667)));
        script.extend([S::Down(0.5, 0.5), S::Up, S::Tick(5.0), S::Tick(100.0)]);
        let mut out = String::new();
        for pick in std::iter::once(None).chain((0..9).map(Some)) {
            let mut p = PlayPage::default();
            p.set_xy_effect(pick);
            let mut h = MixHost::new(RATE, 0.0, 0.0);
            h.play(PlayFx::XyEffect(pick));
            run(&mut h, 10, 0.0, 0.0);
            let read = |h: &MixHost| -> Vec<f32> {
                let c = h.mixer.chain(StripId::Master).unwrap();
                match pick {
                    Some(_) => {
                        let n = c.node(h.xy_slot().unwrap()).unwrap();
                        vec![n.param(xy::X), n.param(xy::Y), n.param(xy::ACTIVE)]
                    }
                    None => {
                        let n = c.node(PERFORM_SLOT).unwrap();
                        vec![n.param(pf::DJ_FILTER), n.param(pf::ECHO)]
                    }
                }
            };
            out.push_str(&format!("# {pick:?}\n"));
            let mut at = (0.0, 0.0);
            for (i, step) in script.iter().enumerate() {
                let fx = match *step {
                    S::Down(x, y) => {
                        at = (x, y);
                        Some(p.xy(x, y, true))
                    }
                    S::Up => Some(p.xy(at.0, at.1, false)),
                    S::Tick(ms) => p.tick_xy(ms),
                };
                if let Some(fx) = fx {
                    h.play_screen(fx);
                }
                run(&mut h, 1, 0.0, 0.0);
                out.push_str(&format!("{i} {fx:?} {:?}\n", read(&h)));
            }
        }
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("golden/xy-ready-made.txt");
        if std::env::var_os("MPFREE_GOLDEN_BLESS").is_some() {
            std::fs::write(&path, &out).unwrap();
        }
        let want = std::fs::read_to_string(&path).expect("golden/xy-ready-made.txt");
        let first = want.lines().zip(out.lines()).position(|(a, b)| a != b);
        assert!(
            want == out,
            "differs from line {:?}: want {:?}, got {:?}",
            first.map(|i| i + 1),
            first.and_then(|i| want.lines().nth(i)),
            first.and_then(|i| out.lines().nth(i))
        );
    }

    #[test]
    fn a_tap_on_the_xy_pad_quicker_than_a_period_plays_for_one() {
        let mut h = MixHost::new(RATE, 0.0, 0.0);
        let active = xy_fx_on(&mut h);
        // Down and up before a period ran: it opens for one period...
        h.play(xy(0.5, 0.5, true));
        h.play(xy(0.5, 0.5, false));
        run(&mut h, 1, 0.0, 0.0);
        assert_eq!(active(&h), Some(1.0), "open for a period");
        // ...and lets go after it.
        run(&mut h, 1, 0.0, 0.0);
        assert_eq!(active(&h), Some(0.0), "then closed");
        // A new touch before the late lift is done: the new touch holds.
        h.play(xy(0.5, 0.5, true));
        h.play(xy(0.5, 0.5, false));
        h.play(xy(0.6, 0.5, true));
        run(&mut h, 3, 0.0, 0.0);
        assert_eq!(active(&h), Some(1.0), "the new touch holds it");
        // A lift in a later period than the press is done at once.
        h.play(xy(0.6, 0.5, false));
        run(&mut h, 1, 0.0, 0.0);
        assert_eq!(active(&h), Some(0.0));
    }

    #[test]
    fn a_learned_controls_gesture_takes_the_xy_pad_from_the_finger_until_it_ends() {
        use mpfree_fx::xyfx::ids as xy_id;
        let mut h = MixHost::new(RATE, 0.0, 0.0);
        let active = xy_fx_on(&mut h);
        let x = |h: &MixHost| {
            let c = h.mixer.chain(StripId::Master).unwrap();
            c.node(h.xy_slot().unwrap()).unwrap().param(xy_id::X)
        };
        h.play_screen(xy(0.2, 0.5, true));
        run(&mut h, 1, 0.0, 0.0);
        assert_eq!(x(&h), 20.0);
        // A learned knob's gesture begins: the newest gesture plays.
        h.play(xy(0.7, 0.5, true));
        run(&mut h, 1, 0.0, 0.0);
        assert_eq!(x(&h), 70.0);
        // The finger moves on and lifts, and the pad glides on the
        // screen: none of it plays, and the effect stays open.
        h.play_screen(xy(0.3, 0.5, true));
        h.play_screen(xy(0.3, 0.5, false));
        h.play_screen(xy(0.4, 0.5, false));
        run(&mut h, 1, 0.0, 0.0);
        assert_eq!((x(&h), active(&h)), (70.0, Some(1.0)));
        h.play(xy(0.8, 0.5, true));
        run(&mut h, 1, 0.0, 0.0);
        assert_eq!(x(&h), 80.0);
        // The knob comes back to rest: it lets go.
        h.play(xy(0.5, 0.25, false));
        run(&mut h, 1, 0.0, 0.0);
        assert_eq!((x(&h), active(&h)), (50.0, Some(0.0)));
        // A new touch takes the pad back.
        h.play_screen(xy(0.1, 0.5, true));
        run(&mut h, 1, 0.0, 0.0);
        assert_eq!((x(&h), active(&h)), (10.0, Some(1.0)));
        assert!(
            h.describe_xy()
                .starts_with("XY pad: 1 gestures taken by the other controller's, 1 moves ignored"),
            "{}",
            h.describe_xy()
        );
    }

    #[test]
    fn the_screens_pad_state_lets_go_when_its_lift_never_came() {
        use crate::xy_slot::XyState;
        let mut h = MixHost::new(RATE, 0.0, 0.0);
        let active = xy_fx_on(&mut h);
        h.play_screen(xy(0.3, 0.6, true));
        run(&mut h, 2, 0.0, 0.0);
        assert_eq!(active(&h), Some(1.0));
        let up = XyState {
            gesture: 1,
            down: false,
            x: 0.3,
            y: 0.6,
        };
        assert!(
            !h.screen_xy(XyState { down: true, ..up }),
            "still down there: nothing to do"
        );
        // The lift action was lost; the pad's state says the finger is up.
        assert!(h.screen_xy(up));
        run(&mut h, 2, 0.0, 0.0);
        assert_eq!(active(&h), Some(0.0), "let go");
        assert_eq!(h.xy_found_up(), 1);
        assert!(!h.screen_xy(up), "only once");
        // A learned control holding the pad isn't the screen: the screen's
        // state leaves it alone.
        h.play(xy(0.3, 0.6, true));
        run(&mut h, 2, 0.0, 0.0);
        assert!(!h.screen_xy(up));
        run(&mut h, 2, 0.0, 0.0);
        assert_eq!(active(&h), Some(1.0));
    }

    /// A Perform pad's throw plays as the pad says (a bar, from the next
    /// beat, cut) and leaves the Perform FX pad's own settings as they
    /// were, for Play's card and the Perform FX page.
    #[test]
    fn a_perform_pads_throw_plays_its_way_and_puts_the_pads_settings_back() {
        let mut h = MixHost::new(RATE, 0.0, 0.0);
        run(&mut h, 10, 0.0, 0.0);
        let k = ThrowFx::Reverb.pad();
        let state = |h: &MixHost| h.perform_param(perf::slot(k) + 3);
        h.play(PlayFx::Throw {
            fx: ThrowFx::Reverb,
            down: true,
            how: Some(ThrowHow {
                starts: 1,
                cut: true,
                bar: true,
                on: StripId::Master,
            }),
        });
        run(&mut h, 2, 0.0, 0.0);
        assert_eq!(h.perform_param(perf::hold(k)), Some(1.0));
        assert_eq!(state(&h), Some(2.0), "on: it started at once");
        for id in [perf::starts(k), perf::let_go(k), perf::plays(k)] {
            assert_eq!(h.perform_param(id), Some(0.0), "id {id} put back");
        }
        // Let go at once: it plays its bar on.
        h.play(PlayFx::Throw {
            fx: ThrowFx::Reverb,
            down: false,
            how: Some(ThrowHow {
                starts: 1,
                cut: true,
                bar: true,
                on: StripId::Master,
            }),
        });
        run(&mut h, 2, 0.0, 0.0);
        assert_eq!(h.perform_param(perf::hold(k)), Some(0.0));
        assert_eq!(state(&h), Some(2.0), "still in its bar");
        assert_eq!(h.perform_param(perf::let_go(k)), Some(0.0));
        // Play's card plays the pad as it is: held, so let go, it stops.
        h.play(PlayFx::Throw {
            fx: ThrowFx::Delay,
            down: true,
            how: None,
        });
        run(&mut h, 2, 0.0, 0.0);
        h.play(PlayFx::Throw {
            fx: ThrowFx::Delay,
            down: false,
            how: None,
        });
        run(&mut h, 2, 0.0, 0.0);
        assert_eq!(
            h.perform_param(perf::slot(ThrowFx::Delay.pad()) + 3),
            Some(0.0)
        );
    }

    /// A Perform pad's throw on a track or a submix: picking its On puts a
    /// Perform FX at the end of that strip's chain (said), and the throw
    /// plays there, the master's at rest. A hit while it is still going in
    /// waits for it; a hit on a strip with none says so and changes
    /// nothing. When no throw plays on a strip any more, the Perform FX
    /// the set put there comes off; a project's set changes nothing; a
    /// full chain says why nothing went on.
    #[test]
    fn a_throws_knobs_are_its_pad_on_its_strips_perform_fx() {
        use mpfree_play::mpfree_param::Strip;
        let mut h = MixHost::new(RATE, 0.0, 0.0);
        run(&mut h, 10, 0.0, 0.0);
        let drums = StripId::Track(DRUMS);
        // The master's Perform FX, its default bank: the Delay throw's pad
        // is an Echo throw, 1/8 dotted, 55 % feedback.
        let k = h.throw_knobs(StripId::Master, ThrowFx::Delay);
        assert!(k.uid.is_some());
        assert_eq!((k.effect, k.length, k.amount), (12, 8.0, 55.0));
        assert_eq!(
            k.addr(perf::AMOUNT),
            Some(ParamAddr::new(
                Owner::Insert(Strip::Master, k.uid.unwrap()),
                perf::slot(ThrowFx::Delay.pad()) + perf::AMOUNT
            ))
        );
        // Drums has none until a throw's On picks it.
        let none = h.throw_knobs(drums, ThrowFx::Reverb);
        assert_eq!((none.uid, none.addr(perf::AMOUNT)), (None, None));
        h.perform_strips(&[drums], false);
        run(&mut h, 10, 0.0, 0.0);
        let _ = h.poll_built();
        run(&mut h, 2, 0.0, 0.0);
        let k = h.throw_knobs(drums, ThrowFx::Reverb);
        assert!(k.uid.is_some());
        assert_eq!((k.effect, k.amount), (14, 70.0));
    }

    #[test]
    fn picking_a_throws_strip_puts_a_perform_fx_there_and_it_plays_on_it() {
        let mut h = MixHost::new(RATE, 0.0, 0.0);
        // The inserts go in once each strip has ducked out (about 10 ms).
        run(&mut h, 10, 0.0, 0.0);
        let drums = StripId::Track(DRUMS);
        let sub = StripId::Bus(1);
        let k = ThrowFx::Delay.pad();
        let how = |on| ThrowHow {
            starts: 1,
            on,
            ..ThrowHow::default()
        };
        let throw = |h: &mut MixHost, on, down: bool| {
            h.play(PlayFx::Throw {
                fx: ThrowFx::Delay,
                down,
                how: Some(how(on)),
            })
        };
        let settle = |h: &mut MixHost| {
            run(h, 10, 0.0, 0.0);
            assert_eq!(h.poll_built(), None);
            run(h, 2, 0.0, 0.0);
        };
        // A hit on Drums before anything put a Perform FX there: said, and
        // nothing goes on.
        let before = h.mixer.chain(drums).map_or(0, |c| c.len());
        throw(&mut h, drums, true);
        assert_eq!(
            h.take_throw_said(),
            [
                "perform: Drums has no Perform FX for the throw: pick the pad's On again on the \
              Pads tab"
            ]
        );
        settle(&mut h);
        assert_eq!(h.mixer.chain(drums).map_or(0, |c| c.len()), before);
        throw(&mut h, drums, false);
        // On picks Drums: a Perform FX at the end of its chain.
        h.perform_strips(&[drums], false);
        assert_eq!(
            h.take_throw_said(),
            [
                "perform: Perform FX put on Drums for the Perform pads' throws (the Mix tab \
              shows it)"
            ]
        );
        settle(&mut h);
        assert_eq!(h.perform_slot(drums), Some(before));
        throw(&mut h, drums, true);
        run(&mut h, 2, 0.0, 0.0);
        assert_eq!(h.perform_param_on(drums, perf::hold(k)), Some(1.0));
        assert_eq!(
            h.perform_param(perf::hold(k)),
            Some(0.0),
            "not the master's"
        );
        throw(&mut h, drums, false);
        run(&mut h, 2, 0.0, 0.0);
        assert_eq!(h.perform_param_on(drums, perf::hold(k)), Some(0.0));
        // Sub 2 picked too, and hit while its Perform FX goes in: the hit
        // waits and plays once it is there. The same set again changes
        // nothing.
        h.perform_strips(&[drums, sub], false);
        throw(&mut h, sub, true);
        assert_eq!(h.take_throw_said().len(), 1, "Sub 2's put on");
        h.perform_strips(&[drums, sub], false);
        assert!(h.take_throw_said().is_empty());
        settle(&mut h);
        assert_eq!(h.perform_param_on(sub, perf::hold(k)), Some(1.0));
        throw(&mut h, sub, false);
        // No throw on Drums any more: its Perform FX comes off.
        h.perform_strips(&[sub], false);
        assert_eq!(
            h.take_throw_said(),
            ["perform: Perform FX taken off Drums (no Perform pad throws there now)"]
        );
        settle(&mut h);
        assert_eq!(h.perform_slot(drums), None);
        assert_eq!(h.mixer.chain(drums).map_or(0, |c| c.len()), before);
        // A project's set: its mixer brings its own, so nothing goes on or
        // comes off, and Sub 2's stays as the project's.
        let synth = StripId::Track(SYNTH);
        h.perform_strips(&[synth], true);
        settle(&mut h);
        assert_eq!(h.perform_slot(synth), None);
        assert!(h.perform_slot(sub).is_some());
        h.perform_strips(&[], false);
        settle(&mut h);
        assert!(
            h.perform_slot(sub).is_some(),
            "not this session's to take off"
        );
        assert!(h.take_throw_said().is_empty());
        // A full chain: said, and nothing put on.
        while h.mixer.chain(synth).map_or(SLOTS, |c| c.len()) < SLOTS {
            h.add_insert(synth, "mpfree.fx.compressor").unwrap();
            run(&mut h, 10, 0.0, 0.0);
        }
        h.perform_strips(&[synth], false);
        assert_eq!(
            h.take_throw_said(),
            ["perform: Synth has no free effect slot for the throw's Perform FX"]
        );
        settle(&mut h);
        assert_eq!(h.perform_slot(synth), None);
    }

    /// The strips a throw can play on: the master, the engines' tracks, a
    /// track's own channel only while in use (by its track's name), and
    /// the submixes. (The app leaves out the old Audio strip unless it
    /// holds an older project's settings: `desk::throw_strips`.)
    #[test]
    fn a_throw_plays_on_the_master_a_track_or_a_submix() {
        let h = MixHost::new(RATE, 0.0, 0.0);
        let names = ["Breaks".to_string(), "Keys".to_string()];
        let strips = throw_strips(&h.mixer, 0b10, &names, &|_| None);
        let named: Vec<&str> = strips.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(
            named[..6],
            ["Master", "Drums", "Synth", "Loop", "Audio (old)", "Keys"]
        );
        assert_eq!(strips[5].1, StripId::Track(TRACKS + 1));
        assert_eq!(strips[6], ("Sub 1".to_string(), StripId::Bus(0)));
        assert_eq!(strips.len(), 6 + 8, "eight submixes");
    }

    /// A macro pad holds its macro at its value while held, then lets go
    /// back to the value underneath.
    #[test]
    fn a_macro_pad_holds_its_macro_while_held() {
        let mut h = MixHost::new(RATE, 0.0, 0.0);
        run(&mut h, 2, 0.0, 0.0);
        let m = ParamAddr::macro_value(2);
        let before = h.modu.engine().value(m).unwrap_or(0.0);
        h.play(PlayFx::Macro {
            card: 2,
            value: Some(1.0),
        });
        run(&mut h, 2, 0.0, 0.0);
        assert_eq!(h.modu.engine().value(m), Some(1.0));
        h.play(PlayFx::Macro {
            card: 2,
            value: None,
        });
        run(&mut h, 2, 0.0, 0.0);
        assert_eq!(h.modu.engine().value(m), Some(before));
    }

    /// Jay after live test 3: several throws at once, a finger on each.
    #[test]
    fn two_throws_held_at_once_each_hold_their_pad() {
        let mut h = MixHost::new(RATE, 0.0, 0.0);
        let held = |h: &MixHost, fx: ThrowFx| {
            h.mixer
                .chain(StripId::Master)
                .and_then(|c| c.node(PERFORM_SLOT))
                .map(|n| n.param(perf::hold(fx.pad())))
        };
        run(&mut h, 10, 0.0, 0.0);
        for fx in [ThrowFx::Delay, ThrowFx::Reverb] {
            h.play(PlayFx::Throw {
                fx,
                down: true,
                how: None,
            });
        }
        run(&mut h, 2, 0.0, 0.0);
        assert_eq!(held(&h, ThrowFx::Delay), Some(1.0));
        assert_eq!(held(&h, ThrowFx::Reverb), Some(1.0));
        // The first finger lifts: the other throw keeps playing.
        h.play(PlayFx::Throw {
            fx: ThrowFx::Delay,
            down: false,
            how: None,
        });
        run(&mut h, 2, 0.0, 0.0);
        assert_eq!(held(&h, ThrowFx::Delay), Some(0.0));
        assert_eq!(held(&h, ThrowFx::Reverb), Some(1.0));
    }

    /// Runs periods (polling as the app does) until track 2's chain has
    /// `len` inserts, for at most 5 s.
    fn until_chain(h: &mut MixHost, len: usize) -> usize {
        let strip = StripId::Track(SYNTH);
        let give_up = Instant::now() + Duration::from_secs(5);
        while h.mixer.chain(strip).unwrap().len() != len && Instant::now() < give_up {
            assert_eq!(h.poll_built(), None);
            run(h, 1, 0.0, 0.0);
            // Built on the builder thread when there is one.
            std::thread::sleep(Duration::from_micros(200));
        }
        assert_eq!(h.poll_built(), None);
        h.mixer.chain(strip).unwrap().len()
    }

    /// Adds then removes a Drive on track 2 `times` times, checking the
    /// chain each time. Removes glide out over a few periods.
    fn add_and_remove(h: &mut MixHost, times: usize) {
        let strip = StripId::Track(SYNTH);
        let base = h.mixer.chain(strip).unwrap().len();
        for round in 0..times {
            h.add_insert(strip, "mpfree.fx.drive").unwrap();
            assert_eq!(until_chain(h, base + 1), base + 1, "add {round}");
            assert_eq!(h.inserts_building(), 0);
            h.mixer.edit(Edit::Remove { strip, at: base }).ok().unwrap();
            assert_eq!(until_chain(h, base), base, "remove {round}");
        }
    }

    /// The chain editor's Capture: the Grains effect's memory comes out a
    /// piece a period, whole, with its settings; one at a time; and an
    /// effect taken out on the way says so.
    #[test]
    fn an_effects_memory_is_captured_a_piece_a_period() {
        let mut h = MixHost::new(RATE, 0.0, 0.0);
        let strip = StripId::Pad(0);
        let base = h.mixer.chain(strip).unwrap().len();
        h.add_insert(strip, "mpfree.grains").unwrap();
        for _ in 0..20 {
            assert_eq!(h.poll_built(), None);
            run(&mut h, 1, 0.5, 0.0);
        }
        assert_eq!(h.mixer.chain(strip).unwrap().len(), base + 1);
        // Two seconds of the pad's tone into its memory.
        run(&mut h, 690, 0.5, 0.0);
        let held = Box::new(Held::new(2, RATE as usize * 8));
        h.capture(strip, base, held).unwrap();
        assert!(h.capturing.is_some());
        let busy = h.capture(strip, base, Box::new(Held::new(2, 10)));
        assert_eq!(busy.err().map(|(_, e)| e), Some(Captured::Busy));
        let mut periods = 0;
        let (held, end) = loop {
            run(&mut h, 1, 0.5, 0.0);
            periods += 1;
            if let Some(done) = h.take_captured() {
                break done;
            }
            assert!(periods < 100, "never done");
        };
        assert_eq!(end, Captured::Done);
        assert!(h.capturing.is_none());
        // Not frozen: all but the oldest quarter of what it heard, which
        // is under its 8 s, so everything from the start.
        let frames = held.frames();
        assert!(frames > RATE as usize * 2, "{frames} frames");
        assert_eq!(periods, frames.div_ceil(CAPTURE_STEP));
        assert!(held.data().iter().any(|&x| x.abs() > 0.1), "the tone");
        let size = held
            .values()
            .iter()
            .find(|(id, _)| *id == mpfree_fx::ids::grains::SIZE)
            .map(|v| v.1);
        assert!(size.is_some(), "the settings came too");
        // Taken out on the way: Gone, and the box still comes back.
        h.capture(strip, base, held).unwrap();
        h.mixer.edit(Edit::Remove { strip, at: base }).ok().unwrap();
        let mut end = None;
        for _ in 0..100 {
            run(&mut h, 1, 0.0, 0.0);
            if let Some((_, e)) = h.take_captured() {
                end = Some(e);
                break;
            }
        }
        assert_eq!(end, Some(Captured::Gone));
        let none = h.capture(strip, base, Box::new(Held::new(2, 10)));
        assert_eq!(
            none.err().map(|(_, e)| e),
            Some(Captured::Gone),
            "no insert there"
        );
    }

    #[test]
    fn chain_edits_keep_landing_after_many_removes() {
        // The app never took retired inserts back from the mixer, which
        // stops applying edits once 32 are waiting: the chain editor froze
        // after 32 removes. poll_built takes them back every period.
        let mut h = MixHost::new(RATE, 0.0, 0.0);
        add_and_remove(&mut h, 40);
    }

    #[test]
    fn a_plugins_presets_are_asked_for_once_and_why_not_comes_back() {
        use crate::control::PresetNews;
        let mut h = MixHost::new(RATE, 0.0, 0.0);
        // An engine has its presets built in: nothing is asked.
        h.ask_presets("mpfree.poly");
        assert!(h.poll_presets().is_none());
        // A plugin not in the folder: asked once, and why not comes back.
        h.ask_presets("clap:org.nope");
        h.ask_presets("clap:org.nope");
        match h.poll_presets() {
            Some(PresetNews::Listed { id, list: Err(e) }) => {
                assert_eq!(id, "clap:org.nope");
                assert!(!e.is_empty());
            }
            _ => panic!("expected the listing's error"),
        }
        assert!(h.poll_presets().is_none(), "asked once");
        assert!(h.plugin_presets("clap:org.nope").is_none());
        // Kept: not asked again.
        h.ask_presets("clap:org.nope");
        assert!(h.poll_presets().is_none());
        // A track without a plugin loads none.
        assert!(!h.load_plugin_preset(0, 0));
        assert!(h.poll_presets().is_none());
    }

    #[test]
    fn effects_built_off_the_audio_thread_land_in_their_chain() {
        let mut h = MixHost::new(RATE, 0.0, 0.0);
        h.build_off_thread().unwrap();
        add_and_remove(&mut h, 40);
        // An effect with no maker is said, and nothing lands.
        let strip = StripId::Track(SYNTH);
        let before = h.mixer.chain(strip).unwrap().len();
        h.add_insert(strip, "no.such.effect").unwrap();
        let give_up = Instant::now() + Duration::from_secs(5);
        let mut said = None;
        while said.is_none() && Instant::now() < give_up {
            said = h.poll_built();
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(said.unwrap().contains("no maker"), "{said:?}");
        assert_eq!(h.mixer.chain(strip).unwrap().len(), before);
        assert_eq!(h.inserts_building(), 0);
    }

    #[test]
    fn the_context_follows_the_transport() {
        let block = mpfree_transport::Block::default();
        let c = ctx(RATE, 128, 97.0, &block);
        assert_eq!(
            (c.rate, c.frame, c.bpm, c.playing),
            (RATE, 128, 97.0, false)
        );
    }

    /// A pan on track `t`'s strip.
    fn pan(t: usize) -> ParamAddr {
        ParamAddr::new(
            Owner::Strip(mpfree_play::mpfree_param::Strip::Track(t as u8)),
            mx::PAN,
        )
    }

    fn pan_now(h: &MixHost, t: usize) -> f32 {
        h.mixer.param(StripId::Track(t), mx::PAN)
    }

    fn pad(h: &mut MixHost, x: f32, y: f32, down: bool) {
        h.play_screen(PlayFx::Xy { x, y, down });
    }

    /// Periods for `ms` of sound.
    fn ms(ms: f32) -> usize {
        (RATE as f32 * ms / 1000.0 / PERIOD as f32).ceil() as usize + 1
    }

    /// A layout of yours (the XY controller plan, phase 2): Drums' pan set
    /// across, from -100 to 100, gliding back over 100 ms.
    fn yours(lift: mpfree_xy::Lift) -> mpfree_xy::Compiled {
        let mut l = mpfree_xy::Layout::new("Test");
        l.put(false, mpfree_xy::Target::linear(pan(DRUMS), -100.0, 100.0))
            .unwrap();
        l.lift = lift;
        l.compile().unwrap()
    }

    /// The setup the tests share: Drums' pan turned to 20 (its own value),
    /// and `layout` on the pad.
    fn playing(layout: mpfree_xy::Compiled) -> MixHost {
        let mut h = MixHost::new(RATE, 0.0, 0.0);
        run(&mut h, 10, 0.0, 0.0);
        h.set(StripId::Track(DRUMS), mx::PAN, 20.0);
        run(&mut h, 1, 0.0, 0.0);
        assert_eq!(pan_now(&h, DRUMS), 20.0);
        h.xy_yours(Some(layout));
        assert!(h.xy_play().playing());
        h
    }

    /// The pad's hand holds a Set setting, leaving its own value alone,
    /// and Go back glides it back to its own value as it is by then.
    #[test]
    fn a_layout_of_yours_holds_its_settings_and_goes_back_to_their_own_values() {
        let mut h = playing(yours(mpfree_xy::Lift::GoBack { snap_ms: 100.0 }));
        pad(&mut h, 1.0, 0.5, true);
        run(&mut h, 2, 0.0, 0.0);
        assert_eq!(pan_now(&h, DRUMS), 100.0, "the pad holds it");
        assert_eq!(
            h.modu.engine().base(pan(DRUMS)),
            Some(20.0),
            "its own value stays"
        );
        pad(&mut h, 0.75, 0.5, true);
        run(&mut h, 1, 0.0, 0.0);
        assert_eq!(pan_now(&h, DRUMS), 50.0);
        // The own value turned meanwhile: the pad lets go to it.
        h.set(StripId::Track(DRUMS), mx::PAN, -40.0);
        run(&mut h, 1, 0.0, 0.0);
        assert_eq!(pan_now(&h, DRUMS), 50.0, "the pad still holds it");
        pad(&mut h, 0.75, 0.5, false);
        run(&mut h, ms(50.0), 0.0, 0.0);
        let mid = pan_now(&h, DRUMS);
        assert!(mid < 50.0 && mid > -40.0, "gliding back: {mid}");
        run(&mut h, ms(100.0), 0.0, 0.0);
        assert_eq!(pan_now(&h, DRUMS), -40.0, "back to its own value");
        assert!(!h.modu.engine().is_touched(pan(DRUMS)));
        assert_eq!(h.take_xy_kept().count(), 0, "Go back keeps nothing");
    }

    /// Stay keeps the pad's values as the settings' own, one UNDO step;
    /// while recording it keeps nothing and lets go.
    #[test]
    fn stay_keeps_the_pads_values_as_their_own_except_while_recording() {
        let mut h = playing(yours(mpfree_xy::Lift::Stay));
        pad(&mut h, 0.25, 0.5, true);
        run(&mut h, 1, 0.0, 0.0);
        pad(&mut h, 0.25, 0.5, false);
        run(&mut h, 2, 0.0, 0.0);
        assert_eq!(pan_now(&h, DRUMS), -50.0, "it stays");
        assert_eq!(h.modu.engine().base(pan(DRUMS)), Some(-50.0), "its own now");
        assert!(!h.modu.engine().is_touched(pan(DRUMS)));
        let kept: Vec<_> = h.take_xy_kept().collect();
        assert_eq!(kept, [(pan(DRUMS), 20.0, -50.0)], "for one UNDO step");
        // Recording: the move would go into the recording, so the own value
        // stays as it was.
        h.set_recording(true);
        pad(&mut h, 1.0, 0.5, true);
        run(&mut h, 1, 0.0, 0.0);
        assert_eq!(pan_now(&h, DRUMS), 100.0);
        pad(&mut h, 1.0, 0.5, false);
        run(&mut h, 2, 0.0, 0.0);
        assert_eq!(pan_now(&h, DRUMS), -50.0);
        assert_eq!(h.take_xy_kept().count(), 0);
    }

    /// Add pushes a setting from wherever it is, by a row from the pad's
    /// own host source, and its push always glides away on a lift, even
    /// with Stay: Stay leaves Add settings alone.
    #[test]
    fn an_add_setting_pushes_from_where_it_is_and_stay_leaves_it_alone() {
        let mut l = mpfree_xy::Layout::new("Push");
        l.put(true, mpfree_xy::Target::add(pan(LOOP), 0.0, 0.25))
            .unwrap();
        l.put(false, mpfree_xy::Target::linear(pan(DRUMS), -100.0, 100.0))
            .unwrap();
        l.lift = mpfree_xy::Lift::Stay;
        let mut h = playing(l.compile().unwrap());
        h.set(StripId::Track(LOOP), mx::PAN, 10.0);
        run(&mut h, 1, 0.0, 0.0);
        let n = crate::xy_play::source(mpfree_xy::TARGETS);
        assert!(
            h.modu
                .engine()
                .rows()
                .any(|r| r.source == mpfree_mod::Source::Host(n) && r.target == pan(LOOP)),
            "a row from its own source"
        );
        pad(&mut h, 0.5, 1.0, true);
        run(&mut h, 2, 0.0, 0.0);
        // A quarter of the travel (-100 to 100) is 50.
        assert!(
            (pan_now(&h, LOOP) - 60.0).abs() < 0.01,
            "{}",
            pan_now(&h, LOOP)
        );
        assert_eq!(h.modu.engine().base(pan(LOOP)), Some(10.0));
        pad(&mut h, 0.5, 1.0, false);
        run(&mut h, ms(crate::xy_play::PUSH_AWAY_MS + 20.0), 0.0, 0.0);
        assert_eq!(pan_now(&h, LOOP), 10.0, "the push glided away");
        assert_eq!(h.modu.engine().base(pan(LOOP)), Some(10.0), "untouched");
        let kept: Vec<_> = h.take_xy_kept().collect();
        assert_eq!(
            kept,
            [(pan(DRUMS), 20.0, 0.0)],
            "only the Set setting stays"
        );
        // Another layout takes the pad: the row goes with the old one.
        h.xy_yours(Some(yours(mpfree_xy::Lift::Stay)));
        assert!(!h
            .modu
            .engine()
            .rows()
            .any(|r| r.source == mpfree_mod::Source::Host(n)));
        assert!(
            !h.modu.save().contains("host"),
            "the pad's rows are never saved"
        );
    }

    /// Hold keeps the pad in charge after a lift until it is turned off;
    /// switching layouts lets go of everything even with Hold on.
    #[test]
    fn hold_keeps_the_settings_until_it_is_off_but_a_switch_lets_go() {
        let mut h = playing(yours(mpfree_xy::Lift::GoBack { snap_ms: 0.0 }));
        h.xy_hold(true);
        pad(&mut h, 0.0, 0.5, true);
        pad(&mut h, 0.0, 0.5, false);
        run(&mut h, 4, 0.0, 0.0);
        assert_eq!(pan_now(&h, DRUMS), -100.0, "held after the lift");
        h.xy_hold(false);
        run(&mut h, 1, 0.0, 0.0);
        assert_eq!(pan_now(&h, DRUMS), 20.0, "Hold off: it goes back");
        h.xy_hold(true);
        pad(&mut h, 1.0, 0.5, true);
        pad(&mut h, 1.0, 0.5, false);
        run(&mut h, 1, 0.0, 0.0);
        assert_eq!(pan_now(&h, DRUMS), 100.0);
        // Picking a ready-made layout lets go of it all, Hold or not.
        h.play(PlayFx::XyEffect(None));
        run(&mut h, 1, 0.0, 0.0);
        assert!(!h.xy_play().playing());
        assert_eq!(pan_now(&h, DRUMS), 20.0);
        assert!(!h.modu.engine().is_touched(pan(DRUMS)));
    }

    /// A learned control has no lift: on a layout of yours its gesture
    /// ends half a second after its last move.
    #[test]
    fn a_learned_controls_gesture_on_a_layout_of_yours_ends_after_it_is_still() {
        let mut h = playing(yours(mpfree_xy::Lift::GoBack { snap_ms: 0.0 }));
        h.play(PlayFx::Xy {
            x: 1.0,
            y: 0.5,
            down: true,
        });
        run(&mut h, ms(400.0), 0.0, 0.0);
        assert_eq!(pan_now(&h, DRUMS), 100.0, "still held");
        // A move starts the half second again.
        h.play(PlayFx::Xy {
            x: 0.75,
            y: 0.5,
            down: true,
        });
        run(&mut h, ms(400.0), 0.0, 0.0);
        assert_eq!(pan_now(&h, DRUMS), 50.0);
        run(&mut h, ms(150.0), 0.0, 0.0);
        assert_eq!(pan_now(&h, DRUMS), 20.0, "let go half a second after");
        assert!(!h.xy_play().down());
    }

    /// While recording, a knob turned on a setting the pad holds takes it
    /// (the newest gesture wins); the pad's moves are ignored and its lift
    /// doesn't let go of the knob's hold.
    #[test]
    fn a_knob_takes_a_setting_from_the_pad_and_the_pads_lift_leaves_it() {
        let mut h = playing(yours(mpfree_xy::Lift::GoBack { snap_ms: 0.0 }));
        h.set_recording(true);
        pad(&mut h, 1.0, 0.5, true);
        run(&mut h, 1, 0.0, 0.0);
        assert!(h
            .modu
            .hand(crate::hand::Hand::Midi, pan(DRUMS), -60.0, true));
        run(&mut h, 1, 0.0, 0.0);
        assert_eq!(pan_now(&h, DRUMS), -60.0, "the knob took it");
        pad(&mut h, 0.9, 0.5, true);
        run(&mut h, 1, 0.0, 0.0);
        assert_eq!(pan_now(&h, DRUMS), -60.0, "the pad's moves are ignored");
        pad(&mut h, 0.9, 0.5, false);
        run(&mut h, 1, 0.0, 0.0);
        assert_eq!(pan_now(&h, DRUMS), -60.0, "the pad's lift leaves it");
        assert!(h.modu.engine().is_touched(pan(DRUMS)));
        // The knob lets go half a second after its last move.
        run(&mut h, ms(550.0), 0.0, 0.0);
        assert_eq!(pan_now(&h, DRUMS), 20.0);
    }
}
