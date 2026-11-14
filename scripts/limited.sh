#!/bin/sh
set -eu

bench_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
exec systemd-run --user --wait --pipe --collect \
    --unit=nautilus-retained-refresh \
    --working-directory="$bench_root" \
    --property=MemoryMax=2G \
    --property=MemorySwapMax=0 \
    --property=CPUQuota=100% \
    --property=IOWeight=10 \
    --property=IOSchedulingClass=idle \
    --property=Nice=10 \
    /bin/sh "$bench_root/scripts/inside-limit.sh" "$@"
