<!--
status: changing
covers: rust/mpfree-app/src/app.rs rust/mpfree-app/src/gui.rs
checked: 2026-11-13 dcaefdd
-->

# Prefs

**Prefs** holds MPFree's settings. Press **SHIFT + MENU** to open it, and **MENU** to close it. Turn the data wheel or tap to pick a row, and press **–** or **+** (or tap the arrows) to change it. Or press the data wheel down: the row's value box lights, and turning the wheel changes the value. Press the wheel again to set it, and the wheel goes back to picking rows. On a row that opens something, such as System info or Touch screen, pressing the wheel opens it. The panel on the right explains the row you picked.

{{shot: prefs | Prefs, with Pad velocity curve picked.}}

| Row | What it does |
|---|---|
| **Pad velocity curve** | How hard you must hit a pad for a loud note. **Soft:** loud notes come easily. **Hard:** more of the pad's travel stays below the top, for more dynamics. **Linear** plays the pad's own value. FULL LEVEL and Half Level override it. |
| **Latch keeps pads** | With Latch on, pads keep repeating after you let go. **Replace:** a fresh press starts a new set. **Toggle:** each press adds or removes that pad. |
| **Audio buffer** | How much sound is queued ahead of the speakers. 3 periods is what the factory software uses; 2 plays about 3 ms sooner after a hit, with more risk of clicks. Changing it restarts the sound. |
| **Find the buttons again** | Runs the button guide, which asks you to press MENU, NOTE REPEAT, TAP TEMPO, – and +, REC, OVERDUB, UNDO and PLAY START one at a time, so MPFree learns them. |
| **Show developer tools** | Adds the Dev tab, for people building MPFree. |
| **Sound cores** | Which of the MPC's cores make the sound, from the next start (the row says "(next start)" until then). **Shared:** as MPFree always ran. **Core 3:** the sound on a core of its own. **Cores 3 and 2:** some of the drums and the mixer's effects on a second core too. **Cores 3, 2 and 1:** a test that also puts the synth on a third core. Core 0 is always left for the MPC's own work and the screen. The Dev tab's CPU by engine page shows how it runs. Shared is the default until the device tests pick one. |
| **Loop memory** | How much sound the looper can hold, all loops and layers together: 1, 2, 5, 10 or 20 minutes of stereo, with the megabytes it takes (5 minutes, about 110 MB, is the default). The memory is set aside and kept in RAM when MPFree starts, so a new size applies from the next start; the row says "(next start)" until then. The Looper tab's Loop memory gauge shows how much is used. See [Recording loops](../making-music/looping.md#layers). |
| **Map controls** | Teach MPFree the MPC's other buttons, one at a time: pick a control, press **+** and press the button (**–** skips it). It also turns on the developer tools. |
| **Line input** | Turns the INPUTS on the back on or off. It is off every time MPFree starts. |
| **System info** | MPFree's version, and details of the MPC it runs on. **MENU** goes back to Prefs. |
| **Touch screen** | Opens the touchscreen's own rows: **Screen pad velocity** (the velocity of pads played on the screen, which can't sense how hard you tap), **Calibrate touch** (tap a target in each corner to check touches land where you touch; this version only reports the result) and **Touch lab** (a test page for touches). |

> [!MPC]
> On the MPC, **SHIFT + MENU** opens Preferences too, but MPFree's are far fewer: most settings live on the page they belong to, under **MENU**.
