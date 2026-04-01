---
title: Manifest drift detection
category: logic-errors
date: 2026-04-01
tags: [state-drift, silent-failure, defensive-checks, worktree-safety, rebase-guard]
modules: [src/drift.rs, src/commands.rs]
severity: high
---

# Manifest Drift Detection

## Problem

git-rig's `.rig.json` manifest tracks repos, branches, and remotes, but git operations outside git-rig (manual `git checkout`, `git branch -D`, moving directories) cause silent divergence between manifest state and reality.

The most dangerous failure: `sync` rebasing onto `effective_upstream` without verifying the worktree is on the expected branch. If a user manually checked out a different branch, `sync` would rebase the wrong branch onto upstream — potentially destroying work.

Each command also handled missing worktrees independently with inconsistent messaging:
- `status`: `"(missing)"`
- `sync`: `"(missing, skipped)"`
- `exec`: `"WARN worktree missing, skipped"`

## Root Cause

No validation existed between manifest state and filesystem/git reality at command entry. Commands trusted the manifest blindly. There was no shared concept of "drift" — each command reimplemented its own ad-hoc checks (or didn't check at all).

## Solution

A new `src/drift.rs` module provides centralized drift detection that runs once at command entry and produces a report consumed by all four multi-repo commands.

### Five drift types

```rust
pub enum DriftKind {
    MissingWorktree,                                    // worktree dir gone
    MissingSource,                                      // source clone dir gone
    BranchMismatch { expected: String, actual: String }, // wrong branch
    UnexpectedDetached { expected: String },             // detached when shouldn't be
    WorktreeUnreachable { error: String },               // .git link broken
}
```

### check_drift() — never-erroring single-pass detection

The function iterates all repos, checks worktree existence, source existence, and branch match. All failures (git errors, permission problems) are absorbed into the report as `WorktreeUnreachable` entries — one broken repo cannot prevent other commands from running.

The report also caches `current_branch` results in a HashMap so `status` can reuse them instead of calling git again per repo.

### Per-command skip logic

Each command uses the drift report differently based on its risk profile:

| Command | What it skips | Why |
|---------|--------------|-----|
| `status` | Nothing (warns only) | Read-only — showing actual state is always useful |
| `sync` | ALL drifted repos | Prevents wrong-branch rebases — the core safety improvement |
| `exec` | Physically unavailable worktrees | Branch mismatch doesn't prevent running arbitrary commands |
| `refresh` | Missing source repos | Refresh fetches from source; worktree state is irrelevant |

### Output format

Warnings appear before normal command output with `DRIFT`-prefixed yellow lines:

```
  DRIFT my-repo: on feature/other, expected rig/my-workspace
  DRIFT broken-repo: source repo missing

Syncing rig 'my-workspace'
  ok healthy-repo already up to date
```

The `exec --repo` filtering scopes drift warnings to only the repos being operated on.

## Key Design Decisions

1. **check_drift() never returns Err.** Follows the `.riginclude` provisioning philosophy: auxiliary validation should not prevent the primary operation from proceeding on healthy repos. See `docs/solutions/partial-failure-must-return-error.md` for the general pattern and its known exceptions.

2. **sync skips ALL drifted repos, not just physically missing ones.** A `BranchMismatch` is just as dangerous as `MissingWorktree` for sync — rebasing the wrong branch onto upstream could destroy work.

3. **Expected detached repos produce zero drift.** When the manifest records `(detached)` and the worktree is detached, that's correct state. But detached-to-named drift (manifest says DETACHED, worktree is on a branch) IS detected as `BranchMismatch`. This case was caught during code review — the initial implementation missed the `else` branch.

4. **DriftScope simplified to a bool.** The initial implementation used a `DriftScope` enum with `All` and `SourceOnly` variants. Code review identified this as a premature abstraction for a boolean flag used at one call site. Simplified to `source_only: bool`.

## What the Code Review Caught

1. **Missing detached-to-named drift** — the `else` branch for when manifest says DETACHED but worktree is on a named branch was initially absent. The `if repo.branch != git::DETACHED` guard skipped the entire check when the manifest said detached.

2. **WorktreeUnreachable had zero test coverage** — the error-absorption boundary (where git errors become drift entries) was untested despite being the single most important safety path.

3. **Weak test assertions** — tests checked substring presence in output (`contains("DRIFT")`) without verifying the actual behavior (HEAD SHA unchanged after a skipped sync). Added `head_sha` helper and behavioral assertions.

4. **Test helpers ignoring exit status** — `checkout_branch_in_worktree` called `Command.output().expect()` which only checks the process spawned, not that it succeeded. A failed checkout would leave the test setup in the wrong state.

5. **Unused return value** — `print_drift_warnings` returned `bool` but no caller used it.

## Prevention Checklist

For future features that touch manifest-vs-reality boundaries:

- [ ] **Use `match` on state pairs, not `if/else if/else`**, when enumerating states. The compiler enforces exhaustive match; `else` does not.
- [ ] **Every error-absorption boundary needs a test.** If a function swallows errors into warnings, test the absorption path explicitly.
- [ ] **Assert behavior, not just output strings.** A test that checks `contains("skipped")` passes even if the operation silently proceeded. Capture state before/after and compare.
- [ ] **Every `Err` variant constructed in the PR needs at least one test that triggers it.**
- [ ] **When adding new manifest fields**, ask: "Can this field diverge from filesystem reality? If so, add it to drift detection simultaneously."

## Related

- `docs/solutions/partial-failure-must-return-error.md` — drift detection is a known exception (warn-and-continue, like provisioning)
- `docs/solutions/git-rs-review-findings.md` — documents `current_branch()` contracts consumed by drift detection
- `docs/solutions/worktree-recovery-ladder.md` — drift detection catches missing source repos upstream, before the recovery ladder runs
- `docs/brainstorms/2026-04-01-manifest-drift-detection-requirements.md` — full requirements and design decisions
