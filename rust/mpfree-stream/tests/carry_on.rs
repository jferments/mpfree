//! A take whose drive goes while it records carries on in a folder on
//! another drive (the SD card plan's B2c, docs/dev/design/recording-files.md):
//! what it committed stays where it was, everything after goes into new
//! files under the same name, which hold zeros where the committed frames
//! would be, and a hole record says what is missing and from where. Against
//! two model drives: the card pulled, the stick fine (or failing, then
//! fine), and a cut on the stick after every step, recovered.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use mpfree_stream::crc32c;
use mpfree_stream::format;
use mpfree_stream::fsio::Fs;
use mpfree_stream::rec::{
    checkpoint_path, final_path, hole_path, recording_path, stem, Hole, Meta, RecFormat, RecId,
    Take, TakeState, Why, HOLE_PATH_MAX,
};
use mpfree_stream::recover::{check_whole, recover, Found};
use mpfree_stream::testkit::{played, Cut, Damage, FakeClock, ModelStick, Names};
use mpfree_stream::writer::{Gate, TakeNo, Writer, WriterConfig};

const STEREO_FLOAT: RecFormat = RecFormat {
    float: true,
    channels: 2,
    rate: 48_000,
};
const MONO_24: RecFormat = RecFormat {
    float: false,
    channels: 1,
    rate: 44_100,
};
const CARD_ID: [u8; 16] = *b"card-marker-id-1";
const SERIAL: u32 = 0x1234_ABCD;

fn card() -> PathBuf {
    PathBuf::from("/card/MPFree/.work/audiotracks")
}

fn stick() -> PathBuf {
    PathBuf::from("/stick/library/.work/audiotracks")
}

fn take_id() -> RecId {
    RecId::parse("0123456789abcdef0123456789abcdef").unwrap()
}

fn name() -> String {
    stem("Take 3", &take_id())
}

/// Frames `from..from+n` of the reference audio, interleaved.
fn audio(fmt: RecFormat, from: u64, n: u64) -> Vec<f32> {
    let ch = usize::from(fmt.channels);
    (0..n as usize * ch)
        .map(|i| played(1, from + (i / ch) as u64, i % ch))
        .collect()
}

/// The bytes a take stores for `samples`.
fn stored(fmt: RecFormat, samples: &[f32]) -> Vec<u8> {
    let mut out = Vec::new();
    for &x in samples {
        if fmt.float {
            out.extend_from_slice(&x.to_le_bytes());
        } else {
            let v = (f64::from(x) * 8_388_608.0)
                .round()
                .clamp(-8_388_608.0, 8_388_607.0) as i32;
            out.extend_from_slice(&v.to_le_bytes()[..3]);
        }
    }
    out
}

/// The card and the stick: each path goes to the drive it is on.
#[derive(Clone, Debug, Default)]
struct Drives {
    card: ModelStick,
    stick: ModelStick,
}

impl Drives {
    fn on(&mut self, path: &Path) -> &mut ModelStick {
        if path.starts_with("/card") {
            &mut self.card
        } else {
            &mut self.stick
        }
    }
}

impl Fs for Drives {
    fn create(&mut self, path: &Path, new_only: bool) -> io::Result<()> {
        self.on(path).create(path, new_only)
    }
    fn write_at(&mut self, path: &Path, at: u64, bytes: &[u8]) -> io::Result<()> {
        self.on(path).write_at(path, at, bytes)
    }
    fn read_at(&mut self, path: &Path, at: u64, buf: &mut [u8]) -> io::Result<usize> {
        self.on(path).read_at(path, at, buf)
    }
    fn len(&mut self, path: &Path) -> io::Result<u64> {
        self.on(path).len(path)
    }
    fn set_len(&mut self, path: &Path, len: u64) -> io::Result<()> {
        self.on(path).set_len(path, len)
    }
    fn sync(&mut self, path: &Path) -> io::Result<()> {
        self.on(path).sync(path)
    }
    fn sync_dir(&mut self, dir: &Path) -> io::Result<()> {
        self.on(dir).sync_dir(dir)
    }
    fn rename(&mut self, from: &Path, to: &Path) -> io::Result<()> {
        self.on(from).rename(from, to)
    }
    fn remove(&mut self, path: &Path) -> io::Result<()> {
        self.on(path).remove(path)
    }
    fn list(&mut self, dir: &Path) -> io::Result<Vec<String>> {
        self.on(dir).list(dir)
    }
    fn exists(&mut self, path: &Path) -> bool {
        self.on(path).exists(path)
    }
}

/// [`Drives`] shared between a writer and the test.
#[derive(Clone, Default)]
struct Shared(Arc<Mutex<Drives>>);

impl Shared {
    fn with<R>(&self, f: impl FnOnce(&mut Drives) -> R) -> R {
        f(&mut self.0.lock().unwrap())
    }
}

impl Fs for Shared {
    fn create(&mut self, path: &Path, new_only: bool) -> io::Result<()> {
        self.with(|d| d.create(path, new_only))
    }
    fn write_at(&mut self, path: &Path, at: u64, bytes: &[u8]) -> io::Result<()> {
        self.with(|d| d.write_at(path, at, bytes))
    }
    fn read_at(&mut self, path: &Path, at: u64, buf: &mut [u8]) -> io::Result<usize> {
        self.with(|d| d.read_at(path, at, buf))
    }
    fn len(&mut self, path: &Path) -> io::Result<u64> {
        self.with(|d| d.len(path))
    }
    fn set_len(&mut self, path: &Path, len: u64) -> io::Result<()> {
        self.with(|d| d.set_len(path, len))
    }
    fn sync(&mut self, path: &Path) -> io::Result<()> {
        self.with(|d| d.sync(path))
    }
    fn sync_dir(&mut self, dir: &Path) -> io::Result<()> {
        self.with(|d| d.sync_dir(dir))
    }
    fn rename(&mut self, from: &Path, to: &Path) -> io::Result<()> {
        self.with(|d| d.rename(from, to))
    }
    fn remove(&mut self, path: &Path) -> io::Result<()> {
        self.with(|d| d.remove(path))
    }
    fn list(&mut self, dir: &Path) -> io::Result<Vec<String>> {
        self.with(|d| d.list(dir))
    }
    fn exists(&mut self, path: &Path) -> bool {
        self.with(|d| d.exists(path))
    }
}

/// The audio of a WAV, as mpfree-stream's reader sees it.
fn read_wav(fs: &mut dyn Fs, path: &Path) -> (format::Layout, Vec<f32>) {
    let len = fs.len(path).unwrap() as usize;
    let mut bytes = vec![0u8; len];
    assert_eq!(fs.read_at(path, 0, &mut bytes).unwrap(), len);
    let l = format::layout(&mut io::Cursor::new(&bytes)).expect("readable WAV");
    let from = l.data_at as usize;
    let mut out = vec![0.0f32; l.frames as usize * usize::from(l.channels)];
    format::decode(
        l.sample,
        l.endian,
        &bytes[from..from + l.frames as usize * l.block_align()],
        &mut out,
    );
    (l, out)
}

fn read_hole(fs: &mut dyn Fs) -> Hole {
    let mut b = vec![0u8; 4096];
    let p = hole_path(&stick(), &name());
    assert_eq!(fs.read_at(&p, 0, &mut b).unwrap(), 4096);
    Hole::decode(&b).expect("a hole record")
}

/// Frames in one of a take's write units.
fn unit(fmt: RecFormat) -> u64 {
    (fmt.unit_bytes() / fmt.block_align()) as u64
}

/// Arm on the card, record `before` frames there with a checkpoint after
/// each write unit, then pull the card and record a unit more: the take
/// is at risk, its memory holding what wasn't committed.
fn pulled_mid_take(d: &mut Drives, fmt: RecFormat, before: u64) -> Take {
    let mut t = Take::arm(
        d,
        &card(),
        "Take 3",
        take_id(),
        fmt,
        Meta::default(),
        1 << 22,
    )
    .unwrap();
    let mut at = 0;
    while at < before {
        let n = 1024.min(before - at);
        assert_eq!(t.push(&audio(fmt, at, n)), n as usize);
        at += n;
        t.write_ready(d);
        if at % unit(fmt) < 1024 {
            t.checkpoint(d);
        }
    }
    d.card.pulled = true;
    let lost = unit(fmt);
    assert_eq!(t.push(&audio(fmt, at, lost)), lost as usize);
    t.write_ready(d);
    t.checkpoint(d);
    assert!(
        matches!(t.state(), TakeState::AtRisk(Why::Gone)),
        "{:?}",
        t.state()
    );
    t
}

#[test]
fn a_take_whose_card_goes_carries_on_on_the_stick_with_the_card_part_as_a_hole() {
    for fmt in [STEREO_FLOAT, MONO_24] {
        let mut d = Drives::default();
        let mut t = pulled_mid_take(&mut d, fmt, 3 * unit(fmt) + 500);
        let committed = t.committed();
        assert!(committed > 0 && committed < t.captured(), "{committed}");
        let card_files = d.card.names();
        assert!(t.carry_on(&mut d, &stick(), CARD_ID, SERIAL).unwrap());
        assert_eq!(t.state(), TakeState::Recording, "no longer at risk");
        assert_eq!(t.hole(), committed);
        assert_eq!(t.dir(), stick());
        assert_eq!(t.committed(), committed, "what the card holds still counts");
        assert_eq!(d.card.names(), card_files, "nothing on the card is touched");

        // The hole record: what the card holds of the take, and its check.
        let h = read_hole(&mut d);
        let want = stored(fmt, &audio(fmt, 0, committed));
        assert_eq!(
            h,
            Hole {
                id: take_id(),
                frames: committed,
                fmt,
                crc: crc32c::of(&want),
                card: CARD_ID,
                serial: SERIAL,
                from: recording_path(&card(), &name()),
            }
        );
        assert_eq!(h.bytes(), want.len() as u64);

        // Recording goes on, then the take is finished on the stick.
        let mut at = t.captured();
        for _ in 0..12 {
            assert_eq!(t.push(&audio(fmt, at, 1024)), 1024);
            at += 1024;
            t.write_ready(&mut d);
        }
        t.checkpoint(&mut d);
        assert!(t.committed() > committed);
        t.stop();
        assert!(t.finish(&mut d));
        assert_eq!(t.state(), TakeState::Saved);
        assert_eq!(t.final_path(), final_path(&stick(), &name()));
        assert!(!d.stick.exists(&checkpoint_path(&stick(), &name())));
        assert!(
            d.stick.exists(&hole_path(&stick(), &name())),
            "the hole record stays until the card's part is back"
        );

        // The whole take's length, zeros for the card's part, then every
        // frame after it exactly.
        let (l, samples) = read_wav(&mut d, &t.final_path());
        assert_eq!(l.frames, at, "{fmt:?}");
        let ch = usize::from(fmt.channels);
        let split = committed as usize * ch;
        assert!(samples[..split].iter().all(|&x| x == 0.0));
        let rest = audio(fmt, committed, at - committed);
        if fmt.float {
            assert!(samples[split..] == rest[..]);
        } else {
            let worst = samples[split..]
                .iter()
                .zip(&rest)
                .map(|(a, b)| (a - b).abs())
                .fold(0.0, f32::max);
            assert!(worst < 2.0 / 8_388_608.0, "{worst}");
        }
    }
}

#[test]
fn a_cut_on_the_stick_after_every_step_recovers_a_take_that_checks_whole() {
    // Count the stick's operations in a run with no cut.
    let run = |d: &mut Drives| -> Option<Take> {
        let mut t = pulled_mid_take(d, STEREO_FLOAT, 2 * 8192 + 100);
        t.carry_on(d, &stick(), CARD_ID, SERIAL).ok()?;
        let mut at = t.captured();
        for i in 0..20 {
            t.push(&audio(STEREO_FLOAT, at, 1024));
            at += 1024;
            t.write_ready(d);
            if i % 6 == 5 {
                t.checkpoint(d);
            }
        }
        t.stop();
        for _ in 0..3 {
            if t.finish(d) {
                break;
            }
        }
        Some(t)
    };
    let mut whole = Drives::default();
    run(&mut whole).unwrap();
    let ops = whole.stick.ops;
    assert!(ops > 10, "{ops}");
    let s = name();
    let mut proved = 0;
    for cut_after in 0..=ops {
        for (how, names) in [
            (Cut::Lost, Names::Landed),
            (Cut::Landed, Names::Landed),
            (Cut::Torn(3), Names::LastOnly),
            (Cut::Lost, Names::Lost),
        ] {
            let mut d = Drives::default();
            d.stick.cut_after = Some(cut_after);
            let t = run(&mut d);
            let what = format!("cut after {cut_after} ({how:?}, {names:?})");
            let mut after = d.stick.cut(how, names, Damage::default());
            let found = recover(&mut after, &stick()).unwrap();
            let rec_left = after.exists(&recording_path(&stick(), &s));
            assert!(!rec_left, "{what}: recovery leaves no recording: {found:?}");
            for f in &found {
                match f {
                    Found::Finished { path, record, .. }
                    | Found::CutOff { path, record, .. }
                    | Found::Unchecked { path, record, .. } => {
                        assert!(
                            check_whole(&mut after, path, record).unwrap(),
                            "{what}: the zeros are part of what its checks cover"
                        );
                        proved += 1;
                        let (l, samples) = read_wav(&mut after, path);
                        assert_eq!(l.frames, record.frames, "{what}");
                        let hole = 2 * 8192u64;
                        assert!(record.frames >= hole, "{what}");
                        let split = (hole * 2) as usize;
                        assert!(samples[..split].iter().all(|&x| x == 0.0), "{what}");
                        assert!(
                            samples[split..] == audio(STEREO_FLOAT, hole, record.frames - hole)[..],
                            "{what}: the stick's part exactly"
                        );
                    }
                    // Audio written past its last record, before the
                    // first one here: kept apart, as for any take.
                    Found::Unverified { path, .. } => {
                        assert!(after.exists(path), "{what}: kept, never removed");
                    }
                    Found::Removed { .. } => {}
                    other => panic!("{what}: {other:?}"),
                }
            }
            if let Some(t) = t {
                if t.state() == TakeState::Saved {
                    assert!(
                        after.exists(&final_path(&stick(), &s)),
                        "{what}: a finished take stays finished"
                    );
                }
            }
            // The hole record, once made, is never removed by recovery.
            if whole.stick.exists(&hole_path(&stick(), &s))
                && after.exists(&hole_path(&stick(), &s))
            {
                let mut b = vec![0u8; 4096];
                after.read_at(&hole_path(&stick(), &s), 0, &mut b).unwrap();
                if let Some(h) = Hole::decode(&b) {
                    assert_eq!(h.frames, 2 * 8192, "{what}");
                }
            }
        }
    }
    assert!(
        proved > ops as usize,
        "most cuts leave a take that checks: {proved}"
    );
}

#[test]
fn a_carry_on_that_fails_leaves_nothing_and_is_tried_again() {
    let mut whole = Drives::default();
    let mut t = pulled_mid_take(&mut whole, STEREO_FLOAT, 8192);
    let before = whole.stick.ops;
    t.carry_on(&mut whole, &stick(), CARD_ID, SERIAL).unwrap();
    let ops = whole.stick.ops - before;
    assert_eq!(ops, 10, "hole: create, write, sync; then as at arming");
    for op in 1..=ops {
        let mut d = Drives::default();
        let mut t = pulled_mid_take(&mut d, STEREO_FLOAT, 8192);
        let base = d.stick.ops;
        d.stick.fail = [base + op].into();
        assert!(
            t.carry_on(&mut d, &stick(), CARD_ID, SERIAL).is_err(),
            "op {op}"
        );
        assert!(d.stick.names().is_empty(), "op {op}: {:?}", d.stick.names());
        assert!(matches!(t.state(), TakeState::AtRisk(Why::Gone)), "op {op}");
        assert_eq!(t.dir(), card());
        assert_eq!(t.hole(), 0);
        // Then it works.
        assert!(
            t.carry_on(&mut d, &stick(), CARD_ID, SERIAL).unwrap(),
            "op {op}"
        );
        t.stop();
        assert!(t.finish(&mut d), "op {op}");
        let (l, _) = read_wav(&mut d, &t.final_path());
        assert_eq!(l.frames, 2 * 8192);
    }
}

#[test]
fn nothing_committed_on_the_card_carries_on_whole_with_no_hole() {
    let mut d = Drives::default();
    let mut t = Take::arm(
        &mut d,
        &card(),
        "Take 3",
        take_id(),
        STEREO_FLOAT,
        Meta::default(),
        1 << 22,
    )
    .unwrap();
    d.card.pulled = true;
    t.push(&audio(STEREO_FLOAT, 0, 3000));
    t.write_ready(&mut d);
    assert!(t.carry_on(&mut d, &stick(), CARD_ID, SERIAL).unwrap());
    assert_eq!(t.hole(), 0);
    assert!(
        !d.stick.exists(&hole_path(&stick(), &name())),
        "nothing is missing"
    );
    t.stop();
    assert!(t.finish(&mut d));
    let (l, samples) = read_wav(&mut d, &t.final_path());
    assert_eq!(l.frames, 3000);
    assert!(samples == audio(STEREO_FLOAT, 0, 3000));
}

#[test]
fn a_take_stopped_with_nothing_left_to_write_still_reaches_its_length() {
    // Everything captured was committed on the card, then it went.
    let mut d = Drives::default();
    let mut t = Take::arm(
        &mut d,
        &card(),
        "Take 3",
        take_id(),
        STEREO_FLOAT,
        Meta::default(),
        1 << 22,
    )
    .unwrap();
    t.push(&audio(STEREO_FLOAT, 0, 8192));
    t.write_ready(&mut d);
    t.checkpoint(&mut d);
    assert_eq!(t.committed(), 8192);
    d.card.pulled = true;
    t.stop();
    assert!(!t.finish(&mut d));
    assert!(t.carry_on(&mut d, &stick(), CARD_ID, SERIAL).unwrap());
    assert!(t.finish(&mut d));
    let (l, samples) = read_wav(&mut d, &t.final_path());
    assert_eq!(l.frames, 8192);
    assert!(samples.iter().all(|&x| x == 0.0));
}

#[test]
fn the_hole_record_round_trips_and_damage_is_refused() {
    let h = Hole {
        id: take_id(),
        frames: 123_456,
        fmt: MONO_24,
        crc: 0xDEAD_BEEF,
        card: CARD_ID,
        serial: SERIAL,
        from: PathBuf::from("/run/mpfree-sd/MPFree/.work/audiotracks/Vocal ü_0123.wav.recording"),
    };
    let b = h.encode().unwrap();
    assert_eq!(b.len(), 4096);
    assert_eq!(Hole::decode(&b), Some(h.clone()));
    for at in [0, 9, 20, 33, 49, 61, 70, 82, 85, 90, 140, 4095] {
        let mut bad = b.clone();
        bad[at] ^= 0x10;
        assert_eq!(Hole::decode(&bad), None, "byte {at}");
    }
    assert_eq!(Hole::decode(&b[..4000]), None);
    let long = Hole {
        from: PathBuf::from("x".repeat(HOLE_PATH_MAX + 1)),
        ..h.clone()
    };
    assert!(long.encode().is_none());
    let longest = Hole {
        from: PathBuf::from("x".repeat(HOLE_PATH_MAX)),
        ..h
    };
    assert_eq!(Hole::decode(&longest.encode().unwrap()), Some(longest));
}

#[test]
fn the_writer_carries_a_take_on_tries_again_until_it_can_and_finishes_it_there() {
    let drives = Shared::default();
    let clock = FakeClock::new();
    let mut w = Writer::new(
        Box::new(drives.clone()),
        clock.clone(),
        Arc::new(Gate::new()),
        WriterConfig::default(),
    );
    let no = TakeNo(1);
    w.arm(
        no,
        &card(),
        "Take 3",
        take_id(),
        STEREO_FLOAT,
        Meta::default(),
    )
    .unwrap();
    let mut pushed = 0u64;
    let step = |w: &mut Writer, s: f64, pushed: &mut u64| {
        clock.advance(s);
        let want = (clock.now_s() * 48_000.0) as u64;
        *pushed += w.push(no, &audio(STEREO_FLOAT, *pushed, want - *pushed)) as u64;
        for _ in 0..64 {
            if w.tick() > 0.0 {
                break;
            }
        }
    };
    for _ in 0..10 {
        step(&mut w, 0.5, &mut pushed);
    }
    let committed = w.view(no).unwrap().committed;
    assert!(committed > 0);
    // The card goes, and the stick is full for a while.
    drives.with(|d| {
        d.card.pulled = true;
        d.stick.capacity = Some(0);
    });
    step(&mut w, 0.5, &mut pushed);
    assert!(w.carry_on(no, &stick(), CARD_ID, SERIAL));
    step(&mut w, 0.5, &mut pushed);
    let v = w.view(no).unwrap();
    assert!(v.carrying, "{v:?}");
    assert_eq!(v.hole, 0);
    assert!(matches!(v.state, TakeState::AtRisk(Why::Gone)), "{v:?}");
    assert!(w.stats().carry_failed >= 2);
    assert_eq!(w.final_path(no), Some(final_path(&card(), &name())));
    // Room again: it carries on at the next pass, and nothing waited for
    // is lost.
    drives.with(|d| d.stick.capacity = None);
    step(&mut w, 0.5, &mut pushed);
    let v = w.view(no).unwrap();
    assert!(!v.carrying);
    assert_eq!(v.hole, committed);
    assert_eq!(v.state, TakeState::Recording);
    assert_eq!(w.final_path(no), Some(final_path(&stick(), &name())));
    for _ in 0..6 {
        step(&mut w, 0.5, &mut pushed);
    }
    assert!(w.view(no).unwrap().committed > committed);
    w.stop(no);
    for _ in 0..4 {
        step(&mut w, 0.5, &mut pushed);
    }
    let v = w.view(no).unwrap();
    assert_eq!(v.state, TakeState::Saved);
    let path = w.final_path(no).unwrap();
    let mut d = drives.0.lock().unwrap();
    let (l, samples) = read_wav(&mut *d, &path);
    assert_eq!(l.frames, v.captured);
    let split = committed as usize * 2;
    assert!(samples[..split].iter().all(|&x| x == 0.0));
    assert!(samples[split..] == audio(STEREO_FLOAT, committed, v.captured - committed)[..]);
    assert!(
        !w.carry_on(TakeNo(9), &stick(), CARD_ID, SERIAL),
        "no such take"
    );
}
