# Nautilus Polymarket bootstrap benchmark

Nautilus populates its Polymarket instrument cache one instrument at a time.
Each
[`AtomicMap::insert`](https://github.com/nautechsystems/nautilus_trader/blob/8705495b380f9b1569cf048d1c0f664707a67a72/crates/core/src/collections.rs#L63-L104)
clones the existing map, so loading K distinct instruments into an empty cache
copies K(K-1)/2 existing entries. The
[`cache_and_publish_instruments` path](https://github.com/nautechsystems/nautilus_trader/blob/8705495b380f9b1569cf048d1c0f664707a67a72/crates/adapters/polymarket/src/data/instruments.rs#L113-L294)
performs these insertions during startup.

This standalone reproducer calls the real Nautilus data client's `connect`
against saved Gamma responses. It pins official Nautilus revision
`8705495b380f9b1569cf048d1c0f664707a67a72`.

## Baseline

```sh
cargo run --release -- run
```

| Markets | Instruments | CPU (s) | CPU/instrument (µs) | RAM/instrument (KiB) |
| ------: | ----------: | ------: | ------------------: | -------------------: |
|     500 |        1000 |    0.55 |              550.00 |                64.17 |
|    1000 |        2000 |    2.14 |             1070.00 |                53.72 |
|    2000 |        4000 |   10.02 |             2505.00 |                49.31 |
|    4000 |        8000 |   61.79 |             7723.75 |                47.16 |

## Proposed version

```sh
cargo run --release --config .cargo/proposed.toml -- run
```

| Markets | Instruments | CPU (s) | CPU/instrument (µs) | RAM/instrument (KiB) |
| ------: | ----------: | ------: | ------------------: | -------------------: |
|     500 |        1000 |    0.01 |               10.00 |                53.46 |
|    1000 |        2000 |    0.03 |               15.00 |                42.77 |
|    2000 |        4000 |    0.06 |               15.00 |                37.83 |
|    4000 |        8000 |    0.15 |               18.75 |                35.89 |

## Captured data

```sh
cargo run --release -- capture
```

Bundled `data/` contains 40 untouched Gamma `/markets/keyset` responses captured
on 2026-10-19 UTC: 4,000 real markets selected with `active=true`,
`closed=false` and 100 markets per page, totaling 24.3 MB of market metadata.
The fixture contains 8,000 outcome tokens.
[Capture metadata](data/index.json) and [SHA-256 checksums](data/SHA256SUMS) are
included.
