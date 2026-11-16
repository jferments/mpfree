//! What an effect page shows: an effect's parameters and a copy of its
//! values, where it sits, its presets, and the tempo for synced times.

use mpfree_fx::{
    Character, Chorus, Comb, Compressor, Delay, DjFilter, Drive, Eq, Flanger, LoFi, MasterSafety,
    MultiFilter, Phaser, Preset,
};
use mpfree_node::{Node, NodeInfo, ParamInfo};
use mpfree_ui::Color;

/// Everything the effect page shows.
#[derive(Clone, Debug)]
pub struct FxState {
    pub info: &'static NodeInfo,
    pub params: &'static [ParamInfo],
    /// The node's values, in `params` order.
    pub values: Vec<f32>,
    /// Where the effect sits, such as "Drums · insert 2" or "Return A".
    pub place: String,
    /// The colour of the track or strip it is on.
    pub color: Color,
    pub presets: Vec<String>,
    /// The preset loaded, and whether it was changed since.
    pub preset: Option<usize>,
    pub edited: bool,
    /// The tempo, for times synced to it (the delay's, the LFOs').
    pub bpm: f64,
    /// The sample rate the effect runs at.
    pub rate: u32,
    /// The effect's input and output peak levels in dBFS, when the host
    /// watches its slot (`mpfree_fx::Chain::watch` and `levels`): the page
    /// then shows a Levels card.
    pub levels: Option<(f32, f32)>,
}

impl FxState {
    /// What the node holds now.
    pub fn from_node(node: &dyn Node, place: &str, color: Color, bpm: f64, rate: u32) -> FxState {
        let params = node.params();
        let info = node.info();
        FxState {
            info,
            params,
            values: params.iter().map(|p| node.param(p.id)).collect(),
            place: place.to_string(),
            color,
            presets: presets(info.id)
                .iter()
                .map(|p| p.name.to_string())
                .collect(),
            preset: None,
            edited: false,
            bpm,
            rate,
            levels: None,
        }
    }

    pub fn index(&self, id: u16) -> Option<usize> {
        self.params.iter().position(|p| p.id == id)
    }

    /// A value by parameter id (0 for an id the node lacks).
    pub fn value(&self, id: u16) -> f32 {
        self.index(id).map(|i| self.values[i]).unwrap_or(0.0)
    }

    /// Sets a value by index, clamped as the node would. Returns the value
    /// set, or None if nothing changed or the parameter is a meter.
    pub(crate) fn set(&mut self, i: usize, v: f32) -> Option<f32> {
        let p = &self.params[i];
        let v = p.clamp(v);
        if p.readonly || v == self.values[i] {
            return None;
        }
        self.values[i] = v;
        self.edited = true;
        Some(v)
    }
}

/// An effect's factory presets (none for the DJ filter, Master safety and
/// Perform FX).
pub fn presets(id: &str) -> &'static [Preset] {
    mpfree_fx::presets(id)
}

/// What loading preset `i` sends to the node: every parameter's default
/// with the preset's values over them, except the Mix and Quality the
/// effect has now (a preset sets the sound, not how much of it is heard or
/// what it costs) and the meters. Empty for a preset that isn't there.
pub fn preset_values(st: &FxState, i: usize) -> Vec<(u16, f32)> {
    let Some(preset) = presets(st.info.id).get(i) else {
        return Vec::new();
    };
    st.params
        .iter()
        .enumerate()
        .filter(|(_, p)| !p.readonly)
        .map(|(k, p)| {
            let keep = matches!(p.name, "Mix" | "Quality");
            let v = if keep {
                st.values[k]
            } else {
                preset
                    .values
                    .iter()
                    .find(|(id, _)| *id == p.id)
                    .map_or(p.default, |&(_, v)| v)
            };
            (p.id, p.clamp(v))
        })
        .collect()
}

/// A new effect by id, at its defaults (from [`mpfree_fx::registry`],
/// the one list of effects).
pub fn make(id: &str) -> Option<Box<dyn Node>> {
    mpfree_fx::registry::make(id)
}

/// A small copy of an effect that the page draws from: the node's own
/// curve functions, fed the page's values. Effects whose drawing needs no
/// node (the reverb, Perform FX) or that the page doesn't draw keep none,
/// so opening a page never allocates a second reverb.
pub enum Scratch {
    Eq(Box<Eq>),
    Filter(MultiFilter),
    Dj(DjFilter),
    Drive(Box<Drive>),
    Comb(Box<Comb>),
    Delay(Box<Delay>),
    Chorus(Box<Chorus>),
    Flanger(Box<Flanger>),
    Phaser(Box<Phaser>),
    LoFi(Box<LoFi>),
    Character(Box<Character>),
    Comp(Compressor),
    Master(Box<MasterSafety>),
    /// The Channel strip's EQ and compressor, set from its values as the
    /// strip sets its own.
    Strip(Box<StripScratch>),
    None,
}

/// The Channel strip's stages the page draws: its EQ (a low shelf, a
/// bell and a high shelf, with the low cut) and its compressor.
pub struct StripScratch {
    pub eq: Eq,
    pub comp: Compressor,
}

impl StripScratch {
    fn new() -> StripScratch {
        use mpfree_fx::ids::eq;
        let mut eq_ = Eq::new();
        // The bands as `ChannelStrip::new` sets them: a low shelf, a bell,
        // an unused bell and a high shelf; no high cut.
        eq_.set_param(eq::band_type(0), 1.0);
        eq_.set_param(eq::band_type(1), 0.0);
        eq_.set_param(eq::gain(2), 0.0);
        eq_.set_param(eq::band_type(3), 2.0);
        eq_.set_param(eq::HIGH_SLOPE, 0.0);
        StripScratch {
            eq: eq_,
            comp: Compressor::new(),
        }
    }

    /// The strip's values onto its stages, as `ChannelStrip` forwards them.
    fn sync(&mut self, st: &FxState) {
        use mpfree_fx::ids::{channel_strip as cs, compressor as comp, eq};
        let v = |id: u16| st.value(id);
        let low_cut_off = st.index(cs::LOW_CUT).map_or(20.0, |i| st.params[i].min);
        let low_cut_on = v(cs::LOW_CUT) > low_cut_off;
        for (id, to) in [
            (comp::THRESHOLD, cs::COMP_THRESHOLD),
            (comp::RATIO, cs::RATIO),
            (comp::ATTACK, cs::COMP_ATTACK),
            (comp::RELEASE, cs::COMP_RELEASE),
            (comp::MAKEUP, cs::MAKEUP),
        ] {
            self.comp.set_param(id, v(to));
        }
        for (id, value) in [
            (eq::LOW_CUT, v(cs::LOW_CUT)),
            (eq::LOW_SLOPE, if low_cut_on { 1.0 } else { 0.0 }),
            (eq::freq(0), v(cs::LOW_FREQ)),
            (eq::gain(0), v(cs::LOW_GAIN)),
            (eq::freq(1), v(cs::MID_FREQ)),
            (eq::gain(1), v(cs::MID_GAIN)),
            (eq::q(1), v(cs::MID_Q)),
            (eq::freq(3), v(cs::HIGH_FREQ)),
            (eq::gain(3), v(cs::HIGH_GAIN)),
        ] {
            self.eq.set_param(id, value);
        }
    }
}

impl Scratch {
    /// A copy for the effect `st` shows, at its rate and values.
    pub fn new(st: &FxState) -> Scratch {
        let mut s = match st.info.id {
            "mpfree.fx.eq" => Scratch::Eq(Box::new(Eq::new())),
            "mpfree.fx.filter" => Scratch::Filter(MultiFilter::new()),
            "mpfree.fx.dj_filter" => Scratch::Dj(DjFilter::new()),
            "mpfree.fx.drive" => Scratch::Drive(Box::new(Drive::new())),
            "mpfree.fx.comb" => Scratch::Comb(Box::new(Comb::new())),
            "mpfree.fx.delay" => Scratch::Delay(Box::new(Delay::new())),
            "mpfree.fx.chorus" => Scratch::Chorus(Box::new(Chorus::new())),
            "mpfree.fx.flanger" => Scratch::Flanger(Box::new(Flanger::new())),
            "mpfree.fx.phaser" => Scratch::Phaser(Box::new(Phaser::new())),
            "mpfree.fx.lofi" => Scratch::LoFi(Box::new(LoFi::new())),
            "mpfree.fx.character" => Scratch::Character(Box::new(Character::new())),
            "mpfree.fx.compressor" => Scratch::Comp(Compressor::new()),
            "mpfree.fx.master_safety" => Scratch::Master(Box::new(MasterSafety::new())),
            "mpfree.fx.channel_strip" => Scratch::Strip(Box::new(StripScratch::new())),
            _ => Scratch::None,
        };
        if let Some(n) = s.node() {
            n.prepare(st.rate.max(1), 64);
        }
        if let Scratch::Strip(x) = &mut s {
            x.eq.prepare(st.rate.max(1), 64);
            x.comp.prepare(st.rate.max(1), 64);
        }
        s.sync(st);
        s
    }

    fn node(&mut self) -> Option<&mut dyn Node> {
        Some(match self {
            Scratch::Eq(n) => n.as_mut(),
            Scratch::Filter(n) => n,
            Scratch::Dj(n) => n,
            Scratch::Drive(n) => n.as_mut(),
            Scratch::Comb(n) => n.as_mut(),
            Scratch::Delay(n) => n.as_mut(),
            Scratch::Chorus(n) => n.as_mut(),
            Scratch::Flanger(n) => n.as_mut(),
            Scratch::Phaser(n) => n.as_mut(),
            Scratch::LoFi(n) => n.as_mut(),
            Scratch::Character(n) => n.as_mut(),
            Scratch::Comp(n) => n,
            Scratch::Master(n) => n.as_mut(),
            Scratch::Strip(_) | Scratch::None => return None,
        })
    }

    /// Takes the page's values (cheap: the curve functions read the
    /// values set, not the gliding ones).
    pub fn sync(&mut self, st: &FxState) {
        if let Scratch::Strip(s) = self {
            s.sync(st);
            return;
        }
        if let Some(n) = self.node() {
            for (p, &v) in st.params.iter().zip(&st.values) {
                if !p.readonly && n.param(p.id) != v {
                    n.set_param(p.id, v);
                }
            }
        }
    }
}
