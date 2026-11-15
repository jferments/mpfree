<!--
status: changing
covers: rust/mpfree-creator rust/mpfree-stick tools/mpc-one/stick rust/mpfree-app/src/setup.rs
checked: 2026-11-15 e171b65
-->

# Installing MPFree

MPFree runs from a USB stick that you make on your computer with the **MPFree USB creator**. You don't need a terminal, a network cable or any typing. It takes about half an hour, most of it waiting:

1. Update your MPC to the factory software MPFree supports, with Akai's own instructions.
2. Make the stick with the creator, from the update file you used.
3. Put the stick in the MPC and tap **Yes** once, so the MPC knows to start MPFree when the stick is in.
4. Wait while MPFree's first start backs up your MPC and brings in its drum kits and samples.

> [!WARNING]
> MPFree has only been tested on the **MPC One (Gen 1)**, the original MPC One. The creator is new: it has been tested on Linux, and the Windows and macOS versions have not yet been tried on a real stick. There are no downloads for musicians yet; this page describes how installing works, and will say where to get the creator when there is a release.

## What you need

- An **MPC One** (the original model).
- A **computer** running Linux, macOS or Windows.
- A **USB stick of 16 GB or more**. 32 GB is better. MPFree runs from it and keeps everything on it: your work, the MPC's own samples, and a backup of your MPC (about 8 GB). Everything already on the stick is erased.
- The **firmware update file** for the MPC One from Akai's website, version **MPC 3.10.1**. MPFree never comes with any of Akai's files: it uses your own copy.

## 1. Update your MPC to 3.10.1

Follow Akai's instructions to update your MPC One to **MPC 3.10.1**, and keep the update file you downloaded. If your MPC already runs 3.10.1, just download the same update file again from Akai.

MPFree only works with the exact factory software it was made for. The creator checks the update file and tells you if it is the wrong one.

## 2. Make the stick

1. Open the **MPFree USB creator**.
   - **Windows:** the first time, Windows may show "Windows protected your PC". Click **More info**, then **Run anyway**.
   - **macOS:** the first time, macOS refuses to open it. Open **System Settings > Privacy & Security**, scroll down, click **Open Anyway** next to the creator's name, and enter your password.
   - **Linux:** allow the file to run as a program (in most file managers: right-click, **Properties**, **Allow executing**), then open it.
2. Under **1. Akai's update file**, the creator shows the update file it found in your Downloads, Desktop or Documents folder, and **Good: MPC 3.10.1 (MPC One, Gen 1)**. If it didn't find it, drag the file onto the creator's window, or type where it is.
3. Put the USB stick into the computer. Under **2. The USB stick**, choose it. The creator lists only USB sticks, never your computer's own disks, with their name and size.
4. Click **Make the stick**, tick **Yes, erase this stick**, and click **Make the stick** again.
5. Your computer asks for your password (on Windows: "Do you want to allow this app to make changes?", click **Yes**). The creator needs it to write the stick.
6. Wait. The creator unpacks the MPC's system from the update file, adds MPFree, writes the stick and reads it all back to check it. This takes a few minutes. Keep the stick in until it says **The stick is ready**.
7. Eject the stick (on macOS: click the eject button next to **MPFREE** in the Finder; on Windows: **Safely Remove Hardware**), and take it out.

If the stick already had MPFree on it, your MPFree settings, work and backups on it are kept, and only MPFree itself is replaced.

## 3. Let the MPC start MPFree (once)

The MPC needs a small start-up helper so that it starts MPFree when the stick is in. You add it once, with Akai's **developer mode** and one tap:

1. With the MPC switched off, put the stick into one of the MPC's USB ports, and switch on. The factory software starts as usual.
2. Press **MENU**, open **Preferences**, then **Enhancements**, and switch on the **developer panel**.
3. Hold **SHIFT** and press **MENU**, then choose **Developer**, and switch developer mode on.
4. The MPC asks whether to run the script on the stick (`autorun.sh`). Tap **Yes**.
5. A few seconds later the MPC restarts by itself, and MPFree starts from the stick.

The helper is three small files in the MPC's own storage. It changes nothing else, and nothing in the factory software. Switch on without the stick, and the factory software starts as before.

**If the MPC doesn't restart,** the script stopped and changed nothing. Put the stick into your computer: it shows a drive called **MPFREE**, and the file **setup-log.txt** on it says why. The most common reason is an MPC that runs a different version from the update file the stick was made with.

## 4. The first start

The first time MPFree starts, it sets itself up before you can play. The screen shows two steps with a progress bar:

1. **Backing up your MPC.** MPFree copies everything in the MPC's own storage onto the stick: the factory software, your projects, samples and settings. It only reads: it never writes the MPC's storage, and checks that nothing was written. This takes 10 to 30 minutes, depending on the stick.
2. **Extracting Akai drum kits and samples.** From the backup on the stick, MPFree makes drum kits from the MPC's factory drum programs, and copies its factory samples into your library, where **BROWSE** finds them in the **Akai factory** folder.

**Keep the stick in and the MPC on until the pads light up.** If the first start is interrupted, nothing is harmed: MPFree tries again at the next start (up to three times), and plays with its own drum kits meanwhile.

When it is done, the Play page opens with the MPC's own drum kits first, then MPFree's own. From now on MPFree starts in about 16 seconds.

> [!MPC]
> The MPC's drum kits and samples are made from your own MPC, on your own stick. MPFree never shares them, and the creator never gives them out.

## 5. Switch developer mode off

While developer mode is on, anyone on the same network as the MPC can get into it with no password. Switch it off once MPFree is set up:

1. Switch off, take the stick out, and switch on. The factory software starts.
2. Hold **SHIFT** and press **MENU**, choose **Developer**, and switch developer mode off.

MPFree's start-up helper should stay on the MPC after this. That has not been checked on an MPC yet: if MPFree no longer starts with the stick in, do step 3 again.

## Keep a copy of the backup on your computer

The backup on the stick is the way back if anything ever goes wrong with your MPC. Keep a copy on your computer too:

1. Put the stick into the computer and open the creator.
2. Choose **More > Copy my MPC's backup to this computer**.
3. Choose the stick, check the folder (**MPFree backups** in your home folder), and click **Copy the backup**.

The creator copies the newest backup and checks every byte of the copy against the stick. It doesn't change anything on the stick. The backup holds your own projects and your MPC's serial numbers: keep it safe, and don't share it.

## Making the stick again

When there is a new version of MPFree, make the stick again with the creator, with the same stick. Your settings, work and backups on it are kept. You don't need developer mode again: the helper is already on the MPC.

After an update of the MPC's factory software, the MPC starts the factory software even with the stick in: it only starts MPFree from a stick made for the version it runs. Make the stick again from the new update file, once MPFree supports it.

To start a stick's MPFree data afresh (erasing its settings, work and backups), tick **Start the stick's data fresh** in the creator's **Preferences** before making it.

## Removing MPFree

To use the factory software, just switch on without the stick. To take MPFree's start-up helper off the MPC as well:

1. In the creator, choose **More > Remove MPFree from my MPC**, choose the stick, and click **Prepare the stick to remove MPFree**.
2. Switch on the MPC with the stick in, switch developer mode on as in step 3, and tap **Yes** when the MPC asks to run the script.
3. The MPC restarts into the factory software. The helper is gone; nothing else on the MPC changed.

To use the stick for MPFree again afterwards, choose **Set it back to install** on the same page and do step 3 again.

## Developer SSH (for developers)

The creator can give the stick a way for developers to log in to the MPC over Ethernet while MPFree runs. It is off unless you turn it on, and musicians don't need it.

1. In the creator, open **More > Preferences** and tick **Developer SSH** under **Developer options**.
2. Under **Your public key file**, the creator fills in your usual SSH public key (`~/.ssh/id_ed25519.pub`, for example) if you have one, and says **Good: a public key**. If you have none, click **Make a key for me**: it makes `mpfree_ed25519` (keep it private) and `mpfree_ed25519.pub` in the `.ssh` folder of your home folder.
3. Make the stick as above. The creator's **Details** list shows the stick's host key, starting with `SHA256:`. Note it.
4. Plug an Ethernet cable from the MPC's Ethernet port into your router (or into your computer, with its internet sharing on). Start MPFree, and find the MPC's address in your router's list of devices.
5. Connect, as root, on port 2222, with your key. On Linux, macOS or Windows (PowerShell), with the key the creator made:

   ```
   ssh -i ~/.ssh/mpfree_ed25519 -p 2222 root@ADDRESS
   ```

   with the MPC's address in place of `ADDRESS`. The first time, `ssh` shows the stick's host key: check it matches the one you noted, then type `yes`.

Only key logins work: there is no password, and only for root, on port 2222. Each stick has its own host key, made when the stick is made. To turn it off, untick **Developer SSH** and make the stick again; the stick then has no SSH server at all.
