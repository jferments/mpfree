//! MPFree's own sounds in the library, so BROWSE has sounds that aren't
//! Akai's: once per stick, MPFree's own drum kits (the Drums synth's kits,
//! rendered when the stick was made: kits.bin) are written as kit folders in
//! `Kits/MPFree/`, one folder per kit with `kit.txt` and its sounds
//! (`mpfree_library::kit`).
//!
//! - It runs on a thread of its own, at the background priority, through
//!   `mpfree-library` (inside `library/` on the `--data` folder only).
//! - It is done once: the browser's lists keep `bundled = mpfree-kits 1`
//!   ([`TAG`]) when it has run, so a kit the musician deletes stays deleted.
//!   A later version with more sounds changes the tag and adds only what is
//!   missing (a kit folder already there is left as it is).
//! - Nothing is written when the pads have none of MPFree's own kits (an
//!   older stick): the next start looks again.

use mpfree_library::{Library, Loc, NewSample, Place};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;

/// What has been put in the library, as the browser's lists keep it.
pub const TAG: &str = "mpfree-kits 1";
/// The folder in Kits.
pub const FOLDER: &str = "MPFree";
/// The prefix MPFree's own kits' names carry (`mpfree_factory::drums`).
const PREFIX: &str = "MPFree ";

pub use crate::sample_page::KitFile;

/// Whether `name` is one of MPFree's own kits (one per Drums synth kit).
pub fn own_kit(name: &str) -> bool {
    mpfree_synth::Drums::presets()
        .iter()
        .any(|p| name.strip_prefix(PREFIX) == Some(p.name))
}

/// The folder name a kit gets in `Kits/MPFree/`: without the prefix.
pub fn folder_name(kit: &str) -> &str {
    kit.strip_prefix(PREFIX).unwrap_or(kit)
}

/// Starts writing `kits` on a thread; its answer is how many kit folders
/// were made, or why it stopped.
pub fn start(lib: Library, kits: Vec<KitFile>) -> Result<Receiver<Result<usize, String>>, String> {
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("mpfree-bundle".into())
        .spawn(move || {
            let _ = crate::cores::background(); // off the sound's cores and priority
            let _ = tx.send(write(&lib, kits));
        })
        .map_err(|e| e.to_string())?;
    Ok(rx)
}

/// Writes each kit not already there into `Kits/MPFree/`.
pub fn write(lib: &Library, kits: Vec<KitFile>) -> Result<usize, String> {
    let e = |e: std::io::Error| e.to_string();
    let top = Loc::top(Place::Kits);
    let folder = top.join(FOLDER).map_err(e)?;
    if !lib.list(&top).map_err(e)?.iter().any(|x| x.name == FOLDER) {
        lib.new_folder(&top, FOLDER).map_err(e)?;
    }
    let there: Vec<String> = lib
        .list(&folder)
        .map_err(e)?
        .into_iter()
        .map(|x| x.name)
        .collect();
    let mut made = 0;
    for (name, text, sounds) in kits {
        let name = folder_name(&name).to_string();
        if there.iter().any(|t| t.eq_ignore_ascii_case(&name)) {
            continue;
        }
        let sounds: Vec<NewSample> = sounds
            .into_iter()
            .map(|(file, s)| {
                NewSample::new(
                    file,
                    Arc::new(move |w: &mut dyn std::io::Write| mpfree_sampler::wav::write(&s, w)),
                )
            })
            .collect();
        lib.save_kit(&folder, &name, &text, &sounds)
            .map_err(|x| format!("{name}: {x}"))?;
        made += 1;
    }
    Ok(made)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_sampler::model::Source;

    #[test]
    fn only_mpfrees_own_kits_are_its_own() {
        let first = mpfree_synth::Drums::presets()[0].name;
        assert!(own_kit(&format!("MPFree {first}")));
        assert!(!own_kit(first));
        assert!(!own_kit("ClassicHouse-Kit-Classy"));
        assert!(!own_kit("Grains: MPFree Init Kit"));
        assert_eq!(folder_name(&format!("MPFree {first}")), first);
    }

    #[test]
    fn kits_go_in_their_folder_once() {
        let dir = std::env::temp_dir().join(format!("mpfree-bundle-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let lib = Library::at(&dir).unwrap();
        let s = Source::new("Kick", 44_100, 1, vec![0.25; 441]).unwrap();
        let kit = |n: &str| -> KitFile {
            (
                n.to_string(),
                "mpfree-sampler 1\nkits = 1\n".into(),
                vec![("Kick.wav".to_string(), s.clone())],
            )
        };
        assert_eq!(
            write(&lib, vec![kit("MPFree Boom Kit"), kit("MPFree Tight Kit")]),
            Ok(2)
        );
        let folder = Loc::new(Place::Kits, &[FOLDER]).unwrap();
        let names: Vec<String> = lib
            .list(&folder)
            .unwrap()
            .into_iter()
            .map(|e| e.name)
            .collect();
        assert_eq!(names, ["Boom Kit", "Tight Kit"]);
        let boom = folder.join("Boom Kit").unwrap();
        assert!(lib.is_kit(&boom));
        assert!(lib.kit_sound_path(&boom, "Kick.wav").is_ok());
        // Again: what is there is left as it is; only a missing kit is made.
        assert_eq!(
            write(&lib, vec![kit("MPFree Boom Kit"), kit("MPFree Grit Kit")]),
            Ok(1)
        );
        assert_eq!(lib.list(&folder).unwrap().len(), 3);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
