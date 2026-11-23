//! The patch screens on Play (live shows C1, layout spec 4q): KEYS PLAY on
//! the transport bar, the patch list and the editor in place of Play's
//! view, their events, the wheel, the Q-Links, UNDO, and what the audio
//! loop says of the keys. The screens' state and the edits are
//! [`crate::patch_screen`]'s; the widgets [`crate::patch_page`]'s.

use super::*;
use crate::patch_page::{self as page, PatchEditor, PatchList};
use crate::patch_screen::{Ctx, Edit, Inst, KeysNow, Open};

/// The hints while the list or the editor shows.
pub(super) const LIST_HINT: &str =
    "Tap a patch: the keys play it · Edit: its zones · wheel: move · push: pick · – / +: the patch before or after · Close: back";
pub(super) const EDIT_HINT: &str =
    "Press keys: see what they play · drag a zone's ends · wheel: pick a row · – / +: change it · Learn: the next key sets the end · UNDO: the last change";

/// The editor's Q-Links: the picked zone's Low key, High key, Transpose
/// and Low velocity.
const QLINK_ROWS: [usize; 4] = [
    page::ROW_LOW,
    page::ROW_HIGH,
    page::ROW_TRANSPOSE,
    page::ROW_VEL_LOW,
];

impl Player {
    /// The song's instruments as a zone names them: what each track
    /// plays (not audio or Loop tracks), then the synths no track has.
    pub(super) fn patch_insts(&self) -> Vec<Inst> {
        let mut out: Vec<Inst> = self
            .time
            .tracks
            .iter()
            .enumerate()
            .take(usize::from(mpfree_midi::setup::ROUTE_TRACKS as u16))
            .filter(|(_, t)| !t.audio && t.loops.is_none())
            .map(|(i, t)| Inst {
                words: crate::patch_in::Plays::Track(i as u8).text(),
                name: t.name.clone(),
                about: (!t.sound.is_empty()).then(|| t.sound.clone()),
                color: time_view::track_color(i),
            })
            .collect();
        if let Some(m) = &self.io_model {
            let m = m.lock().unwrap_or_else(|e| e.into_inner());
            let shared = crate::instrument_list::InstrumentId::SHARED_SYNTH.number();
            for (k, (id, name)) in m.live.synths.iter().enumerate() {
                out.push(Inst {
                    words: crate::patch_in::Plays::Synth(*id).text(),
                    name: name.clone(),
                    about: Some(if *id == shared {
                        "the synth".to_string()
                    } else {
                        "free place".to_string()
                    }),
                    color: mpfree_time_ui::view::audio_color(k + 2),
                });
            }
        }
        out
    }

    /// The keyboards plugged in now, by their ports' names.
    fn patch_ports(&self) -> Vec<String> {
        let Some(m) = &self.io_model else {
            return Vec::new();
        };
        let m = m.lock().unwrap_or_else(|e| e.into_inner());
        m.live
            .ports
            .iter()
            .filter(|p| p.has_in)
            .map(|p| p.name.clone())
            .collect()
    }

    /// What the keys play without a patch: the armed track.
    fn patch_routed(&self) -> String {
        format!("track {}", self.time.armed.unwrap_or(0) + 1)
    }

    /// A patch screen shows in place of Play's view.
    pub(super) fn patch_showing(&self) -> bool {
        self.patch.showing() && self.view == View::Play && !self.play.sequencer
    }

    /// The page the patch screen open shows on.
    pub(super) fn patch_page(&self) -> &'static str {
        match self.patch.open {
            Open::Edit(_) => page::EDIT_PAGE,
            _ => page::LIST_PAGE,
        }
    }

    /// The rig's keyboards for the Show view's keyboards card, by the
    /// names the musician gave them, each with the song's patch on it
    /// (live shows C1): its name and the zones that reach that keyboard.
    pub(super) fn show_keyboards(&self) -> Vec<crate::set_play::KeyboardNow> {
        let Some(m) = &self.io_model else {
            return Vec::new();
        };
        let insts = self.patch_insts();
        let m = m.lock().unwrap_or_else(|e| e.into_inner());
        let mut out: Vec<crate::set_play::KeyboardNow> = Vec::new();
        let known = m.setup.ports.iter().map(|p| p.name.as_str());
        for port in known.chain(m.live.ports.iter().map(|p| p.name.as_str())) {
            let name = m.setup.shown(port).to_string();
            if out.iter().any(|k| k.name == name) {
                continue;
            }
            out.push(crate::set_play::KeyboardNow {
                connected: m.live.ports.iter().any(|p| p.name == port),
                patch: self
                    .patch
                    .keyboard_look(&self.patches, &insts, port)
                    .map(|(patch, zones)| (patch, page::strip_bars(&zones))),
                name,
            });
        }
        out
    }

    /// KEYS PLAY, on Play's transport only while the song has patches.
    pub(super) fn patch_keys_look(&self) -> Option<page::KeysPlay> {
        if self.play.sequencer {
            return None;
        }
        self.patch.keys_play(&self.patches, &self.patch_insts())
    }

    /// KEYS PLAY, the list and the editor follow the patches and the keys.
    pub(super) fn refresh_patches(&mut self) {
        let insts = self.patch_insts();
        let keys = if self.play.sequencer {
            None
        } else {
            self.patch.keys_play(&self.patches, &insts)
        };
        if self
            .ui
            .get::<PlayTransport>(play_page::TRANSPORT)
            .is_some_and(|t| t.look.keys != keys)
        {
            if let Some(t) = self.ui.get_mut::<PlayTransport>(play_page::TRANSPORT) {
                t.look.keys = keys;
            }
        }
        match self.patch.open {
            Open::Closed => {}
            Open::List | Open::Layouts => {
                let count = crate::patch_screen::PatchScreen::count(&self.patches);
                let tiles = self.patch.tiles(&self.patches, &insts);
                let layouts = (self.patch.open == Open::Layouts).then(|| {
                    let ctx = Ctx {
                        insts: &insts,
                        ports: &[],
                        routed: self.patch_routed(),
                    };
                    crate::patch_screen::PatchScreen::layouts(&self.patch_plays_now(&ctx), &insts)
                });
                let cursor = Some(self.patch.cursor);
                if let Some(l) = self.ui.get::<PatchList>(page::LIST) {
                    if l.count == count
                        && l.tiles == tiles
                        && l.layouts == layouts
                        && l.cursor == cursor
                    {
                        return;
                    }
                }
                if let Some(l) = self.ui.get_mut::<PatchList>(page::LIST) {
                    l.count = count;
                    l.tiles = tiles;
                    l.layouts = layouts;
                    l.cursor = cursor;
                }
            }
            Open::Edit(i) => {
                let ports = self.patch_ports();
                let ctx = Ctx {
                    insts: &insts,
                    ports: &ports,
                    routed: self.patch_routed(),
                };
                let Some(look) = self.patch.edit_look(&self.patches, i, &ctx) else {
                    self.patch.open = Open::List;
                    self.refresh_patches();
                    if self.play_frame() {
                        self.show_view(self.view);
                    }
                    return;
                };
                if self
                    .ui
                    .get::<PatchEditor>(page::EDIT)
                    .is_some_and(|e| e.look != look)
                {
                    if let Some(e) = self.ui.get_mut::<PatchEditor>(page::EDIT) {
                        e.look = look;
                    }
                }
            }
        }
    }

    /// What a new patch's zones start on.
    fn patch_plays_now(&self, ctx: &Ctx) -> String {
        self.patch
            .keys
            .playing
            .and_then(|i| self.patches.list.get(i))
            .and_then(|p| p.zones.first())
            .map_or_else(|| ctx.routed.clone(), |z| z.plays.clone())
    }

    /// Shows the patch screen as it now stands (or Play's view again).
    fn patch_show(&mut self) {
        self.refresh_patches();
        if self.play_frame() {
            self.show_view(self.view);
        }
        self.refresh_qlinks();
        self.refresh_hint();
    }

    /// Play's MENU, Patches: the patch list opens (a song with no
    /// patches gets its first there), if it doesn't show already.
    pub(super) fn open_patch_list(&mut self) {
        if self.view == View::Play && !self.play.sequencer && self.patch.open != Open::List {
            self.patch.open = Open::Closed;
            self.toggle_patch_list();
        }
    }

    /// KEYS PLAY tapped: the patch list opens, or what shows closes.
    pub(super) fn toggle_patch_list(&mut self) {
        if self.patch.showing() || self.view != View::Play {
            self.patch.open = Open::Closed;
        } else {
            self.patch.open = Open::List;
            self.patch.cursor = self.patch.keys.playing.map_or(0, |i| i + 1);
            self.rec_list = false;
            self.xy_list = false;
            self.xy_full = false;
        }
        self.patch.learn = None;
        self.patch_show();
        if self.patch.showing() {
            let cursor = self.patch.cursor;
            if let (Some(r), Some(l)) = (
                self.ui.rect_on(page::LIST_PAGE, page::LIST),
                self.ui.get_mut::<PatchList>(page::LIST),
            ) {
                l.top = 0;
                l.show_tile(r, cursor);
            }
        }
    }

    /// The keys play patch `to` (from 0; None: as routed), at once.
    fn pick_patch(&mut self, to: Option<usize>) {
        self.outbox.push(FromUi::PickPatch(to));
        self.patch.keys.playing = to;
        self.patch.keys.waiting = None;
        let name = to
            .and_then(|i| {
                self.patches
                    .list
                    .get(i)
                    .map(|p| format!("{} · {}", i + 1, p.name))
            })
            .unwrap_or_else(|| "Keys as routed".to_string());
        self.tell(&format!("KEYS PLAY: {name}"));
    }

    /// An edit made: the audio loop gets the patches, and the patch that
    /// played keeps playing wherever it went.
    fn patch_edited(&mut self, e: Option<Edit>, say: bool) {
        let Some(e) = e else {
            return;
        };
        self.outbox
            .push(FromUi::Patches(crate::patch_in::PatchRig::of(
                &self.patches,
            )));
        if let Some(to) = e.pick {
            self.outbox.push(FromUi::PickPatch(to));
            self.patch.keys.playing = to;
        }
        if say {
            self.tell(&e.title);
        }
        self.refresh_history();
    }

    /// A tap on the patch list.
    fn patch_list_event(&mut self, kind: EventKind) {
        match kind {
            EventKind::Tab(i) => {
                self.pick_patch(i.checked_sub(1));
                self.patch.open = Open::Closed;
            }
            EventKind::Toggle(i) if i > 0 => self.edit_patch(i - 1),
            EventKind::Select(page::CLOSE) => self.patch.open = Open::Closed,
            EventKind::Select(page::NEW) => {
                if self.patches.list.len() >= mpfree_patch::MAX_PATCHES {
                    self.tell("New patch: a song keeps 128 patches, and this one has them all");
                } else {
                    self.patch.open = Open::Layouts;
                    self.patch.cursor = 0;
                }
            }
            EventKind::Select(page::BACK) => {
                self.patch.open = Open::List;
                self.patch.cursor = 0;
            }
            EventKind::Select(k) if k >= page::LAYOUT => {
                let insts = self.patch_insts();
                let ctx = Ctx {
                    insts: &insts,
                    ports: &[],
                    routed: self.patch_routed(),
                };
                let e = self
                    .patch
                    .new_patch(&mut self.patches, k - page::LAYOUT, &ctx);
                self.patch_edited(e, true);
                if let Open::Edit(i) = self.patch.open {
                    self.pick_patch(Some(i));
                }
            }
            _ => {}
        }
        self.patch_show();
    }

    /// Edit on tile `i`: its editor, and the keys play it, so what they
    /// light is what sounds.
    fn edit_patch(&mut self, i: usize) {
        if i >= self.patches.list.len() {
            return;
        }
        self.patch.open = Open::Edit(i);
        self.patch.zone = 0;
        self.patch.learn = None;
        if let Some(e) = self.ui.get_mut::<PatchEditor>(page::EDIT) {
            e.asking = false;
        }
        if self.patch.keys.playing != Some(i) {
            self.pick_patch(Some(i));
        }
    }

    /// A touch on the editor.
    fn patch_edit_event(&mut self, kind: EventKind) {
        let Open::Edit(i) = self.patch.open else {
            return;
        };
        let insts = self.patch_insts();
        let ports = self.patch_ports();
        let ctx = Ctx {
            insts: &insts,
            ports: &ports,
            routed: self.patch_routed(),
        };
        let s = &mut self.patch;
        let list = &mut self.patches;
        let (e, say) = match kind {
            EventKind::Select(page::E_BACK) => {
                s.open = Open::List;
                s.cursor = i + 1;
                s.learn = None;
                (None, false)
            }
            EventKind::Select(page::E_CLOSE) => {
                s.open = Open::Closed;
                s.learn = None;
                (None, false)
            }
            EventKind::Select(page::E_COPY) => (s.copy(list, i), true),
            EventKind::Select(page::E_DELETE) => {
                let asking = self
                    .ui
                    .get::<PatchEditor>(page::EDIT)
                    .is_some_and(|e| e.asking);
                if asking {
                    (s.delete(list, i), true)
                } else {
                    if let Some(e) = self.ui.get_mut::<PatchEditor>(page::EDIT) {
                        e.asking = true;
                    }
                    let name = list.list.get(i).map_or("", |p| p.name.as_str()).to_string();
                    self.tell(&format!(
                        "Delete {name}? Tap Delete again (UNDO brings it back)"
                    ));
                    return;
                }
            }
            EventKind::Select(page::E_ADD) => (s.add_zone(list, i, &ctx), true),
            EventKind::Select(page::E_REMOVE) => {
                let z = s.zone;
                (s.remove_zone(list, i, z), true)
            }
            // ERASE + a lane removes its zone.
            EventKind::Tab(z) if self.status.erase => (s.remove_zone(list, i, z), true),
            EventKind::Tab(z) => {
                s.zone = z;
                s.learn = None;
                (None, false)
            }
            EventKind::Span { index, from, to } => {
                let lo = u8::try_from(from).unwrap_or(0);
                let hi = u8::try_from(to).unwrap_or(127);
                (s.span(list, i, index / 2, lo, hi), false)
            }
            EventKind::Toggle(k) if k == page::E_LEARN || k == page::E_LEARN + 1 => {
                let high = k == page::E_LEARN + 1;
                s.learn = if s.learn == Some(high) {
                    None
                } else {
                    Some(high)
                };
                (None, false)
            }
            EventKind::Toggle(row) => (s.toggle(list, i, row, &ctx), false),
            EventKind::Step { index, delta } => (s.step(list, i, index, delta, &ctx), false),
            EventKind::Type(_) => {
                let name = list.list.get(i).map(|p| p.name.clone()).unwrap_or_default();
                self.open_textpad(
                    vec![mpfree_ui::textpad::TextAsk {
                        key: "patch".to_string(),
                        label: "Patch".to_string(),
                        title: format!("Name patch {}", i + 1),
                        value: name,
                        max_len: mpfree_patch::MAX_NAME,
                    }],
                    TypeTo::PatchName(i),
                );
                return;
            }
            _ => (None, false),
        };
        self.patch_edited(e, say);
        self.patch_show();
    }

    /// The name typed for patch `i` on the text keyboard.
    pub(super) fn patch_named(&mut self, i: usize, name: &str) {
        let e = self.patch.rename(&mut self.patches, i, name);
        self.patch_edited(e, false);
        self.patch_show();
    }

    /// A touch on the list or the editor.
    pub(super) fn patch_event(&mut self, ev: &UiEvent) {
        if ev.widget == page::LIST {
            self.patch_list_event(ev.kind.clone());
        } else {
            self.patch_edit_event(ev.kind.clone());
        }
    }

    /// The wheel, its push and – / + on the list: move, pick, and the
    /// patch before or after. False when the list doesn't show (the
    /// editor's rows take the wheel's outline as any page's do).
    pub(super) fn patch_nav(&mut self, n: Nav) -> bool {
        if !self.patch_showing() || !matches!(self.patch.open, Open::List | Open::Layouts) {
            return false;
        }
        let layouts = self.patch.open == Open::Layouts;
        let n_tiles = if layouts {
            mpfree_patch::Layout::ALL.len()
        } else {
            self.patches.list.len() + 1
        };
        match n {
            Nav::Turn(d) => self.patch.move_cursor(d, n_tiles),
            Nav::Step(d) | Nav::Fine(d) if !layouts => {
                if let Some(to) = self.patch.next_to(&self.patches, d) {
                    self.pick_patch(to);
                    self.patch.cursor = to.map_or(0, |i| i + 1);
                }
            }
            Nav::Step(d) | Nav::Fine(d) => self.patch.move_cursor(d, n_tiles),
            Nav::Push => {
                let c = self.patch.cursor;
                let kind = if layouts {
                    EventKind::Select(page::LAYOUT + c)
                } else {
                    EventKind::Tab(c)
                };
                self.patch_list_event(kind);
                return true;
            }
        }
        let cursor = self.patch.cursor;
        self.refresh_patches();
        if let (Some(r), Some(l)) = (
            self.ui.rect_on(page::LIST_PAGE, page::LIST),
            self.ui.get_mut::<PatchList>(page::LIST),
        ) {
            l.show_tile(r, cursor);
        }
        self.refresh_hint();
        true
    }

    /// What the keys do now, from the audio loop: Learn takes its key,
    /// and the screens follow.
    pub(super) fn patch_keys(&mut self, now: KeysNow) {
        let was = self.patch.keys.playing;
        let e = self.patch.keys_in(&mut self.patches, now);
        let said = e.is_some();
        self.patch_edited(e, false);
        self.refresh_patches();
        if said {
            self.refresh_qlinks();
        }
        // The Show view's keyboards name the patch playing.
        if self.patch.keys.playing != was {
            self.refresh_show();
        }
    }

    /// The editor's Q-Link cards, while it shows a zone.
    pub(super) fn patch_qlinks(&self) -> Option<[QLinkCard; 4]> {
        let Open::Edit(i) = self.patch.open else {
            return None;
        };
        if !self.patch_showing() {
            return None;
        }
        let z = self.patches.list.get(i)?.zones.get(self.patch.zone)?;
        let note = crate::patch_screen::note;
        let cards = [
            (note(z.low), f32::from(z.low) / 127.0, false),
            (note(z.high), f32::from(z.high) / 127.0, false),
            (
                format!("{:+} st", z.transpose).replace("+0", "0"),
                (f32::from(z.transpose) + 48.0) / 96.0,
                true,
            ),
            (z.vel_low.to_string(), f32::from(z.vel_low) / 127.0, false),
        ];
        let scope = format!("ZONE {}", self.patch.zone + 1);
        Some(std::array::from_fn(|k| {
            let (value, pos, bipolar) = &cards[k];
            let mut c = QLinkCard::new(page::row_label(QLINK_ROWS[k]), &scope, value, *pos);
            c.bipolar = *bipolar;
            c
        }))
    }

    /// Q-Link `k` turned on the editor: its row steps. False when the
    /// editor doesn't show a zone.
    pub(super) fn patch_qlink(&mut self, k: usize, ticks: i32) -> bool {
        if self.patch_qlinks().is_none() || k >= 4 {
            return false;
        }
        self.patch_edit_event(EventKind::Step {
            index: QLINK_ROWS[k],
            delta: ticks,
        });
        self.refresh_qlinks();
        true
    }

    /// The patches' undo history rows.
    pub(super) fn patch_history(&self) -> (Vec<String>, usize) {
        self.patch.history_rows()
    }

    /// UNDO on a patch screen.
    pub(super) fn patch_undo(&mut self, redo: bool) {
        match self.patch.undo(&mut self.patches, redo) {
            Some(e) => {
                let title = format!("{}: {}", if redo { "Redo" } else { "Undo" }, e.title);
                self.patch_edited(Some(e), false);
                self.tell(&title);
            }
            None => self.tell(if redo {
                "Nothing to redo on the patches"
            } else {
                "Nothing to undo on the patches"
            }),
        }
        self.patch_show();
    }

    /// A project opened: its patches, nothing open, no history; the keys
    /// play its patch 1 (the audio loop picks it with the open).
    pub(super) fn patches_opened(&mut self) {
        self.patch = crate::patch_screen::PatchScreen::default();
        self.patch.keys.playing = (!self.patches.list.is_empty()).then_some(0);
        self.refresh_patches();
        self.refresh_show();
    }
}

/// The sample screens' song (live shows C1): eight tracks named for what
/// they play and five patches, as the approved mockups draw them; track
/// 9 isn't in the song, so patch 5's top zone is missing.
pub(super) fn sample(p: &mut Player) {
    use mpfree_patch::{Layout, Patch, Patches, Zone};
    let mut st = crate::time::TimeHost::new(44_100, 120.0)
        .map(|h| h.state())
        .unwrap_or_default();
    for (t, name) in st.tracks.iter_mut().zip([
        "Drums", "Sub Bass", "Grand", "Rhodes", "Strings", "Saw Lead", "Organ", "Vox",
    ]) {
        t.name = name.to_string();
    }
    st.armed = Some(0);
    p.apply(ToUi::Time(Box::new(st)));
    let rhodes = Layout::Whole.patch("Rhodes", "track 4");
    let mut bass_rhodes = Layout::BassAndKeys.patch("Bass + Rhodes", "track 2");
    bass_rhodes.zones[1].plays = "track 4".into();
    let mut chorus = Layout::BassAndKeys.patch("Chorus split", "track 2");
    chorus.zones[1].plays = "track 3".into();
    let mut strings = Zone::new("track 5", 48, 127);
    strings.vel_low = 64;
    chorus.zones.push(strings);
    let mut lead = Layout::Whole.patch("Lead", "track 6");
    lead.zones[0].low = 60;
    lead.zones[0].transpose = 12;
    let mut three: Patch = Layout::ThreeWay.patch("Three-way", "track 2");
    three.zones[1].plays = "track 7".into();
    three.zones[2].plays = "track 9".into();
    p.patches = Patches {
        list: vec![rhodes, bass_rhodes, chorus, lead, three],
    };
    p.patches_opened();
    p.patch.keys.playing = Some(2);
    p.refresh_play();
    p.refresh_patches();
}

/// The editor's sample: Chorus split, its strings picked, C2 held softly
/// and E4 and G4 at 96 on a KeyLab 61.
pub(super) fn sample_edit(p: &mut Player) {
    p.toggle_patch_list();
    p.patch_list_event(EventKind::Toggle(3));
    p.patch_edit_event(EventKind::Tab(2));
    let key = |note, vel| crate::patch_screen::HeldKey {
        port: "KeyLab 61".to_string(),
        ch: 0,
        note,
        vel,
    };
    p.patch_keys(KeysNow {
        playing: Some(2),
        waiting: None,
        held: vec![key(36, 52), key(64, 96), key(67, 96)],
    });
    p.take_actions();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Taps the control `id` (one of several: `index`) of `widget` on the
    /// page showing, and gives what the tap sent the audio loop.
    fn tap(p: &mut Player, widget: &str, id: &str, index: usize) -> Vec<FromUi> {
        paint(p);
        let t =
            p.ui.targets()
                .into_iter()
                .find(|pt| pt.widget == widget && pt.target.id == id && pt.target.index == index)
                .unwrap_or_else(|| panic!("{widget}/{id} {index} isn't on {}", p.page()));
        let (x, y) = t.target.centre();
        p.take_actions();
        p.touch(5, Touch::Down, x, y);
        p.touch(5, Touch::Up, x, y);
        p.take_actions()
    }

    /// The page painted, as a frame shows it: controls know their places
    /// once painted.
    fn paint(p: &mut Player) {
        let mut c = SoftCanvas::new(SCREEN.0, SCREEN.1);
        p.ui.paint(&mut c);
    }

    fn hint(p: &Player) -> String {
        p.ui.get::<Label>("hint").unwrap().text.clone()
    }

    fn sent_patches(a: &[FromUi]) -> usize {
        a.iter().filter(|f| matches!(f, FromUi::Patches(_))).count()
    }

    #[test]
    fn keys_play_opens_the_list_and_a_tile_picks_what_the_keys_play() {
        let mut p = sample_player(Fonts::embedded());
        sample(&mut p);
        p.take_actions();
        paint(&mut p);
        let keys =
            p.ui.targets()
                .into_iter()
                .find(|pt| pt.widget == play_page::TRANSPORT && pt.target.id == "keys")
                .expect("KEYS PLAY on Play's transport");
        assert!(
            keys.target.label.contains("3 · Chorus split"),
            "{}",
            keys.target.label
        );
        assert!(tap(&mut p, play_page::TRANSPORT, "keys", 0).is_empty());
        assert_eq!(p.page(), page::LIST_PAGE);
        assert!(hint(&p).starts_with("Tap a patch"), "{}", hint(&p));
        // Tile 4 is patch 4, Lead (tile 0 is the keys as routed).
        let a = tap(&mut p, page::LIST, "patch", 4);
        assert_eq!(a, vec![FromUi::PickPatch(Some(3))]);
        assert_eq!(p.page(), "Play");
        assert_eq!(p.patch.keys.playing, Some(3));
        // The list again: – / + steps to the patch before; Close leaves it.
        tap(&mut p, play_page::TRANSPORT, "keys", 0);
        p.nav(Nav::Step(-1));
        assert_eq!(p.take_actions(), vec![FromUi::PickPatch(Some(2))]);
        assert_eq!(p.page(), page::LIST_PAGE, "– / + keep the list open");
        assert!(tap(&mut p, page::LIST, "close", 0).is_empty());
        assert_eq!(p.page(), "Play");
        // KEYS PLAY again closes what it opened.
        tap(&mut p, play_page::TRANSPORT, "keys", 0);
        tap(&mut p, play_page::TRANSPORT, "keys", 0);
        assert_eq!(p.page(), "Play");
        // Tile 0: the keys play as routed.
        tap(&mut p, play_page::TRANSPORT, "keys", 0);
        assert_eq!(
            tap(&mut p, page::LIST, "patch", 0),
            vec![FromUi::PickPatch(None)]
        );
        assert_eq!(p.patch_keys_look().unwrap().line, "Keys as routed");
    }

    #[test]
    fn plays_menu_opens_the_patch_list_and_new_patch_makes_the_first() {
        let mut p = sample_player(Fonts::embedded());
        p.take_actions();
        assert!(p.patches.list.is_empty());
        assert!(p.patch_keys_look().is_none(), "no patches: no KEYS PLAY");
        // MENU on Play: Patches, its last row.
        let mut app = crate::app::App::new(44_100, false);
        app.map.menu = Some(1);
        let mut fx = app.button(1, true, 0.0);
        app.button(1, false, 0.0);
        let row = app
            .row_of(crate::app::Item::Patches)
            .expect("Patches on Play's MENU");
        fx.extend(app.step_item(row, 1));
        assert!(fx.contains(&crate::app::Effect::PatchList), "{fx:?}");
        apply_effects(&mut p, fx);
        assert_eq!(p.page(), page::LIST_PAGE);
        let l = p.ui.get::<PatchList>(page::LIST).unwrap();
        assert_eq!(l.count, "No patches yet: New patch makes one");
        // New patch, then Bass + keys: patch 1, in its editor, playing.
        assert!(tap(&mut p, page::LIST, "new", 0).is_empty());
        let a = tap(&mut p, page::LIST, "layout", 1);
        assert_eq!(sent_patches(&a), 1, "{a:?}");
        assert!(a.contains(&FromUi::PickPatch(Some(0))), "{a:?}");
        assert_eq!(p.patches.list.len(), 1);
        assert_eq!(p.patches.list[0].name, "Patch 1");
        assert_eq!(p.patches.list[0].zones.len(), 2);
        assert_eq!(p.page(), page::EDIT_PAGE);
        // Now KEYS PLAY shows, and names it.
        assert_eq!(p.patch_keys_look().unwrap().line, "1 · Patch 1");
    }

    #[test]
    fn the_editor_edits_one_take_at_a_time_and_undo_brings_it_back() {
        let mut p = sample_player(Fonts::embedded());
        sample(&mut p);
        tap(&mut p, play_page::TRANSPORT, "keys", 0);
        // Edit on Rhodes: its editor, and the keys play it.
        let a = tap(&mut p, page::LIST, "edit", 1);
        assert_eq!(a, vec![FromUi::PickPatch(Some(0))]);
        assert_eq!(p.page(), page::EDIT_PAGE);
        assert_eq!(hint(&p), "KEYS PLAY: 1 · Rhodes", "the hint line says so");
        // + Add zone: a second zone, sent to the audio loop.
        let a = tap(&mut p, page::EDIT, "add-zone", 0);
        assert_eq!(sent_patches(&a), 1, "{a:?}");
        assert_eq!(p.patches.list[0].zones.len(), 2);
        assert_eq!(p.patch.zone, 1, "the new zone is picked");
        // Learn low comes armed: the next key starts the zone. A tap on
        // Learn low lets it go, and another arms it again.
        assert_eq!(p.patch.learn, Some(false));
        tap(&mut p, page::EDIT, "learn-low", 0);
        assert_eq!(p.patch.learn, None);
        tap(&mut p, page::EDIT, "learn-low", 0);
        assert_eq!(p.patch.learn, Some(false));
        let held = |note| KeysNow {
            playing: Some(0),
            waiting: None,
            held: vec![crate::patch_screen::HeldKey {
                port: "KeyLab 61".to_string(),
                ch: 0,
                note,
                vel: 90,
            }],
        };
        p.apply(ToUi::Keys(Box::new(held(60))));
        assert_eq!(p.patches.list[0].zones[1].low, 60);
        assert_eq!(p.patch.learn, None, "Learn takes one key");
        assert_eq!(sent_patches(&p.take_actions()), 1);
        // The Q-Links turn the picked zone's Transpose.
        assert!(p.patch_qlink(2, 3));
        assert_eq!(p.patches.list[0].zones[1].transpose, 3);
        // UNDO, three times: the transpose, the key, the zone.
        p.take_actions();
        p.undo(false);
        assert_eq!(p.patches.list[0].zones[1].transpose, 0);
        p.undo(false);
        assert_eq!(p.patches.list[0].zones[1].low, 0);
        p.undo(false);
        assert_eq!(p.patches.list[0].zones.len(), 1);
        assert_eq!(sent_patches(&p.take_actions()), 3);
        // Delete asks first; the second tap deletes, and UNDO brings it back.
        tap(&mut p, page::EDIT, "delete", 0);
        assert_eq!(p.patches.list.len(), 5, "the first tap only asks");
        tap(&mut p, page::EDIT, "delete", 0);
        assert_eq!(p.patches.list.len(), 4);
        assert_eq!(p.page(), page::LIST_PAGE);
        p.undo(false);
        assert_eq!(p.patches.list.len(), 5);
        assert_eq!(p.patches.list[0].name, "Rhodes");
    }
}
