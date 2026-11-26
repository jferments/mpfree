//! Loops on audio tracks: their recordings, their lasting ids, and each
//! slot's sound for its track.

use super::*;

impl LoopHost {
    /// Layers as recordings (audio tracks phase 4c.2; always on in the
    /// app since 4e.5b): with it on, a save keeps each loop as the looper
    /// holds it (`mpfree_looper::Looper::loop_file_raw`, read with
    /// `raw_layer`), each layer with its recording's id, so it opens
    /// exactly and its edits can be taken further; off, each layer as it
    /// plays, as before. A project opens either way.
    pub fn set_recordings(&mut self, on: bool) {
        self.recordings = on;
    }

    /// With recordings on, each closed layer with no recording yet gets
    /// one, `fresh()` (the audio tracks' id maker). Each period; does not
    /// allocate.
    pub fn give_layer_recs(&mut self, fresh: impl FnMut() -> u128) {
        if self.recordings {
            self.layer_recs.give(&self.looper, fresh);
        }
    }

    /// With recordings on, closed layers go to their recordings' files
    /// soon after they close (phase 4c.3, docs/dev/design/loops-on-audio-tracks.md
    /// rule 4): up to [`SAVE_SPEED`] frames of them for each of the
    /// period's `frames`, through `files` (the audio tracks' worker), one
    /// layer at a time. Each period, after
    /// [`LoopHost::give_layer_recs`]; allocates only the log's line when
    /// a file couldn't be written (the layer stays in memory, and a save
    /// writes it with the project).
    pub fn write_layers(
        &mut self,
        files: &mut dyn crate::loops_on_tracks::LayerFiles,
        frames: usize,
    ) -> Option<String> {
        if !self.recordings {
            return None;
        }
        self.layer_writer.tend(
            &mut self.layer_recs,
            &self.looper,
            files,
            frames * SAVE_SPEED,
        );
        // A layer in a synced file this run wrote may leave memory while
        // only UNDO needs it (phase 4d.4).
        for (n, born) in self.layer_recs.written_here() {
            self.looper.set_synced(n, born);
        }
        let why = self.layer_writer.take_failure()?;
        Some(format!(
            "looper: a layer's file couldn't be written ({why}); its audio stays in memory \
             and is saved with the project"
        ))
    }

    /// Layers of loop `n` whose audio is only in memory, not yet in a
    /// synced file (phase 4c.3); 0 with recordings off, where saves are
    /// what keep the audio.
    pub fn unsaved_layers(&self, n: usize) -> usize {
        if self.recordings {
            self.layer_recs.unsaved(&self.looper, n)
        } else {
            0
        }
    }

    /// Loops on audio tracks (phase 4b): a loop with
    /// sound and no lasting id yet gets one, `fresh()` (the audio tracks'
    /// id maker), one a period at most; a first pass starting is a new
    /// loop, so its slot's old id goes first
    /// ([`crate::loops_on_tracks::OnTracks::follow_passes`]).
    pub fn give_loop_id(&mut self, fresh: impl FnOnce() -> u128) {
        let looper = &self.looper;
        self.on_tracks
            .follow_passes(|n| crate::loops_on_tracks::OnTracks::first_pass(looper, n));
        if let Some(n) = self.on_tracks.needs_id(&self.looper) {
            self.on_tracks.set(n, fresh());
        }
    }

    /// The slot holding the loop with lasting id `id` (audio tracks phase
    /// 4b), if one does.
    pub fn slot_of(&self, id: u128) -> Option<usize> {
        self.on_tracks.slot_of(mpfree_seq::LoopId(id))
    }

    /// Slot `n`'s loop takes lasting id `id` (0: none), as a copy of an
    /// audio track's loop cell gives the copy before its audio goes in.
    pub fn set_loop_id(&mut self, n: usize, id: u128) {
        self.on_tracks.set(n, id);
    }

    /// Loops on audio tracks, before [`LoopHost::run`]: each slot a loop
    /// cell names is told its runs from the track player (`loops`, planned
    /// on this period: `TrackHost::plan_loops`). Does not allocate.
    pub fn tell_tracks(
        &mut self,
        project: &mpfree_seq::Project,
        loops: &mpfree_audiotracks::LoopPlayer,
    ) {
        self.on_tracks.tell(&mut self.looper, project, loops);
    }

    /// Loops on audio tracks: every slot's loop's lasting id (0: none
    /// yet), for the time engine (`TimeHost::loops_are_on`).
    pub fn loop_ids(&self) -> [u128; MAX_LOOPS] {
        self.on_tracks.ids()
    }

    /// Loops on audio tracks: the loops heard now that no track places,
    /// by their lasting ids (0 where none): playing or overdubbing,
    /// launched, not muted, and not on a track (`Looper::play`): on the
    /// Loop strip, or through a free cell (4e.4), which plays itself. One
    /// that a track's runs take over carries on there unfaded
    /// (`TrackHost::loops_moving`): a first pass landing in a cell, or a
    /// free cell's loop that Make song made a song loop.
    pub fn moving_ids(&self) -> [u128; MAX_LOOPS] {
        let ids = self.on_tracks.ids();
        std::array::from_fn(|n| {
            let heard = self.looper.loop_status(n).is_some_and(|s| {
                matches!(s.phase, Phase::Playing | Phase::Overdubbing) && s.on && !s.muted
            });
            if heard && !self.looper.on_track(n) {
                ids[n]
            } else {
                0
            }
        })
    }

    /// Loops on audio tracks: a loop cell was just made (a loop landed in
    /// an audio track's cell), so its loop goes on its track at the next
    /// [`LoopHost::tell_tracks`], straight on from where it plays.
    pub fn look_again(&mut self) {
        self.on_tracks.look_again();
    }

    /// After [`LoopHost::run`]: loop `id`'s sound in the period, for its
    /// audio track (`TrackHost::period_with_loops`).
    pub fn track_loop(&self, id: u128) -> Option<&[f32]> {
        self.on_tracks.out(&self.looper, id)
    }

    /// Slot `n`'s sound this period, for its track (None: it isn't on
    /// one), after [`LoopHost::run`]: the Loop group's (phase 4e.1).
    pub fn slot_out(&self, n: usize) -> Option<&[f32]> {
        let id = self.on_tracks.id(n)?;
        self.on_tracks.out(&self.looper, id.0)
    }

    /// The free cells' loops on their tracks (phase 4e.4): (project
    /// track, lasting id), (0, 0) where none (`TrackHost::loop_free`).
    pub fn free_cells(&self) -> [(usize, u128); MAX_LOOPS] {
        self.on_tracks.free_cells(&self.looper)
    }

    /// The project track slot `n`'s loop is on, as last looked.
    pub fn slot_track(&self, n: usize) -> Option<usize> {
        self.on_tracks.track(n)
    }

    /// After [`LoopHost::run`]: the project track Listen is heard through
    /// this period, when the loop it plays is on an audio track (phase
    /// 4e.2: `TrackHost::loop_listen`); its frames are then the track's
    /// (`track_loop(mpfree_audiotracks::LISTEN)`), not the Loop strip's.
    pub fn listen_track(&self) -> Option<usize> {
        let outs = self.looper.outs();
        let (k, _) = outs.listen().filter(|l| outs.on_track(l.0))?;
        self.on_tracks.track(k)
    }
}
