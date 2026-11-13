//! The app serving as the plugin process (`mpfree plughost ...`): a stick
//! with only the app on it still hosts CLAP plugins, each in its own
//! process. The test plugins' real `.clap` file is listed and played
//! through the plugin rack with the app as its program.

use mpfree_clap::testplug::{GAIN_ID, SYNTH_ID};
use mpfree_node::{Ctx, Kind};
use mpfree_plughost::rack::Rack;
use mpfree_plughost::{Deadline, Program};
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

/// The app as the plugin process. Under CI's ARMv7 step (qemu-arm as
/// cargo's runner) the ARM program is started through the runner.
///
/// The device build is static, and a plugin file is a shared library that
/// needs the device's own `libc.so.6` and `libgcc_s.so.1` from `/lib`. On
/// an x86 host those are the cross toolchain's ARM copies, so qemu is told
/// to look there first (`-L`, as `QEMU_LD_PREFIX` would), unless
/// `QEMU_LD_PREFIX` already says where.
fn program() -> Program {
    let exe = env!("CARGO_BIN_EXE_mpfree");
    match std::env::var("CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABIHF_RUNNER")
        .ok()
        .filter(|_| cfg!(target_arch = "arm"))
    {
        Some(r) if is_qemu(&r) && std::env::var_os("QEMU_LD_PREFIX").is_none() => {
            Program::with_lead(r.as_ref(), &["-L", ARM_LIBS, exe, "plughost"])
        }
        Some(r) => Program::with_lead(r.as_ref(), &[exe, "plughost"]),
        None => Program::with_lead(exe.as_ref(), &["plughost"]),
    }
}

/// Where Ubuntu's ARMv7 hard-float cross toolchain keeps the target's
/// `lib/` (gcc-arm-linux-gnueabihf, libc6-armhf-cross).
const ARM_LIBS: &str = "/usr/arm-linux-gnueabihf";

fn is_qemu(runner: &str) -> bool {
    std::path::Path::new(runner)
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.starts_with("qemu-arm"))
}

fn run(args: &[&str]) -> std::process::Output {
    let p = program();
    Command::new(&p.exe)
        .args(&p.lead)
        .args(args)
        .output()
        .expect("run mpfree plughost")
}

/// The test plugins' shared library, which Cargo builds as a dev-dependency
/// (next to the test, or from Cargo 1.100 in its own build folder).
fn plugin_file() -> PathBuf {
    let exe = std::env::current_exe().expect("test exe");
    let name = format!(
        "{}mpfree_clap_testplug{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    );
    let here = exe.parent().expect("test folder");
    let mut dirs = vec![here.to_path_buf(), here.join("..")];
    if let Some(build) = here.ancestors().nth(3).filter(|b| b.ends_with("build")) {
        if let Ok(rd) = std::fs::read_dir(build.join("mpfree-clap-testplug")) {
            dirs.extend(rd.flatten().map(|e| e.path().join("out")));
        }
    }
    dirs.iter()
        .map(|d| d.join(&name))
        .filter_map(|p| Some((p.metadata().ok()?.modified().ok()?, p)))
        .max_by_key(|(t, _)| *t)
        .map(|(_, p)| p)
        .unwrap_or_else(|| panic!("{name} not built next to {}", exe.display()))
}

#[test]
fn the_app_lists_a_plugin_file_as_the_plugin_process() {
    let out = run(&["--list", plugin_file().to_str().unwrap()]);
    assert!(out.status.success(), "{out:?}");
    let text = String::from_utf8(out.stdout).unwrap();
    let ids: Vec<(String, Kind)> = text
        .lines()
        .filter_map(mpfree_plughost::parse_list_line)
        .map(|l| (l.id, l.kind))
        .collect();
    assert_eq!(
        ids,
        vec![
            (SYNTH_ID.to_string(), Kind::Instrument),
            (GAIN_ID.to_string(), Kind::Effect)
        ]
    );
    // Without the sandbox's arguments it refuses and says why.
    let out = run(&[]);
    assert_eq!(out.status.code(), Some(64));
    assert!(String::from_utf8_lossy(&out.stderr).contains("usage: mpfree-plughost"));
}

#[test]
fn a_plugin_made_with_the_app_as_its_process_plays() {
    // Its own folder, so runs at once never share (or delete) one file.
    let d = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("mpfree-plughost-app-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    std::fs::copy(plugin_file(), d.join("test.clap")).unwrap();
    let (mut rack, notes) = Rack::scan(program(), &d, "files/plugins/");
    rack.set_deadline(Deadline::Fixed(Duration::from_secs(5)));
    assert_eq!(rack.entries().len(), 2, "{notes:?}");
    let mut gain = rack.make("clap:org.mpfree.test-gain").expect("make");
    gain.prepare(44_100, 256);
    let id = gain.params().iter().find(|p| p.name == "Gain").unwrap().id;
    gain.set_param(id, -20.0 * 2f32.log10());
    let (mut l, mut r) = (vec![0.5f32; 256], vec![0.5f32; 256]);
    gain.process(&Ctx::new(44_100, 0), &[], &mut l, &mut r);
    assert!((l[0] - 0.25).abs() < 1e-4, "{}", l[0]);
    assert_eq!(rack.summary().unwrap().running, 1);
    let _ = std::fs::remove_dir_all(&d);
}
