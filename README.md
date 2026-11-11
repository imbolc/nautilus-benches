# Polymarket subscription memory benchmark

Measures the memory held by pending Polymarket subscription futures using the real
Nautilus data client and a local HTTP/WebSocket fixture. Nautilus is pinned to
official revision `86e640d623747e5f07b7b7df90e4585193341055`.
No Nautilus source changes are required.

## Run

Linux, Rust 1.99 or later, Python 3, curl and a user systemd session are required.
Run commands from this checkout:

```sh
sh scripts/limited.sh cargo build -q --release
sh scripts/limited.sh target/release/nautilus-bench allocator-check
sh scripts/limited.sh python3 scripts/run.py
```

The wrapper verifies a cgroup with 2 GiB RAM, zero swap and one CPU before running
any command. Builds use one Cargo job; work runs at low CPU and idle I/O priority.
Use the wrapper for builds and every benchmark invocation. Concurrent runs share
one unit name and are rejected.

## Workload

Each sample runs in a fresh process. A separate fixture process serves saved Gamma
metadata and deterministic initial WebSocket book snapshots. Setup connects the
real client and loads instruments before measurement.

The benchmark holds Nautilus's single runtime worker at a gate, submits book,
quote, trade, status and close subscriptions for every instrument, and snapshots
the queued heap and RSS before releasing the gate. This isolates a controlled
burst of pending futures. It measures a burst upper bound, not typical startup
latency or a full live recorder.

After release, every instrument must receive an initial book and quote. The wire
subscription token set must match the loaded instruments exactly. Measurement
ends after native disconnect drains the client's tasks. Snapshot-time metrics
are intermediate observations; post-disconnect metrics establish lifecycle
completion. No arbitrary sleep is used as proof that tasks completed.

## Inputs

The 40 checksummed Gamma pages contain 4,000 real markets and 8,000 outcome tokens,
captured on 2026-10-19 UTC. Original files remain unchanged. The local server
sets market expiry to 2099-01-01 to keep this historical fixture independent of
the execution date. Book messages are synthetic: one bid at 0.4, one ask at 0.6,
fixed sizes, timestamp and v1 protocol. Fixture acquisition and loading are outside
the measured phase. Every connection targets loopback; no live venue is contacted.

## Measurements

Rust allocation counters wrap the system allocator without backtraces. They count
successful allocation/reallocation calls and requested bytes, including the full
new size of a successful reallocation. Live bytes include pre-existing allocations;
peak live bytes reset at the phase boundary. Native C allocations are excluded.
RSS covers the benchmark process; its high-water mark includes setup. The fixture
server is excluded from these process metrics but shares the aggregate cgroup
limit. CPU uses the process CPU clock and includes Nautilus worker threads.
Counter overhead and small measurement-bookkeeping allocations are included.

This is a baseline reproducer, not evidence of an optimization or a memory leak.
It verifies the five subscription request paths, initial books/quotes and native
shutdown. Failure, reconnect and recovery behavior remain covered by Nautilus's
own regression tests, rather than being inferred from this performance workload.
Results are written to `results/baseline.json` after successful verification.

## Baseline results

Median of three fresh processes per case, using the pinned revision and lockfile.
Measured with rustc 1.99.1 (58cf8f9c3 2026-10-13). CPU timings include allocator-counter
overhead; they are not uninstrumented application throughput.

The queued heap delta includes subscription intents and registration state as well
as pending futures; it is not allocation-stack attribution. CPU covers the burst
through initial data and disconnect, with instrument loading excluded.

| Instruments | Queued live heap added (MiB) | Process peak RSS (MiB) | CPU (s) |
| ---: | ---: | ---: | ---: |
| 128 | 14.38 | 37.96 | 0.006 |
| 512 | 57.51 | 90.52 | 0.024 |
| 2,048 | 230.06 | 297.35 | 0.097 |
| 8,000 | 898.71 | 1100.81 | 0.399 |

All 12 samples verified every expected wire token, initial book and quote, and
native disconnect. These burst results do not establish the peak memory of a live
deployment.

The aggregate cgroup peak was 1208.8 MiB, including child processes and page cache,
with zero swap. [Raw results](results/baseline.json).
