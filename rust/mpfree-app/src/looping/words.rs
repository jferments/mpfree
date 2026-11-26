//! The words the looper's log lines and screens use: lengths and readings,
//! sources, and smart loop end's asks and notes.

use super::*;

/// "1 bar", "2 bars".
/// How a first pass with Tempo from the first loop records: "free", or
/// with Length's bars set, "2 bars at the tempo you play".
pub fn free_words(bars: Option<u32>) -> String {
    match bars {
        Some(n) if n > 0 => format!("{} at the tempo you play", bars_words_n(n)),
        _ => "free".to_string(),
    }
}

/// The shortest a free first pass closes at, before [`LOOP_MAX_BPM`]: "a
/// bar", or Length's "2 bars".
pub fn shortest_words(bars: Option<u32>) -> String {
    match bars {
        Some(n) if n > 1 => format!("{n} bars"),
        _ => "a bar".to_string(),
    }
}

/// How a free first loop's length read, for the log: "1 bar at 96.00
/// BPM", and ", as Length set" when the bars were Length's, or what the
/// beat played into it said.
pub fn reading_words(r: &Reading) -> String {
    format!(
        "{} at {:.2} BPM{}",
        bars_words_n(r.bars),
        r.bpm,
        if r.said.is_some() && !r.overruled() {
            ", as Length set"
        } else {
            match r.beat {
                None => "",
                Some(_) if !r.bars.is_power_of_two() => ", from the beat you played",
                Some(f) if f >= BEAT_FITS => ", which the beat you played fits",
                Some(_) => ", which the beat you played fits only loosely",
            }
        }
    )
}

/// What the screen should hear about a reading, besides its bars and
/// tempo: a close call and the other reading, or a Length it couldn't
/// keep. None for a clear reading.
pub fn reading_note(r: &Reading) -> Option<String> {
    if let Some(n) = r.said.filter(|_| r.overruled()) {
        return Some(format!(
            "Length was {}, but that would be slower than {LOOP_MIN_BPM:.0} BPM, \
             so the bars were read from the loop",
            bars_words_n(n)
        ));
    }
    match (r.sure, r.other) {
        (Sure::Ambiguous, Some((b, bpm))) => Some(format!(
            "a close call: it could be {} at {bpm:.2} BPM",
            bars_words_n(b)
        )),
        _ => None,
    }
}

pub fn bars_words_n(bars: u32) -> String {
    if bars == 1 {
        "1 bar".to_string()
    } else {
        format!("{bars} bars")
    }
}

/// Bars `first..first + bars`, as words: "bar 3", "bars 10 and 11",
/// "bars 8 to 11".
pub fn bars_words(first: i64, bars: u32) -> String {
    match bars {
        0 | 1 => format!("bar {first}"),
        2 => format!("bars {first} and {}", first + 1),
        b => format!("bars {first} to {}", first + i64::from(b) - 1),
    }
}

/// What a take records from, in words for the log: the source and, for the
/// pads, their instrument.
pub(crate) fn source_words(s: LoopSource, pads: Option<Instrument>) -> String {
    match (s, pads) {
        (LoopSource::Pads, Some(Instrument::MidiOut)) => format!("silence ({PADS_ON_MIDI_OUT})"),
        (LoopSource::Pads, Some(i)) => format!("the pads ({})", i.name()),
        (LoopSource::Pads, None) => "the pads".to_string(),
        (LoopSource::Line, _) => "the line input".to_string(),
        (LoopSource::Mix(mpfree_fx::StripId::Master), _) => "the master".to_string(),
        (LoopSource::Mix(m), _) => crate::mixing::name(m),
        (LoopSource::Track(t), _) => match t.from {
            TrackFrom::Line(i) => {
                format!("the track's input, {}", crate::audiotracks::input_words(i))
            }
            TrackFrom::Mix(m, p) => format!(
                "the track's input, {} {}",
                if m == mpfree_fx::StripId::Master {
                    "the master".to_string()
                } else {
                    crate::mixing::name(m)
                },
                match p {
                    mpfree_fx::CapturePoint::BeforeInserts => "before its effects",
                    mpfree_fx::CapturePoint::AfterFader => "after its fader",
                    _ => "after its effects",
                }
            ),
        },
    }
}

/// Loops by bit (bit `n`: loop `n`), as the log names them: "Loop 1",
/// "Loop 1 and Loop 3", "Loops 1, 2 and 4".
pub(crate) fn loops_words(m: u32) -> String {
    let n: Vec<String> = (0..MAX_LOOPS)
        .filter(|&k| m & (1 << k) != 0)
        .map(|k| (k + 1).to_string())
        .collect();
    match n.as_slice() {
        [] => "no loop".to_string(),
        [a] => format!("Loop {a}"),
        [a, b] => format!("Loop {a} and Loop {b}"),
        [rest @ .., last] => format!("Loops {} and {last}", rest.join(", ")),
    }
}

pub fn bars_text(b: Option<u32>) -> String {
    match b {
        Some(PATTERN_BARS) => "the pattern's length".to_string(),
        Some(1) => "1 bar".to_string(),
        Some(n) => format!("{n} bars"),
        None => "free (until STOP or Record again)".to_string(),
    }
}

/// How a loop's overdubs record, as the log says it.
pub fn overdubs_text(o: mpfree_looper::Overdubs) -> &'static str {
    match o {
        mpfree_looper::Overdubs::Add => "Add (each pass adds a layer until you stop it)",
        mpfree_looper::Overdubs::Replace => {
            "Replace (what you play takes the place of what was there)"
        }
        mpfree_looper::Overdubs::Once => "Once (one pass, then back to playing)",
    }
}

/// A snap as the log names it; `read`: an Undo or Redo is of Read as.
pub(crate) fn snap_name(s: Snap, read: bool) -> String {
    match s {
        Snap::Found => "Snap".to_string(),
        Snap::To(_) => "Move".to_string(),
        Snap::Tap => "Use my tap".to_string(),
        Snap::Press => "Use my press".to_string(),
        Snap::Bars(b) => format!("Read as {}", bars_words_n(b)),
        Snap::Undo if read => "Undo of Read as".to_string(),
        Snap::Redo if read => "Redo of Read as".to_string(),
        Snap::Undo => "Undo of the snap".to_string(),
        Snap::Redo => "Redo of the snap".to_string(),
    }
}

/// The question a snap over recorded audio asks, on the hint line while
/// it waits ([`LoopHost::snap_asking`]): loop `n`, snap `s`, `read` as
/// for [`snap_name`].
pub fn ask_words(n: usize, s: Snap, read: bool) -> String {
    let verb = snap_verb(s, read);
    let mut c = verb.chars();
    let verb = c
        .next()
        .map(|f| f.to_uppercase().chain(c).collect::<String>())
        .unwrap_or_default();
    format!(
        "Loop {}: that changes the tempo under the audio tracks' recordings. {verb} anyway?",
        n + 1
    )
}

/// The question a first pass waiting on its Length asks ([`Status::asks`]):
/// Length's bars would be slower than the looper goes, so read it as the
/// bars it read?
pub fn length_ask_words(n: usize, r: &Reading) -> String {
    let said = r.said.unwrap_or(r.bars);
    let slow = r.bpm * f64::from(said) / f64::from(r.bars.max(1));
    format!(
        "Loop {}: Length's {} would be {slow:.0} BPM, slower than {LOOP_MIN_BPM:.0}. \
         Read it as {} at {:.0} BPM?",
        n + 1,
        bars_words_n(said),
        bars_words_n(r.bars),
        r.bpm
    )
}

/// Read as's words on the hint line while a first pass waits on its
/// Length: "Read as 2 bars".
pub fn length_ask_button(r: &Reading) -> String {
    format!("Read as {}", bars_words_n(r.bars))
}

/// The hint line's button that answers yes to [`ask_words`]: Snap
/// anyway for a snap, Do it anyway for the rest (Read as, an Undo or a
/// Redo).
pub fn ask_button(s: Snap) -> &'static str {
    match s {
        Snap::Found | Snap::Tap | Snap::Press => "Snap anyway",
        Snap::To(_) => "Move anyway",
        Snap::Bars(_) | Snap::Undo | Snap::Redo => "Do it anyway",
    }
}

/// A snap as "can't ..." says it; `read` as for [`snap_name`].
pub(crate) fn snap_verb(s: Snap, read: bool) -> String {
    match s {
        Snap::Found => "snap".to_string(),
        Snap::To(_) => "move it".to_string(),
        Snap::Tap => "go back to your tap".to_string(),
        Snap::Press => "go back to your press".to_string(),
        Snap::Bars(b) => format!("read it as {}", bars_words_n(b)),
        Snap::Undo if read => "undo Read as".to_string(),
        Snap::Redo if read => "redo Read as".to_string(),
        Snap::Undo => "undo the snap".to_string(),
        Snap::Redo => "redo the snap".to_string(),
    }
}

/// Smart loop end's setting, as the log says it.
pub fn smart_end_text(m: SmartEnd) -> &'static str {
    match m {
        SmartEnd::Off => "Off (the loop closes on your tap)",
        SmartEnd::Suggest => "Suggest (it closes on your tap, then offers the end you played)",
        SmartEnd::Auto => {
            "Auto (it listens a moment past your tap, then closes on the end you played)"
        }
    }
}

/// `frames` at `rate` in whole milliseconds, without the sign.
pub(crate) fn ms(frames: i64, rate: u32) -> f64 {
    (frames.unsigned_abs() as f64 * 1000.0 / f64::from(rate.max(1))).round()
}

/// A snap's moves, for the log: "the start 80 ms later and the end 61 ms
/// earlier".
pub(crate) fn shift_text(by: Shift, rate: u32) -> String {
    let way = |f: i64| if f < 0 { "earlier" } else { "later" };
    let mut parts = Vec::new();
    if by.start != 0 {
        parts.push(format!(
            "the start {:.0} ms {}",
            ms(by.start, rate),
            way(by.start)
        ));
    }
    if by.end != 0 || by.start == 0 {
        parts.push(format!(
            "the end {:.0} ms {}",
            ms(by.end, rate),
            way(by.end)
        ));
    }
    parts.join(" and ")
}

/// Your tap was late or early by a move of `by` frames (negative: the
/// end found is earlier, so the tap was late).
fn tap_off(by: i64, rate: u32) -> String {
    format!(
        "your tap was {:.0} ms {}",
        ms(by, rate),
        if by < 0 { "late" } else { "early" }
    )
}

/// One scored end, for the log: where it is from the tap and its score.
fn candidate_text(c: &Candidate, tap: u64, rate: u32) -> String {
    let at = if c.end == tap {
        "your tap".to_string()
    } else {
        let by = c.end as i64 - tap as i64;
        format!(
            "{:.0} ms {} your tap",
            ms(by, rate),
            if by < 0 { "before" } else { "after" }
        )
    };
    format!(
        "{at}, score {:.2} (onset {:.2}, rhythm {:.2}, context {:.2}, prior {:.2})",
        c.score, c.onset, c.rhythm, c.context, c.prior
    )
}

/// What smart loop end made of a closing tap ([`LoopHost::take_smart`]):
/// the log line, with the best two ends scored, and when the tap stood,
/// the words for the screen (every fallback is shown).
pub(crate) fn smart_words(note: &SmartNote, mode: SmartEnd, rate: u32) -> (String, Option<String>) {
    let n = note.n + 1;
    // A smart start: Auto started the loop on your first hit, Suggest
    // offers to.
    let after = format!(
        "on your first hit, {:.0} ms after REC",
        ms(note.start, rate)
    );
    let (started, start_words) = match (note.start, mode) {
        (0, _) => (String::new(), String::new()),
        (_, SmartEnd::Auto) => (
            format!(", and started it {after}"),
            format!(" Started {after}."),
        ),
        _ => (
            format!(", and offers to start it {after}"),
            format!(" Snap starts it {after}."),
        ),
    };
    let scored = match (note.best, note.second) {
        (Some(b), Some(s)) => format!(
            " (best: {}; next: {})",
            candidate_text(&b, note.tap, rate),
            candidate_text(&s, note.tap, rate)
        ),
        (Some(b), None) => format!(" (best: {})", candidate_text(&b, note.tap, rate)),
        _ => String::new(),
    };
    let sure = note
        .sure
        .map_or(String::new(), |c| format!("; how sure: {}", sure_words(&c)));
    let (line, screen) = match note.outcome {
        SmartOutcome::Moved(by) => (
            format!(
                "looper: Loop {n}: {}; smart loop end closed it on the end you played{started}{scored}",
                tap_off(by, rate)
            ),
            None,
        ),
        SmartOutcome::Offered(by) => (
            format!(
                "looper: Loop {n}: {}; smart loop end offers the end you played{started}{scored}",
                tap_off(by, rate)
            ),
            None,
        ),
        SmartOutcome::Close(by) => (
            format!(
                "looper: Loop {n}: smart loop end found the end you played {:.0} ms from your \
                 tap, too near to hear{started}{scored}",
                ms(by, rate)
            ),
            (note.start != 0).then(|| start_words.trim_start().to_string()),
        ),
        SmartOutcome::Kept(why) => (
            format!("looper: Loop {n}: smart loop end kept your tap: {why}{started}{scored}"),
            Some(if why == mpfree_looper::OVERDUB_AT_ONCE {
                format!("Kept your tap: {why}. Auto can fix this before it does.")
            } else {
                format!("Kept your tap: {why}.{start_words}")
            }),
        ),
    };
    (line + &sure, screen)
}

/// How sure smart loop end was of a first pass's close, four ways
/// ([`close::Confidence`]), for the log: "the end clear, the bars a close
/// call, the tempo from the length alone, the start on your first hit".
pub fn sure_words(c: &close::Confidence) -> String {
    let end = match c.boundary {
        Boundary::Clear => "the end clear",
        Boundary::Possible => "the end only possible",
        Boundary::Unmatched => "no end to go on",
    };
    let bars = match c.bars {
        Bars::Yours => "the bars yours",
        Bars::Clear => "the bars clear",
        Bars::Ambiguous => "the bars a close call",
    };
    let tempo = match c.tempo {
        Tempo::FromYourBars => "the tempo from your bars",
        Tempo::FromTheBeat => "the tempo from the beat you played",
        Tempo::FromTheLength => "the tempo from the length alone",
    };
    let start = match c.start {
        Start::FirstHit => "the start on your first hit",
        Start::Press => "the start on your press",
    };
    format!("{end}, {bars}, {tempo}, {start}")
}

/// A loop's length in bars of `bar` ticks, for the log.
pub(crate) fn ticks_text(ticks: i64, bar: Tick) -> String {
    let bar = bar.max(1);
    if ticks % bar == 0 {
        bars_text(Some((ticks / bar) as u32))
    } else {
        format!("{:.2} bars", ticks as f64 / bar as f64)
    }
}
