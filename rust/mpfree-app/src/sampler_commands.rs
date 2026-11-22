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
use std::ops::Range;

use mpfree_sampler::engine::MAX_KITS;
use mpfree_sampler::model::{ModelError, Project};
pub use mpfree_sampler_ui::command::{Change, Chopped, Landing, SceneSlot};

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
    /// Another kit the change's result goes onto, `kit` staying as it is
    /// (Chop to a new kit: the slices of `kit`'s pad or take on this kit's
    /// pads); None, `kit` itself. Only a chop goes onto another kit.
    pub onto: Option<InstrumentId>,
}

impl Command {
    /// `change` to kit `kit` (a place in the list of kits), a step of its
    /// own.
    pub fn to_kit(kit: usize, change: Change) -> Command {
        Command {
            kit: kit_id(kit),
            change,
            gesture: None,
            onto: None,
        }
    }

    /// The command as part of `gesture`'s undo step.
    pub fn in_gesture(self, gesture: Option<u64>) -> Command {
        Command { gesture, ..self }
    }

    /// The command's result onto kit `kit` (a place in the list of kits)
    /// instead: Chop to a new kit.
    pub fn onto(self, kit: usize) -> Command {
        Command {
            onto: Some(kit_id(kit)),
            ..self
        }
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
    /// Onto another kit, a change that only goes to its own (all but a
    /// chop).
    OntoAnotherKit,
}

/// As the page's log lines have always said a refused edit: the model's
/// reason by its name.
impl fmt::Display for Refused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Refused::NoKit(_) => write!(f, "NoKit"),
            Refused::Model(e) => write!(f, "{e:?}"),
            Refused::OntoAnotherKit => write!(f, "OntoAnotherKit"),
        }
    }
}

/// What a command did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Done {
    /// The kit it changed, as a place in the list of kits (the kit it went
    /// onto, for a chop onto another).
    pub kit: usize,
    /// It may have changed what the kit plays: its new view is owed to the
    /// audio thread. False for a change only saved (a chop kept).
    pub plays: bool,
    /// It renamed, replaced or retyped a pad, which the Play page shows.
    pub renamed: bool,
    /// The pads a chop filled.
    pub pads: Option<Range<usize>>,
    /// The take a recorded sound became, kept on the Sample page.
    pub take: Option<usize>,
    /// Something changed. False when the kit was already as the command
    /// asked (a take given its own name, back to scene A with nothing
    /// changed since): no undo step, nothing owed to the audio thread.
    pub changed: bool,
    /// Back to scene A: the pads put back, and those left as they are
    /// because their sounds are gone.
    pub put_back: Option<(usize, usize)>,
}

/// Runs `cmd` on `kits` (the list of kits): the kit its id names makes the
/// change as one undo step, or as part of its gesture's step. Refused, no
/// kit changes.
pub fn run(kits: &mut [Project], cmd: &Command) -> Result<Done, Refused> {
    let kit = kit_of(cmd.kit, kits.len()).ok_or(Refused::NoKit(cmd.kit))?;
    let onto = match cmd.onto {
        Some(id) => Some(kit_of(id, kits.len()).ok_or(Refused::NoKit(id))?),
        None => None,
    };
    let done = |kit, pads| Done {
        kit,
        plays: cmd.change.plays(),
        renamed: cmd.change.renames(),
        pads,
        take: None,
        changed: true,
        put_back: None,
    };
    match (&cmd.change, onto) {
        (_, Some(to)) if to == kit => Err(Refused::OntoAnotherKit),
        (Change::Chop { from, chop }, Some(to)) => {
            let (f, t) = two_mut(kits, kit, to);
            let pads = chop.place_in(f, *from, t).map_err(Refused::Model)?;
            // The take chopped leaves its kit's page, a step there.
            if let Chopped::Take(i) = from {
                kits[kit].remove_take(*i).map_err(Refused::Model)?;
            }
            Ok(done(to, Some(pads)))
        }
        (_, Some(_)) => Err(Refused::OntoAnotherKit),
        (change, None) => {
            let made = change
                .make(&mut kits[kit], cmd.gesture)
                .map_err(Refused::Model)?;
            let pads = match change {
                Change::Chop { chop, .. } => Some(chop.pads()),
                _ => None,
            };
            let done = done(kit, pads);
            Ok(Done {
                take: made.take,
                put_back: made.put_back,
                changed: !made.nothing,
                plays: done.plays && !made.nothing,
                renamed: done.renamed && !made.nothing,
                ..done
            })
        }
    }
}

/// Kits `a` (to read) and `b` (to change), two places in the list.
fn two_mut(kits: &mut [Project], a: usize, b: usize) -> (&Project, &mut Project) {
    if a < b {
        let (lo, hi) = kits.split_at_mut(b);
        (&lo[a], &mut hi[0])
    } else {
        let (lo, hi) = kits.split_at_mut(a);
        (&hi[0], &mut lo[b])
    }
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
                plays: true,
                renamed: false,
                pads: None,
                take: None,
                changed: true,
                put_back: None,
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
    fn a_chop_goes_onto_another_kit_and_only_a_chop_does() {
        let mut k = kits();
        let before = k.clone();
        let source = k[0].pad(0).unwrap().layers[0].region.source;
        let chop = mpfree_sampler_ui::command::Chop {
            source,
            name: "Chop".to_string(),
            mode: mpfree_sampler::model::ChopMode::Manual,
            bounds: vec![0, 480, 960],
            layer: 0,
            to_pad: 0,
            keys_from: None,
            one_pad: false,
        };
        let take = k[0].pad(0).cloned().expect("pad 0");
        let i = k[0].add_take(take).expect("a take");
        let from_take = Command::to_kit(
            0,
            Change::Chop {
                from: Chopped::Take(i),
                chop: Box::new(chop.clone()),
            },
        );
        let done = run(&mut k, &from_take.clone().onto(2)).expect("onto kit 3");
        assert_eq!((done.kit, done.pads, done.renamed), (2, Some(0..2), true));
        assert!(k[2].pad(1).is_some(), "the slices on kit 3");
        assert!(k[0].take(i).is_none(), "the take left kit 1's page");
        assert_eq!(k[0].pad(0), before[0].pad(0));
        // Onto itself, or another change onto another kit: refused.
        assert_eq!(
            run(&mut k, &from_take.onto(0)),
            Err(Refused::OntoAnotherKit)
        );
        let fade = Command::to_kit(0, Change::KeyFade(1)).onto(1);
        assert_eq!(run(&mut k, &fade), Err(Refused::OntoAnotherKit));
        assert_eq!(k[1].pad(0), before[1].pad(0));
    }

    #[test]
    fn a_command_that_finds_its_kit_as_it_asks_changes_nothing_and_owes_nothing() {
        let mut k = kits();
        let store = Command::to_kit(1, Change::SceneStore(SceneSlot::A));
        let done = run(&mut k, &store).expect("stored");
        assert!(done.changed && done.plays, "saved with the kit, a new view");
        let back = Command::to_kit(1, Change::SceneReturn);
        let done = run(&mut k, &back).expect("back to A");
        assert_eq!(
            (done.changed, done.plays, done.renamed, done.put_back),
            (false, false, false, Some((0, 0))),
            "nothing changed since A"
        );
        let pan = Command::to_kit(
            1,
            Change::Own {
                pad: 0,
                field: PadField::Pan,
                value: 0.5,
            },
        );
        run(&mut k, &pan).expect("panned");
        let done = run(&mut k, &back).expect("back to A");
        assert_eq!(
            (done.changed, done.plays, done.renamed, done.put_back),
            (true, true, true, Some((1, 0)))
        );
        // A take's edit goes to the take of the kit the id names.
        let take = k[2].pad(0).cloned().expect("pad 0");
        k[2].add_take(take).expect("a take");
        let named = Command::to_kit(
            2,
            Change::NameTake {
                take: 0,
                name: "Vox".to_string(),
            },
        );
        assert!(run(&mut k, &named).expect("named").changed);
        assert_eq!(k[2].take(0).map(|t| t.name.as_str()), Some("Vox"));
        assert!(!run(&mut k, &named).expect("the same name").changed);
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
