MPFree setup drive
==================

This is the MPFREE partition of the stick the MPFree USB creator made. Akai's
script runner offers to run autorun.sh when the stick is in and developer mode
is on (MENU > Preferences > Enhancements: developer panel on; then hold SHIFT,
press MENU, choose Developer); tap Yes once. It installs MPFree's startup hook and
restarts the MPC, which then starts MPFree from this stick.

mode.txt says what autorun.sh does: "install" (the default) or "remove" (takes
the startup hook off the MPC again; set it with the creator, which also stops
this stick starting MPFree until it is set back to install). After a remove it
says "removed", and running autorun.sh again does nothing. setup-log.txt says
what it did.

Without the stick in at power-on, the MPC starts Akai's system as usual.
MPFree has only been tested on the MPC One (Gen 1).

The user manual's "Installing MPFree" page has the whole process.
