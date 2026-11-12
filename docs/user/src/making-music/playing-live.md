<!--
status: changing
covers: rust/mpfree-app/src/play_page.rs rust/mpfree-app/src/play_widgets.rs rust/mpfree-app/src/perform.rs
checked: 2026-11-11 414faa1
-->

# Playing live

The **Play** page is MPFree's home: the place for finger drumming, making beats and live looping. It has four **views**, each arranged for a different way of playing. Tap **All**, **Beats**, **Loop** or **Drum** on the right of the transport bar to switch; the page remembers your view while you visit other tabs.

## The four views

| View | Made for | What's on it |
|---|---|---|
| **All** | Everything at once (the home view) | The loop, the pattern beside a small pad map, the throws and the XY pad |
| **Beats** | Programming and editing patterns | The whole pattern, every row, with the pad map and all eight throws |
| **Loop** | Live looping | The loop, large, with its buttons; the pad map, four throws and the XY pad |
| **Drum** | Practising finger drumming | Your timing, the XY pad, the pad map and the throws |

{{shot: play | The All view.}}

## Throws

**Throws** are short performance effects on the whole mix that play only while you hold them: **Delay throw**, **Reverb wash**, **Stutter**, **Tape stop**, **Filter dip**, **Beat repeat**, **Drop drums** and **Vinyl brake**. Each tile shows its timing or length. Hold one at the end of a phrase and let go on the next downbeat.

**Drop drums** mutes the kick, snare and hi-hats while you hold it: the classic breakdown move.

## The XY pad

The **XY pad** controls two effects at once with one finger: left to right sweeps a filter, bottom to top raises the delay's feedback. It plays while your finger is on it.

## Muting

There is no separate mute page. A **speaker icon** sits beside each pattern row and each loop layer; tap it to mute that pad or layer, and again to bring it back.

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

While you hold **NOTE REPEAT**, or the Arp panel is open, the knobs control the repeat instead: rate, gate, swing and octaves. See [Note Repeat, Latch and the arpeggiator](note-repeat-and-arp.md).

> [!MPC]
> The MPC's XYFX mode and Pad Perform are full-screen modes. In MPFree, the XY pad and the throws sit on the Play page next to your pattern and loop, so you can perform without leaving it.
