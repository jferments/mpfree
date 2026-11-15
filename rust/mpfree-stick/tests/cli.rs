//! The `mpfree-stick` program, as the creator's window runs it: each
//! command, its progress lines and status file, and its refusals.

mod common;
use common::*;
use std::path::Path;
use std::process::{Command, Output};

fn stick(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_mpfree-stick"))
        .args(args)
        .output()
        .unwrap()
}

fn text(o: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

fn programs_dir(dir: &Path) -> String {
    let d = dir.join("programs");
    std::fs::create_dir_all(&d).unwrap();
    let p = programs();
    std::fs::write(d.join("mpfree"), p.pad_player).unwrap();
    std::fs::write(d.join("mpfree-screen-log"), p.screen_log).unwrap();
    d.display().to_string()
}

/// Puts a log on the stick's log partition, as the MPC's systems would.
fn write_log(disk: &Path) {
    use mpfree_diskimage::ext2w::{self, Body, Data, Meta, Options, Size, Tree};
    use mpfree_diskimage::{gpt, Offset};
    let f = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(disk)
        .unwrap();
    let table = gpt::read(&f).unwrap();
    let (_, p) = table.by_name(mpfree_stick::stick::LOG_NAME).unwrap();
    let mut tree = Tree::new(Meta::new(0o755, 0, 0, 0));
    tree.mkdir_all("/boot", Meta::new(0o755, 0, 0, 0)).unwrap();
    tree.put(
        "/boot/hook.log",
        Body::File(Data::Bytes(
            b"eth0 address 0a:1b:2c:3d:4e:5f up\nusb 1-1: SerialNumber: AA0011\n".to_vec(),
        )),
        Meta::new(0o644, 0, 0, 0),
    )
    .unwrap();
    let opts = Options {
        label: "MPFREE-LOG".into(),
        uuid: [7; 16],
        time: 0,
        size: Size::Bytes(p.len_bytes()),
    };
    let w = Offset {
        inner: &f,
        start: p.start_bytes(),
        len: p.len_bytes(),
    };
    ext2w::write(&tree, &opts, &w).unwrap();
}

#[test]
fn makes_and_looks_after_a_stick() {
    let t = tmp("cli");
    let upd = update_file(&t.0).display().to_string();
    let work = t.0.display().to_string();
    let progs = programs_dir(&t.0);
    // check: refused without --any-update, since the test update is not 3.10.1.
    let o = stick(&["check", &upd, "--work", &work]);
    assert!(!o.status.success());
    assert!(
        text(&o)
            .contains("error this update (Akai build TEST-BUILD) is not one MPFree supports yet"),
        "{}",
        text(&o)
    );
    let o = stick(&["check", &upd, "--any-update", "--work", &work]);
    assert!(o.status.success(), "{}", text(&o));
    assert!(text(&o).contains(&format!("done the update is good: Akai build TEST-BUILD, kernel {RELEASE}, panel turned 270 degrees")));
    assert!(text(&o).contains("update file SHA-256: "), "{}", text(&o));
    // A folder: the MPC One update in it is found.
    let o = stick(&["check", &work, "--any-update", "--work", &work]);
    assert!(o.status.success(), "{}", text(&o));
    assert!(text(&o).contains("akai-update.img"), "{}", text(&o));
    let o = stick(&["check", &work, "--work", &work]);
    assert!(!o.status.success(), "{}", text(&o));
    // write: the disk must be named twice.
    let disk = t.0.join("stick.img");
    std::fs::File::create(&disk)
        .unwrap()
        .set_len(16_000_000_000)
        .unwrap();
    let d = disk.display().to_string();
    let o = stick(&[
        "write",
        &upd,
        "--device",
        &d,
        "--yes-erase",
        "/dev/sdz",
        "--programs",
        &progs,
        "--any-update",
    ]);
    assert!(!o.status.success());
    assert!(text(&o).contains("--yes-erase must name the same disk as --device"));
    // write never takes "the MPFree stick that is in": the disk must be named.
    let o = stick(&[
        "write",
        &upd,
        "--device",
        "mpfree",
        "--yes-erase",
        "mpfree",
        "--programs",
        &progs,
        "--any-update",
    ]);
    assert!(!o.status.success());
    assert!(text(&o).contains("not mpfree"), "{}", text(&o));
    // A disk that is neither a file nor a USB disk is never opened.
    let o = stick(&[
        "write",
        &upd,
        "--device",
        "/dev/null-disk",
        "--yes-erase",
        "/dev/null-disk",
        "--programs",
        &progs,
        "--any-update",
        "--work",
        &work,
    ]);
    assert!(!o.status.success());
    assert!(
        text(&o).contains("is not a USB disk this computer has now"),
        "{}",
        text(&o)
    );
    let status = t.0.join("status.txt");
    let s = status.display().to_string();
    let o = stick(&[
        "--status",
        &s,
        "write",
        &upd,
        "--device",
        &d,
        "--yes-erase",
        &d,
        "--programs",
        &progs,
        "--any-update",
        "--work",
        &work,
    ]);
    assert!(o.status.success(), "{}", text(&o));
    let lines = std::fs::read_to_string(&status).unwrap();
    assert!(lines.contains("progress 1.0000 Done\n"));
    assert!(lines
        .lines()
        .last()
        .unwrap()
        .starts_with("done the stick is ready"));
    assert!(lines.contains("note partition 4: mpfree-test-data at 1024 MiB"));
    for l in lines.lines() {
        assert!(
            mpfree_stick::Msg::parse(l).is_some(),
            "the window reads every line: {l}"
        );
    }
    // inspect, setup-mode, setup-log, copy-backup.
    let o = stick(&["inspect", "--device", &d]);
    let out = text(&o);
    assert!(
        out.contains("made: MPFree stick, made by MPFree USB creator"),
        "{out}"
    );
    assert!(out.contains("setup mode: install"));
    assert!(
        out.contains("partition 4: new; MPFree makes its filesystem at the first start on the MPC")
    );
    let o = stick(&["setup-mode", "--device", &d, "remove"]);
    assert!(o.status.success(), "{}", text(&o));
    let out = text(&stick(&["inspect", "--device", &d]));
    assert!(out.contains("setup mode: remove"), "{out}");
    assert!(out.contains("partition 1: mpfree-test-root-off"), "{out}");
    assert!(out.contains("made: MPFree stick"), "{out}");
    assert!(text(&stick(&["setup-log", "--device", &d])).contains("no setup log yet"));
    let o = stick(&["setup-mode", "--device", &d, "install"]);
    assert!(o.status.success(), "{}", text(&o));
    let out = text(&stick(&["inspect", "--device", &d]));
    assert!(
        out.contains("setup mode: install") && out.contains("partition 1: mpfree-test-root "),
        "{out}"
    );
    let o = stick(&["copy-backup", "--device", &d, "--to", &work]);
    assert!(!o.status.success());
    // logs: an empty log partition, then one with a log the MPC wrote.
    let o = stick(&["logs", "--device", &d]);
    assert!(o.status.success(), "{}", text(&o));
    assert!(!text(&o).contains("====="), "{}", text(&o));
    write_log(&disk);
    let o = stick(&["logs", "--device", &d]);
    let out = text(&o);
    assert!(o.status.success(), "{out}");
    assert!(out.contains("===== /boot/hook.log ("), "{out}");
    assert!(out.contains("eth0 address <MAC> up"), "{out}");
    assert!(out.contains("SerialNumber: <SERIAL>"), "{out}");
    assert!(
        !out.contains("0a:1b:2c:3d:4e:5f") && !out.contains("AA0011"),
        "{out}"
    );
    // build: an image file.
    let img = t.0.join("small.img").display().to_string();
    let o = stick(&[
        "build",
        &upd,
        "--out",
        &img,
        "--programs",
        &progs,
        "--any-update",
        "--work",
        &work,
    ]);
    assert!(o.status.success(), "{}", text(&o));
    // make-ssh-key, then a stick with developer SSH.
    let keys = t.0.join("keys").display().to_string();
    assert!(stick(&["make-ssh-key", &keys]).status.success());
    let o = stick(&[
        "write",
        &upd,
        "--device",
        &d,
        "--yes-erase",
        &d,
        "--programs",
        &progs,
        "--any-update",
        "--work",
        &work,
        "--ssh-key",
        &format!("{keys}/mpfree_ed25519.pub"),
    ]);
    assert!(o.status.success(), "{}", text(&o));
    assert!(text(&o)
        .contains("developer SSH on: root, key only, port 2222; the stick's host key is SHA256:"));
    assert!(text(&stick(&["inspect", "--device", &d])).contains("developer SSH: on"));
    // A private key given by mistake is refused before anything is written.
    let o = stick(&[
        "write",
        &upd,
        "--device",
        &d,
        "--yes-erase",
        &d,
        "--programs",
        &progs,
        "--any-update",
        "--work",
        &work,
        "--ssh-key",
        &format!("{keys}/mpfree_ed25519"),
    ]);
    assert!(text(&o).contains("that is a private key"), "{}", text(&o));
    // Programs that are not for the MPC are refused.
    std::fs::write(Path::new(&progs).join("mpfree"), b"#!/bin/sh\n").unwrap();
    let o = stick(&[
        "build",
        &upd,
        "--out",
        &format!("{img}2"),
        "--programs",
        &progs,
        "--any-update",
        "--work",
        &work,
    ]);
    assert!(
        text(&o).contains("is not a program for the MPC (32-bit ARM)"),
        "{}",
        text(&o)
    );
    assert!(text(&stick(&["help"])).contains("only been tested on the MPC One (Gen 1)"));
    assert!(!stick(&["frobnicate"]).status.success());
}
