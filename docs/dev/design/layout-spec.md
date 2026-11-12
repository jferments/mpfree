# MPFree pad player: layout spec

**Status:** APPROVED by Jay on 2026-11-11 at 08:15Z ("Approve, Synth before Looper"). Every thread works to this. Any change to it needs Jay's approval again.
**Owner:** the app thread ("Build app rounds and own the layout"; it was the "Hello MPC" thread until Jay renamed the app mpfree, 22:36Z). Send layout proposals there.
**Shared layout means:** the top bar, the tab set and its order, the Play page's structure, the right-hand column, the pad grid, the hint line, Prefs, and the global buttons (MENU, SHIFT + MENU, STOP, SHIFT + STOP, the Console's chord, NOTE REPEAT, the mode buttons).

## 1. The screen

- **Size:** 1280 × 800, landscape.
- **Every page, top to bottom:**
  - the top bar, 60 px;
  - the time engine's transport bar, on pages that use it;
  - the page's own content;
  - the hint line, 26 px, which says what the buttons do on that page.
- **Full-screen pages** (Loading, Touch lab, Calibrate touch) have no top bar. Any page change or MENU closes them.

## 2. The top bar

- **Tabs** start at the left edge. There is no MPFree wordmark: that belongs to the startup and loading screens only (Jay, 08:02Z).
- **Status chips** sit on the right: session, kit and bank (or "Poly · C3 to D#4" when the synth is on the pads), level, tempo, and the arp chip.
  - Chips never push tabs off the bar, and tabs never push chips off.
- **Width:** up to 6 tabs are drawn at full width (112 px). More tabs share that room.
- **Active tab:** the page's own tab is highlighted. Prefs, the mixer, menus and the guide highlight none.
- **The mixer icon:** a small mixer (sliders) icon at the bar's right edge, after the chips, once the mixer is built. A tap opens the mixer (Jay, 09:17Z).

## 3. The tabs

The music tabs, in this order:

| Tab | What it is | Owner thread |
|---|---|---|
| Play | Pads, kits, level, timing; Note Repeat / arp | App (arp inside it) |
| Arrange | The time engine's launch grid, record settings, takes (the Clips tab until Jay renamed it, 12:55Z) | Transport / time |
| Synth | The synth's pages (Poly, later FM) | Synth |
| Looper | The loops' editor and setup (mockup J): the loops, each layer's mix, recording and input, using a loop | App (the looper engine: transport / time) |
| Sample | The pad editor: Wave, Chop, Play, Grain, FX (Jay, 07:12Z) | Sampler |
| Dev | Only when Prefs > "Show developer tools" is On | App |

- **Dev's pages,** on a strip under the top bar: MIDI log | Performance | CPU by engine | Processes | Map controls | Touch | Console. CPU by engine and Processes were asked for by Jay at 08:19Z (the profiling thread builds them). Touch is the graphical touch calibration and testing tool with its tuning (Jay, 22:23Z; the touchscreen thread builds it, with a "Touch targets" page): the wheel and + / - step the tuning, SHIFT + wheel still changes Dev page, and MENU there offers Calibrate touch and Touch lab.
- **Not tabs:**
  - **Prefs.** SHIFT + MENU opens it.
  - **Each page's options.** MENU opens them.
  - **The Arp panel.** NOTE REPEAT + MENU opens it; it belongs to Play.
  - **The mixer.** TRACK MIX or the mixer icon opens it over the current page, under the top bar, as Prefs does. The same button or icon again, or a tab, closes it (Jay, 09:17Z: the button or a small sliders icon in a corner; the FX thread builds it).
  - **The pattern workspace.**
  - **The button guide.**
  - **System info, Touch lab and Calibrate touch.** These open from Prefs.
- **Adding, removing, renaming or reordering a tab** is a layout change, so it needs Jay's approval. There is no Mix tab: the mixer opens with TRACK MIX (above). The "Steps, Clips, Looper, Arrange, Synth, Mix" bar in the synth previews is a design mock, not this list.
- **The order is Play, Arrange, Synth, Looper, Sample, then Dev.** Jay chose it at 08:15Z, and renamed Clips to Arrange at 12:55Z.

## 4. The Play page

Built in round 8 (PR #106) to Jay's approved mockups F to I (10:05Z), with real data and working controls (13:56Z on; side-by-side sheets in /mnt/project-files/hello-mpc/round8-play/compare/). The old big pads are gone.

- **The frame, in every view:**
  - the transport bar: the state, the position, the tempo, the bar and step dots, and the view switch (All, Beats, Loop, Drum) on its right. One tap changes the view, and the screen keeps it while other tabs show;
  - the view under it;
  - at the right edge, from under the top bar to the hint line: the master bar (66 px: meter with peak hold, level handle, level, LIM when the limiter works) and the Q-Link column (168 px: four cards, knob 1 at the top, each with its scope, name, ring and value). A drag on a card turns that knob, and a finger on a knob lights its card.
- **The views:**
  - All (home): the loop strip (150 px), the pattern beside the mini pads (270 px), then the throws beside the XY pad (280 px).
  - Beats: the whole pattern (every lane), then the mini pads (300 px) beside all eight throws. A tap on a lit step picks it for the Q-Links (velocity, length, nudge, chance).
  - Loop: the loop big with its buttons (Record, Overdub, Stop, Undo, Redo, Mute, Clear) and the input in its heading, then the mini pads, four throws and the XY pad.
  - Drum: Your timing (the last two bars of hits against the 1/16 grid, average, spread, velocity, spread of offsets) beside the XY pad (330 px), then the mini pads and all eight throws.
- **The pattern** is the armed track's clip from the sequencer. A speaker beside each lane mutes that pad; a tap on a step toggles it (or starts a pattern when there is none).
- **The loop strip** names each layer by what was played into it (Drums, Bass, Chop; Line in from the line). A speaker beside each name mutes that layer.
- **Throws and the XY pad** play Perform FX (`mpfree_fx::Perform`) on the master while held: each throw holds a pad of its default bank (Delay throw the 1/8 dotted echo throw, Reverb wash the wash throw, Filter dip the 1-bar low-pass sweep), entering on the next 1/16 while the transport plays; the XY pad plays its DJ filter across (low-pass left, off in the middle, high-pass right) and its echo up, and glides home when the finger lifts; the FX mix Q-Link is its Mix (round 10); Drop drums mutes the kick, snare and hat pads while held.
- **The mini pads** have a heading: "Pads · Bank A" and the kit (the Drum view: how hits play, "velocity on"). While the synth has the pads, it shows the Chromatic | In key switch (Jay, 12:22Z). The same heading sits over the Synth tab's presets.
- **Q-Links on Play,** by view: All and Beats: swing, tune and the drums' filter and delay send (a picked step in Beats: its velocity, length, nudge and chance); Loop: loop level, loop delay send, line level, loop filter; Drum: tune, decay, swing, FX mix. The master bar sets the level. While NOTE REPEAT is held or the Arp panel is open, the cards show the arp's four (rate, gate, swing, octaves).
- **The header** keeps to the CPU and kit chips; the session chip shows only when something is wrong, the arp chip only while the arp, a repeat or a latch is on.
- **The hint line** starts where the panels do (left-aligned, as the mockups).
- **NOTE REPEAT's strip** takes the top of the view's first panel, so nothing else moves.
- **The Arp panel** takes the view's left, and the mini pads stay where the home view has them.
- **Macro banks:** Q-LINK steps the Q-Link column to Macros 1-4, then Macros 5-8, then back to the view's own knobs; Play keeps its bank while other pages show. A finger held on a macro card opens the macros panel (section 4b).
- **Still to come:** half length and double on the loop strip (the looper's edit tools, which the Looper tab has since round 10).

## 4a. The Looper tab

Built in round 9 to mockup J (Jay's direction, 10:16Z), with real data; side-by-side sheet in /mnt/project-files/hello-mpc/round9-looper/compare/. The Play page's strip performs the loop; this tab edits and sets it up.

- **The frame:** the loops (220 px) at the left; the editor over Recording and input and Use the loop (330 px) in the middle; the Q-Link column (168 px) at the right; the hint line under them.
- **The loops:** Loop 1 to 4, and on as far as one is used or picked (eight in all), each with its length and layers and its state (PLAYING, MUTED, RECORDING); the picked one outlined. A tap picks a loop; + New loop picks the first empty one. Every loop plays at once; the controls act on the picked one. The loop memory gauge sits at the bottom.
- **The editor:** the tools row (half length, double, reverse, nudge, seam fade, merge layers, undo history; they work while the loop plays and isn't recording, and merge needs two layers; seam fade steps through a cut, 10 ms, 50 ms and the longest, and shows the fade now; merge shows its progress), the loop's wave with its bar lines and the playhead (while the loop can be edited, a handle at each end of the wave drags in by 1/16 notes, the part cut off dimmed and "keep 12 of 16 beats" above it, and trims the loop on release), and a row per layer: its speaker (mute), S (solo), level bar (drag: -24 to +6 dB), mini wave and × (delete). A layer Fade old has turned down says how far ("faded to 77 %"). A tap on a row puts that layer on the Q-Links. Three rows show; more scroll to keep the picked one in view. **Undo history** (lit while open) lists the loop's history in the layers' place: Start (the first pass), then each overdub, delete, edit and merge, oldest first, with now outlined and the steps Redo would bring back faint below it. A tap on a step goes back or forward to just after it; a drag scrolls the list; picking another loop closes it.
- **Recording and input,** two columns: Record from, First pass, Start, After recording, At another tempo; Line input, Input peak, Monitor, Latency. A tap on a row's right half steps it up, the left half down; the source and on-off rows flip, and After recording flips between play and overdub (the picked loop's own). The Play strip uses the same settings.
- **Use the loop:** To a pad, To a clip, Save loop, Resample. Resample picks the first empty loop and records the loops as heard into it from the next bar line, as REC does; REC ends the pass. It works while a loop plays and one is empty.
- **Q-Links:** the picked layer's level and pan, then the loop's Fade old (50 to 100 % kept a pass, a percent a detent) and its seam fade (a millisecond a detent), as mockup J. The loop's level and the line input's are on Play's Loop view.
- **Hardware:** REC records or overdubs, OVERDUB overdubs, UNDO undoes (SHIFT: redo); there are no on-screen record buttons here (the Play strip has them).
- **Still to come (drawn faint until then):** Start, and Use the loop's To a pad, To a clip and Save loop (the looper's bounce).


## 4b. The FX chain editor and macros

Approved by Jay at 22:34Z (the mod thread's card, "Approve as proposed"). The mod thread builds them (crate `mpfree-chain-ui`, PR #102 merged; pictures in /mnt/project-files/mod/chain-ui/).

- **The chain editor:** a track's effects in order (on/off, up, down, remove, "+ Add effect", up to 8), the selected effect's knobs, and a Modulate panel for the knob last touched (put it on macro 1 to 8, learn it to a Q-Link, add an LFO or a follower).
- **Where it opens:** from a mixer strip's insert row, where it takes the mixer's place with a Back to the mixer; and in the Sample tab's FX sub-tab, under Sample's pad bar, showing the pad's chain first with a Pad / Track switch (sampler thread). The insert row works as the mixer already sends it (FX thread).
- **Macros:** the Q-Link column can show "Macros 1-4" or "Macros 5-8" as a bank, on Play and while the chain editor is open. A long press on a macro card opens the full Macros and Morph panel (8 macros, up to 8 targets each, and a morph between two saved scenes A and B) over the page and under the top bar, as the mixer does. Play keeps its four views and no tab is added.
- **Q-Links:** the column is the width the page gives it (168 px), not a wider one. While the chain editor is open they default to the selected effect's first four knobs, with Macros 1-4 and 5-8 as banks. Their cards use the mixer's card shape. A Q-Link the user learns to a knob belongs to the page it was learned on; Macros 1-8 work on every page.
- **The Q-Link hook:** the app routes Q-Link turns per page in one place (main.rs, the `QLinkTurn` arm). A macro bank, when shown, takes the turns there first.
- **Built (round 10):** a strip's insert row opens the editor on that effect, and its "+" opens it with the effect picker up; it takes the mixer's place with no tab lit, and Back opens the mixer again on the view it showed (Track Mix, Pad Mix and so on). The wheel is Q-Link 1. Q-LINK steps the column through the effect's knobs, Macros 1-4 and Macros 5-8; a learn takes the next Q-Link turned. A finger held for half a second on a macro card (Play or the editor) opens the macros panel on that card's bank, and its Back goes back to the page it opened over. Still to come: Sample > FX (the sampler's sub-tab), the effect's own page from a tap on its name, and keeping the macro set-up and learned Q-Links in the store.

## 5. Prefs (SHIFT + MENU)

- **Main rows, in order:**
  1. Pad velocity curve
  2. Latch keeps pads
  3. Audio buffer
  4. Find the buttons again
  5. Show developer tools
  6. Map controls
  7. Line input
  8. System info
  9. Touch screen (opens its own rows)
- **The Touch screen rows** ("Prefs · Touch screen", approved in the touch thread at 08:13Z, #69):
  1. Back
  2. Screen pad velocity
  3. Calibrate touch
  4. Touch lab
- **Touch screen stays the last main row, and Touch lab its last row,** because the runbook names them by place.
- **Prefs always opens on its main rows.** On System info or the Touch screen rows, MENU (or Back) goes back to them.
- **System info** shows the program's version and commit, the OS, the kernel and the hardware.
- **Line input** is Off at every start.

## 6. The buttons everywhere

- **MENU:** the page's options. On System info or the Touch screen rows, back to Prefs.
- **SHIFT + MENU:** Prefs.
- **STOP:** the transport, then silence.
- **SHIFT + STOP:** as on the MPC: stop, silence everything and go back to bar 1 (Jay, 20:40Z, control map decision 2).
- **The Console:** hold SHIFT, hold ERASE, then press TAP TEMPO (and the same again to go back). Jay wants a chord nothing else uses, since only developers open it (22:12Z); the hardware control map thread picked this one. The press is no tempo tap, and nothing fires on the way into the chord. The Dev tab's Console page opens it too. It works without developer tools, but has no strip then.
- **ERASE and SHIFT + ERASE** do nothing yet (live erase, and the FX lock mode with a step held, come later).
- **NOTE REPEAT:** repeat. Press it twice, or SHIFT + it, to Latch.
- **NOTE REPEAT + MENU:** the Arp panel.
- **TRACK MIX:** the mixer. SHIFT + TRACK MIX: the pad mixer, as the FX thread proposed. The guide names the button "Track Mix / Pad Mix"; the pad player hasn't seen it yet, and round 7's mapper lists it.
- **REC, OVERDUB, UNDO:** they act on the page showing (Arrange, Looper, Sample), otherwise on the clips.
- **PAD BANK A–D:** banks. SHIFT, or a second press, gives E–H.
- **SHIFT + REC:** Capture, on every page.
- **SHIFT + PLAY START:** the metronome on or off.
- **PLAY while the transport plays:** FILL while it is held (notes with the FILL and !FILL trig conditions follow it); pressed twice, FILL stays latched until PLAY is pressed again or STOP (Jay, 20:54Z, control map decision 9). Stopped, PLAY plays.
- **The mode buttons** go to the page with the same job (Jay, 20:40Z; the control map doc, https://claude.ai/artifact/CzSwDDjX1Ngc7LkhXP4KqD):
  - MAIN: Play; on Play, the next view (All, Beats, Loop, Drum).
  - STEP SEQ: Play's Beats view.
  - NEXT SEQ: Arrange.
  - SAMPLER: Sample. SHIFT + SAMPLER: Looper.
  - SAMPLE EDIT: Sample, on its Wave editor.
  - PROG EDIT: the sound the pads play: the Synth tab while the synth has them, otherwise Sample > Pads.
  - SHIFT + 16 LEVEL: the synth pads' Chromatic | In key switch.
  - Q-LINK: the Q-Link column's next bank on Play, the chain editor and the macros panel (section 4b).
- **Found with Map controls first:** ERASE, Q-LINK and the mode buttons have no measured note yet, so each works once Map controls has found it (TRACK MUTE, NEXT SEQ, STEP SEQ, SAMPLER, SAMPLE EDIT, PROG EDIT and the data dial's push have rows since round 9).
- **Still to come from the control map:** TRACK MUTE (the pads as mute keys); hold MAIN + pad to pick a track; SHIFT + NEXT SEQ (Perform FX); 16 LEVEL, COPY, TC, BROWSE and the data dial's push.

## 7. How changes happen

- **Each thread owns the inside of its own tab or panel.** A change there needs no layout approval.
- **Anything in the shared layout is a proposal.** Send it to the owner. Jay approves it, this file is updated, and then it lands.
- **Previews draw the real bar.** Every crate's preview pictures use the canonical tab list, `mpfree_ui::widgets::APP_TABS` (and `Header::app`), not a list of their own.

## Proposed, waiting for Jay (not part of the spec yet)

- **Pressure ring on the screen's pad tiles** (synth thread, 08:53Z, poly aftertouch design part 7): a thin ring showing each pad's shaped pressure.
- **Scales on the synth pads** (arp thread; design https://claude.ai/artifact/DceFbX4jtupvJUT4759ZBZ). **Approved by Jay 09:55Z:** the pads are chromatic by default (the scale's notes lit once one is picked), and one simple, direct action switches them to In key and back (Jay likes how the Hydrasynth does it). In key fills the rows in sequence by default, with "a fourth apart" as the other choice. While the synth is on the pads, PAD BANK A to H hold a pad layout per bank (approved for now, to discuss later). Chord pads: triads on the bottom two rows, sevenths on the top two. **Jay 12:22Z (card):** the switch is a Chromatic | In key chip in the pads panel header, on the Play page and on the Synth tab; one tap switches the bank playing, and the status chip reads "Poly · A minor · in key". SHIFT + PAD BANK stays banks E to H.
- **SHIFT + REC = Capture on every page** (time thread, 09:35Z, as the sequencer plan had it; patch shift-rec-capture-17e5961.patch). Without it, Capture works from the Takes panel's button on Arrange (#80).
- **A new Play page** (Jay, 09:34Z and 09:40Z): the big pads go. Play is the home for finger drumming, live looping and making beats, with a small pad map, the four Q-Links in a column beside the knobs, and a thin master bar. Mockups A to E (09:52Z, /mnt/project-files/hello-mpc/play-mockups/). **Jay 10:05Z:** the all-in-one layout (D) is the default home view, and a one-tap switch goes to the other views (Beats, Loop, Drum) and back; a speaker icon beside each instrument (and loop layer) mutes it, instead of a Mutes panel, and the space goes to XY or effects. Round 2 mockups F to I (10:08Z) show that; not built yet. **Jay 10:16Z, the direction for now:** the Looper tab stays and becomes the loop's editor and setup (mockup J-looper-tab, 10:14Z), while Play's looper strip is for performing the loop. The tab has a list of loops with a memory gauge; one lane per layer with level, pan, solo, mute and delete; half length, double, reverse, nudge, trim, seam fade, merging layers and the undo history; recording and input setup; and sending a loop to a pad, a clip, a save or resampling. Several loops, mixing per layer and the edit tools are new engine work for the time thread. With it: the Clips tab becomes **Arrange** (clip launching, takes, Capture; scenes and the song later), and the graphical pad editor becomes the first, half-screen sub-page of Sample (Pads, Wave, Chop, Play, Grain, FX).
- **The mixer's hardware controls** (FX thread, 10:09Z; panel in #83, `mpfree-mixer-ui`): while the mixer overlay is open, the data wheel sets the selected strip's volume, Q-Links 1-4 its volume, pan, send A and send B, and pads 1-16 select strips on the page. The panel sizes to whatever area the layout gives it: 10 strips and the master at full width, about 7 if the new Play page's right column stays up. **The app thread proposes:** while the mixer is open, that column shows only the Q-Links (labelled from the selected strip), so there is no second master meter.
- **Per-pad pressure calibration** (synth thread, 08:53Z): a Touch-lab-style page, opened from a Prefs row, that stores a scale per pad. It needs saved settings (storage spec).
- **Effect pages, the Perform FX page and the DJ knob** (FX thread, 14:06Z; code in PR #116, crate `mpfree-fx-ui`; previews in /mnt/project-files/fx/fx-ui/):
  - an effect's page (title, drawings, tiles, presets, its own Q-Link strip) opens over the current page under the top bar, with Back, as the mixer does: from a tap on the selected insert's name in the chain editor, and from a mixer return or master strip's insert row;
  - the Perform FX page (16 pads with lights, the XY pad with Hold, the selected pad's settings) opens the same way from the Perform FX insert on the master; its pads and XY view can also sit in a Play view;
  - the DJ knob is Q-Link 1 on that page (it stops at the middle on the way through), and can be a Play Q-Link too.
  - **The app thread proposes:** the pages use the 168 px Q-Link column instead of a strip of their own, so the frame stays the same everywhere; Play's throws and XY pad are on the Perform engine since round 10, with no layout change.

## Where things stand (2026-11-11 08:10Z)

| Where | Tab bar |
|---|---|
| main (= the round 5 test app) | Play, Debug, Console, Prefs |
| Round 6, #68 (merged) | Play, Clips, Synth, Console, Prefs (+ Debug in debug mode) |
| Looper patch (round 7) | Play, Clips, Synth, Looper, (Debug), Console, Prefs |
| #75 Sample tab (merged) | Play, Clips, Synth, Sample, (Debug), Console, Prefs |
| Round 7 | Play, Clips, Synth, Looper, Sample, (Dev); Prefs on SHIFT + MENU |
| Round 8 (merged) | Play, Arrange, Synth, Looper, Sample, (Dev); Prefs on SHIFT + MENU |
| Round 9 (this spec) | No tab change (the Looper tab becomes mockup J's editor) |
| #72 themes, arp b1aa92e | No tab change (#72 restyles the drawing only) |
| Preview pictures (not the app) | synth-ui "Steps, Clips, Looper, Arrange, Synth, Mix"; time-ui "Play, Clips, Debug, Console, Prefs"; sampler-ui "Play, Sample, Debug, Prefs"; arp-ui "Play, Debug, Console, Prefs"; wave preview "Record, Sound, Perform, Browse" |
