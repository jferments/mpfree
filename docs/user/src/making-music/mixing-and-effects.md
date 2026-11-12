<!--
status: changing
covers: rust/mpfree-fx rust/mpfree-mixer-ui rust/mpfree-fx-ui rust/mpfree-chain-ui rust/mpfree-app/src/mixing.rs rust/mpfree-app/src/perform.rs
checked: 2026-11-12 a24dbc5
-->

# Mixing and effects

Every sound in MPFree, from the pads, the synth, the loop and the line input, meets in one **mixer**. There you balance levels, place sounds left and right, send them to shared effects such as a delay, and group them. Effects can sit on any one sound, on a group, or on the whole mix.

## Opening the mixer

Press **TRACK MIX**, or tap the small mixer icon at the top right of the screen. The mixer opens over the page you are on, so the music and the page carry on underneath. Press TRACK MIX again to close it. **SHIFT + TRACK MIX** opens it straight on the pads.

{{shot: mixer | The mixer, shown here with example tracks from the preview: inserts at the top of each strip, sends A to D, pan, the fader and meter, mute and solo, and where the strip goes. The master is on the right, ending in Master safety.}}

Along the top are the mixer's four views:

| View | Shows |
|---|---|
| **Tracks** | One strip per instrument. Today these are **Drums** (the kit on the pads), **Synth** and **Loop**. |
| **Pads** | A strip for each of the kit's 16 pads, so you can set the kick's level or put an effect on just the snare. |
| **Inputs** | The line input. |
| **Submixes** | Eight submixes for grouping, then the four send returns A to D. |

The **< >** buttons page through the strips when there are more than fit. The chip beside them shows how much of the MPC's processor the mixer and its effects are using.

## A channel strip

From top to bottom, each strip has:

- **Inserts**: up to four effects that the sound goes through, in order.
- **Sends A to D**: how much of the sound goes to each send return. A return holds a shared effect, so several strips can use one reverb or delay.
- **Pan**: where the sound sits between left and right.
- **The fader** (from silent up to +6 dB) and the **meter** beside it.
- **M** (mute) and **S** (solo). Solo keeps the strip and everything it feeds or is fed by, so soloing the snare's pad still lets you hear it through the Drums track.
- **The output**: where the strip goes. A pad goes to its track; a track to the master or a submix; a submix to the master or a higher-numbered submix.

Drag a fader, pan or send with your finger, and tap M, S or the output to change them.

## Using the Q-Links in the mixer

Tap a strip's name to select it. The four Q-Links then turn its **Volume**, **Pan**, **Send A** and **Send B**, and the data wheel turns its volume too.

## The master and Master safety

The **Master** strip on the right is everything you hear. Its fader is the same level as the master bar on the Play page and Q-Link 1 there.

The last thing on the master is **Master safety**, a limiter that keeps the output from going past -0.3 dB, so a loud moment never distorts harshly or hurts your ears. It shows how many times it has caught a peak. If it is catching all the time, turn something down.

> [!TIP]
> Distortion you want is fine: put Drive, Crush or Lo-fi on a strip and push it. Master safety only stops the final output from clipping by accident.

## The effects

These effects can go on any strip:

| Effect | What it does |
|---|---|
| **EQ** | Shapes the tone: cut mud, add air, thin out a sound. |
| **Multi filter** | Low-pass, high-pass, band-pass and more, with resonance. |
| **DJ filter** | One knob: turn left for a low-pass, right for a high-pass. |
| **Drive** | Warm saturation up to hard distortion. |
| **Comb** | Short tuned delays for metallic, resonant tones. |
| **Crush** | Bit and sample-rate reduction. |
| **Delay** | Echoes in time with the tempo. |
| **Chorus**, **Flanger**, **Phaser** | Movement and width. |
| **Lo-fi** | Old sampler and worn tape character. |
| **Compressor** | Evens out levels and adds punch. It can be keyed from another strip, for side-chain pumping. |
| **Cloud reverb** | A large, lush reverb. |

Adding, removing or reordering an effect while the music plays doesn't click: that strip dips out for a moment and comes back.

> [!NOTE]
> In the current test build the mixer shows each strip's effects, but tapping **+ Insert** to add one is not connected yet. The effect chain editor, where you add, reorder and bypass effects and turn their knobs, comes in the next version. Until then the Play page's own effects (below) are ready to use.

## The Play page's effects

Some effects are already in place when MPFree starts, and the Play page's Q-Links and touch controls play them:

- **A DJ filter** on the Drums and Loop tracks: the **Filter** Q-Link on the Play page.
- **A dotted-eighth delay** on send return A: the **Delay send** Q-Link.
- **The throws and the XY pad** on the master: see [Playing live](playing-live.md#throws).

Each of these costs almost nothing until you use it.

## Coming next

- **The chain editor**: add effects to any strip, reorder and bypass them, and edit them on a page of their own.
- **Macros**: one Q-Link that moves several controls at once, across effects and synths, set up from a macros panel.
- **Automation**: knob moves recorded into clips.

> [!MPC]
> The mixer covers the MPC's routing: pad channels, tracks, submixes, four send returns and the master, with sends before or after the fader and side-chain keys. The differences: the mixer opens over any page instead of being its own mode, solo keeps everything the strip feeds, the master always ends in a safety limiter, and effects with a delay of their own are lined up automatically so nothing drifts out of time.
