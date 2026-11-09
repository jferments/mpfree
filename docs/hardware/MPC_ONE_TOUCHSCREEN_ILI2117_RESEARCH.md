# MPC One touchscreen: Ilitek ILI2117, evdev, Rockchip DSI and Panfrost

**Status:** Based on existing owner's read-only factory MPC One 3.6.0 hardware surveys. The expanded touchscreen collector has **not yet been run** on the device. Factory app remains owner of DRM/evdev/audio. No I²C messages, evdev event reads, DRM opens, modesets or display changes are implemented here.

## Currently observed on the real MPC One

| Part | Observation | Still unknown |
| --- | --- | --- |
| Touch controller | `ILI2117 Touchscreen` | Exact bound driver, firmware revision, coordinate range, active multitouch slots |
| Bus | I²C4 via `/devices/platform/ff160000.i2c/i2c-4/4-0026/input/input0` | Whether the chip has optional properties for touchscreen inversion, swapped axes or reset IRQ |
| Input | `/dev/input/event0`, BUS_I2C, bustype `0018` | Exact `ABS_X/ABS_Y`, `ABS_MT_POSITION_X/Y` and tracking IDs; event handler order may change across boots |
| Other buttons | `/dev/input/event1`, `gpio-keys` | Exact key mapping and role of physical power control |
| Panel | `/sys/class/drm/card0-DSI-1`, 800×1280 native mode | Landscape transform, panel rotation and ownership details |
| Framebuffer | `rockchipdrmfb`, `/dev/fb0`, virtual 800×3840, 32 bpp | Triple-height virtual framebuffer may mean multiple buffers; not proof of triple-buffer algorithm |
| GPU | `arm,mali-t760` compatible, Panfrost, renderD128 | Actual EGL/GLES versions and performance; no graphics test on-device yet |
| Stock app | Factory `MPC` opens `/dev/dri/card0`, `/dev/tty0` and evdev touch/GPIO | Cannot claim DRM master/input ownership while factory app runs |

## What upstream Linux says (not necessarily the same driver version as az0x)

Source: https://github.com/torvalds/linux/blob/master/drivers/input/touchscreen/ili210x.c

Upstream Linux `ili210x.c` supports device-tree identifier `ilitek,ili2117`, parses a **43-byte** I²C touchscreen report with a checksum, and defines `ili211x_chip` with **max_touches=10** and **resolution=2048**. It exposes events via Linux input/evdev, making a custom I²C driver likely unnecessary.

**Do not assume** the vendor's Linux 6.6.95 kernel has the exact same capabilities/driver implementation. Device-tree inversion, swap and active panel geometry may change final evdev values. Read the bound driver and actual event capability bitmaps first. A later explicitly reviewed metadata-only EVIOCGABS ioctl can retrieve the actual axis min/max/flat/fuzz (without consuming touch events or grabbing the device), but that is **not** part of this current script.

## Safe on-device survey (no opens of hardware character nodes)

~~~bash
cd /home/jesse/code/mpfree-hardware-survey
git pull --ff-only origin main
ssh-add /home/jesse/.ssh/mpc_one_autorun_ed25519
bash tools/mpc-one/collect-over-ssh.sh --touchscreen
~~~

The survey reads and reports the input device list/handlers/evdev bitmap capabilities, I²C binding and device tree from `4-0026`, current DSI connector modes, framebuffer virtual dimensions, GPU sysfs driver, backlight settings, input/display interrupts, and currently held factory DRM and input **fd symlinks**.

It **does not open** `/dev/input/event0`, `/dev/i2c-*`, `/dev/dri/card0`, `/dev/fb0` or any touchscreen-controller firmware interface. No polling/touch gesture data is captured; no DRM master, calibration, reset, brightness or I²C writes.

Raw reports stay in locally ignored `research/` and should be reviewed/redacted before sharing. Reports do not include user files, raw proprietary data, device serials, argv or process environment.

## Coordinate mapping: physical landscape GUI vs native portrait panel

Our eventual GUI will use logical coordinates with **1280×800 landscape orientation**. The panel's native DRM timing reports **800×1280 portrait**. Raw input may be rotated/inverted relative to those values.

One reasonable representation is a calibrated affine transform:

`x_ui = a * x_raw + b * y_raw + c`

`y_ui = d * x_raw + e * y_raw + f`

where raw coordinates `x_raw, y_raw` come from evdev, UI coordinates `x_ui, y_ui` are logical landscape pixels, and the six coefficients represent scale, rotation, axis swap/reflection and translation.

Calibrate from known points after safe touch event capture is separately authorized: at least three noncollinear points, preferably four or more with measured residual error. Do not hardcode a 90-degree rotation or 0–2047 range without measuring evdev axis properties on the real unit.

Independent prior art: https://github.com/nzimas/granola-openmpc reports four-corner touch calibration for the original MPC One because the native portrait panel was mounted rotated; its developer reports about six pixels of residual error. That project's firmware and calibration are **not validated substitutes** for measurements from ours.

## MPFree adapter split and later test gates

- `TouchMetadata`: sysfs/input capability inspection, allowed while factory remains active.
- `TouchBackend::EvdevILI2117`: later, after safe read permissions/ownership are qualified, use ABS_MT_SLOT, ABS_MT_TRACKING_ID, ABS_MT_POSITION_X/Y, SYN_REPORT and SYN_DROPPED handling; normalize and calibrate. Never use EVIOCGRAB against a live stock GUI.
- `DisplayBackend::RockchipDRM`: after a safe application handoff and verified absence of the factory DRM master, test a single short-lived known-color/test-card dumb buffer; close all DRM fds and relinquish ownership before stock restart. EGL/GLES and GPU acceleration later.
- `ControlSurfaceBackend`: separate internal USB MIDI for pads/encoders/lights; GPIO keys (including power safety) are independent inputs.

Next: run the read-only survey, then plan metadata-only EVIOCGABS and screen-coordinate calibration. **Only after independent recovery** and safe factory unit restoration proof should a temporary MPFree GUI take over the physical panel.

Our goal is a new factory-application replacement running on the **existing** Linux kernel first, not a different Linux kernel or device firmware.
