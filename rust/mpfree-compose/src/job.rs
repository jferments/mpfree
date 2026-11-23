//! One Continue request, owned, for the composer's own thread (phase 1).
//!
//! The app's time engine never runs the composer: it copies just the clips
//! the reading needs into a [`ContinueJob`] (the clip and the rest of its
//! row, with nothing else of the project), hands it to a thread of its own
//! at idle priority, and goes on. The thread answers with [`Done`]. The
//! result is bound to the clips it read, by their revisions, so the time
//! engine can tell whether it still fits when it arrives.

use mpfree_context::groove::Lane;
use mpfree_context::{PartRole, ReadBudget};
use mpfree_seq::{Project, Track};
use mpfree_transport::time::{Meter, Swing};

use crate::preview::Proposal;
use crate::profile::InstrumentKind;
use crate::read::ToRead;
use crate::request::{continue_clip_how, Continued, Making, OnClip, Refused};
use crate::settings::Settings;

/// The drums each pad of a drum track plays, as the kit names them: only
/// the pads that are drums. A pad not here isn't read for the groove.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Lanes(Vec<(usize, u8, Lane)>);

impl Lanes {
    /// From (track, pad, drum) triples, in any order; a later triple for
    /// the same pad wins.
    pub fn new(mut pads: Vec<(usize, u8, Lane)>) -> Lanes {
        pads.reverse();
        pads.sort_by_key(|&(t, p, _)| (t, p));
        pads.dedup_by_key(|&mut (t, p, _)| (t, p));
        Lanes(pads)
    }

    /// The drum pad `pad` of `track` plays, if it is one.
    pub fn lane(&self, track: usize, pad: u8) -> Option<Lane> {
        self.0
            .binary_search_by_key(&(track, pad), |&(t, p, _)| (t, p))
            .ok()
            .map(|i| self.0[i].2)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// The drum a kit's pad plays, read from the sound's name: Kick ("Kick
/// 1", "BD Deep", "808 Bass Drum"), Snare (a snare, "SD", a clap or a
/// rim), Hats (a hat, "HH", a ride, a cymbal, a shaker or a tambourine),
/// else Other; None for a pad with no sound.
pub fn lane_named(name: &str) -> Option<Lane> {
    let name = name.trim().to_lowercase();
    if name.is_empty() {
        return None;
    }
    let words: Vec<&str> = name.split(|c: char| !c.is_alphanumeric()).collect();
    let word = |w: &[&str]| words.iter().any(|x| w.contains(x));
    let has = |w: &[&str]| w.iter().any(|x| name.contains(x));
    Some(
        if has(&["kick", "bassdrum", "bass drum"]) || word(&["bd", "kik", "kck"]) {
            Lane::Kick
        } else if has(&["snare", "clap", "rim"]) || word(&["sd", "snr", "sn", "cp"]) {
            Lane::Snare
        } else if has(&["hat", "ride", "cymbal", "shaker", "tamb"])
            || word(&["hh", "oh", "ch", "cy"])
        {
            Lane::Hats
        } else {
            Lane::Other
        },
    )
}

/// What the time engine asks for, besides the project.
#[derive(Clone, Debug, PartialEq)]
pub struct Ask {
    pub track: usize,
    pub slot: usize,
    pub role: PartRole,
    pub kind: InstrumentKind,
    /// The rest of the clip's row, to listen to.
    pub others: Vec<ToRead>,
    pub lanes: Lanes,
    pub transport_swing: Swing,
    pub meter: Meter,
    pub bpm: f64,
    pub budget: ReadBudget,
    pub settings: Settings,
    /// A new result, or part of an earlier one again.
    pub how: Making,
}

/// A request with its own copy of what it reads.
#[derive(Clone, Debug)]
pub struct ContinueJob {
    /// Only the tracks it reads, each with only the clips it reads, at
    /// their places: every other slot is empty.
    project: Project,
    ask: Ask,
}

/// The composer's answer.
#[derive(Clone, Debug, PartialEq)]
pub struct Done {
    pub result: Result<(Proposal, Continued), Refused>,
    /// Clips the reading left out (gone, or past its budget).
    pub left_out: usize,
}

impl ContinueJob {
    /// Copy what `ask` reads out of `project`: the clip and the others,
    /// with each track's name, id and swing. `NoClip` if the clip isn't
    /// there.
    pub fn new(project: &Project, ask: Ask) -> Result<ContinueJob, Refused> {
        project.clip(ask.track, ask.slot).ok_or(Refused::NoClip)?;
        let mut copy = Project::new();
        let wanted = std::iter::once((ask.track, ask.slot))
            .chain(ask.others.iter().map(|r| (r.track, r.slot)));
        for (t, s) in wanted {
            let (Some(track), Some(clip)) = (project.tracks.get(t), project.clip(t, s)) else {
                continue;
            };
            while copy.tracks.len() <= t {
                let i = copy.tracks.len();
                let from = &project.tracks[i];
                copy.tracks.push(Track {
                    id: from.id,
                    swing: from.swing,
                    ..Track::new(&from.name, from.dest)
                });
            }
            debug_assert_eq!(copy.tracks[t].id, track.id);
            if let Some(slot) = copy.tracks[t].slots.get_mut(s) {
                *slot = Some(clip.clone());
            }
        }
        Ok(ContinueJob { project: copy, ask })
    }

    pub fn ask(&self) -> &Ask {
        &self.ask
    }

    /// Read and carry on. Pure: the same job gives the same answer.
    pub fn run(&self) -> Done {
        let a = &self.ask;
        let lane_of = |t: usize, pad: u8| a.lanes.lane(t, pad);
        let on = OnClip {
            project: &self.project,
            track: a.track,
            slot: a.slot,
            role: a.role,
            others: &a.others,
            lane_of: &lane_of,
            transport_swing: a.transport_swing,
            meter: a.meter,
            bpm: a.bpm,
            budget: a.budget,
        };
        let (result, left_out) = continue_clip_how(&on, &a.settings, a.kind, &a.how);
        Done { result, left_out }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bind::Binding;
    use crate::request::continue_clip;
    use crate::style::GROOVE;
    use mpfree_seq::{Clip, Holds, Note};
    use mpfree_transport::time::{Tick, PPQ};

    const BEAT: Tick = PPQ as Tick;
    const BAR: Tick = 4 * BEAT;

    fn project() -> Project {
        let mut p = Project::new();
        let mut lead = Track::new("Lead", 0);
        let mut riff = Clip::new("riff", BAR);
        riff.notes = [60u8, 63, 67, 65, 63, 62, 63, 60]
            .iter()
            .enumerate()
            .map(|(i, &n)| Note::new(i as Tick * BEAT / 2, BEAT / 2, n, 96))
            .collect();
        riff.holds = Holds::Notes;
        lead.slots[1] = Some(riff);
        lead.swing = Some(Swing::new(PPQ / 2, 56).unwrap());
        let mut pads = Track::new("Pads", 1);
        pads.slots[0] = Some(Clip::new("not read", BAR));
        let mut drums = Track::new("Drums", 2);
        let mut beat = Clip::new("beat", BAR);
        beat.notes = (0..8)
            .map(|i| Note::new(i * BEAT / 2, 60, if i % 4 == 0 { 0 } else { 2 }, 90))
            .collect();
        drums.slots[1] = Some(beat);
        p.tracks.extend([pads, lead, drums]);
        p.give_clip_ids();
        p
    }

    fn ask() -> Ask {
        Ask {
            track: 1,
            slot: 1,
            role: PartRole::Melody,
            kind: InstrumentKind::OneNote,
            others: vec![ToRead {
                track: 2,
                slot: 1,
                role: PartRole::Drums,
            }],
            lanes: Lanes::new(vec![(2, 2, Lane::Hats), (2, 0, Lane::Kick)]),
            transport_swing: Swing::default(),
            meter: Meter::default(),
            bpm: 110.0,
            budget: ReadBudget::default(),
            settings: Settings {
                style: GROOVE,
                bars: 2,
                seed: 9,
                ..Settings::default()
            },
            how: Making::New,
        }
    }

    #[test]
    fn a_pad_s_drum_is_read_from_its_sound_s_name() {
        assert_eq!(lane_named("Kick 1"), Some(Lane::Kick));
        assert_eq!(lane_named("BD Deep"), Some(Lane::Kick));
        assert_eq!(lane_named("808 Bass Drum"), Some(Lane::Kick));
        assert_eq!(lane_named("SD Tight"), Some(Lane::Snare));
        assert_eq!(lane_named("Clap Wide"), Some(Lane::Snare));
        assert_eq!(lane_named("Hat Open"), Some(Lane::Hats));
        assert_eq!(lane_named("HH closed"), Some(Lane::Hats));
        assert_eq!(lane_named("Shaker"), Some(Lane::Hats));
        assert_eq!(lane_named("Conga Hi"), Some(Lane::Other));
        assert_eq!(lane_named("  "), None);
    }

    #[test]
    fn lanes_find_each_pad_and_only_drums() {
        let l = Lanes::new(vec![
            (2, 2, Lane::Hats),
            (0, 5, Lane::Snare),
            (2, 0, Lane::Kick),
            (2, 2, Lane::Other),
        ]);
        assert_eq!(l.lane(2, 0), Some(Lane::Kick));
        assert_eq!(l.lane(0, 5), Some(Lane::Snare));
        assert_eq!(l.lane(2, 2), Some(Lane::Other), "the later one wins");
        assert_eq!(l.lane(2, 1), None);
        assert_eq!(l.lane(1, 0), None);
    }

    #[test]
    fn the_copy_holds_only_what_is_read_and_answers_as_the_project_would() {
        let p = project();
        let job = ContinueJob::new(&p, ask()).unwrap();
        // Tracks up to the last one read, each with only its read clip.
        assert_eq!(job.project.tracks.len(), 3);
        assert!(job.project.clip(0, 0).is_none(), "the pads aren't read");
        assert_eq!(job.project.clip(1, 1), p.clip(1, 1));
        assert_eq!(job.project.clip(2, 1), p.clip(2, 1));
        assert_eq!(job.project.tracks[1].swing, p.tracks[1].swing);
        assert_eq!(job.project.tracks[1].id, p.tracks[1].id);

        let done = job.run();
        assert_eq!(done.left_out, 0);
        let (proposal, made) = done.result.clone().unwrap();
        assert_eq!(made.to, 3 * BAR);
        // The same as reading the whole project, and bound to it.
        let a = ask();
        let lane_of = |t: usize, pad: u8| a.lanes.lane(t, pad);
        let on = OnClip {
            project: &p,
            track: 1,
            slot: 1,
            role: a.role,
            others: &a.others,
            lane_of: &lane_of,
            transport_swing: a.transport_swing,
            meter: a.meter,
            bpm: a.bpm,
            budget: a.budget,
        };
        let (whole, _) = continue_clip(&on, &a.settings, a.kind).unwrap();
        assert_eq!(proposal, whole);
        let b = Binding::of(&p, 1, 1).unwrap();
        assert!(b.check_all(&p, &proposal.read).is_ok());
        // The same job, the same answer.
        assert_eq!(job.run(), done);
    }

    #[test]
    fn an_empty_slot_is_refused_and_a_missing_other_is_left_out() {
        let p = project();
        let mut a = ask();
        a.slot = 5;
        assert!(matches!(ContinueJob::new(&p, a), Err(Refused::NoClip)));
        let mut a = ask();
        a.others.push(ToRead {
            track: 0,
            slot: 3,
            role: PartRole::Chords,
        });
        let done = ContinueJob::new(&p, a).unwrap().run();
        assert!(done.result.is_ok());
        assert_eq!(done.left_out, 1);
    }
}
