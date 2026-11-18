# USB-B: MPFree as a USB MIDI device for a computer

**As of 2026-11-18** · **Asked for by** the owner, 18 Nov 12:09Z: "write that up in the next device round so that we can understand how to set up USB MIDI. It's an important part of making music." · **Code:** [`tools/mpc-one/read-only-usb-device.sh`](../../../tools/mpc-one/read-only-usb-device.sh), run by `collect-over-ssh.sh --usb-device` · **Status:** the read-only check below waits for the next device round; nothing that writes to the USB-B port is built.

MPFree already takes keyboards, interfaces and USB synths on the **USB-A** socket, where the MPC is the host ([MIDI I/O](../design/midi-io.md)). This page is about the other socket: **USB-B**, where the MPC is the device and a computer is the host, so a DAW could play MPFree's sounds, record its pads and send it clock. Before anything is built, one read-only check on Akai's system answers how that socket is set up today.

## What is known

| Fact | Status | Where from |
|---|---|---|
| A USB device controller `ff580000.usb` (driver `dwc2`, up to high speed) is registered; it read `not attached` with nothing plugged in | OBSERVED, 3.6.0, 9 Nov | [Ethernet and USB](ethernet-usb.md), section 3 |
| `/sys/kernel/config/usb_gadget` exists, so configfs gadgets are possible | OBSERVED, 3.6.0, 9 Nov | [Ethernet and USB](ethernet-usb.md) |
| The USB-B port "allows MPC One to send/receive MIDI and audio data to/from the MPC software on your computer" | DOCUMENTED | MPC 3.6 user guide, MPC One rear panel |
| In Standalone Mode the toolbar's In and Out boxes show MIDI "receiving or sending … from or to your computer" | DOCUMENTED | MPC 3.6 user guide, toolbar |
| Controller Mode is a separate mode, switched from the toolbar, which closes the project | DOCUMENTED | MPC 3.6 user guide, Mode |
| The same socket carries Akai's desktop updater, and factory U-Boot's Fastboot update mode before Linux starts | OBSERVED (device tree) and DOCUMENTED | [Boot architecture](mpc-one-boot-architecture.md) |

**Not known:** whether Akai's system sets up a USB MIDI function (Linux's `f_midi`) on USB-B in Standalone Mode, or does it another way (its own program through functionfs); which program holds it; whether the kernel has the gadget MIDI and audio functions built in; and what a computer sees when it is plugged in. The [Ethernet and USB](ethernet-usb.md) page's rule stands until this check is read: **no binding, unbinding or writing a USB gadget configuration.**

## The check for the next device round

All steps are read-only on the MPC. Plugging the cable is the socket's normal use, as Akai's guide describes it, and unplugging it is the undo. The MPC stays on Akai's system, in Standalone Mode, with developer mode's SSH on port 22, as for `--drivers`.

| Step | Where | Kind | What you do | About |
|---|---|---|---|---|
| 1 | Workstation → MPC | read-only | Nothing in the USB-B socket. Run the survey (block 1) | 1 min |
| 2 | MPC | the socket's normal use | Plug a USB cable from the MPC's USB-B socket into the workstation, while Akai's app is running. If a window asks about Controller Mode, tap **Cancel**: this check stays in Standalone Mode | — |
| 3 | Workstation → MPC | read-only | Run the survey again (block 1) | 1 min |
| 4 | Workstation | read-only | What the workstation sees (block 2) | 1 min |
| 5 | MPC and workstation | read-only (optional) | Only if Akai's **Preferences › MIDI / Sync** lists an output port for the computer: set a MIDI track's output to it (don't save the project), run block 3, and hit a few pads within 10 s | 2 min |
| 6 | MPC | undo | Unplug the cable. Nothing else changed | — |

**Block 1** (read-only, on Akai's system):

~~~sh
bash tools/mpc-one/collect-over-ssh.sh --usb-device
~~~

It writes `research/mpc-one-usb-device-<time>.txt` on the workstation, and prints its first lines. Each section names what it read:

- `USB device controllers`: `ff580000.usb` with its `state` (`not attached` with nothing plugged; `configured` once a computer has set it up) and, on newer kernels, the gadget on it (`function`).
- `Gadgets set up in configfs`: each gadget Akai's system made, the socket it's bound to, its vendor and product, its configs and the functions they use (`midi.*` with its ports, `uac2.*` with its channels and rates; only the names of the others). A gadget's serial number and a network function's addresses are never read.
- `Build settings for being a USB device`: `CONFIG_USB_CONFIGFS_F_MIDI`, `CONFIG_USB_CONFIGFS_F_UAC2` and the rest, from `/proc/config.gz`.
- `Gadget modules`, `configfs and functionfs`, `Sound cards`: a gadget MIDI function shows as a sound card on the MPC (`f_midi`).
- `Programs holding a gadget endpoint or a MIDI or sound device`: which program has the gadget's card or endpoints open.
- `Start-up files that mention the USB device port`: the scripts and units that set the gadget up, with their matching lines (none about a serial number); a program's name only.
- `Kernel log about the USB device port`.

If the SSH step fails, nothing on the MPC changed; the wrapper keeps the report for diagnosis and says what to check (developer mode on, the host key recorded with `--devmode-trust`).

**Block 2** (read-only, on the workstation, after step 2):

~~~sh
lsusb | grep -i -e akai -e 09e8
lsusb -v -d 09e8: 2>/dev/null | grep -v -i serial | grep -E 'idProduct|iProduct|bcdUSB|bNumConfigurations|bInterfaceClass|bInterfaceSubClass|iInterface|MIDIStreaming|AudioStreaming|Jack'
cat /proc/asound/cards
amidi -l
aconnect -l
~~~

Expected: an Akai device (vendor `09e8`) in `lsusb`; if its interfaces include `Audio` with subclass `MIDI Streaming`, the computer sees the MPC as a class-compliant USB MIDI device, and `amidi -l` and `aconnect -l` list its ports by name. If nothing new appears, Akai's system offers nothing on USB-B in Standalone Mode. `lsusb -v` may say it couldn't open the device without `sudo`; the lines above are still printed. The serial number is filtered out.

**Block 3** (optional, read-only, on the workstation; `N` is the card number of the MPC's port in `amidi -l`, such as `hw:2,0,0`):

~~~sh
timeout 10 amidi -p hw:N,0,0 -d
~~~

Expected: a note-on and a note-off (`90 …`, `80 …`) for each pad hit. Copy the output into the round's notes.

**What to send back:** both reports from block 1, and the terminal output of blocks 2 and 3. They hold no serial number or MAC address.

## What the answers decide

- **Akai's system already sets up a MIDI function on USB-B** (a `midi.*` function, an `f_midi` sound card, a `MIDIStreaming` interface on the computer): MPFree may be able to open that sound card as one more MIDI port, with no change to the USB setup. That would be MIDI I/O's ports and routes as they are, plus one port named for the computer. Whether that card is still there under MPFree's own system on the stick is the next question, answered the same way with the stick's kernel config, offline from its build.
- **It's done by Akai's own program** (functionfs endpoints held by Akai's app): MPFree would have to set up its own gadget. That is a state-changing step (configfs writes, RAM only, gone at power-off) and gets its own rule zero review: the exact writes, the undo (unbind, then remove what was made), what happens if it fails half way, and proof that it leaves the updater's Fastboot path alone. It waits for the owner's go.
- **The kernel has no gadget MIDI** (`CONFIG_USB_CONFIGFS_F_MIDI` not set and no `usb_f_midi` module): USB-B MIDI needs MPFree's own kernel ([own kernel](own-kernel.md)).

## Rule zero

Every step above only reads, apart from plugging and unplugging a cable in the socket made for it. The survey script never mounts, loads or unloads modules, binds or unbinds a gadget or a driver, creates or removes anything in configfs, opens a MIDI or sound device, or writes a file; a test checks the script's text for each of those ([`test_read_only_usb_device.py`](../../../tools/mpc-one/tests/test_read_only_usb_device.py)), and runs it under BusyBox against a fake system. No USB gadget write is proposed here.
