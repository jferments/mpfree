# MPC One Panfrost GPU qualification (offscreen only)

This folder preserves the **owner-tested** ARMv7 GPU qualification probes and
workstation launchers. The MPC One tested on 2026-11-09 used RK3288,
Mali-T760/Panfrost, Mesa 24.0.7, and OpenGL ES 3.1.

**Safety boundary:** These programs do *not* open `/dev/dri/card*`, acquire
DRM master, set a display mode, inject touch events, access audio devices,
modify firmware, or change systemd services. They render to offscreen buffers.
They run **only when explicitly invoked** by the owner.

## Workstation setup

On Ubuntu/Kubuntu, install the ARMv7 hard-float cross-compiler (on the
**workstation**, not on the MPC):

```bash
sudo apt install gcc-arm-linux-gnueabihf
```

From the repository root:

```bash
git pull --ff-only origin main
bash tools/mpc-one/gpu-qualification/mpfree-gpu-qualify.sh
bash tools/mpc-one/gpu-qualification/mpfree-gpu-bench.sh
```

Launchers connect with key-based SSH to `root@10.42.0.141:2222` by default.
Override without editing files:

```bash
MPFREE_MPC_HOST=root@10.42.0.141 \
MPFREE_MPC_PORT=2223 \
MPFREE_MPC_KEY="$HOME/.ssh/mpc_one_autorun_ed25519" \
bash tools/mpc-one/gpu-qualification/mpfree-gpu-bench.sh
```

The launchers cross-compile on the workstation, copy binaries to the MPC's
volatile `/tmp` directory, run briefly at a reduced process CPU priority,
and retrieve results to this folder. No permanent MPC installation occurs.
The scripts use the device's dynamic loader path `/usr/lib/ld-linux-armhf.so.3`
and do not require graphics development headers on the MPC.

## Probes

- `mpfree-gpu-qualify.sh`: read-only platform/driver inventory plus 256×256
  offscreen GLES2 smoke test with pixel readback. **Surfaceless EGL** succeeded
  on the tested device; the requested GBM pbuffer config did not.
- `mpfree-gpu-bench.sh`: 800×480 offscreen GLES2 composition of four animated
  waveforms, 64 spectrum bars and a 16×8 sequencer; 20 warmup frames and 120
  measured frames with `glFinish()` synchronization. Produces a PPM preview.
- Generated reports, preview images, and ARM binaries are ignored by git.
  Source and instructions are MIT-licensed.

See [GPU qualification findings](../../../docs/milestones/gpu-panfrost-offscreen.md)
and [M8 display safety boundary](../../../docs/milestones/m08-offscreen-test-card.md).

These numbers are **not on-screen FPS** and do not verify display presentation,
touch mapping, font rendering, compositor integration, or audio underrun safety.
The designed logical MPFree UI is 1280×800, not 800×480.
