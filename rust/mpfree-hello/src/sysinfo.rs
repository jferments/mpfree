//! Prefs > System info: what the pad player is and what it runs on, read
//! from /proc, /sys and /etc when the page opens. It only reads, and it
//! leaves out the serial number and the network addresses.

use std::fs;
use std::path::Path;

/// The page's two lists of name / value rows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SystemInfo {
    pub software: Vec<(String, String)>,
    pub hardware: Vec<(String, String)>,
}

/// The commit this program was built from (build.rs), or "unknown".
pub const COMMIT: &str = env!("MPFREE_COMMIT");

/// Read everything under `root` ("/" on the MPC; a fixture folder in tests).
pub fn read(root: &Path) -> SystemInfo {
    let text = |p: &str| fs::read_to_string(root.join(p)).ok();
    let mut software = vec![("MPFree".to_string(), format!("pad player {}, commit {COMMIT}", env!("CARGO_PKG_VERSION")))];
    let os = text("etc/os-release").or_else(|| text("usr/lib/os-release")).map(|t| os_name(&t));
    software.push(("Operating system".into(), os.flatten().unwrap_or_else(|| "not found".into())));
    let release = text("proc/sys/kernel/osrelease").map(|s| s.trim().to_string());
    let rt = text("proc/version").is_some_and(|v| v.contains("PREEMPT_RT"));
    software.push(("Kernel".into(), match release {
        Some(r) if rt => format!("Linux {r} (real-time)"),
        Some(r) => format!("Linux {r}"),
        None => "not found".into(),
    }));
    if let Some(m) = text("proc/mounts").and_then(|m| root_device(&m)) {
        software.push(("Root file system".into(), m));
    }
    if let Some(h) = text("proc/sys/kernel/hostname") {
        software.push(("Host name".into(), h.trim().to_string()));
    }
    if let Some(s) = text("proc/uptime").and_then(|u| u.split_whitespace().next()?.parse::<f64>().ok()) {
        software.push(("Running for".into(), duration(s)));
    }

    let mut hardware = Vec::new();
    if let Some(m) = text("proc/device-tree/model") {
        hardware.push(("Board".into(), m.trim_end_matches('\0').trim().to_string()));
    }
    if let Some(c) = text("proc/cpuinfo").and_then(|c| cpu(&c)) {
        hardware.push(("Processor".into(), c));
    }
    if let Some(khz) = text("sys/devices/system/cpu/cpu0/cpufreq/scaling_cur_freq")
        .and_then(|f| f.trim().parse::<u64>().ok())
    {
        hardware.push(("Clock".into(), format!("{} MHz", khz / 1000)));
    }
    if let Some(m) = text("proc/meminfo").and_then(|m| memory(&m)) {
        hardware.push(("Memory".into(), m));
    }
    if let Some(t) = text("sys/class/thermal/thermal_zone0/temp").and_then(|t| t.trim().parse::<f64>().ok()) {
        hardware.push(("Temperature".into(), format!("{:.1} °C", t / 1000.0)));
    }
    for (n, name) in text("proc/asound/cards").map(|c| sound_cards(&c)).unwrap_or_default() {
        hardware.push((format!("Sound card {n}"), name));
    }
    SystemInfo { software, hardware }
}

/// PRETTY_NAME, or NAME and VERSION, from os-release.
fn os_name(t: &str) -> Option<String> {
    let field = |k: &str| {
        t.lines().find_map(|l| l.strip_prefix(k)?.strip_prefix('=')).map(|v| v.trim().trim_matches('"').to_string())
    };
    field("PRETTY_NAME").filter(|s| !s.is_empty()).or_else(|| {
        let name = field("NAME")?;
        Some(match field("VERSION") {
            Some(v) => format!("{name} {v}"),
            None => name,
        })
    })
}

/// The root file system: its device, type, and read-only or read-write.
fn root_device(m: &str) -> Option<String> {
    let (dev, fs_type, opts) = m.lines().rev().find_map(|l| {
        let f: Vec<&str> = l.split_whitespace().collect();
        (f.len() >= 4 && f[1] == "/").then(|| (f[0], f[2], f[3]))
    })?;
    let ro = opts.split(',').any(|o| o == "ro");
    Some(format!("{dev}, {fs_type}, {}", if ro { "read-only" } else { "read-write" }))
}

/// "4 × ARMv7 Processor rev 1 (v7l)".
fn cpu(c: &str) -> Option<String> {
    let names: Vec<&str> = c.lines()
        .filter_map(|l| {
            let (k, v) = l.split_once(':')?;
            (k.trim() == "model name").then(|| v.trim())
        })
        .collect();
    let first = names.first()?;
    Some(format!("{} × {first}", names.len()))
}

/// "2042 MB, 1610 MB available".
fn memory(m: &str) -> Option<String> {
    let kb = |k: &str| {
        m.lines().find_map(|l| l.strip_prefix(k)?.strip_prefix(':')?.split_whitespace().next()?.parse::<u64>().ok())
    };
    let total = kb("MemTotal")?;
    Some(match kb("MemAvailable") {
        Some(a) => format!("{} MB, {} MB available", total / 1024, a / 1024),
        None => format!("{} MB", total / 1024),
    })
}

/// The cards' numbers and names from /proc/asound/cards (" 1 [ACVA ]: ... - Name").
fn sound_cards(c: &str) -> Vec<(u32, String)> {
    c.lines()
        .filter_map(|l| {
            let (n, rest) = l.trim_start().split_once(' ')?;
            let n = n.parse::<u32>().ok()?;
            let id = rest.trim_start().strip_prefix('[')?.split_once(']')?.0.trim();
            let name = rest.rsplit_once(" - ").map(|(_, name)| name.trim()).unwrap_or(id);
            Some((n, if name == id { id.to_string() } else { format!("{id}: {name}") }))
        })
        .collect()
}

/// "2 h 05 min", "4 min 10 s".
fn duration(s: f64) -> String {
    let s = s as u64;
    let (h, m) = (s / 3600, s / 60 % 60);
    if h > 0 {
        format!("{h} h {m:02} min")
    } else {
        format!("{m} min {:02} s", s % 60)
    }
}

/// What the preview shows (fixed, so the picture is the same on any computer).
pub fn sample() -> SystemInfo {
    let rows = |r: &[(&str, &str)]| r.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    SystemInfo {
        software: rows(&[("MPFree", "pad player 0.1.0, commit caf5309"), ("Operating system", "Akai MPC OS (sample)"),
                         ("Kernel", "Linux 6.18.44-az01-2026-08-10-rt6 (real-time)"),
                         ("Root file system", "/dev/root, ext4, read-only"), ("Host name", "mpc-one"),
                         ("Running for", "12 min 40 s")]),
        hardware: rows(&[("Board", "Akai Professional MPC One"), ("Processor", "4 × ARMv7 Processor rev 1 (v7l)"),
                         ("Clock", "1608 MHz"), ("Memory", "2042 MB, 1610 MB available"), ("Temperature", "47.8 °C"),
                         ("Sound card 0", "Live: MPC Studio Live"), ("Sound card 1", "ACVA: ACVA")]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_fixture_tree_and_leaves_out_what_is_missing() {
        let dir = std::env::temp_dir().join(format!("mpfree-sysinfo-{}", std::process::id()));
        let put = |p: &str, s: &str| {
            let f = dir.join(p);
            fs::create_dir_all(f.parent().unwrap()).unwrap();
            fs::write(f, s).unwrap();
        };
        put("etc/os-release", "NAME=\"Poky\"\nVERSION=\"5.0 (scarthgap)\"\n");
        put("proc/sys/kernel/osrelease", "6.18.44-az01-2026-08-10-rt6\n");
        put("proc/version", "Linux version 6.18.44 #1 SMP PREEMPT_RT Mon Aug 10\n");
        put("proc/mounts", "tmpfs /run tmpfs rw 0 0\n/dev/sda1 / ext4 ro,relatime 0 0\n");
        put("proc/uptime", "760.25 2900.00\n");
        put("proc/device-tree/model", "Akai Professional MPC One\0");
        put("proc/cpuinfo", "processor\t: 0\nmodel name\t: ARMv7 Processor rev 1 (v7l)\n\nprocessor\t: 1\n\
                             model name\t: ARMv7 Processor rev 1 (v7l)\n");
        put("proc/meminfo", "MemTotal:        2091008 kB\nMemFree:  100 kB\nMemAvailable:    1648640 kB\n");
        put("proc/asound/cards", " 0 [Live           ]: USB-Audio - MPC Studio Live\n                      \
                                  Akai Professional MPC Studio Live at usb-ff500000.usb-1.1, high speed\n \
                                  1 [ACVA           ]: simple-card - ACVA\n                      ACVA\n");
        let info = read(&dir);
        let get = |rows: &[(String, String)], k: &str| rows.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
        assert_eq!(get(&info.software, "Operating system").as_deref(), Some("Poky 5.0 (scarthgap)"));
        assert_eq!(get(&info.software, "Kernel").as_deref(), Some("Linux 6.18.44-az01-2026-08-10-rt6 (real-time)"));
        assert_eq!(get(&info.software, "Root file system").as_deref(), Some("/dev/sda1, ext4, read-only"));
        assert_eq!(get(&info.software, "Running for").as_deref(), Some("12 min 40 s"));
        assert_eq!(get(&info.software, "Host name"), None);
        assert!(get(&info.software, "MPFree").unwrap().starts_with("pad player 0.1.0, commit "));
        assert_eq!(get(&info.hardware, "Board").as_deref(), Some("Akai Professional MPC One"));
        assert_eq!(get(&info.hardware, "Processor").as_deref(), Some("2 × ARMv7 Processor rev 1 (v7l)"));
        assert_eq!(get(&info.hardware, "Memory").as_deref(), Some("2042 MB, 1610 MB available"));
        assert_eq!(get(&info.hardware, "Clock"), None);
        assert_eq!(get(&info.hardware, "Sound card 0").as_deref(), Some("Live: MPC Studio Live"));
        assert_eq!(get(&info.hardware, "Sound card 1").as_deref(), Some("ACVA"));
        assert_eq!(info.hardware.len(), 5);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_empty_tree_still_names_the_program() {
        let info = read(Path::new("/nonexistent-mpfree-root"));
        assert_eq!(info.software[0].0, "MPFree");
        assert_eq!(info.software[1], ("Operating system".to_string(), "not found".to_string()));
        assert!(info.hardware.is_empty());
        assert_eq!(duration(7500.0), "2 h 05 min");
        assert_eq!(os_name("PRETTY_NAME=\"Akai OS 3.10\"\nNAME=x\n").as_deref(), Some("Akai OS 3.10"));
    }
}
