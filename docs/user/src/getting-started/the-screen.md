<!--
status: changing
covers: rust/mpfree-ui/src/widgets.rs rust/mpfree-ui/src/qlink.rs rust/mpfree-app/src/play_page.rs rust/mpfree-app/src/gui.rs
checked: 2026-11-13 9560b32
-->

# The screen

Every page in MPFree is laid out the same way, so once you know one, you know them all.

{{shot: play | The Play page. The tabs are at the top left, the Q-Link column at the right, and the hint line along the bottom.}}

## The top bar

**The tabs** on the left are MPFree's main pages. Tap one to go there:

| Tab | What it is for |
|---|---|
| **Play** | The home page: finger drumming, beats and live looping. |
| **Pads** | The kit's 16 pads beside the browser: put sounds on pads and set each pad up. |
| **Arrange** | Clips: recording them, keeping takes and launching them. |
| **Synth** | MPFree's synths and their sounds. |
| **Looper** | Setting up and editing loops. |
| **Sample** | Recording, chopping and shaping the sound on one pad. |

**The status chips** on the right show what is going on: the kit's name and pad bank, how much of the processor (**CPU**) and memory (**RAM**) is in use, whether Full Level is on, and anything that needs your attention. The small **mixer icon** at the far right opens the mixer, and the **I/O icon** just left of it opens the mixer's I/O view, where the project's audio, MIDI, sync and CV are routed. While the tempo follows a clock from another device, a tempo chip such as **120.0 from MIDI In** shows it, with a green dot.

## The transport bar

On the Play page, a bar under the tabs shows time: whether MPFree is stopped, playing or recording, the position (bar, beat and step), the tempo in BPM (tap it in time to set the tempo, as with **TAP TEMPO**), and dots that light up with each bar and beat. On the right of it is the view switch; see [Playing live](../making-music/playing-live.md).

## The Q-Link column and the master bar

At the right edge, the **Q-Link column** shows four cards, one for each Q-Link knob beside the screen, in the same order (knob 1 at the top). Each card names what its knob controls, what part of MPFree it belongs to, and its value. Turn the knob, or drag up and down on its card.

Next to it, the **master bar** shows how loud MPFree's output is, with a meter that holds its peak. Drag the handle to set the overall level. "LIM" lights when the limiter is stopping the output from distorting.

## The hint line

The line along the bottom of the screen always says what the buttons do on the page you are on. If you're not sure what to press, look there first.

When something you asked for can't happen, or does nothing, the hint line says why in amber for a few seconds, then goes back to the hint. For example: "The chain is full", or why a sample edit was refused.

## Things that open over the page

Some things are not tabs. They open over the page you are on, and close the same way they opened:

- **Options:** press **MENU** for the current page's settings, such as the repeat rate or the tempo on the Play page.
- **Prefs:** press **SHIFT + MENU** for MPFree's settings. See [Prefs](../reference/prefs.md).
- **The mixer:** press **TRACK MIX**, or tap the mixer icon. The I/O icon beside it opens the mixer's [I/O view](../making-music/mixing-and-effects.md#the-io-view).
- **The Arp panel:** hold **NOTE REPEAT** and press **MENU**.

{{shot: options | Play's options, opened with MENU. Turn the wheel or tap to pick a row; – and + change it.}}

In a list of settings like this one, turn the data wheel or tap to pick a row, and press **–** or **+** (or tap the arrows) to change it. Or push the data wheel: the row's value is outlined, the wheel now changes it, and a second push sets it so the wheel moves between rows again. The panel on the right explains the row you picked.

The settings beside the Sample tab's pages and the Pads tab's Pad view, the Synth tab's tiles and Arrange's Record settings work the same way: the wheel picks, **–** and **+** change, and the push lets the wheel change it. Holding **SHIFT** gives the wheel its other job on that page (the next pad, the next page, scrolling).

## Touch

- **Tap** to press a button, pick a tab, a pad or a step.
- **Drag** to move a knob, a fader or the XY pad.
- **Hold** on a throw or the XY pad plays it for as long as you hold.

> [!MPC]
> There is no Main mode with tracks and programs on screen, and the Play page has no pads on screen: you play the real ones, and the screen uses the room for the loop, the pattern and performance tools. Prefs is not a menu item: it opens with **SHIFT + MENU**.
