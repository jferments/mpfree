//! The Sync section: where the tempo comes from, what following does, the
//! clock arriving now, and each port's clock out.

use std::time::Duration;

use mpfree_midi::setup::{ClockOut, TempoSource};

use crate::form::{clamp_step, Field, Form, Row, Tone};
use crate::midi::ports;
use crate::IoModel;

pub(crate) struct Sync;

/// The tempo's own source, as the screen says it.
pub(crate) const OWN: &str = "MPFree";

/// The tempo's choices: MPFree's own, then each port with an input.
fn sources(m: &IoModel) -> Vec<TempoSource> {
    let mut v = vec![TempoSource::Internal];
    for (name, live) in ports(m) {
        if live.is_none_or(|l| l.has_in) {
            v.push(TempoSource::Port(name));
        }
    }
    v
}

fn source_text(s: &TempoSource) -> &str {
    match s {
        TempoSource::Internal => OWN,
        TempoSource::Port(p) => p,
    }
}

impl Form for Sync {
    fn label(&self) -> &'static str {
        crate::SYNC
    }

    fn widths(&self) -> &'static [i32] {
        &[330, 230, 230]
    }

    fn banner(&self, m: &IoModel) -> Option<String> {
        let b = m.live.built;
        let what = match (b.clock_in, b.clock_out) {
            (true, true) => return None,
            (false, false) => {
                "Following another device's clock isn't built yet, and \
                 sending clock is off with MIDI out"
            }
            (false, true) => "Following another device's clock isn't built yet",
            (true, false) => "Sending clock is off with MIDI out",
        };
        Some(format!(
            "{what}: what you set here is kept with the project."
        ))
    }

    fn rows(&self, m: &IoModel, _now: Duration) -> Vec<Row> {
        let s = &m.setup;
        let built = m.live.built;
        let src = sources(m);
        let at = src.iter().position(|x| *x == s.tempo).unwrap_or(0);
        let mut rows = vec![Row::heading("Tempo", vec!["From", ""])];
        let tempo = Row::item("tempo", "Tempo from").field(Field::choice(
            "Tempo from",
            source_text(&s.tempo),
            at,
            src.len(),
        ));
        rows.push(match &s.tempo {
            TempoSource::Internal => tempo.note("MPFree's own tempo", Tone::Faint),
            TempoSource::Port(p) if m.live.port(p).is_none() => tempo.note(
                format!("follows nothing: {p} is not plugged in"),
                Tone::Warn,
            ),
            TempoSource::Port(_) if !built.clock_in => {
                tempo.note("follows nothing yet: no clock in", Tone::Warn)
            }
            TempoSource::Port(p) => match &m.live.clock_in {
                Some((q, bpm)) if q == p => {
                    tempo.note(format!("{bpm:.1} BPM from {p}"), Tone::Normal)
                }
                _ => tempo.note(format!("waiting for clock from {p}"), Tone::Warn),
            },
        });
        let following = s.tempo != TempoSource::Internal;
        rows.push(
            Row::item("follow", "While following")
                .note("start, stop and position from it", Tone::Faint)
                .field(
                    Field::choice(
                        "Start/stop",
                        format!("Start and stop: {}", on_off(s.follow_start_stop)),
                        s.follow_start_stop as usize,
                        2,
                    )
                    .off(following),
                )
                .field(
                    Field::choice(
                        "Position",
                        format!("Position: {}", on_off(s.follow_position)),
                        s.follow_position as usize,
                        2,
                    )
                    .off(following),
                ),
        );
        let arriving = Row::item("arriving", "Clock arriving");
        rows.push(match &m.live.clock_in {
            Some((p, bpm)) => arriving
                .note(format!("{bpm:.1} BPM on {p}"), Tone::Normal)
                .light(true),
            None => arriving.note("none", Tone::Faint).light(false),
        });
        rows.push(Row::heading("Clock out", vec!["Sends"]));
        for (name, live) in ports(m) {
            if live.is_some_and(|l| !l.has_out) {
                continue;
            }
            let p = m.setup.port(&name);
            let i = ClockOut::ALL
                .iter()
                .position(|c| *c == p.clock_out)
                .unwrap_or(0);
            let row = Row::item(format!("clock:{name}"), name.clone()).field(Field::choice(
                "Clock out",
                p.clock_out.label(),
                i,
                ClockOut::ALL.len(),
            ));
            rows.push(match (live, p.clock_out) {
                (None, _) => row.note("not plugged in: its settings are kept", Tone::Warn),
                (_, ClockOut::Off) => row.note("sends no clock", Tone::Faint),
                (_, _) if !built.clock_out => {
                    row.note("sends nothing: MIDI out is off", Tone::Warn)
                }
                (_, ClockOut::WhilePlaying) => {
                    row.note("clock, start and stop while playing", Tone::Normal)
                }
                (_, ClockOut::Always) => {
                    row.note("clock all the time; start and stop with play", Tone::Normal)
                }
            });
        }
        rows
    }

    fn step(&self, m: &mut IoModel, key: &str, col: usize, delta: i32) -> bool {
        match key {
            "tempo" => {
                let src = sources(m);
                let at = src.iter().position(|x| *x == m.setup.tempo).unwrap_or(0);
                let to = clamp_step(at, delta, src.len());
                // A tap on the last source goes round to the first.
                let to = if to == at && delta == 1 { 0 } else { to };
                let changed = src[to] != m.setup.tempo;
                m.setup.tempo = src[to].clone();
                changed
            }
            "follow" => {
                if delta == 0 {
                    return false;
                }
                if col == 0 {
                    m.setup.follow_start_stop = !m.setup.follow_start_stop;
                } else {
                    m.setup.follow_position = !m.setup.follow_position;
                }
                true
            }
            _ => {
                let Some(name) = key.strip_prefix("clock:") else {
                    return false;
                };
                let name = name.to_string();
                let before = m.setup.port(&name).clock_out;
                m.setup.set_port(&name, |p| {
                    let n = ClockOut::ALL.len() as i32;
                    let i = ClockOut::ALL
                        .iter()
                        .position(|c| *c == p.clock_out)
                        .unwrap_or(0) as i32;
                    p.clock_out = ClockOut::ALL[(i + delta).rem_euclid(n) as usize];
                });
                m.setup.port(&name).clock_out != before
            }
        }
    }

    fn live(&self, m: &IoModel, _now: Duration) -> u64 {
        match &m.live.clock_in {
            Some((_, bpm)) => 1 + (bpm * 10.0).round() as u64,
            None => 0,
        }
    }
}

fn on_off(b: bool) -> &'static str {
    if b {
        "On"
    } else {
        "Off"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::form::testing::{paint, tap, AREA};
    use crate::form::FormSection;
    use crate::{Built, Live, PortLive};
    use mpfree_midi::setup::{MidiSetup, DIN_NAME};
    use mpfree_mixer_ui::{IoReply, IoSection};
    use std::sync::{Arc, Mutex};

    fn model() -> crate::Shared {
        let mut live = Live::default();
        live.ports.push(PortLive {
            name: "Keystep 37".into(),
            kind: "USB".into(),
            has_in: true,
            has_out: false,
            last_in: None,
        });
        Arc::new(Mutex::new(IoModel {
            setup: MidiSetup::default(),
            live,
            ..IoModel::default()
        }))
    }

    fn row(m: &crate::Shared, key: &str) -> Row {
        Sync.rows(&m.lock().unwrap(), Duration::ZERO)
            .into_iter()
            .find(|r| r.key == key)
            .expect(key)
    }

    #[test]
    fn tempo_steps_through_ports_with_an_input_and_back() {
        let s = model();
        let mut sec = FormSection::new(Sync, s.clone());
        let _ = paint(&sec);
        let f = sec.field_rect("tempo", 0, AREA).unwrap();
        let (x, y) = (f.x + 10, f.y + 10);
        assert_eq!(tap(&mut sec, x, y), IoReply::Redraw);
        assert_eq!(
            s.lock().unwrap().setup.tempo,
            TempoSource::Port(DIN_NAME.into())
        );
        assert_eq!(row(&s, "tempo").note, "follows nothing yet: no clock in");
        tap(&mut sec, x, y);
        assert_eq!(
            s.lock().unwrap().setup.tempo,
            TempoSource::Port("Keystep 37".into())
        );
        tap(&mut sec, x, y);
        assert_eq!(s.lock().unwrap().setup.tempo, TempoSource::Internal);
    }

    /// The banner says which half of clock isn't working, if either.
    #[test]
    fn the_banner_names_what_is_missing() {
        let s = model();
        let banner = |s: &crate::Shared| Sync.banner(&s.lock().unwrap());
        assert!(banner(&s).unwrap().starts_with(
            "Following another device's clock isn't built yet, and sending clock is off"
        ));
        s.lock().unwrap().live.built.clock_out = true;
        assert_eq!(
            banner(&s).unwrap(),
            "Following another device's clock isn't built yet: what you set here is \
             kept with the project."
        );
        s.lock().unwrap().live.built.clock_in = true;
        assert_eq!(banner(&s), None);
    }

    #[test]
    fn follow_settings_only_work_while_following() {
        let s = model();
        assert!(!row(&s, "follow").fields[0].enabled);
        s.lock().unwrap().setup.tempo = TempoSource::Port(DIN_NAME.into());
        assert!(row(&s, "follow").fields[0].enabled);
        assert_eq!(row(&s, "follow").fields[1].text, "Position: On");
        assert!(Sync.step(&mut s.lock().unwrap(), "follow", 1, 1));
        assert!(!s.lock().unwrap().setup.follow_position);
    }

    #[test]
    fn clock_arriving_and_following_it() {
        let s = model();
        {
            let mut m = s.lock().unwrap();
            m.setup.tempo = TempoSource::Port(DIN_NAME.into());
            m.live.built = Built {
                clock_in: true,
                ..Built::default()
            };
        }
        assert_eq!(row(&s, "tempo").note, "waiting for clock from MIDI");
        s.lock().unwrap().live.clock_in = Some((DIN_NAME.into(), 121.04));
        assert_eq!(row(&s, "tempo").note, "121.0 BPM from MIDI");
        assert_eq!(row(&s, "arriving").light, Some(true));
        let mut sec = FormSection::new(Sync, s.clone());
        assert_eq!(sec.tick(Duration::ZERO), IoReply::Redraw);
        assert_eq!(sec.tick(Duration::ZERO), IoReply::Ignored);
        s.lock().unwrap().live.clock_in = Some((DIN_NAME.into(), 121.5));
        assert_eq!(sec.tick(Duration::ZERO), IoReply::Redraw);
    }

    #[test]
    fn clock_out_lists_ports_with_an_output_and_cycles() {
        let s = model();
        let rows = Sync.rows(&s.lock().unwrap(), Duration::ZERO);
        assert!(rows.iter().any(|r| r.key == "clock:MIDI"));
        assert!(!rows.iter().any(|r| r.key == "clock:Keystep 37"));
        assert!(Sync.step(&mut s.lock().unwrap(), "clock:MIDI", 0, 1));
        assert_eq!(
            s.lock().unwrap().setup.port(DIN_NAME).clock_out,
            ClockOut::WhilePlaying
        );
        assert_eq!(row(&s, "clock:MIDI").note, "sends nothing: MIDI out is off");
        Sync.step(&mut s.lock().unwrap(), "clock:MIDI", 0, 1);
        Sync.step(&mut s.lock().unwrap(), "clock:MIDI", 0, 1);
        assert!(s.lock().unwrap().setup.ports.is_empty());
    }
}
