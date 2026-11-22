//! FLAC as a compressed source: the seek index (the file's own table only
//! when it checks out, else built), thousands of random jumps decoded
//! exactly, a stale index caught, and FLAC played through the coordinator
//! like any other source.

use std::path::Path;
use std::sync::Arc;

use mpfree_flac::Decoder;
use mpfree_stream::coord::Plan;
use mpfree_stream::cursor::Cursor;
use mpfree_stream::flac::{Error, Index, Made, POINT_S};
use mpfree_stream::fsio::{Fs, StdFs};
use mpfree_stream::player::Config;
use mpfree_stream::publish::hash_file;
use mpfree_stream::snapshot::meta;
use mpfree_stream::testkit::{flac, folder, int24_wav, played, Expect, Rig};

/// A small generator, so the test is the same every run.
struct Lcg(u64);
impl Lcg {
    fn next(&mut self, n: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 33) % n
    }
}

/// Decode frames `a..b` of `path` through `index`, reading only the bytes
/// its span names.
fn read(fs: &mut StdFs, path: &Path, index: &Index, a: u64, b: u64) -> Result<Vec<f32>, Error> {
    let (start, end, from) = index.span(a, b);
    let mut bytes = vec![0u8; (end - start) as usize];
    let n = fs.read_at(path, start, &mut bytes).expect("reads");
    bytes.truncate(n);
    let mut out = vec![0.0; (b - a) as usize * 2];
    let mut dec = Decoder::new(index.info);
    let got = index.decode(&mut dec, &bytes, from, a, b, &mut out)?;
    out.truncate(got);
    Ok(out)
}

fn check(out: &[f32], marker: u32, a: u64) {
    for (j, f) in out.chunks_exact(2).enumerate() {
        let at = a + j as u64;
        assert_eq!(
            f,
            [played(marker, at, 0), played(marker, at, 1)],
            "frame {at}"
        );
    }
}

#[test]
fn an_index_is_built_by_walking_every_frame() {
    let dir = folder("flac-build");
    let frames = 300_000u64;
    for (name, blocks) in [
        ("fixed", vec![4096]),
        ("variable", vec![4096, 1152, 2304, 576, 4608]),
    ] {
        let path = dir.join(format!("{name}.flac"));
        flac(&path, 44_100, frames, 7, &blocks, None);
        let m = meta(&path).expect("meta");
        let mut fs = StdFs::read_only();
        assert_eq!(Index::from_table(&mut fs, &path, m).expect("reads"), None);
        let index = Index::build(&mut fs, &path, m).expect("builds");
        assert_eq!(index.made, Made::Scan);
        assert_eq!(index.frames, frames);
        assert_eq!(index.info.channels, 2);
        // A point at frame 0, then one about every second.
        assert_eq!(index.points[0].sample, 0);
        assert_eq!(index.points[0].offset, index.audio.0);
        for w in index.points.windows(2) {
            let gap = w[1].sample - w[0].sample;
            let second = POINT_S * 44_100;
            assert!((second..=second + 4608).contains(&gap), "{gap}");
        }
        assert!(index.points.len() >= 6, "{}", index.points.len());
        // Its hash is the compressed file's.
        let (hash, size) = hash_file(&mut fs, &path).expect("hashes");
        assert_eq!(index.hash, Some(hash));
        assert_eq!(size, m.0);
        // Saved and read back.
        let text = index.to_text();
        assert_eq!(Index::from_text(&text), Some(index.clone()));
        assert!(index.fits(m));
        assert!(!index.fits((m.0 + 1, m.1)));
        // Another indexer's, or another format's, isn't used.
        assert_eq!(Index::from_text(&text.replacen("fi1", "fi0", 1)), None);
        assert_eq!(Index::from_text(&text.replacen(" 1 ", " 2 ", 1)), None);
    }
}

#[test]
fn a_files_own_seek_table_is_used_only_when_it_checks_out() {
    let dir = folder("flac-table");
    let frames = 200_000u64;
    let path = dir.join("good.flac");
    let bytes = flac(&path, 44_100, frames, 3, &[4096], Some(44_100));
    let m = meta(&path).expect("meta");
    let mut fs = StdFs::read_only();
    let t = Index::from_table(&mut fs, &path, m)
        .expect("reads")
        .expect("a good table");
    assert_eq!(t.made, Made::Table);
    assert_eq!(t.hash, None);
    assert_eq!(t.frames, frames);
    let built = Index::build(&mut fs, &path, m).expect("builds");
    for p in &t.points {
        assert!(built.points.iter().any(|q| q == p) || p.sample % 4096 == 0);
    }
    let out = read(&mut fs, &path, &t, 123_456, 130_000).expect("decodes");
    check(&out, 3, 123_456);

    // Points too far apart (every 10 s, as encoders often write): built.
    let wide = dir.join("wide.flac");
    flac(&wide, 44_100, 800_000, 3, &[4096], Some(441_000));
    let mw = meta(&wide).expect("meta");
    assert_eq!(Index::from_table(&mut fs, &wide, mw).expect("reads"), None);

    // A point that lands inside a frame: not used.
    let bad = dir.join("bad.flac");
    let mut b = bytes.clone();
    let table_at = b.windows(4).position(|w| w == b"fLaC").expect("marker") + 4 + 4 + 34 + 4;
    // The second point's offset, one byte on.
    let o = table_at + 18 + 8;
    let v = u64::from_be_bytes(b[o..o + 8].try_into().unwrap()) + 1;
    b[o..o + 8].copy_from_slice(&v.to_be_bytes());
    std::fs::write(&bad, &b).expect("written");
    let mb = meta(&bad).expect("meta");
    assert_eq!(Index::from_table(&mut fs, &bad, mb).expect("reads"), None);
}

#[test]
fn thousands_of_random_jumps_decode_exactly() {
    let dir = folder("flac-jumps");
    let frames = 600_000u64;
    let mut rng = Lcg(99);
    for (name, blocks, marker) in [
        ("fixed", vec![4096], 11u32),
        ("variable", vec![4096, 1152, 2304, 576, 4608, 192], 12),
    ] {
        let path = dir.join(format!("{name}.flac"));
        flac(&path, 48_000, frames, marker, &blocks, None);
        let mut fs = StdFs::read_only();
        let index = Index::build(&mut fs, &path, meta(&path).expect("meta")).expect("builds");
        for _ in 0..1500 {
            let a = rng.next(frames);
            let len = 1 + rng.next(20_000);
            let b = (a + len).min(frames);
            let out = read(&mut fs, &path, &index, a, b).expect("decodes");
            assert_eq!(out.len(), (b - a) as usize * 2);
            check(&out, marker, a);
        }
        // The edges.
        check(
            &read(&mut fs, &path, &index, 0, 1).expect("first"),
            marker,
            0,
        );
        let last = read(&mut fs, &path, &index, frames - 1, frames + 50).expect("last");
        assert_eq!(last.len(), 2);
        check(&last, marker, frames - 1);
    }
}

#[test]
fn a_stale_index_is_caught_never_played() {
    let dir = folder("flac-stale");
    let frames = 300_000u64;
    let path = dir.join("song.flac");
    flac(&path, 44_100, frames, 5, &[4096], None);
    let mut fs = StdFs::read_only();
    let old = Index::build(&mut fs, &path, meta(&path).expect("meta")).expect("builds");
    // The same audio, encoded again with other frames: FLAC's own
    // checksum of the audio would be the same, its frames aren't.
    flac(&path, 44_100, frames, 5, &[1152, 4608], None);
    let m = meta(&path).expect("meta");
    let mut stale = 0;
    let mut rng = Lcg(7);
    for _ in 0..300 {
        let a = rng.next(frames - 10_000);
        match read(&mut StdFs::read_only(), &path, &old, a, a + 5_000) {
            // Where a jump still happens to land on a frame start, what it
            // decodes is the file's audio, never anything else.
            Ok(out) => check(&out, 5, a),
            Err(Error::Stale) => stale += 1,
            Err(e) => panic!("{e}"),
        }
    }
    assert!(stale > 250, "{stale}");
    // Built again, it plays.
    let new = Index::build(&mut fs, &path, m).expect("builds");
    assert_ne!(new.hash, old.hash);
    check(
        &read(&mut fs, &path, &new, 100_000, 101_000).expect("decodes"),
        5,
        100_000,
    );
}

#[test]
fn damage_is_an_error_never_noise() {
    let dir = folder("flac-damage");
    let frames = 100_000u64;
    let path = dir.join("song.flac");
    let mut b = flac(&path, 44_100, frames, 6, &[4096], None);
    let mut fs = StdFs::read_only();
    let index = Index::build(&mut fs, &path, meta(&path).expect("meta")).expect("builds");
    // Flip a byte inside a frame's samples, a few frames in.
    let at = index.point(50_000).offset as usize + 100;
    b[at] ^= 0x40;
    std::fs::write(&path, &b).expect("written");
    let r = read(&mut fs, &path, &index, 49_000, 60_000);
    assert!(matches!(r, Err(Error::Damaged { .. })), "{r:?}");
}

fn rig() -> Rig {
    Rig::new(
        Config {
            slots: 64,
            ..Config::default()
        },
        Plan {
            window_s: 2.0,
            min_window_s: 1.0,
            ..Plan::default()
        },
    )
}

/// Play `n` periods of 256 frames on `k`; every frame silent or exactly
/// the marked frame it should be. Gives (frames that sounded, missed).
fn play(rig: &mut Rig, k: &mut Cursor, n: usize, frames: u64, marker: u32) -> (u64, u64) {
    let (mut sounded, mut missed) = (0, 0);
    for _ in 0..n {
        let e = Expect::of(k);
        let mut outs = vec![vec![0.0; 512]];
        missed += rig.period(&mut [&mut *k], &mut outs);
        sounded += e.check(&rig.p, &outs[0], frames, |_| marker);
    }
    (sounded, missed)
}

#[test]
fn flac_plays_through_the_coordinator_exactly_with_jumps_and_loops() {
    let dir = folder("flac-play");
    let frames = 400_000u64;
    for (name, blocks, marker) in [
        ("fixed", vec![4096], 21u32),
        ("variable", vec![4096, 1152, 576, 4608], 22),
    ] {
        let path = dir.join(format!("{name}.flac"));
        flac(&path, 44_100, frames, marker, &blocks, None);
        let index = Index::build(&mut StdFs::read_only(), &path, meta(&path).expect("meta"))
            .expect("builds");
        let mut rig = rig();
        rig.c.new_project();
        rig.c
            .open_flac(0, &path, Arc::new(index), false)
            .expect("opens");
        assert_eq!(rig.c.frames(0), Some(frames));
        let mut k = Cursor::new(0, 0);
        rig.warm(&mut [&mut k], 2_000);
        // From the start, whole, nothing missed.
        let (sounded, missed) = play(&mut rig, &mut k, 400, frames, marker);
        assert_eq!((sounded, missed), (400 * 256, 0));
        // Jumps anywhere, each warmed first: exact.
        let mut rng = Lcg(5);
        for _ in 0..25 {
            k.seek(rng.next(frames - 20_000));
            rig.warm(&mut [&mut k], 2_000);
            let (sounded, missed) = play(&mut rig, &mut k, 20, frames, marker);
            assert_eq!((sounded, missed), (20 * 256, 0), "{name}");
        }
        // A loop across index points.
        k.set_loop(Some((100_000, 150_000)));
        k.seek(140_000);
        rig.warm(&mut [&mut k], 2_000);
        let (sounded, _) = play(&mut rig, &mut k, 200, frames, marker);
        assert!(sounded > 190 * 256);
        assert_eq!(rig.c.stats().flac_failed, 0);
        assert!(rig.c.unplayable().is_none());
        assert_ne!(rig.c.flac_speed(), 441_000.0, "decoding was measured");
    }
}

#[test]
fn flac_at_another_rate_plays_as_the_same_wav_does() {
    let dir = folder("flac-rate");
    let frames = 200_000u64;
    let fpath = dir.join("song.flac");
    flac(&fpath, 48_000, frames, 9, &[4096, 1152], None);
    let samples: Vec<f32> = (0..frames)
        .flat_map(|f| [played(9, f, 0), played(9, f, 1)])
        .collect();
    let wpath = dir.join("song.wav");
    int24_wav(&wpath, 48_000, 2, &samples);
    let index =
        Index::build(&mut StdFs::read_only(), &fpath, meta(&fpath).expect("meta")).expect("builds");
    let mut rig = rig();
    rig.c.new_project();
    rig.c
        .open_flac(0, &fpath, Arc::new(index), true)
        .expect("opens");
    rig.c.open_at_engine_rate(1, &wpath).expect("opens");
    assert_eq!(rig.c.frames(0), rig.c.frames(1));
    let (mut kf, mut kw) = (Cursor::new(0, 0), Cursor::new(1, 1));
    for start in [0u64, 61_234, 150_000] {
        kf.seek(start);
        kw.seek(start);
        rig.warm(&mut [&mut kf, &mut kw], 2_000);
        for _ in 0..60 {
            let mut outs = vec![vec![0.0; 512]; 2];
            let missed = rig.period(&mut [&mut kf, &mut kw], &mut outs);
            assert_eq!(missed, 0);
            assert_eq!(outs[0], outs[1], "the FLAC and the WAV at {start}");
            assert!(outs[0].iter().any(|&x| x != 0.0));
        }
    }
}

#[test]
fn a_stale_index_stops_the_source_and_nothing_wrong_plays() {
    let dir = folder("flac-stale-play");
    let frames = 400_000u64;
    let path = dir.join("song.flac");
    let other = dir.join("other.flac");
    flac(&path, 44_100, frames, 4, &[4096], None);
    flac(&other, 44_100, frames, 4, &[1152, 4608], None);
    // An index of other frames that claims to fit this file (as if the
    // file was encoded again and its size and time came out the same).
    let mut index =
        Index::build(&mut StdFs::read_only(), &other, meta(&other).expect("meta")).expect("builds");
    index.meta = meta(&path).expect("meta");
    let mut rig = rig();
    rig.c.new_project();
    let tag = rig
        .c
        .open_flac(0, &path, Arc::new(index), false)
        .expect("opens");
    let mut k = Cursor::new(0, 0);
    k.seek(200_000);
    rig.warm(&mut [&mut k], 500);
    let (_, missed) = play(&mut rig, &mut k, 300, frames, 4);
    assert!(missed > 0, "it can't play from a stale index");
    let (t, why) = rig.c.unplayable().expect("said");
    assert_eq!(t, tag);
    assert!(why.contains("changed since it was indexed"), "{why}");
    assert!(rig.c.unplayable().is_none(), "said once");
    assert!(rig.c.stats().flac_failed > 0);
    // An index that doesn't fit the file is refused at the open.
    let stale =
        Index::build(&mut StdFs::read_only(), &other, meta(&other).expect("meta")).expect("builds");
    let e = rig
        .c
        .open_flac(1, &path, Arc::new(stale), false)
        .unwrap_err();
    assert!(e.contains("out of date"), "{e}");
}
