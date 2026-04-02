---
title: "Code review hardening: dead fallbacks, weak assertions, pattern drift"
category: logic-errors
date: 2026-04-02
tags: [code-review, dead-code, test-assertions, api-design, pattern-consistency, subprocess-optimization]
modules: [src/drift.rs, src/commands.rs, src/git.rs, tests/cli_test.rs, tests/git_test.rs]
severity: medium
---

# Code Review Hardening Pass

## Problem

A 5-agent code review (correctness, testing, maintainability, performance, security) of the drift detection + sync/exec/completions features surfaced 12 issues: 4 P2 (important) and 8 P3 (nice-to-have). Zero security findings. The issues fell into five themes:

1. **Dead code masquerading as fallback** — `.unwrap_or_else()` with an expensive git subprocess call that could never execute
2. **Weak E2E test assertions** — tests verified warning output existed but not that commands were actually skipped
3. **Inconsistent patterns across similar commands** — `sync` and `exec` implemented identical `--repo` filtering with different structural patterns
4. **Redundant subprocess call** — `sync` called `git::current_branch()` after rebase when the drift check already guaranteed the answer
5. **Unnecessary `Option` wrapping** — `print_drift_warnings` took `Option<&[String]>` when empty slice is the natural "no filter" value

## Root Cause

Each issue stems from a distinct anti-pattern:

**Dead fallback (#1):** The `status` command's branch display used `report.branches.get(&repo.name).unwrap_or_else(|| git::current_branch(...))`. But if the branch isn't cached, the worktree is unreachable — and the `has_worktree_unavailable` check at line 725 already `continue`d past this code. The fallback was dead code hiding an invariant.

**Weak assertions (#2):** Tests like `drift_exec_skips_missing_worktree` asserted `contains("DRIFT")` — proving the warning fires — without asserting `contains("hello").not()` — proving the command was skipped. A broken skip that still printed the warning would pass.

**Pattern drift (#3):** `exec` pre-filtered repos into a `Vec<_>` via `.filter().collect()`, while `sync` used an inline `if` guard. Same operation, different structures — no guidance for future commands.

**Redundant subprocess (#4):** After rebase, `sync` called `git::current_branch()` to get the branch for `ahead_behind`. But drift detection already confirmed the branch matches `repo.branch` for non-drifted repos, and rebase doesn't change the checked-out branch.

**Option wrapping (#5):** `print_drift_warnings` took `Option<&[String]>` for the repo filter, requiring callers to wrap with `Some()` or pass `None`. Empty slice `&[]` is the natural Rust idiom for "no filter".

## Solution

### 1. Explicit invariant via `.expect()`

```rust
// Before: dead fallback hiding invariant
let branch = report.branches.get(&repo.name).cloned().unwrap_or_else(|| {
    git::current_branch(&worktree_path).unwrap_or_else(|_| "(unknown)".into())
});

// After: invariant is documented, violation panics
let branch = report
    .branches
    .get(&repo.name)
    .expect("branch should be cached if worktree is reachable")
    .clone();
```

### 2. Negative assertions + behavioral verification

```rust
// Before: only checks warning exists
.stdout(predicate::str::contains("DRIFT"))
.stdout(predicate::str::contains("worktree missing"))

// After: proves the command was actually skipped
.stdout(predicate::str::contains("DRIFT"))
.stdout(predicate::str::contains("worktree missing"))
.stdout(predicate::str::contains("worktree unavailable, skipped"))
.stdout(predicate::str::contains("hello").not())
```

For `sync_repo_filter`, push to both repos' remotes and verify the excluded repo's file does NOT exist:

```rust
assert!(ws_dir.join("repo-a").join("new-a.txt").exists());   // synced
assert!(!ws_dir.join("repo-b").join("new-b.txt").exists());  // excluded
```

Six existing tests strengthened. Two new E2E tests added (exec with unreachable worktree, sync --repo with detached repo).

### 3. Unified inline guard pattern

Both `sync` and `exec` now use identical structure:

```rust
for repo in &manifest.repos {
    if !filter_repos.is_empty() && !filter_repos.iter().any(|f| f == &repo.name) {
        continue;
    }
    // ... command-specific logic
}
```

Removed `exec`'s `.filter().collect::<Vec<_>>()` — no allocation, single mental model.

### 4. Use `repo.branch` directly

```rust
// Before: redundant subprocess
let current = git::current_branch(&worktree_path).unwrap_or_else(|_| repo.branch.clone());
let (_ahead, behind) = git::ahead_behind(&worktree_path, &current, effective, &repo.remote);

// After: drift check guarantees match
let (_ahead, behind) = git::ahead_behind(&worktree_path, &repo.branch, effective, &repo.remote);
```

### 5. Simplified API: `&[String]` replaces `Option<&[String]>`

```rust
// Before
pub fn print_drift_warnings(report: &DriftReport, repo_filter: Option<&[String]>, source_only: bool)

// After
pub fn print_drift_warnings(report: &DriftReport, repo_filter: &[String], source_only: bool)
```

Callers pass `&[]` for "no filter" and `filter_repos` directly for filtered commands. Four call sites simplified.

### 6-8. New test coverage

- **8 unit tests** for `DriftReport` query methods (`has_any_drift`, `has_worktree_unavailable`, `has_source_missing`) in `src/drift.rs`
- **3 integration tests** for `find_worktree_for_branch` (found, not-found, detached) in `tests/git_test.rs`
- **2 E2E tests**: exec with corrupt/unreachable worktree, sync --repo with detached repo

Test count: 251 → 264.

## Prevention Checklist

For future code reviews and feature work:

- [ ] **`.unwrap_or_else()` after a guard → use `.expect()`**. If an earlier `continue`/`return` makes the fallback unreachable, the fallback is dead code hiding an invariant. `.expect("reason")` documents the invariant and catches regressions.

- [ ] **Assert behavior, not just output strings.** A test that checks `contains("skipped")` passes even if the operation silently proceeded. Capture state before/after (file existence, HEAD SHA, dirty status) and compare.

- [ ] **One pattern per concept.** When two commands do the same thing (filter repos, collect errors, check drift), they should use the same structural pattern. If you're choosing between two, pick the simpler one (inline guard > filter+collect).

- [ ] **Cache implies no bypass.** When a function caches an expensive result (like `DriftReport.branches`), downstream code should use the cache exclusively. A direct call to the underlying function is a bug unless the cache is known stale.

- [ ] **Use zero-values over Option.** When a type has a natural "empty" value (`&[]`, `""`, `0`), prefer it over `Option<T>`. Reserve `Option` for when absent and empty are semantically different.

## Related

- [Manifest drift detection](manifest-drift-detection.md) — first code review pass; this doc covers findings from the second pass
- [git-rs review findings](git-rs-review-findings.md) — prior review of git.rs contracts (silent error swallowing, format stability)
- [Partial failure must return error](partial-failure-must-return-error.md) — error handling philosophy that drift detection follows as an exception
- [Worktree prune ordering bug](worktree-prune-ordering-bug.md) — prior example of test gap pattern (recovery paths under-tested)
