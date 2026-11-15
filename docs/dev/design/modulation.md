# Modulation: parameter addresses, macros, the mod matrix, morph and Q-Link bindings

Two crates:

- **`mpfree-param`** names any parameter in MPFree with one small `Copy` type, `ParamAddr`. Parameter locks, macros, the mod matrix, morph scenes and Q-Link bindings all use it, so one parameter can be locked on a step, moved by a macro and swept by an LFO at once, and project files name it the same way everywhere.
- **`mpfree-mod`** is the engine that works out the value of every parameter something moves, and the pure model behind binding Q-Links to parameters and macros.

Agreed on 2026-11-11 with the threads that own the neighbours: the time engine (parameter locks), FX (chains, the mixer, effects), synths (instruments), the sampler (the Sample tab's FX sub-tab) and the Hello MPC thread (layout, Q-Links, the control map). In the app, the engine applies the clips' step locks to mixer controls and effects; the panels are not on its screens yet. [What is left](#what-is-left) lists the steps and what each waits on.

## Naming a parameter: `ParamAddr`

`ParamAddr { owner, param }`. `param` is the owner's own stable id: a node's `ParamInfo::id`, a mixer strip's `mpfree_fx::ids::mixer` id, an LFO's id in `mpfree_mod::lfo::ids`. Ids are only ever appended, so an address stays valid.

| Owner | Whose parameter | Text |
|---|---|---|
| `Inst(t)` | track `t`'s instrument (Poly, FM, ...) | `t3.inst.12` |
| `Pad(t, p)` | sampler pad `p`'s sound on drum track `t`; applies to the voice that starts on that frame | `t3.pad5.2` |
| `Strip(s)` | a mixer strip's controls (volume, pan, sends ...) | `t3.mix.0`, `bus2.mix.6`, `master.mix.0` |
| `Insert(s, uid)` | an insert on strip `s`, by its **slot id**, which follows the effect when it is reordered and is saved with the chain | `bus2.fx7.4` |
| `Arp(t)`, `Seq(t)` | a track's arpeggiator and sequencer settings | `t3.arp.1` |
| `Macro(n)` | macro `n`'s value (param 0), 0 to 1 | `macro2.0` |
| `Mod(n)` | mod source `n`: LFOs 0 to 7, followers 8 to 11 | `mod4.1` |
| `Morph(n)` | the morph position (param 0), 0 to 1 | `morph0.0` |

Strips are `t<n>` (track), `in<n>` (input), `ps<n>` (a pad channel of the mixer's pool), `bus<n>`, `ret<n>` and `master`. Indexes count from 0.

A clip can move to another track, so a clip stores a **`RelAddr`** (the address without its track: `inst.12`, `pad5.2`, `fx7.4`), and the player fills in the playing track with `RelAddr::on(t)` when it compiles the clip.

## How a value is worked out

A `ModEngine` looks after its **targets**: the parameters that a macro, a matrix row, a scene or a lock moves. Each target's value is worked out as a knob position (0 to 1 through its `ParamInfo` curve, so a sweep of a cutoff moves evenly in pitch), layer by layer:

1. **Base:** the value the user set (`ModEvent::Set`).
2. **Morph:** once scenes A and B both hold the parameter, the morph position glides it between them, and a turn of its knob edits the scene the morph is nearer.
3. **Macro:** a macro mapped to it puts it at `from + (to - from) × curve(macro)`. One macro per parameter; mapping it again moves it. Curves: linear, exp, log, S.
4. **Lock:** a step's parameter lock (`ModEvent::Lock`), or a move lane's value (`ModEvent::Lane`), replaces all of the above until it is unlocked.
5. **Touch:** a hand on the control (`ModEvent::Touch`) replaces all of the above until it lets go (`ModEvent::Untouch`), except that a step's lock keeps its value under the hand unless recording. Locks and lanes keep landing underneath, so the hand lets go to whatever they hold by then. See [A hand on a control](#a-hand-on-a-control).
6. **Matrix:** each row aimed at it adds `source × depth (× via)`; the sum is clamped to the knob's travel.

Sources: 8 LFOs (sine, triangle, saws, square, sample and hold, drift; free 0.01 to 40 Hz or synced 1/32 to 8 bars; phase; retrigger), 4 envelope followers on audio the host passes in (attack, release, floor, gain), the 8 macros, and 8 host values (velocity, pressure, a touch strip ...). A row's `via` scales it by a second source, so the mod wheel can set an LFO's depth.

Macros and the morph position are targets too, so they can be locked on a step, held by a hand, swept by an LFO and (macros) stored in the scenes.

**Switches** (toggles and choices) can be moved by a macro, a lock, a touch or the morph, but the matrix refuses them, so nothing chatters between two settings. A synth's voice mode is never a target (changing it ends every note).

## Timing and cost

- The engine runs every **32 frames of absolute time** (not of the block), so it sends the same events, frame for frame, however the host cuts its blocks; a test checks that for blocks of 1 to 8192 frames.
- Sets, locks, touches and their ends take effect on their own frame, before a tick on the same frame, so a lock reaches a node before a note-on on that frame.
- A target is only worked out when something moved it, and only sent when its value changed. A parameter that nothing modulates goes out exactly as it was set.
- Nodes smooth their own parameters, so the 32-frame steps never zipper.
- `process` never allocates, locks or panics, and nor do the changes (`add_target`, `map`, `add_row`, `remove_target`, `capture_scene` ...) while there is room: 512 targets, 64 rows, 8 targets per macro.
- **Cost (desktop, `mpfree-bench --filter Mod`):** a busy performance set (8 macros moving 8 parameters each, 16 rows, the morph swept across 80 parameters, a lock every 16th) costs about 7 µs per 128-frame block mean, 0.5 % of the period at p99, and 70 KB. It sends about 320 parameter events per block; the nodes' cost of taking them is theirs. Counted against the `fx` budget until it has a line of its own. FM's `set_param` reworks only what the parameter feeds (about 0.15 µs on a desktop, ten times less than when every knob rebuilt the whole patch), but modulating FM parameters is still worth a bench on the device.

## Where each piece goes

- **Host:** owns the engine on the audio thread, passes it the block's mod events, and sends each event it returns on by owner: `Inst` to the track host, `Insert` to `Mixer::queue_insert` (found by its slot id), `Strip` to `Mixer::queue`. A lock for a parameter that is not a target passes through as it came and is counted, and its `Unlock` cannot put anything back.
- **In the app today** (`mpfree-app`, `modmix.rs`): the engine sits inside the mixer host (`MixHost::modu`) and runs before the mixer each period. `TimeHost` collects the period's `Lock` and `Unlock` events (`TimeHost::locks`, a fixed table of 256) and the audio loop hands them to `MixHost::lock` on their frames. A mixer strip control or insert parameter becomes a target on its first lock, with the value the mixer has then, so the engine costs next to nothing until a clip locks something. From then on the host's own turns of that control (the mixer panel, the Play page, the Level) go through the engine as `Set`s (`MixHost::set`), so the end of a lock puts back the last turn. Locks on a pad's sound or an instrument are counted, not applied, until the sampler and the track host take parameter events. The 30 s status line has a `mod:` part (targets, values sent, values with no place in the mixer, events dropped).
- **The panels' side in the app** (`mpfree-app`, `modpanels.rs`; not on the screens yet): `ChainHost` and `MacrosHost` read the states straight from the mixer's chains and the engine (the mixer and the engine live on the same loop), and apply the panels' actions:
  - Chain edits go to the mixer: `Edit::Insert` with the effect from `mpfree_fx_ui::make` at the end of the chain, `Edit::Move`, `Edit::Remove` (the engine forgets the removed effect's parameters, with their maps and rows), and bypass with `Mixer::set_on`. The editor keeps the same effect selected across a move, and selects an added one when it lands. A full chain (8 inserts a strip in the app) refuses with a reason for the hint line.
  - A knob turn goes to the engine by `ModEvent::hand`'s rule when the engine holds the parameter, else straight to the mixer; `Release` sends `Untouch`.
  - A knob put on a macro is mapped from where it is now to the far end of its travel, Linear (the target editor changes it); a knob already on another macro keeps its range. Matrix rows start at 25 % depth. These, the target editor's maps and the scenes go to the engine as `Change`s (`ModMix::change`), and the parameter becomes a target then.
  - Learn, the full effect page and Back come back to the app (`ChainOut`). `ChainHost::from_mixer` opens from the mixer's insert row or its "+" (picker up), `ChainHost::for_pad` is Sample > FX (the pad's chain and the track's).
  - `ModMix::save` writes the engine's set-up and the macros' names (`name.2 = Sweep`); `ModMix::load` takes it back once the mixer has its inserts.
- **Locks** come from the player as `Lock { addr, value }` and `Unlock { addr }` on exact frames.
- **Inside the nodes:** LFOs and envelopes inside a synth or an effect, and per-voice modulation (velocity, pressure, key tracking per note), stay there. This matrix is per track or per bus.

## Parameter locks

Jay chose the Elektron way on 2026-11-11. The time engine's side (locks in clips, the player, recording them) is in PR #122.

- A clip stores a lock as a `RelAddr` and a value on a tick. A lock without a note on its step (trigless) is allowed.
- The player sends `Lock { addr, value }` on the lock's frame, after that frame's note-offs and before its note-ons, so the note starting there plays with it.
- A lock lasts for its own note: the track's next note without a lock on that parameter sends `Unlock`, and the parameter goes back to what the layers below give. Stop, locate, a loop back and a clip switch unlock everything; a muted track sends none.
- A lock moves the whole instrument, not one voice of a poly synth. A pad's lock (`Pad(t, p)`) reaches the voice that starts on that frame.
- Instrument, pad, mixer strip and insert parameters can be locked, and so can a macro's value and the morph position.
- Turning a Q-Link while recording records a lock: the app passes the move to the time engine (`TimeHost::knob`), which puts it on the held pad's note, else on a note that started in the last 240 ticks, else on the nearest grid line as a trigless lock. So the move is heard while it is being recorded, over the step's old lock too, the app also sends it to the engine as `ModEvent::Touch { over_locks: true }` (what `ModEvent::hand` gives while recording), and `Untouch` when the hand lets go. A `Lock` from the hand would be cut off by the player's `Unlock` at the track's next note without a lock.
- A lock sits above the morph and the macros and below a hand and the matrix, so an LFO still moves a locked value.
- FX moves (a clip's recorded knob, XY pad and Perform FX moves, from the FX thread's `mpfree-moves`) reach the engine through the same slot, but as `ModEvent::Lane` (ended by `Unlock`), so the engine can tell them from a step's lock: a hand beats a lane, but not a step's lock unless recording. A step's own lock beats a move lane on the same parameter for that step; the player sends whichever wins (agreed by the time engine and FX threads).

## A hand on a control

A clip can be moving a parameter (a move lane, or a step's locks) while the user grabs its control. Jay's rule (2026-11-11, 20:42Z): **a hand always beats a move lane, and beats a step's lock only while recording.** When not recording, a locked step keeps its lock under the hand, as on Elektron machines, and the hand is heard again when the step's lock ends.

- The host sends `ModEvent::Touch { addr, value, over_locks }` for each move while a finger or knob is on the control, and `ModEvent::Untouch { addr }` when it lets go. The target follows the hand over a lane always, and over a step's lock only when `over_locks`. Locks and lanes keep landing underneath, so on `Untouch` the target goes back, on that frame, to whatever the lane or lock holds by then.
- The matrix still adds on top, so an LFO keeps moving around the hand.
- There is one touch per target, and the last touch wins. When two controls can hold the same parameter, the host sends `Untouch` when the last of them lets go.
- Stop, locate and the player's unlock-all leave a touch alone; only `Untouch` or removing the target ends it. `ModEngine::is_touched` says whether a hand holds a target.
- A touch on a parameter that is not a target passes through as a plain value, as a set does.

**Which event the host sends for a move:** `ModEvent::hand(addr, value, lane, recording)`, where `lane` says the playing clip has a move lane on the parameter and `recording` that its track is recording:

| | Not recording | Recording |
|---|---|---|
| The parameter has a move lane | `Touch`, `over_locks: false`: beats the lane, not a step's lock | `Touch`, `over_locks: true`: beats both |
| No lane (step locks or nothing) | `Set`: the turn stays when the hand lets go; locked steps keep their locks | `Touch`, `over_locks: true` |

This needs the player to send a lane's values as `Lane`, not `Lock` (for the time engine and FX threads to wire).

The touchscreen's XY pad and DJ knob report touch down and up. Akai sells the MPC One's Q-Links as touch-sensitive, but nobody has yet confirmed that MPFree's controller code sees the touch. Until it does, the host sends `Untouch` about 500 ms after a Q-Link's last detent.

## Q-Link bindings

`mpfree_mod::Bindings` is pure control-thread logic for the app's Q-Link column, in banks of four:

- `turn(knob, detents, fine)` gives the value to set (send it as `ModEvent::Set`). 100 detents cover a binding's range, 1000 with SHIFT; a choice moves one step per detent.
- `to(knob, pos)` for a touch slider or a knob read as a position.
- `card(knob)` gives the card to draw: label, value text, `pos` 0 to 1, colour slot, and the target (or `None`: the knob is free). The same fields as the mixer's Q-Link cards.
- **Pages and banks:** the page on screen sets its key, its name and its own four knobs with `set_page(key, name, knobs)`; they show wherever the bank shown leaves a knob free, so a user binding overrides them. `Bindings::with_macros(names)` makes the usual banks: bank 0 with no bindings of its own (the page's knobs show), bank 1 Macros 1-4, bank 2 Macros 5-8; `next_bank()` steps through them and `title()` names what is shown ("Drive", "Macros 1-4"). In the chain editor the page's knobs are the first four knobs of the effect page shown (`ChainState::qlinks()`: Drive gives Drive, Tone, Bias; its Mode is the Mode chooser, not a knob), or, for an effect with a Main page, Main's four on every page (`ChainState::main_four`, from `mpfree_fx::main_page`).
- **Learn:** `learn(addr, info, label)` when a parameter is touched on screen; the next knob turned takes it, in the bank shown, over the page's knob, **on the page it was learned on** (saved as `page.<key>.<bank>.<knob>`), so a Q-Link learned to a reverb's mix in the chain editor leaves Play's and Synth's knobs alone. `learn_for(..., Scope::EveryPage)` makes it global instead, as the macro banks are. `unbind` takes the page's own binding off first.
- **Sets:** a page can have several named sets of its own four knobs (an instrument's Tone, Shape, Motion, Mix; Jay approved Q-Link pages per sound at 22:47Z): `set_page_sets(key, name, sets)`, `next_set()`, `show_set(i)`, `page_set()`, `page_sets()`. The title reads "Poly · Tone" when there is more than one. A refresh with the same key and number of sets keeps the set shown. A learn while bank 0 is shown lands in the set shown and raises `take_sets_edited()`, so the page saves its sets with what it shows (a sound's preset); they are never in `save()`'s lines. `set_page` is one set, as before, and the macro banks stay on `next_bank()`.
- `save()` and `load()` as `key = value` lines for `mpfree-store`'s "mod" settings (the page's knobs and sets are not saved).

## Saving the set-up, and changing it while it plays

The engine's set-up (macro values and maps, the morph, matrix rows, scenes, LFO and follower settings) saves as `key = value` text, `ModEngine::save()`, for a project file or the store:

```text
macro.0 = 0.62
morph = 0.64
map.0.0 = t4.mix.6 0.45 0.85 linear
map.1.3 = ps0.fx1.1 0.1 0.5 linear
row.0 = lfo0 - bus0.fx2.2 0.18
row.2 = lfo1 host2 bus0.fx1.2 0.12
scene.bus0.fx1.2 = 20000 900
scene.macro0.0 = 0.62 0.9
```

Each line is `key = value`: a macro's value; the morph; macro `m`'s map `k` (target, from, to as knob positions, curve); matrix row `i` (source, `via` or `-`, target, depth); a parameter's scene A and B values (`-` where a scene doesn't hold it); and an LFO's or follower's settings (`source.mod<n>.<param>`). Targets are written as `ParamAddr` text (`t4.mix.6` is track 4's strip parameter 6, `bus0.fx2.2` is bus 1's insert with slot id 2, parameter 2).

`ModEngine::load(text, target)` replaces the maps, rows and scenes with what the text has. `target` gives a parameter's description and value now (from the chain or mixer it lives in); a line naming one it doesn't know (an effect since removed) is skipped, and `load` returns how many lines it skipped. Sources and curves are written as words (`lfo3`, `fol1`, `macro2`, `host0`; `linear`, `exp`, `log`, `s`).

The engine lives on the audio thread, so the control thread changes its set-up with messages: `mpfree_mod::Change` (`Target`, `RemoveTarget`, `Map`, `Unmap`, `AddRow`, `Depth`, `RemoveRow`, `Capture`, `ClearScenes`), applied with `ModEngine::change` at the start of a block. Each does what the call of the same name does, without allocating while there is room. The panels' actions map straight onto them (`MacroAction::Map` is `Change::Map`, `ChainAction::AddRow` is `Change::AddRow` ...).

## The chain editor and the macros panel (`mpfree-chain-ui`)

Two panels on `mpfree-ui`, drawn with theme tokens only: a macro's colour is the theme's series colour (`Theme::track[m]`) and text on an accent is `Theme::on_accent`, so every theme picks its own. Where they open is in the layout spec, section 4b (Jay approved Hello's proposal on 2026-11-11 at 22:34Z):

- The **chain editor** takes the mixer's place when a strip's insert row is tapped (`MixAction::OpenInsert`), no tab lit, with "‹ Mixer" in the list's corner (`ChainState::back`, `ChainAction::Back`); the mixer's "+" on a strip (`MixAction::AddInsert`) opens it with the effect picker up (`ChainState::picking`). In the Sample tab it sits in the FX sub-tab under the pad bar, with a switch between the pad's chain and the track's (`ChainState::scopes`, `ChainAction::Scope`). It shows the pad's own chain first (kit pad `p` is the mixer's pad channel `p`), as agreed with the sampler thread.
- **Macros 1-4 and 5-8** are banks of the Q-Link column; a long press on a macro card opens the **macros panel** over the page under the top bar, no tab lit, with "‹ Back" in its corner (`MacroAction::Back`).
- The Q-Link column is the Play page's (168 px) beside the panel; the hint line says what the buttons do on the page.

Previews: `cargo run -p mpfree-chain-ui --bin mpfree-chain-preview` (workstation only; writes PNGs to a new folder). The preview draws the app's top bar (`Header::app`) and a stand-in for the Q-Link column in the Play mockups' card shape. Nine screens: the drum bus's Drive, filter and the keys' Delay from the mixer, a pad's chain in Sample > FX, an empty chain, the effect picker, Macros 1-4 and 5-8, and the target editor.

- **Chain editor** (`ChainView`):
  - The strip's inserts: number, name, a line about its settings, a dot per macro that moves it, `~` when the matrix does, and a power button that bypasses with spillover. Under them are Up, Down, Remove and + Add effect.
  - **+ Add effect** opens the picker over the effect column: every effect the strip can take (Master safety only on the master), each with a line about it. A tap gives `ChainAction::Add(id)` for `mpfree_fx_ui::make(id)` and `Edit::Insert` at the end; Cancel closes it. A full chain says so instead.
  - The selected effect's page: its name (a tap opens its full effect page in `mpfree-fx-ui`, `ChainAction::OpenPage(slot)`), its parameter groups as tabs, the effect page's drawings for it (`state::picture`, shown with `set_chain_picture`; a tap opens the page too), and up to eight knobs. Each knob has a green ring for how far modulation moves it and a badge in its macro's colour.
  - The Modulate column for the knob last touched: macro 1 to 8, Learn to a Q-Link, matrix rows with a depth bar, + LFO and + Follower. A switch takes macros only.
  - A knob turn is `ChainAction::Set` and the finger lifting is `ChainAction::Release` (send `Untouch`; nothing happens if the turn was a `Set`).
- **Macros and morph** (`MacrosView`): four macro cards at a time. Drag a macro's knob; each target shows its name, its range as a bar, where it is now, and the range as text. A tap on a target opens its **editor** over the cards: drag the From and To handles along the parameter's travel (their values in the parameter's units), pick Linear, Exp, Log or S-curve (a drawing shows what the macro's turn does to the target), Flip the range, or Remove it from the macro. Its changes are `MacroAction::Map { m, map }` and `MacroAction::Unmap(target)`. The morph column has the A to B slider, what changes (each parameter's A and B values and where it is now), Snap back to A, Save A and Save B.
- The panels keep only what's on screen: the app rebuilds `ChainState` and `MacrosState` from the mixer's chains and the engine after each action (`ChainHost::state`, `MacrosHost::state` in `modpanels.rs`). An open picker or target editor stays open across a rebuild (while its target is still there).
- Touches become `ChainAction`s and `MacroAction`s for the app: parameter moves go to the mod engine as `ModEvent::hand` gives (a `Set` unless the parameter has a move lane or its track is recording), chain edits to the mixer as `Edit`s, macro maps and matrix rows to the engine as `Change`s. `strip_of` and `strip_id` convert between the mixer's `StripId` and the address's `Strip`.

## What is left

As of 2026-11-12 the engine, the bindings, both panels, the engine in the app's audio loop and what the panels' actions do there are written and tested; step locks on mixer strips and their effects reach the sound; and the panels are on the app's screens from the mixer and the Q-Link column (round 10, the app thread). In order:

1. **The rest of the panels in the app** (the app thread):
   - Built in round 10: the chain editor in the mixer's place from a strip's insert row (`MixAction::OpenInsert`) or "+" (`AddInsert`), with Back to the mixer view it showed (`ChainOut::Back`); the macros panel from a finger held on a macro card, with Back to the page it opened over; the Q-Link column on `Bindings::with_macros`, Q-LINK stepping its banks on Play and the editor, and a learn taking the next Q-Link turned (`ChainOut::Learn`). The audio loop's side is `modpanels::Panels`; the screen's is `chain_page.rs`. An added effect is built on the `mpfree-build` thread and lands at the end of its chain a few periods later; the effects edits take out are freed there too (see [concurrency.md](concurrency.md), "The control side and helper threads").
   - Left: `ChainOut::OpenPage` to `mpfree-fx-ui`'s effect page (a hint says it comes with the FX pages); the sampler's FX sub-tab ("FX" in `mpfree_sampler_ui::wave_tab::BUILT`, `ChainHost::for_pad`); and the set-up kept in the store (`ModMix::save` and the learned Q-Links), once the mixer's chains are kept too.
2. **Locks on pads and instruments:** the sampler and the track host take parameter events by address; then `modmix.rs` routes `Owner::Pad` and `Owner::Inst` as it routes strips and inserts.
3. **Move lanes:** the player sends a lane's values as `Event::Lane` (the transport and FX threads), and the host passes them on as `ModEvent::Lane`.
4. **Recording a Q-Link turn as a lock:** while a track records, the Q-Link turn also goes to `TimeHost::knob` and to the engine as `ModEvent::hand(.., recording: true)`, with `Untouch` 500 ms after the last detent until Q-Link touch is confirmed.
5. **On the MPC:** the Mod bench case's numbers, and a bench of FM under modulation, since FM rebuilds its patch on every parameter change.
