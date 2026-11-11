//! What the screens show, built on the control thread from its copies of a
//! chain and the mod engine (the `from_*` builders read them directly, for
//! previews and tests).

use mpfree_fx::mpfree_node::{Curve, Node, ParamInfo};
use mpfree_fx::{Chain, StripId};
use mpfree_mod::{value_text, Binding, ModEngine, Source, KNOBS, MACROS};
use mpfree_param::{Owner, ParamAddr, Strip};
use mpfree_ui::{Color, Theme};

/// Knobs on one page of an effect.
pub const KNOBS_PER_PAGE: usize = 8;

/// A macro's colour, from the theme: macro `m` always wears the same one,
/// on its card, its Q-Link and every knob it moves.
pub fn macro_color(t: &Theme, m: usize) -> Color {
    let palette = [
        t.accent.mix(t.bad, 0.55),
        t.warn.mix(t.bad, 0.35),
        t.good,
        t.warn,
        t.accent,
        t.bad,
        t.good.mix(t.accent, 0.5),
        t.text_dim,
    ];
    palette[m % palette.len()]
}

/// The parameter address's name for a mixer strip (`None` past 255).
pub fn strip_of(id: StripId) -> Option<Strip> {
    let n = |i: usize| u8::try_from(i).ok();
    Some(match id {
        StripId::Input(i) => Strip::Input(n(i)?),
        StripId::Pad(i) => Strip::Pad(n(i)?),
        StripId::Track(i) => Strip::Track(n(i)?),
        StripId::Bus(i) => Strip::Bus(n(i)?),
        StripId::Return(i) => Strip::Return(n(i)?),
        StripId::Master => Strip::Master,
    })
}

/// The mixer's id for a strip.
pub fn strip_id(s: Strip) -> StripId {
    match s {
        Strip::Input(i) => StripId::Input(i.into()),
        Strip::Pad(i) => StripId::Pad(i.into()),
        Strip::Track(i) => StripId::Track(i.into()),
        Strip::Bus(i) => StripId::Bus(i.into()),
        Strip::Return(i) => StripId::Return(i.into()),
        Strip::Master => StripId::Master,
    }
}

/// Text on an accent-coloured fill. The theme system (PR #72) adds
/// `Theme::on_accent`; until it lands, the page's background reads on
/// every accent of the dark theme.
pub fn on_accent(t: &Theme) -> Color {
    t.bg
}

/// One knob of the effect's page.
#[derive(Clone, Debug, PartialEq)]
pub struct ParamView {
    pub addr: ParamAddr,
    pub info: ParamInfo,
    /// The value it is at now (after macros, the morph and modulation).
    pub value: f32,
    /// Where the knob points, 0 to 1.
    pub pos: f32,
    /// How far the matrix moves it, as knob positions (a ring round it).
    pub mod_range: Option<(f32, f32)>,
    /// The macro that moves it.
    pub macro_: Option<usize>,
    pub locked: bool,
}

impl ParamView {
    pub fn text(&self) -> String {
        value_text(&self.info, self.value)
    }
}

/// One insert in the chain list.
#[derive(Clone, Debug, PartialEq)]
pub struct SlotView {
    /// The slot id (`Chain::uid`): what macros, locks and bindings name.
    pub uid: u16,
    pub name: String,
    /// A short line about its settings ("Tube · 0.18", "LP 24 · 2.4 kHz").
    pub summary: String,
    pub on: bool,
    /// Macros that move one of its parameters.
    pub macros: Vec<usize>,
    /// Whether a matrix row moves one of its parameters.
    pub modulated: bool,
}

/// A matrix row aimed at the focused parameter.
#[derive(Clone, Debug, PartialEq)]
pub struct RowView {
    /// The row's index in the engine (for depth changes and removal).
    pub index: usize,
    pub source: String,
    pub depth: f32,
}

/// The chain editor's state.
#[derive(Clone, Debug, PartialEq)]
pub struct ChainState {
    /// "Bass FX".
    pub title: String,
    pub strip: Strip,
    pub slots: Vec<SlotView>,
    /// Inserts the strip can hold.
    pub room: usize,
    pub selected: Option<usize>,
    /// The selected effect's parameter groups (its pages) and the one shown.
    pub groups: Vec<&'static str>,
    pub group: usize,
    /// The knobs of that page.
    pub params: Vec<ParamView>,
    /// The knob last touched: the modulation panel works on it.
    pub focus: Option<ParamAddr>,
    pub rows: Vec<RowView>,
    /// The macros' names ("Wash", "Grit" ...).
    pub macro_names: Vec<String>,
    /// A learn is armed: the next Q-Link turned takes the focused knob.
    pub learning: bool,
    /// The share of the period the chain costs (Dev tools on).
    pub cost_pct: Option<f32>,
    /// A Back button in the list's corner and what it goes back to
    /// ("Mixer"), when the editor took another screen's place.
    pub back: Option<String>,
    /// Chains the editor can switch between ("Pad 1", "Drums"), and the
    /// one shown; empty for none.
    pub scopes: Vec<String>,
    pub scope: usize,
}

impl Default for ChainState {
    fn default() -> Self {
        ChainState {
            title: String::new(),
            strip: Strip::Master,
            slots: Vec::new(),
            room: 0,
            selected: None,
            groups: Vec::new(),
            group: 0,
            params: Vec::new(),
            focus: None,
            rows: Vec::new(),
            macro_names: default_macro_names(),
            learning: false,
            cost_pct: None,
            back: None,
            scopes: Vec::new(),
            scope: 0,
        }
    }
}

pub fn default_macro_names() -> Vec<String> {
    (1..=MACROS).map(|m| format!("Macro {m}")).collect()
}

/// A source's name for a row.
pub fn source_name(s: Source) -> String {
    match s {
        Source::Lfo(n) => format!("LFO {}", n + 1),
        Source::Follower(n) => format!("Follower {}", n + 1),
        Source::Macro(n) => format!("Macro {}", n + 1),
        Source::Host(0) => "Velocity".into(),
        Source::Host(1) => "Pressure".into(),
        Source::Host(2) => "Mod wheel".into(),
        Source::Host(n) => format!("Control {}", n + 1),
    }
}

/// A one-line summary of a node's settings: its first choice and its first
/// two continuous parameters ("Tube · +9.5 dB · 3.2 kHz").
fn summary(n: &dyn Node) -> String {
    let ps = n.params();
    let mut parts: Vec<String> = Vec::new();
    if let Some(p) = ps
        .iter()
        .find(|p| matches!(p.curve, Curve::Choice(_)) && !p.readonly)
    {
        parts.push(value_text(p, n.param(p.id)));
    }
    for p in ps
        .iter()
        .filter(|p| !p.readonly && matches!(p.curve, Curve::Linear | Curve::Log))
        .take(2 - parts.len().min(1))
    {
        parts.push(value_text(p, n.param(p.id)));
    }
    parts.join(" · ")
}

/// The groups a node's parameters fall in, in order of first appearance,
/// and the parameters of group `g` (readonly meters left out), at most
/// [`KNOBS_PER_PAGE`].
pub fn page(params: &'static [ParamInfo], g: usize) -> (Vec<&'static str>, Vec<ParamInfo>) {
    let mut groups: Vec<&'static str> = Vec::new();
    for p in params.iter().filter(|p| !p.readonly) {
        if !groups.contains(&p.group) {
            groups.push(p.group);
        }
    }
    // Groups too big for one page split into "Name", "Name 2" ...; kept
    // simple: the page shows the first KNOBS_PER_PAGE of the group.
    let g = g.min(groups.len().saturating_sub(1));
    let ps = groups
        .get(g)
        .map(|name| {
            params
                .iter()
                .filter(|p| !p.readonly && p.group == *name)
                .take(KNOBS_PER_PAGE)
                .copied()
                .collect()
        })
        .unwrap_or_default();
    (groups, ps)
}

impl ChainState {
    /// Reads a chain (on `strip`) and the engine that modulates it.
    /// `selected`, `group` and `focus` are what the screen had.
    #[allow(clippy::too_many_arguments)]
    pub fn from_chain(
        chain: &Chain,
        strip: Strip,
        title: &str,
        room: usize,
        engine: &ModEngine,
        selected: Option<usize>,
        group: usize,
        focus: Option<ParamAddr>,
    ) -> ChainState {
        let mut slots = Vec::new();
        for i in 0..chain.len() {
            let (Some(n), Some(uid)) = (chain.node(i), chain.uid(i)) else {
                continue;
            };
            let owner = Owner::Insert(strip, uid);
            let mut macros: Vec<usize> = n
                .params()
                .iter()
                .filter_map(|p| engine.macro_of(ParamAddr::new(owner, p.id)))
                .collect();
            macros.sort_unstable();
            macros.dedup();
            let modulated = engine.rows().any(|r| r.target.owner == owner);
            slots.push(SlotView {
                uid,
                name: n.info().name.to_string(),
                summary: summary(n),
                on: chain.is_on(i),
                macros,
                modulated,
            });
        }
        let selected = selected
            .filter(|s| *s < slots.len())
            .or(if slots.is_empty() { None } else { Some(0) });
        let (mut groups, mut params, mut group_shown) = (Vec::new(), Vec::new(), 0);
        if let (Some(s), Some(n)) = (selected, selected.and_then(|s| chain.node(s))) {
            let owner = Owner::Insert(strip, slots[s].uid);
            let (gs, ps) = page(n.params(), group);
            group_shown = group.min(gs.len().saturating_sub(1));
            groups = gs;
            params = ps
                .into_iter()
                .map(|info| {
                    let addr = ParamAddr::new(owner, info.id);
                    let value = engine.value(addr).unwrap_or_else(|| n.param(info.id));
                    ParamView {
                        addr,
                        info,
                        value,
                        pos: info.to_unit(value),
                        mod_range: engine.mod_range(addr),
                        macro_: engine.macro_of(addr),
                        locked: engine.is_locked(addr),
                    }
                })
                .collect();
        }
        let focus = focus.filter(|f| params.iter().any(|p| p.addr == *f));
        let rows = focus
            .map(|f| {
                engine
                    .rows()
                    .enumerate()
                    .filter(|(_, r)| r.target == f)
                    .map(|(i, r)| RowView {
                        index: i,
                        source: source_name(r.source),
                        depth: r.depth,
                    })
                    .collect()
            })
            .unwrap_or_default();
        ChainState {
            title: title.to_string(),
            strip,
            slots,
            room,
            selected,
            groups,
            group: group_shown,
            params,
            focus,
            rows,
            macro_names: default_macro_names(),
            learning: false,
            cost_pct: None,
            back: None,
            scopes: Vec::new(),
            scope: 0,
        }
    }

    pub fn focused(&self) -> Option<&ParamView> {
        self.focus
            .and_then(|f| self.params.iter().find(|p| p.addr == f))
    }

    /// The page's own Q-Link knobs for [`mpfree_mod::Bindings::set_page`]:
    /// the first four knobs of the effect page shown (Drive: Mode, Drive,
    /// Tone, Bias), and the effect's name.
    pub fn qlinks(&self) -> (String, [Option<Binding>; KNOBS]) {
        let name = self
            .selected
            .and_then(|s| self.slots.get(s))
            .map(|s| s.name.clone())
            .unwrap_or_default();
        (
            name,
            std::array::from_fn(|k| {
                self.params
                    .get(k)
                    .map(|p| Binding::new(p.addr, p.info, p.info.name))
            }),
        )
    }
}

/// One target on a macro's card.
#[derive(Clone, Debug, PartialEq)]
pub struct TargetView {
    pub addr: ParamAddr,
    /// "Reverb size", "Drive (Drums)".
    pub label: String,
    /// The range as knob positions, and as text ("5 → 65 %").
    pub from: f32,
    pub to: f32,
    pub range_text: String,
    /// Where the target is now, as a knob position.
    pub now: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MacroView {
    pub name: String,
    pub value: f32,
    pub targets: Vec<TargetView>,
}

/// One line of the morph's "what changes" list.
#[derive(Clone, Debug, PartialEq)]
pub struct ChangeView {
    pub label: String,
    pub a: String,
    pub b: String,
    pub now: String,
}

/// The macros and morph panel's state.
#[derive(Clone, Debug, PartialEq)]
pub struct MacrosState {
    pub macros: Vec<MacroView>,
    /// Which four macros are shown: 0 for 1-4, 1 for 5-8.
    pub bank: usize,
    pub morph: f32,
    pub scene_names: [String; 2],
    pub changes: Vec<ChangeView>,
}

impl Default for MacrosState {
    fn default() -> Self {
        MacrosState {
            macros: Vec::new(),
            bank: 0,
            morph: 0.0,
            scene_names: ["A".into(), "B".into()],
            changes: Vec::new(),
        }
    }
}

impl MacrosState {
    /// Reads the engine. `label` names a parameter for the cards ("Reverb
    /// size"); `describe` gives its description.
    pub fn from_engine(
        engine: &ModEngine,
        names: &[String],
        label: &dyn Fn(ParamAddr) -> String,
        bank: usize,
    ) -> MacrosState {
        let macros = (0..MACROS)
            .map(|m| MacroView {
                name: names
                    .get(m)
                    .cloned()
                    .unwrap_or_else(|| format!("Macro {}", m + 1)),
                value: engine.macro_value(m),
                targets: engine
                    .macro_maps(m)
                    .map(|map| {
                        let info = engine.info(map.target);
                        let now = engine
                            .value(map.target)
                            .zip(info)
                            .map_or(0.0, |(v, i)| i.to_unit(v));
                        let range_text = info.map_or(String::new(), |i| {
                            range_text(&i, i.from_unit(map.from), i.from_unit(map.to))
                        });
                        TargetView {
                            addr: map.target,
                            label: label(map.target),
                            from: map.from,
                            to: map.to,
                            range_text,
                            now,
                        }
                    })
                    .collect(),
            })
            .collect();
        let changes = engine
            .scene_changes()
            .map(|(addr, a, b)| {
                let info = engine.info(addr);
                let t = |v: f32| info.map_or(format!("{v:.2}"), |i| value_text(&i, v));
                ChangeView {
                    label: label(addr),
                    a: t(a),
                    b: t(b),
                    now: engine.value(addr).map(t).unwrap_or_default(),
                }
            })
            .collect();
        MacrosState {
            macros,
            bank: bank.min(1),
            morph: engine.morph(),
            scene_names: ["A".into(), "B".into()],
            changes,
        }
    }
}

/// "5 → 65 %": the unit written once, at the end.
pub fn range_text(info: &ParamInfo, a: f32, b: f32) -> String {
    let (ta, tb) = (value_text(info, a), value_text(info, b));
    match (ta.rsplit_once(' '), tb.rsplit_once(' ')) {
        (Some((na, ua)), Some((nb, ub))) if ua == ub && !matches!(info.curve, Curve::Choice(_)) => {
            format!("{na} → {nb} {ub}")
        }
        _ => format!("{ta} → {tb}"),
    }
}
