//! Sample arp screens with a real engine behind them, for the workstation
//! preview and the tests. Pad names are made up (no Akai content).
//!
//! The screens sit in the pad player as it is today: its header tabs (the
//! arp is not one of them), the arp chip in the header, the Arp panel in the
//! Play page's right-hand column, and the quick band over the pads while
//! NOTE REPEAT is held.

use crate::maker::{MakerRow, MakerView};
use crate::phrases::{PhraseList, Shelf};
use crate::view::{chips, hint, hint_for, quick_hint, ArpPanel, ArpState};
use crate::workspace::{self, record_pad_labels, LaneRow, PatternWorkspace, Row};
use mpfree_arp::scale::MINOR;
use mpfree_arp::{
    Engine, Events, Follow, LatchMode, Library, Mode, Order, Pattern, Phrase, Rate, Scale, Settings,
};
use mpfree_ui::widgets::{Header, Label, PadGrid};
use mpfree_ui::{Align, Color, Fonts, Theme, Tone, Ui};

/// The screen as the pad player sees it (the MPC One's panel, turned).
pub const SCREEN: (i32, i32) = (1280, 800);
/// The page names the samples add the panel and the quick band under.
pub const PANEL_PAGE: &str = "Play";
pub const QUICK_PAGE: &str = "Play, NOTE REPEAT held";
const PAD_NAMES: [&str; 16] = [
    "Kick",
    "Snare",
    "Closed Hat",
    "Open Hat",
    "Clap",
    "Rim",
    "Tom Low",
    "Tom Hi",
    "Crash",
    "Ride",
    "Shaker",
    "Perc 1",
    "Perc 2",
    "",
    "Sub Hit",
    "",
];
const SAMPLE_RATE: u32 = 44_100;

/// One sample screen: its file name and the UI showing it.
pub struct Sample {
    pub name: &'static str,
    pub about: &'static str,
    pub ui: Ui,
    pub page: ArpPanel,
    /// The quick band is showing, not the panel.
    pub quick: bool,
    pub state: ArpState,
}

/// The built-in patterns as the page lists them.
pub fn built_in_patterns() -> Vec<(String, mpfree_arp::Pattern)> {
    Library::built_in()
        .iter()
        .map(|(n, p)| (n.to_string(), *p))
        .collect()
}

/// Runs `e` for `steps` steps of its rate, in 128-frame periods.
fn run_steps(e: &mut Engine, steps: f64) {
    let frames = steps * f64::from(e.settings().rate.ticks()) / f64::from(mpfree_arp::PPQ) * 60.0
        / e.bpm()
        * f64::from(SAMPLE_RATE);
    let mut out = Events::new();
    let mut done = 0.0;
    while done < frames {
        e.process(128, &mut out);
        done += 128.0;
    }
}

/// What a sample screen plays: `settings`, built-in pattern `pattern`
/// (mutated or not), pads `held` (slots), latched or still held, run for
/// `steps` steps; and whether it shows the quick band or the panel.
struct Setup<'a> {
    settings: Settings,
    pattern: usize,
    held: &'a [u8],
    steps: f64,
    mutated: bool,
    latch: bool,
    quick: bool,
}

impl Default for Setup<'_> {
    fn default() -> Self {
        Setup {
            settings: Settings::default(),
            pattern: 0,
            held: &[],
            steps: 0.0,
            mutated: false,
            latch: true,
            quick: false,
        }
    }
}

fn scene(
    name: &'static str,
    about: &'static str,
    fonts: Fonts,
    setup: Setup,
) -> Result<Sample, String> {
    let Setup {
        settings,
        pattern,
        held,
        steps,
        mutated,
        latch,
        quick,
    } = setup;
    let patterns = built_in_patterns();
    let mut e = Engine::new(SAMPLE_RATE, 96.0);
    e.set_settings(settings);
    let mut p = patterns.get(pattern).map(|(_, p)| *p).unwrap_or_default();
    if mutated {
        p = p.mutated(7, 40);
    }
    e.set_pattern(p);
    e.set_latch(if latch {
        LatchMode::Replace
    } else {
        LatchMode::Off
    });
    for &slot in held {
        e.press(slot, 100);
    }
    if latch {
        for &slot in held {
            e.release(slot);
        }
    }
    run_steps(&mut e, steps);
    let state = ArpState::from_engine(&e, patterns, pattern, mutated);

    let mut ui = Ui::new(Theme::dark(), fonts, SCREEN.0, SCREEN.1);
    let mut header = Header::app("Play");
    header.chips = chips(&state);
    ui.add("header", header);
    let mut pads = PadGrid::new(16, 4);
    let kit = Color::from_pad([0x3F, 0x22, 0x00]);
    for (i, pad) in pads.pads.iter_mut().enumerate() {
        pad.label = PAD_NAMES[i].to_string();
        pad.color = (!PAD_NAMES[i].is_empty()).then_some(kit);
    }
    // The latched pads glow; the one the last step played is lit.
    for h in &state.notes {
        if let Some(pad) = pads.pads.get_mut(usize::from(h.note)) {
            pad.level = 0.35;
        }
    }
    ui.add("pads", pads);
    let line = if quick {
        quick_hint(settings.mode)
    } else {
        hint(settings.mode)
    };
    ui.add("hint", Label::new(line, Tone::Dim, Align::Center));
    let page = ArpPanel::add(&mut ui, PANEL_PAGE, QUICK_PAGE, &state)?;
    ui.show(if quick { QUICK_PAGE } else { PANEL_PAGE })?;
    Ok(Sample {
        name,
        about,
        ui,
        page,
        quick,
        state,
    })
}

/// What a sample of the Phrase tab shows: a synth track in A minor (the
/// pads' key) with the A under middle C held.
struct PhraseSetup {
    /// The track's pick, by name.
    pick: &'static str,
    /// The phrase still playing while the pick waits for the bar line
    /// (the transport plays).
    playing: Option<&'static str>,
    follow: Follow,
    swing: u8,
    octaves: u8,
    /// The Modifier pushes the rate from 1/16 to this.
    pushed: Option<Rate>,
    /// Beats the note has been held for.
    beats: f64,
    /// The owner's phrases, then one only the open project has, as text.
    own: &'static [(&'static str, &'static str)],
    project: &'static [(&'static str, &'static str)],
    /// The kind the list shows when it is not the pick's.
    shelf: Option<Shelf>,
    /// The track's instrument is a kit: its pads play as pressed.
    kit: bool,
    /// The quick band shows, not the panel.
    quick: bool,
}

impl Default for PhraseSetup {
    fn default() -> Self {
        PhraseSetup {
            pick: "Night drive",
            playing: None,
            follow: Follow::InKey,
            swing: 50,
            octaves: 1,
            pushed: None,
            beats: 3.0,
            own: &[],
            project: &[],
            shelf: None,
            kit: false,
            quick: false,
        }
    }
}

fn phrase_scene(
    name: &'static str,
    about: &'static str,
    fonts: Fonts,
    setup: PhraseSetup,
) -> Result<Sample, String> {
    let mut list = PhraseList::built_in();
    for (i, (n, text)) in setup.own.iter().chain(setup.project).enumerate() {
        let phrase = Phrase::parse(text).map_err(|e| format!("{n}: {e}"))?;
        list.phrases.push((n.to_string(), phrase));
        if i < setup.own.len() {
            list.own += 1;
        }
    }
    let place = |n: &str| {
        list.phrases
            .iter()
            .position(|(name, _)| name == n)
            .ok_or(format!("no phrase {n}"))
    };
    let pick = place(setup.pick)?;
    let playing = setup.playing.map(place).transpose()?;
    let settings = Settings {
        mode: if setup.kit {
            Mode::Direct
        } else {
            Mode::Phrase
        },
        follow: setup.follow,
        swing: setup.swing,
        octaves: setup.octaves,
        ..Settings::default()
    };
    let mut e = Engine::new(SAMPLE_RATE, 120.0);
    e.set_settings(settings);
    e.set_key(Some(Scale::new(9, MINOR)));
    // What sounds: the pick, or the phrase it waits behind.
    e.set_phrase(list.phrases[playing.unwrap_or(pick)].1);
    if !setup.kit {
        e.press(57, 100);
        let frames = setup.beats * 60.0 / e.bpm() * f64::from(SAMPLE_RATE);
        let mut out = Events::new();
        let mut done = 0.0;
        while done < frames {
            e.process(128, &mut out);
            done += 128.0;
        }
    }
    let mut state = ArpState::from_engine(&e, built_in_patterns(), 0, false);
    state.phrases = list;
    state.phrase_index = pick;
    state.phrase.shelf = setup.shelf;
    if playing.is_some() {
        state.phrase.waits = true;
        state.phrase.playing = playing;
    }
    if let Some(rate) = setup.pushed {
        state.live.rate = rate;
        state.push = 1.0;
    }
    if setup.kit {
        state.phrase.kit_track = Some("Track 1".to_string());
    }

    let mut ui = Ui::new(Theme::dark(), fonts, SCREEN.0, SCREEN.1);
    let mut header = Header::app("Play");
    header.chips = chips(&state);
    ui.add("header", header);
    ui.add("pads", PadGrid::new(16, 4));
    let line = if setup.quick {
        quick_hint(settings.mode).to_string()
    } else {
        hint_for(&state)
    };
    ui.add("hint", Label::new(&line, Tone::Dim, Align::Center));
    let page = ArpPanel::add(&mut ui, PANEL_PAGE, QUICK_PAGE, &state)?;
    ui.show(if setup.quick { QUICK_PAGE } else { PANEL_PAGE })?;
    Ok(Sample {
        name,
        about,
        ui,
        page,
        quick: setup.quick,
        state,
    })
}

/// New idea's panel (`more` false) or More like this's over the basses, as
/// boards B and C of the P2 boards draw them. The app makes the tries; here
/// the try is one of MPFree's, under the name Keep would give it.
fn maker_scene(
    name: &'static str,
    about: &'static str,
    fonts: Fonts,
    more: bool,
) -> Result<Sample, String> {
    let mut s = phrase_scene(name, about, fonts, PhraseSetup::default())?;
    let shown = |n: &str, as_: &str| {
        s.state
            .phrases
            .phrases
            .iter()
            .find(|(p, _)| p == n)
            .map(|(_, p)| (as_.to_string(), *p))
            .ok_or(format!("no phrase {n}"))
    };
    let row = |n: &str, v: &str, note: &str| MakerRow {
        name: n.to_string(),
        value: v.to_string(),
        accent: n == "Change",
        note: note.to_string(),
    };
    let view = if more {
        MakerView {
            title: "More like this".into(),
            from: Some(shown("Night drive", "Night drive")?),
            rows: vec![
                row("Change", "New notes", "Its rhythm, with new notes"),
                row("Style", "Groove", ""),
            ],
            notes: vec![
                "After New notes: New rhythm, Up a step, Upside down and".into(),
                "First half twice (each comes out one way).".into(),
                "Keep puts it on Yours as Night drive 2.".into(),
            ],
            status: "Try 2 · hold a pad: it plays from that note".into(),
            step: ("‹ Try 1".into(), true),
            shown: Some(shown("Funk pocket", "Night drive 2")?),
            head: "Try 2 · Back keeps Night drive".into(),
            ..MakerView::default()
        }
    } else {
        MakerView {
            title: "New idea · Bass".into(),
            rows: vec![
                row("Kind", "Bass", ""),
                row("Style", "Groove", ""),
                row("Bars", "2 bars", ""),
                row("Activity", "Middle", ""),
            ],
            notes: vec![
                "Written for the pads' key, A minor.".into(),
                "Try again keeps the last 8 while this is open.".into(),
                "Keep puts it on Yours as Groove bass 1.".into(),
            ],
            status: "Try 3 · hold a pad: it plays from that note".into(),
            step: ("‹ Try 2".into(), true),
            shown: Some(shown("Funk pocket", "Groove bass 1")?),
            head: "Try 3 · Back keeps Night drive".into(),
            ..MakerView::default()
        }
    };
    s.state.maker = Some(MakerView {
        can_try: true,
        can_keep: true,
        ..view
    });
    s.page.update(&mut s.ui, &s.state)?;
    if let Some(l) = s.ui.get_mut::<Label>("hint") {
        *l = Label::new(&hint_for(&s.state), Tone::Dim, Align::Center);
    }
    // The arp chip names the try that plays.
    if let Some(h) = s.ui.get_mut::<Header>("header") {
        h.chips = chips(&s.state);
    }
    Ok(s)
}

/// The Phrase tab's sample screens: the five boards the owner approved
/// (23 Nov 2026), then the cases they left out.
fn phrase_samples(fonts: &impl Fn() -> Fonts) -> Result<Vec<Sample>, String> {
    const OWN: [(&str, &str); 2] = [
        (
            "Verse bass",
            "bass minor : 1! . . 1' . . 1+' . . . b7- 1 . . 1+' .",
        ),
        ("riff-idea", "riff either : 5 . 6 5 . 3 . . 1 . 2 3 . 1 . ."),
    ];
    const PROJECT: [(&str, &str); 1] = [(
        "Climbing line",
        "line either : 1 3 5 1+ 3+ 5+ 1++ 5+ 3+ 1+ 5 3 1 5- 1 3",
    )];
    Ok(vec![
        phrase_scene(
            "phrase-browse",
            "Phrase tab: the basses, Night drive playing from a held note in A minor",
            fonts(),
            PhraseSetup::default(),
        )?,
        phrase_scene(
            "phrase-next-bar",
            "Phrase tab: Acid line picked while the song plays, waiting for the bar line",
            fonts(),
            PhraseSetup {
                pick: "Acid line",
                playing: Some("Night drive"),
                ..PhraseSetup::default()
            },
        )?,
        phrase_scene(
            "phrase-exact",
            "Phrase tab: a riff written in major, Exact, with swing and two octaves",
            fonts(),
            PhraseSetup {
                pick: "Synth pop hook",
                follow: Follow::Exact,
                swing: 58,
                octaves: 2,
                beats: 5.5,
                ..PhraseSetup::default()
            },
        )?,
        phrase_scene(
            "phrase-kit",
            "Phrase tab on a kit track: its pads play as pressed",
            fonts(),
            PhraseSetup {
                kit: true,
                ..PhraseSetup::default()
            },
        )?,
        phrase_scene(
            "phrase-yours",
            "Phrase tab: the owner's phrases, and one only this project has",
            fonts(),
            PhraseSetup {
                pick: "Verse bass",
                beats: 1.5,
                own: &OWN,
                project: &PROJECT,
                ..PhraseSetup::default()
            },
        )?,
        phrase_scene(
            "phrase-yours-empty",
            "Phrase tab: the owner's shelf before they have made a phrase",
            fonts(),
            PhraseSetup {
                shelf: Some(Shelf::Own),
                ..PhraseSetup::default()
            },
        )?,
        phrase_scene(
            "phrase-triplets-pushed",
            "Phrase tab: a phrase in triplets (swing leaves it), the Modifier pushing the speed",
            fonts(),
            PhraseSetup {
                pick: "Shuffle riff",
                swing: 62,
                pushed: Some(Rate::SixteenthTriplet),
                ..PhraseSetup::default()
            },
        )?,
        maker_scene(
            "phrase-new-idea",
            "Phrase tab: New idea's panel, its third try in the detail panel",
            fonts(),
            false,
        )?,
        maker_scene(
            "phrase-more-like",
            "Phrase tab: More like this from Night drive, its second try",
            fonts(),
            true,
        )?,
        phrase_scene(
            "quick-phrase",
            "Quick band: NOTE REPEAT held on a synth track, a phrase playing",
            fonts(),
            PhraseSetup {
                quick: true,
                ..PhraseSetup::default()
            },
        )?,
    ])
}

/// Every sample screen.
pub fn samples(fonts: impl Fn() -> Fonts) -> Result<Vec<Sample>, String> {
    let base = Settings::default();
    let mut all = vec![
        scene(
            "quick-hat-roll",
            "Quick band: NOTE REPEAT held with the closed hat, 1/16",
            fonts(),
            Setup {
                settings: Settings {
                    mode: Mode::Repeat,
                    gate: 40,
                    ..base
                },
                held: &[2],
                steps: 5.5,
                latch: false,
                quick: true,
                ..Setup::default()
            },
        )?,
        scene(
            "quick-arp-latched",
            "Quick band: Arp Up 1/16 over pads 1 3 5, latched",
            fonts(),
            Setup {
                settings: Settings {
                    mode: Mode::Arp,
                    order: Order::Up,
                    octaves: 2,
                    ..base
                },
                held: &[0, 2, 4],
                steps: 3.5,
                quick: true,
                ..Setup::default()
            },
        )?,
        scene(
            "arp-up-2-oct",
            "Panel: Arp, Up over 2 octaves, pads 1 3 5 latched",
            fonts(),
            Setup {
                settings: Settings {
                    mode: Mode::Arp,
                    order: Order::Up,
                    octaves: 2,
                    swing: 58,
                    ..base
                },
                held: &[0, 2, 4],
                steps: 3.5,
                ..Setup::default()
            },
        )?,
        scene(
            "arp-converge",
            "Panel: Arp, Converge, 1/16T, pads 1 2 3 4 latched",
            fonts(),
            Setup {
                settings: Settings {
                    mode: Mode::Arp,
                    order: Order::Converge,
                    rate: Rate::SixteenthTriplet,
                    gate: 80,
                    ..base
                },
                held: &[0, 1, 2, 3],
                steps: 6.5,
                ..Setup::default()
            },
        )?,
        scene(
            "arp-random",
            "Panel: Arp, Random, pads 1 5 latched",
            fonts(),
            Setup {
                settings: Settings {
                    mode: Mode::Arp,
                    order: Order::Random,
                    octaves: 3,
                    ..base
                },
                held: &[0, 4],
                steps: 2.5,
                ..Setup::default()
            },
        )?,
        scene(
            "repeat",
            "Panel: Note Repeat at 1/32, pad 3 latched",
            fonts(),
            Setup {
                settings: Settings {
                    mode: Mode::Repeat,
                    rate: Rate::ThirtySecond,
                    gate: 30,
                    ..base
                },
                held: &[2],
                steps: 5.5,
                ..Setup::default()
            },
        )?,
        scene(
            "pattern",
            "Panel: Pattern \"Climb\" over pads 1 2 5",
            fonts(),
            Setup {
                settings: Settings {
                    mode: Mode::Pattern,
                    ..base
                },
                pattern: 3,
                held: &[0, 1, 4],
                steps: 4.5,
                ..Setup::default()
            },
        )?,
        scene(
            "pattern-mutated",
            "Panel: Pattern \"Rolls\", mutated (Original is lit)",
            fonts(),
            Setup {
                settings: Settings {
                    mode: Mode::Pattern,
                    ..base
                },
                pattern: 6,
                held: &[0, 2, 4],
                steps: 2.5,
                mutated: true,
                ..Setup::default()
            },
        )?,
        scene(
            "direct",
            "Panel: Direct, nothing generated, so no step strip",
            fonts(),
            Setup {
                latch: false,
                ..Setup::default()
            },
        )?,
    ];
    all.extend(phrase_samples(&fonts)?);
    Ok(all)
}

/// One sample screen of the pattern workspace.
pub struct WorkspaceSample {
    pub name: &'static str,
    pub about: &'static str,
    pub ui: Ui,
    pub ws: PatternWorkspace,
}

#[derive(Clone, Copy, PartialEq)]
enum WsScene {
    Edit,
    Record,
    Lanes,
}

fn workspace_scene(
    name: &'static str,
    about: &'static str,
    fonts: Fonts,
    scene: WsScene,
) -> Result<WorkspaceSample, String> {
    let recording = scene == WsScene::Record;
    let climb = built_in_patterns()
        .into_iter()
        .find(|(n, _)| n == "Climb")
        .map(|(_, p)| p)
        .unwrap_or_default();
    let mut ui = Ui::new(Theme::dark(), fonts, SCREEN.0, SCREEN.1);
    let mut e = Engine::new(SAMPLE_RATE, 96.0);
    e.set_settings(Settings {
        mode: Mode::Pattern,
        ..Settings::default()
    });
    let mut header = Header::app("Play");
    let state = ArpState::from_engine(&e, built_in_patterns(), 3, true);
    header.chips = chips(&state);
    ui.add("header", header);
    let mut pads = PadGrid::new(16, 4);
    let notes = Color::from_pad([0x00, 0x22, 0x3F]);
    let marks = Color::from_pad([0x3F, 0x22, 0x00]);
    for (i, (pad, label)) in pads.pads.iter_mut().zip(record_pad_labels()).enumerate() {
        pad.label = label.to_string();
        pad.color = (!label.is_empty()).then_some(if i < 9 { notes } else { marks });
    }
    ui.add("pads", pads);
    ui.add("hint", Label::new("", Tone::Dim, Align::Center));
    let mut ws = PatternWorkspace::add(&mut ui, "Pattern", "Pattern, recording", "Climb", climb)?;
    if recording {
        // Part way through recording on the pads, in Step time.
        let taken = Pattern::parse("1 3 2! . 1+ _").map_err(|e| e.to_string())?;
        let mut steps = [mpfree_arp::Step::Rest; 8];
        steps[..taken.len()].copy_from_slice(taken.steps());
        ws.recorded(Pattern::from_steps(&steps).ok_or("pattern")?);
        ws.recording = true;
        ws.editor.select(Row::Step, 6);
        ws.update(&mut ui, None);
    } else if scene == WsScene::Lanes {
        // A 3-value velocity lane and a 5-value octave lane over Climb's
        // steps, with a little random gate; step 7 playing.
        for (lane, pos, d) in [
            (LaneRow::Velocity, 0, 15),
            (LaneRow::Velocity, 1, -5),
            (LaneRow::Velocity, 2, -3),
            (LaneRow::Octave, 0, 0),
            (LaneRow::Octave, 1, 0),
            (LaneRow::Octave, 2, 1),
            (LaneRow::Octave, 3, 0),
            (LaneRow::Octave, 4, -1),
            (LaneRow::Gate, 0, 6),
        ] {
            ws.editor.turn_lane(lane, pos, d);
        }
        ws.editor.set_lane_len(LaneRow::Velocity, 3);
        ws.editor.turn_random(LaneRow::Gate, 1);
        ws.editor.select_lane(LaneRow::Velocity, 1);
        ws.view = workspace::WsView::Lanes;
        ws.played = Some(6);
        ws.update(&mut ui, Some(6));
    } else {
        // An accent, a ratchet and a chance added; the ratchet is selected.
        ws.editor.tap(0);
        ws.editor.turn(Row::Ratchet, 3, 2);
        ws.editor.turn(Row::Chance, 5, -4);
        ws.editor.select(Row::Ratchet, 3);
        ws.update(&mut ui, Some(4));
    }
    if let Some(h) = ui.get_mut::<Label>("hint") {
        h.text = ws.hint().to_string();
    }
    ui.show(ws.page())?;
    Ok(WorkspaceSample {
        name,
        about,
        ui,
        ws,
    })
}

/// The pattern workspace's sample screens.
pub fn workspace_samples(fonts: impl Fn() -> Fonts) -> Result<Vec<WorkspaceSample>, String> {
    Ok(vec![
        workspace_scene(
            "workspace-edit",
            "Pattern workspace: \"Climb\" with an accent, a ratchet and a chance added",
            fonts(),
            WsScene::Edit,
        )?,
        workspace_scene(
            "workspace-record",
            "Pattern workspace: recording on the pads, 6 of 8 steps in",
            fonts(),
            WsScene::Record,
        )?,
        workspace_scene(
            "workspace-lanes",
            "Pattern workspace: the lanes, each looping at its own length",
            fonts(),
            WsScene::Lanes,
        )?,
    ])
}
