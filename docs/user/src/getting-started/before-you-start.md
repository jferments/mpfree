<!--
status: changing
covers: rust/mpfree-app/src/main.rs rust/mpfree-app/src/power.rs tools/mpc-one/offline-kexec-stick.py tools/mpc-one/hook-install.sh
checked: 2026-11-15 02509f7
-->

# Before you start

MPFree is a new operating system for making music on the Akai MPC One. It replaces the software you see on the MPC's screen with its own: a drum machine, sampler, synths, looper and sequencer, built for playing live and making beats.

## What you need

- An **Akai MPC One** (the original model).
- **Headphones** or **speakers**. Headphones go in the PHONES socket on the front; speakers or a mixer go in the two OUTPUTS on the back.
- Optionally, something to record from, such as a synth, a turntable or a microphone preamp, plugged into the INPUTS on the back.

## How MPFree runs

MPFree starts from a **USB stick**. While it runs, the MPC's own storage, and any memory card in its SD slot, is switched off and never written to, so when you switch off and take the stick out, your MPC starts its factory software again exactly as before. Your projects, samples and settings in the factory software are untouched. What you save in MPFree is kept on the stick.

Once a small start-up helper is on the MPC, switching on with the stick in starts MPFree by itself, with nothing to tap: the MPC logo shows for a few seconds, then MPFree's startup screen, and the pads are ready about 16 seconds after you switch on. The factory software never appears first. Switch on without the stick and the factory software starts as usual.

- **To go back to the factory software,** switch off, take the stick out, and switch on again.
- **If the factory software starts even though the stick is in,** the MPC didn't accept the stick. That happens after an update of the MPC's own software, until the stick is made again for the new version. Nothing has been changed; the factory software works as normal.
- **If the MPC switches itself off while starting MPFree,** it is a safety stop: MPFree refuses to run unless the MPC's own storage is safely switched off first. Take the stick out and switch on to use the factory software.
- **Never switch the MPC off while it shows an update screen,** with or without the stick. MPFree never interrupts an update: if one is running when you switch on, the factory software starts instead.

## Switching off and restarting

Press the **power button** briefly (let go within about a second) and MPFree asks **Shut down the MPC?** over whatever page you are on. The card says whether your project has changes that aren't saved.

{{shot: power-ask | A short press of the power button: Shut down, Restart or Cancel.}}

- **Shut down** switches the MPC off cleanly: MPFree stops, everything on the stick is written and closed, and the screen goes dark.
- **Restart** starts the MPC again from the beginning, as when you switch it on: with the stick in, MPFree starts again; with the stick out, the factory software starts.
- **Cancel**, or another short press of the power button, closes the card.

If the project has unsaved changes, MPFree asks next whether to save it first. **Save and shut down** (or **Save and restart**) saves it as **SHIFT + BROWSE** does, and the MPC only goes off once the save is on the stick. If the save can't be done (a full stick, say), the card says why and offers to go on without saving. **Don't save** goes on and loses the changes since the last save (see [Saving your work](../making-music/saving.md)).

Tap a button, or turn the data wheel to it and push the wheel.

**Holding the power button down** for a few seconds still switches the MPC off at once, without asking and without saving. Use it only if MPFree stops answering, or if the card says it can't shut down from here.

> [!MPC]
> As on the MPC: a short press of the power button asks before switching off, and holding it switches off at once.

> [!WARNING]
> MPFree is early test software, and there is no installer or download for musicians yet. Today the test stick is made by MPFree's developers. When there is a way for anyone to try it, this page will say how.

## What works today

MPFree is being built quickly and changes every week. Pages in this manual carry a banner when what they describe is still changing or not built yet:

- **Changing:** it works, but the screen or the controls may look different in a newer version.
- **Planned:** it isn't built yet. The page says what is coming.

> [!MPC]
> MPFree is not an update to the MPC software, and it can't open MPC projects (`.xpj` files) or expansions yet. It is a different instrument that runs on the same hardware, so some buttons do new jobs. The boxes like this one point out each difference; the [Coming from an MPC](../reference/coming-from-an-mpc.md) appendix lists them all in one place.
