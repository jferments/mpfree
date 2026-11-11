# Modulation: parameter addresses, macros, the mod matrix, morph and Q-Link bindings

Two crates:

- **`mpfree-param`** names any parameter in MPFree with one small `Copy` type, `ParamAddr`. Parameter locks, macros, the mod matrix, morph scenes and Q-Link bindings all use it, so one parameter can be locked on a step, moved by a macro and swept by an LFO at once, and project files name it the same way everywhere.
- **`mpfree-mod`** is the engine that works out the value of every parameter something moves, and the pure model behind binding Q-Links to parameters and macros.

Agreed on 2026-11-11 with the threads that own the neighbours: the time engine (parameter locks), FX (chains, the mixer, effects), synths (instruments), the sampler (the Sample tab's FX sub-tab) and the Hello MPC thread (layout, Q-Links, the control map). Nothing here runs in the app yet; [What is left](#what-is-left) lists the steps and what each waits on.

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
- **Cost (desktop, `mpfree-bench --filter Mod`):** a busy performance set (8 macros moving 8 parameters each, 16 rows, the morph swept across 80 parameters, a lock every 16th) costs about 7 µs per 128-frame block mean, 0.5 % of the period at p99, and 70 KB. It sends about 320 parameter events per block; the nodes' cost of taking them is theirs. Counted against the `fx` budget until it has a line of its own. FM rebuilds its patch on every `set_param`, so modulating FM parameters is worth a bench on the device.

## Where each piece goes

- **Host:** owns the engine on the audio thread, passes it the block's mod events, and sends each event it returns on by owner: `Inst` to the track host, `Insert` to `Chain::queue`, `Strip` to `Mixer::queue`. Every parameter a clip locks must be a target (`add_target` on the control thread when the clip is compiled); a lock for one that is not passes through as it came and is counted.
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
- **Pages and banks:** the page on screen sets its key, its name and its own four knobs with `set_page(key, name, knobs)`; they show wherever the bank shown leaves a knob free, so a user binding overrides them. `Bindings::with_macros(names)` makes the usual banks: bank 0 with no bindings of its own (the page's knobs show), bank 1 Macros 1-4, bank 2 Macros 5-8; `next_bank()` steps through them and `title()` names what is shown ("Drive", "Macros 1-4"). In the chain editor the page's knobs are the first four knobs of the effect page shown (`ChainState::qlinks()`: Drive gives Mode, Drive, Tone, Bias).
- **Learn:** `learn(addr, info, label)` when a parameter is touched on screen; the next knob turned takes it, in the bank shown, over the page's knob, **on the page it was learned on** (saved as `page.<key>.<bank>.<knob>`), so a Q-Link learned to a reverb's mix in the chain editor leaves Play's and Synth's knobs alone. `learn_for(..., Scope::EveryPage)` makes it global instead, as the macro banks are. `unbind` takes the page's own binding off first.
- `save()` and `load()` as `key = value` lines for `mpfree-store`'s "mod" settings (the page's knobs are not saved).

## The chain editor and the macros panel (`mpfree-chain-ui`)

Two panels on `mpfree-ui`, drawn with theme tokens only. Where they open is the Hello MPC thread's call and Jay's; Hello's proposal (layout spec, "The FX chain editor and macros"), which the previews follow:

- The **chain editor** takes the mixer's place when a strip's insert is tapped (`MixAction::OpenInsert` / `AddInsert`), no tab lit, with "‹ Mixer" in the list's corner (`ChainState::back`, `ChainAction::Back`). In the Sample tab it sits in the FX sub-tab under the pad bar, with a switch between the pad's chain and the track's (`ChainState::scopes`, `ChainAction::Scope`). It shows the pad's own chain first (kit pad `p` is the mixer's pad channel `p`), as agreed with the sampler thread.
- **Macros 1-4 and 5-8** are banks of the Q-Link column; a long press on a macro card opens the **macros panel** over the page under the top bar, no tab lit, with "‹ Back" in its corner (`MacroAction::Back`).
- The Q-Link column is the Play page's (168 px) beside the panel; the hint line says what the buttons do on the page.

Previews: `cargo run -p mpfree-chain-ui --bin mpfree-chain-preview` (workstation only; writes PNGs to a new folder). The preview draws the app's top bar (`Header::app`) and a stand-in for the Q-Link column in the Play mockups' card shape.

- **Chain editor** (`ChainView`): the strip's inserts (number, name, a line about its settings, a dot per macro that moves it, `~` when the matrix does, a power button that bypasses with spillover; Up, Down, Remove, + Add effect); the selected effect's page (its parameter groups as tabs, up to eight knobs, each with a green ring for how far modulation moves it and a badge in its macro's colour); and the Modulate column for the knob last touched (macro 1 to 8, Learn to a Q-Link, matrix rows with a depth bar, + LFO, + Follower; a switch takes macros only).
- **Macros and morph** (`MacrosView`): four macro cards at a time (drag the knob; each target shows its name, its range as a bar, where it is now, and the range as text; a tap asks to edit its range and curve), and the morph: A to B slider, what changes (each parameter's A and B values and where it is now), Snap back to A, Save A, Save B.
- Touches become `ChainAction`s and `MacroAction`s for the app: parameter moves go to the mod engine as `ModEvent::hand` gives (a `Set` unless the parameter has a move lane or its track is recording), chain edits to the mixer as `Edit`s, macro maps and rows to the engine. `strip_of` and `strip_id` convert between the mixer's `StripId` and the address's `Strip`.

## What is left

As of 2026-11-11 the crates are on `main` (PR #102), tested and benched on the desktop, and nothing calls them yet. The steps, in order, with what each waits on:

1. **Placing the panels in the app** waits on Jay approving Hello's proposal (layout spec, "The FX chain editor and macros"). The Hello thread's round then wires them in:
   - The mixer's `OpenInsert { strip, slot }` opens the chain editor on that slot and `AddInsert` opens it with the effect picker; `Back` returns to the mixer.
   - The sampler adds "FX" to `mpfree_sampler_ui::wave_tab::BUILT`, and the FX sub-tab shows the chain editor.
   - The Q-Link column takes `Bindings::with_macros`, and a long press on a macro card opens the macros panel.
   - Chain edits go to `Mixer::edit` (bypass is `Chain::set_on`), and parameter moves go to the engine as `ModEvent::Set`.
2. **The engine in the audio loop** waits on PR #122. The host owns a `ModEngine` and turns the player's `Lock` and `Unlock` into `ModEvent`s at the same frame offset. It calls `add_target` for every address a compiled clip locks, and routes the engine's output by owner (see [Where each piece goes](#where-each-piece-goes)). A lane's values go to the engine as `Lane`, and hands as whatever `ModEvent::hand` gives, with `Untouch` when they let go (see [A hand on a control](#a-hand-on-a-control)), and Q-Link turns also go to `TimeHost::knob` while recording. The mod thread writes this and hands it to the Hello thread's round.
3. **Effect pages** wait on PR #116 (`mpfree-fx-ui`). `ChainAction::OpenPage(slot)` lets a tap on the selected insert's name open its full effect page, and the effect's picture (`mpfree_fx_ui::visual`) goes above the knobs.
4. **Not blocked:**
   - An effect picker for "+ Add effect" (`ChainAction::AddEffect` has nothing to open yet).
   - A range and curve editor for a macro's target (`MacroAction::EditTarget` has nothing to open yet).
5. **Later:**
   - Saving macro maps, matrix rows, scenes and LFO settings with a project. The engine has no save yet, and there is no project file to save into; the Q-Link bindings already save to `mpfree-store`.
   - The theme retrofit once the theme system (PR #72) lands.
   - The Mod bench case's numbers on the MPC, and a bench of FM under modulation, since FM rebuilds its patch on every parameter change.
