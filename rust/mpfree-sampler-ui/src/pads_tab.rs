//! The app's Pads tab: the kit's pads at a glance, to pick one, find it a
//! sound and set it up. It is a top-level tab of its own, between Play and
//! Arrange (the owner's pick, 12 Nov 2026 15:46Z: "the pads are the
//! centerpiece of the MPC, and deserve their own top menu"); until then it
//! was a sub-page of the Sample tab.
//!
//! - The **bar** over everything shows the pad picked, the kit's **Kit**
//!   button (a tap is [`KIT_BUTTON`]'s: the app opens its list of kits,
//!   which the top bar's kit chip opened until the Play, Pads and
//!   Sequencer redesign's step 4) and a **Browse for A5** button, which
//!   gives [`PadsAction::Browse`]: the app opens its
//!   full-screen browser, whose Load puts a sound on the pad picked. The
//!   tab has one page ([`LAYOUT`]): the pads, the pad's settings and the
//!   kit's sounds. (It had a Browse view, the browser in a narrow form
//!   beside the pads, until the Play, Pads and Sequencer redesign Jay
//!   approved on 18 Nov 2026 gave browsing a page of its own.)
//! - The **Kit of** row under it ([`KITS`], the sampler review's mockup
//!   5) lists the tracks that play kits, each with its kit, the armed one
//!   outlined, and **+ Kit track**; the app fills it and answers its taps
//!   (a track: arm it, so the pads play its kit; + Kit track: a new track
//!   with a new empty kit).
//! - The **pad map** shows the bank's 16 pads as on the MPC (pad 1 bottom
//!   left): name, layers and play mode, and the clip a pad launches. A tap
//!   picks a pad for every tab (Wave, Chop and Play edit it next); a tap on
//!   the pad picked plays it while the finger is down. Pads held on the
//!   hardware light up.
//! - **Copy**, **Paste** and **Clear** work on the pad picked, each one undo
//!   step. **Map to keys** makes the bank's pads one keygroup instrument:
//!   each pad at its sound's own note (from its keygroup, its name or the
//!   pitch heard, worked out by the app), reaching halfway to its
//!   neighbours, and the pads then play notes; one undo step.
//! - **What A5 is** ([`TYPE`], the Play, Pads and Sequencer redesign's
//!   step 6, board 6) switches what the pad picked is ([`PadType`]):
//!   **Sample** plays its sound; **Loop** is a sample set up to loop in
//!   time (latched, a forward loop over the whole sound, warped, and
//!   started on the launch line: [`as_loop`]); **Instrument** plays one
//!   voice of a Synth tab engine at a note; **MIDI out** sends a note on a
//!   channel to a port of its own. A pad keeps its sound whatever it is,
//!   so going back to Sample loses nothing. Each switch is one undo step.
//! - The **Pad** list holds the settings a pad is set up with, the ones
//!   its type has ([`PadType::rows`]): a sample's sound, play mode,
//!   voices, mute group, level, pan and tune (a loop's tempo in place of
//!   its mode and voices); an instrument's engine and note; a MIDI out
//!   pad's port, channel and note, with where the note goes or why it
//!   would send nothing said under them. Stepping Sound puts the kit's
//!   next or previous sound on the pad.
//! - The **sounds** list holds every sound in the kit, with the pads that
//!   use it; **Put on** puts the sound picked on the pad picked.
//! - **Q-Links 1 to 4** turn the type's main settings
//!   ([`PadType::qlinks`]: a sample's level, pan, tune and voices); the
//!   **wheel** steps the row picked.
//!
//! The app makes a [`PadsState`] for the kit on show, calls
//! [`PadsTab::update`] when the project changes, and applies the
//! [`PadsAction`]s it gets back.

use mpfree_analysis::note_name;
use mpfree_midi::profile::{PadOut, KIT_NOTES};
use mpfree_sampler::model::{
    ChannelPick, Fade, GrainKeys, GrainMode, GrainParams, Layer, Loop, LoopMode, ModelError, Pad,
    PadKind, PlayMode, Project, Region, SourceId, Warp, PADS, WARP_BPM,
};
use mpfree_ui::widgets::{ListItem, ListView};
use mpfree_ui::{EventKind, Ui, UiEvent};

use crate::chop_tab::{bank_of, pad_text, BANK};
use crate::play_tab::{self, stepped, value_text};
use crate::wave_tab::{default_loop, duration_text, mode_label, pad_color};
use crate::widgets::{
    ButtonRow, KitTracks, PadBar, PadCell, PadMap, SectionChip, SectionChips, Setting, SettingList,
};

/// Widget names the tab adds. The app's own are `header` and `hint`.
pub const BAR: &str = "smp-pads-bar";
pub const MAP: &str = "smp-pads-map";
pub const BUTTONS: &str = "smp-pads-buttons";
pub const LIST: &str = "smp-pads-list";
/// What the pad picked is: Sample, Loop, Instrument or MIDI out
/// ([`PadType`]); a tap on chip `i` is [`EventKind::Tab`]`(i)`.
pub const TYPE: &str = "smp-pads-type";
pub const SOUNDS: &str = "smp-pads-sounds";
pub const PUT: &str = "smp-pads-put";
/// The bar's Kit button: a tap is [`EventKind::Tab`] with this on
/// [`BAR`], which the app answers (it opens its list of kits); the tab
/// gives no action for it.
pub const KIT_BUTTON: usize = 1;
/// The "Kit of" row: the tracks that play kits and + Kit track
/// ([`KitTracks`]). The app fills it and answers its taps.
pub const KITS: &str = "smp-pads-kits";

/// The tab's page: the pad map with its buttons, the Pad list, and the
/// kit's sounds with the button that puts one on the pad.
pub const LAYOUT: &str = "col(header:60, smp-pads-bar:60, smp-pads-kits:52, \
                          row(col(smp-pads-map:*, smp-pads-buttons:64):440, \
                          col(smp-pads-type:64, smp-pads-list:*):*, \
                          col(smp-pads-sounds:*, smp-pads-put:64):340):*, hint:26)";

/// The hint line the app shows under the tab.
pub const HINT: &str =
    "Tap a pad to pick it, again to hear it · a type: what the pad plays · Browse for: a sound for it · \
                        pick a sound, then Put it on the pad · Effects: the pad's chain in Mix";

/// Gesture ids the tab hands out start here, clear of other screens'.
const GESTURE_BASE: u64 = 0x5041_4453_0000_0000;

/// The Pad list's rows, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Row {
    Sound,
    Mode,
    Polyphony,
    MuteGroup,
    Level,
    Pan,
    Tune,
    /// The pad's insert effects on its mixer strip: a button that opens
    /// its chain in the Mix tab.
    Effects,
    /// A loop's tempo: the one its sound was played at, which sets how
    /// many bars it lasts.
    Tempo,
    /// The Synth tab engine an Instrument pad plays a voice of.
    Engine,
    /// The note an Instrument or MIDI out pad plays.
    Note,
    /// The port a MIDI out pad sends to.
    Port,
    /// The channel a MIDI out pad sends on, 1 to 16.
    Channel,
}

impl Row {
    pub const ALL: [Row; 8] = [
        Row::Sound,
        Row::Mode,
        Row::Polyphony,
        Row::MuteGroup,
        Row::Level,
        Row::Pan,
        Row::Tune,
        Row::Effects,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Row::Sound => "Sound",
            Row::Mode => "Mode",
            Row::Polyphony => "Polyphony",
            Row::MuteGroup => "Mute group",
            Row::Level => "Level",
            Row::Pan => "Pan",
            Row::Tune => "Tune",
            Row::Effects => "Effects",
            Row::Tempo => "Tempo",
            Row::Engine => "Engine",
            Row::Note => "Note",
            Row::Port => "Port",
            Row::Channel => "Channel",
        }
    }

    /// The Play tab's row for the same setting.
    fn play_row(self) -> Option<play_tab::Row> {
        match self {
            Row::Polyphony => Some(play_tab::Row::Polyphony),
            Row::MuteGroup => Some(play_tab::Row::MuteGroup),
            Row::Level => Some(play_tab::Row::Level),
            Row::Pan => Some(play_tab::Row::Pan),
            Row::Tune => Some(play_tab::Row::Tune),
            Row::Sound
            | Row::Mode
            | Row::Effects
            | Row::Tempo
            | Row::Engine
            | Row::Note
            | Row::Port
            | Row::Channel => None,
        }
    }
}

/// The rows the Q-Links turn on a Sample pad, in knob order.
pub const QLINK_ROWS: [Row; 4] = [Row::Level, Row::Pan, Row::Tune, Row::Polyphony];

/// What a kit pad is, as the type switch shows it (the Play, Pads and
/// Sequencer redesign's step 6, board 6). Loop isn't a kind of its own in
/// the model: it is a Sample pad set up by [`as_loop`], told apart by its
/// `on_launch`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PadType {
    Sample,
    Loop,
    Instrument,
    MidiOut,
}

impl PadType {
    /// In the switch's order.
    pub const ALL: [PadType; 4] = [
        PadType::Sample,
        PadType::Loop,
        PadType::Instrument,
        PadType::MidiOut,
    ];

    /// What `p` is.
    pub fn of(p: &Pad) -> PadType {
        match &p.kind {
            PadKind::Instrument { .. } => PadType::Instrument,
            PadKind::MidiOut { .. } => PadType::MidiOut,
            PadKind::Sample if p.on_launch => PadType::Loop,
            PadKind::Sample => PadType::Sample,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            PadType::Sample => "Sample",
            PadType::Loop => "Loop",
            PadType::Instrument => "Instrument",
            PadType::MidiOut => "MIDI out",
        }
    }

    /// The switch's line under the type's name.
    fn sub(self) -> &'static str {
        match self {
            PadType::Sample => "its sound",
            PadType::Loop => "in time",
            PadType::Instrument => "a synth voice",
            PadType::MidiOut => "to your gear",
        }
    }

    /// The Pad list's rows for a pad of this type, in order.
    pub fn rows(self) -> &'static [Row] {
        match self {
            PadType::Sample => &Row::ALL,
            PadType::Loop => &[
                Row::Sound,
                Row::Tempo,
                Row::MuteGroup,
                Row::Level,
                Row::Pan,
                Row::Tune,
                Row::Effects,
            ],
            PadType::Instrument => &[Row::Engine, Row::Note],
            PadType::MidiOut => &[Row::Port, Row::Channel, Row::Note],
        }
    }

    /// The rows Q-Links 1 to 4 turn on a pad of this type.
    pub fn qlinks(self) -> [Option<Row>; 4] {
        match self {
            PadType::Sample => QLINK_ROWS.map(Some),
            PadType::Loop => [
                Some(Row::Level),
                Some(Row::Pan),
                Some(Row::Tune),
                Some(Row::Tempo),
            ],
            PadType::Instrument => [Some(Row::Note), Some(Row::Engine), None, None],
            PadType::MidiOut => [Some(Row::Note), Some(Row::Channel), Some(Row::Port), None],
        }
    }

    /// Row `row`'s place in this type's list, if it has it.
    fn index_of(self, row: Row) -> Option<usize> {
        self.rows().iter().position(|r| *r == row)
    }
}

/// A MIDI output a MIDI out pad can send to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutPort {
    /// The port's own name, as a pad saves it.
    pub name: String,
    /// The name it shows by (the musician's, if they named it).
    pub shown: String,
    /// Why a note sent to it would go nowhere now ("sends nothing: Volca
    /// Kick is not plugged in"), or None.
    pub why: Option<String>,
    /// The kit pads its device profile gives a note (`pad.A1 = 36 ch 10`),
    /// pad 0 (A1) up, as [`mpfree_midi::profile::Profile::pad`] says.
    pub pads: Vec<(u8, PadOut)>,
}

/// The channel (1-16) and note a new MIDI out pad on kit pad `pad` sends
/// on `port`: what the port's profile has the pad play, as a kit on that
/// port would (its channel, else 10), else the pad's own note,
/// [`KIT_NOTES`] (A1 is 37, as the MPC sends it) on channel 10.
fn first_out(port: Option<&OutPort>, pad: usize) -> (u8, u8) {
    let profile = port.and_then(|o| o.pads.iter().find(|(p, _)| usize::from(*p) == pad));
    match profile {
        Some((_, o)) => (o.channel.map_or(FIRST_CHANNEL, |c| c + 1), o.note),
        None => (FIRST_CHANNEL, KIT_NOTES[pad % KIT_NOTES.len()]),
    }
}

/// The channel a new MIDI out pad sends on (drums, on most gear).
const FIRST_CHANNEL: u8 = 10;

/// The note a new Instrument pad plays (C4).
const FIRST_KEY: u8 = 60;

/// `p` set up as a loop (the type switch's Loop): latched (a hit starts
/// it, the next stops it), every layer looping forward over its whole
/// sound, warped (at `bpm`, the project's tempo, where it had no tempo of
/// its own) so it plays in time, and started on the launch line
/// (`on_launch`). It plays its sound ([`PadKind::Sample`]).
pub fn as_loop(p: &Pad, bpm: f64) -> Pad {
    let mut p = p.clone();
    p.kind = PadKind::Sample;
    p.latch = true;
    p.on_launch = true;
    for l in &mut p.layers {
        let r = &mut l.region;
        r.mode = PlayMode::Loop;
        r.sustain_loop = Some(Loop {
            start: r.start,
            end: r.end,
            crossfade: 0,
            mode: LoopMode::Forward,
        });
        if r.warp.is_none() {
            r.warp = Some(Warp::new(
                (bpm as f32).clamp(*WARP_BPM.start(), *WARP_BPM.end()),
            ));
        }
    }
    p
}

/// A pad with no sound, named `name`, for a type that needs none.
fn blank_pad(name: &str) -> Pad {
    let mut p = Pad::new(name, Layer::new(sound_region(None, SourceId(0), 0)));
    p.layers.clear();
    p
}

const MODES: [PlayMode; 5] = [
    PlayMode::OneShot,
    PlayMode::Gate,
    PlayMode::Toggle,
    PlayMode::Loop,
    PlayMode::Granular,
];

/// The buttons under the map, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Copy,
    Paste,
    Clear,
    MapToKeys,
}

impl Button {
    pub const ALL: [Button; 4] = [
        Button::Copy,
        Button::Paste,
        Button::Clear,
        Button::MapToKeys,
    ];
}

/// A sound in the kit.
#[derive(Clone, Debug, PartialEq)]
pub struct Sound {
    pub id: SourceId,
    pub name: String,
    pub frames: u64,
    pub rate: u32,
    /// The pads (of the whole project) that play it.
    pub pads: Vec<usize>,
}

/// Everything the tab shows: the bank of the pad picked, and the kit's
/// sounds.
#[derive(Clone, Debug)]
pub struct PadsState {
    /// The pad picked.
    pub pad: usize,
    /// The bank's first pad.
    pub first: usize,
    /// The bank's pads, None where a pad has no sound.
    pub pads: Vec<Option<Pad>>,
    pub sounds: Vec<Sound>,
    /// The clip each pad of the bank launches, if any (none yet: the time
    /// engine doesn't launch clips from pads).
    pub clips: Vec<Option<String>>,
    /// The effects on the picked pad's mixer strip, in order: none for a
    /// pad with no strip of its own yet (its + FX lends it one). None
    /// until the app has heard from the mixer.
    pub effects: Option<Vec<String>>,
    /// Each of the kit's banks A to H: its pads in use, pad 1 of the
    /// bank the lowest bit.
    pub banks: Vec<u16>,
    /// The project's tempo: a pad made a loop with no tempo of its own is
    /// warped to it.
    pub bpm: f64,
    /// The Synth tab's engines an Instrument pad can play, by name ("Poly",
    /// "FM"): its saved id is the name in small letters. The app fills it.
    pub engines: Vec<String>,
    /// The MIDI outputs a MIDI out pad can send to. The app fills it.
    pub ports: Vec<OutPort>,
}

/// Pad `p` plays a sound of its own: what Map to keys maps.
fn sounding(p: &Pad) -> bool {
    !p.layers.is_empty() && p.kind.sounds()
}

/// Pad `p` is in use: it has a sound, or plays something else (an engine's
/// voice, a note out to gear).
pub fn in_use(p: &Pad) -> bool {
    !p.layers.is_empty() || !p.kind.sounds()
}

impl PadsState {
    /// The bank of pad `pad` in `p`, and its sounds.
    pub fn from_project(p: &Project, pad: usize) -> PadsState {
        let bank = bank_of(pad);
        let pads = bank
            .clone()
            .map(|i| p.pad(i).filter(|pd| in_use(pd)).cloned())
            .collect();
        let sounds = p
            .sources()
            .iter()
            .map(|s| Sound {
                id: s.id(),
                name: s.name.clone(),
                frames: s.frames(),
                rate: s.rate(),
                pads: p.users_of(s.id()),
            })
            .collect();
        let banks = (0..PADS.div_ceil(BANK))
            .map(|b| {
                (0..BANK).fold(0u16, |m, i| {
                    let on = p.pad(b * BANK + i).is_some_and(in_use);
                    m | (u16::from(on) << i)
                })
            })
            .collect();
        PadsState {
            pad,
            first: bank.start,
            pads,
            sounds,
            clips: vec![None; bank.len()],
            effects: None,
            banks,
            bpm: 120.0,
            engines: Vec::new(),
            ports: Vec::new(),
        }
    }

    /// What the pad picked is (an empty pad is a Sample pad with no sound).
    pub fn picked_type(&self) -> PadType {
        self.picked().map_or(PadType::Sample, PadType::of)
    }

    /// The pad picked, if it is in use ([`in_use`]).
    pub fn picked(&self) -> Option<&Pad> {
        self.pads
            .get(self.pad - self.first)
            .and_then(Option::as_ref)
    }

    /// The sound on the pad picked's first layer, as an index into `sounds`.
    pub fn sound_of_picked(&self) -> Option<usize> {
        let id = self.picked()?.layers.first()?.region.source;
        self.sounds.iter().position(|s| s.id == id)
    }
}

/// What a touch or control on the tab asks for. The app applies it.
#[derive(Clone, Debug, PartialEq)]
pub enum PadsAction {
    /// Pad `pad` picked: every tab of the editor edits it from now on.
    Pick(usize),
    /// Play pad `pad` (down) or let it go, as a hit would.
    Audition { pad: usize, down: bool },
    /// Pad `pad` set to `after` (None clears it). `gesture` merges a run of
    /// changes into one undo step; None is a step of its own.
    Set {
        pad: usize,
        after: Option<Box<Pad>>,
        gesture: Option<u64>,
    },
    /// Map to keys: the app makes `pads` (the bank's pads that play a sound) a
    /// keygroup instrument at their sounds' notes
    /// ([`mpfree_sampler::keymap`], `Project::map_to_keys`).
    MapToKeys { pads: Vec<usize> },
    /// The bar's Browse for button: the app opens its browser, whose Load
    /// puts a sound on the pad picked.
    Browse,
    /// A tap on bank `b` (0 to 7, A to H) of the strip over the pads: the
    /// app puts that bank on the pads, as PAD BANK does.
    Bank(usize),
    /// The Effects row: open pad `pad`'s chain in the Mix tab.
    Effects(usize),
}

impl PadsAction {
    /// Applies [`PadsAction::Set`] to the project; other actions change
    /// nothing here.
    pub fn apply(&self, p: &mut Project) -> Result<(), ModelError> {
        match self {
            PadsAction::Set {
                pad,
                after,
                gesture,
            } => p.set_pad(*pad, after.as_deref().cloned(), *gesture),
            _ => Ok(()),
        }
    }
}

/// `pad` (or a new pad) playing `sound` from start to end on its first
/// layer. Its other settings, and the old sound's gain, tune, channels and
/// play mode, stay; it takes the sound's name. A pad of another kind (an
/// instrument's, a MIDI out pad) becomes a Sample pad: the sound is there
/// to be heard, as `Pad::replacing` has it.
pub fn with_sound(pad: Option<&Pad>, sound: &Sound) -> Pad {
    match pad {
        Some(p) => {
            let mut p = p.clone();
            match p.layers.first_mut() {
                Some(l) => l.region = sound_region(Some(&l.region), sound.id, sound.frames),
                None => p
                    .layers
                    .push(Layer::new(sound_region(None, sound.id, sound.frames))),
            }
            p.name = sound.name.clone();
            p.kind = PadKind::Sample;
            p
        }
        None => Pad::new(
            &sound.name,
            Layer::new(sound_region(None, sound.id, sound.frames)),
        ),
    }
}

/// Source `id`, `frames` long, from start to end, in place of `old`: the
/// old region's gain, tune, channels, play mode and grain settings stay,
/// its fades and loop don't (a loop is made afresh where Loop needs one).
pub fn sound_region(old: Option<&Region>, id: SourceId, frames: u64) -> Region {
    let mut r = match old {
        Some(o) => Region {
            source: id,
            start: 0,
            end: frames,
            sustain_loop: None,
            // Another sound's slices don't fit this one.
            slices: Vec::new(),
            slice_sets: Vec::new(),
            ..o.clone()
        },
        None => Region {
            source: id,
            start: 0,
            end: frames,
            sustain_loop: None,
            fade_in: Fade::NONE,
            fade_out: Fade::NONE,
            gain_db: 0.0,
            reverse: false,
            channels: ChannelPick::Stereo,
            tune_cents: 0,
            mode: PlayMode::OneShot,
            grain: GrainParams::default(),
            grain_mode: GrainMode::Cloud,
            grain_keys: GrainKeys::default(),
            grain_glide_ms: 0.0,
            warp: None,
            slices: Vec::new(),
            slice_sets: Vec::new(),
        },
    };
    (r.fade_in, r.fade_out) = (Fade::NONE, Fade::NONE);
    if r.mode == PlayMode::Loop {
        r.sustain_loop = Some(default_loop(&r));
    }
    r
}

/// `p` with every layer in play mode `mode` (a loop added where Loop needs
/// one).
fn with_mode(p: &Pad, mode: PlayMode) -> Pad {
    let mut p = p.clone();
    for l in &mut p.layers {
        l.region.mode = mode;
        if mode == PlayMode::Loop && l.region.sustain_loop.is_none() {
            l.region.sustain_loop = Some(default_loop(&l.region));
        }
    }
    p
}

/// Which edit an undo step belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Key {
    Row(Row),
}

/// Steps `i` of `n` by `delta`, round and round.
fn wrap(i: usize, delta: i32, n: usize) -> usize {
    (i as i32 + delta).rem_euclid(n.max(1) as i32) as usize
}

/// The Pads tab's widgets and what their touches mean. It keeps the pad
/// copied, the sound picked and the undo step being made.
pub struct PadsTab {
    /// The page name the tab's layout is added under.
    pub name: String,
    /// The bank letter shown in pad names, as [`crate::wave_tab::WaveTab::bank`].
    pub bank: Option<char>,
    /// A kit name for the map's title, such as "Kit 1 · Breaks".
    pub kit: String,
    /// The kit's name on the bar's Kit button ("Boom Bap").
    pub kit_name: String,
    copied: Option<(usize, Pad)>,
    sound: usize,
    shown_pad: Option<usize>,
    gesture: Option<(Key, u64)>,
    next_gesture: u64,
    /// What the last touch did or why it did nothing, for the hint line.
    note: Option<String>,
}

impl PadsTab {
    /// Adds the widgets and the page `name` to `ui` (the app adds `header`
    /// and `hint` first) and shows `st`.
    pub fn add(ui: &mut Ui, name: &str, st: &PadsState) -> Result<PadsTab, String> {
        let mut bar = PadBar::new(&[]);
        bar.button = Some("Browse".into());
        ui.add(BAR, bar);
        ui.add(KITS, KitTracks::new());
        ui.add(MAP, PadMap::new("Pads"));
        ui.add(
            BUTTONS,
            ButtonRow::new(&["Copy", "Paste", "Clear", "Map to keys"]),
        );
        ui.add(TYPE, SectionChips::new(&PadType::ALL.map(PadType::label)));
        ui.add(LIST, SettingList::new("Pad"));
        let mut sounds = ListView::new("Sounds in the kit");
        // A tap picks the sound for Put on; it loads nothing itself.
        sounds.picks_only = true;
        ui.add(SOUNDS, sounds);
        let mut put = ButtonRow::new(&["Put on pad"]);
        put.primary = Some(0);
        ui.add(PUT, put);
        ui.add_page(name, LAYOUT)?;
        let mut tab = PadsTab {
            name: name.to_string(),
            bank: None,
            kit: String::new(),
            kit_name: String::new(),
            copied: None,
            sound: 0,
            shown_pad: None,
            gesture: None,
            next_gesture: GESTURE_BASE,
            note: None,
        };
        tab.update(ui, st);
        Ok(tab)
    }

    /// The row picked in the Pad list of a pad of type `t`.
    pub fn row_of(&self, ui: &Ui, t: PadType) -> Row {
        let rows = t.rows();
        ui.get::<SettingList>(LIST)
            .and_then(|l| rows.get(l.selected).copied())
            .unwrap_or(rows[0])
    }

    /// The row picked in the Pad list of a Sample pad.
    pub fn row(&self, ui: &Ui) -> Row {
        self.row_of(ui, PadType::Sample)
    }

    /// What Q-Links 1 to 4 turn on the pad picked, for the hint line
    /// ("level, pan, tune, voices").
    pub fn qlinks_text(st: &PadsState) -> String {
        let t = st.picked_type();
        let names: Vec<String> = t
            .qlinks()
            .iter()
            .flatten()
            .map(|r| match r {
                Row::Polyphony => "voices".to_string(),
                r => r.label().to_lowercase(),
            })
            .collect();
        names.join(", ")
    }

    /// The sound picked in the sounds list.
    pub fn sound(&self) -> usize {
        self.sound
    }

    /// Ends the turn or run of taps being made: the next edit is a new
    /// undo step.
    pub fn lift(&mut self) {
        self.gesture = None;
    }

    /// Pad `pad` (of the project) is held down, or let go.
    pub fn set_lit(&self, ui: &mut Ui, st: &PadsState, pad: usize, down: bool) {
        let Some(i) = pad.checked_sub(st.first).filter(|i| *i < st.pads.len()) else {
            return;
        };
        if let Some(m) = ui.get_mut::<PadMap>(MAP) {
            m.lit.resize(st.pads.len(), false);
            m.lit[i] = down;
        }
    }

    /// Shows `st`.
    pub fn update(&mut self, ui: &mut Ui, st: &PadsState) {
        if self.shown_pad != Some(st.pad) {
            self.shown_pad = Some(st.pad);
            if let Some(s) = st.sound_of_picked() {
                self.sound = s;
            }
        }
        self.sound = self.sound.min(st.sounds.len().saturating_sub(1));
        self.show(ui, st);
    }

    fn show(&self, ui: &mut Ui, st: &PadsState) {
        let here = pad_text(self.bank, st.pad);
        let picked = st.picked();
        if let Some(bar) = ui.get_mut::<PadBar>(BAR) {
            bar.color = pad_color(st.pad);
            bar.button = Some(format!("Browse for {here}"));
            bar.kit = (!self.kit_name.is_empty()).then(|| self.kit_name.clone());
            bar.title = match picked {
                Some(p) => format!("Pad {here} · {}", p.name),
                None => format!("Pad {here} · no sound"),
            };
            let used = st.pads.iter().filter(|p| p.is_some()).count();
            let n = st.sounds.len();
            bar.subtitle = format!(
                "{used} of {} pads in use · {n} {} in the kit",
                st.pads.len(),
                if n == 1 { "sound" } else { "sounds" }
            );
        }
        let cells: Vec<PadCell> = st
            .pads
            .iter()
            .enumerate()
            .map(|(i, p)| PadCell {
                label: pad_text(self.bank, st.first + i),
                name: p.as_ref().map_or_else(String::new, |p| p.name.clone()),
                detail: p.as_ref().map_or_else(String::new, |p| pad_detail(p, st)),
                clip: st.clips.get(i).cloned().flatten(),
                color: pad_color(st.first + i),
                badge: p.as_ref().and_then(type_tag).map(str::to_string),
            })
            .collect();
        let kit = self.kit.clone();
        if let Some(m) = ui.get_mut::<PadMap>(MAP) {
            m.title = if kit.is_empty() { "Pads".into() } else { kit };
            // The bank the grid shows: the one given, or the page of its
            // first pad (A1 to H16).
            let bank = self
                .bank
                .unwrap_or((b'A' + (st.first / 16 % 26) as u8) as char);
            m.right = format!("bank {bank}");
            m.lit.resize(cells.len(), false);
            m.cells = cells;
            m.selected = st.pad - st.first;
            m.banks.clone_from(&st.banks);
            m.bank = st.first / BANK;
        }
        let paste = self
            .copied
            .as_ref()
            .map(|(from, _)| format!("Paste {}", pad_text(self.bank, *from)));
        if let Some(b) = ui.get_mut::<ButtonRow>(BUTTONS) {
            b.labels[1] = paste.unwrap_or_else(|| "Paste".into());
            let sounds = st.pads.iter().flatten().any(sounding);
            b.enabled = vec![
                picked.is_some(),
                self.copied.is_some(),
                picked.is_some(),
                sounds,
            ];
            let here = pad_text(self.bank, st.pad);
            b.why = vec![
                format!("{here} has no sound to copy"),
                "Copy a pad first".to_string(),
                format!("{here} has no sound to clear"),
                "No pad in this bank has a sound".to_string(),
            ];
        }
        let kind = st.picked_type();
        if let Some(c) = ui.get_mut::<SectionChips>(TYPE) {
            c.chips = PadType::ALL
                .iter()
                .map(|t| SectionChip {
                    name: t.label().to_string(),
                    sub: t.sub().to_string(),
                })
                .collect();
            c.selected = PadType::ALL.iter().position(|t| *t == kind).unwrap_or(0);
        }
        let rows = kind
            .rows()
            .iter()
            .map(|r| Setting {
                name: r.label().into(),
                value: row_text(picked, st, *r),
            })
            .collect();
        let clip = st.clips.get(st.pad - st.first).cloned().flatten();
        let footer = picked.and_then(|p| footer(p, st));
        if let Some(list) = ui.get_mut::<SettingList>(LIST) {
            list.title = format!("Pad {here}");
            list.right = format!("clip: {}", clip.as_deref().unwrap_or("none"));
            list.rows = rows;
            list.selected = list.selected.min(kind.rows().len() - 1);
            list.buttons = match kind.index_of(Row::Effects) {
                Some(k) if st.effects.is_some() => vec![k],
                _ => Vec::new(),
            };
            list.footer = footer;
        }
        let items = st
            .sounds
            .iter()
            .map(|s| ListItem {
                text: s.name.clone(),
                detail: match s.pads.len() {
                    0 => format!("{} · unused", duration_text(s.frames, s.rate)),
                    1 => format!(
                        "{} · {}",
                        duration_text(s.frames, s.rate),
                        pad_text(self.bank, s.pads[0])
                    ),
                    n => format!("{} · {n} pads", duration_text(s.frames, s.rate)),
                },
                color: None,
            })
            .collect();
        if let Some(l) = ui.get_mut::<ListView>(SOUNDS) {
            l.items = items;
            l.selected = self.sound;
        }
        if let Some(b) = ui.get_mut::<ButtonRow>(PUT) {
            b.labels[0] = format!("Put on {here}");
            b.enabled = vec![!st.sounds.is_empty()];
            b.why = vec!["The kit has no sounds yet: Browse for one".to_string()];
        }
    }

    /// What a touch on one of the tab's widgets asks for; None for touches
    /// that only change what the tab shows.
    pub fn action(&mut self, ui: &mut Ui, ev: &UiEvent, st: &PadsState) -> Option<PadsAction> {
        match (ev.widget.as_str(), &ev.kind) {
            (BAR, EventKind::Tab(0)) => Some(PadsAction::Browse),
            (MAP, EventKind::Select(i)) => Some(PadsAction::Pick(st.first + *i)),
            (MAP, EventKind::Tab(b)) if *b < st.banks.len() => Some(PadsAction::Bank(*b)),
            (MAP, EventKind::Pad { index, down }) => Some(PadsAction::Audition {
                pad: st.first + *index,
                down: *down,
            }),
            (BUTTONS, EventKind::Tab(i)) => self.press(ui, st, *Button::ALL.get(*i)?),
            (TYPE, EventKind::Tab(i)) => self.set_type(st, *PadType::ALL.get(*i)?),
            (LIST, EventKind::Step { index, delta }) => {
                self.step(st, *st.picked_type().rows().get(*index)?, *delta)
            }
            (LIST, EventKind::Tab(k)) if st.picked_type().rows().get(*k) == Some(&Row::Effects) => {
                st.effects.as_ref()?;
                Some(PadsAction::Effects(st.pad))
            }
            (SOUNDS, EventKind::Select(k)) => {
                self.sound = *k;
                None
            }
            (PUT, EventKind::Tab(0)) => {
                let sound = st.sounds.get(self.sound)?;
                let after = with_sound(st.picked(), sound);
                if st.picked() == Some(&after) {
                    // Say so rather than do nothing (Jay, 13 Nov).
                    self.note = Some(format!(
                        "{} plays all of {} already",
                        pad_text(self.bank, st.pad),
                        sound.name
                    ));
                    return None;
                }
                self.lift();
                // Nothing else on the tab shows it: the rows stay the same.
                self.note = Some(format!(
                    "{} plays all of {} now",
                    pad_text(self.bank, st.pad),
                    sound.name
                ));
                Some(PadsAction::Set {
                    pad: st.pad,
                    after: Some(Box::new(after)),
                    gesture: None,
                })
            }
            _ => None,
        }
    }

    /// The type switch: makes the pad picked a pad of type `to`, one undo
    /// step. It keeps its sound (and its name, unless it had none), so
    /// going back loses nothing; an instrument or MIDI out pad with no
    /// sound made a Sample pad again is cleared. Loop needs a sound.
    pub fn set_type(&mut self, st: &PadsState, to: PadType) -> Option<PadsAction> {
        let here = pad_text(self.bank, st.pad);
        let picked = st.picked();
        if st.picked_type() == to {
            return None;
        }
        let after = match to {
            PadType::Sample => {
                let p = picked?;
                (!p.layers.is_empty()).then(|| {
                    let mut p = p.clone();
                    p.kind = PadKind::Sample;
                    // Not a loop any more: hits play at once.
                    p.on_launch = false;
                    p
                })
            }
            PadType::Loop => {
                let Some(p) = picked.filter(|p| !p.layers.is_empty()) else {
                    self.note = Some(format!("{here} has no sound to loop: Browse for one"));
                    return None;
                };
                Some(as_loop(p, st.bpm))
            }
            PadType::Instrument => {
                let engine = st
                    .engines
                    .first()
                    .map_or_else(|| "poly".to_string(), |e| e.to_ascii_lowercase());
                let name = st.engines.first().map_or("Synth", String::as_str);
                let mut p = picked.cloned().unwrap_or_else(|| blank_pad(name));
                p.kind = PadKind::Instrument {
                    engine,
                    note: FIRST_KEY,
                };
                p.on_launch = false;
                Some(p)
            }
            PadType::MidiOut => {
                let port = st.ports.first();
                let name = port.map_or("MIDI out", |o| o.shown.as_str());
                let mut p = picked.cloned().unwrap_or_else(|| blank_pad(name));
                let (channel, note) = first_out(port, st.pad);
                p.kind = PadKind::MidiOut {
                    port: port.map_or_else(String::new, |o| o.name.clone()),
                    channel,
                    note,
                };
                p.on_launch = false;
                Some(p)
            }
        };
        self.lift();
        self.note = Some(match &after {
            Some(_) => format!("{here} is a {} pad now", to.label()),
            None => format!("{here} is empty now: it had no sound"),
        });
        Some(PadsAction::Set {
            pad: st.pad,
            after: after.map(Box::new),
            gesture: None,
        })
    }

    /// What the last touch did, or why it did nothing, when nothing else
    /// on the tab shows it; once.
    pub fn take_note(&mut self) -> Option<String> {
        self.note.take()
    }

    /// A button pressed (as a tap on it would).
    pub fn press(&mut self, ui: &mut Ui, st: &PadsState, b: Button) -> Option<PadsAction> {
        self.lift();
        match b {
            Button::Copy => {
                self.copied = Some((st.pad, st.picked()?.clone()));
                self.show(ui, st);
                None
            }
            Button::Paste => {
                let (_, p) = self.copied.as_ref()?;
                (st.picked() != Some(p)).then(|| PadsAction::Set {
                    pad: st.pad,
                    after: Some(Box::new(p.clone())),
                    gesture: None,
                })
            }
            Button::Clear => {
                st.picked()?;
                Some(PadsAction::Set {
                    pad: st.pad,
                    after: None,
                    gesture: None,
                })
            }
            Button::MapToKeys => {
                let pads: Vec<usize> = (0..st.pads.len())
                    .filter(|&k| st.pads[k].as_ref().is_some_and(sounding))
                    .map(|k| st.first + k)
                    .collect();
                (!pads.is_empty()).then_some(PadsAction::MapToKeys { pads })
            }
        }
    }

    /// Q-Link `knob` (0 to 3: level, pan, tune, voices) turned by `delta`
    /// detents.
    pub fn qlink(
        &mut self,
        ui: &mut Ui,
        st: &PadsState,
        knob: usize,
        delta: i32,
    ) -> Option<PadsAction> {
        let t = st.picked_type();
        let row = (*t.qlinks().get(knob)?)?;
        if let (Some(list), Some(k)) = (ui.get_mut::<SettingList>(LIST), t.index_of(row)) {
            list.selected = k;
        }
        self.step(st, row, delta)
    }

    /// The data wheel: steps the row picked.
    pub fn wheel(&mut self, ui: &mut Ui, st: &PadsState, delta: i32) -> Option<PadsAction> {
        let row = self.row_of(ui, st.picked_type());
        self.step(st, row, delta)
    }

    fn step(&mut self, st: &PadsState, row: Row, delta: i32) -> Option<PadsAction> {
        if delta == 0 {
            return None;
        }
        let picked = st.picked();
        let after = match row {
            Row::Sound => {
                if st.sounds.is_empty() {
                    return None;
                }
                let n = st.sounds.len() as i32;
                let k = match st.sound_of_picked() {
                    Some(k) => (k as i32 + delta).rem_euclid(n),
                    None if delta > 0 => 0,
                    None => n - 1,
                };
                if st.sound_of_picked() == Some(k as usize) {
                    // The kit's only sound: stepping round to it again
                    // would put it on whole, quietly dropping the pad's
                    // trim and loop. Say so instead (Jay, 13 Nov).
                    self.note = Some(format!(
                        "{} plays {}, the kit's only sound: Browse for more",
                        pad_text(self.bank, st.pad),
                        st.sounds[k as usize].name
                    ));
                    return None;
                }
                self.sound = k as usize;
                with_sound(picked, &st.sounds[k as usize])
            }
            Row::Mode => {
                let p = picked?;
                let now = p.layers.first()?.region.mode;
                let i = MODES.iter().position(|m| *m == now).unwrap_or(0) as i32;
                with_mode(
                    p,
                    MODES[(i + delta).rem_euclid(MODES.len() as i32) as usize],
                )
            }
            Row::Tempo => {
                let mut p = picked?.clone();
                for l in &mut p.layers {
                    let r = &mut l.region;
                    let Some(w) = r.warp.as_mut() else {
                        continue;
                    };
                    let bpm = (f64::from(w.bpm) * 10.0).round() + f64::from(delta);
                    w.bpm = ((bpm / 10.0) as f32).clamp(*WARP_BPM.start(), *WARP_BPM.end());
                    if let Some(rate) = st.sounds.iter().find(|s| s.id == r.source).map(|s| s.rate)
                    {
                        w.fit(r.start, r.end, rate);
                    }
                }
                p
            }
            Row::Engine => {
                let p = picked?;
                let PadKind::Instrument { engine, note } = &p.kind else {
                    return None;
                };
                if st.engines.is_empty() {
                    return None;
                }
                let at = st
                    .engines
                    .iter()
                    .position(|e| e.eq_ignore_ascii_case(engine));
                let k = match at {
                    Some(i) => wrap(i, delta, st.engines.len()),
                    None => 0,
                };
                let mut p = p.clone();
                p.kind = PadKind::Instrument {
                    engine: st.engines[k].to_ascii_lowercase(),
                    note: *note,
                };
                p
            }
            Row::Note => {
                let mut p = picked?.clone();
                match &mut p.kind {
                    PadKind::Instrument { note, .. } | PadKind::MidiOut { note, .. } => {
                        *note = (i32::from(*note) + delta).clamp(0, 127) as u8;
                    }
                    PadKind::Sample => return None,
                }
                p
            }
            Row::Channel => {
                let mut p = picked?.clone();
                let PadKind::MidiOut { channel, .. } = &mut p.kind else {
                    return None;
                };
                *channel = (i32::from(*channel) + delta).clamp(1, 16) as u8;
                p
            }
            Row::Port => {
                let mut p = picked?.clone();
                let PadKind::MidiOut { port, .. } = &mut p.kind else {
                    return None;
                };
                if st.ports.is_empty() {
                    self.note = Some("No MIDI output to pick: plug one in".to_string());
                    return None;
                }
                let k = match st.ports.iter().position(|o| o.name == *port) {
                    Some(i) => wrap(i, delta, st.ports.len()),
                    None => 0,
                };
                *port = st.ports[k].name.clone();
                p
            }
            r => stepped(picked?, r.play_row()?, delta, false),
        };
        if Some(&after) == picked {
            return None;
        }
        let gesture = match self.gesture {
            Some((k, g)) if k == Key::Row(row) => g,
            _ => {
                let g = self.next_gesture;
                self.next_gesture += 1;
                self.gesture = Some((Key::Row(row), g));
                g
            }
        };
        Some(PadsAction::Set {
            pad: st.pad,
            after: Some(Box::new(after)),
            gesture: Some(gesture),
        })
    }
}

/// "3 layers · Loop", for the map; a sliced pad's slices first ("16
/// slices · One shot"). A pad of another type says what it plays: "2
/// bars" (a loop), "Poly C1" (an instrument), "ch 10 · 36" (MIDI out).
fn pad_detail(p: &Pad, st: &PadsState) -> String {
    match (&p.kind, PadType::of(p)) {
        (PadKind::Instrument { engine, note }, _) => {
            return format!("{} {}", engine_name(st, engine), note_name(*note));
        }
        (PadKind::MidiOut { channel, note, .. }, _) => {
            return format!("ch {channel} · {note}");
        }
        (_, PadType::Loop) => {
            if let Some(len) = loop_length(p, st) {
                return len;
            }
        }
        _ => {}
    }
    let mode = p.layers.first().map_or("", |l| mode_label(l.region.mode));
    let mode = match crate::play_tab::slices(p) {
        0 | 1 => mode.to_string(),
        n => format!("{n} slices · {mode}"),
    };
    match p.layers.len() {
        1 => mode,
        n => format!("{n} layers · {mode}"),
    }
}

/// The map's word for a pad of type `t`: none for a plain sample.
fn badge(t: PadType) -> Option<&'static str> {
    match t {
        PadType::Sample => None,
        PadType::Loop => Some("LOOP"),
        PadType::Instrument => Some("SYNTH"),
        PadType::MidiOut => Some("MIDI"),
    }
}

/// What a change of pad type, or of a type's own settings, did, for the
/// app's log: "type Sample -> MIDI out (Volca Kick, ch 10, note 49)",
/// "note 49 -> 50", "tempo 92.0 -> 92.1 BPM". Empty when neither changed.
pub fn describe_type(before: &Pad, after: &Pad) -> String {
    let (b, a) = (PadType::of(before), PadType::of(after));
    if b != a {
        return match kind_text(&after.kind) {
            Some(k) => format!("type {} -> {} ({k})", b.label(), a.label()),
            None => format!("type {} -> {}", b.label(), a.label()),
        };
    }
    let mut out = Vec::new();
    let mut moved = |what: &str, x: String, y: String| {
        if x != y {
            out.push(format!("{what} {x} -> {y}"));
        }
    };
    match (&before.kind, &after.kind) {
        (
            PadKind::Instrument { engine, note },
            PadKind::Instrument {
                engine: engine_to,
                note: note_to,
            },
        ) => {
            moved("engine", engine.clone(), engine_to.clone());
            moved("note", note_name(*note), note_name(*note_to));
        }
        (
            PadKind::MidiOut {
                port,
                channel,
                note,
            },
            PadKind::MidiOut {
                port: port_to,
                channel: channel_to,
                note: note_to,
            },
        ) => {
            moved("port", port.clone(), port_to.clone());
            moved("channel", channel.to_string(), channel_to.to_string());
            moved("note", note.to_string(), note_to.to_string());
        }
        _ => {}
    }
    if a == PadType::Loop {
        let bpm = |p: &Pad| {
            p.layers
                .first()
                .and_then(|l| l.region.warp.as_ref())
                .map(|w| format!("{:.1}", w.bpm))
        };
        if let (Some(x), Some(y)) = (bpm(before), bpm(after)) {
            if x != y {
                out.push(format!("tempo {x} -> {y} BPM"));
            }
        }
    }
    out.join(", ")
}

/// A pad's kind in a few words for the log ("poly at C4", "Volca Kick,
/// ch 10, note 49"), or None for a sample.
pub fn kind_text(k: &PadKind) -> Option<String> {
    match k {
        PadKind::Sample => None,
        PadKind::Instrument { engine, note } => Some(format!("{engine} at {}", note_name(*note))),
        PadKind::MidiOut {
            port,
            channel,
            note,
        } => Some(format!("{port}, ch {channel}, note {note}")),
    }
}

/// The small tag that names pad `p`'s type where it shows ("LOOP",
/// "SYNTH", "MIDI"): on its cell here and on Play's rows. None for a
/// sample, the type a pad has unless told otherwise.
pub fn type_tag(p: &Pad) -> Option<&'static str> {
    badge(PadType::of(p))
}

/// Engine `id`'s name among `st.engines` ("poly" is "Poly"), or the id as
/// saved when it isn't one of them.
fn engine_name<'a>(st: &'a PadsState, id: &'a str) -> &'a str {
    st.engines
        .iter()
        .find(|e| e.eq_ignore_ascii_case(id))
        .map_or(id, String::as_str)
}

/// How long a loop pad's first layer lasts at its tempo: "2 bars", "3
/// beats", "1.50 bars"; None when it has no tempo or its sound isn't
/// known.
fn loop_length(p: &Pad, st: &PadsState) -> Option<String> {
    let r = &p.layers.first()?.region;
    let w = r.warp.as_ref()?;
    let rate = st.sounds.iter().find(|s| s.id == r.source)?.rate;
    let beats = w.beats(r.frames(), rate);
    let whole = |x: f64| (x - x.round()).abs() < 0.02 && x.round() >= 1.0;
    let bars = beats / 4.0;
    Some(if whole(bars) {
        let n = bars.round();
        format!("{n} bar{}", if n == 1.0 { "" } else { "s" })
    } else if whole(beats) {
        let n = beats.round();
        format!("{n} beat{}", if n == 1.0 { "" } else { "s" })
    } else {
        format!("{bars:.2} bars")
    })
}

/// What the Pad list says under its rows about pad `p`, and whether it
/// warns: where an instrument's or a MIDI out pad's hit goes, or why it
/// would send nothing; how a loop plays. None for a plain sample.
fn footer(p: &Pad, st: &PadsState) -> Option<(String, bool)> {
    match &p.kind {
        PadKind::Instrument { engine, note } => Some(
            match st
                .engines
                .iter()
                .find(|e| e.eq_ignore_ascii_case(engine))
            {
                Some(name) => (
                    format!(
                        "A hit plays the Synth tab's {name} sound at {}, through the Synth strip on Mix",
                        note_name(*note)
                    ),
                    false,
                ),
                None if st.engines.is_empty() => return None,
                None => (format!("plays nothing: there is no engine called {engine}"), true),
            },
        ),
        PadKind::MidiOut {
            port,
            channel,
            note,
        } => {
            if port.is_empty() {
                return Some(if st.ports.is_empty() {
                    ("sends nothing: no MIDI output is plugged in".to_string(), true)
                } else {
                    ("sends nothing: pick a port".to_string(), true)
                });
            }
            Some(match st.ports.iter().find(|o| o.name == *port) {
                Some(OutPort { why: Some(why), .. }) => (why.clone(), true),
                Some(o) => (
                    format!(
                        "A hit sends note {note} on channel {channel} to {}",
                        o.shown
                    ),
                    false,
                ),
                None => (format!("sends nothing: {port} is not plugged in"), true),
            })
        }
        PadKind::Sample if p.on_launch => Some((
            "Loops in time with the song: a hit starts it, the next hit stops it".to_string(),
            false,
        )),
        PadKind::Sample => None,
    }
}

fn row_text(p: Option<&Pad>, st: &PadsState, row: Row) -> String {
    if row == Row::Effects {
        // The strip's effects whether or not the pad has a sound: they're
        // the strip's, not the sound's.
        return match &st.effects {
            None => "—".into(),
            Some(fx) if fx.is_empty() => "+ FX".into(),
            // As many names as the button holds, then how many more.
            Some(fx) => {
                let all = fx.join(", ");
                if fx.len() == 1 || all.chars().count() <= 22 {
                    all
                } else {
                    format!("{} +{}", fx[0], fx.len() - 1)
                }
            }
        };
    }
    let Some(p) = p else {
        return if row == Row::Sound {
            "none".into()
        } else {
            "—".into()
        };
    };
    match row {
        Row::Sound => {
            let name = st
                .sound_of_picked()
                .map_or("?", |k| st.sounds[k].name.as_str());
            if p.layers.len() > 1 {
                format!("{name} +{}", p.layers.len() - 1)
            } else {
                name.to_string()
            }
        }
        Row::Mode => p
            .layers
            .first()
            .map_or("—".into(), |l| mode_label(l.region.mode).to_string()),
        Row::Tempo => {
            let Some(w) = p.layers.first().and_then(|l| l.region.warp.as_ref()) else {
                return "—".into();
            };
            match loop_length(p, st) {
                Some(len) => format!("{:.1} BPM · {len}", w.bpm),
                None => format!("{:.1} BPM", w.bpm),
            }
        }
        Row::Engine => match &p.kind {
            PadKind::Instrument { engine, .. } => engine_name(st, engine).to_string(),
            _ => "—".into(),
        },
        Row::Note => match &p.kind {
            PadKind::Instrument { note, .. } => note_name(*note),
            PadKind::MidiOut { note, .. } => format!("{note} · {}", note_name(*note)),
            PadKind::Sample => "—".into(),
        },
        Row::Channel => match &p.kind {
            PadKind::MidiOut { channel, .. } => channel.to_string(),
            _ => "—".into(),
        },
        Row::Port => match &p.kind {
            PadKind::MidiOut { port, .. } if port.is_empty() => "none".into(),
            PadKind::MidiOut { port, .. } => st
                .ports
                .iter()
                .find(|o| o.name == *port)
                .map_or_else(|| port.clone(), |o| o.shown.clone()),
            _ => "—".into(),
        },
        r => r
            .play_row()
            .map_or_else(String::new, |pr| value_text(p, pr)),
    }
}
