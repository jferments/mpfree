use super::*;

/// An audio track's line pick, as a loop in its cell hears it (phase
/// 4b): one jack on both sides, half of each for Sum (as the take
/// sums them), both as they are for Stereo.
#[test]
fn a_track_line_pick_fills_both_sides_as_its_take_hears_it() {
    use mpfree_audiotracks::Input;
    let (l, r) = ([0.5f32; PERIOD], [-0.25f32; PERIOD]);
    let pick = |p: Input| {
        let (mut a, mut b) = ([9f32; PERIOD], [9f32; PERIOD]);
        pick_line(p, (&l, &r), &mut a, &mut b);
        (a[PERIOD - 1], b[0])
    };
    assert_eq!(pick(Input::Left), (0.5, 0.5));
    assert_eq!(pick(Input::Right), (-0.25, -0.25));
    assert_eq!(pick(Input::Stereo), (0.5, -0.25));
    assert_eq!(pick(Input::Sum), (0.125, 0.125));
    let src = |from| {
        LoopSource::Track(TrackInput {
            channel: mpfree_fx::StripId::Track(3),
            from,
        })
    };
    assert!(src(TrackFrom::Line(Input::Left)).line());
    let mix = TrackFrom::Mix(
        mpfree_fx::StripId::Master,
        mpfree_fx::CapturePoint::BeforeInserts,
    );
    assert!(src(mix).inside() && !src(mix).line());
    assert_eq!(
        source_words(src(mix), None),
        "the track's input, the master before its effects"
    );
}
use mpfree_transport::{Command as T, Transport};

fn s32(v: f32) -> i32 {
    (v * 2_147_483_647.0) as i32
}

#[test]
fn a_closed_input_gives_silence_and_counts_nothing() {
    let mut l = LineIn::closed();
    let (mut a, mut b) = ([1.0; PERIOD], [1.0; PERIOD]);
    assert!(l.period(&mut a, &mut b).is_none());
    assert!(a.iter().chain(b.iter()).all(|&v| v == 0.0));
    assert_eq!(l.stats, LineStats::default());
    assert_eq!(l.close(), "line input: already off");
}

#[test]
fn opening_a_device_that_is_not_there_says_so_and_stays_off() {
    let mut l = LineIn::closed();
    let e = l.open(99).unwrap_err();
    assert!(
        e.contains("/dev/snd/pcmC99D0c") && e.contains("stays off"),
        "{e}"
    );
    assert!(!l.is_open());
}

#[test]
fn the_fifo_hands_out_whole_periods_and_drops_what_runs_ahead() {
    let mut l = LineIn::closed();
    // Pretend it is open: drive the FIFO directly.
    let frames: Vec<i32> = (0..3 * PERIOD)
        .flat_map(|f| [s32(f as f32 / 1000.0), s32(-0.5)])
        .collect();
    l.push_s32(&frames);
    assert_eq!(l.len, 3 * PERIOD);
    let (mut a, mut b) = ([0.0; PERIOD], [0.0; PERIOD]);
    // `period` reads only while open, so take periods by hand as it does.
    let take = |l: &mut LineIn, a: &mut [f32; PERIOD], b: &mut [f32; PERIOD]| {
        for f in 0..PERIOD {
            let at = (l.head + f) % FIFO;
            a[f] = l.fifo[2 * at];
            b[f] = l.fifo[2 * at + 1];
        }
        l.head = (l.head + PERIOD) % FIFO;
        l.len -= PERIOD;
    };
    take(&mut l, &mut a, &mut b);
    assert!((a[5] - 0.005).abs() < 1e-6 && (b[0] + 0.5).abs() < 1e-6);
    // Overfilling drops the oldest frames, counted.
    let lots: Vec<i32> = vec![s32(0.25); 2 * FIFO];
    l.push_s32(&lots);
    assert_eq!(l.len, FIFO);
    assert_eq!(l.stats.dropped, 2 * PERIOD as u64);
    take(&mut l, &mut a, &mut b);
    assert!((a[0] - 0.25).abs() < 1e-6);
}

#[test]
fn the_wave_covers_the_loop_and_answers_any_zoom() {
    let w = LoopWave {
        layers: Vec::new(),
        frames: 1000,
        recorded: None,
        columns: (0..10)
            .map(|i| {
                let v = i as f32 / 10.0;
                [
                    WaveColumn {
                        min: -v,
                        max: v,
                        rms: v / 2.0,
                    },
                    WaveColumn::default(),
                ]
            })
            .collect(),
    };
    let c = w.column(0, 0.0, 1000.0);
    assert_eq!((c.min, c.max), (-0.9, 0.9));
    let c = w.column(0, 520.0, 530.0);
    assert_eq!(c.max, 0.5);
    assert_eq!(w.column(0, 2000.0, 3000.0), WaveColumn::default());
    assert_eq!(w.column(2, 0.0, 10.0), WaveColumn::default());
}

/// In 6/8 a bar is six eighths (2880 ticks), not six quarters: a free
/// first loop of one bar says 1 bar (it said 0), the view's beat is an
/// eighth, and six of them make the loop.
#[test]
fn a_free_loop_counts_its_bars_by_the_meter() {
    const RATE: u32 = 22_050;
    let mut h = LoopHost::new(RATE, 20);
    let mut t = Transport::new(RATE, 100.0).unwrap();
    t.set_meter(mpfree_transport::Meter::new(6, 8).unwrap())
        .unwrap();
    let mut b = Block::new();
    h.set_tempo_from_loop(true);
    h.command(Command::SetBars(None));
    let mut run = |h: &mut LoopHost, periods: usize| {
        let mut took = None;
        for _ in 0..periods {
            assert!(t.process(PERIOD as u32, &mut b));
            let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
            h.period(&b, &mut l, &mut r);
            took = took.or(h.take_tempo().map(|(n, r, at, _)| (n, r, at)));
        }
        took
    };
    h.command(Command::Record);
    // 1.5 s free: one bar of 6/8 at 120 BPM (three quarters).
    assert_eq!(run(&mut h, (3 * RATE as usize / 2) / PERIOD), None);
    h.command(Command::Record);
    let (n, reading, _) = run(&mut h, 4).expect("the loop sets the tempo");
    let bpm = reading.bpm;
    assert_eq!((n, reading.bars), (0, 1), "one bar of 6/8, at {bpm:.2} BPM");
    assert!((bpm - 120.0).abs() < 0.5, "{bpm}");
    let v = h.view(bpm);
    assert_eq!(v.bar_ticks, 3 * PPQ as Tick);
    let six = v.frames_per_beat * f64::from(v.beats_per_bar);
    assert!(
        (six - v.status.frames as f64).abs() < 1.0,
        "six eighths, {six} frames, make the loop's {}",
        v.status.frames
    );
    assert_eq!(crate::looper_page::bars_of(v.status.ticks, v.bar_ticks), 1);
}

/// Smart loop end's log line says how sure it was of the end, the
/// bars, the tempo and the start, kept apart (the looper refactor's
/// four confidences); with no decision (an overdub started on the
/// tap) it says nothing of them.
#[test]
fn the_smart_loop_end_line_says_how_sure_it_was_four_ways() {
    let c = close::Confidence {
        boundary: Boundary::Clear,
        bars: Bars::Ambiguous,
        tempo: Tempo::FromTheLength,
        start: Start::FirstHit,
    };
    assert_eq!(
        sure_words(&c),
        "the end clear, the bars a close call, the tempo from the length alone, \
         the start on your first hit"
    );
    let c = close::Confidence {
        boundary: Boundary::Possible,
        bars: Bars::Yours,
        tempo: Tempo::FromYourBars,
        start: Start::Press,
    };
    assert_eq!(
        sure_words(&c),
        "the end only possible, the bars yours, the tempo from your bars, the start on your press"
    );
    let mut note = SmartNote {
        n: 0,
        outcome: SmartOutcome::Kept("no note near it"),
        start: 0,
        tap: 1_000,
        best: None,
        second: None,
        sure: Some(close::Confidence {
            boundary: Boundary::Unmatched,
            bars: Bars::Clear,
            tempo: Tempo::FromTheBeat,
            start: Start::Press,
        }),
    };
    let (line, words) = smart_words(&note, SmartEnd::Suggest, 44_100);
    assert_eq!(
        line,
        "looper: Loop 1: smart loop end kept your tap: no note near it; how sure: no end to \
         go on, the bars clear, the tempo from the beat you played, the start on your press"
    );
    assert_eq!(words.as_deref(), Some("Kept your tap: no note near it."));
    note.sure = None;
    let (line, _) = smart_words(&note, SmartEnd::Suggest, 44_100);
    assert_eq!(
        line,
        "looper: Loop 1: smart loop end kept your tap: no note near it"
    );
    // Loop memory was full: the screen says why the end after the tap
    // couldn't be offered.
    note.outcome = SmartOutcome::Kept(mpfree_looper::MEMORY_FULL);
    let (_, words) = smart_words(&note, SmartEnd::Suggest, 44_100);
    assert_eq!(
        words.as_deref(),
        Some("Kept your tap: loop memory was full, so nothing after it was kept.")
    );
}

/// Move start / Move end's hits: each onset once, on the first frame
/// at half its peak; not one quieter than -34 dB, not a swell that
/// isn't four times what came before, and not one sooner than `apart`
/// after the last.
#[test]
fn the_hits_in_a_loop_point_window_are_each_onset_once() {
    let mut audio = vec![[0.0f32; 2]; 4_000];
    let mut hit = |at: usize, peak: f32| {
        for (i, f) in audio[at..at + 300].iter_mut().enumerate() {
            let v = peak * (1.0 - i as f32 / 300.0);
            *f = [v, -v];
        }
    };
    hit(200, 0.5);
    // 20 frames into the block it lands in.
    hit(64 * 21 + 20, 0.6);
    // Too soon after that one (apart is 1000 frames).
    hit(2_000, 0.9);
    // Too quiet to count.
    hit(3_600, 0.015);
    let mut out = [0i64; POINT_HITS];
    let n = hits_into(&audio, -500, 1_000, &mut out);
    assert_eq!(&out[..usize::from(n)], &[-500 + 200, -500 + 1_364]);
    // A full window: as many as fit.
    let mut one = [0i64; 1];
    assert_eq!(hits_into(&audio, 0, 100, &mut one), 1);
    assert_eq!(one, [200]);
}

/// A loop moved by hand is one moved somewhere smart loop end didn't
/// put it: not on what it found, nor on that with your tap or your
/// press put back (Use my tap, Use my press after a snap).
#[test]
fn a_hand_move_is_any_move_but_what_was_found() {
    let found = Shift {
        start: 800,
        end: -2_100,
    };
    let empty = LoopHost::new(44_100, 2).looper.status();
    let st = |moved: Option<Shift>, found: Option<Shift>| Status {
        moved,
        found,
        ..empty
    };
    assert!(!hand_moved(&st(None, Some(found))));
    assert!(!hand_moved(&st(Some(Shift::NONE), Some(found))));
    assert!(!hand_moved(&st(Some(found), Some(found))));
    assert!(!hand_moved(&st(
        Some(with_edge(found, Edge::End, 0)),
        Some(found)
    )));
    assert!(!hand_moved(&st(
        Some(with_edge(found, Edge::Start, 0)),
        Some(found)
    )));
    assert!(hand_moved(&st(
        Some(with_edge(found, Edge::End, -2_900)),
        Some(found)
    )));
    assert!(hand_moved(&st(Some(found), None)));
}

/// Length's bars are final (the looper refactor's Finding C): 1 bar
/// in 10 s would be 24 BPM, so the pass waits and the hint line asks;
/// Read as plays it as it read, Discard clears it.
#[test]
fn a_length_too_slow_to_keep_asks_and_read_as_or_discard_answers() {
    const RATE: u32 = 22_050;
    for yes in [true, false] {
        let mut h = LoopHost::new(RATE, 20);
        let mut t = Transport::new(RATE, 100.0).unwrap();
        let mut b = Block::new();
        h.set_tempo_from_loop(true);
        h.command(Command::SetBars(Some(1)));
        let mut run = |h: &mut LoopHost, periods: usize| {
            let mut took = None;
            for _ in 0..periods {
                assert!(t.process(PERIOD as u32, &mut b));
                let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
                h.period(&b, &mut l, &mut r);
                took = took.or(h.take_tempo());
            }
            took
        };
        h.command(Command::Record);
        assert_eq!(run(&mut h, 10 * RATE as usize / PERIOD), None);
        h.command(Command::Record);
        assert_eq!(run(&mut h, 4), None, "no tempo while it asks");
        let (line, tell) = h.take_asks().expect("it says it waits");
        assert!(tell);
        assert!(
            line.starts_with(
                "looper: Loop 1: Length's 1 bar would be 24 BPM, slower than 30. Read it as "
            ),
            "{line}"
        );
        assert_eq!(h.take_asks(), None, "once");
        let r = h.view(100.0).smart.length.expect("the hint line asks");
        assert_eq!(r.said, Some(1));
        assert_eq!(length_ask_button(&r), format!("Read as {} bars", r.bars));
        assert_eq!(h.length_asking(), Some((0, r)));
        if yes {
            let said = h.use_reading().expect("Read as");
            assert!(said.starts_with("looper: Loop 1 plays as "), "{said}");
            let (n, reading, _, into) = run(&mut h, 2).expect("the tempo, once you say");
            assert_eq!((n, reading, into), (0, r, 0));
            assert_eq!(h.looper.loop_status(0).unwrap().phase, Phase::Playing);
        } else {
            let said = h.discard_waiting().expect("Discard");
            assert_eq!(
                said,
                "looper: Loop 1's pass thrown away (UNDO brings it back)"
            );
            assert_eq!(run(&mut h, 2), None);
            assert_eq!(h.looper.loop_status(0).unwrap().phase, Phase::Empty);
        }
        assert_eq!(h.view(100.0).smart.length, None);
        assert_eq!((h.use_reading(), h.discard_waiting()), (None, None));
    }
}

/// The period a free first loop closes on a tap, at a tempo the song
/// doesn't have yet, asks the printer for nothing: the song takes the
/// loop's tempo from the next period, and the printer looks at the
/// loop at that tempo. (It asked for a print at the song's old tempo,
/// and that first message to its worker allocated on the audio
/// thread.) A song that doesn't take the tempo gets its print asked
/// for in the next period, as before.
#[test]
fn the_period_a_free_loop_sets_the_tempo_asks_the_printer_for_nothing() {
    const RATE: u32 = 44_100;
    let mut h = LoopHost::new(RATE, 20);
    let mut t = Transport::new(RATE, 100.0).unwrap();
    let mut b = Block::new();
    h.set_tempo_from_loop(true);
    h.command(Command::SetBars(None));
    let mut period = |h: &mut LoopHost| {
        assert!(t.process(PERIOD as u32, &mut b));
        let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
        let (_, calls) = crate::sys::alloc_count::calls(|| h.period(&b, &mut l, &mut r));
        (calls, h.take_tempo())
    };
    h.command(Command::Record);
    // 2 s free: a bar at 120 BPM, with the song at 100.
    for _ in 0..(2 * RATE as usize) / PERIOD {
        assert_eq!(period(&mut h).1, None);
    }
    h.command(Command::Record);
    let (calls, taken) = (0..8)
        .map(|_| period(&mut h))
        .find(|(_, taken)| taken.is_some())
        .expect("the loop sets the tempo");
    let bpm = taken.unwrap().1.bpm;
    assert!((bpm - 120.0).abs() < 0.5, "{bpm}");
    assert!(!h.printer.asked, "no print asked for at 100 BPM");
    assert_eq!(calls, 0, "allocations or frees on the audio thread's path");
    // The song stayed at 100: now the loop needs its print at 100.
    period(&mut h);
    assert!(h.printer.asked, "{}", h.describe());
}

/// The looper on a real transport with the input closed: presses log,
/// the phases log, and the waveform gets built.
#[test]
fn presses_and_changes_log_and_the_wave_is_built() {
    let mut h = LoopHost::new(44_100, 20);
    let mut t = Transport::new(44_100, 120.0).unwrap();
    let mut b = Block::new();
    h.monitor = true;
    assert_eq!(
        h.command(Command::SetBars(Some(1))),
        "looper: first pass length 1 bar"
    );
    assert!(h
        .command(Command::Record)
        .starts_with("looper: Record: armed"));
    t.command(0, T::Play);
    let mut lines = Vec::new();
    let mut wave = None;
    for p in 0..1200u64 {
        assert!(t.process(PERIOD as u32, &mut b));
        let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
        let (peak, note) = h.period(&b, &mut l, &mut r);
        assert!(note.is_none() && peak == 0.0, "the input is off: silence");
        lines.extend(h.changes(|| format!("period {p}")));
        if let Some(w) = h.wave_step(p, 0) {
            wave = Some(w);
        }
    }
    // Armed and recording in the same period: the bar line was at once.
    assert_eq!(
        lines[0],
        "looper: recording the first pass from period 0 (1 bar)"
    );
    assert!(
        lines[1].starts_with("looper: loop of 1 bar at period 689: 88200 frames"),
        "{lines:?}"
    );
    let w = wave.expect("the wave of the (silent) loop");
    assert_eq!((w.frames, w.columns.len()), (88_200, WAVE_COLUMNS));
    let v = h.view(120.0);
    assert_eq!(
        (v.status.phase, v.status.layers, v.line_on),
        (Phase::Playing, 1, false)
    );
    assert!((v.frames_per_beat - 22_050.0).abs() < 1e-6);
    assert!(h.command(Command::Undo).contains("nothing to undo"));
    assert!(h.command(Command::Overdub).contains("overdub on (layer 2)"));
    assert!(
        h.describe().starts_with("looper: Playing, 1 layers"),
        "{}",
        h.describe()
    );
    assert!(h.describe().contains("line input off, monitor on"));
}

/// A layer's speaker: the mute goes to the layer by its id, the log
/// names it, and the next view shows it muted and not heard.
/// Listen: refused while the music plays or the loop is empty; with
/// the music stopped it plays the picked loop, and asked again stops.
#[test]
fn listen_plays_the_picked_loop_while_stopped() {
    let mut h = LoopHost::new(44_100, 8);
    let mut t = Transport::new(44_100, 120.0).unwrap();
    let mut b = Block::new();
    assert!(h.listen(Some(None)).contains("nothing to play"));
    h.command(Command::SetBars(Some(1)));
    h.command(Command::SetLatency(0));
    h.command(Command::Record);
    t.command(0, T::Play);
    let mut run = |h: &mut LoopHost, t: &mut Transport, periods: u64| {
        for _ in 0..periods {
            assert!(t.process(PERIOD as u32, &mut b));
            let (mut l, mut r) = ([0.1; PERIOD], [0.1; PERIOD]);
            h.take_pads(&l, &r);
            h.period(&b, &mut l, &mut r);
        }
    };
    run(&mut h, &mut t, 800);
    assert_eq!(h.looper.status().phase, Phase::Playing);
    assert!(h.listen(Some(None)).contains("while the music is stopped"));
    let f = t.frame();
    t.command(f, T::Stop);
    run(&mut h, &mut t, 4);
    assert!(h.listen(Some(None)).contains("Loop 1 plays on its own"));
    run(&mut h, &mut t, 4);
    assert_eq!(h.view(120.0).listening.map(|l| (l.0, l.1)), Some((0, None)));
    assert!(h.listen(Some(None)).contains("stopped"));
    assert_eq!(h.view(120.0).listening, None);
}

/// Length: Pattern (the default): an empty loop follows the pattern
/// playing, a tap records exactly that many bars, and picking another
/// Length lets go of it.
#[test]
fn length_pattern_follows_the_pattern_playing() {
    let mut h = LoopHost::new(44_100, 8);
    let mut t = Transport::new(44_100, 120.0).unwrap();
    let mut b = Block::new();
    let mut run = |h: &mut LoopHost, periods: u64| {
        for _ in 0..periods {
            assert!(t.process(PERIOD as u32, &mut b));
            let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
            h.period(&b, &mut l, &mut r);
        }
    };
    assert!(h
        .command(Command::SetBars(Some(PATTERN_BARS)))
        .contains("first pass length"));
    assert!(h.match_pattern);
    // Nothing plays: free.
    run(&mut h, 2);
    assert_eq!(h.looper.status().bars, None);
    h.set_pattern_bars(Some(4));
    run(&mut h, 2);
    assert_eq!(h.looper.status().bars, Some(4));
    let v = h.view(120.0);
    assert!(v.match_pattern);
    assert_eq!(v.pattern_bars, Some(4));
    // Too long to follow: free.
    h.set_pattern_bars(Some(PATTERN_MAX_BARS + 1));
    run(&mut h, 2);
    assert_eq!(h.looper.status().bars, None);
    h.set_pattern_bars(Some(2));
    h.command(Command::Record);
    run(&mut h, 2);
    assert_eq!(h.looper.status().bars, Some(2));
    // Another Length lets go of the pattern.
    h.command(Command::SetBars(Some(8)));
    h.set_pattern_bars(Some(1));
    run(&mut h, 2);
    assert!(!h.match_pattern);
    assert!(!h.view(120.0).match_pattern);
}

/// Starts sticks from loop to loop: set on one empty loop, the next
/// empty loop picked shows it and records with it; a loop that has
/// sound keeps its own.
#[test]
fn starts_sticks_for_every_empty_loop() {
    let mut h = LoopHost::new(44_100, 8);
    let mut t = Transport::new(44_100, 120.0).unwrap();
    let mut b = Block::new();
    let mut run = |h: &mut LoopHost, periods: u64| {
        for _ in 0..periods {
            assert!(t.process(PERIOD as u32, &mut b));
            let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
            h.period(&b, &mut l, &mut r);
        }
    };
    run(&mut h, 1);
    assert_eq!(h.looper.status().starts, Starts::Bar, "on the bar at first");
    h.command(Command::SetStarts(Starts::Play));
    run(&mut h, 1);
    h.select(3);
    run(&mut h, 1);
    assert_eq!(h.looper.status().starts, Starts::Play, "loop 4 shows it");
    h.select(5);
    assert!(h.command(Command::Record).contains("starts when you play"));
    run(&mut h, 1);
    let s = h.looper.loop_status(5).unwrap();
    assert!(s.phase == Phase::Armed && s.listening, "{s:?}");
    // Back to On the bar: the next one takes that.
    h.select(6);
    h.command(Command::SetStarts(Starts::Bar));
    h.select(7);
    run(&mut h, 1);
    assert_eq!(h.looper.status().starts, Starts::Bar);
    assert_eq!(
        h.looper.loop_status(5).unwrap().starts,
        Starts::Play,
        "armed: kept"
    );
}

/// Records from the loops: Record on an empty loop resamples the
/// others; on a loop that plays it still overdubs.
#[test]
fn records_from_the_loops_resamples_into_an_empty_loop() {
    let mut h = LoopHost::new(44_100, 8);
    let mut t = Transport::new(44_100, 120.0).unwrap();
    let mut b = Block::new();
    h.command(Command::SetBars(Some(1)));
    h.command(Command::SetLatency(0));
    h.command(Command::Record);
    t.command(0, T::Play);
    let mut run = |h: &mut LoopHost, periods: u64| {
        for _ in 0..periods {
            assert!(t.process(PERIOD as u32, &mut b));
            let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
            h.period(&b, &mut l, &mut r);
        }
    };
    run(&mut h, 700);
    assert_eq!(h.view(120.0).status.phase, Phase::Playing);
    h.resample = true;
    let line = h.command(Command::Record);
    assert!(line.contains("overdub"), "loop 1 has a loop: {line}");
    h.command(Command::Record);
    run(&mut h, 1);
    h.select(1);
    let line = h.command(Command::Record);
    assert!(line.contains("Resample"), "{line}");
    run(&mut h, 1);
    let v = h.view(120.0);
    assert_eq!(v.selected, 1);
    assert!(v.resample);
    assert!(v.status.resampling, "{:?}", v.status);
    assert_eq!(v.loops[0].phase, Phase::Playing);
}

/// A Loop cell armed on the grid records into the picked loop if it is
/// empty, else the first empty one; with all 8 in use, or the memory
/// full, REC says why (loop-tracks.md part 4).
#[test]
fn a_new_loop_goes_into_a_free_one_or_says_why_not() {
    fn record(h: &mut LoopHost, n: usize) {
        let mut t = Transport::new(44_100, 120.0).unwrap();
        let mut b = Block::new();
        t.command(0, T::Play);
        h.select(n);
        h.command(Command::Record);
        for _ in 0..2000 {
            if h.looper.loop_status(n).map(|s| s.phase) == Some(Phase::Playing) {
                return;
            }
            assert!(t.process(PERIOD as u32, &mut b));
            let (mut l, mut r) = ([0.1; PERIOD], [0.1; PERIOD]);
            h.take_pads(&l, &r);
            h.period(&b, &mut l, &mut r);
        }
        panic!(
            "loop {} never played: {:?}, {:?}",
            n + 1,
            h.looper.loop_status(n),
            h.looper.memory()
        );
    }
    let mut h = LoopHost::new(44_100, 60);
    h.command(Command::SetBars(Some(1)));
    h.command(Command::SetLatency(0));
    assert_eq!(h.free_loop(), Ok(0));
    record(&mut h, 0);
    assert_eq!(h.free_loop(), Ok(1), "loop 1 holds a loop");
    h.select(4);
    assert_eq!(h.free_loop(), Ok(4), "the picked one, empty");
    for n in 1..MAX_LOOPS {
        record(&mut h, n);
    }
    assert_eq!(
        h.free_loop().unwrap_err(),
        "8 loops in use: clear one on the Looper tab to record another"
    );
    // Less than a second of memory left: full.
    let mut h = LoopHost::new(44_100, 2);
    h.command(Command::SetBars(Some(1)));
    h.command(Command::SetLatency(0));
    record(&mut h, 0);
    let m = h.looper.memory();
    assert_eq!(
        h.free_loop().unwrap_err(),
        "The loops' memory is full: clear a loop on the Looper tab to record another",
        "{m:?}"
    );
}

/// A loop cell copied onto another audio track:
/// every layer of the loop copies into an empty one, a layer at a
/// time, with the loop's level, off until launched; the copy's loop
/// isn't free meanwhile, and a busy or full looper says why.
#[test]
fn a_whole_loop_copies_into_an_empty_one_a_layer_at_a_time() {
    let mut h = LoopHost::new(44_100, 20);
    let mut t = Transport::new(44_100, 120.0).unwrap();
    let mut b = Block::new();
    h.command(Command::SetBars(Some(1)));
    h.command(Command::SetLatency(0));
    t.command(0, T::Play);
    let mut run = |h: &mut LoopHost, until: &dyn Fn(&LoopHost) -> bool| {
        for _ in 0..4000 {
            if until(h) {
                return;
            }
            assert!(t.process(PERIOD as u32, &mut b));
            let (mut l, mut r) = ([0.1; PERIOD], [0.1; PERIOD]);
            h.take_pads(&l, &r);
            h.period(&b, &mut l, &mut r);
        }
        panic!("never: {:?}", h.looper.loop_status(3));
    };
    let st = |h: &LoopHost, n: usize| h.looper.loop_status(n).unwrap();
    // Loop 1: a first pass and an overdub, at half level.
    h.command(Command::Record);
    run(&mut h, &|h| st(h, 0).phase == Phase::Playing);
    h.command(Command::Overdub);
    run(&mut h, &|h| st(h, 0).phase == Phase::Overdubbing);
    run(&mut h, &|h| st(h, 0).position > 0.9);
    h.command(Command::Overdub);
    run(&mut h, &|h| {
        st(h, 0).phase == Phase::Playing && st(h, 0).layers == 2
    });
    h.command(Command::SetLevel(0.5));
    run(&mut h, &|h| st(h, 0).level == 0.5);

    // Refused: an empty source.
    assert!(h.copy_loop(5, 3).unwrap_err().contains("can't be copied"));

    assert_eq!(
        h.copy_loop(0, 3),
        Ok("looper: loop 1 copies into loop 4, 2 layers".to_string())
    );
    assert_eq!(
        h.copy_loop(0, 5).unwrap_err(),
        "Loop 1 is still copying into Loop 4: wait for it to finish"
    );
    assert_eq!(h.view(120.0).copy_into, Some(3));
    h.select(3);
    assert_eq!(h.free_loop(), Ok(1), "not the copy's loop, though picked");
    h.select(0);
    run(&mut h, &|h| h.copy_end.is_some());
    assert_eq!(
        h.copy_ended(),
        Some(CopyEnd {
            from: 0,
            to: 3,
            layers: 2,
            of: 2
        })
    );
    assert_eq!(h.copy_ended(), None, "once");
    let (a, c) = (st(&h, 0), st(&h, 3));
    assert_eq!(
        (c.phase, c.layers, c.ticks, c.frames),
        (Phase::Playing, 2, a.ticks, a.frames)
    );
    assert_eq!(
        (c.on, c.level),
        (false, 0.5),
        "silent until its cell is launched"
    );
    assert_eq!(h.view(120.0).copy_into, None);
    assert_eq!(h.copy_loop(0, 3).unwrap_err(), "Loop 4 isn't empty");
    for k in 0..2 {
        let (x, y) = (
            h.looper.loop_layer_info(0, k).unwrap(),
            h.looper.loop_layer_info(3, k).unwrap(),
        );
        assert_eq!((x.level, x.pan, x.muted), (y.level, y.pan, y.muted));
    }

    // The memory can't hold another copy and a second's room.
    let mut h = LoopHost::new(44_100, 3);
    let mut t = Transport::new(44_100, 120.0).unwrap();
    h.command(Command::SetBars(Some(1)));
    h.command(Command::SetLatency(0));
    t.command(0, T::Play);
    h.command(Command::Record);
    for _ in 0..4000 {
        if st(&h, 0).phase == Phase::Playing {
            break;
        }
        assert!(t.process(PERIOD as u32, &mut b));
        let (mut l, mut r) = ([0.1; PERIOD], [0.1; PERIOD]);
        h.take_pads(&l, &r);
        h.period(&b, &mut l, &mut r);
    }
    assert!(
        h.copy_loop(0, 1)
            .unwrap_err()
            .starts_with("The loops' memory can't hold a copy of Loop 1 (1 layer)"),
        "{:?}",
        h.looper.memory()
    );
}

/// The undo history goes back and forward by as many steps as asked,
/// as far as there are steps, and more than the command queue holds.
#[test]
fn the_undo_history_steps_back_and_forward() {
    use mpfree_looper::{Edit, Step};
    let mut h = LoopHost::new(44_100, 40);
    let mut t = Transport::new(44_100, 120.0).unwrap();
    let mut b = Block::new();
    h.command(Command::SetBars(Some(1)));
    h.command(Command::SetLatency(0));
    h.command(Command::Record);
    t.command(0, T::Play);
    let mut run = |h: &mut LoopHost, periods: u64| {
        for _ in 0..periods {
            assert!(t.process(PERIOD as u32, &mut b));
            // The pads play on: an overdub of silence leaves no layer.
            h.take_pads(&[0.1; PERIOD], &[0.1; PERIOD]);
            let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
            h.period(&b, &mut l, &mut r);
        }
    };
    run(&mut h, 700);
    assert_eq!(h.view(120.0).status.phase, Phase::Playing);
    // 12 overdubs, 8 deletes and 16 edits (all the looper keeps): more
    // steps than the command queue (32) takes at once.
    for _ in 0..12 {
        h.command(Command::Overdub);
        run(&mut h, 20);
        h.command(Command::Overdub);
        run(&mut h, 20);
    }
    for _ in 0..8 {
        let id = h.view(120.0).layers[1].unwrap().id;
        h.layer_command(id, LayerCommand::Delete);
        run(&mut h, 20);
    }
    for _ in 0..16 {
        h.command(Command::Edit(Edit::Reverse));
        run(&mut h, 4);
    }
    let v = h.view(120.0);
    assert_eq!((v.history.done().len(), v.status.layers), (36, 5));
    assert_eq!(h.steps(1), "looper: undo history: nothing to go forward to");
    assert_eq!(h.steps(-34), "looper: undo history: back 34 steps");
    run(&mut h, 200);
    let v = h.view(120.0);
    assert_eq!((v.history.done().len(), v.history.redo().len()), (2, 34));
    assert!(matches!(v.history.done()[1], Step::Overdub(_)));
    assert_eq!(v.status.layers, 3);
    assert_eq!(h.steps(5), "looper: undo history: forward 5 steps");
    run(&mut h, 50);
    let v = h.view(120.0);
    assert_eq!((v.history.done().len(), v.history.redo().len()), (7, 29));
    // Back further than there are steps goes back to the first pass.
    assert_eq!(h.steps(-99), "looper: undo history: back 7 steps");
    run(&mut h, 50);
    assert_eq!(h.view(120.0).history.done().len(), 0);
    assert_eq!(h.view(120.0).status.layers, 1, "the first pass stays");
}

#[test]
fn a_layer_mutes_by_its_id_and_the_view_shows_it() {
    let mut h = LoopHost::new(44_100, 8);
    let mut t = Transport::new(44_100, 120.0).unwrap();
    let mut b = Block::new();
    h.command(Command::SetBars(Some(1)));
    h.command(Command::SetLatency(0));
    h.command(Command::Record);
    t.command(0, T::Play);
    let mut run = |h: &mut LoopHost, periods: u64| {
        for _ in 0..periods {
            assert!(t.process(PERIOD as u32, &mut b));
            // The pads play on: an overdub of silence leaves no layer.
            h.take_pads(&[0.1; PERIOD], &[0.1; PERIOD]);
            let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
            h.period(&b, &mut l, &mut r);
        }
    };
    run(&mut h, 700);
    assert!(h.command(Command::Overdub).contains("layer 2"));
    run(&mut h, 4);
    h.command(Command::Overdub);
    run(&mut h, 2);
    let v = h.view(120.0);
    assert_eq!(v.status.layers, 2);
    let second = v.layers[1].expect("two layers");
    assert!(v.layers[2].is_none());
    assert!(!second.muted && second.audible);
    assert_eq!(
        h.layer_command(second.id, LayerCommand::Mute(true)),
        "looper: layer 2 muted"
    );
    run(&mut h, 1);
    let v = h.view(120.0);
    let (first, second) = (v.layers[0].unwrap(), v.layers[1].unwrap());
    assert!(second.muted && !second.audible);
    assert!(!first.muted && first.audible);
    assert_eq!(
        h.layer_command(second.id, LayerCommand::Mute(false)),
        "looper: layer 2 unmuted"
    );
    run(&mut h, 1);
    assert!(h.view(120.0).layers[1].unwrap().audible);
}

/// 13 Nov live test: the first pass's × said "deleted" and the loop
/// stayed. The last layer's × now clears the loop; a loop card's
/// speaker mutes that loop without picking it, and a muted loop stays
/// silent when PLAY starts.
#[test]
fn the_last_layers_x_clears_and_a_loop_mutes_without_being_picked() {
    let mut h = LoopHost::new(44_100, 8);
    let mut t = Transport::new(44_100, 120.0).unwrap();
    let mut b = Block::new();
    h.command(Command::SetBars(Some(1)));
    h.command(Command::SetLatency(0));
    h.command(Command::Record);
    t.command(0, T::Play);
    let mut run = |h: &mut LoopHost, periods: u64| {
        for _ in 0..periods {
            assert!(t.process(PERIOD as u32, &mut b));
            // The pads play on: an overdub of silence leaves no layer.
            h.take_pads(&[0.1; PERIOD], &[0.1; PERIOD]);
            let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
            h.period(&b, &mut l, &mut r);
        }
    };
    run(&mut h, 700);
    // Loop 2 records too, and loop 1 is picked again.
    h.select(1);
    h.command(Command::SetBars(Some(1)));
    h.command(Command::Record);
    run(&mut h, 1400);
    assert_eq!(h.view(120.0).loops[1].phase, Phase::Playing);
    h.select(0);
    run(&mut h, 1);
    // Loop 2's speaker: muted, and loop 1 stays picked.
    assert_eq!(
        h.toggle_mute(Some(1)),
        "looper: loop 2 muted: silent, PLAY doesn't start it"
    );
    run(&mut h, 2);
    let v = h.view(120.0);
    assert!(v.loops[1].muted && !v.loops[0].muted);
    assert_eq!(v.selected, 0);
    assert_eq!(
        h.toggle_mute(Some(2)),
        "looper: loop 3: empty, nothing to mute"
    );
    // The Mute button: the picked loop's own mute.
    assert!(h.toggle_mute(None).contains("loop 1 muted"));
    run(&mut h, 2);
    assert!(h.view(120.0).loops[0].muted);
    h.toggle_mute(None);
    run(&mut h, 2);
    // Loop 1 has one layer: its × clears it.
    let v = h.view(120.0);
    assert_eq!(v.status.layers, 1);
    let only = v.layers[0].unwrap();
    let line = h.layer_command(only.id, LayerCommand::Delete);
    assert_eq!(
        line,
        "looper: Clear: the loop fades out (1 layers); UNDO brings it back"
    );
    run(&mut h, 20);
    let v = h.view(120.0);
    assert_eq!(v.loops[0].phase, Phase::Empty);
    assert_eq!(v.loops[1].phase, Phase::Playing, "the other loop stays");
}

/// The pads' sound for song frame `n`: different every frame.
fn pads_at(n: u64) -> (f32, f32) {
    ((n % 1000) as f32 / 2000.0, -((n % 777) as f32) / 2000.0)
}

/// Recording the pads: the loop plays back what the pads played in the
/// first bar, to the frame, in the bars after it. With the source on
/// the line input (closed), the same pads record silence.
#[test]
fn the_pads_are_recorded_in_time_and_the_line_source_ignores_them() {
    for source in LoopSource::ALL {
        let mut h = LoopHost::new(44_100, 20);
        let _ = h.set_source(source);
        let mut t = Transport::new(44_100, 120.0).unwrap();
        let mut b = Block::new();
        h.command(Command::SetBars(Some(1)));
        h.command(Command::SetLatency(0));
        h.command(Command::Record);
        t.command(0, T::Play);
        const BAR: u64 = 88_200;
        let mut checked = 0;
        for p in 0..(2 * BAR / PERIOD as u64 + 2) {
            assert!(t.process(PERIOD as u32, &mut b));
            let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
            for f in 0..PERIOD {
                let n = p * PERIOD as u64 + f as u64;
                if n < BAR {
                    (l[f], r[f]) = pads_at(n);
                }
            }
            h.take_pads(&l, &r);
            h.period(&b, &mut l, &mut r);
            // The second bar, away from the loop point's fades.
            for f in 0..PERIOD {
                let n = p * PERIOD as u64 + f as u64;
                if (BAR + 1_000..2 * BAR - 1_000).contains(&n) {
                    let want = if source == LoopSource::Pads {
                        pads_at(n - BAR)
                    } else {
                        (0.0, 0.0)
                    };
                    assert!(
                        (l[f] - want.0).abs() < 1e-6 && (r[f] - want.1).abs() < 1e-6,
                        "{source:?} frame {n}: {:?}, want {want:?}",
                        (l[f], r[f])
                    );
                    checked += 1;
                }
            }
        }
        assert_eq!(checked, BAR - 2_000);
        assert_eq!(h.looper.status().phase, Phase::Playing);
        assert!(h
            .describe()
            .contains(&format!("records from {}", source.name().to_lowercase())));
    }
}

/// Capture: the pads played over three bars at 120 BPM; the last two
/// whole bars become the picked empty loop (its Length), playing them
/// in time; with it playing, the next Capture goes into a free loop.
/// What you played shows the bar lines and what Capture takes, and
/// the deck's button says it. A change of source forgets what was
/// heard before.
#[test]
fn capture_turns_the_last_bars_played_into_a_loop() {
    const BAR: u64 = 88_200;
    let mut h = LoopHost::new(44_100, 20);
    let mut t = Transport::new(44_100, 120.0).unwrap();
    let mut b = Block::new();
    h.command(Command::SetLatency(0));
    t.command(0, T::Play);
    // The song's frame is the output's length.
    let mut run = |h: &mut LoopHost, to: u64, out: &mut Vec<(f32, f32)>| {
        while (out.len() as u64) < to {
            let frame = out.len() as u64;
            assert!(t.process(PERIOD as u32, &mut b));
            let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
            for f in 0..PERIOD {
                (l[f], r[f]) = pads_at(frame + f as u64);
            }
            h.take_pads(&l, &r);
            let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
            h.period(&b, &mut l, &mut r);
            out.extend(l.iter().copied().zip(r.iter().copied()));
        }
    };
    let mut out = Vec::new();
    let frame = |out: &Vec<(f32, f32)>| out.len() as u64;
    // Nothing heard on the bars yet.
    assert!(
        h.capture().contains("no whole bar heard yet"),
        "{}",
        h.capture()
    );
    // Bar 4, beat 2.
    run(&mut h, 3 * BAR + 30_000, &mut out);
    let v = h.view(120.0);
    assert_eq!(v.capture_into, Some((0, 2)), "the picked loop, its Length");
    let lines: Vec<i64> = v.heard.lines[..v.heard.n_lines]
        .iter()
        .map(|l| l.0)
        .collect();
    assert_eq!(lines, [1, 2, 3, 4], "the bar lines heard");
    let (a, b2, first, bars) = v.heard.take.expect("a take");
    assert_eq!((first, bars), (2, 2), "bars 2 and 3");
    assert!(a < b2 && b2 < 1.0);
    let button = crate::looper_page::capture_button(&v);
    assert_eq!(
        (button.label.as_str(), button.works),
        ("Capture last 2 bars", true)
    );
    assert_eq!(h.capture(), "looper: Capture: bars 2 and 3 become Loop 1");
    // It copies in at 64 times the speed it plays: about 2 800 frames.
    run(&mut h, frame(&out) + 3_200, &mut out);
    let s = h.looper.status();
    assert_eq!(
        (s.phase, s.frames, s.origin),
        (Phase::Playing, 2 * BAR as usize, 3840)
    );
    // It plays bars 2 and 3 in time: song frame n plays what the pads
    // played at the same place in those bars (past the fade in).
    let from = out.len();
    run(&mut h, frame(&out) + BAR, &mut out);
    for (n, y) in out.iter().enumerate().skip(from + 512) {
        let want = pads_at(BAR + (n as u64 - BAR) % (2 * BAR));
        assert!(
            (y.0 - want.0).abs() < 1e-6 && (y.1 - want.1).abs() < 1e-6,
            "frame {n}: {y:?}, want {want:?}"
        );
    }
    // The picked loop plays: Capture goes into Loop 2, one bar asked.
    assert!(h.select(1).contains("picked"));
    h.command(Command::SetBars(Some(1)));
    run(&mut h, frame(&out) + 256, &mut out);
    assert!(h.select(0).contains("picked"));
    run(&mut h, frame(&out) + 256, &mut out);
    assert_eq!(h.view(120.0).capture_into, Some((1, 1)));
    let said = h.capture();
    assert!(said.ends_with("become Loop 2"), "{said}");
    run(&mut h, frame(&out) + 4_096, &mut out);
    let two = h.looper.loop_status(1).unwrap();
    assert_eq!((two.phase, two.frames), (Phase::Playing, BAR as usize));
    // Into Loop 3; Record on it (its ring, REC) while it copies in
    // calls it off, as Capture again does.
    assert!(h.capture().ends_with("become Loop 3"));
    run(&mut h, frame(&out) + PERIOD as u64, &mut out);
    assert!(h.looper.loop_status(2).unwrap().capturing.is_some());
    assert_eq!(
        h.command(Command::Record),
        "looper: Capture into Loop 3 called off"
    );
    run(&mut h, frame(&out) + 4_096, &mut out);
    let three = h.looper.loop_status(2).unwrap();
    assert_eq!((three.phase, three.capturing), (Phase::Empty, None));
    // Another source: what was heard before is forgotten.
    let _ = h.set_source(LoopSource::Line);
    run(&mut h, frame(&out) + 256, &mut out);
    assert!(h.capture().contains("no whole bar heard yet"));
    assert!(h.view(120.0).heard.take.is_none());
}

/// What you played's window: with a 2-bar pattern it grows by two
/// bars, and shows room for four patterns; moved back, it takes bars
/// that are long past "the last bars", which play in time; Capture,
/// or a new Length, lets go of it; at the newest bar line it keeps up.
#[test]
fn capture_window_moves_back_and_grows_by_the_pattern() {
    const BAR: u64 = 88_200;
    let mut h = LoopHost::new(44_100, 40);
    let mut t = Transport::new(44_100, 120.0).unwrap();
    let mut b = Block::new();
    h.command(Command::SetLatency(0));
    h.command(Command::SetBars(Some(PATTERN_BARS)));
    h.set_pattern_bars(Some(2));
    t.command(0, T::Play);
    let mut run = |h: &mut LoopHost, to: u64, out: &mut Vec<(f32, f32)>| {
        while (out.len() as u64) < to {
            let frame = out.len() as u64;
            assert!(t.process(PERIOD as u32, &mut b));
            let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
            for f in 0..PERIOD {
                (l[f], r[f]) = pads_at(frame + f as u64);
            }
            h.take_pads(&l, &r);
            let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
            h.period(&b, &mut l, &mut r);
            out.extend(l.iter().copied().zip(r.iter().copied()));
        }
    };
    let mut out = Vec::new();
    // The app follows the pattern every few periods.
    run(&mut h, 1_024, &mut out);
    h.set_pattern_bars(Some(2));
    // Bar 10, beat 2.
    run(&mut h, 9 * BAR + 30_000, &mut out);
    let v = h.view(120.0);
    assert_eq!((v.heard.unit, v.heard.pattern), (2, true));
    assert_eq!(v.capture_into, Some((0, 2)), "a pattern's length");
    let lines: Vec<i64> = v.heard.lines[..v.heard.n_lines]
        .iter()
        .map(|l| l.0)
        .collect();
    assert_eq!(
        lines,
        (3..=10).collect::<Vec<_>>(),
        "8 bars: room for 4 patterns"
    );
    // Two patterns, back to bar 8's line: bars 4 to 7.
    assert_eq!(
        h.set_capture_span(Some(8), 4),
        "looper: Capture window: bars 4 to 7"
    );
    let v = h.view(120.0);
    assert_eq!(v.capture_into, Some((0, 4)));
    let (a, b2, first, bars) = v.heard.take.expect("a take");
    assert_eq!((first, bars), (4, 4));
    assert!(0.0 < a && a < b2 && b2 < 1.0, "{a} {b2}");
    assert_eq!(h.capture(), "looper: Capture: bars 4 to 7 become Loop 1");
    assert_eq!(
        h.view(120.0).capture_into,
        Some((0, 2)),
        "let go of: the pattern again"
    );
    run(&mut h, out.len() as u64 + 8_192, &mut out);
    let s = h.looper.status();
    assert_eq!(
        (s.phase, s.frames, s.origin),
        (Phase::Playing, 4 * BAR as usize, 3 * 3840)
    );
    let from = out.len();
    run(&mut h, out.len() as u64 + BAR, &mut out);
    for (n, y) in out.iter().enumerate().skip(from + 512) {
        let want = pads_at(3 * BAR + (n as u64 - 3 * BAR) % (4 * BAR));
        assert!(
            (y.0 - want.0).abs() < 1e-6 && (y.1 - want.1).abs() < 1e-6,
            "frame {n}: {y:?}, want {want:?}"
        );
    }
    // Let go of after the Capture: Loop 2 takes the pattern again.
    h.select(1);
    run(&mut h, out.len() as u64 + 256, &mut out);
    assert_eq!(h.view(120.0).capture_into, Some((1, 2)));
    // At the newest bar line it keeps up: the last 6 bars.
    let newest = {
        let v = h.view(120.0);
        v.heard.lines[v.heard.n_lines - 1].0
    };
    assert_eq!(
        h.set_capture_span(Some(newest), 6),
        "looper: Capture window: the last 6 bars"
    );
    let first = h.view(120.0).heard.take.unwrap().2;
    run(&mut h, out.len() as u64 + BAR, &mut out);
    assert_eq!(
        h.view(120.0).heard.take.unwrap().2,
        first + 1,
        "it moves on"
    );
    // A bar line not kept leaves it; a new Length lets go of it.
    assert!(h.set_capture_span(Some(999), 2).contains("isn't kept"));
    assert_eq!(h.view(120.0).capture_into, Some((1, 6)));
    h.command(Command::SetBars(Some(1)));
    run(&mut h, out.len() as u64 + 256, &mut out);
    assert_eq!(h.view(120.0).capture_into, Some((1, 1)));
}

/// The pads from the mix come a period late (the looper runs before
/// the mixer): the looper makes it up, whatever latency the app asks
/// for (that is the line input's), so the loop is in time to the
/// frame. The source lost is silence, never another sound, said once,
/// and a take won't start on it (said on the screen); back again,
/// said once.
#[test]
fn the_pads_from_the_mix_are_recorded_in_time_though_a_period_late() {
    let (mut h, mut t, mut b) = recorded_in_time_a_period_late(LoopSource::Pads);
    // Dry: said once, and the latency goes to 0 with it.
    let z = [0.0; PERIOD];
    let why = "the Loop track goes to a submix";
    let said = |s: &str| Some((s.to_string(), false));
    assert_eq!(
        h.take_pads_as(&z, &z, Err(why)),
        said(
            "looper: records silence, the pads can't be recorded: the Loop track goes to a submix"
        )
    );
    assert_eq!(h.take_pads_as(&z, &z, Err(why)), None);
    assert_eq!(h.looper.status().phase, Phase::Playing);
    let no = Some("Loop 1 doesn't record: the Loop track goes to a submix".to_string());
    assert_eq!(h.refuse(Command::Overdub), no);
    assert_eq!(h.refuse(Command::Record), no);
    assert_eq!(h.refuse(Command::Undo), None, "only what starts a take");
    let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
    assert!(t.process(PERIOD as u32, &mut b));
    h.period(&b, &mut l, &mut r);
    assert!(t.process(PERIOD as u32, &mut b));
    h.period(&b, &mut l, &mut r);
    assert_eq!(h.looper.status().latency, 0);
    assert_eq!(
        h.take_pads_as(&z, &z, Ok(0)),
        said("looper: hears the pads again")
    );
    assert_eq!(h.refuse(Command::Overdub), None);
    assert_eq!(h.dropouts, 0, "no take recorded then");
}

/// Length: Pattern follows the pattern playing while the loop is
/// empty, and holds once it is armed: a clip launched between the arm
/// and the start doesn't change the take's length.
#[test]
fn length_pattern_holds_once_the_loop_is_armed() {
    let mut h = LoopHost::new(44_100, 8);
    let mut t = Transport::new(44_100, 120.0).unwrap();
    let mut b = Block::new();
    let mut period = |h: &mut LoopHost| {
        assert!(t.process(PERIOD as u32, &mut b));
        let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
        h.period(&b, &mut l, &mut r);
    };
    h.command(Command::SetBars(Some(PATTERN_BARS)));
    period(&mut h);
    h.set_pattern_bars(Some(4));
    period(&mut h);
    assert_eq!(h.looper.status().bars, Some(4));
    h.set_pattern_bars(Some(2));
    period(&mut h);
    assert_eq!(h.looper.status().bars, Some(2), "empty: it follows");
    h.command(Command::Record);
    period(&mut h);
    assert_eq!(h.looper.status().phase, Phase::Armed);
    h.set_pattern_bars(Some(8));
    period(&mut h);
    assert_eq!(h.looper.status().bars, Some(2), "armed: it holds");
}

/// A take records one source from start to end (the design review of
/// 20 Nov, point 6): the pads moved to another instrument, or another
/// Records from picked, while it records wait for its end, said once
/// on the screen too, and the Looper tab shows what this take records
/// beside the pick; then the pick takes over and What you played
/// starts again. A source lost while a take records is a gap of
/// silence in it: counted, said on the screen, and its return too.
#[test]
fn a_take_holds_its_source_and_instrument_to_its_end() {
    use mpfree_fx::StripId;
    let mut h = LoopHost::new(44_100, 8);
    let mut t = Transport::new(44_100, 120.0).unwrap();
    let mut b = Block::new();
    assert_eq!(
        h.follow_pads(Instrument::Kit(0)),
        Some(("looper: records the pads (Kit 1)".to_string(), false))
    );
    assert_eq!(h.follow_pads(Instrument::Kit(0)), None, "said once");
    h.command(Command::SetBars(Some(1)));
    h.command(Command::Record);
    t.command(0, T::Play);
    let z = [0.0; PERIOD];
    let mut period = |h: &mut LoopHost, how: Result<u32, &'static str>| {
        assert!(t.process(PERIOD as u32, &mut b));
        let (mut l, mut r) = (z, z);
        let said = h.take_pads_as(&z, &z, how);
        h.period(&b, &mut l, &mut r);
        said
    };
    for _ in 0..4 {
        period(&mut h, Ok(0));
    }
    assert_eq!(h.looper.status().phase, Phase::Recording);
    // Play on the pads mid-take: the take keeps Kit 1.
    let held = "looper: this take keeps recording the pads (Kit 1); \
                the pads (the synth) from the next take";
    assert_eq!(
        h.follow_pads(Instrument::Synth),
        Some((held.to_string(), true))
    );
    assert_eq!(h.follow_pads(Instrument::Synth), None, "said once");
    assert_eq!(h.takes_from(), (LoopSource::Pads, Some(Instrument::Kit(0))));
    // The master picked mid-take: the take keeps the pads too.
    let (line, tell) = h.set_source(LoopSource::Mix(StripId::Master)).unwrap();
    assert!(
        tell && line.ends_with("; the master from the next take"),
        "{line}"
    );
    assert_eq!(h.takes_from(), (LoopSource::Pads, Some(Instrument::Kit(0))));
    let v = h.view(120.0);
    assert_eq!(
        (v.source, v.taking_from),
        (LoopSource::Mix(StripId::Master), Some(LoopSource::Pads))
    );
    assert!(
        h.set_source(LoopSource::Pads).unwrap().1,
        "back: said again"
    );
    assert_eq!(h.view(120.0).taking_from, None, "the same source");
    // The channel lost mid-take: a gap of silence, said and counted.
    let why = "that strip isn't in the mix";
    assert_eq!(
        period(&mut h, Err(why)),
        Some((
            "looper: the take records silence from here, the pads (Kit 1) \
             can't be recorded: that strip isn't in the mix"
                .to_string(),
            true
        ))
    );
    assert_eq!(period(&mut h, Err(why)), None, "said once");
    assert_eq!(h.dropouts, 1);
    assert_eq!(
        period(&mut h, Ok(0)),
        Some((
            "looper: the take hears the pads (Kit 1) again".to_string(),
            true
        ))
    );
    assert!(h.describe().contains("1 times a take's source was lost"));
    for _ in 0..(88_200 / PERIOD + 2) {
        period(&mut h, Ok(0));
    }
    assert_eq!(h.looper.status().phase, Phase::Playing);
    // The take ended: the synth takes over.
    assert_eq!(
        h.follow_pads(Instrument::Synth),
        Some((
            "looper: records the pads (the synth), What you played starts again".to_string(),
            false
        ))
    );
    assert_eq!(h.takes_from(), (LoopSource::Pads, Some(Instrument::Synth)));
    // Records from picked between takes changes at once.
    let (line, tell) = h.set_source(LoopSource::Line).unwrap();
    assert!(
        !tell && line.starts_with("looper: records the line input"),
        "{line}"
    );
    assert_eq!(h.takes_from(), (LoopSource::Line, None));
}

/// The pads named in the log, silence on MIDI out (said
/// once, not again as a mix lost), and nothing while another source
/// records.
#[test]
fn pads_say_which_instrument_they_record_and_silence_on_midi_out() {
    let mut h = LoopHost::new(44_100, 8);
    assert_eq!(
        h.follow_pads(Instrument::Own(2)).map(|s| s.0).as_deref(),
        Some("looper: records the pads (Track 3's own instrument)")
    );
    assert_eq!(
        h.follow_pads(Instrument::MidiOut).map(|s| s.0).as_deref(),
        Some(
            "looper: records silence (the pads play MIDI out, which sounds outside mpfree), \
             What you played starts again"
        )
    );
    let z = [0f32; PERIOD];
    assert_eq!(h.take_pads_as(&z, &z, Err(PADS_ON_MIDI_OUT)), None);
    let _ = h.set_source(LoopSource::Line);
    assert_eq!(
        h.follow_pads(Instrument::Synth),
        None,
        "another source: nothing to say"
    );
    assert_eq!(h.takes_from(), (LoopSource::Line, None));
}

#[test]
fn a_mix_is_recorded_in_time_like_the_pads() {
    let strip = mpfree_fx::StripId::Bus(0);
    let (mut h, _, _) = recorded_in_time_a_period_late(LoopSource::Mix(strip));
    assert_eq!(h.takes_from(), (LoopSource::Mix(strip), None));
    assert_eq!(h.first_note(), "(the first pad you play)");
    let z = [0.0; PERIOD];
    let why = "that strip isn't in the mix";
    let name = crate::mixing::name(strip);
    assert_eq!(
        h.take_pads_as(&z, &z, Err(why)).map(|s| s.0),
        Some(format!(
            "looper: records silence, {name} can't be recorded: {why}"
        ))
    );
    assert_eq!(
        h.take_pads_as(&z, &z, Ok(0)).map(|s| s.0),
        Some(format!("looper: hears {name} again"))
    );
}

/// A loop of a bar recorded from `source` (the pads or a mix), handed
/// over a period late as the mixer has it: the loop plays it back in
/// time to the frame. The host, the transport and its block, for more.
fn recorded_in_time_a_period_late(source: LoopSource) -> (LoopHost, Transport, Block) {
    let mut h = LoopHost::new(44_100, 20);
    let _ = h.set_source(source);
    let mut t = Transport::new(44_100, 120.0).unwrap();
    let mut b = Block::new();
    h.command(Command::SetBars(Some(1)));
    // The app's latency on the pads: the pads' own stands.
    h.command(Command::SetLatency(512));
    h.command(Command::Record);
    t.command(0, T::Play);
    const BAR: u64 = 88_200;
    let late = PERIOD as u64;
    let mut checked = 0;
    for p in 0..(2 * BAR / PERIOD as u64 + 2) {
        assert!(t.process(PERIOD as u32, &mut b));
        let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
        for f in 0..PERIOD {
            let n = (p * PERIOD as u64 + f as u64).checked_sub(late);
            if let Some(n) = n.filter(|&n| n < BAR) {
                (l[f], r[f]) = pads_at(n);
            }
        }
        assert_eq!(h.take_pads_as(&l, &r, Ok(0)), None, "the mix all along");
        h.period(&b, &mut l, &mut r);
        for f in 0..PERIOD {
            let n = p * PERIOD as u64 + f as u64;
            if (BAR + 1_000..2 * BAR - 1_000).contains(&n) {
                let want = pads_at(n - BAR);
                assert!(
                    (l[f] - want.0).abs() < 1e-6 && (r[f] - want.1).abs() < 1e-6,
                    "frame {n}: {:?}, want {want:?}",
                    (l[f], r[f])
                );
                checked += 1;
            }
        }
    }
    assert_eq!(checked, BAR - 2_000);
    assert_eq!(h.looper.status().latency, PERIOD as u32);
    (h, t, b)
}

/// The audio thread's part: handing over the pads, a period, the log
/// check and the wave step allocate nothing while nothing changes.
/// A loop recorded at 120 BPM, then the tempo at 100: it plays like
/// tape (lower) until the worker's print is in, then at its own pitch
/// and the new length; Overdub then fits it to the tempo.
#[test]
fn a_loop_at_another_tempo_gets_a_print_and_keeps_its_pitch() {
    let mut h = LoopHost::new(44_100, 30);
    let mut t = Transport::new(44_100, 120.0).unwrap();
    let mut b = Block::new();
    h.command(Command::SetBars(Some(1)));
    h.command(Command::Record);
    t.command(0, T::Play);
    // 441 Hz: 100 frames a cycle, 882 to the bar.
    let sine = |f: u64| 0.5 * (f as f64 * std::f64::consts::TAU / 100.0).sin() as f32;
    let mut f = 0u64;
    let mut period = |h: &mut LoopHost, t: &mut Transport, b: &mut Block, rec: bool| {
        assert!(t.process(PERIOD as u32, b));
        let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
        if rec {
            for i in 0..PERIOD {
                l[i] = sine(f + i as u64);
                r[i] = l[i];
            }
        }
        h.take_pads(&l, &r);
        let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
        h.period(b, &mut l, &mut r);
        f += PERIOD as u64;
        l
    };
    while h.looper.status().phase != Phase::Playing {
        period(&mut h, &mut t, &mut b, true);
    }
    assert_eq!(h.looper.status().frames, 88_200);
    t.command(t.frame(), T::SetTempo(100.0));
    // The pitch, from the first and last upward zero crossing.
    let ups = |out: &[f32]| {
        let x: Vec<usize> = (1..out.len())
            .filter(|&i| out[i - 1] < 0.0 && out[i] >= 0.0)
            .collect();
        (x.len() - 1) as f64 * 44_100.0 / (x[x.len() - 1] - x[0]) as f64
    };
    // Like tape at first, once the tempo has changed (on the next 16th,
    // up to 5,513 frames on).
    for _ in 0..50 {
        period(&mut h, &mut t, &mut b, false);
    }
    let mut out = Vec::new();
    for _ in 0..30 {
        out.extend(period(&mut h, &mut t, &mut b, false));
    }
    assert!(!h.looper.status().printed);
    let tape = ups(&out);
    assert!((tape - 367.5).abs() < 2.0, "tape {tape}");
    let start = std::time::Instant::now();
    while !h.looper.status().printed {
        assert!(
            start.elapsed().as_secs() < 120,
            "no print: {}",
            h.describe()
        );
        period(&mut h, &mut t, &mut b, false);
        std::thread::sleep(std::time::Duration::from_micros(200));
    }
    let mut out = Vec::new();
    for _ in 0..400 {
        out.extend(period(&mut h, &mut t, &mut b, false));
    }
    let kept = ups(&out[1_000..]);
    assert!((kept - 441.0).abs() < 2.0, "print {kept}");
    assert!(h.describe().contains("a print plays"), "{}", h.describe());
    assert!(h.describe().contains("prints 1 made"), "{}", h.describe());
    // Overdub fits the loop to the tempo first.
    assert!(h
        .command(Command::Overdub)
        .contains("once the loop is fitted"));
    for _ in 0..40 {
        period(&mut h, &mut t, &mut b, false);
    }
    let s = h.looper.status();
    assert_eq!((s.phase, s.frames), (Phase::Overdubbing, 105_840));
}

/// The opening thread's own counts from the kernel (Linux; None
/// elsewhere): its voluntary context switches, the times it slept,
/// waiting for something (in /proc/thread-self/status; a thread taken
/// off its core for another is not counted), and its page faults,
/// minor and major (in /proc/thread-self/stat). The files stay open,
/// so a read is one seek and one read, no path to look up.
struct Counts {
    status: Option<std::fs::File>,
    stat: Option<std::fs::File>,
    text: String,
}

impl Counts {
    fn open() -> Self {
        let open = |p: &str| std::fs::File::open(p).ok();
        Counts {
            status: open("/proc/thread-self/status"),
            stat: open("/proc/thread-self/stat"),
            text: String::with_capacity(4096),
        }
    }

    fn fill(f: &mut std::fs::File, text: &mut String) -> Option<()> {
        use std::io::{Read, Seek, SeekFrom};
        text.clear();
        f.seek(SeekFrom::Start(0)).ok()?;
        f.read_to_string(text).ok().map(|_| ())
    }

    fn sleeps(&mut self) -> Option<u64> {
        Self::fill(self.status.as_mut()?, &mut self.text)?;
        self.text
            .lines()
            .find_map(|l| l.strip_prefix("voluntary_ctxt_switches:"))?
            .trim()
            .parse()
            .ok()
    }

    /// Fields 10 and 12 of stat (minflt, majflt), counted from the
    /// state after the command's name in parentheses (field 3).
    fn faults(&mut self) -> Option<u64> {
        Self::fill(self.stat.as_mut()?, &mut self.text)?;
        let mut f = self.text[self.text.rfind(')')? + 1..].split_whitespace();
        let minor: u64 = f.nth(7)?.parse().ok()?;
        let major: u64 = f.nth(1)?.parse().ok()?;
        Some(minor + major)
    }
}

/// Smart loop end step 3, Auto by ear while long prints run: a print
/// of a 32 s loop runs over and over on up to four cores at the print
/// worker's idle priority while a free first loop records two bars of
/// a beat from the mix and closes on Auto, the periods paced as they
/// play. The finder, on its own thread at the normal priority, answers
/// in time and the loop closes on the downbeat it heard, or Auto keeps
/// your tap (counted, and said). Either way no period waits for it:
/// the looper counts no ask that waited, a period that speaks to the
/// finder never sleeps (its thread's voluntary context switches,
/// which a busy machine taking its core away doesn't add to; judged
/// natively, see below), and no period allocates.
#[test]
fn auto_by_ear_while_long_prints_run_never_waits_for_the_finder() {
    by_ear_while_long_prints_run(true);
}

/// The same with a finder that never answers, as on a machine too slow
/// for it: Auto keeps your tap at the end of its wait, says why, and
/// starts the song at the tap's tempo; no period sleeps or allocates.
#[test]
fn auto_by_ear_keeps_the_tap_when_the_finder_is_too_slow_while_long_prints_run() {
    by_ear_while_long_prints_run(false);
}

/// The two tests above: `answers` false leaves the finder's answers
/// unread (they never come back).
fn by_ear_while_long_prints_run(answers: bool) {
    use mpfree_fx::StripId;
    use mpfree_looper::hits::kit::Kit;
    use mpfree_looper::Bounce;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::time::{Duration, Instant};
    const RATE: u32 = 44_100;
    const TWO_BARS: u64 = 176_400;
    // The prints: 16 bars at 120 printed at 100, a chunk at a time as
    // the worker runs them, counted.
    let stop = Arc::new(AtomicBool::new(false));
    let chunks = Arc::new(AtomicU64::new(0));
    let frames = 32 * RATE as usize;
    let audio: Vec<f32> = (0..2 * frames)
        .map(|i| 0.3 * ((i / 2) as f64 * std::f64::consts::TAU / 100.0).sin() as f32)
        .collect();
    let bounce = Arc::new(Bounce {
        from_loop: 0,
        audio,
        frames,
        ticks: 64 * i64::from(PPQ),
        origin: 0,
        complete: true,
        version: 1,
        dry: true,
        rate: 0,
    });
    // (Well under the cores: the test suite runs beside it.)
    let cores = std::thread::available_parallelism().map_or(2, |n| n.get());
    let printers: Vec<_> = (0..(cores / 4).clamp(1, 4))
        .map(|_| {
            let (stop, chunks, bounce) = (stop.clone(), chunks.clone(), bounce.clone());
            std::thread::spawn(move || {
                mpfree_lanes::os::idle_job();
                while !stop.load(Ordering::Relaxed) {
                    let mut j = PrintJob::new(&bounce, RATE, frames * 6 / 5).unwrap();
                    while !stop.load(Ordering::Relaxed) && !j.run(1 << 15) {
                        chunks.fetch_add(1, Ordering::Relaxed);
                    }
                }
            })
        })
        .collect();
    // The prints are under way before REC. (Under qemu, or with every
    // core busy, a print's first chunk can take seconds.)
    let wait = Instant::now();
    while chunks.load(Ordering::Relaxed) == 0 {
        assert!(wait.elapsed() < Duration::from_secs(120), "no print ran");
        std::thread::sleep(Duration::from_millis(1));
    }
    let mut h = LoopHost::new(RATE, 20);
    assert!(!h.finder.inline && !h.finder.gone, "its thread");
    // (The answers left unread: the finder's thread goes on, as a slow
    // one would, and none comes back.)
    let _unread = (!answers).then(|| h.finder.back.take());
    // (At the beat's tempo: when the finder answers, the period that
    // closes the loop wants no print.)
    let mut t = Transport::new(RATE, 120.0).unwrap();
    let mut b = Block::new();
    let _ = h.set_source(LoopSource::Mix(StripId::Master));
    h.set_tempo_from_loop(true);
    h.set_smart_end(SmartEnd::Auto);
    h.command(Command::SetBars(None));
    let period = |h: &mut LoopHost, t: &mut Transport, b: &mut Block, l: &[f32; PERIOD]| {
        assert!(t.process(PERIOD as u32, b));
        h.take_pads_as(l, l, Ok(0));
        let (mut ol, mut or) = ([0.0; PERIOD], [0.0; PERIOD]);
        h.period(b, &mut ol, &mut or);
    };
    period(&mut h, &mut t, &mut b, &[0.0; PERIOD]);
    // Two bars of the beat at 120, 80 ms after REC; REC again 40 ms
    // after its next downbeat.
    h.command(Command::Record);
    let rec = t.frame();
    let early = 3_528u64;
    let mut audio = vec![0.0f32; 2 * (TWO_BARS as usize + RATE as usize)];
    Kit::new(RATE).beat(&mut audio, early as usize, 120.0, 2, &|_| 0.0);
    let tap = rec + early + TWO_BARS + 1_764;
    let (mut tapped, mut printing_at_ask, mut started, mut rec_from) = (false, None, None, None);
    let (mut took, mut allocs) = (Vec::new(), 0);
    // Sleeps inside the periods, with their page faults: in those
    // that spoke to the finder (asked it, or looked for its answer)
    // and in the others. And outside them (the pacing: the count is
    // read for real).
    let mut counts = Counts::open();
    let (mut near, mut away, mut read, mut slept) = (Vec::new(), Vec::new(), true, 0u64);
    let (mut faulting, mut spoke) = ((0u32, 0u64), 0u32);
    let mut last = counts.sleeps();
    let beat = Duration::from_secs_f64(PERIOD as f64 / f64::from(RATE));
    let t0 = Instant::now();
    let mut p = 0u32;
    while t.frame() < tap + RATE as u64 / 2 {
        std::thread::sleep((t0 + beat * p).saturating_duration_since(Instant::now()));
        p += 1;
        if !tapped && t.frame() + PERIOD as u64 > tap {
            h.command(Command::Record);
            tapped = true;
        }
        let mut l = [0.0; PERIOD];
        for (i, x) in l.iter_mut().enumerate() {
            let at = (t.frame() - rec) as usize + i;
            *x = audio.get(2 * at).copied().unwrap_or(0.0);
        }
        let (now, from) = (Instant::now(), t.frame());
        let out = h.finder.job.is_none();
        let faults = counts.faults();
        let before = counts.sleeps();
        let ((), calls) = crate::sys::alloc_count::calls(|| period(&mut h, &mut t, &mut b, &l));
        let after = counts.sleeps();
        let faulted = counts.faults();
        allocs += calls;
        match (last, before, after, faults, faulted) {
            (Some(l), Some(b), Some(a), Some(f), Some(g)) => {
                slept += b - l;
                if g > f {
                    faulting = (faulting.0 + 1, faulting.1 + (g - f));
                }
                // (It sent the job, or looked for the answer: the
                // job out, and its answers read.)
                let finder = (!out && h.finder.job.is_none()) || (out && h.finder.back.is_some());
                spoke += u32::from(finder);
                if a > b {
                    (if finder { &mut near } else { &mut away }).push((p, a - b, g - f));
                }
            }
            _ => read = false,
        }
        last = after;
        if let Some((_, r, at, into)) = h.take_tempo() {
            let pos = t.position();
            let start = pos - pos.rem_euclid(t.meter().bar_ticks()) + into;
            t.command(at, T::Locate(start));
            t.command(at, T::SetTempo(r.bpm));
            t.command(at, T::Play);
            started = Some(r.bpm);
        }
        took.push(now.elapsed());
        if rec_from.is_none() && h.looper.status().phase == Phase::Recording {
            rec_from = Some(from);
        }
        if printing_at_ask.is_none() && h.finder.job.is_none() {
            printing_at_ask = Some(chunks.load(Ordering::Relaxed));
        }
    }
    let asked = printing_at_ask.expect("the finder was asked");
    // (A print goes on after the ask. On a starved machine its next
    // chunk may come after the pass.)
    let wait = Instant::now();
    while chunks.load(Ordering::Relaxed) <= asked && wait.elapsed() < Duration::from_secs(120) {
        std::thread::sleep(Duration::from_millis(1));
    }
    let after = chunks.load(Ordering::Relaxed);
    stop.store(true, Ordering::Relaxed);
    for p in printers {
        p.join().unwrap();
    }
    assert!(
        after > asked && asked > 0,
        "the prints ran on: {asked}, then {after}"
    );
    assert!(started.is_some(), "the song started: {}", h.describe());
    let st = h.looper.stats();
    let s = h.looper.status();
    let (line, words) = h.take_smart().expect("a word on the tap");
    if !answers {
        assert_eq!((st.finds_answered, st.finds_late), (0, 1), "{line}");
    }
    if st.finds_answered == 1 {
        assert_eq!(s.frames as u64, TWO_BARS, "{line}");
        assert_eq!(started, Some(120.0));
        // It started on the first kick: a mix comes a period late (the
        // mixer's period), so the kick handed over on a frame was
        // played a period before it.
        let kick = (rec + early - PERIOD as u64 - rec_from.unwrap()) as i64;
        assert_eq!(s.moved.map(|m| m.start), Some(kick), "{line}");
    } else {
        // Too slow a machine: the tap stands, counted and said, and
        // the song starts at its tempo (two bars and the 40 ms late,
        // less the 80 ms before the first kick).
        assert_eq!(st.finds_late, 1, "{line}");
        assert_eq!(
            words.as_deref(),
            Some("Kept your tap: it couldn't listen in time.")
        );
        assert_ne!(started, Some(120.0), "{line}");
    }
    took.sort();
    let worst = *took.last().unwrap();
    // Waiting on the finder would show in the looper's own count, and
    // would sleep in a period that spoke to it: none did. A period
    // away from the finder can sleep on a loaded machine with nothing
    // of ours to wait on: on CI, with 36 app tests at once, one did,
    // early in the recording, long before the ask. (The kernel can
    // stop a thread on its memory, for one: a page it is moving, or
    // another thread's hold on the process's memory map, both through
    // a page fault.) Those are printed with their page faults, not
    // judged. And the periods that spoke to the finder are not judged
    // as 32-bit ARM, which runs under qemu (CI's ARM job): there the
    // count is that of qemu's own thread, which sleeps on qemu's locks
    // while it translates code the period runs for the first time.
    // The loop's closing periods slept there, though they made no
    // blocking call (qemu's syscall trace), and the same pass again in
    // the same program slept in none.
    assert_eq!(
        h.finder.waited,
        0,
        "periods an ask waited for the finder: {}",
        h.describe()
    );
    if read {
        assert!(slept > 0, "the pacing sleeps, so the count moves");
        if !away.is_empty() {
            eprintln!(
                "periods away from the finder that slept (period, times, page faults), \
                 not judged: {away:?} of {p}"
            );
        }
        if cfg!(target_arch = "arm") {
            eprintln!(
                "periods that spoke to the finder and slept under qemu, not judged: {near:?}"
            );
        } else {
            assert!(
                near.is_empty(),
                "periods that spoke to the finder and slept (period, times, page faults): \
                 {near:?} of {p}: {}",
                h.describe()
            );
        }
    }
    eprintln!(
        "periods: 99th {:?}, 99.9th {:?}, worst {worst:?}; {spoke} of {p} spoke to the finder, \
         {} took {} page faults; {line}",
        took[took.len() * 99 / 100],
        took[took.len() * 999 / 1000],
        faulting.0,
        faulting.1
    );
    assert_eq!(allocs, 0, "allocations or frees on the audio thread's path");
    assert!(h.describe().contains("; by ear: "), "{}", h.describe());
}

/// Half speed at the tempo it was recorded at: like tape at once (an
/// octave down), then the worker's print (the octave made cleanly);
/// Overdub waits for it, fits the loop to it and records.
#[test]
fn half_speed_gets_a_print_an_octave_down_and_an_overdub_fits_to_it() {
    let mut h = LoopHost::new(44_100, 30);
    let mut t = Transport::new(44_100, 120.0).unwrap();
    let mut b = Block::new();
    h.command(Command::SetBars(Some(1)));
    h.command(Command::Record);
    t.command(0, T::Play);
    let sine = |f: u64| 0.5 * (f as f64 * std::f64::consts::TAU / 100.0).sin() as f32;
    let mut f = 0u64;
    let mut period = |h: &mut LoopHost, t: &mut Transport, b: &mut Block, rec: bool| {
        assert!(t.process(PERIOD as u32, b));
        let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
        if rec {
            for i in 0..PERIOD {
                l[i] = sine(f + i as u64);
                r[i] = l[i];
            }
        }
        h.take_pads(&l, &r);
        let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
        h.period(b, &mut l, &mut r);
        f += PERIOD as u64;
        l
    };
    while h.looper.status().phase != Phase::Playing {
        period(&mut h, &mut t, &mut b, true);
    }
    let line = h.command(Command::Edit(Edit::Speed(-1)));
    assert_eq!(
        line,
        "looper: speed ½×: an octave down, twice the bars (Undo takes it back)"
    );
    let ups = |out: &[f32]| {
        let x: Vec<usize> = (1..out.len())
            .filter(|&i| out[i - 1] < 0.0 && out[i] >= 0.0)
            .collect();
        (x.len() - 1) as f64 * 44_100.0 / (x[x.len() - 1] - x[0]) as f64
    };
    // Overdub at once: it waits for the print.
    for _ in 0..4 {
        period(&mut h, &mut t, &mut b, false);
    }
    assert!(h
        .command(Command::Overdub)
        .contains("once the loop's print at ½× is in"));
    let start = std::time::Instant::now();
    while h.looper.status().phase != Phase::Overdubbing {
        assert!(start.elapsed().as_secs() < 120, "no fit: {}", h.describe());
        period(&mut h, &mut t, &mut b, false);
        std::thread::sleep(std::time::Duration::from_micros(200));
    }
    let s = h.looper.status();
    assert_eq!((s.frames, s.speed, s.rate), (176_400, -1, 0));
    // The fitted layer plays the octave down, sample for sample.
    h.command(Command::Overdub);
    let mut out = Vec::new();
    for _ in 0..400 {
        out.extend(period(&mut h, &mut t, &mut b, false));
    }
    let down = ups(&out[1_000..]);
    assert!((down - 220.5).abs() < 2.0, "fitted {down}");
}

#[test]
fn a_steady_period_never_allocates() {
    let mut h = LoopHost::new(44_100, 20);
    let mut t = Transport::new(44_100, 120.0).unwrap();
    let mut b = Block::new();
    h.command(Command::SetBars(Some(1)));
    h.command(Command::Record);
    t.command(0, T::Play);
    let (mut l, mut r) = ([0.25; PERIOD], [-0.25; PERIOD]);
    // Record the bar and build its wave first.
    for p in 0..1_400u64 {
        assert!(t.process(PERIOD as u32, &mut b));
        h.take_pads(&l, &r);
        h.period(&b, &mut l, &mut r);
        h.changes(|| format!("period {p}"));
        h.wave_step(p, 0);
    }
    assert_eq!(h.looper.status().phase, Phase::Playing);
    let ((), calls) = crate::sys::alloc_count::calls(|| {
        for p in 1_400..1_800u64 {
            t.process(PERIOD as u32, &mut b);
            h.take_pads(&l, &r);
            h.period(&b, &mut l, &mut r);
            assert!(h.changes(|| format!("period {p}")).is_empty());
            assert!(h.wave_step(p, 0).is_none());
            h.view(120.0);
        }
    });
    assert_eq!(calls, 0, "allocations or frees on the audio thread's path");
}

#[test]
fn the_latency_estimate_is_the_output_buffer_and_one_period() {
    assert_eq!(estimated_latency(3), 512);
    assert_eq!(estimated_latency(2), 384);
}

/// A host with a one-bar loop of two layers (the second at a quarter
/// level), playing at 120 BPM.
fn saving_host() -> (LoopHost, Transport, Block) {
    let mut h = LoopHost::new(44_100, 20);
    let mut t = Transport::new(44_100, 120.0).unwrap();
    let mut b = Block::new();
    h.command(Command::SetBars(Some(1)));
    h.command(Command::Record);
    t.command(0, T::Play);
    let mut f = 0u64;
    let mut period = |h: &mut LoopHost, t: &mut Transport, b: &mut Block, hz: f64| {
        assert!(t.process(PERIOD as u32, b));
        let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
        for i in 0..PERIOD {
            l[i] =
                0.3 * ((f + i as u64) as f64 * hz * std::f64::consts::TAU / 44_100.0).sin() as f32;
            r[i] = -l[i];
        }
        h.take_pads(&l, &r);
        let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
        h.period(b, &mut l, &mut r);
        f += PERIOD as u64;
    };
    while h.looper.status().phase != Phase::Playing {
        period(&mut h, &mut t, &mut b, 441.0);
    }
    h.command(Command::Overdub);
    for _ in 0..200 {
        period(&mut h, &mut t, &mut b, 882.0);
    }
    h.command(Command::Overdub);
    for _ in 0..800 {
        period(&mut h, &mut t, &mut b, 0.0);
    }
    let id = h.looper.layer_info(1).unwrap().id;
    h.looper.layer_command(id, LayerCommand::Level(0.25));
    (h, t, b)
}

/// `periods` periods with no input: the output, interleaved.
fn run_host(h: &mut LoopHost, t: &mut Transport, b: &mut Block, periods: usize) -> Vec<f32> {
    let mut out = Vec::new();
    for _ in 0..periods {
        assert!(t.process(PERIOD as u32, b));
        let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
        h.period(b, &mut l, &mut r);
        out.extend(l.iter().zip(&r).flat_map(|(a, b)| [*a, *b]));
    }
    out
}

/// Run until the save is done.
fn saved_from(h: &mut LoopHost, t: &mut Transport, b: &mut Block) -> Saved {
    let start = std::time::Instant::now();
    loop {
        run_host(h, t, b, 1);
        if let Some(s) = h.take_saved() {
            return s;
        }
        std::thread::sleep(std::time::Duration::from_micros(100));
        assert!(start.elapsed().as_secs() < 60, "the save did not finish");
    }
}

/// A layer dragged onto another loop: moved into an empty loop, a
/// loop of its own, and Undo there takes it back to where it came
/// from, saying so; a loop of another length says why it can't take
/// it; a loop's only layer goes as a copy.
#[test]
fn a_dragged_layer_moves_between_loops_and_undo_says_so() {
    let (mut h, mut t, mut b) = saving_host();
    run_host(&mut h, &mut t, &mut b, 20);
    let over = h.looper.layer_info(1).unwrap().id;
    assert!(h
        .copy_layer(0, over, 0, false)
        .contains("is in loop 1 already"));
    let said = h.copy_layer(0, over, 2, false);
    assert_eq!(
        said,
        "looper: Layer 2 moves into loop 3, a loop of its own (UNDO takes it back)"
    );
    run_host(&mut h, &mut t, &mut b, 400);
    let (a, c) = (
        h.looper.loop_status(0).unwrap(),
        h.looper.loop_status(2).unwrap(),
    );
    assert_eq!((a.layers, c.phase, c.layers), (1, Phase::Playing, 1));
    assert_eq!(c.ticks, a.ticks);
    // Undo on loop 3 puts it back in loop 1, and loop 3 clears.
    h.select(2);
    run_host(&mut h, &mut t, &mut b, 2);
    assert_eq!(
        h.command(Command::Undo),
        "looper: Undo: the layer goes back to loop 1, and this loop clears"
    );
    run_host(&mut h, &mut t, &mut b, 40);
    assert_eq!(h.looper.loop_status(2).unwrap().phase, Phase::Empty);
    assert_eq!(h.looper.loop_status(0).unwrap().layers, 2);
    // Loop 2: two bars. Loop 1's layer can't go onto it.
    h.select(1);
    h.command(Command::SetBars(Some(2)));
    h.command(Command::Record);
    while h.looper.loop_status(1).unwrap().phase != Phase::Playing {
        run_host(&mut h, &mut t, &mut b, 10);
    }
    assert_eq!(
        h.copy_layer(0, over, 1, true),
        "looper: loop 2 is 2 bars and Layer 2 1 bar: a layer goes only onto a loop of its length"
    );
    // Its only layer goes into loop 3 as a copy: loop 2 keeps it.
    let first = h.looper.loop_layer_info(1, 0).unwrap().id;
    assert!(h
        .copy_layer(1, first, 2, false)
        .contains("it has one layer, so loop 2 keeps it"));
    run_host(&mut h, &mut t, &mut b, 800);
    assert_eq!(h.looper.loop_status(1).unwrap().layers, 1);
    assert_eq!(h.looper.loop_status(2).unwrap().layers, 1);
}

#[test]
fn loops_saved_with_the_project_load_back_and_play_the_same() {
    use mpfree_looper::save::{from_text, from_wav, to_text, wav};
    let (mut h, mut t, mut b) = saving_host();
    assert_eq!(h.save(), "looper: saving the loops");
    let mut saved = saved_from(&mut h, &mut t, &mut b);
    assert!(h.saving().is_none());
    assert_eq!(saved.loops.len(), 1);
    assert_eq!(saved.loops[0].1.layers.len(), 2);
    assert_eq!(saved.loops[0].1.layers[1].level, 0.25);
    // As the project keeps them: the text, and a WAV a layer.
    let mut files = Vec::new();
    for (n, f) in &mut saved.loops {
        for (k, y) in f.layers.iter_mut().enumerate() {
            y.file = format!("loop {} layer {}.wav", *n + 1, k + 1);
            files.push((y.file.clone(), wav(&y.audio[..2 * f.frames], saved.rate)));
        }
    }
    let mut back = from_text(&to_text(&saved)).unwrap();
    for (_, f) in &mut back.loops {
        for y in &mut f.layers {
            let bytes = &files.iter().find(|(n, _)| *n == y.file).unwrap().1;
            let (audio, rate) = from_wav(bytes).unwrap();
            assert_eq!(rate, 44_100);
            y.audio = audio;
        }
    }
    for ((_, a), (_, c)) in saved.loops.iter().zip(&back.loops) {
        assert_eq!(a.frames, c.frames);
        for (x, y) in a.layers.iter().zip(&c.layers) {
            assert_eq!(x.audio[..2 * a.frames], y.audio[..]);
            assert_eq!((x.level, &x.file), (y.level, &y.file));
        }
    }
    // Into a fresh host at the same song frame: it plays as the first
    // one does, a bar on.
    let (mut h2, mut t2) = (
        LoopHost::new(44_100, 20),
        Transport::new(44_100, 120.0).unwrap(),
    );
    let mut b2 = Block::new();
    t2.command(0, T::Play);
    let skip = (t.frame() / PERIOD as u64) as usize;
    run_host(&mut h2, &mut t2, &mut b2, skip);
    assert_eq!(t2.frame(), t.frame());
    assert!(h2.load(back).starts_with("looper: loading loops 1"));
    let a = run_host(&mut h, &mut t, &mut b, 1500);
    let c = run_host(&mut h2, &mut t2, &mut b2, 1500);
    assert_eq!(h2.looper.status().phase, Phase::Playing);
    let n = a.len();
    let worst = (n - 2 * 88_200..n)
        .map(|i| (a[i] - c[i]).abs())
        .fold(0.0, f32::max);
    assert!(worst < 1e-5, "differs by {worst}");
}

/// Layers as recordings (audio tracks phase 4c.2): with recordings on, each closed layer gets a
/// recording once; a save keeps each loop as the looper holds it
/// (`mpfree-looper 2`), each layer with its recording and its file
/// named by it; it opens in a fresh host playing the same, and keeps
/// its recordings, so saved again it names the same files and nothing
/// is written twice.
#[test]
fn loops_saved_as_recordings_open_the_same_and_keep_their_recordings() {
    use mpfree_looper::save::{from_text, wav};
    let (mut h, mut t, mut b) = saving_host();
    h.set_recordings(true);
    let mut next = 0u128;
    let mut fresh = || {
        next += 1;
        0xfeed_0000 + next
    };
    h.give_layer_recs(&mut fresh);
    h.give_layer_recs(&mut fresh);
    assert_eq!(next, 2, "a recording a layer, once");
    h.save();
    let saved = saved_from(&mut h, &mut t, &mut b);
    let (text, files, copies) = crate::project::loops_part(saved);
    assert!(copies.is_empty(), "no file is written here");
    assert!(text.starts_with("mpfree-looper 2\n"), "{text}");
    let names: Vec<String> = files.iter().map(|(n, _)| n.clone()).collect();
    assert_eq!(
        names,
        [
            format!("loop_{:032x}.wav", 0xfeed_0001u128),
            format!("loop_{:032x}.wav", 0xfeed_0002u128)
        ]
    );
    // The first pass's audio as written: more than its loop.
    let frames = from_text(&text).unwrap().loops[0].1.frames;
    assert!(files[0].1.len() > 2 * frames);
    let wavs: Vec<(String, Vec<u8>)> = files
        .iter()
        .map(|(n, a)| (n.clone(), wav(a, 44_100)))
        .collect();
    let read = |name: &str| {
        wavs.iter()
            .find(|(n, _)| n == name)
            .map(|(_, b)| b.clone())
            .ok_or_else(|| format!("no {name}"))
    };
    let (back, notes, left) = crate::project::loops_open(&text, read).unwrap();
    assert!(notes.is_empty() && left.is_empty(), "{notes:?}");
    // Into a fresh host at the same song frame: it plays as the first
    // one does, a bar on.
    let (mut h2, mut t2) = (
        LoopHost::new(44_100, 20),
        Transport::new(44_100, 120.0).unwrap(),
    );
    h2.set_recordings(true);
    let mut b2 = Block::new();
    t2.command(0, T::Play);
    let skip = (t.frame() / PERIOD as u64) as usize;
    run_host(&mut h2, &mut t2, &mut b2, skip);
    assert!(h2.load(back).starts_with("looper: loading loops 1"));
    let a = run_host(&mut h, &mut t, &mut b, 1500);
    let c = run_host(&mut h2, &mut t2, &mut b2, 1500);
    let n = a.len();
    let worst = (n - 2 * 88_200..n)
        .map(|i| (a[i] - c[i]).abs())
        .fold(0.0, f32::max);
    assert!(worst < 1e-5, "differs by {worst}");
    // Its layers keep the recordings they opened with: saved again,
    // the same files.
    h2.give_layer_recs(|| panic!("a layer opened with its recording got another"));
    h2.save();
    let again = saved_from(&mut h2, &mut t2, &mut b2);
    // Opened from the project, their files are there: a save copies
    // them (4c.4), reading none of their audio.
    let (text2, files2, copies2) = crate::project::loops_part(again);
    assert!(files2.is_empty());
    assert_eq!(copies2, names);
    assert_eq!(
        from_text(&text2).unwrap().loops,
        from_text(&text).unwrap().loops
    );
    // With recordings off, a save is as before: each layer as it plays.
    h2.set_recordings(false);
    h2.save();
    let (text3, _, _) = crate::project::loops_part(saved_from(&mut h2, &mut t2, &mut b2));
    assert!(text3.starts_with("mpfree-looper 1\n"), "{text3}");
}

/// Phase 4c.4: with recordings on, a save is a snapshot. The loops
/// are kept as they were when it began, each layer read by its
/// number, while the loop goes on: a level turned and an overdub
/// started during the save start nothing again, and it gives the same
/// text and audio as a save with nothing changing.
#[test]
fn a_snapshot_save_keeps_the_loops_as_they_were_while_they_change() {
    let (mut h, mut t, mut b) = saving_host();
    h.set_recordings(true);
    let mut next = 0u128;
    h.give_layer_recs(|| {
        next += 1;
        0xfeed_0000 + next
    });
    h.save();
    let (text, files, copies) = crate::project::loops_part(saved_from(&mut h, &mut t, &mut b));
    assert!(copies.is_empty(), "no file is written here");
    let restarts = h.save_restarts;
    h.save();
    // Into the reading, then the loop changes.
    let start = std::time::Instant::now();
    while !h.saving().is_some_and(|p| p > 0.0) {
        run_host(&mut h, &mut t, &mut b, 1);
        std::thread::sleep(std::time::Duration::from_micros(100));
        assert!(start.elapsed().as_secs() < 60, "the save never read");
    }
    let before = h.looper.loop_status(0).unwrap().version;
    let id = h.looper.layer_info(1).unwrap().id;
    h.looper.layer_command(id, LayerCommand::Level(0.75));
    h.command(Command::Overdub);
    let moved = saved_from(&mut h, &mut t, &mut b);
    assert_ne!(h.looper.loop_status(0).unwrap().version, before);
    assert_eq!(h.save_restarts, restarts, "nothing started again");
    let (text2, files2, _) = crate::project::loops_part(moved);
    assert_eq!(text2, text);
    assert!(files2 == files, "the same audio");
}

/// Phase 4c.5: a merged layer is a recording of its own that names
/// the recordings it was made from (`from=`), and keeps naming them
/// once opened from the project and saved again.
#[test]
fn a_merged_layer_names_the_recordings_it_was_made_from() {
    use mpfree_looper::save::{from_text, wav};
    let (mut h, mut t, mut b) = saving_host();
    h.set_recordings(true);
    let mut next = 0u128;
    let mut fresh = || {
        next += 1;
        0xfeed_0000 + next
    };
    h.give_layer_recs(&mut fresh);
    h.command(Command::Merge);
    for _ in 0..2_000 {
        if h.looper.status().layers == 1 && h.looper.layer_info(0).unwrap().closed {
            break;
        }
        run_host(&mut h, &mut t, &mut b, 1);
    }
    assert_eq!(h.looper.status().layers, 1, "merged");
    h.give_layer_recs(&mut fresh);
    assert_eq!(next, 3, "the merged layer is a recording of its own");
    h.save();
    let (text, files, _) = crate::project::loops_part(saved_from(&mut h, &mut t, &mut b));
    let line = format!(" rec={:032x} ", 0xfeed_0003u128);
    let merged = text
        .lines()
        .find(|l| l.contains(&line))
        .expect("the merged layer");
    assert!(
        merged.contains(&format!(
            " from={:032x}+{:032x} ",
            0xfeed_0001u128, 0xfeed_0002u128
        )),
        "{merged}"
    );
    // Opened and saved again: the same.
    let wavs: Vec<(String, Vec<u8>)> = files
        .iter()
        .map(|(n, a)| (n.clone(), wav(a, 44_100)))
        .collect();
    let read = |name: &str| {
        wavs.iter()
            .find(|(n, _)| n == name)
            .map(|(_, b)| b.clone())
            .ok_or_else(|| format!("no {name}"))
    };
    let (back, _, left) = crate::project::loops_open(&text, read).unwrap();
    assert!(left.is_empty());
    let (mut h2, mut t2) = (
        LoopHost::new(44_100, 20),
        Transport::new(44_100, 120.0).unwrap(),
    );
    h2.set_recordings(true);
    let mut b2 = Block::new();
    t2.command(0, T::Play);
    run_host(&mut h2, &mut t2, &mut b2, 4);
    assert!(h2.load(back).starts_with("looper: loading loops 1"));
    run_host(&mut h2, &mut t2, &mut b2, 400);
    h2.give_layer_recs(|| panic!("opened with its recording"));
    h2.save();
    let (text2, _, _) = crate::project::loops_part(saved_from(&mut h2, &mut t2, &mut b2));
    assert_eq!(
        from_text(&text2).unwrap().loops,
        from_text(&text).unwrap().loops
    );
}

/// Where a test's layers' files go (phase 4c.3): written at once, a
/// piece at a time, each buffer back as soon as it is given.
struct FakeFiles {
    on: bool,
    spare: Vec<Vec<f32>>,
    open: std::collections::HashMap<u128, (usize, Vec<f32>)>,
    done: std::collections::HashMap<u128, Vec<f32>>,
    said: Vec<(u128, Option<String>)>,
    dropped: Vec<u128>,
    fail: Option<String>,
}

impl FakeFiles {
    fn new() -> FakeFiles {
        FakeFiles {
            on: true,
            spare: (0..2).map(|_| vec![0.0; 8_192]).collect(),
            open: Default::default(),
            done: Default::default(),
            said: Vec::new(),
            dropped: Vec::new(),
            fail: None,
        }
    }
}

impl crate::loops_on_tracks::LayerFiles for FakeFiles {
    fn can_write(&self) -> bool {
        self.on
    }
    fn begin(&mut self, rec: u128, frames: usize) -> bool {
        assert!(self.open.is_empty(), "one at a time");
        self.open.insert(rec, (frames, Vec::new()));
        true
    }
    fn buffer(&mut self) -> Option<Vec<f32>> {
        self.spare.pop()
    }
    fn piece(&mut self, rec: u128, buf: Vec<f32>, len: usize) {
        if let Some((_, v)) = self.open.get_mut(&rec) {
            v.extend_from_slice(&buf[..len]);
        }
        self.spare.push(buf);
    }
    fn end(&mut self, rec: u128) {
        let (frames, v) = self.open.remove(&rec).expect("begun");
        assert_eq!(v.len(), 2 * frames, "all of it, once");
        match &self.fail {
            Some(why) => self.said.push((rec, Some(why.clone()))),
            None => {
                self.done.insert(rec, v);
                self.said.push((rec, None));
            }
        }
    }
    fn drop_layer(&mut self, rec: u128) {
        self.open.remove(&rec);
        self.dropped.push(rec);
    }
    fn written(&mut self) -> Option<(u128, Option<String>)> {
        (!self.said.is_empty()).then(|| self.said.remove(0))
    }
}

/// Phase 4d.4: a layer whose file this run wrote and synced may leave
/// memory while only UNDO needs it, when a new layer needs the room;
/// the save still names its file, and its file holds what the looper
/// takes back, so it comes back exactly before that UNDO.
#[test]
fn a_layer_in_its_file_leaves_memory_and_comes_back_from_it_exactly() {
    // Seven seconds: a first pass of a bar (two seconds) with what it
    // keeps and two overdubs, not a third.
    let mut h = LoopHost::new(44_100, 7);
    let mut t = Transport::new(44_100, 120.0).unwrap();
    let mut b = Block::new();
    let mut f = 0u64;
    let mut period = |h: &mut LoopHost, t: &mut Transport, b: &mut Block, hz: f64| {
        assert!(t.process(PERIOD as u32, b));
        let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
        for i in 0..PERIOD {
            l[i] =
                0.2 * ((f + i as u64) as f64 * hz * std::f64::consts::TAU / 44_100.0).sin() as f32;
            r[i] = -l[i];
        }
        h.take_pads(&l, &r);
        let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
        h.period(b, &mut l, &mut r);
        f += PERIOD as u64;
    };
    h.set_recordings(true);
    h.command(Command::SetBars(Some(1)));
    h.command(Command::Record);
    t.command(0, T::Play);
    while h.looper.status().phase != Phase::Playing {
        period(&mut h, &mut t, &mut b, 441.0);
    }
    let bar = 2 * 44_100 / PERIOD + 1;
    let mut overdub = |h: &mut LoopHost, t: &mut Transport, b: &mut Block, hz: f64| {
        h.command(Command::Overdub);
        for _ in 0..bar {
            period(h, t, b, hz);
        }
        h.command(Command::Overdub);
        for _ in 0..20 {
            period(h, t, b, 0.0);
        }
    };
    overdub(&mut h, &mut t, &mut b, 660.0);
    overdub(&mut h, &mut t, &mut b, 880.0);
    assert_eq!(h.looper.status().layers, 3);
    // Every layer to its file.
    let mut files = FakeFiles::new();
    let next = std::cell::Cell::new(0u128);
    h.give_layer_recs(|| {
        next.set(next.get() + 1);
        0xbac0_0000 + next.get()
    });
    while h.unsaved_layers(0) > 0 {
        assert_eq!(h.write_layers(&mut files, PERIOD), None);
        run_host(&mut h, &mut t, &mut b, 1);
    }
    assert_eq!(h.write_layers(&mut files, PERIOD), None);
    let second = h.looper.layer_info(1).unwrap();
    let rec = h.layer_rec(0, second.born);
    assert_ne!(rec, 0);
    // Deleted, then a new overdub needs its room: it leaves memory,
    // and no step is let go.
    h.layer_command(second.id, LayerCommand::Delete);
    run_host(&mut h, &mut t, &mut b, 20);
    overdub(&mut h, &mut t, &mut b, 990.0);
    assert_eq!(h.looper.status().layers, 3, "the overdub had room");
    assert!(h.looper.is_away(0, second.born));
    let file = format!("loop_{rec:032x}.wav");
    assert!(
        h.layer_recs.held_files(&h.looper).contains(&file),
        "the save names its file"
    );
    // The new overdub undone and its Redo forgotten: the Delete is
    // next, and waits for the layer.
    h.command(Command::Undo);
    run_host(&mut h, &mut t, &mut b, 20);
    h.looper.command(Command::ForgetRedo);
    run_host(&mut h, &mut t, &mut b, 2);
    let (lp, slot, born) = h.looper.away_for(0, false).expect("the Delete needs it");
    assert_eq!((lp, born), (0, second.born));
    assert_eq!(h.layer_rec(lp, born), rec);
    // Its file, as the worker reads it back, goes in.
    let audio = files.done[&rec].clone();
    let size = h.looper.begin_back(lp, slot).unwrap();
    assert_eq!(audio.len(), 2 * size);
    assert!(h.looper.put_back(&audio));
    h.command(Command::Undo);
    run_host(&mut h, &mut t, &mut b, 20);
    assert_eq!(h.looper.status().layers, 3, "the deleted layer is back");
    let mut again = vec![0.0; 2 * size];
    assert_eq!(h.looper.raw_of(0, born, 0, &mut again), Some(size));
    assert_eq!(again, audio, "exactly as it was written");
}

/// Phase 4c.3: with recordings on, each closed layer goes to its
/// recording's file soon after it closes, a piece at a time while the
/// loops play on, the same audio a save keeps; a layer still
/// recording waits; one that goes while it is written is dropped and
/// written again if it comes back; a file that can't be written is
/// said once and not tried again.
#[test]
fn closed_layers_go_to_their_files_soon_after_they_close() {
    let (mut h, mut t, mut b) = saving_host();
    let mut files = FakeFiles::new();
    // Recordings off: nothing is written, nothing counts.
    h.give_layer_recs(|| panic!("off"));
    assert_eq!(h.write_layers(&mut files, PERIOD), None);
    assert!(files.open.is_empty() && h.unsaved_layers(0) == 0);
    h.set_recordings(true);
    let next = std::cell::Cell::new(0u128);
    let fresh = || {
        next.set(next.get() + 1);
        0xfeed_0000 + next.get()
    };
    h.give_layer_recs(fresh);
    assert_eq!(h.unsaved_layers(0), 2);
    let write = |h: &mut LoopHost, files: &mut FakeFiles, t: &mut Transport, b: &mut Block| {
        let mut periods = 0;
        while h.unsaved_layers(0) > 0 {
            assert_eq!(h.write_layers(files, PERIOD), None);
            run_host(h, t, b, 1);
            periods += 1;
            assert!(periods < 400, "still {} unsaved", h.unsaved_layers(0));
        }
        periods
    };
    // Two layers of about two seconds at 64 times the period: a few
    // dozen periods, not one.
    let periods = write(&mut h, &mut files, &mut t, &mut b);
    assert!(periods > 10, "{periods}");
    assert_eq!(files.done.len(), 2);
    // A save copies both files (4c.4): none of their audio is read.
    h.save();
    let (_, kept, copies) = crate::project::loops_part(saved_from(&mut h, &mut t, &mut b));
    assert!(kept.is_empty());
    let mut written: Vec<String> = files
        .done
        .keys()
        .map(|rec| format!("loop_{rec:032x}.wav"))
        .collect();
    written.sort();
    let mut copies = copies;
    copies.sort();
    assert_eq!(copies, written);
    // Periods with a tone on the pads, for an overdub to keep.
    let play = |h: &mut LoopHost, t: &mut Transport, b: &mut Block, periods: usize| {
        for k in 0..periods {
            assert!(t.process(PERIOD as u32, b));
            let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
            for i in 0..PERIOD {
                let f = (k * PERIOD + i) as f64;
                l[i] = 0.2 * (f * 660.0 * std::f64::consts::TAU / 44_100.0).sin() as f32;
                r[i] = -l[i];
            }
            h.take_pads(&l, &r);
            let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
            h.period(b, &mut l, &mut r);
        }
    };
    // Run until the loop's newest layer is closed.
    let close = |h: &mut LoopHost, t: &mut Transport, b: &mut Block| {
        for _ in 0..2_000 {
            let s = h.looper.status();
            let last = h.looper.layer_info(s.layers.saturating_sub(1)).unwrap();
            if last.closed && s.phase == Phase::Playing {
                return;
            }
            run_host(h, t, b, 1);
        }
        panic!("the overdub never closed");
    };
    // An overdub: counted while it records, written once it closes.
    h.command(Command::Overdub);
    play(&mut h, &mut t, &mut b, 100);
    h.give_layer_recs(fresh);
    assert_eq!(h.write_layers(&mut files, PERIOD), None);
    assert_eq!((h.unsaved_layers(0), files.open.len()), (1, 0));
    h.command(Command::Overdub);
    close(&mut h, &mut t, &mut b);
    h.give_layer_recs(fresh);
    assert_eq!(next.get(), 3);
    // Undone after its first piece: dropped, and its memory is the
    // only copy again.
    assert_eq!(h.write_layers(&mut files, PERIOD), None);
    assert_eq!(h.write_layers(&mut files, PERIOD), None);
    assert!(files.open.contains_key(&0xfeed_0003));
    h.command(Command::Undo);
    run_host(&mut h, &mut t, &mut b, 40);
    assert_eq!(h.write_layers(&mut files, PERIOD), None);
    assert_eq!(files.dropped, [0xfeed_0003]);
    assert!(
        files.open.is_empty() && files.spare.len() == 2,
        "the buffers back"
    );
    assert_eq!(h.unsaved_layers(0), 0);
    // Redone, it is written after all, under the same recording.
    h.command(Command::Redo);
    run_host(&mut h, &mut t, &mut b, 40);
    h.give_layer_recs(|| panic!("it has its recording"));
    write(&mut h, &mut files, &mut t, &mut b);
    assert!(files.done.contains_key(&0xfeed_0003));
    // A file that can't be written: said once, counted, not tried
    // again; the layer stays only in memory.
    files.fail = Some("the stick is full".into());
    h.command(Command::Overdub);
    play(&mut h, &mut t, &mut b, 100);
    h.command(Command::Overdub);
    close(&mut h, &mut t, &mut b);
    h.give_layer_recs(fresh);
    let mut said = Vec::new();
    for _ in 0..40 {
        said.extend(h.write_layers(&mut files, PERIOD));
        run_host(&mut h, &mut t, &mut b, 1);
    }
    assert_eq!(
        said,
        [
            "looper: a layer's file couldn't be written (the stick is full); its audio stays \
          in memory and is saved with the project"
        ]
    );
    assert_eq!(h.unsaved_layers(0), 1);
    assert!(h
        .describe()
        .contains("1 layers only in memory, 1 layers' files couldn't be written"));
    assert!(!files.done.contains_key(&0xfeed_0004));
    // No folder to write to: nothing starts.
    files.on = false;
    files.fail = None;
    h.command(Command::Overdub);
    play(&mut h, &mut t, &mut b, 100);
    h.command(Command::Overdub);
    close(&mut h, &mut t, &mut b);
    h.give_layer_recs(fresh);
    assert_eq!(h.write_layers(&mut files, PERIOD), None);
    assert!(files.open.is_empty());
    assert_eq!(h.unsaved_layers(0), 2);
}

/// Phase 4c.6: a layer's recording the looper still holds but no
/// layer line names (undone, kept for Redo; in a loop a Clear kept for
/// Undo) is named on a `kept` line, so the project keeps its file;
/// one nothing holds any more (the loop forgotten) is not.
#[test]
fn recordings_held_for_an_undo_are_named_by_the_save() {
    let (mut h, mut t, mut b) = saving_host();
    let mut files = FakeFiles::new();
    h.set_recordings(true);
    let next = std::cell::Cell::new(0u128);
    h.give_layer_recs(|| {
        next.set(next.get() + 1);
        0xbead_0000 + next.get()
    });
    let mut periods = 0;
    while h.unsaved_layers(0) > 0 {
        assert_eq!(h.write_layers(&mut files, PERIOD), None);
        run_host(&mut h, &mut t, &mut b, 1);
        periods += 1;
        assert!(periods < 400, "still {} unsaved", h.unsaved_layers(0));
    }
    let name = |rec: u128| format!("loop_{rec:032x}.wav");
    let (first, over) = (name(0xbead_0001), name(0xbead_0002));
    let save = |h: &mut LoopHost, t: &mut Transport, b: &mut Block| {
        h.save();
        let s = saved_from(h, t, b);
        let layers: Vec<String> = s
            .loops
            .iter()
            .flat_map(|(_, l)| l.layers.iter().map(|y| y.file.clone()))
            .collect();
        let text = crate::project::loops_part(s.clone()).0;
        assert_eq!(text.matches("\nkept file=").count(), s.kept.len(), "{text}");
        (layers, s.kept)
    };
    // Both on their layer lines: nothing kept beside them.
    assert_eq!(
        save(&mut h, &mut t, &mut b),
        (vec![first.clone(), over.clone()], vec![])
    );
    // The overdub undone: Redo can bring it back.
    h.command(Command::Undo);
    run_host(&mut h, &mut t, &mut b, 40);
    assert_eq!(
        save(&mut h, &mut t, &mut b),
        (vec![first.clone()], vec![over.clone()])
    );
    // The loop cleared: Undo can bring both back.
    h.command(Command::Clear);
    run_host(&mut h, &mut t, &mut b, 40);
    assert_eq!(h.looper.loop_status(0).unwrap().phase, Phase::Empty);
    let (layers, mut kept) = save(&mut h, &mut t, &mut b);
    kept.sort();
    assert_eq!((layers, kept), (vec![], vec![first.clone(), over.clone()]));
    // Forgotten (no Undo): nothing holds them.
    h.command(Command::Forget);
    run_host(&mut h, &mut t, &mut b, 40);
    assert_eq!(save(&mut h, &mut t, &mut b), (vec![], vec![]));
}

#[test]
fn an_open_clears_every_loop_then_its_loops_go_in() {
    let (mut h, mut t, mut b) = saving_host();
    h.save();
    let saved = saved_from(&mut h, &mut t, &mut b);
    // Loop 3 armed too: an open clears it as well.
    h.select(2);
    h.command(Command::Record);
    run_host(&mut h, &mut t, &mut b, 2);
    assert_eq!(h.looper.loop_status(2).unwrap().phase, Phase::Armed);
    // A save under way stops.
    h.save();
    assert!(h.clear_all().starts_with("looper: every loop fades out"));
    assert!(h.saving().is_none() && h.opening());
    h.open(saved.clone());
    // Nothing goes in while loop 1 fades out.
    assert_eq!(h.tend_open(), None);
    let start = std::time::Instant::now();
    let line = loop {
        run_host(&mut h, &mut t, &mut b, 1);
        if let Some(line) = h.tend_open() {
            break line;
        }
        assert!(start.elapsed().as_secs() < 60, "the loops never went in");
    };
    assert!(line.starts_with("looper: loading loops 1"), "{line}");
    assert!(!h.opening());
    assert_eq!(h.looper.loop_status(2).unwrap().phase, Phase::Empty);
    run_host(&mut h, &mut t, &mut b, 400);
    let s = h.looper.loop_status(0).unwrap();
    assert_eq!((s.phase, s.layers), (Phase::Playing, 2));
    // New project: cleared, and nothing comes in.
    h.clear_all();
    let start = std::time::Instant::now();
    while h.opening() {
        run_host(&mut h, &mut t, &mut b, 1);
        assert_eq!(h.tend_open(), None);
        assert!(start.elapsed().as_secs() < 60);
    }
    assert!((0..MAX_LOOPS).all(|n| h.looper.loop_status(n).unwrap().phase == Phase::Empty));
}

/// Sync sticks from loop to loop, as Starts does: set on one empty
/// loop, the next empty loop picked takes it and records with it (with
/// nothing playing, in your own time, as the log says); a loop that
/// has sound keeps its own.
#[test]
fn sync_sticks_for_every_empty_loop() {
    let mut h = LoopHost::new(44_100, 8);
    let mut t = Transport::new(44_100, 120.0).unwrap();
    let mut b = Block::new();
    let mut run = |h: &mut LoopHost, periods: u64| {
        for _ in 0..periods {
            assert!(t.process(PERIOD as u32, &mut b));
            let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
            h.period(&b, &mut l, &mut r);
        }
    };
    run(&mut h, 1);
    assert_eq!(h.looper.status().clock, Clock::Song, "the song's at first");
    assert_eq!(
        h.command(Command::SetClock(Clock::Free)),
        "looper: sync: free (the loop keeps its own time)"
    );
    run(&mut h, 1);
    assert_eq!(h.looper.status().clock, Clock::Free);
    h.select(3);
    run(&mut h, 1);
    assert_eq!(h.looper.status().clock, Clock::Free, "loop 4 shows it");
    h.select(5);
    run(&mut h, 1);
    let said = h.command(Command::Record);
    assert!(
        said.contains("free in your own time") && said.contains("the song stays stopped"),
        "{said}"
    );
    run(&mut h, 1);
    let s = h.looper.loop_status(5).unwrap();
    assert_eq!(
        (s.phase, s.clock, s.free),
        (Phase::Recording, Clock::Free, true)
    );
    // Back to the song's, set on the loop recording: it keeps its own
    // (nothing is refused), and the next empty loop takes the song's.
    let said = h.command(Command::SetClock(Clock::Song));
    assert!(
        said.contains("from the next empty loop on (loop 6 keeps its own)"),
        "{said}"
    );
    h.select(7);
    run(&mut h, 1);
    assert_eq!(h.looper.status().clock, Clock::Song);
    assert_eq!(
        h.looper.loop_status(5).unwrap().clock,
        Clock::Free,
        "recording: kept"
    );
    assert_eq!(h.looper.stats().refused, 0);
}

/// Tempo follows: Free loops (the looper refactor's R2): the lead is
/// the free loops' clock as a playing span, giving its ticks on every
/// frame, only while it runs: none before there is a free loop, and
/// none once a second STOP stops it.
#[test]
fn the_tempo_lead_is_the_free_clock_while_it_runs() {
    let mut h = LoopHost::new(44_100, 30);
    let mut t = Transport::new(44_100, 120.0).unwrap();
    let mut b = Block::new();
    assert_eq!(h.tempo_lead(), None, "no free loop");
    h.command(Command::SetClock(Clock::Free));
    h.command(Command::SetBars(Some(1)));
    h.command(Command::Record);
    t.command(0, T::Play);
    let period = |h: &mut LoopHost, t: &mut Transport, b: &mut Block| {
        assert!(t.process(PERIOD as u32, b));
        let l = [0.25; PERIOD];
        h.take_pads(&l, &l);
        let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
        h.period(b, &mut l, &mut r);
    };
    while h.looper.status().phase != Phase::Playing {
        period(&mut h, &mut t, &mut b);
    }
    t.command(t.frame(), T::Stop);
    for _ in 0..20 {
        period(&mut h, &mut t, &mut b);
    }
    assert_eq!(t.state(), mpfree_transport::State::Stopped);
    let c = h.looper.free_clock().expect("the free clock");
    assert!(c.running, "the free loop plays on");
    let s = h.tempo_lead().expect("it leads");
    assert_eq!((s.bpm, s.state, s.meter), (120.0, State::Playing, c.meter));
    for f in [t.frame(), t.frame() + 12_345, t.frame() + 1_000_000] {
        assert_eq!(s.tick_at(f), c.tick_at(f), "its ticks on frame {f}");
    }
    // A second STOP stops the free loops: the song's clock leads.
    t.command(t.frame(), T::Stop);
    for _ in 0..20 {
        period(&mut h, &mut t, &mut b);
    }
    assert!(h.looper.free_clock().is_some_and(|c| !c.running));
    assert_eq!(h.tempo_lead(), None);
}

/// A free loop keeps its own tempo: when the song's tempo changes, the
/// printer asks for no print of it (one made at the song's tempo and
/// played on the free clock would change its pitch) and it plays on
/// at its pitch; a Speed print of it is made at the free clock's
/// tempo.
#[test]
fn a_free_loop_gets_no_print_when_the_songs_tempo_changes() {
    let mut h = LoopHost::new(44_100, 30);
    let mut t = Transport::new(44_100, 120.0).unwrap();
    let mut b = Block::new();
    h.command(Command::SetClock(Clock::Free));
    h.command(Command::SetBars(Some(1)));
    h.command(Command::Record);
    t.command(0, T::Play);
    // 441 Hz: 100 frames a cycle.
    let sine = |f: u64| 0.5 * (f as f64 * std::f64::consts::TAU / 100.0).sin() as f32;
    let mut f = 0u64;
    let mut period = |h: &mut LoopHost, t: &mut Transport, b: &mut Block, rec: bool| {
        assert!(t.process(PERIOD as u32, b));
        let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
        if rec {
            for i in 0..PERIOD {
                l[i] = sine(f + i as u64);
                r[i] = l[i];
            }
        }
        h.take_pads(&l, &r);
        let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
        h.period(b, &mut l, &mut r);
        f += PERIOD as u64;
        l
    };
    while h.looper.status().phase != Phase::Playing {
        period(&mut h, &mut t, &mut b, true);
    }
    let s = h.looper.status();
    assert_eq!((s.frames, s.clock), (88_200, Clock::Free));
    t.command(t.frame(), T::SetTempo(100.0));
    for _ in 0..200 {
        period(&mut h, &mut t, &mut b, false);
    }
    assert_eq!(t.bpm(), 100.0, "the song's tempo changed");
    assert!(
        !h.printer.asked && h.printer.doing.is_none() && !h.looper.status().printed,
        "no print: {}",
        h.describe()
    );
    let ups = |out: &[f32]| {
        let x: Vec<usize> = (1..out.len())
            .filter(|&i| out[i - 1] < 0.0 && out[i] >= 0.0)
            .collect();
        (x.len() - 1) as f64 * 44_100.0 / (x[x.len() - 1] - x[0]) as f64
    };
    let mut out = Vec::new();
    for _ in 0..30 {
        out.extend(period(&mut h, &mut t, &mut b, false));
    }
    let pitch = ups(&out);
    assert!((pitch - 441.0).abs() < 2.0, "pitch {pitch}");
    // Speed: an octave down, two bars at the free clock's 120 BPM.
    h.command(Command::Edit(Edit::Speed(-1)));
    let start = std::time::Instant::now();
    let wanted = loop {
        if let Some(w) = h.printer.doing {
            break w;
        }
        assert!(start.elapsed().as_secs() < 60, "no print: {}", h.describe());
        period(&mut h, &mut t, &mut b, false);
        std::thread::sleep(std::time::Duration::from_micros(200));
    };
    assert_eq!(wanted.frames, frames_at(2 * 3_840, 120.0, 44_100));
    assert_eq!(wanted.frames, 176_400);
}

/// Free loops saved with the project open on their clock, stopped at
/// their tops, as a project opens with nothing launched. The open loads them as soon as the old free
/// loop is empty (its clock goes only with the next period), and
/// PLAY starts them with the song.
#[test]
fn free_loops_saved_with_the_project_open_stopped_on_their_clock() {
    let mut h = LoopHost::new(44_100, 20);
    let mut t = Transport::new(44_100, 120.0).unwrap();
    let mut b = Block::new();
    h.command(Command::SetClock(Clock::Free));
    h.command(Command::SetBars(Some(1)));
    h.command(Command::Record);
    t.command(0, T::Play);
    let mut f = 0u64;
    while h.looper.status().phase != Phase::Playing {
        assert!(t.process(PERIOD as u32, &mut b));
        let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
        for i in 0..PERIOD {
            l[i] = 0.3 * ((f + i as u64) as f64 * 0.0628).sin() as f32;
            r[i] = -l[i];
        }
        h.take_pads(&l, &r);
        let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
        h.period(&b, &mut l, &mut r);
        f += PERIOD as u64;
    }
    // The song stops; the free loop plays on, off the grid.
    t.command(t.frame(), T::Stop);
    let out = run_host(&mut h, &mut t, &mut b, 400);
    assert!(
        out[out.len() / 2..].iter().any(|&v| v != 0.0),
        "it plays on"
    );
    assert_eq!(t.state(), mpfree_transport::State::Stopped);
    // Told as free: a free cell on audio tracks (loops_on_tracks).
    assert!(h.grid_now().unwrap()[0].free, "told as free");
    h.save();
    let saved = saved_from(&mut h, &mut t, &mut b);
    let clock = saved.free_clock.expect("its clock is saved");
    let (_, file) = &saved.loops[0];
    assert_eq!(file.clock, Clock::Free);
    assert_eq!(
        (clock.bpm, clock.top),
        (120.0, file.origin),
        "bar 1: its start"
    );
    // Opened again: every loop empties, then the project's go in.
    h.clear_all();
    h.open(saved);
    let start = std::time::Instant::now();
    let line = loop {
        run_host(&mut h, &mut t, &mut b, 1);
        if let Some(line) = h.tend_open() {
            break line;
        }
        assert!(start.elapsed().as_secs() < 60, "the loops never went in");
    };
    assert!(
        line.contains("loops 1 are free loops, stopped at their tops"),
        "{line}"
    );
    assert!(line.contains("120.00 BPM in 4/4"), "{line}");
    let out = run_host(&mut h, &mut t, &mut b, 400);
    let s = h.looper.loop_status(0).unwrap();
    assert_eq!(
        (s.phase, s.clock, s.loading),
        (Phase::Playing, Clock::Free, None)
    );
    assert!(out.iter().all(|&v| v == 0.0), "stopped at its top");
    assert_eq!(s.position, 0.0);
    assert!(h.looper.free_clock().is_some_and(|c| !c.running));
    assert!(h.grid_now().unwrap()[0].free, "told as free");
    // PLAY: the song, and the free loop with it from its top.
    t.command(t.frame(), T::Play);
    let out = run_host(&mut h, &mut t, &mut b, 200);
    assert!(out.iter().any(|&v| v != 0.0), "it plays");
    assert!(h.looper.free_clock().is_some_and(|c| c.running));
}

/// Capture with Sync: Free, the picked loop playing: it goes into the
/// first empty loop, which takes Sync as last set though it was never
/// picked, and takes the free clock's last bars (the song has
/// stopped; its own last bars are long gone by), which What you
/// played numbers from the free clock's bar 1, where the free loops
/// started.
#[test]
fn capture_with_sync_free_takes_the_free_loops_bars_into_a_free_loop() {
    const BAR: u64 = 88_200;
    const BAR_TICKS: Tick = 3_840;
    let mut h = LoopHost::new(44_100, 20);
    let mut t = Transport::new(44_100, 120.0).unwrap();
    let mut b = Block::new();
    h.command(Command::SetLatency(0));
    h.command(Command::SetClock(Clock::Free));
    h.command(Command::SetBars(Some(1)));
    t.command(0, T::Play);
    let mut run = |h: &mut LoopHost, t: &mut Transport, to: u64, out: &mut Vec<(f32, f32)>| {
        while (out.len() as u64) < to {
            let frame = out.len() as u64;
            assert!(t.process(PERIOD as u32, &mut b));
            let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
            for f in 0..PERIOD {
                (l[f], r[f]) = pads_at(frame + f as u64);
            }
            h.take_pads(&l, &r);
            let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
            h.period(&b, &mut l, &mut r);
            out.extend(l.iter().copied().zip(r.iter().copied()));
        }
    };
    let mut out = Vec::new();
    // Record half a bar in: the free loop starts on bar 2's line, the
    // free clock's bar 1.
    run(&mut h, &mut t, BAR / 2, &mut out);
    h.command(Command::Record);
    while h.looper.status().phase != Phase::Playing {
        let to = out.len() as u64 + PERIOD as u64;
        run(&mut h, &mut t, to, &mut out);
    }
    assert_eq!(h.looper.free_clock().map(|c| c.top), Some(BAR_TICKS));
    // The song stops (its last bar line: bar 4's); the free loop plays
    // on, and its clock with it, on the song's old bar lines.
    run(&mut h, &mut t, 3 * BAR + 1_000, &mut out);
    t.command(t.frame(), T::Stop);
    // Bar 7, beat 2, of the song as it would have gone on.
    run(&mut h, &mut t, 6 * BAR + 30_000, &mut out);
    assert!(h.looper.free_clock().unwrap().running);
    let v = h.view(120.0);
    assert_eq!(
        v.capture_into.map(|i| i.0),
        Some(1),
        "loop 2, the first empty"
    );
    let lines: Vec<i64> = v.heard.lines[..v.heard.n_lines]
        .iter()
        .map(|l| l.0)
        .collect();
    // The song's bars 2 to 7 are the free clock's 1 to 6.
    assert_eq!(lines, [1, 2, 3, 4, 5, 6], "the free clock's bar lines");
    let (_, _, from, bars) = v.heard.take.expect("a take");
    assert_eq!(from + i64::from(bars), 6, "up to its last line");
    let said = h.capture();
    assert!(said.ends_with("become Loop 2, free"), "{said}");
    let end = out.len() as u64 + 8_192;
    run(&mut h, &mut t, end, &mut out);
    let s = h.looper.loop_status(1).unwrap();
    assert_eq!(
        (s.phase, s.clock, s.capturing),
        (Phase::Playing, Clock::Free, None)
    );
    assert_eq!(
        (s.origin, s.ticks),
        (
            (6 - i64::from(bars)) * BAR_TICKS,
            i64::from(bars) * BAR_TICKS
        ),
        "the free clock's bars, not the song's"
    );
    assert_eq!(h.looper.stats().refused, 0);
}

#[test]
fn a_loop_changing_while_it_is_read_starts_the_save_again() {
    let (mut h, mut t, mut b) = saving_host();
    h.save();
    // Wait for the reading to start, then change a layer's mix.
    let start = std::time::Instant::now();
    while h.saving() == Some(0.0) {
        run_host(&mut h, &mut t, &mut b, 1);
        std::thread::sleep(std::time::Duration::from_micros(100));
        assert!(start.elapsed().as_secs() < 60);
    }
    let id = h.looper.layer_info(0).unwrap().id;
    h.looper.layer_command(id, LayerCommand::Level(0.5));
    run_host(&mut h, &mut t, &mut b, 2);
    assert_eq!(h.save_restarts, 1);
    let saved = saved_from(&mut h, &mut t, &mut b);
    assert_eq!(saved.loops[0].1.layers[0].level, 0.5);
    // A load into a loop that is not empty keeps what it has.
    assert!(h.load(saved).contains("loops 1 were not empty"));
}

// ------------------------------------------------------------ the wave

/// A host to record the pads into at 120 BPM (a bar is 88 200 frames,
/// 689 periods and a bit), its first pass `bars` long.
fn wave_rig(bars: Option<u32>) -> (LoopHost, Transport, Block) {
    let mut h = LoopHost::new(44_100, 20);
    let t = Transport::new(44_100, 120.0).unwrap();
    h.command(Command::SetBars(bars));
    h.command(Command::SetLatency(0));
    (h, t, Block::new())
}

/// One period, the pads playing `input` (by song frame, from `n`).
fn wave_period(
    h: &mut LoopHost,
    t: &mut Transport,
    b: &mut Block,
    n: &mut u64,
    input: fn(u64) -> (f32, f32),
) {
    assert!(t.process(PERIOD as u32, b));
    let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
    for f in 0..PERIOD {
        (l[f], r[f]) = input(*n + f as u64);
    }
    *n += PERIOD as u64;
    h.take_pads(&l, &r);
    h.period(b, &mut l, &mut r);
    h.changes(String::new);
}

/// The wave of `layers`' first `upto` frames, drawn against `len`,
/// worked out frame by frame from indexing: the columns, and each
/// layer's peaks.
fn wave_by_frame(
    layers: &[mpfree_looper::LayerView],
    len: usize,
    upto: usize,
) -> (Vec<[WaveColumn; 2]>, Vec<Vec<f32>>) {
    let mut acc = vec![[NO_ACC; 2]; WAVE_COLUMNS];
    let mut peaks = vec![vec![0.0f32; WAVE_COLUMNS]; layers.len()];
    for f in 0..upto {
        let c = f * WAVE_COLUMNS / len;
        for ch in 0..2 {
            let mut v = 0.0;
            for (k, l) in layers.iter().enumerate() {
                let x = l[2 * f + ch];
                v += x;
                peaks[k][c] = peaks[k][c].max(x.abs());
            }
            acc[c][ch] = acc_merge(acc[c][ch], (v, v, v * v, 1));
        }
    }
    let columns = acc.iter().map(|a| [column(a[0]), column(a[1])]).collect();
    (columns, peaks)
}

/// `w`'s first `cols` columns are `want`'s.
fn same_columns(w: &LoopWave, want: &(Vec<[WaveColumn; 2]>, Vec<Vec<f32>>), cols: usize) {
    assert_eq!(w.layers.len(), want.1.len());
    for c in 0..cols {
        for ch in 0..2 {
            let (a, b) = (w.columns[c][ch], want.0[c][ch]);
            assert_eq!((a.min, a.max), (b.min, b.max), "column {c}, channel {ch}");
            assert!(
                (a.rms - b.rms).abs() <= 1e-4,
                "column {c}: rms {} vs {}",
                a.rms,
                b.rms
            );
        }
        for (k, p) in want.1.iter().enumerate() {
            assert_eq!(w.layers[k][c], p[c].min(1.0), "layer {k}, column {c}");
        }
    }
}

/// The loop's wave is built a small slice a period (the budget's
/// frames shared by the layers), and is what the layers hold, frame
/// for frame.
#[test]
fn the_wave_takes_a_small_slice_a_period_and_is_the_layers() {
    let (mut h, mut t, mut b) = wave_rig(Some(1));
    h.command(Command::Record);
    t.command(0, T::Play);
    let mut n = 0;
    while h.looper.status().phase != Phase::Playing {
        wave_period(&mut h, &mut t, &mut b, &mut n, pads_at);
    }
    h.command(Command::Overdub);
    for _ in 0..700 {
        wave_period(&mut h, &mut t, &mut b, &mut n, |f| {
            (0.3 * ((f % 313) as f32 / 313.0), 0.2)
        });
    }
    h.command(Command::Overdub);
    while h.looper.status().phase != Phase::Playing {
        wave_period(&mut h, &mut t, &mut b, &mut n, |_| (0.0, 0.0));
    }
    let s = h.looper.status();
    assert_eq!((s.frames, s.layers), (88_200, 2));
    let mut calls = 0;
    let w = loop {
        wave_period(&mut h, &mut t, &mut b, &mut n, |_| (0.0, 0.0));
        calls += 1;
        if let Some(w) = h.wave_step(calls, 0) {
            break w;
        }
    };
    // 1 024 frames of each layer a period.
    assert_eq!(calls, 88_200usize.div_ceil(WAVE_BUDGET / 2) as u64);
    assert_eq!(
        (w.frames, w.recorded, w.columns.len()),
        (88_200, None, WAVE_COLUMNS)
    );
    let layers = [h.looper.layer(0).unwrap(), h.looper.layer(1).unwrap()];
    same_columns(&w, &wave_by_frame(&layers, 88_200, 88_200), WAVE_COLUMNS);
    // Nothing changes: no new wave.
    for p in 0..200 {
        wave_period(&mut h, &mut t, &mut b, &mut n, |_| (0.0, 0.0));
        assert!(h.wave_step(p, 0).is_none());
    }
}

/// While the first pass records, its wave grows with it every few
/// periods, drawn against the pass's length, and the whole loop's
/// wave follows once it ends.
#[test]
fn the_first_pass_shows_as_it_records() {
    let (mut h, mut t, mut b) = wave_rig(Some(2));
    h.command(Command::Record);
    t.command(0, T::Play);
    let mut n = 0;
    // When each live wave came, and how far it had got (the screen
    // keeps only the last wave).
    let mut live: Vec<(u64, u64)> = Vec::new();
    let mut whole = None;
    let mut checked = false;
    for p in 0..1_600u64 {
        wave_period(&mut h, &mut t, &mut b, &mut n, pads_at);
        let Some(w) = h.wave_step(p, 0) else { continue };
        let s = h.looper.status();
        let Some(r) = w.recorded else {
            whole = Some(w);
            continue;
        };
        assert_eq!(s.phase, Phase::Recording);
        assert!(r as usize <= s.recorded, "{r} of {}", s.recorded);
        assert_eq!(w.frames, 2 * 88_200, "drawn against the 2 bars");
        assert_eq!(w.live(), Some(r as f32 / w.frames as f32));
        if !checked && r > 88_200 {
            // The columns it has filled are the pass so far.
            let pass = h.looper.first_pass(0).unwrap();
            let want = wave_by_frame(&[pass], 2 * 88_200, r as usize);
            same_columns(&w, &want, r as usize * WAVE_COLUMNS / (2 * 88_200));
            checked = true;
        }
        live.push((p, r));
    }
    assert!(checked);
    assert!(live.len() > 100, "{} live waves", live.len());
    for pair in live.windows(2) {
        assert_eq!(pair[1].0 - pair[0].0, LIVE_EVERY);
        assert!(pair[1].1 > pair[0].1);
    }
    let w = whole.expect("the whole loop's wave");
    assert_eq!(
        (w.frames, w.recorded, w.layers.len()),
        (2 * 88_200, None, 1)
    );
    assert_eq!(h.waves.waits, 0);
}

/// A free-length first pass is drawn against two bars, then four once
/// it gets past two, and so on: the columns it had are kept, in pairs.
#[test]
fn a_free_length_pass_widens_its_wave() {
    let (mut h, mut t, mut b) = wave_rig(None);
    h.command(Command::Record);
    t.command(0, T::Play);
    let mut n = 0;
    let mut last = None;
    for p in 0..(3 * 690) {
        wave_period(&mut h, &mut t, &mut b, &mut n, pads_at);
        if let Some(w) = h.wave_step(p, 0) {
            if p < 2 * 680 {
                assert_eq!(w.frames, 2 * 88_200, "period {p}");
            }
            last = Some(w);
        }
    }
    assert_eq!(h.looper.status().phase, Phase::Recording);
    let w = last.unwrap();
    assert_eq!(w.frames, 4 * 88_200);
    let r = w.recorded.unwrap() as usize;
    assert!(r > 2 * 88_200);
    let pass = h.looper.first_pass(0).unwrap();
    let want = wave_by_frame(&[pass], 4 * 88_200, r);
    same_columns(&w, &want, r * WAVE_COLUMNS / (4 * 88_200));
}

/// The wave goes when the loop does: cleared, or its first pass
/// thrown away (STOP before a 1/16 note was played) while its wave was
/// shown.
#[test]
fn the_wave_empties_when_the_loop_does() {
    for clear in [true, false] {
        let (mut h, mut t, mut b) = wave_rig(Some(1));
        h.command(Command::Record);
        t.command(0, T::Play);
        let mut n = 0;
        let until = if clear { 1_000 } else { 30 };
        let mut shown = None;
        for p in 0..until {
            wave_period(&mut h, &mut t, &mut b, &mut n, pads_at);
            if let Some(w) = h.wave_step(p, 0) {
                shown = Some(w);
            }
        }
        assert!(shown.is_some_and(|w| w.frames > 0));
        if clear {
            h.command(Command::Clear);
        } else {
            assert!(t.command(n, T::Stop));
        }
        let mut empty = None;
        for p in until..until + 400 {
            wave_period(&mut h, &mut t, &mut b, &mut n, |_| (0.0, 0.0));
            if let Some(w) = h.wave_step(p, 0) {
                assert!(empty.is_none(), "one empty wave");
                empty = Some(w);
            }
        }
        assert_eq!(h.looper.status().phase, Phase::Empty);
        let w = empty.expect("the empty wave");
        assert_eq!(w.frames, 0, "clear {clear}");
    }
}

/// Making waves never allocates on the audio thread, while the first
/// pass records, overdubs refresh it and a clear empties it: the
/// waves are made again in place once the screen lets go of them.
#[test]
fn making_waves_never_allocates() {
    let (mut h, mut t, mut b) = wave_rig(Some(1));
    h.command(Command::Record);
    t.command(0, T::Play);
    let mut n = 0;
    let mut shown: Option<Arc<LoopWave>> = None;
    let (mut calls, mut waves, mut live) = (0, 0, 0);
    for p in 0..2_400u64 {
        match p {
            900 | 1_700 => {
                h.command(Command::Overdub);
            }
            2_300 => {
                h.command(Command::Clear);
            }
            _ => {}
        }
        wave_period(&mut h, &mut t, &mut b, &mut n, pads_at);
        let ((), c) = crate::sys::alloc_count::calls(|| {
            if let Some(w) = h.wave_step(p, 64) {
                waves += 1;
                live += usize::from(w.recorded.is_some());
                shown = Some(w);
            }
        });
        calls += c;
    }
    assert!(
        live > 40 && waves > live + 5,
        "{live} live of {waves} waves"
    );
    assert_eq!(shown.map(|w| w.frames), Some(0), "cleared");
    assert_eq!(calls, 0, "allocations or frees making waves");
    assert_eq!(h.waves.waits, 0);
}

/// Each loop's small wave for its cells on the Clips grid: made after
/// the loop's own wave, each column the peak of its frames, and
/// emptied with the loop.
#[test]
fn each_loop_gets_a_small_wave_for_its_cells() {
    let (mut h, mut t, mut b) = wave_rig(Some(1));
    h.command(Command::Record);
    t.command(0, T::Play);
    let (mut n, mut p) = (0, 0u64);
    while h.looper.status().phase != Phase::Playing {
        wave_period(&mut h, &mut t, &mut b, &mut n, pads_at);
        h.wave_step(p, 0);
        p += 1;
    }
    // The loop's own wave first, then its small one.
    let mut own = false;
    while h.view(120.0).cell_waves[0] == [0; CELL_WAVE] {
        assert!(p < 2_000, "no small wave");
        wave_period(&mut h, &mut t, &mut b, &mut n, |_| (0.0, 0.0));
        if h.wave_step(p, 0).is_some() {
            own = true;
            assert_eq!(h.view(120.0).cell_waves[0], [0; CELL_WAVE]);
        }
        p += 1;
    }
    assert!(own);
    let len = h.looper.status().frames;
    let mut frames = vec![[0.0f32; 2]; len];
    h.looper.layer(0).unwrap().read(0, &mut frames);
    let mut want = [0u8; CELL_WAVE];
    for (c, w) in want.iter_mut().enumerate() {
        let from = (c * len).div_ceil(CELL_WAVE);
        let to = ((c + 1) * len).div_ceil(CELL_WAVE);
        let peak = frames[from..to]
            .iter()
            .fold(0.0f32, |m, x| m.max(x[0].abs()).max(x[1].abs()));
        *w = (peak.min(1.0) * 255.0).round() as u8;
    }
    assert_eq!(h.view(120.0).cell_waves[0], want);
    assert!(want.iter().all(|&w| w > 100), "{want:?}");
    // Cleared: empty again.
    h.command(Command::Clear);
    for _ in 0..200 {
        wave_period(&mut h, &mut t, &mut b, &mut n, |_| (0.0, 0.0));
        h.wave_step(p, 0);
        p += 1;
    }
    assert_eq!(h.view(120.0).cell_waves[0], [0; CELL_WAVE]);
}

#[test]
fn the_log_says_what_the_beat_played_into_a_loop_said() {
    let r = |bars, beat| Reading {
        bars,
        bpm: 100.0,
        sure: Sure::Clear,
        other: None,
        said: None,
        beat,
    };
    assert_eq!(reading_words(&r(1, None)), "1 bar at 100.00 BPM");
    assert_eq!(
        reading_words(&r(1, Some(0.9))),
        "1 bar at 100.00 BPM, which the beat you played fits"
    );
    assert_eq!(
        reading_words(&r(2, Some(0.3))),
        "2 bars at 100.00 BPM, which the beat you played fits only loosely"
    );
    assert_eq!(
        reading_words(&r(3, Some(0.9))),
        "3 bars at 100.00 BPM, from the beat you played"
    );
}
