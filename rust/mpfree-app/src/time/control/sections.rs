//! A song linked to its list, as it plays (the Arrange refactor's 4b.4,
//! plan D8: playing the list plays the song). In song mode each section
//! made from a scene brings its scene's mutes, program changes and tempo
//! (5.2b; unless the song has tempo points of its own, which lead) in as
//! it starts, as the list launching the scene would for tracks it gives
//! no cue ([`mpfree_perform::Perform::section_starts`]): the section's
//! blocks were made with no scene's mutes, so the mutes come at play
//! time, carrying into the sections after as the list's do.
//!
//! - **The next section** is cued in the bar before it starts, as a follow
//!   action or the list's next step is: its mutes join the mute queue for
//!   its first tick, and its program changes wait for it.
//! - **The section under the playhead** comes in at once when the
//!   playhead didn't get there by playing into it: song mode turned on,
//!   the playhead moved while stopped, or a jump while it plays.
//! - **Loop the list** loops the song from its first section to the end of
//!   its last (the transport's loop, set while song mode plays a linked
//!   list with Loop the list on; the musician's loop is kept aside and
//!   given back after), and the first section is cued for the loop's
//!   jump, on its bar line.
//! - **Play from here** stops the clips launched over the song and plays
//!   the song from the section's first bar, with song mode on and the
//!   section brought in at once.
//!
//! Nothing is cued for a clip: the song plays the sections' blocks, and
//! a scene launched over the song, or Back to song, work as they do.

use mpfree_seq::arrange::{song_sections, SongSection};
use mpfree_seq::LaunchGrid;
use mpfree_transport::time::Tick;
use mpfree_transport::Command;

use super::TimeControl;
use mpfree_perform::When;

impl TimeControl {
    /// After each period: a linked song's sections and Loop the list (see
    /// the module notes), with bars of `bar` ticks. True when the view
    /// needs compiling again (mutes or program changes came in).
    pub(super) fn sections_due(&mut self, bar: Tick) -> bool {
        self.list_loop_due(bar);
        let song = &self.project.arrangement;
        // On a loop's end the playhead is on its start: it jumps there
        // before it plays on.
        let now = match self.info.loop_range {
            Some((start, end)) if self.info.position == end => start,
            _ => self.info.position,
        };
        let last = std::mem::replace(&mut self.section_last, now);
        if !self.song || !song.list.linked {
            (self.section_now, self.section_next) = (None, None);
            return false;
        }
        let rolling = self.info.state.rolling();
        let mut changed = false;
        // The section under the playhead: the cued one started, or it got
        // there another way and comes in at once.
        let here = song.part_at(now, bar);
        let moved = now < last || here.map(|s| s.start) != self.section_now;
        if moved {
            let cued = here.is_some_and(|s| self.section_next == Some(s.start));
            self.section_now = here.map(|s| s.start);
            self.section_next = None;
            if let Some(s) = here.filter(|_| !cued) {
                let line = rolling.then_some(LaunchGrid::Now);
                changed |= self.bring(&s, line);
            }
        }
        // The next, in the bar before it starts: after the last section,
        // with the loop on, the first again on the loop's jump (its bar
        // line, the first the playhead plays after it).
        if rolling && self.section_next.is_none() {
            let song = &self.project.arrangement;
            let after = song
                .part_after(now, bar)
                .filter(|s| self.list_loop.is_none_or(|(_, end)| s.start < end));
            let next = match (after, self.list_loop) {
                (Some(s), _) => Some((s, s.start, LaunchGrid::At(s.start))),
                (None, Some((start, end))) if start % bar.max(1) == 0 && now < end => song
                    .part_at(start, bar)
                    .map(|s| (s, end, LaunchGrid::Bars(1))),
                (None, _) => None,
            };
            if let Some((s, _, line)) = next.filter(|n| now > n.1 - bar) {
                self.section_next = Some(s.start);
                changed |= self.bring(&s, Some(line));
            }
        }
        changed
    }

    /// Section `s`'s scene's mutes and program changes come in on `line`
    /// (None: at once, stopped). False for a section with no scene.
    fn bring(&mut self, s: &SongSection, line: Option<LaunchGrid>) -> bool {
        let Some(from) = s.scene else {
            return false;
        };
        if !self
            .perform
            .section_starts(&mut self.project, from.row, line)
        {
            return false;
        }
        self.queue_scene_mutes();
        self.scene_tempo_on_its_line();
        self.changed = true;
        true
    }

    /// The transport's loop for Loop the list on a linked list in song
    /// mode: the song's sections, first to last, set while it is wanted
    /// (the loop the musician had kept aside), and the musician's given
    /// back once it isn't, unless they set another meanwhile.
    fn list_loop_due(&mut self, bar: Tick) {
        let song = &self.project.arrangement;
        let want = (self.song && song.list.linked && song.list.looped)
            .then(|| song.parts_span(bar))
            .flatten();
        if want == self.list_loop {
            return;
        }
        let set = match want {
            Some(r) => {
                if self.list_loop.is_none() {
                    self.loop_kept = self.info.loop_range.filter(|&l| l != r);
                }
                Some(Some(r))
            }
            None => (self.info.loop_range == self.list_loop).then(|| self.loop_kept.take()),
        };
        if let Some(r) = set {
            self.command(Command::SetLoop(r));
        }
        self.list_loop = want;
    }

    /// The transport's loop as a clip's length sees it: the musician's,
    /// not Loop the list's (a new clip is as long as a loop they set).
    pub(super) fn clip_loop(&self) -> Option<(Tick, Tick)> {
        match self.info.loop_range {
            Some(r) if Some(r) == self.list_loop => self.loop_kept,
            r => r,
        }
    }

    /// Play from here on a linked list (step `k`): see the module notes. A
    /// line for the log.
    pub(super) fn play_from_part(&mut self, k: usize) -> Result<String, String> {
        let bar = self.info.meter.bar_ticks();
        let parts = song_sections(&self.project, bar);
        let s = *parts.get(k).ok_or("song list: no such part")?;
        let name = s.marker.map_or_else(
            || "the Song row".to_string(),
            |m| self.project.arrangement.markers[m].name.clone(),
        );
        let before = (
            self.launches.clone(),
            self.cues.clone(),
            self.perform.clone(),
            self.song,
        );
        // Clips launched over the song stop: the song plays every track.
        self.perform
            .stop_all(When::Now(s.start), &mut self.launches, &mut self.cues);
        self.song = true;
        if let Err(e) = self.compile() {
            (self.launches, self.cues, self.perform, self.song) = before;
            return Err(e);
        }
        let rolling = self.info.state.rolling();
        self.bring(&s, rolling.then_some(LaunchGrid::Now));
        (self.section_now, self.section_next, self.section_last) = (Some(s.start), None, s.start);
        self.compile()?;
        self.command(Command::Locate(s.start));
        if !rolling {
            self.command(Command::Play);
        }
        self.list_pick = Some(k);
        self.changed = true;
        Ok(format!(
            "song list: the song from {name}, bar {}",
            s.start / bar.max(1) + 1
        ))
    }
}
