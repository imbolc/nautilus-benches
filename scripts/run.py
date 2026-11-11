import argparse
import hashlib
import json
import os
import platform
import subprocess
import tempfile
from pathlib import Path

BENCHMARK = "polymarket-subscription-memory"
ROOT = Path(__file__).resolve().parents[1]


def cgroup():
    relative = next(line.split(":", 2)[2] for line in Path("/proc/self/cgroup").read_text().splitlines() if line.startswith("0::"))
    group = Path("/sys/fs/cgroup") / relative.lstrip("/")
    assert (group / "memory.max").read_text().strip() == "2147483648"
    assert (group / "memory.swap.max").read_text().strip() == "0"
    quota, period = (group / "cpu.max").read_text().split()
    assert quota != "max" and int(quota) <= int(period)
    return group


def digest(path):
    value = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(65536), b""):
            value.update(block)
    return value.hexdigest()


def clean(directory):
    # This directory was created by this sample; never follow external symlinks.
    for path in sorted(directory.rglob("*"), key=lambda path: len(path.parts), reverse=True):
        if path.is_symlink() or path.is_file():
            path.unlink()
        else:
            path.rmdir()
    directory.rmdir()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--repeats", type=int, default=3)
    args = parser.parse_args()
    assert args.repeats > 0
    group = cgroup()
    target = Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target"))
    binary = target / "release/nautilus-bench"
    subprocess.run([str(binary), "allocator-check"], check=True, cwd=ROOT)
    if BENCHMARK == "polymarket-subscription-memory":
        for line in (ROOT / "data/SHA256SUMS").read_text().splitlines():
            expected, name = line.split(maxsplit=1)
            assert digest(ROOT / "data" / name.lstrip("*")) == expected, name
        cases = [("subscriptions", count) for count in (128, 512, 2048, 8000)]
    else:
        cases = [(mode, count) for count in (5000, 20000) for mode in ("feather-direct", "feather-staged")]
    runs = ROOT / "var/runs"
    runs.mkdir(parents=True, exist_ok=True)
    rows = []
    for mode, count in cases:
        for repeat in range(1, args.repeats + 1):
            directory = Path(tempfile.mkdtemp(prefix="sample-", dir=runs))
            try:
                command = [str(binary), mode]
                if mode != "subscriptions":
                    command.append(str(directory / "output"))
                command.append(str(count))
                result = subprocess.run(command, cwd=ROOT, capture_output=True, text=True, timeout=240)
                if result.returncode:
                    raise RuntimeError(f"{command}: exit {result.returncode}\n{result.stdout}\n{result.stderr}")
                row = json.loads(result.stdout)
                row["repeat"] = repeat
                rows.append(row)
                print(f"{mode} {count}, repeat {repeat}: {row['metrics']['cpu_seconds']:.3f}s CPU", flush=True)
            finally:
                clean(directory)
    paths = [ROOT / "Cargo.toml", ROOT / "Cargo.lock", *sorted((ROOT / "src").glob("*.rs")), *sorted((ROOT / "scripts").glob("*.*"))]
    source_hashes = {str(path.relative_to(ROOT)): digest(path) for path in paths if path.is_file()}
    output = {
        "benchmark": BENCHMARK,
        "platform": platform.platform(),
        "rustc": subprocess.check_output(["rustc", "--version"], text=True).strip(),
        "profile": "release",
        "allocator": "System with Rust global allocation counters",
        "limits": {"ram_bytes": 2147483648, "swap_bytes": 0, "cpu_quota": 1, "cargo_jobs": 1, "io_priority": "idle"},
        "cgroup_peak_memory_bytes_including_page_cache": int((group / "memory.peak").read_text()),
        "source_sha256": source_hashes,
        "samples": rows,
    }
    destination = ROOT / "results/baseline.json"
    destination.parent.mkdir(exist_ok=True)
    destination.write_text(json.dumps(output, indent=2) + "\n")
    print(f"Verified {len(rows)} samples; saved {destination}", flush=True)


if __name__ == "__main__":
    main()
