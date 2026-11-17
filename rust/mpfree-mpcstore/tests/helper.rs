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

#[test]
fn the_helper_says_what_its_openssl_said_as_this_program_would() {
    let o = Command::new(HELPER).arg("--tls").output().unwrap();
    let said = String::from_utf8(o.stdout).unwrap();
    // This test program is dynamic, so it starts libcurl itself: the helper
    // must say the same.
    match fetch::tls() {
        Some(tls) => {
            assert_eq!((o.status.code(), said), (Some(0), format!("tls {tls}\n")));
            assert_eq!(
                fetch::tls_through_helper(Path::new(HELPER), fetch::TLS_WAIT),
                tls
            );
        }
        None => {
            assert_eq!(o.status.code(), Some(1));
            assert!(said.starts_with("failed "), "{said}");
        }
    }
}

#[test]
fn the_helper_mends_a_config_without_the_default_provider() {
    // As tests/tls.rs, but in the helper, where the MPC's downloads run.
    let dir = std::env::temp_dir().join(format!("mpfree-helper-tls-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let conf = dir.join("openssl.cnf");
    std::fs::write(
        &conf,
        "openssl_conf = openssl_init\n\
         [openssl_init]\nproviders = provider_sect\n\
         [provider_sect]\nnull = null_sect\n\
         [null_sect]\nactivate = 1\n",
    )
    .unwrap();
    let o = Command::new(HELPER)
        .arg("--tls")
        .env("OPENSSL_CONF", &conf)
        .output()
        .unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    let said = String::from_utf8(o.stdout).unwrap();
    if !said.starts_with("tls OpenSSL 3") {
        eprintln!("no libcurl on OpenSSL 3 here ({said:?}): not exercised");
        return;
    }
    assert!(
        said.contains(" ready (its random numbers didn't start: ")
            && said
                .trim_end()
                .ends_with("; MPFree loaded its default provider)"),
        "{said}"
    );
}
