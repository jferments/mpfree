<!--
status: changing
covers: rust/mpfree-app/src/app.rs rust/mpfree-app/src/gui.rs rust/mpfree-app/src/app/saved.rs rust/mpfree-app/src/setup.rs
checked: 2026-11-19 79fc6f63
-->

# Prefs

**Prefs** holds MPFree's settings. Press **SHIFT + MENU** to open it, and **MENU** to close it. A page's own button closes it too: **MAIN**, **BROWSE**, or any other page button MPFree knows, goes straight to that page. Turn the data wheel or tap to pick a row, and press **–** or **+** (or tap the arrows) to change it. Or press the data wheel down: the row's value box lights, and turning the wheel changes the value. Press the wheel again to set it, and the wheel goes back to picking rows. On a row that opens something, such as System info or Touch screen, pressing the wheel opens it. The panel on the right explains the row you picked. The list is longer than the screen: it scrolls a row at a time as you pick, and you can drag it up or down with a finger; the thin bar at its right shows where you are.

{{shot: prefs | Prefs, with Pad velocity curve picked.}}

| Row | What it does |
|---|---|
| **Pad velocity curve** | How hard you must hit a pad for a loud note. **Soft:** loud notes come easily. **Hard:** more of the pad's travel stays below the top, for more dynamics. **Linear** plays the pad's own value. FULL LEVEL and Half Level override it, as does the level you set by holding FULL LEVEL and hitting a pad. |
| **Latch keeps pads** | With Latch on, pads keep repeating after you let go. **Replace:** a fresh press starts a new set. **Toggle:** each press adds or removes that pad. **Add:** every press stays until Latch goes. |
| **Sustain pedal latches** | **On:** while the arp plays (Repeat, Arp or Pattern), a keyboard's sustain pedal latches it: hold the pedal and what you play keeps going after you let go, lift it and the notes you aren't holding stop. The pedal then doesn't hold the sound's notes. **Off** (the default): the pedal sustains the sound as always. See [Latch](../making-music/note-repeat-and-arp.md#latching-with-the-sustain-pedal). |
| **Quick advance** | How **Next song** goes while a [set](../making-music/set-lists.md) plays. **Off:** Next song arms the next song and a second press goes, while the transport is stopped too, so nothing changes song by accident. **On:** with the transport stopped, one press goes. |
| **Audio buffer** | How much sound is queued ahead of the speakers. 3 periods is what the factory software uses; 2 plays about 3 ms sooner after a hit, with more risk of clicks. Changing it restarts the sound. When a busy bar is more than MPFree can make in time, it lets the quietest notes go (one already fading out first) rather than click; the Console says how many. |
| **Find the buttons again** | Runs the button guide, which asks you to press MENU, NOTE REPEAT, TAP TEMPO, – and +, REC, OVERDUB, UNDO and PLAY START one at a time, so MPFree learns them. |
| **Show developer tools** | Adds the Dev tab, for people building MPFree, with pages showing what MPFree is doing inside, such as every MIDI message in and out, the processor's load and the Console. |
| **Sound cores** | Which of the MPC's cores make the sound, from the next start (the row says "(next start)" until then). **Shared:** as MPFree always ran. **Core 3:** the sound on a core of its own. **Cores 3 and 2:** some of the drums, the mixer's effects, the tracks' own synths and the Instrument pads' own synths on a second core too, and a reverb on a return there as well when it has a pre-delay and is all wet (it still sounds on time). The tracks' and pads' synths are shared out between the cores by how much each one costs, and sound exactly the same. **Cores 3, 2 and 1:** a test that also puts the synth, and some of the tracks' own synths, on a third core. Core 0 is always left for the MPC's own work and the screen. The Dev tab's CPU by engine page shows how it runs. Shared is the default until the device tests pick one. |
| **Loop memory** | How much sound the looper can hold, all loops and layers together: 1, 2, 5, 10 or 20 minutes of stereo, with the megabytes it takes (5 minutes, about 131 MB with the 60 seconds kept for Capture, is the default). The memory is set aside and kept in RAM when MPFree starts, so a new size applies from the next start; the row says "(next start)" until then. The Looper tab's Loop memory gauge shows how much is used. See [Recording loops](../making-music/looping.md#layers). |
| **Audio track memory** | How much sound the audio tracks on the Song view can hold, every take together: 30 seconds, 1, 2, 5 or 10 minutes of stereo, with the megabytes it takes (1 minute, about 22 MB, is the default). Like Loop memory, it is set aside when MPFree starts, so a new size applies from the next start; the row says "(next start)" until then. The Song view's legend shows how much is free. See [Arranging a song](../making-music/arranging.md#audio-in-the-song). |
| **Map controls** | For developers: lists every button with the number MPFree knows for it (all of them are built in), and can learn a button again if one ever sends another number: pick a control, press **+** and press the button (**–** skips it). It also turns on the developer tools. |
| **Line input** | Turns the INPUTS on the back on or off. It is off every time MPFree starts. |
| **Line input latency** | How late what you play into the INPUTS reaches MPFree, in frames, so the looper and the audio tracks can line a recording up with the beat you played it against. It is estimated from the audio buffer until you set it ("(estimated)"); to measure it, run a cable from an output to the input. It's the same setting as the Looper's **Latency to make up for** (MENU on the Looper tab): change either and both follow. |
| **Send MIDI and CV out** | Off until you turn it on. **On (sends MIDI)**: tracks set to play on the 5-pin jacks play out of **MIDI OUT**, tracks set to play on a USB device play there, clock goes out as each port's **Clock out** says, a port's page can send a test note and All notes off, and the CV/Gate jacks follow the project's CV outputs. **Off**: nothing is sent and every track plays inside MPFree; turning it off ends the notes MPFree has on and sets CV 1 to 4 to 0 V. MPFree remembers it at the next start. These outputs haven't been checked on an MPC One yet. See [Playing other gear](../making-music/midi-and-cv.md). |
| **System info** | MPFree's version, and details of the MPC it runs on. **MENU** goes back to Prefs. |
| **System status** | Opens System status, as the status chip does: whether anything is wrong, which part, and what to try. See [When something goes wrong](when-something-goes-wrong.md). |
| **Appearance** | The theme: MPFree's colours and the style of its panels. **Default**, **Slate** (mid blue-grey, between Default and Light), **Light**, **Console**, **Synthwave**, **Spaceship** or **AbleTown** (mid greys with an orange highlight), then any themes of your own ([Making your own theme](making-a-theme.md)); the screen changes at once, and the layout and the font stay the same. The right panel then asks **Keep Slate?** (or whichever you stepped to) for 15 seconds: press **Keep** or push the data wheel to keep it, or do nothing (or press **Go back**, or close Prefs) to go back to the theme you had. On a theme of your own, the panel also lists any lines MPFree couldn't use, and names theme files it left out. |
| **At start** | What MPFree opens when it starts. **Last project** (the default): the project you last saved or opened, once the sound is going. **New project:** an empty song. After a crash MPFree opens nothing, so a project that crashes it can't do it again. See [Saving your work](../making-music/saving.md#at-start). |
| **Export** | How an export (BROWSE, Projects, **Export**) plays the song. **Fast, quiet** (the default): faster than real time, with the speakers quiet. **Listen:** once in real time, and you hear it as it records. A song that plays outside gear (over MIDI, CV or clock, through the line input, or with plugins) listens anyway. See [Exporting your music](../making-music/exporting.md#fast-or-listen). |
| **Export file** | The WAV files an export makes, as the Export panel starts each time: **24-bit** (the default), **16-bit** (with a little dither noise, for CDs and older gear) or **32-bit float** (exactly what the mixer made, levels over full scale kept). The panel can change it for one export. See [File formats](../making-music/exporting.md#file-formats). |
| **Export rate** | The export's sample rate: **44.1 kHz** (the default, the rate MPFree plays at) or **48 kHz** (for video, converted as the files are written). The panel can change it for one export. |
| **Backup and Akai kits** | Opens two rows for the first start's long steps: the backup of your MPC, and Akai's drum kits and samples. Each shows whether it is done, and can ask for it at the next start ([below](#backup-and-akai-kits)). |
| **Touch screen** | Opens the touchscreen's own rows: **Screen pad velocity** (the velocity of pads played on the screen, which can't sense how hard you tap), **Calibrate touch** (tap a target in each corner to check touches land where you touch; this version only reports the result) and **Touch lab** (a test page for touches). |

## Backup and Akai kits

The first start backs up your MPC and brings in Akai's drum kits and samples, unless you left them out in the USB creator ([Installing MPFree](../getting-started/installing.md#leaving-out-the-backup-or-akais-kits)). These rows show where each stands and let you ask for it later.

{{shot: prefs-setup | Prefs, Backup and Akai kits: a backup made, and Akai's kits asked for at the next start.}}

| Row | What it shows, and what a push does |
|---|---|
| **Backup of your MPC** | **Made** and the day, **Not made yet**, or **Left out**. Press the wheel or **+** to back up your MPC at the next start (**At the next start**, or **New one at the next start** when there is one already; it is kept). Press again, or **–**, to take it back. A backup takes 10 to 20 minutes and about 8 GB on the stick. |
| **Akai's kits and samples** | **In BROWSE** once they are in, otherwise **Not in yet** or **Left out**. Press the wheel or **+** to bring them in at the next start (about 10 minutes); press again, or **–**, to take it back. Without a backup on the stick, they are read straight from the MPC, which is only read, never changed. |

Nothing runs while you play: a line says what happens at the next start, with the stick in. To start it now, press the power button and choose **Restart**. **Back to Prefs** or **MENU** goes back. On a stick the USB creator didn't make, both rows say **Not on this stick**.

> [!NOTE]
> Reading Akai's kits straight from the MPC, without a backup, is new and hasn't been checked on an MPC yet.

Prefs are yours, not the song's: they are kept on the stick and stay the same whichever project you open (see [Saving your work](../making-music/saving.md#your-song-and-your-setup)). The line input is the exception: it is off every time MPFree starts.

> [!MPC]
> On the MPC, **SHIFT + MENU** opens Preferences too, but MPFree's are far fewer: most settings live on the page they belong to, under **MENU**.
