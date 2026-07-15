# Performance decisions

Each cost center was handled independently and the measured result is recorded in `BASELINE.md`.

1. Cache cleanup removed protected-root content hashing; lease/recovery/path checks remain.
2. Zhihu indexing uses per-page transactional upsert plus checkpoint rather than rebuilding the cumulative task index.
3. Library lookup now defers progress-file reads until the matching manifest is found.
4. Podcast worker already reuses the supervisor/control path and throttles non-terminal UI progress; Podcast state remains per-task plus terminal manifest updates, so no unmeasured global-state migration was introduced.
5. Reader chapter responses stream from files; sessions are capped and expired.
6. Search and task sorting stayed below the synthetic budget, so no speculative index was added.

The benchmark harness intentionally labels its data synthetic and does not turn synthetic 10,000-book timings into a claim about a user's real Library.
