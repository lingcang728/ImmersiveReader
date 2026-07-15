# ImmersiveReader audit baseline — 2026-07-15

Baseline commit for the first verification pass: `9a7ab48`. The measured synthetic harness run was executed from the managed-runtime tree before the final library lookup micro-fix; it used only temporary synthetic files and did not read Library, control databases, profiles, credentials, audio, or outputs.

Environment:

- Windows 11 build `10.0.26200`, AMD64, 16 logical CPUs.
- Node `v24.15.0`; Python `3.12.7`; Rust/Cargo `1.92.0`.
- Free workspace disk at measurement: `143,835,017,216` bytes.
- Mode: `synthetic-temp-data`.

## Synthetic performance measurements

| Cost center | Dataset | p50 | p95 | Read/peak evidence |
| --- | ---: | ---: | ---: | --- |
| Library recursive manifest scan | 100 books | 29.628 ms | 32.036 ms | 100 manifests / 27,700 bytes |
| Library recursive manifest scan | 1,000 books | 214.742 ms | 248.292 ms | 1,000 / 277,000 bytes |
| Library recursive manifest scan | 10,000 books | 3,851.257 ms | 4,285.205 ms | 10,000 / 2,770,000 bytes |
| Protected-root content hash pass | 1,000 × 4 KiB | 96.882 ms | 108.858 ms | 4,096,000 bytes hashed |
| Cache metadata scan | same | 11.000 ms | 13.906 ms | same bytes via `stat` |
| Reader whole-file read | 16 MiB | 3.522 ms | 3.695 ms | 16,785,882 traced peak bytes |
| Reader streaming read | same | 4.601 ms | 5.400 ms | 74,469 traced peak bytes |
| Task sort | 10,000 tasks | 0.692 ms | 1.099 ms | synthetic in-memory |
| Task search | 10,000 tasks | 0.381 ms | 0.684 ms | synthetic in-memory |
| ChunkPlan construction | 60 chunks / 1 hour | 0.338 ms | 0.418 ms | nonuniform final chunk |
| ChunkPlan construction | 600 chunks / 10 hours | 2.933 ms | 3.282 ms | nonuniform final chunk |

The cache comparison is the direct before/after evidence for removing protected-root full-content hashing. The Reader comparison is a memory tradeoff: file-backed streaming adds about 1.7 ms p95 in this synthetic local read while reducing traced peak allocation from about 16 MiB to 73 KiB.

## Decisions from the measurements

- Cache cleanup: changed; path containment, leases, and recovery checks remain the safety gates.
- Reader: changed; chapter content uses a file-backed response and sessions have a 16-entry cap plus a 30-minute idle TTL.
- Library: lookup now reads only the matching manifest before loading progress; a persistent index is not introduced yet because the 10,000-book scan is synthetic and there is no production corpus measurement.
- Zhihu indexing: retained the existing per-page transactional delta upsert/checkpoint path; no cumulative full-index rewrite was found in the audited flow.
- Podcast worker/state: retained existing per-task state files, database reuse, and terminal/non-terminal event policy; no production-scale rewrite count was available from isolated synthetic data, so no speculative state format migration was made.
- Search/storage/task sorting: p95 remained below 2 ms in the synthetic gate, so no extra index or sorting rewrite was justified.

The harness is `scripts/audit_benchmark.py`; every fixture is created under a temporary directory and removed by the process.
