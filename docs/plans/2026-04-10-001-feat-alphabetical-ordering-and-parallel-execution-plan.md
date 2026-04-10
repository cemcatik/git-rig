---
title: "feat: Alphabetical repo ordering and parallel execution"
type: feat
status: active
date: 2026-04-10
origin: docs/brainstorms/2026-04-10-alphabetical-ordering-and-parallel-execution-requirements.md
---

# feat: Alphabetical Repo Ordering & Parallel Execution

## Overview

Two tightly related improvements to multi-repo commands: (1) sort repos alphabetically by name at iteration time, so output is consistent and predictable regardless of insertion order, and (2) run `sync`, `exec`, and `refresh` operations in parallel by default with a live-updating multi-line progress display via `indicatif`.

## Problem Statement / Motivation

Repos are stored in `Vec<RepoEntry>` and iterated in insertion order — the order `git rig add` was called. Two users with the same workspace see different output ordering, making troubleshooting harder. Worse, `sync` runs sequentially against network remotes, which is painfully slow for workspaces with many repos (see origin: `docs/brainstorms/2026-04-10-alphabetical-ordering-and-parallel-execution-requirements.md`).

## Proposed Solution

**Phase 1:** Add `repos_sorted()` helper to `Manifest`, update all 11 user-facing iteration sites to use it. Manifest storage remains insertion-order (see origin: key decision "Sort at iteration time, not storage time").

**Phase 2:** Add `jobs: Option<usize>` to `Manifest` following the established optional-config-with-fallback pattern (see `docs/solutions/upstream-config-with-fallback.md`). Add `--jobs` / `-j` flag to `Sync`, `Exec`, and `Refresh` CLI definitions.

**Phase 3:** Implement parallel execution using `std::thread::scope` (stdlib) with `indicatif` `MultiProgress` for live-updating display. One spinner per repo, alphabetically ordered, with TTY detection and non-TTY fallback.

## Technical Considerations

### Architecture Impacts

- **New dependency:** `indicatif` (adds `console` transitive dep, ~50-150KB binary impact with LTO). Overlaps partially with `colored` but coexists fine.
- **No async runtime needed:** Operations shell out to `git`, so `std::thread::scope` with a `Mutex<Iterator>` work queue is sufficient. No `rayon` or `tokio`.
- **Output capture for parallel paths:** The existing 122 `println!` calls in `commands.rs` cannot be used during parallel execution — per-repo output must flow through `indicatif` progress bars or be buffered. Sequential paths (`-j1`) retain current `println!` behavior unchanged.

### Shared Source Repos — Critical Safety Concern

Two `RepoEntry` items can share the same `source` path (e.g., `git rig add /path/to/repo --name repo-a` then `git rig add /path/to/repo --name repo-b`). Concurrent `git fetch` on the same git repo races on lock files and will fail with `Unable to create '.git/objects/pack/xxx.lock'`.

**Solution:** Before parallel execution, group repos by `source` path. Deduplicate fetches — run one `git fetch` per unique source, then proceed with per-worktree operations (rebase, stash, etc.) in parallel. This is safe because rebase operates on the worktree directory, not the source repo.

### `refresh` Mutable Access

`refresh` currently iterates `&mut manifest.repos` to update `default_branch` in place. Parallel mutable refs across threads violate Rust's borrowing rules.

**Solution:** Parallel phase collects `Vec<(String, String)>` tuples of `(repo_name, new_default_branch)`. Sequential merge phase applies updates to the manifest and saves once. This matches the existing pattern but splits the loop into parallel-fetch + sequential-merge.

### `exec` Output Capture

`exec` currently inherits stdout/stderr from child processes (`.status()`). For parallel execution, child output must be captured (`.output()`) and buffered per-repo.

**Solution:** In parallel mode, capture stdout/stderr per-repo. The `indicatif` spinner shows which repos are running. After all repos complete, print each repo's full captured output in alphabetical order (header + stdout + stderr + status). For `-j1`, output behaves exactly as today (inherited, real-time).

### `--fail-fast` with Parallel `exec`

With `-j4` on 4 repos, all repos launch simultaneously. If the first to fail triggers fail-fast, "stop early" semantics differ from sequential.

**Solution:** Let in-flight operations complete, but don't start new ones from the work queue. When all repos fit within the job cap (all launched at once), fail-fast has no additional effect. This is safe and unsurprising — document the behavior.

### Ctrl-C / SIGINT During Parallel Execution

Accept default behavior (process termination kills all threads) for the initial implementation. Document edge cases:
- `sync --stash`: changes may remain stashed on Ctrl-C
- `sync`: repos may be left in rebase-in-progress state (recoverable via `git rebase --abort`)

A graceful shutdown handler can be added later if users report pain.

### Performance Implications

- Network-bound operations (fetch, rebase) see near-linear speedup with parallelism
- Auto-cap at 8 workers prevents overwhelming the network or git hosting service
- `jobs` count uses post-filter, post-drift-skip repo count (efficient)
- Single-repo workspaces use the sequential code path (no `MultiProgress` overhead)

### Cross-Platform

- `indicatif` works on Windows through the `console` crate (handles `ENABLE_VIRTUAL_TERMINAL_PROCESSING`)
- `std::thread::scope` is cross-platform (stdlib)
- No `#[cfg(unix)]` needed for this feature
- Braille spinner characters render correctly in Windows Terminal and modern PowerShell

## System-Wide Impact

- **Interaction graph:** Parallel execution only changes the *scheduling* of existing git subprocess calls, not what they do. No new git operations are introduced.
- **Error propagation:** Each repo's error is collected independently. The existing collect-and-fail pattern is preserved — errors flow up identically, just collected from thread results instead of a sequential loop.
- **State lifecycle risks:** Shared source repo concurrent fetch is the only risk (see mitigation above). Worktree operations (rebase, stash) are fully isolated per directory.
- **API surface parity:** The `--jobs` flag is added to `sync`, `exec`, and `refresh`. No other interfaces are affected.

## Acceptance Criteria

### Phase 1: Alphabetical Ordering

- [ ] `Manifest::repos_sorted()` returns `Vec<&RepoEntry>` sorted case-insensitively by name (`src/workspace.rs`)
- [ ] `Manifest::repos_sorted_mut()` returns `Vec<&mut RepoEntry>` sorted case-insensitively by name (`src/workspace.rs`)
- [ ] All 11 user-facing iteration sites use sorted helpers:
  - `status` (line 720), `sync` (line 834), `exec` (line 1240), `refresh` (line 776), `doctor` (line 1071) in `src/commands.rs`
  - `list` (line 685), `destroy` dry-run (line 586), `destroy` actual (line 620), `create --from` validation (line 81), `create --from` cloning (line 136) in `src/commands.rs`
  - `check_drift` (line 74) in `src/drift.rs`
- [ ] Migration loop (`src/workspace.rs` line 63) remains unsorted — internal, not user-visible
- [ ] `.rig.json` continues to store repos in insertion order (R2)
- [ ] Tests: workspace with repos added in non-alphabetical order verifies sorted output

### Phase 2: Manifest `jobs` Config + CLI Flag

- [ ] `jobs: Option<usize>` field on `Manifest` with `#[serde(default, skip_serializing_if = "Option::is_none")]` (`src/workspace.rs`)
- [ ] `Manifest::effective_jobs()` method: returns `manifest.jobs` if set, else `0` (meaning "auto") (`src/workspace.rs`)
- [ ] `--jobs` / `-j` flag as `Option<usize>` on `Sync`, `Exec`, `Refresh` variants (`src/main.rs`)
- [ ] Resolution logic: CLI `--jobs` > manifest `jobs` > auto (min(repo_count, 8)). `jobs == 0` means auto. `jobs == 1` means sequential.
- [ ] Serde roundtrip test: manifest with/without `jobs` field
- [ ] CLI parsing test: `--jobs 4`, `-j1`, no flag

### Phase 3: Parallel Execution

- [ ] Add `indicatif` dependency to `Cargo.toml` (default-features = false, features = ["unicode-width"])
- [ ] New module `src/parallel.rs` containing the parallel execution engine
- [ ] `run_parallel()` function: takes sorted repos, job count, per-repo closure, returns collected results
- [ ] Work queue via `Mutex<Iterator>` distributes repos to `std::thread::scope` workers
- [ ] Source-path deduplication: one `git fetch` per unique `repo.source`, then per-worktree operations in parallel
- [ ] `indicatif` `MultiProgress` with one spinner per repo, alphabetically ordered
- [ ] Spinner style: Braille dots, success: green checkmark, failure: red cross
- [ ] Template: `{spinner:.cyan} {prefix:<20!.bold} {wide_msg}`
- [ ] TTY detection via `MultiProgress::is_hidden()` — non-TTY falls back to line-by-line `eprintln!` per repo
- [ ] When effective jobs == 1, use existing sequential code path (no `MultiProgress`, no output capture)
- [ ] `sync` parallel: fetch (deduplicated by source) then stash/rebase/unstash per worktree, collect results
- [ ] `exec` parallel: capture `.output()` per repo, buffer stdout/stderr, print in alphabetical order after all complete
- [ ] `refresh` parallel: fetch + detect default branch, collect `(name, new_branch)` tuples, sequential merge into manifest
- [ ] `--fail-fast` with parallel `exec`: in-flight operations complete, new launches skipped
- [ ] Zero repos after filtering: print "No repos to process" and exit 0
- [ ] `cross-check` passes (all CI targets compile)
- [ ] E2E tests for parallel execution (can test with `-j2` on 3-repo workspaces)

### Quality Gates

- [ ] `just check` passes (fmt + clippy + deny + test)
- [ ] `just cross-check` passes (all release targets compile — especially Windows)
- [ ] No `#[cfg(unix)]` introduced
- [ ] No `unsafe` introduced (beyond existing SIGPIPE handler)
- [ ] Existing tests pass unchanged (alphabetical ordering may require test adjustments if any test relied on insertion order)

## Implementation Phases

### Phase 1: Alphabetical Ordering

**Files:** `src/workspace.rs`, `src/commands.rs`, `src/drift.rs`, `tests/cli_test.rs`

1. Add `repos_sorted()` and `repos_sorted_mut()` to `Manifest` in `src/workspace.rs`:

   ```rust
   impl Manifest {
       /// Repos sorted case-insensitively by name (for display and execution).
       pub fn repos_sorted(&self) -> Vec<&RepoEntry> {
           let mut sorted: Vec<&RepoEntry> = self.repos.iter().collect();
           sorted.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
           sorted
       }

       /// Mutable variant for commands that update repo fields (e.g., refresh).
       pub fn repos_sorted_mut(&mut self) -> Vec<&mut RepoEntry> {
           let mut sorted: Vec<&mut RepoEntry> = self.repos.iter_mut().collect();
           sorted.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
           sorted
       }
   }
   ```

2. Update all iteration sites in `src/commands.rs`:
   - Replace `for repo in &manifest.repos` with `for repo in manifest.repos_sorted()` at: status (720), sync (834), exec (1240), doctor (1071), list (685), destroy dry-run (586), destroy actual (620)
   - Replace `for entry in &source_manifest.repos` with `for entry in source_manifest.repos_sorted()` at create-from validation (81)
   - Replace `for entry in &valid_entries` with sorted iteration at create-from cloning (136) — sort `valid_entries` before iterating
   - Replace `for repo in &mut manifest.repos` with `for repo in manifest.repos_sorted_mut()` at refresh (776)

3. Update `src/drift.rs`:
   - Replace `for repo in &manifest.repos` with `for repo in manifest.repos_sorted()` at check_drift (74)

4. Add unit tests in `src/workspace.rs`:
   - `repos_sorted_returns_alphabetical_order`: create manifest with repos `["zebra", "alpha", "Middle"]`, verify sorted order is `["alpha", "Middle", "zebra"]` (case-insensitive)
   - `repos_sorted_mut_allows_mutation`: verify mutable refs can be written through

5. Update E2E tests in `tests/cli_test.rs`:
   - Add test with repos named `["repo-c", "repo-a", "repo-b"]` (non-alphabetical), verify `status` output shows `repo-a` before `repo-b` before `repo-c`
   - Verify `sync`, `exec`, `list` output ordering similarly

**Estimated complexity:** Low. Mostly mechanical find-and-replace with a small helper method.

### Phase 2: Manifest `jobs` Config + CLI Flag

**Files:** `src/workspace.rs`, `src/main.rs`, `src/commands.rs`, `tests/cli_test.rs`

1. Add `jobs` field to `Manifest` in `src/workspace.rs`:

   ```rust
   #[derive(Debug, Serialize, Deserialize)]
   pub struct Manifest {
       pub name: String,
       #[serde(default, skip_serializing)]
       base_dir: Option<PathBuf>,
       pub repos: Vec<RepoEntry>,
       /// Default parallelism for multi-repo commands. None = auto.
       #[serde(default, skip_serializing_if = "Option::is_none")]
       pub jobs: Option<usize>,
   }
   ```

2. Add `effective_jobs()` method:

   ```rust
   impl Manifest {
       /// Resolve the manifest-level job count. Returns 0 for "auto".
       pub fn effective_jobs(&self) -> usize {
           self.jobs.unwrap_or(0)
       }
   }
   ```

3. Add `--jobs` / `-j` to CLI variants in `src/main.rs`:

   ```rust
   Sync {
       // ... existing fields ...
       /// Number of parallel jobs (default: auto, -j1 for sequential)
       #[arg(short, long)]
       jobs: Option<usize>,
   },
   // Same for Exec and Refresh
   ```

4. Add resolution function in `src/commands.rs`:

   ```rust
   /// Resolve effective job count: CLI flag > manifest > auto.
   fn resolve_jobs(cli_jobs: Option<usize>, manifest: &Manifest, repo_count: usize) -> usize {
       const AUTO_CAP: usize = 8;
       let base = cli_jobs.unwrap_or_else(|| manifest.effective_jobs());
       if base == 0 {
           repo_count.min(AUTO_CAP).max(1)
       } else {
           base
       }
   }
   ```

5. Thread the `jobs` parameter through command signatures: `sync()`, `exec()`, `refresh()`.

6. Tests:
   - Serde: manifest with `"jobs": 4` deserializes correctly, manifest without `jobs` gets `None`
   - `effective_jobs()`: returns value when set, 0 when None
   - `resolve_jobs()`: CLI overrides manifest, auto-cap at 8, minimum 1
   - CLI: `git rig sync --jobs 4`, `git rig sync -j1` parse correctly

**Estimated complexity:** Low. Follows the documented optional-config-with-fallback pattern exactly.

### Phase 3: Parallel Execution Engine

**Files:** New `src/parallel.rs`, `src/commands.rs`, `Cargo.toml`, `tests/cli_test.rs`

#### Step 3a: Add `indicatif` dependency

```toml
# Cargo.toml
indicatif = { version = "0.18", default-features = false, features = ["unicode-width"] }
```

Register the new module in `src/main.rs`:
```rust
mod parallel;
```

#### Step 3b: Create `src/parallel.rs` — the execution engine

This module provides the shared parallel execution infrastructure. Key types:

```rust
use std::sync::Mutex;
use std::thread;
use std::time::Duration;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};

/// Progress handle passed to per-repo closures.
pub struct RepoProgress<'a> {
    bar: &'a ProgressBar,
    is_tty: bool,
}

impl RepoProgress<'_> {
    pub fn set_status(&self, msg: &str) {
        if self.is_tty {
            self.bar.set_message(msg.to_string());
        }
    }
}
```

The core function pattern:

```rust
/// Run `op` for each repo in parallel with up to `jobs` workers.
/// repos must already be sorted alphabetically by the caller.
/// Returns results in input order.
pub fn run_parallel<T, F>(
    repo_names: &[String],
    jobs: usize,
    op: F,
) -> Vec<(String, Result<T, String>)>
where
    T: Send,
    F: Fn(usize, &RepoProgress) -> Result<T, String> + Sync,
{
    let mp = MultiProgress::new();
    let is_tty = !mp.is_hidden();

    // Create spinner bars in sorted order (all visible before work starts)
    let spinner_style = ProgressStyle::with_template(
        " {spinner:.cyan} {prefix:<20!.bold} {wide_msg}"
    ).unwrap().tick_strings(&[
        "⠋","⠙","⠹","⠸","⠼","⠴","⠦","⠧","⠇","⠏","·"
    ]);

    let bars: Vec<ProgressBar> = repo_names.iter().map(|name| {
        let pb = mp.add(ProgressBar::new_spinner());
        pb.set_style(spinner_style.clone());
        pb.set_prefix(name.clone());
        pb.set_message("queued");
        pb.enable_steady_tick(Duration::from_millis(80));
        pb
    }).collect();

    // Pre-allocate result slots
    let results: Vec<Mutex<Option<(String, Result<T, String>)>>> =
        repo_names.iter().map(|_| Mutex::new(None)).collect();

    // Work queue: each worker grabs the next index
    let next_index = std::sync::atomic::AtomicUsize::new(0);

    thread::scope(|s| {
        for _ in 0..jobs {
            s.spawn(|| {
                loop {
                    let idx = next_index
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    if idx >= repo_names.len() { break; }

                    let progress = RepoProgress {
                        bar: &bars[idx],
                        is_tty,
                    };

                    let result = op(idx, &progress);

                    // Update spinner to final state
                    match &result {
                        Ok(_) => { /* set success style + finish */ }
                        Err(_) => { /* set failure style + abandon */ }
                    }

                    *results[idx].lock().unwrap() =
                        Some((repo_names[idx].clone(), result));
                }
            });
        }
    });

    // Collect in order
    results.into_iter()
        .map(|slot| slot.into_inner().unwrap().unwrap())
        .collect()
}
```

Design notes:
- **`AtomicUsize` work index** rather than `Mutex<Iterator>` — simpler, lock-free, each worker atomically grabs the next item.
- **Pre-allocated result slots** — each worker writes to its own index, no contention.
- **TTY branch** — non-TTY path uses `eprintln!` per-repo completion line instead of indicatif.
- **Spinner lifecycle** — all spinners created upfront showing "queued", workers update status as they progress, finish with style swap for success/failure.

#### Step 3c: Source-path deduplication for `sync` and `refresh`

Before entering the per-worktree parallel phase, deduplicate `git fetch` by source path:

```rust
fn deduplicated_fetch(
    repos: &[&RepoEntry],
    jobs: usize,
    mp: &MultiProgress, // or separate progress display
) -> HashMap<PathBuf, Result<(), String>> {
    // Collect unique (source, remote) pairs
    // Fetch each unique source (can also be parallel)
    // Return map: source_path -> fetch result
}
```

This runs before the per-worktree parallel phase. Repos whose source fetch failed are marked as errors without attempting rebase.

#### Step 3d: Integrate into `sync`

Modify `commands::sync()`:

1. Resolve effective jobs via `resolve_jobs()`
2. If `jobs == 1`: use existing sequential code path (unchanged)
3. If `jobs > 1`:
   a. Filter repos (--repo flag + drift skip)
   b. Deduplicate fetch by source path (parallel)
   c. Per-worktree operations in parallel (stash, rebase, unstash)
   d. Collect results, print error summary

The per-repo closure communicates status through `RepoProgress::set_status()` calls at each stage.

#### Step 3e: Integrate into `exec`

Modify `commands::exec()`:

1. Resolve effective jobs
2. If `jobs == 1`: existing code path (`.status()`, inherited stdout)
3. If `jobs > 1`:
   a. Filter repos
   b. Run child processes with `.output()` (capture stdout/stderr) in parallel
   c. After all complete, print each repo's output block in alphabetical order:
      ```
      >>> repo-a
      [captured stdout]
      
      >>> repo-b
      [captured stdout]
      ```
   d. `--fail-fast`: set an `AtomicBool` flag — workers check before grabbing new work

#### Step 3f: Integrate into `refresh`

Modify `commands::refresh()`:

1. Resolve effective jobs
2. If `jobs == 1`: existing code path (`&mut manifest.repos`)
3. If `jobs > 1`:
   a. Deduplicate fetch by source path (parallel)
   b. Detect default branch per repo (parallel, read-only)
   c. Collect `Vec<(String, String)>` of `(repo_name, new_default_branch)`
   d. Sequential merge: apply updates to `manifest.repos`, save if any changed

#### Step 3g: Tests

- **Unit tests** for `resolve_jobs()`: auto-cap, CLI override, manifest default
- **Unit tests** for source deduplication logic
- **E2E tests**:
  - `sync -j2` on a 3-repo workspace: verify all repos synced
  - `exec -j2 -- git status` on a 3-repo workspace: verify output contains all repos
  - `exec -j1 -- echo hello`: verify sequential behavior unchanged
  - `refresh -j2`: verify default branches detected
  - `sync -j1`: verify identical to current behavior
  - Non-TTY: pipe output, verify no ANSI escape codes from `indicatif`

**Estimated complexity:** High. This is the core of the feature — new module, new dependency, output capture, thread coordination.

## Success Metrics

- `git rig sync` on a 6-repo workspace completes in roughly single-repo time (vs. 6x sequential)
- Output from all multi-repo commands is alphabetically ordered regardless of insertion order
- No regressions in `just check` or `just cross-check`

## Dependencies & Risks

- **`indicatif` 0.18** — stable, widely used, actively maintained. Low risk.
- **`console` crate** (transitive via `indicatif`) — partially overlaps with `colored`. Coexistence is harmless; consolidation can happen later.
- **Windows build** — `indicatif` and `std::thread::scope` are fully cross-platform. `just cross-check` validates compilation.
- **Shared source repos** — mitigated by fetch deduplication. If not deduplicated, concurrent fetches produce confusing lock-file errors.

## Sources & References

### Origin

- **Origin document:** [docs/brainstorms/2026-04-10-alphabetical-ordering-and-parallel-execution-requirements.md](docs/brainstorms/2026-04-10-alphabetical-ordering-and-parallel-execution-requirements.md) — Key decisions carried forward: sort at iteration time (not storage), parallel by default, live-updating display, manifest `jobs` field.

### Internal References

- Optional-config-with-fallback pattern: `docs/solutions/upstream-config-with-fallback.md`
- Partial-failure error handling: `docs/solutions/partial-failure-must-return-error.md`
- Cross-platform symlink fallback: `docs/solutions/cross-platform-symlink-fallback.md`
- Code review hardening (unified iteration pattern): `docs/solutions/code-review-hardening-pass.md`
- Worktree prune ordering bug: `docs/solutions/worktree-prune-ordering-bug.md`
- Manifest types: `src/workspace.rs:9-38`
- Sync implementation: `src/commands.rs:823-982`
- Exec implementation: `src/commands.rs:1229-1291`
- Refresh implementation: `src/commands.rs:766-816`
- Drift detection: `src/drift.rs:70-148`
- CLI definitions: `src/main.rs:12-200`

### External References

- indicatif crate docs: https://docs.rs/indicatif/latest/indicatif/
- indicatif MultiProgress: https://docs.rs/indicatif/latest/indicatif/struct.MultiProgress.html
- std::thread::scope: https://doc.rust-lang.org/std/thread/fn.scope.html
