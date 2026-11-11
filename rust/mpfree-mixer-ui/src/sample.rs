//! Sample mixer screens with a real mixer playing behind them, for the
//! workstation preview (`mpfree-mixer-preview`) and the tests: a song's
//! routing (drums into a drum bus, the bass keyed by the kick, music into
//! a second submix, reverb and delay returns, pad channels into the kit
//! track, the line input armed), its meters after a few blocks, and the
//! mixer's own cost.

use mpfree_fx::ids::mixer::{self as mx, monitor, output, source};
use mpfree_fx::mpfree_node::Ctx;
use mpfree_fx::{ids, Cloud, Compressor, Delay, Drive, Eq, Mixer, MixerConfig, StripId};
use mpfree_ui::widgets::{Chip, Header};
use mpfree_ui::{Align, Fonts, Theme, Tone, Touch, Ui};

use crate::state::{default_name, MixState, MixView};
use crate::view::{MixerIcon, MixerView};

/// The music tabs, as the approved layout spec lists them.
pub const TABS: [&str; 5] = ["Play", "Clips", "Synth", "Looper", "Sample"];

/// A preview page: the header with the mixer icon beside it, the mixer,
/// the hint line.
const PREVIEW_LAYOUT: &str = "col(row(header:*, mixicon:64):60, mixer:*, hint:26)";

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

/// The app's screen around the mixer: the header (no tab lit while the
/// mixer is open), the icon, the hint.
pub fn screen(fonts: Fonts, st: MixState) -> Result<Ui, String> {
    let mut ui = Ui::new(Theme::dark(), fonts, 1280, 800);
    let mut header = Header::new(&TABS);
    header.active = usize::MAX;
    header.chips = vec![
        Chip::new("92 BPM", Tone::Normal),
        Chip::new("Level -6 dB", Tone::Normal),
        Chip::new("Kit 808 Basic · A", Tone::Accent),
        Chip::new("Night drive", Tone::Normal),
    ];
    ui.add("header", header);
    ui.add("mixicon", MixerIcon { open: true });
    ui.add(
        "hint",
        mpfree_ui::widgets::Label::new(crate::MIXER_HINT, Tone::Dim, Align::Left),
    );
    ui.add(crate::MIXER, MixerView::new());
    ui.add_page(crate::MIXER_PAGE, PREVIEW_LAYOUT)?;
    ui.show(crate::MIXER_PAGE)?;
    crate::update(&mut ui, st);
    Ok(ui)
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
    let mut out = vec![
        Sample { name: "mixer-tracks", about: "Track Mix while the song plays: the bass selected, its compressor keyed by the kick (SC), the vocal track muted, a pre-fader send on the chords, the mixer's own CPU share (Dev tools on)", ui: screen(fonts(), tracks.clone())? },
        Sample { name: "mixer-pads", about: "Pad Mix (SHIFT + TRACK MIX): eight pad channels into the kit track, the crash straight to the drum bus", ui: screen(fonts(), pads.clone())? },
        Sample { name: "mixer-inputs", about: "Inputs: the line input armed and heard through a compressor (Auto monitoring); the mic on input 2 into the vocal track, not monitored", ui: screen(fonts(), inputs)? },
        Sample { name: "mixer-submixes", about: "Submixes and returns: the drum bus with glue and drive, the music bus, the reverb and delay returns", ui: screen(fonts(), buses)? },
    ];
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
    Ok(out)
}

/// Where the page's next arrow is, for the samples and tests.
pub fn next_button(ui: &Ui) -> (i32, i32) {
    let r = ui.rect_of(crate::MIXER).expect("the mixer page");
    let head_y = r.y + crate::view::HEAD_H / 2;
    (r.right() - 26, head_y)
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
        assert_eq!(s.len(), 7);
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
