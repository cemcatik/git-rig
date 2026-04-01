---
title: "feat: Manifest/Reality Drift Detection"
type: feat
status: completed
date: 2026-04-01
origin: docs/brainstorms/2026-04-01-manifest-drift-detection-requirements.md
---

# feat: Manifest/Reality Drift Detection

## Overview

Add an upfront drift check to `status`, `sync`, `exec`, and `refresh` that detects when the filesystem/git state has diverged from what `.rig.json` records. Print a consolidated warning block before command output. For `sync`, skip repos with branch drift to prevent rebasing the wrong branch.

## Problem Statement / Motivation

When git operations happen outside git-rig (manual `git checkout`, `git branch -D`, moving directories), the manifest silently diverges from reality. The most dangerous case: `sync` rebases onto `effective_upstream` without verifying the worktree is on the expected branch (`commands.rs:869`). Users discover drift mid-operation with confusing failures. Currently, each command handles missing worktrees independently with inconsistent messaging (`status` prints `"(missing)"`, `sync` prints `"(missing, skipped)"`, `exec` prints `"WARN worktree missing, skipped"`), and no command detects branch mismatch or missing source repos.

(see origin: `docs/brainstorms/2026-04-01-manifest-drift-detection-requirements.md`)

## Proposed Solution

Add a new `drift.rs` module containing drift detection types and a `check_drift()` function. Each of the four affected commands calls it immediately after `resolve_workspace()`, prints any warnings via `print_drift_warnings()`, then uses the returned `DriftReport` for skip logic and to reuse branch data.

## Technical Considerations

### Resolved Questions from Origin Document

**Q: Output format for drift warning block?**
Per-line prefix matching existing vocabulary. Use `"DRIFT".yellow()` for consistency with `WARN`/`ERR`/`SKIP`. Show expected vs actual state inline:
```
  DRIFT repo-a: on main, expected rig/feature-x (branch mismatch)
  DRIFT repo-b: worktree missing
  DRIFT repo-c: source repo missing (/path/to/clone)
  DRIFT repo-d: detached HEAD, expected rig/feature-x
```
Followed by an empty line before normal command output. No box/banner — that would be a new pattern not in the codebase.

**Q: Should `exec` and `refresh` skip repos with missing source repos?**
- `exec`: No. Exec runs in the worktree directory; missing source doesn't prevent execution. Show the drift warning but don't skip.
- `refresh`: Yes. Refresh fetches from `repo.source` (`commands.rs:765`); if it's gone, skip with drift warning instead of failing at fetch time.

**Q: Right abstraction for drift check?**
Free function `check_drift(&Manifest, &Path) -> DriftReport` in a new `src/drift.rs` module. Returns a struct containing per-repo drift entries and cached branch data. Commands inspect the report for warnings and skip decisions. This avoids coupling drift logic into workspace resolution.

**Q: How does `current_branch` behave on a broken worktree?**
`git::current_branch()` calls `git branch --show-current` via `git_output()`. If the worktree's `.git` file is broken/missing, git returns a non-zero exit and `git_output` returns `Err`. The drift check handles this as an additional drift kind: `WorktreeUnreachable`. This prevents the entire command from crashing when one worktree is corrupted.

### Per-Command Drift Type Relevance

Not all drift types matter for all commands. The drift check runs all checks uniformly (simplicity > micro-optimization), but each command filters what to report and what to skip:

| Drift Type | status | sync | exec | refresh |
|---|---|---|---|---|
| R2a: Missing worktree | warn | warn+skip | warn+skip | n/a (ignore) |
| R2b: Missing source | warn | warn+skip | warn only | warn+skip |
| R2c: Branch mismatch | warn | warn+skip | warn only | n/a (ignore) |
| R2d: Unexpected detached | warn | warn+skip | warn only | n/a (ignore) |
| WorktreeUnreachable | warn | warn+skip | warn+skip | n/a (ignore) |

- `status` is read-only: warn about everything, skip nothing, display actual state as always.
- `sync` is the most dangerous: skip all drifted repos to prevent wrong-branch rebases and failed fetches.
- `exec` runs user commands in worktree dirs: skip only when the worktree is physically unavailable (missing or unreachable). Branch drift doesn't prevent execution.
- `refresh` operates on `repo.source` only: worktree drift is irrelevant. Skip only when the source repo is missing.

### Performance

The drift check calls `git::current_branch()` (one subprocess) per repo with an existing worktree. For a 10-repo rig, that's ~10 subprocesses adding ~100-200ms total. This is negligible compared to `sync` (network fetches) or `exec` (arbitrary commands). For the common case (no drift), the check completes in the same time but produces no output — invisible to the user.

To avoid redundant work, the `DriftReport` caches branch data so `status` can reuse `current_branch` results instead of calling git again per repo.

### `exec --repo` Scoping

When `exec` uses `--repo` filtering, the drift check still runs across all repos (it's fast), but `print_drift_warnings()` only reports drift for the filtered repos. This prevents noise about repos the user isn't operating on.

### Interaction with Existing Detached Repo Handling

Expected detached repos (`repo.branch == git::DETACHED` and worktree is detached) produce zero drift. They are not in the drift report. `sync`'s existing inline message `"(detached, skipped)"` (`commands.rs:821-826`) remains as-is — it's informational, not a drift warning.

Only *unexpected* detached HEAD (manifest says a named branch, worktree is detached) appears in the drift block.

## Acceptance Criteria

- [ ] `git rig status` shows `DRIFT` warning when a worktree branch doesn't match the manifest
- [ ] `git rig sync` skips rebase for repos with branch mismatch (prints `DRIFT` warning, does not rebase)
- [ ] `git rig sync` skips repos with unexpected detached HEAD
- [ ] `git rig sync` skips repos with missing source repos (drift warning, not fetch error)
- [ ] `git rig exec` skips repos with missing worktrees (drift warning, consistent with status/sync)
- [ ] `git rig refresh` skips repos with missing source (drift warning instead of cryptic fetch failure)
- [ ] Expected detached repos (`repo.branch == DETACHED`) produce no drift warnings
- [ ] No output change when no drift exists (common case is invisible)
- [ ] Existing per-command `worktree_path.exists()` checks in status, sync, exec are removed (replaced by upfront pass)
- [ ] `exec --repo` filtering scopes drift warnings to the filtered repos only
- [ ] Drift warnings use `"DRIFT".yellow()` prefix, show expected vs actual state
- [ ] `WorktreeUnreachable` drift handled gracefully (broken `.git` link doesn't crash the command)

## Implementation Phases

### Phase 1: Core Drift Types and Detection (src/drift.rs)

New file. Contains:

```rust
// src/drift.rs

pub enum DriftKind {
    MissingWorktree,
    MissingSource,
    BranchMismatch { expected: String, actual: String },
    UnexpectedDetached { expected: String },
    WorktreeUnreachable { error: String },
}

pub struct RepoDrift {
    pub repo_name: String,
    pub kind: DriftKind,
}

pub struct DriftReport {
    pub drifts: Vec<RepoDrift>,
    /// Cached current_branch results. Key = repo name, Value = branch (if available).
    /// Commands can reuse this to avoid redundant git calls.
    pub branches: std::collections::HashMap<String, String>,
}
```

`check_drift(manifest: &Manifest, ws_dir: &Path) -> DriftReport`:
- Iterate `manifest.repos`
- For each repo:
  1. Check `worktree_path.exists()` -> if missing, add `MissingWorktree`, skip further checks for this repo
  2. Check `repo.source.exists()` -> if missing, add `MissingSource`
  3. Call `git::current_branch(&worktree_path)`:
     - If `Err` -> add `WorktreeUnreachable`, skip branch checks
     - If `Ok(branch)`:
       - Cache in `report.branches`
       - If `repo.branch != git::DETACHED && branch == git::DETACHED` -> add `UnexpectedDetached`
       - Else if `repo.branch != git::DETACHED && branch != repo.branch` -> add `BranchMismatch`
       - If `repo.branch == git::DETACHED && branch == git::DETACHED` -> no drift (expected)

`print_drift_warnings(report: &DriftReport, repos: Option<&[String]>)`:
- If no drifts (or no drifts for filtered repos), return immediately (no output)
- Filter drifts to `repos` if provided (for exec --repo scoping)
- For each drift, print formatted line:
  - `MissingWorktree`: `"  DRIFT {name}: worktree missing"`
  - `MissingSource`: `"  DRIFT {name}: source repo missing ({path})"`
  - `BranchMismatch`: `"  DRIFT {name}: on {actual}, expected {expected}"`
  - `UnexpectedDetached`: `"  DRIFT {name}: detached HEAD, expected {expected}"`
  - `WorktreeUnreachable`: `"  DRIFT {name}: worktree unreachable ({error})"`
- Print empty line after block

Helper: `DriftReport::has_drift(&self, repo_name: &str, kind_predicate) -> bool` for commands to check skip conditions.

### Phase 2: Integrate into Commands (src/commands.rs)

**status** (`commands.rs:703-748`):
- After `resolve_workspace`, call `check_drift(&manifest, &ws_dir)`
- Call `print_drift_warnings(&report, None)` (all repos, all drift types reported)
- In the per-repo loop: remove `if !worktree_path.exists()` check (lines 718-720). Instead, check `report` for `MissingWorktree` or `WorktreeUnreachable` -> skip with `continue`
- Reuse `report.branches.get(&repo.name)` instead of calling `git::current_branch()` again. Fall back to `git::current_branch()` if not cached (shouldn't happen, but defensive).

**sync** (`commands.rs:804-957`):
- After `resolve_workspace`, call `check_drift(&manifest, &ws_dir)`
- Call `print_drift_warnings(&report, None)`
- In the per-repo loop: remove `if !worktree_path.exists()` check (lines 815-818). Instead, check if repo has ANY drift -> skip with `continue`. The drift block already warned the user.
- Keep the existing `if repo.branch == git::DETACHED` inline skip (lines 820-826) — this is expected behavior, not drift.

**exec** (`commands.rs:963-1035`):
- After `resolve_workspace` and `--repo` validation, call `check_drift(&manifest, &ws_dir)`
- Call `print_drift_warnings(&report, Some(filter_repos))` if `filter_repos` is non-empty, else `None`
- In the per-repo loop: remove `if !worktree_path.exists()` check (lines 995-998). Instead, check for `MissingWorktree` or `WorktreeUnreachable` -> skip. Branch mismatch and missing source do NOT cause skip for exec.

**refresh** (`commands.rs:755-798`):
- After `resolve_workspace`, call `check_drift(&manifest, &ws_dir)`
- Call `print_drift_warnings(&report, None)` — but only `MissingSource` drifts are relevant. The print function should get a filter, OR refresh only prints source-related drifts.
- Simpler: add a `relevant_for_refresh(drift: &DriftKind) -> bool` check inside the print function, or pass a filter closure. Even simpler: have a `DriftScope` enum (`All`, `WorktreeCommands`, `SourceOnly`) that `print_drift_warnings` uses to filter.
- In the per-repo loop: check for `MissingSource` -> skip with `continue` instead of failing at `git::fetch()`.

### Phase 3: Wire Up Module (src/main.rs)

Add `mod drift;` to `main.rs`.

### Phase 4: Tests

**Unit tests in `src/drift.rs`** (inline `#[cfg(test)]` module):
- `test_no_drift_when_everything_matches` — manifest matches reality, report is empty
- `test_missing_worktree_detected` — remove worktree dir, verify `MissingWorktree` in report
- `test_missing_source_detected` — remove source dir, verify `MissingSource`
- `test_branch_mismatch_detected` — checkout different branch in worktree, verify `BranchMismatch` with correct expected/actual
- `test_unexpected_detached_detected` — detach HEAD in worktree that manifest says has a branch
- `test_expected_detached_no_drift` — manifest says `DETACHED`, worktree is detached -> no drift
- `test_multiple_drift_types` — multiple repos with different drift kinds
- `test_branches_cached` — verify `report.branches` contains the branch data for reuse

These tests need real git repos. Use `TestSandbox` (from `tests/common/mod.rs`) pattern: create temp dir, init bare repo, clone, create worktree, then mutate to induce drift.

**E2E tests in `tests/cli_test.rs`**:
- `test_status_shows_drift_warning_on_branch_mismatch` — create rig, checkout different branch in worktree, run `git rig status`, assert output contains `DRIFT` and branch names
- `test_sync_skips_branch_drifted_repo` — create rig with 2 repos, drift one, run `git rig sync`, assert drifted repo is NOT rebased (check HEAD didn't change), healthy repo IS rebased
- `test_sync_skips_missing_source` — remove source repo dir, run `git rig sync`, assert drift warning (not fetch error)
- `test_exec_warns_but_runs_on_branch_mismatch` — drift a branch, run `git rig exec`, assert DRIFT warning appears but command still runs in the worktree
- `test_exec_skips_missing_worktree` — remove worktree dir, run `git rig exec`, assert skip
- `test_no_drift_output_when_clean` — normal rig, run all commands, assert no `DRIFT` in output
- `test_exec_repo_filter_scopes_drift_warnings` — drift repo-b, run `exec --repo repo-a`, assert no drift warning shown
- `test_refresh_skips_missing_source` — remove source dir, run `git rig refresh`, assert drift warning (not fetch error)

Existing test helpers to reuse:
- `TestSandbox::corrupt_worktree_metadata()` for `WorktreeUnreachable` scenarios
- `TestSandbox::move_workspace()` for moved directory scenarios
- Standard `git checkout` via `Command::new("git")` to induce branch mismatch

## System-Wide Impact

- **Interaction graph**: `check_drift()` is called at the top of 4 commands, before any repo iteration. It reads filesystem state and calls `git::current_branch()` per repo. No side effects, no mutation.
- **Error propagation**: `check_drift()` never returns `Err`. All failures (git errors, permission errors) are absorbed into the `DriftReport` as `WorktreeUnreachable` entries. Commands that currently return errors for missing worktrees (via the collect-and-fail pattern in sync/exec) will no longer hit those paths for drift-related issues — the upfront check handles them.
- **State lifecycle risks**: None. Drift detection is read-only. No manifest writes, no git mutations.
- **API surface parity**: The four multi-repo iteration commands all get the same drift check. Single-repo commands (`add`, `remove`) and lifecycle commands (`create`, `destroy`) are unaffected.

## Dependencies & Risks

- **Risk: Removing per-command exists() checks could regress edge cases.** Mitigation: E2E tests cover all skip scenarios. The drift check is strictly more comprehensive (checks 5 conditions vs. the current 1).
- **Risk: `current_branch()` subprocess adds latency.** Mitigation: Already quantified at ~10-20ms per repo. Cached in DriftReport for reuse. Common case (no drift) has same latency but no output.
- **Risk: Behavioral change in sync exit codes.** Today, a missing source repo causes sync to fail at fetch and add to the error list (non-zero exit). With drift detection, the repo is skipped with a warning (potentially zero exit if all other repos succeed). Mitigation: This is intentionally better behavior — the user was warned upfront, and healthy repos succeeded.

## Sources & References

### Origin

- **Origin document:** [docs/brainstorms/2026-04-01-manifest-drift-detection-requirements.md](docs/brainstorms/2026-04-01-manifest-drift-detection-requirements.md) — Key decisions carried forward: warn-and-continue model, skip drifted repos in sync only, centralized upfront pass replacing per-command handling, no override flag.

### Internal References

- Provision report pattern: `src/provision.rs:45-47` (`ProvisionReport` struct), `src/provision.rs:243-296` (`print_provision_report()`) — closest existing multi-item report pattern
- Existing missing-worktree handling: `src/commands.rs:718-720` (status), `src/commands.rs:815-818` (sync), `src/commands.rs:995-998` (exec)
- DETACHED constant: `src/git.rs:9`
- current_branch: `src/git.rs:102-109`
- TestSandbox: `tests/common/mod.rs:21-149`

### Institutional Learnings Applied

- `docs/solutions/git-rs-review-findings.md` — `ahead_behind()` silently returns (0,0) on error; drift detection avoids using it for validation
- `docs/solutions/partial-failure-must-return-error.md` — drift detection follows warn-and-continue (like provisioning), not collect-and-fail, since it's informational
- `docs/solutions/worktree-recovery-ladder.md` — missing source repo is outside the recovery ladder; drift detection catches it separately
- `docs/solutions/riginclude-local-file-provisioning.md` — use `path.exists()` for directories (not symlinks); `symlink_metadata` distinction is relevant for provisioned files but not worktree directories
- `docs/solutions/cross-platform-symlink-fallback.md` — no platform-specific code needed for drift detection (all checks are `path.exists()` and `git` subprocesses)
