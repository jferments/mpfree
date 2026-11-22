# MPFree

**Free, open-source software that turns an Akai MPC One into a new instrument.**

![MPFree's Play page in the middle of a jam: six loops going along the top, a beat in the pattern, the Stutter throw held and the master meter moving.](docs/images/play-all-loop.png)

MPFree is a sampler, drum machine, synth, looper and sequencer that runs on the MPC One itself, with no computer attached. It keeps what makes an MPC fun to play: hitting pads, turning knobs and building a beat with your hands. Around that it adds a looper you can play like an instrument, twelve built-in synths, clips and scenes for playing live, and screens that show what they do, with fewer menus to dig through.

It is made for beatmakers and live performers alike, and it is free to use, change and share.

## Where it stands

MPFree is early test software, and there is no public release yet. It runs from a USB stick that you make on your computer with the MPFree USB creator: the MPC starts MPFree from the stick, and the MPC's own system and software are left exactly as they were. [Make an MPFree USB stick](#make-an-mpfree-usb-stick), below, walks you through it.

Most of what is described below is built and checked on a computer by MPFree's own automated tests. Trying it on the MPC itself is under way, one feature at a time.

## Make an MPFree USB stick

Until there is a release to download, you make the stick from this repository on a Linux computer. One command builds the MPFree USB creator with the latest MPFree inside it, and the creator writes the stick. It takes about 20 minutes the first time, most of it waiting.

> [!WARNING]
> MPFree has only been tested on the **MPC One (Gen 1)**, the original MPC One. Everything on the USB stick is erased. The MPC's own system and software are not changed: switch it on without the stick and the factory software starts as before.

### What you need

- An **MPC One (Gen 1)** updated to **MPC 3.10.1**, with Akai's own instructions.
- Akai's **USB update file for MPC 3.10.1**: a single `.img` file from Akai's download page. When the page asks how you want to update, choose **USB**, not PC or Mac. MPFree never comes with any of Akai's files: the creator uses your copy.
- A **USB stick of 16 GB or more**. 32 GB is better: MPFree keeps your work and a backup of your MPC (about 8 GB) on it.
- A **Linux computer** (a 64-bit PC) with an internet connection. The creator itself also runs on Windows and macOS, but building it needs Linux.

### 1. Install the tools (once)

On Debian or Ubuntu, in a terminal:

```sh
sudo apt install git curl build-essential python3 gcc-arm-linux-gnueabihf libc6-dev-armhf-cross
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

The second line installs Rust. Accept its defaults, then close the terminal and open a new one. On another Linux, install the same tools with its package manager.

### 2. Get the code

```sh
git clone https://github.com/jferments/mpfree.git
cd mpfree
```

### 3. Build the USB creator

```sh
tools/build-usb-creator.sh
```

The first build takes about 10 minutes. It ends with `DONE: the USB creator is ready` and the line that opens it. If a tool is missing, it stops with `STOP:` and says what to install. If your Linux is newer than the MPC's, it asks you to run one more line once, then to run it again.

### 4. Make the stick

Put the USB stick into the computer, then open the creator:

```sh
rust/target/usb-creator/mpfree-creator
```

1. **Akai's update file:** the creator looks in your Downloads, Desktop and Documents folders and shows the file it found, with a green tick and **MPC 3.10.1 (MPC One, Gen 1)**. If your file is somewhere else, drag it onto the window, or type the full path of the file or its folder and press Enter.
2. **Your USB stick:** choose your stick. The creator lists only USB sticks, never your computer's own disks.
3. **Make the stick:** leave the two boxes under **On the first start** ticked. Click **Make the stick**, tick **Yes, erase this stick**, click **Erase and make the stick**, and give your password.

After a few minutes the creator says **The stick is ready**. Eject the stick and take it out.

### 5. Start MPFree on the MPC

The first time, the MPC needs a small start-up helper, which you add with one tap:

1. Switch the MPC on without the stick. Press **MENU**, open **Preferences**, then **Enhancements**, and switch on the **developer panel**. Then hold **SHIFT**, press **MENU**, choose **Developer**, and switch developer mode on.
2. Put the stick into one of the MPC's USB ports. When the MPC asks whether to run `autorun.sh`, tap **Yes**.
3. The MPC switches developer mode off again and restarts by itself into MPFree. The first start backs up your MPC onto the stick and brings in its drum kits and samples, which takes 20 to 30 minutes, once. Keep the stick in until the pads light up.

From then on, switch the MPC on with the stick in to play MPFree, and without it for the factory software. [Installing MPFree](docs/user/src/getting-started/installing.md) has every step in detail, including checking that developer mode is off afterwards and taking MPFree off the MPC again.

### Updating to a newer MPFree

```sh
cd mpfree
git pull
tools/build-usb-creator.sh
rust/target/usb-creator/mpfree-creator
```

In the creator, choose the same stick. It sees MPFree on it, and step 3 becomes **Update MPFree on this stick**. Leave **Keep my backup, settings and files** chosen, click **Update MPFree**, and give your password. Your settings, work and backup stay on the stick, so there is no long first start, and you don't need developer mode again.

## What you can do with it

MPFree is laid out as eight tabs across the top of the screen. Each name below links to its chapter of the manual.

- **[Play](docs/user/src/making-music/playing-live.md)** is the home screen. Your pads, the beat as rows of steps, your loops, an XY pad, and **throws**: effects you play like an instrument, such as Stutter, Tape stop and Reverb wash. Hold one and it comes in, let go and it stops.
- **[Sequencer](docs/user/src/making-music/sequencer.md)** is for building patterns step by step. Steps can be told to play only every other time or only on a fill, and each step can change a sound's settings just for that hit. There is also a piano roll, a note list and a tracker view.
- **[Looper](docs/user/src/making-music/looping.md)** has eight loops you record from the pads, a synth or the line input. Overdub layers, undo them, play a loop in reverse or at half or double speed.
- **[Sample](docs/user/src/making-music/sampling.md)** puts the whole sampler on one page. Record, trim, chop a break onto the pads, stretch it to the song's tempo, or turn it into a cloud of grains. **Keep last 4 bars** catches what you just played, even if you weren't recording.
- **[Synth](docs/user/src/making-music/synths.md)** holds twelve synths: Poly, FM, Drums, Pluck, Keys (electric piano and clav), Drawbar organ, Squelch acid bass, Analog, Strings, Bass, Wind (clarinet, sax and flute) and Choir. Any track can have its own.
- **[Arrange](docs/user/src/making-music/arranging.md)** launches clips and scenes for playing live, then lays them out as a song, with audio tracks and takes.
- **[Pads](docs/user/src/making-music/pads.md)** sets what each pad plays: a sample, a loop that stays in time, a synth note or a note to your other gear.
- **[Mix](docs/user/src/making-music/mixing-and-effects.md)** is the mixer, with an effect chain on every channel (compressors, EQ, delays, reverbs and more, with sidechain), plus your MIDI and CV connections and a monitor that shows every message coming in and going out.

Beyond the tabs:

- **[Other gear](docs/user/src/making-music/midi-and-cv.md).** Play it from MIDI keyboards over 5-pin MIDI or USB, including MPE. Turn a knob on your controller and MIDI Learn hands it the setting you touched. Sync to MIDI clock or send it, and play modular synths with CV and gate.
- **[Plugins](docs/user/src/making-music/plugins.md).** It hosts CLAP plugins and the MPC-format plugins people share. Each one runs apart from the rest of MPFree, so a plugin that crashes can't stop the music.
- **[Saving](docs/user/src/making-music/saving.md) and [exporting](docs/user/src/making-music/exporting.md).** Projects save with their samples and recordings, autosave every two minutes and keep the last ten saves. Songs export as a mixdown, as stems or as a MIDI file.
- **Staying in time.** If the MPC can't keep up, MPFree fades out the quietest notes instead of glitching, and tells you it did.

## Documentation

The docs are split by who they are for. [The docs index](docs/README.md) has all three.

**For musicians: [the user manual](docs/user/src/index.md).** It starts from zero, so no MPC experience is needed, and it builds as a website and a printable PDF. Good places to start:

- [Before you start](docs/user/src/getting-started/before-you-start.md) and [Installing MPFree](docs/user/src/getting-started/installing.md)
- [Your first beat](docs/user/src/getting-started/first-beat.md) and [your first loop](docs/user/src/getting-started/first-loop.md)
- [Coming from an MPC](docs/user/src/reference/coming-from-an-mpc.md): where MPFree works differently from the factory software
- [Buttons and controls](docs/user/src/reference/controls.md): every button and combination in one place
- [Moving your beats from the MPC](docs/user/src/making-music/moving-from-the-mpc.md)

**For developers: [the developer docs](docs/dev/README.md).** How MPFree is designed and why, with a page for each part of the app. Good places to start:

- [Architecture](docs/dev/project/architecture.md): how the pieces fit together
- [Roadmap](docs/dev/project/roadmap.md): what's done, what's next and what has to be proven first
- [Testing and CI](docs/dev/guides/testing-and-ci.md): the tests, and what every pull request is checked for
- [mpfree on the test build](docs/dev/guides/mpfree-app.md): running the app on an MPC One

**For AI agents** working on the code: [the agent docs](docs/agents/README.md) and [CLAUDE.md](CLAUDE.md).

## Building it

MPFree is written mainly in Rust. On a Linux computer with stable Rust installed, this builds everything and runs the tests:

~~~sh
cargo test --manifest-path rust/Cargo.toml --workspace --locked
~~~

`tools/build-usb-creator.sh` builds the USB creator ([Make an MPFree USB stick](#make-an-mpfree-usb-stick) above; [Building the USB creator](docs/dev/guides/build-usb-creator.md) has what it does and how it differs from a release). [Testing and CI](docs/dev/guides/testing-and-ci.md) covers the rest of the tests, and [mpfree on the test build](docs/dev/guides/mpfree-app.md) covers running it on an MPC One.

## License

MPFree is under the [MIT license](LICENSE). That covers MPFree's own code and documentation. Its built-in fonts, Inter and JetBrains Mono, are under the [SIL Open Font License](rust/mpfree-ui/fonts/README.md).

MPFree is not made by or connected with Akai Professional. It does not include or share Akai's firmware, plugins, presets or sounds, and it does not get around their activation. See the [factory content policy](docs/dev/project/factory-content-policy.md).
