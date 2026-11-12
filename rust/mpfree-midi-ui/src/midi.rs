//! The MIDI section: each port's input and thru, where each track plays
//! ("Plays on", the port and channel as one setting, the owner's rule),
//! with its program, and the controls learned with MIDI Learn
//! (`crate::learn`).

use std::time::Duration;

use mpfree_midi::output::MAX_TRACKS;
use mpfree_midi::setup::{PortInput, TrackOut};

use crate::form::{clamp_step, Choice, Field, Form, Group, Picker, Row, Tone};
use crate::{IoModel, PortLive, LIGHT};

pub(crate) struct Midi;

/// What a track that sends nothing out plays.
pub(crate) const INSIDE: &str = "Inside MPFree";

/// Every port the section shows: the ones plugged in now, then the ones
/// the project names that aren't.
pub(crate) fn ports(m: &IoModel) -> Vec<(String, Option<&PortLive>)> {
    let mut out: Vec<(String, Option<&PortLive>)> = m
        .live
        .ports
        .iter()
        .map(|p| (p.name.clone(), Some(p)))
        .collect();
    let s = &m.setup;
    let named = s
        .ports
        .iter()
        .map(|p| p.name.as_str())
        .chain(s.tracks.iter().flatten().map(|t| t.port.as_str()))
        .chain(match &s.tempo {
            mpfree_midi::setup::TempoSource::Port(p) => Some(p.as_str()),
            _ => None,
        });
    for n in named {
        if !out.iter().any(|p| p.0 == n) {
            out.push((n.to_string(), None));
        }
    }
    out
}

/// Ports a track can play out of: those with an output, or not plugged in
/// (whose output can't be known until they are).
fn out_ports(m: &IoModel) -> Vec<(String, bool)> {
    ports(m)
        .into_iter()
        .filter(|p| p.1.is_none_or(|l| l.has_out))
        .map(|p| (p.0, p.1.is_some()))
        .collect()
}

/// A track's "Plays on" choices in order: inside, then each output port's
/// 16 channels.
fn plays_on_list(m: &IoModel) -> Vec<Option<TrackOut>> {
    let mut v = vec![None];
    for (p, _) in out_ports(m) {
        for ch in 0..16 {
            v.push(Some(TrackOut::new(&p, ch)));
        }
    }
    v
}

fn plays_on_index(m: &IoModel, t: &Option<TrackOut>) -> usize {
    let key = t.as_ref().map(|t| (t.port.as_str(), t.channel));
    plays_on_list(m)
        .iter()
        .position(|o| o.as_ref().map(|t| (t.port.as_str(), t.channel)) == key)
        .unwrap_or(0)
}

fn tracks_shown(m: &IoModel) -> usize {
    let routed = m
        .setup
        .tracks
        .iter()
        .rposition(Option::is_some)
        .map_or(0, |i| i + 1);
    m.live.tracks.len().max(routed).min(MAX_TRACKS)
}

fn track_name(m: &IoModel, i: usize) -> String {
    m.live
        .tracks
        .get(i)
        .cloned()
        .unwrap_or_else(|| format!("Track {}", i + 1))
}

fn parse_key<'a>(key: &'a str, kind: &str) -> Option<&'a str> {
    key.strip_prefix(kind)?.strip_prefix(':')
}

impl Form for Midi {
    fn label(&self) -> &'static str {
        crate::MIDI
    }

    fn widths(&self) -> &'static [i32] {
        &[330, 270, 150]
    }

    fn banner(&self, m: &IoModel) -> Option<String> {
        if !m.warnings.is_empty() {
            let n = m.warnings.len();
            return Some(format!(
                "{n} line{} of this project's MIDI settings couldn't be read and \
                 {} skipped (the log names {}).",
                if n == 1 { "" } else { "s" },
                if n == 1 { "was" } else { "were" },
                if n == 1 { "it" } else { "each" },
            ));
        }
        (!m.live.built.notes_out).then(|| {
            "MIDI out is off: tracks still play inside MPFree, and what you set \
             here is kept with the project."
                .to_string()
        })
    }

    fn rows(&self, m: &IoModel, now: Duration) -> Vec<Row> {
        let built = m.live.built;
        let mut rows = vec![Row::heading("Ports", vec!["Input", "Thru"])];
        for (name, live) in ports(m) {
            let p = m.setup.port(&name);
            let has_in = live.is_none_or(|l| l.has_in);
            let has_out = live.is_none_or(|l| l.has_out);
            let input = PortInput::ALL
                .iter()
                .position(|x| *x == p.input)
                .unwrap_or(0);
            let mut row = Row::item(format!("port:{name}"), name.clone())
                .field(
                    Field::choice("Input", p.input.label(), input, PortInput::ALL.len())
                        .off(has_in),
                )
                .field(
                    Field::choice(
                        "Thru",
                        if p.thru { "On" } else { "Off" },
                        p.thru as usize,
                        2,
                    )
                    .off(has_in && has_out),
                );
            row = match live {
                None => row.note("not plugged in: its settings are kept", Tone::Warn),
                Some(_) if p.thru && !built.thru => row.note("thru does nothing yet", Tone::Warn),
                Some(l) => row.note(l.kind.clone(), Tone::Normal),
            };
            if let Some(l) = live.filter(|l| l.has_in) {
                row = row.light(l.last_in.is_some_and(|t| now.saturating_sub(t) < LIGHT));
            }
            rows.push(row);
        }
        rows.push(Row::heading("Tracks out", vec!["Plays on", "Program"]));
        for i in 0..tracks_shown(m) {
            let t = &m.setup.tracks[i];
            let mut row = Row::item(format!("track:{i}"), track_name(m, i));
            let n = plays_on_list(m).len();
            let at = plays_on_index(m, t);
            match t {
                None => {
                    row = row
                        .note("plays its own sound", Tone::Faint)
                        .field(Field::choice("Plays on", INSIDE, at, n).picker())
                        .field(Field::choice("Program", "—", 0, 129).off(false));
                }
                Some(o) => {
                    let plugged = m.live.port(&o.port).is_some();
                    row = if !plugged {
                        row.note(
                            format!("sends nothing: {} is not plugged in", o.port),
                            Tone::Warn,
                        )
                    } else if !built.notes_out {
                        row.note("sends nothing: MIDI out is off", Tone::Warn)
                    } else if !m.live.port(&o.port).is_some_and(|l| l.has_out) {
                        row.note(
                            format!("sends nothing: no output to {} yet", o.port),
                            Tone::Warn,
                        )
                    } else {
                        row.note(format!("plays out of {}", o.port), Tone::Normal)
                    };
                    let prog = o
                        .program
                        .map_or("—".to_string(), |p| (p as u16 + 1).to_string());
                    let pat = o.program.map_or(0, |p| p as usize + 1);
                    row = row
                        .field(
                            Field::choice("Plays on", o.label(), at, n)
                                .picker()
                                .warn(!plugged),
                        )
                        .field(Field::choice("Program", prog, pat, 129));
                }
            }
            rows.push(row);
        }
        crate::learn::rows(m, &mut rows);
        rows
    }

    fn step(&self, m: &mut IoModel, key: &str, col: usize, delta: i32) -> bool {
        if let Some(changed) = crate::learn::step(m, key, col, delta) {
            return changed;
        }
        if let Some(name) = parse_key(key, "port") {
            let name = name.to_string();
            let before = m.setup.port(&name);
            m.setup.set_port(&name, |p| match col {
                0 => {
                    let n = PortInput::ALL.len() as i32;
                    let i = PortInput::ALL
                        .iter()
                        .position(|x| *x == p.input)
                        .unwrap_or(0) as i32;
                    p.input = PortInput::ALL[(i + delta).rem_euclid(n) as usize];
                }
                _ => {
                    if delta != 0 {
                        p.thru = !p.thru;
                    }
                }
            });
            return m.setup.port(&name) != before;
        }
        let Some(i) = parse_key(key, "track").and_then(|i| i.parse::<usize>().ok()) else {
            return false;
        };
        if i >= MAX_TRACKS {
            return false;
        }
        let before = m.setup.tracks[i].clone();
        match col {
            0 => {
                let list = plays_on_list(m);
                let at = plays_on_index(m, &m.setup.tracks[i]);
                let to = clamp_step(at, delta, list.len());
                let program = before.as_ref().and_then(|t| t.program);
                let bank = before.as_ref().and_then(|t| t.bank);
                m.setup.tracks[i] = list[to].clone().map(|mut t| {
                    t.program = program;
                    t.bank = bank;
                    t
                });
            }
            _ => {
                if let Some(t) = m.setup.tracks[i].as_mut() {
                    let at = t.program.map_or(0, |p| p as usize + 1);
                    let to = clamp_step(at, delta, 129);
                    t.program = (to > 0).then(|| (to - 1) as u8);
                }
            }
        }
        m.setup.tracks[i] != before
    }

    fn picker(&self, m: &IoModel, key: &str, col: usize) -> Option<Picker> {
        let i: usize = parse_key(key, "track")?.parse().ok()?;
        if col != 0 || i >= MAX_TRACKS {
            return None;
        }
        let t = &m.setup.tracks[i];
        let mut ports = vec![Choice {
            text: INSIDE.to_string(),
            on: t.is_none(),
            enabled: true,
        }];
        for (p, plugged) in out_ports(m) {
            ports.push(Choice {
                text: if plugged {
                    p.clone()
                } else {
                    format!("{p} (not plugged in)")
                },
                on: t.as_ref().is_some_and(|t| t.port == p),
                enabled: true,
            });
        }
        let channels = (0..16)
            .map(|c| Choice {
                text: (c + 1).to_string(),
                on: t.as_ref().is_some_and(|t| t.channel == c),
                enabled: true,
            })
            .collect();
        Some(Picker {
            title: format!("Where {} plays", track_name(m, i)),
            groups: vec![
                Group {
                    head: "Port".to_string(),
                    w: 250,
                    choices: ports,
                },
                Group {
                    head: "Channel".to_string(),
                    w: 72,
                    choices: channels,
                },
            ],
        })
    }

    fn pick(&self, m: &mut IoModel, key: &str, _col: usize, group: usize, item: usize) -> bool {
        let Some(i) = parse_key(key, "track").and_then(|i| i.parse::<usize>().ok()) else {
            return false;
        };
        if i >= MAX_TRACKS {
            return false;
        }
        let before = m.setup.tracks[i].clone();
        let outs = out_ports(m);
        match group {
            0 if item == 0 => m.setup.tracks[i] = None,
            0 => {
                let Some((p, _)) = outs.get(item - 1) else {
                    return false;
                };
                let ch = before.as_ref().map_or(0, |t| t.channel);
                let mut t = before.clone().unwrap_or_else(|| TrackOut::new(p, ch));
                t.port = p.clone();
                m.setup.tracks[i] = Some(t);
            }
            _ => {
                let ch = item.min(15) as u8;
                match m.setup.tracks[i].as_mut() {
                    Some(t) => t.channel = ch,
                    // A channel picked for a track that plays inside goes to
                    // the first port with an output.
                    None => {
                        let Some((p, _)) = outs.first() else {
                            return false;
                        };
                        m.setup.tracks[i] = Some(TrackOut::new(p, ch));
                    }
                }
            }
        }
        m.setup.tracks[i] != before
    }

    fn live(&self, m: &IoModel, now: Duration) -> u64 {
        let mut v = m.live.ports.len() as u64;
        for (i, p) in m.live.ports.iter().enumerate().take(48) {
            if p.last_in.is_some_and(|t| now.saturating_sub(t) < LIGHT) {
                v |= 1 << (i + 8);
            }
        }
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::form::testing::{paint, tap, AREA};
    use crate::form::FormSection;
    use crate::{Built, Live, Shared};
    use mpfree_midi::setup::{MidiSetup, DIN_NAME};
    use mpfree_mixer_ui::{IoInput, IoReply, IoSection};
    use std::sync::{Arc, Mutex};

    fn model() -> Shared {
        let mut live = Live::default();
        live.ports.push(PortLive {
            name: "Keystep 37".into(),
            kind: "USB".into(),
            has_in: true,
            has_out: true,
            last_in: Some(Duration::from_millis(900)),
        });
        live.tracks = vec!["Drums".into(), "Bass".into(), "Keys".into()];
        Arc::new(Mutex::new(IoModel {
            setup: MidiSetup::default(),
            live,
            ..IoModel::default()
        }))
    }

    fn rows(s: &Shared) -> Vec<Row> {
        Midi.rows(&s.lock().unwrap(), Duration::from_secs(1))
    }

    fn row<'a>(rows: &'a [Row], key: &str) -> &'a Row {
        rows.iter().find(|r| r.key == key).expect(key)
    }

    #[test]
    fn ports_and_tracks_are_listed_with_lights() {
        let s = model();
        let r = rows(&s);
        let titles: Vec<&str> = r.iter().map(|r| r.title.as_str()).collect();
        assert_eq!(
            titles,
            [
                "Ports",
                DIN_NAME,
                "Keystep 37",
                "Tracks out",
                "Drums",
                "Bass",
                "Keys",
                "MIDI Learn",
                "No controls learned"
            ]
        );
        assert_eq!(row(&r, "port:MIDI").light, Some(false));
        assert_eq!(row(&r, "port:Keystep 37").light, Some(true));
        assert_eq!(row(&r, "port:MIDI").fields[0].text, "Both");
        assert_eq!(row(&r, "track:1").fields[0].text, INSIDE);
        assert!(!row(&r, "track:1").fields[1].enabled);
    }

    #[test]
    fn a_track_routed_to_a_missing_port_says_it_sends_nothing() {
        let s = model();
        s.lock().unwrap().setup.tracks[2] = Some(TrackOut::new("Digitone", 3));
        let r = rows(&s);
        let t = row(&r, "track:2");
        assert_eq!(t.fields[0].text, "Digitone · ch 4");
        assert!(t.fields[0].warn);
        assert_eq!(t.note, "sends nothing: Digitone is not plugged in");
        assert_eq!(t.tone, Tone::Warn);
        // The port it names is listed, unplugged.
        let p = row(&r, "port:Digitone");
        assert_eq!(p.note, "not plugged in: its settings are kept");
        // Plugged in while MIDI out is off: still says so.
        s.lock().unwrap().setup.tracks[2] = Some(TrackOut::new(DIN_NAME, 3));
        let r = rows(&s);
        assert_eq!(row(&r, "track:2").note, "sends nothing: MIDI out is off");
        s.lock().unwrap().live.built = Built {
            notes_out: true,
            ..Built::default()
        };
        let r = rows(&s);
        assert_eq!(row(&r, "track:2").note, "plays out of MIDI");
        assert!(Midi.banner(&s.lock().unwrap()).is_none());
    }

    /// A port the app can't send to yet (a USB device, for now) says so on
    /// the track that names it, and thru says when it does nothing.
    #[test]
    fn a_port_with_no_output_yet_and_thru_say_so() {
        let s = model();
        {
            let mut m = s.lock().unwrap();
            m.live.built = Built {
                notes_out: true,
                ..Built::default()
            };
            m.live.ports[1].has_out = false;
            m.setup.tracks[0] = Some(TrackOut::new("Keystep 37", 0));
            m.setup.set_port(DIN_NAME, |p| p.thru = true);
        }
        let r = rows(&s);
        assert_eq!(
            row(&r, "track:0").note,
            "sends nothing: no output to Keystep 37 yet"
        );
        assert_eq!(row(&r, "port:MIDI").note, "thru does nothing yet");
        s.lock().unwrap().live.built.thru = true;
        let r = rows(&s);
        assert_eq!(row(&r, "port:MIDI").note, "5-pin jacks");
    }

    #[test]
    fn a_tap_on_input_cycles_it_and_tells_the_app() {
        let s = model();
        let mut sec = FormSection::new(Midi, s.clone());
        let _ = paint(&sec);
        let f = sec_field(&sec, "port:Keystep 37", 0);
        assert_eq!(tap(&mut sec, f.0, f.1), IoReply::Redraw);
        assert_eq!(
            s.lock().unwrap().setup.port("Keystep 37").input,
            PortInput::PlaysTracks
        );
        assert_eq!(s.lock().unwrap().gen, 1);
        tap(&mut sec, f.0, f.1);
        tap(&mut sec, f.0, f.1);
        // Back to Both, the default: nothing kept for that port.
        assert!(s.lock().unwrap().setup.ports.is_empty());
    }

    #[test]
    fn the_plays_on_picker_sets_port_and_channel_together() {
        let s = model();
        let mut sec = FormSection::new(Midi, s.clone());
        let _ = paint(&sec);
        let f = sec_field(&sec, "track:1", 0);
        tap(&mut sec, f.0, f.1);
        let p = Midi.picker(&s.lock().unwrap(), "track:1", 0).unwrap();
        assert_eq!(p.title, "Where Bass plays");
        let names: Vec<&str> = p.groups[0]
            .choices
            .iter()
            .map(|c| c.text.as_str())
            .collect();
        assert_eq!(names, [INSIDE, DIN_NAME, "Keystep 37"]);
        // Channel 10 on a track that plays inside: the 5-pin jacks.
        assert!(Midi.pick(&mut s.lock().unwrap(), "track:1", 0, 1, 9));
        assert_eq!(
            s.lock().unwrap().setup.tracks[1],
            Some(TrackOut::new(DIN_NAME, 9))
        );
        // Then the port, keeping the channel.
        assert!(Midi.pick(&mut s.lock().unwrap(), "track:1", 0, 0, 2));
        assert_eq!(
            s.lock().unwrap().setup.tracks[1],
            Some(TrackOut::new("Keystep 37", 9))
        );
        assert_eq!(sec.hint(), Some(crate::form::HINT_PICKER));
        // Back closes the picker; a second Back does nothing.
        assert!(sec.back());
        assert!(!sec.back());
        // Inside again.
        assert!(Midi.pick(&mut s.lock().unwrap(), "track:1", 0, 0, 0));
        assert_eq!(s.lock().unwrap().setup.tracks[1], None);
    }

    #[test]
    fn wheel_moves_rows_and_plus_steps_the_field() {
        let s = model();
        let mut sec = FormSection::new(Midi, s.clone());
        let _ = paint(&sec);
        // The first row is selected; the wheel goes down past the heading.
        assert_eq!(sec.input(IoInput::Wheel(2)), IoReply::Redraw);
        let k = sec.qlinks().unwrap();
        assert_eq!(k[0].label, "Plays on");
        assert_eq!(k[0].text, INSIDE);
        assert_eq!(sec.input(IoInput::Step(1)), IoReply::Redraw);
        assert_eq!(
            s.lock().unwrap().setup.tracks[0],
            Some(TrackOut::new(DIN_NAME, 0))
        );
        // Q-Link 2 sets the program, from none.
        assert_eq!(sec.qlink_turn(1, 5, false), IoReply::Redraw);
        assert_eq!(
            s.lock().unwrap().setup.tracks[0].as_ref().unwrap().program,
            Some(4)
        );
        assert_eq!(sec.qlinks().unwrap()[1].text, "5");
        // At the top already: Handled, nothing moves.
        sec.input(IoInput::Wheel(-10));
        assert_eq!(sec.input(IoInput::Wheel(-1)), IoReply::Handled);
    }

    #[test]
    fn a_light_redraws_on_tick_only_when_it_changes() {
        let s = model();
        let mut sec = FormSection::new(Midi, s.clone());
        assert_eq!(sec.tick(Duration::from_millis(1000)), IoReply::Redraw);
        assert_eq!(sec.tick(Duration::from_millis(1016)), IoReply::Ignored);
        assert_eq!(sec.tick(Duration::from_millis(1200)), IoReply::Redraw);
    }

    #[test]
    fn it_paints_into_its_area_only() {
        let s = model();
        s.lock().unwrap().setup.tracks[2] = Some(TrackOut::new("Digitone", 3));
        let sec = FormSection::new(Midi, s);
        let c = paint(&sec);
        let bg = c.pixel(5, 5);
        assert_eq!(c.pixel(AREA.x - 2, AREA.y + 10), bg);
        assert_ne!(c.pixel(AREA.x + 20, AREA.y + 20), bg);
    }

    /// The middle of field `col` of the row `key`.
    fn sec_field(sec: &FormSection<Midi>, key: &str, col: usize) -> (i32, i32) {
        let r = sec.field_rect(key, col, AREA).expect(key);
        (r.x + r.w / 2, r.y + r.h / 2)
    }

    fn learned(port: Option<&str>, cc: u8) -> mpfree_midi::learn::Saved {
        use mpfree_midi::learn::{Mode, Saved, Source, Takeover};
        let target = "t2.inst.7".parse().unwrap();
        Saved {
            port: port.map(String::from),
            channel: 2,
            source: Source::Cc(cc),
            target,
            lo: 0.0,
            hi: 1.0,
            mode: Mode::Absolute,
            takeover: Takeover::Jump,
        }
    }

    /// Each learned control: what it is, what it moves, and when it would
    /// do nothing, the reason in place.
    #[test]
    fn learned_controls_are_listed_and_say_when_they_do_nothing() {
        let s = model();
        {
            let mut m = s.lock().unwrap();
            m.setup.learned(learned(Some("Keystep 37"), 74));
            m.setup.learned(learned(Some("Launch Control"), 21));
            m.setup.learned(learned(None, 1));
            m.live.learn_names = vec!["Cutoff (Drums)".into()];
        }
        let r = rows(&s);
        let a = row(&r, "learn:0");
        assert_eq!(a.title, "CC 74 · ch 3 · Keystep 37");
        assert_eq!(a.note, "moves Cutoff (Drums)");
        assert_eq!(a.fields[0].text, "Knob or fader");
        assert_eq!(
            row(&r, "learn:1").note,
            "moves t2.inst.7; Launch Control is not plugged in"
        );
        assert_eq!(row(&r, "learn:2").title, "CC 1 · ch 3 · any port");
        s.lock()
            .unwrap()
            .setup
            .set_port("Keystep 37", |p| p.input = PortInput::PlaysTracks);
        assert_eq!(
            row(&rows(&s), "learn:0").note,
            "does nothing: Keystep 37's input only plays tracks"
        );
    }

    /// "Works as" steps through the ways a control moves; Forget takes it
    /// off, with its name, and what is set is kept with the project.
    #[test]
    fn a_learned_control_can_change_and_be_forgotten() {
        use mpfree_midi::learn::{Encoding, Mode};
        let s = model();
        {
            let mut m = s.lock().unwrap();
            m.setup.learned(learned(Some("Keystep 37"), 74));
            m.setup.learned(learned(Some("Keystep 37"), 75));
            m.live.learn_names = vec!["A".into(), "B".into()];
        }
        let mut m = s.lock().unwrap();
        assert!(Midi.step(&mut m, "learn:0", 0, 1));
        assert_eq!(
            m.setup.learn[0].mode,
            Mode::Relative(Encoding::TwosComplement)
        );
        assert!(Midi.step(&mut m, "learn:0", 0, -2));
        assert_eq!(m.setup.learn[0].mode, Mode::Momentary);
        assert!(!Midi.step(&mut m, "learn:0", 1, 0));
        assert!(Midi.step(&mut m, "learn:0", 1, 1));
        assert_eq!(m.setup.learn.len(), 1);
        assert_eq!(m.live.learn_names, ["B"]);
        let (back, warn) = MidiSetup::parse(&m.setup.to_text());
        assert!(warn.is_empty(), "{warn:?}");
        assert_eq!(back.learn, m.setup.learn);
        assert!(!Midi.step(&mut m, "learn:5", 1, 1));
    }

    /// Forget works by a tap only: a Q-Link or + on its row does nothing.
    #[test]
    fn forget_takes_a_tap() {
        let s = model();
        s.lock()
            .unwrap()
            .setup
            .learned(learned(Some("Keystep 37"), 74));
        let mut sec = FormSection::new(Midi, s.clone());
        let _ = paint(&sec);
        // Selected by a tap on its title first, then knob 2 and +.
        let t = sec.field_rect("learn:0", 0, AREA).unwrap();
        tap(&mut sec, t.x - 40, t.y + 10);
        assert_eq!(sec.qlink_turn(1, 1, false), IoReply::Ignored);
        assert!(!sec.qlinks().unwrap()[1].enabled);
        sec.input(IoInput::Step(1));
        assert_eq!(s.lock().unwrap().setup.learn.len(), 1);
        // Selecting the row scrolled it; find Forget again.
        let _ = paint(&sec);
        let f = sec.field_rect("learn:0", 1, AREA).unwrap();
        tap(&mut sec, f.x + 10, f.y + 10);
        assert!(s.lock().unwrap().setup.learn.is_empty());
    }
}
