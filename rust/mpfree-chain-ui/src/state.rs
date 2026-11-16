//! What the screens show, built on the control thread from its copies of a
//! chain and the mod engine (the `from_*` builders read them directly, for
//! previews and tests).

use mpfree_fx::mpfree_node::{Curve, Node, ParamInfo};
use mpfree_fx::{Chain, StripId};
use mpfree_fx_ui::visual;
use mpfree_fx_ui::{FxState, Scratch};
use mpfree_mod::{value_text, Binding, MapCurve, ModEngine, Source, KNOBS, MACROS};
use mpfree_param::{Owner, ParamAddr, Strip};
use mpfree_synth_ui::widgets::Card;
use mpfree_ui::{Color, Theme};

use crate::sections::{PageView, SectionState};

/// Knobs on one page of an effect.
pub const KNOBS_PER_PAGE: usize = 8;

/// A macro's colour, from the theme's series colours (`Theme::track`):
/// macro `m` always wears the same one, on its card, its Q-Link and every
/// knob it moves, and every theme picks its own eight.
pub fn macro_color(t: &Theme, m: usize) -> Color {
    t.track[m % t.track.len()]
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

/// One knob of the effect's page.
#[derive(Clone, Debug, PartialEq)]
pub struct ParamView {
    pub addr: ParamAddr,
    pub info: ParamInfo,
    /// What the knob and its Q-Link are called: its name, or its group's
    /// and its own where the name repeats in the effect ("Band 1 gain").
    pub label: String,
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

/// The strip list's groups, in order (the strip button's list, layout
/// spec section 3a).
pub const STRIP_GROUPS: [&str; 6] = ["Tracks", "Pads", "Inputs", "Returns", "Submixes", "Master"];

/// A strip the strip list offers: a tap opens its chain.
#[derive(Clone, Debug, PartialEq)]
pub struct StripPick {
    pub strip: Strip,
    pub name: String,
    /// What it holds: "no effects", "3 effects", "muted · 1 effect".
    pub sub: String,
}

/// The strip list, open over the editor from the strip button.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StripList {
    /// The group shown, in [`STRIP_GROUPS`].
    pub group: usize,
    /// How many strips each group has.
    pub counts: [usize; 6],
    /// The group's strips, in order.
    pub strips: Vec<StripPick>,
}

/// The chain editor's state.
#[derive(Clone, Debug, PartialEq)]
pub struct ChainState {
    /// "Bass FX".
    pub title: String,
    pub strip: Strip,
    /// The strip button: the strip's name ("Bass") and which strip it is
    /// ("Track 4"); no button when the name is empty or there are scopes.
    pub strip_name: String,
    pub strip_kind: String,
    /// The strip list, while it is open ([`crate::ChainAction::Strips`]).
    pub list: Option<StripList>,
    pub slots: Vec<SlotView>,
    /// Inserts the strip can hold.
    pub room: usize,
    pub selected: Option<usize>,
    /// The selected effect's pages' names (a tab each when it has two or
    /// more: the Channel strip's Main and stages, a plugin's groups) and
    /// the one shown ([`crate::sections::PageView`]).
    pub groups: Vec<String>,
    pub group: usize,
    /// Every control, section switch and meter on that page, each once:
    /// what [`ChainState::sections`] point into.
    pub params: Vec<ParamView>,
    /// What Q-Links 1 to 4 turn, the same on every page of the effect
    /// ([`ChainState::qlinks`]): its plan's four (a synced Time's turns
    /// Sync), or a plugin page's first four.
    pub main_four: Vec<ParamView>,
    /// The page's sections and their layout spec (`mpfree-ui` syntax over
    /// their keys), drawn with the Synth tab's section widget.
    pub sections: Vec<SectionState>,
    pub spec: String,
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
    /// Open the effect picker (the mixer's "+" on a strip,
    /// `MixAction::AddInsert`). The editor opens it itself from its own
    /// "+ Add effect".
    pub picking: bool,
    /// Plugin effects the picker offers after the built-in ones (the app's
    /// plugin rack); empty when there are none.
    pub plugins: Vec<Pick>,
    /// The favourite and recent effects (the app keeps them in the store,
    /// `mpfree_fx::picker`).
    pub shelf: mpfree_fx::picker::Shelf,
    /// The selected effect's modes, when it is one of a family
    /// (`mpfree_fx::families`): the Mode chooser.
    pub mode: Option<ModeView>,
    /// The selected effect is the EQ: its page is the curve
    /// ([`crate::eq_page`]), `params` hold every one of its controls and
    /// there are no sections or page tabs.
    pub eq: bool,
    /// The selected effect's presets, for the list its name in the header
    /// opens: "Defaults", then its factory presets (a family's, those of
    /// the mode it is in). Empty when it has none (a plugin).
    pub presets: Vec<PresetPick>,
    /// The one of [`ChainState::presets`] its settings are now, if any.
    pub preset: Option<usize>,
}

/// One entry of the header's presets list: its name and every value
/// loading it sets (each settable control but Mix and Quality, which
/// stay as they are: a preset sets the sound, not how much of it is
/// heard or what it costs).
#[derive(Clone, Debug, PartialEq)]
pub struct PresetPick {
    pub name: &'static str,
    /// The engine it is for (the insert's `info().id`).
    pub id: &'static str,
    pub values: Vec<(u16, f32)>,
}

/// Insert `n`'s presets for the header's list ([`ChainState::presets`]),
/// in the mode `found` of its family, if it is in one.
fn preset_picks(
    n: &dyn Node,
    found: Option<(&'static mpfree_fx::families::Family, Option<usize>)>,
) -> Vec<PresetPick> {
    let id = n.info().id;
    let (list, fixed): (Vec<&'static mpfree_fx::Preset>, &[(u16, f32)]) = match found {
        Some((f, Some(m))) => (
            mpfree_fx::families::presets(f)
                .into_iter()
                .filter(|p| p.mode == m && p.id == id)
                .map(|p| p.preset)
                .collect(),
            f.modes[m].fixed,
        ),
        // A Cloud reverb set to its own Plate or Room: in no mode, so no
        // mode's presets.
        Some((_, None)) => return Vec::new(),
        None => (mpfree_fx::picker::presets(id).iter().collect(), &[]),
    };
    if list.is_empty() {
        return Vec::new();
    }
    let values = |preset: Option<&mpfree_fx::Preset>| -> Vec<(u16, f32)> {
        n.params()
            .iter()
            .filter(|p| !p.readonly && !matches!(p.name, "Mix" | "Quality"))
            .map(|p| {
                let set = |vals: &[(u16, f32)]| vals.iter().find(|(q, _)| *q == p.id).map(|v| v.1);
                let v = set(fixed)
                    .or_else(|| preset.and_then(|pr| set(pr.values)))
                    .unwrap_or(p.default);
                (p.id, p.clamp(v))
            })
            .collect()
    };
    std::iter::once(PresetPick {
        name: "Defaults",
        id,
        values: values(None),
    })
    .chain(list.into_iter().map(|p| PresetPick {
        name: p.name,
        id,
        values: values(Some(p)),
    }))
    .collect()
}

/// Whether insert `n`'s settings are what loading `pick` sets.
fn plays(n: &dyn Node, pick: &PresetPick) -> bool {
    pick.values
        .iter()
        .all(|&(id, v)| (n.param(id) - v).abs() <= 1e-4 * (1.0 + v.abs()))
}

/// The selected insert's family and its modes, for the Mode chooser.
#[derive(Clone, Debug, PartialEq)]
pub struct ModeView {
    /// "Reverb".
    pub family: &'static str,
    /// The modes' names, in the chooser's order.
    pub modes: Vec<&'static str>,
    /// The mode it is in (None: a Cloud reverb set to its own Plate or
    /// Room, in none of them).
    pub current: Option<usize>,
}

impl Default for ChainState {
    fn default() -> Self {
        ChainState {
            title: String::new(),
            strip: Strip::Master,
            strip_name: String::new(),
            strip_kind: String::new(),
            list: None,
            slots: Vec::new(),
            room: 0,
            selected: None,
            groups: Vec::new(),
            group: 0,
            params: Vec::new(),
            main_four: Vec::new(),
            sections: Vec::new(),
            spec: String::new(),
            focus: None,
            rows: Vec::new(),
            macro_names: default_macro_names(),
            learning: false,
            cost_pct: None,
            back: None,
            scopes: Vec::new(),
            scope: 0,
            picking: false,
            plugins: Vec::new(),
            shelf: mpfree_fx::picker::Shelf::default(),
            mode: None,
            eq: false,
            presets: Vec::new(),
            preset: None,
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

/// One page of an effect in the chain editor: its tab's label and its
/// knobs (parameter ids, at most [`KNOBS_PER_PAGE`]).
#[derive(Clone, Debug, PartialEq)]
pub struct Page {
    pub label: String,
    pub ids: Vec<u16>,
}

/// An effect's pages (layout B, Jay 14 Nov 2026): its Main page first when
/// it has one (`mpfree_fx::main_page`), then its groups in order of first
/// appearance, a group over [`KNOBS_PER_PAGE`] split into pages of that
/// many ("EQ", "EQ 2"). Read-only meters and the parameters in `hide` (a
/// family mode's fixed ones, which the Mode chooser stands for) are left
/// out.
pub fn pages(
    params: &'static [ParamInfo],
    hide: &[(u16, f32)],
    main: Option<&mpfree_fx::main_page::MainPage>,
) -> Vec<Page> {
    let shown = |p: &ParamInfo| !p.readonly && !hide.iter().any(|&(h, _)| h == p.id);
    let mut out = Vec::new();
    if let Some(m) = main {
        let ids: Vec<u16> = mpfree_fx::main_page::knob_ids(m, params)
            .into_iter()
            .filter(|id| params.iter().any(|p| p.id == *id && shown(p)))
            .take(KNOBS_PER_PAGE)
            .collect();
        if !ids.is_empty() {
            out.push(Page {
                label: "Main".into(),
                ids,
            });
        }
    }
    let mut groups: Vec<&'static str> = Vec::new();
    for p in params.iter().filter(|p| shown(p)) {
        if !groups.contains(&p.group) {
            groups.push(p.group);
        }
    }
    for g in groups {
        let ids: Vec<u16> = params
            .iter()
            .filter(|p| shown(p) && p.group == g)
            .map(|p| p.id)
            .collect();
        for (k, chunk) in ids.chunks(KNOBS_PER_PAGE).enumerate() {
            out.push(Page {
                label: if k == 0 {
                    g.to_string()
                } else {
                    format!("{g} {}", k + 1)
                },
                ids: chunk.to_vec(),
            });
        }
    }
    out
}

/// Parameter `id` of node `n` (an insert of `owner`) as a knob: its value
/// after the engine's macros and modulation, and its label ("Band 1 gain"
/// where its name repeats in the effect).
pub fn param_view(n: &dyn Node, owner: Owner, engine: &ModEngine, id: u16) -> Option<ParamView> {
    let info = *n.params().iter().find(|p| p.id == id)?;
    let addr = ParamAddr::new(owner, info.id);
    let value = engine.value(addr).unwrap_or_else(|| n.param(info.id));
    let twins = n.params().iter().filter(|p| p.name == info.name).count();
    Some(ParamView {
        addr,
        info,
        label: if twins > 1 && info.name.len() > 2 {
            format!("{} {}", info.group, info.name.to_lowercase())
        } else if twins > 1 {
            format!("{} {}", info.group, info.name)
        } else {
            info.name.to_string()
        },
        value,
        pos: info.to_unit(value),
        mod_range: engine.mod_range(addr),
        macro_: engine.macro_of(addr),
        locked: engine.is_locked(addr),
    })
}

/// The EQ's node id: its page is the curve ([`crate::eq_page`]).
pub const EQ_ID: &str = "mpfree.fx.eq";

/// The EQ's page: every control (the curve and the band panel reach them
/// all), no sections, and the Q-Links on the band last touched
/// ([`eq_four`]).
pub fn eq_page(
    n: &dyn Node,
    owner: Owner,
    engine: &ModEngine,
    focus: Option<ParamAddr>,
) -> crate::sections::PageView {
    let params: Vec<ParamView> = n
        .params()
        .iter()
        .filter(|p| !p.readonly)
        .filter_map(|p| param_view(n, owner, engine, p.id))
        .collect();
    let qlinks = eq_four(&params, focus);
    crate::sections::PageView {
        pages: Vec::new(),
        page: 0,
        spec: String::new(),
        params,
        sections: Vec::new(),
        qlinks,
    }
}

/// What the Q-Links turn on the EQ's page: the picked band's Freq, Gain
/// and Q (a cut: its frequency and slope), then the EQ's Output. The
/// picked band is the one whose control was touched last, else the first
/// band in use.
pub fn eq_four(params: &[ParamView], focus: Option<ParamAddr>) -> Vec<ParamView> {
    use mpfree_fx::ids::eq as ids;
    let value = |id: u16| params.iter().find(|p| p.info.id == id).map(|p| p.value);
    let used = |b: usize| value(ids::band_use(b)).is_some_and(|u| u != ids::USE_UNUSED);
    let touched = focus.and_then(|f| params.iter().find(|p| p.addr == f).map(|p| p.info.id));
    let ids: Vec<u16> = match touched {
        Some(ids::LOW_CUT | ids::LOW_SLOPE) => vec![ids::LOW_CUT, ids::LOW_SLOPE],
        Some(ids::HIGH_CUT | ids::HIGH_SLOPE) => vec![ids::HIGH_CUT, ids::HIGH_SLOPE],
        t => {
            let b = t
                .and_then(ids::band_of)
                .map(|(b, _)| b)
                .filter(|&b| used(b))
                .or_else(|| (0..ids::BANDS).find(|&b| used(b)))
                .unwrap_or(0);
            vec![ids::freq(b), ids::gain(b), ids::q(b)]
        }
    };
    ids.into_iter()
        .chain([ids::OUTPUT])
        .filter_map(|id| params.iter().find(|p| p.info.id == id).cloned())
        .collect()
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
                name: mpfree_fx::families::insert_name(n),
                summary: summary(n),
                on: chain.is_on(i),
                macros,
                modulated,
            });
        }
        let selected = selected
            .filter(|s| *s < slots.len())
            .or(if slots.is_empty() { None } else { Some(0) });
        let mut page = PageView::default();
        let mut mode = None;
        let (mut presets, mut preset) = (Vec::new(), None);
        if let (Some(s), Some(n)) = (selected, selected.and_then(|s| chain.node(s))) {
            let owner = Owner::Insert(strip, slots[s].uid);
            let found = mpfree_fx::families::node_mode(n);
            mode = found.map(|(f, current)| ModeView {
                family: f.name,
                modes: f.modes.iter().map(|m| m.label).collect(),
                current,
            });
            let hide = match found {
                Some((f, Some(m))) => f.modes[m].fixed,
                _ => &[],
            };
            page = if n.info().id == EQ_ID {
                eq_page(n, owner, engine, focus)
            } else {
                crate::sections::page_view(n, owner, engine, hide, group)
            };
            presets = preset_picks(n, found);
            preset = presets.iter().position(|p| plays(n, p));
        }
        let eq = selected
            .and_then(|s| chain.node(s))
            .is_some_and(|n| n.info().id == EQ_ID);
        let PageView {
            pages: groups,
            page: group_shown,
            spec,
            params,
            sections,
            qlinks: main_four,
        } = page;
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
            strip_name: String::new(),
            strip_kind: String::new(),
            list: None,
            slots,
            room,
            selected,
            groups,
            group: group_shown,
            params,
            main_four,
            sections,
            spec,
            focus,
            rows,
            macro_names: default_macro_names(),
            learning: false,
            cost_pct: None,
            back: None,
            scopes: Vec::new(),
            scope: 0,
            picking: false,
            plugins: Vec::new(),
            shelf: mpfree_fx::picker::Shelf::default(),
            mode,
            eq,
            presets,
            preset,
        }
    }

    pub fn focused(&self) -> Option<&ParamView> {
        self.focus
            .and_then(|f| self.params.iter().find(|p| p.addr == f))
    }

    /// The Q-Link knobs for [`mpfree_mod::Bindings::set_page`]: the
    /// effect's four ([`ChainState::main_four`]: its plan's, the same on
    /// every page; a Drive's Drive, Tone, Bias and Mix, its Mode being the
    /// Mode chooser), or the first four of the page shown when it has none;
    /// and the effect's name.
    pub fn qlinks(&self) -> (String, [Option<Binding>; KNOBS]) {
        let name = self
            .selected
            .and_then(|s| self.slots.get(s))
            .map(|s| s.name.clone())
            .unwrap_or_default();
        let four = if self.main_four.is_empty() {
            &self.params
        } else {
            &self.main_four
        };
        (
            name,
            std::array::from_fn(|k| four.get(k).map(|p| Binding::new(p.addr, p.info, &p.label))),
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
    pub curve: MapCurve,
    /// The parameter's description, for the range editor's values.
    pub info: Option<ParamInfo>,
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
    /// The target editor is open on macro `m`'s target `k` (the panel
    /// opens it itself from a tap on the target).
    pub editing: Option<(usize, usize)>,
}

impl Default for MacrosState {
    fn default() -> Self {
        MacrosState {
            macros: Vec::new(),
            bank: 0,
            morph: 0.0,
            scene_names: ["A".into(), "B".into()],
            changes: Vec::new(),
            editing: None,
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
                            curve: map.curve,
                            info,
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
            editing: None,
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

/// The drawings of insert `slot`'s effect for page `page` of its effect
/// page (`""`: every one, which the chain editor wants: each of its
/// sections shows the ones its plan names), from `mpfree-fx-ui`. Empty for
/// an effect the pages don't draw (a hosted plugin, Perform FX).
pub fn picture(
    chain: &Chain,
    slot: usize,
    page: &str,
    t: &Theme,
    rate: u32,
    bpm: f64,
) -> Vec<Card> {
    let Some(n) = chain.node(slot) else {
        return Vec::new();
    };
    if !visual::has_visuals(n.info().id) {
        return Vec::new();
    }
    let st = FxState::from_node(n, "", t.accent, bpm, rate);
    let scratch = Scratch::new(&st);
    visual::strip(&st, page, None, &scratch, t).cards
}

/// One effect the picker offers: its id (for `mpfree_fx_ui::make`, or a
/// plugin's `clap:` id, which the app's plugin rack makes), its name, a
/// line about it and its category (the picker's heading; plugins are
/// [`mpfree_fx::Category::Plugins`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pick {
    pub id: &'static str,
    pub name: &'static str,
    pub about: &'static str,
    pub category: mpfree_fx::Category,
}

/// The built-in effects the picker offers on `strip`, in the registry's
/// order (grouped by category), a family of effects with modes as one
/// tile (`mpfree_fx::families::tiles`). Master safety only on the master
/// strip.
pub fn picks(strip: Strip) -> Vec<Pick> {
    mpfree_fx::families::tiles(strip == Strip::Master)
        .into_iter()
        .map(|t| Pick {
            id: t.id,
            name: t.name,
            about: t.about,
            category: t.category,
        })
        .collect()
}

impl ChainState {
    /// Everything the picker offers: the built-in effects for the strip,
    /// then the plugins ([`ChainState::plugins`]).
    pub fn picks(&self) -> Vec<Pick> {
        let mut p = picks(self.strip);
        p.extend_from_slice(&self.plugins);
        p
    }
}
