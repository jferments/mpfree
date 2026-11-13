# MPC One / az0x 3.6.0: internal controller USB-MIDI protocol (observed)

> **Evidence type:** owner-run `collect-over-ssh.sh --midi-capture` (opt-in passive usbmon), 2026-11-09 22:52 UTC, factory app running. Report SHA-256 `b38a0962d5c377b054009111eca15d34e355c3aa79a3e5863869f8e891b50daf` (owner-local, not committed). Everything below was **observed on this unit** with the factory app driving the controller. Values marked "inferred" are not yet proven.
> **Method:** [audio-midi-diagnostics](../guides/audio-midi-diagnostics.md). Nothing was sent to the controller by MPFree.

## Transport

- USB-MIDI 1.0 class device `09e8:0046`, bulk EP 0x01 OUT / 0x81 IN, 64-byte packets, 4-byte event packets.
- Cable 0 = "Public", cable 1 = "Private", cable 2 = "MIDI Port" (silent in this capture). On 2026-11-12 cable 2 turned out to be **both 5-pin jacks**: MIDI In is cable 2 in and MIDI Out is cable 2 out, under Akai's app ([below](#the-rear-jacks-under-akais-app-2026-11-12)).
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
| Buttons | Note on `0x90` (channel 1): velocity 127 = press, **velocity 0 = release** (not `0x80`) | SHIFT 49, FULL LEVEL 39, PLAY 82, STOP 81; NOTE REPEAT 11, TAP TEMPO 53, + 54, − 55, MENU 123 (Hello MPC round 5 button guide, 2026-11-11). 52 and 73 are real buttons, not yet named. Others not yet captured |
| Master volume | No MIDI | Inferred analog |

## Output (host → controller)

| Purpose | Message | Notes |
| --- | --- | --- |
| Button LEDs | CC on cable 1, channel 1, **CC number = button note number** | Values 0–4 seen. 0 = off; 1 and 3 dominate (dim/bright, inferred). TAP TEMPO (CC 53) blinks 1/3 at the tempo; CC 67 (UNDO, per prior art) toggled 1/3 during Q-Link edits. The pad player uses 1 for off and 3 for on, as the app does |
| Pad RGB | SysEx on cable 0: `F0 47 7F 46 65 00 04 <pad 0–15> <R> <G> <B> F7` | 7-bit color components; pad index is 0-based in display order |
| `5A` setup message | SysEx on cable 0: `F0 47 7F 46 5A 00 20 …` (32-byte payload) | Sent twice at the app's startup; full bytes [below](#akais-app-at-startup-hello-mpc-round-2-2026-11-10). Meaning unknown (the earlier guess, bulk pad colours, is not supported by the bytes) |
| CV/Gate jacks | CC pairs on cable 1, **channel 16**: Akai's CV port 1 on CC 0 and 1, port 2 on CC 2 and 3, and so on, high 7 bits first | One 14-bit value per port, whatever it is used for. A CV track's pitch is `floor((note − 36) × 16383 / 60)`, its gate 16383 or 0. Volts per count not measured ([below](#second-round-the-cv-scale-and-the-next-outputs-2026-11-12)) |
| Session ping | Host → cable 0: `F0 47 00 46 52 00 04 <token> F7` every 4–5 s | Controller answers on cable 1: `F0 47 7F 46 53 00 05 <token> <status> F7`. The app picks a new 4-byte token at each start and keeps it for the whole run ([below](#akais-app-at-startup-hello-mpc-round-2-2026-11-10)) |

## Comparison with prior art

TheKikGen's MPCLiveXplore-libs lists the same device ID (0x46), the same pad note numbers, the same button numbers for SHIFT/FULL LEVEL/PLAY/STOP and the same pad RGB SysEx. Every value in the tables above was measured on this unit; no code or tables were copied (their source headers say CC BY-NC 4.0).

## Still unknown

- Remaining button numbers (52 and 73 seen but not named) and LED color/brightness meaning of values 0–4 (capture each button once).
- Data wheel push, any display/screen messages, and what each setup message (`41`, `5A`, `62`–`6A`) means.
- Which of the setup messages switches the controls to cable 1, and what happens to that mode when the session lapses.

## Without Akai's app (Hello MPC rounds 1 and 2, 2026-11-10)

On the [kexec test build](archive/2026-11/kexec-test-boot.md) (Akai 3.10.0 rootfs, kernel 6.18, Akai's app masked), our [`mpfree-hello`](../../../rust/mpfree-app) listened while the owner hit pads, turned Q-Links and the wheel, and pressed buttons. Nothing was sent to the controller in round 2's listening step.

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

**The transport lights are not known yet.** Akai's startup lights (below) include STOP (CC 81) and PLAY START (CC 83), but not REC (73), OVERDUB (80) or PLAY (82), so mpfree lights only STOP and notes the others as not sent (`App::transport_lights`). The read-only `MPFREE_CAPTURE_PHASES=transport` capture (added 2026-11-13, owner's choice to see them first) records what Akai's app sends as it plays, records and overdubs.

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

## The rear jacks under Akai's app (2026-11-12)

> **Evidence type:** owner-run `collect-over-ssh.sh --midi-capture` with `MPFREE_CAPTURE_PHASES=midi-cv` (opt-in passive usbmon), 2026-11-12 06:05 UTC, stock 3.6.0, Akai's app running, a keyboard on the 5-pin MIDI In jack, nothing on the CV/Gate jacks. Report SHA-256 `4233c54b57427d0f90f4e3123e391d5c556b10f96ceec5647f0a81a0cb9decd4` (owner-local, not committed). Nothing was sent to the controller by MPFree.

| Phase | What changed against idle |
| --- | --- |
| Keys on the keyboard on MIDI In | **IN cable 2**: note-on channel 6 (the keyboard's channel), note-on velocity 0 as note-off, notes 60 to 79 as played. Akai's app lit the pads on cable 0 in answer |
| The keyboard's clock | **IN cable 2**: `F8` at 48 a second (120 BPM) for the whole capture, from the keyboard |
| Pads on a MIDI track set to the MIDI Out jack | Only the pads (IN cable 1) and their colours (OUT cable 0). **Nothing went out on cable 2, or on any cable, as MIDI notes** |
| Pads 1 and 13 on a CV track | **OUT cable 1, channel 16, CC 0, 1, 2 and 3**, first in the transfer, before the pad colours (below) |
| Pad 1 held about 3 s on the CV track | The same four CCs at the press, CC 2 and 3 back to 0 at the release, nothing in between |
| One CV track slider | Only CC 67 on channel 1 blinking 1/3 (a button light, as in Q-Link edits). No channel 16 CCs |

CV messages seen (cable 1, `Bx` = `BF`):

| Event | CC 0 | CC 1 | CC 2 | CC 3 |
| --- | ---: | ---: | ---: | ---: |
| Pad 1 down (velocity 127, and again at 37) | 76 | 101 | 127 | 127 |
| Pad 13 down | 121 | 75 | 127 | 127 |
| Either pad up | — | — | 0 | 0 |

- **OBSERVED:** the 5-pin MIDI In jack is cable 2 in, under Akai's app.
- **OBSERVED:** Akai's app drives the CV/Gate jacks through the controller, as CCs on cable 1, channel 16. Velocity did not change them.
- At the time this looked like two 7-bit halves of one value per output (pad 1: 76 × 128 + 101 = 9829, pad 13: 121 × 128 + 75 = 15563), with the scaling unknown because Akai's pad notes were not known. The second round below settles both.
- **UNKNOWN** after round 1: how Akai's app sends to the MIDI Out jack. Nothing appeared on USB. The second round tried again.

### Second round: the CV scale and the next outputs (2026-11-12)

> **Evidence type:** owner-run `collect-over-ssh.sh --midi-capture` with `MPFREE_CAPTURE_PHASES=cv-scale`, 2026-11-12 06:25 UTC, stock 3.6.0, Akai's app running, the same keyboard on MIDI In, nothing on the CV/Gate jacks. Tool at branch commit `3313621`, clean. Report SHA-256 `771c890710116f27dd3b7533c4076d849932e600df247ab8b1b2442c6dca7690` (owner-local, not committed). Nothing was sent to the controller by MPFree.

The keyboard played known notes into a CV track, and the capture saw each note number on cable 2 next to the CCs Akai's app sent, so no pad layout is needed. Value = CC high × 128 + CC low:

| Note in (cable 2) | CC 0 | CC 1 | Value | `floor((note − 36) × 16383 / 60)` |
| ---: | ---: | ---: | ---: | ---: |
| 48 (C) | 25 | 76 | 3276 | 3276 |
| 60 (C) | 51 | 25 | 6553 | 6553 |
| 72 (C) | 76 | 101 | 9829 | 9829 |
| 61 (C♯) | 53 | 42 | 6826 | 6826 |
| 62 (D) | 55 | 59 | 7099 | 7099 |
| 63 (D♯) | 57 | 76 | 7372 | 7372 |

Every note gave CC 2 and 3 at 127 at the key down and 0 at the key up, as in round 1. Round 1's pads fit the same formula: pad 1 was note 72 and pad 13 note 93, so Akai's CV-track pads are not 12 semitones apart.

| Phase | What changed against idle |
| --- | --- |
| The CV track's CV Port set to 3 and Gate Port to 4 (they were 1 and 2), one C (60) | **CC 4 and 5** = 51, 25 (6553, the pitch of note 60) and **CC 6 and 7** = 127, 127 at the key down; CC 6 and 7 back to 0 at the key up. Nothing on CC 0 to 3 |
| Keys and pads 1 and 13 on a MIDI track (output meant to be the MIDI Out jack) | The keys on cable 2 in and the pads on cable 1 in (channel 10 notes and pressure), as usual. **Nothing out on cable 2, and no change on any OUT cable**, the same as round 1 |

- **OBSERVED:** each CV port is one 14-bit value (0 to 16383) sent as two CCs on cable 1, channel 16, the high 7 bits first. Akai's port 1 is CC 0 and 1, port 2 CC 2 and 3, port 3 CC 4 and 5, port 4 CC 6 and 7. The pair belongs to the port number, not to its use: a CV track's CV Port and Gate Port fields can be set to any port (the owner, 2026-11-12), and ports 1 and 3 carried pitch while 2 and 4 carried the gate only because that is how they were set.
- **OBSERVED:** a CV track's pitch is `floor((note − 36) × 16383 / 60)`: 0 at note 36, about 273 per semitone, 16383 five octaves up (note 96). Its gate is 16383 while the note is held and 0 after.
- **INFERENCE:** ports 5 to 8 are CC 8 to 15, and notes outside 36 to 96 clamp to the ends.
- **INFERENCE:** full scale is 5 V. Five octaves over the full range fits 1 V/oct at 0 to 5 V, but no meter has been on a jack, so the volts per count are **UNKNOWN** until one is.
- **UNKNOWN:** which jack, and tip or ring, each port number is.
- **Found in the third round (below):** why the MIDI track sent nothing in rounds 1 and 2. Only its MIDI Output Ch was set; its MIDI Out routing had to be set as well.
- The bytes are written down as code in `rust/mpfree-midi/src/cv_wire.rs`, with tests holding every value above. Sending any of these CCs from MPFree is a **state-changing** write to the controller and waits for the owner's go ([midi-io](../design/midi-io.md), step M6).

### Third round: the MIDI Out jack (2026-11-12)

> **Evidence type:** owner-run `collect-over-ssh.sh --midi-capture` with `MPFREE_CAPTURE_PHASES=midi-out`, 2026-11-12 07:06 UTC, stock 3.6.0, Akai's app running, the keyboard on MIDI In and a synth on MIDI Out, which the owner heard play. Tool at branch commit `50e4a11`, clean. Report SHA-256 `8803ee78b8cdd5b8d406e192efc157cfabf43879c3c032eabc6d5e3e6618cc13` (owner-local, not committed). Nothing was sent to the controller by MPFree.

Before this round the owner set the MIDI track's MIDI Out routing (to "MPC", 1) as well as its MIDI Output Ch (1). Rounds 1 and 2 had only the channel set, which is why nothing went out then.

| Phase | In | Out on **cable 2** |
| --- | --- | --- |
| C, E, G on the keyboard | cable 2: note-on channel 6, then note-on velocity 0 | note-on **channel 1**, same note and velocity; the release as a real note-off `80 nn 00` |
| One key held about 3 s | the same | note-on, then the note-off 3.1 s later; nothing in between |
| Pad 1 soft, pad 1 hard, pad 13 | cable 1: channel 10 notes 37 and 49 with pressure | note-on channel 1, **notes 72 and 93** (the MIDI track's pad notes), velocity 12, 127 and 17 for 13, 127 and 17 in; the pad's pressure as poly pressure `A0` on that note; a note-off at the release |
| PLAY, then STOP | cable 1: the PLAY and STOP buttons | only `B0 40 00` (sustain off, channel 1) at STOP. **No clock, start or stop** |

- **OBSERVED:** the 5-pin MIDI Out jack is **cable 2 out**, under Akai's app. The keyboard's notes and the pads came out there on the track's channel, and the synth on the jack played them.
- **OBSERVED:** Akai's app took 5.1 to 8.1 ms from a message arriving (cable 2, or a pad on cable 1) to sending it on cable 2 (usbmon times on the host, all 26 messages out). MPFree should do better than that.
- **OBSERVED:** Akai's app sends note-offs as `80 nn 00` even when the keyboard sent note-on velocity 0. It passes pad pressure on as poly pressure, but drops some steps: the closing 0 was not sent, and neither was a 127 just after a hit at 127. It did not pass the keyboard's clock through to MIDI Out.
- **OBSERVED:** round 1's CV-track pads were notes 72 and 93 by the CV formula. Pads 1 and 13 on this MIDI track sent the same notes, 72 and 93.
- **UNKNOWN:** why pad velocity 13 went out as 12 while 127 and 17 went out unchanged (a pad curve in Akai's app, perhaps).
- **UNKNOWN:** clock out. PLAY and STOP sent no clock, start or stop. Whether Sync was ticked for the port in Preferences > MIDI / Sync was not recorded.
- Writing to cable 2 from MPFree is a **state-changing** write to the controller (notes leave the jack) and waits for the owner's go ([midi-io](../design/midi-io.md), step M3).
