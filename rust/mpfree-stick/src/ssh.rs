//! Developer SSH (off unless the owner turns it on in Preferences >
//! Developer options): the stick's own host key, the owner's public key(s),
//! and, for owners without one, a new key pair of their own.

use crate::bad;
use crate::stick::Ssh;
use ssh_key::rand_core::OsRng;
use ssh_key::{Algorithm, LineEnding, PrivateKey, PublicKey};
use std::io;
use std::path::{Path, PathBuf};

/// A new ed25519 host key for one stick, its public line and fingerprint.
pub fn host_key() -> io::Result<(String, String, String)> {
    let key = PrivateKey::random(&mut OsRng, Algorithm::Ed25519)
        .map_err(|e| bad(format!("making the host key: {e}")))?;
    let private = key
        .to_openssh(LineEnding::LF)
        .map_err(|e| bad(format!("the host key: {e}")))?
        .to_string();
    let public = key
        .public_key()
        .to_openssh()
        .map_err(|e| bad(format!("the host key: {e}")))?;
    let fingerprint = key
        .public_key()
        .fingerprint(ssh_key::HashAlg::Sha256)
        .to_string();
    Ok((private, public + " mpfree-stick\n", fingerprint))
}

/// The owner's public key(s), one per line, checked: each must be an
/// OpenSSH public key line (`ssh-ed25519 AAAA... comment`).
pub fn check_public_keys(text: &str) -> io::Result<String> {
    let mut out = String::new();
    for line in text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
    {
        if line.contains("PRIVATE KEY") {
            return Err(bad(
                "that is a private key: give the public one (the file ending in .pub)",
            ));
        }
        PublicKey::from_openssh(line)
            .map_err(|e| bad(format!("not an SSH public key line ({e}): {line:.40}")))?;
        out.push_str(line);
        out.push('\n');
    }
    if out.is_empty() {
        return Err(bad("no public key given"));
    }
    Ok(out)
}

/// Developer SSH for a new stick: the owner's keys and a new host key.
pub fn for_stick(public_keys: &str) -> io::Result<(Ssh, String)> {
    let authorized_keys = check_public_keys(public_keys)?;
    let (host_private, host_public, fingerprint) = host_key()?;
    Ok((
        Ssh {
            authorized_keys,
            host_private,
            host_public,
        },
        fingerprint,
    ))
}

/// A key pair for an owner who has none: `name` and `name.pub` in `dir`,
/// never over an existing file. The private key is readable by its owner
/// only.
pub fn make_user_key(dir: &Path, name: &str) -> io::Result<(PathBuf, PathBuf)> {
    let private_path = dir.join(name);
    let public_path = dir.join(format!("{name}.pub"));
    for p in [&private_path, &public_path] {
        if p.exists() {
            return Err(bad(format!(
                "{} exists already; use it, or pick another name",
                p.display()
            )));
        }
    }
    std::fs::create_dir_all(dir)?;
    let key = PrivateKey::random(&mut OsRng, Algorithm::Ed25519)
        .map_err(|e| bad(format!("making the key: {e}")))?;
    let private = key
        .to_openssh(LineEnding::LF)
        .map_err(|e| bad(format!("the key: {e}")))?;
    let public = key
        .public_key()
        .to_openssh()
        .map_err(|e| bad(format!("the key: {e}")))?
        + " mpfree\n";
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
    io::Write::write_all(&mut opts.open(&private_path)?, private.as_bytes())?;
    std::fs::write(&public_path, public)?;
    Ok((private_path, public_path))
}

/// The public key the owner most likely means: the first of the usual
/// files in ~/.ssh.
pub fn default_public_key(home: &Path) -> Option<(PathBuf, String)> {
    [
        "mpfree_ed25519.pub",
        "id_ed25519.pub",
        "id_ecdsa.pub",
        "id_rsa.pub",
    ]
    .iter()
    .find_map(|n| {
        let p = home.join(".ssh").join(n);
        let text = std::fs::read_to_string(&p).ok()?;
        check_public_keys(&text).ok().map(|t| (p, t))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_made_and_checked() {
        let (private, public, fp) = host_key().unwrap();
        assert!(private.starts_with("-----BEGIN OPENSSH PRIVATE KEY-----\n"));
        assert!(public.starts_with("ssh-ed25519 AAAA"));
        assert!(fp.starts_with("SHA256:"));
        assert_eq!(
            check_public_keys(&format!("# mine\n\n{public}")).unwrap(),
            public
        );
        assert!(
            check_public_keys(&private).is_err(),
            "a private key is refused"
        );
        assert!(check_public_keys("ssh-ed25519 notbase64").is_err());
        assert!(check_public_keys("\n# nothing\n").is_err());
        let dir = std::env::temp_dir().join(format!("mpfree-ssh-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (k, p) = make_user_key(&dir, "mpfree_ed25519").unwrap();
        assert!(check_public_keys(&std::fs::read_to_string(&p).unwrap()).is_ok());
        assert!(std::fs::read_to_string(&k)
            .unwrap()
            .contains("OPENSSH PRIVATE KEY"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&k).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        assert!(
            make_user_key(&dir, "mpfree_ed25519").is_err(),
            "never over a key"
        );
        std::fs::create_dir_all(dir.join("home/.ssh")).unwrap();
        std::fs::copy(&p, dir.join("home/.ssh/id_ed25519.pub")).unwrap();
        assert!(default_public_key(&dir.join("home")).is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
