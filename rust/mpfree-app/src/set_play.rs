//! Playing a set on the screen (live shows B1): the set being played, the
//! set player's rules ([`crate::show::Show`]), where a song change stands,
//! and the Show view drawn from all of it. Pure, so every step is tested
//! here; the screen (gui.rs) feeds it presses, the browser's answers and
//! the audio loop's news, and does what it answers ([`Do`]).
//!
//! A cold change (the only kind in B1), from Go:
//! 1. **Ending**: the song playing ends by its own Ends (Let it finish,
//!    Fade or Cut; song_end.rs runs it in the audio loop), until silent.
//! 2. **Saving takes**: if the song changed on stage, its changes and
//!    takes go into a recovery copy beside it (the browser's
//!    `on_stage_copy_with`), and the set remembers the copy. Nothing asks
//!    and nothing is lost; the song itself stays as saved. Without
//!    changes this step is skipped.
//! 3. **Opening**: the next song opens all or nothing, without asking
//!    (`open_set_song`), and starts by its Starts: stopped (LOADED, PLAY
//!    starts it), playing, or after its count-in.
//!
//! If the copy or the open fails, nothing plays: the card turns red with
//! what failed, **Try again** starts that step again, and **Reopen** gives
//! back the song that was playing, as it was left (in B1 it was never
//! closed: an open only replaces it once the new song is read whole).

use crate::show::{Answer, Change, Press, Show};
use crate::song_end::End;
use mpfree_library::{Ends, Loc, Set, SetSong, Starts};
use mpfree_time_ui::list::ListRow;
use mpfree_time_ui::show::{
    Ending, KeyboardLine, NextCard, PartMark, PartTile, Pill, SetRow, ShowView, SongMark, Starting,
};
use mpfree_ui::Tone;

/// Control ids for the set player's bounce cleaning: one each for taps,
/// the wheel and - / +; a learned control is [`FOOT`] plus its id.
pub const TAP: u32 = 1;
pub const WHEEL: u32 = 2;
/// The wheel's push, apart from its turns.
pub const PUSH: u32 = 3;
pub const FOOT: u32 = 100;

/// The hint line under the Show view.
pub const SHOW_HINT: &str = "Tap a song: arm it   ·   wheel or - / +: arm the song after or before   ·   push: Go   ·   Back to the set order   ·   MENU: Set editor, check, close";

/// Where a song change stands, past the set player's own rules.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Play set: the first song is opening.
    Starting,
    /// No change under way.
    Idle,
    /// The song playing ends by its Ends (FINISHING, FADING).
    Ending,
    /// The changed song's recovery copy is being written.
    Saving,
    /// The next song is opening.
    Opening,
}

/// What the screen does for the set player.
#[derive(Clone, Debug, PartialEq)]
pub enum Do {
    /// The hint line says this.
    Tell(String),
    /// The song playing ends so (the audio loop's `SongEnd`).
    End(End),
    /// Ask for the song's parts, for its recovery copy.
    SaveCopy,
    /// Open this song of the set (the browser's `open_set_song`).
    Open(SetSong),
    /// The song is in: start it so.
    Start(Starts),
    /// Full sound again, with no ending (Reopen).
    Reset,
    /// The first song didn't open: the set doesn't start.
    NotStarted,
}

/// A keyboard the rig knows (live shows B1), by the name the musician
/// gave it, and the song's patch on it (C1).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct KeyboardNow {
    pub name: String,
    pub connected: bool,
    /// The patch its keys reach, and that patch's zones on it; None while
    /// the song has no patches.
    pub patch: Option<(String, Vec<mpfree_time_ui::show::KeyBar>)>,
}

/// The song playing, as the screen knows it, for the Show view.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Now {
    pub bpm: f64,
    /// The transport rolls.
    pub rolling: bool,
    /// The transport counts in.
    pub counting: bool,
    /// The song list's steps (the song's parts).
    pub list: Vec<ListRow>,
    /// Beats in a bar.
    pub beats: u32,
    pub click: bool,
    /// Bars of count-in.
    pub count_in: u8,
    /// The rig's keyboards.
    pub keyboards: Vec<KeyboardNow>,
    pub autosave_waiting: bool,
}

/// A set being played.
#[derive(Clone, Debug)]
pub struct SetPlay {
    pub loc: Loc,
    pub set: Set,
    show: Show,
    step: Step,
    /// Each song already played in this show.
    played: Vec<bool>,
    /// The song changing to still needs its recovery copy written first
    /// (for Try again after the copy failed).
    copy_first: bool,
    /// What failed, after a copy or an open failed.
    failed: Option<String>,
    /// Let it finish's seconds left, as the audio loop last said.
    finishing: Option<u32>,
}

/// The song's own ending, as the audio loop runs it.
fn end_of(song: &SetSong) -> End {
    match song.ends {
        Ends::Finish => End::Finish,
        Ends::Fade(s) => End::Fade(s),
        Ends::Cut => End::Cut,
    }
}

fn can_go(set: &Set) -> Vec<bool> {
    set.songs
        .iter()
        .map(|s| s.check.as_ref().is_none_or(|(_, c)| c.not_ready.is_empty()))
        .collect()
}

impl SetPlay {
    /// Play set: the set at `loc`, its first song opening. What to do
    /// first.
    pub fn new(loc: Loc, set: Set) -> (SetPlay, Vec<Do>) {
        let first = set.songs.first().cloned();
        let p = SetPlay {
            loc,
            show: Show::new(can_go(&set)),
            played: vec![false; set.songs.len()],
            set,
            step: Step::Starting,
            copy_first: false,
            failed: None,
            finishing: None,
        };
        let fx = match first {
            Some(s) => vec![Do::Open(s)],
            None => vec![Do::NotStarted],
        };
        (p, fx)
    }

    pub fn step(&self) -> Step {
        self.step
    }

    /// The samples' set (sample screens): in as if the songs before `k`
    /// were played and song `k` plays now.
    pub fn sample_at(&mut self, k: usize) {
        self.step = Step::Idle;
        self.show.start_at(k);
        for p in self.played.iter_mut().take(k) {
            *p = true;
        }
    }

    pub fn now(&self) -> usize {
        self.show.now()
    }

    #[cfg(test)]
    pub fn armed(&self) -> Option<usize> {
        self.show.armed()
    }

    /// The first song is in, and the set plays.
    pub fn started(&self) -> bool {
        self.step != Step::Starting
    }

    /// The song changing to, while a change is under way.
    fn going(&self) -> Option<usize> {
        self.show.armed()
    }

    /// A press of one of the set's controls (`from`: [`TAP`], [`WHEEL`],
    /// [`FOOT`] + id) at `ms`, with the transport rolling or not and
    /// Quick advance on or off.
    pub fn press(
        &mut self,
        press: Press,
        from: u32,
        ms: u64,
        rolling: bool,
        quick: bool,
    ) -> Vec<Do> {
        if self.step == Step::Starting {
            return vec![Do::Tell(
                "The set's first song is opening: wait for it".into(),
            )];
        }
        match self.show.press(press, from, ms, rolling, quick) {
            Answer::Nothing(Some(why)) => vec![Do::Tell(why.to_string())],
            Answer::Nothing(None) | Answer::Armed(_) | Answer::Disarmed => Vec::new(),
            Answer::Go(_) => {
                self.step = Step::Ending;
                self.finishing = None;
                let end = self.set.songs.get(self.show.now()).map_or(End::Cut, end_of);
                vec![Do::End(end)]
            }
            Answer::GoNow => vec![Do::End(End::Now)],
        }
    }

    /// Let it finish waits: at most `secs` left.
    pub fn finishing(&mut self, secs: u32) {
        if self.step == Step::Ending {
            self.finishing = Some(secs);
        }
    }

    /// The song playing is silent. `changed`: it has changes not saved,
    /// which go into a recovery copy before the next song opens.
    pub fn ended(&mut self, changed: bool) -> Vec<Do> {
        if self.step != Step::Ending {
            return Vec::new();
        }
        self.finishing = None;
        if changed {
            self.step = Step::Saving;
            self.copy_first = true;
            self.show.changed_to(Change::SavingTakes);
            return vec![Do::SaveCopy];
        }
        self.open_next()
    }

    fn open_next(&mut self) -> Vec<Do> {
        let Some(song) = self.going().and_then(|k| self.set.songs.get(k)).cloned() else {
            return Vec::new();
        };
        self.copy_first = false;
        self.step = Step::Opening;
        self.show.changed_to(Change::Opening);
        vec![Do::Open(song)]
    }

    /// The recovery copy is written (the screen tells the set which song
    /// it is for): the next song opens.
    pub fn copied(&mut self) -> Vec<Do> {
        if self.step != Step::Saving {
            return Vec::new();
        }
        self.open_next()
    }

    /// The recovery copy couldn't be written: nothing opens, so nothing is
    /// lost; Try again writes it again, Reopen goes back to the song.
    pub fn copy_failed(&mut self, why: &str) -> Vec<Do> {
        if self.step != Step::Saving {
            return Vec::new();
        }
        self.fail(format!("Its changes couldn\u{2019}t be kept: {why}"));
        vec![Do::Tell(format!(
            "The changes weren\u{2019}t kept ({why}): Try again, or Reopen"
        ))]
    }

    fn fail(&mut self, what: String) {
        self.failed = Some(what);
        self.step = Step::Idle;
        self.show.failed();
    }

    /// The song opening is in (read, swapped in, its loops and takes
    /// read): it plays, as its Starts says.
    pub fn opened(&mut self) -> Vec<Do> {
        match self.step {
            Step::Starting => {
                self.step = Step::Idle;
                let starts = self.set.songs.first().map_or(Starts::Stopped, |s| s.starts);
                vec![Do::Start(starts)]
            }
            Step::Opening => {
                let before = self.show.now();
                self.show.opened();
                if let Some(p) = self.played.get_mut(before) {
                    *p = true;
                }
                self.step = Step::Idle;
                self.failed = None;
                let starts = self
                    .set
                    .songs
                    .get(self.show.now())
                    .map_or(Starts::Stopped, |s| s.starts);
                vec![Do::Start(starts)]
            }
            _ => Vec::new(),
        }
    }

    /// The song opening didn't open: why. At Play set the set doesn't
    /// start; in a change nothing plays, and the card offers Try again and
    /// Reopen.
    pub fn open_failed(&mut self, why: &str) -> Vec<Do> {
        match self.step {
            Step::Starting => {
                let name = self.set.songs.first().map_or("", |s| s.name.as_str());
                vec![
                    Do::Tell(format!(
                        "The set didn\u{2019}t start: {name} didn\u{2019}t open ({why})"
                    )),
                    Do::NotStarted,
                ]
            }
            Step::Opening => {
                self.fail(why.to_string());
                vec![Do::Tell(format!(
                    "The song didn\u{2019}t open ({why}): Try again, or Reopen"
                ))]
            }
            _ => Vec::new(),
        }
    }

    /// Try again (the red card): the step that failed starts again.
    pub fn retry(&mut self) -> Vec<Do> {
        if self.show.retry().is_none() {
            return Vec::new();
        }
        if self.copy_first {
            self.step = Step::Saving;
            self.show.changed_to(Change::SavingTakes);
            return vec![Do::SaveCopy];
        }
        self.open_next()
    }

    /// Reopen (the red card): the song that was playing, as it was left.
    pub fn reopen(&mut self) -> Vec<Do> {
        if self.show.reopen().is_none() {
            return Vec::new();
        }
        self.show.opened();
        self.failed = None;
        self.copy_first = false;
        self.step = Step::Idle;
        vec![Do::Reset]
    }

    /// The set changed while it plays (a check landed, a copy was marked,
    /// the Set editor): the new set. NOW stays the same song where it can.
    pub fn set_changed(&mut self, set: Set) {
        if set.songs.len() == self.set.songs.len() {
            self.show.set_can_go(can_go(&set));
            self.set = set;
            return;
        }
        // Songs added or taken out: the song playing is found again, and
        // nothing stays armed.
        let now = self.set.songs.get(self.show.now());
        let at = now
            .and_then(|n| {
                set.songs
                    .iter()
                    .position(|s| s.id == n.id && s.name == n.name)
            })
            .unwrap_or(0);
        let mut show = Show::new(can_go(&set));
        show.start_at(at);
        self.show = show;
        self.played = vec![false; set.songs.len()];
        self.set = set;
        if self.step != Step::Starting {
            self.step = Step::Idle;
        }
    }

    /// The set's name, as the Sets place shows it.
    pub fn name(&self) -> &str {
        self.loc.name().trim_end_matches(".set")
    }

    fn song_name(&self, k: usize) -> &str {
        self.set.songs.get(k).map_or("", |s| s.name.as_str())
    }

    fn of(&self, k: usize) -> String {
        format!("{} of {}", k + 1, self.set.songs.len())
    }

    /// How many songs changed on stage, in this show or before.
    fn changed_count(&self) -> usize {
        self.set
            .songs
            .iter()
            .filter(|s| !s.on_stage.is_empty())
            .count()
    }

    /// The Show view, as it should look now.
    pub fn view(&self, now: &Now) -> ShowView {
        let n = self.set.songs.len();
        let k_now = self.show.now();
        let armed = self.show.armed();
        let change = self.show.change();
        let failed = change == Change::Failed;
        let next_word = match (self.step, change) {
            (_, Change::Failed) => "didn\u{2019}t open",
            (Step::Ending, _) if self.finishing.is_some() => "finishing",
            (Step::Ending, _) => "fading",
            (Step::Saving, _) => "saving takes",
            (Step::Opening, _) => "opening",
            _ => "cold load",
        };
        let songs = self
            .set
            .songs
            .iter()
            .enumerate()
            .map(|(k, s)| SetRow {
                name: s.name.clone(),
                mark: if k == k_now && self.started() {
                    SongMark::Now
                } else if Some(k) == armed {
                    let blocked = s
                        .check
                        .as_ref()
                        .is_some_and(|(_, c)| !c.not_ready.is_empty());
                    SongMark::Next(if blocked && self.step == Step::Idle && !failed {
                        "blocked".into()
                    } else {
                        next_word.into()
                    })
                } else if self.played.get(k) == Some(&true) {
                    SongMark::Played
                } else {
                    match &s.check {
                        Some((_, c)) if !c.not_ready.is_empty() => SongMark::NotReady,
                        Some((_, c)) if !c.limits.is_empty() => SongMark::Limits,
                        _ => SongMark::Waiting,
                    }
                },
            })
            .collect();
        let back_usable = self.started()
            && self.step == Step::Idle
            && !failed
            && self.show.next_in_order().is_some_and(|k| Some(k) != armed);
        let pill = match (self.step, failed) {
            (_, true) => Pill::nothing_playing(),
            (Step::Starting | Step::Opening, _) => Pill::opening(),
            (Step::Saving, _) => Pill::saving_takes(),
            (Step::Ending, _) => {
                match (self.finishing, self.set.songs.get(k_now).map(|s| s.ends)) {
                    (Some(secs), _) => Pill::finishing(secs),
                    (None, Some(Ends::Fade(s))) => Pill::fading(u32::from(s)),
                    (None, Some(Ends::Finish)) => {
                        Pill::finishing((crate::song_end::LONGEST_MS / 1000) as u32)
                    }
                    _ => Pill::opening(),
                }
            }
            (Step::Idle, _) if now.counting => Pill::loaded("counting in"),
            (Step::Idle, _) if now.rolling => Pill::playing(),
            (Step::Idle, _) => Pill::loaded("press PLAY"),
        };
        let now_name = self.song_name(k_now).to_string();
        let card = self.card(&now_name, failed);
        let notes = self
            .set
            .songs
            .get(k_now)
            .map(|s| s.notes.clone())
            .unwrap_or_default();
        let needs: Vec<&str> = self
            .set
            .songs
            .get(k_now)
            .map(|s| {
                s.needs
                    .iter()
                    .filter(|n| n.needed)
                    .map(|n| n.name.as_str())
                    .collect()
            })
            .unwrap_or_default();
        let keyboards = now
            .keyboards
            .iter()
            .map(|k| KeyboardLine {
                name: k.name.clone(),
                plays: match (&k.patch, k.connected) {
                    (_, false) => "not connected".into(),
                    (Some((patch, _)), true) => patch.clone(),
                    (None, true) => "connected".into(),
                },
                tone: match (k.connected, needs.contains(&k.name.as_str())) {
                    (true, _) => Tone::Normal,
                    (false, true) => Tone::Bad,
                    (false, false) => Tone::Dim,
                },
                bars: match &k.patch {
                    Some((_, bars)) if k.connected => bars.clone(),
                    _ => Vec::new(),
                },
            })
            .collect();
        let mut v = ShowView::new();
        v.set_name = self.name().to_string();
        v.set_line = match n {
            1 => "1 song".into(),
            n => format!("{n} songs"),
        };
        v.songs = songs;
        v.back_usable = back_usable;
        v.title = format!("Now \u{b7} song {} of {n}", k_now + 1);
        v.song_name = now_name;
        v.song_faint = failed;
        v.pill = pill;
        v.song_line = song_line(now);
        v.autosave_waiting = now.autosave_waiting;
        v.parts = parts(&now.list);
        v.card = card;
        v.notes = notes;
        v.keyboards = keyboards;
        v.click = now.click;
        v.click_line = match now.count_in {
            0 => "No count-in.".into(),
            1 => "Count-in 1 bar.".into(),
            b => format!("Count-in {b} bars."),
        };
        v
    }

    fn card(&self, now_name: &str, failed: bool) -> NextCard {
        let armed = self.show.armed();
        if failed {
            let name = armed.map_or(now_name, |k| self.song_name(k));
            let what = self.failed.as_deref().unwrap_or("it didn\u{2019}t open");
            return NextCard::failed(name, what, now_name);
        }
        if !self.started() {
            return NextCard::opening(&self.of(0), self.song_name(0), 0.0, "reading the song");
        }
        let Some(k) = armed else {
            return match self.show.next_in_order() {
                Some(k) => {
                    let limits = self.set.songs[k]
                        .check
                        .as_ref()
                        .and_then(|(_, c)| c.limits.first())
                        .map(String::as_str);
                    NextCard::not_armed(&self.of(k), self.song_name(k), limits)
                }
                None => NextCard::last_song(self.changed_count()),
            };
        };
        let (of, name) = (self.of(k), self.song_name(k));
        let song = &self.set.songs[k];
        let now_song = self.set.songs.get(self.show.now());
        match self.step {
            Step::Ending => match (self.finishing, now_song.map(|s| s.ends)) {
                (Some(secs), _) => NextCard::finishing(&of, name, now_name, secs),
                (None, Some(Ends::Fade(s))) => NextCard::fading(&of, name, now_name, u32::from(s)),
                _ => NextCard::opening(&of, name, 0.0, "the song before stops"),
            },
            Step::Saving => NextCard::saving_takes(
                &of,
                name,
                0.0,
                &format!("{now_name}\u{2019}s changes and takes, kept beside it"),
            ),
            Step::Opening | Step::Starting => NextCard::opening(&of, name, 0.0, "reading the song"),
            Step::Idle => {
                if let Some(why) = song.check.as_ref().and_then(|(_, c)| c.not_ready.first()) {
                    return NextCard::blocked(&of, name, why);
                }
                let ending = match now_song.map(|s| s.ends) {
                    Some(Ends::Fade(s)) => Ending::Fade(s),
                    Some(Ends::Cut) => Ending::Cut,
                    _ => Ending::Finish,
                };
                let starting = match song.starts {
                    Starts::Stopped => Starting::Stopped,
                    Starts::Playing => Starting::Playing,
                    Starts::CountIn => Starting::CountIn,
                };
                NextCard::cold(&of, name, Some(now_name), None, ending, starting)
            }
        }
    }
}

/// "92 BPM · part 4 of 7 · 1:52 left".
fn song_line(now: &Now) -> String {
    let mut line = format!("{:.0} BPM", now.bpm);
    let total = now.list.len();
    if let Some(k) = now.list.iter().position(|r| r.playing.is_some()) {
        line.push_str(&format!("  \u{b7}  part {} of {total}", k + 1));
        // What is left of the list, in bars, at the tempo now.
        let beats = f64::from(now.beats.max(1));
        let row = &now.list[k];
        let (pass, done) = row.playing.unwrap_or((1, 0.0));
        let this =
            row.bars as f64 * (f64::from(row.times) - f64::from(pass) + 1.0 - f64::from(done));
        let after: f64 = now.list[k + 1..]
            .iter()
            .map(|r| r.bars as f64 * f64::from(r.times))
            .sum();
        if now.bpm > 0.0 {
            let secs = ((this + after) * beats * 60.0 / now.bpm).max(0.0).round() as u64;
            line.push_str(&format!("  \u{b7}  {}:{:02} left", secs / 60, secs % 60));
        }
    } else if total > 0 {
        line.push_str(&format!("  \u{b7}  {total} parts"));
    }
    line
}

/// The song list's steps as the Show view's tiles.
fn parts(list: &[ListRow]) -> Vec<PartTile> {
    let playing = list.iter().position(|r| r.playing.is_some());
    list.iter()
        .enumerate()
        .map(|(k, r)| PartTile {
            name: r.name.clone(),
            meta: format!("\u{d7} {} \u{b7} {} bars", r.times, r.bars),
            scene: r.scene,
            mark: match (playing, r.playing) {
                (_, Some((pass, done))) => PartMark::Now {
                    pass: format!("pass {pass} of {}", r.times),
                    done,
                },
                (Some(p), _) if k < p => PartMark::Played,
                (Some(p), _) if k == p + 1 => PartMark::Next,
                _ => PartMark::Waiting,
            },
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_library::{Place, SongCheck};

    fn set() -> SetPlay {
        let mut songs: Vec<SetSong> = ["Lanterns", "Copper sky", "Undertow", "Night drive"]
            .iter()
            .map(|n| SetSong::new("", n))
            .collect();
        songs[1].starts = Starts::Playing;
        songs[1].ends = Ends::Fade(6);
        songs[2].check = Some((
            1,
            SongCheck {
                not_ready: vec!["the file samples/strings.wav is missing".into()],
                ..SongCheck::default()
            },
        ));
        songs[0].notes = "F minor".into();
        let loc = Loc::top(Place::Sets).join("Friday.set").unwrap();
        let (p, fx) = SetPlay::new(
            loc,
            Set {
                songs,
                extra: vec![],
            },
        );
        assert!(matches!(&fx[..], [Do::Open(s)] if s.name == "Lanterns"));
        p
    }

    fn started() -> SetPlay {
        let mut p = set();
        assert_eq!(p.opened(), [Do::Start(Starts::Stopped)]);
        p
    }

    #[test]
    fn play_set_opens_the_first_song_and_waits_for_it() {
        let mut p = set();
        assert_eq!(p.step(), Step::Starting);
        let v = p.view(&Now::default());
        assert_eq!(v.pill, Pill::opening());
        assert_eq!(v.card.word.as_ref().unwrap().0, "OPENING");
        // Presses wait.
        assert!(matches!(
            &p.press(Press::NextSong, TAP, 0, false, false)[..],
            [Do::Tell(_)]
        ));
        assert_eq!(p.opened(), [Do::Start(Starts::Stopped)]);
        let v = p.view(&Now::default());
        assert_eq!(v.title, "Now \u{b7} song 1 of 4");
        assert_eq!(v.songs[0].mark, SongMark::Now);
        assert_eq!(v.pill, Pill::loaded("press PLAY"));
        assert_eq!(v.notes, "F minor");
        assert_eq!(v.card.label, "Next in the set");
        assert!(v.back_usable);
        // The first song didn't open: the set doesn't start.
        let mut p = set();
        let fx = p.open_failed("it doesn\u{2019}t read");
        assert_eq!(fx.last(), Some(&Do::NotStarted));
    }

    /// Next song arms, Go ends the song playing by its own Ends, a song
    /// with no changes opens next, and it starts by its Starts.
    #[test]
    fn a_cold_change_ends_opens_and_starts() {
        let mut p = started();
        assert!(p.press(Press::NextSong, FOOT, 0, true, false).is_empty());
        let v = p.view(&Now {
            rolling: true,
            ..Now::default()
        });
        assert_eq!(v.songs[1].mark, SongMark::Next("cold load".into()));
        assert_eq!(v.card.word.as_ref().unwrap().0, "COLD LOAD");
        assert!(!v.back_usable, "the next in order is the one armed");
        // Lanterns ends by its own Ends: Let it finish.
        assert_eq!(
            p.press(Press::NextSong, FOOT, 500, true, false),
            [Do::End(End::Finish)]
        );
        p.finishing(7);
        let v = p.view(&Now::default());
        assert_eq!(v.pill, Pill::finishing(7));
        assert_eq!(v.card.go.0, "Go now");
        // Go again: now.
        assert_eq!(
            p.press(Press::Go, FOOT, 900, false, false),
            [Do::End(End::Now)]
        );
        // Silent, no changes: Copper sky opens.
        let fx = p.ended(false);
        assert!(matches!(&fx[..], [Do::Open(s)] if s.name == "Copper sky"));
        assert_eq!(p.view(&Now::default()).pill, Pill::opening());
        assert!(matches!(
            &p.press(Press::Go, FOOT, 2000, false, false)[..],
            [Do::Tell(_)]
        ));
        assert_eq!(p.opened(), [Do::Start(Starts::Playing)]);
        let v = p.view(&Now::default());
        assert_eq!(v.songs[0].mark, SongMark::Played);
        assert_eq!(v.songs[1].mark, SongMark::Now);
        assert_eq!(p.now(), 1);
        // Copper sky fades out over its 6 s.
        p.press(Press::Arm(3), TAP, 3000, true, false);
        assert_eq!(
            p.press(Press::Go, TAP, 3100, true, false),
            [Do::End(End::Fade(6))]
        );
        let v = p.view(&Now::default());
        assert_eq!(v.card.word.as_ref().unwrap().0, "FADING");
    }

    /// A song that changed on stage: its recovery copy first, then the
    /// open. A failed copy opens nothing; Try again writes it again.
    #[test]
    fn a_changed_song_is_kept_before_the_next_opens() {
        let mut p = started();
        p.press(Press::NextSong, TAP, 0, false, false);
        p.press(Press::Go, TAP, 100, false, false);
        assert_eq!(p.ended(true), [Do::SaveCopy]);
        let v = p.view(&Now::default());
        assert_eq!(v.pill, Pill::saving_takes());
        assert_eq!(v.card.word.as_ref().unwrap().0, "SAVING TAKES");
        assert!(matches!(
            &p.copy_failed("the stick is full")[..],
            [Do::Tell(_)]
        ));
        let v = p.view(&Now::default());
        assert!(v.card.failed);
        assert_eq!(v.pill, Pill::nothing_playing());
        assert!(v.card.meaning.contains("the stick is full"));
        assert_eq!(p.retry(), [Do::SaveCopy]);
        let fx = p.copied();
        assert!(matches!(&fx[..], [Do::Open(s)] if s.name == "Copper sky"));
    }

    /// A failed open: nothing plays, Try again opens again, and Reopen
    /// gives back the song before with full sound.
    #[test]
    fn a_failed_open_offers_try_again_and_reopen() {
        let mut p = started();
        p.press(Press::NextSong, TAP, 0, false, false);
        p.press(Press::Go, TAP, 100, false, false);
        p.ended(false);
        p.open_failed("the file samples/pad.wav is missing");
        let v = p.view(&Now::default());
        assert!(v.card.failed);
        assert_eq!(v.card.cancel.0, "Reopen Lanterns");
        assert!(v.song_faint);
        assert!(matches!(&p.retry()[..], [Do::Open(s)] if s.name == "Copper sky"));
        p.open_failed("still missing");
        assert_eq!(p.reopen(), [Do::Reset]);
        assert_eq!(p.now(), 0);
        assert_eq!(p.armed(), None);
        assert_eq!(p.step(), Step::Idle);
        assert!(!p.view(&Now::default()).card.failed);
    }

    /// A song the check found Not ready can be armed, shows BLOCKED with
    /// why, and Go does nothing.
    #[test]
    fn a_blocked_song_never_goes() {
        let mut p = started();
        p.press(Press::Arm(2), TAP, 0, false, false);
        let v = p.view(&Now::default());
        assert_eq!(v.songs[2].mark, SongMark::Next("blocked".into()));
        assert_eq!(v.card.word.as_ref().unwrap().0, "BLOCKED");
        assert_eq!(v.card.meaning, "the file samples/strings.wav is missing");
        assert!(matches!(
            &p.press(Press::Go, TAP, 200, false, false)[..],
            [Do::Tell(_)]
        ));
        assert_eq!(p.step(), Step::Idle);
    }

    /// The parts and the song line come from the song list.
    #[test]
    fn the_parts_and_the_time_left_come_from_the_song_list() {
        let row = |name: &str, times, bars, playing| ListRow {
            name: name.into(),
            scene: Some(0),
            times,
            bars,
            starts: 1,
            playing,
            waiting: false,
        };
        let now = Now {
            bpm: 120.0,
            beats: 4,
            list: vec![
                row("Intro", 1, 4, None),
                row("Verse", 2, 8, Some((2, 0.5))),
                row("Outro", 1, 4, None),
            ],
            ..Now::default()
        };
        let p = started();
        let v = p.view(&now);
        // Half of the last pass of 8 bars, then 4 bars: 8 bars at 2 s.
        assert_eq!(
            v.song_line,
            "120 BPM  \u{b7}  part 2 of 3  \u{b7}  0:16 left"
        );
        assert_eq!(v.parts[0].mark, PartMark::Played);
        assert_eq!(
            v.parts[1].mark,
            PartMark::Now {
                pass: "pass 2 of 2".into(),
                done: 0.5
            }
        );
        assert_eq!(v.parts[2].mark, PartMark::Next);
        assert_eq!(v.parts[1].meta, "\u{d7} 2 \u{b7} 8 bars");
    }

    /// The keyboards card (live shows C1): with patches, a keyboard's line
    /// names the patch its keys reach and draws its zones; without, it
    /// says whether it is connected; one not connected says so either way.
    #[test]
    fn the_keyboards_card_names_the_patch_each_keyboard_plays() {
        use mpfree_time_ui::show::KeyBar;
        let bar = KeyBar {
            from: 0,
            to: 333,
            row: 0,
            rows: 1,
            color: mpfree_ui::Color::rgb(0x4FA3FF),
            missing: false,
        };
        let kb = |name: &str, connected, patch: Option<&str>| KeyboardNow {
            name: name.into(),
            connected,
            patch: patch.map(|p| (p.to_string(), vec![bar])),
        };
        let now = Now {
            keyboards: vec![
                kb("KeyLab 61", true, Some("Chorus split")),
                kb("Nord", false, Some("Chorus split")),
                kb("Keystep", true, None),
            ],
            ..Now::default()
        };
        let v = started().view(&now);
        let lines: Vec<(&str, &str, usize)> = v
            .keyboards
            .iter()
            .map(|k| (k.name.as_str(), k.plays.as_str(), k.bars.len()))
            .collect();
        assert_eq!(
            lines,
            [
                ("KeyLab 61", "Chorus split", 1),
                ("Nord", "not connected", 0),
                ("Keystep", "connected", 0)
            ]
        );
        assert_eq!(v.keyboards[1].tone, Tone::Dim);
    }

    /// The set changes while it plays: a check marks a song not ready, and
    /// a song taken out keeps NOW on the same song.
    #[test]
    fn the_set_changes_while_it_plays() {
        let mut p = started();
        p.press(Press::NextSong, TAP, 0, false, false);
        p.press(Press::Go, TAP, 100, false, false);
        p.ended(false);
        p.opened();
        assert_eq!(p.now(), 1);
        let mut set = p.set.clone();
        set.songs[3].check = Some((
            2,
            SongCheck {
                not_ready: vec!["Keystep 37 isn\u{2019}t connected".into()],
                ..SongCheck::default()
            },
        ));
        p.set_changed(set.clone());
        p.press(Press::Arm(3), TAP, 500, false, false);
        assert!(matches!(
            &p.press(Press::Go, TAP, 600, false, false)[..],
            [Do::Tell(_)]
        ));
        set.songs.remove(0);
        p.set_changed(set);
        assert_eq!(p.now(), 0, "Copper sky is first now");
        assert_eq!(p.view(&Now::default()).song_name, "Copper sky");
        assert_eq!(p.armed(), None);
    }
}
