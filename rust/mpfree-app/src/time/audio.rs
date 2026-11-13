//! The time engine's audio half: the transport, the player and the
//! recorder, the play view, the click, and what goes out each period (the
//! pads' notes, the locks and lanes for the mod engine, the pad hits for
//! the Play page). It runs once a period on the audio thread, allocates
//! and frees nothing, and hears from the control half only through its
//! queue (see `link`).
//!
//! It makes each recorded change to the view it plays in place, numbers
//! it and keeps it in `Recent`, then sends it on, so the control half
//! makes it to the project in the same order. A view the control half
//! compiled before the newest changes gets them before it plays
//! (`Recent::catch_up`); one that cannot goes back with a request to
//! compile again, and the view that holds everything plays on.

use mpfree_play::{Event, Out, PlayView, Player, Rec, Recent, Recorder, TrackArps};
use mpfree_play::{MOVE_POINTS, TOUCH_POINTS};
use mpfree_seq::mpfree_moves::{Input, Kind};
use mpfree_seq::TouchPoint;
use mpfree_time_ui::view::TransportInfo;
use mpfree_transport::{Block, Command, Mark, State, Transport};

use super::follow::{Follow, FollowCounts, Follower};
use super::link::{
    Counts, PeriodEnd, Rx, ToAudio, ToControl, Tracks, Tx, POINTS_CHUNK, TOUCH_CHUNK,
};
use super::{Click, Collect, HitInfo, LaneOut, LockOut, Pads, MAX_HITS, MAX_LOCKS, MAX_OUT};

/// Views kept here while the queue back to the control half is full,
/// until they can go back to be freed there.
const RETURNS: usize = 8;

pub struct TimeAudio {
    // Boxed: the fixed tables are large.
    transport: Box<Transport>,
    player: Box<Player>,
    recorder: Box<Recorder>,
    block: Box<Block>,
    pub(super) sink: Box<Collect>,
    pub(super) view: Box<PlayView>,
    /// The last recorded changes, for a view that comes in late.
    pub(super) recent: Box<Recent>,
    /// The tracks' settings, as the control half last sent them.
    tracks: Tracks,
    /// Another device's clock, when the transport follows one, and its
    /// counts as last sent.
    follower: Follower,
    follow_sent: FollowCounts,
    click: Click,
    /// Click while counting in and recording.
    pub metronome: bool,
    /// [`TimeAudio::advance`] ran the transport for the coming period (and
    /// whether it could).
    advanced: Option<bool>,
    /// A start or jump after the first span of the last period: the next
    /// arp clock says it jumped.
    #[allow(dead_code)] // read by arp_clock only
    jump_pending: bool,
    /// Pad hits this period, for the screen.
    hits: [HitInfo; MAX_HITS],
    n_hits: usize,
    /// Each pad's pressure last sent to CAPTURE's history, None after a
    /// press: a value that did not change is not sent again.
    pressure_sent: [Option<u8>; 16],
    /// The clips' parameter locks and lanes' values this period, for the
    /// mixer's mod engine.
    locks: Vec<LockOut>,
    lanes: Vec<LaneOut>,
    locks_dropped: u64,
    tx: Tx<ToControl>,
    rx: Rx<ToAudio>,
    /// Views waiting to go back.
    returns: [Option<Box<PlayView>>; RETURNS],
    lost: u64,
    behind: u64,
    kept: u64,
    refused: u64,
}

impl TimeAudio {
    /// Playing `view` (the control half sends the first real one), with
    /// the tracks' settings `tracks`.
    pub fn new(
        transport: Box<Transport>,
        rate: u32,
        tracks: Tracks,
        tx: Tx<ToControl>,
        rx: Rx<ToAudio>,
    ) -> TimeAudio {
        let blank = Out {
            offset: 0,
            frame: 0,
            track: 0,
            dest: 0,
            event: Event::AllOff,
        };
        TimeAudio {
            transport,
            player: Box::new(Player::new()),
            recorder: Box::new(Recorder::new()),
            block: Box::new(Block::new()),
            sink: Box::new(Collect {
                out: [blank; MAX_OUT],
                arp: [false; MAX_OUT],
                n: 0,
                dropped: 0,
            }),
            view: Box::new(PlayView::empty()),
            recent: Box::new(Recent::new()),
            tracks,
            follower: Follower::new(rate),
            follow_sent: FollowCounts::default(),
            click: Click::new(rate),
            metronome: true,
            advanced: None,
            jump_pending: false,
            hits: [HitInfo::default(); MAX_HITS],
            n_hits: 0,
            pressure_sent: [None; 16],
            locks: Vec::with_capacity(MAX_LOCKS),
            lanes: Vec::with_capacity(MAX_LOCKS),
            locks_dropped: 0,
            tx,
            rx,
            returns: std::array::from_fn(|_| None),
            lost: 0,
            behind: 0,
            kept: 0,
            refused: 0,
        }
    }

    /// Take what the control half sent: views, transport commands and the
    /// tracks' settings. At the start of each period, before the
    /// transport runs; the one-thread host also calls it after each call
    /// on the control half.
    pub fn receive(&mut self) {
        self.send_returns();
        while let Some(m) = self.rx.recv() {
            match m {
                ToAudio::View(v) => self.take_view(v),
                ToAudio::Command(cmd) => {
                    if !self.command(cmd) {
                        self.refused += 1;
                    }
                }
                ToAudio::Tracks(t) => self.tracks = t,
            }
        }
    }

    /// A new view: it plays from now on, with the recorded changes made
    /// since it was compiled. One that cannot catch up goes back.
    fn take_view(&mut self, mut v: Box<PlayView>) {
        let after = v.includes().map_or(self.recent.last(), |i| i.recorded);
        match self.recent.catch_up(&mut v, after) {
            Ok(()) => {
                let old = std::mem::replace(&mut self.view, v);
                self.give_back(old);
            }
            Err(_) => {
                self.behind += 1;
                self.send(ToControl::Compile);
                self.give_back(v);
            }
        }
    }

    /// A view to free on the control half. While the queue is full it
    /// waits here; past [`RETURNS`] of them, it is kept for good rather
    /// than freed on this thread (counted: it should never happen).
    fn give_back(&mut self, v: Box<PlayView>) {
        if let Err(ToControl::Done(v)) = self.tx.send(ToControl::Done(v)) {
            match self.returns.iter_mut().find(|r| r.is_none()) {
                Some(r) => *r = Some(v),
                None => {
                    self.kept += 1;
                    std::mem::forget(v);
                }
            }
        }
    }

    fn send_returns(&mut self) {
        for r in &mut self.returns {
            if let Some(v) = r.take() {
                if let Err(ToControl::Done(v)) = self.tx.send(ToControl::Done(v)) {
                    *r = Some(v);
                    return;
                }
            }
        }
    }

    /// Send a message that holds nothing to free; a full queue loses it
    /// (counted).
    fn send(&mut self, m: ToControl) {
        if self.tx.send(m).is_err() {
            self.lost += 1;
        }
    }

    pub fn hits(&self) -> &[HitInfo] {
        &self.hits[..self.n_hits]
    }

    pub fn clear_hits(&mut self) {
        self.n_hits = 0;
    }

    pub fn locks(&self) -> &[LockOut] {
        &self.locks
    }

    pub fn lanes(&self) -> &[LaneOut] {
        &self.lanes
    }

    pub fn clear_locks(&mut self) {
        self.locks.clear();
        self.lanes.clear();
    }

    fn lock_out(
        locks: &mut Vec<LockOut>,
        dropped: &mut u64,
        offset: u32,
        addr: mpfree_play::mpfree_param::ParamAddr,
        value: Option<f32>,
    ) {
        if locks.len() < locks.capacity() {
            locks.push((offset, addr, value));
        } else {
            *dropped += 1;
        }
    }

    /// A transport command, from the next period on. False if the
    /// transport's queue is full.
    pub fn command(&mut self, cmd: Command) -> bool {
        let at = self.transport.frame();
        self.transport.command(at, cmd)
    }

    /// The app's tempo, unless the transport follows another device's
    /// clock that is arriving.
    pub fn follow_tempo(&mut self, bpm: f64) {
        if self.follower.bpm().is_some() {
            return;
        }
        if (self.transport.bpm() - bpm).abs() > 1e-9 {
            self.command(Command::SetTempo(bpm));
        }
    }

    pub fn pad_pressed(&mut self, offset: u32, event: Event) {
        if self.tracks.records_pressed() {
            self.recorder.input(offset, event);
            self.played(offset, event);
        }
    }

    pub fn pad(&mut self, offset: u32, event: Event) {
        if !self.tracks.records_pressed() {
            self.recorder.input(offset, event);
            self.played(offset, event);
        }
        let Event::NoteOn { note, velocity } = event else {
            return;
        };
        if self.n_hits >= MAX_HITS {
            return;
        }
        let span = self
            .block
            .spans()
            .iter()
            .find(|s| offset >= s.offset && offset < s.offset + s.len.max(1));
        if let Some(s) = span.filter(|s| s.state.rolling()) {
            self.hits[self.n_hits] = HitInfo {
                pad: note,
                velocity,
                tick: s.tick_at(s.frame + u64::from(offset - s.offset)),
                bpm: s.bpm,
                bar_ticks: s.meter.bar_ticks(),
            };
            self.n_hits += 1;
        }
    }

    pub fn pad_pressure(&mut self, offset: u32, pad: u8, value: u8) {
        if pad >= 16 {
            return;
        }
        let event = Event::Pressure { note: pad, value };
        if self.tracks.records_touch() {
            self.recorder.input(offset, event);
        }
        // CAPTURE's history keeps it always, as it keeps the notes (its
        // keep leaves it out for a track that doesn't record pressure).
        let sent = &mut self.pressure_sent[usize::from(pad)];
        if *sent != Some(value) {
            *sent = Some(value);
            self.send(ToControl::Played(offset, event));
        }
    }

    /// A pad note for CAPTURE's history; a press sends its pad's next
    /// pressure value even when it is the last one sent.
    fn played(&mut self, offset: u32, event: Event) {
        if let Event::NoteOn { note, .. } = event {
            if let Some(p) = self.pressure_sent.get_mut(usize::from(note)) {
                *p = None;
            }
        }
        self.send(ToControl::Played(offset, event));
    }

    pub fn touch_dropped(&self) -> u64 {
        self.recorder.touch_dropped()
    }

    pub fn knob(&mut self, offset: u32, addr: mpfree_play::mpfree_param::ParamAddr, value: f32) {
        self.recorder.input(offset, Event::Lock { addr, value });
    }

    pub fn gesture(&mut self, offset: u32, input: Input) {
        self.send(ToControl::Moved(offset, input));
        self.recorder.input_move(offset, input);
    }

    pub fn set_fill(&mut self, on: bool, offset: u32) {
        self.player.set_fill(on, offset);
    }

    pub fn fill(&self) -> bool {
        self.player.fill()
    }

    /// Follow another device's clock with these options (the project's
    /// MIDI setup), or not (None).
    pub fn follow(&mut self, follow: Option<Follow>) {
        self.follower.set(follow);
    }

    /// The followed port's bytes, read at `frame` on the transport's frame
    /// clock ([`TimeAudio::next_frame`] for bytes read at the top of the
    /// period), before [`TimeAudio::advance`].
    pub fn clock_in(&mut self, frame: u64, bytes: &[u8]) {
        self.follower.bytes(&mut self.transport, frame, bytes);
    }

    /// The first frame of the coming period, on the transport's frame
    /// clock.
    pub fn next_frame(&self) -> u64 {
        self.transport.frame()
    }

    /// The followed clock's tempo while one arrives (for the tempo chip).
    pub fn followed_bpm(&self) -> Option<f64> {
        self.follower.bpm()
    }

    /// The transport runs the coming period, after the follower's turn.
    fn process(&mut self, frames: u32) -> bool {
        self.follower.period(&self.transport);
        self.transport.process(frames, &mut self.block)
    }

    pub fn advance(&mut self, frames: u32) -> bool {
        self.receive();
        let ok = self.process(frames);
        self.advanced = Some(ok);
        ok
    }

    #[allow(dead_code)]
    pub fn arp_clock(&mut self) -> Option<mpfree_arp::Clock> {
        let jump = |m: &Mark| {
            matches!(
                m,
                Mark::Started { .. }
                    | Mark::CountIn { .. }
                    | Mark::Located { .. }
                    | Mark::Looped { .. }
            )
        };
        let first = self.block.spans().first().copied();
        let first_offset = first.map_or(0, |s| s.offset);
        let mut jumped = std::mem::take(&mut self.jump_pending);
        for m in self.block.markers() {
            if jump(&m.mark) {
                if m.offset <= first_offset {
                    jumped = true;
                } else {
                    self.jump_pending = true;
                }
            }
        }
        let span = first?;
        if !span.state.rolling() {
            return None;
        }
        Some(mpfree_arp::Clock {
            anchor_tick: span.anchor_tick,
            frames_since_anchor: span.frame.saturating_sub(span.anchor_frame),
            bpm: span.bpm,
            playing: true,
            jumped,
            swing: (span.swing.amount > 50).then_some((span.swing.grid, span.swing.amount)),
            bar_ticks: u32::try_from(span.meter.bar_ticks()).unwrap_or(4 * mpfree_arp::PPQ),
        })
    }

    /// The per-track arps take each track's switch and destination.
    pub fn dress_arps(&self, arps: &mut TrackArps) {
        for t in 0..self.tracks.n {
            arps.set_through(t, self.tracks.through_arp[t]);
            arps.set_dest(t, self.tracks.dest[t]);
        }
    }

    /// One period: the transport (unless [`TimeAudio::advance`] ran it),
    /// the clips into `pads` (through `arps` if given), recording into the
    /// view in place, the click; then everything the control half needs
    /// from it, ending with [`ToControl::Period`].
    pub fn run(&mut self, frames: u32, pads: &mut impl Pads, arps: Option<&mut TrackArps>) {
        let ok = match self.advanced.take() {
            Some(ok) => ok,
            None => {
                self.receive();
                self.process(frames)
            }
        };
        if !ok {
            return;
        }
        self.sink.n = 0;
        self.locks.clear();
        self.lanes.clear();
        match arps {
            // The player's notes go through the per-track arps (a through
            // track's into its arp, the rest as they are), then to the pads,
            // with what the arps played (the live pads among it) marked.
            Some(arps) => {
                self.player
                    .process(&self.view, &self.block, &mut arps.through_sink());
                let sink = &mut *self.sink;
                arps.finish_with(&self.block, |o, arp| sink.push(*o, arp));
            }
            None => self
                .player
                .process(&self.view, &self.block, &mut *self.sink),
        }
        for i in 0..self.sink.n {
            let o = self.sink.out[i];
            if self.sink.arp[i] {
                // Live playing: the pad player's own treatment, and what it
                // says to record goes to the armed track as heard (or, on a
                // track whose clips play through its arp, the pads as
                // pressed do, through pad_pressed).
                let heard = pads.live(&o);
                if let Some(ev) = heard.filter(|_| self.tracks.armed == Some(usize::from(o.track)))
                {
                    self.pad(o.offset, ev);
                }
                continue;
            }
            // A track that plays elsewhere (MIDI OUT) takes its clips'
            // events first.
            if pads.out(&o) {
                continue;
            }
            match o.event {
                Event::NoteOn { note, velocity } => {
                    pads.hit_at(usize::from(note), velocity, o.offset);
                }
                Event::NoteOff { note } => pads.release_at(usize::from(note), o.offset),
                Event::AllOff => pads.stop_all(),
                Event::Pressure { note, value } => {
                    pads.pressure_at(usize::from(note), value, o.offset)
                }
                // Parameter locks go to the mod engine in the mixer
                // (`MixHost::lock`), which the audio loop hands them to.
                Event::Lock { addr, value } => Self::lock_out(
                    &mut self.locks,
                    &mut self.locks_dropped,
                    o.offset,
                    addr,
                    Some(value),
                ),
                Event::Unlock { addr } => Self::lock_out(
                    &mut self.locks,
                    &mut self.locks_dropped,
                    o.offset,
                    addr,
                    None,
                ),
                // A move lane's value goes to the mod engine too, where any
                // hand on the control beats it.
                Event::Lane { addr, value } => {
                    if self.lanes.len() < self.lanes.capacity() {
                        self.lanes.push((o.offset, addr, value));
                    } else {
                        self.locks_dropped += 1;
                    }
                }
            }
        }
        self.recorder.process(&self.view, &self.block);
        // Cues that started on a bar line: the player switched already;
        // the control half makes them the tracks' launches.
        for i in 0..self.player.started().len() {
            let st = self.player.started()[i];
            self.send(ToControl::Started(st));
        }
        // Recorded and erased notes, locks and gestures go into the view
        // in place (unless it has no room left), numbered in this order,
        // and on to the control half, which makes them to the clips.
        let mut no_room = false;
        let mut made = Made {
            view: &mut self.view,
            recent: &mut self.recent,
            tx: &mut self.tx,
            lost: &mut self.lost,
            no_room: &mut no_room,
        };
        for e in self.player.erased() {
            made.make(Rec::Erase(*e), &[], &[]);
        }
        for e in self.player.erased_locks() {
            made.make(Rec::EraseLock(*e), &[], &[]);
        }
        for k in self.recorder.taken() {
            made.make(Rec::Note(*k), self.recorder.touch_points(&k.note), &[]);
        }
        for k in self.recorder.taken_locks() {
            made.make(Rec::Lock(*k), &[], &[]);
        }
        for k in self.recorder.taken_moves() {
            made.make(Rec::Move(*k), &[], self.recorder.move_points(k));
        }
        if no_room {
            self.send(ToControl::Compile);
        }
        if self.metronome {
            for span in self.block.spans() {
                if matches!(span.state, State::CountIn | State::Recording) {
                    let bar = span.meter.bar_ticks();
                    for (tick, offset) in span.lines(span.meter.beat_ticks()) {
                        self.click.schedule(offset, tick.rem_euclid(bar) == 0);
                    }
                }
            }
        }
        for i in 0..self.block.spans().len() {
            let s = self.block.spans()[i];
            self.send(ToControl::Span(s));
        }
        let follow = self.follower.counts();
        if follow != self.follow_sent && self.tx.send(ToControl::Follow(follow)).is_ok() {
            self.follow_sent = follow;
        }
        let end = PeriodEnd {
            start_frame: self.block.start_frame,
            frames: self.block.frames,
            info: self.info(),
            counts: self.counts(),
        };
        self.send(ToControl::Period(end));
    }

    pub fn counts(&self) -> Counts {
        Counts {
            dropped: self.sink.dropped
                + self.recorder.dropped()
                + self.player.stats().dropped
                + self.locks_dropped,
            touch_dropped: self.recorder.touch_dropped(),
            lost: self.lost,
            behind: self.behind,
            kept: self.kept,
            refused: self.refused,
        }
    }

    pub fn mix_click(&mut self, left: &mut [f32], right: &mut [f32]) -> f32 {
        self.click.mix(left, right)
    }

    pub fn block(&self) -> &Block {
        &self.block
    }

    pub fn tempo_map(&self) -> &mpfree_transport::TempoMap {
        self.transport.tempo_map()
    }

    pub fn position_text(&self) -> String {
        let t = &self.transport;
        t.meter().position(t.position()).to_string()
    }

    /// The transport for the screens, with the followed clock's tempo
    /// rather than the nudged one while following.
    pub fn info(&self) -> TransportInfo {
        let mut info = TransportInfo::of(&self.transport);
        if let Some(bpm) = self.follower.bpm() {
            info.bpm = bpm;
        }
        info
    }
}

/// Where a period's recorded changes go: the view in place, the ring of
/// recent changes, and the queue to the control half.
struct Made<'a> {
    view: &'a mut PlayView,
    recent: &'a mut Recent,
    tx: &'a mut Tx<ToControl>,
    lost: &'a mut u64,
    /// The view had no room in place for one.
    no_room: &'a mut bool,
}

impl Made<'_> {
    fn make(&mut self, rec: Rec, touch: &[TouchPoint], moves: &[(f64, f32, Kind)]) {
        let touch = &touch[..touch.len().min(TOUCH_POINTS)];
        let moves = &moves[..moves.len().min(MOVE_POINTS)];
        let number = self.recent.push(rec, touch, moves);
        *self.no_room |= !self.view.record(&rec, touch, moves);
        // Its points first, then the change.
        for c in touch.chunks(TOUCH_CHUNK) {
            let mut a = [TouchPoint { at: 0, value: 0 }; TOUCH_CHUNK];
            a[..c.len()].copy_from_slice(c);
            self.send(ToControl::Touch(a, c.len() as u8));
        }
        for c in moves.chunks(POINTS_CHUNK) {
            let mut a = [(0.0, 0.0, Kind::Step); POINTS_CHUNK];
            a[..c.len()].copy_from_slice(c);
            self.send(ToControl::Points(a, c.len() as u8));
        }
        self.send(ToControl::Rec(number, rec));
    }

    fn send(&mut self, m: ToControl) {
        if self.tx.send(m).is_err() {
            *self.lost += 1;
        }
    }
}
