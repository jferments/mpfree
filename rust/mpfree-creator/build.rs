//! Release builds carry MPFree's programs for the MPC inside the creator, so
//! the creator is one file that works wherever it is put (on Windows, even
//! opened straight from its zip). Set `MPFREE_CREATOR_PROGRAMS` to the folder
//! of programs when building, and every file in it whose name is made of
//! `a-z`, `0-9` and `-` is embedded (`mpfree-stick`'s `PROGRAM_FILES`; the
//! creator checks at run time that they are all there and are ARM programs).
//! Without it, nothing is embedded, and the creator looks for a
//! `stick-programs` folder next to itself, as a developer's build always has.

use std::path::Path;

fn main() {
    println!("cargo:rerun-if-env-changed=MPFREE_CREATOR_PROGRAMS");
    let out = Path::new(&std::env::var_os("OUT_DIR").expect("OUT_DIR")).join("embedded.rs");
    let mut code = String::from(
        "/// MPFree's programs for the MPC carried by this build (name, bytes).\n\
         pub const EMBEDDED: &[(&str, &[u8])] = &[\n",
    );
    if let Some(dir) = std::env::var_os("MPFREE_CREATOR_PROGRAMS") {
        let dir = std::fs::canonicalize(&dir).unwrap_or_else(|e| {
            panic!("MPFREE_CREATOR_PROGRAMS={}: {e}", Path::new(&dir).display())
        });
        println!("cargo:rerun-if-changed={}", dir.display());
        let mut files: Vec<(String, String)> = std::fs::read_dir(&dir)
            .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
            .map(|e| e.expect("a folder entry"))
            .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
            .filter_map(|e| {
                let name = e.file_name().into_string().ok()?;
                let plain = !name.is_empty()
                    && name
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
                plain.then(|| (name, e.path().to_str().expect("a UTF-8 path").to_string()))
            })
            .collect();
        files.sort();
        assert!(
            !files.is_empty(),
            "MPFREE_CREATOR_PROGRAMS={} holds no programs",
            dir.display()
        );
        for (name, path) in files {
            println!("cargo:rerun-if-changed={path}");
            code.push_str(&format!("    ({name:?}, include_bytes!({path:?})),\n"));
        }
    }
    code.push_str("];\n");
    std::fs::write(&out, code).unwrap_or_else(|e| panic!("{}: {e}", out.display()));
}
