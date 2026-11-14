import argparse
from datetime import UTC, datetime
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tomllib

ROOT = Path(__file__).resolve().parents[1]
CRATES = {
    "nautilus-common": "crates/common",
    "nautilus-core": "crates/core",
    "nautilus-model": "crates/model",
    "nautilus-network": "crates/network",
    "nautilus-polymarket": "crates/adapters/polymarket",
}


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--checkout", type=Path)
    parser.add_argument("--expect", choices=("stale", "fresh"), default="stale")
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    label = "proposed" if args.checkout else "baseline"
    output = args.output or Path(f"results/{label}.json")
    manifest = tomllib.loads((ROOT / "Cargo.toml").read_text())
    revision = manifest["dependencies"]["nautilus-polymarket"]["rev"]
    command = ["cargo", "build", "-q"]
    lock = ROOT / "Cargo.lock"
    baseline_lock = lock.read_bytes()
    (ROOT / "var").mkdir(exist_ok=True)
    if args.checkout:
        checkout = args.checkout.resolve()
        revision = subprocess.check_output(
            ["git", "rev-parse", "HEAD"], cwd=checkout, text=True
        ).strip()
        if subprocess.check_output(["git", "status", "--short"], cwd=checkout, text=True):
            raise RuntimeError("Nautilus checkout must be clean")
        config = ROOT / "var/checkout.toml"
        config.write_text(
            '[patch."https://github.com/nautechsystems/nautilus_trader"]\n'
            + "".join(
                f"{name} = {{ path = {json.dumps(str(checkout / path))} }}\n"
                for name, path in CRATES.items()
            )
        )
        command.extend(["--config", str(config)])
    else:
        command.append("--locked")
    try:
        print(f"Building {label} {revision}", flush=True)
        build = subprocess.run(command, cwd=ROOT, capture_output=True, text=True)
        (ROOT / f"var/{label}-build.log").write_text(build.stdout + build.stderr)
        if build.returncode:
            raise RuntimeError(build.stdout + build.stderr)
        resolved_lock = lock.read_bytes()
        run = subprocess.run(
            [str(ROOT / "target/debug/polymarket-retained-refresh")],
            cwd=ROOT, capture_output=True, text=True, timeout=110,
            env={key: value for key, value in os.environ.items()
                 if not key.startswith("POLYMARKET_")},
        )
        (ROOT / f"var/{label}-run.log").write_text(run.stdout + run.stderr)
        expected_exit = 0 if args.expect == "fresh" else 1
        if run.returncode != expected_exit:
            raise RuntimeError(
                f"expected exit {expected_exit}, received {run.returncode}\n"
                + run.stdout + run.stderr
            )
        report = json.loads(run.stdout)
        if report["fresh"] != (args.expect == "fresh"):
            raise RuntimeError("reported freshness does not match expected outcome")
        source_paths = [
            ROOT / "Cargo.toml", *sorted((ROOT / "src").glob("*.rs")),
            *sorted((ROOT / "scripts").glob("*")), ROOT / "data/market.json",
        ]
        result = {
            "nautilus_revision": revision,
            "run_at_utc": datetime.now(UTC).isoformat(),
            "rustc": subprocess.check_output(["rustc", "--version"], text=True).strip(),
            "expected_outcome": args.expect,
            "exit_code": run.returncode,
            "cargo_lock_sha256": hashlib.sha256(resolved_lock).hexdigest(),
            "source_sha256": {
                str(path.relative_to(ROOT)): digest(path)
                for path in source_paths if path.is_file()
            },
            "run": report,
        }
        output = ROOT / output
        output.parent.mkdir(parents=True, exist_ok=True)
        if args.checkout:
            output.with_suffix(".Cargo.lock").write_bytes(resolved_lock)
        output.write_text(json.dumps(result, indent=2) + "\n")
        print(f"Verified {args.expect} metadata; saved {output.relative_to(ROOT)}", flush=True)
    finally:
        lock.write_bytes(baseline_lock)


if __name__ == "__main__":
    try:
        main()
    except (OSError, RuntimeError, ValueError, subprocess.SubprocessError) as error:
        print(error, file=sys.stderr)
        sys.exit(1)
