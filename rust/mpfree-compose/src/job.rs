//! One Continue request, owned, for the composer's own thread (phase 1).
//!
//! The app's time engine never runs the composer: it copies just the clips
//! the reading needs into a [`ContinueJob`] (the clip and the rest of its
//! row, with nothing else of the project), hands it to a thread of its own
//! at idle priority, and goes on. The thread answers with [`Done`]. The
//! result is bound to the clips it read, by their revisions, so the time
//! engine can tell whether it still fits when it arrives.

use mpfree_context::chord::{ChordTimeline, Heard};
use mpfree_context::groove::Lane;
use mpfree_context::harmony::RowHarmony;
use mpfree_context::{ClipRead, Key, Mode, PartRole, ReadBudget};
use mpfree_seq::{row_harmony, scene_length, Clip, Holds, Project, Track};
use mpfree_transport::time::{Meter, Swing, Tick};

use crate::keep::clip_notes;
use crate::parts::{add_part_again, AddPart, Added, Refused as PartRefused, Relation};
use crate::preview::Proposal;
use crate::profile::InstrumentKind;
use crate::progression::suggest_chords;
use crate::read::{read, row_parts, ToRead};
use crate::request::{continue_clip_how, Continued, Making, OnClip, Refused};
use crate::row::{Row, ToHear};
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
    /// The row's chords (contract 4), as the whole project has them.
    row: Option<RowHarmony>,
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
        let row = row_harmony(project, ask.slot, ask.meter.bar_ticks());
        Ok(ContinueJob {
            project: copy,
            ask,
            row,
        })
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
            row: self.row.as_ref(),
        };
        let (result, left_out) = continue_clip_how(&on, &a.settings, a.kind, &a.how);
        Done { result, left_out }
    }
}

/// What Add a part is asked for, besides the project (S2.5).
#[derive(Clone, Debug, PartialEq)]
pub struct AddAsk {
    /// The row: slot `row` of every track.
    pub row: usize,
    /// The row's clips to hear, each with what it is in the music.
    pub parts: Vec<ToRead>,
    pub lanes: Lanes,
    /// What to add: Bass, Chords, Melody (a second line) or Drums.
    pub part: PartRole,
    pub relation: Relation,
    /// What plays it.
    pub kind: InstrumentKind,
    /// For drums, the kit's pad for kick, snare, hats and the rest.
    pub pads: [Option<u8>; 4],
    /// The swing the new clip plays with.
    pub swing: Swing,
    pub transport_swing: Swing,
    pub meter: Meter,
    pub bpm: f64,
    pub budget: ReadBudget,
    pub settings: Settings,
    /// Try again's count: 0 is the first answer.
    pub attempt: u16,
}

/// An Add a part request with its own copy of what it hears.
#[derive(Clone, Debug)]
pub struct AddPartJob {
    /// Only the row's tracks, each with only its clip in the row.
    project: Project,
    ask: AddAsk,
    /// The row's chords and length, as the whole project has them.
    row: Option<RowHarmony>,
    length: Tick,
}

/// A part made, ready to keep.
#[derive(Clone, Debug, PartialEq)]
pub struct PartMade {
    pub added: Added,
    /// The clip Keep puts in the row's empty cell: the part over the
    /// row's length, its notes as heard (groove and all), a kit's hits
    /// for drums.
    pub clip: Clip,
    /// Chords to keep with the part as the row's Harmony line: the chords
    /// suggested for a row with nothing to hear, so the next part plays
    /// over the same ones. `None` when the row's line says them already.
    pub line: Option<ChordTimeline>,
    /// The chords it was made over, over the row's length.
    pub chords: ChordTimeline,
    /// The clips it heard, each at its revision: a part made over clips
    /// changed since doesn't fit any more.
    pub read: Vec<ClipRead>,
}

/// Add a part's answer.
#[derive(Clone, Debug, PartialEq)]
pub struct AddDone {
    pub result: Result<PartMade, PartRefused>,
    /// Clips the reading left out (gone, or past its budget).
    pub left_out: usize,
}

impl AddPartJob {
    /// Copy what `ask` hears out of `project`: the row's clips, with each
    /// track's name, id and swing, and the row's chords and length.
    pub fn new(project: &Project, ask: AddAsk) -> AddPartJob {
        let mut copy = Project::new();
        for r in &ask.parts {
            let (Some(_), Some(clip)) =
                (project.tracks.get(r.track), project.clip(r.track, r.slot))
            else {
                continue;
            };
            while copy.tracks.len() <= r.track {
                let from = &project.tracks[copy.tracks.len()];
                copy.tracks.push(Track {
                    id: from.id,
                    swing: from.swing,
                    ..Track::new(&from.name, from.dest)
                });
            }
            if let Some(slot) = copy.tracks[r.track].slots.get_mut(r.slot) {
                *slot = Some(clip.clone());
            }
        }
        let bar = ask.meter.bar_ticks();
        AddPartJob {
            project: copy,
            row: row_harmony(project, ask.row, bar),
            length: scene_length(project, ask.row, bar),
            ask,
        }
    }

    pub fn ask(&self) -> &AddAsk {
        &self.ask
    }

    /// The job over `line` in place of the row's stored chords: the
    /// chords picked on the Add a part panel and not kept yet. They are
    /// heard again as stored chords are (a chord you set stays), and Keep
    /// puts them on the row with the part. An empty line is the row with
    /// no chords of its own.
    pub fn with_line(mut self, line: ChordTimeline) -> AddPartJob {
        self.row = (!line.is_empty()).then(|| RowHarmony {
            suggested: line.spans().iter().all(|s| s.heard == Heard::Suggested),
            line,
            length: self.length,
        });
        self
    }

    /// The row's chords as it plays now, for the screens' Harmony line:
    /// its line heard again with its clips, or what the clips say when it
    /// has none of its own. Nothing is suggested here. Pure.
    pub fn hear(&self) -> ChordTimeline {
        let a = &self.ask;
        let lane_of = |t: usize, pad: u8| a.lanes.lane(t, pad);
        let (ctx, _) = read(
            &self.project,
            &a.parts,
            &lane_of,
            a.transport_swing,
            a.meter,
            a.budget,
        );
        self.line_heard(&ctx.chords)
    }

    /// The row's line heard again with the chords a reading heard, or
    /// those alone when the row has no line.
    fn line_heard(&self, heard: &ChordTimeline) -> ChordTimeline {
        match self.row.as_ref().map(|r| &r.line) {
            Some(l) => l.rehear(heard),
            None => heard.plain(),
        }
    }

    /// Hear the row and add the part. Pure: the same job gives the same
    /// answer.
    pub fn run(&self) -> AddDone {
        let a = &self.ask;
        let lane_of = |t: usize, pad: u8| a.lanes.lane(t, pad);
        let (ctx, left) = read(
            &self.project,
            &a.parts,
            &lane_of,
            a.transport_swing,
            a.meter,
            a.budget,
        );
        let bar = a.meter.bar_ticks().max(1);
        let length = self.length.max(bar);
        // The row's chords: its line heard again, else the reading; for a
        // row with nothing to hear, suggested ones (contract 4).
        let stored = self.row.as_ref().map(|r| &r.line);
        let heard = self.line_heard(&ctx.chords);
        let clear = |t: &ChordTimeline| t.spans().iter().any(|s| s.answer.chord().is_some());
        let line = if clear(&heard) || a.part == PartRole::Drums {
            heard
        } else {
            let key = ctx.key.best().unwrap_or(Key::new(0, Mode::Minor));
            let bars = (length / bar) as usize;
            suggest_chords(key, &a.settings, bars, a.meter)
        };
        let keep_line = (line.spans().iter().any(|s| s.heard == Heard::Suggested)
            && stored != Some(&line))
        .then(|| line.clone());
        let chords = line.looped(length);
        let parts = row_parts(&self.project, &a.parts, &lane_of, a.transport_swing, length);
        let row = Row::hear(&ToHear {
            parts: &parts,
            length,
            meter: a.meter,
            key: &ctx.key,
            played_pcs: ctx.played_pcs,
            chords: &chords,
            groove: ctx.groove.as_ref(),
            lead: None,
        });
        let req = AddPart {
            row: &row,
            part: a.part,
            relation: a.relation,
            settings: &a.settings,
            kind: a.kind,
            pads: a.pads,
            swing: a.swing,
        };
        let result = add_part_again(&req, a.attempt).and_then(|added| {
            let notes = clip_notes(&added.events, a.bpm, row.length).map_err(PartRefused::Keep)?;
            let mut clip = Clip::new(part_name(a.part), row.length);
            clip.notes = notes;
            clip.holds = if a.part == PartRole::Drums {
                Holds::Hits
            } else {
                Holds::Notes
            };
            clip.swing = Some(a.swing);
            Ok(PartMade {
                added,
                clip,
                line: keep_line,
                chords,
                read: ctx.read.clone(),
            })
        });
        AddDone {
            result,
            left_out: left.len(),
        }
    }
}

/// A new part's clip name: what it is.
fn part_name(part: PartRole) -> &'static str {
    match part {
        PartRole::Bass => "Bass",
        PartRole::Chords => "Chords",
        PartRole::Melody => "Line",
        PartRole::Drums => "Drums",
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
            row: None,
        };
        let (whole, _) = continue_clip(&on, &a.settings, a.kind).unwrap();
        assert_eq!(proposal, whole);
        let b = Binding::of(&p, 1, 1).unwrap();
        assert!(b.check_all(&p, &proposal.read).is_ok());
        // The same job, the same answer.
        assert_eq!(job.run(), done);
    }

    /// A row of block chords held a bar each, on track 0 slot 0, and a
    /// one-bar tune on track 1 slot 0: the chords the row plays and a
    /// tune to carry on over them.
    fn chords_and_tune(chords: &[[u8; 3]]) -> Project {
        let mut p = Project::new();
        let mut pads = Track::new("Pads", 0);
        let mut c = Clip::new("chords", chords.len() as Tick * BAR);
        c.notes = chords
            .iter()
            .enumerate()
            .flat_map(|(b, ch)| {
                ch.iter()
                    .map(move |&n| Note::new(b as Tick * BAR, BAR - 10, n, 90))
            })
            .collect();
        c.holds = Holds::Notes;
        pads.slots[0] = Some(c);
        let mut lead = Track::new("Lead", 1);
        let mut tune = Clip::new("tune", BAR);
        tune.notes = [72u8, 75, 79, 75]
            .iter()
            .enumerate()
            .map(|(i, &n)| Note::new(i as Tick * BEAT, BEAT / 2, n, 96))
            .collect();
        tune.holds = Holds::Notes;
        lead.slots[0] = Some(tune);
        p.tracks.extend([pads, lead]);
        p.give_clip_ids();
        p
    }

    fn tune_ask(bars: u8) -> Ask {
        Ask {
            track: 1,
            slot: 0,
            others: vec![ToRead {
                track: 0,
                slot: 0,
                role: PartRole::Chords,
            }],
            lanes: Lanes::default(),
            settings: Settings {
                style: GROOVE,
                bars,
                seed: 4,
                ..Settings::default()
            },
            ..ask()
        }
    }

    fn root_at(t: &mpfree_context::chord::ChordTimeline, at: Tick) -> Option<u8> {
        t.span_at(at)?.answer.chord().map(|c| c.root)
    }

    /// Contract 4: new bars inside the row play the chords the row plays
    /// there (here the pads' bars 2 to 4); past the row they pick their
    /// own, and the row grows.
    #[test]
    fn new_bars_inside_the_row_play_the_rows_chords() {
        // Cm, Fm, Ab, G: four bars of pads under a one-bar tune.
        let p = chords_and_tune(&[[60, 63, 67], [60, 65, 68], [60, 63, 68], [59, 62, 67]]);
        let (_, made) = ContinueJob::new(&p, tune_ask(4))
            .unwrap()
            .run()
            .result
            .unwrap();
        assert_eq!(made.to, 5 * BAR);
        let roots: Vec<_> = (0..4).map(|b| root_at(&made.chords, b * BAR)).collect();
        assert_eq!(
            roots,
            [Some(0), Some(5), Some(8), Some(7)],
            "{}",
            made.chords.names()
        );
        // The notes of a new bar fit its chord on the beat.
        for n in made
            .notes
            .iter()
            .filter(|n| n.tick % BEAT == 0 && n.tick < 4 * BAR)
        {
            let c = made.chords.span_at(n.tick).unwrap().answer.chord().unwrap();
            let in_key = mpfree_context::Key::new(0, mpfree_context::Mode::Minor).pcs();
            assert!(
                c.has(i32::from(n.note)) || in_key & (1 << (n.note % 12)) != 0,
                "{} on {}",
                n.note,
                c.name()
            );
        }
    }

    /// Contract 4: a chord set on the row's Harmony line is what Continue
    /// plays over, in your bars and in new bars inside the row; heard
    /// chords elsewhere follow the clips.
    #[test]
    fn a_chord_you_set_on_the_row_is_what_continue_plays_over() {
        use mpfree_context::chord::{Chord, ChordAnswer, ChordTimeline, Heard, Quality};
        let mut p = chords_and_tune(&[[60, 63, 67], [60, 65, 68]]);
        // The row's line: what it heard before, with bar 2 set to Bbmaj7
        // and bar 1 heard as Cm7 (the pads play Cm there anyway).
        let mut line = ChordTimeline::of(
            &[
                (Chord::new(0, Quality::Minor7), BAR),
                (Chord::new(5, Quality::Minor), BAR),
            ],
            Heard::Heard,
        );
        line.set_at(BAR, ChordAnswer::Chord(Chord::new(10, Quality::Major7)));
        p.scene_mut(0).unwrap().chords = line;
        let job = ContinueJob::new(&p, tune_ask(2)).unwrap();
        assert!(job.row.is_some());
        let (_, made) = job.run().result.unwrap();
        assert_eq!(root_at(&made.chords, 0), Some(0));
        assert_eq!(
            made.chords.span_at(BAR).unwrap().answer.chord(),
            Some(Chord::new(10, Quality::Major7)),
            "{}",
            made.chords.names()
        );
        // Without the line, the pads' Fm.
        p.scene_mut(0).unwrap().chords = ChordTimeline::default();
        let (_, plain) = ContinueJob::new(&p, tune_ask(2))
            .unwrap()
            .run()
            .result
            .unwrap();
        assert_eq!(
            root_at(&plain.chords, BAR),
            Some(5),
            "{}",
            plain.chords.names()
        );
    }

    /// A row 0 of a four-bar C minor tune on track 0 and, with `beat`, a
    /// one-bar beat on track 1 (kick 0, snare 1, hats 2), and an empty
    /// track 2 for the part.
    fn tune_row(tune: bool, beat: bool) -> Project {
        let mut p = Project::new();
        let mut lead = Track::new("Lead", 0);
        if tune {
            let mut c = Clip::new("tune", 4 * BAR);
            let line = [
                67u8, 70, 72, 70, 68, 67, 65, 63, 62, 63, 65, 67, 65, 63, 62, 60,
            ];
            c.notes = line
                .iter()
                .enumerate()
                .map(|(i, &n)| Note::new(i as Tick * BEAT, BEAT * 3 / 4, n, 96))
                .collect();
            c.holds = Holds::Notes;
            lead.slots[0] = Some(c);
        }
        let mut drums = Track::new("Drums", 1);
        if beat {
            let mut c = Clip::new("beat", BAR);
            c.notes = (0..8)
                .map(|i| Note::new(i * BEAT / 2, 60, 2, 80))
                .chain([0, 2].map(|b| Note::new(b * BEAT, 60, 0, 110)))
                .chain([1, 3].map(|b| Note::new(b * BEAT, 60, 1, 105)))
                .collect();
            drums.slots[0] = Some(c);
        }
        p.tracks.extend([lead, drums, Track::new("Part", 2)]);
        p.give_clip_ids();
        p
    }

    fn add_ask(p: &Project, part: PartRole, relation: Relation) -> AddAsk {
        let parts = p
            .tracks
            .iter()
            .enumerate()
            .filter(|(_, t)| t.slots.get(0).is_some_and(Option::is_some))
            .map(|(i, t)| ToRead {
                track: i,
                slot: 0,
                role: if t.name == "Drums" {
                    PartRole::Drums
                } else {
                    PartRole::Melody
                },
            })
            .collect();
        AddAsk {
            row: 0,
            parts,
            lanes: Lanes::new(vec![
                (1, 0, Lane::Kick),
                (1, 1, Lane::Snare),
                (1, 2, Lane::Hats),
            ]),
            part,
            relation,
            kind: match part {
                PartRole::Bass => InstrumentKind::Bass,
                PartRole::Chords => InstrumentKind::Chords,
                PartRole::Drums => InstrumentKind::Percussive,
                PartRole::Melody => InstrumentKind::OneNote,
            },
            pads: [Some(36), Some(38), Some(42), None],
            swing: Swing::default(),
            transport_swing: Swing::default(),
            meter: Meter::default(),
            bpm: 100.0,
            budget: ReadBudget::default(),
            settings: Settings {
                style: GROOVE,
                bars: 4,
                seed: 3,
                ..Settings::default()
            },
            attempt: 0,
        }
    }

    /// Add a part on a row with a tune and a beat: a clip of the row's
    /// length over the chords heard, nothing to keep on the row's line,
    /// bound to the clips it heard; the same job gives the same part.
    #[test]
    fn a_part_added_to_a_row_is_a_clip_of_its_length_over_its_chords() {
        let p = tune_row(true, true);
        let job = AddPartJob::new(&p, add_ask(&p, PartRole::Bass, Relation::Support));
        assert_eq!(job.length, 4 * BAR);
        assert!(job.project.clip(2, 0).is_none());
        let done = job.run();
        assert_eq!(done.left_out, 0);
        let made = done.result.clone().unwrap();
        assert_eq!(made.clip.length, 4 * BAR);
        assert_eq!(made.clip.holds, Holds::Notes);
        assert!(!made.clip.notes.is_empty());
        assert!(made.clip.check().is_ok());
        assert_eq!(
            made.line, None,
            "heard chords aren't kept: they are heard again"
        );
        assert_eq!(made.chords.len(), 4 * BAR);
        assert_eq!(made.read.len(), 2);
        assert!(made.clip.notes.iter().all(|n| n.note < 60), "a bass");
        assert_eq!(job.run(), done);
        // Try again: another part.
        let mut again = add_ask(&p, PartRole::Bass, Relation::Support);
        again.attempt = 1;
        let other = AddPartJob::new(&p, again).run().result.unwrap();
        assert_ne!(other.clip.notes, made.clip.notes);
    }

    /// A beat alone: chords are suggested from the home chord, kept with
    /// the part as the row's line, and the next part plays over the same
    /// ones without keeping them again. Answer has no tune to answer.
    #[test]
    fn a_beat_alone_gets_suggested_chords_the_next_part_shares() {
        use mpfree_context::chord::Heard;
        let mut p = tune_row(false, true);
        let made = AddPartJob::new(&p, add_ask(&p, PartRole::Bass, Relation::Support))
            .run()
            .result
            .unwrap();
        let line = made.line.clone().expect("suggested chords to keep");
        assert!(line.spans().iter().all(|s| s.heard == Heard::Suggested));
        assert_eq!(
            made.chords
                .span_at(0)
                .unwrap()
                .answer
                .chord()
                .map(|c| c.root),
            Some(0)
        );
        // Kept: the clip in its cell and the line on the row.
        p.tracks[2].slots[0] = Some(made.clip.clone());
        p.give_clip_ids();
        p.scene_mut(0).unwrap().chords = line.clone();
        let mut ask = add_ask(&p, PartRole::Chords, Relation::Interlock);
        ask.parts.retain(|r| r.track != 2);
        ask.parts.push(ToRead {
            track: 2,
            slot: 0,
            role: PartRole::Bass,
        });
        let next = AddPartJob::new(&p, ask).run().result.unwrap();
        assert_eq!(next.line, None);
        assert_eq!(next.chords.names(), made.chords.names());
        // Answer needs a tune.
        let p = tune_row(false, true);
        let refused = AddPartJob::new(&p, add_ask(&p, PartRole::Chords, Relation::Answer)).run();
        assert_eq!(refused.result, Err(PartRefused::NothingToAnswer));
    }

    /// Chords picked on the panel and not kept yet: the part plays over
    /// them, a chord set stays, and nothing new is offered to keep (Keep
    /// keeps the panel's line itself).
    #[test]
    fn a_part_asked_over_picked_chords_plays_them() {
        use mpfree_context::chord::{Chord, Heard, Quality};
        let p = tune_row(true, true);
        let line = ChordTimeline::of(
            &[
                (Chord::new(8, Quality::Major), 2 * BAR),
                (Chord::new(10, Quality::Major), 2 * BAR),
            ],
            Heard::Set,
        );
        let made = AddPartJob::new(&p, add_ask(&p, PartRole::Bass, Relation::Support))
            .with_line(line)
            .run()
            .result
            .unwrap();
        assert_eq!(root_at(&made.chords, 0), Some(8), "{}", made.chords.names());
        assert_eq!(root_at(&made.chords, 3 * BAR), Some(10));
        assert_eq!(made.line, None);
        // The bass plays the picked root in the first bar.
        assert!(made
            .clip
            .notes
            .iter()
            .any(|n| n.tick < BAR && n.note % 12 == 8));
        // An empty line: the row's own chords again.
        let plain = AddPartJob::new(&p, add_ask(&p, PartRole::Bass, Relation::Support))
            .with_line(ChordTimeline::default())
            .run()
            .result
            .unwrap();
        assert_ne!(root_at(&plain.chords, 0), Some(8));
    }

    /// Hearing alone: the row's chords as they play, a chord set kept,
    /// and nothing suggested for a beat alone.
    #[test]
    fn a_row_heard_alone_gives_its_chords() {
        use mpfree_context::chord::{Chord, ChordAnswer, Heard, Quality};
        let mut p = tune_row(true, true);
        let job = AddPartJob::new(&p, add_ask(&p, PartRole::Bass, Relation::Support));
        let heard = job.hear();
        let made = job.run().result.unwrap();
        assert_eq!(root_at(&heard, 0), root_at(&made.chords, 0));
        assert!(root_at(&heard, 0).is_some(), "{}", heard.names());
        let mut line = heard.clone();
        assert!(line.set_at(0, ChordAnswer::Chord(Chord::new(8, Quality::Major))));
        p.scene_mut(0).unwrap().chords = line.plain();
        let again = AddPartJob::new(&p, add_ask(&p, PartRole::Bass, Relation::Support)).hear();
        assert_eq!(again.span_at(0).unwrap().heard, Heard::Set);
        assert_eq!(root_at(&again, 0), Some(8));
        let beat = tune_row(false, true);
        let none = AddPartJob::new(&beat, add_ask(&beat, PartRole::Bass, Relation::Support)).hear();
        assert!(
            none.spans().iter().all(|s| s.answer.chord().is_none()),
            "{}",
            none.names()
        );
    }

    /// Drums under a tune: hits only on the kit's pads, as a clip of hits.
    #[test]
    fn drums_added_under_a_tune_hit_only_the_kits_pads() {
        let p = tune_row(true, false);
        let made = AddPartJob::new(&p, add_ask(&p, PartRole::Drums, Relation::Support))
            .run()
            .result
            .unwrap();
        assert_eq!(made.clip.holds, Holds::Hits);
        assert!(!made.clip.notes.is_empty());
        assert!(made
            .clip
            .notes
            .iter()
            .all(|n| [36, 38, 42].contains(&n.note)));
        let mut ask = add_ask(&p, PartRole::Drums, Relation::Support);
        ask.pads = [None; 4];
        assert_eq!(
            AddPartJob::new(&p, ask).run().result,
            Err(PartRefused::NoDrumPads)
        );
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
