---
date: 2026-04-01
topic: manifest-drift-detection
---

# Manifest/Reality Drift Detection

## Problem Frame

When git operations happen outside of git-rig (manual `git checkout`, `git branch -D`, moving directories), the `.rig.json` manifest can silently diverge from actual filesystem/git state. Users discover this mid-operation — most dangerously during `sync`, which rebases onto `effective_upstream` without verifying the worktree is on the expected branch. Currently, each command handles missing worktrees independently with inconsistent messaging, and no command detects branch mismatch or missing source repos.

## Requirements

- R1. Before executing `status`, `sync`, `exec`, or `refresh`, run an upfront drift check across all repos in the manifest.
- R2. Detect four types of drift:
  - R2a. **Missing worktree** — the worktree directory does not exist.
  - R2b. **Missing source repo** — the source clone directory no longer exists.
  - R2c. **Branch mismatch** — the worktree's checked-out branch differs from the manifest's `repo.branch`.
  - R2d. **Unexpected detached HEAD** — the manifest records a branch name but the worktree is in detached HEAD state.
- R3. When drift is detected, print a warning block at the top of command output (before normal output begins), listing each drifted repo with drift type and details.
- R4. After printing warnings, proceed with the command (warn-and-continue model). Do not block or prompt.
- R5. For `sync` specifically: skip the rebase for repos with branch mismatch or unexpected detached HEAD drift. These repos should not be synced on the wrong branch.
- R6. Replace the existing per-command `worktree_path.exists()` checks in `status`, `sync`, `exec`, and `refresh` with the centralized upfront pass. One code path for drift detection, consistent messaging.
- R7. No `--no-check` or override flag. Drift checks always run.

## Success Criteria

- A user who manually runs `git checkout main` inside a rig worktree sees a clear warning on the next `git rig status` or `git rig sync`.
- `sync` never rebases a branch-drifted repo — it skips with a warning instead of silently rebasing the wrong branch.
- Drift warnings are visually distinct from command output (e.g., prefixed with `DRIFT` or similar).
- No measurable latency increase for the common case (no drift). Checks are local filesystem and git operations only.

## Scope Boundaries

- No self-healing or auto-repair. Drift detection is read-only — it reports, not fixes. Repair belongs in a future `doctor` command.
- No manifest-update option (e.g., "update manifest to match reality"). Out of scope.
- Commands that operate on a single repo (`add`, `remove`) or manage the whole rig (`create`, `destroy`) are not affected. Only the four multi-repo iteration commands get the upfront pass.
- Detached repos that are *expected* to be detached (manifest records `(detached)`) should not trigger drift warnings.

## Key Decisions

- **Warn and continue**: Matches the `.riginclude` provisioning philosophy — warnings not errors. Keeps CI/scripting working without flags.
- **Skip drifted repos in sync**: Prevents the most dangerous drift failure (rebasing `main` onto a feature upstream). Other commands (exec, status, refresh) still process drifted repos since they're less dangerous.
- **Upfront pass replaces per-command handling**: One code path, consistent output format. Avoids two parallel systems for the same concern.
- **No override flag**: Checks are fast (local only) and warnings don't block. Adding a flag for a few milliseconds of overhead isn't worth the surface area.

## Outstanding Questions

### Deferred to Planning

- [Affects R3][Technical] What's the exact output format for the drift warning block? Should it use a box/banner, or per-line `DRIFT` prefix? Look at how other commands format warnings for consistency.
- [Affects R5][Technical] Should `exec` and `refresh` skip repos with missing source repos, or only skip for missing worktrees? The source repo isn't needed for exec (which runs in the worktree dir), but is needed for refresh (which provisions from source).
- [Affects R6][Technical] What's the right abstraction for the shared drift check? A free function returning a `Vec<DriftWarning>` that commands inspect, or something integrated into workspace resolution?
- [Affects R2c][Needs research] How does `git::current_branch` behave when called on a worktree with a missing `.git` file (broken worktree link)? This may need a fallback before the branch comparison.

## Next Steps

-> `/ce:plan` for structured implementation planning
