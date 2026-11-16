//! The creator's command line: `mpfree-stick help` lists the commands.
//! The `mpfree-stick` program is this, and the creator's window runs itself
//! as this (`mpfree-creator --cli ...`) with the rights to write a disk.

use crate::stick::{self, Build, Disk, Programs};
use crate::{disks, recipe, ssh, update, Msg, CREATOR};
use mpfree_diskimage::ext4::Ext4;
use mpfree_diskimage::{gpt, Offset, ReadAt, WriteAt};
use std::fs::File;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

pub const HELP: &str = "\
mpfree-stick: makes an MPFree boot stick for the MPC One (Gen 1) from your own Akai update file.
MPFree has only been tested on the MPC One (Gen 1). Nothing here reads or writes the MPC itself.

  mpfree-stick check UPDATE [--any-update]
      Checks Akai's update file (read-only).
  mpfree-stick disks
      Lists the USB disks (read-only).
  mpfree-stick write UPDATE --device DISK --yes-erase DISK [OPTIONS] [--fresh-data | --keep-data]
      Makes the stick. Erases partitions 1, 2, 3 and 5 and the partition table. Partition 4 (your
      settings, files and backups) is kept if the stick already has MPFree on it, unless --fresh-data.
      With --keep-data, nothing is written if partition 4 holds data this version can't keep.
      DISK may be an image file of 16 GB or more, for trying it out.
  mpfree-stick build UPDATE --out FILE [OPTIONS]
      Makes a small stick image file (partition 4 only 64 MiB), for tests.
  mpfree-stick inspect --device DISK
      What is on an MPFree stick: its partitions, what made it, the setup mode, the backups (read-only).
  mpfree-stick copy-backup --device DISK --to FOLDER
      Copies the newest eMMC backup off the stick into FOLDER, checked against its SHA256SUMS
      (read-only on the stick).
  mpfree-stick setup-mode --device DISK install|remove
      Sets what the setup partition's autorun.sh does on the MPC: install the startup hook, or remove it.
  mpfree-stick setup-log --device DISK
      Prints what autorun.sh did on the MPC (read-only).
  mpfree-stick logs --device DISK
      Prints what the stick's systems saved on its log partition, MAC addresses and USB serial
      numbers hidden (read-only).
  mpfree-stick make-ssh-key FOLDER [--name NAME]
      Makes an SSH key pair for developer SSH (NAME defaults to mpfree_ed25519).

UPDATE may be a folder: the supported update in it is used. DISK may be mpfree for the one MPFree
stick that is in (not for write). Reading or writing a disk needs root or an administrator.

Options for write and build:
  --programs FOLDER  mpfree, mpfree-screen-log and mpfree-startup built for the MPC (default:
                     $MPFREE_STICK_PROGRAMS, stick-programs/ next to this program, or the clone's armv7
                     release build)
  --kits FILE        partition 3's kits.bin (default: MPFree's own kits, from the Drums synth)
  --ssh-key FILE     developer SSH, off without it: your public key file (key-only root, port 2222)
  --any-update       developer option: take an Akai update MPFree does not know
  --work FOLDER      where Akai's system is unpacked while the stick is made (default: the temporary
                     folder; needs about 1.5 GB)

  --status FILE      (before the command) also writes each progress line to FILE, for the window
";

/// Progress and notes: to stdout, and to the status file if there is one.
struct Out {
    status: Option<File>,
}

impl Out {
    fn line(&mut self, line: &str) {
        println!("{line}");
        if let Some(f) = &mut self.status {
            let _ = writeln!(f, "{line}");
            let _ = f.flush();
        }
    }

    fn say(&mut self, m: Msg) {
        self.line(&m.line());
    }
}

struct Args {
    words: Vec<String>,
}

impl Args {
    fn flag(&mut self, name: &str) -> bool {
        match self.words.iter().position(|w| w == name) {
            Some(i) => {
                self.words.remove(i);
                true
            }
            None => false,
        }
    }

    fn value(&mut self, name: &str) -> io::Result<Option<String>> {
        match self.words.iter().position(|w| w == name) {
            Some(i) if i + 1 < self.words.len() => {
                let v = self.words.remove(i + 1);
                self.words.remove(i);
                Ok(Some(v))
            }
            Some(_) => Err(usage(&format!("{name} needs a value"))),
            None => Ok(None),
        }
    }

    fn need(&mut self, name: &str) -> io::Result<String> {
        self.value(name)?
            .ok_or_else(|| usage(&format!("{name} is needed")))
    }

    fn positional(&mut self, what: &str) -> io::Result<String> {
        match self.words.iter().position(|w| !w.starts_with("--")) {
            Some(i) => Ok(self.words.remove(i)),
            None => Err(usage(&format!("{what} is needed"))),
        }
    }

    fn done(&self) -> io::Result<()> {
        match self.words.first() {
            Some(w) => Err(usage(&format!("{w} is not understood here"))),
            None => Ok(()),
        }
    }
}

fn usage(msg: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        format!("{msg} (mpfree-stick help lists the commands)"),
    )
}

/// A stick, or an image file standing in for one.
enum Target {
    File(File, u64),
    Disk(disks::OpenDisk),
}

impl Target {
    fn open(dev: &str, write: bool) -> io::Result<Target> {
        // `mpfree`: the one MPFree stick that is in (never for `write`, which needs the disk named).
        let found;
        let dev = if dev == "mpfree" {
            found = disks::find_mpfree()?;
            found.as_str()
        } else {
            dev
        };
        let p = Path::new(dev);
        if p.is_file() {
            let f = std::fs::OpenOptions::new()
                .read(true)
                .write(write)
                .open(p)?;
            let len = f.metadata()?.len();
            return Ok(Target::File(f, len));
        }
        let d = if write {
            disks::open_write(dev)?
        } else {
            disks::open_read(dev)?
        };
        Ok(Target::Disk(d))
    }

    fn bytes(&self) -> u64 {
        match self {
            Target::File(_, n) => *n,
            Target::Disk(d) => d.info.bytes,
        }
    }

    fn finish(self) -> io::Result<()> {
        match self {
            Target::File(f, _) => f.sync_all(),
            Target::Disk(d) => d.finish(),
        }
    }
}

impl ReadAt for Target {
    fn read_at(&self, buf: &mut [u8], offset: u64) -> io::Result<()> {
        match self {
            Target::File(f, _) => f.read_at(buf, offset),
            Target::Disk(d) => d.read_at(buf, offset),
        }
    }
}

impl WriteAt for Target {
    fn write_at(&self, buf: &[u8], offset: u64) -> io::Result<()> {
        match self {
            Target::File(f, _) => f.write_at(buf, offset),
            Target::Disk(d) => d.write_at(buf, offset),
        }
    }
}

impl Disk for Target {
    fn sync(&self) -> io::Result<()> {
        match self {
            Target::File(f, _) => f.sync_all(),
            Target::Disk(d) => d.sync(),
        }
    }
}

/// MPFree's programs for the MPC: `MPFREE_STICK_PROGRAMS`, else
/// `stick-programs/` next to this program (as a release ships it), else a
/// clone's ARM build.
pub fn default_programs() -> Option<PathBuf> {
    let has = |p: &Path| {
        crate::stick::PROGRAM_FILES
            .iter()
            .all(|f| p.join(f).is_file())
    };
    if let Some(p) = std::env::var_os("MPFREE_STICK_PROGRAMS") {
        return Some(PathBuf::from(p)).filter(|p| has(p));
    }
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    [
        dir.join("stick-programs"),
        dir.join("../Resources/stick-programs"),
        dir.join("../armv7-unknown-linux-gnueabihf/release"),
        dir.join("../../../target/armv7-unknown-linux-gnueabihf/release"),
    ]
    .into_iter()
    .find(|p| has(p))
}

/// What write and build share: the update checked and unpacked, the
/// programs, the kits and developer SSH.
struct Inputs {
    source: stick::Source,
    programs: Programs,
    kits: Vec<u8>,
    ssh: Option<stick::Ssh>,
    work: PathBuf,
}

/// The update file: the path given, or the update to use in a folder.
fn update_path(given: &str, any_update: bool, out: &mut Out) -> io::Result<PathBuf> {
    let p = PathBuf::from(given);
    if !p.is_dir() {
        return Ok(p);
    }
    let found = update::find_in_dir(&p, any_update)?;
    out.say(Msg::Note(format!(
        "in {}: {}",
        p.display(),
        found.display()
    )));
    Ok(found)
}

fn inputs(a: &mut Args, out: &mut Out) -> io::Result<Inputs> {
    let given = a.positional("the update file")?;
    let programs_dir = match a.value("--programs")? {
        Some(p) => PathBuf::from(p),
        None => default_programs().ok_or_else(|| {
            usage("MPFree's programs for the MPC were not found next to this program; give --programs FOLDER")
        })?,
    };
    let kits = match a.value("--kits")? {
        Some(p) => std::fs::read(&p).map_err(|e| io::Error::new(e.kind(), format!("{p}: {e}")))?,
        None => mpfree_factory::drums::kits_file(),
    };
    let ssh_key = a.value("--ssh-key")?;
    let any_update = a.flag("--any-update");
    let update_path = update_path(&given, any_update, out)?;
    let work = a
        .value("--work")?
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    // Everything that needs no unpacking is checked first.
    let programs = Programs::from_dir(&programs_dir)?;
    out.say(Msg::Note(format!(
        "programs for the MPC: {}",
        programs_dir.display()
    )));
    let ssh = match ssh_key {
        Some(p) => {
            let text = std::fs::read_to_string(&p)
                .map_err(|e| io::Error::new(e.kind(), format!("{p}: {e}")))?;
            let (s, fingerprint) = ssh::for_stick(&text)?;
            out.say(Msg::Note(format!(
                "developer SSH on: root, key only, port 2222; the stick's host key is {fingerprint}"
            )));
            Some(s)
        }
        None => None,
    };
    let source = stick::open_update(&update_path, &work, any_update, &mut |m| out.say(m))?;
    Ok(Inputs {
        source,
        programs,
        kits,
        ssh,
        work,
    })
}

fn report(made: &stick::Made, out: &mut Out) {
    for n in &made.notes {
        out.say(Msg::Note(n.clone()));
    }
    for (i, p) in made.layout.table.parts.iter().enumerate() {
        if let Some(p) = p {
            out.say(Msg::Note(format!(
                "partition {}: {} at {} MiB, {} MiB",
                i + 1,
                p.name,
                (p.first * stick::SECTOR) >> 20,
                (p.sectors() * stick::SECTOR) >> 20
            )));
        }
    }
    out.say(Msg::Note(format!(
        "checked by reading back: SHA-256 {}",
        update::hex(&made.head_sha256)
    )));
}

fn inspect(disk: &Target, out: &mut Out) -> io::Result<()> {
    let table = gpt::read(disk)?;
    let r = recipe::Recipe::load();
    for (i, p) in table.parts.iter().enumerate() {
        if let Some(p) = p {
            out.line(&format!(
                "partition {}: {:<18} {:>8} MiB at {} MiB",
                i + 1,
                p.name,
                (p.sectors() * stick::SECTOR) >> 20,
                (p.first * stick::SECTOR) >> 20
            ));
        }
    }
    let Some((_, root)) = stick::root_part(&table) else {
        out.line("not an MPFree stick (no mpfree-test-root partition)");
        return Ok(());
    };
    let fs = Ext4::open(Offset {
        inner: disk,
        start: root.start_bytes(),
        len: root.len_bytes(),
    })?;
    match fs.lookup(stick::ABOUT_FILE)? {
        Some(i) => out.line(&format!(
            "made: {}",
            String::from_utf8_lossy(&fs.read_file(&i)?)
                .trim()
                .replace('\n', "; ")
        )),
        None => out.line("made: (no made-by.txt: not made by the creator)"),
    }
    if let Ok((_, mut img)) = stick::read_setup(disk) {
        let mode = stick::setup_read(&mut img, "mode.txt")?.unwrap_or_default();
        out.line(&format!(
            "setup mode: {}",
            String::from_utf8_lossy(&mode).trim()
        ));
        let log = stick::setup_read(&mut img, "setup-log.txt")?;
        out.line(&format!(
            "setup log: {}",
            if log.is_some() {
                "there (setup-log)"
            } else {
                "none yet"
            }
        ));
    }
    match table.by_name(&r.data_name) {
        Some((_, p)) => {
            let mut head = [0u8; 64];
            disk.read_at(&mut head, p.start_bytes())?;
            if head.starts_with(stick::MARK.as_bytes()) {
                out.line(
                    "partition 4: new; MPFree makes its filesystem at the first start on the MPC",
                );
            } else {
                match Ext4::open(Offset {
                    inner: disk,
                    start: p.start_bytes(),
                    len: p.len_bytes(),
                }) {
                    Ok(data) => {
                        out.line(&format!("partition 4: {} ext4", data.label));
                        let mut names: Vec<String> = match data.lookup("/backup")? {
                            Some(b) => data
                                .read_dir(&b)?
                                .into_iter()
                                .filter_map(|e| String::from_utf8(e.name).ok())
                                .filter(|n| n.starts_with("mpc-one-emmc-"))
                                .collect(),
                            None => Vec::new(),
                        };
                        names.sort();
                        if names.is_empty() {
                            out.line("backups: none yet");
                        }
                        for n in names {
                            out.line(&format!("backup: {n}"));
                        }
                    }
                    Err(e) => out.line(&format!("partition 4: not readable ({e})")),
                }
            }
        }
        None => out.line("partition 4: missing"),
    }
    Ok(())
}

/// Who owns the folder a backup is copied into, so that the copy belongs to
/// them and not to root: the copy runs with administrator rights (pkexec,
/// sudo, or the macOS password prompt), and the musician must be able to
/// move or delete it. Folders made on the way get the same owner.
struct Owner {
    #[cfg(unix)]
    ids: Option<(u32, u32)>,
    made: Vec<PathBuf>,
}

impl Owner {
    fn of(to: &Path) -> Owner {
        let mut made = Vec::new();
        let mut at = Some(to);
        while let Some(p) = at {
            if p.exists() {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::MetadataExt;
                    let ids = std::fs::metadata(p).ok().map(|m| (m.uid(), m.gid()));
                    return Owner { ids, made };
                }
                #[cfg(not(unix))]
                return Owner { made };
            }
            made.push(p.to_path_buf());
            at = p.parent();
        }
        Owner {
            #[cfg(unix)]
            ids: None,
            made,
        }
    }

    /// Gives the folders made on the way, the backup folder and its files to
    /// the owner, where they belong to someone else (only root can).
    fn give(&self, dest: &Path) -> io::Result<()> {
        #[cfg(unix)]
        if let Some((uid, gid)) = self.ids {
            use std::os::unix::fs::MetadataExt;
            let mut paths: Vec<PathBuf> = self.made.clone();
            paths.push(dest.to_path_buf());
            for e in std::fs::read_dir(dest)? {
                paths.push(e?.path());
            }
            for p in paths {
                let m = std::fs::symlink_metadata(&p)?;
                if (m.uid(), m.gid()) != (uid, gid) {
                    std::os::unix::fs::lchown(&p, Some(uid), Some(gid))?;
                }
            }
        }
        #[cfg(not(unix))]
        let _ = (dest, &self.made);
        Ok(())
    }
}

/// Every file the stick's systems saved on partition 2 (the log
/// partition), printed with MAC addresses and USB serial numbers hidden.
fn logs(disk: &Target, out: &mut Out) -> io::Result<()> {
    let table = gpt::read(disk)?;
    let (_, p) = table.by_name(stick::LOG_NAME).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "this is not an MPFree stick (no log partition)",
        )
    })?;
    let fs = Ext4::open(Offset {
        inner: disk,
        start: p.start_bytes(),
        len: p.len_bytes(),
    })?;
    let mut stack = vec![("".to_string(), fs.inode(mpfree_diskimage::ext4::ROOT)?)];
    let mut shown = 0u64;
    while let Some((path, dir)) = stack.pop() {
        let mut entries = fs.read_dir(&dir)?;
        entries.sort_by(|a, b| b.name.cmp(&a.name));
        for e in entries {
            let name = String::from_utf8_lossy(&e.name).into_owned();
            if name == "lost+found" && path.is_empty() {
                continue;
            }
            let ino = fs.inode(e.ino)?;
            let full = format!("{path}/{name}");
            match ino.kind() {
                Some(mpfree_diskimage::ext4::Kind::Dir) => stack.push((full, ino)),
                Some(mpfree_diskimage::ext4::Kind::File) => {
                    out.line(&format!("===== {full} ({} bytes)", ino.size));
                    if ino.size > 4 << 20 || shown > 64 << 20 {
                        out.line("(left out: too big to print)");
                        continue;
                    }
                    shown += ino.size;
                    let text = String::from_utf8_lossy(&fs.read_file(&ino)?).into_owned();
                    for l in text.lines() {
                        out.line(&redact(l));
                    }
                }
                _ => {}
            }
        }
    }
    Ok(())
}

/// A log line with MAC addresses and USB serial numbers replaced.
pub fn redact(line: &str) -> String {
    let mut s = line.to_string();
    for key in ["SerialNumber: ", "ID_SERIAL_SHORT=", "ID_SERIAL="] {
        if let Some(i) = s.find(key) {
            let start = i + key.len();
            let end = s[start..]
                .find(char::is_whitespace)
                .map_or(s.len(), |n| start + n);
            s.replace_range(start..end, "<SERIAL>");
        }
    }
    let b = s.as_bytes();
    let hex = |c: u8| c.is_ascii_hexdigit();
    let mut out = String::new();
    let mut i = 0;
    while i < b.len() {
        let mac = i + 17 <= b.len()
            && (0..6).all(|k| hex(b[i + 3 * k]) && hex(b[i + 3 * k + 1]))
            && (0..5).all(|k| b[i + 3 * k + 2] == b':')
            && (i == 0 || !hex(b[i - 1]))
            && (i + 17 == b.len() || !hex(b[i + 17]));
        if mac {
            out.push_str("<MAC>");
            i += 17;
        } else {
            let c = s[i..].chars().next().unwrap_or(' ');
            out.push(c);
            i += c.len_utf8();
        }
    }
    out
}

/// The status file the window reads. The window makes it, as the person
/// using the computer, in the temporary folder, and this runs as root
/// (pkexec): opening it with O_CREAT there is refused, even to root, where
/// `fs.protected_regular` is on (the default on Fedora and other systemd
/// systems), because root doesn't own it. So an existing file is opened
/// without O_CREAT, and only a missing one is made.
fn open_status(p: &Path) -> io::Result<File> {
    match std::fs::OpenOptions::new().append(true).open(p) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(p),
        r => r,
    }
}

fn main_inner(words: Vec<String>, out: &mut Out) -> io::Result<String> {
    let mut a = Args { words };
    if let Some(p) = a.value("--status")? {
        out.status = Some(
            open_status(Path::new(&p))
                .map_err(|e| io::Error::new(e.kind(), format!("the status file {p}: {e}")))?,
        );
    }
    let cmd = if a.words.is_empty() {
        "help".to_string()
    } else {
        a.words.remove(0)
    };
    match cmd.as_str() {
        "help" | "--help" | "-h" => {
            print!("{HELP}");
            Ok(String::new())
        }
        "--version" | "version" => Ok(CREATOR.to_string()),
        "check" => {
            let any = a.flag("--any-update");
            let path = update_path(&a.positional("the update file")?, any, out)?;
            out.say(Msg::Note(format!(
                "update file SHA-256: {}",
                update::hex(&update::file_sha256(&path)?)
            )));
            let work = a
                .value("--work")?
                .map(PathBuf::from)
                .unwrap_or_else(std::env::temp_dir);
            a.done()?;
            let src = stick::open_update(&path, &work, any, &mut |m| out.say(m))?;
            Ok(format!(
                "the update is good: Akai build {}, kernel {}, panel turned {} degrees",
                src.update.build, src.release, src.rotation
            ))
        }
        "disks" => {
            a.done()?;
            let list = disks::list()?;
            for d in &list {
                out.line(&format!(
                    "disk {}\t{}\t{}\t{}",
                    d.id,
                    d.bytes,
                    d.name.trim(),
                    if d.mounted.is_empty() {
                        "-".to_string()
                    } else {
                        d.mounted.join(", ")
                    }
                ));
            }
            Ok(format!("{} USB disk(s)", list.len()))
        }
        "build" => {
            let out_path = PathBuf::from(a.need("--out")?);
            let i = inputs(&mut a, out)?;
            a.done()?;
            let b = Build {
                source: &i.source,
                programs: &i.programs,
                kits: &i.kits,
                ssh: i.ssh.as_ref(),
                creator: CREATOR,
            };
            let made = stick::build_image(&b, &out_path, &mut |m| out.say(m))?;
            report(&made, out);
            Ok(format!("made {}", out_path.display()))
        }
        "write" => {
            let dev = a.need("--device")?;
            if dev == "mpfree" {
                return Err(usage(
                    "write needs the disk named (see mpfree-stick disks), not mpfree",
                ));
            }
            let yes = a.need("--yes-erase")?;
            if yes != dev {
                return Err(usage("--yes-erase must name the same disk as --device"));
            }
            let fresh = a.flag("--fresh-data");
            let keep = a.flag("--keep-data");
            if fresh && keep {
                return Err(usage("--fresh-data and --keep-data: choose one"));
            }
            let i = inputs(&mut a, out)?;
            a.done()?;
            let target = Target::open(&dev, true)?;
            // An update that must keep the backup never erases it: a partition 4
            // that holds a filesystem this version can't keep stops it here.
            if keep && stick::existing_data(&target).is_none() && stick::holds_data(&target) {
                return Err(io::Error::other(
                    "this stick's saved data (partition 4) is not in a form this version of MPFree \
                     can keep, so nothing was written. Copy the backup to this computer first, \
                     then make the stick again with \"Erase everything and start over\"",
                ));
            }
            out.say(Msg::Note(format!(
                "writing {dev} ({:.1} GB)",
                target.bytes() as f64 / 1e9
            )));
            let b = Build {
                source: &i.source,
                programs: &i.programs,
                kits: &i.kits,
                ssh: i.ssh.as_ref(),
                creator: CREATOR,
            };
            let made = stick::write_disk(&b, &target, target.bytes(), &i.work, fresh, &mut |m| {
                out.say(m)
            })?;
            report(&made, out);
            target.finish()?;
            Ok("the stick is ready: put it in the MPC and follow the creator's next steps".into())
        }
        "inspect" => {
            let dev = a.need("--device")?;
            a.done()?;
            let t = Target::open(&dev, false)?;
            inspect(&t, out)?;
            Ok(String::new())
        }
        "copy-backup" => {
            let dev = a.need("--device")?;
            let to = PathBuf::from(a.need("--to")?);
            a.done()?;
            let t = Target::open(&dev, false)?;
            let owner = Owner::of(&to);
            let dest = stick::copy_backup(&t, &to, &mut |m| out.say(m))?;
            if let Err(e) = owner.give(&dest) {
                out.say(Msg::Note(format!(
                    "the copy could not be given to the folder's owner ({e})"
                )));
            }
            Ok(format!("the backup is in {}", dest.display()))
        }
        "setup-mode" => {
            let dev = a.need("--device")?;
            let mode = a.positional("install or remove")?;
            a.done()?;
            let t = Target::open(&dev, true)?;
            stick::set_mode(&t, t.bytes(), &mode)?;
            t.finish()?;
            Ok(format!("the setup partition now says {mode}"))
        }
        "setup-log" => {
            let dev = a.need("--device")?;
            a.done()?;
            let t = Target::open(&dev, false)?;
            let (_, mut img) = stick::read_setup(&t)?;
            match stick::setup_read(&mut img, "setup-log.txt")? {
                Some(log) => {
                    print!("{}", String::from_utf8_lossy(&log));
                    Ok(String::new())
                }
                None => Ok("no setup log yet: autorun.sh has not run on the MPC".into()),
            }
        }
        "logs" => {
            let dev = a.need("--device")?;
            a.done()?;
            let t = Target::open(&dev, false)?;
            logs(&t, out)?;
            Ok(String::new())
        }
        "make-ssh-key" => {
            let dir = PathBuf::from(a.positional("the folder")?);
            let name = a
                .value("--name")?
                .unwrap_or_else(|| "mpfree_ed25519".into());
            a.done()?;
            let (k, p) = ssh::make_user_key(&dir, &name)?;
            Ok(format!("made {} and {}", k.display(), p.display()))
        }
        other => Err(usage(&format!("unknown command {other}"))),
    }
}

/// Runs one command; the exit status (0: done, 1: an error, said as `error ...`).
pub fn main(args: Vec<String>) -> i32 {
    let mut out = Out { status: None };
    match main_inner(args, &mut out) {
        Ok(msg) => {
            if !msg.is_empty() {
                out.line(&format!("done {msg}"));
            }
            0
        }
        Err(e) => {
            out.line(&format!("error {e}"));
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::redact;

    /// As root (pkexec or sudo), a backup copied into someone's folder is
    /// theirs, with the folders made on the way. Needs root to try.
    #[cfg(unix)]
    #[test]
    fn the_copy_belongs_to_the_folders_owner() {
        use std::os::unix::fs::MetadataExt;
        let base = std::env::temp_dir().join(format!("mpfree-owner-{}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();
        if std::os::unix::fs::chown(&base, Some(4321), Some(4321)).is_err() {
            std::fs::remove_dir_all(&base).unwrap();
            eprintln!("skipped: needs root");
            return;
        }
        let to = base.join("MPFree backups").join("more");
        let owner = super::Owner::of(&to);
        let dest = to.join("mpc-one-emmc-20261115T120000Z");
        std::fs::create_dir_all(&dest).unwrap();
        std::fs::write(dest.join("SHA256SUMS"), b"x").unwrap();
        owner.give(&dest).unwrap();
        for p in [
            base.join("MPFree backups"),
            to.clone(),
            dest.clone(),
            dest.join("SHA256SUMS"),
        ] {
            let m = std::fs::metadata(&p).unwrap();
            assert_eq!((m.uid(), m.gid()), (4321, 4321), "{}", p.display());
        }
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn redact_hides_mac_addresses_and_usb_serial_numbers() {
        assert_eq!(
            redact("eth0: link up, address 0a:1B:2c:3d:4e:5f, 100 Mbps"),
            "eth0: link up, address <MAC>, 100 Mbps"
        );
        assert_eq!(redact("0a:1b:2c:3d:4e:5f"), "<MAC>");
        assert_eq!(
            redact("usb 1-1: SerialNumber: 4C530001230915117325 more"),
            "usb 1-1: SerialNumber: <SERIAL> more"
        );
        assert_eq!(redact("ID_SERIAL=SanDisk_Ultra_1234"), "ID_SERIAL=<SERIAL>");
        // Times and longer hex runs are not addresses.
        assert_eq!(redact("at 12:34:56.789"), "at 12:34:56.789");
        assert_eq!(
            redact("x0a:1b:2c:3d:4e:5f0"),
            "x0a:1b:2c:3d:4e:5f0",
            "an address only stands alone"
        );
        assert_eq!(redact("päd 1 – ok"), "päd 1 – ok");
    }
}
