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
    left: [f32; PERIOD],
    right: [f32; PERIOD],
    /// The tracks' sounds before the mixer, as `player_session` keeps them.
    drums: [[f32; PERIOD]; 2],
    synth_out: [[f32; PERIOD]; 2],
    loop_out: [[f32; PERIOD]; 2],
    audio_out: [[f32; PERIOD]; 2],
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
        Rig {
            player: sampler::Player::new(&mut kits, &PAD_NOTES, RATE).unwrap(),
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
            left: [0.0; PERIOD],
            right: [0.0; PERIOD],
            drums: [[0.0; PERIOD]; 2],
            synth_out: [[0.0; PERIOD]; 2],
            loop_out: [[0.0; PERIOD]; 2],
            audio_out: [[0.0; PERIOD]; 2],
            pads_out: [[0.0; PERIOD]; 2],
            peak: 0.0,
            arp_notes: 0,
            clip_hits: 0,
            midi: midi_out::MidiOut::new(RATE, std::time::Duration::ZERO),
            io: midi_in::io_model(),
            cv: cv_out::CvOut::new(RATE, std::time::Duration::ZERO),
            tap: crate::export::Tap::on(),
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
            left,
            right,
            drums,
            synth_out,
            loop_out,
            audio_out,
            pads_out,
            arp_notes,
            clip_hits,
            midi,
            io,
            cv,
            tap,
            ..
        } = self;
        let synth_on = *synth_on;
        app.set_rolling(time.info().state.rolling());
        time.follow_tempo(app.settings.bpm);
        time.advance(PERIOD as u32);
        midi.sync(io, time.info().state.rolling());
        midi.clock(time.block());
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
        let mut seq = SeqPads {
            player,
            synth: synth_on.then_some(&mut **synth),
            flashes: [0; 16],
            hits: 0,
            live: app.live_map(),
            live_hits: [(0, 0); MAX_LIVE_HITS],
            n_live: 0,
            silent: 0,
            midi,
            cv,
            own: &mut mix.instruments,
            tap,
        };
        time.period(PERIOD as u32, &mut seq);
        seq.midi
            .period_end(PERIOD as u32, std::time::Instant::now());
        seq.cv.period_end(PERIOD as u32, std::time::Instant::now());
        seq.tap.take();
        *clip_hits += seq.hits;
        time.clear_hits();
        // The pads, each into its mixer channel, and on to the Drums track.
        player.set_tempo(time.info().bpm);
        // The period's deadline for the lanes, as the loop sets it after
        // each write; far, so a slow machine (qemu in CI) is never late.
        let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
        synth.start(i * PERIOD as u64, app.settings.bpm);
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
        let [audio_l, audio_r] = audio_out;
        tracks.follow_latency(app.settings.loop_latency);
        tracks.period(
            time.block(),
            time.tempo_map(),
            time.song_on(),
            looper.input(),
            (audio_l, audio_r),
        );
        mix.follow(looper.line.is_open(), looper.monitor);
        let ctx = mixing::ctx(RATE, i * PERIOD as u64, app.settings.bpm, time.block());
        mix.mixer.set_lane_deadline(Some(until));
        let (p, _) = mix.period(
            &ctx,
            (synth_l, synth_r),
            (loop_l, loop_r),
            (audio_l, audio_r),
            looper.input(),
            left,
            right,
        );
        let p = p.max(time.mix_click(left, right));
        self.peak = self.peak.max(p);
        // After the write, in the loop.
        synth.settle(until);
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
    // On two cores the lane took its half of the voices; on three the synth
    // played every period on its lane. (Late parts are allowed here: under
    // qemu or a busy machine a lane can be slow, and a late part allocates
    // nothing either.)
    let n = cores::Counts::read(&rig.player, &rig.mix.mixer, &rig.synth);
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
/// and its clip play the Poly into its own mixer channel, not the kit, and
/// half a minute of it allocates and frees nothing on the sound path.
#[test]
fn a_track_with_its_own_instrument_plays_it_and_never_allocates() {
    use crate::instruments::first_track;
    let mut rig = Rig::new(false);
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
    // Its own mixer channel shows on the panel; the others don't.
    rig.mix.show(Some(mpfree_mixer_ui::MixView::Tracks));
    let st = rig.mix.poll_state(std::time::Instant::now()).unwrap();
    let names: Vec<&str> = st.strips.iter().map(|v| v.name.as_str()).collect();
    assert_eq!(names, vec!["Drums", "Synth", "Loop", "Audio", "Track 1"]);
    assert_eq!(st.strips[4].id, mpfree_fx::StripId::Track(first_track()));
    assert!(st.strips[4].peak[0] > 0.0, "{:?}", st.strips[4].peak);
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
    use mpfree_midi::setup::{ClockOut, TrackOut, DIN_NAME};
    let mut rig = Rig::new(false);
    let mut c =
        cores::Cores::start_unpinned(cores::SoundCores::Shared, &mut rig.player, &mut rig.synth);
    c.start_mixer(&mut rig.mix.mixer);
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
    let sink = MidiSink::default();
    rig.midi
        .open(sink.clone(), std::time::Instant::now())
        .unwrap();
    {
        let mut m = rig.io.lock().unwrap();
        m.setup.tracks[0] = Some(TrackOut::new(DIN_NAME, 9));
        m.setup
            .set_port(DIN_NAME, |p| p.clock_out = ClockOut::WhilePlaying);
        m.gen += 1;
    }
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
    assert_eq!(
        msgs.len(),
        2 * ons + pulses + 3,
        "only notes and clock went out: {bytes:02X?}"
    );
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
        synth: None,
        flashes: [0; 16],
        hits: 0,
        live,
        live_hits: [(0, 0); MAX_LIVE_HITS],
        n_live: 0,
        silent: 0,
        midi: &mut rig.midi,
        cv: &mut rig.cv,
        own: &mut rig.mix.instruments,
        tap: &mut rig.tap,
    };
    let at = |event| Out {
        offset: 0,
        frame: 0,
        track: 0,
        dest: 0,
        event,
    };
    let on = Event::NoteOn {
        note: 2,
        velocity: 100,
    };
    let heard = time::Pads::live(&mut seq, &at(on));
    assert_eq!(
        heard,
        Some(Event::NoteOn {
            note: 2,
            velocity: v
        })
    );
    let off = Event::NoteOff { note: 2 };
    assert_eq!(time::Pads::live(&mut seq, &at(off)), Some(off));
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
        synth: None,
        flashes: [0; 16],
        hits: 0,
        live,
        live_hits: [(0, 0); MAX_LIVE_HITS],
        n_live: 0,
        silent: 0,
        midi: &mut rig.midi,
        cv: &mut rig.cv,
        own: &mut rig.mix.instruments,
        tap: &mut rig.tap,
    };
    let at = |track, event| Out {
        offset: 0,
        frame: 0,
        track,
        dest: track,
        event,
    };
    let on = Event::NoteOn {
        note: 2,
        velocity: 100,
    };
    assert_eq!(
        time::Pads::live(&mut seq, &at(0, on)),
        Some(Event::NoteOn {
            note: 2,
            velocity: v
        })
    );
    assert_eq!(&seq.live_hits[..seq.n_live], [(2, v)]);
    let off = Event::NoteOff { note: 2 };
    assert_eq!(time::Pads::live(&mut seq, &at(0, off)), Some(off));
    // A clip on track 1 is taken by its instrument; one on track 2 isn't,
    // and goes on to the kit.
    assert!(time::Pads::out(&mut seq, &at(0, on)));
    assert!(!time::Pads::out(&mut seq, &at(1, on)));
    assert!(time::Pads::hit_at(&mut seq, 2, 100, 0));
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
