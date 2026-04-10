---
title: "Parallel git fetch deduplication with Condvar-based cache"
date: 2026-04-10
tags: [parallel-execution, git-fetch, condvar, deduplication, concurrency]
module: src/parallel.rs
---

# Parallel Fetch Deduplication with Condvar

## Problem

When parallelizing `git fetch` + `git rebase` across multiple workspace repos, two issues arise:

1. **Lock contention:** Git uses internal lock files (`.git/objects/pack/xxx.lock`). Concurrent fetches on the same source clone produce `Unable to create .lock: File exists` errors.
2. **Remote dimension:** Two repos may share the same source clone but reference different remotes (`origin` vs `upstream`). A naive dedup that keys only on source path silently skips the second remote's fetch.

## Root Cause

Git's internal locking is per-repository, not per-remote. The dedup key must include both the source path and the remote name. Keying on path alone is a silent correctness bug: `sync` would rebase onto a stale upstream ref without any error.

## Solution

`FetchCache` in `src/parallel.rs` — a `Mutex<HashMap>` + `Condvar` that ensures each unique `(source_path, remote)` pair is fetched exactly once:

```rust
pub struct FetchCache {
    state: Mutex<HashMap<(PathBuf, String), FetchState>>,
    done: Condvar,
}

enum FetchState {
    InProgress,
    Done(Result<(), String>),
}
```

The `fetch_once` method:
1. Lock cache, check if `(source, remote)` key exists
2. If `Done` — return cached result (zero subprocess cost)
3. If `InProgress` — `condvar.wait()` releases mutex and blocks until signaled
4. If absent — insert `InProgress`, drop lock, fetch, reacquire lock, insert `Done`, `notify_all()`

Call site (inside parallel worker):
```rust
let fetch_cache = FetchCache::new();
// ...
fetch_cache.fetch_once(&repo.source, &repo.remote, || {
    git::fetch(&repo.source, &repo.remote).map_err(|e| e.to_string())
});
```

## Investigation Journey

### Why Condvar over spin-wait

The initial implementation used `thread::sleep(Duration::from_millis(50))` in a polling loop. For a `git fetch` taking 1-10 seconds, each blocked thread performs 20-200 wasted lock/unlock cycles. Condvar eliminates all polling: waiters block in the kernel and wake within microseconds of `notify_all()`.

**Recognition heuristic:** `thread::sleep` inside a `loop` with a `Mutex` check is almost always a spin-wait. The correct families of solution are signal-based (`Condvar`, channel) or lazy-init (`OnceCell`, `OnceLock`).

### Why `(PathBuf, String)` key over `PathBuf`

If the key is just the source path, then fetching `origin` caches a `Done` result, and a subsequent request for `upstream` on the same source finds `Done` and skips its fetch. The upstream remote is never fetched — a silent bug with no error. The composite key treats each (source, remote) pair independently.

**Recognition heuristic:** Compare the cache key tuple to the function signature being cached. If the key has fewer dimensions than the function's parameters, a dimension is missing.

## Prevention

### Code review checklist for dedup/memoization caches

1. **Key completeness:** Does the key include every parameter that affects the operation? Compare key dimensions to the function signature.
2. **Synchronization:** Is waiting signal-based (`Condvar`, channel) or poll-based (`sleep` in a loop)? Reject poll-based.
3. **Error caching:** Are errors cached and returned to subsequent callers? (Usually correct for network operations — prevents thundering-herd retries.)
4. **Notify strategy:** `notify_all` when multiple threads may wait on the same or different keys.

### Minimum test matrix for dedup caches

| Test | What it verifies |
|---|---|
| Same key, sequential | Second call returns cached result, closure runs once |
| Same key, concurrent | N threads, closure runs once, all get the result |
| Different key per dimension | Each dimension varied independently, all closures run |
| Error caching | Failed result cached and returned to subsequent callers |

All four are implemented in `src/parallel.rs` tests: `fetch_cache_deduplicates_same_key`, `fetch_cache_concurrent_same_key`, `fetch_cache_different_remotes_are_independent`, `fetch_cache_propagates_errors`.

## Related

- `docs/solutions/partial-failure-must-return-error.md` — The parallel executor preserves the collect-and-fail error pattern for multi-repo commands
- `docs/solutions/code-review-hardening-pass.md` — "One pattern per concept" principle: `FetchCache` is shared between `sync_parallel` and `refresh_parallel`
- `docs/solutions/manifest-drift-detection.md` — Drift detection runs sequentially before the parallel phase; the drift report is consumed read-only by parallel workers
- Go's `singleflight` package — Same pattern (request coalescing), different language
