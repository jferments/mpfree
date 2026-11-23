<!--
status: changing
covers: rust/mpfree-synth rust/mpfree-synth-ui rust/mpfree-scale rust/mpfree-app/src/synth.rs rust/mpfree-app/src/app/synth_pads.rs rust/mpfree-app/src/app/pads_focus.rs rust/mpfree-app/src/instruments.rs rust/mpfree-mpchost rust/mpfree-mpcskin rust/mpfree-app/src/skin_page.rs
checked: 2026-11-22 e5518efc
-->

# Synths

MPFree has twelve synths built in. Put one on the pads and they become a keyboard: bass lines, chords, leads and pads, played by hand, through the arpeggiator or into a pattern.

## Put a synth on the pads

Each track plays a kit or a synth, and the pads play what the selected track plays. There are two ways to put a synth on a track:

- Tap **PADS PLAY** on Play's transport bar (or **Kit** on the Pads tab) and pick the last row of the list of kits, **Synth: Poly** (or the synth it had before tracks got their own). On the Play page, holding **SHIFT** and turning the data wheel past the last kit does the same. The selected track plays the synth from then on: the pads play it whenever that track is selected, and so do the track's clips. The other tracks keep what they play.
- Give a track its own synth: see [A synth for each track](#a-synth-for-each-track).

Opening the **Synth** tab changes nothing that plays: it shows the synth, and if the selected track plays a kit, the line at the bottom of the screen says how to put the synth on it. To go back to drums, tap the chip (it names the synth now) and pick a kit, or set the track's instrument back to **Kit**. While the selected track plays the synth, **PAD BANK** changes the pads' layout and **SHIFT** + the wheel their octave, not the kit.

### Try a synth on the pads

To hear a synth without changing any track, open the **Synth** tab while the selected track plays a kit and tap **Play Poly on the pads**, the button under the synth's name (it names the synth the tab shows). The pads play the synth at once, and the track keeps its kit:

- **Nothing changes on the track.** Its clips still play the kit, and it still plays the kit when you come back to it. The synth plays on its own **Synth** channel in the mixer.
- **The pads keep the synth, whichever track you select.** Select another track, arm one, or tap a track in the REC chip's list: the pads go on playing the synth, and every track keeps what it plays. **Pads: Poly** at the top of the screen says so on every page, and Play's transport bar reads **Pads: Poly**.
- **Take your time.** Pick presets, turn the knobs and play over your drums until it sounds right.
- **Follow track** gives the pads back to the selected track: the button on the Synth tab (it reads **Follow track** now), or tap **Pads: Poly** at the top of the screen and then **Follow track** in **What the pads play** (below).
- **Picking a kit for the track leaves the pads alone.** The track plays the kit you picked, and the pads go on playing the synth until you tap **Follow track**. Nothing you didn't choose for the pads changes what they play. While the pads follow the track, they play its new kit.
- **The project remembers it.** Save with the synth on the pads and the project opens with it there again. A new project starts with the pads following the track.
- **REC records the synth, not the drums.** Press **REC** (or **OVERDUB**) and the synth goes onto a free track, as if you had picked **Synth** in the list of kits for it, and the take goes there. A free track is one you haven't set up: no clips, no kit or MIDI output picked, no mixer settings, no settings of its own, and still its first name (**Track 3** for the third track: renamed, even to another "Track" name, it counts as set up). The selected track is used if it is free (never Track 1, the drums track); otherwise the first free track, which becomes the selected track; and if none is free, a new track is added for it (**UNDO** takes it away again). So your drums and anything you set up never change. It is the same synth, not a copy: the track plays exactly the sound you shaped, and changing that sound later changes the track too. The REC chip on the transport bar names where the synth will go before you press: **REC → Track 2 · Poly**, **REC → New track · Poly** when a track will be added for it, or **REC → No free track · Poly** when the project holds no more. The line at the bottom of the screen says where it went. The pads keep the synth, and **REC** again records on that same track. With the loop armed, REC records the loop with the synth as you hear it.
- **A track you chose stays chosen.** If you pinned REC to a track (**Pin the arm** in the REC chip's list), or tapped one in that list, and that track isn't free, the REC chip says so before you press (**REC · PINNED → Track 1 · not free**), and REC records nothing and says why: unpin it, pick a free track, or tap **Follow track** to record the track's own kit.
- **The arp plays the synth too.** Hold **NOTE REPEAT**, turn on Latch or open the Arp panel and the repeats and arpeggios play the synth on the pads, with the selected track's arp settings; the pads keep the synth. See [When the pads play something of their own](note-repeat-and-arp.md#when-the-pads-play-something-of-their-own).

To keep the synth without recording, put it on a track: pick **Synth** in the list of kits, or give the track its own synth (below).

### Pick any synth for the pads

Tap the synth's card at the top left of the **Synth** tab (its name, the track and Native or Plugin) to open the list of synths, **Pick what the pads play**. It works whatever the selected track plays. **Synths** and **Kits** at the top switch between the synths and the kits (see [Put a kit on the pads](#put-a-kit-on-the-pads)).

- **A tap plays it on the pads at once.** Tap **FM** and play the pads: you hear FM, and no track changes. Tap another synth to try that one instead. Each tile says what the synth is good for, and the one on the pads is outlined and marked **ON THE PADS**.
- **Keep** (it names the synth: **Keep FM**) closes the list and the pads keep the synth. Tapping the card again, or going to another tab, keeps it too. **Cancel** gives the pads back what they played when you opened the list. The first tile, **Track 1's** (it names the selected track), gives the pads back to the track.
- **It has its own mixer channel**, named for who plays it: **FM · Pads**.
- **The pads keep it whichever track you select**, as with Play Poly on the pads: **Pads: FM** at the top of the screen says so. **Follow track** under the card gives the pads back to the track; tapping **Pads: FM** shows everything the pads play, with **Follow track** in its first row (see [What the pads play](#what-the-pads-play)).
- **The Synth tab shows it** while the pads play it, so you can pick its presets and turn its knobs. The line under its name reads **Pads, no track**.
- **The line at the bottom of the Synth tab says who plays it** once more than the pads do: **FM: the pads, Track 2**. It works the same for any synth on the tab, such as **Poly: the pads, Bass, Keys**. When only one bank of the pads plays it ("Bank B only" in the list), it names the bank: **FM: pads bank B, Track 2**.
- **A keyboard plays it too**, each key its own note (see [Playing MPFree from a keyboard](midi-and-cv.md#playing-mpfree-from-a-keyboard)).
- **REC records it on a free track**, as above: the REC chip names the track first (**REC → Track 2 · FM**). That track then plays the same FM, heard on the FM's channel, and its parts are notes.
- **A synth you try and leave goes away** once nothing plays it (not the pads, not a track), and the line at the bottom of the screen says so.
- **Up to 16 synths at once.** The synths picked for the pads and the tracks' own synths count together; with 16 loaded, a tap says there is no room.
- **One bank only.** On a kit, the row at the top of the list offers **All banks** and **Bank B only** (it names the bank the pads are on). With **Bank B only**, the synth you tap plays on bank B alone: PAD BANK A gives the pads the track's kit again, and PAD BANK B gives them the synth. The top of the screen reads **Pads B: FM** while you are on that bank. The first tile, **Bank B follows the track**, gives that bank back to the track. **All banks** puts the next synth on every bank again. **Follow track** gives every bank back to the track.
- **The project remembers it.** Saved with FM on the pads, or on bank B, the project opens with FM there again.
- **A synth that is gone keeps its place.** If a synth the pads or a bank point at is no longer in the project (a track's own synth you took off, say), the line at the bottom of the screen says so once, and that place waits for you: the bank plays what the other banks play, or the pads play the selected track's instrument, until you pick another synth there or tap **Follow track**. The project keeps the place too, and opening it says which synth is missing.
- **The arp plays it too.** With NOTE REPEAT held, Latch on or the Arp panel open, the repeats and arpeggios play the synth you picked, and a tap in the list works as usual.

### Put a kit on the pads

The same list puts a whole kit on the pads, with no track changed: tap **Kits** at the top of it. It opens on **Kits** by itself while the pads play a kit.

- **A tile for each kit**, in its colour: its number and name, how many pads it fills (and which banks, if more than one), whether it has keygroups, and the first track that plays it (**2 · Boom Bap**, **16 pads · Track 1**). The kits you haven't filled come last, marked **Empty**.
- **A tap plays it on the pads at once**, on its own, while the selected track keeps its kit. **Keep** names it (**Keep Boom Bap**), the top of the screen reads **Pads: Boom Bap** on every page, and REC records it on a free track. The first tile, **Track 1's kit**, gives the pads back to the track.
- **An empty kit goes on the pads too.** They play nothing, and the line at the bottom says **Kit 9 is empty: load sounds on the Sample page**; sounds you load or record there land on that kit.
- **One bank only.** With **Bank B only**, the kit plays on bank B alone. If the kit fills more than one bank, a row above the tiles picks which of its banks bank B plays (**Bank B plays Big Kit's bank A B C**); **PAD BANK** still picks the bank you play. The first tile, **Follow base**, gives bank B back to what every bank plays.
- **What your keys play on it.** For a kit with keygroups, the row above the tiles sets what a keyboard's keys play on it: **Pads** (each key its pad), **Keygroups** (the keygroups that hold the key) or **Both** (keygroups where one holds the key, the pad elsewhere). It is the kit's own setting, and UNDO on the Sample page takes it back.
- **Layer** works with kits too: with it on, a tap adds the kit to what the pads play, or takes it off.

### What the pads play

Tap the **Pads: ...** chip at the top of the screen, on any page, to see everything the pads play at once. The Synth tab opens with **What the pads play** over it. Only what you pick here or in the list, **Follow**, UNDO, or opening a project changes what the pads play.

- **The base** is what every bank plays: **Follow track** (the selected track's instrument, such as **Track 1's kit: Boom Bap**), or a kit or synth you picked. **Follow track** gives every bank and every layer back to the track. **Pick** opens the list of synths and kits for every bank.
- **Each bank, A to H**, has a row. **The base**, faint, means the bank plays the base. A bank with its own kit or synth names it, with the bank of the kit it plays (**Big Kit**, **From its bank C**); **Follow base** gives the bank back to the base. The bank the pads are on is outlined. **Tap a bank's row** to pick its kit or synth: the list opens for that bank alone, whichever bank you are on.
- **Layers** are listed on the right, up to four with the base (**LAYERS · 2 OF 4**). The first button says where a layer plays: **Every bank**, or one bank (**Bank D**). Tap it to move the layer between every bank and the bank the pads are on. **Remove** takes it off, and **+ Add a layer** opens the list with Layer on.
- **UNDO takes back the last change** while this is open: each pick you keep, each **Follow** and each **Remove** is one step. The box under the layers says what UNDO will take back (**Last change here: bank B got Big Kit, from its bank C.**). There is no redo here: pick it again. Opening a project starts the history again.
- **Something missing** shows in the warning colour: a project that names a kit or synth this one doesn't have (**Missing: instrument 9**). That bank plays the base meanwhile, the base plays the track's instrument, and a layer is silent. **Replace** picks another, and **Remove** takes it away.
- **The list comes back here.** Keep or Cancel in a list opened from this panel brings you back to it.
- **Done**, the chip again, or another tab closes it.

### Recording every bank

When a bank has a synth or kit of its own, REC records everything the pads play, whichever bank you play it on, each sound on its own track, in one take.

- **The selected track stays armed**, so the banks that follow it go on playing its kit, and their hits record there. The REC chip names it and counts the rest: **REC → Track 1 + 1 more**.
- **Each bank's own sound gets a track**, and so does what every other bank plays when you chose a synth or kit for the pads. An armed track that already plays it keeps it. Otherwise it goes on a free track, or a new one at the end, pointed at that same synth or kit and armed with the first.
- **Every hit goes into the track of the sound it plays.** Bank B's hits go into bank B's synth's track, and the arp's notes too, as you hear them. A key a keyboard plays on the pads' synth goes in as the pad that plays its note there.
- **One UNDO takes the whole take back**, and the next REC records on the same tracks with nothing more to set up.
- **REC records nothing when a sound has no track.** When the project holds no more tracks, or more than 16 would record at once, the REC chip says so before you press (**REC → No free track · Bank B's FM**), and the line names the sound.

### Layers: more than one synth on the pads

The pads can play up to four synths at once, such as a soft pad sound and a bass under it. Each hit plays every one of them at the same velocity, and so does each key of a keyboard.

1. Open the list of synths (tap the instrument card).
2. Tap **Layer** at the top of the list, beside Cancel, so it lights up.
3. Tap the synths you want: each tap adds one to what the pads play, outlined and tagged **ON THE PADS**. Tap one again to take it off.
4. Tap **Keep layers**.

- **The top of the screen names them all**: **Pads: FM + Pluck**.
- **Each synth plays on its own mixer channel** ("FM · Pads", "Pluck · Pads"), so you can turn one down, mute it or solo it in the Mix tab.
- **The first tile, Track 1's**, adds the selected track's own synth, or the shared Poly when the track plays it. A kit can't be a layer: if the pads follow a track that plays a kit, the first synth you tap is what they play, and the next one is layered on it.
- **Taking off the first** makes the next one the first. The last one stays: **Follow track** gives the pads back to the track, and the layers go with it.
- **With Layer off**, a tap plays that one synth alone again, and the layers go.
- **Layers are for every bank.** **Layer** and **Bank B only** don't go together: picking one turns the other off. A bank with a synth of its own plays that synth alone.
- **With the arp on**, each layer plays a hit once, as you press it, and the repeats and arpeggios play the first synth. You can add and take off layers while it plays.
- **The project remembers them.** A layer whose synth is no longer in the project stays silent in its place, and the others play.
- **Recording.** REC records each layer on a track of its own, in one take. The REC chip names where the first goes and counts the rest: **REC → Track 2 · FM + 1 more**. When you press REC, the first synth goes on a free track as above, and each layer gets a free track too (or a new one at the end), pointed at that same synth and armed with the first, as SHIFT + a tap in REC's list arms a track. A track you armed that already plays a layer's synth keeps it. Every hit, and every key a pad plays, goes into each track, and one UNDO takes the whole take back. The next REC records on the same tracks, with nothing more to set up.
  - A new track that REC added for a layer goes away with its own UNDO, after the take's.
  - When the project holds no more tracks, or 16 tracks would record at once, REC records nothing. The REC chip says so before you press (**REC → No free track · Pluck**), and the line names the layer that has no track.
  - A keyboard's key goes into a layer's track as the pad that plays its note on that synth; a note no pad plays alone sounds but isn't kept, as on a track a keyboard is routed to.
  - A keyboard routed to a synth no track plays (see [Playing several tracks from your keyboards](midi-and-cv.md#playing-several-tracks-from-your-keyboards)) records the same way: its synth gets a track of its own in the same take, counted on the chip.
- **Looping layers.** To loop all the layers with the Looper, record from a mix that has them all (the Looper tab's **Records from**), such as Master: **Records from: Pads** takes the first synth's channel.

## The twelve synths

Any of the twelve can be a track's own instrument (see [A synth for each track](#a-synth-for-each-track)), and any track can have a different one.

| Synth | What it is | Good for | Presets |
|---|---|---|---|
| **Poly** | A classic two-oscillator subtractive synth, with a sub oscillator, noise, unison, a seven-saw supersaw, tables of waves to sweep through and a resonant filter | Basses, leads, pads, brass, big trance-style stacks, evolving pads, vocal leads, growling basses | Grit Bass, Acid, Supersaw, Dust Pad, Pluck, Brass, Sub Bass, Lead, Singing Lead, Drift Pad, Super Lead, Super Pad, Super Stab, Super Stack, Worn Brass, Morph Pad, Vowel Lead, Sync Sweep, Digital Bell, Growl Bass, Organ, Harmonic Pluck, FM Keys, Portamento Lead, Low Note Lead, Breath Lead, Slide Lead, Unison Lead |
| **FM** | Six-operator FM, in the tradition of the 1980s digital synths | Electric pianos, bells, glassy keys, punchy basses, reedy leads | Glass Keys, Bell Tower, Rubber Bass, Drawbars, Brass Section, Log Drum, Silk Pad, Steel Pluck, Reed Lead |
| **Drums** | A drum synth: a different synthesised drum on each pad, each with its own knobs, from the 808 kick and hats to toms, congas and bells | Electronic kits built from scratch, in the styles of the classic drum machines and of today's genres | 808 Kit, Club Machine Kit, Bright Machine Kit, Vintage Rhythm Kit, Eighties Punch Kit, Miami Bass Kit, Trap Kit, Drill Kit, Boom Bap Kit, Lo-fi Kit, House Kit, Deep House Kit, Techno Kit, Minimal Kit, Hardstyle Kit, Drum and Bass Kit, Amapiano Kit, Latin Percussion Kit, Reggaeton Kit, Synth-pop Kit, Industrial Kit, FM Kit, Boom Kit, Tight Kit, Grit Kit, Electro Kit (see [The drum kits](#the-drum-kits)) |
| **Pluck** | Physically modelled strings and mallets | Guitars, harps, plucked basses, marimbas, bells | Nylon, Steel String, Harp, Koto, Muted Bass, Oud, Banjo, Upright Bass, Harpsichord, Pizzicato, Marimba, Vibes, Bell, Glass, Kalimba, Steel Drum, Glockenspiel, Xylophone, Celesta, Chimes, Music Box |
| **Keys** | A physically modelled electric piano and clav: struck steel tines or reeds in front of their pickups, or a struck string over two pickups | Soul and jazz chords, ballads, barking stabs, tremolo keys, funk clav lines | Soft Tines, Bright Tines, Barking Tines, Bell Keys, Stage Tines, Lo-fi Tines, Phase Tines, Reed, Reed Bite, Dirty Reed, Clav, Funk Clav, Wah Clav, Muted Clav, Stab Clav and 18 more |
| **Drawbar** | A tonewheel organ with drawbars, percussion, scanner vibrato and a rotating speaker, and a 1960s combo organ | Jazz and gospel comping, rock riffs, church chords, soft pads; garage, ska and new wave stabs | Jazz Perc, Gospel, Full Organ, Rock Drive, Ballad, Flute, Church, Reedy, Bright Perc, Hollow, Soft Bass, Combo, Combo Flute, Combo Reed, Combo Square, Combo Vibrato, Combo Garage, Combo Rotary, Combo Perc |
| **Squelch** | A mono acid bass: one oscillator into a squelchy resonant filter, with accent and slide | Acid lines, rubbery basses, screaming leads | Acid Line, Deep Bass, Square Acid, Distorted, Rubber, Plucky, Acid Lead, Screamer |
| **Analog** | A vintage analog poly: voices that each sound a little different, poly-mod and a chorus | Brass, warm pads, string ensembles, sync leads, clangy bells | Poly Brass, Warm Pad, Sync Lead, Chorus Strings, Square Keys, Clang Bell, Unison Bass, Slow Sweep, Hollow Pulse, Worn Cards, Solo Lead, Glide Lead, Fat Lead |
| **Strings** | A string machine, like the string ensembles of the 1970s: every key you hold sounds, through violin, viola, cello, contrabass, horn and trumpet sounds, with its own swirling ensemble | Lush string pads and chords, disco and synth-pop strings, slow swells, brassy stabs | Classic Strings, Bright Strings, Cello and Viola, Full Ensemble, Orchestra, Brass Ensemble, Slow Swell, Staccato Strings, Swirl Strings, Dark Strings, High Strings, Low Strings, Paraphonic 70s, Disco Strings, Ballad Pad, Lo-fi Strings, Wide Strings, Mono Ensemble, Dry Divider, Bowed, Horn Section, Brassy Lead, Vox Strings, Hall Strings, Space Strings |
| **Bass** | A bass synth that plays one note at a time, with three bodies: a synth bass with a sub locked under it, an 808 that drops onto the note, and a reese of two oscillators beating against each other | Synth and funk bass lines, 808s for trap and hip-hop, drum and bass reeses, gliding lines | Round Mono, Pluck Bass, Resonant Sweep, Square Sub, Rubber Bass, Funk Bass, Synth Pop, Power Bass, Fat Saw, Thin Pulse, Soft Triangle, Noisy Bass, Glide Bass, Low Note Lines, Growl Press, Press Wah, Sub Sine, Sub Triangle, 808, Long 808, Distorted 808, 808 Slide, 808 Kick Bass, Short 808, 808 Vibrato, Reese, Dark Reese, Wide Detune, Soft Reese, Growl Reese, Reese Sweep, Amp Bass, Tube 808, Rate Glide Bass, High Note Lead |
| **Wind** | A blown instrument played by the breath: a clarinet, a sax or a flute, each a tube that a reed or a stream of air sets ringing | Sax and clarinet lines, breathy flute melodies, soft reed and flute sections, playing from a breath controller | Warm Clarinet, Jazz Clarinet, Dark Clarinet, Tenor Sax, Breathy Sax, Growl Sax, Soprano Sax, Concert Flute, Breathy Flute, Pan Pipes, Recorder, Ocarina, Bamboo Flute, Reed Section, Flute Choir, Wind Controller Sax, Pressure Swell, Hall Flute, Smoky Sax |
| **Choir** | A choir singing "ooh", "oh", "aah", "eh" or "ee": up to eight singers on every note, each a little apart in pitch and with their own vibrato, and the vowel can move while a note sounds | Choir pads and swells, film and gospel chords, vowel pads, choir stabs, a solo voice | Aah Choir, Ooh Choir, Oh Choir, Eh Pad, Soft Oohs, Bright Aahs, Men's Choir, Sopranos, Straight Trebles, Low Chant, Choir Swell, Vowel Pad, Wah Voices, Ooh to Aah, Aah to Ee, Pressure Vowels, Choir Stab, Whisper Choir, Big Choir, Dry Choir, Solo Voice, Vowel Lead, Hall Choir, Space Choir |

Each synth also has an **Init** preset (Init Kit on Drums): a plain starting point to build your own sound.

A few presets come with **effects and macros of their own**: Phase Tines and Wah Clav on Keys, Hall Strings and Space Strings on Strings, Amp Bass and Tube 808 on Bass, Hall Flute and Smoky Sax on Wind, and Hall Choir and Space Choir on Choir. Eight drum kits bring effects for the whole kit: tape on Vintage Rhythm, tape and a compressor on Boom Bap, fewer bits and worn tape on Lo-fi, fewer bits on Industrial, and a compressor on House, Techno, Hardstyle and Drum and Bass. See [Presets with their own effects](#presets-with-their-own-effects).

## A synth for each track

Every track can play its own instrument: Poly on one, FM on another, a plugin synth on a third, while the rest play the kit. The selected track is the one on the **Armed** row of the Record panel (see [Clips and patterns](clips.md#recording-a-clip)).

1. Select the track.
2. Open the Synth tab's options: press **MENU**, or tap the gear at the right end of the title bar. The top row is named after the track: **Track 2 instrument**.
3. Change it. It steps through **Kit** (the pads' kit, as every track starts), the other tracks' instruments (**Track 3's FM**: see below), **Poly**, **FM**, **Drums**, **Pluck**, **Keys**, **Drawbar**, **Squelch**, **Analog**, **Strings**, **Bass**, **Wind** and **Choir**, then any plugin synths you have put in `files/plugins/` on the stick: CLAP synths and MPC plugin instruments (see [Effects](effects.md#plugins) for where to put them).

{{shot: synth-instrument | Track 2's instrument set to a plugin synth, with what it plays, where it is heard and the row's help on the right.}}

- **What the track plays, and where you hear it.** Over the row's help, two lines say what the selected track plays and which mixer channel its sound comes out of, as the Mix tab names it: **Track 3 plays Track 2's Poly** and **In the mixer: Poly · Track 2**. A track on its kit reads **Track 1 plays Kit 1** and **In the mixer: Drums**; on the shared synth, **Track 2 plays the shared Poly** and **In the mixer: Synth**; with a synth REC pointed it at from the list of synths, **Track 2 plays FM, from the list of synths** and **In the mixer: FM · Pads**; sending MIDI out, **In the mixer: nothing, it sounds outside mpfree**.

- **It starts as a copy** of the synth's sound on the Synth tab, edits included. From then on it is the track's own: the Synth tab edits the selected track's instrument, and its title says whose it is ("Track 2"). Loading a preset gives the track a fresh copy with that preset.
- **The pads play the selected track.** Select another track and the pads play its instrument, or its kit if it has none. The track's clips play its instrument too, whichever track is selected.
- **Its parts are notes.** A take on a track with its own synth, or one that plays another track's or the **Synth** from the list of kits, records the notes you played rather than which pads you hit (a chord pad records each of its notes, and they stay one chord), and a pattern you tap in on its steps holds notes too. Capture, a second armed track and a MIDI file loaded there keep notes the same way. So the part keeps its notes when you change the pad layout, the scale or the octave later. On the Sequencer and the Play page each note shows on the row of the pad that plays it now, and a step you tap on a pad's row gets that pad's note, or its whole chord on the Chord pads layout. A note no pad plays now (after you moved the octave, say) still plays, but has no row until a pad plays it again. A clip that was already there records as it did. The Clip tab's **Melody** row says whether a clip uses notes or follows the pads, and turns an older clip that follows the pads into notes, keeping its tune as it sounds now ([Melody: Convert to notes](clips.md#melody-convert-to-notes)).
- **The arp plays its notes.** NOTE REPEAT, Latch and the arp play the track's synth by note, as they do with the synth on the pads: a chord pad gives the arp each of its notes, and what the arp plays records as those notes.
- **Changing it is smooth.** Notes still sounding on the old instrument finish naturally while the new one starts. **Kit** takes the instrument away, and the track plays its kit again.
- **Share one between tracks.** Pick another track's instrument on the row (**Track 3's FM**) and this track plays that one: its clips, and the pads while it is selected. The sound comes out of that track's mixer channel, and the Synth tab shows and edits that instrument, so a change there is heard on every track that plays it. A track that had its own instrument gives it up. Set the instrument's own track to **Kit** and the tracks that played it go back to their kits; the line along the bottom of the screen says which. Arrange writes **Track 3's FM** under each one. A lock lane on the track's volume, pan or sends moves that channel too (see [Lock lanes](sequencer.md#lock-lanes)).
- **Copy one instead.** Right after **Track 3's FM** on the row comes **Copy of Track 3's FM**: the track gets an FM of its own with the same sound, so you can change one without the other. The line along the bottom of the screen says so ("Track 4 plays its own copy of Track 3's FM: change one and the other stays"). The copy counts as one of the 16 below, and the project keeps both. The row reads **Copy of Track 3's FM** until you pick something else or open a project; after that it simply reads **FM**.
- **Up to 16 at once.** Any track can have its own instrument, and up to 16 tracks can at the same time, since each instrument takes its share of the MPC's power. With 16 already, the line along the bottom of the screen says so and the track stays on the kit: set one of the 16 to **Kit** first, and its place is free at once.
- **Its own mixer channel.** A track with an instrument gets a channel named for the instrument and the track (**FM · Track 2**, **Poly · Track 3**) in the mixer's Tracks view, and in the lists that pick a mix (Perform's **On**, the Looper's **Records from**), where a track you named shows its name (**FM · Bass**), with its own fader, inserts and sends. The channel only shows while the track has an instrument, and its settings stay with the track: set it to Kit and give it an instrument again later, and the channel is as you left it. Track 1's channel is the **Drums** strip, so track 1 has the same strip whether it plays the kit or an instrument; it is called **Poly · Track 1** (or whichever instrument it is) while the instrument plays there. A track that plays another track's instrument has no channel of its own: its notes sound on that instrument's. See [Mixing and effects](mixing-and-effects.md).
- **Shared options.** The scale, pad layouts, tuning and octave on the Synth tab's options are the same for every track's instrument.
- **A plugin synth** runs in a process of its own, as plugin effects do. If it crashes, the track goes quiet and the plugin restarts on its own, while the rest of MPFree plays on, and the status chip turns red ([A plugin stopped](../reference/when-something-goes-wrong.md#a-plugin-stopped)). If it can't be started at all, the line along the bottom of the screen says so for a few seconds and the track keeps what it had. Its controls show on the Synth tab like a built-in synth's.
- **An MPC plugin instrument's presets** are the programs built into it, listed on the Synth tab in the plugin's own order.
- **An MPC plugin instrument's own screen.** Most MPC plugins come with a skin: the screen their makers drew for the MPC. The Synth tab shows it full width, right under the top bar, exactly as the plugin shows on an MPC (see [The plugin's own screen](#the-plugins-own-screen)).
- **A plugin synth's own presets** are listed on the Synth tab, as "Category / Name" (Surge XT's factory presets read "Bass / Attacky", "Pads / ..."). The first time a plugin's presets are asked for, the plugin reads them all, which takes a few seconds; the list appears when it is done. Tap one to load it into the track's plugin. Some plugins load a preset in the background: Surge XT goes quiet for a moment, and notes you play in that moment aren't heard. If a plugin can't load a preset, the line along the bottom of the screen gives its reason. A plugin without presets of its own, such as Dexed, lists none. The project keeps the plugin's sound, but not which preset it came from.

{{shot: synth-track | The Synth tab editing track 2's own FM, with the Glass Keys preset.}}

- **Saved with the project.** Each track's instrument, with its sound (or a plugin synth's settings), is part of the project. Opening a project puts every track's instrument back, and **New project** puts every track on the kit (see [Saving your work](saving.md#projects)).

On Arrange, each track's instrument is written under its name, on the clip grid and in the Song view. Tap that line on the clip grid to go straight to the track's instrument row (see [Clips and patterns](clips.md#tracks-and-slots)).

Coming:
- The instrument's name in the track heading over the Play page's rows (it shows only **Track 2** for now).
- The **Voices on** meter in the Synth tab's Output section shows the pads' synth, not a track's own.

## The Synth tab

{{shot: synth | The Synth tab with Poly playing its Pluck preset: the presets on the left, the Osc page's sections in the middle (both oscillators with their waves drawn and their tuning, the mix as faders with the sub's wave above them), and the Q-Links on the right.}}

- **The gear** at the right end of the title bar opens the Synth tab's options (the scale, pad layout, tuning, note repeat and the rest), as **MENU** does. **‹ Back** at the top left of the options, or **MENU**, goes back to the synth.
- **The title** shows the preset, with **edited** under it once you change anything. Tap the preset's name to open the synth's presets in the browser (see [Trying and saving presets](#trying-and-saving-presets)). The tabs beside it are the synth's **pages**: for Poly, Osc, Filter, Env, LFO, LFO 2, Voice, Routes and Pressure. Tap a tab, or hold **SHIFT** and turn the data wheel, to change page.
- **The sections.** Each page is split into boxes, one for each job, with the job's name at the top: **Osc 1**, **Mix**, **Filter**, **Filter envelope**, **Output** and so on. The colour says what kind of job it is, the same on every synth and effect:
  - **cyan**: what makes the sound (oscillators, wavetables, strings, drawbars)
  - **coral**: what shapes its tone (filter, drive)
  - **lime**: envelopes and dynamics
  - **pink**: movement (LFOs, modulation)
  - **grey**: voices, tuning and output
- **The controls.** The biggest knobs on a page are the ones that matter most (Cutoff and Resonance on Filter). Drag a knob up or down to turn it. A choice with a few options shows them all as buttons: tap one. A longer one shows its value between **‹** and **›**. A whole number (octave, voices) has **–** and **+** either side. Envelopes and the mix are faders: drag them up. Drawbar's drawbars pull down, as on an organ. To use the data wheel instead, pick a control with the wheel (or a tap) and press **–** or **+**. Or push the wheel, and turning it changes the control picked (outlined more strongly) until you push it again.
- **The pictures** sit in the section they belong to and draw the sound as you change it: the oscillator waves over the oscillators, the filter's curve over the filter, the envelope over its faders.
- **Open ›** by a section's name takes you to the page that section is about. For example, the filter envelope's amount on the Filter page opens the Envelopes page.
- **A control that does nothing** with the sound as it is, such as a pulse width while the wave is a saw, is drawn faint, and the section's heading says why ("Pulse width · pulse waves only").
- **The Q-Links** turn the four controls marked **Q1** to **Q4**. The column on the right shows what each one turns on this page and its value. Drag a card there to turn it from the screen. Each Q-Link keeps its colour on its badge and its card, the same as on the Play tab.
- **Locking a step.** With **STEP SEQ** on, hold a step pad of a track this synth plays and turn a knob here: the knob is locked on that step only, and the sound stays as it was for the other steps. While you hold the step the tab shows its locks; let go and it shows the sound's own values again. See [Your first beat](sequencer.md#the-pads-as-steps-step-seq).
- **The presets** are listed on the left, under the synth's name. Tap one to load it. Once you change anything, the preset is marked **edited**. The ones you saved yourself are in the browser, under **Yours**.
- **UNDO** takes back the last knob you turned on this tab (a run of turns of one knob is one UNDO), and **SHIFT + UNDO** puts it back. Loading a preset starts the list again.

**Poly's waves.** Each oscillator plays a **Saw**, **Pulse**, **Triangle**, **Sine** or **Supersaw**. They stay clean all the way up the keyboard: high notes have no harsh, out-of-tune whistle under them. **Supersaw** is seven saws a little apart in pitch, the big, shimmering sound of trance leads and pads. Two knobs in the **Supersaw** box on the **Waves** page shape it, for either oscillator set to Supersaw ("Supersaw only" while neither is):

- **Saw detune** is how far apart the seven saws drift. Low settings move slowly and gently; the top of the knob gets wide and wild.
- **Side saws** is how loud the six outer saws are against the middle one. Turned down, you mostly hear one saw; turned up, the stack.

The level stays about the same whatever you set. **Super Lead**, **Super Pad**, **Super Stab** and **Super Stack** use it. Super Stack adds a wide ensemble and a bouncing delay, with its four macros on Detune, Mix, Octave and Effects.

Both oscillators have **Octave**, **Semi** and **Fine**, and the **Sub** under the mix can be a **Square** or a rounder **Sine** (**Sub wave**; "sub is off" while the Sub is at 0).

{{shot: synth-table | Poly's Waves page: oscillator 1 plays the Vowels table, oscillator 2 the Harmonics table mirrored, each drawn as its stack of waves with the one it plays bright; beside them the Supersaw's two knobs; below, what moves the positions.}}

**Poly's Table wave.** Set an oscillator's **Wave** to **Table** and it plays a table: a stack of single waves that change from one to the next. There are ten: **Basic** (sine to saw to square), **Harmonics**, **Pulse**, **Sync**, **Vowels**, **Organ**, **Bits**, **Resonance**, **FM** and **Drift**. Moving the position while a note sounds is what makes the sound evolve. The **Waves** page holds both oscillators' tables:

- **Table** picks the stack, **Position** where in it the oscillator plays, **Warp** how each wave is bent and **Warp amt** how much. **Bend** squeezes the wave to the front (brighter and more nasal), **Sync** repeats it up to eight times in each cycle (the tearing sync sound, kept clean on high notes and fast sweeps), **Mirror** plays it forward then back, and **FM** lets the other oscillator wobble this one. The Q-Links turn both positions and warp amounts, and the Q-Links' **Tables** set does too from any page.
- **Moves**: the filter envelope can sweep each oscillator's position (**Env to position 1** and **2**) and the warp (**Env to warp**), harder hits can start further in (**Velocity to position**), and LFO 1 can wobble each position (**LFO to position 1** and **2**). Pad pressure and the routes can move them too (**Position**, **Warp**).

A setting that does nothing says so in place: "Table wave only" while its oscillator plays another wave, "warp is off" while its Warp is Off. Everything else on Poly works with a table too: the filter, both LFOs, unison (now up to 8 copies), Age (which nudges the two oscillators' positions apart, note by note). **Morph Pad**, **Vowel Lead**, **Sync Sweep**, **Digital Bell**, **Growl Bass**, **Organ**, **Harmonic Pluck** and **FM Keys** play tables, and **Table Init** is a plain start with both oscillators on one.

**The Wavetable synth is part of Poly now** (since 19 November 2026). Its presets are Poly's, and anything you made with it plays on Poly with both oscillators on Table, sounding as it did: a track that played it, the pads' sound in a project, and presets you saved (they stay in **Instruments/Wavetable** on the stick; load one from there in the browser and it plays on Poly, with its effects, macros and Q-Link sets). The log says when a project's track moved to Poly.

{{shot: synth-voice | Poly's Voice page: how it plays (Poly, Mono or Legato, which held pad sounds, the number of voices, the glide and how it is timed, and Age), Unison (copies of each note, their detune and spread), the tuning and the output.}}

Poly's **Level**, on the Voice page, is -4 dB unless the preset sets it, so a chord of all eight voices at full velocity stays under full scale.

**Poly's envelopes** (on the Env page, and the filter's on the Filter page) have times that mean what they say: an **Attack** of 100 ms reaches full level at 100 ms, and a **Release** of 1 s fades 60 dB in 1 s and stops. **Velocity to level** is how much quieter your softest hit is than your hardest, in dB (8 dB by default; 0 dB plays every hit as loud, and keys-like sounds want 30 dB or more). Presets and projects saved before 19 November 2026 load with their times and velocity converted, so they sound as they did.

{{shot: synth-lfo-2 | Poly's LFO 2 page: each note's own LFO, with its shape, sync, Retrigger, rate and fade-in on the left, and how far it moves the pitch, cutoff, pulse width and level on the right.}}

**LFO 2** is a second LFO on Poly, on its own page, and every note has its own. In a chord each note wobbles by itself, not all together as with the first LFO. **Fade in** brings it in slowly after each note starts: the delayed vibrato of a singer, a violin or a lead line (the lowest setting, 1 ms, is no fade). **Retrigger** starts it again with each note. Turned off, each note starts it somewhere of its own, so a pad's notes drift apart; with **Sync** on, it follows the song instead. **To pitch**, **To cutoff**, **To width** and **To level** are how far it moves each.

{{shot: synth-routes | Poly's Routes page: route 1 takes the mod wheel to vibrato, as on every preset that doesn't use it for something else; routes 2 to 4 are free.}}

**Routes** connect how you play to what the sound does: four of them, on Poly's **Routes** page. **From** is what moves it:

- **Velocity**, how hard you hit;
- **Pressure**, how hard you keep pressing;
- **Tone**, a keyboard's slide or MPE tone;
- the **Mod wheel**, a **Breath** controller or a **Foot** controller (an expression pedal);
- **Key**, low notes one way and high notes the other, from middle C;
- **Random**, a new value for each note.

**To** is what it moves, from the same list as pressure's (Cutoff, Pitch, Level, Vibrato, Drive, Pan and the rest, with LFO 1 amount and LFO 2 amount). **Depth** is how far, either way: a negative depth on Level from Key makes high notes quieter. On **Level**, **LFO 1 amount** and **LFO 2 amount** a route sets how much gets through: at a depth of 50 %, the wheel at rest lets half the LFO through and the wheel at the top all of it. Route 1 takes the mod wheel to vibrato on every preset, so a keyboard's wheel works straight away.

**Singing Lead** and **Drift Pad** show them off. Singing Lead's vibrato fades in on each note, harder hits push its drive, and a keyboard's tone opens its filter. Each of Drift Pad's notes sits at its own place across the speakers and a few cents off, its LFO 2 moves each note's tone by itself, and the mod wheel opens its filter.

**Unison** is on Poly's **Voice** page: **Copies** plays up to eight copies of every note, **Detune** spreads them in pitch and **Spread** across the speakers. It thickens any wave, the supersaw too, but each copy costs as much as a note.

**Playing a lead on Poly.** With **Mode** on **Mono** or **Legato** (the Voice page), **Priority** picks which pad sounds while you hold several: the **Last** you pressed (the default), the **Low**est (a bass line that trills against a held low note) or the **High**est (a tune on top while you hold the notes under it). Let go of the pad that sounds and the note goes back to one still held. **Glide mode** sets how the glide is timed: with **Time** every glide takes the Glide time; with **Rate** an octave takes the Glide time, so a small step is quick and a big leap sweeps. **Portamento Lead** glides by rate, **Low Note Lead** and **Unison Lead** play the lowest and the highest pad held, **Breath Lead** opens up with a breath controller or pad pressure, and **Slide Lead** follows an MPE keyboard: each note bends by itself as you slide your finger, and moving it up the key brightens that note.

**Age**, also on Poly's **Voice** page (Q-Link 2 there), makes each voice a little its own, as on the polysynths of the late 1970s, which had a circuit board for each voice: each note a few cents out of tune with the others, a little brighter or darker, quicker or slower, and each one drifting slowly in pitch as it plays. Notes go to the voices in turn, so the same chord sounds a little different each time you play it. At 0 (the default) every voice is the same; at 100 they differ as much as Analog's do at 100. **Worn Brass** is Brass at 80.

**Drive** and **Distortion**, on Poly's Filter page (Q-Links 3 and 4 there), both add grit. **Drive** pushes the sound into the filter, as on an analog synth: warm, and the filter smooths what it adds, so with the filter nearly closed you hear little of it. **Distortion** comes after the filter, so you hear its grit even with the filter almost closed (Grit Bass uses it). Both keep the sound at about the same level as you turn them up (within about 2 dB on every preset), so they add grit and thickness, not volume; turn up the synth's level for that. Pad pressure can push either one. Both stay clean on high notes, without the harsh, out-of-tune fizz cheap drives add up there. Low settings, under about 9 dB, warm the sound gently, mostly with the third harmonic; from about 12 dB up they give their full grit.

Each synth has its own pages. FM has Algo, Mod and Pressure, a page for each of its six operators and three keyboard-scaling pages; Drums has a page for each of its 16 pads, a **Kit** page, **Effects** and **Sends** (below). Poly, FM and Wind have a **Pressure** page (see [Pad pressure](#pad-pressure)).

**Drums** plays a different synthesised drum on each pad: **Kick**, **808 kick**, **Snare**, **Clap**, **Closed hat**, **Open hat**, **Cymbal**, **Tom**, **Conga**, **Rim**, **Woodblock**, **Bell**, **Cowbell**, **Shaker**, **Zap** or **FM perc**, picked by **Model** on the pad's page. Each drum has its own few knobs, named for what they do on that drum, and the Q-Links take the four you reach for most on it:

| Drum | Its knobs | Q-Links |
|---|---|---|
| Kick | Tune, Decay, Sweep and Sweep time (the pitch's drop at the start), Click, Tone, Drive | Tune, Decay, Sweep, Click |
| 808 kick | Tune, Decay, Tone, Punch (the jump in pitch on the hit), Sigh and Sigh time (the slow settle onto the note after it), Drive | Tune, Decay, Tone, Drive |
| Snare | Tune, Decay, Tone, Snappy (the wires), Ring (the head's overtones), Click, Drive | Tune, Decay, Tone, Snappy |
| Clap | Tone, Decay, Claps (how many hands), Gap (between them), Room, Sharp, Drive | Tone, Decay, Gap, Room |
| Closed hat, Open hat, Cymbal | Tune, Decay, Tone, Noise (against the metal), Spread (how far apart the six metal tones sit), Click, Drive | Tune, Decay, Tone, Noise (Spread on a cymbal) |
| Tom, Conga | Tune, Decay, Tone (how hard the stick or hand), Strike (centre to edge), Bend and Bend time (the head's pitch dropping after a hard hit), Click (Slap on a conga) | Tune, Decay, Tone, Bend |
| Woodblock, Bell | Tune, Decay, Tone, Strike, Click | Tune, Decay, Tone, Click |
| Rim, Cowbell | Tune, Decay, Tone, Click, Noise, Drive | Tune, Decay, Tone, Click |
| Zap | Tune, Decay, Sweep, Sweep time, Tone, Noise, Drive | Tune, Decay, Sweep, Sweep time |
| FM perc | Tune, Decay, Tone, Ratio, Sweep, Sweep time, Noise | Tune, Decay, Tone, Ratio |
| Shaker | Tune, Decay, Attack (how slowly it swells in), Tone, Grain (single beads against a hiss) | Decay, Attack, Tone, Grain |

Every drum also has these, in the **Feel** and **Mix** boxes:

- **Level**, **Pan** and **Choke**: pads in the same choke group cut each other off, as a closed hat cuts the open hat.
- **Vel > level** and **Vel > tone**: how much quieter, and how much darker and softer, a gentle hit is than a hard one. A hard hit is brighter and snappier, not just louder.
- **Accent**: how much harder your hardest hits land (they start above about velocity 90).
- **Variation**: how much each hit differs from the last, in level, pitch, tone and length, so a roll or Note Repeat doesn't sound like a machine gun. At 0 every hit at one velocity is the same.
- **Plays**: **One-shot** (the drum rings out however short your hit) or **While held** (it stops soon after you let go of the pad, for choked cymbals and short hats you play with your finger).

**Decay** means what it says: a decay of 400 ms has the drum 60 dB down at 400 ms, whatever the drum and however hard you hit.

The **Kit** page holds the kit's **Level**, its **Pad 1 note** and **Voices**. The factory kits play the **Current** voices, which have everything above. A kit you saved before this update plays **Classic**: exactly the drums it always had, with the same fourteen knobs on every pad. Switch it to Current to give it the new drums' sound and knobs; Accent, Variation and Plays are faint on a Classic kit ("kit voices are Classic"). The newer drums (808 kick, Conga, Woodblock, Bell, Shaker) play their own sound in either.

**Drums' effects.** Every drum in the kit has the same four effects of its own, each off until you turn it up, and its own sends to the kit's delay and reverb. The **Effects** tab shows the effects of the pad you hit or looked at last (the kit's picture rings it); hit another pad, or tap it in the picture, to see that drum's. The Q-Links turn **Punch**, **Distortion** and the two sends.

- **Punch:** **Punch** makes the front of each hit harder (or softer, below 0) and **Sustain** brings up (or down) what rings after it. It works the same on a quiet hit as on a loud one.
- **Distortion:** grit that doesn't make the drum louder: a hit keeps its peak. **Tone** below 0 makes the grit darker, above 0 makes the highs bite; **Mix** blends it with the clean drum. Kicks and snares stay clean however hard you push them; a high, ringing drum pushed hard gets a fizz of its own.
- **EQ:** **Low** (100 Hz), **Mid** at **Mid freq**, and **High** (8 kHz), each up to 15 dB up or down.
- **Compressor:** off until **Ratio** is above 1:1. **Threshold** is where it starts to act, **Attack** and **Release** how fast it acts and lets go. It holds the hit's peak down so its tail comes up against it; it doesn't make the drum louder by itself, so turn up **Output** after it.
- **Output and sends:** **Output** is the drum's level after its effects (up to 24 dB up or down). **Delay** and **Reverb** send it to the kit's delay and reverb.

The **Sends** page sets those two for the whole kit. The delay: **Time** in beats of the project's tempo, **Feedback** (how many echoes), **Tone** (darker echoes lower) and **Delay level**; its echoes bounce from left to right. The reverb is a room: **Size**, **Decay**, **Tone** (brighter higher), **Gate** (cuts the tail off after that long, for a gated drum) and **Reverb level**. The Q-Links turn the delay's Feedback and level and the reverb's Decay and level.

A control that does nothing as the kit is set is faint and says why: the compressor's other controls at a ratio of 1:1, the distortion's Tone and Mix with no distortion, Mid freq with Mid at 0, and the delay or the reverb while no drum sends to it. Effects on a few drums cost the MPC very little; all four on every drum with both sends take more than the drums themselves, so use them where they count. The kit's effects are saved with it.

### The drum kits

Drums comes with 27 kits. Pick one as you pick any preset (tap its name on the Synth tab). In every kit the drums sit at about the same loudness against its kick, so you can play a kit straight away and mix it later; each drum alone, hit as hard as you can, stays well under the top of the meter.

| Kit | The style | What's on it |
|---|---|---|
| **808 Kit** | The classic 1980s analog drum machine | The long round kick, the snappy snare and clap, metal hats, cowbell, congas, claves and maracas, and a low 808 for bass lines |
| **Club Machine Kit** | The club drum machine of house and techno | A kick with a click and a punch, a noisy snare, a loud clap, bright hats, a ride and a crash, and a second kick, snare and clap to layer |
| **Bright Machine Kit** | A bright, crisp drum machine | Short drums, a ringing snare, sharp hats, tambourine, cowbell, ride and crash |
| **Vintage Rhythm Kit** | The early rhythm boxes | A soft round kick, a dull snare and rim, ticking hats, bongos, congas, claves, maracas, tambourine and guiro, through a little tape |
| **Eighties Punch Kit** | 1980s pop and rock machines | A tight kick and snare, big toms and a snare in a gated room, claps, cabasa, tambourine and congas |
| **Miami Bass Kit** | Miami bass and electro | A very long, low 808 and a short one for punch, a snappy snare and clap, quick hats, toms, cowbell, claves and a zap |
| **Trap Kit** | Trap | A long driven 808, a hard snare and clap, hats for rolls (a tight one on pad 13), toms, a finger snap, a bell and a drop |
| **Drill Kit** | Drill | A dark driven 808, a high crisp snare, a rim with a triplet echo, quick hats, bell and wood percussion, and a second, distorted 808 |
| **Boom Bap Kit** | 1990s hip-hop | A thick kick and a cracking snare, both driven and darkened, dusty hats and percussion, through tape and a compressor |
| **Lo-fi Kit** | Lo-fi hip-hop | Gentle drums with dark tops, a brushy snare and a soft bell, through fewer bits and worn tape (its hiss and crackle are part of it) |
| **House Kit** | House | A round kick, a clap in a short room, open hats for the off-beat, a ride, congas, a rim with an echo, shaker and cowbell, through a compressor |
| **Deep House Kit** | Deep house | A soft deep kick, a clap in a long room, warm hats, congas, a rim, woodblock and bell in a dotted echo, a ride and a sub kick |
| **Techno Kit** | Techno | A distorted kick and a rumble kick in the room, a driven snare and clap, hard hats and ride, metal toms, a rim with an echo, a zap and an FM blip, through a compressor |
| **Minimal Kit** | Minimal | Clicks and blips: a short kick, a thin snare, ticks, rim clicks, and FM and bell blips in a sixteenth-note echo |
| **Hardstyle Kit** | Hardstyle | A heavily distorted kick and a long tuned one, a big clap and snare in the room, bright hats and a laser zap, through a compressor |
| **Drum and Bass Kit** | Drum and bass | A punchy kick, a high cracking snare and a ghost snare, quick hats and ride, tight toms, through a compressor |
| **Amapiano Kit** | Amapiano and Afro house | Log drums low to high (pads 6 to 8), a soft deep kick, a rimmy snare, a clap in the room, two shakers, congas, a rim with an echo, woodblock and an agogo bell |
| **Latin Percussion Kit** | Latin percussion | Congas open, slapped and low (the open and the slap cut each other off), bongos, timbales, claves, cowbell, two agogo bells, maracas, guiro and a block, with a soft kick on pad 1 |
| **Reggaeton Kit** | Reggaeton | A punchy kick and a high snare for the dembow, a long 808, timbales, rim, shaker, guiro, conga, woodblock and a zap |
| **Synth-pop Kit** | Synth-pop | A punchy kick, a snare and clap in a gated room, synth toms that sweep down, bright hats, cowbell, a bell, tambourine and an all-noise snare |
| **Industrial Kit** | Industrial | A distorted kick, an all-noise snare, metal hits (anvil, pipe, sheet, clang), noisy hats and cracks and a noise sweep, in a big gated room and through fewer bits |
| **FM Kit** | FM drums | Every drum but the kick made by FM: snare, hats and cymbal, toms and a wood drum, a bell, a blip, a log drum, a zap and an FM kick |
| **Boom Kit**, **Tight Kit**, **Grit Kit**, **Electro Kit** | MPFree's first four kits, named for their sound | Boom Kit has a long 808 on pad 14, and Grit Kit one driven hard |

A kit's effects for the whole kit (tape, fewer bits, a compressor) are on its **FX 1** and **FX 2** pages, as on any preset with its own effects ([Presets with their own effects](#presets-with-their-own-effects)). Hit every pad at once as hard as you can and the kit goes over the top of the meter, as any loud kit does: the master's safety limiter catches it ([Master safety](mixing-and-effects.md#the-master-and-master-safety)).

The USB stick also carries MPFree's first five kits (Init, Boom, Tight, Grit and Electro) rendered as sample kits, for the **Kits** place.

**FM** builds its sound from six **operators**. Each is a simple wave at its own pitch, and each either plays out loud (a **carrier**) or bends another operator's wave (a **modulator**), which adds overtones: the louder the modulator, the brighter and more metallic the sound. **Algorithm** picks which operators bend which (the picture shows it: the bottom row plays out loud), and **Feedback** lets one operator bend itself, from a little edge to a rough buzz. Its pages:

- **Algo:** **Algorithm**, **Feedback** and **Osc key sync** (every note starts its waves from the same point), the six switches **Op 1 on** to **Op 6 on** (switch one off to hear what it adds), and the voice: Poly or Mono, voices, transpose, bend range and **Level** (-9 dB unless the preset sets it).
- **Mod:** the LFO (its wave, speed and delay, how much it wobbles the pitch and the level) and the pitch envelope.
- **Pressure:** see [Pad pressure](#pad-pressure).
- **Op 1** to **Op 6:** the operator's envelope (four rates and four levels, as on the classic FM synths); the **Oscillator**: its **Wave** (**Sine**, the classic FM sound, or **Triangle**, **Saw**, **Square** or **Half sine**, which bring overtones of their own; each plays as loud as the sine) and its pitch (a **Ratio** to the note or a **Fixed** frequency, **Coarse**, **Fine** and **Detune**); and its **Output**, **Velocity** (how much harder hits make it louder, or a modulator brighter) and **Amp mod** (how much the LFO wobbles its level).
- **Kbd 1-2**, **Kbd 3-4** and **Kbd 5-6:** keyboard scaling, two operators to a page: how an operator gets quieter or louder above and below a **Break** key, and **Rate scale**, how much faster its envelope runs up the keys.

FM keeps itself clean up the keyboard: on the top notes a bright modulator and the feedback ease off just enough that their overtones don't fold back into a harsh fizz, and a Saw or Square that bends another operator hard drops its top overtones first. Changing the algorithm keeps the sound at about the same level. Every factory preset but Init comes with four macros, **Brightness**, **Detune**, **Attack** and **Decay**, and most bring a touch of reverb (Drawbars brings the rotary speaker instead, and Rubber Bass stays dry); Reed Lead plays the Triangle, Saw and Square waves. The Q-Links take Algorithm, Feedback, Transpose and Level on the Algo page; Speed, Pitch depth, Amp depth and Delay on the Mod page; and Output, Coarse, Fine and Velocity on each operator's page.

**Pluck** is a plucked string or a struck bar, picked by **Model** on the String page:

- **String:** **Decay** is how long middle C rings (higher notes ring shorter, lower ones longer), and **Release** how fast a note you let go is damped. **Brightness** is the string's tone, **Pick** how hard and bright the pick is, and **Position** where along the string you pluck: near the end is thin and bright, towards the middle hollow.
- **Mallet:** **Material** is **Marimba**, **Vibes**, **Bell**, **Glass**, **Kalimba**, **Steel drum**, **Glockenspiel**, **Xylophone** or **Chimes**. **Hardness** is the mallet: a harder one is brighter. **Strike** is where on the bar it lands, which changes which overtones ring. **Stretch** pulls the overtones closer together or further apart, for a softer or a more clangy, metallic tone, and **High damp** makes the high overtones die away sooner.
- **Voice:** transpose, fine tuning, bend range, how many notes can sound (**Voices**), **Vel > tone** (softer hits are darker), **Vel > level** (how much quieter your softest hit is than your hardest: 30 dB by default, 0 for every hit the same), level (-2 dB unless the preset sets it), pan and **Spread** (notes spread across the speakers).

Every note is as loud as middle C, whatever the knobs, so a melody up high doesn't jump out and a low line doesn't disappear. Playing a note that is still ringing plucks or strikes it again, on top of what sounds, as a real string or bar does. The Q-Links take Decay, Brightness, Pick and Position on the String page, Hardness, Strike, Stretch and High damp on the Mallet page.

**Keys** is an electric piano or a clav. It has five pages:

- **Strike:** **Model** (**Tine** or **Reed** for the two kinds of electric piano, **Clav** for the clavinet's struck string), **Hardness** (a harder hammer or tangent is brighter; on the tine it rings the bell louder and adds a tick to the top of the attack), **Bell** (the ring of a piano's attack: on the tine, a bell-like tone far above the note that dies away within a second or so; at 40 it is as loud as on the real instrument, and the top octaves ring it less), **Decay** (how long a held middle C rings, 12 seconds on the tine as standard; higher notes ring shorter) and **Release** (how fast the damper stops a note you let go).
- **Tone:** **Voicing** is where the tine or reed sits against its pickup, as a technician sets it. Low values are hollow, with the octave above the note strong; high values are pure and round; the classic sound is in between. **Bark** is how much a hard hit growls (low notes growl first, as on the real instrument; on the clav it is the bite of a hard hit). On the tine, Bark changes the colour of a hard hit, not how loud it is. **Tone** tilts the sound darker or brighter, **Vel > tone** makes soft notes softer in tone, not only in level (on the tine at 60, as on the real instrument: the softest hits are nearly pure and the hardest growl), and **Velocity** is how much quieter your softest hit is than your hardest (0 dB plays every hit as loud).
- **Clav:** **Pickup** is which of the clav's two pickups you hear: **Neck** (round), **Bridge** (thin and bright), **Both**, or **Out of phase** (thin and nasal). **Tone** is the clav's four tone switches, **Soft** to **Brilliant**. **Mute** lays the mute bar on the strings: shorter, darker notes, for funk lines. **Release drop** is how far a note falls in pitch as you let go and the yarn damps the string (0 to 6 semitones): the clav's "wow" on short notes.
- **Amp:** **Tremolo** and **Trem rate**, with **Trem mode** **Pan** (the sound moves between the speakers) or **Amp** (its level pulses; on the reed it also moves the tone, as on the original). **Drive** pushes the amplifier, and **Key noise** is the thump of the hammers and dampers.
- **Voice:** transpose, fine tuning, voices, bend range, level and pan.

The Clav page's controls are faint unless Model is Clav, and Bell and Voicing are faint while it is ("tine and reed only"). Every note across the keyboard plays about as loud as the next, on every model and preset. The tine is matched to recordings of a real tine piano, across the keyboard and from its softest hits to its hardest: the growl, the bell and how long each note rings.

Its pictures, all for middle C: the **Hammer** (a soft hit and a full one; a harder hammer is shorter; on the clav, the tangent), how long C2, C4 and C6 ring (**Decay**), the **Pickup**'s curve with the stretch a hit swings across (the dot is where the tine or reed rests: turn Voicing and watch it move), and the **Tremolo**. With the clav, the Clav page shows the **Pickups** (which of a low note's harmonics the pickups and the tone switch let through: each pickup leaves out the one over its place on the string) and the **Mute** (how long each harmonic rings: turn Mute and watch the top ones fall first).

**Drawbar** plays like an organ: every note is as loud as the next however hard you hit the pad, and a note sounds for as long as you hold it. **Model** picks which organ: **Tonewheel**, the spinning-wheel organ of jazz, gospel and rock, or **Combo**, the thin, cutting transistor organ of 1960s garage bands, ska and new wave. Switching while you play crosses from one to the other in a hundredth of a second, so it can be part of a song. It has four pages:

- **Drawbars:** the nine drawbars, 0 (in) to 8 (all the way out), written by organists as nine digits from the left: Init is 888000000. The first two (16' and 5 1/3') are below the note, the 8' is the note itself, and the rest are its harmonics, brighter the further right. The Q-Links pull the four upper drawbars, since the first three usually stay out. Beside them, **Model** and the combo's **Voices**: **Flute** (a soft, round tone, close to a pure note) and **Reed** (bright and buzzy), two more drawbars from 0 to 8 that mix into every drawbar you pull, and **Reed wave**: **Saw** (bright and full) or **Square** (hollower and rougher). With both voices at 0 the combo is silent, as on the real thing. The voices are faint on the tonewheel ("combo only").
- **Percussion:** **Percussion** adds a short "ping" of the 4' (**Perc harm** Second) or the 2 2/3' (Third) at the start of a note, **Fast** or **Slow**, **Normal** or **Soft**. As on the organ, it only fires when no other key is held, so play detached to hear it on every chord and legato to leave it out; while it is on, the 1' drawbar is silent and Normal makes the drawbars a little quieter. On the combo it pings through the flute. **Vibrato** is **V1** to **V3** (pitch wobble, deeper with each) or **C1** to **C3** (chorus: the same mixed with the plain sound), at the speed **Rate** sets. On the combo the wobble is an even swing of the pitch, the same on every note (10, 20 or 30 cents either way); the original combos had vibrato only, so the chorus settings are an extra.
- **Rotary:** the rotating speaker (both models can play through it). **Speed** is **Slow** (a gentle swirl), **Fast** (a quick tremolo) or **Stop**; switching it is part of playing, and the treble horn speeds up and slows down quickly while the bass drum takes a few seconds. **Spread** is how wide the speaker sounds (0 is mono), and **Rotary** Off takes the speaker out. The tone controls are here too: **Click** is the spit of the key contacts at the start of each note, **Leakage** the faint bleed of other tonewheels, **Drive** the growl of a pushed amplifier, **Tone** darker or brighter, and **Wheels** **Gears** (the organ's own slightly uneven tuning) or **Exact**. Leakage and Wheels are faint on the combo ("tonewheel only"): its notes are always exactly in tune with the project's tuning, and every octave is exact.
- **Voice:** transpose, fine tuning, level and pan, and how many keys are held.

The drawbars themselves are coloured as on the organ: the two below the note in the section's colour, white the note and its octaves, grey the other harmonics. Its pictures: the **Wave** they add up to (on the combo, its flute and reed), how the **Percussion** fades, and how fast the horn and the drum turn after you switch Speed (**Rotor**).

The combo presets start from the classic settings: 16', 8' and 4' with flute and reed and a light vibrato (**Combo**), the flute alone (**Combo Flute**), the reed on every drawbar for a bright buzz (**Combo Reed**), the square reed with a deep, fast vibrato (**Combo Square**), a slow, deep vibrato (**Combo Vibrato**), the reed through a pushed amplifier (**Combo Garage**), the rotating speaker with chorus (**Combo Rotary**), and percussion on the flute (**Combo Perc**). The Q-Link set **Combo** turns Flute, Reed, 8' and 4'.

**Squelch** is a mono acid bass: it plays one note at a time, and a new pad takes over from the last. Two things in how you play shape it, as on the bass line machines it learns from:

- **Accent:** hit a pad at or above **Accent at** (velocity 100 by default) and the note is accented: louder, with a sharper jump of the filter. With Resonance high, accents close together build on each other, each brighter than the last.
- **Slide:** press a new pad before you let go of the last and the pitch glides there without starting the note again. Let go of it and it glides back to the pad still held. Play detached for separate notes.

The classic move is to turn **Cutoff** and **Resonance** while a line plays; the Synth page's Q-Links are set up for it. It has three pages:

- **Synth:** **Wave** (**Saw** or **Square**; both stay clean on high notes), **Cutoff** (where the filter rests between notes), **Resonance** (the squelch; at 100 the filter is on the edge of whistling and screams on high notes), **Env mod** (how far each note opens the filter), **Decay** (how fast it closes again) and **Accent** (how far accented notes jump out).
- **Play:** **Slide** (how long a glide takes), **Accent at**, **Acc decay** (how fast the filter closes on accented notes), **Amp decay** (how long a held note takes to fade; short for plucked notes), **Key track** (how far the filter follows the note; at 0, as on the original, high notes come out darker) and **Drive** (distortion after the filter, as from a pedal).
- **Voice:** transpose, fine tuning, bend range, level and pan.

Its pictures: the **Wave**, the **Filter**'s curve where it rests between notes (the dashed line is where each note opens it), and the cutoff over four quick notes played plain and accented (**Sweep**), so you can see accents build.

**Analog** plays like the polysynths of the late 1970s, which had a circuit board for each voice. No two boards were quite alike, and notes went to them in turn, so the same chord sounds a little different each time you play it. **Age** sets how much: at 0 every voice is identical, at 30 (the default) they differ slightly, and at 100 they differ as much as on an old synth that hasn't been calibrated in a while: each voice a few cents out of tune with the others, a little brighter or darker, quicker or slower. The higher it is, the more each oscillator also drifts slowly as it plays. It has five pages:

- **Osc:** two oscillators, each **Saw**, **Pulse**, **Saw+Pulse** or **Triangle**, with their octave, the second's semitones and **Osc 2 fine** (its detune against the first, the source of the thick, beating sound), and each pulse's width (**PW**). **Sync** locks oscillator 1 to oscillator 2's cycle: tune oscillator 1 up and it tears rather than sweeps. The oscillators stay clean on high notes, with Sync and with poly-mod: no gritty, out-of-tune whine folding down from above what you can hear. **Osc 1**, **Osc 2**, **Sub** (a square an octave down) and **Noise** are the mix.
- **Filter:** **Filter** **LP 24** is the classic four-pole: thick and smooth, and at **Resonance** 100 it whistles on its own. **LP 12**, **BP 12** and **HP 12** are a gentler two-pole that rings but never whistles. **Env amount** is how far the filter envelope opens it (below 0 it closes it), **Key track** how far it follows the note, **Vel > filter** how much softer notes open it less, and **Drive** how hard the oscillators push it (grit, not volume). Turning Resonance up thins the bass a little, as on the old synths.
- **Envelopes:** the filter's (**F**) and the volume's (**A**) attack, decay, sustain and release, from 1 ms to 20 s, and **Vel > amp** (how much softer notes play quieter). The attack has the old synths' shape: fast, then bending over just before the top.
- **Mod:** the **LFO** (**LFO rate**, **LFO wave**, and **LFO delay**, which fades it in after you start a phrase) on the pitch (**Vibrato**), both pulse widths (**LFO > PW**) and the filter (**LFO > cutoff**). **PMod env** and **PMod osc 2** are poly-mod: each voice's own filter envelope and its own oscillator 2 bend oscillator 1's pitch, its pulse width or the filter, as the three **PMod >** switches choose. The envelope on the pitch of a synced oscillator 1 is the classic sync sweep; oscillator 2 on the pitch or the filter at audio speed gives clangy, bell-like sounds. **Press to** sends pad pressure to the **Cutoff** (pressing harder opens it, up to two octaves) or to a **Vibrato** (up to 50 cents), by **Pressure**. **Wheel vibrato** is how deep the mod wheel's vibrato goes with the wheel at the top (50 cents unless you change it); it uses the LFO's rate and wave.
- **Voice:** **Mode** **Poly** or **Unison** (all the voices on one note, spread by **Detune**, for leads and basses), **Voices**, **Glide** (in Poly each voice glides from the last note it played, so chords smear into each other), **Age**, **Chorus** (**Slow** and **Fast** are the wide, swirling stereo chorus of the time, **Vibe** a quick shimmer), transpose, fine tuning, bend range, level (-3 dB unless the preset sets it) and pan. In **Unison**, **Priority** picks which of several held pads sounds (the **Last** pressed, the **Low**est or the **High**est), and **Legato** glides to a pad pressed while another is held without starting the note again; in Poly both are drawn faint.

Its pictures: both oscillators (**Osc**; with Sync on, oscillator 1 restarts with each of oscillator 2's cycles), the **Filter**'s curve, both envelopes (**Env**), the **LFO**, and each voice board's own tuning at the Age set (**Voices**).

On an MPE keyboard each note bends by itself as you slide your finger, and moving it up the key brightens that note. **Solo Lead**, **Glide Lead** and **Fat Lead** are made for playing lines.

The Q-Links take the pulse widths and detune on the Osc page, the filter's four main knobs on the Filter page, the envelopes' times, the vibrato and poly-mod depths on the Mod page, and Age, Glide, Detune and Level on the Voice page.

**Strings** plays like the string machines of the 1970s. Every key you hold sounds, however many, so you can hold big chords with both hands, and the swirl that makes it sound like a string section comes from its built-in **Ensemble**. Hit harder and a note is louder and a little brighter; set **Velocity** to 0 and every hit sounds the same, as on the old machines. It has four pages:

- **Registers:** six faders, **Violin**, **Viola**, **Cello**, **Contrabass**, **Horn** and **Trumpet**. Any mix sounds at once, and a mix of several is about as loud as one alone. Violin and Viola play the note you play (Violin brighter, with a little of the octave above), Cello adds the octave below and Contrabass is the octave below alone; Horn is mellow and Trumpet bright and biting. **Octave** moves the notes you play next up or down, by up to two octaves. The picture is the tone of the mix you have up.
- **Tone:** **Brightness** opens or closes the tone of every register, by up to an octave either way. **Body** is the woody resonance of each register (0 takes it out), and **Bow noise** a breathy edge that follows how loud you play. The **Swell** shapes every key: **Crescendo** is how long a held key takes to come in, from a quick 2 ms to 8 seconds, and **Sustain** how long it takes to die away when you let go. **Envelope** **Per key** gives each key its own swell; **Shared** is how the old machines did it: a chord swells in together, the last chord fades out, and a key you let go while others are held stops at once. **Velocity** is how much quieter a soft hit is than a hard one (soft hits are darker too). The picture shows a held key coming in and dying away.
- **Ensemble:** **Amount** (0 is dry, with none of the swirl), **Lines** (3 or 4: four is a little thicker), **Width** (how wide it spreads between the speakers), and the two wobbles that make it: **Slow rate**, **Fast rate** and **Depth** (how far they move, against the ensemble's own). With Amount at 0 the other ensemble controls are drawn faint. The picture shows the lines moving over one slow cycle.
- **Voice:** transpose, fine tuning, bend range, level, and how many keys are held.

The Q-Links take four registers on the Registers page (Violin, Viola, Cello and Contrabass), Brightness, Crescendo, Sustain and Bow noise on the Tone page, Amount, Depth, Slow rate and Width on the Ensemble page, and Transpose, Fine, Bend range and Level on the Voice page. A Strings track with no keys held stops working after a tenth of a second, so it takes almost none of the MPC's power until you play it.

From a keyboard on MIDI in, Strings answers the sustain and sostenuto pedals, the soft pedal (6 dB quieter and a little darker), the mod wheel (vibrato, from the ensemble's fast wobble) and a breath controller or expression pedal (volume). See [Pedals and the mod wheel](midi-and-cv.md#pedals-and-the-mod-wheel).

**Bass** plays one note at a time, for bass lines. **Body** on the Source page picks one of three sounds:

- **Synth:** an oscillator with a sub an octave or two under it, into a resonant filter: the classic synth bass, from round and soft to rubbery and biting.
- **808:** a deep sine (or a triangle) that starts high and drops onto the note, with no filter: the long, booming bass of trap and hip-hop. Drive makes it growl and lets it be heard on small speakers.
- **Reese:** two oscillators tuned a little apart, so they beat against each other, over a clean sub: the moving, growling bass of drum and bass.

How you play shapes it. Press a new pad before you let go of the last and the pitch glides there without starting the note again (**Retrigger** on starts it again). Hit harder and a note is louder and a little brighter. Low notes are lifted a little, so a line sounds even from the bottom of the keyboard up. It has five pages:

- **Source:** **Body**, **Wave** (**Saw**, **Pulse**, **Triangle** or **Sine**; the 808 plays a sine unless you pick Triangle), **Pulse width** (on the pulse only) and **Reese detune** (how far apart the reese's two oscillators are: more beats faster). Under the note: **Sub** (how much), **Sub mode** (a square one or two octaves down, a narrow pulse two down, or a sine one octave down or on the note; it stays locked to the note, so it never thins out), **Fifth** (a fifth above, for a fuller, more synthy tone) and **Noise**. **Octave** moves the whole sound down two octaves or up one. **Start** makes every note begin its wave at the same point (**0°**, the default, so every note hits the same way; or **180°**), or carry on from where the last one left it (**Free**). The picture is the wave with its sub and fifth.
- **Filter:** **Cutoff** and **Resonance**; the filter envelope's **Amount** (how far each note opens the filter; below 0 it closes it) and **Decay** (how fast it closes again); **Key track** (how far the filter follows the note) and **Drive in** (how hard the oscillators push the filter: grit, not volume). The 808 has no filter, so these are drawn faint when it is picked. The picture is the filter's curve where it rests; the dashed line is where each note opens it.
- **Shape:** **Drop** (how many semitones above the note each note starts) and **Drop time** (how long it takes to land exactly on the note), **Punch** (a quick jump up in pitch for the first few milliseconds, for a sharper hit), the volume's attack, decay, sustain and release, **Velocity to level** (how much quieter a soft hit is) and **Velocity to tone** (how much darker). The picture is the pitch falling onto the note.
- **Play:** **Glide** (how long a glide takes; with **Mode** set to **Rate** it is how long an octave takes, so small steps are quicker), **When** it glides (**Legato**: only to a pad pressed while another is held; **Always**: from whatever note played last), **Priority** (with several pads held, which one sounds: the **Last** pressed, the **Low**est or the **High**est), **Retrigger**, transpose, fine tuning, bend range, and pad pressure: **Moves** picks what pressing harder on a held pad does (opens the **Cutoff**, adds **Drive**, or a **Vibrato**) and **Amount** how much. On an MPE keyboard the note bends as you slide your finger, and moving it up the key brightens it. **Rate Glide Bass** and **High Note Lead** use the glide's Rate and the High priority.
- **Output:** **Drive** (distortion after the filter: **Soft** adds warmth and the overtones that let a bass be heard on phone and laptop speakers, **Hard** squares it off into a harsher growl), **Tone** (takes the fizz off the top), **High-pass** (takes away rumble under the notes) and **Level**. The picture shows a wave through the Drive against the clean wave (dashed).

The Q-Links take Sub, Fifth, Reese detune and Pulse width on the Source page; Cutoff, Resonance, Amount and Decay on the Filter page; Drop, Drop time, Decay and Release on the Shape page; Glide, Fine, Bend range and the pressure's Amount on the Play page; and Drive, Tone, High-pass and Level on the Output page. A Bass track stops working a tenth of a second after its last note has died away, so it takes almost none of the MPC's power until you play it.

**Wind** is a blown instrument: a clarinet, a sax or a flute, played by your breath. As on the real thing, the breath makes the note: blow harder and it gets louder and brighter, blow softly and it gets darker and airier, and with no breath it is silent. **Model** on the Instrument page picks the instrument:

- **Clarinet:** a straight tube with a reed. Hollow and woody, and dark in its low notes.
- **Sax:** a cone with a reed. Fuller and buzzier, and it growls when you push it.
- **Flute:** a pipe you blow across, with no reed. Soft and airy, pure on a gentle breath.

**From** on the Breath page says where the breath comes from:

- **Velocity** (the default): how hard you hit the pad is how hard you blow. Keep pressing on a held pad and you blow harder still, so the note swells and opens under your finger (pressure's first slot, set to Breath; see [Pad pressure](#pad-pressure)).
- **Breath ctrl:** a breath controller or wind controller on MIDI in.
- **Mod wheel:** the mod wheel of a keyboard on MIDI in.
- **Foot pedal:** an expression pedal on MIDI in.

Until the controller moves, velocity plays the notes, so a preset set to a breath controller still plays from the pads.

It plays one note at a time, as a wind player does. Press a new pad before you let go of the last and the note moves there without stopping: the breath carries on and only the fingers change. How hard you hit the new pad is how hard you blow from there, so a legato line can swell or ease without starting a note again. Set **Voices** up to 8 to play chords, as **Reed Section** and **Flute Choir** do. Every note plays about as loud as the next across the keyboard, and in tune. It has four pages:

- **Instrument:** **Model**; **Breath**, how hard the player blows (every source of breath is scaled by it); **Reed**, from a soft reed (speaks easily and buzzes early) to a hard one (needs more breath, rounder), on the clarinet and sax only; and **Edge**, where the flute's air hits the edge of its mouth hole: centred is hollow and pure, off centre brings in the octave above and brightens it, on the flute only. Then **Tone** (dark to bright, the same on every note), **Noise** (the air rushing past the reed or across the edge) and **Growl** (a rough flutter, as when a player hums into the horn). **Vibrato** has its **Depth**, **Rate** and **Delay** (how long into a note before it starts; it fades in over the same time again); the mod wheel adds more, unless the mod wheel is the breath. On the flute the vibrato is mostly in the breath, on the sax mostly in the pitch.
- **Breath:** **From**; **Soft to hard** (how much quieter the softest breath is than the hardest, up to 40 dB); **Tongue** (how much each note starts with a puff of air and a "t", as a player tonguing it); and **Breath now**, which shows the breath as you play. **Over a note** is the breath's **Attack**, **Decay**, **Sustain** and **Release** as faders: how it swells in, settles, and dies away when you let go.
- **Play:** **Voices**; **Over a held key**, what a pad pressed while another is held does (**Slur**: only the pitch moves; **Tongue**: the note starts again); **Priority** (with several pads held, the **Last** pressed, the **Low**est or the **High**est sounds); the glide's **Time** and **Mode** (**Time**: every glide takes that long; **Rate**: an octave takes that long, so small steps are quicker); transpose, fine tuning and bend range; **Level** and **Spread** (with several voices, they alternate left and right).
- **Pressure:** what pressing harder on a held pad does: blow harder (**Breath**, the default), more **Vibrato**, a brighter **Tone**, more **Growl** or **Noise**, or bend the **Pitch** (see [Pad pressure](#pad-pressure)).

Controls that do nothing for the sound as it is are drawn faint: Reed on the flute, Edge on the clarinet and sax, Over a held key, Priority and the glide with more than one voice, the glide's Mode with the glide at 0, Spread with one voice, and Decay with Sustain at 100.

Its pictures: the **Mouthpiece** (how much air gets through the reed as the player blows harder: it rises, then the reed shuts it off; the dot is where a full breath plays it. On the flute, how much of the air goes into the pipe, moved by Edge), the **Tone** (which part of middle C's sound comes through at a full breath, dashed for a soft one), the **Level** (how loud the note is for each breath, on Soft to hard's curve) and the **Breath** over a note.

The Q-Links take Breath, Vibrato, Tone and Noise on the Instrument page; Tongue, Attack, Sustain and Release on the Breath page; and Glide, Bend range, Level and Spread on the Play page. **Hall Flute** and **Smoky Sax** add a hall and four macros: Breath, Vibrato, Tone and Noise. A Wind track stops working a tenth of a second after its last note has died away, so it takes almost none of the MPC's power until you play it.

**Choir** sings. Every note is a few singers singing a vowel, each a little apart in pitch and wavering on their own, through the built-in **Ensemble**, so a chord sounds like a section and not one voice. It plays up to eight notes at once (**Voices** on the Play page, up to 16). Hit harder and a note is louder and brighter, as a singer sings brighter when singing louder. Every vowel is about as loud as the next, and so is every note from the bottom of the keyboard to the top. It has four pages:

- **Choir:** **Vowel** moves smoothly through **Ooh** (0), **Oh**, **Aah** (50, the default), **Eh** and **Ee** (100), and every blend between; **Brightness** makes the voices darker or brighter without making them louder. The picture is the vowel as the voices sing it on middle C: the peaks are what makes it sound like that vowel. Under **Singers**: **Section** (**Bass**, **Tenor**, **Countertenor**, **Alto**, **Soprano**, or **Mixed**, where low notes are sung by basses and high notes by sopranos, with tenors and altos between), **Singers** (how many sing each note, 1 to 8), **Detune** (how far apart their pitches are), **Breath** (breath in the voice, up to a whisper) and **Octave**. Under **Vowel move**: **Start** makes each note start further along the vowels and open into the vowel over **Start time** (below 0 an "aah" opens from an "ooh"), and **Morph** swings the vowel slowly to either side, at **Morph rate**.
- **Shape:** the **Swell**: attack, decay, sustain and release as faders, **Velocity to level** (how much quieter a soft hit is) and **Velocity to tone** (how much darker). Under **Vibrato**: **Depth**, **Rate**, **Delay** (how long after the note starts it comes in) and **Drift** (the small, slow wavering of a real singer's pitch). Each singer has their own vibrato, a little faster or slower and deeper or shallower than the next. The pictures are the swell, and each singer's pitch over the first seconds of a note.
- **Ensemble:** as on Strings: **Amount** (0 is dry), **Lines**, **Width**, and the two wobbles, **Slow rate**, **Fast rate** and **Depth**. The picture shows the lines moving over one slow cycle.
- **Play:** **Mode** (**Poly**; **Mono**, one note at a time; **Legato**, one note at a time where a pad pressed while another is held moves the voice there without starting it again), **Voices** and **Glide** (in Mono and Legato); transpose, fine tuning and bend range; pad pressure: **To swell** (a note you don't press is quieter, and pressing brings it up to full) and **To vowel** (pressing moves the vowel, towards Ee above 0 or towards Ooh below); **Level** and how many notes are sounding.

The Q-Links take Vowel, Singers, Detune and Brightness on the Choir page; Attack, Release, Depth and Drift on the Shape page; Amount, Depth, Slow rate and Width on the Ensemble page; and Glide, To swell, To vowel and Level on the Play page. Controls that do nothing are drawn faint and say why: the ensemble's with Amount at 0, Detune with one singer, Start time with Start at 0, Morph rate with Morph off, the vibrato's with Depth at 0, Glide in Poly. A Choir track stops working a tenth of a second after its last note has died away, so it takes almost none of the MPC's power until you play it.

From a keyboard on MIDI in, Choir answers the sustain and sostenuto pedals, the soft pedal (6 dB quieter and darker), the mod wheel (more vibrato) and a breath controller or expression pedal (volume). See [Pedals and the mod wheel](midi-and-cv.md#pedals-and-the-mod-wheel).

The Synth tab takes its colours from the theme picked in **Prefs > Appearance**, like the rest of MPFree: the sections' colours, the curves and the boxes in the pictures all change with it. The Q-Links keep four colours of their own: blue, green, amber and purple in most themes, deeper shades of them in Light, Slate and AbleTown so they read on their paler panels, and pink, cyan, amber and violet in Synthwave.

{{shot: synth-fm | The FM synth: the algorithm diagram shows which operators modulate which, and the levels show the six operators.}}

> [!NOTE]
> Each synth's sound, edits and all, is saved with the project (see [Saving your work](saving.md#projects)). To keep a sound for other projects, save it as a preset of your own (below).

### Trying and saving presets

Tap the preset's name above the page (it has a small ▾ beside it) and the browser opens on that synth's presets.

{{shot: browser-presets | The browser opened from the Synth tab's title on Poly's presets: Factory picked, Sub Bass being tried, and Pluck (edited) waiting for Cancel. Save preset is at the top.}}

- **Factory** lists the presets that come with the synth, the same as the list on the Synth tab. **Yours** lists the ones you saved. **Recent** and **Favourites** show only this synth's presets. Press **–** or **+** to move between them, or tap one on the left.
- **Turn the data wheel** and each preset plays as you reach it: play the pads to hear it.
- **Push the wheel**, or tap **Load**, to keep the preset you're on. You're back on the Synth tab with it.
- **New track** makes a new track with a synth of its own playing the preset you're on, and selects it, so you can record it straight away. The sound you were trying goes back where it was. With 16 tracks already playing an instrument of their own, the most at once, the line along the bottom says so and the new track plays its kit.
- **Cancel** or **BROWSE** puts back the sound you had when you opened the browser, with your changes, and takes you back to the Synth tab.
- **Save preset** saves the sound you have now in **Yours**. The keyboard opens with the preset's name in it: change it, then tap **Done**. If you already have a preset with that name, MPFree asks before replacing it, and the old one goes to the Trash. The presets that come with the synth are never changed. The title then shows the name you saved it as.

A preset keeps the whole sound: every setting, and the effects and macros it brings. It plays on the synth it was made on. Load a Poly preset while the track plays FM and the line along the bottom says it doesn't play there.

When the track has its own copy of a synth (see above), the browser tries, loads and saves that track's sound. A plugin synth's presets stay in its list on the Synth tab: tapping its name doesn't open the browser.

### Presets with their own effects

Some presets bring their own effects (a phaser, a wah, a reverb, an amp) and a few **macros**: knobs that each turn several settings at once, named for what you hear, such as **Tone**, **Space** or **Drive**.

- **The Macros page comes first**, and Q-Links 1 to 4 turn the first four macros. Turning a macro moves everything it is tied to together (Space on Space Strings turns up both its echo and its reverb). A setting you then turn by hand stays where you put it until the macro moves again.
- **The effects' settings** are on the last pages, under **FX 1** and **FX 2** for the first and second effect.
- **The effects belong to the sound**, not to the track: the track's insert effects on the mixer stay as they are. Loading a preset without effects takes them away, and a track playing its own copy of the synth gets them with the preset.
- The project keeps the sound with its effects and macros.

## The plugin's own screen

When the selected track plays an MPC plugin instrument that has a skin, the Synth tab shows that skin, as its makers drew it for the MPC: the same pictures, knobs, buttons and text, at the same size.

- **Tap and drag** the skin's controls as you would on an MPC. A double tap on a knob or a menu opens its larger editor over the screen; a tap outside it closes it. The larger editor is MPFree's own, in your theme's colours (Prefs > Appearance), and a menu lists every choice by name; the skin itself keeps its makers' colours in every theme.
- **The buttons under the skin** are its tabs, one per function key as on an MPC. A tap shows that tab; when a key has more than one tab (its button reads "Osc  1/2"), another tap goes to the next.
- **Q-Links 1 to 4** turn the knobs the plugin's makers chose for the tab. A tap on a control picks the group of knobs it belongs to, as on an MPC. The skin shows the change.
- **The wheel** picks the skin's controls one after another; push it and turn to change the one picked (see [The screen](../getting-started/the-screen.md#the-wheels-outline)).
- **MPFree page** shows the plugin's controls on MPFree's own Synth page instead, with its presets and the Q-Link column. **Plugin skin** under the instrument card goes back.

The values and their text ("20 %", "Pan Delay") come from the plugin itself, a few times a second, so the skin also follows a preset you load.

A plugin without a skin shows on MPFree's own page. MPC plugin effects show their knobs as a list in the chain editor for now; their skins come next.

## Playing the synth on the pads

With no scale, the pads are laid out like a keyboard: pad 1 is C3 and each pad after it is a semitone higher, up to D#4 on pad 16. On the Play page, hold **SHIFT** and turn the data wheel to move the whole pad range up or down an octave at a time (from C0 to C6 at pad 1).

Choose a scale and the pads can help you stay in key. Press **MENU** on the Synth tab:

| Setting | What it does |
|---|---|
| **Scale** | The pads' scale, or Off. Nearly 50 to choose from, as for the arpeggiator. |
| **Scale root** | The scale's first note. |
| **Pad layout** | How the pads use the scale (below). Each pad bank keeps its own. |
| **In key rows** | For In key: whether the pads climb the scale in sequence, or each row starts a fourth above the one below, so a chord shape is the same anywhere on the pads. |

There are three pad layouts:

- **Chromatic**: every semitone, as without a scale, but the pads are lit by the scale: the root, the other notes in the scale, and the notes outside it in a dimmer colour.
- **In key**: only the scale's notes, starting on the root. Every pad is a "right" note.
- **Chord pads**: each pad plays a whole chord from the scale: triads on the bottom rows, sevenths on the top rows.

With a scale on, the four pad banks start out as four different layouts, so you can switch between them with the **PAD BANK** buttons as you play: **A** Chromatic, **B** In key, **C** Chord pads, **D** In key an octave higher.

The **Chromatic / In key** switch in the pads heading on the Synth tab flips the current bank between the two with one tap. With no scale picked yet, **In key** picks **Major** on the Scale root (C to start), so the pads change at once; the line at the bottom of the screen says which scale they play. Change the scale in the Synth tab's options (**MENU**, **Scale**).

{{shot: play-synth-in-key | The Poly synth in A minor, In key, on the Play page. Every pad plays a note of the scale, and the pattern's rows are named by note.}}

> [!TIP]
> Drums is a kit rather than a keyboard: pad 1 plays the first drum, pad 2 the second and so on, whatever the scale, layout or octave.

## The synth's notes on the Sequencer

With the synth on the pads, the pattern on the **Sequencer** tab's **Steps** (and the rows on the Play page) shows the pads' notes as rows, the highest on top, like a piano roll turned to the scale: with a scale on, the rows are its notes, and the root's rows are shaded and marked **root**.

{{shot: sequencer-steps-synth | The Sequencer's Steps on a synth track in C minor: the rows are the scale's notes from C3 to C4, the roots shaded. Step 1 holds a C minor triad, with an accent, a 50 % note and a 1:2 note later on.}}

- **Tap a step in a row** to put that note there, and tap more rows on the same step for a chord. Tap a lit step to take it out (**UNDO** brings it back), or hold your finger on it to pick it for the Q-Links and the step inspector. **ERASE** + a tap takes it out too.
- **Hold a row's name** to hear its note.
- **‹** and **›** beside **Oct** show the rows an octave lower or higher. The pattern starts from the root under the lowest note you've used.
- **Fold** shows only the notes the pattern uses, so a bass line or a chord part fits on one screen. Tap it again for every note.
- **ERASE** held + a row's name clears that note's steps; **UNDO** brings them back.

Turning **Timing strip** off in the Sequencer's options (**MENU**) gives the pattern room for eight rows, a whole octave of a seven-note scale.

## Steps on the synth

Press **STEP SEQ** with the synth on the pads and the 16 pads become the steps of the synth's whole line: each step can hold a note or a chord, and every note on it comes and goes together.

1. **Pick what to play.** Hold **STEP SEQ** and play a pad: you hear it, and new steps play that note. Hold two or more pads together (up to eight) and new steps play the chord. On the Chord pads layout one pad is already a chord. Until you pick, new steps play the pad you played last. The chip at the top of the screen names it, such as **Pads: steps · Poly · C3 + G3**.
2. **Hit an empty step** to put the note or chord there, as loud as you hit it. While the transport is stopped you hear it.
3. **Hit a lit step and let go** to take the whole step off.
4. **Pick another note** at any time the same way, holding **STEP SEQ**: the steps already there keep theirs.

On the Sequencer's Steps the line is the lane on the pads, named **Line**. Tap an empty step there to put the note or chord on it; tap a lit one to pick it, and tap it again to switch the whole step off, kept, until a tap brings it back.

**Changing a step's notes.** Hold a step and turn **Q-Link 1**: its card says **Note** (or **Chord**) and the lowest note, and each click moves the step a pad up or down, a chord keeping its shape. Or, with a keyboard on **MIDI IN** or the USB socket, hold a step and play: the keys you hold together go on the step in place of what it had, each as loud as you played it. Hold several steps to give them all the same notes. The keys play the pads' notes, so with a scale on, a key outside it (or past the pads' range) can't go on a step, and the hint line says so. **UNDO** takes the whole chord back in one go.

Everything else works as on a drum's steps (see [Your first beat](sequencer.md#the-pads-as-steps-step-seq)): the other Q-Links set the step's velocity, length and chance for every note on it, and a chord with a chance or a condition plays whole or not at all, never a random part of it; press **Q-LINK** for its condition, ratchet and nudge, and again for **Sound** to lock the synth's knobs on that step; hold a step and hit a later one for a long note; **COPY** and **ERASE** copy and clear whole steps; **UNDO** takes any of it back. Tap a step of the **Line** lane on the Sequencer and the step inspector names its notes ("Step 1 · E3 + G3").

## Tuning

Out of the box every synth and kit plays in standard tuning: twelve equal steps to the octave, with the A above middle C at 440 Hz. You can change that for the song in the Synth tab's options (press MENU on the Synth tab; see [Controls](../reference/controls.md)): the synths and the kits' keygroups follow it.

{{shot: synth-options | The Synth tab's options, with the Tuning, Tuning root and A4 rows.}}

- **Tuning** picks how the notes are tuned:
  - **12-TET** is standard tuning, the same in every key.
  - **Just (5-limit)** makes thirds and fifths beat-free and sweet, in keys near its root.
  - **Pythagorean** has pure fifths, for drones and open, medieval-sounding chords.
  - **Meantone 1/4 comma** (quarter-comma meantone) has pure major thirds, as on Renaissance and early Baroque keyboards.
  - **24-TET quarter tones** has 24 equal steps to the octave, so each step up is half a semitone.
  - After those come your own tuning files, by file name (see below).
- **Tuning root** is the note the tuning is built on. Just, Pythagorean, meantone and your own files sound purest in keys near their root, so set it to your song's key. The two equal tunings sound the same in every key and show "C (not used)".
- **A4** is the pitch of the A above middle C, from 380 to 480 Hz, so the synth can match a recording or another instrument. 432 Hz and 442 Hz are common choices.

The drum synth keeps its own pitches whatever you pick here.

Kits follow it where they play notes:

- **Keygroups** (pads that play notes, see [Playing a kit on keys](shaping-a-pad.md#playing-a-kit-on-keys)) play every key at the tuning's pitch. MPFree takes a keygroup's sound to be at its root note's standard pitch, so A4 = 432 Hz moves a keygroup just as it moves a synth.
- **16 Levels by Tune** steps by the tuning's notes from the pad's root note (C4 on a pad without keys), with pad 9 as recorded.
- **The pitch wheel** moves a keygroup's notes along the tuning's notes, as it does a synth's.
- **A pad you hit**, and a slice, plays as you tuned it on the pad, whatever the tuning, as the drum synth does.

Notes already sounding move to a new tuning as you pick it, gliding there if the pad glides. [Auto-sample](sampling.md#auto-sample-a-synth-or-your-gear-onto-the-pads) records the synth in standard tuning, so the keygroups it makes play the tuning once, not twice.

Everything else in the Synth tab's options (the scale, pad layouts and tuning) is the song's: it is saved in your project and comes back when you open it, and New project starts chromatic in C at A4 = 440 Hz (see [Saving your work](saving.md)). If a tuning file or one of your own scales is gone by then, that one choice goes back to its default, and the Console says so.

### Your own tunings

MPFree reads Scala tuning files, the format thousands of published tunings come in. Put them on the MPFREE-DATA partition of your MPFree stick, in the folder `files/tunings/`, and start MPFree. They join the Tuning row after the built-in tunings, in name order, up to 64.

- **A scale** is a `.scl` file. Each step up the keyboard (each pad up, with the Chromatic pad layout) plays the next note of the scale, starting from the Tuning root, and the root keeps its usual pitch for the A4 you pick, just as with the built-in tunings. A scale needn't have 12 notes: a 19-note or a 7-note scale works, and so does one that repeats at something other than the octave.
- **A keyboard map** is a `.kbm` file with the same name beside the scale (`dorian.scl` and `dorian.kbm`). It says which key plays which note of the scale and what pitch the tuning sits at, so with one the Tuning root and A4 rows show "set by its .kbm".
- **Names** must be lower case: letters, digits, `.`, `_` and `-` only, such as `werckmeister3.scl`. A file named any other way isn't seen. The row shows at most 20 characters of a name.
- MPFree only reads the folder; it never changes your files. A file it can't read is left out, and the Console (see [Controls](../reference/controls.md)) says which and why, and lists the files it found.

## Pad pressure

The MPC's pads sense how hard you keep pressing after the hit. On Poly, FM and Wind the **Pressure** page sets what that does; on Bass it is **Moves** and **Amount** on the Play page, on Analog **Press to** and **Pressure** on the Mod page, on Choir **To swell** and **To vowel** on the Play page, and Squelch, Keys, Drawbar, Strings, Pluck and Drums don't answer it. By default on Poly, pressing harder opens the filter, so a held chord brightens as you lean into it; on Wind it blows harder, so a held note swells and opens. You choose up to four things for pressure to change, how much, and how hard you must press before it starts. On Poly, pressure can also bring in either LFO (**LFO 1 amount**, **LFO 2 amount**) or move each note across the speakers (**Pan**).

Clips record pad pressure too, so a recorded part plays back with the same swells: see [Pad pressure](clips.md#pad-pressure) in Clips and patterns.

## Velocity

How hard you hit a pad sets how loud the note plays, as with drums. **FULL LEVEL** and Half Level, and the velocity curve in [Prefs](../reference/prefs.md), work the same for the synth.

Note Repeat, Latch and the arpeggiator all play the synth too: see [Note Repeat, Latch and the arpeggiator](note-repeat-and-arp.md).

> [!MPC]
> The MPC's plugin instruments (such as its bass and electric-piano plugins) and expansions are not in MPFree. MPFree has its own twelve synths instead, with their controls on one tab and the Q-Links following each preset, and it can host CLAP plugin synths and the MPC plugin instruments people share, such as those in the Open MPC catalog. As on the MPC, where each track plays a program, each track can play its own instrument; a track without one plays its kit (see [A kit for each track](pads.md#a-kit-for-each-track)). The scales and chords of the MPC's Pad Perform mode are replaced by the scale settings and the per-bank pad layouts here: switch layouts with the PAD BANK buttons without leaving the page you are on.
