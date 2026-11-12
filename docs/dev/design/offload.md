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
  sandbox and the lanes: that block is silent and the partition carries on.
  Nothing ever waits for the network.
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
| 28 | 2 | flags: GAP (requests went missing before this one), RESET (the node gave a non-finite sample and was reset) |
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
- It stops when its standard input closes.

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
- `tests/no_alloc.rs`: `Remote::process` allocates nothing, on hits and
  misses alike.

The tests wait for each answer (up to 10 s) instead of racing a deadline, so
a busy CI runner can't fail them.

## Not yet

In rough order:
1. Two machines over gigabit Ethernet: the same tool with `--server` and a
   server started with `--allow-lan`.
2. A local fallback with a short fade when answers stop (the plan: local
   Eco for a send), and a `Node` wrapper that reports `latency()` for delay
   compensation.
3. Pairing and a control channel (TCP) for setting up partitions,
   parameters and samples. Then many partitions per socket and one network
   thread, and one render thread per partition on the server.
4. On the MPC, a read-only UDP round-trip probe from the test build (no
   audio). It needs its own review and Jay's go under rule zero, like every
   device step.
5. The USB-B port as a network gadget, if the kernel has it. That changes
   USB state, so it needs its own safety review.

The older `rust/mpfree-network` crate models a different design: streaming
PCM with a playout buffer, where the far side keeps its own clock. The plan
replaced it with numbered blocks.
