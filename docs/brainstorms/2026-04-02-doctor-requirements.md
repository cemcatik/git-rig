---
date: 2026-04-02
topic: doctor
---

# `git rig doctor` — Environment & Workspace Health

## Problem Frame

Users discover environment and workspace problems one-at-a-time during command failures. Git version too old? You find out when `worktree repair` fails on a rare recovery path. `origin/HEAD` not set? You find out when `add` can't detect the default branch. Upstream deleted on remote? You find out mid-`sync`. These are all diagnosable upfront, but nothing checks proactively.

Drift detection (`drift.rs`) catches manifest/reality mismatches before `status`, `sync`, `exec`, and `refresh`, but it deliberately stays fast and local — it doesn't check git version, remote reachability, or upstream validity.

`doctor` fills the gap: a single command that validates the full stack from environment prerequisites through per-repo health, with actionable fix guidance.

## Requirements

- R1. `doctor` runs two tiers of checks: **environment** (always) and **per-repo** (when inside a rig).
- R2. Outside a rig, `doctor` runs environment checks only and reports results. No error for missing rig context.
- R3. Inside a rig, `doctor` runs environment checks first, then per-repo checks for every repo in the manifest.
- R4. Environment checks:
  - R4a. Git is on PATH and executable.
  - R4b. Git version is 2.30 or newer.
- R5. Per-repo checks (inside a rig):
  - R5a. Source repo directory exists.
  - R5b. Worktree directory exists.
  - R5c. Worktree is reachable (git commands succeed in it).
  - R5d. Checked-out branch matches manifest.
  - R5e. No unexpected detached HEAD (manifest expects a named branch but worktree is detached).
  - R5f. `origin/HEAD` is set (required for default branch detection).
  - R5g. Remote is reachable (can contact origin).
  - R5h. Upstream branch exists on remote (when repo has upstream configured).
- R6. For R5a–R5e, reuse `check_drift()` from `drift.rs`. Do not duplicate drift logic.
- R7. Each check result is one of: PASS, WARN, or FAIL.
  - R4a, R4b: FAIL (can't function without these).
  - R5a, R5b, R5c: FAIL (repo is unusable).
  - R5d, R5e: WARN (repo works but may not behave as expected).
  - R5f: WARN (default branch detection won't work).
  - R5g: WARN (network-dependent, may be transient).
  - R5h: WARN (sync will fail but other commands work).
- R8. Non-PASS results include: a short description of the problem and a copy-paste fix command (or instruction when no single command suffices).
- R9. Exit code: 0 when all checks pass, 1 when any check is WARN or FAIL.
- R10. Environment FAIL checks (R4a, R4b) short-circuit: skip per-repo checks since they require a working git.

## Success Criteria

- Running `git rig doctor` in a healthy rig prints all-pass and exits 0.
- Running `git rig doctor` outside any rig prints environment results only and exits 0 (if git is healthy).
- Each known pain point from `docs/solutions/git-rs-review-findings.md` (git version, origin/HEAD) is caught by doctor with a clear fix suggestion.
- A user who hits a confusing git error can run `doctor` and find the root cause without manual debugging.

## Scope Boundaries

- Doctor does not auto-fix anything. It diagnoses and suggests.
- Doctor does not replace drift detection on other commands. Drift continues to run as a fast per-command guard.
- No `--fix` flag in v1. Diagnosis only.
- No JSON output in v1 (that's a separate ideation item).
- No parallel check execution in v1 — sequential is fine for the expected repo count.

## Key Decisions

- **Reuse drift, don't duplicate**: R5a–R5e map directly to `DriftKind` variants. Doctor calls `check_drift()` and translates results into its pass/warn/fail output.
- **Always run network checks**: Doctor is an explicit command, not a pre-command guard. Users expect thoroughness. A few seconds of latency is acceptable.
- **Works outside a rig**: Environment checks (git version) are valuable during initial setup or after system updates, before any rig exists.
- **Tiered fix guidance**: Each issue gets both a short explanation (why it matters) and a concrete command (what to do about it).
- **Exit 1 on any issue**: Enables CI gating and scripting. Users who want advisory-only can ignore the exit code.
- **Short-circuit on environment failure**: If git isn't installed or is too old, per-repo checks would produce misleading errors.

## Dependencies / Assumptions

- Git version parsing: `git --version` output format is stable (`git version X.Y.Z`).
- Remote reachability: `git ls-remote --exit-code` is the standard probe. Requires network access.
- Upstream branch check: `git ls-remote --exit-code origin <branch>` confirms the branch exists on the remote.

## Outstanding Questions

### Deferred to Planning

- [Affects R5g][Technical] What's the right timeout/approach for `git ls-remote` on slow networks? Should we set a timeout or let git's own timeout handle it?
- [Affects R6][Technical] Does `check_drift()` need any changes to support doctor's needs, or can doctor consume its output as-is?
- [Affects R7][Technical] Should PASS results be printed (verbose) or suppressed (only show issues)? Both modes may be valuable — consider a `--verbose` flag during planning.

## Next Steps

-> `/ce:plan` for structured implementation planning
