//! The download helper, `mpfree-fetch`, as the pad player runs it.

use mpfree_mpcstore::fetch;
use std::path::Path;
use std::process::Command;

const HELPER: &str = env!("CARGO_BIN_EXE_mpfree-fetch");

#[test]
fn the_helper_says_why_and_fetches_only_the_catalogs_sites() {
    let out = std::env::temp_dir().join(format!("mpfree-fetch-never-{}", std::process::id()));
    let run = |args: &[&str]| {
        let o = Command::new(HELPER).args(args).output().unwrap();
        (o.status.code(), String::from_utf8(o.stdout).unwrap())
    };
    let o = out.to_str().unwrap();
    assert_eq!(
        run(&["https://example.com/x", o, "10"]),
        (
            Some(1),
            "failed https://example.com/x is not on the catalog's sites\n".into()
        )
    );
    assert_eq!(run(&["https://github.com/x", o]).0, Some(2));
    assert_eq!(run(&["https://github.com/x", o, "ten"]).0, Some(2));
    assert!(!out.exists());
    // The same reason reaches the pad player.
    let e = fetch::through_helper(
        Path::new(HELPER),
        "http://github.com/x.zip",
        &out,
        10,
        &mut |_, _| true,
    )
    .unwrap_err();
    assert_eq!(e, "http://github.com/x.zip is not on the catalog's sites");
    assert!(!out.exists());
}
