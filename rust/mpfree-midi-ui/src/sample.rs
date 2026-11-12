//! Sample models for the preview and the screen tests: a small rig with
//! the 5-pin jacks, a USB keyboard and a drum machine named by the
//! project but not plugged in.

use std::time::Duration;

use mpfree_midi::cv::{CvAssign, Priority, Scale};
use mpfree_midi::setup::{ClockOut, MidiSetup, PortInput, TempoSource, TrackOut, DIN_NAME};

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
    }
}
