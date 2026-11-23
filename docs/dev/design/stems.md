<!--
covers: rust/mpfree-stems rust/mpfree-simd/src/neon.rs rust/mpfree-bench/src/stems.rs rust/mpfree-sampler/src/stems.rs rust/mpfree-sampler-ui/src/stems_page.rs
checked: 2026-11-23 dea01de
-->
# Stem separation (`mpfree-stems`)

**Status:** engine built (plan step 1, 2026-11-23), on the workstation only: the crate, its two NEON kernels in `mpfree-simd`, the model packer and the bench's `--stems` mode. **Nothing here has run on the MPC**. The sampler's side of plan step 4 is built too (the split as a chain of jobs, the stems landing as one undo step, the Stems page), but the app doesn't offer it yet. The plan (Spleeter, the steps after this one and the decisions put to the owner) is in the stems research notes; the device run is plan step 2, the Stems page's mockups step 3, the app step 4.

## What it does

It splits a sound into four: vocals, drums, bass and other, as Akai's MPC Stems does as a Process Sample job (MPC 3.10.1 guide, p. 280). It runs Deezer's [Spleeter](https://github.com/deezer/spleeter) 4-stem model, ported to Rust: Deezer publishes the code and the trained models under the MIT licence, so they can ship with MPFree with Deezer's copyright notice beside them. It is our own code; nothing is cross-built from C++ or Python.

It is a background job's engine: it never runs on the audio thread, it works in steps a worker thread can stop between, and it says before it starts how much memory it will hold.

## How it works

The pipeline is Spleeter's, kept as it is, so its stems are Spleeter's:

1. **STFT:** each channel through 4096-point periodic Hann windows a hop of 1024 apart, after 4096 leading zeros, with TensorFlow's framing (the end padded to whole frames). Each transform is a 2048-point complex FFT and a split (`stft.rs`).
2. **Segments of 512 frames** (11.9 s), the last padded with silence, as Spleeter does. The magnitudes of the first 1024 bins (to 11 kHz), or 1536 at full band, are one segment's input.
3. **Four U-Nets** (`net.rs`), one a stem, as `spleeter/model/functions/unet.py` builds them: six 5×5 stride-2 convolutions (16 to 512 filters, batch norm, ELU), six 5×5 stride-2 transposed convolutions (ELU, batch norm, each but the first joined with the encoder's output of its size), and a 4×4 convolution dilated by 2 with a sigmoid: a soft mask, times the magnitudes.
4. **Masks:** each stem's (o² + ε/4) / (Σ o² + ε), ε = 1e-10, times the complex STFT.
5. **Inverse STFT** with the 2/3 window compensation, overlap-added a frame at a time and handed on 64 frames at a time (1.5 s).

What makes it fit the MPC:

- **Every layer is a GEMM** on `mpfree_simd::gemm4x8`, a 4 × 8 micro-kernel (NEON on the MPC, its plain twin elsewhere). The patches are gathered straight into the kernel's packed layout, four pixels at a time so each store is one row of the kernel's input (`put4`), a skip join is read from its two tensors in place, and the transposed convolutions are four gathers, one for each output parity, so no work is spent on the zeros a naive transposed convolution inserts. The two narrow layers don't pad to the kernel's eight columns: the last transposed convolution (to one channel) is a GEMM to its 25 taps scattered into the output, and the mask layer (one channel to two) runs row by row.
- **Weights as halves:** 79 MB rather than 158. Each layer is unpacked when it runs (`mpfree_simd::f16_to_f32`, NEON's `VCVT.F32.F16`) into one buffer as big as the widest layer (13 MB). The file's halves go into each layer's packed layout as they are, with no round trip through f32 (loading the model takes 0.17 s on the workstation, 0.42 s before). Rounding Spleeter's weights to halves costs nothing audible: the stems stay 65 to 74 dB from Spleeter's own (below).
- **Streaming:** a segment's tensors are all a run holds, so a sound of any length needs the same memory, and the stems go to the caller as they are finished.
- **Threads:** each layer spreads its rows over the threads it is given (two, the job service's sound lane on cores 0 and 1); the threads it starts take the calling thread's cores. The result doesn't depend on how many threads ran it (a test checks it to the bit).

## Bands

| `Band` | The networks see | Above it | Work for a segment (4 stems) |
| --- | --- | --- | --- |
| `Spleeter` | 1024 bins, to 11 kHz (Spleeter's `4stems`) | silence, as Spleeter | 24.45 G multiply-adds |
| `Full` (the default) | 1536 bins, to 16.5 kHz (Spleeter's `4stems-16kHz`, the same model) | each stem keeps the mean of its mask's top 64 bins, so the four stems add up to the sound | 36.68 G |

Full band is the plan's recommendation (the owner's decision 2); until the owner answers, both are built and the default can change in one line.

## The interface

```rust
let model = Model::open(path)?;                       // or Model::read(reader)
let settings = Settings { band: Band::Full, threads: 2 };
let promise = model.need(&settings);                  // bytes, before it starts
let mut sep = Separator::new(&model, &samples, channels, 44_100, &settings)?;
while sep.step(&mut sink)? {                          // a step: an STFT, a U-Net, or the masks
    progress(sep.progress());                         // stop here to cancel
}
```

- `samples` are interleaved, mono or stereo, at 44.1 kHz (anything else is refused: the app converts the rate first). A mono sound goes in as stereo with both sides the same, as Spleeter does; the stems are always stereo.
- `Sink::put(stem, stereo)` gets each stem's next frames, the stems in the model's order (vocals, drums, bass, other), until each has had as many frames as the sound. An error there ends the separation with `Error::Sink`.
- A step is one segment's STFT, one stem's U-Net (about 1.6 s on the MPC's two job cores, by the plan's estimate) or the segment's masks and inverse. `progress()` goes from 0 to 1 a step at a time.
- `Model::need` is what a separation holds at its most besides the model and the sound: 75.9 MB at 1024 bins and 96.9 MB at full band with two threads, whatever the sound's length. A test runs both bands on Spleeter's real shapes with a counting allocator: they held 75.6 and 96.6 MB (`tests/need.rs`, which fails if the promise is short, or more than 5 % over). With the model that is about 155 and 176 MB.
- `stats()` says where the time went (STFT, U-Nets, unpacking, masks, the sink) and the kernel's multiply-adds.

## In the sampler (plan step 4)

The app runs a split as `mpfree_sampler::stems::Split`, on the job service's sound lane (`jobs.rs`), **one step a job**: a segment's STFT, one stem's U-Net (about 1.6 s on the MPC) or the segment's masks. The separation owns its model and the sound read (`Separator<Arc<Model>, Arc<[f32]>>`, which is `Send`), so it moves from one job to the next: each step ends by putting the next at the back of the queue with `Jobs::follow`, which queues a job behind the running one with the same key without stopping it or counting it replaced (`Jobs::submit` would). A job for the sound being shown waits at most one step, and a split of a minute's sound never holds the lane for a minute.

- **Memory:** the caller promises it once for the whole split (the model, the sound, `Model::need` and the stems) and hands the ticket to `Split::start`; the split holds it until it ends however it ends, and the steps ask the service for nothing, so it never runs out of memory halfway.
- **Stop** stops it between steps; nothing it made is kept. A second split of the same sound takes the first one's place (they share the key).
- **The picture as it comes:** `Split::peaks` is each stem's columns so far (the lowest and highest sample of both sides over a column's frames), which fill a segment at a time, so the Stems page draws the stems as they come.
- **16-bit stems:** for a 16-bit sound the stems are rounded to 16 bits with triangular dither (`to_16_bits`, through `i16` so a zero has no sign), and are held as 16 bits at half the memory: a stem can't be finer than its sound.
- **Landing:** `Project::put_stems(sounds, to)` adds the stems' sounds and puts them on pads, one pad's layers (all at full velocity, so they play together) or takes, as **one undo step** (an `Undo::Group`); it refuses a pad past the last or named twice, layers on an empty pad, a layer playing a sound neither given nor in the kit and more takes than the page keeps, changing nothing. The sampler UI's `Change::Stems` makes it.
- **The page:** `mpfree_sampler_ui::stems_page` is the Stems page as the layout spec draws it (the picture, the Split, Put on and Listen sections, the stems' levels on the Q-Links), with what each press asks of the app; `mpfree-sampler-preview` draws its five screens (`stems-*.png`). The app wiring (the model from the stick, the sound read at 44.1 kHz, the memory check, polling the split and landing it) is the rest of step 4.

`mpfree_stems::segments` and `steps` count a sound's segments and steps as a separator on Spleeter's shape counts them, so the page can say how long a split takes (10.4 s a segment on the MPC by the plan's estimate, then as the last split went) before the model is read.

## The model file

`Model::open` reads `MPFSTEM2`: the eight bytes `MPFSTEM2`, the stems (a little-endian u32, 4), then each stem's 50 tensors in Spleeter's order (seven convolutions, kernel and bias, the seventh the mask layer's; six transposed ones; twelve batch norms folded to a scale and a shift), each as its kind (u32: 1 f32, 2 half), its count (u32) and its values. Kernels are halves, the rest f32. The reader refuses a file that is short, long, of another kind or stem count, with a count that isn't the layout's, or with a half that isn't a normal number or zero, and says which tensor.

`pack::pack` makes it from the `4stems` folder in `4stems.tar.gz` (Spleeter's v1.4.0 release), with no TensorFlow or Python: it reads the checkpoint's index (a LevelDB-format table of `BundleEntryProto` entries), checks each tensor's CRC-32C and shape, and folds each batch norm as Spleeter's graph does in f32 (scale = gamma / sqrt(variance + 0.001), shift = beta − mean · scale). Deezer's checkpoint packs to 78,627,740 bytes (SHA-256 `384862bb14f50848999e3e559219acd9086df9374b96337060726b8354fd8491`); 18,771 of its 39,276,928 kernel weights are below the smallest normal half and become zero. The app step (plan step 4) puts the file on the stick through the USB creator.

On the workstation:

```bash
cargo run --release -p mpfree-stems --example stems -- pack 4stems 4stems.mpfstem
cargo run --release -p mpfree-stems --example stems -- split 4stems.mpfstem song.wav out --band full
```

## Checked against

- **Each layer against TensorFlow's definition** of it, written out in f64 (`net.rs`'s tests): the strided convolution and its batch-normed ELU, the four parities against `conv2d_transpose` with and without a skip join, the scatter, the mask layer, and a whole small U-Net layer by layer. The STFT pair against a full complex FFT and a whole-signal inverse; the half rounding against an f64 reference, and every weight half back exactly.
- **Spleeter's own output** (Spleeter 2.4.0 on TensorFlow 2.9.3 on the workstation, on Spleeter's example clip): at `Band::Spleeter` the stems are 74.4, 69.9, 67.5 and 67.6 dB SNR from Spleeter's (vocals, drums, bass, other) on 10.9 s, and 70.4, 67.1, 66.9 and 64.9 dB on 43.5 s (four segments). At full band, against Spleeter's `4stems-16kHz`, bass and other are 67.5 and 67.6 dB; vocals and drums 52.4 and 27.9 dB, the difference being what ours keeps above 16.5 kHz and Spleeter's drops: our four full-band stems add up to the sound to 138 dB.
- **Whole separations** on small networks (the same code): equal masks give each stem a quarter of the sound over several segments; Spleeter's band leaves a 5 kHz tone out of every stem where the networks see 1.4 kHz; full-band stems add up to the sound with random weights; one thread and four give the same bits; a mono sound separates as its stereo copy; nothing and one frame separate; a sink's error stops it.
- **On ARMv7** under qemu-arm, the crate's tests pass with the NEON kernels.
- **The packer** on a checkpoint written in the test (a LevelDB table and its data, Spleeter's names): the model it makes is the one built from the same weights; a CRC mismatch, a damaged table and a kernel stored the wrong way round are refused by name. On Deezer's real checkpoint it writes the same bytes as the research prototype.

## Speed

On the workstation (a 2.1 GHz Xeon VM, two threads, `--release`): 10.9 s splits in 1.8 to 1.9 s at 1024 bins and 2.7 to 2.9 s at full band; 43.5 s in 6.6 and 10.2 s. Loading the model takes 0.17 s. The U-Nets are 92 to 95 % of it (unpacking the halves 3 to 4 %), the STFTs, masks and inverse 5 to 6 %.

**Where an ARMv7 core's instructions go,** counted under qemu-arm for one 64-frame segment at full band (the ignored test `profile_a_short_segment`, 4.58 G multiply-adds, the model's building left out): of 3.54 G instructions, the kernel's loop is 1.72 G (49 %: 12 instructions for 32 multiply-adds), gathering the patches with the layers' bias, batch norms and ELUs 0.76 G (21 %), the FFTs 0.36 G (10 %), `expf` (the ELUs and sigmoids) 0.15 G and the GEMM driver 0.12 G. Gathering four pixels at a time took the gather from 1.37 G to 0.76 G and the segment from 4.15 G to 3.54 G, with the same stems to the bit; on the workstation, where the compiler vectorises the old copy, the split is 3 to 10 % faster. Instructions aren't time on a Cortex-A17, so the device run says what it is worth.

The MPC's figures are the plan's estimate until the device run: 9.4 s for a 10 s sample at 1024 bins (6.2 to 15.6), 12.5 s at full band; 43 and 62 s for 60 s. That run is `mpfree-bench --stems` ([Profiling](profiling.md#stem-separation---stems)). It also times the kernel alone on one core with its data in the L1 cache, the most the U-Nets could do: their own rate against twice it says how much of their time is outside the kernel. The kernels keep their place only if the device run shows them faster than their twins, as every `mpfree-simd` kernel ([NEON kernels](simd.md)).

## What's next

- Plan step 2: `mpfree-bench --stems` in the next device round's benchmark block (RAM only, on the owner's go).
- Plan step 3: the Stems page's mockups, proposed for the layout spec.
- Plan step 4, the rest: the app wiring (the model file on the stick, the sound read and converted to 44.1 kHz, the shared memory check, the split polled and landed, the pad bar's "Stems 42 %"), New kit and Library, and the manual page.
- Faster, if the device run shows the U-Nets well below twice the kernel's rate: a NEON transpose for the gather's stores (about 0.2 G of the 3.54 G above), a radix-4 FFT (it changes the bits, so the SNR checks run again) and a faster `expf`.
- Holding less: both channels' STFT stay for the whole segment (16.8 MB); working them out again for the masks would save that for about 3 % more time.
