//! Reading clips into a Musical Context (phase 0C): the listener's input
//! taken from the project, each clip with its revision so a stale reading
//! is never used.
//!
//! Every clip is heard as it plays, on the song's beats: its notes after
//! its quantise and speed, a shorter clip looping over the longest (as a
//! row plays them, `mpfree_seq::arrange::scene_length`). Pitched clips
//! (clips of notes) give the key and the chords; drum clips give the
//! groove, heard with the swing playback applies, so the template keeps
//! only the drummer's own timing. A clip of pad hits on a pitched part is
//! left out of the key and chords: a pad's pitch depends on the kit or the
//! pad layout, which a reading doesn't see.

use mpfree_context::groove::{GrooveTemplate, Hit, Lane, STEP};
use mpfree_context::listen::{listen, Played};
use mpfree_context::{ClipRead, ClipRef, MusicalContext, PartRole, ReadBudget};
use mpfree_seq::{Holds, Project};
use mpfree_transport::time::{Meter, Swing, Tick};

use crate::bind::revision;
use crate::timing::{heard, Playback};

/// A clip to read, and what it is in the music.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ToRead {
    pub track: usize,
    pub slot: usize,
    pub role: PartRole,
}

/// What a reading left out, said in words beside the result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LeftOut {
    /// A pitched part made of pad hits.
    PadHits { track: usize },
    /// No such clip.
    Missing { track: usize, slot: usize },
}

/// Read `clips` into a Musical Context. `lane_of` names the drum a pad of
/// a drum clip's track plays (the kit knows), or `None` for a pad that
/// isn't one. What was left out, too.
pub fn read(
    project: &Project,
    clips: &[ToRead],
    lane_of: &dyn Fn(usize, u8) -> Option<Lane>,
    transport_swing: Swing,
    meter: Meter,
    budget: ReadBudget,
) -> (MusicalContext, Vec<LeftOut>) {
    let mut left = Vec::new();
    let found: Vec<(ToRead, &mpfree_seq::Clip, Swing)> = clips
        .iter()
        .filter_map(|r| match project.clip(r.track, r.slot) {
            Some(c) => {
                let swing = c
                    .swing
                    .or(project.tracks[r.track].swing)
                    .unwrap_or(transport_swing);
                Some((*r, c, swing))
            }
            None => {
                left.push(LeftOut::Missing {
                    track: r.track,
                    slot: r.slot,
                });
                None
            }
        })
        .collect();
    // The length heard: the longest clip, in song ticks, to whole bars.
    let bar = meter.bar_ticks().max(1);
    let longest = found
        .iter()
        .map(|(_, c, _)| c.speed.to_song(c.length as f64).ceil() as Tick)
        .max()
        .unwrap_or(bar);
    let len = (longest + bar - 1) / bar * bar;

    let mut pitched: Vec<Played> = Vec::new();
    let mut hits: Vec<Hit> = Vec::new();
    let mut groove_swing = transport_swing;
    let mut read = Vec::new();
    for (r, c, swing) in &found {
        read.push(ClipRead {
            clip: ClipRef(c.id.0),
            revision: revision(c),
            role: r.role,
        });
        let pass = c.speed.to_song(c.length as f64);
        if pass <= 0.0 {
            continue;
        }
        let drums = r.role == PartRole::Drums;
        if !drums && c.holds != Holds::Notes {
            left.push(LeftOut::PadHits { track: r.track });
            continue;
        }
        if drums {
            groove_swing = *swing;
        }
        let passes = (len as f64 / pass).ceil() as Tick;
        for k in 0..passes {
            let pb = Playback {
                quantise: c.quantise,
                speed: c.speed,
                swing: *swing,
                length: c.length,
                start: (k as f64 * pass).round() as Tick,
            };
            for n in c.notes.iter().filter(|n| n.on) {
                if drums {
                    let Some(lane) = lane_of(r.track, n.note) else {
                        continue;
                    };
                    let at = heard(n.tick, &pb);
                    if at < len as f64 {
                        hits.push(Hit {
                            lane,
                            heard: at,
                            velocity: n.velocity,
                        });
                    }
                } else {
                    // On the beats, before swing.
                    let straight = Playback {
                        swing: Swing::default(),
                        ..pb
                    };
                    let at = heard(n.tick, &straight).round() as Tick;
                    if at < len {
                        pitched.push(Played::new(
                            at,
                            c.speed.to_song(n.len as f64).round() as Tick,
                            n.note,
                            n.velocity,
                        ));
                    }
                }
            }
        }
    }
    let heard = listen(&pitched, len, budget, meter);
    let bar_steps = (bar / STEP).max(1) as usize;
    let groove = (!hits.is_empty()).then(|| GrooveTemplate::read(&hits, bar_steps, groove_swing));
    (
        MusicalContext {
            key: heard.key,
            chords: heard.chords,
            groove,
            played_pcs: heard.played_pcs,
            motifs: Vec::new(),
            read,
            cut_short: heard.cut_short,
        },
        left,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bind::revision_in;
    use mpfree_seq::{Clip, Note, Speed, Track};
    use mpfree_transport::time::PPQ;

    const BEAT: Tick = PPQ as Tick;
    const BAR: Tick = 4 * BEAT;

    fn lanes(_track: usize, pad: u8) -> Option<Lane> {
        match pad {
            0 => Some(Lane::Kick),
            1 => Some(Lane::Snare),
            2 => Some(Lane::Hats),
            _ => None,
        }
    }

    /// A two-bar C minor bass, a one-bar Cm7 Fm7 pad that loops under it,
    /// and a one-bar beat with a late snare.
    fn project() -> Project {
        let mut p = Project::new();
        let mut bass = Track::new("Bass", 0);
        let mut b = Clip::new("bass", 2 * BAR);
        b.holds = Holds::Notes;
        b.notes = vec![
            Note::new(0, BEAT, 36, 100),
            Note::new(BEAT, BEAT, 43, 100),
            Note::new(2 * BEAT, BEAT, 41, 100),
            Note::new(3 * BEAT, BEAT, 39, 100),
            Note::new(BAR, BEAT, 41, 100),
            Note::new(BAR + BEAT, BEAT, 48, 100),
            Note::new(BAR + 2 * BEAT, BEAT, 44, 100),
            Note::new(BAR + 3 * BEAT, BEAT, 41, 100),
        ];
        bass.slots[0] = Some(b);
        let mut keys = Track::new("Keys", 1);
        let mut k = Clip::new("pad", BAR);
        k.holds = Holds::Notes;
        for (at, pcs) in [(0, [60, 63, 67, 70]), (2 * BEAT, [60, 65, 68, 63])] {
            for p in pcs {
                k.notes.push(Note::new(at, 2 * BEAT - 10, p, 80));
            }
        }
        keys.slots[0] = Some(k);
        let mut drums = Track::new("Drums", 2);
        let mut d = Clip::new("beat", BAR);
        for i in 0..4 {
            d.notes.push(Note::new(i * BEAT, 60, 0, 110));
            d.notes.push(Note::new(i * BEAT + BEAT / 2, 60, 2, 70));
        }
        // The snare on 2 and 4, a 1/64 late.
        d.notes.push(Note::new(BEAT + 60, 60, 1, 100));
        d.notes.push(Note::new(3 * BEAT + 60, 60, 1, 100));
        drums.slots[0] = Some(d);
        p.tracks.extend([bass, keys, drums]);
        p.give_clip_ids();
        p
    }

    fn row() -> [ToRead; 3] {
        [
            ToRead {
                track: 0,
                slot: 0,
                role: PartRole::Bass,
            },
            ToRead {
                track: 1,
                slot: 0,
                role: PartRole::Chords,
            },
            ToRead {
                track: 2,
                slot: 0,
                role: PartRole::Drums,
            },
        ]
    }

    #[test]
    fn a_row_is_read_as_it_plays() {
        let p = project();
        let (ctx, left) = read(
            &p,
            &row(),
            &lanes,
            Swing::default(),
            Meter::default(),
            ReadBudget::default(),
        );
        assert!(left.is_empty());
        assert_eq!(ctx.key.best().map(|k| k.name()), Some("C minor".into()));
        // The one-bar pad loops under the two-bar bass.
        assert_eq!(ctx.chords.len(), 2 * BAR);
        assert_eq!(ctx.chords.names(), "Cm7 Fm7 Cm7 Fm7");
        let g = ctx.groove.clone().expect("the drums give a groove");
        assert!(g.played);
        assert!(
            (g.offset_ticks(Lane::Snare, 4) - 60.0).abs() < 1.0,
            "the late snare"
        );
        assert!(g.offset_ticks(Lane::Kick, 0).abs() < 1.0);
        // Every clip read at its revision.
        assert_eq!(ctx.read.len(), 3);
        assert!(!ctx.is_stale(|c| revision_in(&p, c)));
    }

    #[test]
    fn an_edit_makes_the_reading_stale_and_pad_hits_are_left_out() {
        let mut p = project();
        let (ctx, _) = read(
            &p,
            &row(),
            &lanes,
            Swing::default(),
            Meter::default(),
            ReadBudget::default(),
        );
        p.tracks[1].slots[0].as_mut().unwrap().notes[0].velocity = 81;
        assert!(ctx.is_stale(|c| revision_in(&p, c)));
        p.tracks[1].slots[0].as_mut().unwrap().holds = Holds::Hits;
        let (_, left) = read(
            &p,
            &row(),
            &lanes,
            Swing::default(),
            Meter::default(),
            ReadBudget::default(),
        );
        assert_eq!(left, vec![LeftOut::PadHits { track: 1 }]);
    }

    #[test]
    fn a_half_speed_clip_is_heard_at_its_speed() {
        let mut p = project();
        p.tracks[1].slots[0].as_mut().unwrap().speed = Speed::new(1, 2).unwrap();
        let (ctx, _) = read(
            &p,
            &row()[..2],
            &lanes,
            Swing::default(),
            Meter::default(),
            ReadBudget::default(),
        );
        // Each chord now lasts a bar.
        assert_eq!(ctx.chords.names(), "Cm7 Fm7");
        assert_eq!(ctx.chords.spans()[1].start, BAR);
    }
}
