//! Sample screens with a real chain and mod engine behind them, for the
//! workstation preview (`mpfree-chain-preview`) and the tests: a drum bus
//! (filter, drive, compressor, a bypassed EQ), a keys track (chorus,
//! lo-fi, delay) and a kick pad (drive, compressor), four macros across
//! them, three matrix rows, and the morph between two scenes.
//!
//! The screens are drawn where they open in the app (layout spec section
//! 4b, approved by Jay 2026-11-11 22:34Z): the chain editor in the mixer's
//! place with a Back to it, or in the Sample tab's FX sub-tab under the pad
//! bar; the macros panel over the page under the top bar, no tab lit. The
//! top bar is the app's (`Header::app`); the Q-Link column is a stand-in
//! for the app's own (see [`QLinkColumn`]).

use mpfree_fx::ids;
use mpfree_fx::mpfree_node::{Ctx, ParamInfo};
use mpfree_fx::{strip_params, Chain, Chorus, Compressor, Delay, Drive, Eq, LoFi, MultiFilter};
use mpfree_mod::{
    lfo, Bindings, MacroMap, MapCurve, ModEngine, ModEvent, ModTimed, Row, Source, DEFAULT_TICK,
};
use mpfree_param::{Owner, ParamAddr, Strip};
use mpfree_sampler_ui::widgets::PadBar;
use mpfree_ui::widgets::{Chip, Header, Label, APP_TABS};
use mpfree_ui::Fonts;
use mpfree_ui::{Align, EventKind, FontId, PaintCx, Rect, Theme, Tone, Ui, Widget};

use crate::state::{macro_color, picture, ChainState, MacrosState};
use crate::{ChainView, MacrosView};

pub const DRUMS: Strip = Strip::Bus(0);
pub const KEYS: Strip = Strip::Track(4);
pub const KICK: Strip = Strip::Pad(0);
const RATE: u32 = 44_100;
const BLOCK: usize = 128;

pub struct Sample {
    pub name: &'static str,
    pub about: &'static str,
    pub ui: Ui,
}

/// The sample set: two chains, the engine, the bindings.
pub struct Song {
    pub drums: Chain,
    pub keys: Chain,
    pub kick: Chain,
    pub engine: ModEngine,
    pub bindings: Bindings,
    pub names: Vec<String>,
}

fn chain(nodes: Vec<Box<dyn mpfree_fx::mpfree_node::Node>>) -> Chain {
    let mut c = Chain::with_capacity(8);
    c.prepare(RATE, BLOCK);
    for n in nodes {
        c.push(n);
    }
    c
}

fn info(c: &Chain, slot: usize, id: u16) -> ParamInfo {
    *c.node(slot)
        .unwrap()
        .params()
        .iter()
        .find(|p| p.id == id)
        .unwrap()
}

impl Song {
    pub fn new() -> Song {
        let drums = chain(vec![
            Box::new(MultiFilter::new()),
            Box::new(Drive::new()),
            Box::new(Compressor::new()),
            Box::new(Eq::new()),
        ]);
        let keys = chain(vec![
            Box::new(Chorus::new()),
            Box::new(LoFi::new()),
            Box::new(Delay::new()),
        ]);
        let kick = chain(vec![Box::new(Drive::new()), Box::new(Compressor::new())]);
        let mut e = ModEngine::new(RATE, DEFAULT_TICK);
        let add = |c: &Chain, strip: Strip, slot: usize, id: u16, e: &mut ModEngine| -> ParamAddr {
            let a = ParamAddr::new(Owner::Insert(strip, c.uid(slot).unwrap()), id);
            let i = info(c, slot, id);
            e.add_target(a, i, c.node(slot).unwrap().param(id)).unwrap();
            a
        };
        let cutoff = add(&drums, DRUMS, 0, ids::filter::CUTOFF, &mut e);
        let reso = add(&drums, DRUMS, 0, ids::filter::RESONANCE, &mut e);
        let drive = add(&drums, DRUMS, 1, ids::drive::DRIVE, &mut e);
        let tone = add(&drums, DRUMS, 1, ids::drive::TONE, &mut e);
        let bias = add(&drums, DRUMS, 1, ids::drive::BIAS, &mut e);
        let thresh = add(&drums, DRUMS, 2, ids::compressor::THRESHOLD, &mut e);
        let cmix = add(&drums, DRUMS, 2, ids::compressor::MIX, &mut e);
        let crel = add(&drums, DRUMS, 2, ids::compressor::RELEASE, &mut e);
        let chorus = add(&keys, KEYS, 0, ids::chorus::MIX, &mut e);
        let lofi = add(&keys, KEYS, 1, ids::lofi::AMOUNT, &mut e);
        let fb = add(&keys, KEYS, 2, ids::delay::FEEDBACK, &mut e);
        let dtone = add(&keys, KEYS, 2, ids::delay::TONE, &mut e);
        let dlmix = add(&keys, KEYS, 2, ids::delay::MIX, &mut e);
        let kdrive = add(&kick, KICK, 0, ids::drive::DRIVE, &mut e);
        let send = ParamAddr::new(Owner::Strip(KEYS), ids::mixer::send(0));
        e.add_target(
            send,
            *strip_params()
                .iter()
                .find(|p| p.id == ids::mixer::send(0))
                .unwrap(),
            -30.0,
        )
        .unwrap();
        let map = |e: &mut ModEngine, m: usize, target: ParamAddr, from: f32, to: f32| {
            e.map(
                m,
                MacroMap {
                    target,
                    from,
                    to,
                    curve: MapCurve::Linear,
                },
            )
            .unwrap()
        };
        // 1 Wash, 2 Grit, 3 Pump, 4 Air.
        map(&mut e, 0, send, 0.45, 0.85);
        map(&mut e, 0, fb, 0.2, 0.7);
        map(&mut e, 0, dlmix, 0.1, 0.6);
        map(&mut e, 0, dtone, 0.8, 0.35);
        map(&mut e, 1, drive, 0.0, 0.6);
        map(&mut e, 1, bias, 0.0, 0.4);
        map(&mut e, 1, lofi, 0.0, 0.55);
        map(&mut e, 1, kdrive, 0.1, 0.5);
        map(&mut e, 2, thresh, 0.9, 0.55);
        map(&mut e, 2, crel, 0.2, 0.6);
        map(&mut e, 2, cmix, 0.2, 0.6);
        map(&mut e, 3, chorus, 0.0, 0.35);
        map(&mut e, 3, reso, 0.0, 0.3);
        e.add_row(Row {
            source: Source::Lfo(0),
            via: None,
            target: tone,
            depth: 0.18,
        })
        .unwrap();
        e.add_row(Row {
            source: Source::Follower(0),
            via: None,
            target: dlmix,
            depth: -0.25,
        })
        .unwrap();
        e.add_row(Row {
            source: Source::Lfo(1),
            via: Some(Source::Host(2)),
            target: cutoff,
            depth: 0.12,
        })
        .unwrap();
        // Scene A as it is; scene B with the drums darker and wetter.
        let mut setup = vec![
            ModEvent::Set {
                addr: ParamAddr::new(Owner::Mod(0), lfo::ids::SYNC),
                value: 6.0,
            },
            ModEvent::Set {
                addr: ParamAddr::new(Owner::Mod(1), lfo::ids::RATE),
                value: 0.3,
            },
            ModEvent::Source { n: 2, value: 0.8 },
        ];
        let run = |e: &mut ModEngine, evs: &[ModEvent], blocks: u64| {
            for b in 0..blocks {
                let f0 = b * BLOCK as u64;
                let timed: Vec<ModTimed> = if b == 0 {
                    evs.iter().map(|ev| ModTimed::new(0, *ev)).collect()
                } else {
                    Vec::new()
                };
                let audio: Vec<f32> = (0..BLOCK)
                    .map(|i| 0.4 * (((f0 + i as u64) as f32) * 0.05).sin())
                    .collect();
                let ctx = Ctx {
                    rate: RATE,
                    frame: f0,
                    bpm: 92.0,
                    beats: f0 as f64 * 92.0 / 60.0 / RATE as f64,
                    playing: true,
                    sidechain: None,
                };
                e.process(
                    &ctx,
                    BLOCK,
                    &timed,
                    &[Some((&audio, &audio)), None, None, None],
                    &mut |_, _| {},
                );
            }
        };
        run(&mut e, &setup, 4);
        e.capture_scene(0);
        setup = vec![
            ModEvent::Set {
                addr: cutoff,
                value: 900.0,
            },
            ModEvent::Set {
                addr: ParamAddr::macro_value(0),
                value: 0.9,
            },
            ModEvent::Set {
                addr: ParamAddr::macro_value(2),
                value: 0.8,
            },
        ];
        run(&mut e, &setup, 2);
        e.capture_scene(1);
        let names = [
            "Wash", "Grit", "Pump", "Air", "Macro 5", "Macro 6", "Macro 7", "Macro 8",
        ]
        .map(String::from)
        .to_vec();
        let mut start: Vec<ModEvent> = [0.62, 0.40, 0.55, 0.25]
            .iter()
            .enumerate()
            .map(|(m, v)| ModEvent::Set {
                addr: ParamAddr::macro_value(m as u8),
                value: *v,
            })
            .collect();
        start.push(ModEvent::Set {
            addr: ParamAddr::morph_pos(0),
            value: 0.64,
        });
        run(&mut e, &start, 40);
        let mut bindings = Bindings::with_macros(&names);
        for m in 0..mpfree_mod::MACROS {
            bindings.observe(ParamAddr::macro_value(m as u8), e.macro_value(m));
        }
        Song {
            drums,
            keys,
            kick,
            engine: e,
            bindings,
            names,
        }
    }

    /// A parameter's name on the cards: "Drive (Drums)", "Send A (Keys)".
    pub fn label(&self, a: ParamAddr) -> String {
        if let Owner::Macro(n) = a.owner {
            return format!("Macro {} · {}", n + 1, self.names[n as usize]);
        }
        let (strip_name, node) = match a.owner {
            Owner::Insert(s, uid) => {
                let c = self.chain(s);
                (self.strip_name(s), c.slot_of(uid).and_then(|i| c.node(i)))
            }
            Owner::Strip(s) => (self.strip_name(s), None),
            _ => ("", None),
        };
        let pname = match node {
            Some(n) => n
                .params()
                .iter()
                .find(|p| p.id == a.param)
                .map(|p| p.name)
                .unwrap_or("?"),
            None => strip_params()
                .iter()
                .find(|p| p.id == a.param)
                .map(|p| p.name)
                .unwrap_or("?"),
        };
        let nname = node.map(|n| n.info().name);
        match nname {
            Some(n) if !pname.starts_with(n) && pname != "Drive" => {
                format!("{n} {} ({strip_name})", pname.to_lowercase())
            }
            _ => format!("{pname} ({strip_name})"),
        }
    }

    pub fn chain(&self, s: Strip) -> &Chain {
        match s {
            DRUMS => &self.drums,
            KICK => &self.kick,
            _ => &self.keys,
        }
    }

    pub fn strip_name(&self, s: Strip) -> &'static str {
        match s {
            DRUMS => "Drums",
            KICK => "Pad 1",
            _ => "Keys",
        }
    }

    /// Where a Q-Link's target lives, for its card ("DRUMS", "MACRO 2").
    pub fn place(&self, a: ParamAddr) -> String {
        match a.owner {
            Owner::Macro(n) => format!("Macro {}", n + 1),
            Owner::Insert(s, _) | Owner::Strip(s) => self.strip_name(s).to_string(),
            _ => String::new(),
        }
    }

    pub fn chain_state(
        &self,
        strip: Strip,
        selected: usize,
        group: usize,
        focus: Option<ParamAddr>,
    ) -> ChainState {
        let title = format!("{} FX", self.strip_name(strip));
        let mut st = ChainState::from_chain(
            self.chain(strip),
            strip,
            &title,
            8,
            &self.engine,
            Some(selected),
            group,
            focus,
        );
        st.macro_names = self.names.clone();
        st.cost_pct = Some(match strip {
            DRUMS => 6.2,
            KICK => 2.9,
            _ => 4.8,
        });
        st
    }

    pub fn macros_state(&self, bank: usize) -> MacrosState {
        let mut st = MacrosState::from_engine(&self.engine, &self.names, &|a| self.label(a), bank);
        st.scene_names = ["Dry verse".into(), "Wash drop".into()];
        st
    }
}

impl Default for Song {
    fn default() -> Self {
        Self::new()
    }
}

/// A stand-in for the app's Q-Link column (the Hello MPC thread draws the
/// real one), in the Play page's card shape: the knob's number and where
/// its target lives, the target's name, a ring of dots with the value in
/// it. It shows [`Bindings::cards`]; a macro's knob wears its colour.
pub struct QLinkColumn {
    pub cards: Vec<mpfree_mod::QLinkCard>,
    /// Where each card's target lives ("DRUMS", "MACRO 1").
    pub places: Vec<String>,
    pub title: String,
}

impl QLinkColumn {
    pub fn new(song: &Song) -> QLinkColumn {
        let cards = song.bindings.cards().to_vec();
        let places = cards
            .iter()
            .map(|c| c.target.map(|a| song.place(a)).unwrap_or_default())
            .collect();
        let title = match song.bindings.title().as_str() {
            "" => "Q-Links".to_string(),
            t => format!("Q-Links · {t}"),
        };
        QLinkColumn {
            cards,
            places,
            title,
        }
    }
}

/// A knob's colour on the column when its binding has none of its own.
fn knob_color(t: &Theme, k: usize) -> mpfree_ui::Color {
    [t.accent, t.good, t.warn, t.accent.mix(t.bad, 0.55)][k % 4]
}

impl Widget for QLinkColumn {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.canvas.fill_rect(r, t.bg);
        let (head, body) = r.take_top(26);
        cx.text(
            FontId::Bold,
            t.size_small - 2.0,
            head.inset_xy(4, 0),
            Align::Left,
            t.text_faint,
            &self.title.to_uppercase(),
        );
        let gap = 10;
        let h = (body.h - 3 * gap) / 4;
        for (k, c) in self.cards.iter().enumerate() {
            let cr = Rect::new(body.x, body.y + k as i32 * (h + gap), body.w, h);
            let col = c
                .colour
                .map_or(knob_color(&t, k), |m| macro_color(&t, m as usize));
            cx.canvas.fill_round_rect(cr, t.radius, t.surface);
            cx.canvas
                .fill_round_rect(Rect::new(cr.x, cr.y + 14, 4, cr.h - 28), 2, col);
            let inner = cr.inset_xy(14, 10);
            let (top, rest) = inner.take_top(18);
            cx.text(
                FontId::Bold,
                13.0,
                top,
                Align::Left,
                col,
                &format!("Q{}", k + 1),
            );
            let place = self.places.get(k).map_or("", |p| p.as_str());
            cx.text(
                FontId::Sans,
                12.0,
                top,
                Align::Right,
                t.text_faint,
                &place.to_uppercase(),
            );
            let (name, rest) = rest.take_top(24);
            cx.text(
                FontId::Bold,
                16.0,
                name,
                Align::Left,
                if c.enabled() { t.text } else { t.text_faint },
                if c.enabled() { &c.label } else { "free" },
            );
            // The knob: 270 degrees of dots, open at the bottom.
            let d = rest.h.min(rest.w - 10).max(20);
            let (kx, ky) = (rest.x + rest.w / 2, rest.y + rest.h / 2 + 2);
            let rad = d as f32 / 2.0 - 5.0;
            let n = 33;
            for i in 0..n {
                let f = i as f32 / (n - 1) as f32;
                let a = (135.0 + 270.0 * f).to_radians();
                let (x, y) = (
                    kx + (rad * a.cos()).round() as i32,
                    ky + (rad * a.sin()).round() as i32,
                );
                let lit = c.enabled() && f <= c.pos;
                let s = if lit { 3 } else { 2 };
                cx.canvas.fill_round_rect(
                    Rect::new(x - s, y - s, 2 * s, 2 * s),
                    s,
                    if lit { col } else { t.outline },
                );
            }
            // The value inside the ring: "20.0 kHz" as "20 kHz", and a
            // smaller size when the ring is small.
            let vr = Rect::new(kx - rad as i32 + 4, ky - 12, 2 * rad as i32 - 8, 24);
            let short = c.text.replace(".0 ", " ");
            let (size, text) = [(16.0, &short), (13.0, &short), (11.0, &short)]
                .into_iter()
                .find(|(z, s)| cx.measure(FontId::Bold, *z, s) <= vr.w)
                .unwrap_or((11.0, &short));
            cx.text(FontId::Bold, size, vr, Align::Center, t.text, text);
        }
    }

    fn touch(
        &mut self,
        _f: u64,
        _p: mpfree_ui::Touch,
        _x: i32,
        _y: i32,
        _r: Rect,
    ) -> Option<EventKind> {
        None
    }
}

/// Where a panel opens.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Place {
    /// The chain editor in the mixer's place (no tab lit, Back to the mixer).
    Mixer,
    /// The chain editor in the Sample tab's FX sub-tab, under the pad bar.
    SampleFx,
    /// The macros panel over the page, under the top bar (no tab lit).
    Over,
}

/// The pages: the top bar, the panel and the Q-Link column (under the
/// pad bar on Sample), the hint line.
const PREVIEW_SAMPLE: &str = "col(header:60, smp-bar:60, row(chain:*, qlinks:168):*, hint:26)";

/// The app's screen around a panel. For the chain editor, the Q-Links
/// show the selected effect's first four knobs (bank 0, the page's own);
/// for the macros, the bank of the four shown.
pub fn screen(
    fonts: Fonts,
    song: &mut Song,
    place: Place,
    chain: Option<ChainState>,
    macros: Option<MacrosState>,
) -> Result<Ui, String> {
    let mut ui = Ui::new(Theme::dark(), fonts, 1280, 800);
    // The top bar: Sample lit for its FX sub-tab; nothing lit over the
    // mixer or a page (as the mixer does).
    let mut header = Header::app(if place == Place::SampleFx {
        APP_TABS[5]
    } else {
        ""
    });
    header.chips = vec![
        Chip::new("92 BPM", Tone::Normal),
        Chip::new("Night drive", Tone::Normal),
    ];
    ui.add("header", header);
    match (&chain, &macros) {
        (Some(c), _) => {
            let (name, knobs) = c.qlinks();
            song.bindings.set_page(crate::QLINK_PAGE, &name, knobs);
            for p in &c.params {
                song.bindings.observe(p.addr, p.value);
            }
            song.bindings.show(0);
        }
        (None, Some(m)) => song.bindings.show(1 + m.bank),
        _ => {}
    }
    ui.add(QLINKS, QLinkColumn::new(song));
    let (hint, layout) = match place {
        Place::Mixer => (crate::CHAIN_HINT, crate::CHAIN_LAYOUT),
        Place::SampleFx => (crate::CHAIN_HINT, PREVIEW_SAMPLE),
        Place::Over => (crate::MACROS_HINT, crate::MACROS_LAYOUT),
    };
    let mut bar = PadBar::new(&mpfree_sampler_ui::wave_tab::TABS);
    bar.color = Theme::dark().bad;
    bar.title = "Pad 1 · Kick".into();
    bar.subtitle = "kick_tight · 1 layer · Drums".into();
    bar.active = 5;
    ui.add("smp-bar", bar);
    ui.add("hint", Label::new(hint, Tone::Dim, Align::Left));
    ui.add(crate::CHAIN, ChainView::new());
    ui.add(crate::MACROS_W, MacrosView::new());
    ui.add_page("preview", layout)?;
    ui.show("preview")?;
    if let Some(c) = chain {
        let cards = match (c.selected, c.groups.get(c.group)) {
            (Some(slot), Some(page)) => {
                picture(song.chain(c.strip), slot, page, &Theme::dark(), RATE, 92.0)
            }
            _ => Vec::new(),
        };
        crate::update_chain(&mut ui, c);
        crate::set_chain_picture(&mut ui, cards);
    }
    if let Some(m) = macros {
        crate::update_macros(&mut ui, m);
    }
    Ok(ui)
}

const QLINKS: &str = "qlinks";

/// Every sample screen.
pub fn samples(fonts: impl Fn() -> Fonts) -> Result<Vec<Sample>, String> {
    let mut song = Song::new();
    let drive_uid = song.drums.uid(1).unwrap();
    let filter_uid = song.drums.uid(0).unwrap();
    let delay_uid = song.keys.uid(2).unwrap();
    let kick_uid = song.kick.uid(0).unwrap();
    let from_mixer = |mut st: ChainState| {
        st.back = Some("Mixer".into());
        st
    };
    let mut drive = from_mixer(song.chain_state(
        DRUMS,
        1,
        0,
        Some(ParamAddr::new(
            Owner::Insert(DRUMS, drive_uid),
            ids::drive::TONE,
        )),
    ));
    drive.slots[3].on = false;
    let filter = from_mixer(song.chain_state(
        DRUMS,
        0,
        0,
        Some(ParamAddr::new(
            Owner::Insert(DRUMS, filter_uid),
            ids::filter::CUTOFF,
        )),
    ));
    let delay = from_mixer(song.chain_state(
        KEYS,
        2,
        2,
        Some(ParamAddr::new(
            Owner::Insert(KEYS, delay_uid),
            ids::delay::MIX,
        )),
    ));
    let mut kick = song.chain_state(
        KICK,
        0,
        0,
        Some(ParamAddr::new(
            Owner::Insert(KICK, kick_uid),
            ids::drive::DRIVE,
        )),
    );
    kick.scopes = vec!["Pad 1 · Kick".into(), "Track · Drums".into()];
    let mut empty = ChainState::from_chain(
        &chain(vec![]),
        Strip::Track(6),
        "Vocal chop FX",
        8,
        &song.engine,
        None,
        0,
        None,
    );
    empty.back = Some("Mixer".into());
    let mut adding = song.chain_state(KEYS, 1, 0, None);
    adding.back = Some("Mixer".into());
    adding.picking = true;
    let mut starred = adding.clone();
    for id in [
        "mpfree.fx.plate",
        "mpfree.fx.tape_delay",
        "mpfree.fx.cloud",
        "mpfree.fx.amp",
    ] {
        starred.shelf.set_favourite(id, true);
    }
    for id in ["mpfree.fx.room", "mpfree.fx.cloud", "mpfree.fx.limiter"] {
        starred.shelf.used(id);
    }
    // Plate reverb's presets, opened with the wheel: SHIFT + wheel down
    // to Delay and reverb, the wheel to Plate reverb, a push.
    let mut presets = screen(fonts(), &mut song, Place::Mixer, Some(adding.clone()), None)?;
    if let Some(v) = presets.get_mut::<ChainView>(crate::CHAIN) {
        v.wheel(4, true);
        v.wheel(6, false);
        v.push_wheel();
        v.wheel(2, false);
    }
    let m0 = song.macros_state(0);
    let m1 = song.macros_state(1);
    let mut editing = song.macros_state(0);
    editing.editing = Some((0, 3));
    Ok(vec![
        Sample { name: "chain-mixer-drums-drive", about: "From the mixer (no tab lit, Back to the mixer): the drum bus chain, Drive selected; Q-Links on its first four knobs (Mode, Drive, Tone, Bias); Tone touched: LFO 1 sweeps it (green ring), Grit (macro 2) moves Drive and Bias (badges); the EQ bypassed", ui: screen(fonts(), &mut song, Place::Mixer, Some(drive), None)? },
        Sample { name: "chain-mixer-drums-filter", about: "From the mixer: the drum bus filter, cutoff swept by LFO 2 scaled by the mod wheel; resonance on Air (macro 4)", ui: screen(fonts(), &mut song, Place::Mixer, Some(filter), None)? },
        Sample { name: "chain-mixer-keys-delay", about: "From the mixer: the keys chain, Delay's Mix page (Q-Links on that page's knobs): Mix is on Wash (macro 1) and ducked by follower 1 (a negative row)", ui: screen(fonts(), &mut song, Place::Mixer, Some(delay), None)? },
        Sample { name: "chain-sample-fx-pad", about: "In the Sample tab's FX sub-tab, under the pad bar: pad 1's own chain, with a switch to the drum track's (to agree with the sampler thread)", ui: screen(fonts(), &mut song, Place::SampleFx, Some(kick), None)? },
        Sample { name: "chain-mixer-empty", about: "From the mixer: a track with no effects yet, + Add effect; the Q-Links have nothing of the page's, so they show free", ui: screen(fonts(), &mut song, Place::Mixer, Some(empty), None)? },
        Sample { name: "chain-mixer-add-effect", about: "The effect picker, a category browser over the effect and Modulate columns (+ Add effect, or the mixer's + on a strip): Favourites, Recent and the categories with their counts; with no favourites it opens on Dynamics. A tap adds an effect at its defaults, the star keeps it, a hold or its presets line opens its presets", ui: screen(fonts(), &mut song, Place::Mixer, Some(adding), None)? },
        Sample { name: "chain-mixer-add-favourites", about: "The picker opens on Favourites once there are any (starred, in the order they were starred; Recent holds the last 12 added)", ui: screen(fonts(), &mut song, Place::Mixer, Some(starred), None)? },
        Sample { name: "chain-mixer-add-presets", about: "Plate reverb's presets, reached with SHIFT + wheel (Delay and reverb), the wheel (Plate reverb) and a push; the wheel is on Snare plate, and a push or a tap adds the plate with it (Defaults adds it as it starts)", ui: presets },
        Sample { name: "macros-1-4", about: "The macros panel over the page (a long press on a macro card; no tab lit, Back closes it): Macros 1-4 with their targets and ranges, the Q-Links on Macros 1-4, the morph 64 % of the way from Dry verse to Wash drop", ui: screen(fonts(), &mut song, Place::Over, None, Some(m0))? },
        Sample { name: "macros-5-8", about: "Macros 5-8, not yet assigned; the Q-Links on Macros 5-8", ui: screen(fonts(), &mut song, Place::Over, None, Some(m1))? },
        Sample { name: "macros-edit-target", about: "A tap on a target opens its editor over the cards: Wash's hold on the delay tone, a falling range (80 → 35 %); drag From and To, pick a curve, Flip, Remove", ui: screen(fonts(), &mut song, Place::Over, None, Some(editing))? },
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ChainAction, MacroAction};
    use mpfree_fx::picker::Section;
    use mpfree_ui::{SoftCanvas, Touch};

    fn tap(ui: &mut Ui, x: i32, y: i32) {
        ui.touch(1, Touch::Down, x, y);
        ui.touch(1, Touch::Up, x, y);
    }

    #[test]
    fn the_samples_read_real_state_and_paint() {
        let s = samples(Fonts::embedded).unwrap();
        assert_eq!(s.len(), 11);
        for mut sample in s {
            let (w, h) = sample.ui.size();
            let mut c = SoftCanvas::new(w, h);
            sample.ui.paint(&mut c);
        }
        let song = Song::new();
        let st = song.chain_state(DRUMS, 1, 0, None);
        assert_eq!(
            st.slots.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
            ["Multi filter", "Drive", "Compressor", "EQ"]
        );
        assert!(st.slots[1].macros.contains(&1), "Grit moves the drive");
        assert!(st.slots[1].modulated, "LFO 1 on its tone");
        assert!(st.params.iter().any(|p| p.macro_ == Some(1)));
        assert!(st.params.iter().any(|p| p.mod_range.is_some()));
        let m = song.macros_state(0);
        assert_eq!(m.macros[0].targets.len(), 4);
        assert_eq!(m.macros[0].targets[0].label, "Send A (Keys)");
        assert!(!m.changes.is_empty(), "the scenes differ");
        assert!((m.morph - 0.64).abs() < 1e-6);
    }

    #[test]
    fn chain_touches_become_actions() {
        let mut song = Song::new();
        let mut st = song.chain_state(DRUMS, 1, 0, None);
        st.back = Some("Mixer".into());
        let mut ui = screen(Fonts::embedded(), &mut song, Place::Mixer, Some(st), None).unwrap();
        // The Q-Links show the drive's first four knobs.
        let q = ui.get::<QLinkColumn>(QLINKS).unwrap();
        assert_eq!(
            q.cards.iter().map(|c| c.label.as_str()).collect::<Vec<_>>(),
            ["Mode", "Drive", "Tone", "Bias"]
        );
        assert_eq!(
            (q.title.as_str(), q.places[0].as_str()),
            ("Q-Links · Drive", "Drums")
        );
        let r = ui.rect_of(crate::CHAIN).unwrap();
        let l = ui.get::<ChainView>(crate::CHAIN).unwrap().layout(r);
        assert!(l.picture.is_some(), "the drive's curve above its knobs");
        // Select the compressor, then bypass it.
        let row = l.rows[2];
        tap(&mut ui, row.x + 80, row.y + 20);
        tap(&mut ui, row.right() - 20, row.y + row.h / 2);
        // Touch the second knob (Drive: the app has not sent the
        // compressor's page yet) and drag it up.
        let k = l.knobs[1];
        ui.touch(2, Touch::Down, k.x + k.w / 2, k.y + 40);
        ui.touch(2, Touch::Move, k.x + k.w / 2, k.y + 10);
        ui.touch(2, Touch::Up, k.x + k.w / 2, k.y + 10);
        let acts = crate::take_chain_actions(&mut ui);
        assert_eq!(acts[0], ChainAction::Select(2));
        assert_eq!(acts[1], ChainAction::SetOn { slot: 2, on: false });
        let ChainAction::Focus(addr) = acts[2] else {
            panic!("{acts:?}")
        };
        assert!(matches!(acts[3], ChainAction::Set { addr: a, .. } if a == addr));
        assert_eq!(acts.last(), Some(&ChainAction::Release(addr)));
        // Put the knob on macro 3, and move the selected insert up.
        let l = ui.get::<ChainView>(crate::CHAIN).unwrap().layout(r);
        tap(&mut ui, l.macros[2].x + 5, l.macros[2].y + 5);
        tap(&mut ui, l.learn.x + 5, l.learn.y + 5);
        tap(&mut ui, l.buttons[0].x + 5, l.buttons[0].y + 5);
        tap(&mut ui, l.add_lfo.x + 5, l.add_lfo.y + 5);
        let acts = crate::take_chain_actions(&mut ui);
        assert_eq!(
            acts[0],
            ChainAction::Macro {
                target: addr,
                macro_: Some(2)
            }
        );
        assert_eq!(acts[1], ChainAction::Learn(addr));
        assert_eq!(acts[2], ChainAction::Move { from: 2, to: 1 });
        assert_eq!(acts.len(), 4, "{acts:?} {:?}", l.add_lfo);
        assert!(
            matches!(acts[3], ChainAction::AddRow { target, .. } if target == addr),
            "{acts:?}"
        );
        // The effect's name opens its full page.
        tap(&mut ui, l.title.x + 10, l.title.y + 10);
        assert_eq!(
            crate::take_chain_actions(&mut ui),
            [ChainAction::OpenPage(1)]
        );
        let b = l.back.unwrap();
        tap(&mut ui, b.x + 5, b.y + 5);
        assert_eq!(crate::take_chain_actions(&mut ui), [ChainAction::Back]);
    }

    #[test]
    fn plugins_follow_the_built_in_effects_in_the_picker() {
        let mut song = Song::new();
        let plug = crate::Pick {
            id: "clap:org.example.verb",
            name: "Example Verb",
            about: "Plugin · Example",
            category: mpfree_fx::Category::Plugins,
        };
        let mut st = song.chain_state(KEYS, 0, 0, None);
        st.plugins = vec![plug];
        let all = st.picks();
        let built_in = crate::state::picks(KEYS);
        assert_eq!(all.len(), built_in.len() + 1);
        assert_eq!(&all[..built_in.len()], &built_in[..]);
        assert_eq!(all.last(), Some(&plug));
        let mut ui = screen(
            Fonts::embedded(),
            &mut song,
            Place::Mixer,
            Some(st.clone()),
            None,
        )
        .unwrap();
        let r = ui.rect_of(crate::CHAIN).unwrap();
        let add = ui
            .get::<ChainView>(crate::CHAIN)
            .unwrap()
            .layout(r)
            .add
            .unwrap();
        tap(&mut ui, add.x + 10, add.y + 10);
        assert_eq!(
            crate::take_chain_actions(&mut ui),
            [ChainAction::Picker(true)]
        );
        // The plugins have their own section, after the built-in ones.
        let l = ui.get::<ChainView>(crate::CHAIN).unwrap().layout(r);
        let pl = l.picker.as_ref().unwrap();
        let (list, _) = ui
            .get::<ChainView>(crate::CHAIN)
            .unwrap()
            .picker()
            .current(&st);
        let at = list
            .iter()
            .position(|s| *s == Section::Category(mpfree_fx::Category::Plugins))
            .expect("a Plugins section");
        let row = pl.sections[at];
        tap(&mut ui, row.x + 10, row.y + 10);
        let l = ui.get::<ChainView>(crate::CHAIN).unwrap().layout(r);
        assert_eq!(l.picks.len(), 1, "a card for the plugin");
        let t = l.picks[0];
        assert!(t.h >= 48, "the cards still take a finger: {t:?}");
        tap(&mut ui, t.x + 10, t.y + 10);
        assert_eq!(
            crate::take_chain_actions(&mut ui),
            [ChainAction::Add("clap:org.example.verb")]
        );
    }

    /// A chain editor for the keys strip with the picker open.
    fn picking(song: &mut Song, shelf: mpfree_fx::picker::Shelf) -> Ui {
        let mut st = song.chain_state(KEYS, 0, 0, None);
        st.picking = true;
        st.shelf = shelf;
        screen(Fonts::embedded(), song, Place::Mixer, Some(st), None).unwrap()
    }

    #[test]
    fn stars_keep_favourites_and_favourites_open_first() {
        let mut song = Song::new();
        let mut ui = picking(&mut song, Default::default());
        let r = ui.rect_of(crate::CHAIN).unwrap();
        let l = ui.get::<ChainView>(crate::CHAIN).unwrap().layout(r);
        let star = l.picker.as_ref().unwrap().stars[1];
        let id = ui
            .get::<ChainView>(crate::CHAIN)
            .unwrap()
            .picker()
            .picks(ui.get::<ChainView>(crate::CHAIN).unwrap().state())[1]
            .id;
        tap(&mut ui, star.x + star.w / 2, star.y + star.h / 2);
        assert_eq!(
            crate::take_chain_actions(&mut ui),
            [ChainAction::Favourite { id, on: true }]
        );
        let v = ui.get::<ChainView>(crate::CHAIN).unwrap();
        assert!(v.state().shelf.is_favourite(id), "shown at once");
        assert!(v.state().picking, "a star doesn't add");
        tap(&mut ui, star.x + star.w / 2, star.y + star.h / 2);
        assert_eq!(
            crate::take_chain_actions(&mut ui),
            [ChainAction::Favourite { id, on: false }]
        );
        // A picker opened with favourites starts on them, in their order.
        let mut shelf = mpfree_fx::picker::Shelf::default();
        shelf.set_favourite("mpfree.fx.spring", true);
        shelf.set_favourite("mpfree.fx.gate", true);
        shelf.set_favourite("clap:not.here", true);
        let ui = picking(&mut song, shelf);
        let v = ui.get::<ChainView>(crate::CHAIN).unwrap();
        let ids: Vec<_> = v.picker().picks(v.state()).iter().map(|p| p.id).collect();
        assert_eq!(ids, ["mpfree.fx.spring", "mpfree.fx.gate"]);
    }

    #[test]
    fn a_hold_or_the_presets_line_opens_presets_and_a_tap_adds_one() {
        let mut song = Song::new();
        let mut ui = picking(&mut song, Default::default());
        open_section(&mut ui, Section::Category(mpfree_fx::Category::DelayReverb));
        let t = tile_of(&ui, "mpfree.fx.plate").unwrap();
        // Held for half a second: the presets open instead of an add.
        ui.touch(1, Touch::Down, t.x + 20, t.y + 20);
        ui.get_mut::<ChainView>(crate::CHAIN)
            .unwrap()
            .age_press(0.6);
        ui.touch(1, Touch::Up, t.x + 20, t.y + 20);
        assert!(crate::take_chain_actions(&mut ui).is_empty());
        let r = ui.rect_of(crate::CHAIN).unwrap();
        let v = ui.get::<ChainView>(crate::CHAIN).unwrap();
        assert_eq!(v.picker().presets, Some("mpfree.fx.plate"));
        let pl = v.layout(r).picker.unwrap();
        let n = mpfree_fx::picker::presets("mpfree.fx.plate").len();
        assert_eq!(pl.tiles.len(), n + 1, "Defaults and every preset");
        assert!(v.layout(r).picks.is_empty());
        // A preset row adds the plate with that preset.
        let row = pl.tiles[2];
        tap(&mut ui, row.x + 10, row.y + 10);
        assert_eq!(
            crate::take_chain_actions(&mut ui),
            [ChainAction::AddPreset {
                id: "mpfree.fx.plate",
                preset: 1
            }]
        );
        assert!(!ui.get::<ChainView>(crate::CHAIN).unwrap().state().picking);
        // The presets line opens them too; Back goes back; Defaults adds
        // at defaults.
        let mut ui = picking(&mut song, Default::default());
        open_section(&mut ui, Section::Category(mpfree_fx::Category::DelayReverb));
        let l = ui.get::<ChainView>(crate::CHAIN).unwrap().layout(r);
        let pl = l.picker.unwrap();
        let k = {
            let v = ui.get::<ChainView>(crate::CHAIN).unwrap();
            let picks = v.picker().picks(v.state());
            picks.iter().position(|p| p.id == "mpfree.fx.room").unwrap()
        };
        let chip = pl.chips[k].expect("the room has presets");
        tap(&mut ui, chip.x + 10, chip.y + chip.h / 2);
        let v = ui.get::<ChainView>(crate::CHAIN).unwrap();
        assert_eq!(v.picker().presets, Some("mpfree.fx.room"));
        let pl = v.layout(r).picker.unwrap();
        let back = pl.back.unwrap();
        tap(&mut ui, back.x + 5, back.y + 5);
        assert_eq!(
            ui.get::<ChainView>(crate::CHAIN).unwrap().picker().presets,
            None
        );
        tap(&mut ui, chip.x + 10, chip.y + chip.h / 2);
        let pl = ui
            .get::<ChainView>(crate::CHAIN)
            .unwrap()
            .layout(r)
            .picker
            .unwrap();
        tap(&mut ui, pl.tiles[0].x + 10, pl.tiles[0].y + 10);
        assert_eq!(
            crate::take_chain_actions(&mut ui),
            [ChainAction::Add("mpfree.fx.room")]
        );
        // A finger that slides off the tile adds nothing.
        let mut ui = picking(&mut song, Default::default());
        let t = ui.get::<ChainView>(crate::CHAIN).unwrap().layout(r).picks[0];
        ui.touch(1, Touch::Down, t.x + 20, t.y + 20);
        ui.touch(1, Touch::Up, t.x - 30, t.y + 20);
        assert!(crate::take_chain_actions(&mut ui).is_empty());
    }

    #[test]
    fn the_wheel_moves_through_tiles_and_shift_through_sections() {
        let mut song = Song::new();
        let mut ui = picking(&mut song, Default::default());
        let v = ui.get_mut::<ChainView>(crate::CHAIN).unwrap();
        assert!(v.wheel(1, true), "the picker takes the wheel");
        let st = v.state().clone();
        let (list, at) = v.picker().current(&st);
        assert_eq!(list[at], Section::Category(mpfree_fx::Category::EqFilter));
        v.wheel(-9, true);
        assert_eq!(v.picker().current(&st).1, 0, "stops at the first");
        v.wheel(4, true);
        v.wheel(2, true);
        assert_eq!(
            v.picker().current(&st).0[v.picker().current(&st).1],
            Section::Category(mpfree_fx::Category::DelayReverb)
        );
        // The first push lights the first tile; the wheel moves it.
        assert!(v.push_wheel());
        assert_eq!(v.picker().highlight, Some(0));
        v.wheel(100, false);
        let n = v.picker().picks(&st).len();
        assert_eq!(v.picker().highlight, Some(n - 1), "stops at the last");
        let plate = v
            .picker()
            .picks(&st)
            .iter()
            .position(|p| p.id == "mpfree.fx.plate")
            .unwrap();
        v.wheel(plate as i32 - (n as i32 - 1), false);
        // A push opens the plate's presets on Defaults; the wheel and a
        // push add the plate with the second preset.
        v.push_wheel();
        assert_eq!(v.picker().presets, Some("mpfree.fx.plate"));
        assert_eq!(v.picker().highlight, Some(0));
        v.wheel(2, false);
        v.push_wheel();
        assert_eq!(
            v.take_actions(),
            [ChainAction::AddPreset {
                id: "mpfree.fx.plate",
                preset: 1
            }]
        );
        assert!(!v.wheel(1, false), "a shut picker gives the wheel back");
        assert!(!v.push_wheel());
    }

    #[test]
    fn a_long_section_pages() {
        let mut song = Song::new();
        let mut shelf = mpfree_fx::picker::Shelf::default();
        for p in crate::state::picks(KEYS) {
            shelf.set_favourite(p.id, true);
        }
        let mut ui = picking(&mut song, shelf);
        let r = ui.rect_of(crate::CHAIN).unwrap();
        let pl = ui
            .get::<ChainView>(crate::CHAIN)
            .unwrap()
            .layout(r)
            .picker
            .unwrap();
        assert!(
            pl.pages > 1,
            "{} favourites",
            crate::state::picks(KEYS).len()
        );
        assert_eq!(pl.page, 0);
        let per = pl.tiles.len();
        let next = pl.next.unwrap();
        tap(&mut ui, next.x + 10, next.y + 10);
        let pl = ui
            .get::<ChainView>(crate::CHAIN)
            .unwrap()
            .layout(r)
            .picker
            .unwrap();
        assert_eq!((pl.page, pl.first), (1, per));
        // The wheel's highlight brings its page.
        let v = ui.get_mut::<ChainView>(crate::CHAIN).unwrap();
        v.wheel(1, false);
        assert_eq!(v.layout(r).picker.unwrap().page, 0);
        // Every tile, on every page, still takes a finger.
        for t in &pl.tiles {
            assert!(t.h >= crate::picker::TILE_H && t.w >= 150, "{t:?}");
        }
    }

    /// The picker's tile for `id` in the section shown, if it is on this
    /// page.
    fn tile_of(ui: &Ui, id: &str) -> Option<Rect> {
        let v = ui.get::<ChainView>(crate::CHAIN).unwrap();
        let l = v.layout(ui.rect_of(crate::CHAIN).unwrap());
        let pl = l.picker?;
        let picks = v.picker().picks(v.state());
        let i = picks.iter().position(|p| p.id == id)?;
        pl.tiles.get(i.checked_sub(pl.first)?).copied()
    }

    /// Taps section `s` in the open picker.
    fn open_section(ui: &mut Ui, s: Section) {
        let v = ui.get::<ChainView>(crate::CHAIN).unwrap();
        let l = v.layout(ui.rect_of(crate::CHAIN).unwrap());
        let (list, _) = v.picker().current(v.state());
        let i = list.iter().position(|x| *x == s).unwrap();
        let row = l.picker.unwrap().sections[i];
        tap(ui, row.x + 10, row.y + 10);
    }

    #[test]
    fn the_picker_adds_an_effect_or_cancels() {
        let mut song = Song::new();
        let st = song.chain_state(KEYS, 0, 0, None);
        let mut ui = screen(Fonts::embedded(), &mut song, Place::Mixer, Some(st), None).unwrap();
        let r = ui.rect_of(crate::CHAIN).unwrap();
        let l = ui.get::<ChainView>(crate::CHAIN).unwrap().layout(r);
        let add = l.add.unwrap();
        tap(&mut ui, add.x + 10, add.y + 10);
        // The app hears the picker open, so its next state keeps it open.
        assert_eq!(
            crate::take_chain_actions(&mut ui),
            [ChainAction::Picker(true)]
        );
        let l = ui.get::<ChainView>(crate::CHAIN).unwrap().layout(r);
        assert!(ui.get::<ChainView>(crate::CHAIN).unwrap().state().picking);
        assert!(l.knobs.is_empty(), "the picker takes the knobs' place");
        // With no favourites it opens on the first category.
        let pl = l.picker.as_ref().unwrap();
        assert_eq!(pl.section, 2);
        let dynamics = crate::picker::section_picks(
            ui.get::<ChainView>(crate::CHAIN).unwrap().state(),
            Section::Category(mpfree_fx::Category::Dynamics),
        );
        assert_eq!(l.picks.len(), dynamics.len());
        assert!(!dynamics.iter().any(|p| p.id == "mpfree.fx.master_safety"));
        // The state the app sends while the picker is open keeps it open.
        crate::update_chain(&mut ui, song.chain_state(KEYS, 0, 0, None));
        assert!(ui.get::<ChainView>(crate::CHAIN).unwrap().state().picking);
        open_section(&mut ui, Section::Category(mpfree_fx::Category::DelayReverb));
        let t = tile_of(&ui, "mpfree.fx.cloud").unwrap();
        tap(&mut ui, t.x + 10, t.y + 10);
        assert_eq!(
            crate::take_chain_actions(&mut ui),
            [ChainAction::Add("mpfree.fx.cloud")]
        );
        assert!(!ui.get::<ChainView>(crate::CHAIN).unwrap().state().picking);
        assert!(mpfree_fx_ui::make("mpfree.fx.cloud").is_some());
        // Open again (on the section left) and cancel: the app hears both.
        tap(&mut ui, add.x + 10, add.y + 10);
        assert!(tile_of(&ui, "mpfree.fx.cloud").is_some(), "same section");
        let c = ui
            .get::<ChainView>(crate::CHAIN)
            .unwrap()
            .layout(r)
            .cancel
            .unwrap();
        tap(&mut ui, c.x + 5, c.y + 5);
        assert_eq!(
            crate::take_chain_actions(&mut ui),
            [ChainAction::Picker(true), ChainAction::Picker(false)]
        );
        assert!(!ui.get::<ChainView>(crate::CHAIN).unwrap().state().picking);
        // Every pick is an effect the catalogue makes, with a line about it.
        for p in crate::state::picks(Strip::Master) {
            assert!(mpfree_fx_ui::make(p.id).is_some(), "{}", p.id);
            assert!(!p.about.is_empty(), "{}", p.id);
        }
    }

    #[test]
    fn the_sample_tab_chain_switches_between_pad_and_track() {
        let mut song = Song::new();
        let mut st = song.chain_state(KICK, 0, 0, None);
        st.scopes = vec!["Pad 1 · Kick".into(), "Track · Drums".into()];
        let mut ui = screen(
            Fonts::embedded(),
            &mut song,
            Place::SampleFx,
            Some(st),
            None,
        )
        .unwrap();
        let r = ui.rect_of(crate::CHAIN).unwrap();
        let l = ui.get::<ChainView>(crate::CHAIN).unwrap().layout(r);
        assert!(l.back.is_none());
        tap(&mut ui, l.scopes[1].x + 5, l.scopes[1].y + 5);
        assert_eq!(crate::take_chain_actions(&mut ui), [ChainAction::Scope(1)]);
        assert_eq!(ui.get::<ChainView>(crate::CHAIN).unwrap().state().scope, 1);
    }

    #[test]
    fn macro_touches_become_actions() {
        let mut song = Song::new();
        let st = song.macros_state(0);
        let mut ui = screen(Fonts::embedded(), &mut song, Place::Over, None, Some(st)).unwrap();
        let q = ui.get::<QLinkColumn>(QLINKS).unwrap();
        assert_eq!(
            q.cards.iter().map(|c| c.label.as_str()).collect::<Vec<_>>(),
            ["Wash", "Grit", "Pump", "Air"]
        );
        let r = ui.rect_of(crate::MACROS_W).unwrap();
        let l = crate::macros::layout(r);
        let (kx, ky, _) = l.knobs[1];
        let v0 = ui
            .get::<MacrosView>(crate::MACROS_W)
            .unwrap()
            .state()
            .macros[1]
            .value;
        ui.touch(1, Touch::Down, kx as i32, ky as i32);
        ui.touch(1, Touch::Move, kx as i32, ky as i32 - 30);
        ui.touch(1, Touch::Up, kx as i32, ky as i32 - 30);
        tap(
            &mut ui,
            l.slider.x + l.slider.w / 2,
            l.slider.y + l.slider.h / 2,
        );
        tap(&mut ui, l.save[1].x + 4, l.save[1].y + 4);
        tap(&mut ui, l.banks[1].x + 4, l.banks[1].y + 4);
        let acts = crate::take_macro_actions(&mut ui);
        assert!(
            matches!(acts[0], MacroAction::Macro { m: 1, value } if (value - v0 - 0.1).abs() < 0.01),
            "{acts:?}"
        );
        assert!(matches!(acts[1], MacroAction::Morph(p) if (p - 0.5).abs() < 0.05));
        assert_eq!(acts[2], MacroAction::Capture(1));
        assert_eq!(acts[3], MacroAction::Bank(1));
        // Back closes the panel; a target row opens its editor.
        tap(&mut ui, l.back.x + 5, l.back.y + 5);
        tap(&mut ui, l.banks[0].x + 4, l.banks[0].y + 4);
        assert_eq!(
            crate::take_macro_actions(&mut ui),
            [MacroAction::Back, MacroAction::Bank(0)]
        );
        let c = l.cards[0];
        tap(&mut ui, c.x + 40, c.y + 80 + 30 + 10);
        fn view(ui: &Ui) -> &MacrosView {
            ui.get::<MacrosView>(crate::MACROS_W).unwrap()
        }
        assert_eq!(view(&ui).state().editing, Some((0, 1)));
        let map = song.engine.macro_maps(0).nth(1).unwrap();
        // Drag the To handle to the end, pick Exp, flip, then remove.
        let e = &l.editor;
        let tx = e.track.x + (map.to * e.track.w as f32) as i32;
        let ty = e.track.y + e.track.h / 2;
        ui.touch(1, Touch::Down, tx, ty);
        ui.touch(1, Touch::Move, e.track.right(), ty);
        ui.touch(1, Touch::Up, e.track.right(), ty);
        tap(&mut ui, e.curves[1].x + 5, e.curves[1].y + 5);
        tap(&mut ui, e.flip.x + 5, e.flip.y + 5);
        let acts = crate::take_macro_actions(&mut ui);
        let MacroAction::Map { m: 0, map: last } = *acts.last().unwrap() else {
            panic!("{acts:?}")
        };
        assert_eq!(last.target, map.target);
        assert_eq!((last.from, last.to), (1.0, map.from), "{acts:?}");
        assert_eq!(last.curve, MapCurve::Exp);
        let tv = &view(&ui).state().macros[0].targets[1];
        assert_eq!((tv.from, tv.to, tv.curve), (1.0, map.from, MapCurve::Exp));
        assert!(tv.range_text.contains('→'));
        tap(&mut ui, e.remove.x + 5, e.remove.y + 5);
        assert_eq!(
            crate::take_macro_actions(&mut ui),
            [MacroAction::Unmap(map.target)]
        );
        assert_eq!(view(&ui).state().editing, None);
        assert_eq!(view(&ui).state().macros[0].targets.len(), 3);
        // The app's actions do to an engine what the panel shows.
        let mut engine = Song::new().engine;
        engine
            .change(mpfree_mod::Change::Map { m: 0, map: last })
            .unwrap();
        let back = engine
            .macro_maps(0)
            .find(|x| x.target == map.target)
            .unwrap();
        assert_eq!(
            (back.from, back.to, back.curve),
            (1.0, map.from, MapCurve::Exp)
        );
    }
}
