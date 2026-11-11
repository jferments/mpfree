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
- **Active tab:** the page's own tab is highlighted. Prefs, menus and the guide highlight none.

## 3. The tabs

The music tabs, in this order:

| Tab | What it is | Owner thread |
|---|---|---|
| Play | Pads, kits, level, timing; Note Repeat / arp | Hello MPC (arp inside it) |
| Clips | The time engine's launch grid, record settings, takes | Transport / time |
| Synth | The synth's pages (Poly, later FM) | Synth |
| Looper | The audio looper: loop waveform, Record / Overdub / Undo, line input meter | Transport / time |
| Sample | The pad editor: Wave, Chop, Play, Grain, FX (Jay, 07:12Z) | Sampler |
| Dev | Only when Prefs > "Show developer tools" is On | Hello MPC |

- **Dev's pages,** on a strip under the top bar: MIDI log | Performance | CPU by engine | Processes | Map controls | Console. CPU by engine and Processes were asked for by Jay at 08:19Z (the profiling thread builds them).
- **Not tabs:**
  - **Prefs.** SHIFT + MENU opens it.
  - **Each page's options.** MENU opens them.
  - **The Arp panel.** NOTE REPEAT + MENU opens it; it belongs to Play.
  - **The pattern workspace.**
  - **The button guide.**
  - **System info, Touch lab and Calibrate touch.** These open from Prefs.
- **Adding, removing, renaming or reordering a tab** is a layout change, so it needs Jay's approval. Reserved for later, not built yet: Mix (the FX thread's mixer). The "Steps, Clips, Looper, Arrange, Synth, Mix" bar in the synth previews is a design mock, not this list.
- **The order is Play, Clips, Synth, Looper, Sample, then Dev.** Jay chose it at 08:15Z.

## 4. The Play page

- **Left: the 4 × 4 pads.** Their rectangle never moves or resizes, whatever the arp, the time engine or any panel shows.
- **Right: a 460 px column,** top to bottom:
  - the Note Repeat strip (56 px), only while NOTE REPEAT is held or pads are latched;
  - the kit list;
  - the level fader (96 px);
  - timing (232 px).
- **The Arp panel** takes the right-hand column while it is open. The pads stay put.
- **The transport bar** goes under the top bar when the time engine is on.

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
- **REC, OVERDUB, UNDO:** they act on the page showing (Clips, Looper, Sample), otherwise on the clips.
- **PAD BANK A–D:** banks. SHIFT, or a second press, gives E–H.

## 7. How changes happen

- **Each thread owns the inside of its own tab or panel.** A change there needs no layout approval.
- **Anything in the shared layout is a proposal.** Send it to the owner. Jay approves it, this file is updated, and then it lands.
- **Previews draw the real bar.** Every crate's preview pictures use the canonical tab list from one shared constant (to be added in mpfree-ui), not a list of their own.

## Proposed, waiting for Jay (not part of the spec yet)

- **Pressure ring on the screen's pad tiles** (synth thread, 08:53Z, poly aftertouch design part 7): a thin ring showing each pad's shaped pressure.
- **Per-pad pressure calibration** (synth thread, 08:53Z): a Touch-lab-style page, opened from a Prefs row, that stores a scale per pad. It needs saved settings (storage spec).

## Where things stand (2026-11-11 08:10Z)

| Where | Tab bar |
|---|---|
| main (= the round 5 test app) | Play, Debug, Console, Prefs |
| Round 6, #68 (what Jay ran; not merged yet) | Play, Clips, Synth, Console, Prefs (+ Debug in debug mode) |
| Looper patch (round 7) | Play, Clips, Synth, Looper, (Debug), Console, Prefs |
| #75 Sample tab (on hold) | Play, Clips, Synth, Sample, (Debug), Console, Prefs |
| #72 themes, arp b1aa92e | No tab change (#72 restyles the drawing only) |
| Preview pictures (not the app) | synth-ui "Steps, Clips, Looper, Arrange, Synth, Mix"; time-ui "Play, Clips, Debug, Console, Prefs"; sampler-ui "Play, Sample, Debug, Prefs"; arp-ui "Play, Debug, Console, Prefs"; wave preview "Record, Sound, Perform, Browse" |
