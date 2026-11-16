//! The whole creator, end to end, on a made-up Akai update (common/).
//! The stick is made, then read back with the ext4 reader (and checked
//! with e2fsck where it is installed).

use mpfree_diskimage::ext2w::{self, Body, Data, Meta, Options, Size, Tree};
use mpfree_diskimage::ext4::{Ext4, Kind};
use mpfree_diskimage::{gpt, Offset, WriteAt};
use mpfree_stick::recipe::{self, Recipe};
use mpfree_stick::stick::{self, Build};
use mpfree_stick::{ssh, update, Msg};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::path::Path;

mod common;
use common::*;

fn part_fs<'a>(img: &'a File, p: &gpt::Part) -> Ext4<Offset<&'a File>> {
    Ext4::open(Offset {
        inner: img,
        start: p.start_bytes(),
        len: p.len_bytes(),
    })
    .unwrap()
}

fn read(fs: &Ext4<Offset<&File>>, path: &str) -> Vec<u8> {
    let i = fs
        .lookup(path)
        .unwrap()
        .unwrap_or_else(|| panic!("{path} is missing"));
    fs.read_file(&i).unwrap()
}

/// e2fsck, where it is installed, on one partition copied out.
fn fsck(img: &File, p: &gpt::Part, dir: &Path) {
    let Some(e2fsck) = ["/usr/sbin/e2fsck", "/sbin/e2fsck"]
        .into_iter()
        .find(|p| Path::new(p).exists())
    else {
        return;
    };
    let out = dir.join(format!("{}.img", p.name));
    let mut b = vec![0u8; p.len_bytes() as usize];
    mpfree_diskimage::ReadAt::read_at(img, &mut b, p.start_bytes()).unwrap();
    std::fs::write(&out, b).unwrap();
    let res = std::process::Command::new(e2fsck)
        .args(["-fn"])
        .arg(&out)
        .output()
        .unwrap();
    assert!(
        res.status.success(),
        "{}: {}{}",
        p.name,
        String::from_utf8_lossy(&res.stdout),
        String::from_utf8_lossy(&res.stderr)
    );
    std::fs::remove_file(&out).unwrap();
}

#[test]
fn makes_a_stick_the_hook_takes() {
    let t = tmp("image");
    let upd = update_file(&t.0);
    let mut notes = Vec::new();
    // Not an update MPFree knows: refused unless asked.
    assert!(stick::open_update(&upd, &t.0, false, &mut |_| {}).is_err());
    let src = stick::open_update(&upd, &t.0, true, &mut |m| notes.push(m)).unwrap();
    assert_eq!(src.release, RELEASE);
    assert_eq!(src.rotation, 270);
    let progs = programs();
    let kits = b"MPFK test kits".to_vec();
    let b = Build {
        source: &src,
        programs: &progs,
        kits: &kits,
        ssh: None,
        dev: None,
        creator: "test creator",
    };
    let out = t.0.join("stick.img");
    let made = stick::build_image(&b, &out, &mut |m| notes.push(m)).unwrap();
    assert!(matches!(notes.last(), Some(Msg::Progress(f, _)) if *f == 1.0));
    let img = File::open(&out).unwrap();
    let table = gpt::read(&img).unwrap();
    let names: Vec<&str> = table
        .parts
        .iter()
        .flatten()
        .map(|p| p.name.as_str())
        .collect();
    assert_eq!(
        names,
        [
            stick::ROOT_NAME,
            stick::LOG_NAME,
            stick::KITS_NAME,
            "mpfree-test-data",
            stick::SETUP_NAME
        ]
    );
    let p = |n| table.part(n).unwrap();
    assert_eq!(stick::uuid_text(&p(2).guid), made.values.log_uuid);
    assert_eq!(stick::uuid_text(&p(3).guid), made.values.kits_uuid);
    assert_eq!(stick::uuid_text(&p(4).guid), made.values.data_uuid);
    assert_eq!(p(4).first, 2 * 1024 * 1024);

    // Partition 1: Akai's files, MPFree's changes.
    let root = part_fs(&img, p(1));
    assert_eq!(root.label, "rootfs");
    assert_eq!(read(&root, "/etc/mpfree-test/mpfree"), progs.pad_player);
    assert_eq!(
        read(&root, "/etc/mpfree-test/mpfree-screen-log"),
        progs.screen_log
    );
    assert_eq!(read(&root, "/etc/mpfree-test/mpfree-splash"), progs.startup);
    let splash = String::from_utf8(read(
        &root,
        "/etc/systemd/system/mpfree-test-splash.service",
    ))
    .unwrap();
    assert!(splash.contains("--framebuffer --rotation 270 --seconds 6\n"));
    assert!(
        splash.contains("RemainAfterExit=yes\n"),
        "it plays once per power-on"
    );
    assert!(root
        .lookup("/etc/systemd/system/sysinit.target.wants/mpfree-test-splash.service")
        .unwrap()
        .is_some());
    let start = String::from_utf8(read(&root, "/etc/mpfree-test/start-app.sh")).unwrap();
    assert!(start.contains(
        " --keep-screen --data /run/mpfree-data --power-request /run/mpfree-app/power-request\n"
    ));
    assert!(
        root.lookup("/etc/systemd/system/sysinit.target.wants/mpfree-test-power.path")
            .unwrap()
            .is_some(),
        "the power card's shut down and restart work on creator sticks too"
    );
    assert_eq!(read(&root, "/usr/bin/tool-too"), b"tool");
    let tool = root.lookup("/usr/bin/tool").unwrap().unwrap();
    assert_eq!(tool.links, 2, "the hard link stays one file");
    let console = root.lookup("/dev/console").unwrap().unwrap();
    assert_eq!(
        (console.kind(), console.rdev()),
        (Some(Kind::CharDev), (5, 1))
    );
    let ping = root.lookup("/usr/bin/ping").unwrap().unwrap();
    assert_eq!(root.xattrs(&ping).unwrap()[0].0, "security.capability");
    assert_eq!(
        read(&root, "/usr/sbin/az01-update"),
        Recipe::load().stub.as_bytes(),
        "Akai's updater is stubbed"
    );
    assert_eq!(
        read(&root, "/usr/bin/kexec"),
        Recipe::load().stub.as_bytes()
    );
    let passwd = String::from_utf8(read(&root, "/etc/passwd")).unwrap();
    assert!(passwd.ends_with("mpfree:x:960:960:MPFree pad player:/run/mpfree-app:/sbin/nologin\n"));
    assert_eq!(
        read(&root, "/etc/shadow"),
        b"root:!:19000:0:99999:7:::\n",
        "root stays locked without SSH"
    );
    assert!(
        root.lookup("/etc/mpfree-test/sshd_config")
            .unwrap()
            .is_none(),
        "no SSH unless asked"
    );
    assert!(root
        .lookup("/etc/mpfree-test/authorized_keys")
        .unwrap()
        .is_none());
    assert!(root
        .lookup("/etc/systemd/system/multi-user.target.wants/mpfree-test-sshd.service")
        .unwrap()
        .is_none());
    let acvs = root
        .lookup("/etc/systemd/system/acvs.service")
        .unwrap()
        .unwrap();
    assert_eq!(
        root.read_link(&acvs).unwrap(),
        b"/dev/null",
        "Akai's app is masked"
    );
    // The soft-reboot hook's pieces, with sums that match.
    assert_eq!(
        read(&root, "/boot/mpfree-softreboot/release"),
        format!("{RELEASE}\n").as_bytes()
    );
    let sums = String::from_utf8(read(&root, "/boot/mpfree-softreboot/SHA256SUMS")).unwrap();
    let r = Recipe::load();
    assert_eq!(sums.lines().count(), r.softreboot_listed.len());
    for line in sums.lines() {
        let (sum, path) = line.split_once("  ").unwrap();
        assert_eq!(
            update::hex(&Sha256::digest(read(&root, path))),
            sum,
            "{path}"
        );
    }
    // No placeholder is left anywhere.
    for path in r.files.keys().filter(|p| !r.only_with_ssh.contains(p)) {
        let text = read(&root, path);
        for v in r.placeholders.values() {
            assert!(
                !text.windows(v.len()).any(|w| w == v.as_bytes()),
                "{v} left in {path}"
            );
        }
    }
    let screen = String::from_utf8(read(
        &root,
        "/etc/systemd/system/mpfree-test-screen.service",
    ))
    .unwrap();
    assert!(
        screen.contains("--rotation 270"),
        "the screen log turns with the panel"
    );
    // The first-start setup and the formatter, with this stick's values.
    let fmt = String::from_utf8(read(&root, "/etc/mpfree-test/setup-format.sh")).unwrap();
    assert!(fmt.contains(&made.values.data_uuid) && !fmt.contains("@DATA_UUID@"));
    // The startup screen waits for the format step, in a drop-in folder of its own.
    let drop_in = String::from_utf8(read(
        &root,
        "/etc/systemd/system/mpfree-test-splash.service.d/mpfree-setup.conf",
    ))
    .unwrap();
    assert!(drop_in.contains("Environment=MPFREE_SETUP_STATUS=/run/mpfree-setup/format-status\n"));
    assert_eq!(
        read(&root, stick::TOKEN_FILE),
        format!("{}\n", made.values.token).as_bytes()
    );
    assert_eq!(
        root.lookup(stick::TOKEN_FILE).unwrap().unwrap().perm(),
        0o600
    );
    for (path, target) in recipe::SETUP_LINKS {
        let l = root.lookup(path).unwrap().unwrap();
        assert_eq!(root.read_link(&l).unwrap(), target.as_bytes());
    }
    assert!(String::from_utf8(read(&root, stick::ABOUT_FILE))
        .unwrap()
        .contains("developer SSH: off"));

    // Partitions 2 and 3.
    assert_eq!(part_fs(&img, p(2)).label, "MPFREE-LOG");
    let kp = part_fs(&img, p(3));
    assert_eq!(kp.label, "MPFREE-KITS");
    assert_eq!(read(&kp, "/kits.bin"), kits);
    // Partition 4: marked for the first start.
    let mut head = vec![0u8; 64];
    mpfree_diskimage::ReadAt::read_at(&img, &mut head, p(4).start_bytes()).unwrap();
    assert!(head.starts_with(format!("{}{}", stick::MARK, made.values.token).as_bytes()));
    // Partition 5: the setup drive.
    let (_, mut setup) = stick::read_setup(&img).unwrap();
    let autorun = String::from_utf8(
        stick::setup_read(&mut setup, "autorun.sh")
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert!(autorun.contains(&format!("RELEASE='{RELEASE}'")));
    assert_eq!(
        stick::setup_read(&mut setup, "mode.txt").unwrap().unwrap(),
        b"install\n"
    );
    for n in [1, 2, 3] {
        fsck(&img, p(n), &t.0);
    }
}

#[test]
fn refuses_a_system_without_a_program_the_setup_needs() {
    let t = tmp("no-programs");
    let upd = update_file_with(&t.0, &fake_rootfs_without(&["stat", "udevadm"]));
    let src = stick::open_update(&upd, &t.0, true, &mut |_| {}).unwrap();
    let progs = programs();
    let b = Build {
        source: &src,
        programs: &progs,
        kits: b"MPFK test kits",
        ssh: None,
        dev: None,
        creator: "test creator",
    };
    let out = t.0.join("stick.img");
    let Err(e) = stick::build_image(&b, &out, &mut |_| {}) else {
        panic!("made a stick from a system without stat and udevadm");
    };
    assert_eq!(
        e.to_string(),
        "Akai's root filesystem has no stat, udevadm in /usr/bin or /usr/sbin, which the stick's first-start setup needs"
    );
}

#[test]
fn developer_ssh_when_asked() {
    let t = tmp("ssh");
    let upd = update_file(&t.0);
    let src = stick::open_update(&upd, &t.0, true, &mut |_| {}).unwrap();
    let (_, user_pub, _) = ssh::host_key().unwrap();
    let (s, _) = ssh::for_stick(&user_pub).unwrap();
    let progs = programs();
    let b = Build {
        source: &src,
        programs: &progs,
        kits: b"k",
        ssh: Some(&s),
        dev: None,
        creator: "test",
    };
    let out = t.0.join("stick.img");
    stick::build_image(&b, &out, &mut |_| {}).unwrap();
    let img = File::open(&out).unwrap();
    let table = gpt::read(&img).unwrap();
    let root = part_fs(&img, table.part(1).unwrap());
    assert_eq!(
        read(&root, "/etc/mpfree-test/authorized_keys"),
        user_pub.as_bytes()
    );
    assert_eq!(
        read(&root, "/etc/mpfree-test/host_ed25519"),
        s.host_private.as_bytes()
    );
    assert_eq!(
        read(&root, "/etc/shadow"),
        b"root:*:19000:0:99999:7:::\n",
        "key login only, no password"
    );
    assert!(root
        .lookup("/etc/systemd/system/multi-user.target.wants/mpfree-test-sshd.service")
        .unwrap()
        .is_some());
    assert!(String::from_utf8(read(&root, stick::ABOUT_FILE))
        .unwrap()
        .contains("developer SSH: on"));
}

/// `--dev-tools`: the folder under `mpfree-dev/` on partition 5, with its
/// SHA256SUMS and MPFREE-DEV.txt, and the report naming each file.
#[test]
fn developer_tools_go_on_the_setup_partition() {
    let t = tmp("dev");
    let upd = update_file(&t.0);
    let src = stick::open_update(&upd, &t.0, true, &mut |_| {}).unwrap();
    let kit = t.0.join("dev-tools");
    std::fs::create_dir_all(kit.join("fxprobe/MPFree - VST - FXProbe Tap")).unwrap();
    std::fs::create_dir_all(kit.join("empty")).unwrap();
    std::fs::write(kit.join("fxprobe/fxprobe.sh"), b"#!/bin/sh\necho hi\n").unwrap();
    std::fs::write(
        kit.join("fxprobe/MPFree - VST - FXProbe Tap/plugin.json"),
        b"{}\n",
    )
    .unwrap();
    let dev = mpfree_stick::dev::DevTools::from_dir(&kit).unwrap();
    let progs = programs();
    let b = Build {
        source: &src,
        programs: &progs,
        kits: b"k",
        ssh: None,
        dev: Some(&dev),
        creator: "test",
    };
    let out = t.0.join("stick.img");
    let made = stick::build_image(&b, &out, &mut |_| {}).unwrap();
    let img = File::open(&out).unwrap();
    let (_, mut setup) = stick::read_setup(&img).unwrap();
    let get = |img: &mut Vec<u8>, p: &str| stick::setup_read(img, p).unwrap();
    assert_eq!(
        get(&mut setup, "mpfree-dev/fxprobe/fxprobe.sh").unwrap(),
        b"#!/bin/sh\necho hi\n"
    );
    assert_eq!(
        get(
            &mut setup,
            "mpfree-dev/fxprobe/MPFree - VST - FXProbe Tap/plugin.json"
        )
        .unwrap(),
        b"{}\n"
    );
    let sums = String::from_utf8(get(&mut setup, "mpfree-dev/SHA256SUMS").unwrap()).unwrap();
    assert_eq!(sums, dev.sums());
    assert_eq!(sums.lines().count(), 2);
    assert!(sums.contains(&format!(
        "{}  fxprobe/fxprobe.sh\n",
        update::hex(&Sha256::digest(b"#!/bin/sh\necho hi\n"))
    )));
    assert!(get(&mut setup, "mpfree-dev/MPFREE-DEV.txt").is_some());
    // The setup files are still there, and setting the mode keeps the tools.
    assert!(get(&mut setup, "autorun.sh").is_some());
    assert!(made
        .notes
        .iter()
        .any(|n| n.contains("2 files, 21 bytes, on partition 5 under /mpfree-dev")));
    assert!(made
        .notes
        .iter()
        .any(|n| n.starts_with("  dev tool /mpfree-dev/fxprobe/fxprobe.sh  sha256 ")));
    drop(img);
    let disk = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&out)
        .unwrap();
    let len = disk.metadata().unwrap().len();
    stick::set_mode(&disk, len, "remove").unwrap();
    let (_, mut after) = stick::read_setup(&disk).unwrap();
    assert_eq!(get(&mut after, "mode.txt").unwrap(), b"remove\n");
    assert_eq!(
        get(&mut after, "mpfree-dev/fxprobe/fxprobe.sh").unwrap(),
        b"#!/bin/sh\necho hi\n"
    );
}

#[test]
fn writes_a_stick_keeps_its_data_and_copies_the_backup_off() {
    let t = tmp("disk");
    let upd = update_file(&t.0);
    let src = stick::open_update(&upd, &t.0, true, &mut |_| {}).unwrap();
    let progs = programs();
    let b = Build {
        source: &src,
        programs: &progs,
        kits: b"k",
        ssh: None,
        dev: None,
        creator: "test",
    };
    // A 16 GB stick, as a sparse file.
    let disk_path = t.0.join("disk.img");
    let disk = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&disk_path)
        .unwrap();
    let bytes = 16_000_000_000u64;
    disk.set_len(bytes).unwrap();
    assert!(
        stick::write_disk(&b, &disk, 8_000_000_000, &t.0, false, &mut |_| {}).is_err(),
        "too small"
    );
    let made = stick::write_disk(&b, &disk, bytes, &t.0, false, &mut |_| {}).unwrap();
    assert!(
        stick::existing_data(&disk).is_none(),
        "marked, not yet made"
    );
    // The MPC makes partition 4's filesystem at the first start, and the
    // first-start setup puts a backup in it: stood in for here.
    let p4 = made.layout.part(4).clone();
    let image = b"the eMMC's user area".to_vec();
    let boot = b"boot".to_vec();
    let sums = format!(
        "{}  mmcblk0.img\n{}  mmcblk0boot0.img\n",
        update::hex(&Sha256::digest(&image)),
        update::hex(&Sha256::digest(&boot))
    );
    let m = |perm| Meta::new(perm, 0, 960, 1_700_000_000);
    let mut data = Tree::new(m(0o755));
    let dir = data
        .mkdir_all("/backup/mpc-one-emmc-20261115T120000Z", m(0o750))
        .unwrap();
    for (n, b) in [
        ("mmcblk0.img", &image),
        ("mmcblk0boot0.img", &boot),
        ("SHA256SUMS", &sums.as_bytes().to_vec()),
    ] {
        data.add(
            dir,
            n.as_bytes(),
            Body::File(Data::Bytes(b.clone())),
            m(0o640),
        )
        .unwrap();
    }
    data.mkdir_all("/backup/mpc-one-emmc-20261116T000000Z.part", m(0o750))
        .unwrap();
    ext2w::write(
        &data,
        &Options {
            label: stick::DATA_LABEL.into(),
            uuid: [9; 16],
            time: 1_700_000_000,
            size: Size::Bytes(64 << 20),
        },
        &Offset {
            inner: &disk,
            start: p4.start_bytes(),
            len: 64 << 20,
        },
    )
    .unwrap();
    assert_eq!(stick::existing_data(&disk), Some(p4.last));
    // Made again: partition 4 is kept as it is.
    let again = stick::write_disk(&b, &disk, bytes, &t.0, false, &mut |_| {}).unwrap();
    assert_eq!(stick::existing_data(&disk), Some(p4.last), "kept");
    assert_eq!(
        stick::uuid_text(&again.layout.part(4).guid),
        again.values.data_uuid
    );
    // The newest finished backup, copied off and checked.
    let to = t.0.join("backups");
    let got = stick::copy_backup(&disk, &to, &mut |_| {}).unwrap();
    assert_eq!(got, to.join("mpc-one-emmc-20261115T120000Z"));
    assert_eq!(std::fs::read(got.join("mmcblk0.img")).unwrap(), image);
    assert!(
        stick::copy_backup(&disk, &to, &mut |_| {}).is_err(),
        "never over a copy"
    );
    // A copy that does not match its sums is not kept under the backup's name.
    let bad_sums = sums.replace(&update::hex(&Sha256::digest(&boot)), &"0".repeat(64));
    {
        let mut data = Tree::new(m(0o755));
        let dir = data
            .mkdir_all("/backup/mpc-one-emmc-20261115T120000Z", m(0o750))
            .unwrap();
        for (n, b) in [
            ("mmcblk0.img", &image),
            ("mmcblk0boot0.img", &boot),
            ("SHA256SUMS", &bad_sums.as_bytes().to_vec()),
        ] {
            data.add(
                dir,
                n.as_bytes(),
                Body::File(Data::Bytes(b.clone())),
                m(0o640),
            )
            .unwrap();
        }
        ext2w::write(
            &data,
            &Options {
                label: stick::DATA_LABEL.into(),
                uuid: [9; 16],
                time: 1_700_000_000,
                size: Size::Bytes(64 << 20),
            },
            &Offset {
                inner: &disk,
                start: p4.start_bytes(),
                len: 64 << 20,
            },
        )
        .unwrap();
        let to2 = t.0.join("backups2");
        assert!(stick::copy_backup(&disk, &to2, &mut |_| {}).is_err());
        assert!(!to2.join("mpc-one-emmc-20261115T120000Z").exists());
    }
    // Remove mode: mode.txt says remove, and partition 1 has another name, so
    // the hook passes over the stick at power-on. Install mode undoes both.
    let before_table = gpt::read(&disk).unwrap();
    stick::set_mode(&disk, bytes, "remove").unwrap();
    let (_, mut setup) = stick::read_setup(&disk).unwrap();
    assert_eq!(
        stick::setup_read(&mut setup, "mode.txt").unwrap().unwrap(),
        b"remove\n"
    );
    let off = gpt::read(&disk).unwrap();
    assert!(off.by_name(stick::ROOT_NAME).is_none());
    assert_eq!(off.part(1).unwrap().name, stick::ROOT_OFF_NAME);
    assert_eq!(stick::root_part(&off).unwrap().0, 1);
    stick::set_mode(&disk, bytes, "install").unwrap();
    assert_eq!(
        gpt::read(&disk).unwrap(),
        before_table,
        "install undoes remove"
    );
    let (_, mut setup) = stick::read_setup(&disk).unwrap();
    assert_eq!(
        stick::setup_read(&mut setup, "mode.txt").unwrap().unwrap(),
        b"install\n"
    );
    assert!(stick::set_mode(&disk, bytes, "anything").is_err());
    // A fresh partition 4, when asked: marked again.
    stick::write_disk(&b, &disk, bytes, &t.0, true, &mut |_| {}).unwrap();
    assert!(stick::existing_data(&disk).is_none());
    // Nothing is left behind in the work folder but what the test made.
    let left: Vec<String> = std::fs::read_dir(&t.0)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("mpfree-stick-head") || n.starts_with("akai-rootfs"))
        .collect();
    assert_eq!(
        left.len(),
        1,
        "only the open update's unpacked system: {left:?}"
    );
    drop(src);
    let _ = WriteAt::write_at(&disk, &[0], 0);
}
