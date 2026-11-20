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
//! - What the looper records from the pads ([`MixHost::pads_wet`]): the
//!   pads and the synth as they are heard, with their tracks' effects and
//!   faders and what the send returns add: the master's input, without the
//!   Loop track, the Audio track and the line input. It comes a period late
//!   (the looper runs before the mixer), and the looper makes that up. When
//!   one of those three can't be taken out (it goes to a submix, or a
//!   plugin's latency puts it out of step), the looper records the drums
//!   and the synth as the engines make them, dry, and says why.
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

/// The Drums strip's name while track 1's own instrument plays there,
/// like every other track's own channel ("Track 2"): the strip is named
/// for what track 1 plays. Kit 1's pads, from whichever track plays them,
/// still sound there too (the manual's mixer page says so); the strip's
/// name has no room for both words.
pub const TRACK_1: &str = "Track 1";
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
/// The capture point the sampler resamples from (the recorder's Source):
/// the last of the mixer's [`CAPTURES`], so the others stay free for
/// print, stems and the looper.
pub const SAMPLER_CAPTURE: usize = CAPTURES - 1;
/// The looper's capture points, [`LOOP_CAPTURE`] and the three after it,
/// below Freeze and Print's: the master's input, then the strips taken out
/// of it ([`MixHost::pads_wet`]).
pub const LOOP_CAPTURE: usize = SAMPLER_CAPTURE - 5;
/// The strips the looper's pads leave out: the loops, the audio tracks and
/// the line input, in the order of their capture points after
/// [`LOOP_CAPTURE`].
const LEFT_OUT: [StripId; 3] = [StripId::Track(LOOP), StripId::Track(AUDIO), LINE];

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
    /// Pads muted (a bit each), as the speakers show them.
    pub pad_mutes: u16,
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
    /// A speaker: mute or unmute a pad.
    PadMute { pad: usize, mute: bool },
    /// Solo a pad of the kit, or let go of its solo (TRACK MUTE held over
    /// the kit's pad mutes): while any pad is soloed the others are
    /// silent.
    PadSolo { pad: usize, solo: bool },
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
pub fn throw_strips(mixer: &Mixer, in_use: u128, tracks: &[String]) -> Vec<(String, StripId)> {
    let mut out = vec![(name(StripId::Master), StripId::Master)];
    let mut subs = Vec::new();
    for s in mixer.strips() {
        match s {
            StripId::Track(t) if t < TRACKS => out.push((name(s), s)),
            StripId::Track(t) if t - TRACKS < 128 && in_use & (1 << (t - TRACKS)) != 0 => {
                let named = tracks
                    .get(t - TRACKS)
                    .filter(|n| !n.trim().is_empty())
                    .cloned()
                    .unwrap_or_else(|| name(s));
                out.push((named, s));
            }
            StripId::Bus(_) => subs.push((name(s), s)),
            _ => {}
        }
    }
    out.extend(subs);
    out
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
        StripId::Track(AUDIO) => "Audio".into(),
        StripId::Track(TRACKS) => OLD_TRACK_1.into(),
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
    /// Pads muted by their speakers or the panel, and by Drop drums.
    pad_mutes: u16,
    dropped: u16,
    /// Pads soloed (a bit each): while any is, the others are silent.
    pad_solos: u16,
    /// What the XY pad plays ([`PlayFx::XyEffect`]).
    xy_effect: Option<usize>,
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
            // instrument (unused ones cost a few flag checks).
            tracks: TRACKS + instruments::TRACKS,
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
            pad_solos: 0,
            mutes_sent: 0,
            loop_point: master_in,
            xy_effect: None,
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
                self.pad_solos = load.pad_solos;
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
            PlayFx::Xy { x, y, down } => match self.xy_effect {
                Some(_) => {
                    use mpfree_fx::xyfx::ids as xy;
                    // Still being built: the next move plays it.
                    let Some(slot) = self.xy_slot() else {
                        return;
                    };
                    self.insert(StripId::Master, slot, xy::X, x.clamp(0.0, 1.0) * 100.0);
                    self.insert(StripId::Master, slot, xy::Y, y.clamp(0.0, 1.0) * 100.0);
                    let open = if down { 1.0 } else { 0.0 };
                    self.insert(StripId::Master, slot, xy::ACTIVE, open);
                }
                None => {
                    let pad = mpfree_fx::XyPad::perform();
                    for (axis, pos) in [(pad.x, x), (pad.y, y)] {
                        self.insert(StripId::Master, PERFORM_SLOT, axis.param, axis.value(pos));
                    }
                }
            },
            PlayFx::XyEffect(pick) => {
                use mpfree_fx::xyfx::ids as xy;
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
            PlayFx::PadSolo { pad, solo } if pad < PADS => {
                if solo {
                    self.pad_solos |= 1 << pad;
                } else {
                    self.pad_solos &= !(1 << pad);
                }
                self.apply_mutes();
            }
            PlayFx::PadMute { .. } | PlayFx::PadSolo { .. } => {}
            PlayFx::Drop(pads) => {
                self.dropped = pads;
                self.apply_mutes();
            }
        }
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
        match self.xy_effect {
            Some(_) => {
                use mpfree_fx::xyfx::ids as xy;
                let node = self
                    .xy_slot()
                    .and_then(|slot| self.mixer.chain(StripId::Master)?.node(slot));
                match node {
                    Some(n) => (
                        (n.param(xy::X) / 100.0).clamp(0.0, 1.0),
                        (n.param(xy::Y) / 100.0).clamp(0.0, 1.0),
                    ),
                    None => (0.5, 0.25),
                }
            }
            None => {
                let pad = mpfree_fx::XyPad::perform();
                let at = |a: mpfree_fx::XyAxis| {
                    self.mixer
                        .chain(StripId::Master)
                        .and_then(|c| c.node(PERFORM_SLOT))
                        .map_or(a.rest, |n| a.position(n.param(a.param)))
                };
                (at(pad.x), at(pad.y))
            }
        }
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
        let silent = self.pad_mutes | self.dropped | self.solo_silent();
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

    /// The pads muted by their speakers or SHIFT + TRACK MUTE (a bit
    /// each; Drop drums' are not counted).
    pub fn pad_mutes(&self) -> u16 {
        self.pad_mutes
    }

    /// The pads soloed (a bit each).
    pub fn pad_solos(&self) -> u16 {
        self.pad_solos
    }

    /// The pads a solo silences: every pad but the soloed ones, while any
    /// is (a bit each).
    pub fn solo_silent(&self) -> u16 {
        if self.pad_solos == 0 {
            0
        } else {
            !self.pad_solos
        }
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
            pad_mutes: self.pad_mutes,
        }
    }

    /// What the looper records from the pads, wet, into `left` and `right`:
    /// the master's input as the last period left it, without the Loop
    /// track, the Audio track and the line input. That is every track you
    /// play after its effects and fader, with the echoes and reverbs the
    /// send returns add (the master's own effects and fader come after it,
    /// as they do for the loop when it plays). Returns its latency in frames
    /// after the period it is late; or why it can't be had, when one of the
    /// three, sounding, goes to a submix or is out of step with the master's
    /// input (a plugin's latency). Cheap: three subtractions a frame.
    pub fn pads_wet(
        &self,
        left: &mut [f32; PERIOD],
        right: &mut [f32; PERIOD],
    ) -> Result<u32, &'static str> {
        self.mix_less(LEFT_OUT.len(), left, right)
    }

    /// [`LOOP_CAPTURE`] as the last period left it, without the first
    /// `n` of [`LEFT_OUT`] ([`MixHost::pads_wet`]).
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
        for (i, s) in LEFT_OUT.into_iter().enumerate().take(n) {
            let slot = LOOP_CAPTURE + 1 + i;
            let Some((l, r)) = m.capture(slot).filter(|(l, _)| l.len() == PERIOD) else {
                continue;
            };
            if l.iter().chain(r).all(|&x| x == 0.0) {
                continue;
            }
            if m.destination(s) != Some(StripId::Master) {
                return Err(match i {
                    0 => "the Loop track goes to a submix",
                    1 => "the Audio track goes to a submix",
                    _ => "the line input goes to a submix",
                });
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

    /// What the looper records into `left` and `right` this period, and its
    /// latency, as [`MixHost::pads_wet`] says them: `from` None is the
    /// pads ([`MixHost::pads_wet`]); the master is all its input but the
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

    /// A strip's name on the panel: a lent pad channel by its pad and kit.
    pub fn strip_name(&self, s: StripId) -> String {
        strip_name(&self.pad_channels, s)
    }

    /// The project's mixer part (`project::mixer_text_lent`), once the
    /// lent pad channels that are plain again have been given back.
    pub fn mixer_text(&mut self) -> String {
        self.give_back_plain();
        let lent: Vec<(StripId, usize, usize)> = self.pad_channels.lent().collect();
        let track1 = StripId::Track(self.track_channel(0));
        crate::project::mixer_text_lent(&self.mixer, self.pad_mutes, self.pad_solos, &lent, track1)
    }

    /// The tracks whose own channels are in use, a bit each: playing their
    /// own instrument (or ringing out an old one), or a kit
    /// ([`MixHost::pads_homed`]).
    pub fn channels_in_use(&self) -> u128 {
        self.instruments.in_use() | self.kit_tracks
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
        audio: Stereo,
        line: Stereo,
        left: &mut [f32; PERIOD],
        right: &mut [f32; PERIOD],
    ) -> (f32, bool) {
        for (t, (l, r)) in [(SYNTH, synth), (LOOP, looped), (AUDIO, audio)] {
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
                // The speakers on the Play page follow.
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
        let used = self.own_channels_in_use();
        (0..self.mixer.tracks())
            .filter(|&t| t < TRACKS || used & (1 << (t - TRACKS)) != 0)
            .map(StripId::Track)
            .collect()
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
        // kit plays there); the others aren't read. Track 1's is Drums,
        // named "Track 1" while its own instrument plays there.
        let used = self.own_channels_in_use();
        let first_own = self.instruments.channel(0) == DRUMS && self.instruments.holds(0);
        let ch = &self.pad_channels;
        // Lent pad channels show while lent.
        let shown = |s: StripId| match s {
            StripId::Track(t) if t >= TRACKS => used & (1 << (t - TRACKS)) != 0,
            StripId::Pad(p) if p >= PADS => ch.holder(s).is_some(),
            _ => true,
        };
        let named = |s: StripId| match s {
            StripId::Track(DRUMS) if first_own => TRACK_1.to_string(),
            s => strip_name(ch, s),
        };
        let mut st = MixState::from_mixer_where(&mut self.mixer, view, &named, &shown);
        st.counts[0] = TRACKS + used.count_ones() as usize;
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
            let (pk, c) = h.period(&ctx, (&z, &z), (&z, &z), (&z, &z), (&i, &i), &mut l, &mut r);
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

    /// A pad soloed (TRACK MUTE held over the kit's pad mutes): the other
    /// pads go silent through their strips' mutes, more solos add pads, and
    /// letting go of the last solo gives back the pads' own mutes. The
    /// speakers and UNDO keep reading each pad's own mute.
    #[test]
    fn a_pad_solo_silences_the_kits_other_pads() {
        let mut h = MixHost::new(RATE, -6.0, -6.0);
        let muted = |h: &mut MixHost| -> u16 {
            h.run_silent_period();
            (0..PADS)
                .filter(|&p| h.mixer.param(StripId::Pad(p), mx::MUTE) >= 0.5)
                .fold(0, |m, p| m | 1 << p)
        };
        h.play(PlayFx::PadMute { pad: 5, mute: true });
        h.play(PlayFx::PadSolo { pad: 0, solo: true });
        assert_eq!((h.pad_solos(), muted(&mut h)), (1, !1));
        h.play(PlayFx::PadSolo { pad: 2, solo: true });
        assert_eq!(muted(&mut h), !0b101);
        assert_eq!(h.value(StripId::Pad(1), mx::MUTE), 0.0, "its own mute");
        assert_eq!(h.play_levels().pad_mutes, 1 << 5, "the speakers");
        h.play(PlayFx::PadSolo {
            pad: 0,
            solo: false,
        });
        h.play(PlayFx::PadSolo {
            pad: 2,
            solo: false,
        });
        assert_eq!((h.pad_solos(), muted(&mut h)), (0, 1 << 5));
        h.play(PlayFx::PadSolo {
            pad: 16,
            solo: true,
        });
        assert_eq!(h.pad_solos(), 0, "no pad 17");
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
            h.period(
                &ctx,
                (&z, &z),
                (&z, &z),
                (&z, &z),
                (&z, &z),
                &mut ol,
                &mut or,
            );
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
                h.period(
                    &ctx,
                    (&s, &s),
                    (&lp, &lp),
                    (&z, &z),
                    (&z, &z),
                    &mut l,
                    &mut r,
                );
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
                h.period(
                    &ctx,
                    (&s, &s),
                    (&lp, &lp),
                    (&z, &z),
                    (&z, &z),
                    &mut l,
                    &mut r,
                );
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
            h.period(&ctx, (&z, &z), (&z, &z), (&z, &z), (&z, &z), &mut l, &mut r);
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
            ["Drums", "Synth", "Loop", "Audio"]
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
    /// the submixes.
    #[test]
    fn a_throw_plays_on_the_master_a_track_or_a_submix() {
        let h = MixHost::new(RATE, 0.0, 0.0);
        let names = ["Breaks".to_string(), "Keys".to_string()];
        let strips = throw_strips(&h.mixer, 0b10, &names);
        let named: Vec<&str> = strips.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(
            named[..6],
            ["Master", "Drums", "Synth", "Loop", "Audio", "Keys"]
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
}
