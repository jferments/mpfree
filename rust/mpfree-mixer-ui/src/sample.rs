//! Sample mixer screens with a real mixer playing behind them, for the
//! workstation preview (`mpfree-mixer-preview`) and the tests: a song's
//! routing (drums into a drum bus, the bass keyed by the kick, music into
//! a second submix, reverb and delay returns, pad channels into the kit
//! track, the line input armed), its meters after a few blocks, and the
//! mixer's own cost.

use mpfree_fx::ids::mixer::{self as mx, monitor, output, source};
use mpfree_fx::mpfree_node::Ctx;
use mpfree_fx::{ids, Cloud, Compressor, Delay, Drive, Eq, Frozen, Mixer, MixerConfig, StripId};
use mpfree_ui::widgets::{Chip, Header};
use mpfree_ui::{Align, Fonts, Theme, Tone, Touch, Ui};

use crate::state::{default_name, MixState, MixView, TakeView};
use crate::view::{IoIcon, MixerView};

/// A preview page: the header with the I/O icon beside it, the mixer, the
/// hint line.
const PREVIEW_LAYOUT: &str = "col(row(header:*, ioicon:56):60, mixer:*, hint:26)";

pub struct Sample {
    pub name: &'static str,
    pub about: &'static str,
    pub ui: Ui,
}

const TRACK_NAMES: [&str; 16] = [
    "Kick",
    "Snare",
    "Drum kit",
    "Bass",
    "Chords",
    "Lead",
    "Vocal chop",
    "Pad",
    "Perc",
    "Keys",
    "Strings",
    "FX riser",
    "Track 13",
    "Track 14",
    "Track 15",
    "Track 16",
];
const PAD_NAMES: [&str; 8] = [
    "A01 Kick",
    "A02 Snare",
    "A03 Hat",
    "A04 Clap",
    "A05 Rim",
    "A06 Tom",
    "A07 Shaker",
    "A08 Crash",
];

/// The strips' names in the sample song.
pub fn name(s: StripId) -> String {
    match s {
        StripId::Track(t) if t < TRACK_NAMES.len() => TRACK_NAMES[t].into(),
        StripId::Pad(p) if p < PAD_NAMES.len() => PAD_NAMES[p].into(),
        StripId::Input(0) => "Line in".into(),
        StripId::Input(1) => "Mic (in 2)".into(),
        StripId::Bus(0) => "Drums".into(),
        StripId::Bus(1) => "Music".into(),
        StripId::Return(0) => "Reverb".into(),
        StripId::Return(1) => "Delay".into(),
        s => default_name(s),
    }
}

const RATE: u32 = 44_100;
const BLOCK: usize = 128;

/// The sample song's mixer, set up the way a user would.
pub fn song_mixer() -> Mixer {
    let mut m = Mixer::new(MixerConfig::default(), RATE, BLOCK);
    let t = StripId::Track;
    for s in [t(0), t(1), t(2)] {
        m.set_now(s, mx::OUTPUT, output::bus(0));
    }
    m.set_now(t(1), mx::send(0), -14.0);
    m.set_now(t(2), mx::VOLUME, -2.0);
    let glue = m.chain_mut(StripId::Bus(0)).unwrap();
    glue.push(Box::new(Compressor::new()));
    glue.push(Box::new(Drive::new()));
    m.set_now(t(3), mx::VOLUME, -3.0);
    let duck = m.chain_mut(t(3)).unwrap();
    duck.push(Box::new(Compressor::new()));
    duck.set_param(0, ids::compressor::SIDECHAIN, 1.0);
    duck.push(Box::new(Eq::new()));
    m.set_key(t(3), 0, Some(t(0)));
    for (s, pan) in [
        (t(4), -15.0),
        (t(5), 20.0),
        (t(7), -30.0),
        (t(9), 35.0),
        (t(10), -45.0),
    ] {
        m.set_now(s, mx::OUTPUT, output::bus(1));
        m.set_now(s, mx::PAN, pan);
    }
    m.set_now(t(4), mx::send(0), -10.0);
    m.set_now(t(4), mx::send(1), -9.0);
    m.set_now(t(4), mx::send_pre(1), 1.0);
    m.set_now(t(5), mx::send(1), -6.0);
    m.chain_mut(t(5)).unwrap().push(Box::new(Eq::new()));
    m.chain_mut(t(5)).unwrap().push(Box::new(Delay::new()));
    m.set_now(t(6), mx::MUTE, 1.0);
    m.set_now(t(8), mx::PAN, 40.0);
    m.set_now(t(8), mx::VOLUME, -6.5);
    m.set_now(t(11), mx::VOLUME, -12.0);
    m.set_now(t(11), mx::send(0), -4.0);
    m.chain_mut(StripId::Bus(1))
        .unwrap()
        .push(Box::new(Eq::new()));
    let verb = m.chain_mut(StripId::Return(0)).unwrap();
    verb.push(Box::new(Cloud::new()));
    verb.set_param(0, ids::cloud::MIX, 100.0);
    let echo = m.chain_mut(StripId::Return(1)).unwrap();
    echo.push(Box::new(Delay::new()));
    echo.set_param(0, ids::delay::MIX, 100.0);
    m.set_now(StripId::Return(1), mx::VOLUME, -4.0);
    m.chain_mut(StripId::Master)
        .unwrap()
        .push(Box::new(Eq::new()));
    // Pad channels: the kit's pads that need their own strip, into the kit
    // track; the crash straight to the drum bus.
    for p in 0..8 {
        m.set_now(StripId::Pad(p), mx::OUTPUT, output::TRACK);
        m.set_now(StripId::Pad(p), mx::OUTPUT_TRACK, 2.0);
    }
    m.set_now(StripId::Pad(2), mx::PAN, -25.0);
    m.set_now(StripId::Pad(1), mx::send(0), -12.0);
    m.chain_mut(StripId::Pad(1))
        .unwrap()
        .push(Box::new(Eq::new()));
    m.set_now(StripId::Pad(7), mx::OUTPUT, output::bus(0));
    m.set_now(StripId::Pad(5), mx::VOLUME, -4.0);
    // The line input armed and heard through a compressor; the mic on
    // input 2 recording into the vocal track, not monitored.
    let i = StripId::Input;
    m.set_now(i(0), mx::SOURCE, source::STEREO);
    m.set_now(i(0), mx::ARM, 1.0);
    m.chain_mut(i(0)).unwrap().push(Box::new(Compressor::new()));
    m.set_now(i(1), mx::SOURCE, source::RIGHT);
    m.set_now(i(1), mx::MONITOR, monitor::OFF);
    m.set_now(i(1), mx::OUTPUT, output::TRACK);
    m.set_now(i(1), mx::OUTPUT_TRACK, 6.0);
    m
}

/// Plays `blocks` blocks of a busy song into `m` (each fed strip its own
/// tone and level), with profiling on.
pub fn play(m: &mut Mixer, blocks: u64) {
    m.set_profiling(true);
    let levels = [
        0.7f32, 0.5, 0.35, 0.45, 0.3, 0.28, 0.4, 0.2, 0.25, 0.22, 0.15, 0.3,
    ];
    let mut seed = 1u32;
    let (mut l, mut r) = (vec![0.0f32; BLOCK], vec![0.0f32; BLOCK]);
    for b in 0..blocks {
        let f0 = b * BLOCK as u64;
        for (t, &amp) in levels.iter().enumerate() {
            let hz = 55.0 * (1 + t) as f32;
            let (il, ir) = m.track_input(t).unwrap();
            for k in 0..BLOCK {
                let ph = (f0 + k as u64) as f32 * hz / RATE as f32;
                let x = amp * (std::f32::consts::TAU * ph).sin();
                il[k] = x;
                ir[k] = x * (0.6 + 0.4 * ((t % 3) as f32 / 2.0));
            }
        }
        for p in 0..8 {
            let (il, ir) = m.pad_input(p).unwrap();
            for k in 0..BLOCK {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let n = ((seed >> 9) as f32 / (1u32 << 23) as f32 - 0.5) * 0.9 / (1 + p) as f32;
                il[k] = n;
                ir[k] = n * 0.8;
            }
        }
        let (hl, hr) = m.hw_input();
        for k in 0..BLOCK {
            let ph = (f0 + k as u64) as f32 * 330.0 / RATE as f32;
            hl[k] = 0.25 * (std::f32::consts::TAU * ph).sin();
            hr[k] = 0.18 * (std::f32::consts::TAU * ph * 1.01).sin();
        }
        m.process(
            &Ctx {
                bpm: 92.0,
                playing: true,
                ..Ctx::new(RATE, f0)
            },
            &mut l,
            &mut r,
        );
    }
}

/// The app's screen around the mixer: the header with the Mix tab lit, the
/// I/O icon, the hint.
pub fn screen(fonts: Fonts, st: MixState) -> Result<Ui, String> {
    let mut ui = Ui::new(Theme::dark(), fonts, 1280, 800);
    let mut header = Header::app("Mix");
    header.chips = vec![
        Chip::new("92 BPM", Tone::Normal),
        Chip::new("Level -6 dB", Tone::Normal),
        Chip::new("Kit 808 Basic · A", Tone::Accent),
        Chip::new("Night drive", Tone::Normal),
    ];
    ui.add("header", header);
    let io = st.view == MixView::Io;
    ui.add("ioicon", IoIcon { open: io });
    ui.add(
        "hint",
        mpfree_ui::widgets::Label::new(
            if io {
                crate::MIXER_HINT_IO
            } else if st.takes {
                crate::MIXER_HINT_TAKES
            } else {
                crate::MIXER_HINT
            },
            Tone::Dim,
            Align::Left,
        ),
    );
    ui.add(crate::MIXER, MixerView::new());
    ui.add_page(crate::MIXER_PAGE, PREVIEW_LAYOUT)?;
    ui.show(crate::MIXER_PAGE)?;
    crate::update(&mut ui, st);
    Ok(ui)
}

/// A screen on the Effects look, its hint with it.
fn effects(ui: Ui) -> Ui {
    look(ui, crate::Look::Effects)
}

/// A screen on look `l`, its hint with it.
fn look(mut ui: Ui, l: crate::Look) -> Ui {
    crate::set_look(&mut ui, l);
    if let Some(h) = ui.get_mut::<mpfree_ui::widgets::Label>("hint") {
        h.text = crate::hint_for(l, false).to_string();
    }
    ui
}

/// The song's EQs shaped as a user would (the EQ look's sample): a kick
/// EQ added, the bass's and the lead's and the master's set.
fn shape_eqs(m: &mut Mixer) {
    use mpfree_fx::ids::eq as q;
    let t = StripId::Track;
    m.chain_mut(t(0)).unwrap().push(Box::new(Eq::new()));
    type Settings<'a> = &'a [(u16, f32)];
    let sets: [(StripId, usize, Settings<'_>); 4] = [
        (
            t(0),
            0,
            &[
                (q::LOW_SLOPE, 2.0),
                (q::LOW_CUT, 30.0),
                (q::freq(0), 60.0),
                (q::gain(0), 4.0),
                (q::q(0), 1.2),
                (q::freq(1), 350.0),
                (q::gain(1), -5.0),
                (q::q(1), 1.6),
                (q::band_type(3), 2.0),
                (q::freq(3), 6_000.0),
                (q::gain(3), 2.5),
                (q::band_use(2), q::USE_UNUSED),
            ],
        ),
        (
            t(3),
            1,
            &[
                (q::LOW_SLOPE, 1.0),
                (q::LOW_CUT, 35.0),
                (q::HIGH_SLOPE, 1.0),
                (q::HIGH_CUT, 9_000.0),
                (q::freq(0), 90.0),
                (q::gain(0), 3.0),
                (q::freq(1), 320.0),
                (q::gain(1), -3.5),
                (q::q(1), 1.4),
                (q::freq(2), 1_200.0),
                (q::gain(2), 2.0),
                (q::q(2), 2.0),
                (q::band_use(3), q::USE_UNUSED),
            ],
        ),
        (
            t(5),
            0,
            &[
                (q::LOW_SLOPE, 1.0),
                (q::LOW_CUT, 180.0),
                (q::freq(1), 3_000.0),
                (q::gain(1), 3.0),
                (q::q(1), 1.2),
                (q::band_type(2), 3.0),
                (q::freq(2), 5_200.0),
                (q::q(2), 8.0),
                (q::band_type(3), 2.0),
                (q::freq(3), 12_000.0),
                (q::gain(3), 1.5),
                (q::band_use(0), q::USE_UNUSED),
            ],
        ),
        (
            StripId::Master,
            0,
            &[
                (q::LOW_SLOPE, 1.0),
                (q::LOW_CUT, 25.0),
                (q::band_type(3), 2.0),
                (q::freq(3), 10_000.0),
                (q::gain(3), 1.0),
                (q::band_use(0), q::USE_UNUSED),
                (q::band_use(1), q::USE_UNUSED),
                (q::band_use(2), q::USE_UNUSED),
            ],
        ),
    ];
    for (s, slot, values) in sets {
        let c = m.chain_mut(s).unwrap();
        for &(id, v) in values {
            c.set_param(slot, id, v);
        }
    }
}

fn tap(ui: &mut Ui, x: i32, y: i32) {
    ui.touch(1, Touch::Down, x, y);
    ui.touch(1, Touch::Up, x, y);
}

/// Every sample screen.
pub fn samples(fonts: impl Fn() -> Fonts) -> Result<Vec<Sample>, String> {
    let mut m = song_mixer();
    play(&mut m, 60);
    let state = |m: &mut Mixer, v: MixView| {
        let mut st = MixState::from_mixer(m, v, &name);
        st.selected = Some(StripId::Track(3));
        st
    };
    let tracks = state(&mut m, MixView::Tracks);
    play(&mut m, 4);
    let pads = state(&mut m, MixView::Pads);
    play(&mut m, 4);
    let inputs = state(&mut m, MixView::Inputs);
    play(&mut m, 4);
    let buses = state(&mut m, MixView::Buses);
    play(&mut m, 4);
    let io = state(&mut m, MixView::Io);
    let mut out = vec![
        Sample { name: "mixer-tracks", about: "Track Mix while the song plays: the bass selected, its compressor keyed by the kick (SC), the vocal track muted, a pre-fader send on the chords, the mixer's own CPU share (Dev tools on)", ui: screen(fonts(), tracks.clone())? },
        Sample { name: "mixer-pads", about: "Pad Mix (SHIFT + TRACK MIX): eight pad channels into the kit track, the crash straight to the drum bus", ui: screen(fonts(), pads.clone())? },
        Sample { name: "mixer-inputs", about: "Inputs: the line input armed and heard through a compressor (Auto monitoring); the mic on input 2 into the vocal track, not monitored", ui: screen(fonts(), inputs)? },
        Sample { name: "mixer-submixes", about: "Submixes and returns: the drum bus with glue and drive, the music bus, the reverb and delay returns", ui: screen(fonts(), buses)? },
        Sample { name: "mixer-io", about: "The I/O view, Audio: every strip with an output and where it goes, the inputs with their Source and Monitor (MIDI, Sync and CV join the list from their own crates)", ui: screen(fonts(), io)? },
        Sample { name: "mixer-effects", about: "The Effects look: each track's chain as a row of eight slots in order, + FX in the first free one, the master's row pinned at the bottom", ui: effects(screen(fonts(), tracks.clone())?) },
        Sample { name: "mixer-effects-pads", about: "The Effects look on the pads: each pad channel's chain", ui: effects(screen(fonts(), pads.clone())?) },
    ];
    // The EQ look: the song's EQs shaped, the bass's picked on band 2.
    let mut me = song_mixer();
    shape_eqs(&mut me);
    play(&mut me, 60);
    let eqs = state(&mut me, MixView::Tracks);
    let mut ui = look(screen(fonts(), eqs)?, crate::Look::Eq);
    if let Some((x, y)) = eq_dot(&ui, 3, 1) {
        tap(&mut ui, x, y);
        let _ = crate::take_actions(&mut ui);
    }
    out.push(Sample { name: "mixer-eq", about: "The EQ look: each track's first EQ as its curve, a dot a band, + EQ on a strip without one; the bass's EQ picked on band 2 for the Q-Links", ui });
    // Solo in place: the bass soloed; what does not reach the master with
    // it is dimmed.
    m.set(StripId::Track(3), mx::SOLO, 1.0);
    play(&mut m, 8);
    let _ = state(&mut m, MixView::Tracks);
    play(&mut m, 8);
    let mut solo = state(&mut m, MixView::Tracks);
    solo.selected = Some(StripId::Track(3));
    out.push(Sample {
        name: "mixer-solo",
        about: "Solo in place: the bass soloed, the rest dimmed; the returns and the master stay",
        ui: screen(fonts(), solo)?,
    });
    m.set(StripId::Track(3), mx::SOLO, 0.0);
    // The second page of tracks.
    let mut ui = screen(fonts(), tracks)?;
    let next = next_button(&ui);
    tap(&mut ui, next.0, next.1);
    out.push(Sample { name: "mixer-tracks-page-2", about: "Track Mix, the second page: tracks 11 to 16 (the arrows, or a pad press selecting a track off the page)", ui });
    // The output picker open on the hat's pad channel.
    let mut ui = screen(fonts(), pads)?;
    let (x, y) = output_button(&ui, 2);
    tap(&mut ui, x, y);
    let _ = crate::take_actions(&mut ui);
    out.push(Sample {
        name: "mixer-output-picker",
        about: "The output picker on a pad channel: the master, a submix, any track, or nowhere",
        ui,
    });
    // Freeze and print: the bass frozen (a take of its compressor and EQ
    // playing in their place), the drum kit's freeze waiting for the
    // loop's start, the kick printing to a new sample.
    let bass = StripId::Track(3);
    m.freeze(bass, bass_take())
        .map_err(|_| "the bass would not freeze".to_string())?;
    play(&mut m, 8);
    let _ = state(&mut m, MixView::Tracks);
    play(&mut m, 4);
    let mut frozen = state(&mut m, MixView::Tracks);
    frozen.takes = true;
    if let Some(s) = frozen.strip_mut(StripId::Track(2)) {
        s.take = Some(TakeView {
            print: false,
            done: None,
        });
    }
    if let Some(s) = frozen.strip_mut(StripId::Track(0)) {
        s.take = Some(TakeView {
            print: true,
            done: Some(0.35),
        });
    }
    out.push(Sample {
        name: "mixer-freeze-print",
        about: "Freeze and print: the bass frozen (its effects rest; fader, pan and sends still work), the drum kit's freeze waiting for the loop's start, the kick printing to a new sample",
        ui: screen(fonts(), frozen.clone())?,
    });
    // The selected strip's menu: a second tap on its name.
    let mut ui = screen(fonts(), frozen)?;
    let (x, y) = name_button(&ui, 3);
    tap(&mut ui, x, y);
    let _ = crate::take_actions(&mut ui);
    out.push(Sample {
        name: "mixer-strip-menu",
        about:
            "The bass's menu (a tap on the selected strip's name): Thaw, or Print to a new sample",
        ui,
    });
    Ok(out)
}

/// Four beats of the bass as its frozen take: its tone, at 92 BPM.
fn bass_take() -> Frozen {
    let frames = (4.0 * 60.0 / 92.0 * RATE as f64) as usize;
    let hz = 55.0 * 4.0;
    let wave: Vec<f32> = (0..frames)
        .map(|k| 0.4 * (std::f32::consts::TAU * k as f32 * hz / RATE as f32).sin())
        .collect();
    Frozen {
        left: wave.clone(),
        right: wave,
        origin: 0.0,
        beats: 4.0,
        bpm: 92.0,
        rate: RATE,
    }
}

/// Where strip `col`'s name is (the top of the strip).
pub fn name_button(ui: &Ui, col: usize) -> (i32, i32) {
    let r = ui.rect_of(crate::MIXER).expect("the mixer page");
    let x = r.x + col as i32 * (crate::view::STRIP_W + 6) + crate::view::STRIP_W / 2;
    (x, r.y + crate::view::HEAD_H + 8 + 6 + 15)
}

/// Where the page's next arrow is, for the samples and tests.
pub fn next_button(ui: &Ui) -> (i32, i32) {
    let r = ui.rect_of(crate::MIXER).expect("the mixer page");
    let head_y = r.y + crate::view::HEAD_H / 2;
    (r.right() - 26, head_y)
}

/// Where band `band` of track `track`'s EQ is drawn on the EQ look.
pub fn eq_dot(ui: &Ui, track: usize, band: usize) -> Option<(i32, i32)> {
    let r = ui.rect_of(crate::MIXER)?;
    ui.get::<MixerView>(crate::MIXER)?.eq_dot_at(
        r,
        StripId::Track(track),
        crate::eq_look::Dot::Band(band),
    )
}

/// Where strip `col`'s output button is (the bottom of the strip).
pub fn output_button(ui: &Ui, col: usize) -> (i32, i32) {
    let r = ui.rect_of(crate::MIXER).expect("the mixer page");
    let x = r.x + col as i32 * (crate::view::STRIP_W + 6) + crate::view::STRIP_W / 2;
    (x, r.bottom() - 6 - 15)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_ui::SoftCanvas;

    #[test]
    fn the_samples_read_a_real_mixer_and_paint() {
        let s = samples(Fonts::embedded).unwrap();
        assert_eq!(s.len(), 13);
        let mut st = MixState::default();
        for mut sample in s {
            let mut c = SoftCanvas::new(1280, 800);
            let damage = sample.ui.paint(&mut c);
            assert!(!damage.is_empty(), "{}", sample.name);
            if sample.name == "mixer-tracks" {
                st = sample
                    .ui
                    .get::<MixerView>(crate::MIXER)
                    .unwrap()
                    .state()
                    .clone();
            }
            if sample.name == "mixer-output-picker" {
                assert_eq!(
                    sample.ui.get::<MixerView>(crate::MIXER).unwrap().picker(),
                    Some(StripId::Pad(2))
                );
            }
            if sample.name == "mixer-freeze-print" {
                let v = sample.ui.get::<MixerView>(crate::MIXER).unwrap().state();
                assert!(
                    v.strip(StripId::Track(3)).unwrap().frozen,
                    "read from the mixer"
                );
                assert!(!v.strip(StripId::Track(4)).unwrap().frozen);
            }
            if sample.name == "mixer-strip-menu" {
                assert_eq!(
                    sample.ui.get::<MixerView>(crate::MIXER).unwrap().menu(),
                    Some(StripId::Track(3))
                );
            }
            if sample.name == "mixer-effects" {
                let v = sample.ui.get::<MixerView>(crate::MIXER).unwrap();
                assert_eq!(v.look(), crate::Look::Effects);
            }
            if sample.name == "mixer-tracks-page-2" {
                assert_eq!(
                    sample.ui.get::<MixerView>(crate::MIXER).unwrap().first(),
                    10
                );
            }
        }
        let bass = st.strip(StripId::Track(3)).unwrap();
        assert_eq!(bass.inserts.len(), 2);
        assert_eq!(bass.inserts[0].key.as_deref(), Some("Kick"));
        assert!(bass.peak[0] > 0.1, "meters read: {:?}", bass.peak);
        assert!(st.strip(StripId::Track(6)).unwrap().mute);
        assert!(!st.strip(StripId::Track(6)).unwrap().audible);
        assert_eq!(st.strip(StripId::Track(0)).unwrap().output_label(), "Sub 1");
        assert!(st.cost_pct.is_some_and(|c| c > 0.0));
        assert!(st.master.peak[0] > 0.0);
    }

    #[test]
    fn actions_drive_the_mixer() {
        let mut m = song_mixer();
        let mut ui = screen(
            Fonts::embedded(),
            MixState::from_mixer(&mut m, MixView::Pads, &name),
        )
        .unwrap();
        let (x, y) = output_button(&ui, 2);
        tap(&mut ui, x, y);
        let r = ui.rect_of(crate::MIXER).unwrap();
        let b = ui
            .get::<MixerView>(crate::MIXER)
            .unwrap()
            .picker_button(r, (output::MASTER as u8, 0))
            .unwrap();
        tap(&mut ui, b.x + b.w / 2, b.y + b.h / 2);
        for a in crate::take_actions(&mut ui) {
            assert!(crate::apply(&mut m, &a), "{a:?}");
        }
        play(&mut m, 1);
        assert_eq!(m.param(StripId::Pad(2), mx::OUTPUT), output::MASTER);
    }

    #[test]
    fn q_links_turn_the_selected_strip_on_the_screen_and_in_the_mixer() {
        let mut m = song_mixer();
        let mut st = MixState::from_mixer(&mut m, MixView::Tracks, &name);
        st.selected = Some(StripId::Track(3));
        let mut ui = screen(Fonts::embedded(), st).unwrap();
        let q = crate::qlinks(&ui);
        assert_eq!(
            (q.strip, q.name.as_str()),
            (Some(StripId::Track(3)), "Bass")
        );
        let before = q.knobs[0].value;
        assert_eq!(before, m.param(StripId::Track(3), mx::VOLUME));
        let mut c = SoftCanvas::new(1280, 800);
        ui.paint(&mut c);
        let a = crate::qlink_turn(&mut ui, 0, -3, false).expect("the volume moves");
        assert!(
            ui.paint(&mut c).iter().any(|r| r.w > 0),
            "the fader is drawn again"
        );
        let after = crate::qlinks(&ui).knobs[0].value;
        assert!(after < before, "{before} -> {after}");
        assert!(
            crate::take_actions(&mut ui).is_empty(),
            "the caller sends a Q-Link's action itself"
        );
        assert!(crate::apply(&mut m, &a));
        play(&mut m, 1);
        assert_eq!(m.param(StripId::Track(3), mx::VOLUME), after);
        let a = crate::qlink_to(&mut ui, 1, 1.0).expect("pan moves");
        assert!(crate::apply(&mut m, &a));
        play(&mut m, 1);
        assert_eq!(m.param(StripId::Track(3), mx::PAN), 100.0);
        assert_eq!(crate::qlinks(&ui).knobs[1].text, "R100");
    }
}
