//! The catalogue: every kind of problem MPFree tells a musician about.
//!
//! The rules every entry passes (the System status plan):
//!
//! 1. **It needs the musician.** It says what to try, or it protects their
//!    work. A count with nothing to do about it stays on the Dev pages.
//! 2. **Normal playing is never a problem.** A voice stolen past a pad's
//!    polyphony is how pads work; clipping is a choice.
//! 3. **One kind, one card,** in musicians' words: no "underrun", "buffer",
//!    "lane", "period", "thread" or "DSP" (the test below checks).
//! 4. **A new kind joins here, not on the screen.** The design page of the
//!    feature that reports it says which level it is and why.
//!
//! These are the phase 1 kinds; disk streaming, themes, plugins and the
//! MIDI clock add theirs in phase 3 (`docs/dev/design/status.md`). The
//! words are drafts; any can change.

use crate::{Action, Kind, Level, Shape, System};

const TRY_FEWER: &str = "Play fewer sounds at once, or turn off an effect you don't need.";

// ---------------------------------------------------------------- Sound

/// The sound card ran out of sound to play: a gap you heard.
pub static SOUND_DROPOUT: Kind = Kind {
    id: "sound/dropout",
    system: System::Sound,
    level: Level::Fail,
    shape: Shape::Happens,
    short: "Sound dropped out",
    title: "The sound dropped out",
    what: "MPFree couldn't finish the sound in time, so you heard a short gap.",
    to_try: TRY_FEWER,
    action: None,
    manual: "the-sound-dropped-out",
    hides: &[],
};

/// One of the cores was late with its part of the sound, which was left
/// out for a moment (sampler plan step 10's "Sound skipped").
pub static SOUND_SKIPPED: Kind = Kind {
    id: "sound/skipped",
    system: System::Sound,
    level: Level::Fail,
    shape: Shape::Happens,
    short: "Sound skipped",
    title: "Part of the sound was skipped",
    what: "One of the MPC's cores was late with its part of the sound, so that part was left out for a moment.",
    to_try: TRY_FEWER,
    action: None,
    manual: "part-of-the-sound-was-skipped",
    hides: &[],
};

/// A full queue refused notes (the sampler's, an arp's).
pub static NOTES_DROPPED: Kind = Kind {
    id: "sound/notes-dropped",
    system: System::Sound,
    level: Level::Fail,
    shape: Shape::Happens,
    short: "Notes dropped",
    title: "Some notes didn't play",
    what: "More notes came in at once than the sound could take, so some of them didn't play.",
    to_try: "Play fewer notes at once, or slow down a busy arp or note repeat.",
    action: None,
    manual: "some-notes-didnt-play",
    hides: &[],
};

/// A note's ending found no room: something may keep sounding. It must
/// stay at 0; one is a bug.
pub static NOTE_MAY_HANG: Kind = Kind {
    id: "sound/note-may-hang",
    system: System::Sound,
    level: Level::Fail,
    shape: Shape::Happens,
    short: "A note may hang",
    title: "A note may be stuck on",
    what: "The end of a note got lost, so a sound may keep playing after you let go.",
    to_try: "Press Silence all notes. If it happens again, it is a bug: please report it.",
    action: Some(Action::SilenceNotes),
    manual: "a-note-may-be-stuck-on",
    hides: &[],
};

/// Load shedding faded out quiet voices to keep playing (sampler plan step
/// 10's "Voices given up"; profiling's load shedding).
pub static VOICES_GIVEN_UP: Kind = Kind {
    id: "sound/voices-given-up",
    system: System::Sound,
    level: Level::Warn,
    shape: Shape::Happens,
    short: "Voices given up",
    title: "Some voices were faded out",
    what: "The sound was close to running late, so MPFree faded out a few quiet voices to keep playing.",
    to_try: "Play fewer voices at once, or give the pads a shorter release.",
    action: None,
    manual: "some-voices-were-faded-out",
    hides: &[],
};

/// The grain pads wanted more grains than the sampler plays at once
/// (sampler plan steps 9 and 10).
pub static GRAINS_THINNED: Kind = Kind {
    id: "sound/grains-thinned",
    system: System::Sound,
    level: Level::Warn,
    shape: Shape::Lasts,
    short: "Grains thinned",
    title: "Grains thinned",
    what: "The grain pads wanted more grains at once than the sampler plays, so the oldest held pads thinned or went quiet.",
    to_try: "Hold fewer grain pads at once, or shorten their grains.",
    action: None,
    manual: "grains-thinned",
    hides: &[],
};

/// The audio loop's verdict was Tight or Over (engines.rs): close to
/// running late. It colours the chip's CPU number yellow (phase 2).
pub static NEAR_LIMIT: Kind = Kind {
    id: "sound/near-limit",
    system: System::Sound,
    level: Level::Warn,
    shape: Shape::Lasts,
    short: "Sound near limit",
    title: "The sound is near its limit",
    what: "The MPC has little time to spare making the sound. If it gets any busier you may hear gaps.",
    to_try: TRY_FEWER,
    action: None,
    manual: "the-sound-is-near-its-limit",
    hides: &[],
};

// ---------------------------------------------------------------- Memory

/// Free memory close to the floor kept for the system, or RAM over 85 %.
pub static MEMORY_LOW: Kind = Kind {
    id: "memory/low",
    system: System::Memory,
    level: Level::Warn,
    shape: Shape::Lasts,
    short: "Memory low",
    title: "Memory is running low",
    what: "Little memory is left for sounds, so the next sample or kit may not load.",
    to_try: "Remove samples you don't use, or save and start a smaller project.",
    action: None,
    manual: "memory-is-running-low",
    hides: &[],
};

/// A load was refused for memory (memory::SHORT). The hint line still says
/// why that load did nothing; this keeps it on the page.
pub static NOT_ENOUGH_MEMORY: Kind = Kind {
    id: "memory/not-enough",
    system: System::Memory,
    level: Level::Warn,
    shape: Shape::Happens,
    short: "Not enough memory",
    title: "Something didn't load: not enough memory",
    what: "A sample or kit was too big for the memory left, so it wasn't loaded.",
    to_try: "Remove samples you don't use, then load it again.",
    action: None,
    manual: "something-didnt-load-not-enough-memory",
    hides: &[],
};

// ---------------------------------------------------------------- Storage

/// The store's or the library's save failed ("not saved: ...").
pub static SAVE_FAILED: Kind = Kind {
    id: "storage/save-failed",
    system: System::Storage,
    level: Level::Fail,
    shape: Shape::Happens,
    short: "Save failed",
    title: "Saving didn't work",
    what: "MPFree couldn't write to the stick, so your latest changes aren't saved.",
    to_try: "Check the stick is in and has room, then save again.",
    action: None,
    manual: "saving-didnt-work",
    hides: &[],
};

// ---------------------------------------------------------------- Pads and buttons

/// The audio session isn't answering: the session chip's No session, No
/// reply or Stopped. Nothing the pads do can sound, so the sound's own
/// problems are its echoes and stay hidden while it lasts.
pub static NO_SESSION: Kind = Kind {
    id: "controls/no-session",
    system: System::Controls,
    level: Level::Fail,
    shape: Shape::Lasts,
    short: "Pads not answering",
    title: "The pads aren't making sound",
    what: "MPFree's sound stopped answering, so the pads and buttons can't play anything.",
    to_try: "Wait a few seconds. If it doesn't come back, restart MPFree.",
    action: None,
    manual: "the-pads-arent-making-sound",
    hides: &[
        &SOUND_DROPOUT,
        &SOUND_SKIPPED,
        &NOTES_DROPPED,
        &VOICES_GIVEN_UP,
        &NEAR_LIMIT,
    ],
};

/// The touch reader stopped ringing its doorbell.
pub static TOUCH_STOPPED: Kind = Kind {
    id: "controls/touch-stopped",
    system: System::Controls,
    level: Level::Warn,
    shape: Shape::Lasts,
    short: "Touch stopped",
    title: "The touchscreen stopped answering",
    what: "MPFree stopped getting touches from the screen. The pads, buttons, knobs and wheel still work.",
    to_try: "Use the buttons and the wheel for now. Restarting MPFree brings touch back.",
    action: None,
    manual: "the-touchscreen-stopped-answering",
    hides: &[],
};

// ---------------------------------------------------------------- MIDI and sync

/// A port the project uses is missing: the Controller, a track's MIDI out,
/// the clock it follows. The subject is the port; its name goes in front
/// of the short words on the page ("Juno-X not plugged in").
pub static PORT_MISSING: Kind = Kind {
    id: "midi/port-missing",
    system: System::Midi,
    level: Level::Warn,
    shape: Shape::Stays,
    short: "Not plugged in",
    title: "A MIDI device isn't plugged in",
    what: "The project uses a MIDI device that isn't connected, so nothing is sent to it or heard from it.",
    to_try: "Plug it in, or pick another port where the project uses it.",
    action: None,
    manual: "a-midi-device-isnt-plugged-in",
    hides: &[],
};

// ---------------------------------------------------------------- Project

/// The project opened with parts left out; it is saved as a new project
/// instead of over the old one.
pub static PARTS_MISSING: Kind = Kind {
    id: "project/parts-missing",
    system: System::Project,
    level: Level::Warn,
    shape: Shape::Stays,
    short: "Parts missing",
    title: "The project opened with parts missing",
    what: "Some samples or parts couldn't be found, so they were left out. It saves as a new project, so the original stays as it was.",
    to_try: "Press Show what's missing, then find or replace those parts.",
    action: Some(Action::ShowMissing),
    manual: "the-project-opened-with-parts-missing",
    hides: &[],
};

/// The last run crashed, so this start is a safe start (crash.rs): the new
/// project stays even when Prefs says Last project.
pub static CRASH_RESTART: Kind = Kind {
    id: "project/crash-restart",
    system: System::Project,
    level: Level::Fail,
    shape: Shape::Stays,
    short: "After a crash",
    title: "MPFree restarted after a crash",
    what: "MPFree stopped unexpectedly and started again with a new project, so a project that crashes it can't do it again at every start.",
    to_try: "Press Open the last project to carry on. If it crashes again, please report it.",
    action: Some(Action::OpenLastProject),
    manual: "mpfree-restarted-after-a-crash",
    hides: &[],
};

// ---------------------------------------------------------------- Screen

/// The GPU screen process fell back to drawing in software.
pub static NO_GPU: Kind = Kind {
    id: "screen/no-gpu",
    system: System::Screen,
    level: Level::Warn,
    shape: Shape::Stays,
    short: "No GPU drawing",
    title: "The screen is drawing without the GPU",
    what: "The graphics chip didn't start, so the screen is drawn the slow way and may feel less smooth.",
    to_try: "Restart MPFree. If it keeps happening, please report it.",
    action: None,
    manual: "the-screen-is-drawing-without-the-gpu",
    hides: &[],
};

// ---------------------------------------------------------------- The MPC

/// A strict sound helper couldn't get its core or priority, so its part
/// runs with the rest of the sound on one core (cores.rs).
pub static ONE_CORE: Kind = Kind {
    id: "mpc/one-core",
    system: System::Mpc,
    level: Level::Warn,
    shape: Shape::Stays,
    short: "Sound on one core",
    title: "The sound is running on fewer cores",
    what: "MPFree couldn't give the sound all the cores it asked for, so it has less room before it runs late.",
    to_try: "Restart MPFree. If it keeps happening, please report it.",
    action: None,
    manual: "the-sound-is-running-on-fewer-cores",
    hides: &[],
};

/// Every kind, in page order (by system, then worst first).
pub static ALL: &[&Kind] = &[
    &SOUND_DROPOUT,
    &SOUND_SKIPPED,
    &NOTES_DROPPED,
    &NOTE_MAY_HANG,
    &VOICES_GIVEN_UP,
    &GRAINS_THINNED,
    &NEAR_LIMIT,
    &MEMORY_LOW,
    &NOT_ENOUGH_MEMORY,
    &SAVE_FAILED,
    &NO_SESSION,
    &TOUCH_STOPPED,
    &PORT_MISSING,
    &CRASH_RESTART,
    &PARTS_MISSING,
    &NO_GPU,
    &ONE_CORE,
];

/// The kind with this id.
pub fn by_id(id: &str) -> Option<&'static Kind> {
    ALL.iter().copied().find(|k| k.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SHORT_MAX;
    use std::collections::HashSet;

    /// Words a musician shouldn't have to know (the plan's rule 3).
    const ENGINEER_WORDS: [&str; 6] = ["underrun", "buffer", "lane", "period", "thread", "dsp"];

    #[test]
    fn every_entry_is_filled_in() {
        for k in ALL {
            for (field, words) in [
                ("short", k.short),
                ("title", k.title),
                ("what", k.what),
                ("to_try", k.to_try),
                ("manual", k.manual),
            ] {
                assert!(!words.trim().is_empty(), "{}: {field} is empty", k.id);
            }
            assert!(
                k.what.ends_with('.') && k.to_try.ends_with('.'),
                "{}: what and to try are sentences",
                k.id
            );
            assert!(
                !k.title.ends_with('.'),
                "{}: a title has no full stop (the hint line adds one)",
                k.id
            );
        }
    }

    #[test]
    fn short_words_fit_the_systems_line() {
        for k in ALL {
            assert!(
                k.short.chars().count() <= SHORT_MAX,
                "{}: \"{}\" is longer than {SHORT_MAX}",
                k.id,
                k.short
            );
        }
    }

    #[test]
    fn the_words_are_musicians_words() {
        for k in ALL {
            let all = format!("{} {} {} {}", k.short, k.title, k.what, k.to_try).to_lowercase();
            for w in ENGINEER_WORDS {
                assert!(!all.contains(w), "{}: says \"{w}\"", k.id);
            }
        }
    }

    #[test]
    fn ids_are_unique_and_name_their_system() {
        let mut ids = HashSet::new();
        for k in ALL {
            assert!(ids.insert(k.id), "{} twice", k.id);
            assert!(
                k.id.starts_with(k.system.id_prefix()),
                "{} is in {:?}",
                k.id,
                k.system
            );
            let slug = &k.id[k.system.id_prefix().len()..];
            assert!(
                !slug.is_empty()
                    && slug
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
                "{}: a lower-case slug",
                k.id
            );
        }
        let manuals: HashSet<_> = ALL.iter().map(|k| k.manual).collect();
        assert_eq!(manuals.len(), ALL.len(), "one manual entry per kind");
    }

    #[test]
    fn all_is_in_page_order() {
        for w in ALL.windows(2) {
            assert!(
                w[0].system <= w[1].system,
                "{} before {}: by system",
                w[0].id,
                w[1].id
            );
            if w[0].system == w[1].system {
                assert!(
                    w[0].level >= w[1].level,
                    "{} before {}: worst first",
                    w[0].id,
                    w[1].id
                );
            }
        }
    }

    #[test]
    fn hidden_effects_are_other_kinds_in_the_catalogue() {
        for k in ALL {
            for e in k.hides {
                assert_ne!(e.id, k.id, "{} hides itself", k.id);
                assert!(by_id(e.id).is_some(), "{} hides {}, not in ALL", k.id, e.id);
                assert!(
                    !e.hides.contains(k),
                    "{} and {} hide each other",
                    k.id,
                    e.id
                );
            }
        }
    }

    #[test]
    fn a_red_problem_is_one_you_heard_or_could_lose_work_by() {
        // Fail is for faults heard, work at risk or MPFree not answering;
        // a state near a limit is never red (the plan's levels).
        for k in [&NEAR_LIMIT, &MEMORY_LOW, &GRAINS_THINNED, &VOICES_GIVEN_UP] {
            assert_eq!(k.level, Level::Warn, "{}", k.id);
        }
        for k in [&SOUND_DROPOUT, &SAVE_FAILED, &NO_SESSION] {
            assert_eq!(k.level, Level::Fail, "{}", k.id);
        }
    }

    #[test]
    fn a_button_is_named_in_what_to_try() {
        for k in ALL {
            if let Some(a) = k.action {
                assert!(
                    k.to_try.contains(a.label()),
                    "{}: to try names the button \"{}\"",
                    k.id,
                    a.label()
                );
            }
        }
    }
}
