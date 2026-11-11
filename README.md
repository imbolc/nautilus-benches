# Polymarket subscription memory benchmark

Pending Polymarket reconciliation tasks embed large WebSocket-operation futures.
This benchmark measures their transient memory cost using the real Nautilus data
client and a local HTTP/WebSocket fixture. The proposed change boxes wire operations
after acquiring the existing reconciliation lock, keeping waiting tasks small.

Linux, Rust 1.99+, Python 3.11+, curl and a user systemd session are required.
Run every build and sample through `scripts/limited.sh`: **2 GiB RAM, zero swap,
one CPU**, one Cargo job, low CPU and idle I/O priority. Concurrent runs are rejected.

## Baseline

Official Nautilus `6ba49340934e79c53077cb2cbf174a42cb6d7445`, pinned in `Cargo.toml` and `Cargo.lock`:

```sh
sh scripts/limited.sh cargo build -q --release
sh scripts/limited.sh python3 scripts/run.py
```

| Instruments | Queued heap added (MiB) | Peak heap added (MiB) | Peak RSS (MiB) | CPU (s) |
| ---: | ---: | ---: | ---: | ---: |
| 128 | 14.38 | 14.38 | 39.18 | 0.006 |
| 512 | 57.51 | 57.51 | 90.13 | 0.026 |
| 2,048 | 230.06 | 230.06 | 297.24 | 0.107 |
| 8,000 | 898.71 | 898.71 | 1100.96 | 0.418 |

[Raw baseline results](results/baseline.json)

## Proposed version

Use a clean Nautilus checkout at `0ab26e047dc2dfdb4a154e69e93d81fd1b566b93`
on branch `reduce-polymarket-subscription-memory`. Adjust the checkout path below:

```sh
sh scripts/limited.sh python3 scripts/proposed.py ../nautilus_trader
```

| Instruments | Queued heap added (MiB) | Peak heap added (MiB) | Peak RSS (MiB) | CPU (s) |
| ---: | ---: | ---: | ---: | ---: |
| 128 | 1.24 | 1.25 | 25.78 | 0.003 |
| 512 | 4.98 | 4.98 | 38.54 | 0.012 |
| 2,048 | 19.91 | 19.92 | 86.95 | 0.051 |
| 8,000 | 77.85 | 77.85 | 280.04 | 0.220 |

[Raw proposed results](results/proposed.json) and
[resolved proposed lockfile](results/proposed.Cargo.lock).
The runner records the checkout revision, patches dependencies locally, and restores
the baseline lockfile afterward. The allocator self-check runs before sampling.

## Reading the results

Each row is the median of three fresh processes with one Nautilus runtime worker,
using rustc 1.99.1 (58cf8f9c3 2026-10-13). Both versions use the same fixture, harness and workload.
Raw results include allocation/reallocation calls, requested bytes, live/peak heap,
current/peak RSS, CPU and elapsed time, plus source and fixture checksums.

After preloading instruments, the harness gates the worker, submits book, quote,
trade, status and close subscriptions, and measures the queued state. It releases
the gate, verifies every wire token and initial book/quote, then disconnects through
the native client lifecycle. Peak heap covers the entire measured phase through
disconnect. [Workload implementation](src/subscriptions.rs)

Queued and peak heap columns subtract live heap at phase start. They include intents
and registration state as well as futures. Rust counters exclude native C allocations;
process RSS includes setup. The fixture process is excluded from process metrics
but shares the aggregate cgroup limit. Recorded cgroup peaks also include builds.
This gated burst is an upper-bound workload,
not a live-deployment memory estimate. CPU includes counter overhead and does not
establish uninstrumented application throughput. Failure, reconnect and book recovery
require Nautilus's separate regression tests.

## Inputs

The [checksummed fixture](data/SHA256SUMS) contains 40 untouched Gamma pages captured
on 2026-10-19 UTC: 4,000 markets and 8,000 outcome tokens. Only served copies have
expiry moved to 2099-01-01. Initial v1 books are synthetic, with bid 0.4, ask 0.6 and
fixed sizes/timestamps. Acquisition and loading precede measurement; all connections
target loopback and no live venue is contacted.
