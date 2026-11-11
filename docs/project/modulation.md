# Modulation: parameter addresses, macros, the mod matrix, morph and Q-Link bindings

Two crates:

- **`mpfree-param`** names any parameter in MPFree with one small `Copy` type, `ParamAddr`. Parameter locks, macros, the mod matrix, morph scenes and Q-Link bindings all use it, so one parameter can be locked on a step, moved by a macro and swept by an LFO at once, and project files name it the same way everywhere.
- **`mpfree-mod`** is the engine that works out the value of every parameter something moves, and the pure model behind binding Q-Links to parameters and macros.

Agreed on 2026-11-11 with the threads that own the neighbours: the time engine (parameter locks), FX (chains, the mixer, effects), synths (instruments) and the Hello MPC thread (layout, Q-Links, the control map).

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
4. **Lock:** a parameter lock replaces all of the above until it is unlocked.
5. **Matrix:** each row aimed at it adds `source × depth (× via)`; the sum is clamped to the knob's travel.

Sources: 8 LFOs (sine, triangle, saws, square, sample and hold, drift; free 0.01 to 40 Hz or synced 1/32 to 8 bars; phase; retrigger), 4 envelope followers on audio the host passes in (attack, release, floor, gain), the 8 macros, and 8 host values (velocity, pressure, a touch strip ...). A row's `via` scales it by a second source, so the mod wheel can set an LFO's depth.

Macros and the morph position are targets too, so they can be locked on a step, swept by an LFO and (macros) stored in the scenes.

**Switches** (toggles and choices) can be moved by a macro, a lock or the morph, but the matrix refuses them, so nothing chatters between two settings. A synth's voice mode is never a target (changing it ends every note).

## Timing and cost

- The engine runs every **32 frames of absolute time** (not of the block), so it sends the same events, frame for frame, however the host cuts its blocks; a test checks that for blocks of 1 to 8192 frames.
- Sets, locks and unlocks take effect on their own frame, before a tick on the same frame, so a lock reaches a node before a note-on on that frame.
- A target is only worked out when something moved it, and only sent when its value changed. A parameter that nothing modulates goes out exactly as it was set.
- Nodes smooth their own parameters, so the 32-frame steps never zipper.
- `process` never allocates, locks or panics, and nor do the changes (`add_target`, `map`, `add_row`, `remove_target`, `capture_scene` ...) while there is room: 512 targets, 64 rows, 8 targets per macro.
- **Cost (desktop, `mpfree-bench --filter Mod`):** a busy performance set (8 macros moving 8 parameters each, 16 rows, the morph swept across 80 parameters, a lock every 16th) costs about 7 µs per 128-frame block mean, 0.5 % of the period at p99, and 70 KB. It sends about 320 parameter events per block; the nodes' cost of taking them is theirs. Counted against the `fx` budget until it has a line of its own. FM rebuilds its patch on every `set_param`, so modulating FM parameters is worth a bench on the device.

## Where each piece goes

- **Host:** owns the engine on the audio thread, passes it the block's mod events, and sends each event it returns on by owner: `Inst` to the track host, `Insert` to `Chain::queue`, `Strip` to `Mixer::queue`. Every parameter a clip locks must be a target (`add_target` on the control thread when the clip is compiled); a lock for one that is not passes through as it came and is counted.
- **Locks** come from the player as `Lock { addr, value }` and `Unlock { addr }` on exact frames.
- **Inside the nodes:** LFOs and envelopes inside a synth or an effect, and per-voice modulation (velocity, pressure, key tracking per note), stay there. This matrix is per track or per bus.

## Q-Link bindings

`mpfree_mod::Bindings` is pure control-thread logic for the app's Q-Link column, in banks of four:

- `turn(knob, detents, fine)` gives the value to set (send it as `ModEvent::Set`). 100 detents cover a binding's range, 1000 with SHIFT; a choice moves one step per detent.
- `to(knob, pos)` for a touch slider or a knob read as a position.
- `card(knob)` gives the card to draw: label, value text, `pos` 0 to 1, colour slot, and the target (or `None`: the knob is free). The same fields as the mixer's Q-Link cards.
- **Pages and banks:** the page on screen sets its own four knobs with `set_page(name, knobs)`; they show wherever the bank shown leaves a knob free, so a user binding overrides them. `Bindings::with_macros(names)` makes the usual banks: bank 0 with no bindings of its own (the page's knobs show), bank 1 Macros 1-4, bank 2 Macros 5-8; `next_bank()` steps through them and `title()` names what is shown ("Drive", "Macros 1-4"). In the chain editor the page's knobs are the first four knobs of the effect page shown (`ChainState::qlinks()`: Drive gives Mode, Drive, Tone, Bias).
- **Learn:** `learn(addr, info, label)` when a parameter is touched on screen; the next knob turned takes it, in the bank shown, over the page's knob, on every page until it is unbound.
- `save()` and `load()` as `key = value` lines for `mpfree-store`'s "mod" settings (the page's knobs are not saved).

## The chain editor and the macros panel (`mpfree-chain-ui`)

Two panels on `mpfree-ui`, drawn with theme tokens only. Where they open is the Hello MPC thread's call and Jay's; Hello's proposal (layout spec, "The FX chain editor and macros"), which the previews follow:

- The **chain editor** takes the mixer's place when a strip's insert is tapped (`MixAction::OpenInsert` / `AddInsert`), no tab lit, with "‹ Mixer" in the list's corner (`ChainState::back`, `ChainAction::Back`). In the Sample tab it sits in the FX sub-tab under the pad bar, with a switch between the pad's chain and the track's (`ChainState::scopes`, `ChainAction::Scope`); which one it shows first is to agree with the sampler thread.
- **Macros 1-4 and 5-8** are banks of the Q-Link column; a long press on a macro card opens the **macros panel** over the page under the top bar, no tab lit, with "‹ Back" in its corner (`MacroAction::Back`).
- The Q-Link column is the Play page's (168 px) beside the panel; the hint line says what the buttons do on the page.

Previews: `cargo run -p mpfree-chain-ui --bin mpfree-chain-preview` (workstation only; writes PNGs to a new folder). Until round 7 lands, the preview's top bar is a stand-in drawn as `Header::app` draws it (no wordmark), and its Q-Link column a stand-in in the Play mockups' card shape.

- **Chain editor** (`ChainView`): the strip's inserts (number, name, a line about its settings, a dot per macro that moves it, `~` when the matrix does, a power button that bypasses with spillover; Up, Down, Remove, + Add effect); the selected effect's page (its parameter groups as tabs, up to eight knobs, each with a green ring for how far modulation moves it and a badge in its macro's colour); and the Modulate column for the knob last touched (macro 1 to 8, Learn to a Q-Link, matrix rows with a depth bar, + LFO, + Follower; a switch takes macros only).
- **Macros and morph** (`MacrosView`): four macro cards at a time (drag the knob; each target shows its name, its range as a bar, where it is now, and the range as text; a tap asks to edit its range and curve), and the morph: A to B slider, what changes (each parameter's A and B values and where it is now), Snap back to A, Save A, Save B.
- Touches become `ChainAction`s and `MacroAction`s for the app: parameter moves go to the mod engine as `ModEvent::Set`, chain edits to the mixer as `Edit`s, macro maps and rows to the engine. `strip_of` and `strip_id` convert between the mixer's `StripId` and the address's `Strip`.
