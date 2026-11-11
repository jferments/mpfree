# mpfree-splash-gl

The MPFree startup screen with SDL3 and OpenGL ES 3.0: the same galaxy and
title as the character-cell [`mpfree-splash`](../mpfree-splash), at full
resolution. The galaxy's disc (four arms, dust lanes, star-forming knots, a
gold bulge) is drawn once into a 2048x2048 texture by a shader; each frame
turns and tilts that texture, adds a nebula sky, about 12,000 point stars
that twinkle and a chrome title with a glow. It shares its layout, spiral
constants and title font with the character-cell version: the built-in
block letters, or any TTF/OTF file given with `--font FILE [--tracking N]`
(see [`mpfree-splash`'s README](../mpfree-splash/README.md#title-font)).

It is its own Cargo workspace, outside `rust/Cargo.toml`, because the `sdl3`
crate builds SDL 3 from source (statically linked; zlib licence). That needs
CMake, a C compiler and the X11/Wayland/EGL development headers.

## Build and run on the workstation

Packages, Ubuntu/Debian (this list was tested on Ubuntu 24.04):

```bash
sudo apt-get install -y cmake libx11-dev libxext-dev libxrandr-dev libxcursor-dev libxi-dev libxss-dev \
  libxkbcommon-dev libxfixes-dev libxtst-dev libwayland-dev wayland-protocols libdrm-dev libgbm-dev \
  libegl-dev libgles-dev libasound2-dev libudev-dev libdbus-1-dev
```

Fedora (not tested here; from SDL's Linux build notes):

```bash
sudo dnf install -y cmake gcc libX11-devel libXext-devel libXrandr-devel libXcursor-devel libXi-devel \
  libXScrnSaver-devel libxkbcommon-devel libXfixes-devel libXtst-devel wayland-devel wayland-protocols-devel \
  libdrm-devel mesa-libgbm-devel mesa-libEGL-devel mesa-libGLES-devel alsa-lib-devel systemd-devel dbus-devel
```

Then:

```bash
cargo run --manifest-path rust/mpfree-splash-gl/Cargo.toml --release --locked
cargo run --manifest-path rust/mpfree-splash-gl/Cargo.toml --release --locked -- --fullscreen
SDL_VIDEO_DRIVER=offscreen cargo run --manifest-path rust/mpfree-splash-gl/Cargo.toml --release --locked -- \
  --screenshot /tmp/splash-gl --frames 4 --every 3
```

The first build compiles SDL 3 and takes a few minutes.

## On the MPC One

Not yet. The MPC has Mesa (Panfrost, GLES 3.1), but this program would need
SDL 3 cross-built for ARM with its KMS/DRM backend, linked against the
device's own libEGL, libgbm and libdrm, so it can't be a fully static binary
like `mpfree-splash`. Until that is built and tested, the boot screen on the
device is the framebuffer version, `mpfree-splash --framebuffer`.
