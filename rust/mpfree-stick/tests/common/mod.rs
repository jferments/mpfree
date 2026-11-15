//! A made-up Akai update for the creator's tests: a small root filesystem
//! with what the recipe needs, in an update file like Akai's.
#![allow(dead_code)]

use mpfree_diskimage::ext2w::{self, Body, Data, Meta, Options, Size, Tree};
use mpfree_diskimage::Mem;
use mpfree_stick::recipe::Recipe;
use mpfree_stick::stick::{self, Programs};
use mpfree_stick::{dtb, update};
use std::path::{Path, PathBuf};

pub const RELEASE: &str = "6.18.44-test";

pub struct Tmp(pub PathBuf);
impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub fn tmp(tag: &str) -> Tmp {
    let p = std::env::temp_dir().join(format!("mpfree-stick-test-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    Tmp(p)
}

fn file(t: &mut Tree, path: &str, text: &[u8], perm: u16) {
    t.put(
        path,
        Body::File(Data::Bytes(text.to_vec())),
        Meta::new(perm, 0, 0, 1_700_000_000),
    )
    .unwrap();
}

/// Akai's root filesystem, as small as the recipe allows.
pub fn fake_rootfs() -> Vec<u8> {
    fake_rootfs_without(&[])
}

/// The fake root filesystem without some of the programs the stick's setup
/// scripts need.
pub fn fake_rootfs_without(left_out: &[&str]) -> Vec<u8> {
    let r = Recipe::load();
    let m = |perm| Meta::new(perm, 0, 0, 1_700_000_000);
    let mut t = Tree::new(m(0o755));
    for d in [
        "/usr/bin",
        "/usr/sbin",
        "/usr/lib/systemd/system",
        "/etc/systemd/system/multi-user.target.wants",
        "/boot",
        "/dev",
        "/home/root",
    ] {
        t.mkdir_all(d, m(0o755)).unwrap();
    }
    for top in ["bin", "sbin", "lib"] {
        file_link(&mut t, &format!("/{top}"), &format!("usr/{top}"));
    }
    file(&mut t, "/usr/bin/sh", b"#!/fake/shell\n", 0o755);
    for (path, kind) in &r.required {
        if kind == "regular" && !path.starts_with("/bin/") {
            file(&mut t, path, b"# Akai's\n", 0o644);
        }
    }
    for p in &r.app_user.programs {
        file(&mut t, &format!("/usr/bin/{p}"), b"#!/bin/sh\n", 0o755);
    }
    for p in [
        "/usr/sbin/az01-update",
        "/usr/bin/kexec",
        "/usr/sbin/mke2fs",
    ] {
        file(&mut t, p, b"#!/bin/sh\n# Akai's program\n", 0o755);
    }
    // BusyBox's applets are links to it.
    file(&mut t, "/usr/bin/busybox", b"#!/bin/sh\n# BusyBox\n", 0o755);
    for p in mpfree_stick::recipe::setup_programs() {
        if !left_out.contains(&p)
            && t.lookup(&format!("/usr/bin/{p}")).is_none()
            && t.lookup(&format!("/usr/sbin/{p}")).is_none()
        {
            file_link(&mut t, &format!("/usr/bin/{p}"), "busybox");
        }
    }
    file(
        &mut t,
        "/etc/passwd",
        b"root:x:0:0:root:/home/root:/bin/sh\n",
        0o644,
    );
    file(
        &mut t,
        "/etc/group",
        b"root:x:0:\naudio:x:29:\nvideo:x:44:\ninput:x:101:\nsystemd-journal:x:190:\n",
        0o644,
    );
    file(&mut t, "/etc/shadow", b"root:!:19000:0:99999:7:::\n", 0o600);
    file(&mut t, &format!("/boot/zImage-{RELEASE}"), b"kernel", 0o644);
    file(
        &mut t,
        stick::BOARD_DTB,
        &dtb::testtree::make(&["inmusic,acva", "rockchip,rk3288"], Some(270)),
        0o644,
    );
    // A hard link, a device, a fifo and an extended attribute, to see they come across.
    let a = t
        .put(
            "/usr/bin/tool",
            Body::File(Data::Bytes(b"tool".to_vec())),
            m(0o755),
        )
        .unwrap();
    let bin = t.lookup("/usr/bin").unwrap();
    t.link(bin, b"tool-too", a).unwrap();
    t.put("/dev/console", Body::CharDev(5, 1), m(0o600))
        .unwrap();
    t.put("/dev/initctl", Body::Fifo, m(0o600)).unwrap();
    let mut meta = m(0o755);
    meta.xattrs = vec![(
        "security.capability".into(),
        vec![1, 0, 0, 2, 0, 32, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    )];
    t.put(
        "/usr/bin/ping",
        Body::File(Data::Bytes(b"ping".to_vec())),
        meta,
    )
    .unwrap();
    let out = Mem::new();
    ext2w::write(
        &t,
        &Options {
            label: "rootfs".into(),
            uuid: [7; 16],
            time: 1_700_000_000,
            size: Size::Fit {
                spare_bytes: 4 << 20,
                spare_inodes: 64,
            },
        },
        &out,
    )
    .unwrap();
    out.into_inner()
}

fn file_link(t: &mut Tree, path: &str, target: &str) {
    t.put(
        path,
        Body::Symlink(target.as_bytes().to_vec()),
        Meta::new(0o777, 0, 0, 1_700_000_000),
    )
    .unwrap();
}

pub fn arm_program(tag: &[u8]) -> Vec<u8> {
    let mut b = vec![0u8; 64];
    b[..7].copy_from_slice(b"\x7fELF\x01\x01\x01");
    b[18] = 40;
    b.extend_from_slice(tag);
    b
}

pub fn programs() -> Programs {
    Programs {
        pad_player: arm_program(b"pad player"),
        screen_log: arm_program(b"screen log"),
        startup: arm_program(b"startup screen"),
    }
}

pub fn update_file(dir: &Path) -> PathBuf {
    update_file_with(dir, &fake_rootfs())
}

pub fn update_file_with(dir: &Path, rootfs: &[u8]) -> PathBuf {
    let p = dir.join("akai-update.img");
    std::fs::write(
        &p,
        update::testfile::make("TEST-BUILD", &["ACVA"], rootfs, false),
    )
    .unwrap();
    p
}
