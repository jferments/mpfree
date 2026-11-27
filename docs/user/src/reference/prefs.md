<!--
status: changing
covers: rust/mpfree-app/src/app.rs rust/mpfree-app/src/app/prefs_tabs.rs rust/mpfree-app/src/app/customise.rs rust/mpfree-app/src/theme_edit.rs rust/mpfree-app/src/pad_look.rs rust/mpfree-app/src/play_feel.rs rust/mpfree-app/src/prefs_page.rs rust/mpfree-app/src/gui.rs rust/mpfree-app/src/app/saved.rs rust/mpfree-app/src/setup.rs
checked: 2026-11-24 680edcc7
-->

# Prefs

**Prefs** holds MPFree's settings. Press **SHIFT + MENU** to open it, and **MENU** (or **‹ Back**, at the top left) to close it. A page's own button closes it too: **MAIN**, **BROWSE**, or any other page button MPFree knows, goes straight to that page.

The settings are on five tabs down the left: **Playing**, **Sound**, **Appearance**, **Projects** and **System**. Tap one, or hold **SHIFT** and turn the data wheel. Prefs opens on the tab you used last.

Each tab's settings sit on cards, and each setting says in a line what it does. Tap to set one: a button of a choice, a tile, a switch. Drag a slider along, or a knob up or down: each follows your finger from where you touch it, so a tap leaves it as it was. The data wheel picks a setting (an outline goes round it) and **–** or **+** change it. Or press the wheel down: the outline thickens, and turning the wheel changes the setting; press it again to set it, and the wheel goes back to picking. On a setting that opens something, such as **Touch lab**, pressing the wheel opens it. You can also rest a finger on a setting and turn the wheel ([Hold and turn](../getting-started/the-screen.md#hold-and-turn)).

{{shot: prefs | Prefs, on the Playing tab.}}

## Playing

How the pads, the screen, the Q-Links and the data wheel play.

| Setting | What it does |
|---|---|
| **Pad velocity curve** | How hard you must hit a pad for a loud note. **Soft:** loud notes come easily. **Hard:** more of the pad's travel stays below the top, for more dynamics. **Linear** (the default) plays the pad's own value. **Custom:** your own, set with the three knobs under it. The picture shows the curve: how hard you hit, across, and how loud the note plays, up. Hit a pad and the picture shows where that hit landed, with the two numbers ("Last hit  64 in → 84 out"). FULL LEVEL and Half Level override it, as does the level you set by holding FULL LEVEL and hitting a pad. |
| **Shape**, **Softest**, **Loudest** | Your own curve. **Shape** bends it, from -100 (hard: loud notes need hard hits) through 0 (straight) to +100 (soft). **Softest** is how loud the lightest hit plays and **Loudest** the hardest, each 1 to 127, so you can keep every note between, say, 20 and 110. Turning any of them picks Custom. They are kept, faded, while you use another curve. |
| **Pad threshold** | How light a touch still plays a note. At 1 (the default) every hit plays. Raise it, up to 40, and hits softer than that play nothing, so brushing a pad doesn't make a sound. It works on how hard you hit, before the curve. |
| **Latch keeps pads** | With Latch on, pads keep repeating after you let go. **Replace:** a fresh press starts a new set. **Toggle:** each press adds or removes that pad. **Add:** every press stays until Latch goes. |
| **Sustain pedal latches** | **On:** while the arp plays (Repeat, Arp or Pattern), a keyboard's sustain pedal latches it: hold the pedal and what you play keeps going after you let go, lift it and the notes you aren't holding stop. The pedal then doesn't hold the sound's notes. **Off** (the default): the pedal sustains the sound as always. See [Latch](../making-music/note-repeat-and-arp.md#latching-with-the-sustain-pedal). |
| **Screen pad velocity** | The velocity of pads played on the screen, which can't sense how hard you tap: 10 to 127 on the slider. |
| **Q-Link speed**, **Wheel speed** | How far a turn of a Q-Link, or of the data wheel, goes. **Slow:** half as far, for fine changes. **Normal** (the default). **Fast:** twice as far. |
| **Wheel speeds up** | **Off** (the default): every click of the data wheel goes as far. **Some:** a quick spin goes up to three times as far, so you can cross a long list fast and still step one at a time when you turn slowly. **More:** up to six times. |
| **Fine with SHIFT** | **On** (the default): hold SHIFT and turn a Q-Link or the wheel for finer steps. **Off:** SHIFT doesn't change a turn. |

{{shot: prefs-curve | Prefs, Playing: a custom curve, and the last hit on it.}}

## Sound

The audio, the line input, memory and MIDI out.

{{shot: prefs-sound | Prefs, Sound.}}

| Setting | What it does |
|---|---|
| **Audio buffer** | How much sound is queued ahead of the speakers. 3 periods is what the factory software uses; 2 plays about 3 ms sooner after a hit, with more risk of clicks. Changing it restarts the sound. When a busy bar is more than MPFree can make in time, it lets the quietest notes go (one already fading out first) rather than click; the Console says how many. |
| **Sound cores** | Which of the MPC's cores make the sound, from the next start (until then its buttons ring the cores in use now). **Shared:** as MPFree always ran. **Core 3:** the sound on a core of its own. **Cores 3 and 2:** some of the drums, the mixer's effects, the tracks' own synths and the Instrument pads' own synths on a second core too, and a reverb on a return there as well when it has a pre-delay and is all wet (it still sounds on time). The tracks' and pads' synths are shared out between the cores by how much each one costs, and sound exactly the same. **Cores 3, 2 and 1:** a test that also puts the synth, and some of the tracks' own synths, on a third core. Core 0 is always left for the MPC's own work and the screen. The Dev tab's CPU by engine page shows how it runs. Shared is the default until the device tests pick one. |
| **Line input** | Turns the INPUTS on the back on or off. It is off every time MPFree starts. |
| **Coming in now** | While the line input is on, a bar shows how loud what comes into the INPUTS is: green, then amber when it gets loud and red near the top, where it may clip. Turn the line input on to see it. |
| **Line input latency** | How late what you play into the INPUTS reaches MPFree, in frames, so the looper and the audio tracks can line a recording up with the beat you played it against. It is estimated from the audio buffer until you set it ("estimated"); to measure it, run a cable from an output to the input. It's the same setting as the Looper's **Latency to make up for** (MENU on the Looper tab): change either and both follow. Drag the knob up or down; the wheel or **–** / **+** move it 16 frames at a time. |
| **Loop memory** | How much sound the looper can hold, all loops and layers together: 1, 2, 5, 10 or 20 minutes of stereo, with the megabytes it takes (5 minutes, about 131 MB with the 60 seconds kept for Capture, is the default). The memory is set aside and kept in RAM when MPFree starts, so a new size applies from the next start; the value says "next start" until then. The Looper tab's Loop memory gauge shows how much is used. See [Recording loops](../making-music/looping.md#layers). |
| **Audio track memory** | How much sound the audio tracks on the Song view can hold, every take together: 30 seconds, 1, 2, 5 or 10 minutes of stereo, with the megabytes it takes (1 minute, about 22 MB, is the default). Like Loop memory, it is set aside when MPFree starts, so a new size applies from the next start; the value says "next start" until then. The Song view's legend shows how much is free. See [Arranging a song](../making-music/arranging.md#audio-in-the-song). The bar under the two shows how much memory they set aside together. |
| **Send MIDI and CV out** | Off until you switch it on. **On**: tracks set to play on the 5-pin jacks play out of **MIDI OUT**, tracks set to play on a USB device play there, clock goes out as each port's **Clock out** says, a port's page can send a test note and All notes off, and the CV/Gate jacks follow the project's CV outputs. **Off**: nothing is sent and every track plays inside MPFree; turning it off ends the notes MPFree has on and sets CV 1 to 4 to 0 V. MPFree remembers it at the next start. These outputs haven't been checked on an MPC One yet. See [Playing other gear](../making-music/midi-and-cv.md). |
| **Audition level** | How loud BROWSE plays a sound you pick to listen to, from -40 to +6 dB (-6 is the default). BROWSE's own level control starts from it. |
| **Metronome sound** | What the metronome sounds like. **Click** (the default), **Wood** (a short knock, like a wood block) or **Beep** (a short held tone). Its level is in Play's options. |

## Appearance

The theme, MPFree's colours and the style of its panels, and how the pads light. Appearance has three views, listed under it on the left while it is open: **Themes**, to pick a theme, **Customise**, to change one, and **Screen and lights**, for the pads' lights and the help line. The layout and the font are the same in every theme.

### Themes

Each theme is a tile that shows the Play page in its colours: **Default**, **Slate** (mid blue-grey, between Default and Light), **Light**, **Console**, **Synthwave**, **Spaceship** and **AbleTown** (mid greys with an orange highlight), then any themes of your own ([Making your own theme](making-a-theme.md)). The theme in use has an outline and a check.

Tap another theme to try it, or turn the data wheel. The screen changes at once. The **In use** card at the bottom then asks **Keep Console?** (or whichever you tried) for 15 seconds: tap **Keep**, or push the data wheel, to keep it. Do nothing, tap **Go back**, pick another tab or close Prefs, and the theme you had comes back.

The In use card also says how well the theme's words, its dim words and its accent read against the page. On a theme of your own it lists any lines MPFree couldn't use, and names theme files it left out.

To change a theme, tap **Customise ›** on the In use card, or push the data wheel. The dashed **New theme** tile after the themes does the same; while you are trying a theme, it keeps that one and opens a copy of it.

{{shot: prefs-appearance | Prefs, Appearance: Console tried, and asking whether to keep it.}}

### Customise

Customise works on a copy of the theme in use, called **My theme** (or, for a theme of your own, its name and 2). Every change shows on the whole screen at once, so you see it on the page you are using. Leave Customise and the theme you had comes back; your changes wait there until you open Customise again, or save them.

{{shot: prefs-customise | Prefs, Customise: the second accent turned, not saved yet.}}

| Setting | What it does |
|---|---|
| **Rename** | Opens the letters pad to name your theme (up to 32 letters). |
| **Undo changes** | Puts the copy back as it was when you opened Customise. |
| **Save theme** | Saves your theme as a file in the themes folder (`files/themes/` on the MPFREE-DATA part of your stick) and makes it the theme in use. Its tile joins the others on Themes. Save again to update the same file. The file holds only what you changed, so you can copy it to another stick or open it on a computer ([Making your own theme](making-a-theme.md)). |
| **Accent**, **Second accent** | The highlight colours, round the colour ring. The words on the accent stay readable. |
| **Page lightness** | How light the page is; the panels, buttons and lines move with it. |
| **Warmth** | From cool blue-greys to warm brown-greys. |
| **Every colour** | Tap a colour (the page, panels, buttons, lines, words, dim words, and the good, warning and bad colours) to set it exactly: the four knobs above become **Hue**, **Colour** (how strong) and **Light** for it. Tap **Done**, or the colour again, to get the four knobs back. |
| **Mode** | **Dark** or **Light** turns every grey over and keeps the words readable. **Auto** leaves the colours as they are. |
| **Panels stand out** | How much lighter (or darker) the panels and buttons are than the page. |
| **Corners**, **Corner size** | Round, cut or square corners, and how big they are. |
| **Outlines** | How strong the lines round panels and buttons are. |
| **Glow**, **Text glow**, **Tint pads** | A soft halo round outlines and round the words, and how far the kit and pad colours on the screen lean towards the accent (the pads' own lights don't change). |
| **Backdrop**, **Pattern size** | What is drawn behind the panels: a solid page, a fade, a grid, dots or lines, and how far apart the pattern's lines are. |
| **Backdrop colours** | The fade's second colour and the pattern's lines. Tap one to set it, as with Every colour. |
| **Track colours** | The eight track colours; the Q-Links, graphs and EQ bands follow them. Tap one to set it, as with Every colour. **Set** picks a whole set: the theme's own, Rainbow, Pastel, Neon, Earth, Ocean, or shades of the accent. |

The line at the foot says how well the words, the dim words and the accent read on a panel as you go. MPFree won't make a change that leaves the words too hard to read: the line then says **Stopped there**.

{{shot: prefs-customise-colour | Prefs, Customise: the words' colour on the knobs.}}

### Screen and lights

How the 16 pads on the MPC light, and the help line along the foot of the screen. Every change shows at once.

{{shot: prefs-lights | Prefs, Screen and lights: Hit heat, with pad 6 held.}}

| Setting | What it does |
|---|---|
| **Brightness** | How bright every pad light is, from 10 to 100 % (the default). It dims the pads at rest, held, on the sequencer's steps and on the mute pages alike; a lit pad never goes fully dark. |
| **A held pad** | What a pad shows while you hold it down. **Brighter** (the default): its own colour at full strength. **White.** **Same:** no change. |
| **Empty pads** | What a pad with no sound on it shows. **Dark** (the default), or **Dim**: a faint grey, so you can see the grid. |
| **Hints** | How much the line at the foot of the page says. **Persistent** (the default): what the controls do there, always. **Contextual**: on a page's first three visits, until you first touch the screen (MPFree remembers which pages you know). **On demand**: only while you hold **SHIFT** for a second (a quick SHIFT shows SHIFT's own keys). **Off**: never the page's help. In every one, the line still says what a held button does (such as MAIN, FULL LEVEL, COPY or SHIFT), what MPFree tells you (such as **Saved**), the Looper's questions (its buttons sit at the end of the line), and the keys of the letters pad, the power card and Eject card. Away from Persistent the Play page has no line, and its throws and XY pad get the room: see [The hint line](../getting-started/the-screen.md#the-hint-line). |
| **Pads show** | The colour a pad with a sound rests in. **Kit colour** (the default): each kit's own. **Track:** the colour of the armed track, in the theme in use. **Accent:** the theme's accent. **Hit heat:** how hard you hit the pad last, from blue for soft, through green and amber, to red for hard (a pad you haven't hit yet shows the kit's colour). The pads drawn above show how they'd light with your settings, with pad 6 held. The mute pages keep their own colours. |

> [!NOTE]
> Most of these colours (dimmed pads, white, the track and accent colours, the heat colours and the dim grey) haven't been checked on an MPC yet.

## Projects

What opens at start, how exports come out, and sets.

{{shot: prefs-projects | Prefs, Projects.}}

| Setting | What it does |
|---|---|
| **Export** | How an export (BROWSE, Projects, **Export**) plays the song. **Fast, quiet** (the default): faster than real time, with the speakers quiet. **Listen:** once in real time, and you hear it as it records. A song that plays outside gear (over MIDI, CV or clock, through the line input, or with plugins) listens anyway. See [Exporting your music](../making-music/exporting.md#fast-or-listen). |
| **Export file** | The WAV files an export makes, as the Export panel starts each time: **24-bit** (the default), **16-bit** (with a little dither noise, for CDs and older gear) or **32-bit float** (exactly what the mixer made, levels over full scale kept). The panel can change it for one export. See [File formats](../making-music/exporting.md#file-formats). |
| **Export rate** | The export's sample rate: **44.1 kHz** (the default, the rate MPFree plays at) or **48 kHz** (for video, converted as the files are written). The panel can change it for one export. |
| **Stems by** | When you export stems, whether you get one file for each mixer **Channel** (the default) or for each **Track**. The Export panel starts from it each time; its **Stems by** row can change it for one export. See [Stems by track or by channel](../making-music/exporting.md#stems-by-track-or-by-channel). |
| **At start** | What MPFree opens when it starts. **Last project** (the default): the project you last saved or opened, once the sound is going. **New project:** an empty song. After a crash MPFree opens nothing, so a project that crashes it can't do it again. See [Saving your work](../making-music/saving.md#at-start). |
| **Quick advance** | How **Next song** goes while a [set](../making-music/set-lists.md) plays. **Off:** Next song arms the next song and a second press goes, while the transport is stopped too, so nothing changes song by accident. **On:** with the transport stopped, one press goes. |

## System

How the MPC is doing, the first start's backup and kits, checks for the buttons and the touch screen, and developer tools.

{{shot: prefs-setup | Prefs, System: a backup made, and Akai's kits asked for at the next start.}}

| Setting | What it does |
|---|---|
| **System status** | Says whether anything needs you, as the status chip does. **System status ›** opens System status: which part, and what to try. See [When something goes wrong](when-something-goes-wrong.md). |
| **This MPC's details** | MPFree's version, the operating system, the kernel and the MPC's hardware, listed under System status. **All the details** opens System info, with every row; **MENU** goes back to Prefs. |
| **Backup of your MPC** | Whether the first start's backup of your MPC is made, and a button to ask for one at the next start ([below](#backup-and-akai-kits)). |
| **Akai's kits and samples** | Whether Akai's drum kits and samples are in, and a button to bring them in at the next start ([below](#backup-and-akai-kits)). |
| **Find the buttons again** | **Start** runs the button guide, which asks you to press MENU, NOTE REPEAT, TAP TEMPO, – and +, REC, OVERDUB, UNDO and PLAY START one at a time, so MPFree learns them. |
| **Calibrate touch** | **Start**: tap a target in each corner to check touches land where you touch. This version only reports the result. |
| **Touch lab** | **Open**: a test page for touches, with timed steps for taps, drags and swipes. |
| **Map controls** | For developers: lists every button with the number MPFree knows for it (all of them are built in), and can learn a button again if one ever sends another number: tap **Open**, pick a control, press **+** and press the button (**–** skips it). It also turns on the developer tools. |
| **Show developer tools** | Adds the Dev tab, for people building MPFree, with pages showing what MPFree is doing inside, such as every MIDI message in and out, the processor's load and the Console. |

### Backup and Akai kits

The first start backs up your MPC and brings in Akai's drum kits and samples, unless you left them out in the USB creator ([Installing MPFree](../getting-started/installing.md#leaving-out-the-backup-or-akais-kits)). The System tab's **Backup and Akai kits** card shows where each stands and lets you ask for it later.

| Setting | What it shows, and what its button does |
|---|---|
| **Backup of your MPC** | **Made** and the day, **Not made yet**, or **Left out**. **At next start** (or the wheel's push, or **+**) backs up your MPC at the next start (it then says **At the next start**, or **New one at the next start** when there is one already; it is kept). **Take it back**, or **–**, takes the ask back. A backup takes 10 to 20 minutes and about 8 GB on the stick. |
| **Akai's kits and samples** | **In BROWSE** once they are in, otherwise **Not in yet** or **Left out**. **Bring them in** (or the wheel's push, or **+**) brings them in at the next start (about 10 minutes); **Take it back**, or **–**, takes the ask back. Without a backup on the stick, they are read straight from the MPC, which is only read, never changed. |

Nothing runs while you play: a line says what happens at the next start, with the stick in. To start it now, press the power button and choose **Restart**. On a stick the USB creator didn't make, both say **Not on this stick**, and their buttons are faded.

> [!NOTE]
> Reading Akai's kits straight from the MPC, without a backup, is new and hasn't been checked on an MPC yet.

## Putting a tab back

Playing, Sound and Projects each have a **Reset** button at the foot of the list on the left (**Reset Playing**, and so on). Tap it and it asks **Tap again to reset**; tap again and that tab's settings go back to how MPFree came. Change a setting or pick another tab instead, and nothing is reset. Appearance and System have no Reset: to undo a theme, pick the one you had.

Prefs are yours, not the song's: they are kept on the stick and stay the same whichever project you open (see [Saving your work](../making-music/saving.md#your-song-and-your-setup)). The line input is the exception: it is off every time MPFree starts.

> [!MPC]
> On the MPC, **SHIFT + MENU** opens Preferences too, but MPFree's are far fewer: most settings live on the page they belong to, under **MENU**.
