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
    app, cores, kit, looping, mixing, sampler, synth, time, SeqPads, MAX_LIVE_HITS, PERIOD, RATE,
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
            ..
        } = self;
        let synth_on = *synth_on;
        app.set_rolling(time.info().state.rolling());
        time.follow_tempo(app.settings.bpm);
        time.advance(PERIOD as u32);
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
        };
        time.period(PERIOD as u32, &mut seq);
        *clip_hits += seq.hits;
        time.clear_hits();
        // The pads, each into its mixer channel, and on to the Drums track.
        player.set_tempo(time.info().bpm);
        synth.start(i * PERIOD as u64, app.settings.bpm);
        let mut pads = mix.pads();
        player.render_routed(PERIOD, &mut pads);
        let played = pads.played();
        let [drums_l, drums_r] = drums;
        mix.drums(played, drums_l, drums_r);
        let [synth_l, synth_r] = synth_out;
        synth_l.fill(0.0);
        synth_r.fill(0.0);
        synth.finish(synth_l, synth_r);
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
