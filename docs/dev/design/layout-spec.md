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
- **Touch targets** (Jay after live test 3, 13 Nov): anything tapped answers over its whole visible button, and packed rows (pattern lanes, list rows) answer over the row's full pitch with no dead gap between them. A mute is at least the width of the name it belongs to, never a bare 32 px icon. Where a control can't be that big, the wheel and – / + reach it.
- **Typing a number** (Jay after live test 3: "Warp tempo (and other numeric fields with large range) should have ability to enter a number, instead of only being able to use dial"): a field that can be typed has a dotted line under its value, and a tap on the value opens the number pad over the page, under the top bar (keys 0 to 9, the point and the minus where the field takes them, Delete, Cancel and a two-row Enter, every key at least 64 px). It shows the range and the value now; Enter out of range keeps the pad open and says the range in red; Cancel or a change of page leaves the field as it was. The wheel and – / + step the number, and the wheel's push is Enter. Fields so far: the Sample tab's Warp tempo (Wave and Warp pages) and the tempo (hold Play's BPM). A typed value is one undo step.
- **Button labels** (Jay after live test 3: "light gray on a medium gray button is no bueno"): a label on a `raised` fill is `text`, or `text_dim` while the button can't be used (`Theme::button_ink`); `text_faint` is not for labels. `Theme::check` holds both inks at 4.5:1 on `raised` in every theme, and the built-in themes are tested against it.

## 2. The top bar

- **Tabs** start at the left edge. There is no MPFree wordmark: that belongs to the startup and loading screens only (Jay, 08:02Z).
- **Status chips** sit on the right: CPU %, RAM % (green under 70 %, amber under 85 %, red above), session, the kit's name and bank ("Classic House · Bank A"; or "Poly · C3 to D#4" when the synth is on the pads), level, tempo, and the arp chip.
  - Chips never push tabs off the bar, and tabs never push chips off.
  - **A tap on the kit chip** opens the list of kits over the page: every kit, then the synth (as the wheel steps them), beside a card with **Back**. A tap on a kit loads it and closes the list; Back, the kit chip again or any tab closes it and keeps the kit. The hint line says so while it is open. **A tap on the Full level or Half level chip** turns that level off, as FULL LEVEL again would. (Jay's card on the UI audit thread, 13 Nov 14:57Z.)
  - The CPU, RAM, session and time-left chips only show: a tap says on the hint line what the chip shows.
- **Width:** up to 6 tabs are drawn at full width (112 px). More tabs share that room: the 7 music tabs are 96 px each, as drawn in the Mix tab mockups.
- **Active tab:** the page's own tab is highlighted; the Mix tab stays lit while an effect chain is open. Prefs, menus, the macros panel and the guide highlight none.
- **No mixer icon:** the Mix tab took the small sliders icon's job (Jay's card, 14 Nov 07:15Z; before that the icon opened the mixer, Jay 09:17Z).
- **The I/O icon:** a small in-and-out icon (one arrow in, one arrow out) at the bar's right edge, after the chips. A tap opens the mixer on its I/O view: the project's routing (audio ins and outs, MIDI ports, clock and sync, CV), saved with the project. The same icon again, or a tab, closes it, as with the mixer. A tap on the tempo chip opens the same view at Sync. (Jay's picks on the MIDI thread's cards, 12 Nov 08:38Z and 08:39Z; MIDI and sync are project settings, not Prefs, 08:26Z. The FX thread owns the mixer and its frame; the MIDI thread builds the MIDI, sync and CV sections.)

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
| Mix | The mixer (its strips as Faders or as Effects, the I/O view) and every strip's effect chain (section 3a; Jay's card, 14 Nov 07:15Z) | App (the mixer panel is the FX thread's crate) |
| Dev | Only when Prefs > "Show developer tools" is On | App |

- **Dev's pages,** on a strip under the top bar: MIDI log | Performance | CPU by engine | Processes | Map controls | Touch | Console. CPU by engine and Processes were asked for by Jay at 08:19Z (the profiling thread builds them). Touch is the graphical touch calibration and testing tool with its tuning (Jay, 22:23Z; the touchscreen thread builds it, with a "Touch targets" page): the wheel and + / - step the tuning, SHIFT + wheel still changes Dev page, and MENU there offers Calibrate touch and Touch lab.
- **Not tabs:**
  - **Prefs.** SHIFT + MENU opens it.
  - **Each page's options.** MENU opens them.
  - **The Arp panel.** NOTE REPEAT + MENU opens it; it belongs to Play.
  - **The browser.** BROWSE opens it over the current page, under the top bar, as the mixer does; BROWSE again or a tab closes it (controls decision 13; Jay after live test 3, 13 Nov 11:46Z; built by the project thread). Its Projects place has New project, Save as, Save and Export. **Export** (Jay's pick on the missing-features thread's card, 14 Nov 01:39Z: "Yes, in BROWSE") opens the Export panel over the browser: Mixdown, Stems and MIDI file to tick, Stems by (Track, the default, or Channel, with how many times the range plays; Jay's pick "Yes, a Stems by row" on the missing-features thread's card, 14 Nov 15:20Z), the range (Arrangement, or a number of bars with - / +), the File row (24-bit, 16-bit or 32-bit float, and 44.1 or 48 kHz; it starts as Prefs > Export file and Export rate have it and changes the format for that export only, Jay's pick "Both" on the missing-features thread's card, 14 Nov 07:51Z), how it will play and the folder, then Start; while it runs, the time done and Cancel; when it is over, what it made and Show in BROWSE. While the panel shows, the wheel sets the bars, its push is Start (Close once it is over) and - closes it. BROWSE still closes the browser, and an export keeps running behind it. The Pads tab's Browse view stays the narrow picker of section 4e.
  - **The pattern workspace.**
  - **The button guide.**
  - **System info, Touch lab and Calibrate touch.** These open from Prefs.
- **Adding, removing, renaming or reordering a tab** is a layout change, so it needs Jay's approval. The "Steps, Clips, Looper, Arrange, Synth, Mix" bar in the synth previews is a design mock, not this list.
- **The order is Play, Pads, Arrange, Synth, Looper, Sample, Mix, then Dev.** Jay chose it at 08:15Z, renamed Clips to Arrange at 12:55Z, put Pads after Play (12 Nov 15:46Z) and Mix last (14 Nov 07:15Z).

## 3a. The Mix tab

Approved by Jay on 14 Nov at 07:15Z ("Approve as drawn" on the Mix tab mockups: https://claude.ai/artifact/9w1vtAhfGcKmWj81JZdCUG). It answers his ask 7 after live test 3: a place for the mixer and effects, and a clear way to add an effect to a pad, a track or an audio channel.

- **The tab** holds the mixer and the effect chains. **TRACK MIX** opens it on Track Mix and **SHIFT + TRACK MIX** on Pad Mix, both on Faders; pressed on the tab (the mixer or a chain), either goes back to the page you came from. A tap on the tab opens the mixer on the view and the look it showed last.
- **Two looks of the same strips**, switched by a **Faders | Effects** toggle (170 px) left of the page arrows, where the mixer's CPU chip was (the cost, while profiling, moves to the master's name row, and on Effects to the title line). The strip views (Tracks, Pads, Inputs, Submixes) and the arrows work in both; the I/O view is its own.
- **Faders** is the mixer as section 4b's strips have it.
- **Effects** puts one panel over the strips and the master: a title line ("TRACKS 1-8 · EACH CHAIN IN ORDER, LEFT TO RIGHT"), the slot numbers, then a 52 px row a strip (a page is the rows that fit, 8 on the MPC), and the master's row pinned at the bottom under a rule. A row is the strip's name over its colour stripe, with which strip it is and how many of its 8 slots are used, then its 8 slots: each effect (its name, a dot: green on, blue keyed with "SC from ...", grey bypassed with "Off"), **+ FX** in the first free slot, the rest outlined. A strip has 8 insert slots.
- **On Effects:** a tap on + FX opens the effect picker for that strip; the effect lands in that slot and opens in the chain editor. A tap on an effect picks it (outlined; the title line names it) and the Q-Link column turns its first four knobs, as on the chain editor; a tap on the picked effect opens it in the chain editor. A drag along the row moves the effect (the slot it would land in is outlined). A tap on a strip's name selects the strip and gives the Q-Links back to it, as does Faders.
- **The chain editor's head:** a 44 px **‹** at the top left goes back to where the editor opened (the Mix tab as it was, or the Pads tab), then the **strip button**: the strip's colour, its name and which strip it is ("Pad 3"). A tap on it opens the **strip list** in the editor's place: group buttons (Tracks, Pads, Inputs, Returns, Submixes, Master), < > to page a long group, and the group's strips; a tap on one opens its chain (on its first effect, or with the picker up when it has none), and Back keeps the strip.
- **The effect picker's head** says where the effect goes: "ADD TO KEYS · IT GOES IN SLOT 4 OF 8" (the strip and the slot at the end of its chain), "PICK A PRESET OR DEFAULTS · IT GOES IN SLOT 4 OF KEYS" on the presets, and "KEYS IS FULL" when there is no room; holding a tile for presets is on the hint line.
- **The Pads tab's Effects row:** the Pad view's list ends with **Effects**, a button (no arrows) reading "+ FX" or the effects on the pad's mixer channel ("Compressor, EQ", or "Compressor +2" when they don't fit). A tap, or the wheel's push on the row, opens the pad's chain in the Mix tab (the picker up on an empty chain), and ‹ comes back to the Pads tab. Pads past the 16 with a channel show "—".

## 4. The Play page

Built in round 8 (PR #106) to Jay's approved mockups F to I (10:05Z), with real data and working controls (13:56Z on; side-by-side sheets in /mnt/project-files/hello-mpc/round8-play/compare/). The old big pads are gone, and since 12 Nov the mini pads too: Jay saw no benefit in them on the home screen (08:11Z) and picked "the pattern fills the room" (15:45Z, mockups in /mnt/project-files/hello-mpc/pads-mockups/). The pads live on the Pads tab (section 4e).

- **The frame, in every view:**
  - the transport bar: the state, the position, the tempo, the bar and step dots, and the view switch (All, Beats, Loop) on its right. One tap changes the view, and the screen keeps it while other tabs show;
  - the view under it;
  - at the right edge, from under the top bar to the hint line: the master bar (66 px: meter with peak hold, level handle, level, LIM when the limiter works) and the Q-Link column (168 px: four cards, knob 1 at the top, each with its scope, name, ring and value). A drag on a card turns that knob, and a finger on a knob lights its card.
- **The views:**
  - All (home): the loop strip (150 px), the pattern across the whole width, then the throws beside the XY pad (280 px).
  - Beats: the pattern (six lanes; up to eight with the strip off), then the Your timing strip (72 px: the average offset, the spread, the velocity and the spread of offsets over the last two bars against the 1/16 grid), then all eight throws across the width. A tap on a lit step picks it for the Q-Links (velocity, length, nudge, chance). Play's options have **Timing strip** (On by default, saved); Off gives the pattern the strip's room.
  - Loop: the loop big with its buttons (Record, Overdub, Play, Stop, Undo, Redo, Mute, Clear; Record has a red dot, Overdub a red ring, Play a triangle and Stop a square, and Play was added, Jay 12 Nov 08:11Z) and the input in its heading, then the throws beside the XY pad (300 px).
  - The Drum view is gone (Jay, 13 Nov 13:09Z, "Fold timing into Beats, drop Drum"): its timing is Beats' strip. Its Decay and FX mix Q-Links went with it (Decay is on the Pads tab; the throws' mix stays at full).
- **The pattern** is the armed track's clip from the sequencer. Each lane's name is its mute button: a speaker in a box (red with a × while muted) at the name's right, and a tap anywhere on the name, over the lane's full height and half the gaps, mutes that pad. A tap on a step toggles it (or starts a pattern when there is none). **Its head** (Jay after live test 3: 32, 64 and 128 steps) has ‹ and › round the page number to turn its pages of 16, and – and + round its length: + doubles the steps, up to 128, the new ones repeating the old; – halves them, down to 16, and UNDO brings back what it took off. Each length change is one take. A button that can't act is dimmed, and a tap on it says why on the hint line. Each answers over the head's full height.
- **The loop strip** names each layer by what was played into it (Drums, Bass, Chop; Line in from the line). Each name and its speaker are one outlined button that mutes that layer, answering from the card's top edge to below the heading.
- **Throws and the XY pad** play Perform FX (`mpfree_fx::Perform`) on the master while held: each throw holds a pad of its default bank (Delay throw the 1/8 dotted echo throw, Reverb wash the wash throw, Filter dip the 1-bar low-pass sweep), entering on the next 1/16 while the transport plays; the XY pad plays its DJ filter across (low-pass left, off in the middle, high-pass right) and its echo up, and glides home when the finger lifts; Drop drums mutes the kick, snare and hat pads while held.
- **No pads panel** (Jay, 12 Nov 15:45Z). The Chromatic | In key switch for the synth pads (Jay, 12:22Z) is in the heading over the Synth tab's presets, and SHIFT + 16 LEVEL works everywhere.
- **Q-Links on Play,** by view: All and Beats: swing, tune and the drums' filter and delay send (a picked step in Beats: its velocity, length, nudge and chance); Loop: loop level, loop delay send, line level, loop filter. The master bar sets the level. While NOTE REPEAT is held or the Arp panel is open, the cards show the arp's four (rate, gate, swing, octaves).
- **The header** keeps to the CPU, RAM and kit chips; the session chip shows only when something is wrong, the arp chip only while the arp, a repeat or a latch is on.
- **The hint line** starts where the panels do (left-aligned, as the mockups).
- **NOTE REPEAT's strip** takes the top of the view's first panel, so nothing else moves.
- **The Arp panel** takes the view under the transport bar.
- **Macro banks:** Q-LINK steps the Q-Link column to Macros 1-4, then Macros 5-8, then back to the view's own knobs; Play keeps its bank while other pages show. A finger held on a macro card opens the macros panel (section 4b).
- **Still to come:** half length and double on the loop strip (the looper's edit tools, which the Looper tab has since round 10).

## 4a. The Looper tab

Built in round 9 to mockup J (Jay's direction, 10:16Z), with real data; side-by-side sheet in /mnt/project-files/hello-mpc/round9-looper/compare/. The Play page's strip performs the loop; this tab edits and sets it up.

- **The frame:** the loops (220 px) at the left; the editor over Recording and input and Use the loop (330 px) in the middle; the Q-Link column (168 px) at the right; the hint line under them.
- **The loops:** Loop 1 to 4, and on as far as one is used or picked (eight in all), each with its length and layers and its state (PLAYING, MUTED, RECORDING); the picked one outlined. A tap picks a loop; + New loop picks the first empty one. Every loop plays at once; the controls act on the picked one. Each card has a speaker in a box at its bottom right: a tap there (the card's bottom right, 64 px wide) mutes or unmutes that loop without picking it, so a loop can be kept out of PLAY (Jay, 13 Nov 06:27Z). Under + New loop, Clear loop (outlined in red) clears the picked loop, first pass included; the × on a loop's last layer does the same. The loop memory gauge sits at the bottom ("1:42 of 5 min"); its size is Prefs > Loop memory, 1 to 20 minutes from the next start.
- **The editor:** the tools row (half length, double, reverse, nudge, seam fade, merge layers, undo history; they work while the loop plays and isn't recording, and merge needs two layers; seam fade steps through a cut, 10 ms, 50 ms and the longest, and shows the fade now; merge shows its progress), the loop's wave with its bar lines and the playhead (while the loop can be edited, a handle at each end of the wave drags in by 1/16 notes, the part cut off dimmed and "keep 12 of 16 beats" above it, and trims the loop on release), and a row per layer: its speaker in a box (mute; it answers over the row's left column under the name), S (solo), level bar (drag: -24 to +6 dB), mini wave and × (delete). A layer Fade old has turned down says how far ("faded to 77 %"). A tap on a row puts that layer on the Q-Links. Three rows show; more scroll to keep the picked one in view. **Undo history** (lit while open) lists the loop's history in the layers' place: Start (the first pass), then each overdub, delete, edit and merge, oldest first, with now outlined and the steps Redo would bring back faint below it. A tap on a step goes back or forward to just after it; a drag scrolls the list; picking another loop closes it.
- **Recording and input,** two columns: Record from, First pass, Start, After recording, At another tempo; Line input, Input peak, Monitor, Latency. A tap on a row's right half steps it up, the left half down; the source and on-off rows flip, and After recording flips between play and overdub (the picked loop's own). The Play strip uses the same settings.
- **Use the loop:** To a pad, To a clip, Save loop, Resample. Resample picks the first empty loop and records the loops as heard into it from the next bar line, as REC does; REC ends the pass. It works while a loop plays and one is empty.
- **Q-Links:** the picked layer's level and pan, then the loop's Fade old (50 to 100 % kept a pass, a percent a detent) and its seam fade (a millisecond a detent), as mockup J. The loop's level and the line input's are on Play's Loop view.
- **Hardware:** REC records or overdubs, OVERDUB overdubs, UNDO undoes (SHIFT: redo); there are no on-screen record buttons here (the Play strip has them).
- **Still to come (drawn faint until then):** Start, and Use the loop's To a pad, To a clip and Save loop (the looper's bounce).


## 4b. The FX chain editor and macros

Approved by Jay at 22:34Z (the mod thread's card, "Approve as proposed"). The mod thread builds them (crate `mpfree-chain-ui`, PR #102 merged; pictures in /mnt/project-files/mod/chain-ui/).

- **The chain editor:** a track's effects in order (on/off, up, down, remove, "+ Add effect", up to 8), the selected effect's knobs, and a Modulate panel for the knob last touched (put it on macro 1 to 8, learn it to a Q-Link, add an LFO or a follower).
- **Dragging an effect** (Jay, 14 Nov 12:03Z: drag the FX up and down the chain besides Up and Down): a finger on an effect's row that moves more than a fingertip (12 px) lifts it, drawn under the finger with an accent rim while its place stays outlined; a 4 px accent line shows where it lands, and it moves when the finger lifts. Lifted where it started, or cancelled, nothing moves; a tap still picks it. Up and Down stay. The wheel moves it too (section 6).
- **Where it opens:** from a mixer strip's insert row, where it takes the mixer's place with a Back to the mixer; and in the Sample tab's FX sub-tab, under Sample's pad bar, showing the pad's chain first with a Pad / Track switch (sampler thread). The insert row works as the mixer already sends it (FX thread).
- **Macros:** the Q-Link column can show "Macros 1-4" or "Macros 5-8" as a bank, on Play and while the chain editor is open. A long press on a macro card opens the full Macros and Morph panel (8 macros, up to 8 targets each, and a morph between two saved scenes A and B) over the page and under the top bar, as the mixer does. Play keeps its views and no tab is added.
- **Q-Links:** the column is the width the page gives it (168 px), not a wider one. While the chain editor is open they turn the first four knobs of the selected effect's Main page, whichever page is shown, for an effect with a Main page (Jay, 14 Nov: layout B), else the first four knobs of the page shown, with Macros 1-4 and 5-8 as banks. Their cards use the mixer's card shape. A Q-Link the user learns to a knob belongs to the page it was learned on; Macros 1-8 work on every page.
- **The Q-Link hook:** the app routes Q-Link turns per page in one place (main.rs, the `QLinkTurn` arm). A macro bank, when shown, takes the turns there first.
- **Built (round 10):** a strip's insert row opens the editor on that effect, and its "+" opens it with the effect picker up; it takes the mixer's place with the Mix tab lit, and Back opens the mixer again on the view it showed (Track Mix, Pad Mix and so on). Q-LINK steps the column through the effect's knobs, Macros 1-4 and Macros 5-8; a learn takes the next Q-Link turned. The wheel is Q-Link 1 until a tap puts the outline on a control (section 6). A finger held for half a second on a macro card (Play or the editor) opens the macros panel on that card's bank, and its Back goes back to the page it opened over. Still to come: Sample > FX (the sampler's sub-tab), the effect's own page from a tap on its name, and keeping the macro set-up and learned Q-Links in the store.
- **Mode row** (Jay, 14 Nov: effects with modes): under the effect's name, an effect in a family (`mpfree_fx::families`) shows a chip per mode, or < and > around the mode's name and its place ("Tube   3 of 10") when the chips don't fit. A tap switches the mode in place, keeping the slot (target `chain/mode`).
- **Pages** (Jay, 14 Nov: layout B): an effect with more than eight controls opens on its Main page, the eight a player reaches for (`mpfree_fx::main_page`), then its groups. One with eight or fewer, or a plugin, shows its groups, a group over eight split into pages of eight ("EQ 2"). The tabs have a row of their own under the Mode row; when they don't fit, < and > at its ends, the same arrows as the Mode row, step the page, and the row keeps the page shown in view. An effect with stages it switches (the Channel strip's gate, compressor and EQ) shows them on its Main page under the tabs, each with its on/off pill and a way to its page.
- **The effect page frame** (Jay, 14 Nov 13:41Z, "Build it as drawn" on the FX thread's effect page canvas, https://claude.ai/artifact/BbZs7t2hSJ2ZacDrMq5hqu, to the same rules as the Synth tab, section 4g; the FX thread builds it after the Mix tab, and until it lands the editor is as the lines above): the Synth tab's frame. At the left (220 px), the strip's effects: ‹ and the strip button, the inserts with their on pills, Add effect, then Up, Down and Remove. In the middle (844 px), the selected effect's page. At the right, the Q-Link column (168 px), turning the four the effect's plan picks on every page. The header holds the effect's name (its presets open from it), then its Mode chooser (chips when the modes fit; else a chip that opens the list of modes, with ‹ › beside it, as for Drive's ten), or page tabs, only for an effect that needs more than one page (the Channel strip, Perform FX). The page is labelled sections from the effect's plan, each drawing inside its section, as on the Synth tab; the Channel strip's stages are sections with their switches. The Modulate panel becomes one 54 px row under the page (56 on the canvas: the header 52, the page 540 and the row 54, with 10 between each, fill the 666 px), for the control last touched: macros 1 to 8, Learn, + LFO, + Follower, and a way to its matrix rows (shown over the page).

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
- **Pad:** the picked pad's settings (sound, play mode, voices, mute group, level, pan, tune, and its Effects: section 3a) and the kit's sounds with Put on, as the Sample tab's Pads sub-page had them. Q-Links 1 to 4 turn level, pan, tune and voices.
- **The Sample tab** loses its Pads sub-tab and gains two small buttons beside its bar on every page (330 px, mockup sample-send-to-pad), **Send to pad** (the pad picked, sound and edits, onto the next pad hit; it reads "Hit a pad" while it waits, a second press calls it off) and **Chop to pads** (opens the Chop page; on the Chop page it is that page's own Chop to pads). Both are faint while the pad picked has no sound.
- **PROG EDIT** opens the Pads tab on its Pad view (the Synth tab while the synth has the pads).
- While the synth has the pads, the bar's heading carries the Chromatic | In key switch, as the Synth tab does.

## 4f. Up to 128 tracks

**Jay picked "Up to 128, like Akai's MPC"** on 14 Nov 2026 (01:27Z card, over 16 or 32), asked for mockups first (02:25Z), and **approved the screens at 04:24Z** as drawn in https://claude.ai/artifact/NKJM7t9kNQm7jZc2NdHcvg (PNGs in /mnt/project-files/tracks-128/; screens `clips-tracks`, `clips-tracks-end` and `arrange-song-tracks` in `mpfree --preview`). New projects still start with 8 tracks. Removing a track comes later.

- **The Clips grid** stays 8 tracks across.
  - **+ Track** is a button on the title line, left of Edit. It adds "Track N", armed, with its page in view; UNDO takes it away. It is dim at 128. (The proposal had a column after the last track, but a full page of 8 has no free column.)
  - **The pager:** with more than 8 tracks, "‹ 9–16 of 24 ›" takes the colour legend's place. The arrows, or a swipe of 48 px or more along the track names, show the 8 tracks before or after.
  - **A track's name arms it on the lift**, so a swipe along the names pages without arming a track.
  - **The grid follows the armed track:** arming a track off the page (from the Armed row, for one) brings its page.
  - With 8 tracks or fewer there is no pager, and "+ Track" leaves the legend room for "red: recording" only.
- **The Song view:** "+ Track" is a lane after the last MIDI lane, above the audio lanes and "+ Audio" (section 4c). The lanes scroll as before, by a drag on the names.
- **TRACK MUTE:** all 16 pads are tracks, and the PAD BANK buttons pick which 16, as on Akai's MPC: A is tracks 1-16, B 17-32, up to H, 113-128, with SHIFT or a second press for E-H as when picking kits (section 6). The kit playing stays, and the bank is kept for the next TRACK MUTE.

## 4g. The Synth tab

**Picked by Jay on 14 Nov 2026** (13:25Z, the instruments thread's card, "Build it as drawn"; his ask at 12:40Z: the pages looked like "a wall of knobs", and for consistency the list on the left and the Q-Links on the right). Mockups: the "Synth page layouts" canvas (https://claude.ai/artifact/MSNu169wNrkV8Ag9mSRnD4); rules: the doc "Instrument and effect page design rules" (https://claude.ai/artifact/ABwAw3rxAj5Rgi8P1MdL4Q). Built by the instruments thread (`mpfree-synth-ui`, `view::LAYOUT`).

- **The frame,** as the Looper tab's: at the left (220 px), the instrument card (72 px: swatch, engine, the track and Native or Plugin), then the pads heading with the Chromatic | In key switch while the synth has the pads, then the presets; in the middle, the title bar (56 px: the preset and the pages as tabs) over the page; at the right, the shared Q-Link column (168 px); the hint line under them. The bottom Q-Link strip and the strip of pictures are gone; the page gets their room (844 x 598 px).
- **The page** is labelled sections, each with one job, coloured by it (sources cyan, filter and tone coral, envelopes and dynamics lime, movement pink, voice and output grey) and laid out per page. Pictures sit inside the section they draw. Knobs are sized by importance; choices are buttons or a chip with ‹ ›, whole numbers steppers, toggles pills, envelopes and mixes faders. "Open ›" by a section's name goes to its page. The inside of the page is the Synth tab's own (section 7).
- **The Q-Link column** names what Q-Links 1 to 4 turn on the page, as their sections name them, with the section as its scope; the controls carry Q1 to Q4 badges in the knobs' colours. A drag on a card turns it.
- **Hardware** as before: the wheel picks and turns the controls (section 6), SHIFT + wheel steps the pages, Q-Links 1 to 4 turn the badged controls.

## 4h. Drag and drop

**Jay asked** on 14 Nov 2026 at 12:03Z for dragging effects up and down the chain besides Up and Down, and for drag and drop on the Clips grid, the Song view and the Looper's layers. He picked mockups first (12:12Z) and **approved them as drawn at 12:41Z** (https://claude.ai/artifact/Qj2hoe5JPWGFxgmPShJn4Z).

- **On every screen:** a finger that moves more than a fingertip (12 px) picks a thing up; there is no hold. On a thing that can be dragged, a tap acts on the lift. Its own place stays outlined, and where it would land is outlined (or a line between rows). Lifted where it started, or cancelled, nothing changes. SHIFT held as the finger lifts drops a copy. UNDO takes back any drop. Dragged to an edge, the view pages or scrolls. The wheel: a push grabs, a turn moves, a push drops.
- **The chain editor:** section 4b. Built.
- **The Clips grid:** a clip dropped in an empty slot moves there; onto another clip the two change places (a copy goes only into an empty slot); in another track's column it plays that track's sound, and the song's blocks that use it go with it. A row's number dragged moves the whole row to the line, the rows below shifting down, so nothing is written over; with SHIFT a copy goes in. At the grid's edge the tracks page and the rows scroll. Built.
- **The Song view:** a block slides along its lane snapped to the bar, or up and down to another track's lane; the wheel moves the picked block a beat a detent; a picked block's end grip makes it longer (the clip repeats) or shorter. A section's name on the ruler drags the whole section, on every track, and the sections after shift so nothing is written over; with SHIFT a linked copy. Audio regions drag as they do already. Built, with these details for Jay to confirm: a block put on another track's lane takes its own copy of the clip (a linked block would name that track's slot); a block across a section's edge is cut there when a section moves; a section can't move while the song has morph moves (they would stay behind); the Arrange page has no outline, so the wheel's push grabs the picked block, or a section picked by a tap on its name, in place of opening the record row; Delete and Make unique are undoable edits too; the hint line follows the pick.
- **The Looper:** a layer dragged onto another loop moves there (with SHIFT a copy); onto + New loop it becomes a loop of its own; onto To a pad or To a clip just that layer goes. A loop of another length refuses it and says why. Built, with these details for Jay to confirm: To a pad and To a clip are left out for now, since the loop's own bounce to a pad or a clip isn't in the app yet (their rows are faint); the loops that can take the layer are lit and empty loops are not, as drawn, but + New loop takes it into the first empty one; the loop that took the layer is picked afterwards, so the editor shows where it went; a move from a loop's only layer leaves it there, so it acts as a copy (the looper keeps a loop's last layer); a busy or recording loop, or a full one, says why as a loop of another length does; the layer fades in, in time with the song, as soon as it is copied (a 64th of the loop's length later: about 60 ms for 2 bars at 120 BPM), and UNDO in either loop takes a move back in both; the hint line adds "drag a layer to a loop (SHIFT: copy)", and its first item is cut to "Tap: pick" so STOP's still fits.
- **Left out:** moving tracks (every per-track table would shift) and reordering layers within a loop.

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
  11. Export (Fast, quiet, the default, or Listen: how an export plays; a song that needs outside gear listens anyway; Jay, 14 Nov 01:32Z, on the missing-features thread)
  12. Export file (24-bit, the default, 16-bit with dither, or 32-bit float) and 13. Export rate (44.1 kHz, the default, or 48 kHz): the format each export starts from; the Export panel's File row changes it for one export (Jay's pick "Both", 14 Nov 07:51Z, on the missing-features thread)
  14. Touch screen (opens its own rows)
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
- **The power button** (Jay, 15 Nov: "work like it does on the MPC"; the card approved as drawn 22:24Z, screens power-ask, power-unsaved and power-going): a short press (up within 1.5 s) opens the **power card** over any page, a page of its own like the number pad: "Shut down the MPC?", a line on the project (unsaved changes in warn, or saved, or no data folder), a line on what Restart starts, and Cancel · Restart · Shut down along its foot, 72 px tall, the last in the accent with the outline on it. Over unsaved work, Shut down or Restart asks "Save … first?" with Cancel · Don't save · Save and shut down (or restart); the save is SHIFT + BROWSE's, and the card waits for it, then says "Shutting down…" or "Restarting…" until the app stops. The wheel and – / + move the outline, the push chooses, and another short press is Cancel. Holding the button is the hard power-off, never a press. Engine: [power button](../hardware/power-button.md).
- **STOP:** the transport, then silence.
- **SHIFT + STOP:** as on the MPC: stop, silence everything and go back to bar 1 (Jay, 20:40Z, control map decision 2).
- **The Console:** hold SHIFT, hold ERASE, then press TAP TEMPO (and the same again to go back). Jay wants a chord nothing else uses, since only developers open it (22:12Z); the hardware control map thread picked this one. The press is no tempo tap, and nothing fires on the way into the chord. The Dev tab's Console page opens it too. It works without developer tools, but has no strip then.
- **ERASE held + a tap deletes what the tap lands on**, where UNDO brings it back (Jay after live test 3, 13 Nov 13:27Z): a clip on Arrange (the takes' undo; a clip the song uses is refused, since song edits have no undo yet), a pattern lane's name on Play (that sound's steps, one take), and a layer on the Looper tab (the looper's undo; the last layer stays). An empty slot or a loop card says why nothing happened. While ERASE is held the hint line lists what it deletes; those taps only delete (no mute, launch or new clip), and taps elsewhere work as usual. Song blocks, whole loops, effects, macros and chop slices wait until they have an undo. Live erase (ERASE + a pad while recording) and the FX lock mode with a step held come later.
- **NOTE REPEAT:** repeat. Press it twice, or SHIFT + it, to Latch.
- **NOTE REPEAT + MENU:** the Arp panel.
- **TRACK MIX:** the Mix tab on Track Mix; SHIFT + TRACK MIX on Pad Mix, as the FX thread proposed; on the Mix tab, either goes back to the page before it (section 3a). The guide names the button "Track Mix / Pad Mix"; the pad player hasn't seen it yet, and round 7's mapper lists it.
- **REC, OVERDUB, UNDO:** they act on the page showing (Arrange, Looper, Sample), otherwise on the clips. **On Play, REC and OVERDUB record the loop on the All and Loop views** (as the loop strip's Record and Overdub) and pattern takes on Beats (Jay's pick, 13 Nov 14:37Z); the screen picks, as for UNDO (`Effect::PlayRec`). **UNDO follows the page you're on** (Jay, 13 Nov 13:53Z): the Looper tab and Play's Loop view undo the loop; Play's Beats view, Arrange and the Arp and pattern screens undo the takes; Play's All view undoes whichever of the two changed last; the Sample and Pads tabs undo their edits; the Synth tab undoes knob turns and the mixer strip changes (a run of turns of one control within 1.5 s is one step; `mpfree-app/src/undo.rs`); other pages, for now the chain editor and the macros panel too, say "Nothing to undo on this page". The line in place of the hint says what was undone ("Undid take 3 · Drums / 1", "Undo: layer 3 fades out"). SHIFT + UNDO redoes, the same way. **Held for 0.6 s, UNDO opens that history as a list** (Jay after live test 3; the app thread's 13 Nov 13:30Z proposal, point 5): the press still undoes one step at once, and the list opens over the page under the top bar (the `Undo history` page, `mpfree_ui::history::HistoryPanel`). Its rows are Start, then each step oldest first, then the steps REDO would bring back, faint; the row that is now is filled and tagged "now". A tap on a row goes back (or forward) to just after that step, as many steps as it takes; the wheel and – / + move an outline through the rows and the wheel's push goes to the outlined row. The list stays open and follows its history (UNDO and SHIFT + UNDO still step it); Close or a change of page closes it. Every row and Close is 64 px tall. Steps are named as the page names them: the loop's as the Looper editor's Undo history does, takes as "Edit 3 · Drums / 1", the kit's edits by pad ("Pad A3", "Chop to 8 pads from A5"), knob and fader turns with where they went ("Filter Cutoff to 1.2 kHz", "Drums Volume to -6.0 dB"; the app keeps those and sends them on `FromUi::UndoHistory`). While the list or the number pad is open the app gives the wheel, its push and – / + to it on every page (`FromUi::Overlay`). UNDO's light is bright while the page showing has something to undo and dim when not (blinking while recording, as Akai's); the screen works it out and tells the app (`FromUi::UndoReady`). The idle light is inferred, not confirmed: in an earlier capture Akai's app took UNDO's light from dim to bright during Q-Link edits, which fits lighting it when there is something to undo.
- **PAD BANK A–D:** banks. SHIFT, or a second press, gives E–H. With a drum kit on the pads they are how the kit changes: the wheel and – / + no longer do (Jay, 13 Nov 14:26Z). SHIFT + wheel on Play still steps through every kit while a drum kit plays. While the synth has the pads, PAD BANK picks its pad layouts and SHIFT + wheel its octave, so the kit chip's list is the way back to a kit.
- **SHIFT + REC:** Capture, on every page.
- **SHIFT + PLAY START:** the metronome on or off.
- **PLAY while the transport plays:** FILL while it is held (notes with the FILL and !FILL trig conditions follow it); pressed twice, FILL stays latched until PLAY is pressed again or STOP (Jay, 20:54Z, control map decision 9). Stopped, PLAY plays.
- **The mode buttons** go to the page with the same job (Jay, 20:40Z; the control map doc, https://claude.ai/artifact/CzSwDDjX1Ngc7LkhXP4KqD):
  - MAIN: Play; on Play, the next view (All, Beats, Loop).
  - STEP SEQ: Play's Beats view.
  - NEXT SEQ: Arrange.
  - SAMPLER: Sample. SHIFT + SAMPLER: Looper.
  - SAMPLE EDIT: Sample, on its Wave editor.
  - PROG EDIT: the sound the pads play: the Synth tab while the synth has them, otherwise the Pads tab on Pad.
  - 16 LEVEL (Jay, 13 Nov 13:13Z card "Button plus a 16 Levels panel"; 13:23Z: square cells, in the colour the pads are lit): 16 Levels on for the pad hit last, with the 16 Levels panel on Play; again, off. The panel takes the view under the transport bar, as the Arp panel does: a heading with the pad they play, Pick pad, Close (the panel goes, 16 Levels stays) and Off; 16 square cells as the pads (pad 1 bottom left), each with its value and step bar, in the kit's pad colour with the held pad bright; and the kinds (Velocity, Tune, Cutoff, Decay, Start) in a column beside them, picked by a tap or the wheel. The Q-Links and the master bar stay as on Play. The button is lit while 16 Levels is on, and a "16 Levels · tune" status chip (warn, after Full level) shows it on every page.
  - SHIFT + 16 LEVEL: the synth pads' Chromatic | In key switch.
  - Q-LINK: the Q-Link column's next bank on Play, the chain editor and the macros panel (section 4b).
- **TRACK MUTE** (control map decision 5): the pads become mute keys over whatever page shows, and the page stays. Pads 1-16 mute and unmute 16 tracks at once (their clips keep their place), and while the pads are mute keys the PAD BANK buttons pick which 16 (A: tracks 1-16, B: 17-32, up to H: 113-128; section 4f) and are lit for that bank; a pad is lit green while its track plays a clip, dim green with nothing launched, red while muted, and dark with no track. **SHIFT + TRACK MUTE** does the same for the playing kit's pads (the speakers on Play): lit in the kit's colour while a pad plays, red while muted, dark without a sound. The same press again leaves, and the other one switches. The hint line says what the pads are, and the button is lit while they are mute keys. A pad held when TRACK MUTE is pressed still lets go as a note.
- **Found with Map controls first:** ERASE, Q-LINK and the mode buttons have no measured note yet, so each works once Map controls has found it (TRACK MUTE, NEXT SEQ, STEP SEQ, SAMPLER, SAMPLE EDIT, PROG EDIT and the data dial's push have rows since round 9).
- **The data dial's push** (Jay, 12 Nov 08:11Z): in Prefs and the options menus, it opens the picked row (its value box outlined), the wheel changes the value, and a second push sets it and gives the wheel back to the rows. On a row that opens something (System info, Touch screen, Back, Find the buttons again, Map controls, Calibrate touch, Touch lab) it opens it, as + does.
- **The data wheel on a page with rows** (Jay, 13 Nov 06:27Z): on Sample (every sub-page with a settings list), the Pads tab's Pad view, Synth and Arrange, the wheel moves the highlight through the rows (on Synth, the tiles) and - / + step the highlighted one, as in the menus. A push opens the highlighted row for the wheel (its value box outlined, the Synth tile raised); the wheel then changes it, and a second push gives the wheel back to the rows. Changing page closes it. SHIFT keeps each page's second wheel job: SHIFT + wheel is the next pad with a sound on Sample and Pads, the Synth page on Synth and scrolling on Arrange; SHIFT + - / + zooms the wave on Wave, Warp and Chop. Pads' Browse view keeps the wheel it had; Play, the Looper tab, the mixer and the chain editor have the outline (next).
- **The data wheel's outline** (Jay, 13 Nov 14:26Z: the wheel as the main way round the screen besides touch). On Play, the Looper tab, the mixer, the chain editor and the macros panel, a tap puts an outline on the control it lands on, and the wheel moves the outline from control to control: through the panel in reading order (rows top to bottom, each left to right), then on into the next panel (panels by their top, then their left), stopping at the first and the last. Controls drawn as unavailable are skipped.
  - **The push** does what a tap does on a button, a row, a pad or a step. On a control that is held (a throw) it holds it down until the next push. On a value (a mixer fader, pan or send, a Q-Link card, the master level, an effect's knob, a slider) it grabs it: the outline thickens in the second accent colour, the wheel turns the value by the same steps as its Q-Link, and the next push lets it go. Values the wheel can't turn yet (the XY pad, waveform markers, the Looper tab's trims, the macros panel's knobs) say "drag it with a finger". On the picked effect in the chain editor the push picks it up (the outline thickens), a turn or – / + moves it a place along the chain per detent with the outline on it, and the next push puts it down (Jay, 14 Nov 12:03Z; section 4b).
  - **– / +** step an outlined value without a push, and move the outline on anything else.
  - **When it starts:** on Play and the Looper tab the first turn shows the outline on the first control; on the mixer, the chain editor and the macros panel the wheel keeps its job there (the selected strip's volume, Q-Link 1) until a tap or a push puts the outline on a control. Changing page, or a tap where there is no control, takes it away. SHIFT + wheel and NOTE REPEAT keep their jobs.
  - **In the effect picker** the wheel turns the tiles, SHIFT + wheel the groups, and a push opens presets and then adds, whether or not the outline is on the chain editor (the FX thread's picker, #315). With the picker shut, the push is the outline's.
  - The pages with rows (Sample, Pads, Synth, Arrange), the menus, the Arp panel, the workspace and the Dev tab keep their own wheel for now.
  - How it is built: the screen keeps the outline, since it knows each widget's controls (`Widget::targets`, from the control audit) and which of them the wheel can turn (`Widget::steps` and `Widget::step`). It tells the app whether it shows one (`FromUi::Outline`), and the app sends the wheel, its push and – / + to it as `Effect::Nav` ([UI architecture](ui-architecture.md#the-wheels-outline)).
- **The transport lights** (Jay, 13 Nov 06:37Z, card "Capture Akai's lights first"): PLAY, REC and OVERDUB light only once a read-only capture under Akai's 3.6.0 shows the message its software sends for them; until then the app sends nothing for them and says so once in the Console. **The capture ran in live test 3 (13 Nov 09:33Z):** PLAY is lit while playing, REC and OVERDUB blink while armed (with PLAY) and while recording, all with values 3 and 0 (controller protocol page, "Akai's transport lights"). Wiring them is the next build. The MPC One's STOP button has **no LED** (Jay, 13 Nov), so the STOP light the app sends shows nothing on this model.
- **BROWSE** (controls decision 13; Jay after live test 3): opens the browser over the page and goes back to it (section 3, "Not tabs"). While it shows, the wheel, its push, - / +, PLAY and Q-Links 1-4 work it as on the Pads tab's Browse view, SHIFT + wheel moves through the places, and ERASE deletes the item picked (into the Trash, asking first). **SHIFT + BROWSE** saves the project on any page. BROWSE has no measured note yet, so it works once Map controls has found it.
- **Still to come from the control map:** hold MAIN + pad to pick a track; SHIFT + NEXT SEQ (Perform FX); COPY and TC.

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

- **A Mix/FX tab** in the top bar for the mixer and effects, and a **dramatic overhaul of adding effects** to pads, tracks and audio channels. (**+ FX** on the mixer is built; see below. **The Mix tab** is built: section 3a, #353.)
- **BROWSE opens the top-level browser** (projects, samples and the rest), as the browser plan's overlay under the top bar; projects load from it and **save**. A **New project** button starts from a fresh, empty project. **Built** by the project thread ("Add project New, Save and Load", 13 Nov): see section 3 ("Not tabs") and section 6 (BROWSE).
- **Touch targets big enough to hit, or reachable with the wheel and – / +**, everywhere. The rule is in section 1, and the mutes and + FX are built (below); the wheel's outline reaches every control on Play, the Looper tab, the mixer and the chain editor (section 6). The FX page's buttons and the pages with rows still need a pass.
- **More steps and more tracks** in the sequencer, with a clear way to add them (32, 64, 128 steps; more than 8 tracks). (**The steps** are built: the pattern's head in section 4. **More tracks** are built, up to 128: section 4f, #332.)
- **Several throws at once** with several fingers on the Play page. The screen and the mixer already hold one throw per finger (tests `two_fingers_hold_two_throws_at_once` and `two_throws_held_at_once_each_hold_their_pad`), so what's left is the panel: the device check `touch` (step 3) checks that it reports two fingers.
- **Typing a number** for fields with a wide range (Warp tempo first), instead of only the wheel. (**Built** for the Warp tempo and the tempo: section 1, Typing a number. Other wide fields can ask for the same pad.)
- **UNDO in every context** (Jay, 13 Nov 13:29Z: think about how UNDO and REDO work in different contexts). UNDO follows the page (his pick, 13:53Z) and **hold UNDO for the page's history** are built (section 6, REC, OVERDUB, UNDO). Still to do from the proposal: undo for the chain editor and the macros panel, which come with the effects overhaul.

**Built in the live test 3 fixes PR** (the app thread, 13 Nov), so no longer open above:
- **RAM %** beside CPU % in the top bar, and the kit chip names the kit (section 1, Status chips).
- **A tap on the tempo** on Play's transport bar taps the tempo, as TAP TEMPO does (taps in time set it). The tap area is the tempo and its "BPM", the bar's full height and at least 64 px wide. Held still for 0.6 s, it opens the number pad to type the tempo (section 1, Typing a number; batch 3).
- **A clear way to granular:** the Grain page's Mode row reaches Granular, and the note over its picture is a "tap to play this pad as grains" button, 64 px tall.
- **The metronome** off by default, at -24 dB, with **Metronome** and **Metronome level** rows in the Play page's options; SHIFT + PLAY START still toggles it, and the choice is saved.
- **Saying in place why something did nothing** (Jay, 12 Nov 07:00Z): the hint line shows the reason in amber for 4 s, then the hint comes back.

**Built in the live test 3 batch 2 PR** (the app thread, 13 Nov):
- **Bigger mutes:** a pattern lane's whole name is its mute button, the loop strip's layer names and speakers are one button each, and the Looper tab's loop cards and layer rows have boxed speakers with bigger tap areas (sections 4 and 4a).
- **+ FX on the mixer:** a strip's empty insert rows are one "+ FX" button (on a strip with no effects, all four rows), and a tap on any of them adds an effect.
- **The contrast rule** for button labels (section 1), with the Looper tab's tools and Use the loop rows and the chain editor's and mixer's off effects fixed first.
- **The Drum view folded into Beats** as the Your timing strip (Jay's pick, 13 Nov 13:09Z; section 4).
- **ERASE + a tap deletes** a clip, a pattern lane's steps or a Looper layer, each with its undo (section 6).

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
