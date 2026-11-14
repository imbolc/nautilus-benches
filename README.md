# Polymarket stale metadata

A market held by an open position stops receiving metadata updates when its new
closing date falls outside discovery filters. This example extends a synthetic
market from January to December 2099, beyond discovery's June cutoff.

Uses a local Gamma server and a position event. Requires Rust 1.100+; each run
waits 65 seconds for periodic refresh.

## Baseline

[Develop revision b9e6f468f8](https://github.com/nautechsystems/nautilus_trader/commit/b9e6f468f8dc35088a06742aadb1866e9c274328):

```sh
cargo run -q
```

| Venue closes (UTC) | Nautilus closes (UTC) | Result |
| --- | --- | --- |
| 2099-12-01 | 2099-01-01 | Stale |

[Output](results/baseline.txt). Exits 1 when the bug is reproduced.

## Proposed version

[PR revision 3147f20283](https://github.com/imbolc/nautilus_trader/commit/3147f20283c4355212be0d93429f2f92cdbd8264):

```sh
cargo run -q --config proposed.toml
```

| Venue closes (UTC) | Nautilus closes (UTC) | Result |
| --- | --- | --- |
| 2099-12-01 | 2099-12-01 | Fresh |

[Output](results/proposed.txt). Exits 0 when metadata is refreshed.
