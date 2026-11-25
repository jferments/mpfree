//! The audio thread's sound path, run the way `player_session` runs it each
//! period, under the counting allocator: pads and pressure into the arp app,
//! the transport and clips, the sampler (each pad into its mixer channel) or
//! the synth, the looper, the mixer and the click, out to the period's
//! samples. None of it may allocate or free while nothing is edited,
//! with the sound on one core, on two (`--sound-cores two`: the
//! sampler's and the mixer's lanes, cores.rs) or on three (`three`: the
//! synth's lane too).
//!
//! Left out, because they are the device's or the screen's: reading MIDI
//! and the screen's queue, the log lines and the screen updates (`out.note`,
//! `out.show`, and what is built only for them: `arp_poll`'s view,
//! `time.steps()`, `mix.poll_state`, the meter), and writing the period to
//! ALSA.

use crate::{
    app, audiotracks, cores, cv_out, kit, looping, midi_in, midi_out, mixing, sampler, synth, time,
    SeqPads, MAX_LIVE_HITS, PERIOD, RATE,
};
use mpfree_arp::{EventKind, Events, Mode};
use mpfree_arp_ui::view::ArpAction;
use mpfree_controller::mpc_one::PAD_NOTES;
use mpfree_looper::{Command as LoopCommand, Phase};
use mpfree_time_ui::TimeAction;
use mpfree_transport::Command;

/// What `player_session` keeps between periods, for the sound path.
struct Rig {
    app: app::App,
    time: time::TimeHost,
    player: sampler::Player,
    synth: cores::SynthSlot,
    looper: looping::LoopHost,
    /// The audio tracks, track 1 armed (song mode is off, so it only
    /// monitors).
    tracks: audiotracks::TrackHost,
    mix: mixing::MixHost,
    events: Events,
    synth_on: bool,
    /// What each track plays: every track the synth while `synth_on`,
    /// else the kit.
    track_kits: sampler::TrackKits,
    left: [f32; PERIOD],
    right: [f32; PERIOD],
    /// The tracks' sounds before the mixer, as `player_session` keeps them.
    drums: [[f32; PERIOD]; 2],
    synth_out: [[f32; PERIOD]; 2],
    loop_out: [[f32; PERIOD]; 2],
    pads_out: [[f32; PERIOD]; 2],
    peak: f32,
    /// Notes the arp started, and hits the clip played.
    arp_notes: u64,
    clip_hits: u64,
    /// MIDI Out (closed: every track plays on the pads) and the I/O view's
    /// model it reads each period.
    midi: midi_out::MidiOut,
    io: mpfree_midi_ui::Shared,
    /// CV out (closed: the jacks are not driven).
    cv: cv_out::CvOut,
    /// Export's note tap (export.rs), on as if an export ran.
    tap: crate::export::Tap,
    /// Sample Recall's master, kept every period (sampler R5 part 1).
    recall: sampler::RecFeed,
    /// When kept: every message MIDI out sent, as (the first frame of the
    /// period it was handed over in, its frame, its bytes).
    sent: Option<Vec<(u64, u64, Vec<u8>)>>,
    /// The project's instruments, and what each was handed for its tuning
    /// (tunings step T5): kit 1 in its own just, the rest on the song's
    /// 12-TET.
    listed: crate::instrument_list::InstrumentList,
    tunings: Box<crate::TuningsHanded>,
}

fn tone(frames: usize, f: f32) -> Vec<i16> {
    (0..frames)
        .flat_map(|i| {
            let x = (i as f32 * f).sin();
            [(x * 12000.0) as i16, (x * -9000.0) as i16]
        })
        .collect()
}

impl Rig {
    fn new(synth_on: bool) -> Rig {
        let a = kit::build(&[
            (PAD_NOTES[0], "Kick", &tone(6000, 0.01)[..]),
            (PAD_NOTES[1], "Snare", &tone(4000, 0.07)[..]),
            (PAD_NOTES[2], "Hat", &tone(1500, 0.3)[..]),
            (PAD_NOTES[3], "Clap", &tone(3000, 0.03)[..]),
        ]);
        let mut kits = kit::parse_kits(&kit::build_kits(&[("House", a)])).unwrap();
        let mut app = app::App::new(RATE, false);
        app.arp_action(ArpAction::Mode(Mode::Arp));
        let player = sampler::Player::new(&mut kits, &PAD_NOTES, RATE).unwrap();
        let mut listed = crate::instrument_list::InstrumentList::new();
        listed.follow_kits(player.kits());
        app.set_instrument_list(&listed);
        let kit1 = listed.of_kit(0).unwrap();
        let just = synth::TuningPick {
            builtin: synth::Builtin::Just,
            ..synth::TuningPick::default()
        };
        assert!(!app.set_own_tuning(kit1, Some(just)).is_empty());
        Rig {
            player,
            synth: cores::SynthSlot::new(synth::Synth::new(RATE)),
            looper: looping::LoopHost::new(RATE, 20),
            tracks: {
                let mut t = audiotracks::TrackHost::new(RATE, 20);
                t.host.command(mpfree_audiotracks::Command::Arm(0, true));
                t
            },
            // The device's default: 6 dB under a -12 dB maximum.
            mix: mixing::MixHost::new(RATE, -18.0, -12.0),
            time: time::TimeHost::new(RATE, 120.0).unwrap(),
            app,
            events: Events::new(),
            synth_on,
            track_kits: {
                let mut k = sampler::TrackKits::new(1);
                for t in (0..crate::instruments::TRACKS).filter(|_| synth_on) {
                    k.set(t, k.synth());
                }
                k
            },
            left: [0.0; PERIOD],
            right: [0.0; PERIOD],
            drums: [[0.0; PERIOD]; 2],
            synth_out: [[0.0; PERIOD]; 2],
            loop_out: [[0.0; PERIOD]; 2],
            pads_out: [[0.0; PERIOD]; 2],
            peak: 0.0,
            arp_notes: 0,
            clip_hits: 0,
            midi: midi_out::MidiOut::new(RATE, std::time::Duration::ZERO),
            io: midi_in::io_model(),
            cv: cv_out::CvOut::new(RATE, std::time::Duration::ZERO),
            tap: crate::export::Tap::on(),
            recall: sampler::RecFeed(
                mpfree_sampler::record::input16(crate::sample_page::SCREEN_RECALL_SECONDS, RATE).0,
            ),
            sent: None,
            listed,
            tunings: Box::new(crate::TuningsHanded::new()),
        }
    }

    /// The pads as the MIDI loop hands them over, before the period.
    fn pads(&mut self, i: u64) {
        let pad = (i / 7 % 4) as usize;
        if i.is_multiple_of(7) {
            // The returned effects are empty for a plain press.
            let fx = self.app.pad_down(pad, 60 + (i % 60) as u8);
            assert!(fx.is_empty(), "period {i}: a press made effects");
        }
        if i % 7 == 3 {
            let pressure = (i % 128) as u8;
            self.app.pressure(pad, pressure);
            if self.synth_on {
                self.synth.pressure(pad, pressure);
            } else {
                self.player.pressure(pad, pressure);
            }
        }
        if i % 7 == 5 {
            assert!(self.app.pad_up(pad).is_empty());
        }
    }

    /// One period of sound, in `player_session`'s order.
    fn period(&mut self, i: u64) {
        let Rig {
            app,
            time,
            player,
            synth,
            looper,
            tracks,
            mix,
            events,
            synth_on,
            track_kits,
            left,
            right,
            drums,
            synth_out,
            loop_out,
            pads_out,
            arp_notes,
            clip_hits,
            midi,
            io,
            cv,
            tap,
            recall,
            sent,
            listed,
            tunings,
            ..
        } = self;
        let synth_on = *synth_on;
        app.set_rolling(time.info().state.rolling());
        time.follow_tempo(app.settings.bpm);
        time.advance(PERIOD as u32);
        tap.period(time.block().start_frame);
        midi.sync(io, time.info().state.rolling());
        midi.clock(time.block());
        time.set_ahead(&midi.ahead());
        cv.sync(io);
        cv.clock(time.block());
        app.set_clock(time.arp_clock());
        app.process(PERIOD as u32, events);
        // With the synth on the pads the arp may play notes (its pad layout).
        let notes = synth_on && app.arp_plays_notes();
        for ev in events.as_slice() {
            let (slot, velocity) = match ev.kind {
                EventKind::On { note, velocity } => (note, velocity),
                EventKind::Off { note } if notes => {
                    synth.note_off_note(note, ev.offset);
                    if let Some(pad) = app.pad_of_note(note) {
                        time.pad(ev.offset, mpfree_play::Event::NoteOff { note: pad as u8 });
                    }
                    continue;
                }
                EventKind::Off { note } => {
                    if synth_on {
                        synth.note_off(note as usize, ev.offset);
                    } else {
                        player.release_at(note as usize, ev.offset);
                    }
                    time.pad(ev.offset, mpfree_play::Event::NoteOff { note });
                    continue;
                }
            };
            *arp_notes += 1;
            let v = app.velocity(velocity);
            let pad = if notes {
                synth.note_on_note(slot, v, ev.offset);
                let Some(pad) = app.pad_of_note(slot) else {
                    continue;
                };
                time.pad(
                    ev.offset,
                    mpfree_play::Event::NoteOn {
                        note: pad as u8,
                        velocity: v,
                    },
                );
                pad
            } else {
                slot as usize
            };
            if pad >= 16 || notes {
                continue;
            }
            time.pad(
                ev.offset,
                mpfree_play::Event::NoteOn {
                    note: slot,
                    velocity: v,
                },
            );
            if synth_on {
                synth.note_on(pad, v, ev.offset);
            } else {
                player.hit_at(pad, v, ev.offset);
            }
        }
        // A track's own instrument goes in, and retired ones go back, as
        // the loop does before the notes.
        let _ = mix.poll_instruments();
        // The synth-voice pads' copies follow the Synth tab.
        mix.pad_synths.follow(synth);
        let mut seq = SeqPads {
            player,
            synth,
            kits: track_kits,
            shown: track_kits.entry(app.track()),
            flashes: [0; 128],
            hits: 0,
            live: app.live_map(),
            live_hits: [(0, 0); MAX_LIVE_HITS],
            n_live: 0,
            silent: 0,
            midi,
            cv,
            own: &mut mix.instruments,
            modu: &mix.modu,
            pad_synths: &mut mix.pad_synths,
            tap,
            only: None,
            shifts: [mpfree_seq::Transpose::NONE; mpfree_seq::MAX_TRACKS],
            key: None,
            by_note: &mut [mpfree_arp::scale::Held::new(); 2],
        };
        time.period(PERIOD as u32, &mut seq);
        // MIDI out's period ends after the mixer's, as in the loop.
        seq.cv.period_end(PERIOD as u32, std::time::Instant::now());
        seq.tap.take();
        *clip_hits += seq.hits;
        time.clear_hits();
        // The clips' locks and lanes to the mod engine, as the loop does.
        for &(offset, addr, value) in time.locks() {
            mix.lock(offset, addr, value);
        }
        for &(offset, addr, value) in time.lanes() {
            mix.lane(offset, addr, value);
        }
        time.clear_locks();
        // The pads, each into its mixer channel, and on to the Drums track.
        player.set_tempo(time.info().bpm);
        // Each instrument's tuning, handed over when its pick changes (the
        // first period, here).
        let said = tunings.follow(app.tunings(), listed, synth, mix, Some(&mut *player));
        assert!(said.is_empty(), "12-TET at the start says nothing");
        // The period's deadline for the lanes, as the loop sets it after
        // each write; far, so a slow machine (qemu in CI) is never late.
        let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
        synth.start(i * PERIOD as u64, time.info().bpm);
        let mut pads = mix.pads();
        player.render_routed_until(PERIOD, &mut pads, until);
        let played = pads.played();
        let [drums_l, drums_r] = drums;
        mix.drums(played, drums_l, drums_r);
        let [synth_l, synth_r] = synth_out;
        synth_l.fill(0.0);
        synth_r.fill(0.0);
        synth.finish(until, synth_l, synth_r);
        let [pads_l, pads_r] = pads_out;
        for f in 0..PERIOD {
            pads_l[f] = drums_l[f] + synth_l[f];
            pads_r[f] = drums_r[f] + synth_r[f];
        }
        looper.take_pads(pads_l, pads_r);
        let [loop_l, loop_r] = loop_out;
        let note = looper.run(time.block(), loop_l, loop_r);
        assert!(note.is_none(), "period {i}: {note:?}");
        tracks.follow_latency(app.settings.loop_latency);
        tracks.period(
            time.block(),
            time.tempo_map(),
            time.song_on(),
            looper.input(),
            Some(&|p| time.audio(p)),
        );
        tracks.play_into(mix);
        mix.follow(looper.line.is_open(), looper.monitor);
        let ctx = mixing::ctx(RATE, i * PERIOD as u64, time.info().bpm, time.block());
        mix.mixer.set_lane_deadline(Some(until));
        mix.instruments.set_lane_deadline(Some(until));
        mix.pad_synths.set_lane_deadline(Some(until));
        let (p, _) = mix.period(
            &ctx,
            (synth_l, synth_r),
            (loop_l, loop_r),
            looper.input(),
            left,
            right,
        );
        // The tracks' controllers out, as the loop sends them, and into
        // an export's MIDI file.
        let period_frame = time.block().start_frame;
        for (offset, addr, value) in mix.modu.take_midi_values() {
            tap.control(period_frame + u64::from(offset), addr, value);
            midi.control(offset, addr, value);
        }
        midi.period_end(PERIOD as u32, std::time::Instant::now());
        if let Some(log) = sent {
            let start = i * PERIOD as u64;
            for (_, _, m) in midi.sent() {
                log.push((start, start + u64::from(m.offset), m.bytes().to_vec()));
            }
        }
        // Freeze and Print's check of the frozen strips, each period.
        let n_tracks = time.project().tracks.len();
        let said = mix.takes_period(&ctx, n_tracks, &|t| time.now_playing(t));
        assert!(said.is_none(), "period {i}: a take said something");
        recall.period(time.block(), &left[..], &right[..], mix.mixer.latency());
        let p = p.max(time.mix_click(left, right));
        self.peak = self.peak.max(p);
        // After the write, in the loop.
        synth.settle(until);
        mix.instruments.settle(until);
        mix.pad_synths.settle(until);
    }
}

/// A bar of pads recorded into a clip and the looper's 1-bar loop, then
/// half a minute of steady play (the clip, the arp on held pads, the loop) with
/// no allocation or free on the sound path.
fn steady_play_never_allocates(synth_on: bool, sound_cores: cores::SoundCores) {
    let mut rig = Rig::new(synth_on);
    // Before the sound starts, as player_session does (it allocates and
    // starts the lanes' threads).
    let mut c = cores::Cores::start_unpinned(sound_cores, &mut rig.player, &mut rig.synth);
    c.start_mixer(&mut rig.mix.mixer);
    c.start_tracks(&mut rig.mix.instruments);
    c.start_pads(&mut rig.mix.pad_synths);
    let lanes = matches!(
        sound_cores,
        cores::SoundCores::Two | cores::SoundCores::Three
    );
    if lanes {
        late_reverb(&mut rig.mix.mixer);
    }
    // Setup, which may allocate: a new clip on track 1, which starts
    // recording the pads, and the looper recording its first bar.
    // Recording is an edit: each finished note goes into the clip, and the
    // clips are compiled again (TimeHost::period).
    rig.time
        .act(TimeAction::NewClip { track: 0, slot: 0 })
        .unwrap();
    rig.looper.command(LoopCommand::SetBars(Some(1)));
    rig.looper.command(LoopCommand::Record);
    // 2 bars at 120 BPM: 4 s, about 1380 periods.
    let mut i = 0u64;
    while i < 1_500 {
        rig.pads(i);
        rig.period(i);
        i += 1;
    }
    // Punch out: the clip plays what was recorded.
    rig.time.command(Command::Record);
    for _ in 0..200 {
        rig.pads(i);
        rig.period(i);
        i += 1;
    }
    assert_eq!(rig.looper.looper.status().phase, Phase::Playing);
    assert!(rig.time.info().state.rolling());
    let (arp_before, clip_before) = (rig.arp_notes, rig.clip_hits);
    let ((), calls) = crate::sys::alloc_count::calls(|| {
        // About 35 s.
        for _ in 0..12_000 {
            rig.pads(i);
            rig.period(i);
            i += 1;
        }
    });
    assert_eq!(
        calls, 0,
        "allocations or frees on the sound path (synth on: {synth_on})"
    );
    // The arp, the clip and the sound all ran in the measured half minute.
    assert!(
        rig.arp_notes - arp_before > 500,
        "arp notes: {}",
        rig.arp_notes - arp_before
    );
    assert!(
        rig.clip_hits - clip_before > 100,
        "clip hits: {}",
        rig.clip_hits - clip_before
    );
    assert!(rig.peak > 0.01, "silent: peak {}", rig.peak);
    assert!(rig.peak.is_finite());
    // Sample Recall kept every period of the master.
    assert_eq!(rig.recall.written(), i * PERIOD as u64);
    // On two cores the lane took its half of the voices; on three the synth
    // played every period on its lane. (Late parts are allowed here: under
    // qemu or a busy machine a lane can be slow, and a late part allocates
    // nothing either.)
    let n = cores::Counts::read(&rig.player, &rig.mix, &rig.synth);
    if lanes {
        // The reverb ran a block late on its lane the whole time.
        assert!(n.returns_blocks > 12_000, "{n:?}");
        assert!(n.returns_starts >= 1, "{n:?}");
        assert_eq!(n.returns_lost, 0, "{n:?}");
    }
    match sound_cores {
        cores::SoundCores::Two if !synth_on => assert!(n.voice_parts > 100, "{n:?}"),
        cores::SoundCores::Two => {}
        cores::SoundCores::Three => assert_eq!(n.synth_periods, i, "{n:?}"),
        _ => assert_eq!(n, cores::Counts::default()),
    }
}

/// Track 1 with its own Poly (instruments.rs): the pads (through its arp)
/// and its clip play the Poly into its mixer channel (the Drums strip,
/// R3 part 4b), not the kit, and half a minute of it allocates and frees
/// nothing on the sound path.
#[test]
fn a_track_with_its_own_instrument_plays_it_and_never_allocates() {
    own_instrument(cores::SoundCores::Shared);
}

/// The same on two cores: the Poly plays on the tracks' lane on core 2.
#[test]
fn a_tracks_own_instrument_plays_on_another_core_and_never_allocates() {
    own_instrument(cores::SoundCores::Two);
}

fn own_instrument(sound_cores: cores::SoundCores) {
    let mut rig = Rig::new(false);
    let mut c = cores::Cores::start_unpinned(sound_cores, &mut rig.player, &mut rig.synth);
    c.start_mixer(&mut rig.mix.mixer);
    c.start_tracks(&mut rig.mix.instruments);
    c.start_pads(&mut rig.mix.pad_synths);
    // Setup, which may allocate: Poly made for track 1 (no builder here,
    // so made at once) and put in on the next period.
    let job = crate::instrument_job(&rig.synth, 0, "mpfree.poly", None);
    assert!(rig.mix.build_instrument(job));
    rig.time
        .act(TimeAction::NewClip { track: 0, slot: 0 })
        .unwrap();
    let mut i = 0u64;
    while i < 1_500 {
        rig.pads(i);
        rig.period(i);
        i += 1;
    }
    assert_eq!(rig.mix.instruments.held(0).map(|h| h.name), Some("Poly"));
    rig.time.command(Command::Record);
    for _ in 0..200 {
        rig.pads(i);
        rig.period(i);
        i += 1;
    }
    let (notes_before, sampler_before) = (rig.mix.instruments.stats(0).notes, rig.player.hits());
    let ((), calls) = crate::sys::alloc_count::calls(|| {
        for _ in 0..12_000 {
            rig.pads(i);
            rig.period(i);
            i += 1;
        }
    });
    assert_eq!(calls, 0, "allocations or frees on the sound path");
    // The clip's notes played the Poly. (This rig plays the live pads on
    // the kit itself, as the loop did before the per-track arps, so only
    // the clip reaches the track's instrument here; the live path is
    // `live_pads_on_a_track_with_its_own_instrument_play_it`.)
    let notes = rig.mix.instruments.stats(0).notes - notes_before;
    assert!(notes > 100, "the Poly played {notes} notes");
    assert_eq!(rig.mix.instruments.stats(0).dropped, 0);
    assert!(
        rig.player.hits() > sampler_before,
        "the live pads played the kit"
    );
    assert!(rig.peak > 0.01, "silent: peak {}", rig.peak);
    // Track 1's own instrument plays into the Drums strip (R3 part 4b),
    // which is named for it (step 4d); no own channel shows.
    rig.mix.show(Some(mpfree_mixer_ui::MixView::Tracks));
    let st = rig.mix.poll_state(std::time::Instant::now()).unwrap();
    let names: Vec<&str> = st.strips.iter().map(|v| v.name.as_str()).collect();
    assert_eq!(names, vec!["Poly \u{b7} Track 1", "Synth", "Loop"]);
    assert_eq!(st.strips[0].id, mpfree_fx::StripId::Track(mixing::DRUMS));
    assert!(st.strips[0].peak[0] > 0.0, "{:?}", st.strips[0].peak);
    // On two cores the lane played it every period (late ones allowed, as
    // above: they allocate nothing either).
    let n = cores::Counts::read(&rig.player, &rig.mix, &rig.synth);
    if sound_cores == cores::SoundCores::Two {
        assert_eq!(rig.mix.instruments.lanes(), 1);
        assert!(n.track_periods > 12_000, "{n:?}");
    } else {
        assert_eq!(n.track_periods, 0, "{n:?}");
    }
}

/// A send reverb that goes a block late on the returns lane (cores.rs):
/// a Cloud all wet with a pre-delay on Return B, and the Drums and Synth
/// tracks sent to it.
fn late_reverb(m: &mut mpfree_fx::Mixer) {
    use mpfree_fx::ids::{cloud, mixer as mx};
    use mpfree_fx::StripId;
    let c = m.chain_mut(StripId::Return(1)).unwrap();
    c.push(Box::new(mpfree_fx::Cloud::new()));
    c.set_param(0, cloud::MIX, 100.0);
    c.set_param(0, cloud::PRE_DELAY, 20.0);
    for t in [mixing::DRUMS, mixing::SYNTH] {
        m.set_now(StripId::Track(t), mx::send(1), 0.0);
    }
}

#[test]
fn the_sound_path_never_allocates_with_the_sampler() {
    steady_play_never_allocates(false, cores::SoundCores::Shared);
}

#[test]
fn the_sound_path_never_allocates_with_the_synth() {
    steady_play_never_allocates(true, cores::SoundCores::Shared);
}

#[test]
fn the_sound_path_never_allocates_on_two_cores() {
    steady_play_never_allocates(false, cores::SoundCores::Two);
}

#[test]
fn the_sound_path_never_allocates_on_three_cores_with_the_synth() {
    steady_play_never_allocates(true, cores::SoundCores::Three);
}

/// MIDI Out's port in these tests: every byte written.
#[derive(Clone, Default)]
struct MidiSink(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

impl std::io::Write for MidiSink {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// The clip's track set to play on MIDI Out, channel 10, with clock out on:
/// its recorded pads go out as the MPC's pad notes instead of to the pads,
/// every note-on gets its note-off, the clock joins and pulses, and the
/// sound path still never allocates or frees.
#[test]
fn a_track_on_midi_out_plays_there_and_never_allocates() {
    track_on_midi_out_never_allocates(false, 0);
}

/// The same with the track and the clock on a USB device's output: its
/// own writer, and the jacks get nothing.
#[test]
fn a_track_on_a_usb_device_plays_there_and_never_allocates() {
    track_on_midi_out_never_allocates(true, 0);
}

/// The same on gear slower than the sound's lead (sequencer S5): the
/// track's clip plays ahead, a copy of the transport and the player
/// forecasting each period, and still nothing allocates and every note
/// ends once.
#[test]
fn a_track_on_slow_gear_plays_ahead_and_never_allocates() {
    track_on_midi_out_never_allocates(false, 400);
}

/// `delay_tenths`: the gear's delay on the port's page, in tenths of a ms.
fn track_on_midi_out_never_allocates(usb: bool, delay_tenths: u16) {
    use mpfree_midi::setup::{ClockOut, TrackOut, DIN_NAME};
    let mut rig = Rig::new(false);
    let mut c =
        cores::Cores::start_unpinned(cores::SoundCores::Shared, &mut rig.player, &mut rig.synth);
    c.start_mixer(&mut rig.mix.mixer);
    c.start_tracks(&mut rig.mix.instruments);
    c.start_pads(&mut rig.mix.pad_synths);
    rig.time
        .act(TimeAction::NewClip { track: 0, slot: 0 })
        .unwrap();
    let mut i = 0u64;
    while i < 1_500 {
        rig.pads(i);
        rig.period(i);
        i += 1;
    }
    rig.time.command(Command::Record);
    // Setup, which may allocate: MIDI Out opens and the track goes there.
    let din = MidiSink::default();
    rig.midi
        .open(din.clone(), std::time::Instant::now())
        .unwrap();
    let usb_sink = MidiSink::default();
    let port = if usb {
        let s = usb_sink.clone();
        rig.midi.plugged(
            mpfree_midi::PortId(1),
            "Digitone",
            std::sync::Arc::new(move || Ok(Box::new(s.clone()) as Box<dyn std::io::Write + Send>)),
        );
        "Digitone"
    } else {
        DIN_NAME
    };
    let sink = if usb { usb_sink } else { din.clone() };
    {
        let mut m = rig.io.lock().unwrap();
        m.setup.tracks[0] = Some(TrackOut::new(port, 9));
        m.setup.set_port(port, |p| {
            p.clock_out = ClockOut::WhilePlaying;
            p.delay_tenths = delay_tenths;
        });
        m.gen += 1;
    }
    // An LFO on the track's CC 74 (its outside gear's brightness), set up
    // first as the chain editor would: its values go out with the notes.
    let cc =
        mpfree_play::mpfree_param::ParamAddr::new(mpfree_play::mpfree_param::Owner::Midi(0), 74);
    let row = mpfree_mod::Row {
        source: mpfree_mod::Source::Lfo(0),
        via: None,
        target: cc,
        depth: 0.5,
    };
    assert!(rig
        .mix
        .modu
        .change(&rig.mix.mixer, mpfree_mod::Change::AddRow(row))
        .is_ok());
    for _ in 0..200 {
        rig.pads(i);
        rig.period(i);
        i += 1;
    }
    assert!(rig.midi.takes(0));
    let clip_before = rig.clip_hits;
    let ((), calls) = crate::sys::alloc_count::calls(|| {
        for _ in 0..4_000 {
            rig.pads(i);
            rig.period(i);
            i += 1;
        }
    });
    assert_eq!(calls, 0, "allocations or frees on the sound path");
    assert_eq!(
        rig.clip_hits, clip_before,
        "the clip's notes reached the pads"
    );
    let ahead = rig.time.ahead_counts();
    if delay_tenths > 0 {
        assert!(ahead.ahead > 20, "{ahead:?}");
        assert_eq!(ahead.dropped, 0, "{ahead:?}");
    } else {
        assert_eq!(ahead, Default::default());
    }
    // Closing ends whatever is still on.
    assert!(rig.midi.close());
    let bytes = sink.0.lock().unwrap().clone();
    // Split into messages: one byte for real-time ones, three otherwise.
    let mut msgs: Vec<&[u8]> = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let n = if bytes[at] >= 0xF8 { 1 } else { 3 };
        msgs.push(&bytes[at..(at + n).min(bytes.len())]);
        at += n;
    }
    let count = |status: u8| {
        msgs.iter()
            .filter(|m| m[0] == status && (m.len() == 1 || PAD_NOTES[..4].contains(&m[1])))
            .count()
    };
    let ons = count(0x99);
    assert!(ons > 20, "note-ons on channel 10: {ons}");
    assert_eq!(count(0x89), ons, "every note-on ends");
    // The clock joined with Song Position and Continue, pulsed throughout
    // (4,200 periods at 120 BPM is about 590 pulses) and stopped at close.
    assert_eq!(msgs.iter().filter(|m| m[0] == 0xF2).count(), 1);
    assert_eq!(count(0xFB), 1);
    let pulses = count(0xF8);
    assert!((550..=620).contains(&pulses), "pulses: {pulses}");
    assert_eq!(count(0xFC), 1);
    // The LFO's CC 74 on channel 10, at most once a period (4,200).
    let ccs = msgs.iter().filter(|m| m.starts_with(&[0xB9, 74])).count();
    assert!((20..=4_200).contains(&ccs), "CC 74: {ccs}");
    assert_eq!(
        msgs.len(),
        2 * ons + pulses + 3 + ccs,
        "only notes, clock and the CC went out: {bytes:02X?}"
    );
    if usb {
        assert!(din.0.lock().unwrap().is_empty(), "the jacks got none of it");
    }
}

/// The keys' wheel and pressure on a track that plays on MIDI Out, while
/// it records (the MIDI plan's 3.7): what the gear is sent goes through
/// `ExprRec`'s thinning to `TimeHost::gesture` each period, as the main
/// loop does, without allocating or freeing; let go, the wheel is a lane
/// of the clip with far fewer points than the values it was sent.
#[test]
fn the_keys_wheel_on_a_midi_out_track_records_and_never_allocates() {
    use mpfree_midi::expr_out::In;
    use mpfree_midi::setup::{TrackOut, DIN_NAME};
    use mpfree_play::mpfree_param::{midi, Owner, ParamAddr};
    use mpfree_seq::mpfree_moves::Target;
    let mut rig = Rig::new(false);
    let mut c =
        cores::Cores::start_unpinned(cores::SoundCores::Shared, &mut rig.player, &mut rig.synth);
    c.start_mixer(&mut rig.mix.mixer);
    rig.time.command(Command::SetLoop(Some((0, 4 * 3840))));
    rig.time
        .act(TimeAction::NewClip { track: 0, slot: 0 })
        .unwrap();
    let din = MidiSink::default();
    rig.midi
        .open(din.clone(), std::time::Instant::now())
        .unwrap();
    {
        let mut m = rig.io.lock().unwrap();
        m.setup.tracks[0] = Some(TrackOut::new(DIN_NAME, 0));
        m.gen += 1;
    }
    let mut i = 0u64;
    for _ in 0..200 {
        rig.period(i);
        i += 1;
    }
    assert!(rig.midi.takes(0));
    assert_eq!(rig.time.info().state, mpfree_transport::State::Recording);
    let mut rec = crate::expr_rec::ExprRec::new();
    let mut sent = 0u32;
    let mut step = |rig: &mut Rig, rec: &mut crate::expr_rec::ExprRec, i: u64, wheel: f32| {
        // Four wheel values a period, as a 14-bit wheel can send (the
        // last of them where it rests).
        for k in 0..4 {
            let w = wheel + (3 - k) as f32 * 0.0005;
            assert!(rig.midi.expression(0, In::Wheel(w)));
        }
        let (bend, pressure) = rig.midi.expression_sent(0).unwrap();
        rec.sent(bend, pressure);
        let t = i as f64 * PERIOD as f64 / f64::from(RATE);
        rec.period(Some(0), true, t, |input| rig.time.gesture(0, input));
        rig.period(i);
        sent += 4;
    };
    let ((), calls) = crate::sys::alloc_count::calls(|| {
        for n in 0..600 {
            let wheel = (n as f32 * 0.01).sin() * 0.9;
            step(&mut rig, &mut rec, i, wheel);
            i += 1;
        }
    });
    assert_eq!(calls, 0, "allocations or frees on the sound path");
    // Let go: the wheel back in the middle ends the gesture, and the clip
    // has its lane.
    for _ in 0..100 {
        step(&mut rig, &mut rec, i, 0.0);
        i += 1;
    }
    let bend = ParamAddr::new(Owner::Midi(0), midi::BEND);
    let points = rig
        .time
        .project()
        .clip(0, 0)
        .and_then(|c| c.moves.lane(Target::for_track(bend, 0)))
        .map(|l| l.points.len());
    assert!(
        points.is_some_and(|n| n >= 20 && n < sent as usize / 4),
        "{points:?} points for {sent} values"
    );
    assert!(rig.midi.close());
    assert!(
        din.0.lock().unwrap().windows(1).any(|b| b[0] == 0xE0),
        "the wheel went out on channel 1"
    );
}

/// What a track on gear `delay_tenths` slow (its port page's delay, in
/// tenths of a ms; the rig's lead is 0, so all of it is ahead) sends over
/// two bars of a one-bar clip: a note on step 1 (pad 1) and step 5 (pad
/// 2), a lock of CC 74 at 100 on step 5, and, with `lane`, a move lane on
/// CC 1 that ramps from 0 to 127 over beat 3 and ends on beat 4's second
/// eighth. With `lfo`, an LFO row moves CC 74 too. Each message as (the
/// first frame of the period it was handed over in, its frame, bytes).
fn slow_gear_moves(delay_tenths: u16, lane: bool, lfo: bool) -> Vec<(u64, u64, Vec<u8>)> {
    // A debug build copies the rig (some hundreds of KB) through each frame
    // that builds it, more than a test thread's 2 MiB holds here; the
    // device builds it once, optimised, on the main thread.
    std::thread::Builder::new()
        .name("slow gear".into())
        .stack_size(16 << 20)
        .spawn(move || slow_gear_moves_on(delay_tenths, lane, lfo))
        .expect("spawn the slow gear rig")
        .join()
        .expect("the slow gear rig ran")
}

fn slow_gear_moves_on(delay_tenths: u16, lane: bool, lfo: bool) -> Vec<(u64, u64, Vec<u8>)> {
    use mpfree_midi::setup::{TrackOut, DIN_NAME};
    use mpfree_play::mpfree_param::{OnTrack, Owner, ParamAddr, RelAddr};
    use mpfree_seq::mpfree_moves::{Kind, Map, Steps, Target};
    use mpfree_seq::{Clip, Cond, Join, Lock, Note, Project, Touch, Track};
    let step = 240;
    let mut clip = Clip::new("c", 16 * step);
    clip.notes = [(0, 0u8), (4, 1)]
        .iter()
        .map(|&(k, pad)| Note {
            tick: k * step,
            len: step - 40,
            note: pad,
            velocity: 100,
            chance: 100,
            cond: Cond::Always,
            ratchet: 1,
            touch: Touch::NONE,
            on: true,
            ramp: 0,
            join: Join::Attack,
            group: 0,
        })
        .collect();
    clip.locks = vec![Lock::new(
        4 * step,
        None,
        RelAddr::new(OnTrack::Midi, 74),
        100.0,
    )];
    if lane {
        let map = Map {
            min: 0.0,
            max: 127.0,
            log: false,
            steps: Steps::None,
        };
        clip.moves
            .put(
                Target::Track(RelAddr::new(OnTrack::Midi, 1)),
                map,
                &[
                    (f64::from(8 * step as u32), 0.0, Kind::Ramp),
                    (f64::from(12 * step as u32), 1.0, Kind::Step),
                    (f64::from(14 * step as u32), 0.0, Kind::End),
                ],
                16 * step as u32,
            )
            .unwrap();
    }
    let mut project = Project::new();
    let mut track = Track::new("Track 1", 0);
    track.slots[0] = Some(clip);
    project.tracks.push(track);
    let mut rig = Rig::new(false);
    rig.time = time::TimeHost::with_project(RATE, 120.0, project).unwrap();
    let din = MidiSink::default();
    rig.midi
        .open(din.clone(), std::time::Instant::now())
        .unwrap();
    {
        let mut m = rig.io.lock().unwrap();
        m.setup.tracks[0] = Some(TrackOut::new(DIN_NAME, 9));
        m.setup
            .set_port(DIN_NAME, |p| p.delay_tenths = delay_tenths);
        m.gen += 1;
    }
    if lfo {
        let row = mpfree_mod::Row {
            source: mpfree_mod::Source::Lfo(0),
            via: None,
            target: ParamAddr::new(Owner::Midi(0), 74),
            depth: 0.25,
        };
        assert!(rig
            .mix
            .modu
            .change(&rig.mix.mixer, mpfree_mod::Change::AddRow(row))
            .is_ok());
    }
    let mut i = 0u64;
    rig.period(i);
    i += 1;
    rig.time
        .act(TimeAction::Launch { track: 0, slot: 0 })
        .unwrap();
    rig.time.command(Command::Play);
    rig.sent = Some(Vec::with_capacity(4_096));
    // Two bars at 120 BPM.
    let bars = 2 * 2 * u64::from(RATE);
    while i * (PERIOD as u64) < bars {
        rig.period(i);
        i += 1;
    }
    assert!(rig.midi.takes(0));
    rig.sent.take().unwrap()
}

/// The step 5 note-ons (pad 2) of [`slow_gear_moves`]' log.
fn step_5_notes(log: &[(u64, u64, Vec<u8>)]) -> Vec<(u64, u64)> {
    log.iter()
        .filter(|m| m.2[..2] == [0x99, PAD_NOTES[1]] && m.2[2] > 0)
        .map(|m| (m.0, m.1))
        .collect()
}

/// The messages of controller `cc` on channel 10.
fn ccs(log: &[(u64, u64, Vec<u8>)], cc: u8) -> Vec<(u64, u64, u8)> {
    log.iter()
        .filter(|m| m.2[..2] == [0xB9, cc])
        .map(|m| (m.0, m.1, m.2[2]))
        .collect()
}

/// 90 ms at the rig's rate, in frames.
const NINETY_MS: u64 = RATE as u64 * 90 / 1000;

/// Sequencer S5's gap closed: a track 90 ms ahead with a CC 74 lock on
/// step 5. The lock's CC goes out with step 5's note, handed over in the
/// same period, the same ahead of its frame, and once only; so does its
/// end, back to the control's own value (64).
#[test]
fn a_lock_on_slow_gear_goes_ahead_with_its_note_once() {
    let log = slow_gear_moves(900, false, false);
    let notes = step_5_notes(&log);
    assert_eq!(notes.len(), 2, "two passes: {log:?}");
    let cc = ccs(&log, 74);
    let locks: Vec<_> = cc.iter().filter(|c| c.2 == 100).collect();
    assert_eq!(locks.len(), 2, "once a pass: {cc:?}");
    for (&(handed, frame), lock) in notes.iter().zip(&locks) {
        assert_eq!((lock.0, lock.1), (handed, frame), "with its note: {cc:?}");
        assert!(
            handed <= frame - NINETY_MS && frame - NINETY_MS < handed + PERIOD as u64,
            "ahead: handed at {handed}, plays at {frame}"
        );
        // The lock's CC leaves before the note on its frame.
        let at = |b: &[u8]| log.iter().position(|m| m.1 == frame && m.2 == b);
        assert!(at(&[0xB9, 74, 100]) < at(&[0x99, PAD_NOTES[1], 100]));
    }
    // Its end: CC 74 back to 64, ahead too, once each pass.
    let ends: Vec<_> = cc.iter().filter(|c| c.2 == 64).collect();
    assert_eq!(ends.len(), 2, "{cc:?}");
    for e in ends {
        assert!(e.1 > e.0 + NINETY_MS - PERIOD as u64, "ahead: {e:?}");
    }
    assert_eq!(cc.len(), 4, "nothing more: {cc:?}");
}

/// The same track with a move lane on CC 1: its values go out ahead, each
/// in the period that holds its frame less the ahead, at most two a
/// period, never a value twice in a row, up to 127, then its end (0).
#[test]
fn a_lane_on_slow_gear_goes_ahead_too() {
    let log = slow_gear_moves(900, true, false);
    let cc = ccs(&log, 1);
    assert!(cc.len() > 20, "{cc:?}");
    for c in &cc {
        assert!(
            c.0 <= c.1 - NINETY_MS && c.1 - NINETY_MS < c.0 + PERIOD as u64,
            "ahead: {c:?} in {cc:?}"
        );
    }
    for w in cc.windows(3) {
        assert!(w[2].0 > w[0].0, "at most two a period: {w:?}");
    }
    let values: Vec<u8> = cc.iter().map(|c| c.2).collect();
    // Each pass: the ramp's first value (0), up a value or two at a time to
    // 127, never one twice, then its end, back to the control's own (0).
    let top = values.iter().position(|&v| v == 127).unwrap();
    assert!(values[..=top].windows(2).all(|w| w[0] < w[1]), "{values:?}");
    assert_eq!(values[top + 1], 0, "its end: {values:?}");
    assert_eq!(
        values[..top + 2],
        values[top + 2..],
        "the second pass the same"
    );
    // The lock on the same track still goes ahead, once a pass.
    assert_eq!(ccs(&log, 74).iter().filter(|c| c.2 == 100).count(), 2);
}

/// A track that isn't ahead is as it was: its lock goes out on its note's
/// frame in that frame's own period, from the mod engine.
#[test]
fn a_lock_on_gear_that_isnt_slow_goes_in_its_period() {
    let log = slow_gear_moves(0, false, false);
    let notes = step_5_notes(&log);
    assert_eq!(notes.len(), 2);
    let cc = ccs(&log, 74);
    let locks: Vec<_> = cc.iter().filter(|c| c.2 == 100).collect();
    assert_eq!(locks.len(), 2, "{cc:?}");
    for (&(handed, frame), lock) in notes.iter().zip(&locks) {
        assert!(frame < handed + PERIOD as u64, "in its period");
        assert_eq!((lock.0, lock.1), (handed, frame));
    }
}

/// A CC an LFO row moves stays with the mod engine, a period at a time,
/// though its track plays ahead (the engine's value mixes the lock and
/// the LFO): nothing of it goes ahead, and the track's notes still do.
#[test]
fn a_control_an_lfo_moves_stays_a_period_at_a_time() {
    let log = slow_gear_moves(900, false, true);
    let cc = ccs(&log, 74);
    assert!(cc.len() > 20, "{cc:?}");
    for c in &cc {
        assert!(c.1 < c.0 + PERIOD as u64, "in its own period: {c:?}");
    }
    for (handed, frame) in step_5_notes(&log) {
        assert!(
            handed <= frame - NINETY_MS && frame - NINETY_MS < handed + PERIOD as u64,
            "the notes go ahead"
        );
    }
}

/// An arp's note (the pads played live) on a track that plays on MIDI Out:
/// it goes out with the pads' velocity, the pads don't sound it, the pad
/// still shows, and the armed track records the pad as heard.
#[test]
fn live_notes_on_a_midi_out_track_go_out_and_record_as_their_pad() {
    use mpfree_midi::setup::{TrackOut, DIN_NAME};
    use mpfree_play::{Event, Out};
    let mut rig = Rig::new(false);
    let sink = MidiSink::default();
    rig.midi
        .open(sink.clone(), std::time::Instant::now())
        .unwrap();
    rig.io.lock().unwrap().setup.tracks[0] = Some(TrackOut::new(DIN_NAME, 0));
    rig.io.lock().unwrap().gen += 1;
    rig.midi.sync(&rig.io, false);
    let live = rig.app.live_map();
    let v = live.velocity(100);
    let mut seq = SeqPads {
        player: &mut rig.player,
        synth: &mut rig.synth,
        kits: &rig.track_kits,
        shown: 0,
        flashes: [0; 128],
        hits: 0,
        live,
        live_hits: [(0, 0); MAX_LIVE_HITS],
        n_live: 0,
        silent: 0,
        midi: &mut rig.midi,
        cv: &mut rig.cv,
        own: &mut rig.mix.instruments,
        modu: &rig.mix.modu,
        pad_synths: &mut rig.mix.pad_synths,
        tap: &mut rig.tap,
        only: None,
        shifts: [mpfree_seq::Transpose::NONE; mpfree_seq::MAX_TRACKS],
        key: None,
        by_note: &mut [mpfree_arp::scale::Held::new(); 2],
    };
    let at = |event| Out {
        offset: 0,
        frame: 0,
        track: 0,
        dest: 0,
        event,
        notes: false,
    };
    let on = Event::NoteOn {
        note: 2,
        velocity: 100,
    };
    let heard = time::Pads::live(&mut seq, &at(on));
    assert_eq!(
        heard.pad,
        Some(Event::NoteOn {
            note: 2,
            velocity: v
        })
    );
    let off = Event::NoteOff { note: 2 };
    assert_eq!(time::Pads::live(&mut seq, &at(off)).pad, Some(off));
    assert_eq!((seq.hits, seq.silent), (0, 0), "the pads sounded it");
    assert_eq!(&seq.live_hits[..seq.n_live], [(2, v)]);
    // Another track's notes still play on the pads.
    let other = Out { track: 1, ..at(on) };
    assert!(!time::Pads::out(&mut seq, &other));
    seq.midi
        .period_end(PERIOD as u32, std::time::Instant::now());
    assert!(rig.midi.close());
    assert_eq!(
        *sink.0.lock().unwrap(),
        [0x90, PAD_NOTES[2], v, 0x80, PAD_NOTES[2], 64]
    );
}

/// Live transpose (sequencer S4): where a track's pads become notes they
/// move by its shift, a clip's on MIDI Out and on the synth's pads, and a
/// note already sounding ends where it started when the shift changes.
/// An arp's notes by number keep what each became the same way.
#[test]
fn a_tracks_transpose_moves_its_notes_and_a_sounding_note_ends_where_it_went() {
    use mpfree_arp::scale::{Held, Shift};
    use mpfree_midi::setup::{TrackOut, DIN_NAME};
    use mpfree_play::{Event, Out};
    use mpfree_seq::{Transpose, MAX_TRACKS};
    let mut rig = Rig::new(true);
    let sink = MidiSink::default();
    rig.midi
        .open(sink.clone(), std::time::Instant::now())
        .unwrap();
    rig.io.lock().unwrap().setup.tracks[0] = Some(TrackOut::new(DIN_NAME, 0));
    rig.io.lock().unwrap().gen += 1;
    rig.midi.sync(&rig.io, false);
    let pad1 = rig.synth.notes(0)[0];
    let mut shifts = [Transpose::NONE; MAX_TRACKS];
    shifts[0] = Transpose::semitones(5);
    shifts[1] = Transpose::semitones(-2);
    let mut by_note = [Held::new(); 2];
    let mut seq = SeqPads {
        player: &mut rig.player,
        synth: &mut rig.synth,
        kits: &rig.track_kits,
        shown: 0,
        flashes: [0; 128],
        hits: 0,
        live: rig.app.live_map(),
        live_hits: [(0, 0); MAX_LIVE_HITS],
        n_live: 0,
        silent: 0,
        midi: &mut rig.midi,
        cv: &mut rig.cv,
        own: &mut rig.mix.instruments,
        modu: &rig.mix.modu,
        pad_synths: &mut rig.mix.pad_synths,
        tap: &mut rig.tap,
        only: None,
        shifts,
        key: None,
        by_note: &mut by_note,
    };
    let at = |track, event| Out {
        offset: 0,
        frame: 0,
        track,
        dest: track,
        event,
        notes: false,
    };
    let on = Event::NoteOn {
        note: 0,
        velocity: 100,
    };
    let off = Event::NoteOff { note: 0 };
    // Track 1 plays on MIDI Out: pad 1's note, up 5. The shift goes back
    // to 0 while it sounds, and its end still ends the note it started.
    assert!(time::Pads::out(&mut seq, &at(0, on)));
    seq.shifts[0] = Transpose::NONE;
    assert!(time::Pads::out(&mut seq, &at(0, off)));
    // Track 2 plays the synth: the pads take its notes, its pad's note
    // starts moved down 2 and its note-off ends what the pad started.
    assert!(!time::Pads::out(&mut seq, &at(1, on)));
    assert!(time::Pads::hit_at(&mut seq, 1, 0, 100, 0));
    assert!(!time::Pads::out(&mut seq, &at(1, off)));
    seq.shifts[1] = Transpose::NONE;
    time::Pads::release_at(&mut seq, 1, 0, 4);
    // As written, track 3 plays the synth's pads as before.
    assert!(time::Pads::hit_at(&mut seq, 2, 1, 100, 8));
    let pad2 = seq.synth.notes(1)[0];
    let started: Vec<_> = seq
        .synth
        .queued()
        .into_iter()
        .filter_map(|e| match e {
            mpfree_node::NodeEvent::NoteOn { note, .. } => Some((note, true)),
            mpfree_node::NodeEvent::NoteOff { note } => Some((note, false)),
            _ => None,
        })
        .collect();
    assert_eq!(
        started,
        [(pad1 - 2, true), (pad1 - 2, false), (pad2, true)],
        "track 2 down 2, ended where it went; track 3 as written"
    );
    // An arp's note by number: where the shift took it when it started.
    let up = Shift::semitones(7);
    let note = |n| Event::NoteOn {
        note: n,
        velocity: 90,
    };
    let sounding = |o: Option<Out>| o.map(|o| o.event);
    assert_eq!(
        sounding(seq.sounding(&at(1, note(60)), &up)),
        Some(note(67))
    );
    assert_eq!(
        sounding(seq.sounding(&at(1, Event::NoteOff { note: 60 }), &Shift::NONE)),
        Some(Event::NoteOff { note: 67 })
    );
    assert_eq!(
        seq.sounding(&at(1, Event::NoteOff { note: 60 }), &Shift::NONE),
        None,
        "ended already"
    );
    assert_eq!(sounding(seq.sounding(&at(1, note(125)), &up)), None);
    seq.midi
        .period_end(PERIOD as u32, std::time::Instant::now());
    assert!(rig.midi.close());
    assert_eq!(
        *sink.0.lock().unwrap(),
        [0x90, pad1 + 5, 100, 0x80, pad1 + 5, 64]
    );
}

/// A key a route sends to a track that isn't armed (the I/O view's
/// Routing) plays that track at once, where its clips play: out of MIDI
/// out on its channel, on its own instrument, or the kit's pad for the
/// MPC's pad note. CAPTURE's history takes each as the pad that plays it
/// there. None of it allocates.
#[test]
fn routed_keys_play_their_track_where_its_clips_play() {
    use crate::midi_in::Key;
    use mpfree_midi::setup::{TrackOut, DIN_NAME};
    let mut rig = Rig::new(false);
    let sink = MidiSink::default();
    rig.midi
        .open(sink.clone(), std::time::Instant::now())
        .unwrap();
    rig.io.lock().unwrap().setup.tracks[3] = Some(TrackOut::new(DIN_NAME, 1));
    rig.io.lock().unwrap().gen += 1;
    rig.midi.sync(&rig.io, false);
    let job = crate::instrument_job(&rig.synth, 2, "mpfree.poly", None);
    assert!(rig.mix.build_instrument(job));
    assert!(matches!(
        rig.mix.poll_instruments(),
        Some(mixing::Landed::Playing { track: 2, .. })
    ));
    let (before, notes_before) = (rig.player.hits(), rig.mix.instruments.stats(2).notes);
    let down = |note| Key::Down {
        note,
        velocity: 100,
    };
    let up = |note| Key::Up { note };
    let keys = [
        (3, down(60)),
        (3, up(60)),
        (2, down(64)),
        (2, up(64)),
        (5, down(PAD_NOTES[0])),
        (
            5,
            Key::Pressure {
                note: PAD_NOTES[0],
                value: 50,
            },
        ),
        (5, up(PAD_NOTES[0])),
    ];
    let mut routed_pads = crate::midi_in::RoutedPads::new();
    let ((), calls) = crate::sys::alloc_count::calls(|| {
        for (track, key) in keys {
            // As the sound loop does: CAPTURE's history first, with the
            // synth's pad layout for track 4 (as if it played the synth).
            let t = usize::from(track);
            let synth_has = (t == 3).then_some(&*rig.synth);
            let kept = routed_pads.event(track, key, |n| {
                crate::midi_file::pad_on_track(&rig.mix.instruments, synth_has, t, n)
            });
            if let Some(ev) = kept {
                rig.time.routed_played(track, 0, ev);
            }
            crate::play_routed(
                track,
                key,
                &mut rig.midi,
                &mut rig.cv,
                &mut rig.mix.instruments,
                &mut rig.mix.pad_synths,
                &mut rig.synth,
                false,
                &mut rig.player,
                0,
            );
        }
        rig.midi
            .period_end(PERIOD as u32, std::time::Instant::now());
    });
    assert_eq!(calls, 0, "allocations or frees on the sound path");
    // The sounds play in the next period.
    rig.period(0);
    assert_eq!(rig.player.hits(), before + 1, "only track 6 played the kit");
    assert_eq!(rig.mix.instruments.stats(2).notes, notes_before + 1);
    assert!(rig.time.state().capture.is_some(), "CAPTURE has the keys");
    assert!(rig.midi.close());
    assert_eq!(*sink.0.lock().unwrap(), [0x91, 60, 100, 0x81, 60, 64]);
}

/// Sampler step 15: a route's keys reach a kit as its Keys play sends
/// them. On a kit of keygroups only they play by pitch, in the keygroup's
/// range, and a key outside it plays nothing, not the drum pad that sends
/// it; nothing on the sound path allocates.
#[test]
fn routed_keys_play_a_kits_keygroups_by_pitch() {
    use crate::midi_in::Key;
    use mpfree_sampler::model::Keys;
    let mut rig = Rig::new(false);
    let mut projects = rig.player.take_projects();
    let pads: Vec<usize> = (0..128).filter(|&p| rig.player.has_sound(0, p)).collect();
    let n = pads.len() as u64;
    assert!(n > 0);
    for p in pads {
        projects[0]
            .edit_pad(p, None, |pad| {
                pad.keys = Some(Keys {
                    lo: 48,
                    hi: 72,
                    root: 60,
                })
            })
            .unwrap();
    }
    let mut swap = Box::new(crate::sampler::ViewSwap {
        kit: 0,
        view: Some(mpfree_sampler::engine::EngineView::build(
            &projects[0],
            RATE,
        )),
        routes: crate::sampler::pad_routes(&projects[0]),
    });
    assert!(rig.player.swap_view(&mut swap));
    assert_eq!(
        rig.player.kit_keys(0).play,
        mpfree_sampler::model::KeysPlay::Keygroups,
        "a kit of keygroups only"
    );
    let before = rig.player.hits();
    let keys = [
        Key::Down {
            note: 64,
            velocity: 100,
        },
        Key::Up { note: 64 },
        // The first pad's own note, below the keygroups.
        Key::Down {
            note: PAD_NOTES[0],
            velocity: 100,
        },
        Key::Up { note: PAD_NOTES[0] },
    ];
    let ((), calls) = crate::sys::alloc_count::calls(|| {
        for key in keys {
            crate::play_routed(
                5,
                key,
                &mut rig.midi,
                &mut rig.cv,
                &mut rig.mix.instruments,
                &mut rig.mix.pad_synths,
                &mut rig.synth,
                false,
                &mut rig.player,
                0,
            );
        }
    });
    assert_eq!(calls, 0, "allocations or frees on the sound path");
    rig.period(0);
    // Every pad holds E3, so each plays it once; none holds the pad note
    // below them.
    assert_eq!(
        rig.player.hits() - before,
        n,
        "E3 on every keygroup, nothing else"
    );
    assert!(!rig.player.kit_keys(0).holds(PAD_NOTES[0]));
}

/// What thru passes on (the I/O view's Routing: a keyboard's channel 10
/// straight out of the 5-pin jacks) goes out as it came, and a panic ends
/// the note still on; handing it to MIDI out and the period's end never
/// allocate. Reading the keyboard is left out, as it is for every test
/// here.
#[test]
fn thru_passes_a_keyboards_channel_out_as_it_came() {
    // As slow_gear_moves: a debug build copies the rig through each frame
    // that builds it, and this test's left under 8 KB of a test thread's
    // 2 MiB even before Tempo follows (measured on main 49ef235a).
    std::thread::Builder::new()
        .name("thru".into())
        .stack_size(16 << 20)
        .spawn(thru_passes_a_keyboards_channel_out_on)
        .expect("spawn the thru rig")
        .join()
        .expect("the thru rig ran");
}

fn thru_passes_a_keyboards_channel_out_on() {
    use mpfree_midi::setup::{ThruRoute, DIN_NAME};
    let rig = Rig::new(false);
    let mut midi = rig.midi;
    let sink = MidiSink::default();
    midi.open(sink.clone(), std::time::Instant::now()).unwrap();
    {
        let mut v = rig.io.lock().unwrap();
        v.setup.thru = vec![ThruRoute {
            from: "Keystep 37".to_string(),
            channel: Some(9),
            to: DIN_NAME.to_string(),
            blocks: mpfree_midi::blocks::Blocks::NONE,
        }];
        v.gen += 1;
    }
    let mut music = midi_in::MusicIn::new();
    let keys_port = music.plugged("Keystep 37").unwrap();
    music.sync(&rig.io, 0.0);
    let mut keys = Vec::new();
    music.bytes(
        keys_port,
        &[0x99, 36, 100, 0x99, 38, 90, 0x89, 38, 0, 0xB9, 1, 64],
        0.1,
        &mut keys,
        &mut |_| {},
    );
    assert!(keys.is_empty(), "{keys:?}");
    let ((), calls) = crate::sys::alloc_count::calls(|| {
        for p in music.take_thru() {
            assert!(midi.thru(p.from, p.to, p.msg));
        }
        midi.period_end(PERIOD as u32, std::time::Instant::now());
        midi.all_off(0);
        midi.period_end(PERIOD as u32, std::time::Instant::now());
    });
    assert_eq!(calls, 0, "allocations or frees on the sound path");
    assert!(midi.close());
    assert_eq!(
        *sink.0.lock().unwrap(),
        [0x99, 36, 100, 0x99, 38, 90, 0x89, 38, 0, 0xB9, 1, 64, 0x89, 36, 64]
    );
}

/// The pads played live (through track 1's arp) on track 1 with its own
/// instrument: they play it, not the kit, record as their pad, and
/// another track's clip still plays the kit.
#[test]
fn live_pads_on_a_track_with_its_own_instrument_play_it() {
    use mpfree_play::{Event, Out};
    let mut rig = Rig::new(false);
    let job = crate::instrument_job(&rig.synth, 0, "mpfree.poly", None);
    assert!(rig.mix.build_instrument(job));
    assert!(matches!(
        rig.mix.poll_instruments(),
        Some(mixing::Landed::Playing {
            track: 0,
            name: "Poly"
        })
    ));
    let live = rig.app.live_map();
    let v = live.velocity(100);
    let before = rig.player.hits();
    let mut seq = SeqPads {
        player: &mut rig.player,
        synth: &mut rig.synth,
        kits: &rig.track_kits,
        shown: 0,
        flashes: [0; 128],
        hits: 0,
        live,
        live_hits: [(0, 0); MAX_LIVE_HITS],
        n_live: 0,
        silent: 0,
        midi: &mut rig.midi,
        cv: &mut rig.cv,
        own: &mut rig.mix.instruments,
        modu: &rig.mix.modu,
        pad_synths: &mut rig.mix.pad_synths,
        tap: &mut rig.tap,
        only: None,
        shifts: [mpfree_seq::Transpose::NONE; mpfree_seq::MAX_TRACKS],
        key: None,
        by_note: &mut [mpfree_arp::scale::Held::new(); 2],
    };
    let at = |track, event| Out {
        offset: 0,
        frame: 0,
        track,
        dest: track,
        event,
        notes: false,
    };
    let on = Event::NoteOn {
        note: 2,
        velocity: 100,
    };
    assert_eq!(
        time::Pads::live(&mut seq, &at(0, on)).pad,
        Some(Event::NoteOn {
            note: 2,
            velocity: v
        })
    );
    assert_eq!(&seq.live_hits[..seq.n_live], [(2, v)]);
    let off = Event::NoteOff { note: 2 };
    assert_eq!(time::Pads::live(&mut seq, &at(0, off)).pad, Some(off));
    // A clip on track 1 is taken by its instrument; one on track 2 isn't,
    // and goes on to the kit.
    assert!(time::Pads::out(&mut seq, &at(0, on)));
    assert!(!time::Pads::out(&mut seq, &at(1, on)));
    assert!(time::Pads::hit_at(&mut seq, 1, 2, 100, 0));
    assert_eq!(seq.hits, 3);
    assert_eq!(rig.mix.instruments.stats(0).notes, 2);
    let (mut l, mut r) = ([0.0f32; PERIOD], [0.0f32; PERIOD]);
    rig.player.render_dry(&mut l, &mut r);
    assert_eq!(
        rig.player.hits(),
        before + 1,
        "the kit played only track 2's"
    );
}

/// Instrument redesign, after step 4d: track 5 plays track 3's Poly, so
/// it is heard on track 3's strip. A lock in its clip on "this track's
/// strip" moves that strip's volume, and lets it go there when its step
/// ends; track 5's own strip, which nothing plays through, stays as it
/// was. Nothing on the way allocates.
#[test]
fn a_lock_on_this_tracks_strip_moves_the_strip_its_instrument_sounds_on() {
    use mpfree_fx::ids::mixer as mx;
    use mpfree_fx::StripId;
    use mpfree_play::mpfree_param::{OnTrack, RelAddr};
    use time::StepAction;
    let mut rig = Rig::new(false);
    let job = crate::instrument_job(&rig.synth, 2, "mpfree.poly", None);
    assert!(rig.mix.build_instrument(job));
    assert!(matches!(
        rig.mix.poll_instruments(),
        Some(mixing::Landed::Playing {
            track: 2,
            name: "Poly"
        })
    ));
    assert_eq!(rig.mix.instruments.set_send(4, Some(2)), Ok(()));
    let three = rig.mix.instruments.strip(2).expect("track 3's strip");
    let five = rig.mix.track_channel(4);
    assert_ne!(three, five);
    // Track 5: pad 1 on steps 1 and 9, step 1 locked to -20 dB.
    rig.time.act(TimeAction::Arm(Some(4))).unwrap();
    for step in [0, 8] {
        rig.time
            .step_action(StepAction::Tap {
                pad: 0,
                step,
                velocity: 100,
            })
            .unwrap();
    }
    rig.time
        .step_action(StepAction::Lock {
            pad: 0,
            steps: 1,
            addr: RelAddr::new(OnTrack::Strip, mx::VOLUME),
            value: Some(-20.0),
        })
        .unwrap();
    let volume = |rig: &Rig, t: usize| rig.mix.mixer.param(StripId::Track(t), mx::VOLUME);
    let (was_three, was_five) = (volume(&rig, three), volume(&rig, five));
    assert!(was_three > -20.0, "{was_three}");
    rig.time.command(Command::Play);
    let mut i = 0u64;
    let mut run_to = |rig: &mut Rig, frame: u64| {
        while i * (PERIOD as u64) < frame {
            rig.pads(i);
            rig.period(i);
            i += 1;
        }
    };
    // The first pass may allocate (the mod engine takes the volume on as
    // a target the first time); the second is measured. A bar is 88,200
    // frames at 120 BPM, a step 5,512.5.
    run_to(&mut rig, 88_200);
    let ((), calls) = crate::sys::alloc_count::calls(|| {
        // Between step 1 and step 9.
        run_to(&mut rig, 88_200 + 20_000);
        assert_eq!(
            volume(&rig, three),
            -20.0,
            "track 3's strip, where it sounds"
        );
        assert_eq!(volume(&rig, five), was_five, "track 5's own strip, unused");
        // Past step 9, whose note has no lock: let go on track 3's strip.
        run_to(&mut rig, 88_200 + 50_000);
    });
    assert_eq!(calls, 0, "allocations or frees on the sound path");
    assert_eq!(volume(&rig, three), was_three);
    assert_eq!(volume(&rig, five), was_five);
    let c = rig.time.describe();
    assert!(
        c.contains("this track's strip: 2 locks went to their instrument's strip, 0 waited"),
        "{c}"
    );
}

/// CV out with the clip's track on CV 1 (pitch) and CV 2 (gate), a
/// sixteenth-note clock on CV 3 and run on CV 4: the clip's notes move
/// them while it still plays on the pads, every gate that rises falls,
/// the clock pulses, and the sound path never allocates or frees.
#[test]
fn cv_follows_a_track_and_never_allocates() {
    use mpfree_midi::cv::{CvAssign, Priority, Scale};
    use mpfree_midi::cv_wire::{counts, decode, CV_STATUS, FULL_SCALE};
    use mpfree_midi::setup::MAX_VOLTS;
    let mut rig = Rig::new(false);
    let mut c =
        cores::Cores::start_unpinned(cores::SoundCores::Shared, &mut rig.player, &mut rig.synth);
    c.start_mixer(&mut rig.mix.mixer);
    c.start_tracks(&mut rig.mix.instruments);
    c.start_pads(&mut rig.mix.pad_synths);
    rig.time
        .act(TimeAction::NewClip { track: 0, slot: 0 })
        .unwrap();
    let mut i = 0u64;
    while i < 1_500 {
        rig.pads(i);
        rig.period(i);
        i += 1;
    }
    rig.time.command(Command::Record);
    // Setup, which may allocate: CV out opens and takes its outputs.
    let sink = MidiSink::default();
    rig.cv
        .open(sink.clone(), std::time::Instant::now())
        .unwrap();
    {
        let mut m = rig.io.lock().unwrap();
        m.setup.cv[0] = CvAssign::Pitch {
            track: 0,
            scale: Scale::VoltsPerOctave(1.0),
            zero_note: 36,
            bend_semis: 2,
            priority: Priority::Last,
        };
        m.setup.cv[1] = CvAssign::Gate {
            track: 0,
            volts: 5.0,
            retrigger_ms: 0.0,
        };
        m.setup.cv[2] = CvAssign::Clock {
            ppqn: 4,
            volts: 5.0,
            ms: 5.0,
        };
        m.setup.cv[3] = CvAssign::Run { volts: 5.0 };
        m.gen += 1;
    }
    for _ in 0..200 {
        rig.pads(i);
        rig.period(i);
        i += 1;
    }
    assert_eq!(rig.io.lock().unwrap().live.built.cv_outs, 4);
    let clip_before = rig.clip_hits;
    let ((), calls) = crate::sys::alloc_count::calls(|| {
        for _ in 0..4_000 {
            rig.pads(i);
            rig.period(i);
            i += 1;
        }
    });
    assert_eq!(calls, 0, "allocations or frees on the sound path");
    assert!(
        rig.clip_hits > clip_before,
        "the clip stopped playing on the pads"
    );
    assert!(rig.cv.close());
    let bytes = sink.0.lock().unwrap().clone();
    assert_eq!(bytes.len() % 6, 0);
    let pairs: Vec<(u8, u16)> = bytes
        .chunks(6)
        .map(|p| {
            assert_eq!(
                (p[0], p[3], p[4]),
                (CV_STATUS, CV_STATUS, p[1] + 1),
                "{p:02X?}"
            );
            (p[1] / 2, decode(p[2], p[5]))
        })
        .collect();
    let of = |o: u8| -> Vec<u16> { pairs.iter().filter(|p| p.0 == o).map(|p| p.1).collect() };
    let pitch = of(0);
    assert!(pitch.len() > 20, "pitch changes: {}", pitch.len());
    // The clip's pads 1 to 4 as the MPC's pad notes, 1 V an octave from
    // C2 (36) at 0 V; 0 V at open and close.
    let notes: Vec<u16> = PAD_NOTES[..4]
        .iter()
        .map(|&n| counts((f32::from(n) - 36.0) / 12.0, MAX_VOLTS))
        .collect();
    for n in &notes {
        assert!(pitch.contains(n), "pad note {n} never sent: {pitch:?}");
    }
    assert!(
        pitch.iter().all(|c| *c == 0 || notes.contains(c)),
        "{pitch:?}"
    );
    let gate = of(1);
    let ups = gate.iter().filter(|&&c| c == FULL_SCALE).count();
    assert!(ups > 20, "gates: {ups}");
    assert_eq!(
        gate.iter().filter(|&&c| c == 0).count(),
        ups + 1,
        "every gate falls"
    );
    // 4,200 periods at 120 BPM is about 12.2 s: eight sixteenths a second.
    let pulses = of(2).iter().filter(|&&c| c == FULL_SCALE).count();
    assert!((90..=100).contains(&pulses), "clock pulses: {pulses}");
    // Run joined the playing song on the frame CV out opened (so its 0 V
    // at open was never sent), and fell at close.
    assert_eq!(of(3), [FULL_SCALE, 0]);
}

#[test]
fn the_browsers_preview_mixes_and_stops_without_allocating() {
    let data: Vec<f32> = (0..2 * 44_100).map(|i| (i as f32 * 0.01).sin()).collect();
    let src = mpfree_sampler::model::Source::new("Preview", 48_000, 2, data).unwrap();
    // The box comes from the screen thread; only mixing and stopping are
    // the audio thread's.
    let mut p = Box::new(crate::preview::Preview::new(&src, -6.0));
    let (mut l, mut r) = ([0.0f32; PERIOD], [0.0f32; PERIOD]);
    let (done, calls) = crate::sys::alloc_count::calls(|| {
        for _ in 0..100 {
            p.mix(RATE, &mut l, &mut r);
        }
        p.stop();
        (0..10).any(|_| p.mix(RATE, &mut l, &mut r))
    });
    assert!(done, "the stop faded out");
    assert_eq!(calls, 0, "allocations or frees mixing the preview");
}

/// A kit for each track: in one period, each track's notes play its own
/// kit (kit 1 on track 1, kit 2 on track 2) or the synth (track 3), never
/// allocating; only the hits of the kit the pads show flash them.
#[test]
fn each_track_plays_its_own_kit_at_once() {
    // The rig, a second player and the pads' routing overflow a 2 MiB test
    // thread in a debug build (the time host's start-up frames are most
    // of it), so this one runs on the smoke tests' stack.
    crate::smoke::on_big_stack(each_track_plays_its_own_kit);
}

fn each_track_plays_its_own_kit() {
    let mut rig = Rig::new(false);
    let a = kit::build(&[(PAD_NOTES[0], "Kick", &tone(6000, 0.01)[..])]);
    let b = kit::build(&[(PAD_NOTES[1], "Clap", &tone(3000, 0.03)[..])]);
    let mut kits = kit::parse_kits(&kit::build_kits(&[("House", a), ("Trap", b)])).unwrap();
    rig.player = sampler::Player::new(&mut kits, &PAD_NOTES, RATE).unwrap();
    let mut track_kits = sampler::TrackKits::new(rig.player.kits());
    assert!(track_kits.set(1, 1) && track_kits.set(2, track_kits.synth()));
    let live = rig.app.live_map();
    let mut seq = SeqPads {
        player: &mut rig.player,
        synth: &mut rig.synth,
        kits: &track_kits,
        shown: 0,
        flashes: [0; 128],
        hits: 0,
        live,
        live_hits: [(0, 0); MAX_LIVE_HITS],
        n_live: 0,
        silent: 0,
        midi: &mut rig.midi,
        cv: &mut rig.cv,
        own: &mut rig.mix.instruments,
        modu: &rig.mix.modu,
        pad_synths: &mut rig.mix.pad_synths,
        tap: &mut rig.tap,
        only: None,
        shifts: [mpfree_seq::Transpose::NONE; mpfree_seq::MAX_TRACKS],
        key: None,
        by_note: &mut [mpfree_arp::scale::Held::new(); 3],
    };
    let (sounded, calls) = crate::sys::alloc_count::calls(|| {
        [
            time::Pads::hit_at(&mut seq, 0, 0, 100, 0),
            time::Pads::hit_at(&mut seq, 0, 1, 100, 0),
            time::Pads::hit_at(&mut seq, 1, 1, 100, 0),
            time::Pads::hit_at(&mut seq, 1, 0, 100, 0),
            time::Pads::hit_at(&mut seq, 2, 0, 100, 0),
        ]
    });
    assert_eq!(calls, 0, "allocations or frees routing the tracks' notes");
    assert_eq!(
        sounded,
        [true, false, true, false, true],
        "kit 1's kick, kit 2's clap and the synth; nothing where a kit has no sound"
    );
    assert_eq!(seq.hits, 3);
    assert_eq!(seq.flashes[0], 100, "kit 1's kick shows");
    assert_eq!(
        seq.flashes[1], 0,
        "kit 2's clap doesn't: the pads show kit 1"
    );
    let (mut l, mut r) = ([0.0f32; PERIOD], [0.0f32; PERIOD]);
    rig.player.render_dry(&mut l, &mut r);
    assert_eq!(rig.player.playing(), 2, "both kits sound at once");
}

/// A kit's Instrument and MIDI out pads (the Pads tab's pad types, the
/// redesign's step 6b): a clip's hit on the MIDI out pad goes out on its
/// own port, channel and note though its track plays the kit, and one on
/// the Instrument pad starts a voice of its engine, whichever engine is on
/// the pads; the sampler plays neither, the pads flash, the track's mute
/// ends both, and none of it allocates or frees.
#[test]
fn a_kits_instrument_and_midi_out_pads_play_the_synth_and_the_gear() {
    use crate::synth::Engine;
    use mpfree_midi::setup::DIN_NAME;
    use mpfree_node::NodeEvent;
    let mut rig = Rig::new(false);
    let sink = MidiSink::default();
    rig.midi
        .open(sink.clone(), std::time::Instant::now())
        .unwrap();
    rig.midi.sync(&rig.io, false);
    // Kit 1's pad 3 sends note 36 on channel 10 to the jacks; pad 4 plays
    // Analog at C3. They come with the kit's view, as from the Pads tab.
    let mut routes = vec![None; mpfree_sampler::model::PADS];
    routes[2] = Some(sampler::PadRoute::Midi {
        port: DIN_NAME.into(),
        channel: 9,
        note: 36,
    });
    routes[3] = Some(sampler::PadRoute::Voice(Engine::Analog, 48));
    let mut swap = sampler::ViewSwap {
        kit: 0,
        view: Some(mpfree_sampler::engine::EngineView::build(
            &mpfree_sampler::model::Project::new(),
            RATE,
        )),
        routes,
    };
    assert!(rig.player.swap_view(&mut swap));
    assert!(
        swap.routes.iter().all(Option::is_none),
        "the ones it held go back to be freed"
    );
    assert!(rig.player.has_sound(0, 2) && rig.player.has_sound(0, 3));
    let mut seq = SeqPads {
        player: &mut rig.player,
        synth: &mut rig.synth,
        kits: &rig.track_kits,
        shown: 0,
        flashes: [0; 128],
        hits: 0,
        live: rig.app.live_map(),
        live_hits: [(0, 0); MAX_LIVE_HITS],
        n_live: 0,
        silent: 0,
        midi: &mut rig.midi,
        cv: &mut rig.cv,
        own: &mut rig.mix.instruments,
        modu: &rig.mix.modu,
        pad_synths: &mut rig.mix.pad_synths,
        tap: &mut rig.tap,
        only: None,
        shifts: [mpfree_seq::Transpose::NONE; mpfree_seq::MAX_TRACKS],
        key: None,
        by_note: &mut [mpfree_arp::scale::Held::new(); 2],
    };
    let (sounded, calls) = crate::sys::alloc_count::calls(|| {
        let sounded = [
            time::Pads::hit_at(&mut seq, 0, 2, 100, 0),
            time::Pads::hit_at(&mut seq, 0, 3, 90, 4),
        ];
        time::Pads::pressure_at(&mut seq, 0, 3, 60, 8);
        time::Pads::release_at(&mut seq, 0, 2, 16);
        // Pad 4 is held on: the track's mute ends it.
        seq.synth.end_pad_voices(0, 20);
        sounded
    });
    assert_eq!(calls, 0, "allocations or frees playing the pads' types");
    assert_eq!(sounded, [true, true]);
    assert_eq!((seq.hits, seq.flashes[2], seq.flashes[3]), (2, 100, 90));
    assert_eq!(
        seq.synth.queued_on(Engine::Analog),
        [
            NodeEvent::NoteOn {
                note: 48,
                velocity: 90
            },
            NodeEvent::Pressure {
                note: 48,
                value: 60
            },
            NodeEvent::NoteOff { note: 48 },
        ]
    );
    assert!(
        seq.synth.queued().is_empty(),
        "nothing on the engine on the pads (Poly)"
    );
    seq.midi
        .period_end(PERIOD as u32, std::time::Instant::now());
    assert!(rig.midi.close());
    assert_eq!(
        *sink.0.lock().unwrap(),
        [0x99, 36, 100, 0x89, 36, 64],
        "pad 3's own note on channel 10, and its end"
    );
}

/// Keys a route sends to a track that plays a kit (the I/O view's
/// Routing) play its pads as their kinds have them, as its clips' notes
/// do: the MIDI out pad's key goes out on that pad's own port, channel
/// and note, and the Instrument pad's key starts a voice of its engine
/// (with its pressure and its end); the sampler plays neither, and none
/// of it allocates or frees.
#[test]
fn routed_keys_play_a_kits_midi_out_and_instrument_pads_by_their_kind() {
    use crate::midi_in::Key;
    use crate::synth::Engine;
    use mpfree_midi::profile::kit_pad_of_note;
    use mpfree_midi::setup::DIN_NAME;
    use mpfree_node::NodeEvent;
    let mut rig = Rig::new(false);
    let sink = MidiSink::default();
    rig.midi
        .open(sink.clone(), std::time::Instant::now())
        .unwrap();
    rig.midi.sync(&rig.io, false);
    // Kit 1's pad 3 sends note 36 on channel 10 to the jacks; pad 4 plays
    // Analog at C3.
    let mut routes = vec![None; mpfree_sampler::model::PADS];
    routes[2] = Some(sampler::PadRoute::Midi {
        port: DIN_NAME.into(),
        channel: 9,
        note: 36,
    });
    routes[3] = Some(sampler::PadRoute::Voice(Engine::Analog, 48));
    let mut swap = sampler::ViewSwap {
        kit: 0,
        view: Some(mpfree_sampler::engine::EngineView::build(
            &mpfree_sampler::model::Project::new(),
            RATE,
        )),
        routes,
    };
    assert!(rig.player.swap_view(&mut swap));
    // The keys that hit pads 3 and 4 (the MPC's pad notes).
    let (out_key, voice_key) = (PAD_NOTES[2], PAD_NOTES[3]);
    assert_eq!(
        (kit_pad_of_note(out_key), kit_pad_of_note(voice_key)),
        (2, 3)
    );
    let hits = rig.player.hits();
    let keys = [
        Key::Down {
            note: out_key,
            velocity: 100,
        },
        Key::Down {
            note: voice_key,
            velocity: 90,
        },
        Key::Pressure {
            note: voice_key,
            value: 60,
        },
        Key::Up { note: out_key },
        Key::Up { note: voice_key },
    ];
    let ((), calls) = crate::sys::alloc_count::calls(|| {
        for key in keys {
            // Track 6, not armed, playing kit 1.
            crate::play_routed(
                5,
                key,
                &mut rig.midi,
                &mut rig.cv,
                &mut rig.mix.instruments,
                &mut rig.mix.pad_synths,
                &mut rig.synth,
                false,
                &mut rig.player,
                0,
            );
        }
        rig.midi
            .period_end(PERIOD as u32, std::time::Instant::now());
    });
    assert_eq!(calls, 0, "allocations or frees playing the pads' kinds");
    assert_eq!(
        rig.synth.queued_on(Engine::Analog),
        [
            NodeEvent::NoteOn {
                note: 48,
                velocity: 90
            },
            NodeEvent::Pressure {
                note: 48,
                value: 60
            },
            NodeEvent::NoteOff { note: 48 },
        ]
    );
    assert!(
        rig.synth.queued().is_empty(),
        "nothing on the engine on the pads (Poly)"
    );
    rig.period(0);
    assert_eq!(rig.player.hits(), hits, "the sampler played neither");
    assert!(rig.midi.close());
    assert_eq!(
        *sink.0.lock().unwrap(),
        [0x99, 36, 100, 0x89, 36, 64],
        "pad 3's own note on channel 10, and its end"
    );
}

/// A synth-voice pad with a synth of its own (Jay's card, 18 Nov 20:15Z):
/// asked for, made and put in as the loop does, a clip's hit on it plays
/// its own copy of the engine, not the shared synth, into the pad's own
/// channel (kit 1's pad 4: the A4 strip), not the Synth strip; the periods
/// it sounds in, the copy following the Synth tab, never allocate.
#[test]
fn a_synth_voice_pads_own_synth_plays_into_its_pad_channel() {
    pad_synth_plays(cores::SoundCores::Shared);
}

/// The same on two cores: the copy plays on the pads' lane on core 2, into
/// the same strip, with no allocation.
#[test]
fn a_synth_voice_pads_own_synth_plays_on_another_core() {
    pad_synth_plays(cores::SoundCores::Two);
}

fn pad_synth_plays(sound_cores: cores::SoundCores) {
    use crate::synth::Engine;
    use mpfree_fx::StripId;
    let mut rig = Rig::new(false);
    // Before the sound starts, as player_session does.
    let mut c = cores::Cores::start_unpinned(sound_cores, &mut rig.player, &mut rig.synth);
    c.start_mixer(&mut rig.mix.mixer);
    c.start_tracks(&mut rig.mix.instruments);
    c.start_pads(&mut rig.mix.pad_synths);
    let mut routes = vec![None; mpfree_sampler::model::PADS];
    routes[3] = Some(sampler::PadRoute::Voice(Engine::Analog, 48));
    let mut swap = sampler::ViewSwap {
        kit: 0,
        view: Some(mpfree_sampler::engine::EngineView::build(
            &mpfree_sampler::model::Project::new(),
            RATE,
        )),
        routes,
    };
    assert!(rig.player.swap_view(&mut swap));
    // The time engine drops its first view on the first periods: get
    // that out of the way before counting.
    for i in 0..4 {
        rig.period(i);
    }
    let plan = rig.mix.pad_synths.plan(rig.player.voice_pads(), &rig.synth);
    let p = plan.asks[0].expect("pad 4's copy asked for");
    assert_eq!((p.kit, p.pad, p.engine), (0, 3, Engine::Analog));
    assert_eq!(plan.asks[1], None);
    let (id, extras, values) = rig.synth.sound_of(Engine::Analog);
    assert!(rig.mix.build_instrument(crate::control::InstrumentJob {
        track: 0,
        pad: Some(p),
        id,
        extras,
        values,
        pads: rig.synth.pads_for(Engine::Analog),
        tuning: rig.synth.tuning_now(),
    }));
    assert_eq!(rig.mix.poll_instruments(), None, "a pad's, not a track's");
    assert!(matches!(
        rig.mix.take_pad_landed(),
        Some(mixing::PadLanded::Playing { name: "Analog", .. })
    ));
    let mut seq = SeqPads {
        player: &mut rig.player,
        synth: &mut rig.synth,
        kits: &rig.track_kits,
        shown: 0,
        flashes: [0; 128],
        hits: 0,
        live: rig.app.live_map(),
        live_hits: [(0, 0); MAX_LIVE_HITS],
        n_live: 0,
        silent: 0,
        midi: &mut rig.midi,
        cv: &mut rig.cv,
        own: &mut rig.mix.instruments,
        modu: &rig.mix.modu,
        pad_synths: &mut rig.mix.pad_synths,
        tap: &mut rig.tap,
        only: None,
        shifts: [mpfree_seq::Transpose::NONE; mpfree_seq::MAX_TRACKS],
        key: None,
        by_note: &mut [mpfree_arp::scale::Held::new(); 2],
    };
    assert!(time::Pads::hit_at(&mut seq, 0, 3, 100, 0));
    assert!(
        seq.synth.queued_on(Engine::Analog).is_empty(),
        "not the shared synth"
    );
    let _ = rig.mix.mixer.take_peaks(StripId::Pad(3));
    let _ = rig.mix.mixer.take_peaks(StripId::Track(mixing::SYNTH));
    let ((), calls) = crate::sys::alloc_count::calls(|| {
        for i in 4..12 {
            rig.period(i);
        }
    });
    assert_eq!(calls, 0, "allocations or frees while the copy plays");
    let pad = rig.mix.mixer.take_peaks(StripId::Pad(3));
    assert!(pad[0] > 0.0 && pad[1] > 0.0, "the A4 strip: {pad:?}");
    assert_eq!(
        rig.mix.mixer.take_peaks(StripId::Track(mixing::SYNTH)),
        [0.0, 0.0],
        "nothing on the Synth strip"
    );
    // On two cores the pads' lane played it.
    let n = cores::Counts::read(&rig.player, &rig.mix, &rig.synth);
    if sound_cores == cores::SoundCores::Two {
        assert_eq!(rig.mix.pad_synths.lanes(), 1);
        assert!(n.pad_periods > 0, "{n:?}");
    } else {
        assert_eq!(rig.mix.pad_synths.lanes(), 0);
        assert_eq!(n.pad_periods, 0, "{n:?}");
    }
}
