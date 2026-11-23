//! System status's watchers (phase 1; docs/dev/design/status.md): today's
//! counters and states, looked at on the screen thread each time the audio
//! loop's timing arrives (every 250 ms), turned into reports for the board.
//!
//! The audio thread never reports: it keeps bumping its counters as it
//! does, and [`Watch::look`] compares them with the last look. A state
//! (the sound near its limit, memory low) is reported when it begins and
//! when it ends; the board settles it, so a look that flips once doesn't
//! flicker.
//!
//! What this doesn't watch, because its owner reports it: the sampler's
//! four trades (notes dropped, sound skipped, voices given up, grains
//! thinned: the sampler refactor's step 10), a save that didn't work
//! (main.rs), the safe start after a crash and the GPU falling back
//! (gui.rs, drawing.rs), the touchscreen stopping (the screen loop) and a
//! load refused for memory (the Sample page's lines).

use mpfree_status::{kinds, Kind, Report, Subject};

/// Free memory under this is low: one and a half times the floor loads
/// keep spare ([`crate::memory::FLOOR`]).
pub const LOW_FREE: u64 = crate::memory::FLOOR / 2 * 3;
/// RAM in use over this (%) is low too.
pub const LOW_RAM_PCT: f64 = 85.0;
/// Memory is no longer low once this much more is free than
/// [`LOW_FREE`] and RAM is this far under [`LOW_RAM_PCT`], so a level at
/// the line doesn't come and go.
const BACK_FREE: u64 = 32_000_000;
const BACK_RAM_PCT: f64 = 3.0;

/// What one look reads. A field that is None isn't known on this look,
/// and its problem stays as it was.
#[derive(Clone, Copy, Debug, Default)]
pub struct Look<'a> {
    /// The sound card's underruns since the start.
    pub underruns: u64,
    /// Note endings the queues found no room for, since the start (must
    /// stay 0).
    pub ends_lost: u64,
    /// The period's last window was Tight or Over.
    pub near_limit: Option<bool>,
    /// Memory free (MemAvailable), bytes.
    pub free: Option<u64>,
    /// RAM in use, %.
    pub ram_pct: Option<f64>,
    /// The audio session isn't answering (the session chip's No reply or
    /// Stopped).
    pub session_lost: Option<bool>,
    /// The pads play without Akai's setup (the session chip's yellow No
    /// session: the session didn't start within 20 s).
    pub no_setup: Option<bool>,
    /// The open project left parts out.
    pub parts_missing: Option<bool>,
    /// Part of the sound didn't get its core.
    pub one_core: Option<bool>,
    /// The ports the project uses that aren't plugged in, when looked at
    /// (every 2 s).
    pub missing_ports: Option<&'a [String]>,
}

/// The states a look follows, each a kind that begins and ends.
const STATES: [&Kind; 6] = [
    &kinds::NEAR_LIMIT,
    &kinds::MEMORY_LOW,
    &kinds::NO_SESSION,
    &kinds::NO_SETUP,
    &kinds::PARTS_MISSING,
    &kinds::ONE_CORE,
];

/// The watcher's memory of the last look.
#[derive(Debug, Default)]
pub struct Watch {
    /// The counters at the last look (None before the first).
    counts: Option<(u64, u64)>,
    /// Each of [`STATES`] reported going.
    on: [bool; 6],
    /// Every port named missing this session: port `i` is `Subject(i + 1)`.
    ports: Vec<String>,
    /// Ports reported missing now, by their place in `ports`.
    missing: Vec<bool>,
    /// Ports missing at the last look of ports and not reported yet: a
    /// port must be missing at two looks running (2 s apart), so a port
    /// found a moment after the start, or unplugged and plugged straight
    /// back, says nothing.
    maybe: Vec<String>,
    /// The project's audio tracks by lasting id, with their names, for
    /// the takes' and the drive's problems about one
    /// (`Subject::audio_track`); and the takes found at start whose check
    /// failed, by their names in Recordings (`Subject::found_take(n)` is
    /// `found[n - 1]`).
    audio_tracks: Vec<(u32, String)>,
    found: Vec<String>,
}

impl Watch {
    /// One look: what changed since the last one, as reports to `report`.
    pub fn look(&mut self, l: &Look, report: &mut dyn FnMut(Report)) {
        // Moments: what the counters went up by. A counter that went down
        // (its engine replaced) starts again from where it is.
        let now = (l.underruns, l.ends_lost);
        let (u0, e0) = self.counts.unwrap_or((0, 0));
        for (kind, was, is) in [
            (&kinds::SOUND_DROPOUT, u0, now.0),
            (&kinds::NOTE_MAY_HANG, e0, now.1),
        ] {
            if is > was {
                let times = u32::try_from(is - was).unwrap_or(u32::MAX);
                report(Report::happened(kind, times));
            }
        }
        self.counts = Some(now);
        // States.
        let low = l.free.map(|f| f < LOW_FREE).unwrap_or(false)
            || l.ram_pct.is_some_and(|p| p > LOW_RAM_PCT);
        let back = l.free.is_none_or(|f| f >= LOW_FREE + BACK_FREE)
            && l.ram_pct.is_none_or(|p| p <= LOW_RAM_PCT - BACK_RAM_PCT);
        let memory = if l.free.is_none() && l.ram_pct.is_none() {
            None
        } else if low {
            Some(true)
        } else if back {
            Some(false)
        } else {
            // Between the lines: as it was.
            Some(self.on[1])
        };
        let states = [
            l.near_limit,
            memory,
            l.session_lost,
            l.no_setup,
            l.parts_missing,
            l.one_core,
        ];
        for (i, going) in states.into_iter().enumerate() {
            let Some(going) = going else { continue };
            if going != self.on[i] {
                self.on[i] = going;
                report(if going {
                    Report::began(STATES[i])
                } else {
                    Report::ended(STATES[i])
                });
            }
        }
        if let Some(missing) = l.missing_ports {
            self.ports_look(missing, report);
        }
    }

    fn ports_look(&mut self, missing: &[String], report: &mut dyn FnMut(Report)) {
        // Back: every port reported missing that isn't now.
        for i in 0..self.ports.len() {
            if self.missing[i] && !missing.contains(&self.ports[i]) {
                self.missing[i] = false;
                report(Report::ended(&kinds::PORT_MISSING).about(Subject(i as u32 + 1)));
            }
        }
        // Gone: missing at this look and the last.
        for name in missing {
            if !self.maybe.contains(name) {
                continue;
            }
            let i = match self.ports.iter().position(|p| p == name) {
                Some(i) => i,
                None => {
                    self.ports.push(name.clone());
                    self.missing.push(false);
                    self.ports.len() - 1
                }
            };
            if !self.missing[i] {
                self.missing[i] = true;
                report(Report::began(&kinds::PORT_MISSING).about(Subject(i as u32 + 1)));
            }
        }
        self.maybe.clear();
        self.maybe.extend(missing.iter().cloned());
    }

    /// The project's audio tracks as they are now: each one's lasting id
    /// and its name.
    pub fn set_audio_tracks<'a>(&mut self, tracks: impl IntoIterator<Item = (u32, &'a str)>) {
        self.audio_tracks.clear();
        self.audio_tracks
            .extend(tracks.into_iter().map(|(id, n)| (id, n.to_string())));
    }

    /// The takes recovery found at start whose check failed, by their
    /// names in Recordings (the audio tracks' `Recovered::found`).
    pub fn set_found_takes(&mut self, names: Vec<String>) {
        self.found = names;
    }

    /// What a problem is about, in words, where the watcher named it: a
    /// port's own name, an audio track's name, or a take found at start
    /// by its file's name.
    pub fn subject_name(&self, kind: &Kind, subject: Subject) -> Option<&str> {
        if let Some(id) = subject.audio_track_id() {
            return self
                .audio_tracks
                .iter()
                .find(|(i, _)| *i == id)
                .map(|(_, n)| n.as_str());
        }
        if let Some(n) = subject.found_take_number() {
            return self
                .found
                .get(n.checked_sub(1)? as usize)
                .map(String::as_str);
        }
        if kind != &kinds::PORT_MISSING || subject == Subject::NONE {
            return None;
        }
        self.ports.get(subject.0 as usize - 1).map(String::as_str)
    }
}

/// How many of the Sample page's log lines say something didn't load for
/// want of memory ([`crate::memory::SHORT`]): a load refused, or a sound a
/// kit or project left out. A sound only kept at its own rate still
/// loaded, so it isn't counted.
pub fn refused_for_memory<'a>(lines: impl IntoIterator<Item = &'a String>) -> u32 {
    lines
        .into_iter()
        .filter(|l| l.contains(crate::memory::SHORT) && !l.contains("kept at its own rate"))
        .count() as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn look(w: &mut Watch, l: Look) -> Vec<String> {
        let mut out = Vec::new();
        w.look(&l, &mut |r| {
            out.push(format!(
                "{} {}{}",
                match r.op {
                    mpfree_status::Op::Happened { times } => format!("+{times}"),
                    mpfree_status::Op::Began => "began".to_string(),
                    mpfree_status::Op::Ended => "ended".to_string(),
                },
                r.kind.id,
                if r.subject == Subject::NONE {
                    String::new()
                } else {
                    format!(" #{}", r.subject.0)
                }
            ))
        });
        out
    }

    #[test]
    fn counters_report_what_they_went_up_by() {
        let mut w = Watch::default();
        assert!(look(&mut w, Look::default()).is_empty(), "all quiet");
        let l = Look {
            underruns: 3,
            ..Look::default()
        };
        assert_eq!(look(&mut w, l), ["+3 sound/dropout"]);
        assert!(look(&mut w, l).is_empty(), "no new ones");
        let l = Look {
            underruns: 4,
            ends_lost: 1,
            ..Look::default()
        };
        assert_eq!(
            look(&mut w, l),
            ["+1 sound/dropout", "+1 sound/note-may-hang"]
        );
        assert!(
            look(&mut w, Look::default()).is_empty(),
            "a counter that went back starts again"
        );
    }

    #[test]
    fn states_begin_and_end_once_each() {
        let mut w = Watch::default();
        let near = |on| Look {
            near_limit: Some(on),
            ..Look::default()
        };
        assert!(look(&mut w, near(false)).is_empty());
        assert_eq!(look(&mut w, near(true)), ["began sound/near-limit"]);
        assert!(look(&mut w, near(true)).is_empty());
        assert!(
            look(&mut w, Look::default()).is_empty(),
            "not known: as it was"
        );
        assert_eq!(look(&mut w, near(false)), ["ended sound/near-limit"]);
        let l = Look {
            session_lost: Some(true),
            parts_missing: Some(true),
            one_core: Some(true),
            ..Look::default()
        };
        assert_eq!(
            look(&mut w, l),
            [
                "began controls/no-session",
                "began project/parts-missing",
                "began mpc/one-core"
            ]
        );
        // The session chip's yellow No session, then the session late.
        let setup = |on| Look {
            no_setup: Some(on),
            ..Look::default()
        };
        assert_eq!(look(&mut w, setup(true)), ["began controls/no-setup"]);
        assert_eq!(look(&mut w, setup(false)), ["ended controls/no-setup"]);
    }

    #[test]
    fn memory_is_low_under_its_lines_and_back_well_over_them() {
        let mut w = Watch::default();
        let mem = |free: u64, ram: f64| Look {
            free: Some(free),
            ram_pct: Some(ram),
            ..Look::default()
        };
        assert!(look(&mut w, mem(900_000_000, 50.0)).is_empty());
        assert_eq!(look(&mut w, mem(LOW_FREE - 1, 50.0)), ["began memory/low"]);
        assert!(
            look(&mut w, mem(LOW_FREE + 1, 50.0)).is_empty(),
            "just over the line: still low"
        );
        assert_eq!(
            look(&mut w, mem(LOW_FREE + BACK_FREE, 50.0)),
            ["ended memory/low"]
        );
        assert_eq!(
            look(&mut w, mem(900_000_000, 86.0)),
            ["began memory/low"],
            "RAM over 85 %"
        );
        assert!(look(&mut w, mem(900_000_000, 84.0)).is_empty());
        assert_eq!(look(&mut w, mem(900_000_000, 82.0)), ["ended memory/low"]);
    }

    #[test]
    fn a_port_is_missing_after_two_looks_and_back_at_once() {
        let mut w = Watch::default();
        let ports =
            |names: &[&str]| -> Vec<String> { names.iter().map(|s| s.to_string()).collect() };
        let at = |w: &mut Watch, p: &[String]| {
            look(
                w,
                Look {
                    missing_ports: Some(p),
                    ..Look::default()
                },
            )
        };
        assert!(
            at(&mut w, &ports(&["Juno-X"])).is_empty(),
            "maybe not scanned yet"
        );
        assert_eq!(
            at(&mut w, &ports(&["Juno-X", "Digitakt"])),
            ["began midi/port-missing #1"]
        );
        assert_eq!(
            at(&mut w, &ports(&["Juno-X", "Digitakt"])),
            ["began midi/port-missing #2"]
        );
        assert_eq!(
            w.subject_name(&kinds::PORT_MISSING, Subject(2)),
            Some("Digitakt")
        );
        assert_eq!(w.subject_name(&kinds::SOUND_DROPOUT, Subject(2)), None);
        assert_eq!(
            at(&mut w, &ports(&["Digitakt"])),
            ["ended midi/port-missing #1"]
        );
        assert!(
            look(&mut w, Look::default()).is_empty(),
            "ports not looked at: as they were"
        );
        assert!(at(&mut w, &ports(&["Juno-X", "Digitakt"])).is_empty());
        assert_eq!(
            at(&mut w, &ports(&["Juno-X", "Digitakt"])),
            ["began midi/port-missing #1"],
            "the same subject as before"
        );
    }

    #[test]
    fn an_audio_track_and_a_found_take_are_named_by_their_tag() {
        let mut w = Watch::default();
        w.set_audio_tracks([(7, "Vocal"), (9, "Gtr")]);
        w.set_found_takes(vec!["Bass_0123abcd.unverified.wav".into()]);
        let k = &kinds::RECORDING_AT_RISK;
        assert_eq!(w.subject_name(k, Subject::audio_track(9)), Some("Gtr"));
        assert_eq!(w.subject_name(k, Subject::audio_track(8)), None);
        assert_eq!(
            w.subject_name(&kinds::TAKE_DAMAGED, Subject::found_take(1)),
            Some("Bass_0123abcd.unverified.wav")
        );
        assert_eq!(
            w.subject_name(&kinds::TAKE_DAMAGED, Subject::found_take(2)),
            None
        );
        // A port's subject is still a port's, and only for Port missing.
        assert_eq!(w.subject_name(&kinds::PORT_MISSING, Subject(1)), None);
        assert_eq!(w.subject_name(k, Subject::NONE), None);
        // The tracks as they are now: a renamed track's new name.
        w.set_audio_tracks([(9, "Guitar")]);
        assert_eq!(w.subject_name(k, Subject::audio_track(9)), Some("Guitar"));
        assert_eq!(w.subject_name(k, Subject::audio_track(7)), None);
    }

    #[test]
    fn sample_lines_count_what_memory_kept_out() {
        let lines = [
            "sample: not loaded (not enough memory: needs about 300 MB, 200 MB free, 256 MB kept spare)",
            "sample: Drums: sound kick.wav left out (not enough memory: needs about 9 MB)",
            "sample: kept at its own rate, not enough memory: Pad at 48 kHz",
            "sample: hit a pad to put Kick on it",
        ]
        .map(String::from);
        assert_eq!(refused_for_memory(&lines), 2);
    }
}
