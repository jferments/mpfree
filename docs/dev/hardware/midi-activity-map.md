# MPC One / az0x 3.6.0: which internal MIDI port carries which control

> **Evidence type:** owner-run `collect-over-ssh.sh --midi-activity` (read-only RawMIDI counter deltas), 2026-11-09 22:31 UTC, factory app running (PID 317 in this boot). Report SHA-256 `7919cc567c41debf59362d98ce2174bf03d8466f757fd3f8d58d3e3fcc2c4501` (owner-local, not committed).
> **Method:** for each phase the owner did one kind of action for ~8.2 s; the tool diffed `/proc/asound/card0/midi0` byte counters before and after. Nothing was opened or sent. See [audio-midi-diagnostics](../guides/audio-midi-diagnostics.md).

## Raw deltas (bytes per ~8.2 s window)

| Phase | Input 1 Rx | Output 0 Tx | Output 1 Tx | Others |
| --- | ---: | ---: | ---: | --- |
| idle (baseline) | 26 | 24 | 99 | 0 |
| pads | **695** | **2,400** | 99 | 0 |
| pad pressure | **230** | **828** | 99 | 0 |
| Q-Link knobs | **1,154** | 24 | **873** | 0 |
| data wheel | **329** | **4,248** | **417** | 0 |
| SHIFT / FULL LEVEL | 56 | 24 | **249** | 0 |
| play / stop | 38 | 24 | 123 | 0 |
| master volume | 26 | 24 | 99 | 0 |

Input 0, Input 2 and Output 2 stayed at 0 for the whole run. `/proc/asound/card0/usbmidi` does not exist on this kernel.

Port names from `amidi -l`: substream 0 = "Public", 1 = "Private", 2 = "MIDI Port".

## Findings

- **OBSERVED:** every physical control we tried that reaches software (pads, pad pressure, Q-Links, data wheel, buttons, transport) arrives on **Input 1 ("Private")**. Input 0 ("Public") and Input 2 ("MIDI Port") carried nothing.
- **OBSERVED:** both outputs have a steady idle stream: Output 0 about 3 bytes/s and Output 1 about 12 bytes/s. Input 1 also carries about 3 bytes/s at idle.
- **OBSERVED:** master volume produced no traffic above baseline. **INFERENCE:** the volume knob is probably analog (or handled outside MIDI), so MPFree may not need to handle it in software.
- **INFERENCE:** Output 0 traffic rises with pads, pad pressure and the data wheel, but not with Q-Links or buttons. That fits pad RGB LED updates (the factory app recolors pads when hit and when the wheel changes the selection).
- **INFERENCE:** Output 1 traffic rises with Q-Links, buttons, transport and the wheel. That fits button LEDs and/or other control-surface feedback.
- **UNKNOWN:** the byte formats (note/CC/SysEx layout, velocity and pressure encoding, LED color format) and what the idle streams are (keep-alive, clock or polling). Counter deltas cannot tell us this.

## Next

- Decode message formats either with a passive usbmon capture (if `--audio-midi` shows usbmon is available) or after a reversible stop of the factory app, by opening `hw:0,0,1` read-only.
- LED output (MIDI-02) needs an explicit opt-in write test later; nothing here writes to the controller.
