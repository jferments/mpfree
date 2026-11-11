# MPFree pad player: layout spec

**Status:** APPROVED by Jay on 2026-11-11 at 08:15Z ("Approve, Synth before Looper"). Every thread works to this. Any change to it needs Jay's approval again.
**Owner:** the Hello MPC thread ("Build round 6 with looper, synths and sampler"). Send layout proposals there.
**Shared layout means:** the top bar, the tab set and its order, the Play page's structure, the right-hand column, the pad grid, the hint line, Prefs, and the global buttons (MENU, SHIFT + MENU, STOP, SHIFT + STOP, NOTE REPEAT).

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
| Play | Pads, kits, level, timing; Note Repeat / arp | Hello MPC (arp inside it) |
| Arrange | The time engine's launch grid, record settings, takes (the Clips tab until Jay renamed it, 12:55Z) | Transport / time |
| Synth | The synth's pages (Poly, later FM) | Synth |
| Looper | The audio looper: loop waveform, Record / Overdub / Undo, line input meter | Transport / time |
| Sample | The pad editor: Wave, Chop, Play, Grain, FX (Jay, 07:12Z) | Sampler |
| Dev | Only when Prefs > "Show developer tools" is On | Hello MPC |

- **Dev's pages,** on a strip under the top bar: MIDI log | Performance | CPU by engine | Processes | Map controls | Console. CPU by engine and Processes were asked for by Jay at 08:19Z (the profiling thread builds them).
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
- **The loop strip** names each layer by what was played into it (Drums, Bass, Chop; Line in from the line). Speaker mutes per layer come with the looper's per-layer mix (#110).
- **Throws and the XY pad** play performance effects on the master while held; Drop drums mutes the kick, snare and hat pads while held.
- **The mini pads** have a heading: "Pads · Bank A" and the kit (the Drum view: how hits play, "velocity on"). While the synth has the pads, it shows the Chromatic | In key switch (Jay, 12:22Z). The same heading sits over the Synth tab's presets.
- **Q-Links on Play,** by view: All and Beats: swing, tune and the drums' filter and delay send (a picked step in Beats: its velocity, length, nudge and chance); Loop: loop level, loop delay send, line level, loop filter; Drum: tune, decay, swing, FX mix. The master bar sets the level. While NOTE REPEAT is held or the Arp panel is open, the cards show the arp's four (rate, gate, swing, octaves).
- **The header** keeps to the CPU and kit chips; the session chip shows only when something is wrong, the arp chip only while the arp, a repeat or a latch is on.
- **The hint line** starts where the panels do (left-aligned, as the mockups).
- **NOTE REPEAT's strip** takes the top of the view's first panel, so nothing else moves.
- **The Arp panel** takes the view's left, and the mini pads stay where the home view has them.
- **Still to come:** speaker mutes per loop layer (#110); half length and double on the loop strip (looper edit tools); the FX thread's Perform engine in place of the stand-in (#116); Macros as a Q-Link bank (mod #102).

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
- **SHIFT + STOP:** the Console. It works without developer tools too, but has no strip then.
- **NOTE REPEAT:** repeat. Press it twice, or SHIFT + it, to Latch.
- **NOTE REPEAT + MENU:** the Arp panel.
- **TRACK MIX:** the mixer. SHIFT + TRACK MIX: the pad mixer, as the FX thread proposed. The guide names the button "Track Mix / Pad Mix"; the pad player hasn't seen it yet, and round 7's mapper lists it.
- **REC, OVERDUB, UNDO:** they act on the page showing (Arrange, Looper, Sample), otherwise on the clips.
- **PAD BANK A–D:** banks. SHIFT, or a second press, gives E–H.

## 7. How changes happen

- **Each thread owns the inside of its own tab or panel.** A change there needs no layout approval.
- **Anything in the shared layout is a proposal.** Send it to the owner. Jay approves it, this file is updated, and then it lands.
- **Previews draw the real bar.** Every crate's preview pictures use the canonical tab list, `mpfree_ui::widgets::APP_TABS` (and `Header::app`), not a list of their own.

## Proposed, waiting for Jay (not part of the spec yet)

- **Pressure ring on the screen's pad tiles** (synth thread, 08:53Z, poly aftertouch design part 7): a thin ring showing each pad's shaped pressure.
- **Scales on the synth pads** (arp thread; design https://claude.ai/artifact/DceFbX4jtupvJUT4759ZBZ). **Approved by Jay 09:55Z:** the pads are chromatic by default (the scale's notes lit once one is picked), and one simple, direct action switches them to In key and back (Jay likes how the Hydrasynth does it). In key fills the rows in sequence by default, with "a fourth apart" as the other choice. While the synth is on the pads, PAD BANK A to H hold a pad layout per bank (approved for now, to discuss later). Chord pads: triads on the bottom two rows, sevenths on the top two. **Jay 12:22Z (card):** the switch is a Chromatic | In key chip in the pads panel header, on the Play page and on the Synth tab; one tap switches the bank playing, and the status chip reads "Poly · A minor · in key". SHIFT + PAD BANK stays banks E to H.
- **SHIFT + REC = Capture on every page** (time thread, 09:35Z, as the sequencer plan had it; patch shift-rec-capture-17e5961.patch). Without it, Capture works from the Takes panel's button on Arrange (#80).
- **A new Play page** (Jay, 09:34Z and 09:40Z): the big pads go. Play is the home for finger drumming, live looping and making beats, with a small pad map, the four Q-Links in a column beside the knobs, and a thin master bar. Mockups A to E (09:52Z, /mnt/project-files/hello-mpc/play-mockups/). **Jay 10:05Z:** the all-in-one layout (D) is the default home view, and a one-tap switch goes to the other views (Beats, Loop, Drum) and back; a speaker icon beside each instrument (and loop layer) mutes it, instead of a Mutes panel, and the space goes to XY or effects. Round 2 mockups F to I (10:08Z) show that; not built yet. **Jay 10:16Z, the direction for now:** the Looper tab stays and becomes the loop's editor and setup (mockup J-looper-tab, 10:14Z), while Play's looper strip is for performing the loop. The tab has a list of loops with a memory gauge; one lane per layer with level, pan, solo, mute and delete; half length, double, reverse, nudge, trim, seam fade, merging layers and the undo history; recording and input setup; and sending a loop to a pad, a clip, a save or resampling. Several loops, mixing per layer and the edit tools are new engine work for the time thread. With it: the Clips tab becomes **Arrange** (clip launching, takes, Capture; scenes and the song later), and the graphical pad editor becomes the first, half-screen sub-page of Sample (Pads, Wave, Chop, Play, Grain, FX).
- **The mixer's hardware controls** (FX thread, 10:09Z; panel in #83, `mpfree-mixer-ui`): while the mixer overlay is open, the data wheel sets the selected strip's volume, Q-Links 1-4 its volume, pan, send A and send B, and pads 1-16 select strips on the page. The panel sizes to whatever area the layout gives it: 10 strips and the master at full width, about 7 if the new Play page's right column stays up. **Hello proposes:** while the mixer is open, that column shows only the Q-Links (labelled from the selected strip), so there is no second master meter.
- **Per-pad pressure calibration** (synth thread, 08:53Z): a Touch-lab-style page, opened from a Prefs row, that stores a scale per pad. It needs saved settings (storage spec).
- **The FX chain editor and macros** (mod thread, 11:39Z; code in PR #102; pictures in /mnt/project-files/mod/chain-ui/, redrawn at 11:59Z to Hello's points below):
  - **The chain editor:** a track's effects in order (on/off, up, down, remove, "+ Add effect", up to 8), the selected effect's knobs, and a Modulate panel for the knob last touched (put it on macro 1 to 8, learn it to a Q-Link, add an LFO or a follower). It opens from a mixer strip's insert row and from the Sample tab's FX sub-tab.
  - **Macros:** the Play page's Q-Link column can show "Macros 1-4" or "Macros 5-8" as a bank. The full Macros and Morph panel (8 macros, up to 8 targets each, and a morph between two saved scenes A and B) is either a fifth Play view or opens from a long press on a macro card.
  - **Hello proposes:**
    - the full panel opens from a long press on a macro card, over the page and under the top bar as the mixer does, so Play keeps its four approved views and no tab is added;
    - from the mixer, the chain editor takes the mixer's place with a Back to the mixer; from Sample, it sits inside the FX sub-tab, under Sample's pad bar;
    - the Q-Link column is the width the Play page gives it, not a wider column of its own;
    - while the chain editor is open, the Q-Links default to the selected effect's first four knobs, with Macros 1-4 and 5-8 as banks;
    - the Q-Links reach the mod engine through Hello's binding hook (round 8), and their cards use the mixer's card shape (agreed with the mod thread at 11:12Z);
    - a Q-Link the user learns to a knob belongs to the page it was learned on; Macros 1-8 work on every page.
  - **Agreed between threads (12:04Z):** the Sample tab's FX sub-tab shows the pad's chain first, with a Pad / Track switch (sampler thread), and the insert row works as the mixer already sends it (FX thread).

- **Effect pages, the Perform FX page and the DJ knob** (FX thread, 14:06Z; code in PR #116, crate `mpfree-fx-ui`; previews in /mnt/project-files/fx/fx-ui/):
  - an effect's page (title, drawings, tiles, presets, its own Q-Link strip) opens over the current page under the top bar, with Back, as the mixer does: from a tap on the selected insert's name in the chain editor, and from a mixer return or master strip's insert row;
  - the Perform FX page (16 pads with lights, the XY pad with Hold, the selected pad's settings) opens the same way from the Perform FX insert on the master; its pads and XY view can also sit in a Play view;
  - the DJ knob is Q-Link 1 on that page (it stops at the middle on the way through), and can be a Play Q-Link too.
  - **Hello proposes:** the pages use the 168 px Q-Link column instead of a strip of their own, so the frame stays the same everywhere; Play's throws and XY pad move onto the Perform engine when #116 merges, with no layout change.

## Where things stand (2026-11-11 08:10Z)

| Where | Tab bar |
|---|---|
| main (= the round 5 test app) | Play, Debug, Console, Prefs |
| Round 6, #68 (merged) | Play, Clips, Synth, Console, Prefs (+ Debug in debug mode) |
| Looper patch (round 7) | Play, Clips, Synth, Looper, (Debug), Console, Prefs |
| #75 Sample tab (merged) | Play, Clips, Synth, Sample, (Debug), Console, Prefs |
| Round 7 | Play, Clips, Synth, Looper, Sample, (Dev); Prefs on SHIFT + MENU |
| Round 8 (this spec) | Play, Arrange, Synth, Looper, Sample, (Dev); Prefs on SHIFT + MENU |
| #72 themes, arp b1aa92e | No tab change (#72 restyles the drawing only) |
| Preview pictures (not the app) | synth-ui "Steps, Clips, Looper, Arrange, Synth, Mix"; time-ui "Play, Clips, Debug, Console, Prefs"; sampler-ui "Play, Sample, Debug, Prefs"; arp-ui "Play, Debug, Console, Prefs"; wave preview "Record, Sound, Perform, Browse" |
