//! One loop: its state, its layers and its history, and what it does in a
//! period (its methods, by job, in the files under `loops/`).

mod commands;
mod edit;
mod first_pass;
mod jobs;
mod layers;
mod overdub;
mod play;
mod start;
mod undo;

use super::*;

/// One loop: its state, its layers and its history. Its audio is in the
/// store, under its number.
#[derive(Clone)]
pub(crate) struct Loop {
    pub(crate) phase: Phase,
    pub(crate) bars: Option<u32>,
    pub(crate) latency: u32,
    pub(crate) level: f32,
    pub(crate) muted: bool,
    /// Launched: the loop plays (fading in and out as a mute does). Off,
    /// it is silent but keeps its place in the song, so a launch brings it
    /// back where it would have been ([`Looper::launch`]).
    pub(crate) on: bool,
    /// A launch waiting for a bar line: on or off from that tick.
    pub(crate) cue: Option<(bool, Tick)>,
    pub(crate) origin: Tick,
    pub(crate) len_ticks: Tick,
    /// Frames; 0 until the first pass ends.
    pub(crate) len: usize,
    /// [`Edit::Speed`]'s speed, and the octaves the audio plays faster
    /// than recorded ([`Status::rate`]).
    pub(crate) speed: i8,
    pub(crate) rate: i8,
    /// The play order: slots, first pass first. Leaving layers stay until
    /// they are silent.
    pub(crate) order: [u8; MAX_LAYERS],
    pub(crate) in_order: usize,
    pub(crate) slots: [Slot; MAX_LAYERS],
    /// For each slot, the layers a merge or a fit made it from (audio
    /// tracks phase 4c.5): the layer it was made as ([`Slot::born`]) and
    /// theirs, in play order, 0 after the last. Kept for the host, which
    /// names the recordings ([`Looper::layer_from`]); stale once the slot
    /// holds another layer.
    pub(crate) derived: [(u64, [u64; MAX_LAYERS]); MAX_LAYERS],
    pub(crate) done: [Act; HISTORY],
    pub(crate) n_done: usize,
    pub(crate) undone: [Act; HISTORY],
    pub(crate) n_undone: usize,
    /// The newest step's transaction the history let go of from its
    /// oldest end (full, or for a shape's place or a layer's memory:
    /// [`Looper::loop_steps`]).
    pub(crate) let_go: TransactionId,
    // The first pass.
    start_frame: u64,
    /// Frames of the first pass written so far, while it records.
    pub(crate) written: usize,
    pub(crate) end_tick: Option<Tick>,
    pub(crate) ending: bool,
    /// A bar's ticks when the first pass started (STOP rounds the loop to
    /// whole bars of it).
    pub(crate) bar: Tick,
    /// The last bar line the first pass reached (its tick and frame), and
    /// the frame the next one falls on at the tempo then.
    last_bar: (Tick, u64),
    next_bar: u64,
    /// Frames of the first pass that hold its sound: `len` unless STOP cut
    /// it short ([`Loop::stop_first`]), and then the rest is silent.
    pub(crate) cut: usize,
    /// Go straight into an overdub when the first pass ends.
    pub(crate) then_overdub: bool,
    /// The overdub opened as smart loop end's Auto closed the first pass:
    /// its first frame takes what was heard after the loop's end first
    /// ([`Loop::hand_off`]).
    pub(crate) od_handoff: bool,
    /// Overdubs begun with what was heard after the loop's end
    /// ([`Loop::hand_off`]), ever: [`Looper::stats`] adds them up.
    pub(crate) handoffs: u64,
    pub(crate) after: After,
    /// Fade old: the share of the older audio kept each pass while
    /// overdubbing.
    pub(crate) fade_old: f32,

    /// A/B on the seam ([`Command::AbSeam`]) runs.
    pub(crate) ab_on: bool,
    /// This time round plays the other side's seam: through `ab_views`.
    pub(crate) ab_other: bool,
    /// A/B goes off at the next half way.
    ab_off: bool,
    /// The loop's place the frame before, for the half way.
    ab_last: f64,
    /// The loop's version when A/B started: any change ends it.
    ab_version: u64,
    /// The views the rounds on the other side play through: the layers'
    /// own, but the first pass's last beat (and first, when the start
    /// moves) is the other side's.
    ab_views: [View; MAX_LAYERS],
    /// A move's A/B asked for another side while it ran
    /// ([`Command::AbMove`]): made at the next half way.
    ab_next: Option<Shift>,

    /// The first pass's writer is still at work (it runs `latency` frames
    /// behind).
    pub(crate) first_open: bool,
    /// Its writer keeps what it hears after the loop's end, up to
    /// [`KEEP_AFTER`] frames, while the loop plays.
    pub(crate) tail: bool,
    /// Its first pass closed with loop memory full: nothing after its end
    /// was kept ([`Bounds::short`]).
    pub(crate) short: bool,
    /// What was heard before the first pass, copying in as its first
    /// [`KEEP_BEFORE`] frames: the frame heard that is the pass's first,
    /// and the frames copied.
    pub(crate) before: Option<(u64, usize)>,
    /// The latency the first pass is written with (fixed when it starts).
    pub(crate) rec_latency: u32,
    /// What the writer took before the first pass's first frame, oldest
    /// at `ring_at` (for a resample's loop point, [`Loop::keep_pre`]).
    ring: [f32; 2 * FADE],
    ring_at: usize,
    // The overdub writer.
    pub(crate) od_open: bool,
    /// The slot it writes.
    pub(crate) od_layer: usize,
    od_in: usize,
    /// The loop frame it wrote last, and what it wrote there: the song
    /// slower than the loop was recorded brings it to a frame again, and
    /// faster takes it past frames, which it fills ([`Loop::write_overdub`]).
    od_q: usize,
    od_y: [f32; 2],
    /// Frames since the song jumped under it, for its fade-in at the new
    /// place (`FADE`: none).
    od_rise: usize,
    /// Its slot's count of frames recorded when it last went back over
    /// them (the song jumped back): the frames after that it has recorded
    /// once, and only those fade out where it was at the next jump.
    od_fresh_from: usize,
    /// Frames of the writer's fade-in.
    od_fade: usize,
    od_stop_in: Option<u32>,
    od_out: usize,
    /// Frames of the writer's fade-out (an overdub of one pass ends with
    /// its fade-in's length, so its end and start crossfade).
    od_out_len: usize,
    /// How overdubs record, and whether this one ends after a pass.
    pub(crate) overdubs: Overdubs,
    od_once: bool,
    /// The loudest input sample this overdub took, and whether Fade old
    /// turned the older layers down during it: an overdub of silence that
    /// faded nothing leaves no layer.
    od_peak: f32,
    od_faded: bool,
    // Gains.
    pub(crate) gain: f32,
    pub(crate) clearing: bool,
    /// The clear fading out keeps the loop for Undo ([`Command::Clear`]).
    keep_clear: bool,
    /// Soloed, and silent because another loop is.
    pub(crate) solo: bool,
    pub(crate) solo_out: bool,
    pub(crate) pos: f64,
    /// A jump's crossfade: frames left, and where the old place is now.
    jump: usize,
    jump_pos: f64,
    /// A layer's gains or presence may still be moving (cleared when they
    /// have all settled, so a still mix costs nothing per frame).
    mixing: bool,
    /// How each layer plays its audio (the edits), and the crossfade at
    /// the cuts they make.
    pub(crate) views: [View; MAX_LAYERS],
    pub(crate) seam: usize,
    /// The first pass's loop in the audio it kept ([`Shape::region`]).
    pub(crate) region: Option<(usize, usize)>,
    /// Where the first pass began and closed in the audio it kept.
    pub(crate) captured: (usize, usize),
    /// The shapes before the edits in the history.
    shapes: [Shape; EDITS],
    /// An edit's crossfade: frames left, the shape it comes from, and
    /// where that shape is now.
    pub(crate) xf: usize,
    xf_shape: usize,
    xf_pos: f64,
    /// A merge rendering: into which slot, the shape kept for its Undo,
    /// frames rendered, the period it started in, and the frame the merged
    /// layer takes over on (a frame by which it is surely rendered, so
    /// that the period size changes nothing).
    pub(crate) merging: bool,
    mg_slot: usize,
    mg_shape: usize,
    pub(crate) mg_done: usize,
    mg_from: u64,
    mg_at: u64,
    /// Each layer's gains in the merge or bounce (what it is heard at).
    pub(crate) mg_g: [[f32; 2]; MAX_LAYERS],
    /// Being bounced out: its commands wait.
    pub(crate) bouncing: bool,
    /// The first pass armed or recording records the other loops.
    pub(crate) resample: bool,
    /// Moves on whenever the layers' sound changes ([`Status::version`]).
    pub(crate) version: u64,
    /// How much of the print plays, 0 to 1 (the layers play the rest).
    pub(crate) print_on: f32,
    /// The print the looper holds for this loop: its frames (0 for none),
    /// ticks, origin and version, where it is, and where it was before a
    /// jump (as `jump_pos`).
    p_frames: usize,
    p_ticks: Tick,
    p_origin: Tick,
    p_version: u64,
    ppos: f64,
    pjump_pos: f64,
    /// A fit copying the print in: into which slot, the shape kept for its
    /// Undo, frames copied of `fit_frames`, the period it started in, and
    /// the frame it takes over on (as a merge's).
    pub(crate) fitting: bool,
    /// An overdub starts when the fit takes over.
    pub(crate) fit_overdub: bool,
    /// Overdub pressed at another speed before its print was in: the fit
    /// (and the overdub) start once it is.
    pub(crate) print_wait: bool,
    fit_slot: usize,
    fit_shape: usize,
    pub(crate) fit_done: usize,
    pub(crate) fit_frames: usize,
    fit_from: u64,
    fit_at: u64,
    /// A fit's undo or redo: the loop dips out, swaps to the shape kept at
    /// `swap_shape`, and comes back.
    pub(crate) swapping: bool,
    swap_shape: usize,
    /// A saved loop copying in: its layers (slots 0 on, their mixes set),
    /// frames, ticks and start, frames copied of all the layers, the
    /// period it started in, and the frame it starts playing on.
    pub(crate) loading: bool,
    pub(crate) ld_layers: usize,
    pub(crate) ld_frames: usize,
    /// Frames a load copies in, all its layers' together.
    pub(crate) ld_total: usize,
    /// A loop opening as the looper held it (format 2): its layers'
    /// views and its shape, which it takes when all of it is in.
    pub(crate) ld_raw: Option<LoadRaw>,
    pub(crate) ld_ticks: Tick,
    pub(crate) ld_origin: Tick,
    pub(crate) ld_done: usize,
    pub(crate) ld_from: u64,
    pub(crate) ld_at: u64,
    /// A layer of another loop copying in ([`Looper::copy_layer`]): from
    /// which loop and slot, into which slot, frames copied of
    /// `ad_frames`, the period it started in and the frame it joins on
    /// (as a load's), where the other loop's audio is read for this
    /// loop's frame 0 and how many of its frames go by for each frame
    /// here, and whether it then leaves the other loop (a move). Into an
    /// empty loop, the loop's length in ticks and start once it joins.
    pub(crate) adding: bool,
    pub(crate) ad_from: usize,
    pub(crate) ad_src: usize,
    pub(crate) ad_slot: usize,
    pub(crate) ad_frames: usize,
    pub(crate) ad_done: usize,
    pub(crate) ad_start: u64,
    pub(crate) ad_at: u64,
    pub(crate) ad_shift: f64,
    pub(crate) ad_rate: f64,
    pub(crate) ad_move: bool,
    pub(crate) ad_ticks: Tick,
    pub(crate) ad_origin: Tick,
    /// Another loop is copying one of this loop's layers.
    pub(crate) lend: Option<Lend>,
    /// A capture waiting for its bar line or its last frames.
    pub(crate) capture: Option<Capture>,
    /// A load from what was heard (a capture copying in): the frame heard
    /// that is the loop's first.
    pub(crate) ld_heard: Option<u64>,
    /// When the next first pass starts ([`Starts`]), and whether a loop
    /// waiting for you to play listens to its input
    /// ([`Command::StartsByEar`]).
    pub(crate) starts: Starts,
    pub(crate) by_ear: bool,
    /// The clock it keeps time with ([`Clock`]), and, for a free loop that
    /// holds audio, which free clock it was made on (they are numbered as
    /// they are set: `FreeTime::gen`).
    pub(crate) clock: Clock,
    pub(crate) clock_gen: u32,
    /// STOP closed its free first pass in your own time: the free clock
    /// that pass sets starts stopped.
    pub(crate) stop_close: bool,
    /// [`Status::asks`]: the reading it waits on, and whether
    /// [`Command::UseReading`] came (taken on the next period).
    pub(crate) asks: Option<Reading>,
    pub(crate) asks_yes: bool,
    /// Armed to start when you play: the song frame a note was played on,
    /// not placed on a bar line yet ([`Looper::place_hits`]).
    pub(crate) hit: Option<u64>,
    /// A note placed: the first pass starts on the next bar line, with
    /// its first frame played on this song frame, and whether that line
    /// stays the loop's start (a pickup: the frames before it lead into
    /// the first bar) or the note is the start (the song started on it).
    pub(crate) pending: Option<(u64, bool)>,
    /// Armed on the bar this period ([`Command::Record`]): the next
    /// period's start notes whether the song was playing.
    pub(crate) just_armed: bool,
    /// Armed on the bar with the song playing: the song tick of the tap,
    /// and the one [`LEAD_BEATS`] after it. No bar line between them
    /// starts the first pass, so a tap just before a bar line waits for
    /// the next one instead of starting at once (a line on the tap's own
    /// tick, the song starting with it, still does).
    pub(crate) tap_lead: Option<(f64, f64)>,
    /// The song is stopped and a note came: the host is asked to start
    /// it ([`Looper::take_start`]).
    pub(crate) want_start: bool,
    /// The song frame the first pass's first frame was played on: its
    /// start, or before it with a pickup or a start from a note.
    pub(crate) rec_from: u64,
    /// A pickup's frames: the first pass's view starts that far in, so
    /// they play at the loop's end ([`Status::lead`]).
    pub(crate) lead: usize,
    /// The first pass's frames heard before it started, copying in from
    /// what was heard.
    pub(crate) fill: Option<Fill>,
    /// The first pass records free, with the song stopped: no bars yet,
    /// its length sets the tempo ([`Command::TempoFromLoop`]).
    pub(crate) free: bool,
    /// Smart loop end: the frame the free first pass would close on
    /// without it (the closing tap), once Record closed it and it reads.
    pub(crate) tap: Option<u64>,
    /// Auto: listening on past the tap to the end of the reach.
    pub(crate) closing: bool,
    /// Suggest: the closed pass waits to the end of the reach for its
    /// decision.
    pub(crate) smart_wait: Option<SmartWait>,
    /// What smart loop end found (None: nothing to change), and where the
    /// loop's start and end now are, from the press and the tap; both hold while the loop is at `smart_version` (any other
    /// change to the sound ends them).
    pub(crate) found: Option<Shift>,
    pub(crate) moved: Shift,
    pub(crate) smart_version: u64,
    /// A snap to land on its frame ([`Looper::snap_at`]), and what became
    /// of the last one, for the host.
    pub(crate) snap: Option<SnapPlan>,
    pub(crate) snap_done: Option<SnapDone>,
    /// The snap's frame (`u64::MAX` with none): checked frame by frame.
    pub(crate) snap_frame: u64,
}

impl Loop {
    pub(crate) const NEW: Loop = Loop {
        phase: Phase::Empty,
        bars: Some(2),
        latency: 0,
        level: 1.0,
        muted: false,
        on: true,
        cue: None,
        origin: 0,
        len_ticks: 0,
        len: 0,
        speed: 0,
        rate: 0,
        order: [0; MAX_LAYERS],
        in_order: 0,
        slots: [Slot::NEW; MAX_LAYERS],
        derived: [(0, [0; MAX_LAYERS]); MAX_LAYERS],
        done: [NO_ACT; HISTORY],
        n_done: 0,
        undone: [NO_ACT; HISTORY],
        n_undone: 0,
        let_go: TransactionId::NONE,
        start_frame: 0,
        written: 0,
        end_tick: None,
        ending: false,
        bar: 4 * PPQ as Tick,
        last_bar: (0, 0),
        next_bar: 0,
        cut: 0,
        then_overdub: false,
        od_handoff: false,
        handoffs: 0,
        after: After::Play,
        fade_old: 1.0,
        ab_on: false,
        ab_other: false,
        ab_off: false,
        ab_last: 0.0,
        ab_version: 0,
        ab_views: [View::EMPTY; MAX_LAYERS],
        ab_next: None,
        first_open: false,
        tail: false,
        short: false,
        before: None,
        rec_latency: 0,
        ring: [0.0; 2 * FADE],
        ring_at: 0,
        od_open: false,
        od_layer: 0,
        od_in: 0,
        od_q: 0,
        od_y: [0.0; 2],
        od_rise: FADE,
        od_fresh_from: 0,
        od_fade: FADE,
        od_stop_in: None,
        od_out: 0,
        od_out_len: FADE,
        overdubs: Overdubs::Add,
        od_once: false,
        od_peak: 0.0,
        od_faded: false,
        gain: 0.0,
        clearing: false,
        keep_clear: false,
        solo: false,
        solo_out: false,
        pos: 0.0,
        jump: 0,
        jump_pos: 0.0,
        mixing: false,
        views: [View::EMPTY; MAX_LAYERS],
        seam: FADE,
        region: None,
        captured: (0, 0),
        shapes: [NO_SHAPE; EDITS],
        xf: 0,
        xf_shape: 0,
        xf_pos: 0.0,
        merging: false,
        mg_slot: 0,
        mg_shape: 0,
        mg_done: 0,
        mg_from: 0,
        mg_at: 0,
        mg_g: [[0.0; 2]; MAX_LAYERS],
        bouncing: false,
        resample: false,
        version: 0,
        print_on: 0.0,
        p_frames: 0,
        p_ticks: 0,
        p_origin: 0,
        p_version: 0,
        ppos: 0.0,
        pjump_pos: 0.0,
        fitting: false,
        fit_overdub: false,
        print_wait: false,
        fit_slot: 0,
        fit_shape: 0,
        fit_done: 0,
        fit_frames: 0,
        fit_from: 0,
        fit_at: 0,
        swapping: false,
        swap_shape: 0,
        loading: false,
        ld_layers: 0,
        ld_frames: 0,
        ld_total: 0,
        ld_raw: None,
        ld_ticks: 0,
        ld_origin: 0,
        ld_done: 0,
        ld_from: 0,
        ld_at: 0,
        adding: false,
        ad_from: 0,
        ad_src: 0,
        ad_slot: 0,
        ad_frames: 0,
        ad_done: 0,
        ad_start: 0,
        ad_at: 0,
        ad_shift: 0.0,
        ad_rate: 1.0,
        ad_move: false,
        ad_ticks: 0,
        ad_origin: 0,
        lend: None,
        capture: None,
        ld_heard: None,
        starts: Starts::Bar,
        by_ear: false,
        clock: Clock::Song,
        clock_gen: 0,
        stop_close: false,
        asks: None,
        asks_yes: false,
        hit: None,
        pending: None,
        just_armed: false,
        tap_lead: None,
        want_start: false,
        rec_from: 0,
        lead: 0,
        fill: None,
        free: false,
        tap: None,
        closing: false,
        smart_wait: None,
        found: None,
        moved: Shift::NONE,
        smart_version: 0,
        snap: None,
        snap_done: None,
        snap_frame: u64::MAX,
    };

    /// Armed to start when you play, and no note yet.
    pub(crate) fn listening(&self) -> bool {
        self.phase == Phase::Armed
            && self.starts == Starts::Play
            && !self.resample
            && self.hit.is_none()
            && self.pending.is_none()
    }

    /// Whether it goes by the free clock now (`rolling`: whether that
    /// runs): a free loop, unless it is armed while the free clock doesn't
    /// run, when it waits for the song's bar line (or, with nothing
    /// playing, starts at once or on your note) as a song loop waits.
    pub(crate) fn on_free(&self, rolling: bool) -> bool {
        self.clock == Clock::Free && (rolling || self.phase != Phase::Armed)
    }

    /// The run it plays from in [`Looper::process`]: `free_run` (the free
    /// clock's ticks for the same frames) while it goes by the free clock
    /// (`any_free`: some free loop holds, starts or loads one; `rolling`:
    /// the free clock runs), else the song's `run`. One place for the
    /// choice, so that whatever cuts a loop's run further (audio tracks'
    /// play runs) starts from the run this gives.
    #[inline]
    pub(crate) fn run_from<'r, 'a>(
        &self,
        any_free: bool,
        rolling: bool,
        run: &'r Run<'a>,
        free_run: &'r Run<'a>,
    ) -> &'r Run<'a> {
        if any_free && self.on_free(rolling) {
            free_run
        } else {
            run
        }
    }

    /// A free loop with audio, or with audio coming in: the free clock
    /// lasts while one does.
    pub(crate) fn holds_free(&self) -> bool {
        self.clock == Clock::Free
            && (!matches!(self.phase, Phase::Empty | Phase::Armed) || self.loading || self.adding)
    }

    /// Make song, or its Undo: the loop keeps time with `clock` from here,
    /// its ticks moved `by` (the new clock's less the old one's), so it
    /// plays on in its place: its start, the shapes its history goes back
    /// to and its print's.
    pub(crate) fn change_clock(&mut self, clock: Clock, by: Tick) {
        self.clock = clock;
        self.origin += by;
        self.p_origin += by;
        for s in &mut self.shapes {
            s.origin += by;
        }
    }

    /// Why this loop, holding sound, can't change clock as it plays (Make
    /// song, or its Undo), or None: it plays as it was left, with nothing
    /// under way that counts in its clock's ticks (an overdub, a snap, a
    /// merge, a fit, a load, a copy, a capture, a launch, a clear) and no
    /// print (a print keeps the ticks it was made at).
    pub(crate) fn clock_refusal(&self, print: bool) -> Option<&'static str> {
        if self.phase != Phase::Playing || self.od_open {
            Some("a loop is overdubbing")
        } else if self.busy() || self.clearing || self.loading || self.capturing() {
            Some("a loop is busy")
        } else if self.cue.is_some() {
            Some("a loop waits for a launch")
        } else if print || self.p_frames > 0 || self.print_on > 0.0 || self.print_wait {
            Some("a loop plays at another speed")
        } else {
            None
        }
    }

    /// An armed loop lets go: empty again (Record pressed again).
    pub(crate) fn disarm(&mut self) {
        self.phase = Phase::Empty;
        self.resample = false;
        self.unhear();
    }

    /// Forget a note heard or placed (the loop listens again, if armed).
    pub(crate) fn unhear(&mut self) {
        self.hit = None;
        self.pending = None;
        self.want_start = false;
    }

    /// Recording or about to: armed, the first pass, or an overdub.
    pub(crate) fn recording(&self) -> bool {
        matches!(self.phase, Phase::Armed | Phase::Recording) || self.od_open
    }

    /// A capture waiting, or copying in.
    pub(crate) fn capturing(&self) -> bool {
        self.capture.is_some() || (self.loading && self.ld_heard.is_some())
    }

    /// The capture is called off: the loop stays empty (its memory goes
    /// back with the reclaim after the command).
    pub(crate) fn call_off_capture(&mut self) {
        self.capture = None;
        if self.ld_heard.is_some() {
            self.loading = false;
            self.ld_heard = None;
        }
    }
}

/// A run of frames, the same for every loop.
pub(crate) struct Run<'a> {
    /// The run's first frame, counted in its span.
    pub(crate) from: usize,
    /// Each frame's song position.
    pub(crate) ticks: &'a [f64],
    /// The line input, interleaved stereo.
    pub(crate) input: &'a [f32],
    /// The run's first frame.
    pub(crate) f: u64,
    pub(crate) playing: bool,
    /// For a loop on an audio track ([`Looper::play`]): whether its first
    /// frame looks for a jump, in place of the span's first frame doing so
    /// (`on_track::first_of`).
    pub(crate) first: Option<bool>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Overdub,
    Delete,
    Edit,
    Merge,
    Fit,
    /// A layer copied in from another loop.
    Added,
    /// Smart loop end's change of the start and the end, with the tempo
    /// (an [`Edit::Move`] and an [`Edit::Resize`] at once, made on the
    /// loop's downbeat).
    Snap,
    /// Read as: the loop's bars changed, its length and sound kept, with
    /// the tempo, on its downbeat.
    Bars,
}

/// The other loop's side of a layer copied between loops: the loop and
/// the layer's slot there, and whether the two steps undo together (a
/// move).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Link {
    pub(crate) lp: u8,
    pub(crate) slot: u8,
    pub(crate) tied: bool,
}

/// An overdub added, a layer deleted, an edit or a merge, for Undo and
/// Redo: for an overdub or a delete, the layer's slot and its place in the
/// play order; for an edit, where the loop's shape before it is kept; for
/// a merge, the merged layer's slot, and where the play order it did not
/// choose (the layers before it, or after an undo the merged one) is kept.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Act {
    kind: Kind,
    pub(crate) slot: u8,
    index: u8,
    /// The edit, for the history the screen shows.
    edit: Option<Edit>,
    /// An overdub's: every layer's Fade old turn-down from the other side
    /// of it (before it while done, after it while undone).
    ages: [f32; MAX_LAYERS],
    /// A layer copied in from, or moved to, another loop.
    pub(crate) link: Option<Link>,
    /// A snap's move ([`Kind::Snap`]).
    shift: Shift,
    /// The bars Read as made ([`Kind::Bars`]).
    bars: u32,
    /// The step's transaction (audio tracks phase 4d): given as it goes
    /// on the history ([`Loop::push_done`]), kept through Undo and Redo,
    /// so the one UNDO across the engines knows it ([`Looper::loop_steps`]).
    pub(crate) txn: TransactionId,
}

pub(crate) const NO_ACT: Act = Act {
    kind: Kind::Overdub,
    slot: 0,
    index: 0,
    edit: None,
    ages: [1.0; MAX_LAYERS],
    link: None,
    shift: Shift::NONE,
    bars: 0,
    txn: TransactionId::NONE,
};

impl Act {
    pub(crate) fn step(&self) -> Step {
        match self.kind {
            Kind::Overdub => Step::Overdub(self.slot),
            Kind::Delete => match self.link {
                Some(l) => Step::Moved(self.slot, l.lp as usize),
                None => Step::Delete(self.slot),
            },
            Kind::Added => Step::Added(self.slot, self.link.map_or(0, |l| l.lp as usize)),
            Kind::Edit => Step::Edit(self.edit.unwrap_or(Edit::Half)),
            Kind::Snap => Step::Snap(self.shift),
            Kind::Bars => Step::Bars(self.bars),
            Kind::Merge => Step::Merge(self.slot),
            Kind::Fit => Step::Fit(self.slot),
        }
    }

    /// Whether taking its shape, or giving it back, crossfades: Read as
    /// keeps every frame where it was (only the bars and the tempo change),
    /// so there is nothing to fade.
    fn fades(&self) -> bool {
        self.kind != Kind::Bars
    }

    /// Where the act keeps a shape, if it does.
    fn shape(&self) -> Option<usize> {
        match self.kind {
            Kind::Edit | Kind::Snap | Kind::Bars => Some(self.slot as usize),
            Kind::Merge | Kind::Fit => Some(self.index as usize),
            Kind::Overdub | Kind::Delete | Kind::Added => None,
        }
    }
}

/// What an edit changes: the loop's length and place, its seam fade, and
/// every layer's view. A merge keeps a play order in one.
#[derive(Clone, Copy)]
pub(crate) struct Shape {
    len: usize,
    len_ticks: Tick,
    origin: Tick,
    seam: usize,
    /// The first pass's loop in the audio it kept (frames: its first,
    /// and the one on bar 1), while layer 0 plays that region alone
    /// ([`Loop::bounds_now`]).
    region: Option<(usize, usize)>,
    speed: i8,
    rate: i8,
    views: [View; MAX_LAYERS],
    order: [u8; MAX_LAYERS],
    in_order: usize,
}

const NO_SHAPE: Shape = Shape {
    len: 0,
    len_ticks: 0,
    origin: 0,
    seam: FADE,
    region: None,
    speed: 0,
    rate: 0,
    views: [View::EMPTY; MAX_LAYERS],
    order: [0; MAX_LAYERS],
    in_order: 0,
};

/// A loop opening as the looper held it (format 2, audio tracks phase
/// 4c): each layer's view of its audio, and the loop's shape, checked
/// before anything changes ([`Looper::load`]) and taken when all of its
/// audio is in.
#[derive(Clone, Copy)]
pub(crate) struct LoadRaw {
    pub(crate) views: [View; MAX_LAYERS],
    shape: save::RawShape,
}

impl LoadRaw {
    /// Its views and shape, if `f` was kept as the looper held it (None:
    /// as it plays). Err when they don't hold together: a layer's view
    /// that doesn't play the loop's length, frames written past its audio,
    /// a seam too long, or a first pass's region or capture outside layer
    /// 0's audio; or only some of it kept that way.
    pub(crate) fn of(f: &LoopFile) -> Result<Option<LoadRaw>, ()> {
        let Some(shape) = f.raw else {
            return match f.layers.iter().any(|y| y.raw.is_some()) {
                true => Err(()),
                false => Ok(None),
            };
        };
        let mut views = [View::EMPTY; MAX_LAYERS];
        for (k, y) in f.layers.iter().enumerate() {
            let r = y.raw.as_ref().ok_or(())?;
            let v = View::from_pieces(r.size, &r.pieces).ok_or(())?;
            if v.len() != f.frames || r.valid.0 >= r.size || r.valid.1 > r.size {
                return Err(());
            }
            *views.get_mut(k).ok_or(())? = v;
        }
        let first = views[0].size();
        let inside = |(a, b): (usize, usize)| a <= first && b <= first;
        let ok = shape.seam <= MAX_SEAM
            && shape.region.is_none_or(inside)
            && shape.captured.0 <= shape.captured.1
            && inside(shape.captured);
        ok.then_some(Some(LoadRaw { views, shape })).ok_or(())
    }
}

/// A layer's place in its loop and its mix. Its audio is in the store.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Slot {
    /// (first position written, frames written).
    pub(crate) valid: (usize, usize),
    pub(crate) level: f32,
    pub(crate) pan: f32,
    pub(crate) mute: bool,
    pub(crate) solo: bool,
    /// Gains now, left and right: they move to the settings over FADE.
    pub(crate) gl: f32,
    pub(crate) gr: f32,
    /// 0 to 1: fades in when the layer comes back, out when it goes.
    pub(crate) presence: f32,
    /// Fading out, to leave the play order when silent.
    pub(crate) leaving: bool,
    /// How far Fade old has turned it down: 1 not at all.
    pub(crate) age: f32,
    /// Recorded with Overdubs: Replace: its fade-in and fade-out in frames.
    /// The layers before it in the play order are silent where it was
    /// written, crossfading over those fades ([`Slot::cover`]).
    pub(crate) replace: Option<(u32, u32)>,
    /// How much it silences the layers under it now, 0 to 1: it moves to
    /// 0 while the layer is muted or left out by a solo, and back, a step
    /// a frame.
    pub(crate) duck: f32,
    /// Which layer it holds ([`LayerInfo::born`]).
    pub(crate) born: u64,
}

impl Slot {
    pub(crate) const NEW: Slot = Slot {
        valid: (0, 0),
        level: 1.0,
        pan: 0.0,
        mute: false,
        solo: false,
        gl: 1.0,
        gr: 1.0,
        presence: 1.0,
        leaving: false,
        age: 1.0,
        replace: None,
        duck: 1.0,
        born: 0,
    };

    /// Heard: not muted, and not left out by another layer's solo.
    pub(crate) fn heard(&self, any_solo: bool) -> bool {
        !self.mute && (!any_solo || self.solo)
    }

    /// How far a Replace layer covers audio frame `a` of its `size`: 1
    /// where it was written, 0 where not, and between over its fades. 0
    /// for a layer that is not a Replace.
    fn cover(&self, a: usize, size: usize) -> f32 {
        let Some((fin, fout)) = self.replace else {
            return 0.0;
        };
        let (start, count) = self.valid;
        if size == 0 || count == 0 {
            return 0.0;
        }
        if count >= size {
            return 1.0;
        }
        // Frames from the first one written.
        let d = (a + size - start % size) % size;
        if d >= count {
            return 0.0;
        }
        let up = (d as f32 + 0.5) / fin.max(1) as f32;
        let down = ((count - d) as f32 - 0.5) / fout.max(1) as f32;
        up.min(down).min(1.0)
    }

    /// The gains the settings ask for.
    pub(crate) fn target(&self, any_solo: bool) -> (f32, f32) {
        if self.mute || (any_solo && !self.solo) {
            return (0.0, 0.0);
        }
        let level = self.level * self.age;
        (
            level * (1.0 - self.pan).min(1.0),
            level * (1.0 + self.pan).min(1.0),
        )
    }
}

/// The start of a first pass that was heard before the pass began (its
/// bar line had passed, or a pickup came before it): copied in from what
/// was heard at [`LOAD_SPEED`], while the writer takes the input from
/// `to` on.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Fill {
    /// The frame heard that is the pass's first.
    from: u64,
    done: usize,
    to: usize,
    /// The frame it began on (its pace counts from there).
    at: u64,
}

/// A capture waiting ([`Command::Capture`]): the bars asked for, the bar
/// line it ends on, whether that line is still to come, and once it has
/// come, the take.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Capture {
    pub(crate) bars: u32,
    pub(crate) end: CaptureEnd,
    pub(crate) wait: bool,
    pub(crate) take: Option<Take>,
}

/// The bar line a capture ends on: one of the song's, counted as
/// [`Heard`] counts them (a free loop with no free clock yet takes the
/// song's bars, and the free clock is set from them); or, while the free
/// clock runs, one of its own for a free loop: its tick, and the clock's
/// stamp ([`ClockStamp`]), so that a capture on a clock since started
/// again, set again or given another tempo is refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CaptureEnd {
    Song(u64),
    Free { end: Tick, stamp: ClockStamp },
}

/// Whose bar lines a capture takes ([`Looper::capture_bars`]): the
/// song's, as [`Heard`] keeps them, or a run of the free clock's.
#[derive(Clone, Copy, Debug)]
pub(crate) enum CaptureBars {
    Song,
    Free(FreeRun),
}

/// One of a loop's layers being copied into another loop: its slot, the
/// loop and the slot it goes to, the frame the copy joins that loop on,
/// and whether it leaves this loop then (a move).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Lend {
    pub(crate) slot: u8,
    pub(crate) to: u8,
    pub(crate) to_slot: u8,
    pub(crate) at: u64,
    pub(crate) drop: bool,
}
