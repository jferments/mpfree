# MPC One / az0x 3.6.0: internal controller USB-MIDI protocol (observed)

> **Evidence type:** owner-run `collect-over-ssh.sh --midi-capture` (opt-in passive usbmon), 2026-11-09 22:52 UTC, factory app running. Report SHA-256 `b38a0962d5c377b054009111eca15d34e355c3aa79a3e5863869f8e891b50daf` (owner-local, not committed). Everything below was **observed on this unit** with the factory app driving the controller. Values marked "inferred" are not yet proven.
> **Method:** [AUDIO_MIDI_DIAGNOSTICS](../engineering/AUDIO_MIDI_DIAGNOSTICS.md). Nothing was sent to the controller by MPFree.

## Transport

- USB-MIDI 1.0 class device `09e8:0046`, bulk EP 0x01 OUT / 0x81 IN, 64-byte packets, 4-byte event packets.
- Cable 0 = "Public", cable 1 = "Private", cable 2 = "MIDI Port" (external DIN/USB MIDI path; silent in this capture).
- **All physical controls arrive on cable 1.** Button/encoder LEDs are written on cable 1; pad RGB colors on cable 0.
- SysEx manufacturer `0x47` (Akai), device/product ID **`0x46`** for the MPC One.

## Input (controller → host), cable 1

| Control | Message | Values |
| --- | --- | --- |
| Pads | Note on `0x99` / note off `0x89` (**channel 10**) | Velocity 1–127 on press; note off velocity 0 |
| Pad pressure | Poly aftertouch `0xA9` (channel 10), same note as the pad | 0–127, streamed while held |
| Pad note numbers, pads 1–16 (bottom-left → top-right, row by row) | `0x25 0x24 0x2A 0x52 0x28 0x26 0x2E 0x2C 0x30 0x2F 0x2D 0x2B 0x31 0x37 0x33 0x35` | Non-sequential: the engine needs a lookup table |
| Q-Link 1–4 turn | CC 16, 17, 18, 19 (channel 1) | **Relative**: 1, 2 = clockwise steps; 127, 126 = counter-clockwise steps |
| Q-Link 1–4 touch | Note on 84–87 (`0x54`–`0x57`) velocity 127 (channel 1); release = note off `0x80` | Capacitive touch |
| Data wheel turn | CC 100 (channel 1) | Relative: 1 = clockwise, 127 = counter-clockwise |
| Buttons | Note on `0x90` (channel 1): velocity 127 = press, **velocity 0 = release** (not `0x80`) | SHIFT 49, FULL LEVEL 39, PLAY 82, STOP 81 (others not yet captured) |
| Master volume | No MIDI | Inferred analog |

## Output (host → controller)

| Purpose | Message | Notes |
| --- | --- | --- |
| Button LEDs | CC on cable 1, channel 1, **CC number = button note number** | Values 0–4 seen. 0 = off; 1 and 3 dominate (dim/bright, inferred). TAP TEMPO (CC 53) blinks 1/3 at the tempo; CC 67 (UNDO, per prior art) toggled 1/3 during Q-Link edits |
| Pad RGB | SysEx on cable 0: `F0 47 7F 46 65 00 04 <pad 0–15> <R> <G> <B> F7` | 7-bit color components; pad index is 0-based in display order |
| Bulk pad colors (inferred) | SysEx on cable 0: `F0 47 7F 46 5A 00 20 …` (32-byte payload) | Seen once; usbmon text truncated it. Meaning not yet decoded |
| Keep-alive / ping | Host → cable 0: `F0 47 00 46 52 00 04 7D 6D 57 35 F7` about every 5 s | Controller answers on cable 1: `F0 47 7F 46 53 00 05 7D 6D 57 35 01 F7`. The 4-byte token was constant for the whole run. **Unknown whether the controller needs it**; MPFree should send it until proven otherwise |

## Comparison with prior art

TheKikGen's MPCLiveXplore-libs lists the same device ID (0x46), the same pad note numbers, the same button numbers for SHIFT/FULL LEVEL/PLAY/STOP and the same pad RGB SysEx. Every value in the tables above was measured on this unit; no code or tables were copied (their source headers say CC BY-NC 4.0).

## Still unknown

- Remaining button numbers and LED color/brightness meaning of values 0–4 (capture each button once).
- Data wheel push, the `0x5A` bulk message, any display/screen messages, and whether the keep-alive is mandatory.
- Behavior when the factory app is not running (startup handshake, default LED state). Needs the reversible handoff gate.
