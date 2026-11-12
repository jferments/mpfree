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
    app, cores, kit, looping, midi_in, midi_out, mixing, sampler, synth, time, SeqPads,
    MAX_LIVE_HITS, PERIOD, RATE,
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
    mix: mixing::MixHost,
    events: Events,
    synth_on: bool,
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
            pads_out: [[0.0; PERIOD]; 2],
            peak: 0.0,
            arp_notes: 0,
            clip_hits: 0,
            midi: midi_out::MidiOut::new(RATE, std::time::Duration::ZERO),
            io: midi_in::io_model(),
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
            mix,
            events,
            synth_on,
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
            ..
        } = self;
        let synth_on = *synth_on;
        app.set_rolling(time.info().state.rolling());
        time.follow_tempo(app.settings.bpm);
        time.advance(PERIOD as u32);
        midi.sync(io, time.info().state.rolling());
        midi.clock(time.block());
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
        };
        time.period(PERIOD as u32, &mut seq);
        seq.midi
            .period_end(PERIOD as u32, std::time::Instant::now());
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
        mix.follow(looper.line.is_open(), looper.monitor);
        let ctx = mixing::ctx(RATE, i * PERIOD as u64, app.settings.bpm, time.block());
        mix.mixer.set_lane_deadline(Some(until));
        let (p, _) = mix.period(
            &ctx,
            (synth_l, synth_r),
            (loop_l, loop_r),
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
    match sound_cores {
        cores::SoundCores::Two if !synth_on => assert!(n.voice_parts > 100, "{n:?}"),
        cores::SoundCores::Two => {}
        cores::SoundCores::Three => assert_eq!(n.synth_periods, i, "{n:?}"),
        _ => assert_eq!(n, cores::Counts::default()),
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
