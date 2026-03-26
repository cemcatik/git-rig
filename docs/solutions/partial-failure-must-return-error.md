---
title: "Partial failure in multi-repo command silently returns Ok(())"
category: logic-errors
date: 2026-03-26
tags:
  - error-handling
  - partial-failure
  - exit-code
  - consistency
  - multi-repo
  - create-from
components:
  - src/commands.rs
severity: medium
resolution_time_estimate: 15min
related:
  - docs/solutions/git-rs-review-findings.md
  - docs/solutions/worktree-prune-ordering-bug.md
---

# Partial failure in multi-repo command silently returns Ok(())

## Problem

The `create --from` command's `create_from_source` function returned `Ok(())` even when some repositories failed to clone, causing the CLI to exit with code 0 despite incomplete workspace creation. Scripts and CI pipelines had no way to detect the partial failure.

## Root Cause

The function followed the codebase's continue-and-collect pattern — iterating over repos, printing per-repo warnings on failure, and collecting errors into a vec — but omitted the final guard that converts collected errors into a returned `Err`. Every other multi-repo command (`sync`, `exec`, `destroy`) checks `!errors.is_empty()` at the end and returns `Err(anyhow!(...))`. The new function printed a summary but fell through to `Ok(())`.

This is the same class of bug as the silent `(0, 0)` return in `ahead_behind()` (documented in `git-rs-review-findings.md`) and the warning-only error handling in the worktree prune ordering bug — functions that return a "success" signal when the underlying operation actually failed.

## Solution

One line added in the `!errors.is_empty()` branch, after printing the summary:

```rust
    // In create_from_source, after the per-repo error summary:
    if errors.is_empty() {
        println!("{} Created rig ...", "ok".green());
    } else {
        // ... print WARN summary ...
        for (repo_name, err) in &errors {
            println!("  {} {}: {}", "ERR".red(), repo_name, err);
        }
        return Err(anyhow!("{} repo(s) failed to clone", errors.len()));
    }
```

This matches the exact pattern used by:
- `sync` (commands.rs): `return Err(anyhow!("{} repo(s) had issues", errors.len()));`
- `exec` (commands.rs): `return Err(anyhow!("{} repo(s) had errors", errors.len()));`
- `destroy` (commands.rs): `return Err(anyhow!("{failed} worktree(s) could not be removed"));`

Three test gaps were also filled:
- `--skip` when ALL source repos are invalid (the "no valid repos" error path)
- Source path exists but is not a git repo (second pre-validation branch)
- Partial runtime failure during worktree creation (continue-and-report path)

## Investigation

1. Code review agents compared the error-handling pattern across all multi-repo commands and found `create_from_source` was the only one returning `Ok(())` on partial failure.
2. Traced control flow: the `!errors.is_empty()` branch printed warnings but had no `return Err(...)`.
3. Audited test coverage: found three untested branches in the new code.

## The Pattern: Collect-and-Fail for Multi-Repo Commands

When a function iterates over multiple items, uses `continue` to skip failures, and collects errors into a vec, it **must** check `!errors.is_empty()` at the end and return `Err`.

The print-and-continue pattern handles graceful degradation (do as much work as possible). The final error return handles correctness (report that the operation was not fully successful). Omitting either half breaks the contract.

## Prevention

### Checklist for new multi-repo commands

- [ ] Does the command collect per-repo errors into a `Vec`?
- [ ] After the loop, does it check `!errors.is_empty()` and return `Err`?
- [ ] Is there a test where one repo succeeds and one fails, asserting non-zero exit?

### Test pattern

Every multi-repo command should have a partial-failure test:

```rust
#[test]
fn cmd_returns_error_on_partial_failure() {
    // Set up workspace with two repos
    // Sabotage one repo (delete dir, break remote, create branch conflict)
    // Run the command
    // Assert: exit code != 0
    // Assert: stdout contains the failure summary
}
```

### Future consideration

Extract a shared `run_for_each_repo` helper that handles the iteration, error collection, summary printing, and error return. Individual commands supply only the per-repo closure. This eliminates the bug class entirely because no new command author writes the error-aggregation code.

## Cross-References

- `docs/solutions/git-rs-review-findings.md` — prior art for silent-success bugs in git-rig (findings #4 and #5)
- `docs/solutions/worktree-prune-ordering-bug.md` — another instance where warning-only error handling masked a real failure ("Why It Survived" section)
