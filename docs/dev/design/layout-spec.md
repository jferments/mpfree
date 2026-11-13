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
- **The I/O icon:** a small in-and-out icon (one arrow in, one arrow out) just left of the mixer icon. A tap opens the mixer on its I/O view: the project's routing (audio ins and outs, MIDI ports, clock and sync, CV), saved with the project. The same icon again, or a tab, closes it, as with the mixer. A tap on the tempo chip opens the same view at Sync. (Jay's picks on the MIDI thread's cards, 12 Nov 08:38Z and 08:39Z; MIDI and sync are project settings, not Prefs, 08:26Z. The FX thread owns the mixer and its frame; the MIDI thread builds the MIDI, sync and CV sections.)

## 3. The tabs

The music tabs, in this order:

| Tab | What it is | Owner thread |
|---|---|---|
| Play | Kits, level, timing, the pattern, the loop, throws; Note Repeat / arp | App (arp inside it) |
| Pads | The pad bank beside the browser or the pad's settings (section 4e; Jay, 12 Nov 15:46Z) | App (the browser and the sampler's pad screens inside it) |
| Arrange | The time engine's launch grid, record settings, takes (the Clips tab until Jay renamed it, 12:55Z) | Transport / time |
| Synth | The synth's pages (Poly, later FM) | Synth |
| Looper | The loops' editor and setup (mockup J): the loops, each layer's mix, recording and input, using a loop | App (the looper engine: transport / time) |
| Sample | The sound editor: Wave, Warp, Chop, Play, Grain, FX, Rec, with Send to pad and Chop to pads (Jay, 07:12Z; its Pads sub-tab moves to the Pads tab, 12 Nov 15:46Z) | Sampler |
| Dev | Only when Prefs > "Show developer tools" is On | App |

- **Dev's pages,** on a strip under the top bar: MIDI log | Performance | CPU by engine | Processes | Map controls | Touch | Console. CPU by engine and Processes were asked for by Jay at 08:19Z (the profiling thread builds them). Touch is the graphical touch calibration and testing tool with its tuning (Jay, 22:23Z; the touchscreen thread builds it, with a "Touch targets" page): the wheel and + / - step the tuning, SHIFT + wheel still changes Dev page, and MENU there offers Calibrate touch and Touch lab.
- **Not tabs:**
  - **Prefs.** SHIFT + MENU opens it.
  - **Each page's options.** MENU opens them.
  - **The Arp panel.** NOTE REPEAT + MENU opens it; it belongs to Play.
  - **The mixer.** TRACK MIX or the mixer icon opens it over the current page, under the top bar, as Prefs does. The same button or icon again, or a tab, closes it (Jay, 09:17Z: the button or a small sliders icon in a corner; the FX thread builds it).
  - **The browser.** BROWSE opens it over the current page, under the top bar, as the mixer does; BROWSE again or a tab closes it (controls decision 13; Jay after live test 3, 13 Nov 11:46Z; built by the project thread). Its Projects place has New project, Save as and Save. The Pads tab's Browse view stays the narrow picker of section 4e.
  - **The pattern workspace.**
  - **The button guide.**
  - **System info, Touch lab and Calibrate touch.** These open from Prefs.
- **Adding, removing, renaming or reordering a tab** is a layout change, so it needs Jay's approval. There is no Mix tab: the mixer opens with TRACK MIX (above). The "Steps, Clips, Looper, Arrange, Synth, Mix" bar in the synth previews is a design mock, not this list.
- **The order is Play, Arrange, Synth, Looper, Sample, then Dev.** Jay chose it at 08:15Z, and renamed Clips to Arrange at 12:55Z.

## 4. The Play page

Built in round 8 (PR #106) to Jay's approved mockups F to I (10:05Z), with real data and working controls (13:56Z on; side-by-side sheets in /mnt/project-files/hello-mpc/round8-play/compare/). The old big pads are gone, and since 12 Nov the mini pads too: Jay saw no benefit in them on the home screen (08:11Z) and picked "the pattern fills the room" (15:45Z, mockups in /mnt/project-files/hello-mpc/pads-mockups/). The pads live on the Pads tab (section 4e).

- **The frame, in every view:**
  - the transport bar: the state, the position, the tempo, the bar and step dots, and the view switch (All, Beats, Loop, Drum) on its right. One tap changes the view, and the screen keeps it while other tabs show;
  - the view under it;
  - at the right edge, from under the top bar to the hint line: the master bar (66 px: meter with peak hold, level handle, level, LIM when the limiter works) and the Q-Link column (168 px: four cards, knob 1 at the top, each with its scope, name, ring and value). A drag on a card turns that knob, and a finger on a knob lights its card.
- **The views:**
  - All (home): the loop strip (150 px), the pattern across the whole width, then the throws beside the XY pad (280 px).
  - Beats: the whole pattern (every lane), then all eight throws across the width. A tap on a lit step picks it for the Q-Links (velocity, length, nudge, chance).
  - Loop: the loop big with its buttons (Record, Overdub, Play, Stop, Undo, Redo, Mute, Clear; Record has a red dot, Overdub a red ring, Play a triangle and Stop a square, and Play was added, Jay 12 Nov 08:11Z) and the input in its heading, then the throws beside the XY pad (300 px).
  - Drum: Your timing (the last two bars of hits against the 1/16 grid, average, spread, velocity, spread of offsets) beside the XY pad (330 px), then all eight throws across the width.
- **The pattern** is the armed track's clip from the sequencer. A speaker beside each lane mutes that pad; a tap on a step toggles it (or starts a pattern when there is none).
- **The loop strip** names each layer by what was played into it (Drums, Bass, Chop; Line in from the line). A speaker beside each name mutes that layer.
- **Throws and the XY pad** play Perform FX (`mpfree_fx::Perform`) on the master while held: each throw holds a pad of its default bank (Delay throw the 1/8 dotted echo throw, Reverb wash the wash throw, Filter dip the 1-bar low-pass sweep), entering on the next 1/16 while the transport plays; the XY pad plays its DJ filter across (low-pass left, off in the middle, high-pass right) and its echo up, and glides home when the finger lifts; the FX mix Q-Link is its Mix (round 10); Drop drums mutes the kick, snare and hat pads while held.
- **No pads panel** (Jay, 12 Nov 15:45Z). The Chromatic | In key switch for the synth pads (Jay, 12:22Z) is in the heading over the Synth tab's presets, and SHIFT + 16 LEVEL works everywhere.
- **Q-Links on Play,** by view: All and Beats: swing, tune and the drums' filter and delay send (a picked step in Beats: its velocity, length, nudge and chance); Loop: loop level, loop delay send, line level, loop filter; Drum: tune, decay, swing, FX mix. The master bar sets the level. While NOTE REPEAT is held or the Arp panel is open, the cards show the arp's four (rate, gate, swing, octaves).
- **The header** keeps to the CPU and kit chips; the session chip shows only when something is wrong, the arp chip only while the arp, a repeat or a latch is on.
- **The hint line** starts where the panels do (left-aligned, as the mockups).
- **NOTE REPEAT's strip** takes the top of the view's first panel, so nothing else moves.
- **The Arp panel** takes the view under the transport bar.
- **Macro banks:** Q-LINK steps the Q-Link column to Macros 1-4, then Macros 5-8, then back to the view's own knobs; Play keeps its bank while other pages show. A finger held on a macro card opens the macros panel (section 4b).
- **Still to come:** half length and double on the loop strip (the looper's edit tools, which the Looper tab has since round 10).

## 4a. The Looper tab

Built in round 9 to mockup J (Jay's direction, 10:16Z), with real data; side-by-side sheet in /mnt/project-files/hello-mpc/round9-looper/compare/. The Play page's strip performs the loop; this tab edits and sets it up.

- **The frame:** the loops (220 px) at the left; the editor over Recording and input and Use the loop (330 px) in the middle; the Q-Link column (168 px) at the right; the hint line under them.
- **The loops:** Loop 1 to 4, and on as far as one is used or picked (eight in all), each with its length and layers and its state (PLAYING, MUTED, RECORDING); the picked one outlined. A tap picks a loop; + New loop picks the first empty one. Every loop plays at once; the controls act on the picked one. Each card has a speaker: a tap mutes or unmutes that loop without picking it, so a loop can be kept out of PLAY (Jay, 13 Nov 06:27Z). Under + New loop, Clear loop (outlined in red) clears the picked loop, first pass included; the × on a loop's last layer does the same. The loop memory gauge sits at the bottom ("1:42 of 5 min"); its size is Prefs > Loop memory, 1 to 20 minutes from the next start.
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

## 4c. Audio tracks in the Song view

**Approved by Jay on 12 Nov 2026 (05:33Z), as proposed** in mockups A to C (/mnt/project-files/audiotracks/mockup/). The audio tracks thread built the engine. The time thread draws the lanes and take lanes. The app thread wires the header, panel, bottom row and Q-Links. No tab is added: everything is inside Arrange's Song view.

- **Audio lanes:**
  - They sit under the MIDI lanes, with a thin divider. They share the ruler, the playhead, bar scrolling and the lane height (up to 56 px).
  - A lane shows once its track has a region or is armed. The last lane is "+ Audio", which adds the next track (up to 8).
  - Regions are blocks in the track's colour with the take's waveform. An older take has a "take N" tag, and fades show as slopes.
  - While recording, a red block grows from the punch in.
  - A tap selects a region. The selection has a kind (placement or audio region).
- **The header (the 96 px name area):**
  - It shows the name, an arm dot (tap to arm), an input chip (L, R, L+R, ST) and a 3 px level bar along its bottom edge.
  - Tapping the name opens a panel over the lanes with Arm, Input (Left by default), Monitor (Auto, In, Off), level, pan, mute, solo and Takes.
- **Q-Links: Play's Q-Link column, always on Arrange** (Jay's pick on 12 Nov 2026 at 06:01Z, mockup E in /mnt/project-files/audiotracks/mockup/qlinks-right/).
  - It's the same 168 px column with four vertical cards as on Play, at the right edge from under the top bar to the bottom. The Record and Takes panels narrow to about 250 px to make room, and the arp chip leaves the transport bar on Arrange.
  - The cards follow the selection. With an audio region selected, or the track panel open, they are Q1 Level, Q2 Pan (both the track's), Q3 the region's gain and Q4 its fade.
  - With nothing selected, they are Play's knobs, the same cards as on Play. The mockup showed the armed track's level, pan and sends, but a sequencer track has no mixer strip of its own yet: every track plays the kit. Track knobs come when tracks get strips.
  - Q-LINK steps to Macros 1-4 and 5-8 as on Play, and Play and Arrange share the bank. The track panel no longer lists the Q-Links at its foot.
- **The bottom row with an audio region selected:** Split · Erase · Fades · Gain · Takes · Undo, instead of the placement buttons. SHIFT + Undo redoes.
  - Dragging a region moves it, snapped to the grid (1/16 by default). SHIFT + drag moves it freely.
  - With nothing selected, the row is the Song view's own.
- **Take lanes (comping):**
  - Takes opens the track's takes as thin lanes (about 28 px each) under its lane, newest first. They borrow height from the other lanes.
  - The Song view gains a vertical lane scroll: a drag on the name column. SHIFT + wheel keeps scrolling the bars, as it already did on the Song view (12 Nov 06:45Z, time thread and app thread).
  - Swiping across a take lane plays that stretch on the track, snapped to the grid (SHIFT + swipe is free). Tapping a whole take lane uses that take everywhere it has audio.
- **Buttons:**
  - REC records an armed audio track in song mode as the transport records. There is no new button.
  - Audio tracks play in song mode only, and Rec song doesn't arm them.
- **Memory:** "Audio 0:42 of 1:00 free" on the Song view's legend line while any audio track exists.
- **Prefs:** two rows come with it:
  - **Audio track memory:** 60 s stereo by default, applied at the next start.
  - **Line input latency:** in frames, shared with the looper.

  They go before System info when built, and Touch screen stays last.

## 4d. MIDI

**Agreed by Jay on 12 Nov 2026** (08:23Z "go for it" on the MIDI thread's mockups, https://claude.ai/artifact/5ygP92smCQVhu31qq2kfYZ, and the design doc's decisions, https://claude.ai/artifact/RxdZ76ve5hE5VptdkU3JMo; 08:26Z "I agree with all of your other recommendations"). The MIDI thread builds it.

- **The tempo chip** reads "120.0 from MIDI In", with a green dot, while the tempo follows an outside clock.
- **A track's settings on Arrange:** Instrument gains "External MIDI", with one "Plays on" field that holds the port and the channel together. Under the track name it says "MIDI Out · ch 1", or "Sends nothing: no port" in the warn colour. There is one MIDI output setting per track, never two look-alike settings (Jay, 12 Nov 07:00Z).
- **Dev's MIDI log page** gains port and kind filters, and Clock, Timing and Counts panels.
- **MIDI Learn:** SHIFT + a touch on a value. It takes values only: SHIFT + drag on an audio region and SHIFT + swipe on a take lane keep their jobs (section 4c).
- **The MIDI, sync and CV settings** are the project's, in the mixer's I/O view (section 2, the I/O icon). Facts about the rig (CV tuning, port names, delays) stay in Prefs.

## 4e. The Pads tab

**Picked by Jay on 12 Nov 2026** (15:46Z, "Pads + Browse / Pad switch"; his ask at 08:11Z: "the pads are the centerpiece of the MPC, and deserve their own top menu"). Mockups pads-1-browse and pads-2-pad in /mnt/project-files/hello-mpc/pads-mockups/. Built to them by the app thread on 12 Nov (screens `pads-browse` and `pads-pad` in `mpfree --preview`).

- **Its place:** a top-level tab between Play and Arrange (Play, Pads, Arrange, Synth, Looper, Sample).
- **The pad bank on the left** (400 to 440 px), as the Sample tab's Pads sub-page had it: the bank's 16 pads as on the MPC (pad 1 bottom left), each with its name, layers and play mode; a tap picks a pad, a tap on the pad picked plays it; pads held on the hardware light up. Under it: Copy, Paste, Clear and Map to keys.
- **A bar over everything:** the picked pad's colour, name and a line about its sound, and a **Browse | Pad** switch on the right.
- **Browse** (the default): the browser in a narrow form beside the pads (places, the list, the selected item with its waveform and buttons), a picker for sounds that stays open: Samples, Recordings, Recent and Favourites, with Load (onto the pad picked), Load to pad (onto the next pad hit), Audition and Favourite only. The wheel moves through the list, SHIFT + wheel through the places, the push opens or loads, - goes up a folder, + into one, and PLAY hears the sound picked (it doesn't start the transport while Browse shows). Q-Link 1 scrolls, Q-Link 3 is the preview level, Q-Link 4 the places. Without a library (no `--data`), a card says so and the Pad view still works.
- **Pad:** the picked pad's settings (sound, play mode, voices, mute group, level, pan, tune) and the kit's sounds with Put on, as the Sample tab's Pads sub-page had them. Q-Links 1 to 4 turn level, pan, tune and voices.
- **The Sample tab** loses its Pads sub-tab and gains two small buttons beside its bar on every page (330 px, mockup sample-send-to-pad), **Send to pad** (the pad picked, sound and edits, onto the next pad hit; it reads "Hit a pad" while it waits, a second press calls it off) and **Chop to pads** (opens the Chop page; on the Chop page it is that page's own Chop to pads). Both are faint while the pad picked has no sound.
- **PROG EDIT** opens the Pads tab on its Pad view (the Synth tab while the synth has the pads).
- While the synth has the pads, the bar's heading carries the Chromatic | In key switch, as the Synth tab does.

## 5. Prefs (SHIFT + MENU)

- **Main rows, in order:**
  1. Pad velocity curve
  2. Latch keeps pads
  3. Audio buffer
  4. Find the buttons again
  5. Show developer tools
  6. Sound cores (Shared, Core 3, Cores 3 and 2, Cores 3, 2 and 1 (a test Jay asked for on 12 Nov): where the sound runs from the next start; asked for by the profiling thread, agreed by the app thread on 12 Nov; Jay's go 12 Nov 07:10Z, merged in PR #200, train 14)
  7. Map controls
  8. Line input
  9. System info
  10. At start (New project, the default, or Last project: what opens when MPFree starts, once the sound is going; Jay's pick on the project thread's settings card, 13 Nov 14:09Z)
  11. Touch screen (opens its own rows)
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
  - PROG EDIT: the sound the pads play: the Synth tab while the synth has them, otherwise the Pads tab on Pad.
  - SHIFT + 16 LEVEL: the synth pads' Chromatic | In key switch.
  - Q-LINK: the Q-Link column's next bank on Play, the chain editor and the macros panel (section 4b).
- **TRACK MUTE** (control map decision 5): the pads become mute keys over whatever page shows, and the page stays. Pads 1-8 mute and unmute tracks 1-8 at once (their clips keep their place); a pad is lit green while its track plays a clip, dim green with nothing launched, red while muted, and pads 9-16 are dark. **SHIFT + TRACK MUTE** does the same for the playing kit's pads (the speakers on Play): lit in the kit's colour while a pad plays, red while muted, dark without a sound. The same press again leaves, and the other one switches. The hint line says what the pads are, and the button is lit while they are mute keys. A pad held when TRACK MUTE is pressed still lets go as a note.
- **Found with Map controls first:** ERASE, Q-LINK and the mode buttons have no measured note yet, so each works once Map controls has found it (TRACK MUTE, NEXT SEQ, STEP SEQ, SAMPLER, SAMPLE EDIT, PROG EDIT and the data dial's push have rows since round 9).
- **The data dial's push** (Jay, 12 Nov 08:11Z): in Prefs and the options menus, it opens the picked row (its value box outlined), the wheel changes the value, and a second push sets it and gives the wheel back to the rows. On a row that opens something (System info, Touch screen, Back, Find the buttons again, Map controls, Calibrate touch, Touch lab) it opens it, as + does.
- **The data wheel on a page with rows** (Jay, 13 Nov 06:27Z): on Sample (every sub-page with a settings list), the Pads tab's Pad view, Synth and Arrange, the wheel moves the highlight through the rows (on Synth, the tiles) and - / + step the highlighted one, as in the menus. A push opens the highlighted row for the wheel (its value box outlined, the Synth tile raised); the wheel then changes it, and a second push gives the wheel back to the rows. Changing page closes it. SHIFT keeps each page's second wheel job: SHIFT + wheel is the next pad with a sound on Sample and Pads, the Synth page on Synth and scrolling on Arrange; SHIFT + - / + zooms the wave on Wave, Warp and Chop. The Play page (the kit), the Looper tab and Pads' Browse view keep the wheel they had.
- **The transport lights** (Jay, 13 Nov 06:37Z, card "Capture Akai's lights first"): PLAY, REC and OVERDUB light only once a read-only capture under Akai's 3.6.0 shows the message its software sends for them; until then the app sends nothing for them and says so once in the Console. **The capture ran in live test 3 (13 Nov 09:33Z):** PLAY is lit while playing, REC and OVERDUB blink while armed (with PLAY) and while recording, all with values 3 and 0 (controller protocol page, "Akai's transport lights"). Wiring them is the next build. The MPC One's STOP button has **no LED** (Jay, 13 Nov), so the STOP light the app sends shows nothing on this model.
- **BROWSE** (controls decision 13; Jay after live test 3): opens the browser over the page and goes back to it (section 3, "Not tabs"). While it shows, the wheel, its push, - / +, PLAY and Q-Links 1-4 work it as on the Pads tab's Browse view, SHIFT + wheel moves through the places, and ERASE deletes the item picked (into the Trash, asking first). **SHIFT + BROWSE** saves the project on any page. BROWSE has no measured note yet, so it works once Map controls has found it.
- **Still to come from the control map:** hold MAIN + pad to pick a track; SHIFT + NEXT SEQ (Perform FX); 16 LEVEL, COPY and TC.

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

## Asked for by Jay after live test 3 (13 Nov 11:46Z), not built yet

Jay asked for these in the app thread (message of 13 Nov 11:46Z). The ones that change the shared layout are his decision already; each needs this file updated in the same pull request that builds it. Details, causes and file references: mpfree-claude `handoffs/2026-11-13-app-thread.md`.

- **A Mix/FX tab** in the top bar for the mixer and effects (today the mixer opens with TRACK MIX or the mixer icon), and a **dramatic overhaul of adding effects** to pads, tracks and audio channels. On the mixer, **+ Insert** becomes **+ FX** (or a button).
- **BROWSE opens the top-level browser** (projects, samples and the rest), as the browser plan's overlay under the top bar; projects load from it and **save**. A **New project** button starts from a fresh, empty project. **Built** by the project thread ("Add project New, Save and Load", 13 Nov): see section 3 ("Not tabs") and section 6 (BROWSE).
- **Touch targets big enough to hit, or reachable with the wheel and – / +**, everywhere: the sequencer tracks' mute speakers, the mixer's insert rows (+ Insert is the example Jay gave) and the FX page's buttons first.
- **Higher contrast for buttons as a general rule** (light grey text on a mid-grey button is out): the Looper tab's top and bottom buttons first.
- **RAM %** beside CPU % in the top bar. The "Kit N of 10" chip names the kit instead.
- **A tap on the tempo** on the Play page (and its views) sets the tempo.
- **More steps and more tracks** in the sequencer, with a clear way to add them (32, 64, 128 steps; more than 8 tracks).
- **ERASE + a clip deletes the clip.**
- **Several throws at once** with several fingers on the Play page.
- **Typing a number** for fields with a wide range (Warp tempo first), instead of only the wheel.
- **A clear way to granular** from the Sample tab's Grain page.
- **The metronome** off by default, with a toggle, and much quieter (Jay, live test 3 phase 6).

## Where things stand (2026-11-11 08:10Z)

| Where | Tab bar |
|---|---|
| main (= the round 5 test app) | Play, Debug, Console, Prefs |
| Round 6, #68 (merged) | Play, Clips, Synth, Console, Prefs (+ Debug in debug mode) |
| Looper patch (round 7) | Play, Clips, Synth, Looper, (Debug), Console, Prefs |
| #75 Sample tab (merged) | Play, Clips, Synth, Sample, (Debug), Console, Prefs |
| Round 7 | Play, Clips, Synth, Looper, Sample, (Dev); Prefs on SHIFT + MENU |
| Round 8 (merged) | Play, Arrange, Synth, Looper, Sample, (Dev); Prefs on SHIFT + MENU |
| Round 9 (merged) | No tab change (the Looper tab becomes mockup J's editor) |
| Round 10 | No tab change: the chain editor and the macros panel take the mixer's place or open over the page (section 4b); TRACK MUTE works over any page |
| #72 themes, arp b1aa92e | No tab change (#72 restyles the drawing only) |
| Preview pictures (not the app) | synth-ui "Steps, Clips, Looper, Arrange, Synth, Mix"; time-ui "Play, Clips, Debug, Console, Prefs"; sampler-ui "Play, Sample, Debug, Prefs"; arp-ui "Play, Debug, Console, Prefs"; wave preview "Record, Sound, Perform, Browse" |
