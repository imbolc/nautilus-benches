import argparse
import json
import os
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
CRATES = {
    "nautilus-common": "crates/common",
    "nautilus-core": "crates/core",
    "nautilus-model": "crates/model",
    "nautilus-network": "crates/network",
    "nautilus-polymarket": "crates/adapters/polymarket",
}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("checkout", type=Path)
    parser.add_argument("--repeats", type=int, default=3)
    args = parser.parse_args()
    checkout = args.checkout.resolve()
    revision = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=checkout, text=True).strip()
    assert not subprocess.check_output(["git", "status", "--porcelain"], cwd=checkout, text=True), "Nautilus checkout must be clean"
    (ROOT / "var").mkdir(exist_ok=True)
    config = ROOT / "var/proposed.toml"
    config.write_text('[patch."https://github.com/nautechsystems/nautilus_trader"]\n' + "".join(
        f"{name} = {{ path = {json.dumps(str(checkout / path))} }}\n" for name, path in CRATES.items()
    ))
    lock = ROOT / "Cargo.lock"
    baseline_lock = lock.read_bytes()
    env = os.environ.copy()
    env["NAUTILUS_BENCH_REVISION"] = revision
    try:
        subprocess.run(["cargo", "build", "-q", "--release", "--config", str(config)], cwd=ROOT, env=env, check=True)
        proposed_lock = ROOT / "results/proposed.Cargo.lock"
        proposed_lock.parent.mkdir(exist_ok=True)
        proposed_lock.write_bytes(lock.read_bytes())
        subprocess.run([
            sys.executable, "scripts/run.py", "--output", "results/proposed.json",
            "--lockfile", "results/proposed.Cargo.lock", "--revision", revision,
            "--repeats", str(args.repeats),
        ], cwd=ROOT, env=env, check=True)
    finally:
        lock.write_bytes(baseline_lock)


if __name__ == "__main__":
    main()
