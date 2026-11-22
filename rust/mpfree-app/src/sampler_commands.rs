//! Sampler commands: a change to a kit, and the kit it goes to (the
//! sampler refactor's step 12, `docs/dev/design/sampler-architecture.md`).
//!
//! A [`Command`] is one musical change ([`Change`], the kit's half in
//! `mpfree_sampler_ui::command`) and the kit it changes. The Sample page's
//! touches, the Play page's Q-Links and MIDI Learn each make one and hand
//! it to [`run`], so they share one path into the kits, and a command runs
//! in a test without the screen. The page keeps drawing, taking input and
//! saying what happened.
//!
//! **A kit is named by its instrument id, never by a track.** Instruments
//! make the sound and tracks record and play back to them, so a command
//! keeps its shape when kits join the project's list of instruments (the
//! refactor's phase 3, with the instrument redesign). Until then the list
//! has no kits, and kit `k` in the list of kits answers to a stand-in id,
//! [`kit_id`]: past any number a list gives out (it counts up one at a
//! time from 2), never saved and never shown. Phase 3 swaps [`kit_id`] and
//! [`kit_of`] for the list's own ids; nothing that makes or runs a command
//! changes.

use std::fmt;

use mpfree_sampler::engine::MAX_KITS;
use mpfree_sampler::model::{ModelError, Project};
pub use mpfree_sampler_ui::command::Change;

use crate::instrument_list::InstrumentId;

/// The stand-in id of the first kit, until kits join the list of
/// instruments: the top half of the numbers, which a list that gives out
/// one number at a time from 2 never reaches.
const FIRST_KIT_ID: u32 = 1 << 31;

/// The id kit `kit` (its place in the list of kits, from 0) answers to.
pub fn kit_id(kit: usize) -> InstrumentId {
    let n = FIRST_KIT_ID + (kit.min(MAX_KITS) as u32);
    InstrumentId::from_number(n).expect("the stand-in ids start past 0")
}

/// The kit that answers to `id`, as a place in a list of `kits` kits, or
/// None (not a kit's id, or a kit the list hasn't got).
pub fn kit_of(id: InstrumentId, kits: usize) -> Option<usize> {
    (0..kits.min(MAX_KITS)).find(|&k| kit_id(k) == id)
}

/// A change to a kit, the kit named by its instrument id.
#[derive(Clone, Debug, PartialEq)]
pub struct Command {
    /// The kit it changes ([`kit_id`]).
    pub kit: InstrumentId,
    pub change: Change,
    /// The same for every change of one drag or turn, so they make one
    /// undo step; None is a step of its own.
    pub gesture: Option<u64>,
}

impl Command {
    /// `change` to kit `kit` (a place in the list of kits), a step of its
    /// own.
    pub fn to_kit(kit: usize, change: Change) -> Command {
        Command {
            kit: kit_id(kit),
            change,
            gesture: None,
        }
    }

    /// The command as part of `gesture`'s undo step.
    pub fn in_gesture(self, gesture: Option<u64>) -> Command {
        Command { gesture, ..self }
    }
}

/// Why a command changed nothing.
#[derive(Clone, Debug, PartialEq)]
pub enum Refused {
    /// No kit answers to the id.
    NoKit(InstrumentId),
    /// The kit refused the change (no such pad, layer or sound, or a pad
    /// the model doesn't take).
    Model(ModelError),
}

/// As the page's log lines have always said a refused edit: the model's
/// reason by its name.
impl fmt::Display for Refused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Refused::NoKit(_) => write!(f, "NoKit"),
            Refused::Model(e) => write!(f, "{e:?}"),
        }
    }
}

/// What a command did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Done {
    /// The kit it changed, as a place in the list of kits: its new view is
    /// owed to the audio thread.
    pub kit: usize,
    /// It renamed, replaced or retyped a pad, which the Play page shows.
    pub renamed: bool,
}

/// Runs `cmd` on `kits` (the list of kits): the kit its id names makes the
/// change as one undo step, or as part of its gesture's step. Refused, no
/// kit changes.
pub fn run(kits: &mut [Project], cmd: &Command) -> Result<Done, Refused> {
    let kit = kit_of(cmd.kit, kits.len()).ok_or(Refused::NoKit(cmd.kit))?;
    cmd.change
        .apply(&mut kits[kit], cmd.gesture)
        .map_err(Refused::Model)?;
    Ok(Done {
        kit,
        renamed: cmd.change.renames(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::instrument_list::InstrumentList;
    use mpfree_sampler::engine::PadField;
    use mpfree_sampler::model::{Layer, Pad, Region, Source};
    use mpfree_sampler_ui::command::PadKnob;

    /// Three kits, each with a sound on pad 0.
    fn kits() -> Vec<Project> {
        (0..3)
            .map(|k| {
                let mut p = Project::new();
                let data: Vec<f32> = (0..960).map(|i| ((i + k) as f32 * 0.1).sin()).collect();
                let s = Source::new("Hit", 48_000, 1, data).expect("a sound");
                let r = Region::whole(&s);
                p.add_source(s);
                p.set_pad(0, Some(Pad::new("Hit", Layer::new(r))), None)
                    .expect("a pad");
                p
            })
            .collect()
    }

    #[test]
    fn a_kit_answers_to_its_id_and_no_instrument_of_the_list_is_a_kit() {
        for k in [0, 1, 5, MAX_KITS - 1] {
            assert_eq!(kit_of(kit_id(k), MAX_KITS), Some(k));
        }
        assert_eq!(kit_of(kit_id(5), 3), None, "a kit the list hasn't got");
        // The shared synth and the ids a list gives tracks' instruments
        // are never a kit's.
        let mut list = InstrumentList::new();
        assert!(list.follow(0b111));
        let given: Vec<InstrumentId> = (0..3).filter_map(|t| list.own_of(t)).collect();
        assert_eq!(given.len(), 3);
        for id in given.into_iter().chain([InstrumentId::SHARED_SYNTH]) {
            assert_eq!(kit_of(id, MAX_KITS), None, "instrument {id}");
        }
    }

    #[test]
    fn a_command_changes_only_the_kit_its_id_names() {
        let mut k = kits();
        let before = k.clone();
        let cmd = Command::to_kit(
            1,
            Change::Own {
                pad: 0,
                field: PadField::Level,
                value: -6.0,
            },
        );
        assert_eq!(
            run(&mut k, &cmd),
            Ok(Done {
                kit: 1,
                renamed: false
            })
        );
        assert_eq!(k[1].pad(0).map(|p| p.level_db), Some(-6.0));
        assert_eq!(k[0].pad(0), before[0].pad(0));
        assert_eq!(k[2].pad(0), before[2].pad(0));
        // A pad replaced says so, for the Play page's names.
        let put = Command::to_kit(
            2,
            Change::Pad {
                pad: 4,
                to: before[2].pad(0).cloned().map(Box::new),
            },
        );
        assert_eq!(run(&mut k, &put).map(|d| d.renamed), Ok(true));
    }

    #[test]
    fn a_refused_command_changes_no_kit_and_says_why_as_the_page_did() {
        let mut k = kits();
        let before = k.clone();
        let nowhere = Command::to_kit(7, Change::KeyFade(2));
        assert_eq!(run(&mut k, &nowhere), Err(Refused::NoKit(kit_id(7))));
        let no_pad = Command::to_kit(
            0,
            Change::Turn {
                pad: 9,
                knob: PadKnob::Tune,
                detents: 1,
                fine: false,
            },
        );
        let e = run(&mut k, &no_pad).unwrap_err();
        assert_eq!(e, Refused::Model(ModelError::BadPad));
        assert_eq!(e.to_string(), format!("{:?}", ModelError::BadPad));
        for (a, b) in k.iter().zip(&before) {
            assert_eq!(a.pad(0), b.pad(0));
            assert!(!a.clone().undo() || b.clone().undo());
        }
    }

    #[test]
    fn a_gesture_makes_one_step_across_commands() {
        let mut k = kits();
        let turn = |d| {
            Command::to_kit(
                0,
                Change::Turn {
                    pad: 0,
                    knob: PadKnob::Tune,
                    detents: d,
                    fine: false,
                },
            )
            .in_gesture(Some(42))
        };
        for _ in 0..4 {
            run(&mut k, &turn(1)).expect("a turn");
        }
        assert_eq!(k[0].pad(0).map(|p| p.tune_cents), Some(400));
        assert!(k[0].undo());
        assert_eq!(k[0].pad(0).map(|p| p.tune_cents), Some(0), "one step");
    }
}
