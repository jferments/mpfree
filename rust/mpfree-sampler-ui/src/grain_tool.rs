//! The Sample page's **Grain** tool (the one Sample page's step 5, Jay 18
//! Nov 2026: "Grain could also be in the waveform editing page"): how the
//! sound plays as a grain cloud, on the one wave.
//!
//! - **The grain window** on the wave: a band from where grains start,
//!   one grain long, over the band the spray scatters their starts in.
//!   Its tabs drag it: **GRAIN** moves where grains start (the position),
//!   **SIZE** its end (the grain size). A tap on the wave moves the start
//!   there. It takes the Grain page's touch pad's place.
//! - **The Grain section**, the panel's first, holds the Grain page's
//!   settings a part at a time: a switch along its top (Grains, Shape,
//!   Filter, Keys, LFO, Envelope, Presets) and the part's controls under
//!   it ([`GrainPart`]). Shape holds the play mode, the grain mode and
//!   **Quality** (Clean, Tape, Lo-fi). A grain mode shows only the rows it
//!   uses, as the Grain page did ([`Row::used_in`]).
//! - **Q-Links 1 to 4** turn Position, Size, Density and Pitch, as on the
//!   Grain page (with SHIFT: the spray, finer size and density, the pitch
//!   spray).
//!
//! The rows, their values and how they step are the Grain page's own
//! ([`crate::grain_tab`]), so the two never disagree.

use mpfree_sampler::grain_presets;
use mpfree_sampler::model::{GrainFilterKind, GrainQuality, MotionTarget, Region};
use mpfree_ui::wave::{GrainSpan, Marker, MarkerKind, TabSide};
use mpfree_ui::Color;

use crate::grain_tab::{self, size_ms_at, Row};
use crate::grain_widgets::GRAIN_COLOR;
use crate::panel::{Control, Item, Look, Panel, Section};
use crate::widgets::Setting;

/// The Grain section's parts, as its switch names them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GrainPart {
    /// How grains scatter: scan, the sprays, timing, reverse, width,
    /// shape and level.
    #[default]
    Grains,
    /// The play mode, the grain mode, Quality, and the spray's shape,
    /// pitch quantise and tempo sync.
    Shape,
    /// A filter on each grain.
    Filter,
    /// What keys do: size and scan following them, glide.
    Keys,
    /// The LFO and what it moves.
    Lfo,
    /// The envelope and what it moves.
    Envelope,
    /// The eight starting points.
    Presets,
}

impl GrainPart {
    pub const ALL: [GrainPart; 7] = [
        GrainPart::Grains,
        GrainPart::Shape,
        GrainPart::Filter,
        GrainPart::Keys,
        GrainPart::Lfo,
        GrainPart::Envelope,
        GrainPart::Presets,
    ];

    pub fn label(self) -> &'static str {
        match self {
            GrainPart::Grains => "Grains",
            GrainPart::Shape => "Shape",
            GrainPart::Filter => "Grain filter",
            GrainPart::Keys => "Keys",
            GrainPart::Lfo => "Grain LFO",
            GrainPart::Envelope => "Grain env",
            GrainPart::Presets => "Presets",
        }
    }

    pub fn index(self) -> usize {
        GrainPart::ALL.iter().position(|p| *p == self).unwrap_or(0)
    }

    /// The part's lines of (row, its name over it), before a grain mode
    /// leaves rows out. One line of knobs, or two of selectors, so the
    /// part fits the panel under its switch.
    fn layout(self) -> &'static [&'static [(Row, bool)]] {
        match self {
            GrainPart::Grains => &[&[
                (Row::Scan, false),
                (Row::Spray, false),
                (Row::SizeSpray, false),
                (Row::PitchSpray, false),
                (Row::LevelSpray, false),
                (Row::Timing, false),
                (Row::Reverse, false),
                (Row::Spread, false),
                (Row::Level, false),
                (Row::SprayShape, true),
            ]],
            GrainPart::Shape => &[
                &[
                    (Row::Mode, true),
                    (Row::GrainMode, true),
                    (Row::Quality, true),
                ],
                &[(Row::Shape, true), (Row::Quantise, true), (Row::Sync, true)],
            ],
            GrainPart::Filter => &[&[
                (Row::Filter, true),
                (Row::Cutoff, false),
                (Row::Resonance, false),
                (Row::FilterSpray, false),
            ]],
            GrainPart::Keys => &[&[
                (Row::KeySize, false),
                (Row::KeyScan, false),
                (Row::Glide, false),
            ]],
            GrainPart::Lfo => &[&[
                (Row::LfoTo, true),
                (Row::LfoShape, true),
                (Row::LfoRate, false),
                (Row::LfoDepth, false),
                (Row::LfoSync, false),
            ]],
            GrainPart::Envelope => &[&[
                (Row::EnvTo, true),
                (Row::EnvDepth, false),
                (Row::EnvAttack, false),
                (Row::EnvDecay, false),
                (Row::EnvSustain, false),
            ]],
            GrainPart::Presets => &[&[(Row::Preset, false)], &[(Row::PresetMore, false)]],
        }
    }

    /// The part's lines for `r`'s grain mode: the rows it uses.
    pub fn lines(self, r: &Region) -> Vec<Vec<(Row, bool)>> {
        let mode = r.grain_mode;
        self.layout()
            .iter()
            .map(|line| {
                line.iter()
                    .copied()
                    .filter(|(row, _)| {
                        matches!(row, Row::Preset | Row::PresetMore) || row.used_in(mode)
                    })
                    .collect::<Vec<_>>()
            })
            .filter(|line| !line.is_empty())
            .collect()
    }

    /// The part's rows for `r`, in the order the wheel walks them.
    pub fn rows(self, r: &Region) -> Vec<Row> {
        self.lines(r)
            .into_iter()
            .flatten()
            .map(|(row, _)| row)
            .collect()
    }

    /// The part of the Grain section that shows `row` (None for the
    /// Q-Links' rows, which no part shows).
    pub fn of(row: Row) -> Option<GrainPart> {
        GrainPart::ALL
            .iter()
            .copied()
            .find(|p| p.layout().iter().any(|l| l.iter().any(|(r, _)| *r == row)))
    }
}

/// What the Grain section's note says with part `part` open, when the
/// part has its own word (as the Grain page's sections did): what the LFO,
/// the envelope and the filter do, the preset lit. None: the cloud's.
pub fn part_note(r: &Region, part: GrainPart) -> Option<&'static str> {
    let g = &r.grain;
    let moves = |t: MotionTarget, on: &'static str| {
        if t == MotionTarget::Off {
            "off: pick what it moves"
        } else {
            on
        }
    };
    match part {
        GrainPart::Lfo => Some(moves(g.lfo.target, "over and over, from each hit")),
        GrainPart::Envelope => Some(moves(g.env.target, "once, from each hit")),
        GrainPart::Filter => Some(if g.filter.kind == GrainFilterKind::Off {
            "off: pick a pass"
        } else {
            "set as each grain starts"
        }),
        GrainPart::Presets => Some(grain_presets::matching(r).map_or(
            "a starting point: the sound, position, level and quality stay",
            |i| grain_presets::presets()[i].about,
        )),
        _ => None,
    }
}

/// The list's row of the parts' switch; the part's rows follow it.
pub const PARTS_ROW: usize = 0;
/// Spray shape's width at the end of Grains' line of knobs.
const SPRAY_SHAPE_W: i32 = 124;
/// The grain filter's passes (Off, Low pass, Band pass, High pass).
const PASSES_W: i32 = 330;

/// The Grain section as the panel's rows, controls and layout, for part
/// `part` of `r`. `note` is the section's note (what the cloud is doing).
pub fn section(r: &Region, part: GrainPart, note: &str) -> (Vec<Setting>, Vec<Control>, Panel) {
    let mut rows = vec![Setting {
        name: "Grain".into(),
        value: part.label().into(),
    }];
    let mut controls = vec![Control::new(Look::Choice {
        options: GrainPart::ALL.iter().map(|p| p.label().into()).collect(),
        on: part.index(),
        off: Vec::new(),
    })];
    let mut lines = vec![vec![Item::new(PARTS_ROW)]];
    for line in part.lines(r) {
        let mut items = Vec::new();
        for (row, labelled) in line {
            let item = Item::new(rows.len());
            items.push(match (labelled, row) {
                // A selector among Grains' knobs, and the passes' four
                // buttons beside the filter's knobs: wide enough for their
                // words.
                (true, Row::SprayShape) => item.labelled().wide(SPRAY_SHAPE_W),
                (true, Row::Filter) => item.labelled().wide(PASSES_W),
                (true, _) => item.labelled(),
                // Grains' nine knobs are narrow: their names on two lines
                // ("Size / spray"), so each reads whole.
                _ if part == GrainPart::Grains => item.two_lines(),
                _ => item,
            });
            rows.push(Setting {
                name: row.short().into(),
                value: grain_tab::panel_text(r, row),
            });
            controls.push(control(r, row));
        }
        lines.push(items);
    }
    let panel = Panel {
        sections: vec![Section::new("Grain", 760, lines).note(note).from_top()],
    };
    (rows, controls, panel)
}

/// Row `row`'s control in the Grain section: the Grain page's, with
/// Quality's three as buttons (Clean, Tape, Lo-fi), and the grains' Shape
/// as ‹ value ›, so it fits on Shape's second line under the selectors.
fn control(r: &Region, row: Row) -> Control {
    if row == Row::Shape {
        return Control::new(Look::Chip);
    }
    if row == Row::Quality {
        return Control::new(Look::Choice {
            options: GrainQuality::ALL
                .iter()
                .map(|q| q.name().to_string())
                .collect(),
            on: r.grain.quality.index(),
            off: Vec::new(),
        });
    }
    grain_tab::control_of(r, row)
}

/// The grain window over the wave for `r` (a sound at `rate`, played at
/// `bpm`): where grains start, one grain long, and the spray's band
/// (`spray_now`, the spray as it plays, when a voice says).
pub fn span(r: &Region, rate: u32, bpm: f64, spray_now: Option<f32>) -> GrainSpan {
    let g = &r.grain;
    let len = r.frames().max(1);
    let at = r.start + (f64::from(g.position.clamp(0.0, 1.0)) * len as f64).round() as u64;
    let grain = (f64::from(size_ms_at(g, bpm)) * f64::from(rate) / 1000.0).round() as u64;
    let spray = spray_now.unwrap_or(g.spray).clamp(0.0, 1.0);
    GrainSpan {
        at: at.min(r.end),
        len: grain.max(1),
        spray: (f64::from(spray) * len as f64).round() as u64,
        within: (r.start, r.end),
        color: Color::rgb(GRAIN_COLOR),
    }
}

/// The window's two tabs, after the other marks: GRAIN where grains
/// start, SIZE where the first one ends; GRAIN's tab sits left of its
/// line and SIZE's right of it, so a short window keeps both clear.
pub fn markers(s: &GrainSpan) -> [Marker; 2] {
    let c = Color::rgb(GRAIN_COLOR);
    [
        Marker::new(s.at, GRAIN_TAB, MarkerKind::Cue)
            .colored(c)
            .tab_on(TabSide::Before),
        Marker::new(s.at + s.len, SIZE_TAB, MarkerKind::Cue)
            .colored(c)
            .tab_on(TabSide::After),
    ]
}

/// The window's tabs' names on the wave.
pub const GRAIN_TAB: &str = "GRAIN";
pub const SIZE_TAB: &str = "SIZE";

/// `r` with the grain window's start dragged to `frame`.
pub fn moved_start(r: &Region, frame: u64) -> Region {
    let len = r.frames().max(1) as f64;
    let p = (frame.clamp(r.start, r.end) - r.start) as f64 / len;
    grain_tab::with_position(r, p as f32)
}

/// `r` with the grain window's end dragged to `frame` (grains `frame`
/// less the start long).
pub fn moved_end(r: &Region, frame: u64, rate: u32, bpm: f64) -> Region {
    let s = span(r, rate, bpm, None);
    grain_tab::with_size_frames(r, frame.saturating_sub(s.at).max(1), rate, bpm)
}
