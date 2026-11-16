//! Sample models for the preview and the screen tests: a small rig with
//! the 5-pin jacks, a USB keyboard and a drum machine named by the
//! project but not plugged in.

use std::time::Duration;

use mpfree_controller::mpc_one::{button, ControlEvent};
use mpfree_midi::cv::{CvAssign, Priority, Scale};
use mpfree_midi::monitor::{Dir, Event, Feed, Mpc, To};
use mpfree_midi::port::Ports;
use mpfree_midi::setup::{ClockOut, MidiSetup, PortInput, TempoSource, TrackOut, DIN_NAME};
use mpfree_midi::DIN;

use crate::{IoModel, Live, PortLive};

pub fn rig() -> IoModel {
    let mut live = Live::default();
    live.ports[0].last_in = Some(Duration::from_millis(950));
    live.ports.push(PortLive {
        name: "Keystep 37".into(),
        kind: "USB".into(),
        has_in: true,
        has_out: true,
        last_in: None,
    });
    live.tracks = ["Drums", "Bass", "Pads", "Lead", "Vox chops", "Perc"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    live.clock_in = Some((DIN_NAME.into(), 120.0));
    let mut setup = MidiSetup {
        tempo: TempoSource::Port(DIN_NAME.into()),
        ..MidiSetup::default()
    };
    setup.set_port("Keystep 37", |p| p.input = PortInput::LearnOnly);
    setup.set_port(DIN_NAME, |p| p.clock_out = ClockOut::WhilePlaying);
    setup.tracks[1] = Some(TrackOut {
        program: Some(4),
        ..TrackOut::new(DIN_NAME, 1)
    });
    setup.tracks[3] = Some(TrackOut::new("Digitakt", 9));
    setup.cv[0] = CvAssign::Pitch {
        track: 1,
        scale: Scale::VoltsPerOctave(1.0),
        zero_note: 36,
        bend_semis: 2,
        priority: Priority::Last,
    };
    setup.cv[1] = CvAssign::Gate {
        track: 1,
        volts: 5.0,
        retrigger_ms: 0.0,
    };
    setup.cv[4] = CvAssign::Clock {
        ppqn: 4,
        volts: 5.0,
        ms: 5.0,
    };
    IoModel {
        setup,
        live,
        warnings: Vec::new(),
        gen: 0,
        monitor: Default::default(),
    }
}

/// `m` with its Monitor fed, as though the app had run for a second: a
/// chord and the mod wheel from the USB keyboard (its input is Learn
/// only, so they play nothing) and its CC 74, learned as the Drums
/// filter's Cutoff; a drum machine on the 5-pin jacks playing the armed
/// Drums kit (one note no pad has); the Bass track playing out of the
/// 5-pin jacks on channel 2 (program 5 first); clock arriving on the 5-pin
/// jacks at 120 BPM (the tempo follows it) and an Akai SysEx. Each line
/// says where it went.
pub fn monitored(mut m: IoModel) -> IoModel {
    // The knob's MIDI Learn, unless `m` has it already.
    let cutoff = "port Keystep 37 | ch 1 | cc 74 | t1.fx1.0 | 0 1 | absolute | jump";
    let cutoff = mpfree_midi::learn::Saved::parse(cutoff).expect("a learned control");
    let target = cutoff.target;
    if !m.setup.learn.iter().any(|l| l.target == target) && m.setup.learned(cutoff) {
        m.live.learn_names.push("Cutoff (Filter, Drums)".into());
    }
    let mut ports = Ports::new();
    ports.set_din(true);
    let keys = ports
        .plugged("Keystep 37", true, true)
        .expect("room for a port");
    m.monitor.set_ports(ports.all());
    m.live.built.monitor = true;
    let mut f = Feed::new();
    let mut fed = |port, dir, t, bytes: &[u8], to| {
        // Fed a message at a time, as the app routes each.
        for b in bytes {
            if f.bytes(port, dir, t, &[*b]) {
                f.to(to);
            }
        }
    };
    for i in 0..48 {
        // 120 BPM: a tick every 1/48 s.
        fed(DIN, Dir::In, 0.02 + i as f64 / 48.0, &[0xF8], To::Follows);
    }
    let bass = To::SentBy(1);
    fed(DIN, Dir::Out, 0.10, &[0xC1, 4], bass);
    let sysex = [0xF0, 0x47, 0x7F, 0x00, 0x01, 0xF7];
    fed(DIN, Dir::In, 0.25, &sysex, To::Ignored);
    let learns = To::LearnOnly;
    fed(keys, Dir::In, 0.40, &[0x90, 60, 96], learns);
    fed(keys, Dir::In, 0.41, &[0x90, 64, 88, 67, 90], learns);
    fed(keys, Dir::In, 0.55, &[0xB0, 1, 12], learns);
    fed(keys, Dir::In, 0.59, &[0xB0, 1, 30], learns);
    let learned = To::Learned(target);
    fed(keys, Dir::In, 0.63, &[0xB0, 74, 60], learned);
    fed(keys, Dir::In, 0.67, &[0xB0, 74, 72], learned);
    // The drum machine: pad 1's note, then one no pad of the kit has.
    let pad1 = To::Track {
        track: 0,
        pad: Some(0),
    };
    fed(DIN, Dir::In, 0.70, &[0x99, 37, 100], pad1);
    fed(DIN, Dir::In, 0.74, &[0x99, 20, 90], To::NoPad { track: 0 });
    fed(DIN, Dir::Out, 0.78, &[0x91, 36, 110], bass);
    fed(keys, Dir::In, 0.82, &[0xE0, 0x00, 0x50], learns);
    fed(DIN, Dir::In, 0.86, &[0x89, 37, 0], pad1);
    fed(DIN, Dir::Out, 0.90, &[0x81, 36, 0], bass);
    fed(keys, Dir::In, 0.95, &[0x80, 60, 40], learns);
    f.drain_into(&mut m.monitor, 1.0);
    m
}

/// `m` (fed by [`monitored`]) with what debug mode adds, as on the Dev
/// tab's MIDI page: the keyboard plugged in, a pad hit and its light, PLAY
/// pressed and lit, a Q-Link turned, CV 1 set and the keep-alive.
pub fn debugged(mut m: IoModel) -> IoModel {
    let keys = m
        .monitor
        .ports()
        .iter()
        .find(|p| p.name == "Keystep 37")
        .map_or(mpfree_midi::PortId(1), |p| p.id);
    let mut f = Feed::new();
    f.debug = true;
    f.event(1.02, Event::Plugged(keys));
    let control = |ev, bytes| Mpc::Control { ev, bytes };
    let pad = ControlEvent::PadDown {
        pad: 5,
        velocity: 87,
    };
    f.mpc(Dir::In, 1.10, control(pad, [0x99, 0x2E, 0x57]));
    f.mpc(
        Dir::Out,
        1.10,
        Mpc::PadColour {
            pad: 5,
            rgb: [127, 127, 127],
        },
    );
    let play = ControlEvent::Button {
        note: button::PLAY,
        pressed: true,
    };
    f.mpc(Dir::In, 1.20, control(play, [0x90, button::PLAY, 0x7F]));
    f.mpc(
        Dir::Out,
        1.21,
        Mpc::Light {
            button: button::PLAY,
            value: 127,
        },
    );
    let knob = ControlEvent::QLinkTurn { knob: 0, delta: 2 };
    f.mpc(Dir::In, 1.30, control(knob, [0xB0, 0x10, 0x02]));
    f.mpc(
        Dir::Out,
        1.40,
        Mpc::Cv {
            out: 0,
            value: 3277,
        },
    );
    f.mpc(Dir::Out, 1.50, Mpc::KeepAlive);
    f.drain_into(&mut m.monitor, 1.6);
    m
}
