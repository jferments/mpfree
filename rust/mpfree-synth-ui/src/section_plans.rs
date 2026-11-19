//! The native engines' section plans ([`crate::sections`]): for each page,
//! its sections (what job, which controls, which are heroes, which show
//! as faders, which drawing) and how they lie.
//!
//! Plans name parameters by id; [`plan`] turns them into indexes for the
//! node at hand and leaves out any id the node lacks.

use crate::sections::{Arrangement, Job, Section};
use mpfree_node::pressure::field;
use mpfree_node::{NodeInfo, ParamInfo};

use Job::{Motion, Shape, Source, Tone, Voice};

/// A section as a plan writes it, by parameter id.
struct S {
    key: &'static str,
    name: &'static str,
    job: Job,
    ids: Vec<u16>,
    hero: Vec<u16>,
    main: Vec<u16>,
    trim: Vec<u16>,
    faders: Vec<u16>,
    drawbars: bool,
    rows: bool,
    labels: Vec<(u16, String)>,
    drawing: Option<String>,
    trace: Option<usize>,
    link: Option<&'static str>,
    note: Option<&'static str>,
}

fn s(key: &'static str, name: &'static str, job: Job, ids: &[u16]) -> S {
    S {
        key,
        name,
        job,
        ids: ids.to_vec(),
        hero: Vec::new(),
        main: Vec::new(),
        trim: Vec::new(),
        faders: Vec::new(),
        drawbars: false,
        rows: false,
        labels: Vec::new(),
        drawing: None,
        trace: None,
        link: None,
        note: None,
    }
}

impl S {
    fn hero(mut self, ids: &[u16]) -> S {
        self.hero = ids.to_vec();
        self
    }
    fn main(mut self, ids: &[u16]) -> S {
        self.main = ids.to_vec();
        self
    }
    fn trim(mut self, ids: &[u16]) -> S {
        self.trim = ids.to_vec();
        self
    }
    fn faders(mut self, ids: &[u16]) -> S {
        self.faders = ids.to_vec();
        self
    }
    fn drawbars(mut self) -> S {
        self.drawbars = true;
        self
    }
    fn rows(mut self) -> S {
        self.rows = true;
        self
    }
    fn labels(mut self, l: &[(u16, &str)]) -> S {
        self.labels
            .extend(l.iter().map(|&(id, s)| (id, s.to_string())));
        self
    }
    fn draw(mut self, title: &str) -> S {
        self.drawing = Some(title.to_string());
        self
    }
    /// Only curve `k` of the drawing.
    fn draw_trace(mut self, title: &str, k: usize) -> S {
        self.drawing = Some(title.to_string());
        self.trace = Some(k);
        self
    }
    fn link(mut self, page: &'static str) -> S {
        self.link = Some(page);
        self
    }
    fn note(mut self, note: &'static str) -> S {
        self.note = Some(note);
        self
    }
}

/// The plan's sections as indexes into `params`.
fn build(params: &[ParamInfo], spec: &str, list: Vec<S>) -> Arrangement {
    let at = |id: &u16| params.iter().position(|p| p.id == *id);
    let all = |ids: &[u16]| ids.iter().filter_map(at).collect::<Vec<usize>>();
    let sections = list
        .into_iter()
        .map(|p| {
            let mut sec = Section::new(p.key, p.name, p.job);
            sec.params = all(&p.ids);
            sec.hero = all(&p.hero);
            sec.main = all(&p.main);
            sec.trim = all(&p.trim);
            sec.faders = all(&p.faders);
            sec.drawbars = p.drawbars;
            sec.rows = p.rows;
            sec.labels = p
                .labels
                .into_iter()
                .filter_map(|(id, l)| at(&id).map(|i| (i, l)))
                .collect();
            sec.drawing = p.drawing;
            sec.trace = p.trace;
            sec.link = p.link.map(str::to_string);
            sec.note = p.note.map(str::to_string);
            sec
        })
        .collect();
    Arrangement {
        spec: spec.to_string(),
        sections,
    }
}

/// The engine's plan for page `page`, if it has one, with the node's
/// values (in `params` order: Drums' pad pages follow the pad's model).
pub fn plan(
    info: &NodeInfo,
    params: &[ParamInfo],
    values: &[f32],
    page: &str,
) -> Option<Arrangement> {
    let value = |id: u16| {
        params
            .iter()
            .position(|p| p.id == id)
            .map_or(0.0, |i| values.get(i).copied().unwrap_or(params[i].default))
    };
    let (spec, list) = match info.id {
        "mpfree.poly" => poly(page)?,
        "mpfree.fm" => fm(page)?,
        "mpfree.wavetable" => wavetable(page)?,
        "mpfree.drums" => drums(page, &value)?,
        "mpfree.pluck" => pluck(page)?,
        "mpfree.keys" => keys(page)?,
        "mpfree.drawbar" => drawbar(page)?,
        "mpfree.squelch" => squelch(page)?,
        "mpfree.analog" => analog(page)?,
        "mpfree.strings" => strings(page)?,
        "mpfree.bass" => bass(page)?,
        "mpfree.wind" => wind(page)?,
        "mpfree.choir" => choir(page)?,
        _ => return None,
    };
    Some(build(params, spec, list))
}

type Plan = (&'static str, Vec<S>);

const ADSR: [&str; 4] = ["Attack", "Decay", "Sustain", "Release"];

fn adsr_labels(ids: [u16; 4]) -> Vec<(u16, &'static str)> {
    ids.into_iter().zip(ADSR).collect()
}

/// An engine's Pressure page: the shaper with its curve, the four slots
/// as routing rows, and each sounding voice's pressure.
fn pressure(base: u16) -> Plan {
    let f = |k: u16| base + k;
    let slots: Vec<u16> = (0..4)
        .flat_map(|n| [f(field::target(n)), f(field::depth(n))])
        .collect();
    let slot_labels: Vec<(u16, String)> = (0..4)
        .map(|n| (f(field::target(n)), format!("Slot {}", n + 1)))
        .collect();
    let mut slots = s("slots", "Sends to", Motion, &slots).rows();
    slots.labels = slot_labels;
    (
        "row(shaper:50*, col(slots:*, voices:170):50*)",
        vec![
            s(
                "shaper",
                "Pressure",
                Motion,
                &[
                    f(field::MODE),
                    f(field::CURVE),
                    f(field::RELEASE),
                    f(field::THRESHOLD),
                    f(field::FULL_AT),
                    f(field::HOLDOFF),
                    f(field::RISE),
                    f(field::FALL),
                ],
            )
            .draw("Pressure"),
            slots,
            s("voices", "Voices", Voice, &[]).draw("Voices"),
        ],
    )
}

fn poly(page: &str) -> Option<Plan> {
    use mpfree_synth::params::*;
    Some(match page {
        "Osc" => (
            "col(row(osc-1:*, osc-2:*):*, mix:230)",
            vec![
                s(
                    "osc-1",
                    "Osc 1",
                    Source,
                    &[OSC1_WAVE, OSC1_OCT, OSC1_SEMI, OSC1_FINE, OSC1_PW],
                )
                .labels(&[(OSC1_PW, "Pulse width")])
                .draw("Osc"),
                s(
                    "osc-2",
                    "Osc 2",
                    Source,
                    &[OSC2_WAVE, OSC2_OCT, OSC2_SEMI, OSC2_FINE, OSC2_PW],
                )
                .labels(&[(OSC2_PW, "Pulse width")]),
                s(
                    "mix",
                    "Mix",
                    Source,
                    &[OSC1_LEVEL, OSC2_LEVEL, SUB_LEVEL, NOISE_LEVEL, SUB_WAVE],
                )
                .faders(&[OSC1_LEVEL, OSC2_LEVEL, SUB_LEVEL, NOISE_LEVEL])
                .labels(&[(OSC1_LEVEL, "Osc 1"), (OSC2_LEVEL, "Osc 2")])
                .note("into the filter"),
            ],
        ),
        // The two waves with settings of their own: each oscillator's
        // Table wave, its table drawn, with what moves the positions, and
        // the Supersaw.
        "Waves" => (
            "col(row(table-1:*, table-2:*, supersaw:210):*, moves:150)",
            vec![
                s(
                    "table-1",
                    "Osc 1 table",
                    Source,
                    &[OSC1_TABLE, OSC1_WARP_MODE, OSC1_POS, OSC1_WARP],
                )
                .hero(&[OSC1_POS])
                .draw("Table 1"),
                s(
                    "table-2",
                    "Osc 2 table",
                    Source,
                    &[OSC2_TABLE, OSC2_WARP_MODE, OSC2_POS, OSC2_WARP],
                )
                .hero(&[OSC2_POS])
                .draw("Table 2"),
                s(
                    "moves",
                    "Moves",
                    Motion,
                    &[FENV_POS1, FENV_POS2, FENV_WARP, VEL_POS, LFO_POS1, LFO_POS2],
                )
                .main(&[FENV_POS1, FENV_POS2, FENV_WARP, VEL_POS, LFO_POS1, LFO_POS2])
                .labels(&[
                    (FENV_POS1, "Env to position 1"),
                    (FENV_POS2, "Env to position 2"),
                    (FENV_WARP, "Env to warp"),
                    (VEL_POS, "Velocity to position"),
                    (LFO_POS1, "LFO to position 1"),
                    (LFO_POS2, "LFO to position 2"),
                ])
                .note("the filter envelope and LFO 1"),
                // Its own words, so they don't read as the unison's.
                s("supersaw", "Supersaw", Source, &[SUPER_DETUNE, SUPER_MIX])
                    .labels(&[(SUPER_DETUNE, "Saw detune"), (SUPER_MIX, "Side saws")])
                    .note("seven saws"),
            ],
        ),
        "Filter" => (
            "row(filter:52*, col(fenv:*, follow:200):48*)",
            vec![
                s(
                    "filter",
                    "Filter",
                    Tone,
                    &[FILTER_MODE, CUTOFF, RESONANCE, DRIVE, DIST],
                )
                .hero(&[CUTOFF, RESONANCE])
                .draw("Filter"),
                s(
                    "fenv",
                    "Filter envelope",
                    Shape,
                    &[FENV_A, FENV_D, FENV_S, FENV_R, FENV_AMOUNT],
                )
                .faders(&[FENV_A, FENV_D, FENV_S, FENV_R])
                .main(&[FENV_AMOUNT])
                .labels(&[(FENV_AMOUNT, "Amount")]),
                s("follow", "Follow", Voice, &[KEY_TRACK, VEL_CUTOFF]),
            ],
        ),
        "Env" => (
            "amp",
            vec![s(
                "amp",
                "Amp envelope",
                Shape,
                &[AMP_A, AMP_D, AMP_S, AMP_R, VEL_AMP],
            )
            .faders(&[AMP_A, AMP_D, AMP_S, AMP_R])
            .labels(&[(VEL_AMP, "Velocity to level")])
            .draw_trace("Env", 0)
            .note("to the level")],
        ),
        "LFO" => (
            "row(lfo:*, sends:*)",
            vec![
                s(
                    "lfo",
                    "LFO",
                    Motion,
                    &[LFO_SHAPE, LFO_SYNC, LFO_RETRIG, LFO_RATE],
                )
                .hero(&[LFO_RATE])
                .draw("LFO"),
                s(
                    "sends",
                    "Sends to",
                    Motion,
                    &[LFO_PITCH, LFO_CUTOFF, LFO_PW, LFO_AMP],
                )
                .main(&[LFO_PITCH, LFO_CUTOFF, LFO_PW, LFO_AMP])
                .labels(&[
                    (LFO_PITCH, "Pitch"),
                    (LFO_CUTOFF, "Cutoff"),
                    (LFO_PW, "Pulse width"),
                    (LFO_AMP, "Level"),
                ]),
            ],
        ),
        // Each note's own LFO. Its sends are named as LFO 1's are, on a
        // page of their own, so no screen shows two of one name.
        "LFO 2" => (
            "row(lfo-2:*, sends-2:*)",
            vec![
                s(
                    "lfo-2",
                    "LFO 2",
                    Motion,
                    &[LFO2_SHAPE, LFO2_SYNC, LFO2_RETRIG, LFO2_RATE, LFO2_FADE],
                )
                .hero(&[LFO2_RATE])
                .draw("LFO 2")
                .note("each note's own"),
                s(
                    "sends-2",
                    "Sends to",
                    Motion,
                    &[LFO2_PITCH, LFO2_CUTOFF, LFO2_PW, LFO2_AMP],
                )
                .main(&[LFO2_PITCH, LFO2_CUTOFF, LFO2_PW, LFO2_AMP])
                .labels(&[
                    (LFO2_PITCH, "Pitch"),
                    (LFO2_CUTOFF, "Cutoff"),
                    (LFO2_PW, "Pulse width"),
                    (LFO2_AMP, "Level"),
                ]),
            ],
        ),
        // A box per route: where it comes from, what it moves, how far.
        "Routes" => (
            "col(row(route-1:*, route-2:*):*, row(route-3:*, route-4:*):*)",
            (0..ROUTES)
                .map(|n| {
                    const KEYS: [&str; ROUTES] = ["route-1", "route-2", "route-3", "route-4"];
                    const NAMES: [&str; ROUTES] = ["Route 1", "Route 2", "Route 3", "Route 4"];
                    let id = |f: u16| route_id(n, f);
                    let sec = s(
                        KEYS[n],
                        NAMES[n],
                        Motion,
                        &[id(route::SOURCE), id(route::TARGET), id(route::DEPTH)],
                    )
                    .main(&[id(route::DEPTH)]);
                    if n == 0 {
                        sec.note("wheel, breath, foot, velocity, key, tone, random")
                    } else {
                        sec
                    }
                })
                .collect(),
        ),
        "Voice" => (
            "row(col(voices:*, unison:*):55*, col(tuning:*, output:*):45*)",
            vec![
                // Age: how far each voice is from the others (the voice
                // core's cards, as on Analog).
                s(
                    "voices",
                    "Voices",
                    Voice,
                    &[VOICE_MODE, POLYPHONY, GLIDE, AGE],
                ),
                // Not "Voices": the Voices section has that word.
                s(
                    "unison",
                    "Unison",
                    Voice,
                    &[UNISON, UNISON_DETUNE, UNISON_SPREAD],
                )
                .labels(&[(UNISON, "Copies")])
                .note("of each note"),
                s("tuning", "Tuning", Voice, &[BEND_RANGE, PRESS_VIB_RATE]),
                s("output", "Output", Voice, &[LEVEL, PAN, METER_VOICES]),
            ],
        ),
        "Pressure" => pressure(PRESS_BASE),
        _ => return None,
    })
}

/// A Voice page: how it plays, its tuning and its output.
fn voice_page(voices: &[u16], tuning: &[u16], output: &[u16]) -> Plan {
    (
        "row(col(voices:*, tuning:*):55*, output:45*)",
        vec![
            s("voices", "Voices", Voice, voices),
            s("tuning", "Tuning", Voice, tuning),
            s("output", "Output", Voice, output),
        ],
    )
}

fn fm(page: &str) -> Option<Plan> {
    use mpfree_synth::fm_params::*;
    // The six operators' On switches sit on the Algo page.
    const OPS_ON: [u16; 6] = [
        op_id(1, op::ON),
        op_id(2, op::ON),
        op_id(3, op::ON),
        op_id(4, op::ON),
        op_id(5, op::ON),
        op_id(6, op::ON),
    ];
    if let Some(n) = page
        .strip_prefix("Op ")
        .and_then(|n| n.parse::<usize>().ok())
        .filter(|n| (1..=6).contains(n))
    {
        let o = |f: u16| op_id(n, f);
        let env = [
            o(op::R1),
            o(op::R2),
            o(op::R3),
            o(op::R4),
            o(op::L1),
            o(op::L2),
            o(op::L3),
            o(op::L4),
        ];
        return Some((
            "col(row(env:*, freq:*):*, row(out:*, algo:*):230)",
            vec![
                s("env", "Envelope", Shape, &env)
                    .faders(&env)
                    .draw(&format!("Op {n} env")),
                s(
                    "freq",
                    "Oscillator",
                    Source,
                    &[
                        o(op::WAVE),
                        o(op::MODE),
                        o(op::COARSE),
                        o(op::FINE),
                        o(op::DETUNE),
                    ],
                ),
                s(
                    "out",
                    "Output",
                    Voice,
                    &[o(op::LEVEL), o(op::VEL_SENS), o(op::AMS)],
                ),
                s("algo", "Algorithm", Source, &[]).draw("Algorithm"),
            ],
        ));
    }
    if let Some((a, b)) = page.strip_prefix("Kbd ").and_then(|r| {
        let (a, b) = r.split_once('-')?;
        Some((a.parse::<usize>().ok()?, b.parse::<usize>().ok()?))
    }) {
        let kbd = |n: usize| {
            let o = |f: u16| op_id(n, f);
            [
                o(op::L_CURVE),
                o(op::R_CURVE),
                o(op::BREAK),
                o(op::L_DEPTH),
                o(op::R_DEPTH),
                o(op::RATE_SCALE),
            ]
        };
        let labels = |n: usize| {
            let o = |f: u16| op_id(n, f);
            vec![
                (o(op::L_CURVE), "Left curve"),
                (o(op::R_CURVE), "Right curve"),
                (o(op::L_DEPTH), "Left depth"),
                (o(op::R_DEPTH), "Right depth"),
                (o(op::RATE_SCALE), "Rate scale"),
            ]
        };
        let knobs = |n: usize| {
            let o = |f: u16| op_id(n, f);
            [o(op::BREAK), o(op::L_DEPTH), o(op::R_DEPTH)]
        };
        let (ka, kb) = (kbd(a), kbd(b));
        let name =
            |n: usize| -> &'static str { ["Op 1", "Op 2", "Op 3", "Op 4", "Op 5", "Op 6"][n - 1] };
        return Some((
            "col(scaling:190, row(kbd-a:*, kbd-b:*):*)",
            vec![
                s("scaling", "Keyboard scaling", Tone, &[]).draw("Scaling"),
                s("kbd-a", name(a), Tone, &ka)
                    .trim(&knobs(a))
                    .labels(&labels(a)),
                s("kbd-b", name(b), Tone, &kb)
                    .trim(&knobs(b))
                    .labels(&labels(b)),
            ],
        ));
    }
    Some(match page {
        "Algo" => (
            "row(col(algo:*, ops:170):52*, col(levels:180, voice:*):48*)",
            vec![
                s(
                    "algo",
                    "Algorithm",
                    Source,
                    &[ALGORITHM, FEEDBACK, OSC_SYNC],
                )
                .draw("Algorithm"),
                s("ops", "Operators", Source, &OPS_ON).labels(&[
                    (OPS_ON[0], "Op 1 on"),
                    (OPS_ON[1], "Op 2 on"),
                    (OPS_ON[2], "Op 3 on"),
                    (OPS_ON[3], "Op 4 on"),
                    (OPS_ON[4], "Op 5 on"),
                    (OPS_ON[5], "Op 6 on"),
                ]),
                s("levels", "Levels", Voice, &[]).draw("Levels"),
                s(
                    "voice",
                    "Voice",
                    Voice,
                    &[
                        VOICE_MODE,
                        LEVEL,
                        TRANSPOSE,
                        POLYPHONY,
                        BEND_RANGE,
                        METER_VOICES,
                    ],
                ),
            ],
        ),
        "Mod" => {
            let peg = [
                PEG_R1, PEG_R2, PEG_R3, PEG_R4, PEG_L1, PEG_L2, PEG_L3, PEG_L4,
            ];
            (
                "row(lfo:*, peg:*)",
                vec![
                    s(
                        "lfo",
                        "LFO",
                        Motion,
                        &[
                            LFO_WAVE, LFO_SYNC, LFO_SPEED, LFO_DELAY, LFO_PMD, LFO_AMD, LFO_PMS,
                        ],
                    )
                    .hero(&[LFO_SPEED])
                    .draw("LFO"),
                    s("peg", "Pitch envelope", Shape, &peg)
                        .faders(&peg)
                        .draw("Pitch env"),
                ],
            )
        }
        "Pressure" => pressure(PRESS_BASE),
        _ => return None,
    })
}

fn wavetable(page: &str) -> Option<Plan> {
    use mpfree_synth::wavetable::params::*;
    if let Some(o) = match page {
        "Osc A" => Some(0),
        "Osc B" => Some(1),
        _ => None,
    } {
        let id = |f: u16| osc_id(o, f);
        return Some((
            "row(table:58*, col(pitch:*, level:200):42*)",
            vec![
                s(
                    "table",
                    "Table",
                    Source,
                    &[id(TABLE), id(WARP_MODE), id(POSITION), id(WARP)],
                )
                .hero(&[id(POSITION)])
                .labels(&[(id(WARP), "Warp")])
                .draw(page),
                s("pitch", "Pitch", Source, &[id(OCT), id(SEMI), id(FINE)]),
                s("level", "Level", Voice, &[id(OSC_LEVEL)]),
            ],
        ));
    }
    Some(match page {
        "Mix" => (
            "row(mix:*, unison:*)",
            vec![
                s("mix", "Mix", Source, &[SUB_LEVEL, NOISE_LEVEL])
                    .faders(&[SUB_LEVEL, NOISE_LEVEL])
                    .note("into the filter"),
                s(
                    "unison",
                    "Unison",
                    Voice,
                    &[UNISON, UNISON_DETUNE, UNISON_SPREAD],
                )
                .labels(&[(UNISON, "Voices")]),
            ],
        ),
        "Filter" => (
            "row(filter:56*, col(env:*, follow:*):44*)",
            vec![
                s(
                    "filter",
                    "Filter",
                    Tone,
                    &[FILTER_MODE, CUTOFF, RESONANCE, DRIVE, DIST],
                )
                .hero(&[CUTOFF, RESONANCE])
                .draw("Filter"),
                s("env", "Mod envelope", Shape, &[MENV_CUTOFF])
                    .main(&[MENV_CUTOFF])
                    .labels(&[(MENV_CUTOFF, "To cutoff")])
                    .draw_trace("Env", 1)
                    .link("Mod"),
                s("follow", "Follow", Voice, &[KEY_TRACK, VEL_CUTOFF]),
            ],
        ),
        "Mod" => (
            "row(menv:*, sends:*)",
            vec![
                s(
                    "menv",
                    "Mod envelope",
                    Shape,
                    &[MENV_A, MENV_D, MENV_S, MENV_R],
                )
                .faders(&[MENV_A, MENV_D, MENV_S, MENV_R])
                .labels(&adsr_labels([MENV_A, MENV_D, MENV_S, MENV_R]))
                .draw_trace("Env", 1),
                s(
                    "sends",
                    "Sends to",
                    Motion,
                    &[MENV_POS_A, MENV_POS_B, MENV_WARP, VEL_POS],
                )
                .main(&[MENV_POS_A, MENV_POS_B, MENV_WARP, VEL_POS])
                .labels(&[
                    (MENV_POS_A, "Position A"),
                    (MENV_POS_B, "Position B"),
                    (MENV_WARP, "Warp"),
                    (VEL_POS, "Velocity to position"),
                ]),
            ],
        ),
        "LFO" => (
            "row(lfo:*, sends:*)",
            vec![
                s(
                    "lfo",
                    "LFO",
                    Motion,
                    &[LFO_SHAPE, LFO_SYNC, LFO_RETRIG, LFO_RATE],
                )
                .hero(&[LFO_RATE]),
                s(
                    "sends",
                    "Sends to",
                    Motion,
                    &[LFO_POS_A, LFO_POS_B, LFO_PITCH, LFO_CUTOFF, LFO_AMP],
                )
                .labels(&[
                    (LFO_POS_A, "Position A"),
                    (LFO_POS_B, "Position B"),
                    (LFO_PITCH, "Pitch"),
                    (LFO_CUTOFF, "Cutoff"),
                    (LFO_AMP, "Level"),
                ]),
            ],
        ),
        "Env" => (
            "amp",
            vec![s(
                "amp",
                "Amp envelope",
                Shape,
                &[AMP_A, AMP_D, AMP_S, AMP_R, VEL_AMP],
            )
            .faders(&[AMP_A, AMP_D, AMP_S, AMP_R])
            .labels(&[(VEL_AMP, "Velocity to level")])
            .draw_trace("Env", 0)
            .note("to the level")],
        ),
        "Voice" => voice_page(
            &[VOICE_MODE, POLYPHONY, GLIDE],
            &[BEND_RANGE, PRESS_VIB_RATE],
            &[LEVEL, PAN, METER_VOICES],
        ),
        "Pressure" => pressure(PRESS_BASE),
        _ => return None,
    })
}

fn drums(page: &str, value: &dyn Fn(u16) -> f32) -> Option<Plan> {
    use mpfree_synth::drum_params::{
        self as dp, field as f, pad_id, BASE_NOTE, LEVEL, METER_VOICES, PADS, VOICES,
    };
    if let Some(p) = page
        .strip_prefix("Pad ")
        .and_then(|n| n.parse::<usize>().ok())
        .filter(|n| (1..=PADS).contains(n))
        .map(|n| n - 1)
    {
        let id = |k: u16| pad_id(p, k);
        let m = dp::model_of(value(id(f::MODEL)));
        let current = value(VOICES) >= 0.5;
        if !current && m < dp::model::FIRST {
            // A Classic kit's first-engine drum: its fourteen, as always.
            return Some((
                "row(col(kit:150, sound:*):50*, col(body:*, mix:*):50*)",
                vec![
                    s("kit", "Kit", Source, &[]).draw("Kit").link("Kit"),
                    s(
                        "sound",
                        "Sound",
                        Source,
                        &[
                            id(f::MODEL),
                            id(f::TUNE),
                            id(f::DECAY),
                            id(f::TONE),
                            id(f::SNAP),
                        ],
                    )
                    .draw("Hit"),
                    s(
                        "body",
                        "Noise and sweep",
                        Tone,
                        &[id(f::NOISE), id(f::SWEEP), id(f::SWEEP_TIME), id(f::DRIVE)],
                    )
                    .draw("Sweep"),
                    s(
                        "mix",
                        "Mix",
                        Voice,
                        &[
                            id(f::CHOKE),
                            id(f::LEVEL),
                            id(f::PAN),
                            id(f::VEL_TONE),
                            id(f::VEL_LEVEL),
                        ],
                    ),
                ],
            ));
        }
        // Any other drum: the model and the four its Q-Links turn, the rest
        // of its own knobs, then how it sits in the kit and plays. Each knob
        // named as that drum calls it ("Punch" on the 808).
        let q = dp::qlinks(m);
        let mut sound = vec![id(f::MODEL)];
        sound.extend(q.iter().map(|&k| id(k)));
        let shape: Vec<u16> = dp::knobs(m)
            .iter()
            .filter(|(k, _)| !q.contains(k))
            .map(|&(k, _)| id(k))
            .collect();
        let labels: Vec<(u16, &str)> = dp::knobs(m).iter().map(|&(k, l)| (id(k), l)).collect();
        let bends = dp::knobs(m).iter().any(|&(k, _)| k == f::SWEEP);
        let shape = s("body", "Shape", Tone, &shape).labels(&labels);
        return Some((
            "row(col(kit:150, sound:*, feel:150):50*, col(body:*, mix:300):50*)",
            vec![
                s("kit", "Kit", Source, &[]).draw("Kit").link("Kit"),
                s("sound", "Sound", Source, &sound)
                    .labels(&labels)
                    .draw("Hit"),
                s(
                    "feel",
                    "Feel",
                    Motion,
                    &[
                        id(f::VEL_LEVEL),
                        id(f::VEL_TONE),
                        id(f::ACCENT),
                        id(f::VARIATION),
                    ],
                ),
                if bends { shape.draw("Sweep") } else { shape },
                s(
                    "mix",
                    "Mix",
                    Voice,
                    &[id(f::LEVEL), id(f::PAN), id(f::CHOKE), id(f::PLAYS)],
                ),
            ],
        ));
    }
    if let Some(p) = page
        .strip_prefix("Pad ")
        .and_then(|r| r.strip_suffix(" effects"))
        .and_then(|n| n.parse::<usize>().ok())
        .filter(|n| (1..=PADS).contains(n))
        .map(|n| n - 1)
    {
        use mpfree_synth::drum_fx::{field as x, fx_id};
        let id = |k: u16| fx_id(p, k);
        return Some((
            "col(row(kit:3*, punch:2*, dist:3*):45*, row(eq:4*, comp:4*, out:3*):55*)",
            vec![
                s("kit", "Kit", Source, &[]).draw("Kit"),
                s("punch", "Punch", Shape, &[id(x::PUNCH), id(x::SUSTAIN)]).hero(&[id(x::PUNCH)]),
                s(
                    "dist",
                    "Distortion",
                    Tone,
                    &[id(x::DISTORTION), id(x::DIST_TONE), id(x::DIST_MIX)],
                )
                .hero(&[id(x::DISTORTION)])
                .labels(&[(id(x::DIST_TONE), "Tone"), (id(x::DIST_MIX), "Mix")]),
                s(
                    "eq",
                    "EQ",
                    Tone,
                    &[id(x::LOW), id(x::MID), id(x::MID_FREQ), id(x::HIGH)],
                )
                .trim(&[id(x::MID_FREQ)]),
                s(
                    "comp",
                    "Compressor",
                    Shape,
                    &[
                        id(x::THRESHOLD),
                        id(x::RATIO),
                        id(x::COMP_ATTACK),
                        id(x::COMP_RELEASE),
                    ],
                )
                .main(&[id(x::THRESHOLD), id(x::RATIO)])
                .labels(&[
                    (id(x::COMP_ATTACK), "Attack"),
                    (id(x::COMP_RELEASE), "Release"),
                ]),
                s(
                    "out",
                    "Output and sends",
                    Voice,
                    &[id(x::OUTPUT), id(x::DELAY_SEND), id(x::REVERB_SEND)],
                )
                .labels(&[(id(x::DELAY_SEND), "Delay"), (id(x::REVERB_SEND), "Reverb")])
                .link("Sends"),
            ],
        ));
    }
    Some(match page {
        "Sends" => {
            use mpfree_synth::drum_fx::kit as k;
            (
                "row(delay:4*, reverb:5*)",
                vec![
                    s(
                        "delay",
                        "Delay",
                        Motion,
                        &[
                            k::DELAY_TIME,
                            k::DELAY_FEEDBACK,
                            k::DELAY_TONE,
                            k::DELAY_LEVEL,
                        ],
                    )
                    .labels(&[
                        (k::DELAY_TIME, "Time"),
                        (k::DELAY_FEEDBACK, "Feedback"),
                        (k::DELAY_TONE, "Tone"),
                    ]),
                    s(
                        "reverb",
                        "Reverb",
                        Motion,
                        &[
                            k::REVERB_SIZE,
                            k::REVERB_DECAY,
                            k::REVERB_TONE,
                            k::REVERB_GATE,
                            k::REVERB_LEVEL,
                        ],
                    )
                    .main(&[k::REVERB_DECAY])
                    .labels(&[
                        (k::REVERB_SIZE, "Size"),
                        (k::REVERB_DECAY, "Decay"),
                        (k::REVERB_TONE, "Tone"),
                        (k::REVERB_GATE, "Gate"),
                    ]),
                ],
            )
        }
        "Kit" => (
            "col(kit:*, row(levels:*, out:*):240)",
            vec![
                s("kit", "Kit", Source, &[]).draw("Kit"),
                s("levels", "Levels", Voice, &[]).draw("Levels"),
                s(
                    "out",
                    "Output",
                    Voice,
                    &[LEVEL, BASE_NOTE, VOICES, METER_VOICES],
                ),
            ],
        ),
        _ => return None,
    })
}

fn pluck(page: &str) -> Option<Plan> {
    use mpfree_synth::pluck_params::*;
    Some(match page {
        "String" => (
            "row(col(model:140, decay:*):44*, string:56*)",
            vec![
                s("model", "Model", Source, &[MODEL]),
                s("decay", "Decay", Shape, &[DECAY, RELEASE]).draw("Decay"),
                s("string", "String", Source, &[BRIGHTNESS, PICK, POSITION]).draw("Pick"),
            ],
        ),
        "Mallet" => (
            "row(mallet:*, modes:*)",
            vec![
                s("mallet", "Mallet", Source, &[MATERIAL, HARDNESS, STRIKE]).draw("Mallet"),
                s("modes", "Modes", Tone, &[STRETCH, HIGH_DAMP]).draw("Modes"),
            ],
        ),
        "Voice" => (
            "row(col(tuning:*, voices:*):55*, output:45*)",
            vec![
                s("tuning", "Tuning", Voice, &[TRANSPOSE, FINE, BEND_RANGE]),
                s(
                    "voices",
                    "Voices and touch",
                    Voice,
                    &[POLYPHONY, VEL_TONE, VELOCITY],
                )
                .labels(&[(VELOCITY, "Vel > level")]),
                s(
                    "output",
                    "Output",
                    Voice,
                    &[LEVEL, PAN, SPREAD, METER_VOICES],
                ),
            ],
        ),
        _ => return None,
    })
}

fn keys(page: &str) -> Option<Plan> {
    use mpfree_synth::keys_params::*;
    Some(match page {
        "Strike" => (
            "row(col(model:140, hammer:*):*, decay:*)",
            vec![
                s("model", "Model", Source, &[MODEL]),
                s("hammer", "Hammer", Source, &[HARDNESS, BELL]).draw("Hammer"),
                s("decay", "Decay", Shape, &[DECAY, RELEASE]).draw("Decay"),
            ],
        ),
        "Tone" => (
            "row(pickup:56*, tone:44*)",
            vec![
                s("pickup", "Pickup", Tone, &[VOICING, BARK])
                    .hero(&[VOICING])
                    .draw("Pickup"),
                s("tone", "Tone and touch", Tone, &[TONE, VEL_TONE, VELOCITY]),
            ],
        ),
        "Clav" => (
            "row(pickups:56*, damping:44*)",
            vec![
                s("pickups", "Pickups", Tone, &[PICKUP, CLAV_TONE])
                    .labels(&[(CLAV_TONE, "Tone")])
                    .draw("Pickups"),
                s("damping", "Mute and drop", Shape, &[MUTE, DROP])
                    .labels(&[(DROP, "Drop")])
                    .draw("Mute"),
            ],
        ),
        "Amp" => (
            "row(trem:*, amp:*)",
            vec![
                s("trem", "Tremolo", Motion, &[TREM_MODE, TREMOLO, TREM_RATE])
                    .labels(&[(TREMOLO, "Depth"), (TREM_RATE, "Rate"), (TREM_MODE, "Mode")])
                    .draw("Tremolo"),
                s("amp", "Drive and noise", Tone, &[DRIVE, KEY_NOISE]),
            ],
        ),
        "Voice" => (
            "row(col(tuning:*, voices:*):55*, output:45*)",
            vec![
                s("tuning", "Tuning", Voice, &[TRANSPOSE, FINE, BEND_RANGE]),
                s("voices", "Voices", Voice, &[POLYPHONY]),
                s("output", "Output", Voice, &[LEVEL, PAN, METER_VOICES]),
            ],
        ),
        _ => return None,
    })
}

fn drawbar(page: &str) -> Option<Plan> {
    use mpfree_synth::drawbar_params::*;
    let bars = [
        DB_16, DB_5_13, DB_8, DB_4, DB_2_23, DB_2, DB_1_35, DB_1_13, DB_1,
    ];
    Some(match page {
        "Drawbars" => (
            "row(bars:*, col(model:150, voices:*):206, col(wave:190, perc:*, rotary:*):282)",
            vec![
                s("model", "Model", Source, &[MODEL]),
                s("voices", "Voices", Tone, &[FLUTE, REED, REED_WAVE])
                    .faders(&[FLUTE, REED])
                    .drawbars()
                    .labels(&[(REED_WAVE, "Reed wave")]),
                s("bars", "Drawbars", Source, &bars)
                    .faders(&bars)
                    .drawbars()
                    .note("pull a bar down to add it"),
                s("wave", "The wave", Source, &[]).draw("Wave"),
                s("perc", "Percussion", Shape, &[PERC])
                    .draw("Percussion")
                    .link("Percussion"),
                s("rotary", "Rotary", Motion, &[SPEED])
                    .link("Rotary")
                    .labels(&[(SPEED, "Speed")]),
            ],
        ),
        "Percussion" => (
            "row(perc:*, vib:*)",
            vec![
                s(
                    "perc",
                    "Percussion",
                    Shape,
                    &[PERC, PERC_HARM, PERC_DECAY, PERC_VOL],
                )
                .labels(&[
                    (PERC, "On"),
                    (PERC_HARM, "Harmonic"),
                    (PERC_DECAY, "Decay"),
                    (PERC_VOL, "Volume"),
                ])
                .draw("Percussion"),
                s("vib", "Vibrato and chorus", Motion, &[VIBRATO, SCANNER])
                    .labels(&[(SCANNER, "Rate")]),
            ],
        ),
        "Rotary" => (
            "row(rotary:*, tone:*)",
            vec![
                s("rotary", "Rotary speaker", Motion, &[ROTARY, SPEED, SPREAD])
                    .labels(&[(ROTARY, "On")])
                    .draw("Rotor"),
                s("tone", "Tone", Tone, &[WHEELS, DRIVE, TONE, CLICK, LEAKAGE]),
            ],
        ),
        "Voice" => (
            "row(tuning:*, output:*)",
            vec![
                s("tuning", "Tuning", Voice, &[TRANSPOSE, FINE]),
                s("output", "Output", Voice, &[LEVEL, PAN, METER_KEYS]),
            ],
        ),
        _ => return None,
    })
}

fn squelch(page: &str) -> Option<Plan> {
    use mpfree_synth::squelch_params::*;
    Some(match page {
        "Synth" => (
            "col(row(filter:*, wave:272):*, row(env:*, playing:272):*)",
            vec![
                s("filter", "Filter", Tone, &[CUTOFF, RESONANCE])
                    .hero(&[CUTOFF, RESONANCE])
                    .draw("Filter"),
                s("wave", "Wave", Source, &[WAVE]).draw("Wave"),
                s(
                    "env",
                    "Envelope and accent",
                    Shape,
                    &[ENV_MOD, DECAY, ACCENT],
                )
                .note("how far each note opens the filter"),
                s("playing", "Playing", Voice, &[SLIDE, AMP_DECAY])
                    .trim(&[SLIDE, AMP_DECAY])
                    .link("Play"),
            ],
        ),
        "Play" => (
            "row(slide:56*, col(amp:*, drive:*):44*)",
            vec![
                s(
                    "slide",
                    "Slide and accent",
                    Motion,
                    &[SLIDE, ACCENT_AT, ACCENT_DECAY],
                )
                .draw("Sweep"),
                s("amp", "Amp", Shape, &[AMP_DECAY]),
                s("drive", "Drive", Tone, &[DRIVE, KEY_TRACK]),
            ],
        ),
        "Voice" => (
            "row(tuning:*, output:*)",
            vec![
                s("tuning", "Tuning", Voice, &[TRANSPOSE, FINE, BEND_RANGE]),
                s("output", "Output", Voice, &[LEVEL, PAN]),
            ],
        ),
        _ => return None,
    })
}

fn analog(page: &str) -> Option<Plan> {
    use mpfree_synth::analog_params::*;
    Some(match page {
        "Osc" => (
            "row(osc-1:270, osc-2:*, mix:220)",
            vec![
                s("osc-1", "Osc 1", Source, &[OSC1_WAVE, OSC1_OCT, OSC1_PW])
                    .labels(&[
                        (OSC1_WAVE, "Wave"),
                        (OSC1_OCT, "Octave"),
                        (OSC1_PW, "Pulse width"),
                    ])
                    .draw("Osc"),
                s(
                    "osc-2",
                    "Osc 2",
                    Source,
                    &[OSC2_WAVE, SYNC, OSC2_OCT, OSC2_SEMI, OSC2_FINE, OSC2_PW],
                )
                .labels(&[
                    (OSC2_WAVE, "Wave"),
                    (SYNC, "Sync to Osc 1"),
                    (OSC2_OCT, "Octave"),
                    (OSC2_SEMI, "Semitones"),
                    (OSC2_PW, "Pulse width"),
                ]),
                s(
                    "mix",
                    "Mix",
                    Source,
                    &[OSC1_LEVEL, OSC2_LEVEL, SUB_LEVEL, NOISE_LEVEL],
                )
                .faders(&[OSC1_LEVEL, OSC2_LEVEL, SUB_LEVEL, NOISE_LEVEL])
                .note("into the filter"),
            ],
        ),
        "Filter" => (
            "row(filter:520, col(fenv:300, row(follow:*, drive:*):*):*)",
            vec![
                s("filter", "Filter", Tone, &[FILTER_TYPE, CUTOFF, RESONANCE])
                    .hero(&[CUTOFF, RESONANCE])
                    .labels(&[(FILTER_TYPE, "Type")])
                    .draw("Filter"),
                s("fenv", "Filter envelope", Shape, &[ENV_AMT])
                    .draw_trace("Env", 1)
                    .link("Envelopes")
                    .note("to the cutoff"),
                s("follow", "Follow", Voice, &[KEY_TRACK, VEL_FILTER])
                    .labels(&[(VEL_FILTER, "Velocity")]),
                s("drive", "Drive", Tone, &[DRIVE]),
            ],
        ),
        "Envelopes" => {
            let f = [F_ATTACK, F_DECAY, F_SUSTAIN, F_RELEASE];
            let a = [A_ATTACK, A_DECAY, A_SUSTAIN, A_RELEASE];
            (
                "row(fenv:*, amp:*)",
                vec![
                    s("fenv", "Filter envelope", Shape, &f)
                        .faders(&f)
                        .labels(&adsr_labels(f))
                        .draw_trace("Env", 1)
                        .note("to the cutoff"),
                    s(
                        "amp",
                        "Amp envelope",
                        Shape,
                        &[A_ATTACK, A_DECAY, A_SUSTAIN, A_RELEASE, VEL_AMP],
                    )
                    .faders(&a)
                    .labels(&adsr_labels(a))
                    .labels(&[(VEL_AMP, "Velocity to level")])
                    .draw_trace("Env", 0)
                    .note("to the level"),
                ],
            )
        }
        "Mod" => (
            "row(lfo:*, pm:*)",
            vec![
                s(
                    "lfo",
                    "LFO",
                    Motion,
                    &[LFO_WAVE, LFO_RATE, LFO_DELAY, LFO_PITCH, LFO_PW, LFO_CUTOFF],
                )
                .hero(&[LFO_RATE])
                .labels(&[
                    (LFO_WAVE, "Wave"),
                    (LFO_RATE, "Rate"),
                    (LFO_DELAY, "Delay"),
                    (LFO_PITCH, "Vibrato"),
                    (LFO_PW, "To pulse width"),
                    (LFO_CUTOFF, "To cutoff"),
                ])
                .draw("LFO"),
                s(
                    "pm",
                    "Poly-mod",
                    Motion,
                    &[PM_PITCH, PM_PW, PM_CUTOFF, PM_ENV, PM_OSC2],
                )
                .labels(&[
                    (PM_ENV, "From filter env"),
                    (PM_OSC2, "From Osc 2"),
                    (PM_PITCH, "Osc 1 pitch"),
                    (PM_PW, "Osc 1 PW"),
                    (PM_CUTOFF, "Cutoff"),
                ])
                .note("each voice bends its own Osc 1"),
            ],
        ),
        "Voice" => (
            "col(row(character:470, voices:*):*, row(tuning:470, output:*):*)",
            vec![
                s("character", "Character", Tone, &[CHORUS, AGE])
                    .draw("Voices")
                    .note("how worn the instrument is"),
                s(
                    "voices",
                    "Voices",
                    Voice,
                    &[VOICE_MODE, POLYPHONY, GLIDE, UNI_DETUNE],
                )
                .labels(&[(UNI_DETUNE, "Detune")]),
                s("tuning", "Tuning", Voice, &[TRANSPOSE, BEND_RANGE, FINE]),
                s("output", "Output", Voice, &[LEVEL, PAN]),
            ],
        ),
        _ => return None,
    })
}

fn strings(page: &str) -> Option<Plan> {
    use mpfree_synth::strings_params::*;
    let regs = [VIOLIN, VIOLA, CELLO, CONTRABASS, HORN, TRUMPET];
    Some(match page {
        "Registers" => (
            "row(regs:62*, col(tone:*, octave:150):38*)",
            vec![
                s("regs", "Registers", Source, &regs)
                    .faders(&regs)
                    .note("any mix sounds at once"),
                s("tone", "The tone", Tone, &[]).draw("Tone").link("Tone"),
                s("octave", "Octave", Voice, &[OCTAVE]),
            ],
        ),
        "Tone" => (
            "row(tone:*, swell:*)",
            vec![
                s("tone", "Tone", Tone, &[BRIGHTNESS, BODY, NOISE])
                    .hero(&[BRIGHTNESS])
                    .draw("Tone"),
                s(
                    "swell",
                    "Swell",
                    Shape,
                    &[CRESCENDO, SUSTAIN, SWELL, VELOCITY],
                )
                .labels(&[(SWELL, "Envelope")])
                .draw("Swell")
                .note("how a held key comes in and dies away"),
            ],
        ),
        "Ensemble" => (
            "row(ens:*, sweeps:*)",
            vec![
                s("ens", "Ensemble", Motion, &[ENSEMBLE, LINES, WIDTH])
                    .hero(&[ENSEMBLE])
                    .labels(&[(ENSEMBLE, "Amount")])
                    .draw("Ensemble"),
                s("sweeps", "Sweeps", Motion, &[SLOW_RATE, FAST_RATE, DEPTH])
                    .note("the lines' slow and fast wobble"),
            ],
        ),
        "Voice" => (
            "row(tuning:*, output:*)",
            vec![
                s("tuning", "Tuning", Voice, &[TRANSPOSE, FINE, BEND_RANGE]),
                s("output", "Output", Voice, &[LEVEL, METER_KEYS]),
            ],
        ),
        _ => return None,
    })
}

fn choir(page: &str) -> Option<Plan> {
    use mpfree_synth::choir_params::*;
    Some(match page {
        "Choir" => (
            "row(vowel:*, col(singers:*, move:190):*)",
            vec![
                s("vowel", "Vowel", Tone, &[VOWEL, BRIGHTNESS])
                    .hero(&[VOWEL])
                    .draw("Vowel"),
                s(
                    "singers",
                    "Singers",
                    Source,
                    &[SECTION, SINGERS, DETUNE, BREATH, OCTAVE],
                )
                .note("how many sing each note, how far apart"),
                s(
                    "move",
                    "Vowel move",
                    Motion,
                    &[START, START_TIME, MORPH, MORPH_RATE],
                )
                .note("a note opens from another vowel, or the vowel swings"),
            ],
        ),
        "Shape" => {
            let a = [ATTACK, DECAY, SUSTAIN, RELEASE];
            (
                "row(swell:*, vib:*)",
                vec![
                    s(
                        "swell",
                        "Swell",
                        Shape,
                        &[ATTACK, DECAY, SUSTAIN, RELEASE, VELOCITY, VEL_TONE],
                    )
                    .faders(&a)
                    .labels(&adsr_labels(a))
                    .labels(&[
                        (VELOCITY, "Velocity to level"),
                        (VEL_TONE, "Velocity to tone"),
                    ])
                    .draw("Swell"),
                    s(
                        "vib",
                        "Vibrato",
                        Motion,
                        &[VIBRATO, VIB_RATE, VIB_DELAY, DRIFT],
                    )
                    .labels(&[(VIBRATO, "Depth"), (VIB_RATE, "Rate"), (VIB_DELAY, "Delay")])
                    .draw("Singers")
                    .note("each singer their own"),
                ],
            )
        }
        "Ensemble" => (
            "row(ens:*, sweeps:*)",
            vec![
                s("ens", "Ensemble", Motion, &[ENSEMBLE, LINES, WIDTH])
                    .hero(&[ENSEMBLE])
                    .labels(&[(ENSEMBLE, "Amount")])
                    .draw("Ensemble"),
                s("sweeps", "Sweeps", Motion, &[SLOW_RATE, FAST_RATE, DEPTH])
                    .note("the lines' slow and fast wobble"),
            ],
        ),
        "Play" => (
            "col(row(notes:*, tuning:*):*, row(pressure:*, output:*):*)",
            vec![
                s("notes", "Notes", Voice, &[MODE, VOICES, GLIDE])
                    .labels(&[(GLIDE, "Glide")])
                    .note("Mono and Legato sing one note"),
                s("tuning", "Tuning", Voice, &[TRANSPOSE, FINE, BEND_RANGE]),
                s("pressure", "Pressure", Motion, &[PRESS_SWELL, PRESS_VOWEL])
                    .labels(&[(PRESS_SWELL, "To swell"), (PRESS_VOWEL, "To vowel")]),
                s("output", "Output", Voice, &[LEVEL, METER_NOTES]),
            ],
        ),
        _ => return None,
    })
}

fn bass(page: &str) -> Option<Plan> {
    use mpfree_synth::bass_params::*;
    Some(match page {
        "Source" => (
            "row(body:*, col(mix:*, pitch:220):*)",
            vec![
                s("body", "Body", Source, &[BODY, WAVE, PW, DETUNE])
                    .labels(&[(DETUNE, "Reese detune")])
                    .draw("Wave"),
                s(
                    "mix",
                    "Under the note",
                    Source,
                    &[SUB, SUB_MODE, FIFTH, NOISE],
                )
                .faders(&[SUB, FIFTH, NOISE])
                .note("locked to the oscillator"),
                s("pitch", "Octave and start", Voice, &[OCTAVE, START]),
            ],
        ),
        "Filter" => (
            "row(filter:520, col(fenv:*, feel:*):*)",
            vec![
                s("filter", "Filter", Tone, &[CUTOFF, RESONANCE])
                    .hero(&[CUTOFF, RESONANCE])
                    .draw("Filter"),
                s(
                    "fenv",
                    "Filter envelope",
                    Shape,
                    &[ENV_AMOUNT, FILTER_DECAY],
                )
                .labels(&[(ENV_AMOUNT, "Amount"), (FILTER_DECAY, "Decay")])
                .note("each note opens it, then it falls"),
                s("feel", "Key and drive", Tone, &[KEY_TRACK, FILTER_DRIVE])
                    .labels(&[(FILTER_DRIVE, "Drive in")])
                    .note("grit, not volume"),
            ],
        ),
        "Shape" => {
            let a = [ATTACK, DECAY, SUSTAIN, RELEASE];
            (
                "row(pitch:*, amp:*)",
                vec![
                    s("pitch", "Pitch drop", Shape, &[DROP, DROP_TIME, PUNCH])
                        .hero(&[DROP])
                        .draw("Drop")
                        .note("where each note starts"),
                    s(
                        "amp",
                        "Amp",
                        Shape,
                        &[ATTACK, DECAY, SUSTAIN, RELEASE, VELOCITY, VEL_TONE],
                    )
                    .faders(&a)
                    .labels(&adsr_labels(a))
                    .labels(&[
                        (VELOCITY, "Velocity to level"),
                        (VEL_TONE, "Velocity to tone"),
                    ]),
                ],
            )
        }
        "Play" => (
            "col(row(glide:*, keys:*):*, row(tuning:*, pressure:*):*)",
            vec![
                s("glide", "Glide", Motion, &[GLIDE, GLIDE_MODE, GLIDE_ON]).labels(&[
                    (GLIDE, "Time"),
                    (GLIDE_MODE, "Mode"),
                    (GLIDE_ON, "When"),
                ]),
                s("keys", "Keys", Voice, &[PRIORITY, RETRIGGER]).note("one note at a time"),
                s("tuning", "Tuning", Voice, &[TRANSPOSE, FINE, BEND_RANGE]),
                s("pressure", "Pressure", Motion, &[PRESS_TO, PRESSURE])
                    .labels(&[(PRESS_TO, "Moves"), (PRESSURE, "Amount")]),
            ],
        ),
        "Output" => (
            "row(drive:*, out:*)",
            vec![
                s("drive", "Drive", Tone, &[DRIVE, DRIVE_MODE])
                    .hero(&[DRIVE])
                    .labels(&[(DRIVE_MODE, "Mode")])
                    .draw("Drive")
                    .note("heard on small speakers"),
                s("out", "Tone and level", Voice, &[TONE, HIGH_PASS, LEVEL]),
            ],
        ),
        _ => return None,
    })
}

fn wind(page: &str) -> Option<Plan> {
    use mpfree_synth::wind_params::*;
    Some(match page {
        "Instrument" => (
            "row(horn:*, col(colour:*, vib:*):*)",
            vec![
                s("horn", "Instrument", Source, &[MODEL, BREATH, REED, EDGE])
                    .hero(&[BREATH])
                    .draw("Mouthpiece")
                    .note("how hard the player blows"),
                s("colour", "Tone", Tone, &[TONE, NOISE, GROWL])
                    .hero(&[TONE])
                    .draw("Tone"),
                s("vib", "Vibrato", Motion, &[VIB_DEPTH, VIB_RATE, VIB_DELAY])
                    .labels(&[(VIB_DEPTH, "Depth")])
                    .note("the mod wheel adds more"),
            ],
        ),
        "Breath" => {
            let a = [ATTACK, DECAY, SUSTAIN, RELEASE];
            (
                "row(source:*, shape:*)",
                vec![
                    s(
                        "source",
                        "Breath",
                        Source,
                        &[BREATH_FROM, VELOCITY, TONGUE, METER_BREATH],
                    )
                    .labels(&[(BREATH_FROM, "From"), (VELOCITY, "Soft to hard")])
                    .draw("Level"),
                    s("shape", "Over a note", Shape, &a)
                        .faders(&a)
                        .labels(&adsr_labels(a))
                        .draw("Breath")
                        .note("the breath, held and let go"),
                ],
            )
        }
        "Play" => (
            "col(row(keys:*, glide:*):*, row(tuning:*, out:*):*)",
            vec![
                s("keys", "Keys", Voice, &[VOICES, LEGATO, PRIORITY])
                    .labels(&[(LEGATO, "Over a held key")]),
                s("glide", "Glide", Motion, &[GLIDE, GLIDE_MODE])
                    .labels(&[(GLIDE, "Time"), (GLIDE_MODE, "Mode")]),
                s("tuning", "Tuning", Voice, &[TRANSPOSE, FINE, BEND_RANGE]),
                s("out", "Output", Voice, &[LEVEL, SPREAD]),
            ],
        ),
        "Pressure" => pressure(PRESS_BASE),
        _ => return None,
    })
}
