<!--
covers: rust/mpfree-mpcstore rust/mpfree-app/src/plugin_store.rs rust/mpfree-app/src/plugin_page.rs rust/mpfree-browser-ui/src/plugins.rs rust/mpfree-plughost/src/rack.rs
checked: 2026-11-16 d653b57
-->

# Installing MPC plugins: the Plugins place

**Status:** 2026-11-16. Built and tested on the workstation, including a live run against the real catalog and a real release (fetched, checked, unpacked and loaded). Not yet run on an MPC: whether the soft-boot stick reaches HTTPS (the system's libcurl and CA certificates under Akai's root filesystem) is a read-only check for Jay's next combined device run. The musician's side is [Getting plugins](../../user/src/making-music/plugins.md).

MPFree never ships MPC plugins (Jay, 15 Nov: "I don't want to bundle these plugins"). The musician picks them. Jay asked for an easier install than copying folders, ideally a browser of the [Open MPC catalog](https://open-mpc.github.io) inside MPFree (22:06Z), and chose the route on two cards:

- **"Ethernet cable, or from the stick"** (23:34Z): the place downloads over the MPC's network port; without a network it installs release zips copied to the stick. Wi-Fi is out under the soft-boot route: LoadPin on Akai's kernel refuses firmware loads after the handover, so the Wi-Fi chip never gets its firmware.
- **"Yes, build it as drawn"** (23:36Z): a **Plugins** place in BROWSE's Library column after Recordings, recorded in the [layout spec](layout-spec.md), section 3. Mockups: `/mnt/project-files/mpc-plugins/browser/`.

## The pieces

| Piece | What it does |
|---|---|
| [`rust/mpfree-mpcstore`](../../../rust/mpfree-mpcstore) | No-dependency library: `catalog` (reads `catalog.json`, picks each plugin's release for 32-bit ARM, says whether it fits), `fetch` (HTTPS through the system's libcurl), `zip` (reads zips, stored and deflate, with CRC checks), `sha256`, and `install` (unpacks a release's plugin folder safely, records what it wrote, updates and removes). |
| [`plugin_store.rs`](../../../rust/mpfree-app/src/plugin_store.rs) | The app's worker thread: keeps the catalog, downloads, installs, lists what is installed and on the stick, draws the pictures, keeps the favourites. The screen never waits on the network or the stick. |
| [`plugin_page.rs`](../../../rust/mpfree-app/src/plugin_page.rs) | The place's state: views, chips, order, search, selection, the two-tap Remove, progress, what each row and the Selected column say. |
| [`mpfree-browser-ui/src/plugins.rs`](../../../rust/mpfree-browser-ui/src/plugins.rs) | Draws the list and the Selected column from a `PluginsScreen` and turns touches into `PluginInput`s. The browser forwards the dial, push, + and - while the place shows (`BrowseAction::Plugins`). |
| `Rack::add` ([`rack.rs`](../../../rust/mpfree-plughost/src/rack.rs)) | Lists a newly installed folder in its own plugin process and adds its entries to the running rack, so the plugin is offered at once. |

## The catalog

`https://open-mpc.github.io/catalog.json` (the older `sd88me.github.io` address serves the same file), schema 1. For each plugin MPFree takes the catalog's `latest` stable version when it has an `armv7` asset with an `https://` URL and a 64-digit SHA-256, else the newest stable one that has; yanked and `gen2_only` versions never count. `Fit` says why a plugin can't be installed: an add-in for Akai's app, 64-bit only, or no release (build it yourself). Of the 255 entries on 15 Nov, 241 have a 32-bit ARM release.

The catalog is kept as `files/plugins/.catalog.json`, so the list is there without a network. It is fetched again once per run when the place first opens, and by **Update list**; the status line says "Ethernet" or "no network" after a try.

## Downloading

`fetch` opens `libcurl.so.4` with `dlopen` (no new build dependency, nothing to cross-compile) and refuses anything but `https://` to `open-mpc.github.io`, `sd88me.github.io`, `github.com`, `raw.githubusercontent.com` and `objects.githubusercontent.com`. Redirects (GitHub's release downloads go through two) must stay on HTTPS and are capped at 8. It fails on an HTTP error, gives up on a stalled transfer (under 1 KB/s for 30 s), caps the size (512 MB for a zip, 32 MB for the catalog, 16 MB for a picture) and removes a partial file. Downloads land in `files/plugins/.downloads/` and are removed once installed. No libcurl: Install is not offered and the place says to copy zips instead.

## Installing

`install::install` takes the zip's bytes, the catalog's SHA-256 (when installing from the catalog) and the catalog id and version:

- **Checked first.** The SHA-256 is computed every time and must match the catalog's when one is given. A zip on the stick is matched to the catalog by its sum; an unknown one says "Not the catalog's: check where it came from".
- **What counts as the plugin.** A folder holding `plugin.json`, or a description file and a `.so`; folders inside one belong to it. Releases put it at `<Top>/portable/<Maker - VST - Name>/`. A zip with none is refused.
- **Nothing escapes the folder.** Every name is checked (no absolute paths, `.` or `..`, backslashes, drive letters, control characters or a trailing dot or space); links are skipped and counted; ZIP64, split zips and more than 20,000 entries are refused; 512 MB unpacked at most.
- **Never half-installed.** Files are written to `.mpfree-installing-<folder>` and synced, then the folder is swapped in by renames. An update moves the old folder aside as `.mpfree-replaced-<folder>`, carries over every file the musician added (anything not in the old record), swaps, and removes the old one.
- **A record.** `.mpfree-install` in the folder lists the catalog id, version, sum and every file written, so **Remove** deletes only those files and empty folders and leaves the musician's own. A folder without a record (copied by hand) is removed whole, after the same two taps.

After unpacking, the worker calls `Rack::add` for each folder. The rack lists it in its own plugin process, as at start, and adds the entries (the cap of 128 plugins still holds). The screen sends `FromUi::PluginsChanged`; the audio loop rebuilds `App::plugin_synths` (the Synth tab's Instrument row) and sends the plugin folders again (the Synth tab's skins). The chain editor's picker reads the rack each time it opens. A removed plugin stays in the rack until restart, so a track playing it keeps playing; the place says so.

## The screen

As drawn and approved. The rows are name, maker and a tag: **Installed**, **Update** and the version (yellow), **Zip on the stick**, **Can't install**, or the kind. Favourites (kept in `files/plugins/.favourites`) come first in every order; Most used is the catalog's download count, Newest its `updated` date. The Selected column shows the picture, Kind (with the catalog's style), Version and size, Licence, CPU (the catalog's bench verdict and figure: Light, Heavy, Too heavy), Needs and Also needs (the catalog's `requires` and `depends`), the summary, one line on what the main button will do, and the buttons.

**Pictures** step through the plugin's own screen, each skin tab drawn by `mpfree-mpcskin` at 1280 x 628 with every parameter at 0 and scaled to the column (only once installed), then the catalog's screenshot, downloaded once into `.downloads/` and named for its address. The page keeps the last six drawn. A lone picture has no Pictures button.

Controls: the dial moves, its push shows the next picture (a tap on the selected row too, as in the browser), **+** presses Install, Update or Install from zip (or answers Tap again to remove), **-** takes back a Remove or clears the search, Q-Link 2 steps the chips. A tap on the view or chip already on takes the list to its top, so every control does something (the controls sweep). Two things differ from the mockups and want Jay's look: an **Update list** button in the list's head (the mockups had no way to fetch again), and the hint says "push: next picture" where the mockup said "push: details".

## Tests

- `mpfree-mpcstore`: the zip reader (entry kinds, a damaged entry, unsafe names), SHA-256 against the standard examples, the catalog's release pick, finding a release's plugin folder, install with the sum checked and an update keeping the musician's file, an unsafe name stopping the install, the hosts `fetch` allows, and `fetch` against a local server (progress, the size cap, an HTTP error, cancel). `examples/install.rs` and `examples/fetch.rs` run the real thing by hand; on 15 Nov they installed 6W6 from the live catalog.
- `plugin_store.rs`: listing, a zip that isn't one refused and deleted, no data folder, the picture fit.
- `plugin_page.rs`: the catalog view, orders, chips and search, Update and the two-tap Remove, zips on the stick, favourites and the selection kept in view, version order, the sample screen.
- `gui.rs`: `the_plugins_place_lists_the_stick_and_deletes_a_zip_on_the_second_tap` drives the real browser and worker on a scratch data folder; the sample screen `browser-plugins` is the manual's picture and goes through the controls sweep.
- `mpfree-plughost/tests/mpc.rs`: `a_plugin_installed_while_running_is_offered_at_once`.

## Not yet

1. **On the MPC.** Does the soft-boot stick reach HTTPS: libcurl present, CA certificates found, DNS over the Ethernet port. Read-only; folded into Jay's next combined device run.
2. **A zip's pictures before installing.** Only the catalog's picture shows until it is installed.
3. **Wi-Fi.** Needs the firmware loaded before the handover, or the opt-in chain route (`loadpin.enforce=0`), and a Wi-Fi page; the stick's owner thread would carry it.
