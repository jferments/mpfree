//! Every effect MPFree ships, in one table: its node id, name, a line
//! about it for the picker, its category, how to make one and its factory
//! presets.
//!
//! This is the one list an effect has to join. The "+ FX" picker groups by
//! [`Category`], project files make nodes by id through [`make`], and the
//! effect pages find presets here. Effect pages may also name their own
//! pages and drawings (in `mpfree-fx-ui`), but an effect without them still
//! gets a page from its parameter groups.
//!
//! Hosted plugins are not in this table (they are found at run time); a
//! picker lists them under [`Category::Plugins`] beside these.

use mpfree_node::Node;

use crate::presets as p;
use crate::{
    AutoPan, Character, Chorus, Cloud, Comb, Compressor, Crush, DeEsser, Delay, DjFilter, Drive,
    Ducker, Eq, Flanger, Gate, Limiter, LoFi, MasterSafety, MultiFilter, MultiTap,
    MultibandCompressor, Perform, Phaser, PitchShifter, Plate, Preset, RingMod, Room, Rotary,
    Spring, TapeDelay, TransientShaper, Tremolo,
};

/// Where an effect sits in the picker.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Category {
    Dynamics,
    EqFilter,
    DriveColour,
    Modulation,
    DelayReverb,
    PitchVocal,
    Performance,
    /// Hosted plugins (none in [`EFFECTS`]).
    Plugins,
    /// The master's safety stage.
    Utility,
}

impl Category {
    /// Every category, in the picker's order.
    pub const ALL: [Category; 9] = [
        Category::Dynamics,
        Category::EqFilter,
        Category::DriveColour,
        Category::Modulation,
        Category::DelayReverb,
        Category::PitchVocal,
        Category::Performance,
        Category::Plugins,
        Category::Utility,
    ];

    /// The category's name as a picker heading shows it.
    pub fn name(self) -> &'static str {
        match self {
            Category::Dynamics => "Dynamics",
            Category::EqFilter => "EQ and filter",
            Category::DriveColour => "Drive and colour",
            Category::Modulation => "Modulation",
            Category::DelayReverb => "Delay and reverb",
            Category::PitchVocal => "Pitch and vocal",
            Category::Performance => "Performance",
            Category::Plugins => "Plugins",
            Category::Utility => "Utility",
        }
    }
}

/// One effect MPFree ships.
#[derive(Clone, Copy)]
pub struct FxEntry {
    /// The node id, as project files store it (`"mpfree.fx.delay"`).
    pub id: &'static str,
    /// The name the picker and the effect page show.
    pub name: &'static str,
    /// One line about what it does, under its name in the picker.
    pub about: &'static str,
    pub category: Category,
    /// A new node at its defaults.
    pub make: fn() -> Box<dyn Node>,
    /// Factory presets (empty for an effect with no sound to pick).
    pub presets: &'static [Preset],
    /// Offered only on the master strip.
    pub master_only: bool,
}

impl std::fmt::Debug for FxEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FxEntry")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("category", &self.category)
            .finish()
    }
}

const fn e(
    id: &'static str,
    name: &'static str,
    about: &'static str,
    category: Category,
    make: fn() -> Box<dyn Node>,
    presets: &'static [Preset],
) -> FxEntry {
    FxEntry {
        id,
        name,
        about,
        category,
        make,
        presets,
        master_only: false,
    }
}

use Category::*;

/// Every effect, grouped by category in the picker's order.
pub static EFFECTS: &[FxEntry] = &[
    e(
        "mpfree.fx.compressor",
        "Compressor",
        "Glue and pump",
        Dynamics,
        || Box::new(Compressor::new()),
        p::COMPRESSOR,
    ),
    e(
        "mpfree.fx.ducker",
        "Ducker",
        "Pump from the kick or the tempo",
        Dynamics,
        || Box::new(Ducker::new()),
        p::DUCKER,
    ),
    e(
        "mpfree.fx.transient",
        "Transient shaper",
        "More or less punch and tail",
        Dynamics,
        || Box::new(TransientShaper::new()),
        p::TRANSIENT,
    ),
    e(
        "mpfree.fx.limiter",
        "Limiter",
        "Loud, with nothing over",
        Dynamics,
        || Box::new(Limiter::new()),
        crate::dynamics::LIMITER_PRESETS,
    ),
    e(
        "mpfree.fx.gate",
        "Gate",
        "Silence between the hits",
        Dynamics,
        || Box::new(Gate::new()),
        crate::dynamics::GATE_PRESETS,
    ),
    e(
        "mpfree.fx.deesser",
        "De-esser",
        "Tames harsh S sounds",
        Dynamics,
        || Box::new(DeEsser::new()),
        crate::dynamics::DEESSER_PRESETS,
    ),
    e(
        "mpfree.fx.multiband",
        "Multiband comp",
        "Low, mid and high apart",
        Dynamics,
        || Box::new(MultibandCompressor::new()),
        crate::dynamics::MULTIBAND_PRESETS,
    ),
    e(
        "mpfree.fx.eq",
        "EQ",
        "Shelves and bands",
        EqFilter,
        || Box::new(Eq::new()),
        p::EQ,
    ),
    e(
        "mpfree.fx.filter",
        "Multi filter",
        "Low, high, band, notch",
        EqFilter,
        || Box::new(MultiFilter::new()),
        p::FILTER,
    ),
    e(
        "mpfree.fx.dj_filter",
        "DJ filter",
        "One knob: low-pass to high-pass",
        EqFilter,
        || Box::new(DjFilter::new()),
        &[],
    ),
    e(
        "mpfree.fx.comb",
        "Comb",
        "Tuned resonance",
        EqFilter,
        || Box::new(Comb::new()),
        p::COMB,
    ),
    e(
        "mpfree.fx.drive",
        "Drive",
        "Tube, tape, fold, fuzz",
        DriveColour,
        || Box::new(Drive::new()),
        p::DRIVE,
    ),
    e(
        "mpfree.fx.character",
        "Character",
        "Console, tape, iron, soften",
        DriveColour,
        || Box::new(Character::new()),
        p::CHARACTER,
    ),
    e(
        "mpfree.fx.crush",
        "Crush",
        "Bits and sample rate",
        DriveColour,
        || Box::new(Crush::new()),
        p::CRUSH,
    ),
    e(
        "mpfree.fx.lofi",
        "Lo-fi",
        "Wear, wow and noise",
        DriveColour,
        || Box::new(LoFi::new()),
        crate::lofi::PRESETS,
    ),
    e(
        "mpfree.fx.chorus",
        "Chorus",
        "Width and shimmer",
        Modulation,
        || Box::new(Chorus::new()),
        p::CHORUS,
    ),
    e(
        "mpfree.fx.flanger",
        "Flanger",
        "Jet sweeps",
        Modulation,
        || Box::new(Flanger::new()),
        p::FLANGER,
    ),
    e(
        "mpfree.fx.phaser",
        "Phaser",
        "Swirling notches",
        Modulation,
        || Box::new(Phaser::new()),
        p::PHASER,
    ),
    e(
        "mpfree.fx.tremolo",
        "Tremolo",
        "Wobble, chop or swirl",
        Modulation,
        || Box::new(Tremolo::new()),
        crate::motion::TREMOLO_PRESETS,
    ),
    e(
        "mpfree.fx.autopan",
        "Auto-pan",
        "Moves the sound side to side",
        Modulation,
        || Box::new(AutoPan::new()),
        crate::motion::AUTOPAN_PRESETS,
    ),
    e(
        "mpfree.fx.ringmod",
        "Ring mod",
        "Bells, clang and robots",
        Modulation,
        || Box::new(RingMod::new()),
        crate::motion::RINGMOD_PRESETS,
    ),
    e(
        "mpfree.fx.rotary",
        "Rotary",
        "Spinning organ speaker",
        Modulation,
        || Box::new(Rotary::new()),
        crate::motion::ROTARY_PRESETS,
    ),
    e(
        "mpfree.fx.delay",
        "Delay",
        "Synced echoes",
        DelayReverb,
        || Box::new(Delay::new()),
        p::DELAY,
    ),
    e(
        "mpfree.fx.tape_delay",
        "Tape delay",
        "Wow, flutter and dub repeats",
        DelayReverb,
        || Box::new(TapeDelay::new()),
        crate::echoes::TAPE_PRESETS,
    ),
    e(
        "mpfree.fx.multitap",
        "Multi-tap delay",
        "Four echoes on the grid",
        DelayReverb,
        || Box::new(MultiTap::new()),
        crate::echoes::MULTITAP_PRESETS,
    ),
    e(
        "mpfree.fx.cloud",
        "Cloud reverb",
        "Plate, room, hall, cloud",
        DelayReverb,
        || Box::new(Cloud::new()),
        crate::cloud::PRESETS,
    ),
    e(
        "mpfree.fx.room",
        "Room reverb",
        "Booths to studios, gated too",
        DelayReverb,
        || Box::new(Room::new()),
        crate::rooms::ROOM_PRESETS,
    ),
    e(
        "mpfree.fx.plate",
        "Plate reverb",
        "The classic bright plate",
        DelayReverb,
        || Box::new(Plate::new()),
        crate::rooms::PLATE_PRESETS,
    ),
    e(
        "mpfree.fx.spring",
        "Spring reverb",
        "Amp tank drip and splash",
        DelayReverb,
        || Box::new(Spring::new()),
        crate::rooms::SPRING_PRESETS,
    ),
    e(
        "mpfree.fx.pitch",
        "Pitch shifter",
        "Octaves, harmony, doubling",
        PitchVocal,
        || Box::new(PitchShifter::new()),
        p::PITCH,
    ),
    e(
        "mpfree.fx.perform",
        "Perform FX",
        "Throws, stutters, the XY pad",
        Performance,
        || Box::new(Perform::new()),
        &[],
    ),
    FxEntry {
        master_only: true,
        ..e(
            "mpfree.fx.master_safety",
            "Master safety",
            "Keeps the output from clipping",
            Utility,
            || Box::new(MasterSafety::new()),
            &[],
        )
    },
];

/// The effect with node id `id`.
pub fn entry(id: &str) -> Option<&'static FxEntry> {
    EFFECTS.iter().find(|e| e.id == id)
}

/// A new effect by node id, at its defaults.
pub fn make(id: &str) -> Option<Box<dyn Node>> {
    entry(id).map(|e| (e.make)())
}

/// The effects a strip's picker offers (Master safety only on the
/// master), in the table's order.
pub fn offered(master: bool) -> impl Iterator<Item = &'static FxEntry> {
    EFFECTS.iter().filter(move |e| master || !e.master_only)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_entry_makes_its_own_node_once() {
        for (i, e) in EFFECTS.iter().enumerate() {
            let n = (e.make)();
            assert_eq!(n.info().id, e.id);
            assert_eq!(
                n.info().name,
                e.name,
                "{}: the page and the picker agree",
                e.id
            );
            assert!(!e.about.is_empty(), "{}", e.id);
            assert!(EFFECTS[..i].iter().all(|o| o.id != e.id), "{} twice", e.id);
            assert_ne!(e.category, Category::Plugins);
        }
        assert!(make("mpfree.fx.nothing").is_none());
    }

    #[test]
    fn the_table_is_grouped_in_category_order() {
        let rank = |c: Category| Category::ALL.iter().position(|&a| a == c).unwrap();
        for w in EFFECTS.windows(2) {
            assert!(
                rank(w[0].category) <= rank(w[1].category),
                "{} before {}",
                w[0].id,
                w[1].id
            );
        }
        assert_eq!(offered(false).count() + 1, offered(true).count());
    }
}
