# Offload: render partitions on another computer

A partition of the audio graph can run on a computer on the network instead
of on the MPC One: a send effect first, sequenced instruments later. This is
step 2 of the offload plan in the performance plan
(https://claude.ai/artifact/Su4KF6BVbzkm9d5NfnWQMT): **desktop only**, a
server and a client on one machine over loopback. Nothing here runs on or
talks to an MPC.

Jay's decisions (2026-11-11) that shape it:
- The second MPC core comes first ([Concurrency](concurrency.md)).
- Send effects are offloaded first, then sequenced instruments. Live
  inserts and live-played drums stay on the MPC.
- Offloaded parts add 2 periods by default (5.8 ms at 128 frames).
- The remote side is Linux first, other systems later.
- When the link drops, sends fall back to the local Eco tier.

The code is `rust/mpfree-offload`.

## How it works

- **The MPC stays the only clock.** Each period, the MPC sends block n's
  events and audio as one UDP datagram, numbered n. The server renders it
  with the same Rust node the MPC would run and sends the answer back. The
  server never plays anything, so there is no drift between two sound cards
  and no sample-rate conversion.
- **Answers are played L periods later** (`Config::latency`, 2 by
  default). Block n's answer has L periods to come back: 5.8 ms at L = 2.
  `Remote::latency_frames` gives the delay for delay compensation, and a
  reverb send can take it off its pre-delay so the return lands where it
  would have locally.
- **A late or lost answer is a miss**, the same contract as the plugin
  sandbox and the lanes: `Remote` gives silence for that block and the
  partition carries on. Nothing ever waits for the network. The offloaded
  node (below) fades a miss out, and if the link stays down, a stand-in
  here takes over.
- **The audio thread never touches a socket.** `Remote::process` writes the
  request into a ring and takes block n − L's answer from a table, both
  lock-free and without allocating (`tests/no_alloc.rs` checks). The
  `Link`'s two threads do the network: one sends what the ring holds, the
  other files each answer under its block number. The table's slots are
  seqlocks, so a half-written answer is never played. There is no `unsafe`
  in the crate; the only system call on the audio thread is a futex wake,
  and only when the sender thread is asleep.

## The datagram (`wire.rs`)

One block of one partition, each way, little-endian. It fits a 1500-byte
MTU, so a block is never split and a lost datagram loses one block.

| Offset | Size | Field |
|---|---|---|
| 0 | 4 | magic `MPFO` |
| 4 | 1 | version (1) |
| 5 | 1 | kind: 1 request, 2 reply |
| 6 | 2 | partition |
| 8 | 8 | block number |
| 16 | 8 | the block's first frame on the MPC's clock |
| 24 | 2 | frames, 1 to 128 |
| 26 | 1 | channels of audio that follow: 0 (an instrument's request) or 2 |
| 27 | 1 | events that follow, 0 to 34 (a reply has none) |
| 28 | 2 | flags: in a request, CLEAR (reset the node first); in a reply, GAP (requests went missing before this one) and RESET (the node gave a non-finite sample and was reset) |
| 30 | 2 | the server's render time for the block, µs |
| 32 | 12 each | events: frame offset, kind, note, parameter id, value |
| ... | 4 each | audio: the left channel, then the right, as f32 |

A stereo period of 128 frames is 1,056 bytes each way: 2.9 Mbit/s per
partition, so eight partitions use about 23 Mbit/s of a gigabit link.

`parse` refuses anything else:
- a wrong length, magic, version or kind;
- no frames, more than 128 frames, or channels other than 0 or 2;
- more than 34 events, or an event past the block's end or out of order;
- a sample or value that isn't finite;
- a block number or frame of 2^62 or more.

`tests/robust_wire.rs` feeds the reader and the server 50,000 damaged
datagrams. Each one gets an error or a reply that keeps these promises,
never a panic.

## The server (`server.rs`, `mpfree-offload serve`)

- One partition per node named on the command line: `echo` (sends its input
  back, for timing the network alone), `cloud-full` or `cloud-eco`.
- It listens on 127.0.0.1 unless given `--allow-lan`, because it has no
  pairing yet. A datagram can only carry audio and events to render; nothing
  in one can run a command or touch a file.
- Block numbers must rise:
  - A request up to 64 blocks behind the last one is a late copy, and gets
    no answer, since answering would play the node backwards.
  - A jump ahead is a gap. The answer carries the GAP flag.
  - A request further behind than 64 blocks is a new stream. The node is
    reset and follows it. So one damaged or forged block number can't lock a
    partition out.
- A request with CLEAR resets the node first. A client sets CLEAR on four
  requests in a row so one lost datagram doesn't lose it, and the server
  resets on the first of a run.
- It stops when its standard input closes.

## The offloaded node (`offloaded.rs`)

`Offloaded` wraps a link as an ordinary `Node`, so it goes in a mixer
return's insert chain like any effect. `cloud_send` builds the first one:
Cloud Full on the server, Cloud Eco here as the stand-in.

What it plays:

| What the link does | What you hear |
|---|---|
| Answers on time | The server's answer, exactly |
| One answer missing | The last block heard, played backwards from its last sample and faded out over the block; the next answer fades in over a block |
| No answers for L periods | The stand-in, faded in over a block. The song keeps playing |
| Answers again, 32 in a row (93 ms) | An equal-power crossfade back to the server over 46 ms; then the stand-in stops |

How it stays click-free and on time:
- **The stand-in starts at the first miss, unheard.** For its first L
  periods it is silent by construction, so if the link comes back in that
  time it stops again and nobody hears it. It runs only while it is needed,
  so the MPC pays for Cloud Eco only during an outage. Its tail starts from
  the first miss; the server's tail from before then is lost.
- **The pre-delay takes the latency.** The server renders Cloud with its
  pre-delay shortened by L periods, and the stand-in with it at least L
  periods long. Both land where Cloud here would have, so `latency()` is 0
  and the mixer delays nothing. A pre-delay set shorter than 5.8 ms (L = 2)
  acts as 5.8 ms. A node without such a parameter reports L periods of
  latency for the mixer to compensate, and its stand-in is held back to
  match.
- **Parameters travel in the requests.** A change goes at the next block's
  start, and a timed event keeps its frame. Each block also resends one
  parameter in turn, so a change lost with a datagram is put right within
  44 ms for Cloud. A block with more changes than fit holds the rest over
  to the next one.
- **A reset clears both sides.** The stand-in stops, the server's node is
  reset with CLEAR, and the L answers already on their way play as silence.
- **Blocks must be the link's period**, which is what the audio thread
  processes. A block of another length counts as a miss, and the stand-in
  covers it.

`process` never blocks, locks or allocates, in any of these states.

## Measuring a link (`mpfree-offload measure`)

```
cargo build --manifest-path rust/Cargo.toml --release --locked -p mpfree-offload
rust/target/release/mpfree-offload measure --node cloud-full --partitions 1 --latency 2 --seconds 20
```

This plays blocks at the real period, as an audio thread would, through N
partitions. Without `--server`, it starts `serve` on loopback as a separate
process. It reports:
- the round trip's p50, p99, p99.9 and worst;
- the server's own render time;
- what each partition costs this side's audio thread per block;
- the misses, late answers and lost datagrams;
- the link's bit rate;
- whether every answer matched the same node rendered here, L blocks later.

It exits 1 on any miss or mismatch.

First numbers, on the shared development VM over loopback (5 s each,
L = 2, so a 5,805 µs window):

| Partitions | Round trip p50 / p99.9 / worst (µs) | Server render p50 (µs) | Audio thread per block p50 / p99 (µs) | Misses |
|---|---|---|---|---|
| 1 × echo | 270 / 1,448 / 1,561 | 0 | 16 / 44 | 0 |
| 1 × Cloud Full | 419 / 2,559 / 2,869 | 136 | 15 / 47 | 0 |
| 8 × Cloud Eco | 268 / 1,914 / 2,439 | 47 | 12 / 34 | 0 |
| 4 × Cloud Full | 389 / 3,643 / 5,786 | 127 | 13 / 38 | 10, in one 6 ms stall |

Every answer that came back in time matched the local render bit for bit.

How to read them:
- They are the VM's numbers, not the MPC's. The server renders its
  partitions one after another on one thread; four Cloud Full partitions
  are about 0.5 ms of render per period before any stall.
- The measuring "audio thread" runs at normal priority here, so waking the
  sender thread can let the kernel switch to it on the spot. That cost shows
  in "audio thread per block". On the MPC, the audio thread is SCHED_FIFO,
  so a wake doesn't preempt it.

## Tests

- `wire.rs` and `server.rs` unit tests cover the layout, the limits, gaps,
  late copies and new streams.
- `tests/loopback.rs`, with a real server on loopback:
  - each answer is the local render L blocks later, bit for bit (Cloud Full
    at L = 2 and 5, Cloud Eco at L = 1);
  - a lost request (a relay drops one) is exactly one silent block, and the
    next answer carries GAP;
  - with no server, every block is silent and nothing hangs.
- `tests/offloaded.rs`, with a real server on loopback and losses made by a
  relay or by stopping the server, so each run is the same:
  - while the link holds, the server is heard exactly;
  - one lost request is one block faded out and one faded in, exactly, and
    the stand-in is never heard;
  - 40 lost requests: the stand-in is heard after L periods and lines up
    exactly; after 32 answers the crossfade back follows the equal-power
    curve; then the server again, exactly;
  - when the server stops, or there is none, the stand-in plays on;
  - none of these jumps by more than 0.03 between two samples of a sine
    that moves 0.016 at most (no clicks);
  - a Cloud send's tail starts on the same frame (±1) as Cloud's here, and
    the stand-in's does too (±2, Eco works in pairs of frames);
  - parameter changes, set and timed, give the server's Cloud Full output
    bit for bit as Cloud here with the same changes;
  - after a reset, the server's tail is gone at once.
- `tests/no_alloc.rs`: `Remote::process` allocates nothing, on hits and
  misses alike, and neither does a Cloud send through an outage, the
  stand-in and the crossfade back.

The tests wait for each answer (up to 10 s) instead of racing a deadline, so
a busy CI runner can't fail them.

## Not yet

In rough order:
1. Two machines over gigabit Ethernet: the same tool with `--server` and a
   server started with `--allow-lan`.
2. Using it in the app: a mixer return built with `cloud_send`, behind a
   Dev setting, with the link's state on the Dev page. The app thread owns
   the audio loop, so it would build that part.
3. A warm stand-in that runs all the time, as an option: a fallback would
   keep the tail, at the cost of Cloud Eco's CPU always.
4. Pairing and a control channel (TCP) for setting up partitions,
   parameters and samples. Then many partitions per socket and one network
   thread, and one render thread per partition on the server.
5. On the MPC, a read-only UDP round-trip probe from the test build (no
   audio). It needs its own review and Jay's go under rule zero, like every
   device step.
6. The USB-B port as a network gadget, if the kernel has it. That changes
   USB state, so it needs its own safety review.

The older `rust/mpfree-network` crate models a different design: streaming
PCM with a playout buffer, where the far side keeps its own clock. The plan
replaced it with numbered blocks.
