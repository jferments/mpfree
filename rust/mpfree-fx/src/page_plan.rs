//! Page plans: how each built-in effect's page is laid out, as labelled
//! sections by job instead of a wall of same-size knobs (Jay, 14 Nov
//! 2026: "Build it as drawn", the effect page mockups, to the instrument
//! and effect page design rules).
//!
//! A plan is plain data, so the chain editor draws it with the shared
//! section renderer and the tests here hold it to the effect: every
//! settable control on the page once, every meter shown, every name real.
//!
//! - A **page** is a layout spec in `mpfree-ui`'s syntax over its sections'
//!   keys (`row(reverb:520, col(tone:252, row(movement, output)))`) in the
//!   844 by 540 pixels between the header and the Modulate row. Most
//!   effects have one page; one with stages it switches (the Channel
//!   strip) has a Main page and a page for each stage; Perform FX has its
//!   play controls and two pages of slots.
//! - A **section** names its controls in order, and its job, which gives
//!   its colour ([`Job`]). Its size tiers: `hero` (104 to 120 px), `main`
//!   (72 to 92), `trim` (44 to 56), anything else standard (56 to 64); a
//!   Q-Linked control is main unless it is listed as trim. `faders` are
//!   drawn as vertical faders, `rows` two to a line as compact rows. The
//!   widget follows the control's curve: five choices or fewer a segmented
//!   button, more a chip with arrows, a toggle a pill.
//! - Sections are a view: they never rename a control, so presets, project
//!   files and automation are untouched. `labels` only change what the
//!   page prints ("Gate thresh" reads "Threshold" in the Gate section).
//! - A control another one makes do nothing is dimmed, with the reason
//!   ([`Dim`]); a section with a `switch` dims its controls while it is
//!   off.
//!
//! Controls are named as their effect names them, or "Group/Name" where a
//! name repeats (the EQ's bands), as on the Main pages
//! ([`crate::main_page::find`]).

use mpfree_node::ParamInfo;

use crate::main_page::find;

/// What a section does, which gives its colour. The names are the shared
/// section renderer's, so a plan maps straight across; for an effect:
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Job {
    /// The effect's main process: the space, the echoes, the pitch (cyan).
    Source,
    /// Filters, tone and colour (coral): an EQ's bands, a drive.
    Tone,
    /// Dynamics (lime): a compressor's threshold and timing, an envelope.
    Shape,
    /// Movement (pink): LFOs, wow and flutter, a rotor.
    Motion,
    /// Output (grey): mix, width, level, quality.
    Voice,
}

/// A control's size on the page.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tier {
    Trim,
    Std,
    Main,
    Hero,
}

/// A labelled section of a page.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Section {
    /// Its slot in the page's layout spec.
    pub key: &'static str,
    /// Its heading ("Tone").
    pub name: &'static str,
    pub job: Job,
    /// Its controls, in the order they are laid out.
    pub params: &'static [&'static str],
    pub hero: &'static [&'static str],
    pub main: &'static [&'static str],
    pub trim: &'static [&'static str],
    /// Drawn as vertical faders.
    pub faders: &'static [&'static str],
    /// Drawn two to a line as compact rows (choices and switches).
    pub rows: &'static [&'static str],
    /// What the page prints for a control, where not its own name.
    pub labels: &'static [(&'static str, &'static str)],
    /// The effect's drawings shown in it, by their card titles.
    pub drawings: &'static [&'static str],
    /// Read-only controls it shows as live meters ("Reduction").
    pub meters: &'static [&'static str],
    /// The toggle in its heading that switches it (a stage).
    pub switch: Option<&'static str>,
    /// Another page of the effect, opened from its heading ("Open ›").
    pub link: Option<&'static str>,
    /// A short line beside its heading.
    pub note: Option<&'static str>,
}

impl Section {
    /// The size of control `name` here, `qlinked` if a Q-Link turns it.
    pub fn tier(&self, name: &str, qlinked: bool) -> Tier {
        if self.hero.contains(&name) {
            Tier::Hero
        } else if self.trim.contains(&name) {
            Tier::Trim
        } else if qlinked || self.main.contains(&name) {
            Tier::Main
        } else {
            Tier::Std
        }
    }

    /// What the page prints for control `name`: its label here, else its
    /// name without the group ("Band 2/Gain" prints "Gain").
    pub fn label<'a>(&self, name: &'a str) -> &'a str {
        match self.labels.iter().find(|(n, _)| *n == name) {
            Some((_, l)) => l,
            None => name.split_once('/').map_or(name, |(_, n)| n),
        }
    }
}

/// One page of an effect.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Page {
    /// Its tab ("Main", "Gate"); the tab row shows only when there are
    /// two pages or more.
    pub name: &'static str,
    /// The layout over its sections' keys.
    pub spec: &'static str,
    pub sections: &'static [Section],
}

/// The values of a control that make a rule hold.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum When {
    Is(&'static [f32]),
    Not(&'static [f32]),
}

impl When {
    /// Whether value `v` (of a choice or a toggle) makes it hold.
    pub fn holds(&self, v: f32) -> bool {
        let is = |vs: &[f32]| vs.iter().any(|x| (x - v).abs() < 0.5);
        match self {
            When::Is(vs) => is(vs),
            When::Not(vs) => !is(vs),
        }
    }
}

/// Controls dimmed while another control does not use them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Dim {
    pub controls: &'static [&'static str],
    /// The choice or toggle that decides.
    pub by: &'static str,
    pub when: When,
    /// Shown under the dimmed control ("Sync sets it").
    pub reason: &'static str,
    /// While it holds, a Q-Link on a dimmed control turns `by` instead: a
    /// synced delay's Time Q-Link picks the note value.
    pub swap: bool,
}

/// An effect's page plan.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Plan {
    /// The engine's node id.
    pub id: &'static str,
    /// What the Q-Links turn, in order (up to four): Main's four for an
    /// effect with a Main page.
    pub qlinks: &'static [&'static str],
    pub pages: &'static [Page],
    pub dims: &'static [Dim],
}

impl Plan {
    /// Why control `name` is dimmed now, if it is: the first rule naming it
    /// that holds, reading values with `value`.
    pub fn dimmed(
        &self,
        params: &[ParamInfo],
        name: &str,
        value: impl Fn(u16) -> f32,
    ) -> Option<&'static str> {
        self.dims
            .iter()
            .filter(|d| d.controls.contains(&name))
            .find(|d| find(params, d.by).is_some_and(|p| d.when.holds(value(p.id))))
            .map(|d| d.reason)
    }

    /// What a Q-Link on control `name` turns now: `by` of a rule that
    /// swaps and holds (Sync, while the delay's Time is synced), else
    /// `name`.
    pub fn qlink_turns<'a>(
        &self,
        params: &[ParamInfo],
        name: &'a str,
        value: impl Fn(u16) -> f32,
    ) -> &'a str {
        self.dims
            .iter()
            .filter(|d| d.swap && d.controls.contains(&name))
            .find(|d| find(params, d.by).is_some_and(|p| d.when.holds(value(p.id))))
            .map_or(name, |d| d.by)
    }

    /// Whether control `name` is one the Q-Links turn.
    pub fn qlinked(&self, name: &str) -> bool {
        self.qlinks.contains(&name)
    }
}

/// The plan for engine `id`, if it has one.
pub fn plan(id: &str) -> Option<&'static Plan> {
    PLANS.iter().find(|p| p.id == id)
}

/// The ids of controls `names` among `params`, in order (a name not found
/// is left out).
pub fn ids(names: &[&str], params: &[ParamInfo]) -> Vec<u16> {
    names
        .iter()
        .filter_map(|n| find(params, n).map(|p| p.id))
        .collect()
}

/// Drawings the plans place that the effect pages do not draw yet (none
/// now: the reverbs' tails, the tape's heads and the Channel strip's
/// stages came with the new pages). A plan may name a drawing before it
/// exists by listing it here.
pub static TO_DRAW: &[(&str, &str)] = &[];

const fn s(
    key: &'static str,
    name: &'static str,
    job: Job,
    params: &'static [&'static str],
) -> Section {
    Section {
        key,
        name,
        job,
        params,
        hero: &[],
        main: &[],
        trim: &[],
        faders: &[],
        rows: &[],
        labels: &[],
        drawings: &[],
        meters: &[],
        switch: None,
        link: None,
        note: None,
    }
}

const fn page(spec: &'static str, sections: &'static [Section]) -> Page {
    Page {
        name: "Main",
        spec,
        sections,
    }
}

const fn sync_dim(controls: &'static [&'static str], by: &'static str) -> Dim {
    Dim {
        controls,
        by,
        when: When::Not(&[0.0]),
        reason: "Sync sets it",
        swap: true,
    }
}

use Job::{Motion, Shape, Source, Tone, Voice};

/// A Perform FX slot: its effect, length, amount and hold, dimmed while
/// its effect is Off.
macro_rules! slot {
    ($n:literal) => {
        Section {
            rows: &[concat!("Slot ", $n, "/Hold")],
            meters: &[concat!("Slot ", $n, "/State")],
            ..s(
                concat!("s", $n),
                concat!("Slot ", $n),
                Motion,
                &[
                    concat!("Slot ", $n, "/Effect"),
                    concat!("Slot ", $n, "/Length"),
                    concat!("Slot ", $n, "/Amount"),
                    concat!("Slot ", $n, "/Hold"),
                ],
            )
        }
    };
}

macro_rules! slot_dim {
    ($n:literal) => {
        Dim {
            controls: &[
                concat!("Slot ", $n, "/Length"),
                concat!("Slot ", $n, "/Amount"),
                concat!("Slot ", $n, "/Hold"),
            ],
            by: concat!("Slot ", $n, "/Effect"),
            when: When::Is(&[0.0]),
            reason: "the slot is off",
            swap: false,
        }
    };
}

/// An EQ band: its type, gain (a Q-Link), frequency and Q, and its Use.
macro_rules! band {
    ($n:literal) => {
        Section {
            trim: &[concat!("Band ", $n, "/Freq"), concat!("Band ", $n, "/Q")],
            rows: &[concat!("Band ", $n, "/Use"), concat!("Band ", $n, "/Type")],
            ..s(
                concat!("b", $n),
                concat!("Band ", $n),
                Tone,
                &[
                    concat!("Band ", $n, "/Use"),
                    concat!("Band ", $n, "/Type"),
                    concat!("Band ", $n, "/Gain"),
                    concat!("Band ", $n, "/Freq"),
                    concat!("Band ", $n, "/Q"),
                ],
            )
        }
    };
}

/// An EQ band's controls dim while it isn't On.
macro_rules! band_dim {
    ($n:literal) => {
        Dim {
            controls: &[
                concat!("Band ", $n, "/Type"),
                concat!("Band ", $n, "/Gain"),
                concat!("Band ", $n, "/Freq"),
                concat!("Band ", $n, "/Q"),
            ],
            by: concat!("Band ", $n, "/Use"),
            when: When::Not(&[1.0]),
            reason: "the band is off",
            swap: false,
        }
    };
}

/// Every built-in effect's plan.
pub static PLANS: &[Plan] = &[
    // Dynamics.
    Plan {
        id: "mpfree.fx.compressor",
        qlinks: &["Threshold", "Ratio", "Makeup", "Mix"],
        pages: &[page(
            "row(comp:520, col(detector:252, output))",
            &[
                Section {
                    hero: &["Threshold", "Ratio"],
                    trim: &["Attack", "Release", "Knee"],
                    drawings: &["Curve", "Timing"],
                    meters: &["Reduction"],
                    ..s(
                        "comp",
                        "Compressor",
                        Shape,
                        &["Threshold", "Ratio", "Attack", "Release", "Knee"],
                    )
                },
                Section {
                    rows: &["Detect", "Sidechain"],
                    trim: &["SC filter"],
                    ..s(
                        "detector",
                        "Detector",
                        Shape,
                        &["Detect", "Sidechain", "SC filter"],
                    )
                },
                s("output", "Output", Voice, &["Makeup", "Mix"]),
            ],
        )],
        dims: &[],
    },
    Plan {
        id: "mpfree.fx.ducker",
        qlinks: &["Depth", "Release", "Shape", "Mix"],
        pages: &[page(
            "row(duck:520, col(trigger:*, output:200))",
            &[
                Section {
                    hero: &["Depth"],
                    drawings: &["Pump"],
                    meters: &["Gain"],
                    ..s(
                        "duck",
                        "Duck",
                        Shape,
                        &["Depth", "Attack", "Hold", "Release", "Shape"],
                    )
                },
                Section {
                    rows: &["Source", "Rate"],
                    ..s(
                        "trigger",
                        "Trigger",
                        Shape,
                        &["Source", "Rate", "Threshold"],
                    )
                },
                s("output", "Output", Voice, &["Mix"]),
            ],
        )],
        dims: &[
            Dim {
                controls: &["Threshold"],
                by: "Source",
                when: When::Is(&[1.0]),
                reason: "Tempo ducks on the beat",
                swap: false,
            },
            Dim {
                controls: &["Rate"],
                by: "Source",
                when: When::Is(&[0.0]),
                reason: "the sidechain sets it",
                swap: false,
            },
        ],
    },
    Plan {
        id: "mpfree.fx.transient",
        qlinks: &["Attack", "Sustain", "Speed", "Output"],
        pages: &[page(
            "row(shape:520, output)",
            &[
                Section {
                    hero: &["Attack", "Sustain"],
                    drawings: &["Hit"],
                    meters: &["Gain"],
                    ..s("shape", "Shape", Shape, &["Attack", "Sustain", "Speed"])
                },
                s("output", "Output", Voice, &["Output", "Mix"]),
            ],
        )],
        dims: &[],
    },
    Plan {
        id: "mpfree.fx.limiter",
        qlinks: &["Gain", "Ceiling", "Release"],
        pages: &[page(
            "row(limit:520, release)",
            &[
                Section {
                    hero: &["Gain"],
                    rows: &["True peak"],
                    meters: &["Reduction"],
                    ..s("limit", "Limiter", Shape, &["Gain", "Ceiling", "True peak"])
                },
                Section {
                    rows: &["Auto release"],
                    ..s("release", "Release", Shape, &["Release", "Auto release"])
                },
            ],
        )],
        dims: &[],
    },
    Plan {
        id: "mpfree.fx.gate",
        qlinks: &["Threshold", "Range", "Attack", "Release"],
        pages: &[page(
            "row(gate:520, key)",
            &[
                Section {
                    hero: &["Threshold"],
                    meters: &["Gain"],
                    ..s(
                        "gate",
                        "Gate",
                        Shape,
                        &["Threshold", "Range", "Attack", "Hold", "Release"],
                    )
                },
                Section {
                    rows: &["Key"],
                    note: Some("what opens it"),
                    ..s("key", "Key", Tone, &["Key", "Key HP", "Key LP"])
                },
            ],
        )],
        dims: &[],
    },
    Plan {
        id: "mpfree.fx.deesser",
        qlinks: &["Freq", "Threshold", "Range"],
        pages: &[page(
            "row(deess:520, col(band:300, detect))",
            &[
                Section {
                    hero: &["Threshold"],
                    meters: &["Reduction"],
                    ..s("deess", "De-esser", Shape, &["Threshold", "Range"])
                },
                Section {
                    rows: &["Mode", "Listen"],
                    note: Some("the band it listens to"),
                    ..s("band", "Band", Tone, &["Freq", "Mode", "Listen"])
                },
                Section {
                    rows: &["Detect"],
                    ..s("detect", "Detector", Shape, &["Detect"])
                },
            ],
        )],
        dims: &[],
    },
    Plan {
        id: "mpfree.fx.multiband",
        qlinks: &["Low thresh", "Mid thresh", "High thresh", "Mix"],
        pages: &[page(
            "col(row(split, timing:220):180, row(low, mid, high, output:150):*)",
            &[
                Section {
                    rows: &["Solo"],
                    trim: &["Low/mid", "Mid/high"],
                    labels: &[("Low/mid", "Low | mid"), ("Mid/high", "Mid | high")],
                    ..s("split", "Bands", Tone, &["Low/mid", "Mid/high", "Solo"])
                },
                Section {
                    labels: &[
                        ("Low thresh", "Threshold"),
                        ("Low ratio", "Ratio"),
                        ("Low gain", "Gain"),
                    ],
                    meters: &["Low red."],
                    ..s(
                        "low",
                        "Low",
                        Shape,
                        &["Low thresh", "Low ratio", "Low gain"],
                    )
                },
                Section {
                    labels: &[
                        ("Mid thresh", "Threshold"),
                        ("Mid ratio", "Ratio"),
                        ("Mid gain", "Gain"),
                    ],
                    meters: &["Mid red."],
                    ..s(
                        "mid",
                        "Mid",
                        Shape,
                        &["Mid thresh", "Mid ratio", "Mid gain"],
                    )
                },
                Section {
                    labels: &[
                        ("High thresh", "Threshold"),
                        ("High ratio", "Ratio"),
                        ("High gain", "Gain"),
                    ],
                    meters: &["High red."],
                    ..s(
                        "high",
                        "High",
                        Shape,
                        &["High thresh", "High ratio", "High gain"],
                    )
                },
                s("timing", "Timing", Shape, &["Attack", "Release"]),
                Section {
                    trim: &["Output"],
                    ..s("output", "Output", Voice, &["Mix", "Output"])
                },
            ],
        )],
        dims: &[],
    },
    Plan {
        id: "mpfree.fx.channel_strip",
        qlinks: &["Comp thresh", "Makeup", "Low cut", "High gain"],
        pages: &[
            page(
                "col(row(gate:220, comp:300, eq):*, row(path, output:220):198)",
                &[
                    Section {
                        switch: Some("Gate"),
                        link: Some("Gate"),
                        main: &["Gate thresh"],
                        labels: &[("Gate thresh", "Threshold")],
                        meters: &["Gate gain"],
                        ..s("gate", "Gate", Shape, &["Gate thresh"])
                    },
                    Section {
                        switch: Some("Comp"),
                        link: Some("Compressor"),
                        hero: &["Comp thresh"],
                        trim: &["Ratio", "Makeup"],
                        labels: &[("Comp thresh", "Threshold")],
                        meters: &["Reduction"],
                        ..s(
                            "comp",
                            "Compressor",
                            Shape,
                            &["Comp thresh", "Ratio", "Makeup"],
                        )
                    },
                    Section {
                        switch: Some("EQ"),
                        link: Some("EQ"),
                        drawings: &["Response"],
                        ..s("eq", "EQ", Tone, &["Low cut", "Mid gain", "High gain"])
                    },
                    Section {
                        drawings: &["Signal path"],
                        ..s("path", "Signal path", Voice, &["Order"])
                    },
                    Section {
                        main: &["Output"],
                        ..s("output", "Output", Voice, &["Output"])
                    },
                ],
            ),
            Page {
                name: "Gate",
                spec: "row(gate)",
                sections: &[Section {
                    switch: Some("Gate"),
                    hero: &["Gate thresh"],
                    labels: &[
                        ("Gate thresh", "Threshold"),
                        ("Gate range", "Range"),
                        ("Gate attack", "Attack"),
                        ("Gate release", "Release"),
                    ],
                    meters: &["Gate gain"],
                    ..s(
                        "gate",
                        "Gate",
                        Shape,
                        &["Gate thresh", "Gate range", "Gate attack", "Gate release"],
                    )
                }],
            },
            Page {
                name: "Compressor",
                spec: "row(comp)",
                sections: &[Section {
                    switch: Some("Comp"),
                    hero: &["Comp thresh", "Ratio"],
                    labels: &[
                        ("Comp thresh", "Threshold"),
                        ("Comp attack", "Attack"),
                        ("Comp release", "Release"),
                    ],
                    drawings: &["Curve"],
                    meters: &["Reduction"],
                    ..s(
                        "comp",
                        "Compressor",
                        Shape,
                        &[
                            "Comp thresh",
                            "Ratio",
                            "Comp attack",
                            "Comp release",
                            "Makeup",
                        ],
                    )
                }],
            },
            Page {
                name: "EQ",
                spec: "col(eq:220, row(low, mid, high):*)",
                sections: &[
                    Section {
                        switch: Some("EQ"),
                        drawings: &["Response"],
                        ..s("eq", "EQ", Tone, &[])
                    },
                    Section {
                        labels: &[("Low freq", "Freq"), ("Low gain", "Gain")],
                        ..s("low", "Low", Tone, &["Low cut", "Low freq", "Low gain"])
                    },
                    Section {
                        labels: &[("Mid freq", "Freq"), ("Mid gain", "Gain"), ("Mid Q", "Q")],
                        ..s("mid", "Mid", Tone, &["Mid freq", "Mid gain", "Mid Q"])
                    },
                    Section {
                        labels: &[("High freq", "Freq"), ("High gain", "Gain")],
                        ..s("high", "High", Tone, &["High freq", "High gain"])
                    },
                ],
            },
        ],
        dims: &[],
    },
    // EQ and filter.
    Plan {
        id: "mpfree.fx.eq",
        qlinks: &["Band 1/Gain", "Band 2/Gain", "Band 3/Gain", "Band 4/Gain"],
        pages: &[
            page(
                "col(row(curve:540, cuts):220, row(b1:172, b2:172, b3:172, b4:172, output):*)",
                &[
                    Section {
                        drawings: &["Response · ±18 dB"],
                        ..s("curve", "Curve", Tone, &[])
                    },
                    Section {
                        rows: &["Low cut", "Low slope", "High cut", "High slope"],
                        ..s(
                            "cuts",
                            "Cuts",
                            Tone,
                            &["Low cut", "Low slope", "High cut", "High slope"],
                        )
                    },
                    band!(1),
                    band!(2),
                    band!(3),
                    band!(4),
                    Section {
                        rows: &["Listen"],
                        ..s("output", "Output", Voice, &["Output", "Listen"])
                    },
                ],
            ),
            Page {
                name: "Bands 5-8",
                spec: "row(b5:200, b6:200, b7:200, b8:200)",
                sections: &[band!(5), band!(6), band!(7), band!(8)],
            },
        ],
        dims: &[
            band_dim!(1),
            band_dim!(2),
            band_dim!(3),
            band_dim!(4),
            band_dim!(5),
            band_dim!(6),
            band_dim!(7),
            band_dim!(8),
            Dim {
                controls: &["Low cut"],
                by: "Low slope",
                when: When::Is(&[0.0]),
                reason: "off: pick a slope",
                swap: false,
            },
            Dim {
                controls: &["High cut"],
                by: "High slope",
                when: When::Is(&[0.0]),
                reason: "off: pick a slope",
                swap: false,
            },
        ],
    },
    Plan {
        id: "mpfree.fx.filter",
        qlinks: &["Cutoff", "Resonance", "Gain", "Drive"],
        pages: &[page(
            "row(filter:520, col(type:252, drive))",
            &[
                Section {
                    hero: &["Cutoff", "Resonance"],
                    drawings: &["Response"],
                    ..s("filter", "Filter", Tone, &["Cutoff", "Resonance", "Gain"])
                },
                Section {
                    rows: &["Type", "Slope"],
                    ..s("type", "Type", Tone, &["Type", "Slope"])
                },
                s("drive", "Drive", Tone, &["Drive"]),
            ],
        )],
        dims: &[Dim {
            controls: &["Gain"],
            by: "Type",
            when: When::Not(&[4.0, 5.0, 6.0]),
            reason: "only a bell or a shelf uses it",
            swap: false,
        }],
    },
    Plan {
        id: "mpfree.fx.dj_filter",
        qlinks: &["Position", "Resonance"],
        pages: &[page(
            "row(filter)",
            &[Section {
                hero: &["Position"],
                drawings: &["Response"],
                note: Some("left low-pass, right high-pass"),
                ..s("filter", "DJ filter", Tone, &["Position", "Resonance"])
            }],
        )],
        dims: &[],
    },
    Plan {
        id: "mpfree.fx.comb",
        qlinks: &["Pitch", "Feedback", "Damping", "Mix"],
        pages: &[page(
            "row(comb:520, col(keys:252, output))",
            &[
                Section {
                    hero: &["Pitch", "Feedback"],
                    drawings: &["Teeth", "Ring"],
                    ..s(
                        "comb",
                        "Comb",
                        Tone,
                        &["Pitch", "Feedback", "Damping", "Spread"],
                    )
                },
                Section {
                    rows: &["Key follow"],
                    note: Some("play its pitch from notes"),
                    ..s("keys", "Keys", Voice, &["Key follow", "Glide"])
                },
                s("output", "Output", Voice, &["Mix"]),
            ],
        )],
        dims: &[],
    },
    Plan {
        id: "mpfree.fx.autowah",
        qlinks: &["Center", "Depth", "Resonance", "Mix"],
        pages: &[page(
            "row(wah:520, col(env:200, row(lfo, output):*))",
            &[
                Section {
                    hero: &["Center", "Depth"],
                    rows: &["Source", "Filter"],
                    trim: &["Low", "High"],
                    ..s(
                        "wah",
                        "Wah",
                        Tone,
                        &[
                            "Source",
                            "Filter",
                            "Center",
                            "Depth",
                            "Resonance",
                            "Low",
                            "High",
                        ],
                    )
                },
                s(
                    "env",
                    "Envelope",
                    Shape,
                    &["Sensitivity", "Attack", "Release"],
                ),
                Section {
                    rows: &["Sync"],
                    ..s("lfo", "LFO", Motion, &["Rate", "Sync"])
                },
                s("output", "Output", Voice, &["Mix"]),
            ],
        )],
        dims: &[
            Dim {
                controls: &["Sensitivity", "Attack", "Release"],
                by: "Source",
                when: When::Not(&[0.0]),
                reason: "Source isn't Envelope",
                swap: false,
            },
            Dim {
                controls: &["Rate", "Sync"],
                by: "Source",
                when: When::Not(&[1.0]),
                reason: "Source isn't LFO",
                swap: false,
            },
            sync_dim(&["Rate"], "Sync"),
        ],
    },
    // Drive and colour.
    Plan {
        id: "mpfree.fx.drive",
        qlinks: &["Drive", "Tone", "Bias", "Mix"],
        pages: &[page(
            "row(drive:520, col(output:344, quality))",
            &[
                Section {
                    hero: &["Drive", "Tone"],
                    rows: &["Mode"],
                    drawings: &["Curve", "A sine through it"],
                    ..s("drive", "Drive", Tone, &["Mode", "Drive", "Tone", "Bias"])
                },
                Section {
                    rows: &["Auto-gain"],
                    meters: &["Level change"],
                    ..s("output", "Output", Voice, &["Mix", "Output", "Auto-gain"])
                },
                Section {
                    rows: &["Quality"],
                    ..s("quality", "Quality", Voice, &["Quality"])
                },
            ],
        )],
        dims: &[],
    },
    Plan {
        id: "mpfree.fx.character",
        qlinks: &["Amount", "Output", "Mix"],
        pages: &[page(
            "row(character:520, output)",
            &[
                Section {
                    hero: &["Amount"],
                    rows: &["Mode"],
                    drawings: &["Curve", "A sine through it"],
                    ..s("character", "Character", Tone, &["Mode", "Amount"])
                },
                s("output", "Output", Voice, &["Mix", "Output"]),
            ],
        )],
        dims: &[],
    },
    Plan {
        id: "mpfree.fx.crush",
        qlinks: &["Bits", "Rate", "Mix"],
        pages: &[page(
            "row(crush:520, output)",
            &[
                Section {
                    hero: &["Bits", "Rate"],
                    rows: &["Dither"],
                    drawings: &["A sine through it"],
                    ..s("crush", "Crush", Tone, &["Bits", "Rate", "Dither"])
                },
                s("output", "Output", Voice, &["Mix"]),
            ],
        )],
        dims: &[],
    },
    Plan {
        id: "mpfree.fx.lofi",
        qlinks: &["Amount", "Saturation", "High cut", "Mix"],
        pages: &[page(
            "row(col(lofi:300, row(tape, vinyl)):520, col(tone:300, output))",
            &[
                Section {
                    hero: &["Amount"],
                    drawings: &["Wear"],
                    note: Some("one knob for all of it"),
                    ..s("lofi", "Lo-fi", Tone, &["Amount", "Saturation"])
                },
                s("tape", "Wow and flutter", Motion, &["Wow", "Flutter"]),
                s("vinyl", "Vinyl", Source, &["Crackle", "Noise"]),
                Section {
                    drawings: &["Tone"],
                    ..s("tone", "Tone", Tone, &["Low cut", "High cut"])
                },
                Section {
                    trim: &["Width"],
                    ..s("output", "Output", Voice, &["Mix", "Width"])
                },
            ],
        )],
        dims: &[],
    },
    Plan {
        id: "mpfree.fx.amp",
        qlinks: &["Gain", "Bass", "Mid", "Treble"],
        pages: &[page(
            "row(col(amp:260, stack):520, output)",
            &[
                Section {
                    hero: &["Gain"],
                    rows: &["Model"],
                    ..s("amp", "Amp", Tone, &["Model", "Gain"])
                },
                s(
                    "stack",
                    "Tone stack",
                    Tone,
                    &["Bass", "Mid", "Treble", "Presence"],
                ),
                Section {
                    rows: &["Cab"],
                    labels: &[("Cab", "Cabinet")],
                    ..s("output", "Output", Voice, &["Cab", "Level"])
                },
            ],
        )],
        dims: &[],
    },
    Plan {
        id: "mpfree.fx.exciter",
        qlinks: &["Freq", "Drive", "Colour", "Amount"],
        pages: &[page(
            "row(exciter:520, output)",
            &[
                Section {
                    hero: &["Drive"],
                    note: Some("adds sparkle above the frequency"),
                    ..s("exciter", "Exciter", Tone, &["Freq", "Drive", "Colour"])
                },
                s("output", "Output", Voice, &["Amount"]),
            ],
        )],
        dims: &[],
    },
    // Modulation.
    Plan {
        id: "mpfree.fx.chorus",
        qlinks: &["Rate", "Depth", "Delay", "Mix"],
        pages: &[page(
            "row(lfo:520, col(voices:252, row(tone, output)))",
            &[
                Section {
                    hero: &["Rate", "Depth"],
                    rows: &["Sync"],
                    drawings: &["Sweep"],
                    ..s("lfo", "Movement", Motion, &["Rate", "Depth", "Sync"])
                },
                s("voices", "Voices", Source, &["Delay", "Voices", "Width"]),
                s("tone", "Tone", Tone, &["Tone"]),
                s("output", "Output", Voice, &["Mix"]),
            ],
        )],
        dims: &[sync_dim(&["Rate"], "Sync")],
    },
    Plan {
        id: "mpfree.fx.flanger",
        qlinks: &["Rate", "Depth", "Feedback", "Mix"],
        pages: &[page(
            "row(lfo:520, col(flanger:300, output))",
            &[
                Section {
                    hero: &["Rate", "Depth"],
                    rows: &["Sync"],
                    drawings: &["Sweep"],
                    ..s("lfo", "Movement", Motion, &["Rate", "Depth", "Sync"])
                },
                Section {
                    drawings: &["Comb"],
                    ..s(
                        "flanger",
                        "Flanger",
                        Source,
                        &["Delay", "Feedback", "Width"],
                    )
                },
                s("output", "Output", Voice, &["Mix"]),
            ],
        )],
        dims: &[sync_dim(&["Rate"], "Sync")],
    },
    Plan {
        id: "mpfree.fx.phaser",
        qlinks: &["Rate", "Depth", "Center", "Feedback"],
        pages: &[page(
            "row(lfo:520, col(phaser:340, output))",
            &[
                Section {
                    hero: &["Rate", "Depth"],
                    rows: &["Sync"],
                    drawings: &["Sweep"],
                    ..s("lfo", "Movement", Motion, &["Rate", "Depth", "Sync"])
                },
                Section {
                    rows: &["Stages"],
                    drawings: &["Notches"],
                    ..s(
                        "phaser",
                        "Phaser",
                        Source,
                        &["Stages", "Center", "Feedback", "Width"],
                    )
                },
                s("output", "Output", Voice, &["Mix"]),
            ],
        )],
        dims: &[sync_dim(&["Rate"], "Sync")],
    },
    Plan {
        id: "mpfree.fx.tremolo",
        qlinks: &["Rate", "Depth", "Shape", "Width"],
        pages: &[page(
            "row(tremolo:520, col(shape:252, output))",
            &[
                Section {
                    hero: &["Rate", "Depth"],
                    rows: &["Sync", "Mode"],
                    ..s(
                        "tremolo",
                        "Tremolo",
                        Motion,
                        &["Rate", "Depth", "Sync", "Mode"],
                    )
                },
                s("shape", "Shape", Motion, &["Shape"]),
                Section {
                    note: Some("left and right apart"),
                    ..s("output", "Stereo", Voice, &["Width"])
                },
            ],
        )],
        dims: &[sync_dim(&["Rate"], "Sync")],
    },
    Plan {
        id: "mpfree.fx.autopan",
        qlinks: &["Rate", "Depth", "Shape", "Centre"],
        pages: &[page(
            "row(pan:520, col(shape:252, output))",
            &[
                Section {
                    hero: &["Rate", "Depth"],
                    rows: &["Sync"],
                    ..s("pan", "Pan", Motion, &["Rate", "Depth", "Sync"])
                },
                s("shape", "Shape", Motion, &["Shape"]),
                s("output", "Output", Voice, &["Centre"]),
            ],
        )],
        dims: &[sync_dim(&["Rate"], "Sync")],
    },
    Plan {
        id: "mpfree.fx.ringmod",
        qlinks: &["Freq", "Sweep", "LFO rate", "Mix"],
        pages: &[page(
            "row(carrier:520, col(sweep:300, output))",
            &[
                Section {
                    hero: &["Freq"],
                    rows: &["Wave"],
                    ..s("carrier", "Carrier", Source, &["Freq", "Wave"])
                },
                Section {
                    rows: &["LFO sync"],
                    labels: &[("LFO rate", "Rate"), ("LFO sync", "Sync")],
                    ..s("sweep", "Sweep", Motion, &["Sweep", "LFO rate", "LFO sync"])
                },
                s("output", "Output", Voice, &["Mix"]),
            ],
        )],
        dims: &[sync_dim(&["LFO rate"], "LFO sync")],
    },
    Plan {
        id: "mpfree.fx.rotary",
        qlinks: &["Speed", "Ramp", "Drive", "Mix"],
        pages: &[page(
            "row(rotor:520, col(drive:252, output))",
            &[
                Section {
                    hero: &["Ramp"],
                    rows: &["Speed"],
                    ..s("rotor", "Rotor", Motion, &["Speed", "Ramp", "Balance"])
                },
                s("drive", "Drive", Tone, &["Drive"]),
                Section {
                    trim: &["Width"],
                    ..s("output", "Output", Voice, &["Mix", "Width"])
                },
            ],
        )],
        dims: &[],
    },
    Plan {
        id: "mpfree.fx.freqshift",
        qlinks: &["Shift", "Feedback", "Mix"],
        pages: &[page(
            "row(shift:520, output)",
            &[
                Section {
                    hero: &["Shift"],
                    rows: &["Mode"],
                    ..s("shift", "Shift", Source, &["Shift", "Mode", "Feedback"])
                },
                s("output", "Output", Voice, &["Mix"]),
            ],
        )],
        dims: &[],
    },
    // Delay and reverb.
    Plan {
        id: "mpfree.fx.delay",
        qlinks: &["Time", "Feedback", "Tone", "Mix"],
        pages: &[page(
            "row(echo:520, col(repeats:252, output))",
            &[
                Section {
                    hero: &["Time", "Feedback"],
                    rows: &["Sync", "Ping-pong", "Freeze"],
                    drawings: &["Repeats"],
                    ..s(
                        "echo",
                        "Echo",
                        Source,
                        &["Time", "Sync", "Feedback", "Ping-pong", "Freeze"],
                    )
                },
                s("repeats", "Repeats", Tone, &["Tone", "Saturation"]),
                s("output", "Output", Voice, &["Mix"]),
            ],
        )],
        dims: &[sync_dim(&["Time"], "Sync")],
    },
    Plan {
        id: "mpfree.fx.tape_delay",
        qlinks: &["Time", "Feedback", "Tone", "Mix"],
        pages: &[page(
            "row(echo:520, col(tape:290, row(wow, output)))",
            &[
                Section {
                    hero: &["Time", "Feedback"],
                    rows: &["Sync", "Heads"],
                    trim: &["Spread"],
                    drawings: &["Repeats"],
                    ..s(
                        "echo",
                        "Echo",
                        Source,
                        &["Time", "Sync", "Feedback", "Heads", "Spread"],
                    )
                },
                Section {
                    drawings: &["Tone"],
                    ..s("tape", "Tape", Tone, &["Saturation", "Tone", "Low cut"])
                },
                s("wow", "Wow and flutter", Motion, &["Wow", "Flutter"]),
                s("output", "Output", Voice, &["Mix"]),
            ],
        )],
        dims: &[sync_dim(&["Time"], "Sync")],
    },
    Plan {
        id: "mpfree.fx.multitap",
        qlinks: &["Feedback", "Tone", "Low cut", "Mix"],
        pages: &[page(
            "col(row(repeats, tone:240, output:140):230, row(t1, t2, t3, t4):*)",
            &[
                Section {
                    drawings: &["Repeats"],
                    ..s("repeats", "Repeats", Source, &["Feedback"])
                },
                s("tone", "Tone", Tone, &["Tone", "Low cut"]),
                s("output", "Output", Voice, &["Mix"]),
                Section {
                    labels: &[
                        ("Tap 1 time", "Time"),
                        ("Tap 1 level", "Level"),
                        ("Tap 1 pan", "Pan"),
                    ],
                    ..s(
                        "t1",
                        "Tap 1",
                        Source,
                        &["Tap 1 time", "Tap 1 level", "Tap 1 pan"],
                    )
                },
                Section {
                    labels: &[
                        ("Tap 2 time", "Time"),
                        ("Tap 2 level", "Level"),
                        ("Tap 2 pan", "Pan"),
                    ],
                    ..s(
                        "t2",
                        "Tap 2",
                        Source,
                        &["Tap 2 time", "Tap 2 level", "Tap 2 pan"],
                    )
                },
                Section {
                    labels: &[
                        ("Tap 3 time", "Time"),
                        ("Tap 3 level", "Level"),
                        ("Tap 3 pan", "Pan"),
                    ],
                    ..s(
                        "t3",
                        "Tap 3",
                        Source,
                        &["Tap 3 time", "Tap 3 level", "Tap 3 pan"],
                    )
                },
                Section {
                    labels: &[
                        ("Tap 4 time", "Time"),
                        ("Tap 4 level", "Level"),
                        ("Tap 4 pan", "Pan"),
                    ],
                    ..s(
                        "t4",
                        "Tap 4",
                        Source,
                        &["Tap 4 time", "Tap 4 level", "Tap 4 pan"],
                    )
                },
            ],
        )],
        dims: &[],
    },
    Plan {
        id: "mpfree.fx.cloud",
        qlinks: &["Decay", "Size", "Pre-delay", "Mix"],
        pages: &[page(
            "row(reverb:520, col(tone:236, row(movement:110, output)))",
            &[
                Section {
                    hero: &["Decay", "Size"],
                    trim: &["Diffusion", "Early"],
                    rows: &["Character", "Freeze"],
                    drawings: &["Decay · 60 dB"],
                    ..s(
                        "reverb",
                        "Reverb",
                        Source,
                        &[
                            "Character",
                            "Decay",
                            "Size",
                            "Pre-delay",
                            "Diffusion",
                            "Early",
                            "Freeze",
                        ],
                    )
                },
                Section {
                    drawings: &["Tone"],
                    ..s("tone", "Tone", Tone, &["Damping", "Low cut", "High cut"])
                },
                Section {
                    labels: &[("Modulation", "Depth"), ("Mod rate", "Rate")],
                    ..s("movement", "Movement", Motion, &["Modulation", "Mod rate"])
                },
                Section {
                    trim: &["Width"],
                    rows: &["Quality"],
                    ..s("output", "Output", Voice, &["Mix", "Width", "Quality"])
                },
            ],
        )],
        dims: &[],
    },
    Plan {
        id: "mpfree.fx.room",
        qlinks: &["Decay", "Size", "Pre-delay", "Mix"],
        pages: &[page(
            "row(reverb:520, col(tone:252, row(movement, output)))",
            &[
                Section {
                    hero: &["Decay", "Size"],
                    trim: &["Early", "Gate"],
                    drawings: &["Decay · 60 dB"],
                    ..s(
                        "reverb",
                        "Reverb",
                        Source,
                        &["Decay", "Size", "Pre-delay", "Early", "Gate"],
                    )
                },
                Section {
                    drawings: &["Tone"],
                    ..s("tone", "Tone", Tone, &["Damping", "Low cut"])
                },
                s("movement", "Movement", Motion, &["Mod"]),
                Section {
                    trim: &["Width"],
                    ..s("output", "Output", Voice, &["Mix", "Width"])
                },
            ],
        )],
        dims: &[],
    },
    Plan {
        id: "mpfree.fx.plate",
        qlinks: &["Decay", "Size", "Pre-delay", "Mix"],
        pages: &[page(
            "row(reverb:520, col(tone:252, row(movement, output)))",
            &[
                Section {
                    hero: &["Decay", "Size"],
                    drawings: &["Decay · 60 dB"],
                    ..s("reverb", "Reverb", Source, &["Decay", "Size", "Pre-delay"])
                },
                Section {
                    drawings: &["Tone"],
                    note: Some("darker as it fades"),
                    ..s("tone", "Tone", Tone, &["Damping", "Tone", "Low cut"])
                },
                Section {
                    note: Some("so the tail never rings"),
                    ..s("movement", "Movement", Motion, &["Mod"])
                },
                Section {
                    trim: &["Width"],
                    ..s("output", "Output", Voice, &["Mix", "Width"])
                },
            ],
        )],
        dims: &[],
    },
    Plan {
        id: "mpfree.fx.spring",
        qlinks: &["Decay", "Length", "Drip", "Mix"],
        pages: &[page(
            "row(reverb:520, col(tone:252, output))",
            &[
                Section {
                    hero: &["Decay", "Length"],
                    drawings: &["Decay · 60 dB"],
                    ..s(
                        "reverb",
                        "Spring",
                        Source,
                        &["Decay", "Length", "Tension", "Drip"],
                    )
                },
                s("tone", "Tone", Tone, &["Tone", "Low cut"]),
                s("output", "Output", Voice, &["Mix"]),
            ],
        )],
        dims: &[],
    },
    Plan {
        id: "mpfree.fx.granulator",
        qlinks: &["Size", "Density", "Pitch", "Mix"],
        pages: &[page(
            "col(row(grains:520, pitch):*, row(time, output:200):270)",
            &[
                Section {
                    hero: &["Size", "Density"],
                    ..s("grains", "Grains", Source, &["Size", "Density", "Reverse"])
                },
                s("pitch", "Pitch", Source, &["Pitch", "Detune"]),
                Section {
                    rows: &["Sync", "Freeze"],
                    ..s(
                        "time",
                        "Time",
                        Motion,
                        &["Delay", "Sync", "Jitter", "Feedback", "Freeze"],
                    )
                },
                Section {
                    trim: &["Width"],
                    ..s("output", "Output", Voice, &["Mix", "Width"])
                },
            ],
        )],
        dims: &[sync_dim(&["Delay"], "Sync")],
    },
    // Pitch and vocal.
    Plan {
        id: "mpfree.fx.pitch",
        qlinks: &["Pitch", "Fine", "Feedback", "Mix"],
        pages: &[page(
            "row(pitch:520, col(sound:252, output))",
            &[
                Section {
                    hero: &["Pitch"],
                    drawings: &["Shift"],
                    ..s("pitch", "Pitch", Source, &["Pitch", "Fine", "Spread"])
                },
                s("sound", "Sound", Tone, &["Window", "Feedback"]),
                s("output", "Output", Voice, &["Mix"]),
            ],
        )],
        dims: &[],
    },
    Plan {
        id: "mpfree.fx.pitch_correct",
        qlinks: &["Key", "Scale", "Speed", "Amount"],
        pages: &[page(
            "row(notes:520, voice)",
            &[
                Section {
                    hero: &["Speed", "Amount"],
                    rows: &["Key", "Scale"],
                    drawings: &["Tune"],
                    meters: &["Note", "Correction"],
                    ..s(
                        "notes",
                        "Notes",
                        Source,
                        &["Key", "Scale", "Speed", "Amount"],
                    )
                },
                Section {
                    rows: &["Range"],
                    ..s("voice", "Voice", Voice, &["Range", "Mix"])
                },
            ],
        )],
        dims: &[],
    },
    Plan {
        id: "mpfree.fx.harmonizer",
        qlinks: &["Level 1", "Level 2", "Glide", "Dry"],
        pages: &[page(
            "row(notes:260, v1, v2, output:148)",
            &[
                Section {
                    rows: &["Key", "Scale", "Range"],
                    drawings: &["Voices"],
                    meters: &["Note"],
                    ..s(
                        "notes",
                        "Notes",
                        Source,
                        &["Key", "Scale", "Range", "Glide"],
                    )
                },
                Section {
                    faders: &["Level 1"],
                    labels: &[
                        ("Voice 1", "Interval"),
                        ("Level 1", "Level"),
                        ("Pan 1", "Pan"),
                    ],
                    ..s("v1", "Voice 1", Source, &["Voice 1", "Level 1", "Pan 1"])
                },
                Section {
                    faders: &["Level 2"],
                    labels: &[
                        ("Voice 2", "Interval"),
                        ("Level 2", "Level"),
                        ("Pan 2", "Pan"),
                    ],
                    ..s("v2", "Voice 2", Source, &["Voice 2", "Level 2", "Pan 2"])
                },
                s("output", "Output", Voice, &["Dry"]),
            ],
        )],
        dims: &[
            Dim {
                controls: &["Level 1", "Pan 1"],
                by: "Voice 1",
                when: When::Is(&[0.0]),
                reason: "Voice 1 is off",
                swap: false,
            },
            Dim {
                controls: &["Level 2", "Pan 2"],
                by: "Voice 2",
                when: When::Is(&[0.0]),
                reason: "Voice 2 is off",
                swap: false,
            },
        ],
    },
    Plan {
        id: "mpfree.fx.talkbox",
        qlinks: &["Vowel", "Formant", "Env depth", "Mix"],
        pages: &[page(
            "col(row(voice:520, output):270, row(env, lfo):*)",
            &[
                s("voice", "Voice", Source, &["Vowel", "Formant", "Resonance"]),
                s("output", "Output", Voice, &["Mix"]),
                Section {
                    labels: &[("Env depth", "Depth")],
                    ..s(
                        "env",
                        "Envelope",
                        Shape,
                        &["Env depth", "Sensitivity", "Attack", "Release"],
                    )
                },
                Section {
                    rows: &["LFO sync", "LFO shape"],
                    labels: &[
                        ("LFO rate", "Rate"),
                        ("LFO depth", "Depth"),
                        ("LFO sync", "Sync"),
                        ("LFO shape", "Shape"),
                    ],
                    ..s(
                        "lfo",
                        "LFO",
                        Motion,
                        &["LFO rate", "LFO depth", "LFO sync", "LFO shape"],
                    )
                },
            ],
        )],
        dims: &[sync_dim(&["LFO rate"], "LFO sync")],
    },
    // Performance.
    Plan {
        id: "mpfree.fx.perform",
        qlinks: &["DJ filter", "Echo", "Echo time", "Mix"],
        pages: &[
            page(
                "row(play:520, col(punch:252, output))",
                &[
                    Section {
                        hero: &["DJ filter"],
                        ..s("play", "Play", Source, &["DJ filter", "Echo", "Echo time"])
                    },
                    Section {
                        rows: &["Punch in on"],
                        ..s("punch", "Punch in", Voice, &["Punch in on", "Fade"])
                    },
                    s("output", "Output", Voice, &["Mix"]),
                ],
            ),
            Page {
                name: "Slots 1-4",
                spec: "row(s1, s2, s3, s4)",
                sections: &[slot!(1), slot!(2), slot!(3), slot!(4)],
            },
            Page {
                name: "Slots 5-8",
                spec: "row(s5, s6, s7, s8)",
                sections: &[slot!(5), slot!(6), slot!(7), slot!(8)],
            },
            Page {
                name: "Slots 9-12",
                spec: "row(s9, s10, s11, s12)",
                sections: &[slot!(9), slot!(10), slot!(11), slot!(12)],
            },
            Page {
                name: "Slots 13-16",
                spec: "row(s13, s14, s15, s16)",
                sections: &[slot!(13), slot!(14), slot!(15), slot!(16)],
            },
        ],
        dims: &[
            slot_dim!(1),
            slot_dim!(2),
            slot_dim!(3),
            slot_dim!(4),
            slot_dim!(5),
            slot_dim!(6),
            slot_dim!(7),
            slot_dim!(8),
            slot_dim!(9),
            slot_dim!(10),
            slot_dim!(11),
            slot_dim!(12),
            slot_dim!(13),
            slot_dim!(14),
            slot_dim!(15),
            slot_dim!(16),
        ],
    },
    Plan {
        id: "mpfree.fx.xy",
        qlinks: &["X", "Y", "Effect", "Mix"],
        pages: &[page(
            "row(pad:520, col(touch:252, output))",
            &[
                Section {
                    hero: &["X", "Y"],
                    rows: &["Effect", "Active"],
                    ..s("pad", "Pad", Source, &["Effect", "X", "Y", "Active"])
                },
                s("touch", "Touch", Shape, &["Attack", "Release"]),
                s("output", "Output", Voice, &["Mix"]),
            ],
        )],
        dims: &[],
    },
    // Utility.
    Plan {
        id: "mpfree.fx.tuner",
        qlinks: &["Reference", "Mute"],
        pages: &[page(
            "row(tuner:560, settings)",
            &[
                Section {
                    drawings: &["Tuner"],
                    meters: &["Note", "Cents", "Frequency"],
                    ..s("tuner", "Tuner", Source, &[])
                },
                Section {
                    rows: &["Mute"],
                    note: Some("Mute silences the strip while you tune"),
                    ..s("settings", "Settings", Voice, &["Reference", "Mute"])
                },
            ],
        )],
        dims: &[],
    },
    Plan {
        id: "mpfree.fx.width",
        qlinks: &["Width", "Low width", "High width", "Delay"],
        pages: &[page(
            "row(width:300, bands, output:150)",
            &[
                Section {
                    hero: &["Width"],
                    ..s("width", "Width", Source, &["Width", "Delay"])
                },
                Section {
                    faders: &["Low width", "Mid width", "High width"],
                    trim: &["Low split", "High split"],
                    labels: &[
                        ("Low width", "Low"),
                        ("Mid width", "Mid"),
                        ("High width", "High"),
                    ],
                    ..s(
                        "bands",
                        "Bands",
                        Source,
                        &[
                            "Low width",
                            "Mid width",
                            "High width",
                            "Low split",
                            "High split",
                        ],
                    )
                },
                s("output", "Output", Voice, &["Pan", "Output"]),
            ],
        )],
        dims: &[],
    },
    Plan {
        id: "mpfree.fx.sample_delay",
        qlinks: &["Left", "Right"],
        pages: &[page(
            "row(samples:520, polarity)",
            &[
                Section {
                    note: Some("48 samples = 1 ms at 48 kHz"),
                    ..s("samples", "Delay", Source, &["Left", "Right"])
                },
                Section {
                    rows: &["Flip left", "Flip right"],
                    ..s("polarity", "Polarity", Voice, &["Flip left", "Flip right"])
                },
            ],
        )],
        dims: &[],
    },
    Plan {
        id: "mpfree.fx.master_safety",
        qlinks: &["Trim", "Ceiling", "Release"],
        pages: &[page(
            "row(safety:520, trim)",
            &[
                Section {
                    rows: &["Mode"],
                    drawings: &["Curve", "Catches"],
                    meters: &["Limiting", "Overs", "Clipped"],
                    ..s(
                        "safety",
                        "Master safety",
                        Shape,
                        &["Mode", "Ceiling", "Release"],
                    )
                },
                s("trim", "Trim", Voice, &["Trim"]),
            ],
        )],
        dims: &[
            Dim {
                controls: &["Ceiling"],
                by: "Mode",
                when: When::Is(&[2.0]),
                reason: "Raw doesn't limit",
                swap: false,
            },
            Dim {
                controls: &["Release"],
                by: "Mode",
                when: When::Not(&[0.0]),
                reason: "only the Safe limiter uses it",
                swap: false,
            },
        ],
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::main_page::main_page;
    use crate::registry::{self, EFFECTS};
    use mpfree_node::Curve;

    /// Every name a section uses, with what it is used as.
    fn names(sec: &Section) -> impl Iterator<Item = &'static str> + '_ {
        sec.params
            .iter()
            .chain(sec.hero)
            .chain(sec.main)
            .chain(sec.trim)
            .chain(sec.faders)
            .chain(sec.rows)
            .chain(sec.labels.iter().map(|(n, _)| n))
            .chain(sec.meters)
            .chain(sec.switch.iter())
            .copied()
    }

    /// A page's settable controls: its sections' controls and switches.
    fn on_page(p: &Page) -> Vec<&'static str> {
        p.sections
            .iter()
            .flat_map(|s| s.params.iter().copied().chain(s.switch))
            .collect()
    }

    #[test]
    fn every_built_in_effect_has_a_plan_and_only_those() {
        for e in EFFECTS {
            assert!(plan(e.id).is_some(), "{} has no page plan", e.name);
        }
        for (i, p) in PLANS.iter().enumerate() {
            assert!(registry::entry(p.id).is_some(), "{}", p.id);
            assert!(!PLANS[..i].iter().any(|q| q.id == p.id), "{} twice", p.id);
        }
    }

    /// Every name in a plan is a control of its effect: settable ones as
    /// controls, read-only ones only as meters, toggles as switches.
    #[test]
    fn plans_name_real_controls() {
        for p in PLANS {
            let n = registry::make(p.id).unwrap();
            let params = n.params();
            let get = |name: &str| {
                find(params, name).unwrap_or_else(|| panic!("{}: no control {name}", p.id))
            };
            for page in p.pages {
                for sec in page.sections {
                    for name in names(sec) {
                        get(name);
                    }
                    for name in sec.params.iter().chain(sec.switch.iter()) {
                        assert!(!get(name).readonly, "{} {name} is a meter", p.id);
                    }
                    for name in sec.meters {
                        assert!(get(name).readonly, "{} {name} isn't a meter", p.id);
                    }
                    if let Some(sw) = sec.switch {
                        assert_eq!(get(sw).curve, Curve::Toggle, "{} {sw}", p.id);
                        assert!(!sec.params.contains(&sw), "{} {sw}", p.id);
                    }
                    let lists = [sec.hero, sec.main, sec.trim, sec.faders, sec.rows];
                    for name in lists.iter().flat_map(|l| l.iter()) {
                        assert!(
                            sec.params.contains(name),
                            "{} {name} not in {}",
                            p.id,
                            sec.key
                        );
                    }
                    for (name, _) in sec.labels {
                        assert!(
                            sec.params.contains(name),
                            "{} {name} not in {}",
                            p.id,
                            sec.key
                        );
                    }
                    let tiers = [sec.hero, sec.main, sec.trim];
                    for name in sec.params {
                        let n = tiers.iter().filter(|l| l.contains(name)).count();
                        assert!(n <= 1, "{} {name} in two tiers", p.id);
                    }
                }
            }
            for q in p.qlinks {
                assert!(!get(q).readonly, "{} Q-Link {q}", p.id);
            }
            for d in p.dims {
                let by = get(d.by);
                assert!(
                    matches!(by.curve, Curve::Choice(_) | Curve::Toggle),
                    "{} {} decides by a choice or a toggle",
                    p.id,
                    d.by
                );
                for c in d.controls {
                    assert!(!get(c).readonly && *c != d.by, "{} {c}", p.id);
                }
            }
        }
    }

    /// The page shows every control: a one-page effect each settable
    /// control once and each meter once; an effect with more pages each at
    /// most once a page and every one somewhere.
    #[test]
    fn every_control_is_on_the_page_once() {
        for p in PLANS {
            let n = registry::make(p.id).unwrap();
            let params = n.params();
            let count = |name: &str, page: &Page, meters: bool| {
                page.sections
                    .iter()
                    .flat_map(|s| {
                        let list: Vec<&str> = if meters {
                            s.meters.to_vec()
                        } else {
                            s.params.iter().copied().chain(s.switch).collect()
                        };
                        list
                    })
                    .filter(|n| find(params, n).map(|q| q.id) == find(params, name).map(|q| q.id))
                    .count()
            };
            for q in params {
                let name = if params.iter().filter(|o| o.name == q.name).count() > 1 {
                    format!("{}/{}", q.group, q.name)
                } else {
                    q.name.to_string()
                };
                let per_page: Vec<usize> = p
                    .pages
                    .iter()
                    .map(|pg| count(&name, pg, q.readonly))
                    .collect();
                if p.pages.len() == 1 {
                    assert_eq!(per_page[0], 1, "{} {name}", p.id);
                } else {
                    assert!(per_page.iter().all(|&c| c <= 1), "{} {name}", p.id);
                    assert!(per_page.contains(&1), "{} {name}", p.id);
                }
            }
        }
    }

    #[test]
    fn pages_and_sections_are_well_formed() {
        for p in PLANS {
            assert!(!p.pages.is_empty(), "{}", p.id);
            assert!(p.qlinks.len() <= 4, "{}", p.id);
            for (i, q) in p.qlinks.iter().enumerate() {
                assert!(!p.qlinks[..i].contains(q), "{} {q}", p.id);
                let shown = p.pages[0].sections.iter().any(|s| s.params.contains(q));
                assert!(shown, "{}: Q-Link {q} isn't on the first page", p.id);
            }
            for (i, page) in p.pages.iter().enumerate() {
                assert!(
                    !p.pages[..i].iter().any(|o| o.name == page.name),
                    "{} {}",
                    p.id,
                    page.name
                );
                let on = on_page(page);
                for (j, name) in on.iter().enumerate() {
                    assert!(
                        !on[..j].contains(name),
                        "{} {name} twice on {}",
                        p.id,
                        page.name
                    );
                }
                for (j, sec) in page.sections.iter().enumerate() {
                    assert!(
                        !page.sections[..j].iter().any(|o| o.key == sec.key),
                        "{} {}",
                        p.id,
                        sec.key
                    );
                    assert!(page.spec.contains(sec.key), "{} {}", p.id, sec.key);
                    assert!(
                        !sec.params.is_empty() || !sec.drawings.is_empty(),
                        "{} {} is empty",
                        p.id,
                        sec.key
                    );
                    if let Some(l) = sec.link {
                        assert!(
                            p.pages.iter().any(|o| o.name == l && o.name != page.name),
                            "{} links to {l}",
                            p.id
                        );
                    }
                }
            }
        }
    }

    /// An effect with a Main page keeps Main's four on the Q-Links.
    #[test]
    fn qlinks_agree_with_the_main_pages() {
        for p in PLANS {
            if let Some(m) = main_page(p.id) {
                assert_eq!(p.qlinks, &m.qlinks[..], "{}", p.id);
            }
        }
    }

    /// A family mode's fixed controls (Drive's Mode) sit in compact rows,
    /// since the Mode chooser stands for them and they show only on an
    /// insert in no mode.
    #[test]
    fn fixed_controls_sit_in_rows() {
        for f in crate::families::FAMILIES {
            for m in f.modes {
                let p = plan(m.id).unwrap();
                let params = registry::make(m.id).unwrap().params().to_vec();
                for &(id, _) in m.fixed {
                    let name = params.iter().find(|q| q.id == id).unwrap().name;
                    let in_rows = p.pages[0].sections.iter().any(|s| s.rows.contains(&name));
                    assert!(in_rows, "{} {name}", m.id);
                    assert!(!p.qlinked(name), "{} {name}", m.id);
                }
            }
        }
    }

    #[test]
    fn a_dim_rule_reads_its_control() {
        let p = plan("mpfree.fx.delay").unwrap();
        let n = registry::make(p.id).unwrap();
        let sync = find(n.params(), "Sync").unwrap().id;
        let free = |id: u16| if id == sync { 0.0 } else { 1.0 };
        let synced = |id: u16| if id == sync { 4.0 } else { 1.0 };
        assert_eq!(p.dimmed(n.params(), "Time", free), None);
        assert_eq!(p.dimmed(n.params(), "Time", synced), Some("Sync sets it"));
        assert_eq!(p.dimmed(n.params(), "Feedback", synced), None);
        assert_eq!(p.qlink_turns(n.params(), "Time", free), "Time");
        assert_eq!(p.qlink_turns(n.params(), "Time", synced), "Sync");
        assert_eq!(p.qlink_turns(n.params(), "Mix", synced), "Mix");
    }

    /// Only a Sync rule swaps a Q-Link, and only onto a choice.
    #[test]
    fn swaps_are_sync_rules() {
        for p in PLANS {
            for d in p.dims.iter().filter(|d| d.swap) {
                assert_eq!(d.reason, "Sync sets it", "{}", p.id);
                assert!(d.by.ends_with("Sync") || d.by.ends_with("sync"), "{}", p.id);
            }
        }
    }

    #[test]
    fn tiers_and_labels() {
        let p = plan("mpfree.fx.plate").unwrap();
        let reverb = &p.pages[0].sections[0];
        assert_eq!(reverb.tier("Decay", true), Tier::Hero);
        assert_eq!(reverb.tier("Pre-delay", true), Tier::Main);
        let output = &p.pages[0].sections[3];
        assert_eq!(output.tier("Width", false), Tier::Trim);
        assert_eq!(output.tier("Mix", true), Tier::Main);
        let eq = plan("mpfree.fx.eq").unwrap();
        let band = &eq.pages[0].sections[2];
        assert_eq!(band.label("Band 1/Gain"), "Gain");
        assert_eq!(band.tier("Band 1/Freq", false), Tier::Trim);
        let strip = plan("mpfree.fx.channel_strip").unwrap();
        assert_eq!(strip.pages[0].sections[0].label("Gate thresh"), "Threshold");
    }
}
