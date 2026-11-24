//! The safe controls of every factory preset (S3's sound moves): which knob
//! Continue's Brightness and Length turn on each of MPFree's own synths'
//! presets, from where, and how far ([`mpfree_compose::safe_controls`]).
//!
//! `cargo run -p mpfree-compose --example safe_controls` prints a table
//! (Markdown), one row a preset; compose.md keeps the counts.

use mpfree_compose::{safe_controls, Control, Knob, Safe, Swing};
use mpfree_node::{Curve, Node};
use mpfree_synth::{
    Analog, Bass, Choir, Drawbar, Fm, Keys, Pluck, Poly, Squelch, Strings, Wavetable, Wind,
};

const RATE: u32 = 48_000;

fn knobs(node: &dyn Node) -> Vec<Knob<'static>> {
    node.params()
        .iter()
        .filter(|i| !i.readonly)
        .map(|i| Knob {
            id: i.id,
            name: i.name,
            group: i.group,
            min: i.min,
            max: i.max,
            log: i.curve == Curve::Log,
            value: node.param(i.id),
        })
        .collect()
}

fn words(node: &dyn Node, s: Option<Safe>) -> String {
    let Some(s) = s else {
        return "none".into();
    };
    let info = node.params().iter().find(|i| i.id == s.id);
    let name = info.map_or("?", |i| i.name);
    let group = info.map_or("?", |i| i.group);
    let how = match s.swing {
        Swing::Octaves(o) => format!("±{o} oct"),
        Swing::Travel(t) => format!("±{:.0} % of its travel", t * 100.0),
        Swing::Times(f) => format!("×{f} / ÷{f}"),
    };
    format!(
        "{name} ({group}) from {:.0}, {how}, {:.0} to {:.0}",
        s.base,
        s.value(-1.0),
        s.value(1.0)
    )
}

#[derive(Default)]
struct Counts {
    presets: usize,
    brightness: usize,
    length: usize,
    neither: usize,
}

fn row(synth: &str, preset: &str, node: &dyn Node, n: &mut Counts) {
    let s = safe_controls(&knobs(node));
    n.presets += 1;
    n.brightness += usize::from(s.brightness.is_some());
    n.length += usize::from(s.length.is_some());
    n.neither += usize::from(s.brightness.is_none() && s.length.is_none());
    println!(
        "| {synth} | {preset} | {} | {} |",
        words(node, s.get(Control::Brightness)),
        words(node, s.get(Control::Length))
    );
}

macro_rules! synth {
    ($ty:ident, $n:expr) => {
        for p in $ty::presets() {
            let mut s = $ty::new(RATE);
            s.load_preset(p);
            row(stringify!($ty), p.name, &s, $n);
        }
    };
}

fn main() {
    println!("| Synth | Preset | Brightness | Length |");
    println!("|---|---|---|---|");
    let mut n = Counts::default();
    synth!(Poly, &mut n);
    synth!(Analog, &mut n);
    synth!(Squelch, &mut n);
    synth!(Bass, &mut n);
    synth!(Wavetable, &mut n);
    synth!(Fm, &mut n);
    synth!(Keys, &mut n);
    synth!(Drawbar, &mut n);
    synth!(Pluck, &mut n);
    synth!(Strings, &mut n);
    synth!(Wind, &mut n);
    synth!(Choir, &mut n);
    println!();
    println!(
        "{} presets: {} with a Brightness knob, {} with a Length knob, {} with neither (Space only).",
        n.presets, n.brightness, n.length, n.neither
    );
}
