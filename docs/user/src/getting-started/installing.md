<!--
status: changing
covers: rust/mpfree-creator rust/mpfree-stick tools/mpc-one/stick rust/mpfree-app/src/setup.rs
checked: 2026-11-16 14e8184
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
   The creator opens on **Make a stick**, with three numbered steps. Its other pages are the tabs along the top: **Copy the backup** and **Take MPFree off my MPC**. **Preferences** and **Help** are at the top right.
2. In step **1, Akai's update file**, the creator shows the update file it found in your Downloads, Desktop or Documents folder, with a green tick and **MPC 3.10.1 (MPC One, Gen 1)**. If it didn't find it, drag the file onto the creator's window, or type where it is (the file, or the folder it is in).
3. Put the USB stick into the computer. In step **2, Your USB stick**, choose it. The creator lists only USB sticks, never your computer's own disks, with their name and size.
4. In step **3**, click **Make the stick**, tick **Yes, erase this stick**, and click **Erase and make the stick**.
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

The first time MPFree starts, it sets itself up before you can play. It can take 20 to 30 minutes in all, once; the stick's speed decides most of it. The screen always says what it is doing, and one progress bar covers the whole setup: it moves as the data is copied, and once MPFree has measured how fast the stick is, the screen says how long is left.

1. **Preparing the stick for its first start.** Over the startup screen, MPFree makes the space on the stick for your sounds and settings. This takes a few seconds.
2. **Backing up your MPC, so you can always go back.** MPFree copies everything in the MPC's own storage onto the stick: the factory software, your projects, samples and settings (about 8 GB), then **checks the backup on the stick** by reading it all back. It only reads the MPC's storage: it never writes it, and checks that nothing was written. This is the longest step.
3. **Extracting Akai drum kits and samples.** From the backup on the stick, MPFree makes drum kits from the MPC's factory drum programs, and copies its factory samples into your library, where **BROWSE** finds them in the **Akai factory** folder: a kit for each of the MPC's drum kits, which loads onto the pads whole as any kit does (**+** opens it to show its sounds one by one), then **Instruments** (the MPC's keygroup instruments, one folder each) and **Other sounds**. Your own projects and sounds on the MPC (its **MPC Documents**) are not copied there; they are in the backup.

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
2. Choose the **Copy the backup** tab.
3. Choose the stick, check the folder (**MPFree backups** in your home folder), and click **Copy the backup**.

The creator copies the newest backup and checks every byte of the copy against the stick. It doesn't change anything on the stick. The backup holds your own projects and your MPC's serial numbers: keep it safe, and don't share it.

## Updating MPFree on the stick

When there is a new version of MPFree, put it on the same stick with the new creator. Your backup, settings, work and the Akai kits and samples stay on the stick, so there is no long first start again.

1. Put the stick into the computer and open the creator. Step 1 finds Akai's update file as before.
2. In step **2, Your USB stick**, choose the stick. The creator sees that it already has MPFree, and step 3 becomes **Update MPFree on this stick**.
3. Leave **Keep my backup, settings and files** chosen, and click **Update MPFree**. Approve your computer's password prompt.
4. When the creator says **MPFree is updated**, put the stick back into the MPC and turn it on. MPFree starts with your settings and files.

You don't need developer mode again: the helper is already on the MPC. If the stick was set up by an earlier MPFree, its first start after the update may sort the Akai samples into a folder per kit, which takes a minute or two; what is left of the old folders goes to the **Trash**. If the creator can't keep what is on the stick (a stick from a much older or newer MPFree), it stops before writing anything and says so: copy the backup to your computer first, then choose **Erase everything and start over**.

To start the stick afresh instead, choose **Erase everything and start over** in step 3. That erases the backup on the stick, your MPFree settings and work too, and the next start backs up your MPC again. (For a stick the creator doesn't recognise as MPFree's, the same choice is **Start the stick's data fresh** in **Preferences**.)

After an update of the MPC's factory software, the MPC starts the factory software even with the stick in: it only starts MPFree from a stick made for the version it runs. Make the stick again from the new update file, once MPFree supports it. If MPFree still doesn't start, do step 3 again: Akai's update may have removed the helper.

## Removing MPFree

To use the factory software, just switch on without the stick. MPFree changes one thing on the MPC, the small start-up helper from step 3. To take it off as well, you turn the stick into a **removal stick** and let the MPC run it once:

1. In the creator, choose the **Take MPFree off my MPC** tab. Choose the stick and click **Make it a removal stick**. A removal stick no longer starts MPFree; your MPFree settings, work and backups on it are kept.
2. Put the stick into the MPC and switch on. The factory software starts, not MPFree.
3. Switch developer mode on as in step 3 above, and tap **Yes** when the MPC asks to run the script.
4. The MPC restarts into the factory software. The helper is gone; nothing else on the MPC changed. If the MPC asks to run the script again, tapping **Yes** does nothing, and **No** is fine too.

Then switch developer mode off as in step 5.

**Changed your mind?** On the same page, click **Make it an MPFree stick again**: the stick starts MPFree again, with your settings and work. If you already took the helper off the MPC, do step 3 again to put it back.

## Developer SSH (for developers)

The creator can give the stick a way for developers to log in to the MPC over Ethernet while MPFree runs. It is off unless you turn it on, and musicians don't need it.

1. In the creator, click **Preferences** (top right) and tick **Developer SSH** under **Developer options**.
2. Under **Your public key file**, the creator fills in your usual SSH public key (`~/.ssh/id_ed25519.pub`, for example) if you have one, and says **A public key** with a green tick. If you have none, click **Make a key for me**: it makes `mpfree_ed25519` (keep it private) and `mpfree_ed25519.pub` in the `.ssh` folder of your home folder.
3. Make the stick as above. The creator's **Details** list shows the stick's host key, starting with `SHA256:`. Note it.
4. Plug an Ethernet cable from the MPC's Ethernet port into your router (or into your computer, with its internet sharing on). Start MPFree, and find the MPC's address in your router's list of devices.
5. Connect, as root, on port 2222, with your key. On Linux, macOS or Windows (PowerShell), with the key the creator made:

   ```
   ssh -i ~/.ssh/mpfree_ed25519 -p 2222 root@ADDRESS
   ```

   with the MPC's address in place of `ADDRESS`. The first time, `ssh` shows the stick's host key: check it matches the one you noted, then type `yes`.

Only key logins work: there is no password, and only for root, on port 2222. Each stick has its own host key, made when the stick is made. To turn it off, untick **Developer SSH** and make the stick again; the stick then has no SSH server at all.
