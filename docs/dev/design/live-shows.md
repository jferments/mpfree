# Live shows: patches, keygroups and set lists

The plan for playing whole shows from one MPC: songs in a set, the next song ready before you need it, keyboard patches that scenes recall, keys that play keygroups, and a Show view in Perform. The full plan, its research, two reviews and the twelve decisions are in the Claude Doc [Live shows: patches, keygroups and set lists](https://claude.ai/artifact/PBWnfRy2bfrJwhgS1eVjf6), approved as written by the owner on 22 Nov 2026. This page says what is built, as it is built.

The rules it keeps: one track list per project; instruments make the sound, tracks record, pads and keys play instruments; no instrument swaps on a track per scene. It uses what the other redesigns make and never its own copy: the arranger's one scheduler, the stream layer ([Disk streaming](disk-streaming.md)), the instrument list ([Instruments](instruments.md)) and the audio tracks' recordings ([Audio tracks](audio-tracks.md)).

## Roadmap

| Phase | What lands | State |
|---|---|---|
| A1 | All-or-nothing opens: the parts gate is the audio tracks rethink's phase 0.1 ([Project safety](project-safety.md)); A1 adds a refusal when sounds, takes or loops would be left out, and Open what reads | built ([Project safety](project-safety.md#a-project-that-cant-open-whole-asks-first-live-shows-a1-as-built)) |
| A2 | The engines an open replaced always go back to the screen thread, never freed on the audio thread | built ([Project safety](project-safety.md#the-engines-an-open-replaced-go-back-live-shows-a2-as-built)) |
| A3 | Lasting project ids (Save As gives a new one, a rename keeps it, twins are noticed) and a manifest per project | built ([The browser and projects](browser.md#projects)) |
| A4 | Real songs measured on the MPC (read-only, on the owner's go) | planned |
| K1 | Keys play keygroups by note on the sampler's single note path, with each kit's Keys play: Pads, Keygroups or Both | planned |
| B1, B2 | Sets, the Set editor and the Show view with cold changes; the set check as a preflight, Prepare show media and Soundcheck | planned |
| C1 to C4 | Patches: zones, the note owner table, scenes and parts picking patches, REC with zones, knobs and pads per patch | planned |
| D1, D2 | The next song prepared in the background; the switch at once or on its frame; Let it finish, Fade and Cut | planned |
| E1, E2, V1, F1 | Click, cues and the Split output; Next, Repeat and Hold part; the track view; Overlap | planned |
