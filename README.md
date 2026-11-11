# Feather allocation benchmark

Measures allocation traffic, live Rust heap, RSS, process CPU and elapsed time for
NautilusTrader's native Feather writer. The direct Feather writer and the Parquet writer's Feather staging worker receive
identical deterministic events through `write_any`. The staged case disables
promotion on close and periodic promotion, so no Parquet conversion is measured.

Nautilus is pinned to official revision `86e640d623747e5f07b7b7df90e4585193341055`.
No Nautilus source changes are required.

## Run

Linux, Rust 1.99 or later, Python 3 and a user systemd session are required.
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

## Workload and verification

Each fresh process preconstructs an equal mix of quotes, trades, individual book
deltas, instrument statuses and closes across 64 identifiers. Price and quantity
precision changes over time, including for the same identifier. Setup is outside
the measured phase; writing, flush and close are inside it. Files rotate at
256 KiB. The fixture is synthetic and uses exact decimal values.

After measurement, every stored row is decoded and compared with its input.
Checks include every serialized field, row metadata, field metadata, metadata
hashes, ordering, sealed-file time ranges and absence of partial files.

Rust allocation counters wrap the system allocator without backtraces. They count
successful Rust allocation/reallocation calls and requested bytes, including the
full new size of a successful reallocation. Live bytes include pre-existing
allocations; peak live bytes reset at the phase boundary. Native C allocations
are excluded. RSS covers the whole process; its high-water mark includes setup.
CPU uses the process CPU clock and includes worker threads. Counter overhead is
included in timings. Inputs and output verification are excluded.

This is a baseline reproducer, not evidence of an optimization or a memory leak.
It does not cover instrument definitions, custom data or execution records.
Results are written to `results/baseline.json` after successful verification.

## Baseline results

Median of three fresh processes per case, using the pinned revision and lockfile.
Measured with rustc 1.99.1 (58cf8f9c3 2026-10-13). CPU timings include allocator-counter
overhead; they are not uninstrumented application throughput.

| Rows | Route | Alloc + realloc / row | Requested bytes / row | CPU (s) |
| ---: | --- | ---: | ---: | ---: |
| 5,000 | direct | 263.3 | 68,794 | 0.073 |
| 5,000 | staged | 266.3 | 69,592 | 0.100 |
| 20,000 | direct | 265.6 | 69,139 | 0.288 |
| 20,000 | staged | 268.6 | 69,935 | 0.400 |

All 12 samples passed the full row and metadata comparisons. The direct runs had
no staging worker; staged runs observed the native `feather-staging` thread. No
Parquet promotion occurred. Both routes sealed 13 files at 5,000 rows and 45 files
at 20,000 rows. The worker adds request/reply cost without removing per-event
encoding or allocation traffic in this fixture. This does not establish a CPU
improvement attainable by changing the encoder or metadata handling.

The aggregate cgroup peak was 40.8 MiB, including child processes and page cache,
with zero swap. [Raw results](results/baseline.json).
