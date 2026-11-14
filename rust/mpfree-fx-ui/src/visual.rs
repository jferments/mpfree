//! The drawings on an effect's page, worked out from its values.
//!
//! The curves come from the effects' own functions (`Eq::response_db`,
//! `MultiFilter::response_db`, `Comb::response_db`, `Flanger::
//! response_db` and the rest), fed the page's values through a small copy
//! of the effect ([`Scratch`]), so what is drawn is what plays: the tests in
//! `mpfree-fx/tests/curves.rs` measure each against a sine. Where a
//! drawing is a picture of behaviour over time rather than a curve (the
//! delay's repeats, the comb's ring, the reverb's decay, the compressor's
//! timing), it is worked out from the same values and says so in its
//! caption.
//!
//! Every card is in the unit box (x 0 to 1 left to right, y 0 at the
//! bottom), drawn by the Synth tab's `Visuals` widget. Colours come from
//! the theme, and from the Q-Link colours where a drawing names what a
//! Q-Link turns (the EQ's bands are Q-Links 1 to 4 on the Bands page).

use crate::state::{FxState, Scratch};
use mpfree_fx::ids::{
    character, chorus, cloud, comb, compressor as comp, crush, delay, drive, ducker, eq, filter,
    flanger, harmonizer as harm, lofi, master, phaser, pitch, pitch_correct as tune, transient,
};
use mpfree_fx::{
    interval_semis, nearest_note, Cloud, Ducker, TransientShaper, DUCKER_RATE_BEATS, VOCAL_KEYS,
};
use mpfree_node::{Ctx, Node, MAX_BLOCK};
use mpfree_synth_ui::format::value_text;
use mpfree_synth_ui::widgets::{Card, CardNode, Trace, QLINK_COLORS};
use mpfree_ui::{Color, Theme};

/// The cards for a page, with the page a tap on each card (and on each of
/// its boxes, counted across the cards) opens.
#[derive(Default)]
pub struct Strip {
    pub cards: Vec<Card>,
    pub pages: Vec<String>,
    pub node_pages: Vec<String>,
}

impl Strip {
    fn push(&mut self, card: Card, page: &str) {
        self.cards.push(card);
        self.pages.push(page.to_string());
    }
}

/// Whether the page draws anything for this effect.
pub fn has_visuals(id: &str) -> bool {
    matches!(
        id,
        "mpfree.fx.eq"
            | "mpfree.fx.filter"
            | "mpfree.fx.dj_filter"
            | "mpfree.fx.drive"
            | "mpfree.fx.comb"
            | "mpfree.fx.crush"
            | "mpfree.fx.delay"
            | "mpfree.fx.chorus"
            | "mpfree.fx.flanger"
            | "mpfree.fx.phaser"
            | "mpfree.fx.lofi"
            | "mpfree.fx.compressor"
            | "mpfree.fx.cloud"
            | "mpfree.fx.master_safety"
            | "mpfree.fx.character"
            | "mpfree.fx.ducker"
            | "mpfree.fx.transient"
            | "mpfree.fx.pitch"
            | "mpfree.fx.pitch_correct"
            | "mpfree.fx.harmonizer"
    )
}

/// The lowest and highest frequency drawn.
pub const F_LO: f32 = 20.0;
pub const F_HI: f32 = 20_000.0;
/// Points along a curve.
const N: usize = 180;

/// Where `hz` sits across a frequency axis (log, 20 Hz to 20 kHz).
pub fn freq_x(hz: f32) -> f32 {
    ((hz / F_LO).ln() / (F_HI / F_LO).ln()).clamp(0.0, 1.0)
}

/// The frequency at `x` across the axis.
pub fn x_freq(x: f32) -> f32 {
    F_LO * (F_HI / F_LO).powf(x)
}

/// A level in dB on an axis from `lo` to `hi` dB.
pub fn y_db(db: f32, lo: f32, hi: f32) -> f32 {
    ((db - lo) / (hi - lo)).clamp(0.0, 1.0)
}

/// A curve in dB across the frequency axis.
fn freq_curve(f: impl Fn(f32) -> f32, lo: f32, hi: f32) -> Vec<(f32, f32)> {
    (0..N)
        .map(|k| {
            let x = k as f32 / (N - 1) as f32;
            (x, y_db(f(x_freq(x)), lo, hi))
        })
        .collect()
}

fn freq_grid() -> Vec<(f32, String)> {
    [(100.0, "100"), (1000.0, "1k"), (10_000.0, "10k")]
        .iter()
        .map(|&(hz, s)| (freq_x(hz), s.to_string()))
        .collect()
}

fn line(pts: Vec<(f32, f32)>, color: Color) -> Trace {
    Trace {
        pts,
        color,
        width: 2.5,
        fill: false,
        dashed: false,
    }
}

fn filled(pts: Vec<(f32, f32)>, color: Color) -> Trace {
    Trace {
        fill: true,
        ..line(pts, color)
    }
}

fn dashed(pts: Vec<(f32, f32)>, color: Color) -> Trace {
    Trace {
        width: 1.5,
        dashed: true,
        ..line(pts, color)
    }
}

/// A parameter's value as the page writes it ("1.2 kHz", "-18 dB").
pub fn text(st: &FxState, id: u16) -> String {
    let Some(i) = st.index(id) else {
        return String::new();
    };
    let (n, u) = value_text(&st.params[i], st.values[i]);
    if u.is_empty() {
        n
    } else {
        format!("{n} {u}")
    }
}

/// A frequency as the page writes it.
fn hz_text(hz: f32) -> String {
    if hz >= 1000.0 {
        let k = hz / 1000.0;
        if k >= 10.0 {
            format!("{k:.1} kHz")
        } else {
            format!("{k:.2} kHz")
        }
        .replace(".00 ", " ")
    } else {
        format!("{hz:.0} Hz")
    }
}

/// A time as the page writes it.
fn ms_text(ms: f32) -> String {
    if ms >= 1000.0 {
        format!("{:.2} s", ms / 1000.0)
    } else if ms >= 10.0 {
        format!("{ms:.0} ms")
    } else {
        format!("{ms:.1} ms")
    }
}

/// The nearest note to `hz`, such as "A3" (C4 is middle C).
pub fn note_name(hz: f32) -> String {
    const NAMES: [&str; 12] = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    let midi = (69.0 + 12.0 * (hz / 440.0).log2()).round() as i32;
    format!(
        "{}{}",
        NAMES[midi.rem_euclid(12) as usize],
        midi.div_euclid(12) - 1
    )
}

/// The cards for `page` of the effect `st` shows. `focus` is the selected
/// parameter's index (the EQ rings the band it belongs to).
pub fn strip(
    st: &FxState,
    page: &str,
    focus: Option<usize>,
    scratch: &Scratch,
    t: &Theme,
) -> Strip {
    let mut s = Strip::default();
    let v = |id: u16| st.value(id);
    match scratch {
        Scratch::Eq(e) => eq_card(&mut s, st, page, focus, e, t),
        Scratch::Filter(f) => {
            let ty = text(st, filter::TYPE);
            let kind = if v(filter::TYPE) < 4.0 {
                format!("{ty} {}", text(st, filter::SLOPE))
            } else {
                format!("{ty} {}", text(st, filter::GAIN))
            };
            s.push(
                Card {
                    title: "Response".into(),
                    caption: format!("{kind} · {}", hz_text(v(filter::CUTOFF))),
                    traces: vec![filled(
                        freq_curve(|hz| f.response_db(hz), -30.0, 30.0),
                        t.accent,
                    )],
                    grid: freq_grid(),
                    midline: true,
                    dot: Some(freq_x(v(filter::CUTOFF))),
                    active: true,
                    ..Card::default()
                },
                page,
            );
        }
        Scratch::Dj(f) => {
            let caption = match f.corner() {
                None => "Off · the knob is in the middle".to_string(),
                Some((true, hz)) => format!("Low-pass · {}", hz_text(hz)),
                Some((false, hz)) => format!("High-pass · {}", hz_text(hz)),
            };
            s.push(
                Card {
                    title: "Response".into(),
                    caption,
                    traces: vec![filled(
                        freq_curve(|hz| f.response_db(hz), -30.0, 30.0),
                        t.accent,
                    )],
                    grid: freq_grid(),
                    midline: true,
                    dot: f.corner().map(|(_, hz)| freq_x(hz)),
                    active: true,
                    ..Card::default()
                },
                page,
            );
        }
        Scratch::Drive(d) => {
            let y = |out: f32| (0.5 + out / 2.5).clamp(0.0, 1.0);
            let curve: Vec<(f32, f32)> = (0..N)
                .map(|k| {
                    let x = k as f32 / (N - 1) as f32;
                    (x, y(d.curve(2.0 * x - 1.0)))
                })
                .collect();
            let unity = vec![(0.0, y(-1.0)), (1.0, y(1.0))];
            s.push(
                Card {
                    title: "Curve".into(),
                    caption: format!(
                        "{} · {} · bias {}",
                        text(st, drive::MODE),
                        text(st, drive::DRIVE),
                        text(st, drive::BIAS)
                    ),
                    traces: vec![dashed(unity, t.text_faint), line(curve, t.warn)],
                    midline: true,
                    active: true,
                    ..Card::default()
                },
                page,
            );
            let sine = |k: usize| {
                let ph = k as f32 / (N - 1) as f32;
                (ph, 0.7 * (2.0 * std::f32::consts::PI * ph).sin())
            };
            s.push(
                Card {
                    title: "A sine through it".into(),
                    caption: "at -3 dB, before tone and mix".into(),
                    traces: vec![
                        dashed(
                            (0..N).map(|k| (sine(k).0, y(sine(k).1))).collect(),
                            t.text_faint,
                        ),
                        line(
                            (0..N).map(|k| (sine(k).0, y(d.curve(sine(k).1)))).collect(),
                            t.warn,
                        ),
                    ],
                    midline: true,
                    active: true,
                    ..Card::default()
                },
                page,
            );
        }
        Scratch::Character(c) => character_cards(&mut s, st, page, c, t),
        Scratch::Comb(c) => comb_cards(&mut s, st, page, c, t),
        Scratch::Delay(d) => delay_card(&mut s, st, page, d, t),
        Scratch::Chorus(c) => {
            let base = v(chorus::DELAY);
            let voices = v(chorus::VOICES) as usize;
            let colors = [t.accent, t.good, t.warn, t.text];
            let traces = (0..4)
                .filter_map(|k| {
                    let pts: Option<Vec<(f32, f32)>> = (0..N)
                        .map(|i| {
                            let x = i as f32 / (N - 1) as f32;
                            c.delay_ms(k, (2.0 * x).fract())
                                .map(|d| (x, ((d - 0.5 * base) / base).clamp(0.0, 1.0)))
                        })
                        .collect();
                    pts.map(|p| line(p, colors[k]))
                })
                .collect();
            let swing = 0.5 * v(chorus::DEPTH) / 100.0 * base;
            s.push(
                Card {
                    title: "Sweep".into(),
                    caption: format!(
                        "{voices} voice{} · {} ± {} · {}",
                        if voices == 1 { "" } else { "s" },
                        ms_text(base),
                        ms_text(swing),
                        lfo_rate(st, chorus::SYNC, chorus::RATE)
                    ),
                    traces,
                    midline: true,
                    active: true,
                    ..Card::default()
                },
                page,
            );
        }
        Scratch::Flanger(f) => {
            let base = v(flanger::DELAY);
            let off = 0.25 * v(flanger::WIDTH) / 100.0;
            let sweep = |off: f32| -> Vec<(f32, f32)> {
                (0..N)
                    .map(|i| {
                        let x = i as f32 / (N - 1) as f32;
                        let d = f.delay_ms((2.0 * x + off).fract());
                        (x, ((d / base).log2() / 4.0 + 0.5).clamp(0.0, 1.0))
                    })
                    .collect()
            };
            s.push(
                Card {
                    title: "Sweep".into(),
                    caption: format!(
                        "{} · ±{:.1} oct · {}",
                        ms_text(base),
                        2.0 * v(flanger::DEPTH) / 100.0,
                        lfo_rate(st, flanger::SYNC, flanger::RATE)
                    ),
                    traces: vec![dashed(sweep(off), t.good), line(sweep(0.0), t.accent)],
                    midline: true,
                    active: true,
                    ..Card::default()
                },
                page,
            );
            s.push(
                Card {
                    title: "Comb".into(),
                    caption: format!("feedback {}", text(st, flanger::FEEDBACK)),
                    traces: vec![
                        dashed(
                            freq_curve(|hz| f.response_db(hz, 0.25), -30.0, 30.0),
                            t.text_faint,
                        ),
                        filled(
                            freq_curve(|hz| f.response_db(hz, 0.0), -30.0, 30.0),
                            t.accent,
                        ),
                    ],
                    grid: freq_grid(),
                    midline: true,
                    active: true,
                    ..Card::default()
                },
                page,
            );
        }
        Scratch::Phaser(p) => {
            let off = 0.25 * v(phaser::WIDTH) / 100.0;
            let sweep = |off: f32| -> Vec<(f32, f32)> {
                (0..N)
                    .map(|i| {
                        let x = i as f32 / (N - 1) as f32;
                        (x, freq_x(p.center_hz((2.0 * x + off).fract())))
                    })
                    .collect()
            };
            s.push(
                Card {
                    title: "Sweep".into(),
                    caption: format!(
                        "around {} · {}",
                        hz_text(v(phaser::CENTER)),
                        lfo_rate(st, phaser::SYNC, phaser::RATE)
                    ),
                    traces: vec![dashed(sweep(off), t.good), line(sweep(0.0), t.accent)],
                    active: true,
                    ..Card::default()
                },
                page,
            );
            s.push(
                Card {
                    title: "Notches".into(),
                    caption: format!("{} stages", text(st, phaser::STAGES)),
                    traces: vec![
                        dashed(
                            freq_curve(|hz| p.response_db(hz, 0.25), -30.0, 30.0),
                            t.text_faint,
                        ),
                        filled(
                            freq_curve(|hz| p.response_db(hz, 0.0), -30.0, 30.0),
                            t.accent,
                        ),
                    ],
                    grid: freq_grid(),
                    midline: true,
                    active: true,
                    ..Card::default()
                },
                page,
            );
        }
        Scratch::LoFi(l) => {
            let a = v(lofi::AMOUNT) / 100.0;
            let wear = [
                lofi::WOW,
                lofi::FLUTTER,
                lofi::SATURATION,
                lofi::CRACKLE,
                lofi::NOISE,
            ];
            s.push(
                Card {
                    title: "Wear".into(),
                    caption: format!("Amount {}", text(st, lofi::AMOUNT)),
                    bars: wear.iter().map(|&id| a * v(id) / 100.0).collect(),
                    bar_colors: vec![t.accent, t.accent, t.accent, t.warn, t.warn],
                    bar_labels: ["Wow", "Flutter", "Sat", "Crackle", "Noise"]
                        .iter()
                        .map(|s| s.to_string())
                        .collect(),
                    active: true,
                    ..Card::default()
                },
                page,
            );
            s.push(
                Card {
                    title: "Tone".into(),
                    caption: format!(
                        "{} to {}",
                        text(st, lofi::LOW_CUT),
                        text(st, lofi::HIGH_CUT)
                    ),
                    traces: vec![filled(
                        freq_curve(|hz| l.cuts_db(hz), -30.0, 30.0),
                        t.accent,
                    )],
                    grid: freq_grid(),
                    midline: true,
                    active: true,
                    ..Card::default()
                },
                page,
            );
        }
        Scratch::Comp(c) => comp_cards(&mut s, st, page, c, t),
        Scratch::Master(m) => {
            let y = |out: f32| (out / 1.25).clamp(0.0, 1.0);
            let curve: Vec<(f32, f32)> = (0..N)
                .map(|k| {
                    let x = k as f32 / (N - 1) as f32;
                    (x, y(m.curve(2.0 * x)))
                })
                .collect();
            let ceil = 10f32.powf(v(master::CEILING) / 20.0);
            s.push(
                Card {
                    title: "Curve".into(),
                    caption: format!(
                        "{} · ceiling {}",
                        text(st, master::MODE),
                        text(st, master::CEILING)
                    ),
                    traces: vec![
                        dashed(vec![(0.0, 0.0), (0.625, 1.0)], t.text_faint),
                        dashed(vec![(0.0, y(ceil)), (1.0, y(ceil))], t.warn),
                        line(curve, t.good),
                    ],
                    grid: vec![(0.5, "0 dBFS".to_string())],
                    active: true,
                    ..Card::default()
                },
                page,
            );
            let limiting = -v(master::REDUCTION);
            let (overs, clipped) = (v(master::OVERS), v(master::CLIPPED));
            let count = |n: f32| ((1.0 + n).log10() / 6.0).clamp(0.0, 1.0);
            s.push(
                Card {
                    title: "Catches".into(),
                    caption: format!(
                        "{:.1} dB · {} over{} · {} clipped",
                        -limiting,
                        overs as u64,
                        if overs as u64 == 1 { "" } else { "s" },
                        clipped as u64
                    ),
                    bars: vec![
                        (limiting / 12.0).clamp(0.0, 1.0),
                        count(overs),
                        count(clipped),
                    ],
                    bar_colors: vec![t.accent, t.warn, t.bad],
                    bar_labels: ["Limiting", "Overs", "Clipped"]
                        .iter()
                        .map(|s| s.to_string())
                        .collect(),
                    active: true,
                    ..Card::default()
                },
                page,
            );
        }
        Scratch::None => match st.info.id {
            "mpfree.fx.crush" => crush_card(&mut s, st, page, t),
            "mpfree.fx.cloud" => cloud_cards(&mut s, st, page, t),
            "mpfree.fx.ducker" => ducker_card(&mut s, st, page, t),
            "mpfree.fx.transient" => transient_card(&mut s, st, page, t),
            "mpfree.fx.pitch" => pitch_card(&mut s, st, page, t),
            "mpfree.fx.pitch_correct" => tune_card(&mut s, st, page, t),
            "mpfree.fx.harmonizer" => harmony_card(&mut s, st, page, t),
            _ => {}
        },
    }
    if let Some(levels) = st.levels {
        levels_card(&mut s, st, page, levels, t);
    }
    s
}

/// The meters every effect gets while its host watches it: the input and
/// output peaks (-60 to 0 dBFS), and the effect's own gain meter where it
/// has one (its first read-only parameter in dB: the compressor's
/// reduction, the ducker's gain), as bars.
fn levels_card(s: &mut Strip, st: &FxState, page: &str, (inp, out): (f32, f32), t: &Theme) {
    let bar = |db: f32| ((db + 60.0) / 60.0).clamp(0.0, 1.0);
    let db_text = |db: f32| {
        if db <= -60.0 {
            "-inf".to_string()
        } else {
            format!("{db:.1}")
        }
    };
    let mut bars = vec![bar(inp), bar(out)];
    let mut bar_colors = vec![t.accent, t.good];
    let mut bar_labels = vec!["In".to_string(), "Out".to_string()];
    let mut caption = format!("in {} · out {} dB", db_text(inp), db_text(out));
    let own = st
        .params
        .iter()
        .zip(&st.values)
        .find(|(p, _)| p.readonly && p.unit == mpfree_node::Unit::Db);
    if let Some((p, &v)) = own {
        let reach = p.min.abs().max(p.max.abs()).max(1.0);
        bars.push((v.abs() / reach).clamp(0.0, 1.0));
        bar_colors.push(t.warn);
        bar_labels.push(p.name.to_string());
        caption += &format!(" · {} {v:.1}", p.name.to_lowercase());
    }
    s.push(
        Card {
            title: "Levels".into(),
            caption,
            bars,
            bar_colors,
            bar_labels,
            active: true,
            ..Card::default()
        },
        page,
    );
}

/// An LFO's rate as a page writes it: the synced length or the Hz.
fn lfo_rate(st: &FxState, sync: u16, rate: u16) -> String {
    if st.value(sync) >= 1.0 {
        format!("{} a cycle", text(st, sync))
    } else {
        text(st, rate)
    }
}

/// The EQ: the whole response, each band a box at its frequency and gain
/// in its Q-Link's colour (dim while flat), the focused band ringed.
fn eq_card(
    s: &mut Strip,
    st: &FxState,
    page: &str,
    focus: Option<usize>,
    e: &mpfree_fx::Eq,
    t: &Theme,
) {
    let hz: Vec<f32> = (0..N).map(|k| x_freq(k as f32 / (N - 1) as f32)).collect();
    let mut db = vec![0.0; N];
    e.response_db(&hz, &mut db);
    let pts = db
        .iter()
        .enumerate()
        .map(|(k, &d)| (k as f32 / (N - 1) as f32, y_db(d, -18.0, 18.0)))
        .collect();
    let focus_band = focus
        .map(|i| st.params[i].id)
        .filter(|&id| (eq::band_type(0)..eq::OUTPUT).contains(&id))
        .map(|id| usize::from((id - eq::band_type(0)) / 4));
    let nodes = (0..eq::BANDS)
        .map(|b| {
            let gain = st.value(eq::gain(b));
            CardNode {
                x: freq_x(st.value(eq::freq(b))),
                y: y_db(gain, -18.0, 18.0).clamp(0.08, 0.92),
                label: (b + 1).to_string(),
                fill: QLINK_COLORS[b],
                text: t.bg,
                ring: (focus_band == Some(b)).then_some(t.text),
                dim: gain == 0.0,
                feedback: None,
            }
        })
        .collect();
    let cut = |f: u16, slope: u16, name: &str| {
        if st.value(slope) < 1.0 {
            String::new()
        } else {
            format!("{name} {} {} · ", text(st, f), text(st, slope))
        }
    };
    let caption = format!(
        "{}{}out {}",
        cut(eq::LOW_CUT, eq::LOW_SLOPE, "low cut"),
        cut(eq::HIGH_CUT, eq::HIGH_SLOPE, "high cut"),
        text(st, eq::OUTPUT)
    );
    s.node_pages
        .extend((0..eq::BANDS).map(|_| "Bands".to_string()));
    s.push(
        Card {
            title: "Response · ±18 dB".into(),
            caption,
            traces: vec![filled(pts, t.accent)],
            grid: freq_grid(),
            midline: true,
            nodes,
            active: page == "Bands",
            ..Card::default()
        },
        if page == "Cuts" { "Cuts" } else { "Bands" },
    );
}

/// The comb: its teeth on a linear axis (the harmonics evenly spaced, the
/// pitch and its note marked), and how long it rings.
fn comb_cards(s: &mut Strip, st: &FxState, page: &str, c: &mpfree_fx::Comb, t: &Theme) {
    let f0 = st.value(comb::PITCH);
    let g = st.value(comb::FEEDBACK) / 100.0;
    let top = (8.5 * f0).min(0.45 * st.rate as f32);
    const M: usize = 420;
    let pts = (0..M)
        .map(|k| {
            let x = k as f32 / (M - 1) as f32;
            (x, y_db(c.response_db(x * top), -36.0, 6.0))
        })
        .collect();
    // The peaks: the pitch and its harmonics, or with negative feedback
    // the odd harmonics of the note an octave down.
    let (first, harmonic) = if g < 0.0 { (0.5 * f0, 2) } else { (f0, 1) };
    let grid = (0..16)
        .map(|k| (first * (1 + harmonic * k) as f32, 1 + harmonic * k))
        .take_while(|&(hz, _)| hz < top)
        .map(|(hz, n)| {
            let label = if n == 1 {
                note_name(first)
            } else {
                n.to_string()
            };
            (hz / top, label)
        })
        .collect();
    s.push(
        Card {
            title: "Teeth".into(),
            caption: if g < 0.0 {
                format!("{} · odd harmonics of {}", hz_text(f0), note_name(first))
            } else {
                format!("{} · {}", hz_text(f0), note_name(f0))
            },
            traces: vec![filled(pts, t.accent)],
            grid,
            active: true,
            ..Card::default()
        },
        page,
    );
    // The ring: a pulse each period, falling by |g| a time (flipping sign
    // with negative feedback), to -60 dB. The damping takes the top off
    // sooner; the pulses show the feedback's share.
    let periods = if g.abs() < 1e-3 {
        1.0
    } else {
        3.0 / -(g.abs().log10())
    };
    let secs = (periods / f0).min(30.0);
    let shown = periods.clamp(4.0, 64.0);
    let mut pts = vec![(0.0, 0.5)];
    let n = shown.floor() as usize;
    for k in 0..n {
        let x = k as f32 / shown;
        let amp = 0.46 * g.abs().powi(k as i32) * if g < 0.0 && k % 2 == 1 { -1.0 } else { 1.0 };
        pts.push((x, 0.5));
        pts.push((x, 0.5 + amp));
        pts.push((x + 0.3 / shown, 0.5));
    }
    pts.push((1.0, 0.5));
    s.push(
        Card {
            title: "Ring".into(),
            caption: format!("to -60 dB in {}", ms_text(secs * 1000.0)),
            traces: vec![Trace {
                width: 1.5,
                ..line(pts, t.good)
            }],
            midline: true,
            active: true,
            ..Card::default()
        },
        page,
    );
}

/// The delay: the dry hit and each repeat as a line at its time and level
/// (left and right in their colours with ping-pong), on a beat grid.
fn delay_card(s: &mut Strip, st: &FxState, page: &str, d: &mpfree_fx::Delay, t: &Theme) {
    let time = d.time_ms(st.bpm);
    let fb = st.value(delay::FEEDBACK) / 100.0;
    let frozen = st.value(delay::FREEZE) >= 0.5;
    let ping = st.value(delay::PING_PONG) >= 0.5;
    let mut taps = Vec::new();
    let mut amp = 1.0f32;
    while taps.len() < 16 && (frozen || amp > 0.01) {
        taps.push(amp.min(1.0));
        if !frozen {
            amp *= fb;
        }
        if taps.len() >= 8 && frozen {
            break;
        }
    }
    let span = time * (taps.len() as f32 + 0.6);
    let x = |ms: f32| 0.01 + 0.98 * ms / span;
    let mut traces = vec![Trace {
        width: 4.0,
        ..line(vec![(x(0.0), 0.0), (x(0.0), 0.92)], t.text_dim)
    }];
    for (k, &a) in taps.iter().enumerate() {
        let at = x(time * (k + 1) as f32);
        let color = if ping && k % 2 == 1 { t.good } else { t.accent };
        traces.push(Trace {
            width: 4.0,
            ..line(vec![(at, 0.0), (at, 0.92 * a)], color)
        });
    }
    let beat = 60_000.0 / st.bpm.max(1.0) as f32;
    let step = if span / beat > 16.0 { 4.0 } else { 1.0 };
    let grid = (1..)
        .map(|b| b as f32 * step)
        .take_while(|&b| b * beat < span)
        .map(|b| {
            (
                x(b * beat),
                format!("{} beat{}", b as u32, if b == 1.0 { "" } else { "s" }),
            )
        })
        .collect();
    let synced = st.value(delay::SYNC) >= 1.0;
    let mut caption = if synced {
        format!("{} · {}", text(st, delay::SYNC), ms_text(time))
    } else {
        ms_text(time)
    };
    if ping {
        caption += " · ping-pong";
    }
    caption += &if frozen {
        " · frozen".to_string()
    } else {
        format!(" · {} repeats", taps.len())
    };
    s.push(
        Card {
            title: "Repeats".into(),
            caption,
            traces,
            grid,
            active: true,
            ..Card::default()
        },
        page,
    );
}

/// The compressor: its static curve with the threshold marked, and a burst
/// 12 dB over the threshold going in and out at its attack and release.
fn comp_cards(s: &mut Strip, st: &FxState, page: &str, c: &mpfree_fx::Compressor, t: &Theme) {
    let thr = st.value(comp::THRESHOLD);
    let y = |out: f32| y_db(out, -60.0, 12.0);
    let curve: Vec<(f32, f32)> = (0..N)
        .map(|k| {
            let x = k as f32 / (N - 1) as f32;
            (x, y(c.curve_db(-60.0 + 60.0 * x)))
        })
        .collect();
    let now = st.value(comp::REDUCTION);
    let mut caption = format!(
        "{}:1 · knee {}",
        trim_num(st.value(comp::RATIO)),
        text(st, comp::KNEE)
    );
    // The detector, when it isn't plain peak on the full band.
    if st.value(comp::DETECT) >= 0.5 {
        caption += " · RMS";
    }
    let sc = st.value(comp::SC_FILTER);
    if st
        .index(comp::SC_FILTER)
        .is_some_and(|i| sc > st.params[i].min)
    {
        caption += &format!(" · SC from {}", hz_text(sc));
    }
    if now < -0.05 {
        caption += &format!(" · now {now:.1} dB");
    }
    s.push(
        Card {
            title: "Curve".into(),
            caption,
            traces: vec![
                dashed(vec![(0.0, y(-60.0)), (1.0, y(0.0))], t.text_faint),
                line(curve, t.accent),
            ],
            grid: vec![((thr + 60.0) / 60.0, "thr".to_string())],
            active: true,
            ..Card::default()
        },
        page,
    );
    let (att, rel) = (st.value(comp::ATTACK), st.value(comp::RELEASE));
    let burst = (5.0 * att).max(30.0);
    let tail = 3.0 * rel;
    let lead = 0.1 * (burst + tail);
    let span = lead + burst + tail;
    let want = (thr + 12.0) - (c.curve_db(thr + 12.0) - st.value(comp::MAKEUP));
    let depth = want.max(0.1);
    let gain: Vec<(f32, f32)> = (0..N)
        .map(|k| {
            let x = k as f32 / (N - 1) as f32;
            let ms = x * span;
            let gr = if ms < lead {
                0.0
            } else if ms < lead + burst {
                want * (1.0 - (-(ms - lead) / att).exp())
            } else {
                let at_end = want * (1.0 - (-burst / att).exp());
                at_end * (-(ms - lead - burst) / rel).exp()
            };
            (x, 0.92 - 0.7 * gr / depth)
        })
        .collect();
    let (a, b) = (lead / span, (lead + burst) / span);
    let input = vec![
        (0.0, 0.04),
        (a, 0.04),
        (a, 0.14),
        (b, 0.14),
        (b, 0.04),
        (1.0, 0.04),
    ];
    s.push(
        Card {
            title: "Timing".into(),
            caption: format!("{} in · {} out · {want:.1} dB", ms_text(att), ms_text(rel)),
            traces: vec![dashed(input, t.text_faint), line(gain, t.good)],
            active: true,
            ..Card::default()
        },
        page,
    );
}

fn trim_num(v: f32) -> String {
    if (v - v.round()).abs() < 0.05 {
        format!("{}", v.round() as i32)
    } else {
        format!("{v:.1}")
    }
}

/// Crush: one cycle of a 441 Hz sine (dashed) and what comes out, held at
/// the crush's rate and rounded to its bits.
fn crush_card(s: &mut Strip, st: &FxState, page: &str, t: &Theme) {
    let bits = st.value(crush::BITS);
    let rate = st.value(crush::RATE);
    let levels = (bits - 1.0).exp2();
    const HZ: f32 = 441.0;
    const M: usize = 400;
    let y = |v: f32| 0.5 + 0.42 * v;
    let sine = |secs: f32| (2.0 * std::f32::consts::PI * HZ * secs).sin();
    let (mut dry, mut wet) = (Vec::new(), Vec::new());
    for k in 0..M {
        let x = k as f32 / (M - 1) as f32;
        let secs = x / HZ;
        let held = (secs * rate).floor() / rate;
        dry.push((x, y(sine(secs))));
        wet.push((x, y((sine(held) * levels).round() / levels)));
    }
    s.push(
        Card {
            title: "A sine through it".into(),
            caption: format!(
                "{} bits · {}{}",
                trim_num(bits),
                text(st, crush::RATE),
                if st.value(crush::DITHER) >= 0.5 {
                    " · dither"
                } else {
                    ""
                }
            ),
            traces: vec![dashed(dry, t.text_faint), line(wet, t.warn)],
            midline: true,
            active: true,
            ..Card::default()
        },
        page,
    );
}

/// The reverb: its decay (the pre-delay, the early taps, then the tail to
/// -60 dB, flat while frozen) and its tone (the input's cuts, and the
/// damping's loss each time round, dashed).
fn cloud_cards(s: &mut Strip, st: &FxState, page: &str, t: &Theme) {
    let v = |id: u16| st.value(id);
    let (pre, decay, size) = (v(cloud::PRE_DELAY), v(cloud::DECAY), v(cloud::SIZE));
    let frozen = v(cloud::FREEZE) >= 0.5;
    let span = pre + decay * if frozen { 1.0 } else { 1.15 };
    let x = |ms: f32| (ms / span).clamp(0.0, 1.0);
    let tail = if frozen {
        vec![(0.0, 0.0), (x(pre), 0.0), (x(pre), 0.9), (1.0, 0.9)]
    } else {
        vec![
            (0.0, 0.0),
            (x(pre), 0.0),
            (x(pre), 0.9),
            (x(pre + decay), 0.0),
            (1.0, 0.0),
        ]
    };
    let early = v(cloud::EARLY) / 100.0;
    let mut traces = vec![filled(tail, t.accent)];
    for k in 1..=5 {
        let at = x(pre + size * k as f32 / 5.0);
        let h = 0.9 * early * (1.0 - 0.12 * k as f32);
        if h > 0.01 {
            traces.push(Trace {
                width: 3.0,
                ..line(vec![(at, 0.0), (at, h)], t.good)
            });
        }
    }
    let reverb = page != "Tone";
    s.push(
        Card {
            title: "Decay · 60 dB".into(),
            caption: format!(
                "{} · {} · pre {}{}",
                text(st, cloud::CHARACTER),
                ms_text(decay),
                ms_text(pre),
                if frozen { " · frozen" } else { "" }
            ),
            traces,
            active: reverb,
            ..Card::default()
        },
        "Reverb",
    );
    let rate = st.rate.max(1);
    let cuts = freq_curve(|hz| Cloud::cuts_db_for(&st.values, rate, hz).0, -30.0, 30.0);
    let damp = freq_curve(|hz| Cloud::cuts_db_for(&st.values, rate, hz).1, -30.0, 30.0);
    s.push(
        Card {
            title: "Tone".into(),
            caption: format!(
                "{} to {} · damping {}",
                text(st, cloud::LOW_CUT),
                text(st, cloud::HIGH_CUT),
                text(st, cloud::DAMPING)
            ),
            traces: vec![dashed(damp, t.warn), filled(cuts, t.accent)],
            grid: freq_grid(),
            midline: true,
            active: !reverb,
            ..Card::default()
        },
        "Tone",
    );
}

/// Character: the curve a sample meets (Console, Tape, Iron) or, for
/// Soften, how its steps are held; and a sine through it as it plays.
fn character_cards(s: &mut Strip, st: &FxState, page: &str, c: &mpfree_fx::Character, t: &Theme) {
    let y = |out: f32| (0.5 + out / 2.5).clamp(0.0, 1.0);
    let soften = st.value(character::MODE) >= character::SOFTEN;
    let unity = vec![(0.0, y(-1.0)), (1.0, y(1.0))];
    let curve: Vec<(f32, f32)> = (0..N)
        .map(|k| {
            let x = k as f32 / (N - 1) as f32;
            (x, y(c.curve(2.0 * x - 1.0)))
        })
        .collect();
    s.push(
        Card {
            title: "Curve".into(),
            caption: if soften {
                format!(
                    "{} · no curve: it holds steps, not levels",
                    text(st, character::MODE)
                )
            } else {
                format!(
                    "{} · {} · quiet sounds pass at their level",
                    text(st, character::MODE),
                    text(st, character::AMOUNT)
                )
            },
            traces: vec![dashed(unity, t.text_faint), line(curve, t.warn)],
            midline: true,
            active: true,
            ..Card::default()
        },
        page,
    );
    // Soften shows a loud high sine (what it softens); the others a loud
    // low one (what they hold).
    let (hz, label) = if soften {
        (6000.0, "a 6 kHz sine at -3 dB")
    } else {
        (100.0, "a 100 Hz sine at -3 dB")
    };
    let mut out = vec![0.0f32; N];
    c.sine_through(hz, 0.7, &mut out);
    let phase = |k: usize| k as f32 / (N - 1) as f32;
    s.push(
        Card {
            title: "A sine through it".into(),
            caption: format!("{label}, before Output and Mix"),
            traces: vec![
                dashed(
                    (0..N)
                        .map(|k| {
                            let p = phase(k);
                            (p, y(0.7 * (2.0 * std::f32::consts::PI * p).sin()))
                        })
                        .collect(),
                    t.text_faint,
                ),
                line(
                    out.iter()
                        .enumerate()
                        .map(|(k, &v)| (phase(k), y(v)))
                        .collect(),
                    t.warn,
                ),
            ],
            midline: true,
            active: true,
            ..Card::default()
        },
        page,
    );
}

/// A fresh copy of the effect `st` shows, running at `rate`, with the
/// page's values: for the drawings that are the effect itself played
/// on a test signal.
fn copy_of(st: &FxState, mut n: Box<dyn Node>, rate: u32) -> Box<dyn Node> {
    n.prepare(rate, MAX_BLOCK);
    for (p, &v) in st.params.iter().zip(&st.values) {
        if !p.readonly {
            n.set_param(p.id, v);
        }
    }
    n.reset();
    n
}

/// Plays `input` (on both sides) through `n` from frame 0, stopped at
/// `bpm`, with `key` as the sidechain: the left side out.
fn play(n: &mut dyn Node, rate: u32, bpm: f64, input: &[f32], key: Option<&[f32]>) -> Vec<f32> {
    let (mut l, mut r) = (input.to_vec(), input.to_vec());
    let mut at = 0;
    while at < l.len() {
        let end = (at + MAX_BLOCK).min(l.len());
        let ctx = Ctx {
            bpm,
            sidechain: key.map(|k| (&k[at..end], &k[at..end])),
            ..Ctx::new(rate, at as u64)
        };
        n.process(&ctx, &[], &mut l[at..end], &mut r[at..end]);
        at = end;
    }
    l
}

/// `x` cut into `N` stretches, each folded to one value by `f`.
fn buckets(x: &[f32], f: impl Fn(f32, f32) -> f32, start: f32) -> Vec<f32> {
    (0..N)
        .map(|k| {
            let (a, b) = (
                k * x.len() / N,
                ((k + 1) * x.len() / N).max(k * x.len() / N + 1),
            );
            x[a.min(x.len() - 1)..b.min(x.len())]
                .iter()
                .fold(start, |m, &v| f(m, v))
        })
        .collect()
}

/// A rate that plays `ms` in about 4000 frames: the drawing's 180 points
/// need no more, and the page redraws on every turn of a knob.
fn draw_rate(ms: f32) -> u32 {
    (4_000_000.0 / ms.max(1.0)).clamp(1_000.0, 48_000.0) as u32
}

/// The Ducker: its gain over time, played on a steady signal. On Tempo,
/// two cycles of the Rate at the song's tempo (a trigger at each line); on
/// Sidechain, one 10 ms key 6 dB over full scale (the dashed pulse), until
/// the duck has come back.
fn ducker_card(s: &mut Strip, st: &FxState, page: &str, t: &Theme) {
    let v = |id: u16| st.value(id);
    let tempo = v(ducker::SOURCE) >= 0.5;
    let bpm = if st.bpm > 0.0 { st.bpm } else { 120.0 };
    let (att, hold, rel) = (v(ducker::ATTACK), v(ducker::HOLD), v(ducker::RELEASE));
    let rates = DUCKER_RATE_BEATS.len() - 1;
    let beats = DUCKER_RATE_BEATS[(v(ducker::RATE).max(0.0) as usize).min(rates)];
    let lead = 0.1 * (att + hold + rel);
    // The duck played over `span` ms: the gain out, and the key in.
    let render = |span: f32| {
        let rate = draw_rate(span);
        let frames = (span * 0.001 * rate as f32).ceil().max(2.0) as usize;
        let at = (lead * 0.001 * rate as f32) as usize;
        let key: Vec<f32> = (0..frames)
            .map(|i| (i >= at && i < at + rate as usize / 100) as u8 as f32 * 2.0)
            .collect();
        let mut n = copy_of(st, Box::new(Ducker::new()), rate);
        let ones = vec![1.0f32; frames];
        (
            play(n.as_mut(), rate, bpm, &ones, (!tempo).then_some(&key[..])),
            rate,
        )
    };
    let span = if tempo {
        2.0 * (beats * 60_000.0 / bpm) as f32
    } else {
        // Long enough for the key to fall back under the threshold and the
        // release to finish, then cut to where the gain is back.
        let most = lead + 300.0 + 1.2 * (att + hold + rel);
        let (out, rate) = render(most);
        let back = out.iter().rposition(|&g| g < 0.99).unwrap_or(0);
        (1.1 * back as f32 * 1000.0 / rate as f32).clamp(3.0 * lead + 1.0, most)
    };
    let (out, _) = render(span);
    let lo = -(v(ducker::DEPTH).max(12.0) * 1.1);
    let gain: Vec<(f32, f32)> = buckets(&out, f32::min, f32::MAX)
        .into_iter()
        .enumerate()
        .map(|(k, g)| {
            let db = 20.0 * g.max(1e-6).log10();
            (k as f32 / (N - 1) as f32, 0.12 + 0.8 * y_db(db, lo, 0.0))
        })
        .collect();
    let mut traces = Vec::new();
    let mut grid = Vec::new();
    if tempo {
        grid = vec![(0.0, text(st, ducker::RATE)), (0.5, String::new())];
    } else {
        let (a, b) = (lead / span, ((lead + 10.0) / span).min(1.0));
        traces.push(dashed(
            vec![
                (0.0, 0.04),
                (a, 0.04),
                (a, 0.1),
                (b, 0.1),
                (b, 0.04),
                (1.0, 0.04),
            ],
            t.text_faint,
        ));
    }
    traces.push(line(gain, t.good));
    let trigger = if tempo {
        format!("every {}", text(st, ducker::RATE))
    } else {
        format!("key over {}", text(st, ducker::THRESHOLD))
    };
    s.push(
        Card {
            title: "Pump".into(),
            caption: format!(
                "{trigger} · {} down · release {}",
                text(st, ducker::DEPTH),
                ms_text(rel)
            ),
            traces,
            grid,
            active: true,
            ..Card::default()
        },
        page,
    );
}

/// The Transient shaper: a drum hit's level over 400 ms (dashed) and what
/// the shaper makes of it (the line), played through a copy of it.
fn transient_card(s: &mut Strip, st: &FxState, page: &str, t: &Theme) {
    const SPAN: f32 = 400.0;
    let rate = draw_rate(SPAN).max(8_000);
    let frames = (SPAN * 0.001 * rate as f32) as usize;
    let lead = rate as usize / 50;
    // A hit: a fast crack and a slower room tail, on noise.
    let mut seed = 0x2545_f491_u32;
    let hit: Vec<f32> = (0..frames)
        .map(|i| {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let noise = (seed >> 8) as f32 / (1u32 << 23) as f32 - 1.0;
            if i < lead {
                return 0.0;
            }
            let ms = (i - lead) as f32 * 1000.0 / rate as f32;
            0.5 * noise * ((-ms / 25.0).exp() + 0.12 * (-ms / 150.0).exp())
        })
        .collect();
    let mut n = copy_of(st, Box::new(TransientShaper::new()), rate);
    let out = play(n.as_mut(), rate, st.bpm, &hit, None);
    let env = |x: &[f32]| -> Vec<(f32, f32)> {
        buckets(x, |m, v| m.max(v.abs()), 0.0)
            .into_iter()
            .enumerate()
            .map(|(k, p)| {
                let db = 20.0 * p.max(1e-6).log10();
                (k as f32 / (N - 1) as f32, y_db(db, -54.0, 6.0))
            })
            .collect()
    };
    s.push(
        Card {
            title: "Hit".into(),
            caption: format!(
                "attack {} · sustain {} · speed {}",
                text(st, transient::ATTACK),
                text(st, transient::SUSTAIN),
                text(st, transient::SPEED)
            ),
            traces: vec![dashed(env(&hit), t.text_faint), line(env(&out), t.accent)],
            active: true,
            ..Card::default()
        },
        page,
    );
}

/// The Pitch shifter: where the sound goes, from two octaves down to two
/// up (octaves dashed). The dry note is the middle line; the shifted one
/// comes in after half a window, and with feedback each pass is shifted
/// again, fainter, half a window later (the stairs), while it stays in
/// range and over -30 dB.
fn pitch_card(s: &mut Strip, st: &FxState, page: &str, t: &Theme) {
    let v = |id: u16| st.value(id);
    let shift = v(pitch::PITCH) + v(pitch::FINE) / 100.0;
    let win = v(pitch::WINDOW);
    let fb = v(pitch::FEEDBACK) / 100.0;
    let span = 5.0 * win;
    let y = |semis: f32| 0.5 + semis / 52.0;
    let mut traces: Vec<Trace> = [-24.0, -12.0, 12.0, 24.0]
        .iter()
        .map(|&o| dashed(vec![(0.0, y(o)), (1.0, y(o))], t.text_faint))
        .collect();
    traces.push(line(vec![(0.0, y(0.0)), (1.0, y(0.0))], t.text_faint));
    let mut level = 1.0f32;
    for k in 1..=8 {
        let at = (k as f32 * 0.5 * win / span).min(1.0);
        let semis = k as f32 * shift;
        if at >= 1.0 || semis.abs() > 24.5 || level < 0.03 {
            break;
        }
        let first = k == 1;
        traces.push(Trace {
            width: if first { 3.0 } else { 2.0 },
            ..line(
                vec![(at, y(semis)), (1.0, y(semis))],
                if first { t.accent } else { t.good },
            )
        });
        level *= fb;
    }
    let mut caption = format!("{} · {} window", text(st, pitch::PITCH), ms_text(win));
    if v(pitch::FINE).abs() >= 0.5 {
        caption += &format!(" · fine {}", text(st, pitch::FINE));
    }
    if v(pitch::SPREAD) >= 0.5 {
        caption += &format!(" · spread {}", text(st, pitch::SPREAD));
    }
    if fb > 0.005 {
        caption += &format!(" · feedback {}", text(st, pitch::FEEDBACK));
    }
    s.push(
        Card {
            title: "Shift".into(),
            caption,
            traces,
            active: true,
            ..Card::default()
        },
        page,
    );
}

/// The key's notes across one octave from its tonic, as grid lines: each
/// note of the scale, named when the scale has seven or fewer (else the
/// tonic alone), so a page shows which notes it will sing.
fn scale_grid(key: usize, scale: usize) -> Vec<(f32, String)> {
    let notes: Vec<i32> = (0..12)
        .filter(|&k| nearest_note((60 + key as i32 + k) as f32, key, scale) == 60 + key as i32 + k)
        .collect();
    let named = notes.len() <= 7;
    notes
        .iter()
        .map(|&k| {
            let name = if named || k == 0 {
                VOCAL_KEYS[(key + k as usize) % 12].to_string()
            } else {
                String::new()
            };
            ((k as f32 + 0.5) / 12.0, name)
        })
        .collect()
}

/// The note sung, as an x across the octave from the key (None while
/// unvoiced), from the effect's Note meter.
fn sung_x(st: &FxState, note_id: u16, key: usize) -> Option<f32> {
    let note = st.value(note_id);
    (note >= 1.0).then(|| (((note as i32 - key as i32).rem_euclid(12)) as f32 + 0.5) / 12.0)
}

/// Pitch correct: the note sung (across, one octave from the key) against
/// the note heard (up): stairs on the scale's notes at full Amount, a
/// slope part way to them below it, and the dashed line where nothing
/// would change. The dot is the note being sung now.
fn tune_card(s: &mut Strip, st: &FxState, page: &str, t: &Theme) {
    let v = |id: u16| st.value(id);
    let (key, scale) = (v(tune::KEY) as usize, v(tune::SCALE) as usize);
    let amount = v(tune::AMOUNT) / 100.0;
    let base = 60.0 + key as f32;
    // Up the card: half a semitone under the tonic to half over the octave.
    let y = |midi: f32| ((midi - base + 0.5) / 13.0).clamp(0.0, 1.0);
    let pts: Vec<(f32, f32)> = (0..=N)
        .map(|k| {
            let x = k as f32 / N as f32;
            let sung = base - 0.5 + 12.0 * x;
            let to = nearest_note(sung, key, scale) as f32;
            (x, y(sung + amount * (to - sung)))
        })
        .collect();
    let even = vec![(0.0, y(base - 0.5)), (1.0, y(base + 11.5))];
    let mut caption = format!(
        "{} {} · {} · {}",
        text(st, tune::KEY),
        text(st, tune::SCALE),
        if v(tune::SPEED) < 0.5 {
            "snap".to_string()
        } else {
            ms_text(v(tune::SPEED))
        },
        text(st, tune::AMOUNT)
    );
    if v(tune::NOTE) >= 1.0 {
        caption += &format!(" · {} {:+.0} c", text(st, tune::NOTE), v(tune::CORRECTION));
    }
    s.push(
        Card {
            title: "Tune".into(),
            caption,
            traces: vec![line(pts, t.accent), dashed(even, t.text_faint)],
            grid: scale_grid(key, scale),
            dot: sung_x(st, tune::NOTE, key),
            active: true,
            ..Card::default()
        },
        page,
    );
}

/// The Harmonizer: for each note sung across the octave from the key, how
/// far up or down each voice sings (an octave either way, unison in the
/// middle, dashed), in its Q-Link's colour, faint while its level is
/// down. A 3rd up steps between major and minor as the key has it. The
/// dot is the note being sung now.
fn harmony_card(s: &mut Strip, st: &FxState, page: &str, t: &Theme) {
    let v = |id: u16| st.value(id);
    let (key, scale) = (v(harm::KEY) as usize, v(harm::SCALE) as usize);
    let y = |semis: f32| 0.5 + semis / 26.0;
    let mut traces = Vec::new();
    let mut names = Vec::new();
    let voices = [
        (harm::VOICE1, harm::LEVEL1, QLINK_COLORS[0]),
        (harm::VOICE2, harm::LEVEL2, QLINK_COLORS[2]),
    ];
    for (pick, level, color) in voices {
        let k = v(pick) as usize;
        let mut pts = Vec::new();
        for n in 0..12 {
            let sung = 60 + key as i32 + n;
            // The shift the voice makes on this note (from the note of the
            // scale nearest it), flat across its twelfth of the card.
            let Some(semis) = interval_semis(k, nearest_note(sung as f32, key, scale), key, scale)
            else {
                break;
            };
            pts.push((n as f32 / 12.0, y(semis as f32)));
            pts.push(((n + 1) as f32 / 12.0, y(semis as f32)));
        }
        if pts.is_empty() {
            continue;
        }
        let color = if v(level) < 0.5 { t.text_faint } else { color };
        traces.push(line(pts, color));
        names.push(text(st, pick));
    }
    traces.push(dashed(vec![(0.0, y(0.0)), (1.0, y(0.0))], t.text_faint));
    traces.extend(
        [-12.0, 12.0]
            .iter()
            .map(|&o| dashed(vec![(0.0, y(o)), (1.0, y(o))], t.text_faint)),
    );
    let mut caption = format!("{} {}", text(st, harm::KEY), text(st, harm::SCALE));
    if names.is_empty() {
        caption += " · no voices";
    } else {
        caption += &format!(" · {}", names.join(" · "));
    }
    if v(harm::NOTE) >= 1.0 {
        caption += &format!(" · {}", text(st, harm::NOTE));
    }
    s.push(
        Card {
            title: "Voices".into(),
            caption,
            traces,
            grid: scale_grid(key, scale),
            dot: sung_x(st, harm::NOTE, key),
            active: true,
            ..Card::default()
        },
        page,
    );
}
