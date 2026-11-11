//! Sample screens with a real chain and mod engine behind them, for the
//! workstation preview (`mpfree-chain-preview`) and the tests: a drum bus
//! (filter, drive, compressor, a bypassed EQ) and a keys track (chorus,
//! lo-fi, delay), four macros across both, three matrix rows, and the
//! morph between two scenes.

use mpfree_fx::ids;
use mpfree_fx::mpfree_node::{Ctx, ParamInfo};
use mpfree_fx::{strip_params, Chain, Chorus, Compressor, Delay, Drive, Eq, LoFi, MultiFilter};
use mpfree_mod::{lfo, Binding, Bindings, MacroMap, MapCurve, ModEngine, ModEvent, ModTimed, Row, Source, DEFAULT_TICK};
use mpfree_param::{Owner, ParamAddr, Strip};
use mpfree_ui::widgets::{Chip, Header, Label};
use mpfree_ui::{Align, EventKind, FontId, Fonts, PaintCx, Rect, Theme, Tone, Ui, Widget};

use crate::state::{macro_color, ChainState, MacrosState};
use crate::{ChainView, MacrosView};

/// The music tabs, as the approved layout spec lists them.
pub const TABS: [&str; 5] = ["Play", "Clips", "Synth", "Looper", "Sample"];
pub const DRUMS: Strip = Strip::Bus(0);
pub const KEYS: Strip = Strip::Track(4);
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

fn info(c: &Chain, slot: usize, id: u16) -> ParamInfo { *c.node(slot).unwrap().params().iter().find(|p| p.id == id).unwrap() }

impl Song {
    pub fn new() -> Song {
        let drums = chain(vec![Box::new(MultiFilter::new()), Box::new(Drive::new()), Box::new(Compressor::new()), Box::new(Eq::new())]);
        let keys = chain(vec![Box::new(Chorus::new()), Box::new(LoFi::new()), Box::new(Delay::new())]);
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
        let send = ParamAddr::new(Owner::Strip(KEYS), ids::mixer::send(0));
        e.add_target(send, *strip_params().iter().find(|p| p.id == ids::mixer::send(0)).unwrap(), -30.0).unwrap();
        let map = |e: &mut ModEngine, m: usize, target: ParamAddr, from: f32, to: f32| e.map(m, MacroMap { target, from, to, curve: MapCurve::Linear }).unwrap();
        // 1 Wash, 2 Grit, 3 Pump, 4 Air.
        map(&mut e, 0, send, 0.45, 0.85);
        map(&mut e, 0, fb, 0.2, 0.7);
        map(&mut e, 0, dlmix, 0.1, 0.6);
        map(&mut e, 0, dtone, 0.8, 0.35);
        map(&mut e, 1, drive, 0.0, 0.6);
        map(&mut e, 1, bias, 0.0, 0.4);
        map(&mut e, 1, lofi, 0.0, 0.55);
        map(&mut e, 2, thresh, 0.9, 0.55);
        map(&mut e, 2, crel, 0.2, 0.6);
        map(&mut e, 2, cmix, 0.2, 0.6);
        map(&mut e, 3, chorus, 0.0, 0.35);
        map(&mut e, 3, reso, 0.0, 0.3);
        e.add_row(Row { source: Source::Lfo(0), via: None, target: tone, depth: 0.18 }).unwrap();
        e.add_row(Row { source: Source::Follower(0), via: None, target: dlmix, depth: -0.25 }).unwrap();
        e.add_row(Row { source: Source::Lfo(1), via: Some(Source::Host(2)), target: cutoff, depth: 0.12 }).unwrap();
        // Scene A as it is; scene B with the drums darker and wetter.
        let mut setup = vec![
            ModEvent::Set { addr: ParamAddr::new(Owner::Mod(0), lfo::ids::SYNC), value: 6.0 },
            ModEvent::Set { addr: ParamAddr::new(Owner::Mod(1), lfo::ids::RATE), value: 0.3 },
            ModEvent::Source { n: 2, value: 0.8 },
        ];
        let run = |e: &mut ModEngine, evs: &[ModEvent], blocks: u64| {
            for b in 0..blocks {
                let f0 = b * BLOCK as u64;
                let timed: Vec<ModTimed> = if b == 0 { evs.iter().map(|ev| ModTimed::new(0, *ev)).collect() } else { Vec::new() };
                let audio: Vec<f32> = (0..BLOCK).map(|i| 0.4 * (((f0 + i as u64) as f32) * 0.05).sin()).collect();
                let ctx = Ctx { rate: RATE, frame: f0, bpm: 92.0, beats: f0 as f64 * 92.0 / 60.0 / RATE as f64, playing: true, sidechain: None };
                e.process(&ctx, BLOCK, &timed, &[Some((&audio, &audio)), None, None, None], &mut |_, _| {});
            }
        };
        run(&mut e, &setup, 4);
        e.capture_scene(0);
        setup = vec![
            ModEvent::Set { addr: cutoff, value: 900.0 },
            ModEvent::Set { addr: ParamAddr::macro_value(0), value: 0.9 },
            ModEvent::Set { addr: ParamAddr::macro_value(2), value: 0.8 },
        ];
        run(&mut e, &setup, 2);
        e.capture_scene(1);
        let names = ["Wash", "Grit", "Pump", "Air", "Macro 5", "Macro 6", "Macro 7", "Macro 8"].map(String::from).to_vec();
        let mut start: Vec<ModEvent> = [0.62, 0.40, 0.55, 0.25].iter().enumerate().map(|(m, v)| ModEvent::Set { addr: ParamAddr::macro_value(m as u8), value: *v }).collect();
        start.push(ModEvent::Set { addr: ParamAddr::morph_pos(0), value: 0.64 });
        run(&mut e, &start, 40);
        let mut bindings = Bindings::new();
        bindings.bank_mut(0).name = "Macros 1-4".into();
        for (m, name) in names.iter().enumerate().take(4) {
            bindings.bind(0, m, Binding { colour: Some(m as u8), ..Binding::new(ParamAddr::macro_value(m as u8), mpfree_mod::macro_info(m), name) });
            bindings.observe(ParamAddr::macro_value(m as u8), e.macro_value(m));
        }
        Song { drums, keys, engine: e, bindings, names }
    }

    /// A parameter's name on the cards: "Drive (Drums)", "Send A (Keys)".
    pub fn label(&self, a: ParamAddr) -> String {
        if let Owner::Macro(n) = a.owner {
            return format!("Macro {} · {}", n + 1, self.names[n as usize]);
        }
        let (strip_name, node) = match a.owner {
            Owner::Insert(s, uid) => {
                let c = if s == DRUMS { &self.drums } else { &self.keys };
                (if s == DRUMS { "Drums" } else { "Keys" }, c.slot_of(uid).and_then(|i| c.node(i)))
            }
            Owner::Strip(s) => (if s == DRUMS { "Drums" } else { "Keys" }, None),
            _ => ("", None),
        };
        let pname = match node {
            Some(n) => n.params().iter().find(|p| p.id == a.param).map(|p| p.name).unwrap_or("?"),
            None => strip_params().iter().find(|p| p.id == a.param).map(|p| p.name).unwrap_or("?"),
        };
        let nname = node.map(|n| n.info().name);
        match nname {
            Some(n) if !pname.starts_with(n) && pname != "Drive" => format!("{n} {} ({strip_name})", pname.to_lowercase()),
            _ => format!("{pname} ({strip_name})"),
        }
    }

    pub fn chain_state(&self, strip: Strip, selected: usize, group: usize, focus: Option<ParamAddr>) -> ChainState {
        let (c, title) = if strip == DRUMS { (&self.drums, "Drums · FX chain") } else { (&self.keys, "Keys · FX chain") };
        let mut st = ChainState::from_chain(c, strip, title, 8, &self.engine, Some(selected), group, focus);
        st.macro_names = self.names.clone();
        st.cost_pct = Some(if strip == DRUMS { 6.2 } else { 4.8 });
        st
    }

    pub fn macros_state(&self, bank: usize) -> MacrosState {
        let mut st = MacrosState::from_engine(&self.engine, &self.names, &|a| self.label(a), bank);
        st.scene_names = ["Dry verse".into(), "Wash drop".into()];
        st
    }
}

impl Default for Song {
    fn default() -> Self { Self::new() }
}

/// A stand-in for the app's Q-Link column (the Hello MPC thread draws the
/// real one), showing the cards [`Bindings::cards`] gives.
pub struct QLinkColumn {
    pub cards: Vec<mpfree_mod::QLinkCard>,
    pub bank: String,
}

impl Widget for QLinkColumn {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.canvas.fill_rect(r, t.bg);
        let inner = r.inset_xy(8, 0);
        cx.text(FontId::Bold, t.size_small - 1.0, inner.take_top(28).0, Align::Left, t.text_faint, &format!("Q-LINKS · {}", self.bank.to_uppercase()));
        let h = (inner.h - 28 - 3 * 8) / 4;
        for (k, c) in self.cards.iter().enumerate() {
            let card = Rect::new(inner.x, inner.y + 28 + k as i32 * (h + 8), inner.w, h);
            cx.canvas.fill_round_rect(card, t.radius, t.surface);
            let col = c.colour.map_or(t.accent, |m| macro_color(&t, m as usize));
            let d = 54.0;
            let (kx, ky) = (card.x as f32 + 14.0 + d * 0.5, card.y as f32 + card.h as f32 * 0.5);
            mpfree_synth_ui::widgets::knob(cx, kx, ky, d, c.pos, false, t.outline, col);
            let text = Rect::new((kx + d * 0.5) as i32 + 10, card.y, card.right() - (kx + d * 0.5) as i32 - 14, card.h);
            let (top, bottom) = text.take_top(card.h / 2);
            cx.text(FontId::Bold, t.size_small - 2.0, top.take_bottom(22).0, Align::Left, t.text_dim, &format!("Q-LINK {} · {}", k + 1, c.label.to_uppercase()));
            cx.text(FontId::Bold, t.size_title - 2.0, bottom.take_top(30).0, Align::Left, if c.enabled() { t.text } else { t.text_faint }, if c.enabled() { &c.text } else { "free" });
        }
    }

    fn touch(&mut self, _f: u64, _p: mpfree_ui::Touch, _x: i32, _y: i32, _r: Rect) -> Option<EventKind> { None }
}

const PREVIEW_CHAIN: &str = "col(header:60, row(chain:*, qlinks:230):*, hint:26)";
const PREVIEW_MACROS: &str = "col(header:60, row(macros:*, qlinks:230):*, hint:26)";

/// The app's screen around a panel: the header (Sample tab lit for the
/// chain, Play for the macros bank), the panel, a Q-Link column, the hint.
pub fn screen(fonts: Fonts, song: &Song, chain: Option<ChainState>, macros: Option<MacrosState>) -> Result<Ui, String> {
    let mut ui = Ui::new(Theme::dark(), fonts, 1280, 800);
    let mut header = Header::new(&TABS);
    header.active = if chain.is_some() { 4 } else { 0 };
    header.chips = vec![Chip::new("92 BPM", Tone::Normal), Chip::new("Night drive", Tone::Normal)];
    ui.add("header", header);
    ui.add("qlinks", QLinkColumn { cards: song.bindings.cards().to_vec(), bank: song.bindings.banks()[0].name.clone() });
    let (hint, layout) = if chain.is_some() { (crate::CHAIN_HINT, PREVIEW_CHAIN) } else { (crate::MACROS_HINT, PREVIEW_MACROS) };
    ui.add("hint", Label::new(hint, Tone::Dim, Align::Left));
    ui.add(crate::CHAIN, ChainView::new());
    ui.add(crate::MACROS_W, MacrosView::new());
    ui.add_page("preview", layout)?;
    ui.show("preview")?;
    if let Some(c) = chain { crate::update_chain(&mut ui, c); }
    if let Some(m) = macros { crate::update_macros(&mut ui, m); }
    Ok(ui)
}

/// Every sample screen.
pub fn samples(fonts: impl Fn() -> Fonts) -> Result<Vec<Sample>, String> {
    let song = Song::new();
    let drive_uid = song.drums.uid(1).unwrap();
    let filter_uid = song.drums.uid(0).unwrap();
    let delay_uid = song.keys.uid(2).unwrap();
    let mut drive = song.chain_state(DRUMS, 1, 0, Some(ParamAddr::new(Owner::Insert(DRUMS, drive_uid), ids::drive::TONE)));
    drive.slots[3].on = false;
    let filter = song.chain_state(DRUMS, 0, 0, Some(ParamAddr::new(Owner::Insert(DRUMS, filter_uid), ids::filter::CUTOFF)));
    let delay = song.chain_state(KEYS, 2, 2, Some(ParamAddr::new(Owner::Insert(KEYS, delay_uid), ids::delay::MIX)));
    let empty = ChainState::from_chain(&chain(vec![]), Strip::Track(6), "Vocal chop · FX chain", 8, &song.engine, None, 0, None);
    Ok(vec![
        Sample { name: "chain-drums-drive", about: "The drum bus chain, Drive selected; Tone touched: LFO 1 sweeps it (green ring), Grit (macro 2) moves Drive and Bias (badges); the EQ bypassed", ui: screen(fonts(), &song, Some(drive), None)? },
        Sample { name: "chain-drums-filter", about: "The drum bus filter: cutoff swept by LFO 2 scaled by the mod wheel; resonance on Air (macro 4)", ui: screen(fonts(), &song, Some(filter), None)? },
        Sample { name: "chain-keys-delay", about: "The keys chain, Delay selected: Mix is on Wash (macro 1) and ducked by follower 1 (a negative row)", ui: screen(fonts(), &song, Some(delay), None)? },
        Sample { name: "chain-empty", about: "A track with no effects yet: + Add effect", ui: screen(fonts(), &song, Some(empty), None)? },
        Sample { name: "macros-1-4", about: "Macros 1-4 (Wash, Grit, Pump, Air) with their targets and ranges; the morph 64 % of the way from Dry verse to Wash drop and what it changes", ui: screen(fonts(), &song, None, Some(song.macros_state(0)))? },
        Sample { name: "macros-5-8", about: "Macros 5-8, not yet assigned", ui: screen(fonts(), &song, None, Some(song.macros_state(1)))? },
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ChainAction, MacroAction};
    use mpfree_ui::{SoftCanvas, Touch};

    fn tap(ui: &mut Ui, x: i32, y: i32) {
        ui.touch(1, Touch::Down, x, y);
        ui.touch(1, Touch::Up, x, y);
    }

    #[test]
    fn the_samples_read_real_state_and_paint() {
        let s = samples(Fonts::embedded).unwrap();
        assert_eq!(s.len(), 6);
        for mut sample in s {
            let (w, h) = sample.ui.size();
            let mut c = SoftCanvas::new(w, h);
            sample.ui.paint(&mut c);
        }
        let song = Song::new();
        let st = song.chain_state(DRUMS, 1, 0, None);
        assert_eq!(st.slots.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["Multi filter", "Drive", "Compressor", "EQ"]);
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
        let song = Song::new();
        let mut ui = screen(Fonts::embedded(), &song, Some(song.chain_state(DRUMS, 1, 0, None)), None).unwrap();
        let r = ui.rect_of(crate::CHAIN).unwrap();
        let l = crate::view::layout(r, ui.get::<ChainView>(crate::CHAIN).unwrap().state());
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
        let ChainAction::Focus(addr) = acts[2] else { panic!("{acts:?}") };
        assert!(matches!(acts[3], ChainAction::Set { addr: a, .. } if a == addr));
        // Put the knob on macro 3, and move the selected insert up.
        let st = ui.get::<ChainView>(crate::CHAIN).unwrap().state().clone();
        let l = crate::view::layout(r, &st);
        tap(&mut ui, l.macros[2].x + 5, l.macros[2].y + 5);
        tap(&mut ui, l.learn.x + 5, l.learn.y + 5);
        tap(&mut ui, l.buttons[0].x + 5, l.buttons[0].y + 5);
        tap(&mut ui, l.add_lfo.x + 5, l.add_lfo.y + 5);
        let acts = crate::take_chain_actions(&mut ui);
        assert_eq!(acts[0], ChainAction::Macro { target: addr, macro_: Some(2) });
        assert_eq!(acts[1], ChainAction::Learn(addr));
        assert_eq!(acts[2], ChainAction::Move { from: 2, to: 1 });
        assert_eq!(acts.len(), 4, "{acts:?} {:?}", l.add_lfo);
        assert!(matches!(acts[3], ChainAction::AddRow { target, .. } if target == addr), "{acts:?}");
    }

    #[test]
    fn macro_touches_become_actions() {
        let song = Song::new();
        let mut ui = screen(Fonts::embedded(), &song, None, Some(song.macros_state(0))).unwrap();
        let r = ui.rect_of(crate::MACROS_W).unwrap();
        let l = crate::macros::layout(r);
        let (kx, ky, _) = l.knobs[1];
        let v0 = ui.get::<MacrosView>(crate::MACROS_W).unwrap().state().macros[1].value;
        ui.touch(1, Touch::Down, kx as i32, ky as i32);
        ui.touch(1, Touch::Move, kx as i32, ky as i32 - 30);
        ui.touch(1, Touch::Up, kx as i32, ky as i32 - 30);
        tap(&mut ui, l.slider.x + l.slider.w / 2, l.slider.y + l.slider.h / 2);
        tap(&mut ui, l.save[1].x + 4, l.save[1].y + 4);
        tap(&mut ui, l.banks[1].x + 4, l.banks[1].y + 4);
        let acts = crate::take_macro_actions(&mut ui);
        assert!(matches!(acts[0], MacroAction::Macro { m: 1, value } if (value - v0 - 0.1).abs() < 0.01), "{acts:?}");
        assert!(matches!(acts[1], MacroAction::Morph(p) if (p - 0.5).abs() < 0.05));
        assert_eq!(acts[2], MacroAction::Capture(1));
        assert_eq!(acts[3], MacroAction::Bank(1));
    }
}
