//! What the mixer screen shows: plain values the control thread builds from
//! its copy of the mixer's settings and the meters the audio thread sends.

use mpfree_fx::ids::mixer::{self as mx, output};
use mpfree_fx::mpfree_node::Curve;
use mpfree_fx::{gain_to_db, mixer_param_applies, strip_params, Mixer, StripId, BUSES, RETURNS};

/// Which strips the screen lists. The master is always on the right.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MixView {
    /// Track Mix: the tracks.
    #[default]
    Tracks,
    /// Pad Mix (SHIFT + TRACK MIX): the pad channels.
    Pads,
    /// The line input strips.
    Inputs,
    /// Submixes 1 to 8, then returns A to D.
    Buses,
}

impl MixView {
    pub const ALL: [MixView; 4] = [MixView::Tracks, MixView::Pads, MixView::Inputs, MixView::Buses];

    pub fn label(self) -> &'static str {
        match self {
            MixView::Tracks => "Tracks",
            MixView::Pads => "Pads",
            MixView::Inputs => "Inputs",
            MixView::Buses => "Submixes",
        }
    }

    pub fn index(self) -> usize { MixView::ALL.iter().position(|v| *v == self).unwrap_or(0) }
}

/// One insert slot as the strip shows it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InsertView {
    pub name: String,
    pub on: bool,
    /// The strip keying it, by name ("Kick"), for a sidechain.
    pub key: Option<String>,
    /// Its share of a period while profiling, per cent.
    pub cost_pct: Option<f32>,
}

/// One send: its level and whether it is taken before the fader.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SendView {
    pub db: f32,
    pub pre: bool,
}

impl Default for SendView {
    fn default() -> SendView { SendView { db: mpfree_fx::SILENT_DB, pre: false } }
}

/// An input strip's own settings.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InputView {
    /// [`mx::source`]: 0 off, 1 the stereo pair, 2 input 1, 3 input 2.
    pub source: u8,
    /// [`mx::monitor`]: 0 off, 1 on, 2 auto.
    pub monitor: u8,
    pub arm: bool,
}

/// One channel strip.
#[derive(Clone, Debug, PartialEq)]
pub struct StripView {
    pub id: StripId,
    pub name: String,
    pub volume_db: f32,
    /// -100 (left) to 100 (right).
    pub pan: f32,
    pub mute: bool,
    pub solo: bool,
    /// False when the strip is muted or soloed out (it is drawn dimmed).
    pub audible: bool,
    /// [`mx::OUTPUT`]'s value and, for a track output, the track.
    pub output: u8,
    pub output_track: u8,
    /// Sends A to D; `None` for a return and the master.
    pub sends: Option<[SendView; RETURNS]>,
    pub inserts: Vec<InsertView>,
    /// Insert slots the strip has room for.
    pub slots: usize,
    /// The loudest sample on each side since the last screen update, linear:
    /// after the fader, or, on an input, as it comes in.
    pub peak: [f32; 2],
    pub input: Option<InputView>,
    /// The strip's share of a period while profiling, per cent.
    pub cost_pct: Option<f32>,
}

impl StripView {
    /// A strip at its defaults, for building a state by hand.
    pub fn new(id: StripId, name: impl Into<String>) -> StripView {
        let has_sends = matches!(id, StripId::Input(_) | StripId::Pad(_) | StripId::Track(_) | StripId::Bus(_));
        StripView {
            id,
            name: name.into(),
            volume_db: 0.0,
            pan: 0.0,
            mute: false,
            solo: false,
            audible: true,
            output: output::MASTER as u8,
            output_track: 0,
            sends: has_sends.then(|| [SendView::default(); RETURNS]),
            inserts: Vec::new(),
            slots: 8,
            peak: [0.0; 2],
            input: matches!(id, StripId::Input(_)).then(|| InputView { source: 1, monitor: 2, arm: false }),
            cost_pct: None,
        }
    }

    /// Whether the strip has an output to choose (a return and the master
    /// do not).
    pub fn has_output(&self) -> bool { !matches!(self.id, StripId::Return(_) | StripId::Master) }

    /// What the output button says: "Master", "Sub 3", "Track 2", "None".
    pub fn output_label(&self) -> String { output_label(self.output, self.output_track) }
}

/// The name of an output choice ([`mx::OUTPUT`] value and track).
pub fn output_label(out: u8, track: u8) -> String {
    let o = f32::from(out);
    if o == output::TRACK {
        format!("Track {}", track as usize + 1)
    } else {
        choice(mx::OUTPUT, out).to_string()
    }
}

/// The name of a choice parameter's value, from the mixer's own list.
pub fn choice(id: u16, value: u8) -> &'static str {
    match strip_params().get(id as usize).map(|p| p.curve) {
        Some(Curve::Choice(names)) => names.get(value as usize).copied().unwrap_or("?"),
        _ => "?",
    }
}

/// Master safety as the master strip shows it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SafetyView {
    /// "Safe limiter", "Soft clip" or "Raw".
    pub mode: String,
    pub ceiling_db: f32,
    /// The limiting now, dB (0 or below).
    pub reduction_db: f32,
    /// Frames that came in above the ceiling since the start.
    pub overs: u64,
}

/// Everything the mixer screen shows.
#[derive(Clone, Debug, PartialEq)]
pub struct MixState {
    pub view: MixView,
    /// The strips of `view`, in order.
    pub strips: Vec<StripView>,
    pub master: StripView,
    pub safety: SafetyView,
    /// How many strips each view has, in [`MixView::ALL`] order.
    pub counts: [usize; 4],
    /// Tracks there are, for the output choices of inputs and pads.
    pub tracks: usize,
    pub selected: Option<StripId>,
    /// The whole mixer's share of a period while profiling, per cent.
    pub cost_pct: Option<f32>,
}

impl Default for MixState {
    fn default() -> MixState {
        MixState {
            view: MixView::Tracks,
            strips: Vec::new(),
            master: StripView::new(StripId::Master, "Master"),
            safety: SafetyView { mode: "Safe limiter".into(), ceiling_db: -0.3, reduction_db: 0.0, overs: 0 },
            counts: [0; 4],
            tracks: 16,
            selected: None,
            cost_pct: None,
        }
    }
}

impl MixState {
    pub fn strip(&self, id: StripId) -> Option<&StripView> {
        if id == StripId::Master { Some(&self.master) } else { self.strips.iter().find(|s| s.id == id) }
    }

    pub fn strip_mut(&mut self, id: StripId) -> Option<&mut StripView> {
        if id == StripId::Master { Some(&mut self.master) } else { self.strips.iter_mut().find(|s| s.id == id) }
    }

    /// Reads a mixer the control thread can see (the workstation preview,
    /// tests, or a mixer the app mirrors): the strips of `view`, the
    /// master, and their meters since the last call (it takes the peaks
    /// and, while profiling, the probe windows). `name` names each strip.
    pub fn from_mixer(m: &mut Mixer, view: MixView, name: &dyn Fn(StripId) -> String) -> MixState {
        let ids: Vec<StripId> = match view {
            MixView::Tracks => (0..m.tracks()).map(StripId::Track).collect(),
            MixView::Pads => (0..m.pads()).map(StripId::Pad).collect(),
            MixView::Inputs => (0..m.inputs()).map(StripId::Input).collect(),
            MixView::Buses => (0..BUSES).map(StripId::Bus).chain((0..RETURNS).map(StripId::Return)).collect(),
        };
        let strips = ids.iter().map(|&s| read_strip(m, s, name)).collect();
        let master = read_strip(m, StripId::Master, name);
        let rate = m.rate();
        let cost_pct = m.profiling().then(|| m.probe_mut().take_window().realtime_pct(rate) as f32);
        let ms = m.safety();
        let p = |id: u16| mpfree_fx::mpfree_node::Node::param(ms, id);
        let mode = match p(mpfree_fx::ids::master::MODE) as u8 {
            0 => "Safe limiter",
            1 => "Soft clip",
            _ => "Raw",
        };
        let safety = SafetyView {
            mode: mode.into(),
            ceiling_db: p(mpfree_fx::ids::master::CEILING),
            reduction_db: p(mpfree_fx::ids::master::REDUCTION),
            overs: ms.stats().overs,
        };
        MixState {
            view,
            strips,
            master,
            safety,
            counts: [m.tracks(), m.pads(), m.inputs(), BUSES + RETURNS],
            tracks: m.tracks(),
            selected: None,
            cost_pct,
        }
    }
}

fn read_strip(m: &mut Mixer, s: StripId, name: &dyn Fn(StripId) -> String) -> StripView {
    let mut v = StripView::new(s, name(s));
    let p = |id: u16| m.param(s, id);
    v.volume_db = p(mx::VOLUME);
    v.pan = p(mx::PAN);
    v.mute = p(mx::MUTE) >= 0.5;
    v.solo = p(mx::SOLO) >= 0.5;
    if mixer_param_applies(s, mx::OUTPUT) {
        v.output = p(mx::OUTPUT) as u8;
        v.output_track = p(mx::OUTPUT_TRACK) as u8;
    }
    if let Some(sends) = v.sends.as_mut() {
        for (r, sv) in sends.iter_mut().enumerate() {
            *sv = SendView { db: p(mx::send(r)), pre: p(mx::send_pre(r)) >= 0.5 };
        }
    }
    if let Some(i) = v.input.as_mut() {
        *i = InputView { source: p(mx::SOURCE) as u8, monitor: p(mx::MONITOR) as u8, arm: p(mx::ARM) >= 0.5 };
    }
    v.audible = m.is_audible(s);
    let rate = m.rate();
    let profiling = m.profiling();
    let keys: Vec<Option<StripId>> = (0..m.chain(s).map_or(0, |c| c.len())).map(|slot| m.key(s, slot)).collect();
    if let Some(c) = m.chain_mut(s) {
        v.slots = c.capacity();
        v.inserts = (0..c.len())
            .map(|slot| InsertView {
                name: c.node(slot).map_or("?", |n| n.info().name).to_string(),
                on: c.is_on(slot),
                key: keys[slot].map(name),
                cost_pct: if profiling { c.probe_mut(slot).map(|p| p.take_window().realtime_pct(rate) as f32) } else { None },
            })
            .collect();
    }
    v.cost_pct = if profiling { m.strip_probe_mut(s).map(|p| p.take_window().realtime_pct(rate) as f32) } else { None };
    // An input's meter shows what comes in (what a take records), monitored
    // or not; every other strip's shows what it sends on.
    v.peak = if matches!(s, StripId::Input(_)) { m.take_input_peaks(s) } else { m.take_peaks(s) };
    v
}

/// A strip's usual name: "Track 1", "Pad 1", "In 1", "Sub 1", "Return A",
/// "Master".
pub fn default_name(s: StripId) -> String {
    match s {
        StripId::Input(i) => format!("In {}", i + 1),
        StripId::Pad(p) => format!("Pad {}", p + 1),
        StripId::Track(t) => format!("Track {}", t + 1),
        StripId::Bus(b) => format!("Sub {}", b + 1),
        StripId::Return(r) => format!("Return {}", (b'A' + r as u8) as char),
        StripId::Master => "Master".into(),
    }
}

/// A level in dBFS for a meter, from a linear peak (-80 for silence).
pub fn peak_db(p: f32) -> f32 { gain_to_db(p).max(-80.0) }
