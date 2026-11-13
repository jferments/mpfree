<!--
status: changing
covers: rust/mpfree-app/src/play_page.rs rust/mpfree-app/src/play_widgets.rs rust/mpfree-app/src/mixing.rs rust/mpfree-fx/src/perform.rs
checked: 2026-11-13 dcaefdd
-->

# Playing live

The **Play** page is MPFree's home: the place for finger drumming, making beats and live looping. It has four **views**, each arranged for a different way of playing. Tap **All**, **Beats**, **Loop** or **Drum** on the right of the transport bar to switch; the page remembers your view while you visit other tabs.

## The four views

| View | Made for | What's on it |
|---|---|---|
| **All** | Everything at once (the home view) | The loop, the pattern across the page, the throws and the XY pad |
| **Beats** | Programming and editing patterns | The whole pattern, every row, with all eight throws under it |
| **Loop** | Live looping | The loop, large, with its buttons; the throws and the XY pad |
| **Drum** | Practising finger drumming | Your timing beside the XY pad, and the throws |

There are no pads on the Play page: you play the real ones, and the room goes to the pattern, the loop and the effects. The pads' own settings and sounds are on the Pads tab, next to Play.

{{shot: play | The All view.}}

## Throws

**Throws** are effects you play like an instrument: hold one and the effect comes in, let go and it stops. They are **Delay throw** (a dotted echo), **Reverb wash**, **Stutter**, **Tape stop**, **Filter dip** (a low-pass sweep), **Beat repeat**, **Drop drums** and **Vinyl brake**. Each tile shows its timing or length. While the music plays, a throw starts on the next sixteenth note, so it lands in time even if you press a little early. Hold one at the end of a phrase and let go on the next downbeat.

**Drop drums** mutes the kick, snare and hi-hats while you hold it: the classic breakdown move.

## The XY pad

The **XY pad** is a filter and an echo under one finger. Left of the middle is a low-pass filter (darker), right of it a high-pass filter (thinner), and the middle is no filter at all. Higher up adds more echo. When you lift your finger the pad glides back to the middle and the bottom, so the sound comes back clean. Its label says where it is, for example "HP 173 Hz · echo 62 %".

The **FX mix** Q-Link (in the Drum view) sets how much of the throws and the XY pad you hear, from none to all.

## Fills

While the music plays, hold **PLAY** for a fill: notes set to play only in fills come in, and notes set to drop out in fills go quiet, until you let go. Press **PLAY** twice to keep the fill on; press it again (or STOP) to end it. When the music is stopped, PLAY just starts it. The exact buttons are in [Buttons and controls](../reference/controls.md).

## Muting

There is no separate mute page. A **speaker icon** sits beside each pattern row and each loop layer; tap it to mute that pad or layer, and again to bring it back.

### Muting tracks and pads

**TRACK MUTE** turns the pads into mute buttons over whatever page you are on, so you can drop parts in and out without leaving the screen you are using. Pads 1-8 are tracks 1-8: green while the track plays a clip, dim green when it has nothing playing, red while it is muted. Tap a pad to mute or unmute its track; the track's clip keeps its place, so it comes back in time. **SHIFT + TRACK MUTE** does the same for the pads of the kit: a pad lit in the kit's colour plays, a red one is muted. Press the same button again to play the pads as usual. The hint line at the bottom of the screen says which mode the pads are in.

## Your timing

The **Drum** view shows the last two bars you played as dots against the beat grid, so you can see whether you rush or drag. Under it, **Average** says how early or late you play on average, **Spread** how consistent you are, and **Velocity** how hard you hit.

{{shot: play-drum | The Drum view: each dot is a hit; dots left of a grid line are early.}}

## The Q-Links on the Play page

The four knobs change with the view:

| View | Q-Link 1 | Q-Link 2 | Q-Link 3 | Q-Link 4 |
|---|---|---|---|---|
| All and Beats | Swing | Tune (last pad played) | Drums' filter | Drums' delay send |
| Beats, with a step picked | Velocity | Length | Nudge | Chance |
| Loop | Loop level | Loop's delay send | Line input level | Loop filter |
| Drum | Tune | Decay | Swing | FX mix |

Swing reads **off** at 50 %, which is straight time, as it does in the transport bar. Turn it up for a shuffle.

While you hold **NOTE REPEAT**, or the Arp panel is open, the knobs control the repeat instead: rate, gate, swing and octaves. See [Note Repeat, Latch and the arpeggiator](note-repeat-and-arp.md).

> [!MPC]
> The MPC's XYFX mode and Pad Perform are full-screen modes, and Track Mute opens a page of its own. In MPFree, the XY pad and the throws sit on the Play page next to your pattern and loop, and TRACK MUTE only changes what the pads do, so you can perform without leaving the page you are on.
