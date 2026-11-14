# Polymarket retained metadata reproduction

Polymarket periodic discovery can stop refreshing a market's instrument definition
while an open position still depends on it. If the venue extends the closing time
beyond the configured discovery date range, Nautilus keeps the old closing time.

This standalone correctness reproduction runs the real data client against one
synthetic Gamma market. It publishes a normal open-position event, extends the
mock market's closing time, and checks instrument updates after periodic refresh.
All endpoints point to loopback; no credentials or live trades are involved.

Linux, Rust 1.100+, Python 3.11+, and a user systemd session are required.
Run builds and samples through `scripts/limited.sh`: **2 GiB RAM, zero swap,
one CPU**, and one Cargo job. Each sample takes about 65 seconds because the
public refresh interval has a one-minute minimum.

## Baseline

Official Nautilus revision `b9e6f468f8dc35088a06742aadb1866e9c274328`,
pinned in `Cargo.toml` and `Cargo.lock`:

```sh
sh scripts/limited.sh python3 scripts/run.py
```

| Venue closing date (UTC) | Emitted closing date (UTC) | Metadata updates | Outcome |
| --- | --- | ---: | --- |
| 2099-12-01 | 2099-01-01 | 0 | Stale |

[Raw baseline result](results/baseline.json), including source checksums and the
Gamma request trace. Recorded with Rust 1.100.0 on 2026-11-14 UTC.

The runner expects stale metadata and verifies exit code 1 from the reproducer.
Exit code 2 means setup or validation failed and is never accepted as reproduction.
Build and run diagnostics are saved under `var/`.

## Proposed version

Pass a clean Nautilus checkout containing the retained-refresh change, adjusting
the path below. Recorded results use local merge revision
`af1811c29252929de0b3925060fff316ae16893e`, which combines the baseline above
with the retained-refresh PR head `3147f20283c4355212be0d93429f2f92cdbd8264`.

```sh
sh scripts/limited.sh python3 scripts/run.py --checkout /path/to/nautilus_trader --expect fresh
```

| Venue closing date (UTC) | Emitted closing date (UTC) | Metadata updates | Outcome |
| --- | --- | ---: | --- |
| 2099-12-01 | 2099-12-01 | 1 | Fresh |

[Raw proposed result](results/proposed.json) and
[resolved proposed lockfile](results/proposed.Cargo.lock). Recorded with Rust
1.100.0 on 2026-11-14 UTC, using the same harness, fixture, and registry dependency
versions as the baseline. The request trace shows a condition-ID lookup without
the discovery date filter.

The runner patches dependencies to the supplied checkout, verifies exit code 0,
records source checksums and the resolved lockfile, and restores the baseline
lockfile. Baseline and proposed samples both use the real one-minute refresh
interval and observe the watched Yes outcome for 65 seconds. Query ordering
between discovery and retained refresh is intentionally unconstrained.

## Scenario

1. Discovery uses `end_date_max=2099-06-01T00:00:00Z` and loads market M,
   initially closing at `2099-01-01T00:00:00Z`
2. A synthetic `PositionOpened` event reaches the client's normal message-bus
   handler, leaving an open-position resolution watch for M
3. The fixture extends M's closing time to `2099-12-01T00:00:00Z`.
   Subsequent discovery excludes M; a direct condition-ID lookup can still return it
4. After one refresh interval plus five seconds of delivery slack, the harness
   checks the most recently emitted definition for M

A completed discovery request excluding M is required; an idle or broken refresh
task does not qualify as reproduction. Both dates remain in the future, so expiry
retirement and settlement cannot explain the result.

The fixture and results describe a synthetic adapter failure, not an observed
live-venue incident. The harness reads public data events, not private watchlist
fields. It does not demonstrate watch-deadline updates, reconnect restoration,
order-only ownership, quota fairness, or response-ordering failures. Those need
separate justification if included in the proposed fix.
