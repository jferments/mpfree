//! Records the commit the pad player is built from, for Prefs > System info.
//! It reads the clone's .git folder directly (no git program) and gives
//! "unknown" when there is none, such as in a source copy.
//!
//! For a static ARM build (the MPC's), it also asks the linker to put the
//! sections in name order, so glibc's code sits beside the `.iplt` (see
//! "Code over 16 MiB" in docs/dev/guides/mpfree-app.md).

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    let manifest =
        PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR"));
    let git = manifest.join("..").join("..").join(".git");
    let (commit, watched) = head_commit(&git);
    println!(
        "cargo:rustc-env=MPFREE_COMMIT={}",
        commit.as_deref().unwrap_or("unknown")
    );
    // Only files that exist: a missing one would rebuild the program every time.
    for f in watched {
        if f.is_file() {
            println!("cargo:rerun-if-changed={}", f.display());
        }
    }
    println!("cargo:rerun-if-changed=build.rs");
    static_arm_link_order();
}

/// Past 16 MiB of code, GNU ld 2.42 links a static ARM program whose glibc
/// start-up calls memcpy's IFUNC resolver in ARM mode instead of Thumb, and
/// the program dies before `main` (SIGSEGV or SIGILL; train 155 crossed the
/// line with mpfree's .text at 0x10232b8). `--sort-section=name` keeps
/// glibc's Thumb code beside the `.iplt` at the start of `.text`, so no
/// long-branch stub is needed and the IRELATIVE entries keep their Thumb bit.
/// It only orders sections: the code itself is the same.
fn static_arm_link_order() {
    let arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let features = env::var("CARGO_CFG_TARGET_FEATURE").unwrap_or_default();
    if arch == "arm" && os == "linux" && features.split(',').any(|f| f == "crt-static") {
        println!("cargo:rustc-link-arg-bins=-Wl,--sort-section=name");
    }
}

/// The short commit HEAD names, and the files it was read from.
fn head_commit(git: &Path) -> (Option<String>, Vec<PathBuf>) {
    let head_file = git.join("HEAD");
    let mut watched = vec![head_file.clone()];
    let Ok(head) = fs::read_to_string(&head_file) else {
        return (None, watched);
    };
    let head = head.trim();
    let full = match head.strip_prefix("ref: ") {
        // A branch: its loose ref file, or its line in packed-refs.
        Some(name) => {
            let loose = git.join(name);
            let packed = git.join("packed-refs");
            watched.push(loose.clone());
            watched.push(packed.clone());
            fs::read_to_string(&loose)
                .ok()
                .map(|s| s.trim().to_string())
                .or_else(|| {
                    fs::read_to_string(&packed).ok()?.lines().find_map(|l| {
                        let (sha, r) = l.split_once(' ')?;
                        (r == name).then(|| sha.to_string())
                    })
                })
        }
        // A detached HEAD holds the commit itself.
        None => Some(head.to_string()),
    };
    let short = full
        .filter(|s| s.len() >= 7 && s.chars().all(|c| c.is_ascii_hexdigit()))
        .map(|s| s[..7].to_string());
    (short, watched)
}
