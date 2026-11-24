//! The Sample page's **Grain** tool (the one Sample page's step 5, Jay 18
//! Nov 2026: "Grain could also be in the waveform editing page"): how the
//! sound plays as a grain cloud, on the one wave.
//!
//! - **Picking Grain plays the sound as grains**: on a sound in another
//!   mode the tool sets Granular, one undo step (Jay's device notes, 20
//!   Nov 2026: the Grain tool is granular; there's no separate mode to
//!   find). The Trim tool's Mode puts it back.
//! - **The grain window** on the wave: a band from where grains start,
//!   one grain long, over the band the spray scatters their starts in.
//!   Its tabs drag it: **GRAIN** moves where grains start (the position),
//!   **SIZE** its end (the grain size). A tap on the wave moves the start
//!   there. It takes the Grain page's touch pad's place.
//! - **The Grain section**, the panel's first, holds the Grain page's
//!   settings a part at a time: a switch along its top (Grains, Shape,
//!   Filter, Keys, Mod, LFO, Env, Presets) and the part's controls under
//!   it ([`GrainPart`]). Shape holds the grain mode and
//!   **Quality** (Clean, Tape, Lo-fi). A grain mode shows only the rows it
//!   uses, as the Grain page did ([`Row::used_in`]).
//! - **Mod** (sampler refactor step 17c, layout A as Jay picked it, 24
//!   Nov 2026): the layer's four routes from velocity, pressure or the key
//!   to one of its grain settings each. Two pickers, From and To, edit the
//!   route outlined; four depth knobs, named by their routes ("Vel →
//!   size"), turn each route's depth, and a touch on one outlines it.
//! - **Q-Links 1 to 4** turn Position, Size, Density and Pitch, as on the
//!   Grain page (with SHIFT: the spray, finer size and density, the pitch
//!   spray).
//!
//! The rows, their values and how they step are the Grain page's own
//! ([`crate::grain_tab`]), so the two never disagree.

use mpfree_sampler::grain_presets;
use mpfree_sampler::model::{
    GrainFilterKind, GrainQuality, MotionTarget, Region, RouteFrom, GRAIN_ROUTES,
};
use mpfree_ui::wave::{GrainSpan, Marker, MarkerKind, TabSide};
use mpfree_ui::{Paint, Role};

use crate::grain_tab::{self, size_ms_at, Row};
use crate::panel::{Control, Item, Look, Panel, Section};
use crate::widgets::Setting;

/// The Grain section's parts, as its switch names them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GrainPart {
    /// How grains scatter: scan, the sprays, timing, reverse, width,
    /// shape and level.
    #[default]
    Grains,
    /// The grain mode, Quality, and the spray's shape,
    /// pitch quantise and tempo sync.
    Shape,
    /// A filter on each grain.
    Filter,
    /// What keys do: size and scan following them, glide.
    Keys,
    /// The routes from velocity, pressure and the key (step 17c).
    Mod,
    /// The LFO and what it moves.
    Lfo,
    /// The envelope and what it moves.
    Envelope,
    /// The eight starting points.
    Presets,
}

impl GrainPart {
    pub const ALL: [GrainPart; 8] = [
        GrainPart::Grains,
        GrainPart::Shape,
        GrainPart::Filter,
        GrainPart::Keys,
        GrainPart::Mod,
        GrainPart::Lfo,
        GrainPart::Envelope,
        GrainPart::Presets,
    ];

    pub fn label(self) -> &'static str {
        match self {
            GrainPart::Grains => "Grains",
            GrainPart::Shape => "Shape",
            // Short, so eight fit along the switch (step 17c); the
            // section's title says they are the grains'.
            GrainPart::Filter => "Filter",
            GrainPart::Keys => "Keys",
            GrainPart::Mod => "Mod",
            GrainPart::Lfo => "LFO",
            GrainPart::Envelope => "Env",
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
                &[(Row::GrainMode, true), (Row::Quality, true)],
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
            GrainPart::Mod => &[&[
                (Row::RouteFrom, true),
                (Row::RouteTo, true),
                (Row::Route1, false),
                (Row::Route2, false),
                (Row::Route3, false),
                (Row::Route4, false),
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
                        matches!(
                            row,
                            Row::Preset | Row::PresetMore | Row::RouteFrom | Row::RouteTo
                        ) || row.route().is_some()
                            || row.used_in(mode)
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

/// What the Grain section's note says with the Mod part open: what route
/// `route` (from 0, the one outlined) does, in words ("route 1: harder
/// hits move the size up").
pub fn route_note(r: &Region, route: usize) -> String {
    let k = route.min(GRAIN_ROUTES - 1);
    let x = r.grain_routes.0[k];
    let to = grain_tab::route_to_name(x.to);
    let to = if to.starts_with("LFO") {
        to.to_string()
    } else {
        to.to_lowercase()
    };
    let way = if x.depth < 0.0 { "down" } else { "up" };
    let what = match x.from {
        RouteFrom::Off => return format!("route {}: off, pick what moves it", k + 1),
        _ if x.depth == 0.0 => return format!("route {}: turn its knob to set how far", k + 1),
        RouteFrom::Velocity => "harder hits move",
        RouteFrom::Pressure => "pressing harder moves",
        RouteFrom::Key => "keys above the root move",
    };
    format!("route {}: {what} the {to} {way}", k + 1)
}

/// The list's row of the parts' switch; the part's rows follow it.
pub const PARTS_ROW: usize = 0;
/// Spray shape's width at the end of Grains' line of knobs.
const SPRAY_SHAPE_W: i32 = 124;
/// The Mod part's From and To pickers.
const ROUTE_PICK_W: i32 = 150;
/// The grain filter's passes (Off, Low pass, Band pass, High pass).
const PASSES_W: i32 = 330;

/// The Grain section as the panel's rows, controls and layout, for part
/// `part` of `r`. `note` is the section's note (what the cloud is doing);
/// `route` the Mod part's route outlined (from 0), the one its pickers
/// edit.
pub fn section(
    r: &Region,
    part: GrainPart,
    note: &str,
    route: usize,
) -> (Vec<Setting>, Vec<Control>, Panel) {
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
                // The Mod part's pickers: wide enough for "Velocity".
                (true, Row::RouteFrom | Row::RouteTo) => item.labelled().wide(ROUTE_PICK_W),
                (true, _) => item.labelled(),
                // Grains' nine knobs are narrow, and a route's name can be
                // long ("Press → filter spray"): their names on two lines
                // ("Size / spray"), so each reads whole.
                _ if part == GrainPart::Grains || part == GrainPart::Mod => item.two_lines(),
                _ => item,
            });
            rows.push(match row {
                // The Mod part's names and values say which route.
                Row::RouteFrom => Setting {
                    name: format!("Route {} from", route + 1),
                    value: grain_tab::route_text(r, row, route),
                },
                Row::RouteTo => Setting {
                    name: format!("Route {} to", route + 1),
                    value: grain_tab::route_text(r, row, route),
                },
                _ => match row.route() {
                    Some(k) => Setting {
                        name: grain_tab::route_name(r, k),
                        value: grain_tab::route_text(r, row, route),
                    },
                    None => Setting {
                        name: row.short().into(),
                        value: grain_tab::panel_text(r, row),
                    },
                },
            });
            controls.push(control(r, row).outlined(row.route() == Some(route)));
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
        color: Paint::from(Role::Grain),
    }
}

/// The window's two tabs, after the other marks: GRAIN where grains
/// start, SIZE where the first one ends; GRAIN's tab sits left of its
/// line and SIZE's right of it, so a short window keeps both clear.
pub fn markers(s: &GrainSpan) -> [Marker; 2] {
    let c = Paint::from(Role::Grain);
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
