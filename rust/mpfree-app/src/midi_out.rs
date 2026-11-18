//! MIDI out: a track whose "Plays on" is a port with an output (the 5-pin
//! MIDI Out jack, or a USB device on the USB-A socket) plays there instead
//! of on the pads. Step M3 of docs/dev/design/midi-io.md.
//!
//! **State-changing on the MPC and on the gear.** The jacks' bytes go to
//! the controller's cable 2 ("MIDI Port"), which is the MIDI Out jack (the
//! owner's read-only captures of 2026-11-12,
//! docs/dev/hardware/controller-protocol.md). A USB device's bytes go to
//! its own output, the substream of its port on its own card; never to the
//! MPC's own controller or sound card, which the scanner leaves out.
//! Nothing is stored on the MPC. Only note, pressure, bank and program
//! messages, a track's controllers (0 to 119, never the channel mode
//! messages), pitch bend and channel pressure, and clock are ever sent,
//! never SysEx (plus, from a port's
//! page, its test note and All notes off, and what thru passes on: the
//! channel messages that came in on a port, as they came). The outputs
//! are open only while Prefs' "Send MIDI and CV out" is on (off until the
//! musician turns it on; kept across restarts).
//!
//! - The sound loop calls [`MidiOut::sync`] once a period before the notes:
//!   it takes the project's track outputs when they change, and sends each
//!   routed track's bank and program when it is set and when the song
//!   starts. [`MidiOut::send`] takes each note of a track that plays out,
//!   and [`MidiOut::period_end`] hands each port's messages on. None of
//!   them allocates, waits or writes to a port.
//! - **A writer thread for each port** gets each message with the moment
//!   its frame is heard (the period's start plus the sound's lead plus its
//!   offset), sleeps until then and writes it. A message that finds the
//!   writer's queue full is dropped and counted. The app opens the jacks;
//!   a USB device's writer opens the device's output itself, in its own
//!   thread, when the device is plugged in while sending is on
//!   ([`MidiOut::plugged`]) or when sending is switched on
//!   ([`MidiOut::switch_on`]), so the sound loop never waits for a device.
//! - **Thru** ([`MidiOut::thru`], the Routing section's squares from a
//!   port to an output): a message that came in on a port goes out of
//!   another in this period, as it came, when the armed track's live notes
//!   would. Its notes are kept with the tracks', so a panic (STOP while
//!   stopped), the Monitor's Send note-offs or the port's All notes off
//!   ends them too.
//! - **The gear's delay** (each port's page): each message leaves that
//!   much before its frame is heard, but never before its period was made,
//!   so at most the sound's lead early ([`leave_at`]).
//! - `OutputRouter` remembers every note that is on, so a route change, a
//!   stop or a panic ends exactly those notes, and closing a port ends
//!   them before it lets go. A USB device unplugged
//!   ([`MidiOut::unplugged`]) can't be sent its note-offs: its notes are
//!   forgotten and its tracks play on the pads.
//! - A track set to a port that isn't plugged in, or set while MIDI out is
//!   off, or whose output failed, plays on the pads, and the I/O view says
//!   so on its row ([`mpfree_midi_ui::PortLive::sending`]).
//! - **Clock out** ([`MidiOut::clock`]): each port's "Clock out" (While
//!   playing or Always) makes the transport's period (`TimeHost::block`)
//!   into MIDI clock on that port through `mpfree_midi_clock::ClockOut`:
//!   Start, or Song Position and Continue, where playback starts; a pulse
//!   (F8) every 40 ticks (24 a quarter note) while it plays, following
//!   every tempo change; Stop, Song Position and Continue on a locate or a
//!   loop's jump; Stop when it stops. A count-in sends nothing until
//!   playback starts. Always also sends pulses at the tempo while stopped.
//! - **The Sample tab's auto-sample of outside gear** ([`MidiOut::gear_out`])
//!   plays the gear's notes out of the jacks from the job's own thread:
//!   only note on and off, through a bounded queue the sound loop drains
//!   each period ([`GEAR_QUEUE`]), out at once with no song timing. Its
//!   notes still on end at a panic, when sending is switched off, when the
//!   jacks' writer stops and when the job ends.
//! - **A track's controllers, pitch bend and pressure** (sequencer S6's CC
//!   lanes, `Owner::Midi`): the mod engine moves them (a step's lock, a
//!   lane, a macro, the morph, a learned knob) and [`MidiOut::control`]
//!   sends each value on the track's port and channel on its frame, ahead
//!   of the notes already there (`OutQueue::lift_last_before_notes`). At most one message a control goes out
//!   in a period; a value given again in the same period goes at the
//!   next period's start, so a fast LFO can't flood the 5-pin wire. The
//!   last value of each control of each track is kept, also for a track on
//!   the pads, and sent again when the track's port or channel is set and
//!   at each song start, after its program.

use std::io::{self, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::sync::{Arc, TryLockError};
use std::time::{Duration, Instant};

use mpfree_midi::expr_out::{self, ExprOut};
use mpfree_midi::mpe::Zone;
use mpfree_midi::mpe_out::MpeOut;
use mpfree_midi::msg::Msg;
use mpfree_midi::output::{DinPacer, OutQueue, OutRoute, OutputRouter, Timed, MAX_TRACKS};
use mpfree_midi::port::{PortId, DIN, MAX_PORTS};
use mpfree_midi::profile::{PadOut, KIT_NOTES, KIT_PADS};
use mpfree_midi::setup::{ClockOut, TrackOut, DIN_NAME};
use mpfree_midi_clock::{ClockOut as Pulses, Messages, PULSE_TICKS};
use mpfree_midi_ui::{IoModel, PortSend, Shared, ToPort};
use mpfree_node::Ctl;
use mpfree_notefx::TrackFx;
use mpfree_play::mpfree_param::midi::{self as ctl, Midi};
use mpfree_play::mpfree_param::{Owner, ParamAddr};
use mpfree_play::{Event, Out};
use mpfree_transport::{Block, Mark, Marker, Span, State};

use crate::port_writer::{Closing, Wire, Writer, STALL};
use crate::sample_page::GearOut;

/// Notes one pad can start (a synth pad's chord); more are not sent.
const CHORD: usize = 8;
/// A track's pads: a kit's 128 ([`KIT_PADS`]), a synth layout's 16.
const PADS: usize = KIT_PADS;
/// The most messages a period lists for the MIDI monitor, over every port.
const SENT: usize = 1024;
/// The most messages the Sample tab's auto-sample of outside gear can have
/// waiting for the sound loop; more are dropped and counted. The job sends
/// one note at a time, seconds apart.
pub const GEAR_QUEUE: usize = 64;
/// The jacks' name on the Sample tab's auto-sample.
pub const GEAR_NAME: &str = "MIDI Out (5-pin)";

/// What the auto-sample's job hands the sound loop.
#[derive(Clone, Copy, Debug)]
enum GearMsg {
    /// A note on or off, as it goes out.
    Note(Msg),
    /// The job is over (or was cancelled): every note it left on ends.
    End,
}

/// The Sample tab's auto-sample of outside gear: its notes from the job's
/// thread to the jacks, through a bounded queue the sound loop drains.
struct Gear {
    tx: SyncSender<GearMsg>,
    rx: Receiver<GearMsg>,
    /// Its notes on, a bit for each note, by channel.
    held: [u128; 16],
    /// Counted on the job's thread: messages refused (not a note on or
    /// off), and dropped because the queue was full.
    refused: Arc<AtomicU64>,
    full: Arc<AtomicU64>,
    /// Notes sent, and not sent because sending was off or the jacks
    /// closed.
    sent: u64,
    unsent: u64,
    /// Whether [`MidiOut::gear_changed`] last said there is a MIDI out.
    told: Option<bool>,
}

impl Gear {
    fn new() -> Gear {
        let (tx, rx) = sync_channel(GEAR_QUEUE);
        Gear {
            tx,
            rx,
            held: [0; 16],
            refused: Arc::new(AtomicU64::new(0)),
            full: Arc::new(AtomicU64::new(0)),
            sent: 0,
            unsent: 0,
            told: None,
        }
    }
}

/// What the auto-sample's `send` hands on: a note on or off of exactly
/// three bytes, or the end of the job (no bytes). None: refused.
fn gear_msg(b: &[u8]) -> Option<GearMsg> {
    match *b {
        [] => Some(GearMsg::End),
        [s, n, v] if n < 0x80 && v < 0x80 => match Msg::from_bytes(s, n, v)? {
            m @ (Msg::NoteOn { .. } | Msg::NoteOff { .. }) => Some(GearMsg::Note(m)),
            _ => None,
        },
        _ => None,
    }
}

/// Opens a USB device's output, in its writer's thread: the app's way to
/// that device's substream, checked before anything is written.
pub type Opener = Arc<dyn Fn() -> io::Result<Box<dyn Write + Send>> + Send + Sync>;

/// What an event's note number is.
#[derive(Clone, Copy, Debug)]
pub enum NoteIs<'a> {
    /// A MIDI note (the arps' notes while a synth's pad layout plays
    /// notes).
    Note,
    /// A pad, 0 to 15, that plays these notes: a synth pad's notes. A
    /// pad's note-off and pressure go to the notes its note-on sent.
    Pad(&'a [u8]),
    /// A kit's pad, 0 (A1) to 127: the note, and the channel, its port's
    /// device profile gives it, else its own note ([`kit_notes`]). Ends
    /// and pressure go where its note-on went.
    Kit,
}

/// When a message heard at `heard` leaves for gear that sounds `early`
/// late: that much before, but never before `t0`, when its period was made
/// (so at most the sound's lead early).
fn leave_at(heard: Instant, early: Duration, t0: Instant) -> Instant {
    heard.checked_sub(early).map_or(t0, |d| d.max(t0))
}

/// The test note's note-off.
fn test_off(ch: u8) -> Msg {
    Msg::NoteOff {
        ch,
        note: TEST_NOTE,
        vel: 64,
    }
}

/// A kit pad's note on MIDI Out when its port's profile names none: bank
/// A as Akai's MPC sends its pads (pad 1 is C#2, note 37), banks B to H a
/// note each of the rest ([`KIT_NOTES`]).
pub fn kit_notes(pad: usize) -> &'static [u8] {
    KIT_NOTES.get(pad).map_or(&[], std::slice::from_ref)
}

/// A track's MIDI out controls: CC 0-119, the bend and the pressure
/// (`mpfree_param::midi`), by their ids.
const CTLS: usize = ctl::COUNT as usize;
const CTL_WORDS: usize = CTLS.div_ceil(64);
/// A control no value has been given for.
const UNSET: i16 = i16::MIN;
/// No message out yet, in [`Controls::last`].
const NEVER: u32 = u32::MAX;

/// Each track's controls out ([`MidiOut::control`]): the value last
/// given, and which went out this period or wait for the next.
struct Controls {
    /// The value last given, raw (before the track's CC effects).
    value: Box<[[i16; CTLS]; MAX_TRACKS]>,
    /// The value that last went out, after the track's CC effects
    /// (`UNSET` before any): where a glide starts, and what a change of
    /// the effects compares with.
    out: Box<[[i16; CTLS]; MAX_TRACKS]>,
    /// Each control's glide to its value (CC smooth), stepped once a
    /// period ([`MidiOut::glides`]).
    glide: Box<[[Glide; CTLS]; MAX_TRACKS]>,
    /// Tracks with a control gliding.
    gliding: u128,
    /// Tracks whose CC effects changed: their controls' values go out
    /// again where the effects now make them something else.
    remap: u128,
    /// Sent this period: a second value waits ([`Controls::waiting`]).
    sent: [[u64; CTL_WORDS]; MAX_TRACKS],
    /// Given again after this period's message: sent at the next
    /// period's start.
    waiting: [[u64; CTL_WORDS]; MAX_TRACKS],
    /// Tracks with a bit in `sent` or `waiting`.
    touched: u128,
    /// The period each control's last message went out in ([`Controls::now`]),
    /// for the 5-pin jacks' floor ([`DIN_GLIDE_FLOOR`]); `NEVER` before any.
    last: Box<[[u32; CTLS]; MAX_TRACKS]>,
    /// Periods since the start, counted at each period's end.
    now: u32,
    /// Tracks whose every control with a value goes out again (a new port
    /// or channel, the song's start).
    due: u128,
    /// Messages sent, for the summary.
    messages: u64,
}

impl Controls {
    fn new() -> Controls {
        Controls {
            value: Box::new([[UNSET; CTLS]; MAX_TRACKS]),
            out: Box::new([[UNSET; CTLS]; MAX_TRACKS]),
            glide: Box::new([[Glide::IDLE; CTLS]; MAX_TRACKS]),
            gliding: 0,
            remap: 0,
            sent: [[0; CTL_WORDS]; MAX_TRACKS],
            waiting: [[0; CTL_WORDS]; MAX_TRACKS],
            touched: 0,
            last: Box::new([[NEVER; CTLS]; MAX_TRACKS]),
            now: 0,
            due: 0,
            messages: 0,
        }
    }

    /// Control `id` at `v` as a message on channel 1 (the router puts it
    /// on the track's).
    fn msg(id: usize, v: i16) -> Option<Msg> {
        if v == UNSET {
            return None;
        }
        Some(match ctl::of(id as u16)? {
            Midi::Cc(cc) => Msg::Cc {
                ch: 0,
                cc,
                value: v.clamp(0, 127) as u8,
            },
            Midi::Bend => Msg::PitchBend {
                ch: 0,
                value: v.clamp(-8192, 8191),
            },
            Midi::Pressure => Msg::ChanPressure {
                ch: 0,
                value: v.clamp(0, 127) as u8,
            },
        })
    }
}

/// A control gliding to its value (CC smooth).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Glide {
    /// Where it started and where it goes.
    from: f32,
    to: i16,
    /// Frames it takes, and frames left (0: not gliding).
    total: u32,
    left: u32,
    /// Frames since its last message went out, for the jacks' floor.
    since: u32,
}

impl Glide {
    const IDLE: Glide = Glide {
        from: 0.0,
        to: 0,
        total: 0,
        left: 0,
        since: 0,
    };
}

/// The least time between one control's glide messages on the 5-pin
/// jacks, whose wire carries about 1,000 messages a second, so gliding
/// controls leave room for the notes and the clock. None on USB.
const DIN_GLIDE_FLOOR: Duration = Duration::from_millis(4);

/// A track's output as it was taken from the project.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Applied {
    port: PortId,
    channel: u8,
    program: Option<u8>,
    bank: Option<u16>,
    /// The port's MPE zone ([`Zone::Off`] for gear without MPE).
    mpe: Zone,
}

impl Applied {
    /// `o` on a port MIDI out has: the jacks, or a USB device plugged in
    /// with an output. None for any other. On MPE gear the track's channel
    /// is the zone's master channel, whatever its Plays on says: the
    /// notes take the members ([`MpeOut`]), and program, bank and
    /// controllers go to the whole zone.
    fn of(o: &TrackOut, ports: &[PortOut]) -> Option<Applied> {
        let i = ports.iter().position(|p| p.here && p.name == o.port)?;
        let mpe = ports[i].mpe;
        Some(Applied {
            port: PortId(i as u8),
            channel: mpe.master().unwrap_or(o.channel.min(15)),
            program: o.program.map(|p| p.min(127)),
            bank: o.bank.map(|b| b.min(0x3FFF)),
            mpe,
        })
    }
}

/// One of track `t`'s messages to the router at `offset`, on `ch` (None:
/// the track's). On MPE gear (`mpe` has a zone) a note, its end and its
/// pressure without a channel of their own take the track's member
/// channels ([`MpeOut`]); everything else goes as it is.
fn route_msg(
    router: &mut OutputRouter,
    queue: &mut [OutQueue],
    mpe: &mut MpeOut,
    t: usize,
    m: Msg,
    ch: Option<u8>,
    offset: u32,
) {
    let mut on = |m: Msg| router.send_on(t as u8, m, Some(m.channel()), offset, queue);
    match (m, ch, mpe.zone()) {
        (_, Some(_), _) | (_, _, Zone::Off) => {}
        (Msg::NoteOn { note, vel, .. }, None, _) => return mpe.note_on(note, vel, &mut on),
        (Msg::NoteOff { note, .. }, None, _) => return mpe.note_off(note, &mut on),
        (Msg::PolyPressure { note, value, .. }, None, _) => {
            return mpe.pressure(note, value, &mut on)
        }
        _ => {}
    }
    router.send_on(t as u8, m, ch, offset, queue);
}

/// The notes a pad's note-on sent, and the channel of its own a kit pad
/// sent them on.
#[derive(Clone, Copy, Debug, Default)]
struct Chord {
    notes: [u8; CHORD],
    len: u8,
    ch: Option<u8>,
}

impl Chord {
    fn notes(&self) -> &[u8] {
        &self.notes[..usize::from(self.len)]
    }
}

/// What each track's pads' note-ons sent, so a pad's note-off and
/// pressure go to the same notes even if its layout (or its port's
/// profile) changed in between. MIDI Out, CV out and an export's tap each
/// keep one. Boxed: 128 pads of 128 tracks.
#[derive(Clone, Debug)]
pub struct Chords(Box<[[Chord; PADS]]>);

impl Default for Chords {
    fn default() -> Self {
        Self::new()
    }
}

impl Chords {
    pub fn new() -> Chords {
        Chords(vec![[Chord::default(); PADS]; MAX_TRACKS].into_boxed_slice())
    }

    /// The note messages (on channel 0) one note event of track `t` makes,
    /// in order, to `f`. A pad hit again ends its earlier notes first, as
    /// the synth does. Velocity 0 sounds as 1, so a note-on is never read
    /// as a note-off. A clip's program change (S5) makes its bank select,
    /// CC 0 then CC 32, when it has a bank, then the program change, as
    /// the track's own program goes (`MidiOut::program`). Other events
    /// make none. A kit's pads play their own notes ([`kit_notes`]).
    pub fn messages(&mut self, t: usize, event: Event, is: NoteIs, mut f: impl FnMut(Msg)) {
        self.messages_on(t, event, is, |_| None, |m, _| f(m));
    }

    /// [`Chords::messages`], with what a kit's pad plays from `kit` (its
    /// port's profile; None: its own note), and each message's channel
    /// of its own (None: the track's) to `f`.
    pub fn messages_on(
        &mut self,
        t: usize,
        event: Event,
        is: NoteIs,
        kit: impl Fn(usize) -> Option<PadOut>,
        mut f: impl FnMut(Msg, Option<u8>),
    ) {
        let Some(pads) = self.0.get_mut(t) else {
            return;
        };
        // A kit's pad: its notes and channel, as a pad layout's.
        let kit_note;
        let (is, ch) = match (is, event) {
            (NoteIs::Kit, Event::NoteOn { note, .. }) => {
                let pad = usize::from(note);
                match kit(pad) {
                    Some(o) => {
                        kit_note = [o.note];
                        (NoteIs::Pad(&kit_note), o.channel)
                    }
                    None => (NoteIs::Pad(kit_notes(pad)), None),
                }
            }
            (NoteIs::Kit, _) => (NoteIs::Pad(&[]), None),
            (is, _) => (is, None),
        };
        let off = |note| Msg::NoteOff {
            ch: 0,
            note,
            vel: 64,
        };
        match (event, is) {
            (Event::NoteOn { note, velocity }, NoteIs::Note) => f(
                Msg::NoteOn {
                    ch: 0,
                    note: note & 0x7F,
                    vel: velocity.clamp(1, 127),
                },
                None,
            ),
            (Event::NoteOn { note, velocity }, NoteIs::Pad(notes)) => {
                let Some(c) = pads.get_mut(usize::from(note)) else {
                    return;
                };
                let was = std::mem::take(c);
                for &n in was.notes() {
                    f(off(n), was.ch);
                }
                c.ch = ch;
                for &n in notes.iter().filter(|&&n| n < 128).take(CHORD) {
                    c.notes[usize::from(c.len)] = n;
                    c.len += 1;
                }
                for &n in c.notes() {
                    f(
                        Msg::NoteOn {
                            ch: 0,
                            note: n,
                            vel: velocity.clamp(1, 127),
                        },
                        ch,
                    );
                }
            }
            (Event::NoteOff { note }, NoteIs::Note) => f(off(note & 0x7F), None),
            (Event::NoteOff { note }, NoteIs::Pad(_)) => {
                if let Some(c) = pads.get_mut(usize::from(note)).map(std::mem::take) {
                    for &n in c.notes() {
                        f(off(n), c.ch);
                    }
                }
            }
            (Event::Pressure { note, value }, NoteIs::Note) => f(
                Msg::PolyPressure {
                    ch: 0,
                    note: note & 0x7F,
                    value,
                },
                None,
            ),
            (Event::Pressure { note, value }, NoteIs::Pad(_)) => {
                if let Some(c) = pads.get(usize::from(note)) {
                    for &n in c.notes() {
                        f(
                            Msg::PolyPressure {
                                ch: 0,
                                note: n,
                                value,
                            },
                            c.ch,
                        );
                    }
                }
            }
            (Event::Program { program, bank }, _) => {
                if let Some(b) = bank {
                    for (cc, value) in [(0, (b >> 7) as u8 & 0x7F), (32, (b & 0x7F) as u8)] {
                        f(Msg::Cc { ch: 0, cc, value }, None);
                    }
                }
                f(
                    Msg::Program {
                        ch: 0,
                        program: program & 0x7F,
                    },
                    None,
                );
            }
            _ => {}
        }
    }

    /// Note-offs for every pad of track `t` that is on.
    /// Note-offs for every pad of track `t` that is on, each with the
    /// channel of its own it started on.
    pub fn end_track(&mut self, t: usize, mut f: impl FnMut(Msg, Option<u8>)) {
        let Some(pads) = self.0.get_mut(t) else {
            return;
        };
        for c in pads.iter_mut() {
            let c = std::mem::take(c);
            for &n in c.notes() {
                f(
                    Msg::NoteOff {
                        ch: 0,
                        note: n,
                        vel: 64,
                    },
                    c.ch,
                );
            }
        }
    }

    /// Forgets every pad's notes, in place (no allocation).
    pub fn clear(&mut self) {
        for pads in self.0.iter_mut() {
            pads.fill(Chord::default());
        }
    }
}

/// One port's clock out: its setting and the clock it makes.
struct PortClock {
    mode: ClockOut,
    /// Its clock runs this many times slower (`PortSetup::clock_divide`):
    /// the gear's ticks are the transport's divided by it.
    divide: i64,
    /// The divide changed while it ran: Stop, then it joins again at the
    /// next sixteenth at the new rate.
    restart: bool,
    pulses: Pulses,
    /// Always, while stopped: frames from this period's start to the next
    /// pulse.
    free_next: f64,
}

impl PortClock {
    fn new() -> PortClock {
        PortClock {
            mode: ClockOut::Off,
            divide: 1,
            restart: false,
            pulses: Pulses::new(),
            free_next: 0.0,
        }
    }

    /// The divide the setup asks for (1 and up).
    fn set_divide(&mut self, divide: u8) {
        let d = i64::from(divide.max(1));
        if d != self.divide {
            self.restart = self.pulses.running();
            self.divide = d;
        }
    }

    /// A transport tick as the gear's, at this port's divide.
    fn tick(&self, tick: i64) -> i64 {
        tick.div_euclid(self.divide)
    }

    /// The transport's period as this port's clock: its messages to `out`,
    /// in the order they must leave, and Always's pulses while stopped
    /// straight to `q`.
    fn period(&mut self, block: &Block, out: &mut Messages, q: &mut OutQueue) {
        if self.mode == ClockOut::Off {
            // Turned off while playing: the other device stops too.
            self.stop(out);
            return;
        }
        if std::mem::take(&mut self.restart) {
            self.stop(out);
        }
        let markers = block.markers();
        let mut next = 0;
        for span in block.spans() {
            // A marker on a span's first frame happened just before it.
            while next < markers.len() && markers[next].offset <= span.offset {
                self.mark(&markers[next], out);
                next += 1;
            }
            self.span(span, out, q);
        }
        for m in &markers[next..] {
            self.mark(m, out);
        }
        self.free_next = (self.free_next - f64::from(block.frames)).max(0.0);
    }

    fn mark(&mut self, m: &Marker, out: &mut Messages) {
        match m.mark {
            Mark::Started { tick } => self.pulses.started(m.offset, self.tick(tick), out),
            Mark::Stopped { .. } => {
                self.pulses.stopped(m.offset, out);
                // Always: pulses go on from here at the tempo.
                self.free_next = f64::from(m.offset);
            }
            Mark::Located { to, .. } => self.pulses.located(m.offset, self.tick(to), out),
            Mark::Looped { start, .. } => self.pulses.located(m.offset, self.tick(start), out),
            // Back to bar 1 while stopped: the other device's position too.
            Mark::Rewound => self.pulses.located(m.offset, 0, out),
            _ => {}
        }
    }

    fn span(&mut self, span: &Span, out: &mut Messages, q: &mut OutQueue) {
        match span.state {
            State::Playing | State::Recording => {
                // Clock turned on (or the port opened) while playing: the
                // other device joins from the next sixteenth.
                if !self.pulses.running() {
                    let tick = self.tick(span.ticks().start);
                    self.pulses.started(span.offset, tick, out);
                }
                // Divided: a pulse every `divide` of the transport's.
                for (tick, offset) in span.lines(PULSE_TICKS * self.divide) {
                    self.pulses.pulse(self.tick(tick), offset, out);
                }
            }
            // Nothing until playback starts, when Started comes.
            State::CountIn => {}
            State::Stopped if self.mode == ClockOut::Always && !self.pulses.running() => {
                let every =
                    60.0 * f64::from(span.rate) * self.divide as f64 / (span.bpm.max(1.0) * 24.0);
                let (start, end) = (f64::from(span.offset), f64::from(span.offset + span.len));
                self.free_next = self.free_next.max(start);
                while self.free_next < end {
                    let offset = self.free_next as u32;
                    q.push(Timed::realtime(offset, mpfree_midi_clock::CLOCK));
                    self.free_next += every;
                }
            }
            State::Stopped => {}
        }
    }

    /// Stop, if the clock is running.
    fn stop(&mut self, out: &mut Messages) {
        if self.pulses.running() {
            self.pulses.stopped(0, out);
        }
    }
}

/// One port MIDI out can send to, at its port number: the 5-pin jacks
/// ([`DIN`]) or one USB device's output.
struct PortOut {
    /// Its name in the project; empty while nothing is plugged in here.
    name: String,
    /// Plugged in with an output (the jacks always are).
    here: bool,
    /// How a USB device's output is opened (the app opens the jacks).
    open: Option<Opener>,
    writer: Option<Writer>,
    /// The writer Prefs switched off, until its last messages have left.
    closing: Option<Closing>,
    /// Its gear's delay ("This gear's delay" on its page): messages leave
    /// this much before their frame is heard, at most the sound's lead.
    early: Duration,
    /// Its clock's nudge (`PortSetup::clock_nudge_tenths`): later above 0.
    nudge_tenths: i16,
    /// What its device profile has each kit pad play (None: the pad's own
    /// note on the track's channel), by pad.
    pads: Box<[Option<PadOut>]>,
    /// Its profile's bend range in semitones (0: it doesn't say), and
    /// whether its gear takes each note's own pressure (`pressure = poly`).
    bend: u8,
    poly_pressure: bool,
    /// Its profile's MPE zone (`mpe = lower 15`): MPE gear, whose tracks'
    /// notes each take a member channel ([`MpeOut`]).
    mpe: Zone,
    clock: PortClock,
    /// Its page's test note still on: its channel and the frames until its
    /// note-off.
    test_off: Option<(u8, u32)>,
    /// Messages handed to its writer.
    queued: u64,
    /// Messages dropped because its writer's queue was full.
    full: u64,
    /// Its writer stopped (or its output didn't open), once the log has
    /// said so.
    lost: bool,
}

impl PortOut {
    fn new() -> PortOut {
        PortOut {
            name: String::new(),
            here: false,
            open: None,
            writer: None,
            closing: None,
            early: Duration::ZERO,
            nudge_tenths: 0,
            pads: vec![None; KIT_PADS].into_boxed_slice(),
            bend: 0,
            poly_pressure: false,
            mpe: Zone::Off,
            clock: PortClock::new(),
            test_off: None,
            queued: 0,
            full: 0,
            lost: false,
        }
    }

    /// Lets go of its writers without waiting: each writes what it was
    /// handed (if its port still takes it) and stops.
    fn let_go(&mut self) {
        if let Some(w) = self.writer.take() {
            w.let_go();
        }
        if let Some(c) = self.closing.take() {
            c.let_go();
        }
    }

    /// What the log calls it: "MIDI Out" for the jacks, as before USB
    /// devices could be sent to.
    fn label(&self, i: usize) -> String {
        if i == usize::from(DIN.0) {
            "MIDI Out".to_string()
        } else {
            format!("MIDI out to {}", self.name)
        }
    }
}

/// Tracks to MIDI out's ports, from the sound loop.
pub struct MidiOut {
    router: OutputRouter,
    /// Each port's messages this period, at its port number.
    queue: Vec<OutQueue>,
    /// Each port MIDI out can send to, at its port number.
    ports: Vec<PortOut>,
    /// The jacks' 31250 bit/s wire: when each message really leaves them,
    /// for the summary.
    pacer: DinPacer,
    rate: u32,
    lead: Duration,
    /// Prefs' "Send MIDI and CV out" is on: a USB device plugged in opens
    /// its output.
    sending: bool,
    /// The app's start, for the writers' log times.
    started: Instant,
    tracks: [Option<Applied>; MAX_TRACKS],
    /// Bank and program to send before this period's notes.
    program_due: [bool; MAX_TRACKS],
    chords: Chords,
    /// Each track's keys' expression for its gear, made one bend, pressure
    /// and CC 74 ([`MidiOut::expression`]).
    expr: Box<[ExprOut]>,
    /// Each track's notes on MPE gear, a member channel each.
    mpe: Box<[MpeOut]>,
    /// Each track's pedals the keys put down on its gear, a bit each in
    /// [`Ctl::ALL`]'s order, so that they come up with its expression.
    keys_pedals: Box<[u8]>,
    applied_gen: Option<u64>,
    shown_on: Option<bool>,
    rolling: bool,
    /// Periods whose settings update was skipped because the screen held
    /// the model.
    busy: u64,
    clock_msgs: Messages,
    /// The last period's messages the writers took, with their ports, for
    /// the MIDI monitor ([`MidiOut::sent`]), and when its frame 0 is heard.
    sent: Vec<(PortId, Timed)>,
    sent_at: Instant,
    /// Log lines from the sound loop's side (a writer that couldn't
    /// start), said at the next [`MidiOut::tick`].
    said: Vec<String>,
    /// Messages thru couldn't send (MIDI out off, or no output there).
    thru_unsent: u64,
    /// Messages controller mode couldn't send, as for thru.
    controller_unsent: u64,
    /// MIDI Follow's values sent back to its controller.
    sent_back: u64,
    /// The Sample tab's auto-sample of outside gear, out of the jacks.
    gear: Gear,
    /// The tracks' controllers, bend and pressure out.
    controls: Controls,
    /// Each track's MIDI effects as they are now (the mod engine's values
    /// on them too), for its CC scale and CC smooth
    /// ([`MidiOut::set_note_fx`]).
    note_fx: Box<[TrackFx; MAX_TRACKS]>,
    /// Frames in a period (the last one's), for the glides' steps.
    period: u32,
    /// Where each kit pad that plays a port of its own sent its note
    /// ([`MidiOut::pad_to`]), per track and pad, so its end and pressure
    /// follow it there.
    pad_out: Box<[Option<(OutRoute, u8)>]>,
}

/// Where glide `g` is now, `left` frames from its end.
fn glide_at(g: &Glide) -> f32 {
    let k = g.left as f32 / g.total.max(1) as f32;
    f32::from(g.to) + (g.from - f32::from(g.to)) * k
}

/// The port page's test note: C4, held half a second.
pub const TEST_NOTE: u8 = 60;
const TEST_VEL: u8 = 100;
const TEST_HELD: Duration = Duration::from_millis(500);
/// All notes off (CC 123).
const ALL_NOTES_OFF: u8 = 123;

impl MidiOut {
    /// Closed: no track is taken until [`MidiOut::open`] or
    /// [`MidiOut::switch_on`]. `lead` is how long after a period is made
    /// its sound is heard (the sound card's buffer).
    pub fn new(rate: u32, lead: Duration) -> MidiOut {
        let mut pacer = DinPacer::new(rate);
        // The MPC's controller sends the jack's bytes; MPFree hands it whole
        // messages over USB, so the wire time is estimated without running
        // status.
        pacer.running_status = false;
        let mut ports: Vec<PortOut> = (0..MAX_PORTS).map(|_| PortOut::new()).collect();
        let din = &mut ports[usize::from(DIN.0)];
        din.name = DIN_NAME.to_string();
        din.here = true;
        MidiOut {
            router: OutputRouter::new(),
            queue: (0..MAX_PORTS).map(|_| OutQueue::new()).collect(),
            ports,
            pacer,
            rate: rate.max(1),
            lead,
            sending: false,
            started: Instant::now(),
            tracks: [None; MAX_TRACKS],
            program_due: [false; MAX_TRACKS],
            chords: Chords::new(),
            expr: vec![ExprOut::new(); MAX_TRACKS].into_boxed_slice(),
            mpe: vec![MpeOut::new(); MAX_TRACKS].into_boxed_slice(),
            keys_pedals: vec![0; MAX_TRACKS].into_boxed_slice(),
            applied_gen: None,
            shown_on: None,
            rolling: false,
            busy: 0,
            clock_msgs: Messages::new(),
            sent: Vec::with_capacity(SENT),
            sent_at: Instant::now(),
            said: Vec::new(),
            thru_unsent: 0,
            controller_unsent: 0,
            sent_back: 0,
            gear: Gear::new(),
            controls: Controls::new(),
            note_fx: Box::new([TrackFx::default(); MAX_TRACKS]),
            period: (rate / 375).max(1),
            pad_out: vec![None; MAX_TRACKS * PADS].into_boxed_slice(),
        }
    }

    /// Starts the jacks' writer thread on the MIDI Out port (`w`), and
    /// sends to every USB device plugged in ([`MidiOut::switch_on`]).
    /// `started` is the app's start, for the log's times.
    pub fn open<W: Write + Send + 'static>(&mut self, w: W, started: Instant) -> io::Result<()> {
        let din = &mut self.ports[usize::from(DIN.0)];
        din.writer = Some(Writer::start(w, "midi-out", "MIDI Out", describe, started)?);
        din.lost = false;
        self.switch_on(started);
        Ok(())
    }

    /// Whether the jacks' writer is open.
    pub fn is_open(&self) -> bool {
        self.ports[usize::from(DIN.0)].writer.is_some()
    }

    /// Whether Prefs' "Send MIDI and CV out" is on, as last applied.
    pub fn sending(&self) -> bool {
        self.sending
    }

    /// Prefs switched sending on again before the jacks' last writer let
    /// go of the port: it takes that writer back. False when there is
    /// none. (USB devices: [`MidiOut::switch_on`].)
    pub fn reopen(&mut self) -> bool {
        let din = &mut self.ports[usize::from(DIN.0)];
        match din.closing.take().and_then(Closing::take_back) {
            Some(w) => {
                din.writer = Some(w);
                din.lost = false;
                self.sending = true;
                true
            }
            None => false,
        }
    }

    /// Prefs switched sending on: each USB device plugged in with an
    /// output takes back the writer it was letting go, or gets a new one,
    /// which opens its output in its own thread and says so in the log.
    /// The jacks are the app's to open ([`MidiOut::reopen`],
    /// [`MidiOut::open`]).
    pub fn switch_on(&mut self, started: Instant) {
        self.started = started;
        self.sending = true;
        for i in 0..self.ports.len() {
            if i != usize::from(DIN.0) {
                self.start_usb(i);
            }
        }
    }

    /// A USB device's port `i`, plugged in, gets a writer if it has none.
    fn start_usb(&mut self, i: usize) {
        let label = self.ports[i].label(i);
        let p = &mut self.ports[i];
        if !p.here || p.writer.is_some() {
            return;
        }
        if let Some(w) = p.closing.take().and_then(Closing::take_back) {
            p.writer = Some(w);
            p.lost = false;
            self.said.push(format!(
                "{label}: on again (Prefs); its output was still open"
            ));
            return;
        }
        let Some(open) = p.open.clone() else { return };
        match Writer::start_opening(
            move || open(),
            "midi-out-usb",
            &label,
            describe,
            self.started,
        ) {
            Ok(w) => {
                p.writer = Some(w);
                p.lost = false;
            }
            Err(e) => {
                p.lost = true;
                self.said.push(format!("{label}: not started: {e}"));
            }
        }
    }

    /// A USB device's port with an output was plugged in, at port number
    /// `id` (its number in the MIDI in port table), named `name`; `open`
    /// opens its output. While sending is on, its writer starts now and
    /// opens it. A track set to it plays there from the next period.
    pub fn plugged(&mut self, id: PortId, name: &str, open: Opener) {
        let i = usize::from(id.0);
        if i == usize::from(DIN.0) || i >= self.ports.len() {
            return;
        }
        // Another device had this number: what it had is let go.
        self.forget(i);
        let p = &mut self.ports[i];
        p.let_go();
        p.name = name.to_string();
        p.here = true;
        p.open = Some(open);
        p.lost = false;
        self.applied_gen = None;
        if self.sending {
            self.start_usb(i);
        }
    }

    /// The USB device's port `id` went away: nothing can reach it, so the
    /// notes on there are forgotten, its writer let go, and its tracks play
    /// on the pads from the next period.
    pub fn unplugged(&mut self, id: PortId) {
        let i = usize::from(id.0);
        if i == usize::from(DIN.0) || !self.ports.get(i).is_some_and(|p| p.here) {
            return;
        }
        self.forget(i);
        let p = &mut self.ports[i];
        p.let_go();
        p.here = false;
        p.open = None;
        self.applied_gen = None;
    }

    /// Forgets the notes, test note and clock on port `i`: it can't be
    /// sent their ends.
    fn forget(&mut self, i: usize) {
        self.end_port(i, 0);
        self.queue[i].clear();
        if i == usize::from(DIN.0) {
            self.gear.held = [0; 16];
        }
        let p = &mut self.ports[i];
        p.test_off = None;
        // Its setting stays; the clock starts afresh when it is sent to.
        let mode = p.clock.mode;
        p.clock = PortClock::new();
        p.clock.mode = mode;
    }

    /// Whether MIDI clock (or start and stop) goes out to any port, to gear
    /// that follows it: an export then plays in real time.
    pub fn sends_clock(&self) -> bool {
        self.ports
            .iter()
            .any(|p| p.writer.is_some() && p.clock.mode != ClockOut::Off)
    }

    /// Whether `track`'s events go to MIDI out (and not to the pads).
    pub fn takes(&self, track: u16) -> bool {
        self.tracks
            .get(usize::from(track))
            .copied()
            .flatten()
            .is_some_and(|a| self.ports[usize::from(a.port.0)].writer.is_some())
    }

    /// Once a period, before the notes: takes the project's track outputs
    /// when they changed (a changed route first ends the notes on at the
    /// old one) and tells the I/O view whether MIDI out works and which
    /// ports it sends to. Never waits for the screen. On the song's start
    /// each routed track's bank and program go out ahead of its notes.
    pub fn sync(&mut self, model: &Shared, rolling: bool) {
        match model.try_lock() {
            Ok(mut m) => self.apply(&mut m),
            Err(TryLockError::Poisoned(e)) => self.apply(&mut e.into_inner()),
            Err(TryLockError::WouldBlock) => self.busy += 1,
        }
        if rolling && !self.rolling {
            for t in 0..MAX_TRACKS {
                self.program_due[t] |= self.tracks[t].is_some();
                if self.tracks[t].is_some() {
                    self.controls.due |= 1 << t;
                }
            }
        }
        self.rolling = rolling;
        for t in 0..MAX_TRACKS {
            if std::mem::take(&mut self.program_due[t]) {
                self.program(t);
            }
        }
        self.controls_due();
    }

    /// At a period's start, after the programs: the controls that waited
    /// from the last period, and every control with a value of a track
    /// whose port or channel was set or whose song started (straight to
    /// its value, no glide); then the glides' next steps, and the controls
    /// a change of their track's CC effects moves.
    fn controls_due(&mut self) {
        let due = std::mem::take(&mut self.controls.due);
        let mut tracks = self.controls.touched | due;
        while tracks != 0 {
            let t = tracks.trailing_zeros() as usize;
            tracks &= tracks - 1;
            if !self.takes(t as u16) {
                self.controls.waiting[t] = [0; CTL_WORDS];
                continue;
            }
            for id in 0..CTLS {
                let (w, bit) = (id / 64, 1u64 << (id % 64));
                let waited = self.controls.waiting[t][w] & bit != 0;
                // On the jacks a value too soon after the last waits on.
                if waited && due & (1 << t) == 0 && !self.ctl_free(t, id) {
                    self.controls.touched |= 1 << t;
                    continue;
                }
                if waited {
                    self.controls.waiting[t][w] &= !bit;
                    // A waiting value counts as this period's message.
                    self.controls.sent[t][w] |= bit;
                    self.controls.touched |= 1 << t;
                }
                if due & (1 << t) != 0 {
                    self.send_ctl(t, id, 0, true);
                } else if waited {
                    self.send_ctl(t, id, 0, false);
                }
            }
        }
        self.glides();
        let mut tracks = std::mem::take(&mut self.controls.remap);
        while tracks != 0 {
            let t = tracks.trailing_zeros() as usize;
            tracks &= tracks - 1;
            if !self.takes(t as u16) {
                continue;
            }
            for id in 0..CTLS {
                let v = self.controls.value[t][id];
                if v == UNSET {
                    continue;
                }
                let (to, _) = self.shaped(t, id, v);
                let g = self.controls.glide[t][id];
                let goes = if g.total > 0 {
                    g.to
                } else {
                    self.controls.out[t][id]
                };
                if to == goes {
                    continue;
                }
                let (w, bit) = (id / 64, 1u64 << (id % 64));
                self.controls.touched |= 1 << t;
                if self.controls.sent[t][w] & bit != 0 {
                    self.controls.waiting[t][w] |= bit;
                } else {
                    self.controls.sent[t][w] |= bit;
                    self.send_ctl(t, id, 0, false);
                }
            }
        }
    }

    /// Control `id`'s raw value `v` on track `t` as it goes out: through
    /// the track's CC effects (a CC's scale), and the CC smooth that
    /// takes it, if any.
    fn shaped(&self, t: usize, id: usize, v: i16) -> (i16, Option<mpfree_notefx::CcSmooth>) {
        match ctl::of(id as u16) {
            Some(Midi::Cc(cc)) => {
                let (v, smooth) = self.note_fx[t].cc_out(cc, v.clamp(0, 127) as u8);
                (i16::from(v), smooth)
            }
            _ => (v, None),
        }
    }

    /// Whether track `t` has a note starting at `offset` this period.
    fn note_at(&self, t: usize, offset: u32) -> bool {
        let Some(port) = self.tracks[t].map(|a| usize::from(a.port.0)) else {
            return false;
        };
        self.queue[port].as_slice().iter().any(|m| {
            let b = m.bytes();
            m.offset == offset
                && m.track() == Some(t as u8)
                && b.len() == 3
                && b[0] & 0xF0 == 0x90
                && b[2] > 0
        })
    }

    /// Control `id` of track `t` goes out at `offset` as its effects make
    /// it: at once, or gliding there from where it is (CC smooth), from
    /// the next period's start. `jump`: at once whatever the effects say
    /// (a new port, the song's start). A value that comes with one of the
    /// track's notes jumps too unless the smooth glides on notes, so the
    /// gear reads it as the note starts.
    fn send_ctl(&mut self, t: usize, id: usize, offset: u32, jump: bool) {
        let v = self.controls.value[t][id];
        if v == UNSET {
            return;
        }
        let (to, smooth) = self.shaped(t, id, v);
        let was = self.controls.out[t][id];
        let g = self.controls.glide[t][id];
        let glide = smooth.filter(|s| {
            !jump
                && was != UNSET
                && (to != was || g.total > 0)
                && (s.glide_on_note || !self.note_at(t, offset))
        });
        match glide {
            Some(s) => {
                // From where it is now, mid-glide or not.
                let from = if g.total > 0 {
                    glide_at(&g)
                } else {
                    f32::from(was)
                };
                let total = (u64::from(s.ms) * u64::from(self.rate) / 1000).max(1) as u32;
                self.controls.glide[t][id] = Glide {
                    from,
                    to,
                    total,
                    left: total,
                    // A glide from rest may step at once.
                    since: if g.total > 0 { g.since } else { u32::MAX / 2 },
                };
                self.controls.gliding |= 1 << t;
            }
            None => {
                self.controls.glide[t][id].left = 0;
                self.controls.glide[t][id].total = 0;
                // Only a change goes (an LFO gives a value every period),
                // unless the gear must be told again (a new route, the
                // song's start).
                if to == was && !jump {
                    return;
                }
                if let Some(m) = Controls::msg(id, to) {
                    self.controls.out[t][id] = to;
                    self.controls.last[t][id] = self.controls.now;
                    self.ctl_msg(t, m, offset);
                }
            }
        }
    }

    /// The gliding controls' next steps, at the period's start: a message
    /// only when the value that goes out changes, none for a control that
    /// already sent one this period, and on the 5-pin jacks none sooner
    /// than [`DIN_GLIDE_FLOOR`] after the last.
    fn glides(&mut self) {
        let floor = (DIN_GLIDE_FLOOR.as_secs_f64() * f64::from(self.rate)) as u32;
        let mut tracks = self.controls.gliding;
        while tracks != 0 {
            let t = tracks.trailing_zeros() as usize;
            tracks &= tracks - 1;
            if !self.takes(t as u16) {
                for g in self.controls.glide[t].iter_mut() {
                    *g = Glide::IDLE;
                }
                self.controls.gliding &= !(1 << t);
                continue;
            }
            let din = self.tracks[t].is_some_and(|a| a.port == DIN);
            let mut busy = false;
            for id in 0..CTLS {
                let mut g = self.controls.glide[t][id];
                if g.total == 0 {
                    continue;
                }
                let out = self.controls.out[t][id];
                g.left = g.left.saturating_sub(self.period);
                g.since = g.since.saturating_add(self.period);
                let v = if g.left == 0 {
                    g.to
                } else {
                    glide_at(&g).round() as i16
                };
                let (w, bit) = (id / 64, 1u64 << (id % 64));
                let free = self.controls.sent[t][w] & bit == 0 && (!din || g.since >= floor);
                if v != out && free {
                    if let Some(m) = Controls::msg(id, v) {
                        self.controls.out[t][id] = v;
                        self.controls.sent[t][w] |= bit;
                        self.controls.touched |= 1 << t;
                        self.controls.last[t][id] = self.controls.now;
                        g.since = 0;
                        self.ctl_msg(t, m, 0);
                    }
                }
                if g.left == 0 && self.controls.out[t][id] == g.to {
                    // There: idle.
                    g.total = 0;
                } else {
                    busy = true;
                }
                self.controls.glide[t][id] = g;
            }
            if !busy {
                self.controls.gliding &= !(1 << t);
            }
        }
    }

    /// Each track's MIDI effects as they are now, with the mod engine's
    /// values on them (`TimeHost::note_fx_now`), once a period: a track's
    /// CC scale and CC smooth shape its controllers from the next
    /// message, and a change sends again the controllers whose value out
    /// it changes. Never allocates.
    pub fn set_note_fx(&mut self, t: usize, fx: &TrackFx) {
        let Some(now) = self.note_fx.get_mut(t) else {
            return;
        };
        if now != fx {
            let shaped = now.shapes_ccs() || fx.shapes_ccs();
            *now = *fx;
            if shaped {
                self.controls.remap |= 1 << t;
            }
        }
    }

    /// The mod engine's value for a track's controller, bend or pressure
    /// (`Owner::Midi`, from `ModMix::take_midi_values`), at `offset` frames
    /// into this period. Kept for the track either way; sent on its port
    /// and channel while MIDI out takes the track, only when the value
    /// that goes out changes, at most once a period a control (a second
    /// value in the period goes at the next period's start), and on the
    /// 5-pin jacks no sooner than [`DIN_GLIDE_FLOOR`] after the last (a
    /// value too soon waits, and the newest goes). Never allocates.
    pub fn control(&mut self, offset: u32, addr: ParamAddr, value: f32) {
        let Owner::Midi(t) = addr.owner else { return };
        let (t, id) = (usize::from(t), usize::from(addr.param));
        let Some((lo, hi)) = ctl::range(addr.param) else {
            return;
        };
        if t >= MAX_TRACKS || !value.is_finite() {
            return;
        }
        self.controls.value[t][id] = (value.round() as i32).clamp(lo.into(), hi.into()) as i16;
        if !self.takes(t as u16) {
            return;
        }
        let (w, bit) = (id / 64, 1u64 << (id % 64));
        self.controls.touched |= 1 << t;
        if self.controls.sent[t][w] & bit != 0 || !self.ctl_free(t, id) {
            self.controls.waiting[t][w] |= bit;
            return;
        }
        self.controls.sent[t][w] |= bit;
        self.send_ctl(t, id, offset, false);
    }

    /// Whether track `t`'s control `id` may go out now: always on USB, and
    /// for a control CC smooth glides; else on the 5-pin jacks no sooner
    /// than [`DIN_GLIDE_FLOOR`] after its last message, so an LFO on a
    /// control (a value each period) leaves the notes room on the wire.
    fn ctl_free(&self, t: usize, id: usize) -> bool {
        if !self.tracks[t].is_some_and(|a| a.port == DIN) {
            return true;
        }
        // CC smooth's glide keeps its own floor ([`MidiOut::glides`]).
        let v = self.controls.value[t][id];
        if v != UNSET && self.shaped(t, id, v).1.is_some() {
            return true;
        }
        let last = self.controls.last[t][id];
        if last == NEVER {
            return true;
        }
        let floor = DIN_GLIDE_FLOOR.as_secs_f64() * f64::from(self.rate);
        let since = u64::from(self.controls.now.wrapping_sub(last)) * u64::from(self.period);
        since as f64 >= floor
    }

    /// A control's message from track `t`, put before the notes already
    /// on its frame (the mixer, and so the mod engine, runs after the
    /// notes): a step's lock reaches the gear before the step's note.
    fn ctl_msg(&mut self, t: usize, m: Msg, offset: u32) {
        let Some(port) = self.tracks[t].map(|a| usize::from(a.port.0)) else {
            return;
        };
        let before = self.queue[port].len();
        self.msg(t, m, offset);
        if self.queue[port].len() > before {
            self.queue[port].lift_last_before_notes();
            self.controls.messages += 1;
        }
    }

    fn apply(&mut self, m: &mut IoModel) {
        if self.shown_on != Some(self.sending) {
            m.live.built.notes_out = self.sending;
            m.live.built.clock_out = self.sending;
            m.live.early_ms = self.lead.as_secs_f32() * 1000.0;
            self.shown_on = Some(self.sending);
        }
        // Which ports MIDI out sends to now, for their rows and pages (the
        // port list is rebuilt when a port comes or goes).
        for lp in m.live.ports.iter_mut() {
            let p = self
                .ports
                .iter()
                .find(|p| p.here && p.writer.is_some() && p.name == lp.name);
            lp.sending = p.is_some();
            // The clock as it left, to a tenth of a ms.
            let tenth = |x: f32| (x * 10.0).round() / 10.0;
            let measured = p
                .and_then(|p| p.writer.as_ref())
                .and_then(Writer::clock_measured)
                .map(|(g, s)| (tenth(g), tenth(s)));
            if lp.clock_measured != measured {
                lp.clock_measured = measured;
            }
        }
        if let Some(ask) = m.asks.port_send.take() {
            let (said, sent) = self.port_send(&ask);
            if sent && ask.what == ToPort::AllNotesOff {
                if let Some(id) = m
                    .monitor
                    .ports()
                    .iter()
                    .find(|p| p.name == ask.port)
                    .map(|p| p.id)
                {
                    m.monitor.port_out_notes_up(id);
                }
            }
            m.live.port_said = Some(said);
        }
        if self.applied_gen == Some(m.gen) {
            return;
        }
        self.applied_gen = Some(m.gen);
        for p in self.ports.iter_mut().filter(|p| p.here) {
            // Found in place: `MidiSetup::port` would make a copy.
            let s = m.setup.ports.iter().find(|s| s.name == p.name);
            p.clock.mode = s.map_or(ClockOut::default(), |s| s.clock_out);
            p.clock.set_divide(s.map_or(1, |s| s.clock_divide));
            p.nudge_tenths = s.map_or(0, |s| s.clock_nudge_tenths);
            p.early = s.map_or(Duration::ZERO, |s| {
                Duration::from_micros(u64::from(s.delay_tenths) * 100)
            });
            // Its profile's kit pads, in place.
            let profile = m.profile(&p.name);
            for (pad, out) in p.pads.iter_mut().enumerate() {
                *out = profile.and_then(|f| f.pad(pad));
            }
            p.bend = profile.and_then(|f| f.bend).unwrap_or(0);
            p.poly_pressure = profile.is_some_and(|f| f.poly_pressure);
            p.mpe = profile.and_then(|f| f.mpe).unwrap_or(Zone::Off);
        }
        for t in 0..MAX_TRACKS {
            let want = m.setup.tracks[t]
                .as_ref()
                .and_then(|o| Applied::of(o, &self.ports));
            if want == self.tracks[t] {
                continue;
            }
            let route = want.map(|a| OutRoute {
                port: a.port,
                channel: a.channel,
            });
            let was = self.tracks[t].map(|a| OutRoute {
                port: a.port,
                channel: a.channel,
            });
            let zone = want.map_or(Zone::Off, |a| a.mpe);
            if route != was || zone != self.mpe[t].zone() {
                // Another port or channel, or none, or another MPE zone:
                // what is on ends where it started, and its gear's bend
                // and pressure go back to rest. A new program alone leaves
                // held notes alone.
                self.end_chords(t, 0);
                self.release_expression(t, 0);
                self.mpe[t].set_zone(zone);
            }
            if route != was {
                self.router.set_route(t as u8, route, 0, &mut self.queue);
                // The gear there gets the track's controls (after its
                // program).
                if route.is_some() {
                    self.controls.due |= 1 << t;
                }
            }
            self.tracks[t] = want;
            // MPE gear gets its zone with the program.
            self.program_due[t] =
                want.is_some_and(|a| a.program.is_some() || a.bank.is_some() || a.mpe != Zone::Off);
        }
    }

    /// A port page's send: what it said, and whether anything went out.
    fn port_send(&mut self, ask: &PortSend) -> (String, bool) {
        if !self.sending {
            return ("Nothing sent: MIDI out is off (Prefs)".to_string(), false);
        }
        let Some(i) = self.ports.iter().position(|p| p.here && p.name == ask.port) else {
            return (
                format!(
                    "Nothing sent: {} isn't plugged in, or has no output",
                    ask.port
                ),
                false,
            );
        };
        if self.ports[i].writer.is_none() {
            return (
                format!(
                    "Nothing sent: the output to {} didn't open (the log says why)",
                    ask.port
                ),
                false,
            );
        }
        match ask.what {
            ToPort::TestNote { channel } => {
                let ch = channel.min(15);
                // One test note at a time: a second press ends the first.
                if let Some((old, _)) = self.ports[i].test_off.take() {
                    self.queue[i].push(Timed::msg(0, test_off(old)).from_page());
                }
                self.queue[i].push(
                    Timed::msg(
                        0,
                        Msg::NoteOn {
                            ch,
                            note: TEST_NOTE,
                            vel: TEST_VEL,
                        },
                    )
                    .from_page(),
                );
                let frames = (TEST_HELD.as_secs_f64() * f64::from(self.rate)) as u32;
                self.ports[i].test_off = Some((ch, frames));
                (
                    format!(
                        "Test note C4 sent to {} on ch {}, held half a second",
                        ask.port,
                        ch + 1
                    ),
                    true,
                )
            }
            ToPort::AllNotesOff => {
                self.end_port(i, 0);
                for ch in 0..16 {
                    self.queue[i].push(
                        Timed::msg(
                            0,
                            Msg::Cc {
                                ch,
                                cc: ALL_NOTES_OFF,
                                value: 0,
                            },
                        )
                        .from_page(),
                    );
                }
                (
                    format!("All notes off sent to {} on all 16 channels", ask.port),
                    true,
                )
            }
        }
    }

    /// Ends the notes MPFree has on at port `i` (and its test note), at
    /// `offset`; every other port's notes go on.
    fn end_port(&mut self, i: usize, offset: u32) {
        if let Some((ch, _)) = self.ports[i].test_off.take() {
            self.queue[i].push(Timed::msg(offset, test_off(ch)).from_page());
        }
        let port = PortId(i as u8);
        for t in 0..MAX_TRACKS {
            if self.tracks[t].is_some_and(|a| a.port == port) {
                // The router sends the note-offs; the pads' chords only
                // forget.
                self.chords.end_track(t, |_, _| {});
            }
        }
        self.router.port_off(port, offset, &mut self.queue);
    }

    /// Bank select (CC 0, then CC 32) and program change, for what is set.
    fn program(&mut self, t: usize) {
        let Some(a) = self.tracks[t] else { return };
        // MPE gear: its zone first (RPN 6 on the master channel).
        let (router, queue) = (&mut self.router, &mut self.queue);
        self.mpe[t].configure(|m| router.send_on(t as u8, m, Some(m.channel()), 0, queue));
        if let Some(b) = a.bank {
            for (cc, value) in [(0, (b >> 7) as u8), (32, (b & 0x7F) as u8)] {
                self.msg(t, Msg::Cc { ch: 0, cc, value }, 0);
            }
        }
        if let Some(program) = a.program {
            self.msg(t, Msg::Program { ch: 0, program }, 0);
        }
    }

    /// One message from track `t`; the router puts it on the track's port
    /// and channel.
    fn msg(&mut self, t: usize, m: Msg, offset: u32) {
        self.router.send(t as u8, m, offset, &mut self.queue);
    }

    /// One of the player's events. True when MIDI out took it, so the pads
    /// don't play it. An `AllOff` (a panic) ends every note MIDI out has on
    /// and is left to the pads too; locks and lanes are left to the mixer.
    pub fn send(&mut self, o: &Out, is: NoteIs) -> bool {
        if o.event == Event::AllOff {
            self.all_off(o.offset);
            return false;
        }
        if !self.takes(o.track) {
            return false;
        }
        if matches!(
            o.event,
            Event::Lock { .. } | Event::Lane { .. } | Event::Unlock { .. }
        ) {
            return false;
        }
        let t = usize::from(o.track);
        // A kit's pads play what the track's port's profile says.
        let kit = self
            .router
            .route(t as u8)
            .and_then(|r| self.ports.get(usize::from(r.port.0)))
            .map(|p| &p.pads[..]);
        let (router, queue, mpe) = (&mut self.router, &mut self.queue, &mut self.mpe[t]);
        self.chords.messages_on(
            t,
            o.event,
            is,
            |pad| kit.and_then(|k| k.get(pad).copied().flatten()),
            |m, ch| route_msg(router, queue, mpe, t, m, ch, o.offset),
        );
        true
    }

    /// A kit pad that plays other gear (the Pads tab's MIDI out pad): pad
    /// `o.event`'s note of track `o.track` goes out as `note` on `channel`
    /// (0 to 15) of the port named `port`, not by the track's Plays on,
    /// which needn't be set. Its note-off and pressure (as poly pressure)
    /// go where its note-on went, whatever the pad says by then; a pad
    /// pressed again while its note is on ends that note first. True when
    /// the note went out (or ended); false when nothing can send it:
    /// MIDI out is off, the port isn't plugged in, or the track is past
    /// the 16 MIDI out serves (`mpfree_midi_ui::sends_nothing` says why
    /// in words). The notes end with the track's pads
    /// ([`MidiOut::end_pads`]), the port, and a panic. Never allocates.
    pub fn pad_to(&mut self, o: &Out, port: &str, channel: u8, note: u8) -> bool {
        let t = usize::from(o.track);
        let pad = match o.event {
            Event::NoteOn { note, .. } | Event::NoteOff { note } | Event::Pressure { note, .. } => {
                usize::from(note)
            }
            _ => return false,
        };
        if t >= MAX_TRACKS || pad >= PADS {
            return false;
        }
        let key = t * PADS + pad;
        let (router, queue) = (&mut self.router, &mut self.queue);
        let mut send = |to, m| router.send_to(t as u8, to, m, o.offset, queue);
        let end = |(to, n): (OutRoute, u8), send: &mut dyn FnMut(OutRoute, Msg)| {
            send(
                to,
                Msg::NoteOff {
                    ch: 0,
                    note: n,
                    vel: 64,
                },
            );
        };
        match o.event {
            Event::NoteOn { velocity, .. } if velocity > 0 => {
                let here = self.ports.iter().position(|p| p.here && p.name == port);
                let Some(i) = here.filter(|_| self.sending) else {
                    return false;
                };
                if let Some(was) = self.pad_out[key].take() {
                    end(was, &mut send);
                }
                let to = OutRoute {
                    port: PortId(i as u8),
                    channel: channel & 0x0F,
                };
                let n = note & 0x7F;
                send(
                    to,
                    Msg::NoteOn {
                        ch: 0,
                        note: n,
                        vel: velocity.min(127),
                    },
                );
                self.pad_out[key] = Some((to, n));
                true
            }
            Event::NoteOn { .. } | Event::NoteOff { .. } => match self.pad_out[key].take() {
                Some(was) => {
                    end(was, &mut send);
                    true
                }
                None => false,
            },
            Event::Pressure { value, .. } => match self.pad_out[key] {
                Some((to, n)) => {
                    send(
                        to,
                        Msg::PolyPressure {
                            ch: 0,
                            note: n,
                            value: value.min(127),
                        },
                    );
                    true
                }
                None => false,
            },
            _ => false,
        }
    }

    /// Ends the notes track `track`'s pads have on ports of their own
    /// ([`MidiOut::pad_to`]), at `offset`: its mute, its clip's stop.
    /// A change of the track's Plays on leaves them alone.
    pub fn end_pads(&mut self, track: usize, offset: u32) {
        if track >= MAX_TRACKS {
            return;
        }
        self.pad_out[track * PADS..(track + 1) * PADS].fill(None);
        self.router.pads_off(track as u8, offset, &mut self.queue);
    }

    /// A message thru passes on from the input `from` to the output `to`
    /// ([`crate::midi_in::Passed`]), at the start of this period. False
    /// when nothing can send it: MIDI out is off, or `to` isn't plugged in
    /// or its output didn't open.
    pub fn thru(&mut self, from: PortId, to: PortId, msg: Msg) -> bool {
        let i = usize::from(to.0);
        if !self.sending || !self.ports.get(i).is_some_and(|p| p.writer.is_some()) {
            self.thru_unsent += 1;
            return false;
        }
        self.router.thru(from, to, msg, 0, &mut self.queue);
        true
    }

    /// A message controller mode sends to the output `to` (the MPC's pads
    /// and Q-Links), at the start of this period. False when nothing can
    /// send it: MIDI out is off, or `to` isn't plugged in or its output
    /// didn't open.
    pub fn controller(&mut self, to: PortId, msg: Msg) -> bool {
        let i = usize::from(to.0);
        if !self.sending || !self.ports.get(i).is_some_and(|p| p.writer.is_some()) {
            self.controller_unsent += 1;
            return false;
        }
        self.router.controller(to, msg, 0, &mut self.queue);
        true
    }

    /// A value MIDI Follow sends back to its controller on `to` (a knob's
    /// CC, so its lights or motor fader show the Q-Link's value), at the
    /// start of this period. False when nothing can send it, as for
    /// [`MidiOut::controller`]; it isn't counted then, since the next
    /// change sends again.
    pub fn send_back(&mut self, to: PortId, msg: Msg) -> bool {
        let i = usize::from(to.0);
        if !self.sending || !self.ports.get(i).is_some_and(|p| p.writer.is_some()) {
            return false;
        }
        let Some(q) = self.queue.get_mut(i) else {
            return false;
        };
        q.push(Timed::msg(0, msg).sent_back());
        self.sent_back += 1;
        true
    }

    /// A key's expression for track `t`'s gear, at the start of this
    /// period: a keyboard's wheel and pressure, an MPE controller's slide,
    /// pressure and tone on each of its notes, and its keys going down and
    /// up (which say whose count). Gear without MPE takes them as one bend
    /// (over its profile's `bend` range), one pressure and one CC 74
    /// ([`ExprOut`]); gear whose profile says `pressure = poly` takes each
    /// key's pressure on its note. True when MIDI out takes the track, so
    /// the pads and instruments don't.
    pub fn expression(&mut self, track: u16, i: expr_out::In) -> bool {
        if !self.takes(track) {
            return false;
        }
        let t = usize::from(track);
        let (bend, poly) = self.tracks[t]
            .and_then(|a| self.ports.get(usize::from(a.port.0)))
            .map_or((0, false), |p| (p.bend, p.poly_pressure));
        let (router, queue) = (&mut self.router, &mut self.queue);
        // MPE gear: each note's own on its member channel, the wheel on
        // the master; a key going up drops what was kept for a note that
        // never came.
        let mpe = &mut self.mpe[t];
        if mpe.zone() != Zone::Off {
            let mut on = |m: Msg| router.send_on(t as u8, m, Some(m.channel()), 0, queue);
            match i {
                expr_out::In::Wheel(v) => mpe.wheel(v, &mut on),
                expr_out::In::Slide { note, semis } => mpe.slide(note, semis, &mut on),
                expr_out::In::Pressure { note, value } => mpe.pressure(note, value, &mut on),
                expr_out::In::Tone { note, value } => mpe.tone(note, value, &mut on),
                expr_out::In::Up { note } => mpe.key_up(note),
                expr_out::In::Down { .. } => {}
            }
            return true;
        }
        match i {
            expr_out::In::Pressure { note, value } if poly => {
                let m = Msg::PolyPressure {
                    ch: 0,
                    note: note & 0x7F,
                    value: value.min(127),
                };
                router.send(t as u8, m, 0, queue);
            }
            _ => self.expr[t].take(i, bend, |m| router.send(t as u8, m, 0, queue)),
        }
        true
    }

    /// What [`MidiOut::expression`] last sent `track`'s gear from the keys:
    /// its bend (-8192 to 8191) and, unless the gear takes pressure note
    /// by note (its profile's poly pressure), its channel pressure, for
    /// recording them (`expr_rec`). None while the track's events don't go
    /// to MIDI out, or its gear is MPE, whose notes each bend on their own.
    pub fn expression_sent(&self, track: u16) -> Option<(i16, Option<u8>)> {
        if !self.takes(track) {
            return None;
        }
        let t = usize::from(track);
        if self.mpe.get(t)?.zone() != Zone::Off {
            return None;
        }
        let poly = self.tracks[t]
            .and_then(|a| self.ports.get(usize::from(a.port.0)))
            .is_some_and(|p| p.poly_pressure);
        let (bend, pressure) = self.expr.get(t)?.sent();
        Some((bend, (!poly).then_some(pressure)))
    }

    /// A keyboard's pedal, mod wheel, breath or foot controller for track
    /// `track`'s gear, at the start of this period: its CC as it came, on
    /// the track's channel (MPE gear's master channel). A pedal it put
    /// down comes up when the track's expression goes back to rest
    /// (another track armed, its route changed, a stop, a panic). True
    /// when MIDI out takes the track, so its instrument and the synth
    /// don't.
    pub fn keys_control(&mut self, track: u16, ctl: Ctl, value: f32) -> bool {
        if !self.takes(track) {
            return false;
        }
        let Some(v) = Ctl::clamp(value) else {
            return true;
        };
        let t = usize::from(track);
        if let Some(i) = Ctl::ALL.iter().position(|&c| c == ctl && c.is_pedal()) {
            if Ctl::down(v) {
                self.keys_pedals[t] |= 1 << i;
            } else {
                self.keys_pedals[t] &= !(1 << i);
            }
        }
        let m = Msg::Cc {
            ch: 0,
            cc: ctl.cc(),
            value: (v * 127.0).round() as u8,
        };
        self.router.send(t as u8, m, 0, &mut self.queue);
        true
    }

    /// Track `track`'s gear's bend and pressure back to rest (another
    /// track is armed), at the start of this period.
    pub fn rest_expression(&mut self, track: usize) {
        self.release_expression(track, 0);
    }

    /// Track `t`'s gear's bend and pressure back to rest, and the pedals
    /// the keys put down up, at `offset`.
    fn release_expression(&mut self, t: usize, offset: u32) {
        let (router, queue) = (&mut self.router, &mut self.queue);
        if let Some(down) = self.keys_pedals.get_mut(t) {
            for (i, ctl) in Ctl::ALL.into_iter().enumerate() {
                if *down & (1 << i) != 0 {
                    let m = Msg::Cc {
                        ch: 0,
                        cc: ctl.cc(),
                        value: 0,
                    };
                    router.send(t as u8, m, offset, queue);
                }
            }
            *down = 0;
        }
        if let Some(e) = self.expr.get_mut(t) {
            e.release(|m| router.send(t as u8, m, offset, queue));
        }
        if let Some(mpe) = self.mpe.get_mut(t) {
            mpe.release(|m| router.send_on(t as u8, m, Some(m.channel()), offset, queue));
        }
    }

    fn end_chords(&mut self, t: usize, offset: u32) {
        let (router, queue) = (&mut self.router, &mut self.queue);
        let Some(mpe) = self.mpe.get_mut(t) else {
            return;
        };
        self.chords
            .end_track(t, |m, ch| route_msg(router, queue, mpe, t, m, ch, offset));
    }

    /// Ends every note MIDI out has on, on every port (a panic, a stop),
    /// at `offset`: only the notes that are on, never 2048 blind
    /// note-offs. A gear bent or pressed by the keys goes back to rest.
    pub fn all_off(&mut self, offset: u32) {
        self.chords.clear();
        self.pad_out.fill(None);
        // Bend and pressure back to rest, where they aren't.
        for t in 0..MAX_TRACKS {
            self.release_expression(t, offset);
        }
        self.router.all_off(offset, &mut self.queue);
        self.gear_off(offset);
    }

    /// The jacks as the Sample tab's MIDI out for auto-sampling outside
    /// gear, while they are open and Prefs' Send MIDI and CV out is on;
    /// None otherwise. Its `send`, from any thread, takes a note on or off
    /// (three bytes), or no bytes when the job is over or cancelled, which
    /// ends every note it left on. Anything else is refused. It never
    /// waits: a message that finds [`GEAR_QUEUE`] full is dropped. Both
    /// are counted for the log. Allocates (its name and `send`).
    pub fn gear_out(&self) -> Option<GearOut> {
        if !self.gear_ready() {
            return None;
        }
        let tx = self.gear.tx.clone();
        let (refused, full) = (self.gear.refused.clone(), self.gear.full.clone());
        let send = move |b: &[u8]| match gear_msg(b) {
            Some(m) => {
                if tx.try_send(m).is_err() {
                    full.fetch_add(1, Ordering::Relaxed);
                }
            }
            None => {
                refused.fetch_add(1, Ordering::Relaxed);
            }
        };
        Some(GearOut {
            name: GEAR_NAME.to_string(),
            send: Arc::new(send),
        })
    }

    /// Whether the auto-sample's notes can go out: the jacks are open and
    /// sending is on.
    fn gear_ready(&self) -> bool {
        self.sending && self.is_open()
    }

    /// Once a period: the Sample tab's MIDI out ([`MidiOut::gear_out`])
    /// when it came or went since the last call (None inside: there is
    /// none now), to hand to `SamplePage::set_gear`. Allocates only then.
    pub fn gear_changed(&mut self) -> Option<Option<GearOut>> {
        let ready = self.gear_ready();
        if self.gear.told == Some(ready) {
            return None;
        }
        self.gear.told = Some(ready);
        Some(self.gear_out())
    }

    /// The auto-sample's waiting notes, into the jacks' queue at the start
    /// of this period (at most [`GEAR_QUEUE`]). Never waits.
    fn gear_period(&mut self) {
        let din = usize::from(DIN.0);
        for _ in 0..GEAR_QUEUE {
            let Ok(m) = self.gear.rx.try_recv() else {
                break;
            };
            let msg = match m {
                GearMsg::End => {
                    self.gear_off(0);
                    continue;
                }
                GearMsg::Note(msg) => msg,
            };
            if !self.gear_ready() {
                self.gear.unsent += 1;
                continue;
            }
            match msg {
                Msg::NoteOn { ch, note, .. } => {
                    self.gear.held[usize::from(ch & 0x0F)] |= 1u128 << note;
                }
                Msg::NoteOff { ch, note, .. } => {
                    self.gear.held[usize::from(ch & 0x0F)] &= !(1u128 << note);
                }
                _ => {}
            }
            self.queue[din].push(Timed::msg(0, msg).from_sampler());
            self.gear.sent += 1;
        }
    }

    /// Ends the auto-sample's notes still on, at `offset`.
    fn gear_off(&mut self, offset: u32) {
        let din = usize::from(DIN.0);
        for ch in 0..16u8 {
            let mut held = std::mem::take(&mut self.gear.held[usize::from(ch)]);
            while held != 0 {
                let note = held.trailing_zeros() as u8;
                held &= held - 1;
                let off = Msg::NoteOff { ch, note, vel: 64 };
                self.queue[din].push(Timed::msg(offset, off).from_sampler());
            }
        }
    }

    /// The transport's period as MIDI clock on each port, by its "Clock
    /// out" setting. Call it once a period after `TimeHost::advance`.
    pub fn clock(&mut self, block: &Block) {
        for i in 0..self.ports.len() {
            let p = &mut self.ports[i];
            if p.writer.is_none() {
                continue;
            }
            let q = &mut self.queue[i];
            self.clock_msgs.clear();
            p.clock.period(block, &mut self.clock_msgs, q);
            for m in self.clock_msgs.as_slice() {
                q.push(Timed::raw(m.offset, m.bytes()));
            }
        }
    }

    /// After the notes: each port's messages, in frame order, go to its
    /// writer, each due when its frame is heard less the port's gear
    /// delay. `t0` is when the period was made (the loop's start); the
    /// sound is heard `lead` later. A port with no writer drops what it
    /// was given.
    pub fn period_end(&mut self, frames: u32, t0: Instant) {
        self.period = frames.max(1);
        self.gear_period();
        let at = t0 + self.lead;
        let rate = f64::from(self.rate);
        self.sent.clear();
        self.sent_at = at;
        for i in 0..self.ports.len() {
            let p = &mut self.ports[i];
            let q = &mut self.queue[i];
            // The port page's test note ends in this period, or later.
            if let Some((ch, left)) = p.test_off {
                if left < frames {
                    q.push(Timed::msg(left, test_off(ch)).from_page());
                    p.test_off = None;
                } else {
                    p.test_off = Some((ch, left - frames));
                }
            }
            let din = i == usize::from(DIN.0);
            if q.is_empty() && !din {
                continue;
            }
            // The clock went in first (`clock` runs before the notes), in
            // the order it must leave; nudged, it leaves that much later
            // or earlier than the notes on its frame.
            q.sort_by_frame();
            let nudge = i64::from(p.nudge_tenths) * i64::from(self.rate) / 10_000;
            if nudge != 0 {
                q.sort_by_due(|m| if m.is_clock() { nudge } else { 0 });
            }
            let early = p.early;
            let writer = p.writer.as_ref();
            let sent = &mut self.sent;
            let (mut queued, mut full) = (0u64, 0u64);
            let mut each = |m: &Timed| {
                let Some(w) = writer else { return };
                let frames = i64::from(m.offset) + if m.is_clock() { nudge } else { 0 };
                let heard = if frames >= 0 {
                    at + Duration::from_secs_f64(frames as f64 / rate)
                } else {
                    at.checked_sub(Duration::from_secs_f64(-frames as f64 / rate))
                        .unwrap_or(t0)
                };
                // The gear's delay: each message leaves that much before
                // its frame is heard, never before the period was made.
                let leave = leave_at(heard, early, t0);
                let wire = if nudge != 0 && m.is_clock() {
                    Wire::nudged(leave, m.bytes())
                } else {
                    Wire::new(leave, m.bytes())
                };
                if w.send(wire) {
                    queued += 1;
                    if sent.len() < SENT {
                        sent.push((PortId(i as u8), *m));
                    }
                } else {
                    full += 1;
                }
            };
            if din {
                // Paced every period, so the wire's backlog drains.
                self.pacer.pace(q, frames, |m, _| each(m));
            } else {
                for m in q.as_slice() {
                    each(m);
                }
            }
            q.clear();
            p.queued += queued;
            p.full += full;
        }
        // A new period: each control may send once again.
        self.controls.now = self.controls.now.wrapping_add(1);
        let mut tracks = std::mem::take(&mut self.controls.touched);
        while tracks != 0 {
            let t = tracks.trailing_zeros() as usize;
            tracks &= tracks - 1;
            self.controls.sent[t] = [0; CTL_WORDS];
            if self.controls.waiting[t] != [0; CTL_WORDS] {
                self.controls.touched |= 1 << t;
            }
        }
    }

    /// The last period's messages the writers took, each with when it is
    /// due to leave, its port and the track that sent it (none: MPFree's
    /// clock, or [`Timed::is_from_page`]), for the MIDI monitor.
    pub fn sent(&self) -> impl Iterator<Item = (Instant, PortId, &Timed)> + '_ {
        let rate = f64::from(self.rate);
        self.sent.iter().map(move |(port, m)| {
            let due = self.sent_at + Duration::from_secs_f64(f64::from(m.offset) / rate);
            (due, *port, m)
        })
    }

    /// Messages dropped so far at `port` because its writer's queue was
    /// full.
    pub fn dropped_full(&self, port: PortId) -> u64 {
        self.ports.get(usize::from(port.0)).map_or(0, |p| p.full)
    }

    /// The writers' log lines (the first messages sent, ports opened,
    /// failures), and which ports stopped now (to `stopped`). A port whose
    /// writer stopped is let go: its tracks play on the pads again and its
    /// row says so. Its notes are forgotten: nothing reaches it.
    pub fn tick(&mut self, log: &mut dyn FnMut(String), stopped: &mut dyn FnMut(PortId)) {
        for l in self.said.drain(..) {
            log(l);
        }
        for i in 0..self.ports.len() {
            let p = &mut self.ports[i];
            if p.closing.as_ref().is_some_and(|c| c.tick(log)) {
                if let Some(c) = p.closing.take() {
                    c.let_go();
                }
            }
            let Some(w) = &p.writer else { continue };
            // `gone` first, then the lines: a port that fails between the
            // two still has its failure's line read before the writer
            // goes.
            let gone = w.gone();
            w.log_lines(log);
            if !gone {
                continue;
            }
            p.lost = true;
            if i == usize::from(DIN.0) {
                log(
                    "MIDI Out stopped: tracks set to play on MIDI play on the pads again"
                        .to_string(),
                );
            } else {
                log(format!(
                    "{} stopped: tracks set to play on it play on the pads again",
                    p.label(i)
                ));
            }
            if let Some(w) = p.writer.take() {
                w.let_go();
            }
            // Nothing can reach the port now; forget what was on.
            self.forget(i);
            stopped(PortId(i as u8));
        }
    }

    /// Ends the notes that are on (and the clock, with Stop) on every
    /// port, lets the writers send them and stops them, waiting at most a
    /// second each. True when everything queued was written or there was
    /// nothing to close.
    pub fn close(&mut self) -> bool {
        let mut done = true;
        for p in self.ports.iter_mut() {
            done &= p.closing.take().is_none_or(Closing::finish);
        }
        self.sending = false;
        if self.ports.iter().all(|p| p.writer.is_none()) {
            return done;
        }
        self.wind_down();
        for p in self.ports.iter_mut() {
            done &= p.writer.take().is_none_or(Writer::finish);
        }
        self.shown_on = None;
        done
    }

    /// Prefs turned sending off: as [`MidiOut::close`], but the sound loop
    /// never waits: each writer is kept aside until its last messages have
    /// left ([`Closing`]; [`MidiOut::tick`] lets it go).
    pub fn switch_off(&mut self) {
        self.sending = false;
        if self.ports.iter().all(|p| p.writer.is_none()) {
            return;
        }
        self.wind_down();
        let lead = self.lead;
        for p in self.ports.iter_mut() {
            if let Some(w) = p.writer.take() {
                if let Some(old) = p.closing.replace(Closing::new(w, lead)) {
                    old.let_go();
                }
            }
        }
        self.shown_on = None;
    }

    /// The test notes, the notes on and the clock end on every port, handed
    /// to the writers.
    fn wind_down(&mut self) {
        for i in 0..self.ports.len() {
            if let Some((ch, _)) = self.ports[i].test_off.take() {
                self.queue[i].push(Timed::msg(0, test_off(ch)).from_page());
            }
        }
        self.all_off(0);
        // The other devices stop with us.
        for i in 0..self.ports.len() {
            self.clock_msgs.clear();
            self.ports[i].clock.stop(&mut self.clock_msgs);
            for m in self.clock_msgs.as_slice() {
                self.queue[i].push(Timed::raw(m.offset, m.bytes()));
            }
        }
        self.period_end(0, Instant::now() - self.lead);
    }

    /// The run's counts, for the log at exit.
    pub fn summary(&self) -> Vec<String> {
        let mut lines = Vec::new();
        for (i, p) in self.ports.iter().enumerate() {
            let label = p.label(i);
            if let Some(w) = &p.writer {
                lines.push(format!(
                    "{label}: {} messages written, {} dropped (writer's queue full), {} dropped \
                     (port took nothing for {} ms); the latest left {:.1} ms after its frame",
                    w.written(),
                    p.full,
                    w.stalled(),
                    STALL.as_millis(),
                    w.max_late_ms(),
                ));
            } else if p.queued > 0 || p.lost {
                lines.push(format!(
                    "{label}: {} messages queued before the port stopped",
                    p.queued
                ));
            }
        }
        if self.ports[usize::from(DIN.0)].queued > 0 {
            let fpb = self.pacer.frames_per_byte();
            lines.push(format!(
                "MIDI Out: the 5-pin wire (31250 bit/s, {:.2} ms a byte) would leave a message \
                 at most {:.1} ms after its frame",
                fpb * 1000.0 / f64::from(self.rate),
                self.pacer.stats.max_late * 1000.0 / f64::from(self.rate),
            ));
        }
        let dropped = self.router.dropped + self.queue.iter().map(|q| q.dropped).sum::<u64>();
        if dropped > 0 {
            lines.push(format!(
                "MIDI out: {dropped} messages dropped (more than {} for one port in one period)",
                mpfree_midi::output::QUEUE
            ));
        }
        if self.thru_unsent > 0 {
            lines.push(format!(
                "Thru: {} messages not sent (MIDI out off, or no output to send them to)",
                self.thru_unsent
            ));
        }
        if self.sent_back > 0 {
            lines.push(format!(
                "MIDI Follow: {} values sent back to its controller",
                self.sent_back
            ));
        }
        if self.controller_unsent > 0 {
            lines.push(format!(
                "Controller mode: {} messages not sent (MIDI out off, or no output to send them to)",
                self.controller_unsent
            ));
        }
        if self.controls.messages > 0 {
            lines.push(format!(
                "Tracks' controllers, bend and pressure (locks, lanes, macros): {} messages sent",
                self.controls.messages
            ));
        }
        let g = &self.gear;
        let (refused, full) = (
            g.refused.load(Ordering::Relaxed),
            g.full.load(Ordering::Relaxed),
        );
        if g.sent + g.unsent + refused + full > 0 {
            lines.push(format!(
                "Auto-sample: its notes to the gear on MIDI Out: {} sent, {} not sent (MIDI out \
                 off), {refused} refused (not a note on or off), {full} dropped (the queue was full)",
                g.sent, g.unsent
            ));
        }
        if self.busy > 0 {
            lines.push(format!(
                "MIDI Out: {} periods kept the old settings while the screen held them",
                self.busy
            ));
        }
        lines
    }
}

impl Drop for MidiOut {
    fn drop(&mut self) {
        self.close();
    }
}

/// "note on, ch 10, C#2, velocity 100", as the log shows a message.
fn describe(b: &[u8]) -> String {
    let at = |i: usize| b.get(i).copied().unwrap_or(0);
    let Some(m) = Msg::from_bytes(at(0), at(1), at(2)) else {
        return "not a channel message".to_string();
    };
    let ch = m.channel() + 1;
    let name = mpfree_midi_ui::note_name;
    match m {
        Msg::NoteOn { note, vel, .. } => {
            format!("note on, ch {ch}, {}, velocity {vel}", name(note))
        }
        Msg::NoteOff { note, .. } => format!("note off, ch {ch}, {}", name(note)),
        Msg::PolyPressure { note, value, .. } => {
            format!("pressure, ch {ch}, {}, {value}", name(note))
        }
        Msg::Cc { cc, value, .. } => format!("CC {cc}, ch {ch}, value {value}"),
        Msg::Program { program, .. } => format!("program {}, ch {ch}", u16::from(program) + 1),
        Msg::ChanPressure { value, .. } => format!("channel pressure, ch {ch}, {value}"),
        Msg::PitchBend { value, .. } => format!("pitch bend, ch {ch}, {value}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_midi::setup::MidiSetup;
    use mpfree_play::ALL;
    use std::sync::{Arc, Mutex};
    use std::thread;

    const RATE: u32 = 44_100;
    const PERIOD: u32 = 128;

    /// The port: every write with when it came.
    #[derive(Clone, Default)]
    struct Sink(Arc<Mutex<Writes>>);

    /// Each write, with when it came.
    type Writes = Vec<(Instant, Vec<u8>)>;

    impl Write for Sink {
        fn write(&mut self, b: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().push((Instant::now(), b.to_vec()));
            Ok(b.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl Sink {
        fn bytes(&self) -> Vec<u8> {
            self.0
                .lock()
                .unwrap()
                .iter()
                .flat_map(|(_, b)| b.clone())
                .collect()
        }
    }

    /// A port that fails, as an unplugged one does.
    struct Broken;

    impl Write for Broken {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::Error::from_raw_os_error(19)) // ENODEV
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn model(tracks: &[(usize, TrackOut)]) -> Shared {
        let mut m = IoModel {
            setup: MidiSetup::default(),
            ..IoModel::default()
        };
        for (t, o) in tracks {
            m.setup.tracks[*t] = Some(o.clone());
        }
        m.gen = 1;
        Arc::new(Mutex::new(m))
    }

    fn open(lead: Duration) -> (MidiOut, Sink) {
        let mut mo = MidiOut::new(RATE, lead);
        let sink = Sink::default();
        mo.open(sink.clone(), Instant::now()).unwrap();
        (mo, sink)
    }

    fn ev(track: u16, offset: u32, event: Event) -> Out {
        Out {
            offset,
            frame: u64::from(offset),
            track,
            dest: Default::default(),
            event,
        }
    }

    fn on(note: u8, velocity: u8) -> Event {
        Event::NoteOn { note, velocity }
    }

    /// A kit track on the jacks, channel 10: its pads go out as the MPC's
    /// pad notes and the pads don't play them; another track isn't taken.
    #[test]
    fn a_kit_track_plays_the_mpcs_pad_notes_on_its_channel() {
        let (mut mo, sink) = open(Duration::ZERO);
        let m = model(&[(1, TrackOut::new(DIN_NAME, 9))]);
        mo.sync(&m, false);
        assert!(m.lock().unwrap().live.built.notes_out);
        assert!(mo.takes(1));
        assert!(!mo.takes(0));
        let hit = ev(1, 10, on(0, 100));
        assert!(mo.send(&hit, NoteIs::Pad(kit_notes(0))));
        let other = ev(0, 10, on(0, 100));
        assert!(!mo.send(&other, NoteIs::Pad(kit_notes(0))));
        let off = ev(1, 90, Event::NoteOff { note: 0 });
        assert!(mo.send(&off, NoteIs::Pad(kit_notes(0))));
        mo.period_end(PERIOD, Instant::now());
        assert!(mo.close());
        // Pad 1 is C#2, note 37 (0x25), on channel 10.
        assert_eq!(sink.bytes(), [0x99, 0x25, 100, 0x89, 0x25, 64]);
    }

    /// A kit track on a drum machine's port: each pad plays what the
    /// port's device profile says, on a channel of its own when it gives
    /// one, else its own note on the track's channel, in every bank. A
    /// note ends where it started, though the profile changed meanwhile.
    #[test]
    fn a_kit_pad_plays_what_the_drum_machines_profile_says() {
        use mpfree_midi::profile::Profile;
        use mpfree_midi::setup::ProfilePick;
        let (mut mo, sink) = open(Duration::ZERO);
        let m = model(&[(1, TrackOut::new(DIN_NAME, 9))]);
        let pick = |m: &Shared, to: ProfilePick| {
            let mut g = m.lock().unwrap();
            g.setup.set_port(DIN_NAME, |p| p.profile = to.clone());
            g.gen += 1;
        };
        {
            let mut p = Profile::new("Drum box");
            let out = |note, channel| PadOut { note, channel };
            p.set_pad(0, out(36, Some(2)));
            p.set_pad(17, out(40, None));
            m.lock().unwrap().profiles.add(p).unwrap();
        }
        pick(&m, ProfilePick::Named("Drum box".into()));
        mo.sync(&m, false);
        // A1, B2, and B5, which the profile leaves to its own note.
        assert!(mo.send(&ev(1, 0, on(0, 100)), NoteIs::Kit));
        assert!(mo.send(&ev(1, 1, on(17, 90)), NoteIs::Kit));
        assert!(mo.send(&ev(1, 2, on(20, 80)), NoteIs::Kit));
        // No profile now: A1's end still goes to 36 on channel 3, and its
        // next hit is its own note, on the track's channel.
        pick(&m, ProfilePick::Off);
        mo.sync(&m, false);
        assert!(mo.send(&ev(1, 3, Event::NoteOff { note: 0 }), NoteIs::Kit));
        assert!(mo.send(&ev(1, 4, on(0, 70)), NoteIs::Kit));
        mo.period_end(PERIOD, Instant::now());
        // Closing ends what is still on, on the track's channel.
        assert!(mo.close());
        let b5 = KIT_NOTES[20];
        assert_eq!(b5, 52);
        assert_eq!(
            sink.bytes(),
            [
                0x92, 36, 100, 0x99, 40, 90, 0x99, b5, 80, 0x82, 36, 64, 0x99, 37, 70, 0x89, 37,
                64, 0x89, 40, 64, 0x89, b5, 64
            ]
        );
    }

    /// A kit pad on a port of its own (the Pads tab's MIDI out pad) plays
    /// its note there whether its track has a Plays on or not; its
    /// pressure and end follow it, a change of the track's Plays on leaves
    /// it on, and the track's pads' end and closing end it.
    #[test]
    fn a_pad_on_a_port_of_its_own_plays_there_and_ends_there() {
        let (mut mo, sink) = open(Duration::ZERO);
        let m = model(&[(1, TrackOut::new(DIN_NAME, 9))]);
        mo.sync(&m, false);
        let pressure = |pad, value| Event::Pressure { note: pad, value };
        // Track 2's pad 13 plays 36 on channel 3; track 3, with no Plays
        // on, plays 40 on channel 1.
        assert!(mo.pad_to(&ev(1, 0, on(12, 100)), DIN_NAME, 2, 36));
        assert!(mo.pad_to(&ev(2, 0, on(0, 90)), DIN_NAME, 0, 40));
        assert!(mo.pad_to(&ev(1, 1, pressure(12, 50)), DIN_NAME, 2, 36));
        // Pressed again while on: the note ends first.
        assert!(mo.pad_to(&ev(1, 2, on(12, 80)), DIN_NAME, 2, 36));
        // Its end goes where its on went, even if the pad changed since.
        assert!(mo.pad_to(&ev(1, 3, Event::NoteOff { note: 12 }), DIN_NAME, 5, 99));
        assert!(!mo.pad_to(&ev(1, 3, Event::NoteOff { note: 12 }), DIN_NAME, 2, 36));
        // A port not plugged in sends nothing.
        assert!(!mo.pad_to(&ev(1, 4, on(3, 100)), "Volca Kick", 9, 36));
        // Track 3's pads end.
        mo.end_pads(2, 5);
        // Track 2's Plays on changes: its pad's note stays on until closing.
        assert!(mo.pad_to(&ev(1, 6, on(5, 70)), DIN_NAME, 2, 38));
        m.lock().unwrap().setup.tracks[1] = Some(TrackOut::new(DIN_NAME, 0));
        m.lock().unwrap().gen += 1;
        mo.sync(&m, false);
        mo.period_end(PERIOD, Instant::now());
        assert!(mo.close());
        assert_eq!(
            sink.bytes(),
            [
                0x92, 36, 100, 0x90, 40, 90, 0xA2, 36, 50, 0x82, 36, 64, 0x92, 36, 80, 0x82, 36,
                64, 0x80, 40, 64, 0x92, 38, 70, 0x82, 38, 64
            ]
        );
    }

    /// A keyboard's pedals and mod wheel go to a track's gear as they
    /// came, on its channel; arming another track lets the sustain pedal
    /// up there, once. A track not on MIDI out isn't taken.
    #[test]
    fn the_keys_pedals_and_mod_wheel_reach_the_gear_and_come_up() {
        let (mut mo, sink) = open(Duration::ZERO);
        let m = model(&[(1, TrackOut::new(DIN_NAME, 2))]);
        mo.sync(&m, false);
        assert!(!mo.keys_control(0, Ctl::Sustain, 1.0), "not on MIDI out");
        assert!(mo.keys_control(1, Ctl::Sustain, 1.0));
        assert!(mo.keys_control(1, Ctl::ModWheel, 0.5));
        assert!(mo.keys_control(1, Ctl::Sostenuto, 1.0));
        assert!(mo.keys_control(1, Ctl::Sostenuto, 0.0));
        assert!(mo.keys_control(1, Ctl::Breath, f32::NAN), "taken, not sent");
        mo.rest_expression(1);
        mo.rest_expression(1);
        mo.period_end(PERIOD, Instant::now());
        assert!(mo.close());
        assert_eq!(
            sink.bytes(),
            [
                // Sustain down, the wheel halfway, sostenuto down and up,
                // on channel 3.
                0xB2, 64, 127, 0xB2, 1, 64, 0xB2, 66, 127, 0xB2, 66, 0,
                // Another track armed: sustain up, once; the wheel stays.
                0xB2, 64, 0,
            ]
        );
    }

    /// Expression out: a track's keys bend and press its gear, as one
    /// bend over its profile's range, the hardest key's channel pressure
    /// and the newest key's tone on CC 74; a panic puts the gear back at
    /// rest, keeping the wheel, as closing does; a profile that says poly
    /// gets each key's pressure on its note. A track not on MIDI out isn't
    /// taken.
    #[test]
    fn the_keys_bend_and_press_the_gear() {
        use mpfree_midi::expr_out::In;
        use mpfree_midi::profile::Profile;
        use mpfree_midi::setup::ProfilePick;
        let (mut mo, sink) = open(Duration::ZERO);
        let m = model(&[(1, TrackOut::new(DIN_NAME, 2))]);
        let pick = |m: &Shared, p: Profile| {
            let mut g = m.lock().unwrap();
            let name = p.name.clone();
            g.profiles.add(p).unwrap();
            g.setup
                .set_port(DIN_NAME, |p| p.profile = ProfilePick::Named(name.clone()));
            g.gen += 1;
        };
        let mut juno = Profile::new("Juno");
        juno.bend = Some(12);
        pick(&m, juno);
        mo.sync(&m, false);
        assert!(!mo.expression(0, In::Wheel(0.5)), "not on MIDI out");
        assert!(mo.expression(1, In::Wheel(0.5)));
        assert!(mo.expression(1, In::Down { note: 60 }));
        assert!(mo.expression(
            1,
            In::Slide {
                note: 60,
                semis: 3.0
            }
        ));
        assert!(mo.expression(
            1,
            In::Pressure {
                note: 60,
                value: 50
            }
        ));
        assert!(mo.expression(
            1,
            In::Tone {
                note: 60,
                value: 0.5
            }
        ));
        mo.all_off(0);
        mo.period_end(PERIOD, Instant::now());
        let mut osmose = Profile::new("Osmose");
        osmose.poly_pressure = true;
        pick(&m, osmose);
        mo.sync(&m, false);
        assert!(mo.expression(1, In::Down { note: 62 }));
        assert!(mo.expression(
            1,
            In::Pressure {
                note: 62,
                value: 70
            }
        ));
        mo.period_end(PERIOD, Instant::now());
        assert!(mo.close());
        assert_eq!(
            sink.bytes(),
            [
                // The wheel halfway (4096), then a quarter of the 12
                // semitones more (6144), on channel 3.
                0xE2, 0, 96, 0xE2, 0, 112, // The pressure, and the tone on CC 74.
                0xD2, 50, 0xB2, 74, 64, // The panic: bend and pressure at rest.
                0xE2, 0, 64, 0xD2, 0, // The wheel still held bends the next key.
                0xE2, 0, 96, // Each key's own pressure, on its note.
                0xA2, 62, 70, // Closing puts the bend back at rest too.
                0xE2, 0, 64
            ]
        );
    }

    /// Expression out to MPE gear (its profile says `mpe = lower 15`): the
    /// zone goes out first, the track's channel is the master whatever its
    /// Plays on says, each note takes a member channel with its own slide
    /// (sent just ahead of it when it came first) and pressure, the wheel
    /// bends the master, and closing puts every channel at rest and ends
    /// the note still on where it started.
    #[test]
    fn notes_on_mpe_gear_take_a_member_channel_each() {
        use mpfree_midi::expr_out::In;
        use mpfree_midi::profile::Profile;
        use mpfree_midi::setup::ProfilePick;
        let (mut mo, sink) = open(Duration::ZERO);
        let m = model(&[(1, TrackOut::new(DIN_NAME, 5))]);
        {
            let mut g = m.lock().unwrap();
            let mut p = Profile::new("MPE synth");
            p.mpe = Some(Zone::Lower(15));
            g.profiles.add(p).unwrap();
            g.setup.set_port(DIN_NAME, |p| {
                p.profile = ProfilePick::Named("MPE synth".into())
            });
            g.gen += 1;
        }
        mo.sync(&m, false);
        // A slide that comes before its note waits for it.
        assert!(mo.expression(
            1,
            In::Slide {
                note: 60,
                semis: 12.0
            }
        ));
        assert!(mo.send(&ev(1, 0, on(60, 100)), NoteIs::Note));
        assert!(mo.send(&ev(1, 0, on(64, 90)), NoteIs::Note));
        assert!(mo.expression(
            1,
            In::Pressure {
                note: 64,
                value: 80
            }
        ));
        assert!(mo.expression(1, In::Wheel(0.5)));
        assert!(mo.send(&ev(1, 0, Event::NoteOff { note: 60 }), NoteIs::Note));
        mo.period_end(PERIOD, Instant::now());
        assert!(mo.close());
        assert_eq!(
            sink.bytes(),
            [
                // The zone: RPN 6 = 15 members on channel 1, then the null RPN.
                0xB0, 101, 0, 0xB0, 100, 6, 0xB0, 6, 15, 0xB0, 101, 127, 0xB0, 100, 127,
                // C4 on channel 2, its 12 of 48 semitones' slide first.
                0xE1, 0, 80, 0x91, 60, 100, // E4 on channel 3, and its pressure.
                0x92, 64, 90, 0xD2, 80, // The wheel on the master channel.
                0xE0, 0, 96, // C4 ends on channel 2.
                0x81, 60, 64, // Closing: the master, then each member at rest.
                0xE0, 0, 64, 0xE1, 0, 64, 0xD2, 0, // E4 ends where it started.
                0x82, 64, 64
            ]
        );
    }

    /// The MIDI monitor gets what the writer took, each with when it is
    /// due and the track that sent it; the clock has none. A period that
    /// sends nothing leaves the list empty.
    #[test]
    fn the_monitor_gets_what_went_out_and_who_sent_it() {
        let (mut mo, _sink) = open(Duration::from_millis(10));
        let m = model(&[(1, TrackOut::new(DIN_NAME, 9))]);
        mo.sync(&m, false);
        // A pulse of the clock, as `MidiOut::clock` queues it.
        mo.queue[0].push(Timed::realtime(0, mpfree_midi_clock::CLOCK));
        mo.send(&ev(1, 441, on(0, 100)), NoteIs::Pad(kit_notes(0)));
        let t0 = Instant::now();
        mo.period_end(PERIOD, t0);
        let sent: Vec<(Duration, Vec<u8>, Option<u8>)> = mo
            .sent()
            .map(|(due, port, m)| {
                assert_eq!(port, DIN);
                (due - t0, m.bytes().to_vec(), m.track())
            })
            .collect();
        assert_eq!(
            sent,
            [
                (Duration::from_millis(10), vec![0xF8], None),
                (Duration::from_millis(20), vec![0x99, 0x25, 100], Some(1)),
            ]
        );
        mo.period_end(PERIOD, Instant::now());
        assert_eq!(mo.sent().count(), 0);
        assert!(mo.close());
    }

    /// While MIDI out is off, a routed track plays on the pads and the I/O
    /// view is told MIDI out is off.
    #[test]
    fn closed_takes_nothing_and_says_so() {
        let mut mo = MidiOut::new(RATE, Duration::ZERO);
        let m = model(&[(0, TrackOut::new(DIN_NAME, 0))]);
        m.lock().unwrap().live.built.notes_out = true;
        mo.sync(&m, false);
        assert!(!m.lock().unwrap().live.built.notes_out);
        assert!(!mo.takes(0));
        assert!(!mo.send(&ev(0, 0, on(0, 100)), NoteIs::Pad(kit_notes(0))));
    }

    /// The USB device the tests plug in, at port number 1.
    const USB: PortId = PortId(1);

    /// Opens `sink` as a USB device's output, counting the opens.
    fn opener(sink: &Sink, opens: &Arc<std::sync::atomic::AtomicUsize>) -> Opener {
        let (s, n) = (sink.clone(), opens.clone());
        Arc::new(move || {
            n.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(Box::new(s.clone()) as Box<dyn Write + Send>)
        })
    }

    /// `m` with the Digitone plugged in, as the MIDI in side lists it.
    fn with_digitone(m: &Shared) {
        m.lock().unwrap().live.ports.push(mpfree_midi_ui::PortLive {
            name: "Digitone".into(),
            kind: "USB".into(),
            about: "USB".into(),
            has_in: true,
            has_out: true,
            sending: false,
            last_in: None,
            clock_measured: None,
        });
    }

    /// Waits until `f` holds, for at most two seconds.
    fn wait(mut f: impl FnMut() -> bool) -> bool {
        let give_up = Instant::now() + Duration::from_secs(2);
        while !f() && Instant::now() < give_up {
            thread::sleep(Duration::from_millis(1));
        }
        f()
    }

    /// A track set to a USB device plays there, its program first; the
    /// jacks get nothing of it. The device's row hears MIDI out sends to
    /// it, and the monitor gets each message with its port.
    #[test]
    fn a_track_on_a_usb_device_plays_there() {
        let (mut mo, din) = open(Duration::ZERO);
        let (usb, opens) = (Sink::default(), Arc::default());
        mo.plugged(USB, "Digitone", opener(&usb, &opens));
        let m = model(&[(
            0,
            TrackOut {
                program: Some(5),
                ..TrackOut::new("Digitone", 2)
            },
        )]);
        with_digitone(&m);
        mo.sync(&m, true);
        assert!(mo.takes(0));
        assert!(m.lock().unwrap().live.ports[1].sending);
        assert!(mo.send(&ev(0, 0, on(60, 100)), NoteIs::Note));
        assert!(mo.send(&ev(0, 64, Event::NoteOff { note: 60 }), NoteIs::Note));
        mo.period_end(PERIOD, Instant::now());
        let ports: Vec<PortId> = mo.sent().map(|(_, p, _)| p).collect();
        assert_eq!(ports, [USB, USB, USB]);
        assert!(mo.close());
        assert_eq!(usb.bytes(), [0xC2, 5, 0x92, 60, 100, 0x82, 60, 64]);
        assert!(din.bytes().is_empty());
        assert_eq!(opens.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    /// A device plugged in while MIDI out is off is not opened; switching
    /// on opens it in its writer's thread, and switching off and on again
    /// at once takes the same writer back rather than opening it twice.
    #[test]
    fn a_usb_device_opens_only_while_sending_is_on() {
        use std::sync::atomic::Ordering::SeqCst;
        let mut mo = MidiOut::new(RATE, Duration::ZERO);
        let (usb, opens) = (Sink::default(), Arc::default());
        mo.plugged(USB, "Digitone", opener(&usb, &opens));
        let m = model(&[(0, TrackOut::new("Digitone", 0))]);
        mo.sync(&m, false);
        thread::sleep(Duration::from_millis(20));
        assert_eq!(opens.load(SeqCst), 0);
        assert!(!mo.takes(0));
        mo.switch_on(Instant::now());
        assert!(mo.sending());
        assert!(mo.takes(0));
        assert!(wait(|| opens.load(SeqCst) == 1));
        let mut lines = Vec::new();
        assert!(wait(|| {
            mo.tick(&mut |l| lines.push(l), &mut |_| {});
            !lines.is_empty()
        }));
        assert_eq!(lines, ["MIDI out to Digitone: open"]);
        mo.switch_off();
        assert!(!mo.takes(0));
        mo.switch_on(Instant::now());
        assert!(mo.takes(0));
        let mut lines = Vec::new();
        mo.tick(&mut |l| lines.push(l), &mut |_| {});
        assert_eq!(
            lines,
            ["MIDI out to Digitone: on again (Prefs); its output was still open"]
        );
        assert!(mo.close());
        assert_eq!(opens.load(SeqCst), 1);
    }

    /// Unplugged, a device can't be sent its note-offs: its notes are
    /// forgotten and its track plays on the pads again.
    #[test]
    fn unplugging_forgets_its_notes_and_its_tracks_play_on_the_pads() {
        let (mut mo, din) = open(Duration::ZERO);
        let (usb, opens) = (Sink::default(), Arc::default());
        mo.plugged(USB, "Digitone", opener(&usb, &opens));
        let m = model(&[(0, TrackOut::new("Digitone", 0))]);
        mo.sync(&m, true);
        assert!(mo.send(&ev(0, 0, on(60, 100)), NoteIs::Note));
        mo.period_end(PERIOD, Instant::now());
        assert!(wait(|| usb.bytes().len() == 3));
        mo.unplugged(USB);
        assert_eq!(mo.router.sounding(), 0);
        mo.sync(&m, true);
        assert!(!mo.takes(0));
        assert!(!mo.send(&ev(0, 0, Event::NoteOff { note: 60 }), NoteIs::Note));
        mo.period_end(PERIOD, Instant::now());
        assert!(mo.close());
        assert_eq!(usb.bytes(), [0x90, 60, 100]);
        assert!(din.bytes().is_empty());
        // Plugged in again: it plays there again.
        mo.switch_on(Instant::now());
        mo.plugged(USB, "Digitone", opener(&usb, &opens));
        mo.sync(&m, true);
        assert!(mo.takes(0));
    }

    /// An output that won't open is said in the log, the port is let go,
    /// its track plays on the pads and its row says nothing goes there.
    #[test]
    fn a_usb_output_that_wont_open_is_said_and_let_go() {
        let (mut mo, _din) = open(Duration::ZERO);
        let busy: Opener = Arc::new(|| Err(io::Error::from_raw_os_error(16)));
        mo.plugged(USB, "Digitone", busy);
        let m = model(&[(0, TrackOut::new("Digitone", 0))]);
        with_digitone(&m);
        mo.sync(&m, false);
        let (mut lines, mut stopped) = (Vec::new(), Vec::new());
        assert!(wait(|| {
            mo.tick(&mut |l| lines.push(l), &mut |p| stopped.push(p));
            !stopped.is_empty()
        }));
        assert_eq!(stopped, [USB]);
        assert!(
            lines[0].starts_with("MIDI out to Digitone: could not open its output: "),
            "{lines:?}"
        );
        assert_eq!(
            lines[1],
            "MIDI out to Digitone stopped: tracks set to play on it play on the pads again"
        );
        assert!(!mo.takes(0));
        mo.sync(&m, false);
        assert!(!m.lock().unwrap().live.ports[1].sending);
        assert!(mo
            .summary()
            .iter()
            .any(|l| l == "MIDI out to Digitone: 0 messages queued before the port stopped"));
        assert!(mo.close());
    }

    /// A USB port's page: the test note goes to it, and All notes off ends
    /// only the notes MPFree has on there; the jacks' note goes on. Each
    /// port has its own gear delay.
    #[test]
    fn a_usb_ports_sends_and_delay_are_its_own() {
        let (mut mo, din) = open(Duration::ZERO);
        let (usb, opens) = (Sink::default(), Arc::default());
        mo.plugged(USB, "Digitone", opener(&usb, &opens));
        let m = model(&[
            (0, TrackOut::new(DIN_NAME, 0)),
            (1, TrackOut::new("Digitone", 1)),
        ]);
        m.lock()
            .unwrap()
            .setup
            .set_port("Digitone", |p| p.delay_tenths = 30);
        mo.sync(&m, true);
        assert_eq!(mo.ports[1].early, Duration::from_millis(3));
        assert_eq!(mo.ports[0].early, Duration::ZERO);
        mo.send(&ev(0, 0, on(60, 100)), NoteIs::Note);
        mo.send(&ev(1, 0, on(62, 100)), NoteIs::Note);
        mo.period_end(PERIOD, Instant::now());
        m.lock().unwrap().asks.port_send = Some(PortSend {
            port: "Digitone".into(),
            what: ToPort::AllNotesOff,
        });
        mo.sync(&m, true);
        assert_eq!(
            m.lock().unwrap().live.port_said.as_deref(),
            Some("All notes off sent to Digitone on all 16 channels")
        );
        mo.period_end(PERIOD, Instant::now());
        assert!(mo
            .sent()
            .all(|(_, p, t)| p == USB && t.is_from_page() == t.track().is_none()));
        m.lock().unwrap().asks.port_send = Some(PortSend {
            port: "Digitone".into(),
            what: ToPort::TestNote { channel: 4 },
        });
        mo.sync(&m, true);
        mo.period_end(PERIOD, Instant::now());
        assert!(wait(|| usb.bytes().len() == 3 + 3 + 16 * 3 + 3));
        let b = usb.bytes();
        assert_eq!(b[..6], [0x91, 62, 100, 0x81, 62, 64]);
        assert_eq!(b[6..9], [0xB0, 123, 0]);
        assert_eq!(b[b.len() - 3..], [0x94, 60, 100]);
        assert!(wait(|| din.bytes().len() == 3));
        // The jacks' note is still on; closing ends it.
        assert_eq!(mo.router.sounding(), 1);
        assert!(mo.close());
        assert_eq!(din.bytes(), [0x90, 60, 100, 0x80, 60, 64]);
    }

    /// Clock goes to each port by its own "Clock out": here the USB device
    /// gets Start and pulses while the jacks, set to Off, get nothing.
    #[test]
    fn clock_goes_to_each_port_by_its_setting() {
        use mpfree_transport::Command;
        let (mut mo, din) = open(Duration::ZERO);
        let (usb, opens) = (Sink::default(), Arc::default());
        mo.plugged(USB, "Digitone", opener(&usb, &opens));
        let m = model(&[]);
        m.lock()
            .unwrap()
            .setup
            .set_port("Digitone", |p| p.clock_out = ClockOut::WhilePlaying);
        mo.sync(&m, false);
        assert!(mo.sends_clock());
        let mut tr = mpfree_transport::Transport::new(RATE, 120.0).unwrap();
        let mut block = Block::new();
        tr.command(tr.frame(), Command::Play);
        // A second at 120 BPM: 48 pulses.
        for _ in 0..(RATE / PERIOD) {
            tr.process(PERIOD, &mut block);
            mo.sync(&m, tr.state().rolling());
            mo.clock(&block);
            mo.period_end(PERIOD, Instant::now());
        }
        assert!(mo.close());
        let b = usb.bytes();
        assert_eq!(b[0], 0xFA, "{b:02X?}");
        let n = b.iter().filter(|&&x| x == 0xF8).count();
        assert!((47..=49).contains(&n), "{n}");
        assert_eq!(*b.last().unwrap(), 0xFC, "closing stops it");
        assert!(din.bytes().is_empty());
    }

    /// A synth pad plays its chord, and its note-off ends the same notes
    /// even when the layout changed in between; pressure reaches each.
    #[test]
    fn a_pad_chord_ends_the_notes_it_started() {
        let (mut mo, sink) = open(Duration::ZERO);
        let m = model(&[(0, TrackOut::new(DIN_NAME, 0))]);
        mo.sync(&m, false);
        mo.send(&ev(0, 0, on(3, 90)), NoteIs::Pad(&[60, 64, 67]));
        mo.send(
            &ev(0, 5, Event::Pressure { note: 3, value: 40 }),
            NoteIs::Pad(&[]),
        );
        mo.send(&ev(0, 10, Event::NoteOff { note: 3 }), NoteIs::Pad(&[61]));
        mo.period_end(PERIOD, Instant::now());
        assert!(mo.close());
        assert_eq!(
            sink.bytes(),
            [
                0x90, 60, 90, 0x90, 64, 90, 0x90, 67, 90, // the chord
                0xA0, 60, 40, 0xA0, 64, 40, 0xA0, 67, 40, // pressure
                0x80, 60, 64, 0x80, 64, 64, 0x80, 67, 64, // its end
            ]
        );
    }

    /// The arps' notes with a synth layout are MIDI notes; velocity 0
    /// still sounds (as 1), so a note-on is never read as a note-off.
    #[test]
    fn midi_notes_go_as_they_are() {
        let (mut mo, sink) = open(Duration::ZERO);
        let m = model(&[(2, TrackOut::new(DIN_NAME, 1))]);
        mo.sync(&m, false);
        mo.send(&ev(2, 0, on(72, 0)), NoteIs::Note);
        mo.send(&ev(2, 3, Event::NoteOff { note: 72 }), NoteIs::Note);
        mo.period_end(PERIOD, Instant::now());
        assert!(mo.close());
        assert_eq!(sink.bytes(), [0x91, 72, 1, 0x81, 72, 64]);
    }

    /// A changed channel ends the notes on at the old one first; notes
    /// later go to the new one.
    #[test]
    fn a_route_change_ends_the_old_notes_there() {
        let (mut mo, sink) = open(Duration::ZERO);
        let m = model(&[(0, TrackOut::new(DIN_NAME, 0))]);
        mo.sync(&m, true);
        mo.send(&ev(0, 0, on(1, 100)), NoteIs::Pad(kit_notes(1)));
        mo.period_end(PERIOD, Instant::now());
        {
            let mut g = m.lock().unwrap();
            g.setup.tracks[0] = Some(TrackOut::new(DIN_NAME, 4));
            g.gen += 1;
        }
        mo.sync(&m, true);
        mo.send(&ev(0, 0, on(1, 100)), NoteIs::Pad(kit_notes(1)));
        mo.period_end(PERIOD, Instant::now());
        assert!(mo.close());
        assert_eq!(
            sink.bytes(),
            [
                0x90, 0x24, 100, // channel 1
                0x80, 0x24, 64, // ended there by the change
                0x94, 0x24, 100, // channel 5
                0x84, 0x24, 64, // ended by close
            ]
        );
    }

    /// Bank and program go out when set, before the notes, and again when
    /// the song starts; not again while it plays.
    #[test]
    fn bank_and_program_go_when_set_and_at_each_start() {
        let (mut mo, sink) = open(Duration::ZERO);
        let mut o = TrackOut::new(DIN_NAME, 2);
        o.program = Some(4);
        o.bank = Some(129); // MSB 1, LSB 1
        let m = model(&[(0, o)]);
        mo.sync(&m, false);
        mo.period_end(PERIOD, Instant::now());
        let program = [0xB2, 0, 1, 0xB2, 32, 1, 0xC2, 4];
        // The song starts: again, ahead of the first note.
        mo.sync(&m, true);
        mo.send(&ev(0, 0, on(0, 100)), NoteIs::Pad(kit_notes(0)));
        mo.period_end(PERIOD, Instant::now());
        // Still playing: not again.
        mo.sync(&m, true);
        mo.period_end(PERIOD, Instant::now());
        assert!(mo.close());
        let mut want = program.to_vec();
        want.extend(program);
        want.extend([0x92, 0x25, 100, 0x82, 0x25, 64]);
        assert_eq!(sink.bytes(), want);
    }

    /// A clip's program change (S5) goes after the track's own at the
    /// song's start, on the track's channel: bank select when it has one,
    /// then the program; a track on the pads leaves it to them.
    #[test]
    fn a_clips_program_goes_after_the_tracks_own() {
        let (mut mo, sink) = open(Duration::ZERO);
        let mut o = TrackOut::new(DIN_NAME, 2);
        o.program = Some(4);
        let m = model(&[(0, o)]);
        mo.sync(&m, true);
        let clip = |bank| Event::Program { program: 9, bank };
        assert!(mo.send(&ev(0, 0, clip(Some(258))), NoteIs::Pad(&[])));
        assert!(mo.send(&ev(0, 0, on(0, 100)), NoteIs::Pad(kit_notes(0))));
        assert!(mo.send(&ev(0, 10, clip(None)), NoteIs::Note));
        assert!(!mo.send(&ev(1, 0, clip(None)), NoteIs::Note), "on the pads");
        mo.period_end(PERIOD, Instant::now());
        assert!(mo.close());
        let mut want = vec![0xC2, 4, 0xB2, 0, 2, 0xB2, 32, 2, 0xC2, 9];
        want.extend([0x92, 0x25, 100, 0xC2, 9, 0x82, 0x25, 64]);
        assert_eq!(sink.bytes(), want);
    }

    fn ctl(t: u8, id: u16) -> ParamAddr {
        ParamAddr::new(Owner::Midi(t), id)
    }

    /// A track's controller, bend and pressure (S6's CC lanes) go out on
    /// its channel on their frames, rounded and kept in range, a step's
    /// ahead of the note on its frame; a channel mode message never.
    #[test]
    fn a_tracks_controls_go_on_its_channel_ahead_of_its_note() {
        let (mut mo, sink) = open(Duration::ZERO);
        let m = model(&[(0, TrackOut::new(DIN_NAME, 2))]);
        mo.sync(&m, true);
        assert!(mo.send(&ev(0, 10, on(0, 100)), NoteIs::Pad(kit_notes(0))));
        mo.control(10, ctl(0, 74), 20.4);
        mo.control(5, ctl(0, ctl::BEND), -100.0);
        mo.control(0, ctl(0, ctl::PRESSURE), 300.0);
        mo.control(0, ctl(0, 123), 0.0);
        mo.control(0, ParamAddr::new(Owner::Inst(0), 74), 1.0);
        mo.period_end(PERIOD, Instant::now());
        assert!(mo.close());
        let b = sink.bytes();
        // -100 is 8092 from the bottom: LSB 28, MSB 63.
        assert_eq!(
            b[..12],
            [0xD2, 127, 0xE2, 28, 63, 0xB2, 74, 20, 0x92, 0x25, 100, 0x82]
        );
    }

    /// At most one message a control a period: a value given again in
    /// the period goes later, the last one given; on the 5-pin jacks no
    /// sooner than 4 ms after the last (every other period of 128 frames),
    /// on USB at the next period's start. Only a change goes.
    #[test]
    fn a_control_goes_once_a_period_and_its_last_value_follows() {
        let sent = |mo: &MidiOut| -> Vec<(u32, Vec<u8>)> {
            mo.sent()
                .map(|(_, _, t)| (t.offset, t.bytes().to_vec()))
                .collect()
        };
        for usb in [false, true] {
            let (mut mo, _din) = open(Duration::ZERO);
            let port = if usb { "Digitone" } else { DIN_NAME };
            let (sink, opens) = (Sink::default(), Arc::default());
            if usb {
                mo.plugged(USB, "Digitone", opener(&sink, &opens));
            }
            let m = model(&[(0, TrackOut::new(port, 0))]);
            if usb {
                with_digitone(&m);
            }
            mo.sync(&m, true);
            for (offset, v) in [(0, 10.0), (40, 20.0), (80, 30.0)] {
                mo.control(offset, ctl(0, 1), v);
            }
            mo.control(90, ctl(0, 2), 5.0);
            mo.period_end(PERIOD, Instant::now());
            assert_eq!(sent(&mo), [(0, vec![0xB0, 1, 10]), (90, vec![0xB0, 2, 5])]);
            if !usb {
                mo.sync(&m, true);
                mo.period_end(PERIOD, Instant::now());
                assert!(sent(&mo).is_empty(), "the jacks: 2.9 ms is too soon");
            }
            mo.sync(&m, true);
            mo.period_end(PERIOD, Instant::now());
            assert_eq!(sent(&mo), [(0, vec![0xB0, 1, 30])], "once, the last");
            mo.sync(&m, true);
            mo.period_end(PERIOD, Instant::now());
            assert!(sent(&mo).is_empty());
            // The same value again (an LFO at its peak, a lock): nothing.
            mo.sync(&m, true);
            mo.control(0, ctl(0, 1), 30.0);
            mo.control(0, ctl(0, 2), 5.2);
            mo.period_end(PERIOD, Instant::now());
            assert!(sent(&mo).is_empty(), "no change");
            // An LFO: a value every period. The jacks get one every other
            // period, the newest given by then (at the period's start);
            // USB each, on its own period.
            let mut got = Vec::new();
            for p in 0..8u8 {
                mo.sync(&m, true);
                mo.control(0, ctl(0, 1), f32::from(40 + p));
                mo.period_end(PERIOD, Instant::now());
                got.extend(sent(&mo).into_iter().map(|(_, b)| b[2]));
            }
            if usb {
                assert_eq!(got, [40, 41, 42, 43, 44, 45, 46, 47]);
            } else {
                assert_eq!(got, [40, 41, 43, 45]);
            }
            assert!(mo.close());
        }
    }

    /// A track on the pads sends nothing but keeps its controls' values:
    /// they go out when its port is set and again at each song start,
    /// after its program; a control never given a value never goes.
    #[test]
    fn a_tracks_controls_wait_for_its_port_and_go_again_at_each_start() {
        let (mut mo, sink) = open(Duration::ZERO);
        let none = model(&[]);
        mo.sync(&none, false);
        mo.control(0, ctl(1, 7), 90.0);
        mo.control(0, ctl(1, ctl::BEND), 8191.0);
        mo.period_end(PERIOD, Instant::now());
        assert!(sink.bytes().is_empty());
        let mut o = TrackOut::new(DIN_NAME, 0);
        o.program = Some(4);
        let m = model(&[(1, o)]);
        m.lock().unwrap().gen = 2;
        mo.sync(&m, false);
        mo.period_end(PERIOD, Instant::now());
        let set = [0xC0, 4, 0xB0, 7, 90, 0xE0, 0x7F, 0x7F];
        // Stopped, then the song starts: again; still playing: not again.
        mo.sync(&m, false);
        mo.period_end(PERIOD, Instant::now());
        mo.sync(&m, true);
        mo.period_end(PERIOD, Instant::now());
        mo.sync(&m, true);
        mo.period_end(PERIOD, Instant::now());
        assert!(mo.close());
        let mut want = set.to_vec();
        want.extend(set);
        assert_eq!(sink.bytes(), want);
    }

    fn cc_fx(fx: &[mpfree_notefx::Fx]) -> TrackFx {
        let mut t = TrackFx::default();
        for f in fx {
            t.add(*f);
        }
        t
    }

    fn smooth(ms: u16, glide_on_note: bool) -> mpfree_notefx::Fx {
        mpfree_notefx::Fx::CcSmooth(mpfree_notefx::CcSmooth {
            cc: None,
            ms,
            glide_on_note,
        })
    }

    fn sent_bytes(mo: &MidiOut) -> Vec<Vec<u8>> {
        mo.sent().map(|(_, _, t)| t.bytes().to_vec()).collect()
    }

    /// CC scale fits a track's controllers into its range as they go out
    /// (the bend isn't a controller); a change of the effect sends again
    /// only the values it changes, once, and taking it away sends the
    /// raw value again.
    #[test]
    fn cc_scale_fits_a_tracks_controllers_and_a_change_sends_them_again() {
        use mpfree_notefx::{CcScale, Fx};
        let (mut mo, _sink) = open(Duration::ZERO);
        let m = model(&[(0, TrackOut::new(DIN_NAME, 0))]);
        let scale = |low, high| {
            cc_fx(&[Fx::CcScale(CcScale {
                cc: Some(74),
                low,
                high,
            })])
        };
        mo.set_note_fx(0, &scale(20, 100));
        mo.sync(&m, true);
        mo.control(0, ctl(0, 74), 127.0);
        mo.control(0, ctl(0, 1), 127.0);
        mo.control(0, ctl(0, ctl::BEND), 8191.0);
        mo.period_end(PERIOD, Instant::now());
        assert_eq!(
            sent_bytes(&mo),
            [
                vec![0xB0, 74, 100],
                vec![0xB0, 1, 127],
                vec![0xE0, 0x7F, 0x7F]
            ]
        );
        let period = |mo: &mut MidiOut, fx: &TrackFx| {
            mo.set_note_fx(0, fx);
            mo.sync(&m, true);
            mo.period_end(PERIOD, Instant::now());
            sent_bytes(mo)
        };
        assert_eq!(period(&mut mo, &scale(0, 64)), [vec![0xB0, 74, 64]]);
        assert!(period(&mut mo, &scale(0, 64)).is_empty(), "no change");
        assert!(period(&mut mo, &scale(10, 64)).is_empty(), "64 still");
        assert_eq!(
            period(&mut mo, &TrackFx::default()),
            [vec![0xB0, 74, 127]],
            "raw again"
        );
        assert!(mo.close());
    }

    /// CC smooth glides a controller to each new value: a message a
    /// period at most, only when the value changes, ending on the value;
    /// on the jacks none sooner than 4 ms after the last, on USB every
    /// period. The first value ever, and a resend at the song's start,
    /// go at once.
    #[test]
    fn cc_smooth_glides_to_each_value_and_spares_the_jacks() {
        // 20 ms at 44.1 kHz is 882 frames: 7 periods of 128.
        let glide = |usb: bool| -> Vec<(usize, u8)> {
            let (mut mo, _din) = open(Duration::ZERO);
            let port = if usb { "Digitone" } else { DIN_NAME };
            let (sink, opens) = (Sink::default(), Arc::default());
            if usb {
                mo.plugged(USB, "Digitone", opener(&sink, &opens));
            }
            let m = model(&[(0, TrackOut::new(port, 0))]);
            if usb {
                with_digitone(&m);
            }
            mo.set_note_fx(0, &cc_fx(&[smooth(20, false)]));
            mo.sync(&m, true);
            assert!(mo.takes(0));
            mo.control(0, ctl(0, 74), 0.0);
            mo.period_end(PERIOD, Instant::now());
            assert_eq!(sent_bytes(&mo), [vec![0xB0, 74, 0]], "the first at once");
            mo.sync(&m, true);
            mo.control(0, ctl(0, 74), 127.0);
            mo.period_end(PERIOD, Instant::now());
            assert!(sent_bytes(&mo).is_empty(), "from the next period");
            let mut got = Vec::new();
            for p in 0..12 {
                mo.sync(&m, true);
                mo.period_end(PERIOD, Instant::now());
                for b in sent_bytes(&mo) {
                    assert_eq!(b[..2], [0xB0, 74]);
                    got.push((p, b[2]));
                }
            }
            // While it glides, the song starts again: straight there.
            mo.control(0, ctl(0, 74), 0.0);
            mo.period_end(PERIOD, Instant::now());
            mo.sync(&m, false);
            mo.period_end(PERIOD, Instant::now());
            mo.sync(&m, true);
            mo.period_end(PERIOD, Instant::now());
            assert_eq!(sent_bytes(&mo), [vec![0xB0, 74, 0]], "a resend jumps");
            mo.sync(&m, true);
            mo.period_end(PERIOD, Instant::now());
            assert!(sent_bytes(&mo).is_empty(), "and stays");
            got
        };
        for usb in [false, true] {
            let got = glide(usb);
            assert_eq!(got.last().map(|g| g.1), Some(127), "{got:?}");
            assert!(got.windows(2).all(|w| w[0].1 < w[1].1), "{got:?}");
            assert!(got[0].1 > 0 && got[0].0 == 0, "{got:?}");
            let gaps: Vec<usize> = got.windows(2).map(|w| w[1].0 - w[0].0).collect();
            if usb {
                assert_eq!(got.len(), 7, "every period: {got:?}");
            } else {
                // 4 ms is 176 frames: every other period.
                assert!(gaps.iter().all(|&g| g >= 2), "{got:?}");
                assert!(got.len() <= 4, "{got:?}");
            }
        }
    }

    /// A value that comes with one of the track's notes (a step's lock)
    /// goes out at once, ahead of the note, unless the smooth glides on
    /// notes too.
    #[test]
    fn a_smoothed_value_with_a_note_goes_ahead_of_it_unless_told_to_glide() {
        let (mut mo, _sink) = open(Duration::ZERO);
        let m = model(&[(0, TrackOut::new(DIN_NAME, 0))]);
        mo.set_note_fx(0, &cc_fx(&[smooth(50, false)]));
        mo.sync(&m, true);
        mo.control(0, ctl(0, 74), 0.0);
        mo.period_end(PERIOD, Instant::now());
        mo.sync(&m, true);
        assert!(mo.send(&ev(0, 10, on(60, 100)), NoteIs::Note));
        mo.control(10, ctl(0, 74), 127.0);
        mo.period_end(PERIOD, Instant::now());
        assert_eq!(
            sent_bytes(&mo)[..2],
            [vec![0xB0, 74, 127], vec![0x90, 60, 100]]
        );
        // Told to glide with notes too: the note goes alone.
        mo.set_note_fx(0, &cc_fx(&[smooth(50, true)]));
        mo.sync(&m, true);
        assert!(mo.send(&ev(0, 20, on(62, 100)), NoteIs::Note));
        mo.control(20, ctl(0, 74), 0.0);
        mo.period_end(PERIOD, Instant::now());
        let b = sent_bytes(&mo);
        assert!(b.iter().all(|m| m[0] != 0xB0), "{b:02X?}");
        mo.sync(&m, true);
        mo.period_end(PERIOD, Instant::now());
        let b = sent_bytes(&mo);
        assert!(
            matches!(b[..], [ref m] if m[..2] == [0xB0, 74] && m[2] < 127),
            "{b:?}"
        );
        assert!(mo.close());
    }

    /// A panic ends exactly the notes that are on, is left to the pads too
    /// (false), and nothing is left on for close to end.
    #[test]
    fn all_off_ends_only_what_is_on() {
        let (mut mo, sink) = open(Duration::ZERO);
        let m = model(&[
            (0, TrackOut::new(DIN_NAME, 0)),
            (1, TrackOut::new(DIN_NAME, 1)),
        ]);
        mo.sync(&m, true);
        mo.send(&ev(0, 0, on(0, 100)), NoteIs::Pad(kit_notes(0)));
        mo.send(&ev(1, 0, on(64, 100)), NoteIs::Note);
        assert!(!mo.send(&ev(ALL, 50, Event::AllOff), NoteIs::Note));
        mo.period_end(PERIOD, Instant::now());
        assert!(mo.close());
        let b = sink.bytes();
        assert_eq!(b.len(), 12, "{b:02X?}");
        assert_eq!(&b[6..], [0x80, 0x25, 64, 0x81, 64, 64]);
    }

    /// Locks and lanes stay with the mixer.
    #[test]
    fn locks_are_not_taken() {
        let (mut mo, _sink) = open(Duration::ZERO);
        let m = model(&[(0, TrackOut::new(DIN_NAME, 0))]);
        mo.sync(&m, true);
        let lock = Event::Unlock {
            addr: mpfree_play::mpfree_param::ParamAddr::new(
                mpfree_play::mpfree_param::Owner::Inst(0),
                5,
            ),
        };
        assert!(!mo.send(&ev(0, 0, lock), NoteIs::Note));
    }

    /// Each message leaves when its frame is heard: the period's start,
    /// plus the lead, plus its offset; never before.
    #[test]
    fn messages_leave_when_their_frame_is_heard() {
        let lead = Duration::from_millis(30);
        let (mut mo, sink) = open(lead);
        let m = model(&[(0, TrackOut::new(DIN_NAME, 0))]);
        mo.sync(&m, true);
        let t0 = Instant::now();
        // 4410 frames is 100 ms.
        mo.send(&ev(0, 4410, on(0, 100)), NoteIs::Pad(kit_notes(0)));
        mo.send(&ev(0, 0, on(1, 100)), NoteIs::Pad(kit_notes(1)));
        mo.period_end(8820, t0);
        thread::sleep(Duration::from_millis(250));
        let writes = sink.0.lock().unwrap().clone();
        assert_eq!(writes.len(), 2);
        // In frame order, each no earlier than due.
        assert_eq!(writes[0].1, [0x90, 0x24, 100]);
        assert_eq!(writes[1].1, [0x90, 0x25, 100]);
        assert!(writes[0].0 >= t0 + lead);
        assert!(writes[1].0 >= t0 + lead + Duration::from_millis(100));
        assert!(mo.close());
    }

    /// The 5-pin jacks' gear delay (the port page): each message leaves
    /// that much before its frame is heard, but never before its period
    /// was made, so MIDI out sends at most the sound's lead early (which
    /// the page shows).
    #[test]
    fn the_gears_delay_sends_early_up_to_the_lead() {
        let ms = Duration::from_millis;
        let (mut mo, _sink) = open(ms(10));
        let m = model(&[(0, TrackOut::new(DIN_NAME, 0))]);
        m.lock()
            .unwrap()
            .setup
            .set_port(DIN_NAME, |p| p.delay_tenths = 45);
        mo.sync(&m, false);
        assert_eq!(mo.ports[0].early, Duration::from_micros(4500));
        assert_eq!(m.lock().unwrap().live.early_ms, 10.0);
        let t0 = Instant::now();
        // Heard 20 ms after its period was made: it leaves at 15.5 ms.
        let early = mo.ports[0].early;
        assert_eq!(leave_at(t0 + ms(20), early, t0), t0 + ms(20) - early);
        // 40 ms early is more than there is: it leaves when it is made.
        assert_eq!(leave_at(t0 + ms(20), ms(40), t0), t0);
        // Back to none.
        {
            let mut g = m.lock().unwrap();
            g.setup.set_port(DIN_NAME, |p| p.delay_tenths = 0);
            g.gen += 1;
        }
        mo.sync(&m, false);
        assert_eq!(mo.ports[0].early, Duration::ZERO);
        assert!(mo.close());
    }

    /// The port page's test note: C4 on the asked channel, its note-off
    /// half a second of frames later; a second press ends the first. It
    /// says what it sent.
    #[test]
    fn the_test_note_is_held_half_a_second() {
        let (mut mo, sink) = open(Duration::ZERO);
        let m = model(&[]);
        m.lock().unwrap().asks.port_send = Some(PortSend {
            port: DIN_NAME.into(),
            what: ToPort::TestNote { channel: 9 },
        });
        mo.sync(&m, false);
        let said = m.lock().unwrap().live.port_said.clone().unwrap();
        assert!(
            said.starts_with("Test note C4 sent to MIDI on ch 10"),
            "{said}"
        );
        // 0.5 s is 22050 frames: 172 periods of 128 hold it, the 173rd
        // ends it.
        for _ in 0..172 {
            mo.period_end(PERIOD, Instant::now());
        }
        assert!(mo.ports[0].test_off.is_some());
        mo.period_end(PERIOD, Instant::now());
        assert!(mo.ports[0].test_off.is_none());
        assert!(mo.close());
        assert_eq!(sink.bytes(), [0x99, 60, 100, 0x89, 60, 64]);
    }

    /// All notes off: MPFree's own notes on end first, then CC 123 goes on
    /// all 16 channels, and the monitor forgets the notes it held on.
    #[test]
    fn all_notes_off_ends_mpfrees_notes_then_every_channel() {
        let (mut mo, sink) = open(Duration::ZERO);
        let m = model(&[(0, TrackOut::new(DIN_NAME, 2))]);
        mo.sync(&m, true);
        mo.send(
            &ev(
                0,
                0,
                Event::NoteOn {
                    note: 64,
                    velocity: 90,
                },
            ),
            NoteIs::Note,
        );
        mo.period_end(PERIOD, Instant::now());
        m.lock().unwrap().asks.port_send = Some(PortSend {
            port: DIN_NAME.into(),
            what: ToPort::AllNotesOff,
        });
        mo.sync(&m, true);
        mo.period_end(PERIOD, Instant::now());
        assert!(mo.close());
        let b = sink.bytes();
        let mut want = vec![0x92, 64, 90, 0x82, 64, 64];
        for ch in 0..16u8 {
            want.extend([0xB0 | ch, 123, 0]);
        }
        assert_eq!(b, want);
        assert_eq!(
            m.lock().unwrap().live.port_said.as_deref(),
            Some("All notes off sent to MIDI on all 16 channels")
        );
    }

    /// A send to a device that isn't plugged in, or while MIDI out is off,
    /// sends nothing and says why.
    #[test]
    fn a_send_says_why_nothing_went() {
        let (mut mo, sink) = open(Duration::ZERO);
        let m = model(&[]);
        m.lock().unwrap().asks.port_send = Some(PortSend {
            port: "Keystep 37".into(),
            what: ToPort::AllNotesOff,
        });
        mo.sync(&m, false);
        assert_eq!(
            m.lock().unwrap().live.port_said.as_deref(),
            Some("Nothing sent: Keystep 37 isn't plugged in, or has no output")
        );
        mo.period_end(PERIOD, Instant::now());
        assert!(mo.close());
        assert!(sink.bytes().is_empty());
        let mut off = MidiOut::new(RATE, Duration::ZERO);
        m.lock().unwrap().asks.port_send = Some(PortSend {
            port: DIN_NAME.into(),
            what: ToPort::TestNote { channel: 0 },
        });
        off.sync(&m, false);
        assert_eq!(
            m.lock().unwrap().live.port_said.as_deref(),
            Some("Nothing sent: MIDI out is off (Prefs)")
        );
    }

    /// Prefs turns sending off without waiting: the notes on end and the
    /// writer finishes on its own; the I/O view hears MIDI out is off.
    #[test]
    fn switching_off_ends_the_notes_without_waiting() {
        let (mut mo, sink) = open(Duration::ZERO);
        let m = model(&[(0, TrackOut::new(DIN_NAME, 0))]);
        mo.sync(&m, true);
        mo.send(
            &ev(
                0,
                0,
                Event::NoteOn {
                    note: 60,
                    velocity: 80,
                },
            ),
            NoteIs::Note,
        );
        mo.period_end(PERIOD, Instant::now());
        mo.switch_off();
        assert!(!mo.is_open());
        mo.sync(&m, false);
        assert!(!m.lock().unwrap().live.built.notes_out);
        let give_up = Instant::now() + Duration::from_secs(2);
        while sink.bytes().len() < 6 && Instant::now() < give_up {
            thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(sink.bytes(), [0x90, 60, 80, 0x80, 60, 64]);
        // Switched on again at once: the same writer, on the same port.
        assert!(mo.reopen());
        assert!(mo.is_open());
        mo.sync(&m, false);
        assert!(m.lock().unwrap().live.built.notes_out);
        // Off again; once its last messages have left, tick lets it go.
        mo.switch_off();
        thread::sleep(Duration::from_millis(300));
        mo.tick(&mut |_| {}, &mut |_| {});
        assert!(!mo.reopen());
    }

    /// A port that fails stops the writer; the loop says so, lets the port
    /// go, and the track plays on the pads again.
    #[test]
    fn a_failed_port_is_let_go_and_said() {
        let mut mo = MidiOut::new(RATE, Duration::ZERO);
        mo.open(Broken, Instant::now()).unwrap();
        let m = model(&[(0, TrackOut::new(DIN_NAME, 0))]);
        mo.sync(&m, true);
        assert!(mo.send(&ev(0, 0, on(0, 100)), NoteIs::Pad(kit_notes(0))));
        mo.period_end(PERIOD, Instant::now());
        let mut lines = Vec::new();
        let mut stopped = Vec::new();
        let give_up = Instant::now() + Duration::from_secs(2);
        while mo.is_open() && Instant::now() < give_up {
            mo.tick(&mut |l| lines.push(l), &mut |p| stopped.push(p));
            thread::sleep(Duration::from_millis(2));
        }
        assert!(!mo.is_open());
        assert_eq!(stopped, [DIN]);
        assert!(
            lines[0].starts_with("MIDI Out: writing 90 25 64 failed"),
            "{lines:?}"
        );
        assert!(lines.last().unwrap().starts_with("MIDI Out stopped"));
        assert!(!mo.takes(0));
        // MIDI out is still on; the jacks' row says nothing goes there.
        mo.sync(&m, true);
        let g = m.lock().unwrap();
        assert!(g.live.built.notes_out);
        assert!(!g.live.ports[0].sending);
    }

    /// The screen holding the model never makes the loop wait.
    #[test]
    fn a_busy_model_is_skipped_and_counted() {
        let (mut mo, _sink) = open(Duration::ZERO);
        let m = model(&[(0, TrackOut::new(DIN_NAME, 0))]);
        let held = m.lock().unwrap();
        mo.sync(&m, false);
        assert!(!mo.takes(0));
        drop(held);
        mo.sync(&m, false);
        assert!(mo.takes(0));
        assert!(mo.summary().iter().any(|l| l.contains("1 periods kept")));
    }

    /// The log lists the first messages sent, named.
    #[test]
    fn the_log_names_what_was_sent() {
        let (mut mo, _sink) = open(Duration::ZERO);
        let m = model(&[(0, TrackOut::new(DIN_NAME, 9))]);
        mo.sync(&m, true);
        mo.send(&ev(0, 0, on(0, 100)), NoteIs::Pad(kit_notes(0)));
        mo.period_end(PERIOD, Instant::now());
        let mut lines = Vec::new();
        let give_up = Instant::now() + Duration::from_secs(2);
        while lines.is_empty() && Instant::now() < give_up {
            mo.tick(&mut |l| lines.push(l), &mut |_| {});
            thread::sleep(Duration::from_millis(2));
        }
        assert!(
            lines[0].ends_with("MIDI Out: 99 25 64 = note on, ch 10, C#2, velocity 100"),
            "{lines:?}"
        );
        assert!(mo.close());
    }

    /// The jacks with "Clock out" set, a transport, and its period.
    struct ClockRig {
        mo: MidiOut,
        sink: Sink,
        m: Shared,
        tr: mpfree_transport::Transport,
        block: Block,
    }

    impl ClockRig {
        fn new(mode: ClockOut) -> ClockRig {
            let (mut mo, sink) = open(Duration::ZERO);
            let m = model(&[]);
            m.lock()
                .unwrap()
                .setup
                .set_port(DIN_NAME, |p| p.clock_out = mode);
            mo.sync(&m, false);
            ClockRig {
                mo,
                sink,
                m,
                tr: mpfree_transport::Transport::new(RATE, 120.0).unwrap(),
                block: Block::new(),
            }
        }

        fn command(&mut self, c: mpfree_transport::Command) {
            self.tr.command(self.tr.frame(), c);
        }

        /// `seconds` of periods; returns the bytes they sent.
        fn run(&mut self, seconds: f64) -> Vec<u8> {
            let periods = (seconds * f64::from(RATE) / f64::from(PERIOD)).round() as u32;
            for _ in 0..periods {
                self.tr.process(PERIOD, &mut self.block);
                self.mo.sync(&self.m, self.tr.state().rolling());
                self.mo.clock(&self.block);
                self.mo.period_end(PERIOD, Instant::now());
            }
            self.flush()
        }

        /// What the writer has written since the last call.
        fn flush(&mut self) -> Vec<u8> {
            let give_up = Instant::now() + Duration::from_secs(2);
            let din = &self.mo.ports[0];
            while din
                .writer
                .as_ref()
                .is_some_and(|w| w.written() < din.queued)
                && Instant::now() < give_up
            {
                thread::sleep(Duration::from_millis(1));
            }
            let b = self.sink.bytes();
            self.sink.0.lock().unwrap().clear();
            b
        }
    }

    fn pulses(b: &[u8]) -> usize {
        b.iter().filter(|&&x| x == 0xF8).count()
    }

    /// While playing: Start from the top, 24 pulses a quarter note (48 a
    /// second at 120 BPM), Stop; nothing while stopped.
    #[test]
    fn clock_out_while_playing() {
        use mpfree_transport::Command;
        let mut r = ClockRig::new(ClockOut::WhilePlaying);
        assert!(r.m.lock().unwrap().live.built.clock_out);
        assert_eq!(r.run(0.5), []);
        r.command(Command::Play);
        let b = r.run(1.0);
        assert_eq!(b[0], 0xFA, "{b:02X?}");
        assert!(b[1..].iter().all(|&x| x == 0xF8), "{b:02X?}");
        assert!((48..=49).contains(&pulses(&b)), "{}", pulses(&b));
        r.command(Command::Stop);
        assert_eq!(r.run(0.5), [0xFC]);
        assert!(r.mo.close());
    }

    /// Always: pulses at the tempo while stopped too, then Start and
    /// pulses from the top when it plays; close sends Stop.
    #[test]
    fn clock_out_always() {
        use mpfree_transport::Command;
        let mut r = ClockRig::new(ClockOut::Always);
        let b = r.run(1.0);
        assert!(b.iter().all(|&x| x == 0xF8), "{b:02X?}");
        assert!((47..=49).contains(&b.len()), "{}", b.len());
        r.command(Command::Play);
        let b = r.run(0.5);
        assert_eq!(b[0], 0xFA, "{b:02X?}");
        assert!((24..=25).contains(&pulses(&b)));
        assert!(r.mo.close());
        assert_eq!(r.sink.bytes(), [0xFC]);
    }

    /// A locate while playing: Stop, Song Position, Continue, then pulses
    /// from there.
    #[test]
    fn clock_out_follows_a_locate() {
        use mpfree_transport::Command;
        let mut r = ClockRig::new(ClockOut::WhilePlaying);
        r.command(Command::Play);
        r.run(0.25);
        // Bar 2: 16 sixteenths in.
        r.command(Command::Locate(3840));
        let b = r.run(0.1);
        assert_eq!(&b[..5], [0xFC, 0xF2, 16, 0, 0xFB], "{b:02X?}");
        assert!(b[5..].iter().all(|&x| x == 0xF8));
        assert!(pulses(&b) >= 4);
    }

    /// A count-in sends no clock; Start comes when playback starts.
    #[test]
    fn clock_out_waits_for_the_count_in() {
        use mpfree_transport::Command;
        let mut r = ClockRig::new(ClockOut::WhilePlaying);
        r.command(Command::SetCountIn(1));
        r.command(Command::Record);
        // A bar at 120 BPM is 2 s: still counting in.
        assert_eq!(r.run(1.5), []);
        let b = r.run(1.0);
        assert_eq!(b[0], 0xFA, "{b:02X?}");
        assert!(b[1..].iter().all(|&x| x == 0xF8));
    }

    /// Clock turned on while playing joins at the next sixteenth (Song
    /// Position, Continue); turned off while playing, it sends Stop.
    #[test]
    fn clock_out_turned_on_and_off_while_playing() {
        use mpfree_transport::Command;
        let mut r = ClockRig::new(ClockOut::Off);
        r.command(Command::Play);
        assert_eq!(r.run(0.5), []);
        let set = |r: &mut ClockRig, mode| {
            let mut g = r.m.lock().unwrap();
            g.setup.set_port(DIN_NAME, |p| p.clock_out = mode);
            g.gen += 1;
        };
        set(&mut r, ClockOut::WhilePlaying);
        let b = r.run(0.5);
        assert_eq!(b[0], 0xF2, "{b:02X?}");
        assert_eq!(b[3], 0xFB);
        assert!(b[4..].iter().all(|&x| x == 0xF8));
        set(&mut r, ClockOut::Off);
        assert_eq!(r.run(0.5), [0xFC]);
    }

    /// Divided by 2: half the pulses, and a locate's Song Position in the
    /// gear's sixteenths; back to 1 while playing: Stop, then it joins
    /// again at the tempo.
    #[test]
    fn clock_out_divided() {
        use mpfree_transport::Command;
        let mut r = ClockRig::new(ClockOut::WhilePlaying);
        let divide = |r: &mut ClockRig, d| {
            let mut g = r.m.lock().unwrap();
            g.setup.set_port(DIN_NAME, |p| p.clock_divide = d);
            g.gen += 1;
        };
        divide(&mut r, 2);
        r.command(Command::Play);
        let b = r.run(1.0);
        assert_eq!(b[0], 0xFA, "{b:02X?}");
        assert!((24..=25).contains(&pulses(&b)), "{}", pulses(&b));
        // Bar 2 is 16 sixteenths in; the gear is at its 8th.
        r.command(Command::Locate(3840));
        let b = r.run(0.1);
        assert_eq!(&b[..5], [0xFC, 0xF2, 8, 0, 0xFB], "{b:02X?}");
        divide(&mut r, 1);
        let b = r.run(0.5);
        assert_eq!(b[0], 0xFC, "{b:02X?}");
        assert_eq!((b[1], b[4]), (0xF2, 0xFB), "{b:02X?}");
        assert!((21..=25).contains(&pulses(&b)), "{}", pulses(&b));
    }

    /// Nudged later, the clock leaves after the notes on its frame;
    /// nudged earlier, before them.
    #[test]
    fn a_nudged_clock_leaves_later_or_earlier_than_the_notes() {
        let note = Timed::msg(
            0,
            Msg::NoteOn {
                ch: 0,
                note: 60,
                vel: 100,
            },
        );
        let pulse = Timed::realtime(0, 0xF8);
        for (nudge, pushed, first) in [
            (0, [pulse, note], 0xF8),
            (20, [pulse, note], 0x90),
            (0, [note, pulse], 0x90),
            (-20, [note, pulse], 0xF8),
        ] {
            let (mut mo, sink) = open(Duration::ZERO);
            let m = model(&[]);
            m.lock()
                .unwrap()
                .setup
                .set_port(DIN_NAME, |p| p.clock_nudge_tenths = nudge);
            mo.sync(&m, false);
            for t in pushed {
                mo.queue[0].push(t);
            }
            mo.period_end(PERIOD, Instant::now());
            wait_for(&sink, 4);
            assert_eq!(sink.bytes()[0], first, "nudge {nudge}");
            assert!(mo.close());
        }
    }

    /// Steady playing on MIDI Out allocates and frees nothing in the sound
    /// loop.
    /// Waits up to 2 s for the port to have `n` bytes.
    fn wait_for(sink: &Sink, n: usize) {
        let give_up = Instant::now() + Duration::from_secs(2);
        while sink.bytes().len() < n && Instant::now() < give_up {
            thread::sleep(Duration::from_millis(2));
        }
    }

    /// The Sample tab's auto-sample of outside gear: its notes, sent from
    /// the job's own thread, leave the jacks in the next period, marked as
    /// its own for the monitor; anything but a note on or off is refused;
    /// the end of the job ends every note it left on.
    #[test]
    fn the_auto_samples_notes_go_out_at_once_and_end_with_the_job() {
        let (mut mo, sink) = open(Duration::ZERO);
        let g = mo.gear_changed().expect("said once").expect("the jacks");
        assert_eq!(g.name, GEAR_NAME);
        assert!(mo.gear_changed().is_none(), "said only when it changes");
        let send = g.send.clone();
        thread::spawn(move || {
            send(&[0x92, 60, 100]);
            send(&[0xB2, 1, 64]); // a CC: refused
            send(&[0xF0, 0x7E, 0xF7]); // SysEx: refused
            send(&[0x92, 60]); // short: refused
            send(&[0x92, 0x80, 100]); // a data byte with its top bit: refused
        })
        .join()
        .unwrap();
        mo.period_end(PERIOD, Instant::now());
        let sent: Vec<(Vec<u8>, bool)> = mo
            .sent()
            .map(|(_, port, m)| {
                assert_eq!(port, DIN);
                (m.bytes().to_vec(), m.is_from_sampler())
            })
            .collect();
        assert_eq!(sent, [(vec![0x92, 60, 100], true)]);
        (g.send)(&[0x92, 64, 90]);
        (g.send)(&[]);
        mo.period_end(PERIOD, Instant::now());
        wait_for(&sink, 12);
        assert_eq!(
            sink.bytes(),
            [0x92, 60, 100, 0x92, 64, 90, 0x82, 60, 64, 0x82, 64, 64]
        );
        // Nothing left on: a second end sends nothing.
        (g.send)(&[]);
        mo.period_end(PERIOD, Instant::now());
        assert_eq!(mo.sent().count(), 0);
        let said = mo.summary().join("\n");
        assert!(
            said.contains(
                "Auto-sample: its notes to the gear on MIDI Out: 2 sent, 0 not sent (MIDI out off), 4 refused"
            ),
            "{said}"
        );
        assert!(mo.close());
    }

    /// A panic ends the auto-sample's note; switched off, the Sample tab is
    /// told it has no MIDI out and what was still sent isn't; a full queue
    /// drops and counts, never waits.
    #[test]
    fn the_auto_sample_stops_with_a_panic_and_has_no_midi_out_while_sending_is_off() {
        let (mut mo, sink) = open(Duration::ZERO);
        let g = mo.gear_out().expect("the jacks, sending");
        (g.send)(&[0x90, 48, 100]);
        mo.period_end(PERIOD, Instant::now());
        mo.all_off(0);
        mo.period_end(PERIOD, Instant::now());
        wait_for(&sink, 6);
        assert_eq!(sink.bytes(), [0x90, 48, 100, 0x80, 48, 64]);
        mo.gear_changed();
        mo.switch_off();
        assert!(matches!(mo.gear_changed(), Some(None)));
        assert!(mo.gear_out().is_none());
        (g.send)(&[0x90, 50, 100]);
        mo.period_end(PERIOD, Instant::now());
        for _ in 0..GEAR_QUEUE + 6 {
            (g.send)(&[0x90, 52, 100]);
        }
        let said = mo.summary().join("\n");
        assert!(said.contains("6 dropped (the queue was full)"), "{said}");
        mo.period_end(PERIOD, Instant::now());
        let said = mo.summary().join("\n");
        assert!(
            said.contains(&format!("MIDI Out: 1 sent, {} not sent", GEAR_QUEUE + 1)),
            "{said}"
        );
        assert_eq!(sink.bytes().len(), 6, "nothing went out while off");
    }

    /// The auto-sample's notes cost the sound loop no allocation, and its
    /// `send` none on the job's side.
    #[test]
    fn the_auto_samples_notes_never_allocate() {
        let (mut mo, _sink) = open(Duration::ZERO);
        let g = mo.gear_out().expect("the jacks, sending");
        let run = |mo: &mut MidiOut, periods: u32| {
            for i in 0..periods {
                let note = (i % 100) as u8;
                (g.send)(&[0x90, note, 100]);
                mo.period_end(PERIOD, Instant::now());
                (g.send)(&[0x80, note, 0]);
                (g.send)(&[]);
                mo.period_end(PERIOD, Instant::now());
            }
        };
        run(&mut mo, 20);
        thread::sleep(Duration::from_millis(50));
        let ((), calls) = crate::sys::alloc_count::calls(|| run(&mut mo, 500));
        assert_eq!(calls, 0, "allocations or frees");
        assert!(mo.close());
    }

    #[test]
    fn steady_play_never_allocates() {
        let (mut mo, sink) = open(Duration::ZERO);
        let m = model(&[(0, TrackOut::new(DIN_NAME, 9))]);
        mo.sync(&m, true);
        let run = |mo: &mut MidiOut, periods: u32| {
            for i in 0..periods {
                mo.sync(&m, true);
                let pad = (i % 16) as u8;
                mo.send(&ev(0, 0, on(pad, 100)), NoteIs::Pad(kit_notes(pad.into())));
                mo.send(
                    &ev(0, 64, Event::NoteOff { note: pad }),
                    NoteIs::Pad(kit_notes(pad.into())),
                );
                mo.period_end(PERIOD, Instant::now());
            }
        };
        // Warm up (the writer's first wait), then count.
        run(&mut mo, 50);
        thread::sleep(Duration::from_millis(50));
        let ((), calls) = crate::sys::alloc_count::calls(|| run(&mut mo, 1000));
        assert_eq!(calls, 0, "allocations or frees in the sound loop");
        assert!(mo.close());
        assert_eq!(sink.bytes().len(), 1050 * 6);
    }

    /// CC scale and CC smooth on a track's lanes, with the effects
    /// changing and controls gliding every period, allocate nothing.
    #[test]
    fn shaped_and_gliding_controls_never_allocate() {
        use mpfree_notefx::{CcScale, Fx};
        let (mut mo, _sink) = open(Duration::ZERO);
        let m = model(&[(0, TrackOut::new(DIN_NAME, 0))]);
        mo.sync(&m, true);
        let fx: [TrackFx; 2] = std::array::from_fn(|i| {
            cc_fx(&[
                Fx::CcScale(CcScale {
                    cc: None,
                    low: 10 * i as u8,
                    high: 100,
                }),
                smooth(30, i == 1),
            ])
        });
        let run = |mo: &mut MidiOut, periods: u32| {
            for i in 0..periods {
                mo.set_note_fx(0, &fx[(i / 7 % 2) as usize]);
                mo.sync(&m, true);
                mo.send(&ev(0, 0, on(60, 100)), NoteIs::Note);
                for cc in [1u16, 74, 71] {
                    let v = ((i * 37 + u32::from(cc)) % 128) as f32;
                    mo.control((i % 3) * 10, ctl(0, cc), v);
                }
                mo.send(&ev(0, 64, Event::NoteOff { note: 60 }), NoteIs::Note);
                mo.period_end(PERIOD, Instant::now());
            }
        };
        run(&mut mo, 50);
        thread::sleep(Duration::from_millis(50));
        let ((), calls) = crate::sys::alloc_count::calls(|| run(&mut mo, 1000));
        assert_eq!(calls, 0, "allocations or frees in the sound loop");
        assert!(mo.close());
    }
}
