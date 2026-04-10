---
date: 2026-04-10
topic: alphabetical-ordering-and-parallel-execution
---

# Alphabetical Repo Ordering & Parallel Execution

## Problem Frame

Multi-repo commands (`sync`, `exec`, `status`, `refresh`, `doctor`) iterate repos in insertion order — the order `git rig add` was called. This makes output unpredictable across users and hard to scan in large workspaces. Worse, `sync` runs sequentially against network remotes, which is painfully slow when a workspace has many repos.

## Requirements

### Alphabetical Ordering

- R1. All multi-repo commands (`sync`, `exec`, `status`, `refresh`, `doctor`) process repos in case-insensitive alphabetical order by repo name, regardless of insertion order in `.rig.json`.
- R2. The manifest file (`.rig.json`) continues to store repos in insertion order — sorting is applied at iteration time, not at write time. This preserves `git diff` stability and avoids churn.
- R3. Drift detection (`check_drift`) also iterates in alphabetical order so drift warnings appear in sorted order before command output.

### Parallel Execution

- R4. `sync`, `exec`, and `refresh` execute repo operations in parallel by default. `status` and `doctor` remain sequential (fast enough, output-heavy).
- R5. Default parallelism is auto: worker count = number of repos, capped at a sensible maximum (e.g., 8). No user action needed for parallel to kick in.
- R6. `--jobs N` / `-j N` flag controls worker count. `-j1` forces sequential execution. Available on `sync`, `exec`, and `refresh`.
- R7. Optional `jobs` field in `.rig.json` sets a persistent default for the workspace. CLI `--jobs` overrides it. Follows the existing optional-config-with-fallback pattern (`Option<usize>`, `#[serde(default, skip_serializing_if)]`, `effective_jobs()` method).
- R8. Output behavior during parallel execution: live-updating multi-line display where each repo gets a persistent line showing current status (`⠋ fetching...` → `⠋ rebasing...` → `✓ synced`). Lines are ordered alphabetically. Uses the `indicatif` crate's `MultiProgress` API. When output is piped or non-TTY, falls back to simple line-by-line output (one line per repo on completion).
- R9. Errors in one repo do not block other repos. Each repo's success/failure is independent. The command exits non-zero if any repo failed.
- R10. When `-j1` (sequential), output behaves exactly as it does today — no live display, no multi-progress. The live-updating display only activates when jobs > 1.

## Success Criteria

- Running `git rig sync` on a 6-repo workspace is noticeably faster than sequential (wall-clock improvement proportional to network latency).
- Output from all multi-repo commands is alphabetically ordered and identical across users with the same workspace.
- Existing tests continue to pass; new tests cover sorted ordering and parallel behavior.

## Scope Boundaries

- `status` and `doctor` remain sequential — they are fast and their output formatting is complex enough that parallelizing adds risk without meaningful speed gain.
- No changes to `create` or `destroy` workflows — these are one-shot operations.
- `add` and `remove` operate on a single repo, so parallelism doesn't apply.
- No parallel provisioning (`.riginclude`) — provisioning happens during `add`/`create`, not during the parallel-eligible commands.

## Key Decisions

- **Sort at iteration time, not storage time**: Avoids manifest churn and preserves insertion order as a stable property of `.rig.json`. Sorting is a display/execution concern.
- **Parallel by default**: Network-bound operations (fetch, rebase) dominate `sync`/`refresh` time. git-rig's isolated-source-per-repo architecture means concurrent git operations don't interfere. Sequential is the escape hatch (`-j1`), not the default.
- **Live-updating multi-line display**: Each repo gets a persistent, in-place-updating line (like Docker Compose / Homebrew). Uses `indicatif` crate with TTY fallback for piped output. Gives real-time visibility into every repo's state without scrollback noise.
- **Manifest `jobs` field**: Follows the established optional-config-with-fallback pattern. Workspaces with many repos can set a default without passing `--jobs` every time.

## Dependencies / Assumptions

- Rust async runtime or thread pool needed for parallel execution (e.g., `rayon`, `tokio`, or `std::thread::scope`). Choice deferred to planning.
- `indicatif` crate for live-updating multi-line progress display. Handles TTY detection and non-TTY fallback.
- Assumes git operations on separate source repos are safe to run concurrently (no shared lock files). This is true for git-rig's architecture where each repo has its own clone.

## Outstanding Questions

### Deferred to Planning

- [Affects R5][Technical] What should the auto-parallelism cap be? 8 is a starting point but may need tuning based on typical workspace sizes and network behavior.
- [Affects R4][Needs research] Should `std::thread::scope`, `rayon`, or `tokio` be used for parallel execution? Consider binary size impact, existing dependencies, and complexity. The operations shell out to git, so true async I/O isn't needed — thread-based parallelism is likely sufficient.
- [Affects R8][Needs research] Exact `indicatif` configuration: spinner style, line format, finish/error states. Review `indicatif` `MultiProgress` docs for best practices with process-spawning workloads.

## Next Steps

-> `/ce:plan` for structured implementation planning
