//! Which parameters go on which page of the Synth tab.
//!
//! Any node gets pages from its [`ParamInfo`] groups: consecutive groups are
//! packed onto one page while they fit, and the page takes the first group's
//! name. That is what a hosted plugin or an effect will show. A native
//! engine can name its own pages ([`plan`] knows Poly's), so related groups
//! sit together the way a player expects (the filter with its envelope).

use mpfree_node::{NodeInfo, ParamInfo};

/// The most tiles one page shows (four rows of four).
pub const MAX_TILES: usize = 16;
/// Generic pages stop adding groups at this many tiles, so most pages keep
/// big tiles (three rows of four).
pub const SOFT_TILES: usize = 12;
/// The group of a node's macros (a sound's, `mpfree_instruments::sound`,
/// or a plugin's of that name): they get the first page, and Q-Links 1 to 4
/// turn the first four.
pub const MACROS: &str = "Macros";

#[derive(Clone, Debug, PartialEq)]
pub struct Page {
    pub name: String,
    /// Indexes into the node's parameter list, in page order.
    pub params: Vec<usize>,
    /// What Q-Links 1 to 4 turn, if the engine chose (indexes, on this page).
    pub qlinks: Vec<usize>,
}

/// A native engine's own page: its name, the groups on it, and the
/// parameters its Q-Links turn (by id; empty for the first four knobs).
struct PlanPage {
    name: &'static str,
    groups: &'static [&'static str],
    /// Parameters of other groups, after the groups' own.
    params: &'static [u16],
    qlinks: &'static [u16],
}

const fn pg(
    name: &'static str,
    groups: &'static [&'static str],
    qlinks: &'static [u16],
) -> PlanPage {
    pg_with(name, groups, &[], qlinks)
}

/// A page that also takes some parameters of other groups. Their groups
/// stay as they are, since a saved preset names each parameter by its
/// group.
const fn pg_with(
    name: &'static str,
    groups: &'static [&'static str],
    params: &'static [u16],
    qlinks: &'static [u16],
) -> PlanPage {
    PlanPage {
        name,
        groups,
        params,
        qlinks,
    }
}

use mpfree_synth::params as pp;

const POLY: &[PlanPage] = &[
    // Osc: the detune, the second oscillator's level, the sub and the
    // supersaw's detune are what players reach for; octaves and waves are
    // set once.
    pg(
        "Osc",
        &["Osc 1", "Osc 2", "Mix", "Supersaw"],
        &[
            pp::OSC2_FINE,
            pp::OSC2_LEVEL,
            pp::SUB_LEVEL,
            pp::SUPER_DETUNE,
        ],
    ),
    // Table: each oscillator's Table wave (its table, warp, position and
    // warp amount) and what moves the positions and warps. The positions
    // and warps are what players turn.
    pg_with(
        "Table",
        &[],
        &[
            pp::OSC1_TABLE,
            pp::OSC1_WARP_MODE,
            pp::OSC1_POS,
            pp::OSC1_WARP,
            pp::OSC2_TABLE,
            pp::OSC2_WARP_MODE,
            pp::OSC2_POS,
            pp::OSC2_WARP,
            pp::FENV_POS1,
            pp::FENV_POS2,
            pp::FENV_WARP,
            pp::VEL_POS,
            pp::LFO_POS1,
            pp::LFO_POS2,
        ],
        &[pp::OSC1_POS, pp::OSC1_WARP, pp::OSC2_POS, pp::OSC2_WARP],
    ),
    pg("Filter", &["Filter", "Filter env"], &[]),
    pg("Env", &["Amp env"], &[]),
    pg("LFO", &["LFO"], &[]),
    // LFO 2, each note's own: its rate, its fade-in and its two main sends.
    pg(
        "LFO 2",
        &["LFO 2"],
        &[
            pp::LFO2_RATE,
            pp::LFO2_FADE,
            pp::LFO2_PITCH,
            pp::LFO2_CUTOFF,
        ],
    ),
    // Voice: unison moved here when the supersaw's knobs filled the Osc
    // page (16 a page, one per Q-Link). Age beside Glide, as on Analog's.
    pg(
        "Voice",
        &["Voice", "Unison", "Output"],
        &[pp::GLIDE, pp::AGE, pp::UNISON_DETUNE, pp::LEVEL],
    ),
    // Routes: what the mod wheel, breath, foot, velocity and the rest move.
    // The depths are what a player tunes.
    pg(
        "Routes",
        &["Routes"],
        &[
            pp::route_id(0, pp::route::DEPTH),
            pp::route_id(1, pp::route::DEPTH),
            pp::route_id(2, pp::route::DEPTH),
            pp::route_id(3, pp::route::DEPTH),
        ],
    ),
    // Pressure: the threshold, where it's full, and the first two slots'
    // depths are what a player tunes while playing.
    pg(
        "Pressure",
        &["Pressure"],
        &[
            pp::PRESS_THRESHOLD,
            pp::PRESS_FULL_AT,
            pp::press_depth(0),
            pp::press_depth(1),
        ],
    ),
];

use mpfree_synth::fm_params::{self as fp, op, op_id};

/// FM: the algorithm and voice with the six operators' On switches (as
/// the DX7's operator buttons), the modulation (pitch envelope and LFO), a
/// page per operator, and the operators' keyboard scaling two to a page.
const FM: &[PlanPage] = &[
    pg_with(
        "Algo",
        &["Algo", "Voice", "Output"],
        &FM_ONS,
        &[fp::ALGORITHM, fp::FEEDBACK, fp::TRANSPOSE, fp::LEVEL],
    ),
    pg(
        "Mod",
        &["LFO", "Pitch env"],
        &[fp::LFO_SPEED, fp::LFO_PMD, fp::LFO_AMD, fp::LFO_DELAY],
    ),
    pg(
        "Pressure",
        &["Pressure"],
        &[
            fp::PRESS_THRESHOLD,
            fp::PRESS_FULL_AT,
            fp::press_depth(0),
            fp::press_depth(1),
        ],
    ),
    pg("Op 1", &["Op 1"], &FM_OP_Q[0]),
    pg("Op 2", &["Op 2"], &FM_OP_Q[1]),
    pg("Op 3", &["Op 3"], &FM_OP_Q[2]),
    pg("Op 4", &["Op 4"], &FM_OP_Q[3]),
    pg("Op 5", &["Op 5"], &FM_OP_Q[4]),
    pg("Op 6", &["Op 6"], &FM_OP_Q[5]),
    pg("Kbd 1-2", &["Op 1 kbd", "Op 2 kbd"], &[]),
    pg("Kbd 3-4", &["Op 3 kbd", "Op 4 kbd"], &[]),
    pg("Kbd 5-6", &["Op 5 kbd", "Op 6 kbd"], &[]),
];

/// The six operators' On switches, on the Algo page.
const FM_ONS: [u16; 6] = {
    let mut q = [0u16; 6];
    let mut n = 0;
    while n < 6 {
        q[n] = op_id(n + 1, op::ON);
        n += 1;
    }
    q
};

/// An operator page's Q-Links: output level, coarse, fine and velocity.
const FM_OP_Q: [[u16; 4]; 6] = {
    let mut q = [[0u16; 4]; 6];
    let mut n = 0;
    while n < 6 {
        q[n] = [
            op_id(n + 1, op::LEVEL),
            op_id(n + 1, op::COARSE),
            op_id(n + 1, op::FINE),
            op_id(n + 1, op::VEL_SENS),
        ];
        n += 1;
    }
    q
};

use mpfree_synth::wavetable::params::{self as wp, osc_id};

/// Wavetable: a page per oscillator (Q-Links on table position, warp,
/// fine and level, the knobs played live), then the mix, filter, mod
/// envelope, LFO, amp envelope, voice and pressure.
const WT: &[PlanPage] = &[
    pg(
        "Osc A",
        &["Osc A"],
        &[
            osc_id(0, wp::POSITION),
            osc_id(0, wp::WARP),
            osc_id(0, wp::FINE),
            osc_id(0, wp::OSC_LEVEL),
        ],
    ),
    pg(
        "Osc B",
        &["Osc B"],
        &[
            osc_id(1, wp::POSITION),
            osc_id(1, wp::WARP),
            osc_id(1, wp::FINE),
            osc_id(1, wp::OSC_LEVEL),
        ],
    ),
    pg(
        "Mix",
        &["Mix", "Unison"],
        &[
            wp::SUB_LEVEL,
            wp::NOISE_LEVEL,
            wp::UNISON_DETUNE,
            wp::UNISON_SPREAD,
        ],
    ),
    pg("Filter", &["Filter"], &[]),
    pg(
        "Mod",
        &["Mod env"],
        &[wp::MENV_POS_A, wp::MENV_WARP, wp::MENV_D, wp::VEL_POS],
    ),
    pg(
        "LFO",
        &["LFO"],
        &[wp::LFO_RATE, wp::LFO_POS_A, wp::LFO_POS_B, wp::LFO_PITCH],
    ),
    pg("Env", &["Amp env"], &[]),
    pg(
        "Voice",
        &["Voice", "Output"],
        &[wp::GLIDE, wp::BEND_RANGE, wp::LEVEL, wp::PAN],
    ),
    pg(
        "Pressure",
        &["Pressure"],
        &[
            wp::PRESS_THRESHOLD,
            wp::PRESS_FULL_AT,
            wp::press_depth(0),
            wp::press_depth(1),
        ],
    ),
];

use mpfree_synth::drum_params::{field as df, pad_id, BASE_NOTE, LEVEL as DRUM_LEVEL};

/// The drum synth's Q-Links on pad `n`'s page: tune, decay, tone, snap.
const fn drum_q(n: usize) -> [u16; 4] {
    [
        pad_id(n, df::TUNE),
        pad_id(n, df::DECAY),
        pad_id(n, df::TONE),
        pad_id(n, df::SNAP),
    ]
}
const DRUM_Q: [[u16; 4]; 16] = {
    let mut q = [[0u16; 4]; 16];
    let mut n = 0;
    while n < 16 {
        q[n] = drum_q(n);
        n += 1;
    }
    q
};

use mpfree_synth::drum_fx::{field as xf, fx_id, kit as xk};

/// A drum's effects page's Q-Links: Punch, Distortion and its two sends.
const fn drum_fx_q(n: usize) -> [u16; 4] {
    [
        fx_id(n, xf::PUNCH),
        fx_id(n, xf::DISTORTION),
        fx_id(n, xf::DELAY_SEND),
        fx_id(n, xf::REVERB_SEND),
    ]
}
const DRUM_FX_Q: [[u16; 4]; 16] = {
    let mut q = [[0u16; 4]; 16];
    let mut n = 0;
    while n < 16 {
        q[n] = drum_fx_q(n);
        n += 1;
    }
    q
};

/// Drums: a page per pad (its 14 knobs), the kit, then each pad's effects
/// (shown under one tab, [`DRUM_FX_TAB`], for the pad shown last) and the
/// kit's sends.
const DRUMS: &[PlanPage] = &[
    pg("Pad 1", &["Pad 1"], &DRUM_Q[0]),
    pg("Pad 2", &["Pad 2"], &DRUM_Q[1]),
    pg("Pad 3", &["Pad 3"], &DRUM_Q[2]),
    pg("Pad 4", &["Pad 4"], &DRUM_Q[3]),
    pg("Pad 5", &["Pad 5"], &DRUM_Q[4]),
    pg("Pad 6", &["Pad 6"], &DRUM_Q[5]),
    pg("Pad 7", &["Pad 7"], &DRUM_Q[6]),
    pg("Pad 8", &["Pad 8"], &DRUM_Q[7]),
    pg("Pad 9", &["Pad 9"], &DRUM_Q[8]),
    pg("Pad 10", &["Pad 10"], &DRUM_Q[9]),
    pg("Pad 11", &["Pad 11"], &DRUM_Q[10]),
    pg("Pad 12", &["Pad 12"], &DRUM_Q[11]),
    pg("Pad 13", &["Pad 13"], &DRUM_Q[12]),
    pg("Pad 14", &["Pad 14"], &DRUM_Q[13]),
    pg("Pad 15", &["Pad 15"], &DRUM_Q[14]),
    pg("Pad 16", &["Pad 16"], &DRUM_Q[15]),
    pg("Kit", &["Kit"], &[DRUM_LEVEL, BASE_NOTE]),
    pg("Pad 1 effects", &["Pad 1 effects"], &DRUM_FX_Q[0]),
    pg("Pad 2 effects", &["Pad 2 effects"], &DRUM_FX_Q[1]),
    pg("Pad 3 effects", &["Pad 3 effects"], &DRUM_FX_Q[2]),
    pg("Pad 4 effects", &["Pad 4 effects"], &DRUM_FX_Q[3]),
    pg("Pad 5 effects", &["Pad 5 effects"], &DRUM_FX_Q[4]),
    pg("Pad 6 effects", &["Pad 6 effects"], &DRUM_FX_Q[5]),
    pg("Pad 7 effects", &["Pad 7 effects"], &DRUM_FX_Q[6]),
    pg("Pad 8 effects", &["Pad 8 effects"], &DRUM_FX_Q[7]),
    pg("Pad 9 effects", &["Pad 9 effects"], &DRUM_FX_Q[8]),
    pg("Pad 10 effects", &["Pad 10 effects"], &DRUM_FX_Q[9]),
    pg("Pad 11 effects", &["Pad 11 effects"], &DRUM_FX_Q[10]),
    pg("Pad 12 effects", &["Pad 12 effects"], &DRUM_FX_Q[11]),
    pg("Pad 13 effects", &["Pad 13 effects"], &DRUM_FX_Q[12]),
    pg("Pad 14 effects", &["Pad 14 effects"], &DRUM_FX_Q[13]),
    pg("Pad 15 effects", &["Pad 15 effects"], &DRUM_FX_Q[14]),
    pg("Pad 16 effects", &["Pad 16 effects"], &DRUM_FX_Q[15]),
    pg(
        "Sends",
        &["Sends"],
        &[
            xk::DELAY_FEEDBACK,
            xk::DELAY_LEVEL,
            xk::REVERB_DECAY,
            xk::REVERB_LEVEL,
        ],
    ),
];

/// The page that holds pad `pad`'s knobs (0 to 15), so an app can follow
/// the last pad hit.
pub fn drum_page(pad: usize) -> usize {
    pad.min(15)
}

/// Drums' Kit page, after the pads' own.
pub const DRUM_KIT_PAGE: usize = 16;

/// The page of pad `pad`'s effects (0 to 15), after the Kit page.
pub fn drum_fx_page(pad: usize) -> usize {
    DRUM_KIT_PAGE + 1 + pad.min(15)
}

/// The tab every drum's effects page shows under: one tab, not sixteen,
/// that shows the effects of the pad shown last.
pub const DRUM_FX_TAB: &str = "Effects";

/// The page to show when pad `pad` is hit while Drums' page `shown` is on
/// screen: that pad's effects while a pad's effects show, else its own
/// page (so the Synth tab edits the pad hit last).
pub fn drum_follow(shown: usize, pad: usize) -> usize {
    if (drum_fx_page(0)..=drum_fx_page(15)).contains(&shown) {
        drum_fx_page(pad)
    } else {
        drum_page(pad)
    }
}

/// Pages that share one tab: for a page called `name`, the tab's name and
/// the pad it is for (a drum's effects page: "Pad 3 effects" shows under
/// "Effects", for pad 2 counting from 0). None for a page with its own tab.
pub fn shared_tab(name: &str) -> Option<(&'static str, usize)> {
    let n: usize = name
        .strip_prefix("Pad ")?
        .strip_suffix(" effects")?
        .parse()
        .ok()?;
    (1..=16).contains(&n).then_some((DRUM_FX_TAB, n - 1))
}

/// The pad a page is about (0 to 15): a drum's own page or its effects.
pub fn page_pad(name: &str) -> Option<usize> {
    if let Some((_, pad)) = shared_tab(name) {
        return Some(pad);
    }
    let n: usize = name.strip_prefix("Pad ")?.parse().ok()?;
    (1..=16).contains(&n).then(|| n - 1)
}

use mpfree_synth::pluck_params as kp;

/// Pluck: the string (with the model, decay and release), the mallet, and
/// the voice.
const PLUCK: &[PlanPage] = &[
    pg(
        "String",
        &["Model", "String"],
        &[kp::DECAY, kp::BRIGHTNESS, kp::PICK, kp::POSITION],
    ),
    pg(
        "Mallet",
        &["Mallet"],
        &[kp::HARDNESS, kp::STRIKE, kp::STRETCH, kp::HIGH_DAMP],
    ),
    pg(
        "Voice",
        &["Voice", "Output"],
        &[kp::VEL_TONE, kp::BEND_RANGE, kp::LEVEL, kp::SPREAD],
    ),
];

use mpfree_synth::keys_params as ek;

/// Keys: the model and the strike, the pickup's tone, the clav's own
/// switches, the amp (tremolo, drive, key noise), and the voice.
const KEYS: &[PlanPage] = &[
    pg_with(
        "Strike",
        &["Model"],
        &[ek::HARDNESS, ek::BELL],
        &[ek::HARDNESS, ek::BELL, ek::DECAY, ek::RELEASE],
    ),
    pg(
        "Tone",
        &["Tone"],
        &[ek::VOICING, ek::BARK, ek::TONE, ek::VEL_TONE],
    ),
    pg(
        "Clav",
        &["Clav"],
        &[ek::PICKUP, ek::CLAV_TONE, ek::MUTE, ek::DROP],
    ),
    pg(
        "Amp",
        &["Amp"],
        &[ek::TREMOLO, ek::TREM_RATE, ek::DRIVE, ek::KEY_NOISE],
    ),
    pg(
        "Voice",
        &["Voice", "Output"],
        &[ek::TRANSPOSE, ek::BEND_RANGE, ek::LEVEL, ek::PAN],
    ),
];

use mpfree_synth::drawbar_params as db;

/// Drawbar: the model, the drawbars and the combo's voices, percussion
/// with the scanner, the rotary speaker with the tone, and the voice. The
/// Q-Links pull the upper drawbars (the 16', 5 1/3' and 8' usually stay
/// out); the switches stay on the screen.
const DRAWBAR: &[PlanPage] = &[
    pg(
        "Drawbars",
        &["Model", "Drawbars", "Combo"],
        &[db::DB_4, db::DB_2_23, db::DB_2, db::DB_1],
    ),
    pg("Percussion", &["Percussion", "Vibrato"], &[]),
    pg(
        "Rotary",
        &["Rotary", "Tone"],
        &[db::DRIVE, db::TONE, db::CLICK, db::SPREAD],
    ),
    pg(
        "Voice",
        &["Voice", "Output"],
        &[db::TRANSPOSE, db::FINE, db::LEVEL, db::PAN],
    ),
];

use mpfree_synth::squelch_params as sq;

/// Squelch: the original's six controls, then how it is played (slide,
/// accent, the envelopes' times, key tracking and drive), then the voice.
const SQUELCH: &[PlanPage] = &[
    pg(
        "Synth",
        &["Synth"],
        &[sq::CUTOFF, sq::RESONANCE, sq::ENV_MOD, sq::DECAY],
    ),
    pg(
        "Play",
        &["Play"],
        &[sq::SLIDE, sq::ACCENT_DECAY, sq::AMP_DECAY, sq::DRIVE],
    ),
    pg(
        "Voice",
        &["Voice", "Output"],
        &[sq::TRANSPOSE, sq::FINE, sq::LEVEL, sq::PAN],
    ),
];

use mpfree_synth::analog_params as an;

/// Analog: the oscillators and their mix, the filter, both envelopes, the
/// LFO and poly-mod, then the voice (Age and the chorus) with the output.
/// The Q-Links take what players turn while a chord holds: the pulse
/// widths and the detune, the filter's four knobs, the envelopes' times,
/// the vibrato and poly-mod depths, and how worn the cards are.
const ANALOG: &[PlanPage] = &[
    pg(
        "Osc",
        &["Osc"],
        &[an::OSC1_PW, an::OSC2_PW, an::OSC2_FINE, an::OSC2_LEVEL],
    ),
    pg(
        "Filter",
        &["Filter"],
        &[an::CUTOFF, an::RESONANCE, an::ENV_AMT, an::DRIVE],
    ),
    pg(
        "Envelopes",
        &["Envelopes"],
        &[an::F_ATTACK, an::F_DECAY, an::A_ATTACK, an::A_RELEASE],
    ),
    pg(
        "Mod",
        &["Mod"],
        &[an::LFO_RATE, an::LFO_PITCH, an::PM_ENV, an::PM_OSC2],
    ),
    pg(
        "Voice",
        &["Voice", "Output"],
        &[an::AGE, an::GLIDE, an::UNI_DETUNE, an::LEVEL],
    ),
];

use mpfree_synth::strings_params as st;

/// Strings: the registers (their levels on the Q-Links, as the old
/// machines' sliders), the tone with the swell, the ensemble, then the
/// voice.
const STRINGS: &[PlanPage] = &[
    pg(
        "Registers",
        &["Registers"],
        &[st::VIOLIN, st::VIOLA, st::CELLO, st::CONTRABASS],
    ),
    pg(
        "Tone",
        &["Tone", "Swell"],
        &[st::BRIGHTNESS, st::CRESCENDO, st::SUSTAIN, st::NOISE],
    ),
    pg(
        "Ensemble",
        &["Ensemble"],
        &[st::ENSEMBLE, st::DEPTH, st::SLOW_RATE, st::WIDTH],
    ),
    pg(
        "Voice",
        &["Voice", "Output"],
        &[st::TRANSPOSE, st::FINE, st::BEND_RANGE, st::LEVEL],
    ),
];

use mpfree_synth::bass_params as bp;

/// Bass: the body and its oscillators, the filter, the pitch drop with
/// the amp, the playing (glide, keys, tuning, pressure), then the output
/// (drive and tone). The Q-Links take what players ride under a line: the
/// sub, the fifth, the reese's detune and the pulse width; the filter's
/// four knobs; the drop and the decays; the glide and the bend; the drive
/// and the tone.
const BASS: &[PlanPage] = &[
    pg(
        "Source",
        &["Body", "Source"],
        &[bp::SUB, bp::FIFTH, bp::DETUNE, bp::PW],
    ),
    pg(
        "Filter",
        &["Filter"],
        &[bp::CUTOFF, bp::RESONANCE, bp::ENV_AMOUNT, bp::FILTER_DECAY],
    ),
    pg(
        "Shape",
        &["Pitch", "Amp"],
        &[bp::DROP, bp::DROP_TIME, bp::DECAY, bp::RELEASE],
    ),
    pg(
        "Play",
        &["Play", "Pressure"],
        &[bp::GLIDE, bp::FINE, bp::BEND_RANGE, bp::PRESSURE],
    ),
    pg(
        "Output",
        &["Output"],
        &[bp::DRIVE, bp::TONE, bp::HIGH_PASS, bp::LEVEL],
    ),
];

use mpfree_node::pressure::field as pf;
use mpfree_synth::wind_params as wn;

/// Wind: the instrument with its vibrato, the breath (where it comes from,
/// the tongue and its shape over a note), the playing (voices, glide,
/// tuning, level) and the pressure. The Q-Links take what a player rides
/// on a held note: the breath, the vibrato, the tone and the air noise;
/// the tongue and the breath's shape; the glide, the bend, the level and
/// the spread; and the pressure's feel and first two sends.
const WIND: &[PlanPage] = &[
    pg(
        "Instrument",
        &["Instrument", "Vibrato"],
        &[wn::BREATH, wn::VIB_DEPTH, wn::TONE, wn::NOISE],
    ),
    pg(
        "Breath",
        &["Breath"],
        &[wn::TONGUE, wn::ATTACK, wn::SUSTAIN, wn::RELEASE],
    ),
    pg(
        "Play",
        &["Play"],
        &[wn::GLIDE, wn::BEND_RANGE, wn::LEVEL, wn::SPREAD],
    ),
    pg(
        "Pressure",
        &["Pressure"],
        &[
            wn::press(pf::THRESHOLD),
            wn::press(pf::FULL_AT),
            wn::press_depth(0),
            wn::press_depth(1),
        ],
    ),
];

use mpfree_synth::choir_params as ch;

/// Choir: the singers and the vowel (with how it moves), the swell with
/// the vibrato, the ensemble, then the playing. The Q-Links take what
/// players ride: the vowel, how many singers and how far apart, the
/// brightness; the swell's ends, the vibrato and drift; the ensemble's
/// amount, depth, sweep and width; the glide, pressure and level.
const CHOIR: &[PlanPage] = &[
    pg(
        "Choir",
        &["Choir", "Vowel move"],
        &[ch::VOWEL, ch::SINGERS, ch::DETUNE, ch::BRIGHTNESS],
    ),
    pg(
        "Shape",
        &["Swell", "Vibrato"],
        &[ch::ATTACK, ch::RELEASE, ch::VIBRATO, ch::DRIFT],
    ),
    pg(
        "Ensemble",
        &["Ensemble"],
        &[ch::ENSEMBLE, ch::DEPTH, ch::SLOW_RATE, ch::WIDTH],
    ),
    pg(
        "Play",
        &["Voice", "Pressure", "Output"],
        &[ch::GLIDE, ch::PRESS_SWELL, ch::PRESS_VOWEL, ch::LEVEL],
    ),
];

fn known(id: &str) -> Option<&'static [PlanPage]> {
    match id {
        "mpfree.poly" => Some(POLY),
        "mpfree.fm" => Some(FM),
        "mpfree.wavetable" => Some(WT),
        "mpfree.drums" => Some(DRUMS),
        "mpfree.pluck" => Some(PLUCK),
        "mpfree.keys" => Some(KEYS),
        "mpfree.drawbar" => Some(DRAWBAR),
        "mpfree.squelch" => Some(SQUELCH),
        "mpfree.analog" => Some(ANALOG),
        "mpfree.strings" => Some(STRINGS),
        "mpfree.bass" => Some(BASS),
        "mpfree.wind" => Some(WIND),
        "mpfree.choir" => Some(CHOIR),
        _ => None,
    }
}

/// The pages for a node. Every parameter is on exactly one page. A node's
/// macros (group [`MACROS`]) come first, then the engine's own plan; a
/// group the plan doesn't name goes on generic pages after it.
pub fn plan(info: &NodeInfo, params: &[ParamInfo]) -> Vec<Page> {
    let mut placed = vec![false; params.len()];
    let mut pages = Vec::new();
    let macros: Vec<usize> = (0..params.len())
        .filter(|&i| params[i].group == MACROS)
        .collect();
    for (k, chunk) in macros.chunks(MAX_TILES).enumerate() {
        for &i in chunk {
            placed[i] = true;
        }
        pages.push(Page {
            name: if k == 0 {
                MACROS.to_string()
            } else {
                format!("{MACROS} {}", k + 1)
            },
            params: chunk.to_vec(),
            qlinks: if k == 0 {
                chunk.iter().copied().take(4).collect()
            } else {
                Vec::new()
            },
        });
    }
    if let Some(plan) = known(info.id) {
        // A setting a page names itself is that page's, even when an
        // earlier page takes its group (Poly's Table page takes the Table
        // settings out of the Osc groups).
        let named: Vec<Option<usize>> = (0..params.len())
            .map(|i| plan.iter().position(|pp| pp.params.contains(&params[i].id)))
            .collect();
        for (
            k,
            PlanPage {
                name,
                groups,
                params: more,
                qlinks,
            },
        ) in plan.iter().enumerate()
        {
            let mut idx: Vec<usize> = Vec::new();
            let named = &named;
            let own = groups.iter().flat_map(|g| {
                (0..params.len())
                    .filter(move |&i| params[i].group == *g && named[i].is_none_or(|n| n == k))
            });
            let other = more
                .iter()
                .filter_map(|id| params.iter().position(|p| p.id == *id));
            for i in own.chain(other) {
                if !placed[i] && !idx.contains(&i) {
                    idx.push(i);
                }
            }
            for &i in &idx {
                placed[i] = true;
            }
            for (k, chunk) in idx.chunks(MAX_TILES).enumerate() {
                let name = if k == 0 {
                    name.to_string()
                } else {
                    format!("{name} {}", k + 1)
                };
                let q = qlinks
                    .iter()
                    .filter_map(|id| chunk.iter().copied().find(|&i| params[i].id == *id))
                    .take(4)
                    .collect();
                pages.push(Page {
                    name,
                    params: chunk.to_vec(),
                    qlinks: q,
                });
            }
        }
    }
    pages.extend(generic(params, &placed));
    pages
}

/// Groups in the order they first appear, packed onto pages.
fn generic(params: &[ParamInfo], placed: &[bool]) -> Vec<Page> {
    let mut groups: Vec<(&str, Vec<usize>)> = Vec::new();
    for (i, p) in params.iter().enumerate() {
        if placed[i] {
            continue;
        }
        match groups.iter_mut().find(|(g, _)| *g == p.group) {
            Some((_, v)) => v.push(i),
            None => groups.push((p.group, vec![i])),
        }
    }
    let mut pages: Vec<Page> = Vec::new();
    for (group, idx) in groups {
        let name = if group.is_empty() { "Main" } else { group };
        let fits = pages
            .last()
            .is_some_and(|p| p.params.len() + idx.len() <= SOFT_TILES);
        if fits {
            pages.last_mut().unwrap().params.extend(idx);
            continue;
        }
        for (k, chunk) in idx.chunks(MAX_TILES).enumerate() {
            let name = if k == 0 {
                name.to_string()
            } else {
                format!("{name} {}", k + 1)
            };
            pages.push(Page {
                name,
                params: chunk.to_vec(),
                qlinks: Vec::new(),
            });
        }
    }
    pages
}

/// A tile's label: the parameter's name, with its group when the group is
/// not the page's own ("Env · Attack" on the Filter page, "Osc 2 · Wave").
pub fn tile_label(page: &str, p: &ParamInfo) -> String {
    if p.group == page || p.group.is_empty() || p.group == p.name {
        return p.name.to_string();
    }
    // "Filter env" on the Filter page is the page's env; "Osc 2" on the Osc
    // page is its own thing and keeps its whole name.
    match p.group.strip_prefix(page).and_then(|r| r.strip_prefix(' ')) {
        Some(rest) if rest.starts_with(|c: char| c.is_alphabetic()) => {
            format!("{} · {}", cap(rest), p.name)
        }
        _ => format!("{} · {}", p.group, p.name),
    }
}

/// What Q-Links 1 to 4 turn on a page: the engine's choice, or else the
/// first four knobs (a choice, a switch or a meter is left to the touch
/// screen), then whatever is left.
pub fn qlink_targets(page: &Page, params: &[ParamInfo]) -> Vec<usize> {
    if !page.qlinks.is_empty() {
        return page.qlinks.clone();
    }
    let knob = |i: &usize| {
        let p = &params[*i];
        !p.readonly
            && !matches!(
                p.curve,
                mpfree_node::Curve::Choice(_) | mpfree_node::Curve::Toggle
            )
    };
    let mut v: Vec<usize> = page.params.iter().copied().filter(knob).take(4).collect();
    for &i in &page.params {
        if v.len() == 4 {
            break;
        }
        if !v.contains(&i) && !params[i].readonly {
            v.push(i);
        }
    }
    v
}

fn cap(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_node::{Curve, Kind, Unit};
    use mpfree_synth::params::PARAMS;

    static POLY_INFO: NodeInfo = NodeInfo {
        id: "mpfree.poly",
        name: "Poly",
        kind: Kind::Instrument,
    };
    static OTHER: NodeInfo = NodeInfo {
        id: "test.other",
        name: "Other",
        kind: Kind::Effect,
    };

    fn every_param_once(pages: &[Page], n: usize) {
        let mut seen = vec![0; n];
        for p in pages {
            assert!(!p.params.is_empty(), "page {} is empty", p.name);
            assert!(
                p.params.len() <= MAX_TILES,
                "page {} has {} tiles",
                p.name,
                p.params.len()
            );
            for &i in &p.params {
                seen[i] += 1;
            }
        }
        assert!(seen.iter().all(|&s| s == 1), "{seen:?}");
    }

    #[test]
    fn poly_has_its_own_pages_and_every_param_once() {
        let pages = plan(&POLY_INFO, PARAMS);
        let names: Vec<&str> = pages.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(
            names,
            ["Osc", "Table", "Filter", "Env", "LFO", "LFO 2", "Voice", "Routes", "Pressure"]
        );
        every_param_once(&pages, PARAMS.len());
        // Pressure: the 8 shaper settings and 4 slots fill one page.
        assert_eq!(pages[8].params.len(), 16);
        let q: Vec<&str> = qlink_targets(&pages[8], PARAMS)
            .iter()
            .map(|&i| PARAMS[i].name)
            .collect();
        assert_eq!(q, ["Threshold", "Full at", "Depth 1", "Depth 2"]);
        // Routes: four routes of three, the Q-Links on their depths.
        assert_eq!(pages[7].params.len(), 12);
        let q: Vec<&str> = qlink_targets(&pages[7], PARAMS)
            .iter()
            .map(|&i| PARAMS[i].name)
            .collect();
        assert_eq!(q, ["Depth 1", "Depth 2", "Depth 3", "Depth 4"]);
        // LFO 2: its rate, fade-in and two sends.
        let q: Vec<&str> = qlink_targets(&pages[5], PARAMS)
            .iter()
            .map(|&i| PARAMS[i].name)
            .collect();
        assert_eq!(q, ["Rate", "Fade in", "To pitch", "To cutoff"]);
        // The Q-Links turn the filter page's first four knobs, not its Type:
        // both kinds of grit side by side (Jay after live test 3).
        let q: Vec<&str> = qlink_targets(&pages[2], PARAMS)
            .iter()
            .map(|&i| PARAMS[i].name)
            .collect();
        assert_eq!(q, ["Cutoff", "Resonance", "Drive", "Distortion"]);
        // A page of choices still gets four.
        let lfo = qlink_targets(&pages[4], PARAMS);
        assert_eq!(lfo.len(), 4);
        assert!(lfo.iter().all(|&i| !PARAMS[i].readonly));
        // The engine's own choice, where it made one.
        let osc: Vec<&str> = qlink_targets(&pages[0], PARAMS)
            .iter()
            .map(|&i| PARAMS[i].name)
            .collect();
        assert_eq!(osc, ["Fine", "Level", "Sub", "Detune"]);
        // Table: both oscillators' tables and what moves them, the Q-Links
        // on the positions and warps.
        assert_eq!(pages[1].params.len(), 14);
        let q: Vec<&str> = qlink_targets(&pages[1], PARAMS)
            .iter()
            .map(|&i| PARAMS[i].name)
            .collect();
        assert_eq!(q, ["Position", "Warp amt", "Position", "Warp amt"]);
        for page in &pages {
            assert!(
                page.qlinks.iter().all(|i| page.params.contains(i)),
                "{}",
                page.name
            );
        }
    }

    #[test]
    fn fm_has_its_own_pages_and_every_param_once() {
        use mpfree_node::Node;
        let fm = mpfree_synth::Fm::new(48_000);
        let params = fm.params();
        let pages = plan(fm.info(), params);
        let names: Vec<&str> = pages.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "Algo", "Mod", "Pressure", "Op 1", "Op 2", "Op 3", "Op 4", "Op 5", "Op 6",
                "Kbd 1-2", "Kbd 3-4", "Kbd 5-6"
            ]
        );
        assert_eq!(pages[2].params.len(), 16);
        let q: Vec<&str> = qlink_targets(&pages[2], params)
            .iter()
            .map(|&i| params[i].name)
            .collect();
        assert_eq!(q, ["Threshold", "Full at", "Depth 1", "Depth 2"]);
        let mut seen = vec![0; params.len()];
        for p in &pages {
            assert!(
                p.params.len() <= MAX_TILES,
                "{}: {} tiles",
                p.name,
                p.params.len()
            );
            for &i in &p.params {
                seen[i] += 1;
            }
        }
        assert!(
            seen.iter().all(|&n| n == 1),
            "every parameter on exactly one page"
        );
        // The Algo page holds the six operators' On switches; each operator
        // page has its wave in their place.
        let on: Vec<&str> = pages[0]
            .params
            .iter()
            .filter(|&&i| params[i].name == "On")
            .map(|&i| params[i].group)
            .collect();
        assert_eq!(on, ["Op 1", "Op 2", "Op 3", "Op 4", "Op 5", "Op 6"]);
        for p in &pages[3..9] {
            assert_eq!(p.params.len(), 16);
            assert!(p.params.iter().any(|&i| params[i].name == "Wave"));
            let q: Vec<&str> = qlink_targets(p, params)
                .iter()
                .map(|&i| params[i].name)
                .collect();
            assert_eq!(q, ["Output", "Coarse", "Fine", "Velocity"], "{}", p.name);
        }
        // Labels stay short enough for a tile.
        for p in &pages {
            for &i in &p.params {
                let l = tile_label(&p.name, &params[i]);
                assert!(l.len() <= 20, "{}: {l}", p.name);
            }
        }
    }

    #[test]
    fn drums_get_a_page_per_pad_and_the_kit() {
        use mpfree_node::Node;
        let d = mpfree_synth::Drums::new(48_000);
        let params = d.params();
        let pages = plan(d.info(), params);
        let names: Vec<&str> = pages.iter().map(|p| p.name.as_str()).collect();
        let mut want: Vec<String> = (1..=16).map(|n| format!("Pad {n}")).collect();
        want.push("Kit".into());
        want.extend((1..=16).map(|n| format!("Pad {n} effects")));
        want.push("Sends".into());
        assert_eq!(names, want);
        every_param_once(&pages, params.len());
        for (n, p) in pages[..16].iter().enumerate() {
            assert_eq!(p.params.len(), 14);
            let q: Vec<&str> = qlink_targets(p, params)
                .iter()
                .map(|&i| params[i].name)
                .collect();
            assert_eq!(q, ["Tune", "Decay", "Tone", "Snap"], "{}", p.name);
            assert_eq!(drum_page(n), n);
            for &i in &p.params {
                assert!(tile_label(&p.name, &params[i]).len() <= 20);
            }
        }
        assert_eq!(names[DRUM_KIT_PAGE], "Kit");
        for (n, p) in pages[17..33].iter().enumerate() {
            assert_eq!(drum_fx_page(n), 17 + n);
            assert_eq!(p.params.len(), 16, "{}", p.name);
            assert_eq!(shared_tab(&p.name), Some((DRUM_FX_TAB, n)));
            assert_eq!(page_pad(&p.name), Some(n));
            let q: Vec<&str> = qlink_targets(p, params)
                .iter()
                .map(|&i| params[i].name)
                .collect();
            assert_eq!(q, ["Punch", "Distortion", "Delay send", "Reverb send"]);
            for &i in &p.params {
                assert!(tile_label(&p.name, &params[i]).len() <= 20);
            }
        }
        // A hit follows to its pad's effects while effects show, else to
        // its own page.
        assert_eq!(drum_follow(drum_fx_page(2), 9), drum_fx_page(9));
        assert_eq!(drum_follow(3, 9), drum_page(9));
        assert_eq!(drum_follow(DRUM_KIT_PAGE, 9), drum_page(9));
        assert_eq!(drum_follow(33, 9), drum_page(9));
        assert_eq!(names[33], "Sends");
        assert_eq!(pages[33].params.len(), 9);
        assert_eq!(shared_tab("Pad 3"), None);
        assert_eq!(page_pad("Pad 3"), Some(2));
        assert_eq!(page_pad("Kit"), None);
    }

    #[test]
    fn strings_has_its_own_pages() {
        use mpfree_node::Node;
        let s = mpfree_synth::Strings::new(48_000);
        let params = s.params();
        let pages = plan(s.info(), params);
        let names: Vec<&str> = pages.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["Registers", "Tone", "Ensemble", "Voice"]);
        every_param_once(&pages, params.len());
        let q: Vec<&str> = qlink_targets(&pages[0], params)
            .iter()
            .map(|&i| params[i].name)
            .collect();
        assert_eq!(q, ["Violin", "Viola", "Cello", "Contrabass"]);
    }

    #[test]
    fn bass_has_its_own_pages() {
        use mpfree_node::Node;
        let b = mpfree_synth::Bass::new(48_000);
        let params = b.params();
        let pages = plan(b.info(), params);
        let names: Vec<&str> = pages.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["Source", "Filter", "Shape", "Play", "Output"]);
        every_param_once(&pages, params.len());
        let q: Vec<&str> = qlink_targets(&pages[0], params)
            .iter()
            .map(|&i| params[i].name)
            .collect();
        assert_eq!(q, ["Sub", "Fifth", "Detune", "Pulse width"]);
    }

    #[test]
    fn wind_has_its_own_pages() {
        use mpfree_node::Node;
        let w = mpfree_synth::Wind::new(48_000);
        let params = w.params();
        let pages = plan(w.info(), params);
        let names: Vec<&str> = pages.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["Instrument", "Breath", "Play", "Pressure"]);
        every_param_once(&pages, params.len());
        let q = |page: usize| -> Vec<&str> {
            qlink_targets(&pages[page], params)
                .iter()
                .map(|&i| params[i].name)
                .collect()
        };
        assert_eq!(q(0), ["Breath", "Vibrato", "Tone", "Noise"]);
        assert_eq!(q(1), ["Tongue", "Attack", "Sustain", "Release"]);
        assert!(pages.iter().all(|p| p.params.len() <= MAX_TILES));
    }

    #[test]
    fn keys_has_its_own_pages() {
        use mpfree_node::Node;
        let s = mpfree_synth::Keys::new(48_000);
        let params = s.params();
        let pages = plan(s.info(), params);
        let names: Vec<&str> = pages.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["Strike", "Tone", "Clav", "Amp", "Voice"]);
        every_param_once(&pages, params.len());
        let names =
            |page: &Page| -> Vec<&str> { page.params.iter().map(|&i| params[i].name).collect() };
        assert_eq!(
            names(&pages[0]),
            ["Model", "Decay", "Release", "Hardness", "Bell"]
        );
        let q: Vec<&str> = qlink_targets(&pages[0], params)
            .iter()
            .map(|&i| params[i].name)
            .collect();
        assert_eq!(q, ["Hardness", "Bell", "Decay", "Release"]);
        let q: Vec<&str> = qlink_targets(&pages[1], params)
            .iter()
            .map(|&i| params[i].name)
            .collect();
        assert_eq!(q, ["Voicing", "Bark", "Tone", "Vel > tone"]);
        assert_eq!(
            names(&pages[2]),
            ["Pickup", "Clav tone", "Mute", "Release drop"]
        );
        for p in &pages {
            for &i in &p.params {
                assert!(tile_label(&p.name, &params[i]).len() <= 20);
            }
        }
    }

    #[test]
    fn squelch_has_its_own_pages() {
        use mpfree_node::Node;
        let s = mpfree_synth::Squelch::new(48_000);
        let params = s.params();
        let pages = plan(s.info(), params);
        let names: Vec<&str> = pages.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["Synth", "Play", "Voice"]);
        every_param_once(&pages, params.len());
        let q: Vec<&str> = qlink_targets(&pages[0], params)
            .iter()
            .map(|&i| params[i].name)
            .collect();
        assert_eq!(q, ["Cutoff", "Resonance", "Env mod", "Decay"]);
        for p in &pages {
            for &i in &p.params {
                assert!(tile_label(&p.name, &params[i]).len() <= 20);
            }
        }
    }

    #[test]
    fn analog_has_its_own_pages() {
        use mpfree_node::Node;
        let s = mpfree_synth::Analog::new(48_000);
        let params = s.params();
        let pages = plan(s.info(), params);
        let names: Vec<&str> = pages.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["Osc", "Filter", "Envelopes", "Mod", "Voice"]);
        every_param_once(&pages, params.len());
        let q = |page: usize| -> Vec<&str> {
            qlink_targets(&pages[page], params)
                .iter()
                .map(|&i| params[i].name)
                .collect()
        };
        assert_eq!(q(1), ["Cutoff", "Resonance", "Env amount", "Drive"]);
        assert_eq!(q(3), ["LFO rate", "Vibrato", "PMod env", "PMod osc 2"]);
        assert_eq!(q(4), ["Age", "Glide", "Detune", "Level"]);
        for p in &pages {
            assert!(p.params.len() <= MAX_TILES, "{}", p.name);
            for &i in &p.params {
                assert!(tile_label(&p.name, &params[i]).len() <= 20);
            }
        }
    }

    #[test]
    fn drawbar_has_its_own_pages() {
        use mpfree_node::Node;
        let s = mpfree_synth::Drawbar::new(48_000);
        let params = s.params();
        let pages = plan(s.info(), params);
        let names: Vec<&str> = pages.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["Drawbars", "Percussion", "Rotary", "Voice"]);
        every_param_once(&pages, params.len());
        let q: Vec<&str> = qlink_targets(&pages[0], params)
            .iter()
            .map(|&i| params[i].name)
            .collect();
        assert_eq!(q, ["4'", "2 2/3'", "2'", "1'"]);
        let q: Vec<&str> = qlink_targets(&pages[2], params)
            .iter()
            .map(|&i| params[i].name)
            .collect();
        assert_eq!(q, ["Drive", "Tone", "Click", "Spread"]);
        for p in &pages {
            for &i in &p.params {
                assert!(tile_label(&p.name, &params[i]).len() <= 20);
            }
        }
    }

    #[test]
    fn pluck_has_its_own_pages() {
        use mpfree_node::Node;
        let s = mpfree_synth::Pluck::new(48_000);
        let params = s.params();
        let pages = plan(s.info(), params);
        let names: Vec<&str> = pages.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["String", "Mallet", "Voice"]);
        every_param_once(&pages, params.len());
        let q: Vec<&str> = qlink_targets(&pages[0], params)
            .iter()
            .map(|&i| params[i].name)
            .collect();
        assert_eq!(q, ["Decay", "Brightness", "Pick", "Position"]);
        let q: Vec<&str> = qlink_targets(&pages[1], params)
            .iter()
            .map(|&i| params[i].name)
            .collect();
        assert_eq!(q, ["Hardness", "Strike", "Stretch", "High damp"]);
    }

    #[test]
    fn any_node_gets_generic_pages() {
        let pages = plan(&OTHER, PARAMS);
        every_param_once(&pages, PARAMS.len());
        assert_eq!(pages[0].name, "Osc 1");
    }

    #[test]
    fn a_big_group_is_split() {
        const fn q(id: u16) -> ParamInfo {
            ParamInfo {
                id,
                name: "P",
                group: "Big",
                min: 0.0,
                max: 1.0,
                default: 0.0,
                unit: Unit::None,
                curve: Curve::Linear,
                readonly: false,
            }
        }
        let params: Vec<ParamInfo> = (0..40).map(q).collect();
        let pages = plan(&OTHER, &params);
        let names: Vec<&str> = pages.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["Big", "Big 2", "Big 3"]);
        every_param_once(&pages, 40);
    }

    #[test]
    fn labels_name_the_group_when_it_is_not_the_page() {
        let p = |g: &'static str, n: &'static str| ParamInfo {
            id: 0,
            name: n,
            group: g,
            min: 0.0,
            max: 1.0,
            default: 0.0,
            unit: Unit::None,
            curve: Curve::Linear,
            readonly: false,
        };
        assert_eq!(tile_label("Filter", &p("Filter", "Cutoff")), "Cutoff");
        assert_eq!(
            tile_label("Filter", &p("Filter env", "Attack")),
            "Env · Attack"
        );
        assert_eq!(tile_label("Osc", &p("Osc 2", "Wave")), "Osc 2 · Wave");
        assert_eq!(tile_label("Osc", &p("Mix", "Sub")), "Mix · Sub");
        assert_eq!(tile_label("Osc", &p("Unison", "Unison")), "Unison");
    }

    /// A sound's macros are its first page, turned by Q-Links 1 to 4; the
    /// engine's own pages follow, then each effect's.
    #[test]
    fn a_sounds_macros_come_first_and_its_effects_last() {
        use mpfree_instruments::{Design, Macro, MacroMap, Sound};
        use mpfree_mod::MapCurve;
        use mpfree_node::Node;
        use mpfree_synth::keys_params as kp;
        let map = |target| MacroMap {
            target,
            from: 0.0,
            to: 50.0,
            curve: MapCurve::Linear,
        };
        let design = Design {
            fx: vec!["mpfree.fx.ensemble".into()],
            macros: ["Bark", "Tone", "Hammer", "Space", "Width"]
                .iter()
                .zip([kp::BARK, kp::TONE, kp::HARDNESS, kp::DRIVE, kp::BELL])
                .map(|(name, t)| Macro {
                    name: name.to_string(),
                    maps: vec![map(t)],
                })
                .collect(),
        };
        let s = Sound::new(Box::new(mpfree_synth::Keys::new(48_000)), &design).unwrap();
        let params = s.params();
        let pages = plan(s.info(), params);
        let names: Vec<&str> = pages.iter().map(|p| p.name.as_str()).collect();
        let keys: Vec<String> = plan(s.info(), mpfree_synth::Keys::new(48_000).params())
            .into_iter()
            .map(|p| p.name)
            .collect();
        assert_eq!(names[0], MACROS);
        assert_eq!(names[1..=keys.len()], keys[..]);
        assert_eq!(names[keys.len() + 1..], ["FX 1"]);
        every_param_once(&pages, params.len());
        let q: Vec<&str> = qlink_targets(&pages[0], params)
            .iter()
            .map(|&i| params[i].name)
            .collect();
        assert_eq!(q, ["Bark", "Tone", "Hammer", "Space"]);
        // The page's tiles are the macros' own names.
        assert_eq!(tile_label(MACROS, &params[pages[0].params[4]]), "Width");
    }
}
