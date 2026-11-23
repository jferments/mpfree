//! Kits in a tuning (the tunings plan, step T2): a kit's keygroup notes
//! play the tuning's pitch for each key, and 16 Levels set to Tune step by
//! the tuning's keys. Pad hits and slices play as the pad is tuned, as drum
//! hits do everywhere.
//!
//! A keygroup's sound is taken to be recorded at its root's 12-TET pitch
//! (after its layer and pad Tune), so key `n` adds
//! [`Tuning::offset_cents`]`(n)` to its voice's tune: A4 = 432 moves a
//! keygroup as it moves a synth. The offset at the start is read into the
//! voice's step with its semitones ([`Voice::tuned`] keeps it), so it
//! reaches grain and warped voices as semitones do; after that a voice
//! follows a change of tuning and the pitch wheel through its program
//! Tune ([`Live::program_of`]), gliding there as it does for Tune. A bend
//! of `b` keys plays the tuning's pitch at `n + b`, as the synths' bends
//! do. In 12-TET every offset is exactly 0.0 and none of this adds or
//! changes a bit.

use super::*;

/// The root a 16 Levels Tune step on a pad with no keys counts from: C4.
pub(super) const LEVELS_ROOT: u8 = 60;

/// A kit's tuning, and whether it is anything but 12-TET at A4 = 440 Hz.
#[derive(Clone, Debug, Default)]
pub(super) struct KitTuning {
    pub(super) tuning: Tuning,
    pub(super) on: bool,
}

impl KitTuning {
    /// The cents key `note` sounds from its 12-TET pitch: 0.0 in 12-TET.
    #[inline]
    pub(super) fn offset(&self, note: f32) -> f32 {
        if self.on {
            self.tuning.offset_cents(note)
        } else {
            0.0
        }
    }

    /// The tuning's interval, in cents less 100 a key, from key `root` to
    /// `semis` keys above it: what a 16 Levels Tune step adds to its
    /// semitones, so pad 9 still plays as recorded. 0.0 in 12-TET.
    pub(super) fn interval(&self, root: u8, semis: i32) -> f32 {
        if !self.on || semis == 0 {
            return 0.0;
        }
        let r = f32::from(root);
        self.tuning.offset_cents(r + semis as f32) - self.tuning.offset_cents(r)
    }
}

impl Live {
    /// [`Live::program_for`] as voice `v` hears it in its kit's tuning `t`:
    /// a keygroup note that follows the tuning also moves from the offset
    /// its start took in ([`Voice::tuned`]) to the tuning's offset at its
    /// bent note.
    #[inline]
    pub(super) fn program_of(&self, v: &Voice, t: &KitTuning) -> [f32; ProgramField::COUNT] {
        self.program_tuned(v.note, v.tuned, t)
    }

    /// [`Live::program_of`] for a voice not made yet: `note`, and the
    /// offset `tuned` it starts with (None if it doesn't follow).
    pub(super) fn program_tuned(
        &self,
        note: u8,
        tuned: Option<f32>,
        t: &KitTuning,
    ) -> [f32; ProgramField::COUNT] {
        let mut program = self.program_for(note);
        if let Some(at) = tuned.filter(|&at| t.on || at != 0.0) {
            let bent = f32::from(note) + self.bend / 100.0;
            program[ProgramField::Tune as usize] += t.offset(bent) - at;
        }
        program
    }
}

impl Engine {
    /// Kit `kit`'s tuning (12-TET at A4 = 440 Hz until set): its keygroup
    /// notes play the tuning's pitch for each key, the sound's root at its
    /// 12-TET pitch, and 16 Levels set to Tune step by the tuning's keys
    /// from the pad's root (its keygroup root, else C4). Pad hits and
    /// slices don't change. Its keygroup voices playing move there,
    /// gliding as they do for Tune (a warped one keeps its pitch, as it
    /// does for a bend); a held 16 Levels voice keeps its pitch until the
    /// next hit. A kit out of range gets nothing. Never allocates; call it
    /// between renders, as [`Engine::set_tempo`].
    pub fn set_tuning(&mut self, kit: usize, tuning: &Tuning) {
        let Some(t) = self.tunings.get_mut(kit) else {
            return;
        };
        if t.tuning == *tuning {
            return;
        }
        t.tuning.clone_from(tuning);
        t.on = !tuning.is_standard();
        let (live, t) = (&self.lives[kit], &self.tunings[kit]);
        for (v, _) in self.pool.iter_mut() {
            let p = usize::from(v.pad);
            if v.active && usize::from(v.kit) == kit && p < PADS && v.tuned.is_some() {
                retarget(v, &live.row(p), &live.program_of(v, t));
            }
        }
        // Split, with the lane's share away: its voices follow when it
        // comes back, as for an edit.
        if self.pool.split && self.pool.b.is_none() {
            self.edit_lane[kit] = true;
        }
    }

    /// A 16 Levels step `k` on pad `pad` of the kit being played: by Tune,
    /// its semitones are keys of the kit's tuning from the pad's root (its
    /// keygroup root, else C4), so pad 9 plays as recorded.
    pub(super) fn level_vary(&self, pad: u8, by: Levels, k: u8) -> Vary {
        let mut vary = Vary::level(by, k);
        let t = &self.tunings[self.at];
        if t.on && vary.semis != 0 {
            let root = self.views[self.at]
                .pads
                .get(usize::from(pad))
                .and_then(Option::as_ref)
                .and_then(|vp| vp.keys)
                .map_or(LEVELS_ROOT, |k| k.root);
            vary.cents = t.interval(root, vary.semis);
        }
        vary
    }

    /// Kit `kit`'s tuning (None out of range).
    pub fn kit_tuning(&self, kit: usize) -> Option<&Tuning> {
        self.tunings.get(kit).map(|t| &t.tuning)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Layer, Mono, Pad, Region, Source, Warp, WarpMethod};
    use mpfree_tuning::Builtin;

    const RATE: u32 = 44_100;
    const ROOT: u8 = 57;

    fn engine(region: impl Fn(&Source) -> Region, mono: Mono) -> Engine {
        let s = Source::new("tone", RATE, 1, vec![0.25; RATE as usize * 2]).unwrap();
        let mut p = Project::new();
        p.add_source(s.clone());
        let mut pad = Pad::new("keys", Layer::new(region(&s)));
        pad.mono = mono;
        p.set_pad(0, Some(pad), None).unwrap();
        p.spread_to_keys(0, ROOT).unwrap();
        let mut e = Engine::new(RATE, 8);
        e.set_view(EngineView::build(&p, RATE));
        e
    }

    fn voice(e: &Engine) -> &Voice {
        e.pool
            .iter()
            .map(|(v, _)| v)
            .find(|v| v.active && v.kill_left == 0)
            .expect("a voice")
    }

    fn key(e: &mut Engine, note: u8) {
        e.apply(Event::Key {
            note,
            velocity: 100,
        });
    }

    #[test]
    fn grain_and_warped_keygroups_take_the_tuning_as_they_take_semitones() {
        let just = Builtin::Just.tuning(0, 440.0);
        let e_off = just.offset_cents(64.0);
        assert!((e_off + 13.686).abs() < 0.01);
        // A grain keygroup: the key's hit, in semitones, has the offset.
        let grains = |r: &Source| Region {
            mode: PlayMode::Granular,
            ..Region::whole(r)
        };
        let mut e = engine(grains, Mono::default());
        e.set_tuning(0, &just);
        key(&mut e, 64);
        let v = voice(&e);
        assert!(v.granular);
        assert!(
            (v.grain_hit - (7.0 + e_off / 100.0)).abs() < 1e-4,
            "{}",
            v.grain_hit
        );
        assert_eq!(v.tuned, Some(e_off));
        assert!((v.key_cents - (700.0 + e_off)).abs() < 1e-3);
        // A warped keygroup: its step reads the offset as its semitones.
        let warped = |r: &Source| Region {
            warp: Some(Warp {
                bpm: 120.0,
                method: WarpMethod::Repitch,
                pins: Vec::new(),
            }),
            ..Region::whole(r)
        };
        let step = |t: &Tuning| {
            let mut e = engine(warped, Mono::default());
            e.set_tuning(0, t);
            key(&mut e, 64);
            voice(&e).plain_step as f64
        };
        let ratio = step(&just) / step(&Tuning::standard());
        let want = f64::from(e_off / 1200.0).exp2();
        assert!((ratio / want - 1.0).abs() < 1e-6, "{ratio} vs {want}");
    }

    #[test]
    fn legato_keeps_the_keys_cents_and_tuning_up_to_date() {
        let just = Builtin::Just.tuning(0, 440.0);
        let mono = Mono {
            on: true,
            legato: true,
            ..Mono::default()
        };
        let mut e = engine(Region::whole, mono);
        e.set_tuning(0, &just);
        key(&mut e, 64);
        let started = voice(&e).age;
        key(&mut e, 67);
        let v = voice(&e);
        assert_eq!(v.age, started, "legato moved the voice, not a new one");
        let g = just.offset_cents(67.0);
        assert_eq!(v.tuned, Some(g));
        assert!((v.key_cents - (1000.0 + g)).abs() < 1e-3, "{}", v.key_cents);
        // In 12-TET, the key's cents are its semitones.
        let mut e = engine(Region::whole, Mono::default());
        key(&mut e, 64);
        assert_eq!(voice(&e).key_cents, 700.0);
        assert_eq!(voice(&e).tuned, Some(0.0));
    }
}
