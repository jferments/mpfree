//! One way to name any parameter in MPFree.
//!
//! A [`ParamAddr`] says *whose* parameter it is (an [`Owner`]) and *which*
//! one (`param`, the owner's stable parameter id: a node's
//! `mpfree_node::ParamInfo::id`, a mixer strip's `mpfree_fx::ids::mixer`
//! id, a mod source's id in `mpfree-mod`). Parameter locks, macros, the mod
//! matrix, morph scenes and Q-Link bindings all name their targets this
//! way, so one target can be locked on a step, moved by a macro and swept
//! by an LFO at once, and a project file names it the same way everywhere.
//!
//! The crate has no dependencies and every type is `Copy` and 8 bytes or
//! less, so the sequencer, the effects, the synths and the screens can all
//! use it without pulling each other in, and it is free to pass on the
//! audio thread.
//!
//! # Track-relative addresses
//!
//! A clip can move to another track, so what it stores is a [`RelAddr`]:
//! the address without the track ([`OnTrack`]). The player fills in the
//! playing track ([`RelAddr::on`]) when it compiles the clip.
//!
//! # Text form
//!
//! For project files, logs and tests, every address has a short text form
//! that [`Display`] writes and [`FromStr`] reads back. Indexes count from
//! 0, as in the code; screens show their own names.
//!
//! | Address | Text |
//! |---|---|
//! | track 3's instrument, parameter 12 | `t3.inst.12` |
//! | track 3, sampler pad 5, parameter 2 | `t3.pad5.2` |
//! | kit 9's pad 5 (the kit's instrument number), parameter 2, whichever track plays it (MIDI Learn) | `kit9.pad5.2` |
//! | kit 9's pad 5, its third layer's grain setting 12 ([`pad::layer`]: 256 + 2 × 64 + 12) | `kit9.pad5.396` |
//! | instrument 9's parameter 12, whichever track plays it (MIDI Learn) | `i9.12` |
//! | mixer track 7's strip, volume (0): sequencer track 3's own ([`track_strip`]; the Drums strip, `t0`, is track 0's) | `t7.mix.0` |
//! | bus 2, the insert with slot id 7, parameter 4 | `bus2.fx7.4` |
//! | input 0 / pad channel 9 / return 1 / master strip | `in0.mix.0` / `ps9.mix.0` / `ret1.mix.0` / `master.mix.0` |
//! | track 3's arpeggiator / sequencer settings | `t3.arp.1` / `t3.seq.0` |
//! | track 3's MIDI effect with id 2, setting 4 | `t3.nfx2.4` |
//! | macro 2's value | `macro2.0` |
//! | mod source 4's parameter 1 (an LFO's rate) | `mod4.1` |
//! | morph 0's position | `morph0.0` |
//! | the transport's tempo (MIDI Learn) | `transport.7` |
//! | launch scene 3 (MIDI Learn) | `clips.258` |
//! | track 3's CC 74 / pitch bend out (its outside gear) | `t3.midi.74` / `t3.midi.128` |
//! | Play's Swing / the XY pad across (MIDI Learn) | `play.0` / `play.2` |
//!
//! A [`RelAddr`] leaves the track out: `inst.12`, `pad5.2`, `mix.0`,
//! `fx7.4`, `arp.1`, `seq.0`, `nfx2.4`, `midi.74`. Its `mix` and `fx` go
//! to the playing track's strip, [`track_strip`]; a strip address names
//! the mixer's own number for it.
#![forbid(unsafe_code)]

use std::fmt;
use std::str::FromStr;

/// A mixer strip. Mirrors `mpfree_fx::mixer::StripId` (which uses
/// `usize`); this crate has no dependency on the mixer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Strip {
    Input(u8),
    /// A pad channel of the mixer's pad pool (not a sampler pad's sound;
    /// that is [`Owner::Pad`]).
    Pad(u8),
    /// A mixer track: the engines' first ([`ENGINE_STRIPS`]: Drums, Synth,
    /// Loop, Audio), then each sequencer track's own ([`track_strip`]).
    Track(u8),
    Bus(u8),
    Return(u8),
    Master,
}

/// The mixer tracks the engines have before the sequencer tracks' own:
/// Drums, Synth, Loop and Audio (the app's `mixing::TRACKS`).
pub const ENGINE_STRIPS: u8 = 4;

/// Sequencer track `t`'s mixer strip (0 is track 1), where its own
/// instrument plays and what "this track's strip" in a clip names
/// (sampler R3 part 4b): track 1's is the Drums strip, `Track(0)`, so a
/// track has one strip whatever it plays; every other track's is its own,
/// `Track(ENGINE_STRIPS + t)`. `Track(ENGINE_STRIPS)` is no track's (it
/// was track 1's before; a project saved with settings on it keeps
/// playing track 1 there, the app's choice). Tracks go up to 128, so
/// this never runs past 255.
pub const fn track_strip(t: u8) -> Strip {
    if t == 0 {
        Strip::Track(0)
    } else {
        Strip::Track(ENGINE_STRIPS.saturating_add(t))
    }
}

/// The sequencer track whose strip `s` is ([`track_strip`]'s inverse):
/// the Drums strip is track 1's (0), `Track(ENGINE_STRIPS + t)` for `t` of
/// 1 and up is track `t`'s; Synth, Loop, Audio, `Track(ENGINE_STRIPS)`
/// and every other strip are no track's.
pub const fn strip_track(s: Strip) -> Option<u8> {
    match s {
        Strip::Track(0) => Some(0),
        Strip::Track(i) if i > ENGINE_STRIPS => Some(i - ENGINE_STRIPS),
        _ => None,
    }
}

/// Whose parameter it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Owner {
    /// A track's instrument node (Poly, FM, ...).
    Inst(u8),
    /// A sampler pad's sound on a drum track: `(track, pad)`. A value set
    /// here applies to the voice that starts on that frame. Its parameters
    /// are the pad's own settings and its layers' grain settings ([`pad`]).
    Pad(u8, u8),
    /// A kit's pad by the kit's instrument number in its project's list
    /// and the pad, whichever track plays the kit (sampler step 16): what
    /// MIDI Learn names a pad's setting by, so a learned knob stays on its
    /// kit when a track picks another. Its parameters are [`Owner::Pad`]'s.
    KitPad(u16, u8),
    /// An instrument of the project's list by its number, whichever track
    /// plays it (sampler step 16): what MIDI Learn names a synth's knob by,
    /// so it stays on that synth when a track picks another. Its
    /// parameters are [`Owner::Inst`]'s.
    Sound(u16),
    /// A mixer strip's own controls (volume, pan, sends ...).
    Strip(Strip),
    /// An insert on a strip, named by its slot id: a number the chain
    /// gives the insert when it is made and keeps across reordering,
    /// replacing and saving, so a lock or a macro follows the effect, not
    /// the position.
    Insert(Strip, u16),
    /// A track's arpeggiator settings: its rate, gate, swing and octaves
    /// for MIDI Learn, ids in [`arp`].
    Arp(u8),
    /// A setting of one of a track's MIDI effects (its Notes stage),
    /// named by the effect's id on the track (`mpfree_notefx::TrackFx::ids`),
    /// which it keeps when it moves, so a binding follows the effect as an
    /// insert's slot id does. The parameter is the setting's place in the
    /// effect's list (`mpfree_notefx::Fx::settings`).
    NoteFx(u8, u16),
    /// A track's sequencer settings (swing, length ...).
    Seq(u8),
    /// A macro knob. Parameter 0 is its value, 0 to 1.
    Macro(u8),
    /// A modulation source (an LFO, an envelope follower). Its parameter
    /// ids are defined by `mpfree-mod`.
    Mod(u8),
    /// A morph between two scenes. Parameter 0 is its position, 0 to 1.
    Morph(u8),
    /// The transport and the tempo, for MIDI Learn's remote control: a
    /// learned button presses PLAY, STOP and the rest, and a learned knob
    /// sets the tempo. Its parameter ids are in [`transport`]. Locks,
    /// macros and the mod matrix never target it.
    Transport,
    /// The Clips grid, for MIDI Learn's remote control (sequencer S5,
    /// "being driven"): a learned button launches a clip or a scene, stops
    /// a track or all of them, goes back to the song or arms a track, as a
    /// tap on the grid does; a learned key or fader mutes or solos a
    /// track. Its parameter ids are in [`clips`]. Locks, macros and the
    /// mod matrix never target it.
    Clips,
    /// What a track sends its outside gear on MIDI out besides its notes
    /// (sequencer S6, the CC, pitch bend and pressure lanes): a
    /// controller, the pitch bend or the channel pressure, on the track's
    /// own port and channel. Its parameter ids are in [`midi`]. A step's
    /// lock, a lane, a macro, the morph and the mod matrix move it as they
    /// move a mixer control; a track that plays on the pads sends nothing.
    Midi(u8),
    /// The looper, for MIDI Learn's remote control (the looper redesign's
    /// footswitch, step 5): a learned footswitch, button or key plays a
    /// loop's key as its pad does, mutes a loop, picks the next one, or
    /// presses Undo, Redo, Capture or Clear. Its parameter ids are in
    /// [`looper`]. Locks, macros and the mod matrix never target it.
    Looper,
    /// The project's Perform pads, for MIDI Learn's remote control (the
    /// Play, Pads and Sequencer redesign's board 7, Learn a key): a
    /// learned key, pad or footswitch plays one Perform pad as its pad
    /// does, whatever the MPC's pads show. Its parameter ids are in
    /// [`perform`]. Locks, macros and the mod matrix never target it.
    Perform,
    /// The Play page's own controls, for MIDI Learn's remote control (the
    /// Learn section's Play card, Jay's pick on 18 Nov 2026): a learned
    /// knob turns the Swing or the Filter as Play's Q-Links do, or
    /// moves the XY pad across or up as a finger does. Its parameter ids
    /// are in [`play`]. Locks, macros and the mod matrix never target it.
    Play,
}

/// [`Owner::Midi`]'s parameters: a track's controllers, pitch bend and
/// channel pressure out. A controller is its own number (CC 74 is 74), so
/// a project names it as the gear's manual does.
pub mod midi {
    /// Controllers 0 to 119 can be sent. 120 to 127 are the channel mode
    /// messages (All sound off, Reset all controllers, Local control, All
    /// notes off, omni and mono/poly): they change how the gear listens,
    /// not its sound, so no lock or lane ever sends them.
    pub const CONTROLLERS: u16 = 120;
    /// The pitch bend, -8192 to 8191, 0 in the middle.
    pub const BEND: u16 = 128;
    /// The channel pressure (aftertouch), 0 to 127.
    pub const PRESSURE: u16 = 129;
    /// One past the last id, for a table of every control of a track.
    pub const COUNT: u16 = 130;

    /// What an [`Owner::Midi`](super::Owner::Midi) parameter sends.
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub enum Midi {
        /// Control change `n` (0-119).
        Cc(u8),
        Bend,
        Pressure,
    }

    /// What parameter `id` sends, if it is one.
    pub const fn of(id: u16) -> Option<Midi> {
        match id {
            BEND => Some(Midi::Bend),
            PRESSURE => Some(Midi::Pressure),
            n if n < CONTROLLERS => Some(Midi::Cc(n as u8)),
            _ => None,
        }
    }

    /// Its lowest and highest values: -8192 to 8191 for the bend, else 0
    /// to 127.
    pub const fn range(id: u16) -> Option<(i16, i16)> {
        match of(id) {
            Some(Midi::Bend) => Some((-8192, 8191)),
            Some(_) => Some((0, 127)),
            None => None,
        }
    }

    /// The value a control has before anything sets it, as General MIDI 2
    /// has its controllers after Reset all controllers or at power-on:
    /// volume (7) 100, balance (8) and pan (10) 64, expression (11) 127,
    /// the sound controllers (70-79) 64, reverb send (91) 40; every other
    /// controller 0, the bend in the middle (0) and no pressure.
    pub const fn default(id: u16) -> Option<i16> {
        match of(id) {
            Some(Midi::Cc(7)) => Some(100),
            Some(Midi::Cc(8 | 10 | 70..=79)) => Some(64),
            Some(Midi::Cc(11)) => Some(127),
            Some(Midi::Cc(91)) => Some(40),
            Some(_) => Some(0),
            None => None,
        }
    }

    /// Its name on screens and in the log: "CC 74", "Pitch bend",
    /// "Pressure".
    pub fn name(id: u16) -> Option<String> {
        Some(match of(id)? {
            Midi::Cc(n) => format!("CC {n}"),
            Midi::Bend => "Pitch bend".to_string(),
            Midi::Pressure => "Pressure".to_string(),
        })
    }
}

/// [`Owner::Clips`]' parameters. A clip, a scene and a track are named by
/// where they are (a clip by its track and row, from 0), so a controller's
/// grid keeps its place whatever the clips are called.
pub mod clips {
    /// Stop every track (the grid's Stop all).
    pub const STOP_ALL: u16 = 0;
    /// Every track back to the arrangement (the grid's Back to song).
    pub const BACK_TO_SONG: u16 = 1;
    /// The tracks the ids below count to (`mpfree_seq::MAX_TRACKS`).
    pub const TRACKS: u16 = 128;
    /// The rows of the grid (`mpfree_seq::SLOTS`).
    pub const ROWS: u16 = 16;
    const SCENE: u16 = 0x100;
    const STOP: u16 = 0x200;
    const MUTE: u16 = 0x300;
    const SOLO: u16 = 0x400;
    const ARM: u16 = 0x500;
    const CLIP: u16 = 0x1000;

    /// What a [`Owner::Clips`](super::Owner::Clips) parameter does.
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub enum Clips {
        StopAll,
        BackToSong,
        /// Launch row `row` as a scene (the row's button).
        Scene(u8),
        /// The cell at `(track, row)`, as a tap on it: launch its clip,
        /// stop it while it plays, take back a launch waiting.
        Clip(u8, u8),
        /// Stop the track's clip.
        Stop(u8),
        /// Mute the track (on) or let it play (off), as TRACK MUTE does.
        Mute(u8),
        /// Solo the track (on) or let go of its solo.
        Solo(u8),
        /// Arm the track, as a tap on its name does.
        Arm(u8),
    }

    /// Launch row `row` as a scene.
    pub const fn scene(row: u8) -> u16 {
        SCENE + row as u16
    }
    /// The cell of track `track`, row `row`.
    pub const fn clip(track: u8, row: u8) -> u16 {
        CLIP + track as u16 * ROWS + row as u16
    }
    pub const fn stop(track: u8) -> u16 {
        STOP + track as u16
    }
    pub const fn mute(track: u8) -> u16 {
        MUTE + track as u16
    }
    pub const fn solo(track: u8) -> u16 {
        SOLO + track as u16
    }
    pub const fn arm(track: u8) -> u16 {
        ARM + track as u16
    }

    /// What parameter `id` does, if it is one.
    pub fn of(id: u16) -> Option<Clips> {
        if id >= CLIP {
            let i = id - CLIP;
            let (track, row) = ((i / ROWS) as u8, (i % ROWS) as u8);
            return (i < TRACKS * ROWS).then_some(Clips::Clip(track, row));
        }
        let (kind, n) = (id & !0xFF, id & 0xFF);
        match kind {
            0 => match id {
                STOP_ALL => Some(Clips::StopAll),
                BACK_TO_SONG => Some(Clips::BackToSong),
                _ => None,
            },
            SCENE => (n < ROWS).then_some(Clips::Scene(n as u8)),
            STOP | MUTE | SOLO | ARM if n < TRACKS => {
                let t = n as u8;
                Some(match kind {
                    STOP => Clips::Stop(t),
                    MUTE => Clips::Mute(t),
                    SOLO => Clips::Solo(t),
                    _ => Clips::Arm(t),
                })
            }
            _ => None,
        }
    }

    /// Whether `id` takes a value (a mute or a solo, on or off) rather
    /// than a press.
    pub fn is_value(id: u16) -> bool {
        matches!(of(id), Some(Clips::Mute(_) | Clips::Solo(_)))
    }
}

/// [`Owner::Looper`]'s parameters. The numbers are fixed and never
/// renumbered: a rig keeps its footswitches by them.
pub mod looper {
    /// The picked loop's key: what a tap on its ring on the Looper tab
    /// does (record, end the first pass, overdub, play), held for a
    /// second it clears the loop.
    pub const PICKED_KEY: u16 = 0;
    /// The picked loop's mute: on (muted) from the middle up.
    pub const PICKED_MUTE: u16 = 1;
    /// Pick the next loop (after Loop 8, Loop 1).
    pub const NEXT_LOOP: u16 = 2;
    /// The picked loop's Undo.
    pub const UNDO: u16 = 3;
    /// The picked loop's Redo.
    pub const REDO: u16 = 4;
    /// Capture: the last bars heard become a loop.
    pub const CAPTURE: u16 = 5;
    /// The picked loop's Clear (Undo brings it back).
    pub const CLEAR: u16 = 6;
    /// The loops the ids below count to (`mpfree_looper::MAX_LOOPS`).
    pub const LOOPS: u16 = 8;
    const KEY: u16 = 0x100;
    const MUTE: u16 = 0x200;

    /// Which loop a key or a mute is for.
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub enum Which {
        /// The loop picked when the control moves.
        Picked,
        /// Loop `n` (0 is Loop 1).
        Loop(u8),
    }

    /// What a [`Owner::Looper`](super::Owner::Looper) parameter does.
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub enum Looper {
        /// A loop's key, pressed and let go as its pad is.
        Key(Which),
        /// A loop's mute, on or off.
        Mute(Which),
        NextLoop,
        Undo,
        Redo,
        Capture,
        Clear,
    }

    /// Loop `n`'s key (0 is Loop 1).
    pub const fn key(n: u8) -> u16 {
        KEY + n as u16
    }
    /// Loop `n`'s mute.
    pub const fn mute(n: u8) -> u16 {
        MUTE + n as u16
    }
    /// A key or a mute for `which`.
    pub const fn key_of(which: Which) -> u16 {
        match which {
            Which::Picked => PICKED_KEY,
            Which::Loop(n) => key(n),
        }
    }
    pub const fn mute_of(which: Which) -> u16 {
        match which {
            Which::Picked => PICKED_MUTE,
            Which::Loop(n) => mute(n),
        }
    }

    /// What parameter `id` does, if it is one.
    pub fn of(id: u16) -> Option<Looper> {
        let (kind, n) = (id & !0xFF, id & 0xFF);
        Some(match kind {
            0 => match id {
                PICKED_KEY => Looper::Key(Which::Picked),
                PICKED_MUTE => Looper::Mute(Which::Picked),
                NEXT_LOOP => Looper::NextLoop,
                UNDO => Looper::Undo,
                REDO => Looper::Redo,
                CAPTURE => Looper::Capture,
                CLEAR => Looper::Clear,
                _ => return None,
            },
            KEY if n < LOOPS => Looper::Key(Which::Loop(n as u8)),
            MUTE if n < LOOPS => Looper::Mute(Which::Loop(n as u8)),
            _ => return None,
        })
    }

    /// Whether `id` takes a value (a mute, on or off) rather than a press.
    pub fn is_value(id: u16) -> bool {
        matches!(of(id), Some(Looper::Mute(_)))
    }

    /// The parameter's name, as the Learn section lists it after
    /// "Looper · ": "Loop 3 key", "picked loop's key", "Loop 3 mute",
    /// "picked loop's mute", "Next loop", "Undo" ...
    pub fn name(id: u16) -> Option<String> {
        let loop_name = |w: Which, what: &str| match w {
            Which::Picked => format!("picked loop's {what}"),
            Which::Loop(n) => format!("Loop {} {what}", u16::from(n) + 1),
        };
        Some(match of(id)? {
            Looper::Key(w) => loop_name(w, "key"),
            Looper::Mute(w) => loop_name(w, "mute"),
            Looper::NextLoop => "Next loop".to_string(),
            Looper::Undo => "Undo".to_string(),
            Looper::Redo => "Redo".to_string(),
            Looper::Capture => "Capture".to_string(),
            Looper::Clear => "Clear".to_string(),
        })
    }
}

/// [`Owner::Perform`]'s parameters: one per Perform pad, page A's pad 1
/// first. The numbers are fixed and never renumbered: a rig keeps its
/// learned keys by them.
pub mod perform {
    /// The Perform set's pages (A to H).
    pub const PAGES: u8 = 8;
    /// The pads on a page.
    pub const PADS: u8 = 16;

    /// Page `page`'s pad `pad` (0 is page A, pad 1).
    pub const fn pad(page: u8, pad: u8) -> u16 {
        page as u16 * PADS as u16 + pad as u16
    }

    /// The page and the pad parameter `id` plays, if it is one.
    pub fn of(id: u16) -> Option<(u8, u8)> {
        let (page, pad) = (id / u16::from(PADS), id % u16::from(PADS));
        (page < u16::from(PAGES)).then_some((page as u8, pad as u8))
    }

    /// The pad's name, as the Perform view labels it: "B2".
    pub fn name(id: u16) -> Option<String> {
        let (page, pad) = of(id)?;
        Some(format!("{}{}", (b'A' + page) as char, u16::from(pad) + 1))
    }
}

/// [`Owner::Transport`]'s parameters.
pub mod transport {
    /// PLAY and STOP on one button: on plays (as PLAY), off stops (as
    /// STOP). A learned key flips it on each press.
    pub const PLAYING: u16 = 0;
    /// The MPC's PLAY, pressed while the control is.
    pub const PLAY: u16 = 1;
    /// STOP.
    pub const STOP: u16 = 2;
    /// PLAY START.
    pub const PLAY_START: u16 = 3;
    /// REC.
    pub const REC: u16 = 4;
    /// OVERDUB.
    pub const OVERDUB: u16 = 5;
    /// TAP TEMPO.
    pub const TAP_TEMPO: u16 = 6;
    /// The tempo, in BPM over the app's range.
    pub const TEMPO: u16 = 7;
    /// Every one, in the order screens list them, with its name.
    pub const ALL: [(u16, &str); 8] = [
        (PLAYING, "PLAY and STOP"),
        (PLAY, "PLAY"),
        (STOP, "STOP"),
        (PLAY_START, "PLAY START"),
        (REC, "REC"),
        (OVERDUB, "OVERDUB"),
        (TAP_TEMPO, "TAP TEMPO"),
        (TEMPO, "Tempo"),
    ];

    /// The name of parameter `id`, if it is one.
    pub fn name(id: u16) -> Option<&'static str> {
        ALL.iter().find(|(i, _)| *i == id).map(|(_, n)| *n)
    }

    /// Whether `id` is a press (a button of the MPC's), not a value.
    pub const fn is_press(id: u16) -> bool {
        matches!(id, PLAY | STOP | PLAY_START | REC | OVERDUB | TAP_TEMPO)
    }
}

/// [`Owner::Arp`]'s parameters: the arp settings its Q-Links turn, for MIDI
/// Learn (a learned knob turns a track's arp as Q-Links 1 to 4 do while
/// NOTE REPEAT is held). Learned controls are saved with these numbers in
/// the rig and the project, so they never change: a new setting takes the
/// next number.
pub mod arp {
    /// The rate, through the rates from 1/4 to 1/32T.
    pub const RATE: u16 = 0;
    /// The gate: a note's length, 1 to 100 % of a step.
    pub const GATE: u16 = 1;
    /// The swing, 50 % (straight) to 75 %.
    pub const SWING: u16 = 2;
    /// The octaves an order climbs, 1 to 4.
    pub const OCTAVES: u16 = 3;
    /// Every one, in Q-Link order, with its name.
    pub const ALL: [(u16, &str); 4] = [
        (RATE, "Rate"),
        (GATE, "Gate"),
        (SWING, "Swing"),
        (OCTAVES, "Octaves"),
    ];

    /// The name of parameter `id`, if it is one.
    pub fn name(id: u16) -> Option<&'static str> {
        ALL.iter().find(|(i, _)| *i == id).map(|(_, n)| *n)
    }
}

/// [`Owner::Pad`]'s and [`Owner::KitPad`]'s parameters, as far as this
/// crate names them (sampler step 17). Ids below [`LAYER_BASE`] are the
/// pad's own settings (the sampler's `PadField` numbers). From it, each of
/// the pad's layers has [`LAYER_SETTINGS`] ids for its grain settings (the
/// sampler's `GrainField` numbers): layer `l`'s setting `g` is
/// `LAYER_BASE + l × LAYER_SETTINGS + g`. The text form is the number, so
/// a build that doesn't know an id reads and keeps it, moving nothing.
pub mod pad {
    /// The first layer id: layer 0's setting 0.
    pub const LAYER_BASE: u16 = 256;
    /// Ids a layer has, one a grain setting.
    pub const LAYER_SETTINGS: u16 = 64;
    /// Layers a pad can have (the sampler's `MAX_LAYERS`).
    pub const LAYERS: u8 = 16;

    /// Layer `layer`'s setting `setting`; None past the last layer or
    /// setting.
    pub const fn layer(layer: u8, setting: u8) -> Option<u16> {
        if layer >= LAYERS || setting as u16 >= LAYER_SETTINGS {
            return None;
        }
        Some(LAYER_BASE + layer as u16 * LAYER_SETTINGS + setting as u16)
    }

    /// The layer and setting id `id` names ([`layer`]'s inverse); None for
    /// one of the pad's own settings, or past the last layer.
    pub const fn layer_of(id: u16) -> Option<(u8, u8)> {
        if id < LAYER_BASE {
            return None;
        }
        let n = id - LAYER_BASE;
        let l = n / LAYER_SETTINGS;
        if l >= LAYERS as u16 {
            return None;
        }
        Some((l as u8, (n % LAYER_SETTINGS) as u8))
    }
}

/// [`Owner::Play`]'s parameters: the Play page's own controls, for MIDI
/// Learn. Learned controls are saved with these numbers in the rig and the
/// project, so they never change: a new control takes the next number.
pub mod play {
    /// The swing, 50 % (straight) to 75 %, as Play's Swing Q-Link turns it.
    pub const SWING: u16 = 0;
    /// The filter Play's Filter Q-Link turns: track 1's DJ filter (Play
    /// calls it Drums), whatever the project names track 1. -100
    /// (low-pass) to 100 (high-pass), 0 off.
    pub const FILTER: u16 = 1;
    /// The XY pad across, 0 (left) to 1 (right).
    pub const XY_ACROSS: u16 = 2;
    /// The XY pad up, 0 (bottom) to 1 (top).
    pub const XY_UP: u16 = 3;
    /// Every one, in the order the Learn section lists them, with its name.
    pub const ALL: [(u16, &str); 4] = [
        (SWING, "Swing"),
        (FILTER, "Filter"),
        (XY_ACROSS, "XY across"),
        (XY_UP, "XY up"),
    ];

    /// The name of parameter `id`, if it is one.
    pub fn name(id: u16) -> Option<&'static str> {
        ALL.iter().find(|(i, _)| *i == id).map(|(_, n)| *n)
    }
}

/// What on a track a [`RelAddr`] names, without the track.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum OnTrack {
    Inst,
    Pad(u8),
    /// The track's mixer strip.
    Strip,
    /// An insert on the track's mixer strip, by slot id.
    Insert(u16),
    Arp,
    Seq,
    /// One of the track's MIDI effects, by its id.
    NoteFx(u16),
    /// The track's controllers, bend and pressure out ([`Owner::Midi`]).
    Midi,
}

impl OnTrack {
    /// The owner on track `t`.
    pub const fn on(self, t: u8) -> Owner {
        match self {
            OnTrack::Inst => Owner::Inst(t),
            OnTrack::Pad(p) => Owner::Pad(t, p),
            OnTrack::Strip => Owner::Strip(track_strip(t)),
            OnTrack::Insert(uid) => Owner::Insert(track_strip(t), uid),
            OnTrack::Arp => Owner::Arp(t),
            OnTrack::Seq => Owner::Seq(t),
            OnTrack::NoteFx(id) => Owner::NoteFx(t, id),
            OnTrack::Midi => Owner::Midi(t),
        }
    }
}

impl Owner {
    /// The track this owner belongs to and what on it, or `None` for an
    /// owner that is not on a track (a bus, a macro ...).
    pub const fn split(self) -> Option<(u8, OnTrack)> {
        match self {
            Owner::Inst(t) => Some((t, OnTrack::Inst)),
            Owner::Pad(t, p) => Some((t, OnTrack::Pad(p))),
            Owner::Strip(s) => match strip_track(s) {
                Some(t) => Some((t, OnTrack::Strip)),
                None => None,
            },
            Owner::Insert(s, uid) => match strip_track(s) {
                Some(t) => Some((t, OnTrack::Insert(uid))),
                None => None,
            },
            Owner::Arp(t) => Some((t, OnTrack::Arp)),
            Owner::Seq(t) => Some((t, OnTrack::Seq)),
            Owner::NoteFx(t, id) => Some((t, OnTrack::NoteFx(id))),
            Owner::Midi(t) => Some((t, OnTrack::Midi)),
            _ => None,
        }
    }

    /// The track, for an owner on a track.
    pub const fn track(self) -> Option<u8> {
        match self.split() {
            Some((t, _)) => Some(t),
            None => None,
        }
    }

    /// The same owner moved to track `t`; an owner not on a track stays.
    pub const fn with_track(self, t: u8) -> Owner {
        match self.split() {
            Some((_, on)) => on.on(t),
            None => self,
        }
    }

    /// The mixer strip this owner lives on, for strips and inserts.
    pub const fn strip(self) -> Option<Strip> {
        match self {
            Owner::Strip(s) | Owner::Insert(s, _) => Some(s),
            _ => None,
        }
    }
}

/// A parameter: whose, and which.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ParamAddr {
    pub owner: Owner,
    pub param: u16,
}

impl ParamAddr {
    pub const fn new(owner: Owner, param: u16) -> ParamAddr {
        ParamAddr { owner, param }
    }

    /// The same parameter on track `t` (see [`Owner::with_track`]).
    pub const fn with_track(self, t: u8) -> ParamAddr {
        ParamAddr {
            owner: self.owner.with_track(t),
            param: self.param,
        }
    }

    /// The address without its track, for a clip to store; `None` when the
    /// owner is not on a track.
    pub const fn rel(self) -> Option<RelAddr> {
        match self.owner.split() {
            Some((_, on)) => Some(RelAddr {
                on,
                param: self.param,
            }),
            None => None,
        }
    }

    /// A macro's value.
    pub const fn macro_value(n: u8) -> ParamAddr {
        ParamAddr::new(Owner::Macro(n), 0)
    }

    /// A morph's position.
    pub const fn morph_pos(n: u8) -> ParamAddr {
        ParamAddr::new(Owner::Morph(n), 0)
    }
}

/// A parameter on "this track", for clips (see the crate docs).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RelAddr {
    pub on: OnTrack,
    pub param: u16,
}

impl RelAddr {
    pub const fn new(on: OnTrack, param: u16) -> RelAddr {
        RelAddr { on, param }
    }

    /// The address on track `t`.
    pub const fn on(self, t: u8) -> ParamAddr {
        ParamAddr {
            owner: self.on.on(t),
            param: self.param,
        }
    }
}

// ---- Text form -------------------------------------------------------------

impl fmt::Display for Strip {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Strip::Input(n) => write!(f, "in{n}"),
            Strip::Pad(n) => write!(f, "ps{n}"),
            Strip::Track(n) => write!(f, "t{n}"),
            Strip::Bus(n) => write!(f, "bus{n}"),
            Strip::Return(n) => write!(f, "ret{n}"),
            Strip::Master => f.write_str("master"),
        }
    }
}

impl fmt::Display for OnTrack {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            OnTrack::Inst => f.write_str("inst"),
            OnTrack::Pad(p) => write!(f, "pad{p}"),
            OnTrack::Strip => f.write_str("mix"),
            OnTrack::Insert(uid) => write!(f, "fx{uid}"),
            OnTrack::Arp => f.write_str("arp"),
            OnTrack::Seq => f.write_str("seq"),
            OnTrack::NoteFx(id) => write!(f, "nfx{id}"),
            OnTrack::Midi => f.write_str("midi"),
        }
    }
}

impl fmt::Display for Owner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Owner::Strip(s) => write!(f, "{s}.mix"),
            Owner::Insert(s, uid) => write!(f, "{s}.fx{uid}"),
            Owner::Macro(n) => write!(f, "macro{n}"),
            Owner::Mod(n) => write!(f, "mod{n}"),
            Owner::Morph(n) => write!(f, "morph{n}"),
            Owner::Transport => f.write_str("transport"),
            Owner::Clips => f.write_str("clips"),
            Owner::Looper => f.write_str("looper"),
            Owner::Perform => f.write_str("perform"),
            Owner::Play => f.write_str("play"),
            Owner::KitPad(k, p) => write!(f, "kit{k}.pad{p}"),
            Owner::Sound(n) => write!(f, "i{n}"),
            o => {
                let (t, on) = o.split().expect("every other owner is on a track");
                write!(f, "t{t}.{on}")
            }
        }
    }
}

impl fmt::Display for ParamAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.owner, self.param)
    }
}

impl fmt::Display for RelAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.on, self.param)
    }
}

/// Why a text address did not read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError(pub String);

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "bad parameter address: {}", self.0)
    }
}

impl std::error::Error for ParseError {}

/// `word` as `prefix` followed by a number that fits `T`.
fn num<T: FromStr>(word: &str, prefix: &str) -> Option<T> {
    let digits = word.strip_prefix(prefix)?;
    // Plain decimal only: no sign, no leading "+", nothing empty.
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

fn strip(word: &str) -> Option<Strip> {
    if word == "master" {
        return Some(Strip::Master);
    }
    // Longer prefixes first: "ps" before "p", "ret" before "t".
    if let Some(n) = num(word, "ps") {
        return Some(Strip::Pad(n));
    }
    if let Some(n) = num(word, "in") {
        return Some(Strip::Input(n));
    }
    if let Some(n) = num(word, "bus") {
        return Some(Strip::Bus(n));
    }
    if let Some(n) = num(word, "ret") {
        return Some(Strip::Return(n));
    }
    num(word, "t").map(Strip::Track)
}

fn on_track(word: &str) -> Option<OnTrack> {
    match word {
        "inst" => Some(OnTrack::Inst),
        "mix" => Some(OnTrack::Strip),
        "arp" => Some(OnTrack::Arp),
        "seq" => Some(OnTrack::Seq),
        "midi" => Some(OnTrack::Midi),
        _ => num(word, "pad")
            .map(OnTrack::Pad)
            .or_else(|| num(word, "fx").map(OnTrack::Insert))
            .or_else(|| num(word, "nfx").map(OnTrack::NoteFx)),
    }
}

/// The owner named by `words` (the text before the parameter, split at
/// dots).
fn owner(words: &[&str]) -> Option<Owner> {
    match *words {
        ["transport"] => Some(Owner::Transport),
        ["clips"] => Some(Owner::Clips),
        ["looper"] => Some(Owner::Looper),
        ["perform"] => Some(Owner::Perform),
        ["play"] => Some(Owner::Play),
        [w] => num(w, "macro")
            .map(Owner::Macro)
            .or_else(|| num(w, "mod").map(Owner::Mod))
            .or_else(|| num(w, "morph").map(Owner::Morph))
            .or_else(|| num(w, "i").map(Owner::Sound)),
        [s, what] => {
            if let Some(k) = num(s, "kit") {
                return Some(Owner::KitPad(k, num(what, "pad")?));
            }
            let s = strip(s)?;
            if what == "mix" {
                return Some(Owner::Strip(s));
            }
            if let Some(uid) = num(what, "fx") {
                return Some(Owner::Insert(s, uid));
            }
            // Instrument, sampler pad, arp, sequencer, MIDI effects and
            // MIDI out: tracks only.
            let Strip::Track(t) = s else { return None };
            Some(on_track(what)?.on(t))
        }
        _ => None,
    }
}

impl FromStr for ParamAddr {
    type Err = ParseError;
    fn from_str(s: &str) -> Result<ParamAddr, ParseError> {
        let bad = || ParseError(s.to_string());
        let (head, param) = s.rsplit_once('.').ok_or_else(bad)?;
        let param = num(param, "").ok_or_else(bad)?;
        let words: Vec<&str> = head.split('.').collect();
        Ok(ParamAddr {
            owner: owner(&words).ok_or_else(bad)?,
            param,
        })
    }
}

impl FromStr for RelAddr {
    type Err = ParseError;
    fn from_str(s: &str) -> Result<RelAddr, ParseError> {
        let bad = || ParseError(s.to_string());
        let (on, param) = s.split_once('.').ok_or_else(bad)?;
        Ok(RelAddr {
            on: on_track(on).ok_or_else(bad)?,
            param: num(param, "").ok_or_else(bad)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn every_owner() -> Vec<Owner> {
        let strips = [
            Strip::Input(0),
            Strip::Pad(127),
            Strip::Track(3),
            Strip::Bus(7),
            Strip::Return(3),
            Strip::Master,
        ];
        let mut v = vec![
            Owner::Inst(0),
            Owner::Inst(63),
            Owner::Pad(3, 5),
            Owner::KitPad(1, 0),
            Owner::KitPad(u16::MAX, 127),
            Owner::Sound(1),
            Owner::Sound(u16::MAX),
            Owner::Arp(1),
            Owner::Seq(2),
            Owner::NoteFx(3, 2),
            Owner::NoteFx(0, u16::MAX),
            Owner::Midi(0),
            Owner::Midi(127),
            Owner::Macro(7),
            Owner::Mod(4),
            Owner::Morph(0),
            Owner::Transport,
            Owner::Clips,
            Owner::Looper,
            Owner::Perform,
            Owner::Play,
        ];
        for s in strips {
            v.push(Owner::Strip(s));
            v.push(Owner::Insert(s, 0));
            v.push(Owner::Insert(s, u16::MAX));
        }
        v
    }

    #[test]
    fn small_and_copy() {
        assert!(
            std::mem::size_of::<ParamAddr>() <= 8,
            "{}",
            std::mem::size_of::<ParamAddr>()
        );
        assert!(std::mem::size_of::<RelAddr>() <= 6);
    }

    #[test]
    fn a_pads_layer_ids_follow_its_own_settings_and_come_back() {
        assert_eq!(pad::layer(0, 0), Some(pad::LAYER_BASE));
        assert_eq!(pad::layer(2, 12), Some(396));
        assert_eq!(pad::layer(15, 63), Some(1279));
        assert_eq!(pad::layer(16, 0), None, "a pad has 16 layers");
        assert_eq!(pad::layer(0, 64), None, "a layer has 64 ids");
        for id in 0..pad::LAYER_BASE {
            assert_eq!(pad::layer_of(id), None, "{id} is the pad's own");
        }
        for l in 0..pad::LAYERS {
            for g in 0..pad::LAYER_SETTINGS as u8 {
                let id = pad::layer(l, g).unwrap();
                assert_eq!(pad::layer_of(id), Some((l, g)));
            }
        }
        assert_eq!(pad::layer_of(1280), None, "past the last layer");
    }

    #[test]
    fn text_examples() {
        let cases = [
            (ParamAddr::new(Owner::Inst(3), 12), "t3.inst.12"),
            (ParamAddr::new(Owner::Pad(3, 5), 2), "t3.pad5.2"),
            (ParamAddr::new(Owner::KitPad(9, 5), 2), "kit9.pad5.2"),
            (ParamAddr::new(Owner::Sound(9), 12), "i9.12"),
            (
                ParamAddr::new(Owner::KitPad(9, 5), pad::layer(2, 12).unwrap()),
                "kit9.pad5.396",
            ),
            (ParamAddr::new(Owner::Strip(Strip::Track(3)), 0), "t3.mix.0"),
            (
                ParamAddr::new(Owner::Insert(Strip::Bus(2), 7), 4),
                "bus2.fx7.4",
            ),
            (ParamAddr::new(Owner::Strip(Strip::Pad(9)), 0), "ps9.mix.0"),
            (
                ParamAddr::new(Owner::Strip(Strip::Master), 1),
                "master.mix.1",
            ),
            (
                ParamAddr::new(Owner::Insert(Strip::Return(1), 3), 0),
                "ret1.fx3.0",
            ),
            (ParamAddr::new(Owner::Arp(3), 1), "t3.arp.1"),
            (ParamAddr::new(Owner::NoteFx(3, 2), 4), "t3.nfx2.4"),
            (ParamAddr::new(Owner::Midi(3), 74), "t3.midi.74"),
            (ParamAddr::new(Owner::Midi(3), midi::BEND), "t3.midi.128"),
            (ParamAddr::macro_value(2), "macro2.0"),
            (ParamAddr::new(Owner::Mod(4), 1), "mod4.1"),
            (ParamAddr::morph_pos(0), "morph0.0"),
            (
                ParamAddr::new(Owner::Transport, transport::TEMPO),
                "transport.7",
            ),
            (ParamAddr::new(Owner::Clips, clips::scene(2)), "clips.258"),
            (ParamAddr::new(Owner::Looper, looper::key(2)), "looper.258"),
            (ParamAddr::new(Owner::Looper, looper::CLEAR), "looper.6"),
            (
                ParamAddr::new(Owner::Perform, perform::pad(1, 1)),
                "perform.17",
            ),
            (ParamAddr::new(Owner::Play, play::SWING), "play.0"),
            (ParamAddr::new(Owner::Play, play::XY_UP), "play.3"),
        ];
        for (a, s) in cases {
            assert_eq!(a.to_string(), s);
            assert_eq!(s.parse::<ParamAddr>(), Ok(a), "{s}");
        }
    }

    #[test]
    fn a_tracks_strip_is_drums_for_track_1_and_its_own_for_the_rest() {
        // Track 1's (0) is the Drums strip; track 2's (1) and track 5's
        // (4) their own, after the engines' four.
        let mix = RelAddr::new(OnTrack::Strip, 0);
        assert_eq!(mix.on(0).owner, Owner::Strip(Strip::Track(0)));
        assert_eq!(mix.on(1).owner, Owner::Strip(Strip::Track(5)));
        assert_eq!(mix.on(4).owner, Owner::Strip(Strip::Track(8)));
        let fx = RelAddr::new(OnTrack::Insert(3), 2);
        assert_eq!(fx.on(0).owner, Owner::Insert(Strip::Track(0), 3));
        assert_eq!(fx.on(4).owner, Owner::Insert(Strip::Track(8), 3));
        // And back: each to its track, the round trip unchanged.
        for t in [0u8, 1, 4, 127] {
            for a in [mix.on(t), fx.on(t)] {
                assert_eq!(a.owner.track(), Some(t), "{a}");
                assert_eq!(a.rel().map(|r| r.on(t)), Some(a));
                assert_eq!(strip_track(a.owner.strip().unwrap()), Some(t));
            }
        }
        // Synth, Loop, Audio and the strip that was track 1's are no
        // track's: a knob on them is not this track's strip.
        for i in 1..=ENGINE_STRIPS {
            let o = Owner::Strip(Strip::Track(i));
            assert_eq!(o.split(), None, "t{i}");
            assert_eq!(ParamAddr::new(o, 0).rel(), None);
            assert_eq!(ParamAddr::new(o, 0).with_track(3).owner, o);
        }
        // Moved to another track, a Drums lock follows the track.
        assert_eq!(mix.on(0).with_track(2).owner, Owner::Strip(Strip::Track(6)));
        assert_eq!(mix.on(6).with_track(0).owner, Owner::Strip(Strip::Track(0)));
        // Absolute addresses read and write as they did.
        assert_eq!(mix.on(4).to_string(), "t8.mix.0");
        assert_eq!("t8.mix.0".parse::<ParamAddr>(), Ok(mix.on(4)));
    }

    #[test]
    fn every_owner_round_trips() {
        for o in every_owner() {
            for p in [0u16, 1, 999, u16::MAX] {
                let a = ParamAddr::new(o, p);
                assert_eq!(a.to_string().parse::<ParamAddr>(), Ok(a), "{a}");
            }
        }
    }

    #[test]
    fn the_clips_ids_say_what_they_do() {
        use clips::Clips;
        assert_eq!(clips::of(clips::STOP_ALL), Some(Clips::StopAll));
        assert_eq!(clips::of(clips::BACK_TO_SONG), Some(Clips::BackToSong));
        assert_eq!(clips::of(clips::scene(15)), Some(Clips::Scene(15)));
        assert_eq!(clips::of(clips::scene(16)), None, "16 rows");
        assert_eq!(clips::of(clips::clip(0, 0)), Some(Clips::Clip(0, 0)));
        assert_eq!(clips::of(clips::clip(127, 15)), Some(Clips::Clip(127, 15)));
        assert_eq!(clips::of(clips::clip(127, 15) + 1), None);
        for t in [0u8, 7, 127] {
            assert_eq!(clips::of(clips::stop(t)), Some(Clips::Stop(t)));
            assert_eq!(clips::of(clips::mute(t)), Some(Clips::Mute(t)));
            assert_eq!(clips::of(clips::solo(t)), Some(Clips::Solo(t)));
            assert_eq!(clips::of(clips::arm(t)), Some(Clips::Arm(t)));
            assert!(clips::is_value(clips::mute(t)) && clips::is_value(clips::solo(t)));
            assert!(!clips::is_value(clips::arm(t)) && !clips::is_value(clips::stop(t)));
        }
        assert_eq!(clips::of(clips::mute(127) + 1), None, "128 tracks");
        assert_eq!(clips::of(2), None);
        // No two say the same.
        let mut seen = std::collections::HashSet::new();
        for id in 0..=u16::MAX {
            if let Some(c) = clips::of(id) {
                assert!(seen.insert(c), "{id}");
            }
        }
        assert_eq!(seen.len(), 2 + 16 + 4 * 128 + 128 * 16);
    }

    /// The looper's footswitch ids are fixed numbers (a rig keeps its
    /// footswitches by them): every one reads back, no two say the same,
    /// and only the mutes take a value.
    #[test]
    fn the_loopers_ids_are_fixed_and_each_says_one_thing() {
        use looper::{Looper, Which};
        // The numbers a saved rig has: never renumbered.
        assert_eq!(
            [
                looper::PICKED_KEY,
                looper::PICKED_MUTE,
                looper::NEXT_LOOP,
                looper::UNDO,
                looper::REDO,
                looper::CAPTURE,
                looper::CLEAR,
                looper::key(0),
                looper::key(7),
                looper::mute(0),
                looper::mute(7),
            ],
            [0, 1, 2, 3, 4, 5, 6, 0x100, 0x107, 0x200, 0x207]
        );
        assert_eq!(
            looper::of(looper::key(2)),
            Some(Looper::Key(Which::Loop(2)))
        );
        assert_eq!(
            looper::of(looper::mute(7)),
            Some(Looper::Mute(Which::Loop(7)))
        );
        assert_eq!(
            looper::of(looper::PICKED_KEY),
            Some(Looper::Key(Which::Picked))
        );
        assert_eq!(looper::of(looper::key(8)), None, "8 loops");
        assert_eq!(looper::of(looper::mute(8)), None, "8 loops");
        assert_eq!(looper::of(0x1FF), None);
        assert_eq!(looper::of(7), None);
        for w in [Which::Picked, Which::Loop(0), Which::Loop(7)] {
            assert_eq!(looper::of(looper::key_of(w)), Some(Looper::Key(w)));
            assert_eq!(looper::of(looper::mute_of(w)), Some(Looper::Mute(w)));
            assert!(looper::is_value(looper::mute_of(w)));
            assert!(!looper::is_value(looper::key_of(w)));
        }
        let mut seen = std::collections::HashSet::new();
        let mut names = std::collections::HashSet::new();
        for id in 0..=u16::MAX {
            if let Some(c) = looper::of(id) {
                assert!(seen.insert(c), "{id}");
                assert!(names.insert(looper::name(id).unwrap()), "{id}");
            }
        }
        assert_eq!(seen.len(), 7 + 2 * 8);
        assert_eq!(looper::name(looper::key(2)).unwrap(), "Loop 3 key");
        assert_eq!(
            looper::name(looper::PICKED_KEY).unwrap(),
            "picked loop's key"
        );
        assert_eq!(looper::name(looper::mute(0)).unwrap(), "Loop 1 mute");
        assert_eq!(looper::name(looper::NEXT_LOOP).unwrap(), "Next loop");
    }

    /// The Perform pads' ids are fixed numbers (a rig keeps its learned
    /// keys by them): page A's pad 1 is 0, page H's pad 16 is 127, each
    /// reads back to its page and pad, and nothing past them is one.
    #[test]
    fn the_perform_pads_ids_are_fixed_and_each_names_its_pad() {
        assert_eq!(perform::pad(0, 0), 0);
        assert_eq!(perform::pad(1, 1), 17);
        assert_eq!(perform::pad(7, 15), 127);
        let mut names = std::collections::HashSet::new();
        for page in 0..perform::PAGES {
            for pad in 0..perform::PADS {
                let id = perform::pad(page, pad);
                assert_eq!(perform::of(id), Some((page, pad)));
                assert!(names.insert(perform::name(id).unwrap()), "{id}");
            }
        }
        assert_eq!(perform::of(128), None);
        assert_eq!(perform::name(perform::pad(1, 1)).as_deref(), Some("B2"));
        assert_eq!(perform::name(perform::pad(7, 15)).as_deref(), Some("H16"));
    }

    /// A track's MIDI out controls: controllers 0-119 by their own number,
    /// the bend and the pressure; never the channel mode messages. Each
    /// starts at General MIDI 2's value.
    #[test]
    fn a_tracks_midi_controls_are_named_and_start_where_general_midi_does() {
        use midi::Midi;
        assert_eq!(midi::of(0), Some(Midi::Cc(0)));
        assert_eq!(midi::of(74), Some(Midi::Cc(74)));
        assert_eq!(midi::of(119), Some(Midi::Cc(119)));
        for mode in 120..128 {
            assert_eq!(midi::of(mode), None, "CC {mode} is a channel mode message");
        }
        assert_eq!(midi::of(midi::BEND), Some(Midi::Bend));
        assert_eq!(midi::of(midi::PRESSURE), Some(Midi::Pressure));
        assert_eq!(midi::of(midi::COUNT), None);
        assert_eq!(midi::range(midi::BEND), Some((-8192, 8191)));
        assert_eq!(midi::range(1), Some((0, 127)));
        assert_eq!(midi::range(121), None);
        let d = |id| midi::default(id).unwrap();
        assert_eq!((d(7), d(10), d(11), d(74), d(91)), (100, 64, 127, 64, 40));
        assert_eq!(
            (d(1), d(64), d(midi::BEND), d(midi::PRESSURE)),
            (0, 0, 0, 0)
        );
        for id in 0..midi::COUNT {
            if let (Some((lo, hi)), Some(v)) = (midi::range(id), midi::default(id)) {
                assert!((lo..=hi).contains(&v), "{id}");
            }
        }
        assert_eq!(midi::name(74).as_deref(), Some("CC 74"));
        assert_eq!(midi::name(midi::BEND).as_deref(), Some("Pitch bend"));
        // On a track, and in a clip without it.
        let a = ParamAddr::new(Owner::Midi(2), 74);
        assert_eq!(a.rel().map(|r| r.to_string()).as_deref(), Some("midi.74"));
        assert_eq!("midi.74".parse::<RelAddr>().map(|r| r.on(2)), Ok(a));
        assert_eq!(a.with_track(5).owner, Owner::Midi(5));
    }

    #[test]
    fn bad_text_is_refused() {
        for s in [
            "",
            "t3",
            "t3.inst",
            "t3.inst.",
            "t3.inst.x",
            "t3.inst.-1",
            "t3.inst.+1",
            "t3.inst.70000",
            "t300.inst.1",
            "bus2.inst.1",
            "master.pad1.0",
            "t3.fx.1",
            "t3.nfx.1",
            "bus2.nfx1.0",
            "macro.0",
            "foo.1",
            "t3.mix.0.1",
            "t.mix.0",
            "T3.mix.0",
            "transport0.1",
            "t1.transport.0",
            "bus2.midi.7",
            "t3.midi3.7",
            "play0.1",
            "t1.play.0",
            "kit9.inst.1",
            "kit.pad1.0",
            "kit9.pad.0",
            "kit70000.pad1.0",
            "kit9.pad1",
            "i.1",
            "i9.pad1.0",
            "i70000.1",
        ] {
            assert!(s.parse::<ParamAddr>().is_err(), "{s:?} should not parse");
        }
        for s in ["", "inst", "inst.", "t3.inst.1", "pad.1", "mix.x"] {
            assert!(s.parse::<RelAddr>().is_err(), "{s:?} should not parse");
        }
    }

    /// Play's ids are a saved format: these numbers never change.
    #[test]
    fn plays_ids_stay_put() {
        assert_eq!(
            [play::SWING, play::FILTER, play::XY_ACROSS, play::XY_UP],
            [0, 1, 2, 3]
        );
        assert_eq!(play::name(play::XY_ACROSS), Some("XY across"));
        assert_eq!(play::name(4), None);
        assert_eq!(Owner::Play.track(), None);
    }

    #[test]
    fn tracks_come_off_and_go_back_on() {
        for o in every_owner() {
            let a = ParamAddr::new(o, 9);
            match a.rel() {
                Some(r) => {
                    let t = o.track().unwrap();
                    assert_eq!(r.on(t), a);
                    assert_eq!(r.to_string().parse::<RelAddr>(), Ok(r));
                    let moved = a.with_track(t.wrapping_add(5));
                    assert_eq!(moved.owner.track(), Some(t.wrapping_add(5)));
                    assert_eq!(moved.rel(), Some(r));
                }
                None => {
                    assert_eq!(a.with_track(5), a, "{a} is not on a track");
                    assert_eq!(o.track(), None);
                }
            }
        }
        // An insert on a track's strip moves with the track (track 1's is
        // the Drums strip, t0; track 5's is t8); on a bus it stays.
        assert_eq!(
            ParamAddr::new(Owner::Insert(Strip::Track(0), 7), 2)
                .with_track(4)
                .to_string(),
            "t8.fx7.2"
        );
        assert_eq!(
            ParamAddr::new(Owner::Insert(Strip::Bus(1), 7), 2)
                .with_track(4)
                .to_string(),
            "bus1.fx7.2"
        );
        assert_eq!(
            "fx7.2".parse::<RelAddr>().unwrap().on(0).to_string(),
            "t0.fx7.2"
        );
    }
}
