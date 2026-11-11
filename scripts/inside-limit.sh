#!/bin/sh
set -eu

export PATH="$HOME/.cargo/bin:$PATH"
export CARGO_BUILD_JOBS=1
export RAYON_NUM_THREADS=1
export TOKIO_WORKER_THREADS=1
export NAUTILUS_WORKER_THREADS=1

python3 - <<'PY'
from pathlib import Path

relative = next(line.split(':', 2)[2] for line in Path('/proc/self/cgroup').read_text().splitlines() if line.startswith('0::'))
group = Path('/sys/fs/cgroup') / relative.lstrip('/')
assert (group / 'memory.max').read_text().strip() == '2147483648', '2 GiB RAM limit not active'
assert (group / 'memory.swap.max').read_text().strip() == '0', 'zero-swap limit not active'
quota, period = (group / 'cpu.max').read_text().split()
assert quota != 'max' and int(quota) <= int(period), 'one-CPU quota not active'
print('Verified cgroup: 2 GiB RAM, zero swap, one CPU', flush=True)
PY

exec "$@"
