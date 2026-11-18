//! Sample Recall's overlay (sampler R5 part 2): the bracket over the
//! master's last two minutes, its lengths, where it ends, its caption and
//! its Q-Links, fed as the audio thread feeds the buffer.

use mpfree_sampler::record::{input16, Feed, Input};
use mpfree_sampler_ui::recall::{Ask, Bracket, Length, Recall, RecallPage, Row, LAYOUT, PAGE};

const RATE: u32 = 44_100;
const PERIOD: u64 = 128;
/// 4/4 at 92 BPM.
const BAR: u64 = 115_043;

/// `seconds` of a quiet ramp, with bar lines numbered from bar 1 at frame
/// 0 while `rolling`.
fn feed(f: &mut Feed, seconds: f64, rolling: bool) {
    let periods = (seconds * f64::from(RATE) / PERIOD as f64).round() as u64;
    for _ in 0..periods {
        let base = f.audio.written();
        let end = base + PERIOD;
        if rolling {
            let mut b = base.div_ceil(BAR) * BAR;
            while b < end {
                f.bars.mark_bar(b, (b / BAR) as u32 + 1);
                b += BAR;
            }
        }
        f.bars.set_bar(rolling.then_some(BAR as f64));
        let v: Vec<f32> = (base..end)
            .flat_map(|n| {
                let x = (n % 1_000) as f32 / 2_000.0;
                [x, -x]
            })
            .collect();
        f.audio.push(&v);
    }
}

fn input() -> (Feed, Input) {
    input16(120, RATE)
}

#[test]
fn while_the_music_plays_it_opens_on_the_last_four_bars_warped() {
    let (mut f, i) = input();
    // 50 s: bar lines up to bar 20 (frame 19 * BAR = 2 185 817).
    feed(&mut f, 50.0, true);
    let r = Recall::open(&i, 5, 5, 128);
    assert_eq!((r.length, r.warp, r.back), (Length::Bars(4), true, 0));
    let b = r.bracket(&i).unwrap();
    assert_eq!(
        b,
        Bracket {
            start: 15 * BAR,
            end: 19 * BAR,
            bars: Some((16, 19)),
        }
    );
    assert_eq!(
        r.caption(Some(&b), RATE, 92.0),
        "Bars 16 to 19 · 10.43 s · lands warped at 92 BPM"
    );
    assert_eq!(r.ends_text(Some(&b)), "bar 19");
    // Ends at, a bar back and forward again; never past now.
    let mut r2 = r;
    r2.qlink(1, -1, &i);
    assert_eq!(r2.bracket(&i).unwrap().bars, Some((15, 18)));
    r2.qlink(1, 5, &i);
    assert_eq!(r2.back, 0);
    // No further back than the buffer holds: 2 minutes are not there yet,
    // 50 s are.
    r2.qlink(1, -100, &i);
    assert_eq!(r2.back, 0, "50 s hold no 100 bars back");
    r2.qlink(1, -10, &i);
    assert_eq!(r2.bracket(&i).unwrap().bars, Some((6, 9)));
}

#[test]
fn lengths_step_through_bars_then_free_seconds() {
    let (mut f, i) = input();
    feed(&mut f, 50.0, true);
    let mut r = Recall::open(&i, 5, 5, 128);
    assert_eq!(r.step(Row::Length, -2, &i), Ask::Nothing);
    assert_eq!(r.length, Length::Bars(1));
    assert_eq!(r.bracket(&i).unwrap().bars, Some((19, 19)));
    assert!(r
        .caption(r.bracket(&i).as_ref(), RATE, 92.0)
        .starts_with("Bar 19 · 2.6"));
    // Free: as played, ending at the frame it opened at.
    r.step(Row::Length, 5, &i);
    assert_eq!(r.length, Length::Free(10));
    assert!(!r.warps(), "Free lands as played");
    let b = r.bracket(&i).unwrap();
    assert_eq!(
        (b.end, b.frames(), b.bars),
        (r.at, 10 * u64::from(RATE), None)
    );
    assert_eq!(
        r.caption(Some(&b), RATE, 92.0),
        "The last · 10.00 s · lands as played"
    );
    // The Q-Link goes on through Free's seconds, and stops at the ends.
    for _ in 0..20 {
        r.qlink(0, 1, &i);
    }
    assert_eq!(r.length, Length::Free(120));
    let b = r.bracket(&i).unwrap();
    assert_eq!(b.start, i.audio.oldest(), "as much as is kept");
    for _ in 0..20 {
        r.qlink(0, -1, &i);
    }
    assert_eq!(r.length, Length::Bars(1));
    // Ends at on Free: a tenth of a second a tick.
    r.step(Row::Length, 5, &i);
    r.qlink(1, -15, &i);
    assert_eq!(r.ends_text(r.bracket(&i).as_ref()), "1.5 s back");
    assert_eq!(r.bracket(&i).unwrap().end, r.at - 15 * u64::from(RATE) / 10);
}

#[test]
fn stopped_it_opens_on_the_last_ten_seconds_and_the_rest_steps() {
    let (mut f, i) = input();
    feed(&mut f, 30.0, false);
    let mut r = Recall::open(&i, 37, 38, 128);
    assert_eq!((r.length, r.warp), (Length::Free(10), false));
    assert_eq!(r.bracket(&i).unwrap().frames(), 10 * u64::from(RATE));
    // Whole bars with none come in: nothing to keep, and it says why.
    r.step(Row::Length, -5, &i);
    assert_eq!(r.length, Length::Bars(1));
    assert_eq!(r.bracket(&i), None);
    assert_eq!(
        r.caption(None, RATE, 92.0),
        "Not enough bars kept yet: pick fewer, or Free"
    );
    // Lands on, Fade, Pad, Warp; the buttons ask.
    r.step(Row::LandsOn, 1, &i);
    assert_eq!(r.onto, 39);
    r.qlink(3, -100, &i);
    assert_eq!(r.onto, 0);
    r.qlink(2, 100, &i);
    assert_eq!(r.fade_ms, 50);
    r.qlink(2, -3, &i);
    assert_eq!(r.fade_ms, 47);
    r.step(Row::Warp, 1, &i);
    assert!(r.warp);
    assert_eq!(r.step(Row::Audition, 0, &i), Ask::Audition);
    assert_eq!(r.step(Row::Keep, 0, &i), Ask::Keep);
}

/// The Sample page's Keep card (one-page step 2c) keeps what Recall opens
/// on, in one tap: the last 4 bars playing (fewer early on), the last 10 s
/// stopped, and says which.
#[test]
fn the_keep_card_keeps_what_recall_opens_on_and_says_how_much() {
    let (mut f, i) = input();
    // 6 s in: bar lines at bars 1 to 3, two whole bars behind the latest.
    feed(&mut f, 6.0, true);
    let r = Recall::last(&i, 4, 10, 0, 0, 128);
    assert_eq!(r.length, Length::Bars(2), "as many as have passed");
    assert_eq!(r.length.keep_text(), "Keep last 2 bars");
    assert!(r.warps());
    feed(&mut f, 44.0, true);
    let r = Recall::last(&i, 4, 10, 0, 0, 128);
    assert_eq!(r, Recall::open(&i, 0, 0, 128));
    assert_eq!(r.length.keep_text(), "Keep last 4 bars");
    assert_eq!(Recall::last(&i, 8, 10, 0, 0, 128).length, Length::Bars(8));
    // Stopped: seconds, as played.
    let (mut f, i) = input();
    feed(&mut f, 30.0, false);
    let r = Recall::last(&i, 4, 10, 0, 0, 128);
    assert_eq!((r.length, r.warps()), (Length::Free(10), false));
    assert_eq!(r.length.keep_text(), "Keep last 10 s");
    assert_eq!(Length::Bars(1).keep_text(), "Keep last bar");
    assert_eq!(Length::Free(120).keep_text(), "Keep last 2:00");
}

#[test]
fn the_page_shows_the_bracket_the_panel_and_the_cards() {
    use mpfree_sampler_ui::recall::{RecallView, LIST, QLINKS, VIEW};
    use mpfree_sampler_ui::widgets::SettingList;
    use mpfree_ui::qlink::QLinkColumn;
    let (mut f, i) = input();
    feed(&mut f, 50.0, true);
    let (project, _) = mpfree_sampler_ui::sample::project();
    let mut ui = mpfree_sampler_ui::sample::app_ui_with(
        mpfree_ui::Fonts::embedded(),
        &project,
        mpfree_sampler_ui::recall::HINT,
    );
    RecallPage::add(&mut ui, RATE).unwrap();
    assert!(LAYOUT.contains(VIEW));
    let r = Recall::open(&i, 5, 6, 128);
    // A6 has a sound; A7 is the next free pad.
    let mut filled = vec![false; 128];
    filled[5] = true;
    let mut p = RecallPage::open(&mut ui, &i, r, 92.0, filled, Ok("Line in".to_string()));
    ui.show(PAGE).unwrap();
    let v = ui.get::<RecallView>(VIEW).unwrap();
    assert_eq!(v.chip, "REC → Pad A7 · Master, effects on");
    assert_eq!(
        v.caption,
        "Bars 16 to 19 · 10.43 s · lands warped at 92 BPM"
    );
    assert_eq!(
        v.note,
        "Lands trimmed on A7, ready to chop: SAMPLE EDIT opens it."
    );
    assert_eq!(
        v.zoom_bars.iter().map(|z| z.1).collect::<Vec<_>>(),
        [16, 17, 18, 19]
    );
    assert!(!v.zoom.is_empty() && v.bars.len() >= 19);
    let list = ui.get::<SettingList>(LIST).unwrap();
    assert_eq!(list.rows[Row::LandsOn.index()].value, "Pad A7 · next free");
    let q = ui.get::<QLinkColumn>(QLINKS).unwrap();
    let cards: Vec<(&str, &str)> = q
        .cards
        .iter()
        .map(|c| (c.name.as_str(), c.value.as_str()))
        .collect();
    assert_eq!(
        cards,
        [
            ("Length", "4 bars"),
            ("Ends at", "bar 19"),
            ("Fade", "5 ms"),
            ("Pad", "A7")
        ]
    );
    // A tap on Free, the wheel and its push, a Q-Link.
    assert_eq!(p.step(&mut ui, &i, Row::Length.index(), 3), Ask::Nothing);
    assert_eq!(p.recall.length, Length::Free(10));
    assert_eq!(p.wheel(&mut ui, &i, 4, false), Ask::Nothing);
    assert_eq!(p.wheel(&mut ui, &i, 1, true), Ask::Keep);
    p.qlink(&mut ui, &i, 3, -1);
    let v = ui.get::<RecallView>(VIEW).unwrap();
    assert_eq!(v.chip, "REC → Pad A6 · Master, effects on");
    assert!(v.note.starts_with("Replaces A6's sound"));
    let list = ui.get::<SettingList>(LIST).unwrap();
    assert_eq!(
        list.rows[Row::LandsOn.index()].value,
        "Pad A6 · replaces its sound"
    );
    p.qlink(&mut ui, &i, 3, 3);
    let list = ui.get::<SettingList>(LIST).unwrap();
    assert_eq!(list.rows[Row::LandsOn.index()].value, "Pad A9 · empty");
    // It paints, and is drawn again only when the music moves it on.
    let mut c = mpfree_ui::SoftCanvas::new(1280, 800);
    ui.paint(&mut c);
    let painted = ui.get::<RecallView>(VIEW).unwrap().painted();
    assert!(painted.is_some());
    p.tick(&mut ui, &i, 0.5);
    ui.paint(&mut c);
    assert_eq!(
        ui.get::<RecallView>(VIEW).unwrap().painted(),
        painted,
        "nothing came in"
    );
    feed(&mut f, 1.0, true);
    p.tick(&mut ui, &i, 0.01);
    ui.paint(&mut c);
    let moved = ui.get::<RecallView>(VIEW).unwrap().painted();
    assert_ne!(moved, painted, "a second came in");
    feed(&mut f, 1.0, true);
    p.tick(&mut ui, &i, 0.01);
    ui.paint(&mut c);
    assert_eq!(
        ui.get::<RecallView>(VIEW).unwrap().painted(),
        moved,
        "too soon"
    );
    p.tick(&mut ui, &i, 0.1);
    ui.paint(&mut c);
    assert_ne!(ui.get::<RecallView>(VIEW).unwrap().painted(), moved);
}

/// From (the one Sample page's step 2f: Skip back moves into Sample
/// Recall): the master, or what the recorder's Source has kept. A switch
/// reads the other buffer from its start, with the bracket ending there
/// now; with no Source to read, From is faint and says why.
#[test]
fn from_reads_the_master_or_what_the_recorders_source_kept() {
    use mpfree_sampler_ui::panel::Look;
    use mpfree_sampler_ui::recall::{RecallFrom, RecallView, LIST, VIEW};
    use mpfree_sampler_ui::widgets::SettingList;
    let (mut f, master) = input();
    feed(&mut f, 50.0, true);
    // The Source's buffer holds 75 s, and has had 20 s of the music.
    let (mut lf, line) = input16(75, RATE);
    feed(&mut lf, 20.0, true);
    let (project, _) = mpfree_sampler_ui::sample::project();
    let mut ui = mpfree_sampler_ui::sample::app_ui_with(
        mpfree_ui::Fonts::embedded(),
        &project,
        mpfree_sampler_ui::recall::HINT,
    );
    RecallPage::add(&mut ui, RATE).unwrap();
    let r = Recall::open(&master, 5, 5, 128);
    assert_eq!(r.from, RecallFrom::Master);
    let mut p = RecallPage::open(
        &mut ui,
        &master,
        r,
        92.0,
        vec![false; 128],
        Ok("Line in".to_string()),
    );
    ui.show(PAGE).unwrap();
    let from = Row::From.index();
    let list = ui.get::<SettingList>(LIST).unwrap();
    assert_eq!(list.rows[from].value, "Master");
    assert!(matches!(
        &list.controls[from].look,
        Look::Choice { options, on: 0, off } if options == &["Master", "Line in"] && off.is_empty()
    ));
    assert_eq!(
        ui.get::<RecallView>(VIEW).unwrap().sub,
        "the master's last 2:00, always kept"
    );
    // A tap on Line in asks for its buffer; the app hands it over.
    assert_eq!(
        p.step(&mut ui, &master, from, 1),
        Ask::Switch(RecallFrom::Source)
    );
    assert_eq!(p.recall.from, RecallFrom::Master, "not until it reads it");
    p.switch(&mut ui, &line, RecallFrom::Source);
    assert_eq!(
        (p.recall.from, p.recall.at, p.recall.length),
        (RecallFrom::Source, line.now(), Length::Bars(4))
    );
    let b = p.recall.bracket(&line).unwrap();
    assert_eq!(b.end, 7 * BAR, "bar 8's line, the latest in 20 s");
    let v = ui.get::<RecallView>(VIEW).unwrap();
    assert_eq!(v.chip, "REC → Pad A6 · Line in");
    assert_eq!(v.sub, "Line in's last 1:15, kept while it is the Source");
    // The overview spans the 75 s (to a frame a column).
    assert!(
        75 * u64::from(RATE) - v.strip.span() < 600,
        "{}",
        v.strip.span()
    );
    let list = ui.get::<SettingList>(LIST).unwrap();
    assert_eq!(list.rows[from].value, "Line in");
    // Every section's note fits beside its title with Line in picked, on
    // the screen's 1280 px (panel.rs: 12 px in from a section's edge, 8 px
    // between the title and the note).
    let rect = ui.rect_of(LIST).unwrap();
    let panel = list.panel.as_ref().unwrap();
    let mut fonts = mpfree_ui::Fonts::embedded();
    for (s, sr) in panel.sections.iter().zip(panel.section_rects(rect)) {
        let used = fonts.measure(mpfree_ui::FontId::Bold, 12.0, &s.title.to_uppercase())
            + 8
            + fonts.measure(mpfree_ui::FontId::Sans, 12.0, &s.note);
        assert!(
            s.note.is_empty() || used <= sr.w - 2 * 12,
            "{}: {used} px of {}",
            s.title,
            sr.w - 2 * 12
        );
    }
    // The wheel's push on From goes back to the master.
    assert_eq!(
        p.step(&mut ui, &line, from, 0),
        Ask::Switch(RecallFrom::Master)
    );
    p.switch(&mut ui, &master, RecallFrom::Master);
    assert_eq!(p.recall.bracket(&master).unwrap().bars, Some((16, 19)));
    assert_eq!(
        ui.get::<RecallView>(VIEW).unwrap().strip.span(),
        120 * u64::from(RATE)
    );
    // No Source to read: its choice is faint (Master stays lit), and the
    // control holds why for a tap to say.
    p.second = Err("Line in off: Looper tab, MENU".to_string());
    p.show(&mut ui, &master);
    let list = ui.get::<SettingList>(LIST).unwrap();
    let c = &list.controls[from];
    assert!(!c.faint && c.why == "Line in off: Looper tab, MENU");
    assert!(
        matches!(&c.look, Look::Choice { options, on: 0, off } if off == &[1] && options[1] == "Source")
    );
}
