---
title: Worktree Recovery Ladder
category: git-worktree
tags: [worktree, recovery, error-handling, git]
date: 2026-03-24
trigger: git worktree remove fails on a previously valid worktree
---

# Worktree Recovery Ladder

## Problem

`git worktree remove` can fail when the worktree directory has been moved,
renamed, or its internal `.git` link file has become stale. This happens when:

1. A user moves the worktree directory outside of git's knowledge
2. The filesystem path changed (e.g., parent directory renamed)
3. The source repo was moved after the worktree was created
4. The worktree's `.git` file points to a non-existent gitdir

When this happens, git doesn't recognize the directory as a worktree and
`git worktree remove` fails, even though the directory is clearly a worktree
from git-rig's perspective (it's tracked in `.rig.json`).

## Solution: Three-Rung Recovery Ladder

The recovery follows a strict escalation sequence. Each rung is tried only
if the previous one fails.

### Rung 1: Normal Remove

```
git worktree remove [--force] <path>
```

The happy path. Works when git's internal worktree tracking is intact.
`--force` allows removal of dirty worktrees (used by `destroy`, not `remove`).

### Rung 2: Repair + Remove

```
git worktree repair <path>
git worktree remove [--force] <path>
```

`git worktree repair` rebuilds the bidirectional link between the source repo's
`.git/worktrees/<name>/gitdir` and the worktree's `.git` file. This fixes the
common case where the worktree directory was moved but still exists.

### Rung 3: Prune + Filesystem Remove

```
git worktree prune
rm -rf <path>
```

Nuclear option. When repair fails (the worktree state is too corrupted for git
to understand), we prune all stale worktree entries from git's tracking and
then remove the directory directly. The `prune` call is fire-and-forget
(`let _ = ...`) because it cleans up git's internal state on a best-effort
basis.

## Why This Order Matters

- **Rung 1 first** because it's the only path that properly cleans up git's
  internal worktree tracking in one atomic operation.
- **Rung 2 before 3** because repair preserves git's knowledge of the worktree
  relationship. After repair + remove, git's state is fully consistent.
- **Rung 3 last** because it's lossy: `prune` removes ALL stale entries, not
  just the one we're targeting, and the filesystem remove bypasses git entirely.

## Where It Appears

The pattern is implemented in two places:

- **`commands::remove()`** (`src/commands.rs:190-206`) — uses `if/else` chain
  with explicit WARN messages at each escalation step.
- **`commands::destroy()`** (`src/commands.rs:341-352`) — uses `or_else` chain
  for a more compact expression since destroy processes multiple repos.

Both share the same escalation logic but `remove()` also checks for a missing
source repo (an additional failure mode where the original clone directory was
deleted).

## Additional Failure Mode: Missing Source Repo

If `entry.source` no longer exists (the original cloned repo was deleted),
there's no git repo to run worktree commands against. In this case, we skip
the git worktree ladder entirely and go straight to `rm -rf` on the worktree
directory. This is handled in `remove()` at `src/commands.rs:208-215`.

## Testing

The recovery ladder is exercised by integration tests in `tests/cli_test.rs`
for the basic remove and destroy paths. The moved-worktree and corrupted-link
scenarios are harder to test because they require simulating filesystem-level
changes that break git's internal tracking.

## Commit Context

This pattern was formalized in commit `f076dc8` ("fix: recover from moved
worktree directories on remove and destroy"). Prior to this, a moved worktree
would leave git-rig unable to clean up, requiring manual `git worktree prune`
and `rm -rf`.
