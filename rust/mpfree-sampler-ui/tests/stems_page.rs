//! The Stems page (the stems plan's step 3): where the stems land, what
//! the main button says and asks, what can't start and why, the split's
//! progress on the lanes, the levels on the Q-Links, all on the preview's
//! kit ("Soul loop", 24.6 s, on A5; sounds on A1 to A4 and A13).

use mpfree_sampler::stems::Peaks;
use mpfree_sampler_ui::panel::Look;
use mpfree_sampler_ui::stems_page::{
    self, sample, Ask, Phase, Put, Reads, Row, StemsPage, StemsView, LIST, NAMES, QLINKS, TINTS,
    VIEW,
};
use mpfree_sampler_ui::widgets::SettingList;
use mpfree_ui::qlink::QLinkColumn;
use mpfree_ui::{Fonts, Gesture, SoftCanvas, Touch, Ui};

fn page() -> (Ui, StemsPage) {
    sample(Fonts::embedded()).expect("the preview's page")
}

fn list(ui: &Ui) -> &SettingList {
    ui.get::<SettingList>(LIST).expect("the panel")
}

/// The main button's text and whether it is faint.
fn go(ui: &Ui) -> (String, bool) {
    let c = &list(ui).controls[Row::Go.index()];
    match &c.look {
        Look::Primary(t) | Look::Button(t) => (t.clone(), c.faint),
        l => panic!("the main button is a button, not {l:?}"),
    }
}

fn note(ui: &Ui, title: &str) -> (String, bool) {
    let s = list(ui)
        .panel
        .as_ref()
        .unwrap()
        .sections
        .iter()
        .find(|s| s.title == title)
        .unwrap_or_else(|| panic!("no {title} section"));
    (s.note.clone(), s.warn)
}

/// Taps the control labelled `label` on `widget`, and gives the page its
/// event.
fn tap(ui: &mut Ui, page: &mut StemsPage, widget: &str, label: &str) -> Ask {
    let t = ui
        .targets()
        .into_iter()
        .find(|t| t.widget == widget && t.target.label == label)
        .unwrap_or_else(|| panic!("no \"{label}\" on {widget}"))
        .target;
    let (x, y) = t.centre();
    let ev = ui
        .touch(1, Touch::Down, x, y)
        .expect("the tap does something");
    ui.touch(1, Touch::Up, x, y);
    if ev.widget == VIEW {
        page.view_event(ui, &ev.kind)
    } else {
        page.event(ui, &ev.kind)
    }
}

#[test]
fn it_opens_ready_to_split_onto_the_four_empty_pads_after_the_sounds() {
    let (ui, page) = page();
    let s = &page.stems;
    assert_eq!(
        (s.reads, s.put, s.keep),
        (Reads::Trimmed, Put::Pads, [true; 4])
    );
    assert_eq!(s.landing(), Some(vec![5, 6, 7, 8]), "A6 to A9");
    assert_eq!(go(&ui), ("Split to A6–A9".into(), false));
    assert_eq!(note(&ui, "Put on"), ("no pad replaced".into(), false));
    assert_eq!(note(&ui, "Split").0, "4 of 4 kept");
    // 24.6 s: three segments at the plan's 10.4 s each.
    assert_eq!(s.segments(), 3);
    let values: Vec<_> = list(&ui).rows.iter().map(|r| r.value.clone()).collect();
    assert_eq!(
        values[Row::Takes.index()],
        "about 31 s (2 cores, full band)"
    );
    assert_eq!(
        values[Row::Holds.index()],
        "176 MB while it runs · 1.1 GB free"
    );
    // Four 16-bit stereo stems of 24.6 s: 4.3 MB each.
    assert_eq!(values[Row::Makes.index()], "4 sounds · 17 MB");
    let v = ui.get::<StemsView>(VIEW).unwrap();
    assert_eq!(v.segments.len(), 3, "the segments marked along the top");
    assert!(v.lanes.iter().all(Vec::is_empty), "nothing until the split");
    assert_eq!(v.states, ["kept", "kept", "kept", "kept"].map(String::from));
}

#[test]
fn a_pad_tapped_on_the_map_starts_them_there_and_says_what_it_replaces() {
    let (mut ui, mut page) = page();
    // The map's pad 3 (A3) has a sound, and A4 too.
    let ask = tap(&mut ui, &mut page, LIST, "Pads: pad 3");
    assert_eq!(ask, Ask::Nothing);
    assert_eq!(page.stems.landing(), Some(vec![2, 3, 4, 5]));
    assert_eq!(go(&ui).0, "Split to A3–A6");
    assert_eq!(note(&ui, "Put on"), ("replaces A3, A4, A5".into(), false));
    // From A15 four don't fit in the kit's bank... they do (A15 to B2);
    // from the kit's last pad they don't.
    page.stems.from = Some(page.stems.room.filled.len() - 2);
    page.show(&mut ui);
    assert!(go(&ui).1, "faint");
    assert!(note(&ui, "Put on").1, "the reason in the warning colour");
    assert!(page.stems.step(Row::Go, 1) != Ask::Split);
}

#[test]
fn a_stem_switched_off_is_not_kept_and_with_none_kept_nothing_starts() {
    let (mut ui, mut page) = page();
    assert_eq!(tap(&mut ui, &mut page, VIEW, "Drums kept"), Ask::Nothing);
    assert_eq!(page.stems.keep, [true, false, true, true]);
    assert_eq!(page.stems.landing(), Some(vec![5, 6, 7]));
    assert_eq!(go(&ui).0, "Split to A6–A8");
    assert_eq!(note(&ui, "Split").0, "3 of 4 kept");
    assert_eq!(ui.get::<StemsView>(VIEW).unwrap().states[1], "not kept");
    for k in [0, 2, 3] {
        tap(&mut ui, &mut page, VIEW, &format!("{} kept", NAMES[k]));
    }
    assert_eq!(page.stems.kept(), 0);
    assert!(go(&ui).1);
    let why = "Every stem is switched off: tap one to keep it";
    assert_eq!(note(&ui, "Put on"), (why.into(), true));
    assert_eq!(page.step(&mut ui, Row::Go.index(), 1), Ask::Say(why.into()));
    assert_eq!(page.stems.hint(), why);
}

#[test]
fn put_on_names_what_the_main_button_does() {
    let (mut ui, mut page) = page();
    let put = |page: &mut StemsPage, ui: &mut Ui, p: Put| {
        let at = Put::ALL.iter().position(|q| *q == p).unwrap() as i32;
        let on = Put::ALL.iter().position(|q| *q == page.stems.put).unwrap() as i32;
        page.step(ui, Row::Put.index(), at - on)
    };
    for (p, text) in [
        (Put::Layers, "Split into A5's layers"),
        (Put::Takes, "Split to 4 takes"),
        (Put::NewKit, "Split to new kit"),
        (Put::Library, "Save 4 stems"),
        (Put::Pads, "Split to A6–A9"),
    ] {
        assert_eq!(put(&mut page, &mut ui, p), Ask::Nothing);
        assert_eq!(go(&ui), (text.to_string(), false), "{p:?}");
    }
    // A take on the page has no pad to layer.
    page.stems.sound.pad = None;
    page.show(&mut ui);
    let ask = put(&mut page, &mut ui, Put::Layers);
    assert!(
        matches!(ask, Ask::Say(ref w) if w.contains("no pad")),
        "{ask:?}"
    );
    assert_eq!(page.stems.put, Put::Pads);
    // The page full of takes.
    page.stems.room.takes = page.stems.room.max_takes - 2;
    page.show(&mut ui);
    assert!(matches!(put(&mut page, &mut ui, Put::Takes), Ask::Say(_)));
}

#[test]
fn too_little_memory_says_how_much_it_needs() {
    let (mut ui, mut page) = page();
    page.stems.room.holds = 190_000_000;
    page.stems.room.free = 120_000_000;
    page.show(&mut ui);
    assert_eq!(
        note(&ui, "Put on"),
        ("Needs 190 MB, 120 MB free".into(), true)
    );
    assert!(go(&ui).1);
}

#[test]
fn while_it_splits_the_main_button_stops_it_and_the_settings_wait() {
    let (mut ui, mut page) = page();
    assert_eq!(page.step(&mut ui, Row::Go.index(), 1), Ask::Split);
    page.stems.started();
    // The first segment out, the drums' network running in the second.
    let peaks = Peaks {
        per: 1_085,
        columns: 1_000,
        stems: [0, 1, 2, 3].map(|_| vec![(-0.2, 0.2); 484]),
    };
    let progress = 8.0 / 18.0;
    page.splitting(&mut ui, progress, 12.0, &peaks);
    assert_eq!(go(&ui), ("Stop".into(), false));
    assert_eq!(note(&ui, "Put on").0, "lands on A6–A9");
    let v = ui.get::<StemsView>(VIEW).unwrap();
    assert_eq!(v.progress, Some(progress));
    assert_eq!(v.lanes[2].len(), 484);
    assert_eq!(
        v.states,
        ["1 of 3 done", "working…", "1 of 3 done", "1 of 3 done"].map(String::from)
    );
    assert!(v.title.contains("splitting segment 2 of 3"), "{}", v.title);
    // 44 % in 12 s: 15 s left.
    assert_eq!(v.right, "44 % · about 15 s left");
    // Its settings say why they wait; the switches too.
    assert!(matches!(
        page.step(&mut ui, Row::Reads.index(), 1),
        Ask::Say(_)
    ));
    assert!(matches!(page.stems.toggle(0), Ask::Say(_)));
    assert_eq!(page.stems.reads, Reads::Trimmed);
    assert_eq!(page.step(&mut ui, Row::Go.index(), 1), Ask::Stop);
}

#[test]
fn once_landed_the_stems_play_and_a_change_lets_it_split_again() {
    let (mut ui, mut page) = page();
    page.stems.started();
    page.stems.landed(29.4);
    page.show(&mut ui);
    assert_eq!(page.stems.phase, Phase::Landed("On A6–A9".into()));
    assert_eq!(go(&ui), ("On A6–A9".into(), true));
    assert_eq!(note(&ui, "Put on").0, "UNDO takes them back");
    let v = ui.get::<StemsView>(VIEW).unwrap();
    assert_eq!(v.states[2], "on A8");
    // The next estimate goes by this split: 9.8 s a segment.
    assert_eq!(
        list(&ui).rows[Row::Takes.index()].value,
        "about 29 s (2 cores, full band)"
    );
    page.set_lanes(
        &mut ui,
        &Peaks {
            per: 1_085,
            columns: 1_000,
            stems: [0, 1, 2, 3].map(|_| vec![(-0.2, 0.2); 1_000]),
        },
    );
    assert_eq!(tap(&mut ui, &mut page, VIEW, "Hear Bass"), Ask::Play(2));
    assert_eq!(tap(&mut ui, &mut page, LIST, "Play all four"), Ask::PlayAll);
    // Whole sound now: it can split again.
    page.step(&mut ui, Row::Reads.index(), 1);
    assert_eq!(page.stems.phase, Phase::Ready);
    assert_eq!(go(&ui), ("Split to A6–A9".into(), false));
}

#[test]
fn the_q_links_are_the_stems_levels_in_their_colours() {
    let (mut ui, mut page) = page();
    assert_eq!(page.qlink(&mut ui, 0, 3), Ask::Level(0));
    assert_eq!(page.qlink(&mut ui, 3, -200), Ask::Level(3));
    assert_eq!(page.stems.levels, [1.5, 0.0, 0.0, -48.0]);
    let t = ui.theme.clone();
    let q = ui.get::<QLinkColumn>(QLINKS).unwrap();
    let names: Vec<_> = q.cards.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, NAMES);
    assert_eq!(q.cards[0].value, "1.5 dB");
    assert_eq!(q.cards[3].value, "-48.0 dB");
    for (k, c) in q.cards.iter().enumerate() {
        assert_eq!(c.color, Some(t.track(TINTS[k])));
    }
    // The panel's knobs are the same levels.
    assert_eq!(list(&ui).rows[Row::Level(0).index()].value, "1.5 dB");
}

#[test]
fn the_wheel_walks_the_controls_and_passes_the_lines_of_text() {
    let (mut ui, mut page) = page();
    ui.get_mut::<SettingList>(LIST).unwrap().selected = Row::Reads.index();
    page.wheel(&mut ui, 1, false);
    assert_eq!(list(&ui).selected, Row::Map.index());
    page.wheel(&mut ui, -1, false);
    assert_eq!(list(&ui).selected, Row::Reads.index());
}

#[test]
fn every_screen_paints_and_every_control_answers() {
    for (name, _, mut ui) in stems_page::samples(Fonts::embedded).unwrap() {
        let (w, h) = ui.size();
        let mut c = SoftCanvas::new(w, h);
        ui.paint(&mut c);
        assert!(
            ui.get::<StemsView>(VIEW).unwrap().painted().is_some(),
            "{name}"
        );
        for t in ui.targets() {
            // Taps; the Q-Link cards and the knobs are dragged.
            if !t.target.enabled
                || !t.widget.starts_with(stems_page::WIDGETS)
                || t.target.gesture != Gesture::Tap
            {
                continue;
            }
            let (x, y) = t.target.centre();
            let ev = ui.touch(9, Touch::Down, x, y);
            ui.touch(9, Touch::Up, x, y);
            assert!(
                ev.is_some(),
                "{name}: {} on {} does nothing ({:?})",
                t.target.label,
                t.widget,
                t.target.gesture
            );
        }
    }
}
