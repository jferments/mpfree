//! Sample models for the preview and the screen tests: a small rig with
//! the 5-pin jacks, a USB keyboard and a drum machine named by the
//! project but not plugged in.

use std::time::Duration;

use mpfree_midi::cv::{CvAssign, Priority, Scale};
use mpfree_midi::monitor::{Dir, Feed};
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
/// chord and the mod wheel from the USB keyboard, the Bass track playing
/// out of the 5-pin jacks on channel 2 (program 5 first), clock arriving
/// on the 5-pin jacks at 120 BPM and an Akai SysEx.
pub fn monitored(mut m: IoModel) -> IoModel {
    let mut ports = Ports::new();
    ports.set_din(true);
    let keys = ports
        .plugged("Keystep 37", true, true)
        .expect("room for a port");
    m.monitor.set_ports(ports.all());
    m.live.built.monitor = true;
    let mut f = Feed::new();
    for i in 0..48 {
        // 120 BPM: a tick every 1/48 s.
        f.bytes(DIN, Dir::In, 0.02 + i as f64 / 48.0, &[0xF8]);
    }
    f.bytes(DIN, Dir::Out, 0.10, &[0xC1, 4]);
    f.bytes(DIN, Dir::In, 0.25, &[0xF0, 0x47, 0x7F, 0x00, 0x01, 0xF7]);
    f.bytes(keys, Dir::In, 0.40, &[0x90, 60, 96]);
    f.bytes(keys, Dir::In, 0.41, &[0x90, 64, 88, 67, 90]);
    for (i, v) in [12u8, 30, 47, 63].iter().enumerate() {
        f.bytes(keys, Dir::In, 0.55 + i as f64 * 0.04, &[0xB0, 1, *v]);
    }
    f.bytes(DIN, Dir::Out, 0.78, &[0x91, 36, 110]);
    f.bytes(keys, Dir::In, 0.82, &[0xE0, 0x00, 0x50]);
    f.bytes(DIN, Dir::Out, 0.90, &[0x81, 36, 0]);
    f.bytes(keys, Dir::In, 0.95, &[0x80, 60, 40]);
    f.drain_into(&mut m.monitor, 1.0);
    m
}
