//! A sound's analysis kept in a file beside it (`SourceView::to_cache`,
//! `SourceView::from_cache`): read back, it is the view the analysis made,
//! and a file that isn't this sound's, or doesn't read, is never used.

use mpfree_sampler::model::Source;
use mpfree_sampler_ui::source::{cache_file, SourceView};

const RATE: u32 = 44_100;

/// Two bars of hits at 120 BPM, each an A3 (220 Hz) dying away, with a
/// little noise; every sample a 16-bit value when `sixteen`.
fn hits(channels: u8, sixteen: bool) -> Source {
    let beat = RATE as usize / 2;
    let frames = beat * 8;
    let mut x = 1u32;
    let data: Vec<f32> = (0..frames * usize::from(channels))
        .map(|i| {
            let f = i / usize::from(channels);
            let t = (f % beat) as f32 / RATE as f32;
            x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let noise = (x >> 8) as f32 / (1u32 << 24) as f32 - 0.5;
            let y =
                0.6 * (t * 220.0 * std::f32::consts::TAU).sin() * (-6.0 * t).exp() + 0.02 * noise;
            if sixteen {
                ((y * 32_767.0).round() + 0.0) / 32_768.0
            } else {
                y
            }
        })
        .collect();
    Source::new("hits", RATE, channels, data).unwrap()
}

fn same(a: &SourceView, b: &SourceView) {
    assert_eq!(a.id, b.id);
    assert!(b.ready);
    assert_eq!(a.tempo.map(f64::to_bits), b.tempo.map(f64::to_bits));
    assert_eq!(a.root_note, b.root_note);
    assert_eq!(a.root, b.root);
    let (oa, ob) = (a.onsets.as_ref().unwrap(), b.onsets.as_ref().unwrap());
    assert_eq!(oa.candidates(), ob.candidates());
    assert_eq!(oa.flux(), ob.flux());
    assert_eq!(&*a.mono, &*b.mono);
    assert_eq!(a.peaks.frames(), b.peaks.frames());
    assert_eq!(a.peaks.channels(), b.peaks.channels());
    // The waveform draws the same at every zoom: samples, each level of
    // the pyramid, the whole sound in one column.
    let n = a.peaks.frames() as f64;
    for per in [0.5, 7.0, 100.0, 700.0, 5_000.0, n] {
        for ch in 0..a.peaks.channels() {
            let mut f = 0.0;
            while f < n {
                assert_eq!(
                    a.peaks.column(ch, f, f + per),
                    b.peaks.column(ch, f, f + per),
                    "{per} frames a column, channel {ch}, at {f}"
                );
                f += per * 997.0;
            }
        }
    }
}

#[test]
fn a_view_reads_back_from_its_cache_as_it_was_made() {
    for (channels, sixteen) in [(2, false), (1, false), (2, true)] {
        let s = hits(channels, sixteen);
        let made = SourceView::analyse(&s);
        assert!(
            made.tempo.is_some() && made.root_note.is_some(),
            "a test sound with a beat and a note"
        );
        let bytes = made.to_cache().expect("a ready view has a cache");
        let back = SourceView::from_cache(&s, &bytes).expect("reads back");
        same(&made, &back);
        // Small beside the sound: under 4 % of it as floats (the pyramid
        // is about 3 %).
        let floats = s.frames() as usize * usize::from(channels) * 4;
        assert!(bytes.len() * 25 < floats, "{} bytes", bytes.len());
    }
}

#[test]
fn a_cache_that_isnt_this_sounds_is_never_used() {
    let s = hits(2, false);
    let bytes = SourceView::analyse(&s).to_cache().unwrap();
    // Another sound (another id), or the same audio as mono.
    assert!(SourceView::from_cache(&hits(2, true), &bytes).is_none());
    assert!(SourceView::from_cache(&hits(1, false), &bytes).is_none());
    // Cut short anywhere, one byte too many, another version's magic.
    for cut in [0, 7, 8, 20, 40, 45, 60, bytes.len() / 2, bytes.len() - 1] {
        assert!(
            SourceView::from_cache(&s, &bytes[..cut]).is_none(),
            "cut at {cut}"
        );
    }
    let mut over = bytes.clone();
    over.push(0);
    assert!(SourceView::from_cache(&s, &over).is_none());
    let mut magic = bytes.clone();
    magic[7] = b'2';
    assert!(SourceView::from_cache(&s, &magic).is_none());
    // A view still being worked out has no cache.
    assert!(SourceView::pending(&s).to_cache().is_none());
    assert!(SourceView::quick(&s).to_cache().is_none());
}

#[test]
fn the_cache_is_named_by_the_sound_and_the_analysis_version() {
    let s = hits(1, false);
    assert_eq!(cache_file(s.id()), format!("{}.a1.peaks", s.id()));
    assert_ne!(cache_file(s.id()), mpfree_sampler::wav::file_name(&s));
}
