//! USB sticks: finding them, and opening one to read or write, on Linux,
//! macOS and Windows.
//!
//! Only USB disks are listed or opened, never an internal disk, and never
//! the disk the computer runs from. Opening a stick for writing first lets
//! go of whatever the system mounted from it (Linux: `umount`, macOS:
//! `diskutil unmountDisk`, Windows: each volume locked and dismounted), and
//! needs the rights to write a disk (root, an administrator).
//!
//! Linux is tested. macOS and Windows are written to their documented
//! interfaces and checked to compile, but not yet tried on a real stick.

use crate::bad;
use mpfree_diskimage::{ReadAt, WriteAt};
use std::fs::File;
use std::io;
use std::process::Command;

/// A USB disk.
#[derive(Clone, Debug, PartialEq)]
pub struct DiskInfo {
    /// What to open: `/dev/sdb`, `/dev/disk4`, `\\.\PhysicalDrive2`.
    pub id: String,
    /// Its maker and model, as the system names it.
    pub name: String,
    pub bytes: u64,
    /// Bytes per sector; MPFree's stick needs 512.
    pub sector: u64,
    /// What the system has mounted from it.
    pub mounted: Vec<String>,
}

impl DiskInfo {
    pub fn describe(&self) -> String {
        let name = if self.name.trim().is_empty() {
            "USB disk"
        } else {
            self.name.trim()
        };
        format!("{name}, {:.1} GB ({})", self.bytes as f64 / 1e9, self.id)
    }
}

fn run(cmd: &str, args: &[&str]) -> io::Result<String> {
    let out = Command::new(cmd)
        .args(args)
        .output()
        .map_err(|e| io::Error::new(e.kind(), format!("{cmd}: {e}")))?;
    if !out.status.success() {
        return Err(bad(format!(
            "{cmd} {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The USB disks there are now.
pub fn list() -> io::Result<Vec<DiskInfo>> {
    imp::list()
}

/// A stick opened for reading or writing. Reads and writes may be at any
/// offset and length: where the system needs whole sectors they are read
/// around and written back.
pub struct OpenDisk {
    pub info: DiskInfo,
    file: File,
    /// The alignment the system's raw device needs (1: none).
    align: u64,
    #[cfg(windows)]
    _locks: Vec<File>,
}

impl OpenDisk {
    /// Lets the system see the new partitions (best effort).
    pub fn finish(self) -> io::Result<()> {
        self.file.sync_all()?;
        let info = self.info.clone();
        drop(self);
        imp::reread(&info);
        Ok(())
    }
}

/// Opens a USB stick (an `id` from [`list`]) for reading.
pub fn open_read(id: &str) -> io::Result<OpenDisk> {
    let info = find(id)?;
    imp::open(info, false)
}

/// Opens a USB stick (an `id` from [`list`]) for writing, unmounting
/// whatever the system mounted from it.
pub fn open_write(id: &str) -> io::Result<OpenDisk> {
    let info = find(id)?;
    if info.sector != 512 {
        return Err(bad(format!(
            "{} has {}-byte sectors; MPFree's stick needs 512-byte sectors (another stick will do)",
            info.describe(),
            info.sector
        )));
    }
    imp::open(info, true)
}

fn find(id: &str) -> io::Result<DiskInfo> {
    list()?.into_iter().find(|d| d.id == id).ok_or_else(|| {
        bad(format!(
            "{id} is not a USB disk this computer has now; only USB sticks are written (see the list of disks)"
        ))
    })
}

fn align_range(offset: u64, len: usize, align: u64) -> (u64, usize) {
    let start = offset / align * align;
    let end = (offset + len as u64).div_ceil(align) * align;
    (start, (end - start) as usize)
}

impl ReadAt for OpenDisk {
    fn read_at(&self, buf: &mut [u8], offset: u64) -> io::Result<()> {
        if self.align <= 1
            || (offset.is_multiple_of(self.align) && (buf.len() as u64).is_multiple_of(self.align))
        {
            return self.file.read_at(buf, offset);
        }
        let (start, len) = align_range(offset, buf.len(), self.align);
        let mut tmp = vec![0u8; len];
        self.file.read_at(&mut tmp, start)?;
        let at = (offset - start) as usize;
        buf.copy_from_slice(&tmp[at..at + buf.len()]);
        Ok(())
    }
}

impl WriteAt for OpenDisk {
    fn write_at(&self, buf: &[u8], offset: u64) -> io::Result<()> {
        if self.align <= 1
            || (offset.is_multiple_of(self.align) && (buf.len() as u64).is_multiple_of(self.align))
        {
            return self.file.write_at(buf, offset);
        }
        let (start, len) = align_range(offset, buf.len(), self.align);
        let mut tmp = vec![0u8; len];
        self.file.read_at(&mut tmp, start)?;
        let at = (offset - start) as usize;
        tmp[at..at + buf.len()].copy_from_slice(buf);
        self.file.write_at(&tmp, start)
    }
}

impl crate::stick::Disk for OpenDisk {
    fn sync(&self) -> io::Result<()> {
        self.file.sync_all()
    }
}

/// The value after `<key>KEY</key>` in a macOS property list: the text of
/// the next `<string>` or `<integer>`, or "true"/"false".
pub fn plist_value(plist: &str, key: &str) -> Option<String> {
    let at = plist.find(&format!("<key>{key}</key>"))? + key.len() + 11;
    let rest = plist[at..].trim_start();
    for b in ["true", "false"] {
        if rest.starts_with(&format!("<{b}/>")) {
            return Some(b.to_string());
        }
    }
    for tag in ["string", "integer"] {
        if let Some(r) = rest.strip_prefix(&format!("<{tag}>")) {
            return r
                .split_once(&format!("</{tag}>"))
                .map(|(v, _)| v.to_string());
        }
    }
    None
}

/// The strings of the array after `<key>KEY</key>`.
pub fn plist_strings(plist: &str, key: &str) -> Vec<String> {
    let Some(at) = plist.find(&format!("<key>{key}</key>")) else {
        return Vec::new();
    };
    let rest = &plist[at..];
    let Some(end) = rest.find("</array>") else {
        return Vec::new();
    };
    rest[..end]
        .split("<string>")
        .skip(1)
        .filter_map(|s| s.split_once("</string>").map(|(v, _)| v.to_string()))
        .collect()
}

#[cfg(target_os = "linux")]
mod imp {
    use super::*;
    use std::os::unix::fs::OpenOptionsExt;

    /// Where the computer runs from: a disk with one of these mounted is never listed.
    const SYSTEM_MOUNTS: &[&str] = &["/", "/boot", "/boot/efi", "/efi", "/home", "/usr", "/var"];

    fn read(path: &str) -> String {
        std::fs::read_to_string(path)
            .unwrap_or_default()
            .trim()
            .to_string()
    }

    fn mounts() -> Vec<(String, String)> {
        let text = std::fs::read_to_string("/proc/mounts").unwrap_or_default();
        let mut out: Vec<(String, String)> = text
            .lines()
            .filter_map(|l| {
                let mut f = l.split(' ');
                Some((f.next()?.to_string(), f.next()?.replace("\\040", " ")))
            })
            .collect();
        // Swap in use counts as the system's.
        for l in std::fs::read_to_string("/proc/swaps")
            .unwrap_or_default()
            .lines()
            .skip(1)
        {
            if let Some(dev) = l.split_whitespace().next() {
                out.push((dev.to_string(), "/".to_string()));
            }
        }
        out
    }

    fn holds(disk: &str, dev: &str) -> bool {
        // /dev/sdb1, and links such as /dev/disk/by-uuid/... resolved.
        let real = std::fs::canonicalize(dev)
            .map(|p| p.display().to_string())
            .unwrap_or(dev.to_string());
        real.strip_prefix(disk)
            .is_some_and(|rest| rest.chars().all(|c| c.is_ascii_digit()))
    }

    pub fn list() -> io::Result<Vec<DiskInfo>> {
        let mut out = Vec::new();
        let all = mounts();
        for e in std::fs::read_dir("/sys/block")? {
            let name = e?.file_name().to_string_lossy().into_owned();
            if !name.starts_with("sd") || !name[2..].chars().all(|c| c.is_ascii_lowercase()) {
                continue;
            }
            let sys = format!("/sys/block/{name}");
            let real = std::fs::canonicalize(&sys)
                .map(|p| p.display().to_string())
                .unwrap_or_default();
            if !real.contains("/usb") {
                continue;
            }
            let id = format!("/dev/{name}");
            let mounted: Vec<(String, String)> =
                all.iter().filter(|(d, _)| holds(&id, d)).cloned().collect();
            if mounted
                .iter()
                .any(|(_, m)| SYSTEM_MOUNTS.contains(&m.as_str()))
            {
                continue;
            }
            let sectors: u64 = read(&format!("{sys}/size")).parse().unwrap_or(0);
            if sectors == 0 {
                continue; // a card reader with no card
            }
            out.push(DiskInfo {
                id,
                name: format!(
                    "{} {}",
                    read(&format!("{sys}/device/vendor")),
                    read(&format!("{sys}/device/model"))
                ),
                bytes: sectors * 512,
                sector: read(&format!("{sys}/queue/logical_block_size"))
                    .parse()
                    .unwrap_or(512),
                mounted: mounted
                    .into_iter()
                    .map(|(d, m)| format!("{d} on {m}"))
                    .collect(),
            });
        }
        out.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(out)
    }

    pub fn open(info: DiskInfo, write: bool) -> io::Result<OpenDisk> {
        if write {
            for (dev, _) in mounts().into_iter().filter(|(d, _)| holds(&info.id, d)) {
                run("umount", &[&dev])?;
            }
        }
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(write)
            // Exclusive: fails while anything still has a partition of it mounted.
            .custom_flags(if write { libc::O_EXCL } else { 0 })
            .open(&info.id)
            .map_err(|e| match e.kind() {
                io::ErrorKind::PermissionDenied => {
                    io::Error::new(e.kind(), format!("{}: needs root ({e})", info.id))
                }
                _ => io::Error::new(
                    e.kind(),
                    format!("{}: {e} (is something still using the stick?)", info.id),
                ),
            })?;
        Ok(OpenDisk {
            info,
            file,
            align: 1,
        })
    }

    pub fn reread(info: &DiskInfo) {
        let _ = run("blockdev", &["--rereadpt", &info.id]);
        let _ = run("udevadm", &["settle", "--timeout=10"]);
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use super::*;

    pub fn list() -> io::Result<Vec<DiskInfo>> {
        let all = run("diskutil", &["list", "-plist", "external", "physical"])?;
        let mut out = Vec::new();
        for disk in plist_strings(&all, "WholeDisks") {
            if !disk.starts_with("disk") || !disk[4..].chars().all(|c| c.is_ascii_digit()) {
                continue;
            }
            let info = run("diskutil", &["info", "-plist", &disk])?;
            if plist_value(&info, "Internal").as_deref() != Some("false")
                || plist_value(&info, "BusProtocol").as_deref() != Some("USB")
            {
                continue;
            }
            out.push(DiskInfo {
                id: format!("/dev/{disk}"),
                name: plist_value(&info, "MediaName").unwrap_or_default(),
                bytes: plist_value(&info, "TotalSize")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0),
                sector: plist_value(&info, "DeviceBlockSize")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(512),
                mounted: Vec::new(),
            });
        }
        Ok(out)
    }

    pub fn open(info: DiskInfo, write: bool) -> io::Result<OpenDisk> {
        if write {
            run("diskutil", &["unmountDisk", &info.id])?;
        }
        // The raw device: much faster, and needs whole sectors.
        let raw = info.id.replacen("/dev/disk", "/dev/rdisk", 1);
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(write)
            .open(&raw)
            .map_err(|e| {
                io::Error::new(
                    e.kind(),
                    format!("{raw}: {e} (the creator needs your password to write a stick)"),
                )
            })?;
        let align = info.sector.max(512);
        Ok(OpenDisk { info, file, align })
    }

    pub fn reread(_info: &DiskInfo) {}
}

#[cfg(windows)]
mod imp {
    use super::*;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::System::Ioctl::{FSCTL_DISMOUNT_VOLUME, FSCTL_LOCK_VOLUME};
    use windows_sys::Win32::System::IO::DeviceIoControl;

    fn powershell(script: &str) -> io::Result<String> {
        run(
            "powershell",
            &["-NoProfile", "-NonInteractive", "-Command", script],
        )
    }

    pub fn list() -> io::Result<Vec<DiskInfo>> {
        let text = powershell(
            "Get-Disk | Where-Object { $_.BusType -eq 'USB' -and -not $_.IsBoot -and -not $_.IsSystem } | \
             ForEach-Object { '{0}|{1}|{2}|{3}' -f $_.Number, $_.Size, $_.LogicalSectorSize, $_.FriendlyName }",
        )?;
        Ok(text
            .lines()
            .filter_map(|l| {
                let mut f = l.trim().splitn(4, '|');
                let n: u32 = f.next()?.parse().ok()?;
                Some(DiskInfo {
                    id: format!(r"\\.\PhysicalDrive{n}"),
                    bytes: f.next()?.parse().ok()?,
                    sector: f.next()?.parse().unwrap_or(512),
                    name: f.next().unwrap_or("").to_string(),
                    mounted: Vec::new(),
                })
            })
            .collect())
    }

    fn number(info: &DiskInfo) -> io::Result<u32> {
        info.id
            .strip_prefix(r"\\.\PhysicalDrive")
            .and_then(|n| n.parse().ok())
            .ok_or_else(|| bad(format!("{} is not a disk number", info.id)))
    }

    fn ioctl(f: &File, code: u32) -> io::Result<()> {
        let mut n = 0u32;
        // SAFETY: a valid handle for the file's lifetime, no in or out buffers.
        let ok = unsafe {
            DeviceIoControl(
                f.as_raw_handle() as _,
                code,
                std::ptr::null(),
                0,
                std::ptr::null_mut(),
                0,
                &mut n,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    pub fn open(info: DiskInfo, write: bool) -> io::Result<OpenDisk> {
        let mut locks = Vec::new();
        if write {
            let n = number(&info)?;
            let paths = powershell(&format!(
                "Get-Partition -DiskNumber {n} -ErrorAction SilentlyContinue | ForEach-Object {{ $_.AccessPaths }} | \
                 Where-Object {{ $_ -like '\\\\?\\Volume*' }}"
            ))?;
            for p in paths
                .lines()
                .map(|l| l.trim().trim_end_matches('\\'))
                .filter(|l| !l.is_empty())
            {
                let v = std::fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(p)
                    .map_err(|e| io::Error::new(e.kind(), format!("{p}: {e}")))?;
                ioctl(&v, FSCTL_LOCK_VOLUME).map_err(|e| {
                    bad(format!(
                        "could not lock {p}: {e} (close any window showing the stick)"
                    ))
                })?;
                ioctl(&v, FSCTL_DISMOUNT_VOLUME)
                    .map_err(|e| bad(format!("could not dismount {p}: {e}")))?;
                locks.push(v);
            }
        }
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(write)
            .open(&info.id)
            .map_err(|e| {
                io::Error::new(
                    e.kind(),
                    format!(
                        "{}: {e} (the creator needs to run as administrator)",
                        info.id
                    ),
                )
            })?;
        let align = info.sector.max(512);
        Ok(OpenDisk {
            info,
            file,
            align,
            _locks: locks,
        })
    }

    pub fn reread(info: &DiskInfo) {
        if let Ok(n) = number(info) {
            let _ = powershell(&format!("Update-Disk -Number {n}"));
        }
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
mod imp {
    use super::*;

    pub fn list() -> io::Result<Vec<DiskInfo>> {
        Ok(Vec::new())
    }

    pub fn open(info: DiskInfo, _write: bool) -> io::Result<OpenDisk> {
        Err(bad(format!("{}: this system is not supported", info.id)))
    }

    pub fn reread(_info: &DiskInfo) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_macos_property_lists() {
        let p = "<dict>\n\t<key>WholeDisks</key>\n\t<array>\n\t\t<string>disk4</string>\n\t\t<string>disk5</string>\n\t</array>\n\
                 \t<key>Internal</key>\n\t<false/>\n\t<key>TotalSize</key>\n\t<integer>31004295168</integer>\n\
                 \t<key>MediaName</key>\n\t<string>SanDisk Ultra</string>\n</dict>";
        assert_eq!(plist_strings(p, "WholeDisks"), ["disk4", "disk5"]);
        assert_eq!(plist_value(p, "Internal").as_deref(), Some("false"));
        assert_eq!(plist_value(p, "TotalSize").as_deref(), Some("31004295168"));
        assert_eq!(
            plist_value(p, "MediaName").as_deref(),
            Some("SanDisk Ultra")
        );
        assert_eq!(plist_value(p, "Missing"), None);
    }

    #[test]
    fn unaligned_reads_and_writes_go_through_whole_sectors() {
        assert_eq!(align_range(700, 100, 512), (512, 512));
        assert_eq!(align_range(500, 100, 512), (0, 1024));
        let path = std::env::temp_dir().join(format!("mpfree-disks-test-{}", std::process::id()));
        let f = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)
            .unwrap();
        f.set_len(4096).unwrap();
        let d = OpenDisk {
            info: DiskInfo {
                id: "test".into(),
                name: String::new(),
                bytes: 4096,
                sector: 512,
                mounted: Vec::new(),
            },
            file: f,
            align: 512,
            #[cfg(windows)]
            _locks: Vec::new(),
        };
        d.write_at(b"hello", 510).unwrap();
        let mut b = [0u8; 7];
        d.read_at(&mut b, 509).unwrap();
        assert_eq!(&b, b"\0hello\0");
        let _ = std::fs::remove_file(&path);
    }
}
