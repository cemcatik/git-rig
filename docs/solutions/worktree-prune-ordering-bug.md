---
title: "Worktree prune ordering bug: prune before rm is a silent no-op"
category: logic-errors
date: 2026-03-24
tags: [git, worktree, ordering-bug, recovery-logic, precondition]
component: src/commands.rs
functions: [remove, destroy, remove_worktree_with_recovery]
discovery: ce:review (correctness reviewer, 95% confidence)
commits: [edda116, cd6a647]
related: [docs/solutions/worktree-recovery-ladder.md]
---

# Worktree Prune Ordering Bug

## Problem

After `git rig remove` or `git rig destroy` on a workspace whose directory
had been moved, branches were silently left behind even though the command
reported success. The worktree directory was cleaned up correctly, but the
branch that had been checked out in that worktree survived.

## Root Cause

In the 3-step worktree recovery ladder, the last-resort fallback (rung 3)
called `git worktree prune` before `std::fs::remove_dir_all`.

`git worktree prune` is a garbage collector, not a deletion command. It scans
git's internal worktree registry and removes entries whose backing directory
**no longer exists on disk**. Calling it while the directory still exists makes
it a no-op — it finds no stale entries to clean.

The sequence was:

```
# WRONG (original)
git worktree prune        # directory still present → no-op, metadata survives
fs::remove_dir_all(path)  # directory gone, but git still thinks branch is checked out
git branch -D branch      # fails: "cannot delete branch used by worktree"
```

The branch deletion failure was caught and printed as a warning, so the overall
command still reported `ok`. The user saw success while the branch accumulated
as an orphaned ref.

## Fix

Swap the ordering. Remove the directory first, then prune:

```
# CORRECT (after fix)
fs::remove_dir_all(path)  # directory removed — satisfies prune's precondition
git worktree prune         # finds absent directory → removes stale metadata entry
git branch -D branch       # succeeds — no worktree claims this branch
```

The fix also extracted the duplicate recovery ladder into a shared helper:

```rust
fn remove_worktree_with_recovery(
    source_repo: &Path,
    worktree_path: &Path,
    force: bool,
) -> Result<()> {
    // Rung 1: normal remove
    if git::worktree_remove(source_repo, worktree_path, force).is_ok() {
        return Ok(());
    }

    // Rung 2: repair broken link, then retry
    if git::worktree_repair(source_repo, worktree_path).is_ok()
        && git::worktree_remove(source_repo, worktree_path, force).is_ok()
    {
        return Ok(());
    }

    // Rung 3: remove directory first, then prune stale metadata
    std::fs::remove_dir_all(worktree_path)?;
    let _ = git::worktree_prune(source_repo);
    Ok(())
}
```

## How It Was Found

The bug was discovered by `ce:review` (correctness reviewer) with 95%
confidence during a review of the unpushed commits. No existing test covered
the prune+rm fallback path (rung 3) — all moved-workspace tests only exercised
the repair path (rung 2) where `git worktree repair` succeeds.

## Why It Survived

1. **Silent no-op**: `git worktree prune` exits 0 whether or not it found
   anything to prune. The wrong ordering produced no error signal.
2. **Warning-only branch deletion**: The `git branch -D` failure was caught
   and printed as a `WARN`, not propagated as an error. The command still
   reported success.
3. **Test gap**: All tests exercised rung 2 (repair succeeds). Rung 3
   (repair fails) had zero test coverage.
4. **Duplication**: The same incorrect ordering existed independently in
   both `remove()` and `destroy()`, written in different code styles.

## Prevention

### The general pattern

This is a **precondition-dependent ordering bug**: operation A has a
precondition (directory must be absent), operation B creates that condition
(removes the directory), but A was called before B. The bug is silent because
A is designed to be tolerant — it does nothing when its precondition is unmet
rather than failing.

**Recognition heuristics:**

- Idempotent cleanup operations (`prune`, `vacuum`, `sweep`, `gc`) are the
  highest-risk site. They're designed to be safe to call at any time, which
  means they silently no-op when called out of order.
- Two-phase delete patterns (delete data, then delete metadata referencing
  the data) are the most common manifestation.
- Recovery paths are tested less frequently than happy paths, so ordering
  bugs in recovery code survive longer.

### For git worktree operations

**Rule**: In any recovery path that combines a filesystem deletion with a git
metadata cleanup, the filesystem deletion must always precede the git cleanup.
State-observers (prune) come after state-mutators (rm).

### Test strategy

The `remove_with_corrupted_worktree_metadata` E2E test exercises rung 3.
To fully validate the ordering, tests should also assert on git's metadata
state using `git worktree list --porcelain`, not just the filesystem outcome.
