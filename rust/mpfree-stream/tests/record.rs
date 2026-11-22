//! The recording file protocol against a model stick
//! (docs/dev/design/recording-files.md): a cut after every operation,
//! three ways and with extra damage, then recovery; damaged headers,
//! damage the fast check can't see, nothing provable; faults; and round
//! trips through the real file system and mpfree-stream's reader.

use std::io;
use std::path::{Path, PathBuf};

use mpfree_stream::format::{self, Container};
use mpfree_stream::fsio::{Fs, StdFs};
use mpfree_stream::rec::{
    checkpoint_path, final_path, header, recording_path, stem, tail_path, Meta, RecFormat, RecId,
    Record, Sizes, Slot, Take, TakeState, Why, DATA_AT, RIFF_MAX, SLOT,
};
use mpfree_stream::recover::{check_whole, checked, recover, Found};
use mpfree_stream::testkit::{folder, played, Cut, Damage, ModelStick, Names};

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

fn media() -> PathBuf {
    PathBuf::from("/stick/media")
}

fn take_id() -> RecId {
    RecId::parse("0123456789abcdef0123456789abcdef").unwrap()
}

/// Frames `from..from+n` of the reference audio, interleaved.
fn audio(fmt: RecFormat, from: u64, n: u64) -> Vec<f32> {
    let ch = usize::from(fmt.channels);
    (0..n as usize * ch)
        .map(|i| played(1, from + (i / ch) as u64, i % ch))
        .collect()
}

/// What a recording run reported before it ended (or the power went).
#[derive(Clone, Copy, Debug)]
struct Run {
    committed: u64,
    captured: u64,
    saved: bool,
}

/// Record `frames` frames: units written as they fill, a checkpoint
/// round every 6000 frames, then stop and finish.
fn scenario(fs: &mut dyn Fs, fmt: RecFormat, frames: u64) -> Run {
    let Ok(mut t) = Take::arm(
        fs,
        &media(),
        "Take 3",
        take_id(),
        fmt,
        Meta::default(),
        1 << 22,
    ) else {
        return Run {
            committed: 0,
            captured: 0,
            saved: false,
        };
    };
    let mut at = 0;
    while at < frames {
        let n = 1000.min(frames - at);
        assert_eq!(t.push(&audio(fmt, at, n)), n as usize);
        at += n;
        t.write_ready(fs);
        if at % 6000 == 0 {
            t.checkpoint(fs);
        }
    }
    t.stop();
    for _ in 0..3 {
        if t.finish(fs) {
            break;
        }
    }
    Run {
        committed: t.committed(),
        captured: t.captured(),
        saved: t.state() == TakeState::Saved,
    }
}

/// The audio of a WAV on the stick, as mpfree-stream's reader sees it.
fn read_wav(fs: &mut dyn Fs, path: &Path) -> (format::Layout, Vec<f32>) {
    let len = fs.len(path).unwrap() as usize;
    let mut bytes = vec![0u8; len];
    assert_eq!(fs.read_at(path, 0, &mut bytes).unwrap(), len);
    let l = format::layout(&mut io::Cursor::new(&bytes)).expect("readable WAV");
    let align = l.block_align();
    let from = l.data_at as usize;
    let mut out = vec![0.0f32; l.frames as usize * usize::from(l.channels)];
    format::decode(
        l.sample,
        l.endian,
        &bytes[from..from + l.frames as usize * align],
        &mut out,
    );
    (l, out)
}

fn exact(fmt: RecFormat, samples: &[f32], first: u64) -> bool {
    let ch = usize::from(fmt.channels);
    samples
        .iter()
        .enumerate()
        .all(|(i, &x)| x == played(1, first + (i / ch) as u64, i % ch))
}

/// Check what recovery made of a cut against what the run reported.
/// Returns the frames recovered.
fn verify(after: &mut ModelStick, run: Run, fmt: RecFormat, damage: Damage, what: &str) -> u64 {
    let s = stem("Take 3", &take_id());
    let rec = recording_path(&media(), &s);
    let before_len = after.exists(&rec).then(|| after.len(&rec).unwrap());
    let found = recover(after, &media()).unwrap();
    let extra = damage.next_block || damage.other_slot;
    assert!(found.len() <= 1, "{what}: {found:?}");
    let fin = final_path(&media(), &s);
    let check_file = |after: &mut ModelStick, path: &Path, r: Option<&Record>| -> u64 {
        let (l, samples) = read_wav(after, path);
        let frames = l.frames;
        // Damage to a synced record can take recovery back to the older
        // one; the audio after it is then kept as the tail (checked below).
        assert!(
            extra || frames >= run.committed,
            "{what}: recovered {frames} < committed {}",
            run.committed
        );
        assert!(
            frames <= run.captured,
            "{what}: recovered more than captured"
        );
        if run.saved {
            assert_eq!(frames, run.captured, "{what}: a Saved take came back short");
        }
        let whole_ok = r.is_none_or(|r| check_whole(after, path, r).unwrap());
        let good = exact(fmt, &samples, 0);
        if extra {
            assert!(
                good || !whole_ok,
                "{what}: damaged audio passed the whole-take check"
            );
        } else {
            assert!(
                good,
                "{what}: recovered audio isn't the first {frames} frames recorded"
            );
            assert!(
                whole_ok,
                "{what}: the whole-take check failed on good audio"
            );
        }
        frames
    };
    match found.as_slice() {
        [] => {
            if after.exists(&fin) {
                check_file(after, &fin, None)
            } else {
                assert_eq!(run.committed, 0, "{what}: committed audio vanished");
                0
            }
        }
        [Found::Finished {
            path, record, tail, ..
        }]
        | [Found::CutOff {
            path, record, tail, ..
        }] => {
            let frames = check_file(after, path, Some(record));
            assert_eq!(frames, record.frames);
            if let Some(t) = tail {
                let (l, samples) = read_wav(after, t);
                assert!(l.frames > 0);
                if !extra {
                    assert!(
                        exact(fmt, &samples, frames),
                        "{what}: the tail isn't the frames after the take"
                    );
                }
            }
            if let Some(len) = before_len {
                let kept = DATA_AT
                    + frames * fmt.block_align() as u64
                    + tail.as_ref().map_or(0, |t| after.len(t).unwrap() - DATA_AT);
                assert!(
                    kept + fmt.block_align() as u64 + 1 > len,
                    "{what}: audio was thrown away ({kept} kept of {len})"
                );
            }
            checked(after, &media(), &s).unwrap();
            assert!(
                recover(after, &media()).unwrap().is_empty(),
                "{what}: second recovery found more"
            );
            frames
        }
        [Found::Unchecked { path, record, .. }] => {
            let frames = check_file(after, path, Some(record));
            checked(after, &media(), &s).unwrap();
            frames
        }
        [Found::Unverified { path, .. }] => {
            assert!(
                run.committed == 0 || extra,
                "{what}: committed audio made unverified"
            );
            assert_eq!(
                Some(after.len(path).unwrap()),
                before_len,
                "{what}: unverified audio cut"
            );
            assert!(!after.exists(&checkpoint_path(&media(), &s)));
            0
        }
        [Found::Removed { .. }] => {
            assert_eq!(run.committed, 0, "{what}: committed audio removed");
            assert!(
                before_len.is_none_or(|l| l <= DATA_AT),
                "{what}: audio removed"
            );
            0
        }
        other => panic!("{what}: {other:?}"),
    }
}

/// A cut after every operation, every way, with failures mixed in.
fn every_cut(fmt: RecFormat, frames: u64, fail: &[u64], damages: &[Damage]) {
    let mut dry = ModelStick::new();
    dry.fail = fail.iter().copied().collect();
    let full = scenario(&mut dry, fmt, frames);
    assert!(full.saved, "the run without a cut must finish: {full:?}");
    let total = dry.ops;
    let mut cases = 0;
    for n in 0..=total {
        let mut stick = ModelStick::new();
        stick.fail = dry.fail.clone();
        stick.cut_after = Some(n);
        let run = scenario(&mut stick, fmt, frames);
        for how in [
            Cut::Lost,
            Cut::Landed,
            Cut::Torn(0),
            Cut::Torn(1),
            Cut::Torn(64),
        ] {
            for names in [Names::Lost, Names::Landed, Names::LastOnly] {
                for &d in damages {
                    let mut after = stick.cut(how, names, d);
                    verify(
                        &mut after,
                        run,
                        fmt,
                        d,
                        &format!("cut after op {n}, {how:?}, names {names:?}, {d:?}"),
                    );
                    cases += 1;
                }
            }
        }
    }
    assert!(cases > 100);
}

const ALL_DAMAGE: [Damage; 4] = [
    Damage {
        next_block: false,
        other_slot: false,
        header: false,
    },
    Damage {
        next_block: true,
        other_slot: false,
        header: false,
    },
    Damage {
        next_block: false,
        other_slot: true,
        header: false,
    },
    Damage {
        next_block: false,
        other_slot: false,
        header: true,
    },
];

#[test]
fn a_cut_after_every_step_stereo_float() {
    every_cut(STEREO_FLOAT, 8192 * 3 + 3000, &[], &ALL_DAMAGE);
}

#[test]
fn a_cut_after_every_step_with_failures_mixed_in() {
    // Failures land in unit writes, a round's audio sync, a record and
    // the stop's steps (arm is ops 1 to 7).
    every_cut(
        STEREO_FLOAT,
        8192 * 3 + 3000,
        &[8, 11, 13, 25],
        &ALL_DAMAGE[..2],
    );
}

#[test]
fn a_cut_after_every_step_mono_24_bit_with_a_pad_byte() {
    every_cut(MONO_24, 24_576 * 2 + 1001, &[], &ALL_DAMAGE);
}

/// Record and commit `units` units, write one more without committing it,
/// and come back from a cut in which everything landed.
fn cut_take(units: u64) -> (ModelStick, Run) {
    let mut stick = ModelStick::new();
    let mut t = Take::arm(
        &mut stick,
        &media(),
        "Take 3",
        take_id(),
        STEREO_FLOAT,
        Meta::default(),
        1 << 22,
    )
    .unwrap();
    t.push(&audio(STEREO_FLOAT, 0, units * 8192));
    t.write_ready(&mut stick);
    t.checkpoint(&mut stick);
    t.push(&audio(STEREO_FLOAT, units * 8192, 8192 + 500));
    t.write_ready(&mut stick);
    let run = Run {
        committed: t.committed(),
        captured: t.captured(),
        saved: false,
    };
    assert!(run.committed > 0);
    (
        stick.cut(Cut::Landed, Names::Landed, Damage::default()),
        run,
    )
}

#[test]
fn a_damaged_header_recovers_as_a_good_one() {
    let s = stem("Take 3", &take_id());
    let rec = recording_path(&media(), &s);
    for how in ["zeros", "noise"] {
        let (mut stick, run) = cut_take(5);
        let mut b = stick.bytes(&rec).unwrap().to_vec();
        for (i, v) in b[..4096].iter_mut().enumerate() {
            *v = if how == "zeros" {
                0
            } else {
                (i * 151 + 7) as u8
            };
        }
        stick.put(&rec, &b);
        let found = recover(&mut stick, &media()).unwrap();
        let [Found::CutOff {
            path, record, tail, ..
        }] = found.as_slice()
        else {
            panic!("{how}: {found:?}")
        };
        assert_eq!(record.frames, run.committed);
        let (_, samples) = read_wav(&mut stick, path);
        assert!(exact(STEREO_FLOAT, &samples, 0), "{how}");
        let (_, t) = read_wav(&mut stick, tail.as_ref().expect("the uncommitted frames"));
        assert!(exact(STEREO_FLOAT, &t, run.committed), "{how}");
        assert!(check_whole(&mut stick, path, record).unwrap());
    }
}

#[test]
fn a_clean_header_naming_another_take_is_kept_unverified() {
    let s = stem("Take 3", &take_id());
    let rec = recording_path(&media(), &s);
    let (mut stick, _) = cut_take(3);
    let mut b = stick.bytes(&rec).unwrap().to_vec();
    let other = RecId([9; 16]);
    b[..4096].copy_from_slice(&header(STEREO_FLOAT, &other, Sizes::Open));
    stick.put(&rec, &b);
    let len = b.len() as u64;
    let found = recover(&mut stick, &media()).unwrap();
    let [Found::Unverified { path, .. }] = found.as_slice() else {
        panic!("{found:?}")
    };
    assert!(path.to_string_lossy().ends_with(".unverified.wav"));
    assert_eq!(stick.len(path).unwrap(), len, "nothing cut away");
    let (l, samples) = read_wav(&mut stick, path);
    assert_eq!(l.frames, 8192 * 4);
    assert!(exact(STEREO_FLOAT, &samples, 0));
}

#[test]
fn damage_the_fast_check_cant_see_is_caught_by_the_whole_take_check() {
    let s = stem("Take 3", &take_id());
    let rec = recording_path(&media(), &s);
    let (mut stick, run) = cut_take(4);
    let mut b = stick.bytes(&rec).unwrap().to_vec();
    b[DATA_AT as usize + 1000] ^= 0x40;
    stick.put(&rec, &b);
    let found = recover(&mut stick, &media()).unwrap();
    let [Found::CutOff { path, record, .. }] = found.as_slice() else {
        panic!("{found:?}")
    };
    assert_eq!(
        record.frames, run.committed,
        "finished at start by the fast check"
    );
    assert!(
        !check_whole(&mut stick, path, record).unwrap(),
        "marked Damaged"
    );
    assert!(stick.exists(path), "kept");
}

#[test]
fn nothing_provable_is_kept_and_never_removed() {
    let s = stem("Take 3", &take_id());
    let rec = recording_path(&media(), &s);
    let ck = checkpoint_path(&media(), &s);
    // Both slots spoiled.
    let (mut stick, _) = cut_take(3);
    let len = stick.len(&rec).unwrap();
    stick.put(&ck, &vec![0x5A; 2 * SLOT]);
    let found = recover(&mut stick, &media()).unwrap();
    let [Found::Unverified { path, .. }] = found.as_slice() else {
        panic!("{found:?}")
    };
    assert!(
        path.to_string_lossy().ends_with(".unverified.wav"),
        "format from the header"
    );
    assert_eq!(stick.len(path).unwrap(), len);
    // Both slots and the header spoiled: no format anywhere.
    let (mut stick, _) = cut_take(3);
    stick.put(&ck, &vec![0x5A; 2 * SLOT]);
    let mut b = stick.bytes(&rec).unwrap().to_vec();
    b[..4096].fill(0);
    stick.put(&rec, &b);
    let found = recover(&mut stick, &media()).unwrap();
    let [Found::Unverified { path, .. }] = found.as_slice() else {
        panic!("{found:?}")
    };
    assert!(path.to_string_lossy().ends_with(".unverified.raw"));
    assert_eq!(stick.bytes(path).unwrap(), &b[..], "the bytes untouched");
    // Nothing committed, but audio written.
    let mut stick = ModelStick::new();
    let mut t = Take::arm(
        &mut stick,
        &media(),
        "Take 3",
        take_id(),
        STEREO_FLOAT,
        Meta::default(),
        1 << 22,
    )
    .unwrap();
    t.push(&audio(STEREO_FLOAT, 0, 8192 * 2));
    t.write_ready(&mut stick);
    let mut after = stick.cut(Cut::Landed, Names::Landed, Damage::default());
    let found = recover(&mut after, &media()).unwrap();
    let [Found::Unverified { path, .. }] = found.as_slice() else {
        panic!("{found:?}")
    };
    let (l, samples) = read_wav(&mut after, path);
    assert_eq!(l.frames, 8192 * 2);
    assert!(exact(STEREO_FLOAT, &samples, 0));
}

#[test]
fn files_from_a_newer_mpfree_and_strangers_are_left_alone() {
    let s = stem("Take 3", &take_id());
    let (mut stick, _) = cut_take(2);
    let ck = checkpoint_path(&media(), &s);
    let mut b = stick.bytes(&ck).unwrap().to_vec();
    b[8] = 2; // version 2, both slots
    b[SLOT + 8] = 2;
    stick.put(&ck, &b);
    let stranger = media().join("notes.wav.recording");
    stick.put(&stranger, b"not ours");
    let names = stick.names();
    let found = recover(&mut stick, &media()).unwrap();
    assert!(
        matches!(found.as_slice(), [Found::Newer { .. }]),
        "{found:?}"
    );
    assert_eq!(stick.names(), names, "nothing renamed or removed");
}

#[test]
fn the_first_record_is_sequence_0_in_slot_0_and_the_first_checkpoint_goes_in_slot_1() {
    let mut stick = ModelStick::new();
    let mut t = Take::arm(
        &mut stick,
        &media(),
        "Take 3",
        take_id(),
        STEREO_FLOAT,
        Meta::default(),
        1 << 22,
    )
    .unwrap();
    let ck = checkpoint_path(&media(), t.stem());
    let slot = |stick: &ModelStick, k: usize| {
        Record::decode(
            &stick.bytes(&ck).unwrap()[k * SLOT..(k + 1) * SLOT],
            &take_id(),
        )
    };
    assert!(matches!(slot(&stick, 0), Slot::Valid(r) if r.seq == 0 && r.frames == 0));
    assert_eq!(slot(&stick, 1), Slot::Invalid);
    t.push(&audio(STEREO_FLOAT, 0, 8192));
    t.checkpoint(&mut stick);
    assert!(matches!(slot(&stick, 1), Slot::Valid(r) if r.seq == 1 && r.frames == 8192));
    assert!(
        matches!(slot(&stick, 0), Slot::Valid(r) if r.seq == 0),
        "slot 0 untouched"
    );
}

#[test]
fn failed_writes_are_retried_and_the_take_comes_back_from_at_risk() {
    let mut stick = ModelStick::new();
    let mut t = Take::arm(
        &mut stick,
        &media(),
        "Take 3",
        take_id(),
        STEREO_FLOAT,
        Meta::default(),
        1 << 22,
    )
    .unwrap();
    // Arm is ops 1-7; the first unit's three tries are 8, 9, 10.
    stick.fail = [8, 9, 10].into();
    t.push(&audio(STEREO_FLOAT, 0, 8192 * 2));
    t.write_ready(&mut stick);
    assert_eq!(t.state(), TakeState::AtRisk(Why::Writes));
    t.checkpoint(&mut stick);
    assert_eq!(t.committed(), 8192 * 2, "the next round wrote it again");
    assert_eq!(t.state(), TakeState::Recording);
    assert!(t.stats().write_errors >= 3);
    t.stop();
    assert!(t.finish(&mut stick));
    let (l, samples) = read_wav(&mut stick, &t.final_path());
    assert_eq!(l.frames, 8192 * 2);
    assert!(exact(STEREO_FLOAT, &samples, 0));
}

#[test]
fn a_failed_sync_that_drops_the_pages_is_written_again() {
    let mut stick = ModelStick::new();
    let mut t = Take::arm(
        &mut stick,
        &media(),
        "Take 3",
        take_id(),
        STEREO_FLOAT,
        Meta::default(),
        1 << 22,
    )
    .unwrap();
    t.push(&audio(STEREO_FLOAT, 0, 8192 * 2));
    t.write_ready(&mut stick); // ops 8, 9
    stick.fail = [10].into(); // the round's audio sync
    t.checkpoint(&mut stick);
    assert_eq!(t.committed(), 8192 * 2);
    assert!(t.stats().rewrites >= 2 && t.stats().sync_errors == 1);
    // The dropped pages never reached the stick by themselves: only the
    // rewrite put them there.
    let mut after = stick.cut(Cut::Lost, Names::Lost, Damage::default());
    let found = recover(&mut after, &media()).unwrap();
    let [Found::CutOff { path, record, .. }] = found.as_slice() else {
        panic!("{found:?}")
    };
    assert_eq!(record.frames, 8192 * 2);
    let (_, samples) = read_wav(&mut after, path);
    assert!(exact(STEREO_FLOAT, &samples, 0));
    assert!(check_whole(&mut after, path, record).unwrap());
}

#[test]
fn a_full_stick_holds_the_take_in_memory_until_room_is_made() {
    let mut stick = ModelStick::new();
    let mut t = Take::arm(
        &mut stick,
        &media(),
        "Take 3",
        take_id(),
        STEREO_FLOAT,
        Meta::default(),
        1 << 22,
    )
    .unwrap();
    stick.capacity = Some(8192 + 4096 + 65_536 + 100);
    let mut at = 0;
    for _ in 0..4 {
        t.push(&audio(STEREO_FLOAT, at, 8192));
        at += 8192;
        t.write_ready(&mut stick);
        t.checkpoint(&mut stick);
    }
    assert_eq!(t.state(), TakeState::AtRisk(Why::Full));
    assert_eq!(t.committed(), 8192);
    assert_eq!(t.unsaved(), 8192 * 3);
    stick.capacity = None;
    t.checkpoint(&mut stick);
    assert_eq!(t.state(), TakeState::Recording);
    assert_eq!(t.committed(), 8192 * 4);
    t.stop();
    assert!(t.finish(&mut stick));
    let (l, samples) = read_wav(&mut stick, &t.final_path());
    assert_eq!(l.frames, 8192 * 4);
    assert!(exact(STEREO_FLOAT, &samples, 0));
}

#[test]
fn a_pulled_stick_holds_the_take_until_capture_memory_fills() {
    let mut stick = ModelStick::new();
    let limit = 4 * 65_536;
    let mut t = Take::arm(
        &mut stick,
        &media(),
        "Take 3",
        take_id(),
        STEREO_FLOAT,
        Meta::default(),
        limit,
    )
    .unwrap();
    stick.pulled = true;
    let mut at = 0;
    while t.push(&audio(STEREO_FLOAT, at, 1000)) == 1000 {
        at += 1000;
        t.write_ready(&mut stick);
        t.checkpoint(&mut stick);
        if t.memory() >= 65_536 {
            assert!(
                matches!(t.state(), TakeState::AtRisk(Why::Gone)),
                "{:?}",
                t.state()
            );
        }
    }
    assert_eq!(t.state(), TakeState::Stopped(Why::Memory));
    assert!(t.nearly_full());
    assert_eq!(t.committed(), 0);
    assert_eq!(t.unsaved(), t.captured());
    assert_eq!(
        t.captured(),
        (limit / 8) as u64,
        "every frame that fitted was kept"
    );
}

#[test]
fn arming_that_fails_leaves_nothing_and_disarming_removes_the_files() {
    for op in 1..=7 {
        let mut stick = ModelStick::new();
        stick.fail = [op].into();
        let r = Take::arm(
            &mut stick,
            &media(),
            "Take 3",
            take_id(),
            STEREO_FLOAT,
            Meta::default(),
            1 << 22,
        );
        assert!(r.is_err(), "op {op}");
        assert!(stick.names().is_empty(), "op {op}: {:?}", stick.names());
    }
    let mut stick = ModelStick::new();
    let t = Take::arm(
        &mut stick,
        &media(),
        "Take 3",
        take_id(),
        STEREO_FLOAT,
        Meta::default(),
        1 << 22,
    )
    .unwrap();
    assert_eq!(stick.names().len(), 2);
    assert!(t.disarm(&mut stick));
    assert!(stick.names().is_empty());
}

#[test]
fn recovery_cut_at_every_step_gives_the_same_take() {
    let s = stem("Take 3", &take_id());
    let (base, run) = cut_take(3);
    let mut clean = base.clone();
    let want = verify(
        &mut clean,
        run,
        STEREO_FLOAT,
        Damage::default(),
        "uncut recovery",
    );
    let mut probe = base.clone();
    probe.ops = 0;
    recover(&mut probe, &media()).unwrap();
    let steps = probe.ops;
    assert!(steps >= 6);
    for k in 0..steps {
        for how in [Cut::Lost, Cut::Landed, Cut::Torn(1)] {
            for names in [Names::Lost, Names::Landed, Names::LastOnly] {
                let mut s1 = base.clone();
                s1.ops = 0;
                s1.cut_after = Some(k);
                let _ = recover(&mut s1, &media());
                let mut s2 = s1.cut(how, names, Damage::default());
                let got = verify(
                    &mut s2,
                    run,
                    STEREO_FLOAT,
                    Damage::default(),
                    &format!("recovery cut at {k}, {how:?}"),
                );
                assert_eq!(got, want, "recovery cut at {k}, {how:?}, names {names:?}");
                assert!(
                    !s2.exists(&tail_path(&media(), &s))
                        || s2.len(&tail_path(&media(), &s)).unwrap() > DATA_AT
                );
            }
        }
    }
}

// ---- real files ---------------------------------------------------------

#[test]
fn round_trips_through_the_real_file_system_and_the_reader() {
    for (fmt, frames) in [
        (STEREO_FLOAT, 70_001),
        (
            RecFormat {
                channels: 1,
                ..STEREO_FLOAT
            },
            30_000,
        ),
        (
            RecFormat {
                channels: 2,
                ..MONO_24
            },
            50_000,
        ),
        (MONO_24, 30_001),
    ] {
        let dir = folder("rec-roundtrip");
        let mut fs = StdFs::new();
        let id = RecId::random();
        let mut t = Take::arm(&mut fs, &dir, "Take 1", id, fmt, Meta::default(), 1 << 24).unwrap();
        let mut at = 0;
        while at < frames {
            let n = 4096.min(frames - at);
            t.push(&audio(fmt, at, n));
            at += n;
            t.write_ready(&mut fs);
            if at % (4096 * 5) == 0 {
                t.checkpoint(&mut fs);
            }
        }
        t.stop();
        assert!(t.finish(&mut fs), "{fmt:?}");
        let path = t.final_path();
        assert!(path.exists() && !checkpoint_path(&dir, t.stem()).exists());
        let mut f = std::fs::File::open(&path).unwrap();
        let l = format::layout(&mut f).unwrap();
        assert_eq!(
            (l.container, l.frames, l.channels, l.rate),
            (Container::Wav, frames, fmt.channels, fmt.rate)
        );
        assert!(!l.cut_short);
        let bytes = std::fs::read(&path).unwrap();
        let data = frames as usize * fmt.block_align();
        assert_eq!(bytes.len(), DATA_AT as usize + data + (data & 1), "{fmt:?}");
        let mut out = vec![0.0; frames as usize * usize::from(fmt.channels)];
        format::decode(l.sample, l.endian, &bytes[4096..4096 + data], &mut out);
        assert!(exact(fmt, &out, 0), "{fmt:?}");
        if fmt.float {
            assert_eq!(&bytes[74..78], b"fact");
            assert_eq!(
                u32::from_le_bytes(bytes[82..86].try_into().unwrap()),
                frames as u32
            );
        }
        assert!(recover(&mut fs, &dir).unwrap().is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

#[test]
fn the_riff_limit_and_rf64_with_sparse_files() {
    let dir = folder("rec-riff-limit");
    let id = take_id();
    let align = STEREO_FLOAT.block_align() as u64;
    let under = RIFF_MAX / align;
    for (frames, container) in [
        (under, Container::Wav),
        (under + 1, Container::Rf64),
        (under * 2, Container::Rf64),
    ] {
        let p = dir.join(format!("{frames}.wav"));
        let sizes = Sizes::done(STEREO_FLOAT, frames);
        let Sizes::Done { end, .. } = sizes else {
            unreachable!()
        };
        let f = std::fs::File::create(&p).unwrap();
        f.set_len(end).unwrap();
        drop(f);
        let mut fs = StdFs::new();
        fs.write_at(&p, 0, &header(STEREO_FLOAT, &id, sizes))
            .unwrap();
        let mut f = std::fs::File::open(&p).unwrap();
        let l = format::layout(&mut f).unwrap();
        assert_eq!(
            (l.container, l.frames, l.cut_short),
            (container, frames, false),
            "{frames}"
        );
        let mut h = vec![0u8; 4096];
        assert_eq!(fs.read_at(&p, 0, &mut h).unwrap(), 4096);
        if container == Container::Wav {
            assert_ne!(&h[4..8], &[0xFF; 4]);
            assert_ne!(&h[4092..4096], &[0xFF; 4]);
        } else {
            assert_eq!(&h[0..4], b"RF64");
            assert_eq!(&h[12..16], b"ds64");
            assert_eq!(&h[82..86], &[0xFF; 4], "fact says see ds64");
        }
        std::fs::remove_file(&p).unwrap();
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn rf64_takes_read_back_through_the_stream_reader() {
    let dir = folder("rec-rf64-small");
    for fmt in [STEREO_FLOAT, MONO_24] {
        let path = mpfree_stream::testkit::record_take(&dir, fmt, 9001, 1);
        let rf = mpfree_stream::testkit::as_rf64(&std::fs::read(&path).unwrap());
        let l = format::layout(&mut io::Cursor::new(&rf)).unwrap();
        assert_eq!((l.container, l.frames), (Container::Rf64, 9001));
        let mut out = vec![0.0; 9001 * usize::from(fmt.channels)];
        let data = 9001 * fmt.block_align();
        format::decode(l.sample, l.endian, &rf[4096..4096 + data], &mut out);
        assert!(exact(fmt, &out, 0));
    }
    std::fs::remove_dir_all(&dir).unwrap();
}
