//! Cues that wait for their audio (the audio tracks rethink's phase 2,
//! with streaming on; docs/dev/design/audio-tracks.md, "Cues wait for
//! their audio"). A cue that launches an audio clip stays in the cue list,
//! and the screens show it cued, but it goes into the view only once the
//! app says the start of its audio is held in memory
//! ([`TimeControl::audio_held`]). A slow drive can make a cue late; it can
//! never start audio that isn't there.
//!
//! - **What waits.** The cues decided together (a tap, a scene, a follow
//!   action: the cues new since the last compile) wait together, under one
//!   [`CueKey`], when any of them launches an audio clip and the app has
//!   turned waits on ([`TimeControl::set_audio_waits`]). A scene's program
//!   changes and the mutes it queued for the tracks it gave no cue wait
//!   with its cues. Song mode's blocks, Rec song and a launch while stopped
//!   (not a cue) never wait. The offline copy never turns waits on.
//! - **What it needs.** [`AudioNeed`]: for each region of each audio clip
//!   it launches, the first second of the region's recording from where it
//!   will start to play: its start, or for a legato clip the place in the
//!   pass it takes over.
//! - **Its line.** Held before its line, it lands there. A hold that comes
//!   in the last few ms before the line lands late, in phase with it, as
//!   any late decision does (Arrange 2.5, counted by the player). The line
//!   played without it, it moves to the next line of its grid, said and
//!   counted, and after [`MOVES`] moves it is cancelled. Launch at Now has
//!   no line: it waits up to [`NOW_BARS`] bars. A loop jump or a locate
//!   aims it at the next line from the new place without counting a move.
//!   A hold given back before the cue lands takes it out of the view again
//!   and moves it ([`TimeControl::audio_given_back`]).
//!
//! [`TimeControl::audio_held`]: super::control::TimeControl::audio_held
//! [`TimeControl::set_audio_waits`]: super::control::TimeControl::set_audio_waits
//! [`TimeControl::audio_given_back`]: super::control::TimeControl::audio_given_back

use mpfree_perform::Perform;
use mpfree_play::{pass_into, Cue, Launch, PlayView};
use mpfree_seq::{LaunchGrid, Project, RecId};
use mpfree_time_ui::view::{cue_tick, AudioWaitCounts, MuteWait, TransportInfo};
use mpfree_transport::time::{frames_between, Tick};

/// Lines a cue may move to before it is cancelled.
pub const MOVES: u8 = 2;
/// Bars a cue launched at Now waits before it is cancelled.
pub const NOW_BARS: Tick = 2;

/// The cues decided together that wait for their audio together: a tap's
/// one cue, or every cue of a scene or a follow action.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CueKey(u32);

/// Audio a waiting cue needs held before it can go: frames `from..to` of
/// recording `rec`, the first second of a region from where it starts.
// The app's streaming reads it (the audio tracks rethink's phase 2); until
// then only tests do.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AudioNeed {
    pub cue: CueKey,
    pub rec: RecId,
    pub from: i64,
    pub to: i64,
}

/// Cues decided together, kept back until their audio is held.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Wait {
    key: CueKey,
    /// The cues it keeps back, as they were decided.
    pub cues: Vec<Cue>,
    /// The tracks whose scene program change waits with them.
    pub programs: u128,
    /// The tracks whose change in the mute queue they queued (a scene's
    /// mutes for the tracks it gave no cue).
    pub mutes: Vec<usize>,
    /// The scene they cue, if they are a scene's.
    pub scene: Option<usize>,
    grid: LaunchGrid,
    /// The line it lands on. At Now (and at a once clip's end), none.
    line: Tick,
    /// At Now: the frame it gives up on.
    until: u64,
    moves: u8,
    held: bool,
    /// What the log calls it.
    pub what: String,
}

/// What the control half knows that a wait needs: borrowed for a call.
pub(super) struct Ctx<'a> {
    pub project: &'a Project,
    pub cues: &'a [Cue],
    pub launches: &'a [Launch],
    pub perform: &'a Perform,
    pub queue: Option<&'a MuteWait>,
    pub info: &'a TransportInfo,
    pub rate: u32,
    /// The frame the music has reached, as far as the control half has
    /// heard.
    pub heard: u64,
}

/// The waits, and what the control half saw at its last compile to tell
/// the next decision from what was there.
#[derive(Clone, Debug, Default)]
pub(super) struct AudioWaits {
    on: bool,
    waits: Vec<Wait>,
    needs: Vec<AudioNeed>,
    /// The cues at the last compile.
    seen: Vec<Cue>,
    /// Each track's scene program change at the last compile, by id.
    seen_programs: Vec<Option<u32>>,
    /// The mute queue's tracks at the last compile.
    seen_wants: Vec<(usize, bool, bool)>,
    seen_scene: Option<usize>,
    next: u32,
    /// The song position at the last period, to tell a jump.
    last_pos: Option<Tick>,
    counts: AudioWaitCounts,
}

/// A launch of an audio clip on an audio track.
fn launches_audio(project: &Project, c: &Cue) -> bool {
    project.tracks.get(c.track).is_some_and(|t| t.is_audio())
        && c.slot
            .and_then(|s| project.clip(c.track, s))
            .is_some_and(|clip| clip.audio.is_some())
}

/// Where a line is, for the log: "bar 9", or "bar 9 beat 3" off the bar.
/// A line is the next one the playhead plays from where it is
/// ([`cue_tick`]: in a loop whose end comes first, the loop start's).
fn line_name(info: &TransportInfo, line: Tick) -> String {
    let p = info.meter.position(line);
    if p.beat <= 1 && p.tick == 0 {
        format!("bar {}", p.bar)
    } else {
        format!("bar {} beat {}", p.bar, p.beat)
    }
}

impl AudioWaits {
    /// Waits on (streaming on) or off. Off, every waiting cue goes into the
    /// next view as it is: true when one did, and it should compile.
    pub fn set_on(&mut self, on: bool) -> bool {
        self.on = on;
        if on || self.waits.is_empty() {
            return false;
        }
        self.waits.clear();
        self.set_needs(Vec::new());
        true
    }

    pub fn needs(&self) -> &[AudioNeed] {
        &self.needs
    }

    /// The counts, with how many wait now, for the screens' state.
    pub fn counts(&self) -> AudioWaitCounts {
        AudioWaitCounts {
            waiting: self.waits.iter().filter(|w| !w.held).count() as u32,
            ..self.counts
        }
    }

    fn set_needs(&mut self, needs: Vec<AudioNeed>) {
        if needs != self.needs {
            self.needs = needs;
            self.counts.needs_rev += 1;
        }
    }

    /// Bring the waits up to date with the cues as they are now, before a
    /// compile: cues gone (landed, taken back, replaced) leave their waits,
    /// and the cues decided since the last compile wait together if any of
    /// them launches audio.
    pub fn decided(&mut self, cx: &Ctx) {
        let before = self.waits.len();
        let mut changed = false;
        for w in &mut self.waits {
            let n = w.cues.len();
            w.cues.retain(|c| cx.cues.contains(c));
            changed |= w.cues.len() != n;
        }
        self.waits.retain(|w| !w.cues.is_empty());
        changed |= self.waits.len() != before;
        let tracks = cx.project.tracks.len();
        let programs: Vec<Option<u32>> = (0..tracks)
            .map(|t| cx.perform.program(t).map(|p| p.id))
            .collect();
        let wants = cx.queue.map_or(&[][..], |q| &q.want[..]);
        let scene = cx.perform.cued();
        let fresh: Vec<Cue> = cx
            .cues
            .iter()
            .filter(|c| !self.seen.contains(c))
            .copied()
            .collect();
        if self.on {
            if let Some(first) = fresh.iter().find(|c| launches_audio(cx.project, c)) {
                let new_programs = programs
                    .iter()
                    .enumerate()
                    .filter(|&(t, p)| p.is_some() && self.seen_programs.get(t) != Some(p))
                    .fold(0u128, |m, (t, _)| {
                        m | 1u128.checked_shl(t as u32).unwrap_or(0)
                    });
                let mutes = wants
                    .iter()
                    .filter(|w| !self.seen_wants.contains(w))
                    .map(|w| w.0)
                    .collect();
                let scene = scene.filter(|_| scene != self.seen_scene);
                let w = self.wait(cx, &fresh, *first, new_programs, mutes, scene);
                self.waits.push(w);
                self.counts.kept += 1;
                changed = true;
            }
        }
        self.seen.clear();
        self.seen.extend_from_slice(cx.cues);
        self.seen_programs = programs;
        self.seen_wants.clear();
        self.seen_wants.extend_from_slice(wants);
        self.seen_scene = scene;
        if changed {
            self.make_needs(cx);
        }
    }

    fn wait(
        &mut self,
        cx: &Ctx,
        cues: &[Cue],
        first: Cue,
        programs: u128,
        mutes: Vec<usize>,
        scene: Option<usize>,
    ) -> Wait {
        let p = cx.project;
        let clip = first.slot.and_then(|s| p.clip(first.track, s));
        let grid = cx
            .perform
            .cue_line(first.track)
            .or(clip.and_then(|c| c.launch))
            .unwrap_or(p.launch);
        let bar = cx.info.meter.bar_ticks().max(1);
        let pos = cx.info.position;
        let (line, until) = match grid {
            LaunchGrid::Now => (pos, cx.heard + self.bars_frames(cx, NOW_BARS * bar)),
            LaunchGrid::At(at) => {
                let to = at.max(pos) + NOW_BARS * bar;
                (at, cx.heard + self.bars_frames(cx, to - pos))
            }
            _ => (cue_tick(cx.info, grid), 0),
        };
        let what = match (scene, cues) {
            (Some(row), _) => format!("scene {}", row + 1),
            (None, [c]) => {
                let track = &p.tracks[c.track].name;
                clip.map_or_else(|| track.clone(), |cl| format!("{track}: {}", cl.name))
            }
            (None, _) => format!("{} cues", cues.len()),
        };
        self.next = self.next.wrapping_add(1);
        Wait {
            key: CueKey(self.next),
            cues: cues.to_vec(),
            programs,
            mutes,
            scene,
            grid,
            line,
            until,
            moves: 0,
            held: false,
            what,
        }
    }

    fn bars_frames(&self, cx: &Ctx, ticks: Tick) -> u64 {
        frames_between(0, ticks.max(0), cx.info.bpm, cx.rate).ceil() as u64
    }

    /// Every waiting cue's needs, in the order they wait.
    fn make_needs(&mut self, cx: &Ctx) {
        let mut needs = Vec::new();
        for w in &self.waits {
            for c in &w.cues {
                needs_of(cx, w, c, &mut needs);
            }
        }
        self.set_needs(needs);
    }

    /// The view leaves out what waits: the cues of each wait not held,
    /// with the scene program changes that wait with them.
    pub fn hold_back(&self, v: &mut PlayView) {
        for w in self.waits.iter().filter(|w| !w.held) {
            for c in &w.cues {
                v.hold_back_cue(c.track);
            }
            let mut p = w.programs;
            while p != 0 {
                v.set_scene_program(p.trailing_zeros() as usize, None);
                p &= p - 1;
            }
        }
    }

    /// The mute queue waits too: a wait not held queued some of it.
    pub fn holds_queue(&self) -> bool {
        self.waits.iter().any(|w| !w.held && !w.mutes.is_empty())
    }

    /// The app held the audio `key` needs (`ok`), or can't hold it yet.
    /// Some(true) when the view must compile (the cues go in, or come out
    /// again); None when nothing waits under `key`.
    pub fn held(&mut self, key: CueKey, ok: bool) -> Option<bool> {
        let w = self.waits.iter_mut().find(|w| w.key == key)?;
        if w.held == ok {
            return Some(false);
        }
        w.held = ok;
        if ok {
            self.counts.held += 1;
        }
        Some(true)
    }

    /// The app gave back the audio `key` held before its cues landed: they
    /// come out of the view and move to their next line, or are cancelled
    /// (returned) past [`MOVES`]. None when nothing held waits under `key`.
    pub fn given_back(&mut self, cx: &Ctx, key: CueKey) -> Option<(Option<Wait>, String)> {
        let i = self.waits.iter().position(|w| w.key == key && w.held)?;
        self.counts.given_back += 1;
        let w = &mut self.waits[i];
        w.held = false;
        let line = match w.grid {
            // At Now it just waits again, until it gives up.
            LaunchGrid::Now | LaunchGrid::At(_) => {
                return Some((None, format!("audio not ready: {} waits again", w.what)));
            }
            // The line after the one it was aiming at.
            g => {
                let mut after = *cx.info;
                after.position = w.line.max(after.position) + 1;
                cue_tick(&after, g)
            }
        };
        Some(self.moved(cx, i, line))
    }

    /// Wait `i` moves to `line`, or is cancelled past [`MOVES`]: what to
    /// take back, and the log's line.
    fn moved(&mut self, cx: &Ctx, i: usize, line: Tick) -> (Option<Wait>, String) {
        let w = &mut self.waits[i];
        w.moves += 1;
        if w.moves > MOVES {
            let w = self.waits.remove(i);
            self.counts.cancelled += 1;
            let said = format!(
                "audio not ready: {} cancelled after {} moves",
                w.what, MOVES
            );
            self.make_needs(cx);
            return (Some(w), said);
        }
        w.line = line;
        self.counts.moved += 1;
        let said = format!(
            "audio not ready: {} moves to {}",
            w.what,
            line_name(cx.info, line)
        );
        // A legato clip's place in the pass moves with the line.
        self.make_needs(cx);
        (None, said)
    }

    /// After each period: a line played without its cues moves them, past
    /// [`MOVES`] they are cancelled, and at Now they give up after
    /// [`NOW_BARS`] bars. The waits cancelled, with the log's lines.
    /// `period` is the period's length in ticks. Allocates only when one
    /// moves or is cancelled.
    pub fn period(&mut self, cx: &Ctx, period: Tick) -> Vec<(Option<Wait>, String)> {
        let pos = cx.info.position;
        let last = self.last_pos.replace(pos);
        if !self.on || self.waits.is_empty() || !cx.info.state.rolling() {
            return Vec::new();
        }
        let jumped = last.is_none_or(|l| pos < l || pos > l + period + 1);
        let mut out = Vec::new();
        let mut i = 0;
        while i < self.waits.len() {
            let w = &mut self.waits[i];
            if w.held {
                i += 1;
                continue;
            }
            match w.grid {
                LaunchGrid::Now | LaunchGrid::At(_) => {
                    if cx.heard >= w.until {
                        let w = self.waits.remove(i);
                        self.counts.cancelled += 1;
                        let said = format!(
                            "audio not ready: {} cancelled after {} bars",
                            w.what, NOW_BARS
                        );
                        out.push((Some(w), said));
                        self.make_needs(cx);
                        continue;
                    }
                }
                g if jumped => {
                    w.line = cue_tick(cx.info, g);
                    self.make_needs(cx);
                }
                // The line played in this period ([last, pos)) without
                // them. A line a loop's end sent back to its start comes
                // after the jump, which aims it again.
                g if last.is_some_and(|l| l <= w.line) && w.line < pos => {
                    let (gone, said) = self.moved(cx, i, cue_tick(cx.info, g));
                    let cancelled = gone.is_some();
                    out.push((gone, said));
                    if cancelled {
                        continue;
                    }
                }
                _ => {}
            }
            i += 1;
        }
        out
    }
}

/// Cue `c`'s needs, if it launches audio: each region's first second from
/// where it will start.
fn needs_of(cx: &Ctx, w: &Wait, c: &Cue, out: &mut Vec<AudioNeed>) {
    let p = cx.project;
    if !launches_audio(p, c) {
        return;
    }
    let Some(clip) = c.slot.and_then(|s| p.clip(c.track, s)) else {
        return;
    };
    let Some(audio) = &clip.audio else {
        return;
    };
    // Legato over a playing clip: it takes over that clip's place in its
    // pass on the line (`Switches::start`). Audio clips play at normal
    // speed, so it starts that far into its own pass.
    let line = match w.grid {
        LaunchGrid::Now => cx.info.position,
        LaunchGrid::At(at) => at.max(cx.info.position),
        _ => w.line,
    };
    let into = cx
        .launches
        .iter()
        .find(|l| l.track == c.track)
        .filter(|_| clip.legato)
        .and_then(|l| {
            let old = p.clip(l.track, l.slot)?;
            Some(pass_into(line, l.origin, old.length, old.speed).rem_euclid(clip.length.max(1)))
        })
        .unwrap_or(0);
    let (bpm, rate) = (cx.info.bpm, cx.rate);
    let second = i64::from(rate);
    for r in &audio.regions {
        let start = if r.start < into && into < r.end {
            into
        } else {
            r.start
        };
        let into_region = frames_between(r.start, start, bpm, rate) as i64;
        let len = (frames_between(r.start, r.end, bpm, rate).ceil() as i64).max(1);
        out.push(AudioNeed {
            cue: w.key,
            rec: r.rec,
            from: r.src + into_region,
            to: r.src + len.min(into_region + second).max(into_region + 1),
        });
    }
}
