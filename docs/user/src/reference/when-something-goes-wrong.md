<!--
status: changing
covers: rust/mpfree-status/src/kinds.rs rust/mpfree-app/src/status_screen.rs rust/mpfree-ui/src/status_page.rs
checked: 2026-11-22 e7409223
-->

# When something goes wrong

MPFree keeps an eye on itself while you play. When something goes wrong, or is close to it, the **status chip** at the top right of the screen changes colour, and the **System status** page says what happened, what it means for your music and what to try. Nothing ever covers the screen while you play.

## The status chip

The status chip is the chip at the right end of the top bar, beside the folder. It always shows how much of the processor the sound uses and how much memory is in use, **CPU 24 %  RAM 38 %**, and it always keeps the same size, so the chips beside it never move.

| Colour | Means |
|---|---|
| **Green** | Everything is working. |
| **Yellow** | MPFree gave something up to keep playing, is near a limit, or something you use isn't working. Nothing is lost. |
| **Red** | You heard a fault, some work may not be saved, or part of MPFree isn't answering. |

A number past its line takes the colour too: RAM turns yellow over 70 % and red over 85 %, and CPU turns yellow while the sound is near its limit and red while the sound is dropping out. A plain number on a coloured chip means the colour comes from a problem on the System status page.

When a problem is over but you haven't looked at it yet, the chip keeps its colour as an outline (a ring) instead of a fill. Opening System status clears the ring.

The chip never shows the words of a problem. Tap it to see them.

## The System status page

Tap the status chip, or pick **System status** in Prefs, to open the page over the one you are on. Tap the chip again, press **MENU** or tap a tab to close it.

{{shot: system-status | System status: the sound dropped out three times just now (red), Juno-X isn't plugged in (yellow), and some voices were faded out 30 seconds ago.}}

- **The top line** says the worst problem happening now, in its colour, or **Everything is working**, with how many problems there are now and earlier. On its right are the CPU and RAM meters.
- **The systems** are always listed on the left, all nine: Sound, Memory, Storage, Pads and buttons, MIDI and sync, Project, Screen, Plugins and The MPC. Each has a dot in its colour, a few plain words ("41 % used · 1.1 GB free", "Juno-X: not plugged in") and a count of its problems. The page opens on the worst one.
- **The picked system's problems** are on the right, worst and newest first. A problem happening now shows in full: what happened, what to try and, where MPFree can do it for you, a button. One that is over folds to a line with when it happened; tap it to open it. Each kind of problem has one card, with how many times and when ("3 times · last 12 s ago · first 4 min ago").
- **Now | This session**, at the top right, shows only what is happening now, or everything since MPFree started.

Tap a system to pick it. The data wheel, or **–** and **+**, step through the systems, and pressing the wheel opens the next folded card. A card's button only ever does what it says.

{{shot: system-status-good | System status with nothing wrong.}}

## On the hint line

When a red problem starts, its title shows briefly on the hint line at the bottom of the screen, in red: "The sound dropped out. Tap the status chip to see why." The same problem shows there again only after a minute, so it never fills the hint line while you play. Yellow problems never go on the hint line.

{{shot: status-red-hint | The sound dropped out: the status chip turns red and the hint line says so for a few seconds.}}

## What each problem means

Each problem below starts with its colour. A **Note** is listed on the System status page so you know it happened, but it never changes the chip's colour.

### The sound dropped out

**Red.** MPFree couldn't finish the sound in time, so you heard a short gap. **Try:** play fewer sounds at once, or turn off an effect you don't need.

### Part of the sound was skipped

**Red.** One of the MPC's cores was late with its part of the sound, so that part was left out for a moment. **Try:** play fewer sounds at once, or turn off an effect you don't need.

### Some notes didn't play

**Red.** More notes came in at once than the sound could take, so some of them didn't play. **Try:** play fewer notes at once, or slow down a busy arp or Note Repeat.

### A note may be stuck on

**Red.** The end of a note got lost, so a sound may keep playing after you let go. **Try:** press **Silence all notes** on its card (the same as pressing **STOP** twice). If it happens again, it is a bug: please report it.

### Some voices were faded out

**Yellow.** The sound was close to running late, so MPFree faded out a few quiet voices to keep playing. **Try:** play fewer voices at once, or give the pads a shorter release.

### Grains thinned

**Yellow.** The grain pads wanted more grains at once than the sampler plays, so the oldest held pads thinned or went quiet. **Try:** hold fewer grain pads at once, or shorten their grains.

### The sound is near its limit

**Yellow.** The MPC has little time to spare making the sound. If it gets any busier you may hear gaps. **Try:** play fewer sounds at once, or turn off an effect you don't need.

### Memory is running low

**Yellow.** Little memory is left for sounds, so the next sample or kit may not load. **Try:** remove samples you don't use, or save and start a smaller project.

### Something didn't load: not enough memory

**Yellow.** A sample or kit was too big for the memory left, so it wasn't loaded. **Try:** remove samples you don't use, then load it again.

### MPFree can't write to the stick

**Red.** The stick's space for your music is gone or can only be read, so nothing new can be saved or recorded. **Try:** turn the MPC off, check the stick is in firmly, and turn it on again. If it happens again, the stick may be failing.

### A recording isn't being saved

**Red.** The stick isn't taking a take: it may be full, out or failing. The take is kept in memory and MPFree keeps trying, but if the power goes now, everything since it was last saved is lost. **Try:** keep the MPC on, and check the stick is in and has room. The take is saved as soon as the stick works again.

### A recording stopped by itself

**Red.** The stick fell so far behind that the memory kept for a take filled up, so recording stopped there. Everything recorded up to then is kept. **Try:** record fewer tracks at once, or use a faster stick. Check the stick is in and has room.

### The SD card came out while it was playing

**Red.** Sounds were playing from the SD card when it came out, so they stopped. **Try:** put the card back in, then start them again.

### Part of a sound went quiet

**Red.** The stick or SD card didn't keep up with a sound playing from it, so part of the sound played as silence. **Try:** play fewer audio tracks or long samples at once. If it keeps happening, try a faster stick or card.

### Saving didn't work

**Red.** MPFree couldn't write to the stick, so your latest changes aren't saved. **Try:** check the stick is in and has room, then save again.

### A recording may stop soon

**Yellow.** The stick is falling behind, and the memory kept for a take is nearly full. If it fills, recording stops there and what was recorded is kept. **Try:** stop any recording you don't need, or record fewer tracks at once.

### A recording is slow to save

**Yellow.** The stick is slow, so the newest part of a take has waited longer than usual to be saved. Saving still works; only a power cut now would lose that part. **Try:** record fewer tracks at once, or use a faster stick.

### Too much is playing from the stick or card

**Yellow.** More is playing from the stick or SD card than MPFree can safely read ahead, so a slow moment there could make a sound go quiet. **Try:** play fewer audio tracks or long samples at once.

### The stick is nearly full

**Yellow.** Less than 1 GB, or less than 10 minutes of recording, is left on the stick. **Try:** remove takes, samples or projects you don't need, or copy them to a computer.

### A take didn't read back as it was recorded

**Yellow.** MPFree read a finished take back from the stick, and part of it isn't what was recorded. The take is kept so you can listen and decide. **Try:** listen to the take and record it again if it sounds wrong. If it happens again, the stick may be failing.

### A take was finished after a power cut

**Note.** A take was cut off when the power went or MPFree stopped, so MPFree finished it at this start, up to the last moment it had saved. **Try:** listen to its end: if the power went while it was recording, the last few seconds may be missing.

### The pads aren't making sound

**Red.** MPFree's sound stopped answering, so the pads and buttons can't play anything. **Try:** wait a few seconds. If it doesn't come back, restart MPFree.

### The pads started without Akai's setup

**Yellow.** The MPC's pads didn't answer MPFree's setup in time when it started, so MPFree plays without it. The pads and buttons still play. **Try:** keep playing. If a pad, button or knob doesn't answer, restart MPFree.

### The touchscreen stopped answering

**Yellow.** MPFree stopped getting touches from the screen. The pads, buttons, knobs and wheel still work. **Try:** use the buttons and the wheel for now. Restarting MPFree brings touch back.

### A MIDI device isn't plugged in

**Yellow.** The project uses a MIDI device that isn't connected, so nothing is sent to it or heard from it. Its card and its system's line name the device. **Try:** plug it in, or pick another port where the project uses it. The problem ends by itself once the device is back.

### The project opened with parts missing

**Yellow.** Some samples or parts couldn't be found, so they were left out. It saves as a new project, so the original stays as it was. **Try:** press **Show what's missing**, which opens the browser with the missing parts named on its status line, then find or replace them.

### MPFree restarted after a crash

**Red.** MPFree stopped unexpectedly and started again with a new project, so a project that crashes it can't do it again at every start. **Try:** press **Open the last project** to carry on. If you haven't changed the new project, it opens straight away; if you have, the browser opens, so you can save your changes before opening it. If it crashes again, please report it.

### The screen is drawing without the GPU

**Yellow.** The graphics chip didn't start, so the screen is drawn the slow way and may feel less smooth. **Try:** restart MPFree. If it keeps happening, please report it.

### The sound is running on fewer cores

**Yellow.** MPFree couldn't give the sound all the cores it asked for, so it has less room before it runs late. **Try:** restart MPFree. If it keeps happening, please report it.
