# MPC One / az0x 3.6.0: internal controller USB-MIDI protocol (observed)

> **Evidence type:** owner-run `collect-over-ssh.sh --midi-capture` (opt-in passive usbmon), 2026-11-09 22:52 UTC, factory app running. Report SHA-256 `b38a0962d5c377b054009111eca15d34e355c3aa79a3e5863869f8e891b50daf` (owner-local, not committed). Everything below was **observed on this unit** with the factory app driving the controller. Values marked "inferred" are not yet proven.
> **Method:** [audio-midi-diagnostics](../guides/audio-midi-diagnostics.md). Nothing was sent to the controller by MPFree.

## Transport

- USB-MIDI 1.0 class device `09e8:0046`, bulk EP 0x01 OUT / 0x81 IN, 64-byte packets, 4-byte event packets.
- Cable 0 = "Public", cable 1 = "Private", cable 2 = "MIDI Port" (external DIN/USB MIDI path; silent in this capture).
- **With Akai's app running, all physical controls arrive on cable 1.** Without it, they arrive on cable 0 ([below](#without-akais-app-hello-mpc-rounds-1-and-2-2026-11-10)). Button/encoder LEDs are written on cable 1; pad RGB colors on cable 0.
- SysEx manufacturer `0x47` (Akai), device/product ID **`0x46`** for the MPC One.

## Input (controller → host), cable 1

| Control | Message | Values |
| --- | --- | --- |
| Pads | Note on `0x99` / note off `0x89` (**channel 10**) | Velocity 1–127 on press; note off velocity 0. In 323 hits (Hello MPC round 4, after Akai's setup) every value from 1 to 127 occurred, but 28 % were exactly 127: firm hits saturate in the controller (inferred) |
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
| Button LEDs | CC on cable 1, channel 1, **CC number = button note number** | Values 0–4 seen. 0 = off; 1 and 3 dominate (dim/bright, inferred). TAP TEMPO (CC 53) blinks 1/3 at the tempo; CC 67 (UNDO, per prior art) toggled 1/3 during Q-Link edits. The pad player uses 1 for off and 3 for on, as the app does |
| Pad RGB | SysEx on cable 0: `F0 47 7F 46 65 00 04 <pad 0–15> <R> <G> <B> F7` | 7-bit color components; pad index is 0-based in display order |
| `5A` setup message | SysEx on cable 0: `F0 47 7F 46 5A 00 20 …` (32-byte payload) | Sent twice at the app's startup; full bytes [below](#akais-app-at-startup-hello-mpc-round-2-2026-11-10). Meaning unknown (the earlier guess, bulk pad colours, is not supported by the bytes) |
| Session ping | Host → cable 0: `F0 47 00 46 52 00 04 <token> F7` every 4–5 s | Controller answers on cable 1: `F0 47 7F 46 53 00 05 <token> <status> F7`. The app picks a new 4-byte token at each start and keeps it for the whole run ([below](#akais-app-at-startup-hello-mpc-round-2-2026-11-10)) |

## Comparison with prior art

TheKikGen's MPCLiveXplore-libs lists the same device ID (0x46), the same pad note numbers, the same button numbers for SHIFT/FULL LEVEL/PLAY/STOP and the same pad RGB SysEx. Every value in the tables above was measured on this unit; no code or tables were copied (their source headers say CC BY-NC 4.0).

## Still unknown

- Remaining button numbers and LED color/brightness meaning of values 0–4 (capture each button once).
- Data wheel push, any display/screen messages, and what each setup message (`41`, `5A`, `62`–`6A`) means.
- Which of the setup messages switches the controls to cable 1, and what happens to that mode when the session lapses.

## Without Akai's app (Hello MPC rounds 1 and 2, 2026-11-10)

On the [kexec test build](kexec-test-boot.md) (Akai 3.10.0 rootfs, kernel 6.18, Akai's app masked), our [`mpfree-hello`](../../rust/mpfree-hello) listened while the owner hit pads, turned Q-Links and the wheel, and pressed buttons. Nothing was sent to the controller in round 2's listening step.

Round 1 listened on cable 1 only and heard nothing. With the old keep-alive (`F0 47 00 46 52 00 04 7D 6D 57 35 F7` every 5 s) the controller answered on cable 1, but no control data arrived there.

Round 2 listened on all three cables. **Without Akai's app the controller sends almost everything on cable 0 (Public)**, with the same message numbers as under the app:

| Control | Message on cable 0 | Difference from the app's mode |
| --- | --- | --- |
| Pads | Note on `0x99` / note off `0x89` (channel 10), same notes; poly aftertouch `0xA9` | None seen |
| Q-Link 1 turn | CC 16 (channel 1) | **Absolute** 0–127: one turn counted 1, 2, 3 … 50. Under the app it is relative (1/2, 127/126). Q-Links 2–4 were not turned; CC 17–19 inferred |
| Q-Link 1 touch | Note on 84 (channel 1) | None seen |
| Data wheel | CC 100 (channel 1), 1 = clockwise, 127 = counter-clockwise | None |
| Buttons | Note on `0x90` (channel 1), velocity 127 press, 0 release, same numbers | None seen |
| Buttons 73, STOP (81), PLAY (82) | The note arrives on **cable 1**; cable 0 gets CC 119, 117, 118 (channel 1) with 127 on press and 0 on release | Not seen under the app |

No light came on: no LED or pad colour message was sent, and the controller does not light anything by itself.

## Akai's app at startup (Hello MPC round 2, 2026-11-10)

Captured on stock 3.6.0 with the binary usbmon reader while systemd restarted Akai's app (`collect-over-ssh.sh --app-start-capture`; owner-local report, not committed). Times are seconds from the capture's start. All host messages go to cable 0 unless stated. What each setup message means is not known; Hello MPC replays them byte for byte.

1. **Identity.** The new app sends the universal identity request `F0 7E 00 06 01 F7`. The controller answers on cables 0 and 1 with `F0 7E 00 06 02 47 46 00 19 …`, whose last bytes are its **serial number in ASCII**. `mpfree-hello` never shows those bytes. The capture tool's raw report does hold them, one more reason it stays with the owner; never quote them in the repo.
2. **Session.** Right after each identity request, the app sends `F0 47 00 46 52 00 04 <token> F7`. Its token is new at each start (this run: `15 21 6D 43`). The controller answers on cable 1 with `F0 47 7F 46 53 00 05 <token> <status> F7`:
   - status `00` while it still holds the old app's token; the reply carries the old token (+20.30 s and +21.77 s);
   - status `02` when it accepts the new token (+25.77 s, about 14 s after the old app's last ping);
   - status `01` at every later ping, every 4 s.
   Inferred from the timing: the controller lets an old token go about 10–14 s after its last ping.
3. **Setup, part 1** (+25.772 s, at once after `02`):
   `F0 47 7F 46 62 00 01 65 F7`,
   `F0 47 7F 46 64 00 03 00 01 01 F7`, `… 64 00 03 01 01 01 F7`, `… 64 00 03 02 00 00 F7`, `… 64 00 03 03 00 00 F7`,
   `F0 47 7F 46 63 00 03 00 00 00 F7`, `… 63 00 03 01 00 00 F7`,
   `F0 47 7F 46 41 00 00 F7`, then the identity request again.
   The controller echoes the four `64` messages on cable 1, the last as `03 00 0A` instead of `03 00 00`. It answers `41` with `F0 47 7F 46 41 00 13` and 19 zero bytes.
4. **Setup, part 2** (+25.784 s), twice: `F0 47 7F 46 5A 00 20 00 00 01 08 00 00 02 07 00 00 00 02 0E 08 0F 06 00 00 00 01 00 00 00 00 00 00 00 01 00 00 00 00 F7`.
5. **Lights** (+25.798 to +25.828 s): pad colours for pads 1–16, a gradient from `3F 0F 00` (orange) to `2C 5B 4C` (teal), and 58 button lights on cable 1 (CC = button note, value 1 or 3, once 4).
6. **Setup, part 3** (+26.170 s): `F0 47 7F 46 66 00 01 32 F7`, `… 67 00 01 14 F7`, `… 68 00 01 14 F7`, `… 69 00 01 14 F7`, `… 6A 00 03 00 00 00 F7`, `… 6A 00 03 01 00 00 F7`, `… 6A 00 03 02 01 00 F7`.
7. **The project's pad colours** (+27.134 s), then pings every 4 s, and TAP TEMPO's light (CC 53) blinking between 1 and 3.

In every Akai message above, byte 6 is the number of data bytes that follow it, before `F7`.

**Pad colour while hit.** When a pad is hit, the app sends it its resting colour scaled from 0–63 to 0–127, rounded down (`3F 00 00` → `7F 00 00`, `3B 3B 09` → `76 76 12`, `3F 22 00` → `7F 44 00`), then fades it back to the resting colour in a few steps (pad 1: `7F 00 00`, `72 00 00`, `54 00 00`, `3F 00 00` over about 0.19 s). Hello MPC sends the hit colour on press and the resting colour on release.

**Checked on the test build (Hello MPC round 3, 2026-11-10):**

- Pad colours work without any session or setup: the pads lit, and hit colours showed while a pad was held. (The button lights were not reported either way.)
- Replaying this sequence moves the controls to cable 1 with relative Q-Links, as under Akai's app. With nothing holding the controller, it accepted a new token at once (`02`), echoed the `64` messages and answered `41` exactly as above, and replied `01` to every later ping. After the setup, pads, Q-Link 1 (1/2 clockwise, 127/126/125 counter-clockwise), the wheel and the buttons arrived on cable 1, and only the identity replies came on cable 0.
