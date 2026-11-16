<!--
status: changing
covers: rust/mpfree-app/src/plugin_page.rs rust/mpfree-app/src/plugin_store.rs rust/mpfree-browser-ui/src/plugins.rs rust/mpfree-mpcstore
checked: 2026-11-16 d653b57
-->

# Getting plugins

The **Plugins** place in BROWSE finds MPC plugins in the [Open MPC catalog](https://open-mpc.github.io), a free library of plugins people have made for the MPC, and installs the ones you pick. You can also install a plugin from a zip you copied to the stick. MPFree comes with none of them: you choose the ones you want.

{{shot: browser-plugins | The Plugins place: the catalog's plugins with what is installed, an update and a zip on the stick; Mono Bass picked, with its own screen, its version, licence and how hard it works the MPC.}}

## Opening it

Press **BROWSE** and tap **Plugins** in the Library column, or turn the data dial with **SHIFT** held until it is picked. The list has three views at its top:

- **Catalog**: the catalog's plugins that run on the MPC One. A plugin you have is tagged **Installed**, or **Update** and the new version when a newer one is out, or **Zip on the stick** when its zip is waiting there.
- **Installed**: the MPC plugins on your stick, however they got there.
- **On the stick**: plugin zips you copied to the stick.

The chips under the list show **All**, **Instruments** or **Effects**. **Search** looks in the names, the makers and the catalog's tags. The last chip changes the order: **Most used** (the catalog's download count), **Newest** or **A to Z**. Your favourites always come first. Tapping the view or chip that is already on takes the list back to its top.

## What the column on the right tells you

Pick a plugin, with a tap or the data dial, and the right-hand column shows it:

- **The picture**: the plugin's own screen, drawn as it looks on an MPC once it is installed, then the catalog's picture of it. Tap the picture, press the data dial or tap **Pictures** for the next one.
- **Kind**, **Version** and size, and **Licence**.
- **CPU**: how hard the plugin works the processor in the catalog's own test. **Light** is fine to use on several tracks; **Heavy** may need a track or two frozen (see [Mixing](mixing-and-effects.md)).
- **Needs**: anything the plugin wants that MPFree doesn't give it, in its maker's words.

## Installing from the catalog

The MPC One downloads plugins through its network cable (the **Link** port on the back).

1. Plug a network cable from the MPC One into your router.
2. Open **Plugins**. MPFree fetches the catalog when you first open it; **Update list** fetches it again. Without a network, the list you had last time stays.
3. Pick a plugin and tap **Install**, or press **+**. A bar shows the download; **Cancel** stops it.

MPFree checks the download against the catalog before it unpacks anything, so a damaged or swapped file is refused. An instrument is then on the Synth tab's **Instrument** row (see [Synths](synths.md#a-synth-for-each-track)) and an effect in the list you add effects from (see [Effects](effects.md#plugins)), straight away.

> [!NOTE]
> **Not yet checked:** downloading on the MPC One itself hasn't been tried yet. If Update list says the catalog didn't download, install from zips instead (below).

## Installing from a zip

Without a network, copy plugins to the stick from a computer:

1. On the [Open MPC catalog](https://open-mpc.github.io), download a plugin's release zip for the MPC One (the 32-bit **armv7** one; plugins only for the newer 64-bit MPCs won't run).
2. Copy the zip, as it is, into the folder `files/plugins/` on the MPFREE-DATA part of your MPFree stick.
3. In **Plugins**, open **On the stick**, pick the zip and tap **Install from zip**.

A zip the catalog knows shows the plugin's name and says **The catalog's own zip**. One it doesn't know says **Not the catalog's: check where it came from**: install it only if you trust where you got it. A zip with no plugin in it says **Can't install**. **Delete zip** removes a zip you no longer need: tap it, then **Tap again to delete**.

## Updating and removing

When a newer version is out, the plugin is tagged **Update** and the column says which version. **Update** downloads and installs it (or **Install from zip** when its zip is on the stick). Settings you saved inside the plugin's folder are kept.

**Remove** takes a plugin off the stick: tap it, then **Tap again to remove**; press **-** to change your mind. Only the files that came with the plugin are removed, so anything you added to its folder stays. A track already playing the plugin keeps it until you restart MPFree.

## Favourites

Tap **Favourite** to keep a plugin at the top of every list. Tap it again to take it off.

> [!MPC]
> On the MPC, you install plugins by copying them to a drive from a computer. MPFree can do that too, and it can also fetch them from the Open MPC catalog by itself, check them and install them from the screen.
