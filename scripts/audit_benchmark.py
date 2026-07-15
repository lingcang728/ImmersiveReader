from __future__ import annotations

import hashlib
import json
import os
import platform
import shutil
import statistics
import subprocess
import sys
import tempfile
import time
import tracemalloc
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


def percentile(values: list[float], ratio: float) -> float:
    ordered = sorted(values)
    index = min(len(ordered) - 1, max(0, round((len(ordered) - 1) * ratio)))
    return round(ordered[index], 3)


def summary(values: list[float]) -> dict[str, float]:
    return {
        "p50_ms": percentile(values, 0.5),
        "p95_ms": percentile(values, 0.95),
        "mean_ms": round(statistics.fmean(values), 3),
    }


def command_version(command: list[str]) -> str:
    try:
        result = subprocess.run(command, capture_output=True, text=True, check=True)
    except (OSError, subprocess.CalledProcessError):
        return "unavailable"
    return result.stdout.strip().splitlines()[0] if result.stdout.strip() else result.stderr.strip().splitlines()[0]


def benchmark_library() -> dict[str, dict[str, float | int]]:
    results: dict[str, dict[str, float | int]] = {}
    for count in (100, 1000, 10000):
        with tempfile.TemporaryDirectory(prefix="immersive-audit-library-") as temporary:
            root = Path(temporary)
            manifest = json.dumps(
                {
                    "schemaVersion": 1,
                    "bookId": "manual:synthetic",
                    "title": "synthetic",
                    "source": "manual",
                    "generatedAt": "2026-07-10T00:00:00.000Z",
                    "updatedAt": "2026-07-10T00:00:00.000Z",
                    "chapters": [
                        {"id": "chapter:1", "path": "001.md", "title": "one", "voteCount": 0, "wordCount": 1}
                    ],
                }
            ).encode()
            for index in range(count):
                book = root / "manual" / f"book-{index:05d}"
                book.mkdir(parents=True)
                (book / "manifest.json").write_bytes(manifest)
            measurements: list[float] = []
            found = 0
            bytes_read = 0
            for _ in range(5):
                started = time.perf_counter()
                found = 0
                bytes_read = 0
                for path in root.rglob("manifest.json"):
                    bytes_read += path.stat().st_size
                    json.loads(path.read_bytes())
                    found += 1
                measurements.append((time.perf_counter() - started) * 1000)
            results[str(count)] = {**summary(measurements), "manifests": found, "bytes_read": bytes_read}
    return results


def benchmark_cache() -> dict[str, object]:
    with tempfile.TemporaryDirectory(prefix="immersive-audit-cache-") as temporary:
        root = Path(temporary)
        for index in range(1000):
            (root / f"item-{index:04d}.bin").write_bytes(bytes([index % 251]) * 4096)
        hash_times: list[float] = []
        stat_times: list[float] = []
        hashed_bytes = 0
        stat_bytes = 0
        for _ in range(5):
            started = time.perf_counter()
            digest = hashlib.sha256()
            hashed_bytes = 0
            for path in root.iterdir():
                data = path.read_bytes()
                digest.update(data)
                hashed_bytes += len(data)
            digest.digest()
            hash_times.append((time.perf_counter() - started) * 1000)
            started = time.perf_counter()
            stat_bytes = sum(path.stat().st_size for path in root.iterdir())
            stat_times.append((time.perf_counter() - started) * 1000)
        return {
            "files": 1000,
            "hashed_bytes": hashed_bytes,
            "stat_bytes": stat_bytes,
            "content_hash_before": summary(hash_times),
            "metadata_scan_after": summary(stat_times),
        }


def benchmark_chunk_plan() -> dict[str, dict[str, float | int]]:
    scripts = ROOT / "tools" / "podcast-transcriber" / "scripts"
    sys.path.insert(0, str(scripts))
    from podcast_transcriber.chunk_plan import ChunkPlan

    results: dict[str, dict[str, float | int]] = {}
    for label, count in (("1h", 60), ("10h", 600)):
        durations = [60.0] * (count - 1) + [59.25]
        measurements: list[float] = []
        signature = ""
        for _ in range(20):
            started = time.perf_counter()
            plan = ChunkPlan.from_durations(durations)
            signature = plan.signature()
            measurements.append((time.perf_counter() - started) * 1000)
        results[label] = {**summary(measurements), "chunks": count, "signature_prefix": signature}
    return results


def benchmark_reader() -> dict[str, object]:
    with tempfile.TemporaryDirectory(prefix="immersive-audit-reader-") as temporary:
        path = Path(temporary) / "large.md"
        path.write_bytes(b"x" * (16 * 1024 * 1024))
        direct: list[float] = []
        streaming: list[float] = []
        direct_peak = 0
        streaming_peak = 0
        for _ in range(3):
            tracemalloc.start()
            started = time.perf_counter()
            value = path.read_bytes()
            direct.append((time.perf_counter() - started) * 1000)
            _, direct_peak = tracemalloc.get_traced_memory()
            del value
            tracemalloc.stop()
            tracemalloc.start()
            started = time.perf_counter()
            with path.open("rb") as stream:
                while stream.read(64 * 1024):
                    pass
            streaming.append((time.perf_counter() - started) * 1000)
            _, streaming_peak = tracemalloc.get_traced_memory()
            tracemalloc.stop()
        return {
            "bytes": path.stat().st_size,
            "direct_read": {**summary(direct), "peak_traced_bytes": direct_peak},
            "stream_read": {**summary(streaming), "peak_traced_bytes": streaming_peak},
        }


def benchmark_task_views() -> dict[str, dict[str, float]]:
    tasks = [{"updated_at": index, "title": f"synthetic task {index}"} for index in range(10000)]
    sort_times: list[float] = []
    search_times: list[float] = []
    for _ in range(10):
        started = time.perf_counter()
        sorted(tasks, key=lambda item: item["updated_at"], reverse=True)
        sort_times.append((time.perf_counter() - started) * 1000)
        started = time.perf_counter()
        [item for item in tasks if "9999" in item["title"]]
        search_times.append((time.perf_counter() - started) * 1000)
    return {"task_sort_10000": summary(sort_times), "search_10000": summary(search_times)}


def environment() -> dict[str, object]:
    usage = shutil.disk_usage(ROOT)
    return {
        "platform": platform.platform(),
        "processor": platform.processor(),
        "cpu_count": os.cpu_count(),
        "python": sys.version.split()[0],
        "node": command_version(["node", "--version"]),
        "rustc": command_version(["rustc", "--version"]),
        "cargo": command_version(["cargo", "--version"]),
        "workspace_disk_free_bytes": usage.free,
        "mode": "synthetic-temp-data",
    }


def main() -> int:
    print(
        json.dumps(
            {
                "environment": environment(),
                "library_manifest_scan": benchmark_library(),
                "cache_cleanup": benchmark_cache(),
                "chunk_plan": benchmark_chunk_plan(),
                "reader": benchmark_reader(),
                "task_views": benchmark_task_views(),
            },
            ensure_ascii=False,
            indent=2,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
