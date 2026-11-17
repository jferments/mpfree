//! The Plugins place on the MPC (16 Nov) said only "Insufficient randomness":
//! libcurl's OpenSSL couldn't make random numbers. One way that happens is a
//! system `openssl.cnf` that turns on some provider but not the default one,
//! which then never loads. This test gives OpenSSL such a config, in a
//! process of its own, and checks that MPFree loads the default provider and
//! says so. It is skipped without libcurl on OpenSSL 3.

#[test]
fn a_config_without_the_default_provider_is_mended_and_said() {
    let dir = std::env::temp_dir().join(format!("mpfree-tls-{}", std::process::id()));
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
    // Before libcurl starts OpenSSL, which reads the config once.
    std::env::set_var("OPENSSL_CONF", &conf);
    let tls = mpfree_mpcstore::fetch::tls();
    let _ = std::fs::remove_dir_all(&dir);
    let Some(tls) = tls else {
        eprintln!("no libcurl on this machine: not exercised");
        return;
    };
    if !tls.starts_with("OpenSSL 3") {
        eprintln!("libcurl isn't on OpenSSL 3 here ({tls}): not exercised");
        return;
    }
    assert!(
        tls.contains(" ready (its random numbers didn't start: "),
        "{tls}"
    );
    assert!(tls.contains("unable to fetch drbg"), "{tls}");
    assert!(
        tls.ends_with("; MPFree loaded its default provider)"),
        "{tls}"
    );
}
