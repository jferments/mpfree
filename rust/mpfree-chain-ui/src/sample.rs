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
    // The top bar: Sample lit for its FX sub-tab; Mix for the editor
    // itself (the chain editor is the Mix tab's).
    let mut header = Header::app(if place == Place::SampleFx {
        APP_TABS[5]
    } else {
        APP_TABS[6]
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
            for p in c.params.iter().chain(&c.main_four) {
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
        let cards = match c.selected {
            Some(slot) => picture(song.chain(c.strip), slot, "", &Theme::dark(), RATE, 92.0),
            None => Vec::new(),
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
        st.strip_name = st.title.trim_end_matches(" FX").to_string();
        st.strip_kind = match st.strip {
            Strip::Track(t) => format!("Track {}", t + 1),
            Strip::Bus(b) => format!("Submix {}", b + 1),
            Strip::Pad(p) => format!("Pad {}", p + 1),
            _ => String::new(),
        };
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
        0,
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
    let empty = ChainState::from_chain(
        &chain(vec![]),
        Strip::Track(6),
        "Vocal chop FX",
        8,
        &song.engine,
        None,
        0,
        None,
    );
    let empty = from_mixer(empty);
    let mut adding = from_mixer(song.chain_state(KEYS, 1, 0, None));
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
    // The Reverb's presets, opened with the wheel: SHIFT + wheel down to
    // Delay and reverb, the wheel to Reverb, a push.
    let mut presets = screen(fonts(), &mut song, Place::Mixer, Some(adding.clone()), None)?;
    if let Some(v) = presets.get_mut::<ChainView>(crate::CHAIN) {
        v.wheel(4, true);
        v.wheel(2, false);
        v.push_wheel();
        v.wheel(6, false);
    }
    // The strip button's list over the keys chain: the tracks.
    let keys = from_mixer(song.chain_state(KEYS, 1, 0, None));
    let mut listed = keys.clone();
    let pick = |t: u8, name: &str, sub: &str| crate::StripPick {
        strip: Strip::Track(t),
        name: name.into(),
        sub: sub.into(),
    };
    listed.list = Some(crate::StripList {
        group: 0,
        counts: [8, 16, 1, 4, 8, 1],
        strips: vec![
            pick(0, "Drums", "4 effects"),
            pick(1, "Synth", "no effects"),
            pick(2, "Loop", "1 effect"),
            pick(3, "Audio", "no effects"),
            pick(4, "Keys", "3 effects"),
            pick(5, "Bass", "2 effects"),
            pick(6, "Vocal chop", "muted · no effects"),
            pick(7, "Pad", "no effects"),
        ],
    });
    let mut strips = screen(fonts(), &mut song, Place::Mixer, Some(keys), None)?;
    let r = strips.rect_of(crate::CHAIN).ok_or("the chain editor")?;
    let b = strips
        .get::<crate::ChainView>(crate::CHAIN)
        .and_then(|v| v.layout(r).strip)
        .ok_or("the strip button")?;
    strips.touch(1, mpfree_ui::Touch::Down, b.x + 5, b.y + 5);
    strips.touch(1, mpfree_ui::Touch::Up, b.x + 5, b.y + 5);
    let _ = crate::take_chain_actions(&mut strips);
    crate::update_chain(&mut strips, listed);
    if let Some(h) = strips.get_mut::<Label>("hint") {
        h.text = crate::STRIPS_HINT.to_string();
    }
    // A long effect on one page (the drum bus EQ), and an effect of
    // several pages: a Channel strip with the Vocal preset on its first
    // (its stages as sections, with their switches).
    // The drum bus EQ shaped as on the approved mockups: a low cut, a
    // low bump, a mud cut (band 3, picked), presence and a high cut.
    {
        use mpfree_fx::ids::eq as q;
        let e = song.drums.node_mut(3).ok_or("the drum bus EQ")?;
        for (id, v) in [
            (q::LOW_CUT, 35.0),
            (q::LOW_SLOPE, 1.0),
            (q::HIGH_CUT, 9_000.0),
            (q::HIGH_SLOPE, 1.0),
            (q::band_type(0), 0.0),
            (q::freq(0), 90.0),
            (q::gain(0), 3.0),
            (q::q(0), 1.0),
            (q::band_type(1), 0.0),
            (q::freq(1), 320.0),
            (q::gain(1), -3.5),
            (q::q(1), 1.4),
            (q::band_type(2), 0.0),
            (q::freq(2), 1_200.0),
            (q::gain(2), 2.0),
            (q::q(2), 2.0),
            (q::band_use(3), q::USE_UNUSED),
        ] {
            e.set_param(id, v);
        }
    }
    let eq_uid = song.drums.uid(3).ok_or("the drum bus EQ")?;
    let mud = ParamAddr::new(Owner::Insert(DRUMS, eq_uid), mpfree_fx::ids::eq::gain(1));
    let eq = from_mixer(song.chain_state(DRUMS, 3, 0, Some(mud)));
    // A band held: band 2's dot dragged down and left (board C).
    let mut held = screen(fonts(), &mut song, Place::Mixer, Some(eq.clone()), None)?;
    {
        let r = held.rect_of(crate::CHAIN).ok_or("the chain editor")?;
        let v = held
            .get::<crate::ChainView>(crate::CHAIN)
            .ok_or("the chain editor")?;
        let page = v.layout(r).page;
        let el = crate::eq_page::layout(page, v.eq_page());
        let (fx, fy) = v.eq_page().dot(crate::eq_page::Band::Band(1));
        let x = el.plot.x + (fx * el.plot.w as f32) as i32;
        let y = el.plot.bottom() - (fy * el.plot.h as f32) as i32;
        held.touch(1, mpfree_ui::Touch::Down, x, y);
        held.touch(1, mpfree_ui::Touch::Move, x - 30, y + 20);
        held.touch(1, mpfree_ui::Touch::Move, x - 40, y + 30);
        let _ = crate::take_chain_actions(&mut held);
    }
    let mut eq_ui = screen(fonts(), &mut song, Place::Mixer, Some(eq), None)?;
    for ui in [&mut held, &mut eq_ui] {
        let text = crate::chain_hint(ui).to_string();
        if let Some(h) = ui.get_mut::<Label>("hint") {
            h.text = text;
        }
    }
    let mut strip =
        mpfree_fx::registry::make("mpfree.fx.channel_strip").ok_or("no channel strip")?;
    if let Some(p) = mpfree_fx::picker::presets("mpfree.fx.channel_strip")
        .iter()
        .find(|p| p.name == "Vocal")
    {
        for &(id, v) in p.values {
            strip.set_param(id, v);
        }
    }
    let strip_slot = song.keys.push(strip);
    let strip_st = from_mixer(song.chain_state(KEYS, strip_slot, 0, None));
    // Drive's list of modes (its chip tapped), and the delay's Mix with
    // its mod rows open (the line under its name tapped).
    let tap_on =
        |ui: &mut Ui, at: fn(&crate::view::Layout) -> Option<Rect>| -> Result<(), String> {
            let r = ui.rect_of(crate::CHAIN).ok_or("the chain editor")?;
            let b = ui
                .get::<crate::ChainView>(crate::CHAIN)
                .and_then(|v| at(&v.layout(r)))
                .ok_or("nothing to tap")?;
            ui.touch(1, mpfree_ui::Touch::Down, b.x + 5, b.y + 5);
            ui.touch(1, mpfree_ui::Touch::Up, b.x + 5, b.y + 5);
            let _ = crate::take_chain_actions(ui);
            Ok(())
        };
    let mut modes = screen(fonts(), &mut song, Place::Mixer, Some(drive.clone()), None)?;
    tap_on(&mut modes, |l| l.mode_label)?;
    let mut rows = screen(fonts(), &mut song, Place::Mixer, Some(delay.clone()), None)?;
    tap_on(&mut rows, |l| Some(l.rows_link))?;
    // The drawings the new pages brought, each on a song of its own so
    // the screens above keep their chains: a Plate, a three-head Tape
    // delay, the Multi-tap delay and the Channel strip's Compressor page.
    let drawn = |id: &str, preset: Option<&str>, group: usize| -> Result<Ui, String> {
        let mut other = Song::new();
        let mut n = mpfree_fx::registry::make(id).ok_or("no such effect")?;
        if let Some(p) = preset.and_then(|name| {
            mpfree_fx::picker::presets(id)
                .iter()
                .find(|p| p.name == name)
        }) {
            for &(pid, v) in p.values {
                n.set_param(pid, v);
            }
        }
        let slot = other.keys.push(n);
        let st = from_mixer(other.chain_state(KEYS, slot, group, None));
        screen(fonts(), &mut other, Place::Mixer, Some(st), None)
    };
    let plate = drawn("mpfree.fx.plate", Some("Vocal plate"), 0)?;
    let room = drawn("mpfree.fx.room", None, 0)?;
    let spring = drawn("mpfree.fx.spring", Some("Dub splash"), 0)?;
    let tape = drawn("mpfree.fx.tape_delay", Some("Space heads"), 0)?;
    let mut tape_presets = drawn("mpfree.fx.tape_delay", Some("Space heads"), 0)?;
    tap_on(&mut tape_presets, |l| l.preset_chip)?;
    let multitap = drawn("mpfree.fx.multitap", None, 0)?;
    let strip_comp = drawn("mpfree.fx.channel_strip", Some("Vocal"), 2)?;
    let m0 = song.macros_state(0);
    let m1 = song.macros_state(1);
    let mut editing = song.macros_state(0);
    editing.editing = Some((0, 3));
    Ok(vec![
        Sample { name: "chain-mixer-drums-drive", about: "The effect frame from the mixer (no tab lit, Back to the mixer): the drum bus chain down the left (the EQ bypassed), Drive's page in sections in the middle, its Q-Links (Drive, Tone, Bias, Mix) at the right, the Modulate row under the page. Drive is in its Soft mode: ten modes don't fit as buttons, so the chip beside its name opens the list. Tone touched: LFO 1 sweeps it (green ring, and the Modulate row's line), Grit (macro 2) moves Drive and Bias (M2 badges)", ui: screen(fonts(), &mut song, Place::Mixer, Some(drive), None)? },
        Sample { name: "chain-mixer-drums-drive-modes", about: "Drive's chip tapped: its ten modes as a list over the page, the one it is in lit; a tap switches, a touch anywhere else closes it", ui: modes },
        Sample { name: "chain-mixer-drums-filter", about: "From the mixer: the drum bus filter, its response drawn in its section; Cutoff swept by LFO 2 scaled by the mod wheel, Resonance on Air (macro 4)", ui: screen(fonts(), &mut song, Place::Mixer, Some(filter), None)? },
        Sample { name: "chain-mixer-keys-delay", about: "From the mixer: the keys chain's Delay, all its controls on one page; Mix touched: it is on Wash (macro 1) and ducked by follower 1 (a negative row), which the Modulate row's line says", ui: screen(fonts(), &mut song, Place::Mixer, Some(delay), None)? },
        Sample { name: "chain-mixer-keys-delay-rows", about: "The line under Mix tapped: its mod rows over the page (drag a bar for the depth, x takes a row off; + LFO and + Follower add one); the Modulate row still works under it", ui: rows },
        Sample { name: "chain-mixer-drums-eq", about: "The EQ's page: its whole curve over each band's own shape, a dot per band in use (LC and HC for the cuts), band 2 picked (ringed, its shape stronger). Under the graph: a chip per band and + Band, then band 2's type, Freq, Gain, Q, On and Remove. The Q-Links turn its Freq, Gain and Q, and the Output", ui: eq_ui },
        Sample { name: "chain-mixer-drums-eq-held", about: "Band 2's dot held and dragged down and left: it follows the finger, frequency across and gain up, with a readout over it", ui: held },
        Sample { name: "chain-mixer-keys-strip", about: "An effect of several pages: a Channel strip (Vocal preset) on its first page, a tab per page beside its name. Its stages are sections with their switches (the gate off here, so its controls are dim) and Open › to each stage's page. The Q-Links are the strip's four on every page: Comp thresh, Makeup, Low cut, High gain", ui: screen(fonts(), &mut song, Place::Mixer, Some(strip_st), None)? },
        Sample { name: "chain-sample-fx-pad", about: "In the Sample tab's FX sub-tab, under the pad bar: pad 1's own chain, with a switch to the drum track's (to agree with the sampler thread)", ui: screen(fonts(), &mut song, Place::SampleFx, Some(kick), None)? },
        Sample { name: "chain-mixer-empty", about: "From the mixer: a track with no effects yet, + Add effect; the Q-Links have nothing of the page's, so they show free", ui: screen(fonts(), &mut song, Place::Mixer, Some(empty), None)? },
        Sample { name: "chain-mixer-add-effect", about: "The effect picker, a category browser over the page (+ Add effect, or the mixer's + on a strip): Favourites, Recent and the categories with their counts; with no favourites it opens on Dynamics. A tap adds an effect at its defaults, the star keeps it, a hold or its presets line opens its presets", ui: screen(fonts(), &mut song, Place::Mixer, Some(adding), None)? },
        Sample { name: "chain-mixer-add-favourites", about: "The picker opens on Favourites once there are any (starred, in the order they were starred; Recent holds the last 12 added)", ui: screen(fonts(), &mut song, Place::Mixer, Some(starred), None)? },
        Sample { name: "chain-mixer-add-presets", about: "The Reverb's presets, every mode's with its mode under the name, reached with SHIFT + wheel (Delay and reverb), the wheel (Reverb) and a push; the wheel is on Snare plate, and a push or a tap adds the Reverb as a Plate with it (Defaults adds a Room as it starts)", ui: presets },
        Sample { name: "chain-mixer-keys-plate", about: "The Plate (Vocal plate preset): its decay to -60 dB after the pre-delay, and its tone (the low cut and Tone, with the damping's loss each time round, dashed)", ui: plate },
        Sample { name: "chain-mixer-keys-room", about: "The Room: the pre-delay, its early reflections (green) and the tail to -60 dB; its tone, the low cut and how much sooner the highs die over 100 ms (dashed)", ui: room },
        Sample { name: "chain-mixer-keys-spring", about: "The Spring (Dub splash preset): its decay, with the spring's echo coming back each Length", ui: spring },
        Sample { name: "chain-mixer-keys-tape", about: "The Tape delay (Space heads preset, three heads): each head's first echo and the loop's repeats from all three; the tone one pass keeps, and four passes' (dashed)", ui: tape },
        Sample { name: "chain-mixer-keys-tape-presets", about: "The preset's name in the header tapped: the Tape delay's presets over the page (Defaults, then its own), the one it plays lit; a tap loads one (its Mix stays), a touch anywhere else closes the list", ui: tape_presets },
        Sample { name: "chain-mixer-keys-multitap", about: "The Multi-tap delay: its four taps on the beat grid (left in the accent, right in green) and the pattern again from the last, as loud as Feedback", ui: multitap },
        Sample { name: "chain-mixer-keys-strip-comp", about: "The Channel strip (Vocal) on its Compressor page: the dynamics curve, the gate's drop under its threshold and the compressor over its own, with makeup", ui: strip_comp },
        Sample { name: "chain-strip-list", about: "The strip button's list over the keys chain: the groups down the left, the tracks as tiles with what each holds (the open one outlined); a tap opens that strip's chain", ui: strips },
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

    /// On every page of every effect, each section holds its controls
    /// inside its box, and each section whose plan names a drawing has
    /// room to draw it (a section too full for its controls and a drawing
    /// drops the drawing).
    #[test]
    fn every_section_holds_its_controls_and_drawings() {
        let mut bad = Vec::new();
        for plan in mpfree_fx::page_plan::PLANS {
            for (group, page) in plan.pages.iter().enumerate() {
                let mut song = Song::new();
                let n = mpfree_fx::registry::make(plan.id).unwrap();
                let slot = song.keys.push(n);
                let st = song.chain_state(KEYS, slot, group, None);
                let ui =
                    screen(Fonts::embedded(), &mut song, Place::Mixer, Some(st), None).unwrap();
                let r = ui.rect_of(crate::CHAIN).unwrap();
                let v = ui.get::<ChainView>(crate::CHAIN).unwrap();
                let at = v.layout(r).page;
                let sv = v.page();
                for (s, g) in sv.sections.iter().zip(sv.geometry(at)) {
                    if !s.cards.is_empty() && g.drawing.is_none() {
                        bad.push(format!(
                            "{} {}: no room to draw in {}",
                            plan.id, page.name, s.title
                        ));
                    }
                    for (c, cg) in s.controls.iter().zip(&g.controls) {
                        let r = cg.cell;
                        if r.w > 0 && g.rect.union(r) != g.rect {
                            bad.push(format!(
                                "{} {}: {} spills out of {}",
                                plan.id, page.name, c.label, s.title
                            ));
                        }
                    }
                }
            }
        }
        assert!(bad.is_empty(), "{}", bad.join("\n"));
    }

    #[test]
    fn the_samples_read_real_state_and_paint() {
        let s = samples(Fonts::embedded).unwrap();
        assert_eq!(s.len(), 24);
        for mut sample in s {
            let (w, h) = sample.ui.size();
            let mut c = SoftCanvas::new(w, h);
            sample.ui.paint(&mut c);
        }
        let song = Song::new();
        let st = song.chain_state(DRUMS, 1, 0, None);
        assert_eq!(
            st.slots.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
            ["Filter · Multi", "Drive · Soft", "Compressor", "EQ"]
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
    fn the_data_wheel_turns_a_knob_as_a_finger_would() {
        let mut song = Song::new();
        let st = song.chain_state(DRUMS, 1, 0, None);
        let mut ui = screen(Fonts::embedded(), &mut song, Place::Mixer, Some(st), None).unwrap();
        let mut c = mpfree_ui::SoftCanvas::new(1280, 800);
        ui.paint(&mut c);
        let r = ui.rect_of(crate::CHAIN).unwrap();
        let v = ui.get::<ChainView>(crate::CHAIN).unwrap();
        let knobs: Vec<Rect> = (0..2).map(|k| v.control_rect(r, k).unwrap()).collect();
        let param = |ui: &Ui, i: usize| {
            ui.get::<ChainView>(crate::CHAIN).unwrap().state().params[i].clone()
        };
        // The Drive's Drive knob (its Mode is the chooser, not a knob).
        let k = knobs[0];
        let t = ui.focus_at(k.x + k.w / 2, k.y + k.h / 2).unwrap();
        assert_eq!((t.id, t.index), ("knob", 0));
        assert!(ui.focus_steps());
        let before = param(&ui, 0);
        assert!(ui.focus_step(2).is_some());
        let acts = crate::take_chain_actions(&mut ui);
        let addr = before.addr;
        let want = before.info.from_unit(before.pos + 0.02);
        assert_eq!(
            acts,
            vec![
                ChainAction::Focus(addr),
                ChainAction::Set { addr, value: want },
                ChainAction::Release(addr)
            ]
        );
        // Tone is at the top of its travel: up does nothing, down moves it.
        let k = knobs[1];
        ui.focus_at(k.x + k.w / 2, k.y + k.h / 2);
        let tone = param(&ui, 1);
        assert_eq!(tone.pos, 1.0, "{}", tone.info.name);
        assert!(ui.focus_step(1).is_none());
        assert!(crate::take_chain_actions(&mut ui).is_empty());
        assert!(ui.focus_step(-1).is_some());
        let acts = crate::take_chain_actions(&mut ui);
        let Some(ChainAction::Set { value, .. }) = acts.get(1) else {
            panic!("{acts:?}")
        };
        assert!(*value < tone.value, "{} -> {value}", tone.value);
    }

    /// Jay, 14 Nov 12:03Z: drag an effect up or down its chain. It lifts
    /// once the finger has moved more than a fingertip, lands where the
    /// finger lifts, and a cancel or a small wobble only picks it. The
    /// wheel moves the picked one a place a detent, the outline with it.
    #[test]
    fn an_effect_drags_along_its_chain() {
        let mut song = Song::new();
        let st = song.chain_state(DRUMS, 1, 0, None);
        let mut ui = screen(Fonts::embedded(), &mut song, Place::Mixer, Some(st), None).unwrap();
        let mut c = SoftCanvas::new(1280, 800);
        ui.paint(&mut c);
        let r = ui.rect_of(crate::CHAIN).unwrap();
        let rows = ui
            .get::<ChainView>(crate::CHAIN)
            .unwrap()
            .layout(r)
            .rows
            .clone();
        let names = |ui: &Ui| {
            let v = ui.get::<ChainView>(crate::CHAIN).unwrap();
            let names: Vec<String> = v.state().slots.iter().map(|s| s.name.clone()).collect();
            (names, v.state().selected)
        };
        let mid = |i: usize| rows[i].y + rows[i].h / 2;
        let x = rows[0].x + 60;
        // Multi filter, carried down past Drive and Compressor.
        ui.touch(3, Touch::Down, x, mid(0));
        ui.touch(3, Touch::Move, x, mid(1));
        ui.touch(3, Touch::Move, x, mid(2));
        ui.paint(&mut c);
        ui.touch(3, Touch::Up, x, mid(2));
        assert_eq!(
            crate::take_chain_actions(&mut ui),
            [ChainAction::Select(0), ChainAction::Move { from: 0, to: 2 }]
        );
        let want = ["Drive · Soft", "Compressor", "Filter · Multi", "EQ"];
        assert_eq!(names(&ui), (want.map(String::from).to_vec(), Some(2)));
        // Under a fingertip: a tap, it only picks.
        ui.touch(3, Touch::Down, x, mid(3));
        ui.touch(3, Touch::Move, x, mid(3) + 6);
        ui.touch(3, Touch::Up, x, mid(3) + 6);
        assert_eq!(crate::take_chain_actions(&mut ui), [ChainAction::Select(3)]);
        // Cancelled on the way: nothing moves.
        ui.touch(3, Touch::Down, x, mid(3));
        ui.touch(3, Touch::Move, x, mid(0));
        ui.touch(3, Touch::Cancel, x, mid(0));
        assert_eq!(crate::take_chain_actions(&mut ui), [ChainAction::Select(3)]);
        // Lifted where it started: nothing moves either.
        ui.touch(3, Touch::Down, x, mid(3));
        ui.touch(3, Touch::Move, x, mid(1));
        ui.touch(3, Touch::Move, x, mid(3));
        ui.touch(3, Touch::Up, x, mid(3));
        assert_eq!(crate::take_chain_actions(&mut ui), [ChainAction::Select(3)]);
        assert_eq!(names(&ui).0, want.map(String::from).to_vec());
        // The wheel: the picked effect (EQ) is one it can move.
        let t = ui.focus_at(x, mid(3)).unwrap();
        assert_eq!((t.id, t.index, t.moves), ("slot", 3, true));
        assert!(ui.focus_steps());
        assert!(ui.focus_step(-2).is_some());
        assert_eq!(
            crate::take_chain_actions(&mut ui),
            [ChainAction::Move { from: 3, to: 1 }]
        );
        let want = ["Drive · Soft", "EQ", "Compressor", "Filter · Multi"];
        assert_eq!(names(&ui), (want.map(String::from).to_vec(), Some(1)));
        let f = ui.focused().unwrap().target;
        assert_eq!((f.id, f.index), ("slot", 1), "the outline goes with it");
        assert!(ui.focus_step(-5).is_some(), "to the top, no further");
        assert_eq!(
            crate::take_chain_actions(&mut ui),
            [ChainAction::Move { from: 1, to: 0 }]
        );
        assert!(ui.focus_step(-1).is_none(), "first already");
        // An effect not picked is a plain tap for the wheel.
        let t = ui.focus_at(x, mid(2)).unwrap();
        assert_eq!((t.id, t.index, t.moves), ("slot", 2, false));
    }

    #[test]
    fn chain_touches_become_actions() {
        let mut song = Song::new();
        let mut st = song.chain_state(DRUMS, 1, 0, None);
        st.back = Some("Mixer".into());
        let mut ui = screen(Fonts::embedded(), &mut song, Place::Mixer, Some(st), None).unwrap();
        // The Q-Links show the drive's four from its plan (its Mode is
        // the Mode chooser).
        let q = ui.get::<QLinkColumn>(QLINKS).unwrap();
        assert_eq!(
            q.cards.iter().map(|c| c.label.as_str()).collect::<Vec<_>>(),
            ["Drive", "Tone", "Bias", "Mix"]
        );
        assert_eq!(
            (q.title.as_str(), q.places[0].as_str()),
            ("Q-Links · Drive · Soft", "Drums")
        );
        let r = ui.rect_of(crate::CHAIN).unwrap();
        let v = ui.get::<ChainView>(crate::CHAIN).unwrap();
        let l = v.layout(r);
        let drawn: Vec<&str> = v.page().sections[0]
            .cards
            .iter()
            .map(|c| c.title.as_str())
            .collect();
        assert_eq!(
            drawn,
            ["Curve", "A sine through it"],
            "the drive's drawings"
        );
        let k = v.control_rect(r, 0).unwrap();
        // Select the compressor, then bypass it.
        let row = l.rows[2];
        tap(&mut ui, row.x + 80, row.y + 20);
        tap(&mut ui, row.right() - 20, row.y + row.h / 2);
        // Touch the first knob (Drive: the app has not sent the
        // compressor's page yet) and drag it up.
        ui.touch(2, Touch::Down, k.x + k.w / 2, k.y + k.h / 2);
        ui.touch(2, Touch::Move, k.x + k.w / 2, k.y + k.h / 2 - 30);
        ui.touch(2, Touch::Up, k.x + k.w / 2, k.y + k.h / 2 - 30);
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
        // + LFO opened the rows over the page, to set the new one's depth;
        // a touch anywhere else shuts them.
        let v = ui.get::<ChainView>(crate::CHAIN).unwrap();
        assert!(v.open().rows);
        assert!(v.layout(r).rows_panel.is_some());
        // Only what answers is a target: the rows and the Modulate row.
        let modulate = [
            "macro",
            "learn",
            "add-lfo",
            "add-follower",
            "rows",
            "depth",
            "remove-row",
        ];
        assert!(v.targets(r).iter().all(|t| modulate.contains(&t.id)));
        // A macro answers over its whole pitch, the gap after it included:
        // macro 3 again takes it off.
        let gap = l.macros[2].right() + 1;
        tap(&mut ui, gap, l.macros[2].y + 5);
        assert_eq!(
            crate::take_chain_actions(&mut ui),
            [ChainAction::Macro {
                target: addr,
                macro_: None
            }]
        );
        tap(&mut ui, l.title.x + 10, l.title.y + 10);
        assert!(!ui.get::<ChainView>(crate::CHAIN).unwrap().open().rows);
        assert!(
            crate::take_chain_actions(&mut ui).is_empty(),
            "the name is no button"
        );
        let b = l.back.unwrap();
        tap(&mut ui, b.x + 5, b.y + 5);
        assert_eq!(crate::take_chain_actions(&mut ui), [ChainAction::Back]);
    }

    /// The Mode chooser under an effect's name: a chip per mode when they
    /// fit (Reverb's six), else an arrow each side of the mode's name
    /// (Drive's ten). A tap on another mode switches; the lit one does
    /// nothing. Drive's Mode is the chooser, not a knob; a Cloud reverb
    /// set to its own Plate is in none of the modes and keeps its
    /// Character knob.
    #[test]
    fn the_mode_chooser_switches_an_effect_with_modes() {
        let modes = |v: &ChainView, r: Rect| -> Vec<(usize, String, bool)> {
            v.targets(r)
                .into_iter()
                .filter(|t| t.id == "mode")
                .map(|t| (t.index, t.label, t.enabled))
                .collect()
        };
        let mut song = Song::new();
        let drive = song.chain_state(DRUMS, 1, 0, None);
        let mv = drive.mode.clone().unwrap();
        assert_eq!(
            (mv.family, mv.modes.len(), mv.current),
            ("Drive", 10, Some(0))
        );
        assert!(!drive.params.iter().any(|p| p.info.name == "Mode"));
        let mut ui = screen(
            Fonts::embedded(),
            &mut song,
            Place::Mixer,
            Some(drive),
            None,
        )
        .unwrap();
        let r = ui.rect_of(crate::CHAIN).unwrap();
        let v = ui.get::<ChainView>(crate::CHAIN).unwrap();
        let l = v.layout(r);
        assert!(l.mode_label.is_some(), "ten modes don't fit as chips");
        // Soft is the first: only the arrow to Hard, an arrow of its own.
        assert!(modes(v, r).is_empty());
        let arrows: Vec<_> = v
            .targets(r)
            .into_iter()
            .filter(|t| t.id.starts_with("mode-"))
            .map(|t| (t.id, t.label))
            .collect();
        assert_eq!(
            arrows,
            [
                ("mode-next", "Drive mode Hard ›".to_string()),
                ("mode-list", "Drive modes".to_string())
            ]
        );
        let (next, _) = l.modes[0];
        tap(&mut ui, next.x + 5, next.y + 5);
        assert_eq!(
            crate::take_chain_actions(&mut ui),
            [ChainAction::SetMode { slot: 1, mode: 1 }]
        );

        // A Room, then a Cloud reverb set to its own Plate.
        let make = |id| mpfree_fx::registry::make(id).unwrap();
        let room = song.drums.push(make("mpfree.fx.room"));
        let mut cloud = make("mpfree.fx.cloud");
        cloud.set_param(ids::cloud::CHARACTER, 0.0);
        let cloud = song.drums.push(cloud);
        let st = song.chain_state(DRUMS, room, 0, None);
        assert_eq!(st.slots[room].name, "Reverb · Room");
        assert_eq!(st.slots[cloud].name, "Cloud reverb");
        let mut ui = screen(Fonts::embedded(), &mut song, Place::Mixer, Some(st), None).unwrap();
        let v = ui.get::<ChainView>(crate::CHAIN).unwrap();
        let l = v.layout(r);
        assert!(l.mode_label.is_none(), "six fit as chips");
        let lit: Vec<bool> = modes(v, r).into_iter().map(|m| m.2).collect();
        assert_eq!(lit, [false, true, true, true, true, true]);
        for m in [0, 1] {
            let (c, _) = l.modes[m];
            tap(&mut ui, c.x + 5, c.y + 5);
        }
        assert_eq!(
            crate::take_chain_actions(&mut ui),
            [ChainAction::SetMode {
                slot: room,
                mode: 1
            }]
        );
        // One page, so no tabs: Character is on it.
        let st = song.chain_state(DRUMS, cloud, 1, None);
        assert_eq!(st.mode.as_ref().unwrap().current, None);
        assert_eq!(st.groups, ["Main"]);
        assert!(st.params.iter().any(|p| p.info.name == "Character"));
    }

    /// Drive's ten modes: the chip between the arrows opens the list of
    /// them over the page; a row switches the mode and shuts it, and a
    /// touch anywhere else shuts it.
    #[test]
    fn the_mode_chip_opens_the_list_of_modes() {
        let mut song = Song::new();
        let drive = song.chain_state(DRUMS, 1, 0, None);
        let mut ui = screen(
            Fonts::embedded(),
            &mut song,
            Place::Mixer,
            Some(drive),
            None,
        )
        .unwrap();
        let r = ui.rect_of(crate::CHAIN).unwrap();
        let l = ui.get::<ChainView>(crate::CHAIN).unwrap().layout(r);
        let chip = l.mode_label.unwrap();
        tap(&mut ui, chip.x + 10, chip.y + 10);
        let v = ui.get::<ChainView>(crate::CHAIN).unwrap();
        assert!(v.open().modes);
        let picks = ids_of(v, r, "mode-pick");
        assert_eq!(picks.len(), 10);
        assert_eq!(picks[0], (0, "Drive mode Soft".to_string(), false));
        // Only the list and its chip are targets: a touch anywhere else
        // only shuts it.
        assert!(v
            .targets(r)
            .iter()
            .all(|t| t.id == "mode-pick" || t.id == "mode-list"));
        let (row, m) = v.layout(r).mode_rows[3];
        tap(&mut ui, row.x + 10, row.y + 10);
        assert_eq!(
            crate::take_chain_actions(&mut ui),
            [ChainAction::SetMode { slot: 1, mode: m }]
        );
        assert!(!ui.get::<ChainView>(crate::CHAIN).unwrap().open().modes);
        // Open again, then a touch on the chain shuts it and only that.
        tap(&mut ui, chip.x + 10, chip.y + 10);
        let b = l.buttons[1];
        tap(&mut ui, b.x + 5, b.y + 5);
        assert!(!ui.get::<ChainView>(crate::CHAIN).unwrap().open().modes);
        assert!(crate::take_chain_actions(&mut ui).is_empty());
    }

    /// The effect's name in the header opens its presets: "Defaults",
    /// then its own (a family's, the mode's). The header names the preset
    /// its settings are; a row loads one (all but Mix and Quality) and
    /// shuts the list, and a touch anywhere else only shuts it.
    #[test]
    fn the_effects_name_opens_its_presets() {
        use mpfree_fx::ids::{drive, plate};
        let mut song = Song::new();
        let mut n = mpfree_fx::registry::make("mpfree.fx.plate").unwrap();
        let vocal = mpfree_fx::picker::presets("mpfree.fx.plate")
            .iter()
            .find(|p| p.name == "Vocal plate")
            .unwrap();
        for &(id, v) in vocal.values {
            n.set_param(id, v);
        }
        n.set_param(plate::MIX, 40.0);
        let slot = song.keys.push(n);
        let st = song.chain_state(KEYS, slot, 0, None);
        assert_eq!(st.presets[0].name, "Defaults");
        let k = st
            .presets
            .iter()
            .position(|p| p.name == "Vocal plate")
            .unwrap();
        assert_eq!(st.preset, Some(k), "its Mix is its own, not the preset's");
        assert!(st
            .presets
            .iter()
            .all(|p| p.values.iter().all(|&(id, _)| id != plate::MIX)));
        let mut ui = screen(
            Fonts::embedded(),
            &mut song,
            Place::Mixer,
            Some(st.clone()),
            None,
        )
        .unwrap();
        let r = ui.rect_of(crate::CHAIN).unwrap();
        let v = ui.get::<ChainView>(crate::CHAIN).unwrap();
        let chip = v.layout(r).preset_chip.unwrap();
        assert_eq!(
            ids_of(v, r, "presets-open"),
            [(0, "Reverb presets".to_string(), true)]
        );
        tap(&mut ui, chip.x + 10, chip.y + 10);
        let v = ui.get::<ChainView>(crate::CHAIN).unwrap();
        assert!(v.open().presets);
        let rows = ids_of(v, r, "preset-load");
        assert_eq!(rows.len(), st.presets.len());
        assert_eq!(
            rows[k],
            (k, "Vocal plate".to_string(), false),
            "the one it plays"
        );
        assert!(v
            .targets(r)
            .iter()
            .all(|t| t.id == "preset-load" || t.id == "presets-open"));
        let (row, _) = v.layout(r).preset_rows[0];
        tap(&mut ui, row.x + 10, row.y + 10);
        assert_eq!(
            crate::take_chain_actions(&mut ui),
            [ChainAction::Preset {
                slot,
                id: "mpfree.fx.plate",
                values: st.presets[0].values.clone()
            }]
        );
        assert!(!ui.get::<ChainView>(crate::CHAIN).unwrap().open().presets);
        // Open again: a touch on the chain only shuts it.
        tap(&mut ui, chip.x + 10, chip.y + 10);
        let b = ui.get::<ChainView>(crate::CHAIN).unwrap().layout(r).buttons[1];
        tap(&mut ui, b.x + 5, b.y + 5);
        assert!(!ui.get::<ChainView>(crate::CHAIN).unwrap().open().presets);
        assert!(crate::take_chain_actions(&mut ui).is_empty());
        // Turned away from it: no preset, and the header names the effect.
        let mut n = mpfree_fx::registry::make("mpfree.fx.plate").unwrap();
        n.set_param(plate::DECAY, 1234.0);
        let slot = song.keys.push(n);
        assert_eq!(song.chain_state(KEYS, slot, 0, None).preset, None);
        // A Drive in Soft: the Soft presets only, each keeping it in Soft.
        let soft = song.chain_state(DRUMS, 1, 0, None);
        assert!(soft.presets.len() > 1);
        assert!(soft
            .presets
            .iter()
            .all(|p| p.values.contains(&(drive::MODE, 0.0))));
    }

    fn ids_of(v: &ChainView, r: Rect, id: &str) -> Vec<(usize, String, bool)> {
        v.targets(r)
            .into_iter()
            .filter(|t| t.id == id)
            .map(|t| (t.index, t.label, t.enabled))
            .collect()
    }

    /// The Channel strip (the redesign): a Main page with its stages as
    /// sections, each with its switch in its heading and "Open ›" to its
    /// own page, then a page per stage; the Q-Links stay on Main's four on
    /// every page.
    #[test]
    fn the_channel_strip_keeps_its_stages_as_sections_and_the_qlinks_on_its_four() {
        let mut song = Song::new();
        let strip = mpfree_fx::registry::make("mpfree.fx.channel_strip").unwrap();
        let slot = song.keys.push(strip);
        let main = song.chain_state(KEYS, slot, 0, None);
        assert_eq!(main.groups, ["Main", "Gate", "Compressor", "EQ"]);
        let labels = |ps: &[crate::state::ParamView]| -> Vec<String> {
            ps.iter().map(|p| p.label.clone()).collect()
        };
        let four = ["Comp thresh", "Makeup", "Low cut", "High gain"];
        assert_eq!(labels(&main.main_four), four);
        let keys: Vec<&str> = main.sections.iter().map(|s| s.key.as_str()).collect();
        assert_eq!(keys, ["gate", "comp", "eq", "path", "output"]);
        let gate = song.chain_state(KEYS, slot, 1, None);
        assert_eq!(gate.group, 1);
        for st in [&main, &gate] {
            let (name, knobs) = st.qlinks();
            assert_eq!(name, "Channel strip");
            let q: Vec<_> = knobs.iter().flatten().map(|b| b.label.clone()).collect();
            assert_eq!(q, four);
        }
        // The tabs fit the header; a stage's switch flips it on the lift,
        // its "Open ›" shows its page.
        let gate_switch = main.sections[0].switch.as_ref().unwrap().param;
        let gate_addr = main.params[gate_switch].addr;
        let mut ui = screen(Fonts::embedded(), &mut song, Place::Mixer, Some(main), None).unwrap();
        let r = ui.rect_of(crate::CHAIN).unwrap();
        let v = ui.get::<ChainView>(crate::CHAIN).unwrap();
        let l = v.layout(r);
        assert!(l.tab_arrows.is_none(), "four tabs fit");
        assert_eq!(ids_of(v, r, "group")[0], (0, "Main".to_string(), false));
        let find = |v: &ChainView, id: &str, index: usize| {
            v.targets(r)
                .into_iter()
                .find(|t| t.id == id && t.index == index)
                .unwrap_or_else(|| panic!("no {id} {index}"))
        };
        let (x, y) = find(v, "section-switch", gate_switch).centre();
        let (lx, ly) = find(v, "section-link", 2).centre();
        tap(&mut ui, x, y);
        let v = ui.get::<ChainView>(crate::CHAIN).unwrap();
        assert_eq!(
            v.page().sections[0].switch.as_ref().map(|c| c.kind.clone()),
            Some(mpfree_synth_ui::section_view::Kind::Toggle(true)),
            "flipped at once"
        );
        tap(&mut ui, lx, ly);
        assert_eq!(
            crate::take_chain_actions(&mut ui),
            [
                ChainAction::Focus(gate_addr),
                ChainAction::Set {
                    addr: gate_addr,
                    value: 1.0
                },
                ChainAction::Release(gate_addr),
                ChainAction::Group(2)
            ]
        );
    }

    /// More tabs than the header holds (a plugin with many groups): a run
    /// around the page shown, with arrows to the pages either side; only
    /// the tabs in view are targets.
    #[test]
    fn a_long_tab_row_scrolls_with_the_page_shown() {
        let mut song = Song::new();
        let mut st = song.chain_state(KEYS, 1, 0, None);
        st.groups = (1..=9).map(|k| format!("Oscillator {k}")).collect();
        st.group = 4;
        let mut ui = screen(
            Fonts::embedded(),
            &mut song,
            Place::Mixer,
            Some(st.clone()),
            None,
        )
        .unwrap();
        let r = ui.rect_of(crate::CHAIN).unwrap();
        let v = ui.get::<ChainView>(crate::CHAIN).unwrap();
        let l = v.layout(r);
        let (prev, next) = l.tab_arrows.expect("the tabs don't all fit");
        assert!(l.tabs[4].w > 0, "the page shown is in view");
        assert!(l.tabs[8].is_empty(), "the last is out of view");
        let shown: Vec<usize> = ids_of(v, r, "group").into_iter().map(|t| t.0).collect();
        assert!(shown.contains(&4) && !shown.contains(&8), "{shown:?}");
        for t in l.tabs.iter().filter(|t| !t.is_empty()) {
            assert!(t.x >= prev.right() && t.right() <= next.x, "{t:?}");
        }
        assert!(
            l.title.right() <= l.tab_group.unwrap().x,
            "the name keeps its room"
        );
        assert_eq!(
            ids_of(v, r, "group-next"),
            [(0, "pages ›".to_string(), true)]
        );
        // The editor shows the new page at once, so ‹ then goes back.
        tap(&mut ui, next.x + 5, next.y + 5);
        tap(&mut ui, prev.x + 5, prev.y + 5);
        assert_eq!(
            crate::take_chain_actions(&mut ui),
            [ChainAction::Group(5), ChainAction::Group(4)]
        );
        // On the last page the row has moved along to show it.
        st.group = 8;
        let ui = screen(Fonts::embedded(), &mut song, Place::Mixer, Some(st), None).unwrap();
        let v = ui.get::<ChainView>(crate::CHAIN).unwrap();
        let l = v.layout(r);
        assert!(l.tabs[8].w > 0 && l.tabs[0].is_empty(), "{:?}", l.tabs);
        assert_eq!(
            ids_of(v, r, "group-next"),
            [(0, "pages ›".to_string(), false)]
        );
    }

    /// No Main page (eight controls or fewer, or a plugin): the groups,
    /// one over eight split into pages of eight.
    #[test]
    fn without_a_main_page_a_long_group_splits_into_pages_of_eight() {
        use mpfree_fx::mpfree_node::{Curve, Unit};
        let params: &'static [ParamInfo] = Box::leak(
            (0..11u16)
                .map(|i| ParamInfo {
                    id: i,
                    name: Box::leak(format!("P{i}").into_boxed_str()),
                    group: if i < 10 { "Synth" } else { "Out" },
                    min: 0.0,
                    max: 1.0,
                    default: 0.0,
                    unit: Unit::None,
                    curve: Curve::Linear,
                    readonly: false,
                })
                .collect::<Vec<_>>()
                .into_boxed_slice(),
        );
        let ps = crate::state::pages(params, &[], None);
        let shape: Vec<(String, usize)> =
            ps.iter().map(|p| (p.label.clone(), p.ids.len())).collect();
        assert_eq!(
            shape,
            [
                ("Synth".to_string(), 8),
                ("Synth 2".to_string(), 2),
                ("Out".to_string(), 1)
            ]
        );
        // A built-in effect follows its plan instead: one page, its four.
        let drive = Song::new().chain_state(DRUMS, 1, 0, None);
        assert_eq!(drive.groups, ["Main"]);
        let four: Vec<&str> = drive.main_four.iter().map(|p| p.label.as_str()).collect();
        assert_eq!(four, ["Drive", "Tone", "Bias", "Mix"]);
    }

    #[test]
    fn the_strip_button_lists_the_strips_and_opens_one() {
        let mut song = Song::new();
        let mut st = song.chain_state(KEYS, 1, 0, None);
        st.back = Some("Mixer".into());
        st.strip_name = "Keys".into();
        st.strip_kind = "Track 5".into();
        let mut ui = screen(
            Fonts::embedded(),
            &mut song,
            Place::Mixer,
            Some(st.clone()),
            None,
        )
        .unwrap();
        let r = ui.rect_of(crate::CHAIN).unwrap();
        let view = |ui: &Ui| {
            ui.get::<crate::ChainView>(crate::CHAIN)
                .unwrap()
                .state()
                .clone()
        };
        let l = ui.get::<crate::ChainView>(crate::CHAIN).unwrap().layout(r);
        let b = l.strip.expect("the strip button");
        assert!(l.back.unwrap().w < 60, "‹ alone");
        tap(&mut ui, b.x + 5, b.y + 5);
        assert_eq!(
            crate::take_chain_actions(&mut ui),
            [ChainAction::Strips(Some(0))],
            "on the open strip's group: the tracks"
        );
        assert_eq!(crate::chain_hint(&ui), crate::STRIPS_HINT);
        // A state from the app without the list leaves it open; with it,
        // its tiles show.
        crate::update_chain(&mut ui, st.clone());
        assert_eq!(view(&ui).list.map(|l| l.group), Some(0));
        let mut listed = st.clone();
        listed.list = Some(crate::StripList {
            group: 0,
            counts: [2, 16, 1, 4, 8, 1],
            strips: vec![
                crate::StripPick {
                    strip: Strip::Track(0),
                    name: "Drums".into(),
                    sub: "4 effects".into(),
                },
                crate::StripPick {
                    strip: KEYS,
                    name: "Keys".into(),
                    sub: "3 effects".into(),
                },
            ],
        });
        crate::update_chain(&mut ui, listed);
        let targets = ui.get::<crate::ChainView>(crate::CHAIN).unwrap().targets(r);
        assert!(
            targets.iter().all(|t| t.id.starts_with("strips-")),
            "the list takes the editor"
        );
        let mut c = SoftCanvas::new(1280, 800);
        ui.paint(&mut c);
        // Pads: the group changes (its tiles come from the app).
        let pads = targets
            .iter()
            .find(|t| t.id == "strips-group" && t.index == 1)
            .unwrap();
        let (x, y) = pads.centre();
        tap(&mut ui, x, y);
        assert_eq!(
            crate::take_chain_actions(&mut ui),
            [ChainAction::Strips(Some(1))]
        );
        assert!(view(&ui).list.unwrap().strips.is_empty());
        let tracks = targets
            .iter()
            .find(|t| t.id == "strips-group" && t.index == 0)
            .unwrap();
        let (x, y) = tracks.centre();
        tap(&mut ui, x, y);
        let _ = crate::take_chain_actions(&mut ui);
        let mut listed = st.clone();
        listed.list = Some(crate::StripList {
            group: 0,
            counts: [2, 16, 1, 4, 8, 1],
            strips: vec![crate::StripPick {
                strip: Strip::Track(0),
                name: "Drums".into(),
                sub: "4 effects".into(),
            }],
        });
        crate::update_chain(&mut ui, listed);
        let tile = ui
            .get::<crate::ChainView>(crate::CHAIN)
            .unwrap()
            .targets(r)
            .into_iter()
            .find(|t| t.id == "strips-pick")
            .unwrap();
        let (x, y) = tile.centre();
        tap(&mut ui, x, y);
        assert_eq!(
            crate::take_chain_actions(&mut ui),
            [ChainAction::OpenStrip(Strip::Track(0))]
        );
        assert!(view(&ui).list.is_none(), "shut");
        assert_eq!(crate::chain_hint(&ui), crate::CHAIN_HINT);
        // Back on the list shuts it and keeps the strip.
        tap(&mut ui, b.x + 5, b.y + 5);
        let _ = crate::take_chain_actions(&mut ui);
        let back = crate::view::list_layout(r).back;
        tap(&mut ui, back.x + 5, back.y + 5);
        assert_eq!(
            crate::take_chain_actions(&mut ui),
            [ChainAction::Strips(None)]
        );
        assert!(view(&ui).list.is_none());
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
        // The spring is a Reverb mode: it shows as the Reverb tile.
        assert_eq!(ids, ["mpfree.fx.room", "mpfree.fx.gate"]);
    }

    #[test]
    fn a_hold_or_the_presets_line_opens_presets_and_a_tap_adds_one() {
        let mut song = Song::new();
        let mut ui = picking(&mut song, Default::default());
        open_section(&mut ui, Section::Category(mpfree_fx::Category::DelayReverb));
        // The Reverb tile (its first mode, the room).
        let t = tile_of(&ui, "mpfree.fx.room").unwrap();
        // Held for half a second: the presets open instead of an add.
        ui.touch(1, Touch::Down, t.x + 20, t.y + 20);
        ui.get_mut::<ChainView>(crate::CHAIN)
            .unwrap()
            .age_press(0.6);
        ui.touch(1, Touch::Up, t.x + 20, t.y + 20);
        assert!(crate::take_chain_actions(&mut ui).is_empty());
        let r = ui.rect_of(crate::CHAIN).unwrap();
        let v = ui.get::<ChainView>(crate::CHAIN).unwrap();
        assert_eq!(v.picker().presets, Some("mpfree.fx.room"));
        let pl = v.layout(r).picker.unwrap();
        let rows = mpfree_fx::picker::preset_rows("mpfree.fx.room");
        let n = rows.len().min(pl.tiles.len() - 1);
        assert!(n >= 8, "every reverb mode's presets");
        assert_eq!(pl.tiles.len(), n + 1, "Defaults and the presets");
        assert!(v.layout(r).picks.is_empty());
        // A preset row adds the room with that preset.
        let row = pl.tiles[2];
        tap(&mut ui, row.x + 10, row.y + 10);
        assert_eq!(
            crate::take_chain_actions(&mut ui),
            [ChainAction::AddPreset {
                id: "mpfree.fx.room",
                preset: 1
            }]
        );
        // A Plate row adds the plate engine with its own preset.
        let mut ui2 = picking(&mut song, Default::default());
        open_section(
            &mut ui2,
            Section::Category(mpfree_fx::Category::DelayReverb),
        );
        let t = tile_of(&ui2, "mpfree.fx.room").unwrap();
        ui2.touch(1, Touch::Down, t.x + 20, t.y + 20);
        ui2.get_mut::<ChainView>(crate::CHAIN)
            .unwrap()
            .age_press(0.6);
        ui2.touch(1, Touch::Up, t.x + 20, t.y + 20);
        let k = rows.iter().position(|r| r.name == "Vocal plate").unwrap();
        let pl = ui2
            .get::<ChainView>(crate::CHAIN)
            .unwrap()
            .layout(r)
            .picker
            .unwrap();
        let row = pl.tiles[k + 1];
        tap(&mut ui2, row.x + 10, row.y + 10);
        assert_eq!(
            crate::take_chain_actions(&mut ui2),
            [ChainAction::AddPreset {
                id: "mpfree.fx.plate",
                preset: 0
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
    fn a_hold_opens_presets_with_the_finger_down_and_rows_act_on_the_lift() {
        let mut song = Song::new();
        let mut ui = picking(&mut song, Default::default());
        open_section(&mut ui, Section::Category(mpfree_fx::Category::DelayReverb));
        let r = ui.rect_of(crate::CHAIN).unwrap();
        let t = tile_of(&ui, "mpfree.fx.room").unwrap();
        ui.touch(1, Touch::Down, t.x + 20, t.y + 20);
        assert!(!crate::chain_hold(&mut ui), "not held long enough yet");
        ui.get_mut::<ChainView>(crate::CHAIN)
            .unwrap()
            .age_press(0.6);
        // Past the hold time the presets open while the finger is down.
        assert!(crate::chain_hold(&mut ui));
        assert_eq!(
            ui.get::<ChainView>(crate::CHAIN).unwrap().picker().presets,
            Some("mpfree.fx.room")
        );
        assert!(!crate::chain_hold(&mut ui), "once per press");
        // The finger now sits over a preset row: its lift adds nothing.
        ui.touch(1, Touch::Up, t.x + 20, t.y + 20);
        assert!(crate::take_chain_actions(&mut ui).is_empty());
        assert!(ui.get::<ChainView>(crate::CHAIN).unwrap().state().picking);
        // A preset row acts on the lift, and a slip off it adds nothing.
        let pl = ui
            .get::<ChainView>(crate::CHAIN)
            .unwrap()
            .layout(r)
            .picker
            .unwrap();
        let row = pl.tiles[1];
        ui.touch(2, Touch::Down, row.x + 10, row.y + 10);
        assert!(
            crate::take_chain_actions(&mut ui).is_empty(),
            "not on the way down"
        );
        ui.touch(2, Touch::Up, row.x + 10, row.bottom() + 40);
        assert!(crate::take_chain_actions(&mut ui).is_empty());
        ui.touch(3, Touch::Down, row.x + 10, row.y + 10);
        ui.touch(3, Touch::Up, row.x + 12, row.y + 12);
        assert_eq!(
            crate::take_chain_actions(&mut ui),
            [ChainAction::AddPreset {
                id: "mpfree.fx.room",
                preset: 0
            }]
        );
        // The presets line is as tall as the star, and acts on the lift.
        let mut ui = picking(&mut song, Default::default());
        open_section(&mut ui, Section::Category(mpfree_fx::Category::DelayReverb));
        let pl = ui
            .get::<ChainView>(crate::CHAIN)
            .unwrap()
            .layout(r)
            .picker
            .unwrap();
        let chip = pl.chips.iter().flatten().next().copied().unwrap();
        assert_eq!(chip.h, 44);
        ui.touch(4, Touch::Down, chip.x + 10, chip.y + 10);
        assert_eq!(
            ui.get::<ChainView>(crate::CHAIN).unwrap().picker().presets,
            None
        );
        ui.touch(4, Touch::Up, chip.x + 10, chip.y + 10);
        assert!(ui
            .get::<ChainView>(crate::CHAIN)
            .unwrap()
            .picker()
            .presets
            .is_some());
        // An effect with no presets waits for the lift, which adds it.
        let mut ui = picking(&mut song, Default::default());
        open_section(&mut ui, Section::Category(mpfree_fx::Category::Performance));
        let t = tile_of(&ui, "mpfree.fx.perform").unwrap();
        ui.touch(5, Touch::Down, t.x + 20, t.y + 20);
        ui.get_mut::<ChainView>(crate::CHAIN)
            .unwrap()
            .age_press(0.6);
        assert!(!crate::chain_hold(&mut ui));
        ui.touch(5, Touch::Up, t.x + 20, t.y + 20);
        assert_eq!(
            crate::take_chain_actions(&mut ui),
            [ChainAction::Add("mpfree.fx.perform")]
        );
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
        let reverb = v
            .picker()
            .picks(&st)
            .iter()
            .position(|p| p.name == "Reverb")
            .unwrap();
        v.wheel(reverb as i32 - (n as i32 - 1), false);
        // A push opens the Reverb's presets on Defaults; the wheel and a
        // push add the room with its second preset.
        v.push_wheel();
        assert_eq!(v.picker().presets, Some("mpfree.fx.room"));
        assert_eq!(v.picker().highlight, Some(0));
        v.wheel(2, false);
        v.push_wheel();
        assert_eq!(
            v.take_actions(),
            [ChainAction::AddPreset {
                id: "mpfree.fx.room",
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
        let v = ui.get::<ChainView>(crate::CHAIN).unwrap();
        assert!(
            !v.targets(r)
                .iter()
                .any(|t| t.id == "knob" || t.id == "macro"),
            "the picker takes the effect column"
        );
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
        // The Reverb tile adds its first mode, the room.
        let t = tile_of(&ui, "mpfree.fx.room").unwrap();
        tap(&mut ui, t.x + 10, t.y + 10);
        assert_eq!(
            crate::take_chain_actions(&mut ui),
            [ChainAction::Add("mpfree.fx.room")]
        );
        assert!(!ui.get::<ChainView>(crate::CHAIN).unwrap().state().picking);
        assert!(mpfree_fx_ui::make("mpfree.fx.cloud").is_some());
        // Open again (on the section left) and cancel: the app hears both.
        tap(&mut ui, add.x + 10, add.y + 10);
        assert!(tile_of(&ui, "mpfree.fx.room").is_some(), "same section");
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

    /// The drum bus EQ (slot 3) on an EQ editor page.
    fn eq_editor() -> (crate::eq_page::EqPage, Rect) {
        let song = Song::new();
        let st = song.chain_state(DRUMS, 3, 0, None);
        assert!(st.eq);
        let mut page = crate::eq_page::EqPage::default();
        page.update(&st.params, None);
        (page, Rect::new(244, 146, 844, 540))
    }

    fn sets(out: &[ChainAction]) -> Vec<(u16, f32)> {
        out.iter()
            .filter_map(|a| match a {
                ChainAction::Set { addr, value } => Some((addr.param, *value)),
                _ => None,
            })
            .collect()
    }

    /// A dot dragged past the lift moves its band's frequency and gain,
    /// and the finger's lift releases them.
    #[test]
    fn dragging_a_dot_sets_frequency_and_gain() {
        use crate::eq_page::{layout, Band};
        use mpfree_fx::ids::eq::{freq, gain};
        let (mut page, r) = eq_editor();
        let l = layout(r, &page);
        let (fx, fy) = page.dot(Band::Band(0));
        let x = l.plot.x + (fx * l.plot.w as f32) as i32;
        let y = l.plot.bottom() - (fy * l.plot.h as f32) as i32;
        let mut out = Vec::new();
        page.down(1, x, y, r, &mut out);
        page.moved(1, x + 4, y - 4, r, &mut out);
        assert!(sets(&out).is_empty(), "under the lift nothing moves");
        page.moved(1, x + 60, y - 60, r, &mut out);
        page.up(1, x + 60, y - 60, r, false, &mut out);
        let set = sets(&out);
        assert!(
            set.iter().any(|&(id, v)| id == freq(0) && v > 100.0),
            "{set:?}"
        );
        assert!(
            set.iter().any(|&(id, v)| id == gain(0) && v > 1.0),
            "{set:?}"
        );
        assert!(matches!(out.last(), Some(ChainAction::Release(_))));
        assert_eq!(page.picked(), Some(Band::Band(0)));
    }

    /// Two taps on an empty part of the graph put a bell there in the
    /// first Unused band.
    #[test]
    fn a_double_tap_adds_a_band() {
        use crate::eq_page::{layout, Band};
        use mpfree_fx::ids::eq::{band_use, USE_ON};
        let (mut page, r) = eq_editor();
        let l = layout(r, &page);
        let free = page.free().expect("bands 5 to 8 are free");
        let (x, y) = (l.plot.x + l.plot.w * 3 / 4, l.plot.y + l.plot.h / 5);
        let mut out = Vec::new();
        for _ in 0..2 {
            page.down(1, x, y, r, &mut out);
            page.up(1, x, y, r, false, &mut out);
        }
        assert!(sets(&out).contains(&(band_use(free), USE_ON)), "{out:?}");
        assert_eq!(page.picked(), Some(Band::Band(free)));
    }

    /// Listen plays the picked band, follows the pick, and stops when the
    /// page lets it go.
    #[test]
    fn listen_follows_the_pick_and_stops() {
        use crate::eq_page::{Band, Button};
        use mpfree_fx::ids::eq::LISTEN;
        let (mut page, _) = eq_editor();
        let mut out = Vec::new();
        page.press(Button::Chip(Band::Band(1)), &mut out);
        page.press(Button::Listen, &mut out);
        assert_eq!(page.listening(), Some(1));
        page.press(Button::Chip(Band::Band(2)), &mut out);
        assert_eq!(page.listening(), Some(2));
        page.press(Button::Chip(Band::Low), &mut out);
        assert_eq!(page.listening(), None, "a cut has nothing to hear");
        page.press(Button::Chip(Band::Band(0)), &mut out);
        page.press(Button::Listen, &mut out);
        assert_eq!(
            sets(&out)
                .iter()
                .filter(|s| s.0 == LISTEN)
                .copied()
                .collect::<Vec<_>>(),
            [(LISTEN, 2.0), (LISTEN, 3.0), (LISTEN, 0.0), (LISTEN, 1.0)]
        );
        let stop = page.stop_listening();
        assert_eq!(sets(&stop), [(LISTEN, 0.0)]);
        assert!(page.stop_listening().is_empty());
    }
}
