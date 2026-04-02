---
date: 2026-04-01
topic: open-ideation
focus: open-ended product improvement
---

# Ideation: Open-Ended Product Improvement

## Codebase Context

- **Project**: Rust CLI (edition 2024, MSRV 1.88), single-binary git worktree workspace manager
- **Shape**: 6 source files (~2670 LOC total), 9 subcommands, clap derive, shells out to git
- **Version**: v0.4.1, early stage, single maintainer, ships cross-platform (macOS/Linux/Windows)
- **Strengths**: Clean architecture, isolated test suite (TestSandbox), well-documented design decisions (7 solution docs), cross-platform discipline
- **Known debt**: commands.rs monolith (1036 lines), no shell completions, no --json output, `ahead_behind()` silently swallows errors, git version requirement undocumented, no `--dry-run` beyond `destroy`

### Past Learnings (docs/solutions/)
- git.rs hidden contracts: git 2.30+ required, ahead_behind fabricates (0,0) on error, stderr lost in git_run
- Partial failure must return error: shared helper proposed but not implemented
- Worktree recovery ladder: 3-rung escalation (remove -> repair+retry -> rm+prune)
- Cross-platform: thin abstractions with #[cfg] guards, symlink_or_copy pattern
- Riginclude: ignore crate directory matching quirks, symlink_metadata over exists
- Upstream config: Option<T> + effective_*() fallback pattern for extensible per-repo config

## Ranked Ideas

### 1. Manifest/Reality Drift Detection
**Description:** Before executing `status`, `sync`, `exec`, or `refresh`, validate that each repo's worktree exists, the checked-out branch matches what the manifest says, and the source repo is accessible. Surface discrepancies as warnings at command entry rather than letting them cause confusing mid-operation failures.
**Rationale:** `sync` rebases `effective_upstream` without verifying the worktree is on the expected branch. `status` reads `current_branch` from git but never compares it to `repo.branch`. One guard clause per command entry transforms confusing mid-operation failures into clear upfront warnings. Changes the trust equation for the tool.
**Downsides:** Adds small latency to every command. Must be fast enough to be invisible.
**Confidence:** 90%
**Complexity:** Low
**Status:** Explored (2026-04-01 — brainstorm)

### 2. `git rig doctor` — Environment & Workspace Health
**Description:** New subcommand performing: git version validation (2.30+), per-repo source path existence, worktree link integrity, remote reachability, upstream branch existence, `origin/HEAD` presence. Outputs a pass/warn/fail table with actionable fix suggestions.
**Rationale:** Multiple documented pain points converge here: undocumented git 2.30 requirement, `DefaultBranchNotFound` when `origin/HEAD` isn't set, broken worktree links after directory moves, upstream branches deleted on remote. Users currently discover these one-at-a-time during command failures. Doctor makes them proactively discoverable and serves as a teaching tool for the git worktree mental model.
**Downsides:** Scope creep risk — must resist adding every possible check. Network checks (remote reachability) can be slow.
**Confidence:** 80%
**Complexity:** Medium
**Status:** Done (2026-04-02 — brainstorm + implementation)

### 3. Cross-Workspace Branch Conflict Detection
**Description:** When `add` fails with "already checked out in another worktree," parse `git worktree list` on the source repo, cross-reference known rigs, and tell the user exactly which workspace holds the conflicting branch. Currently the `branch_hint` closure just says "may already be checked out" with no actionable detail.
**Rationale:** Branch conflicts are git's worst error message for worktree users, and git-rig is in the perfect position to provide context. Power users with multiple rigs sharing repos hit this constantly. Tiny feature, massive trust-building.
**Downsides:** Requires adding a `worktree_list` function to git.rs. Cross-referencing rigs means resolving multiple manifests.
**Confidence:** 85%
**Complexity:** Low-Medium
**Status:** Done (2026-04-02 — find_worktree_for_branch() in git.rs, branch_hint shows worktree path)

### 4. Shell Completions
**Description:** Generate bash/zsh/fish/PowerShell completions via `clap_complete`. Dynamic completions for rig names (via `find_workspaces`) and repo names (via manifest). Approximately 20 lines of code for static completions; dynamic completions require a custom completer.
**Rationale:** Highest-ROI quality-of-life feature for any CLI. Disproportionate impact on discoverability and adoption. Every new subcommand or flag automatically gets completions. Near-zero cost for static, moderate for dynamic.
**Confidence:** 95%
**Complexity:** Low
**Status:** Done (2026-04-02 — static completions via `completions` subcommand, dynamic left for future)

### 5. Partial Sync with `--repo` Filtering
**Description:** Add `--repo` flag to `sync` (matching the pattern `exec` already uses). Allow syncing individual repos instead of all-or-nothing. Also add merge-base pre-check to skip repos that are already up-to-date.
**Rationale:** `exec` already has `--repo` filtering with validation against the manifest. `sync` is all-or-nothing, meaning one problematic repo blocks the workflow for all others. A small surface area change with disproportionate practical value — especially when one repo has conflicts and you need to sync the rest NOW.
**Downsides:** Minimal. Consistency improvement that reuses existing patterns.
**Confidence:** 95%
**Complexity:** Low
**Status:** Done (2026-04-02 — --repo flag on sync with validation, drift scoping, 5 E2E tests)

### 6. Machine-Readable Output (`--json`)
**Description:** Global `--json` flag switching `status`, `list`, and `exec` output to structured JSON. Enables scripting (`git rig status --json | jq '.repos[] | select(.dirty)'`), CI integration, and editor plugins. `Manifest` already derives `Serialize`; `RigError` has structured fields.
**Rationale:** Gateway to ecosystem integration. Scripts, CI pipelines, editor plugins, and jq pipelines all need this. Forces a clean compute-then-render separation that benefits future output changes. Makes every existing command more valuable without changing semantics.
**Downsides:** Doubles output testing surface for each command. Must define a stable JSON schema early.
**Confidence:** 70%
**Complexity:** Medium
**Status:** Unexplored

## Rejection Summary

| # | Idea | Reason Rejected |
|---|------|-----------------|
| 1 | `git rig switch` (multi-repo branch change) | Conflicts with rig-as-branch-context identity model; `create --from` covers the use case |
| 2 | Extract `run_for_each_repo` helper | Internal refactor — no user impact at 4 call sites |
| 3 | Split `commands.rs` into modules | Cosmetic for single maintainer; 1036 lines manageable with editor search |
| 4 | Honest `ahead_behind()` | Bug fix, not ideation-level — just do it |
| 5 | Global `--verbose`/`--quiet` | Premature abstraction over println for 6-file tool |
| 6 | Workspace-level config/defaults | Shell aliases solve this; not enough knobs to justify config system |
| 7 | Rename/move rig | Rare operation; destroy+recreate works; high complexity for low payoff |
| 8 | Lifecycle hooks | `exec` already handles this; hooks add hidden execution model |
| 9 | Workspace snapshots/lockfile | Git already has commit SHAs; parallel versioning is over-engineering |
| 10 | Sparse checkout integration | Niche monorepo use case; users can run via `exec` |
| 11 | Interactive `add` with discovery | TUI maintenance burden; explicit paths are intentional |
| 12 | Parallel git operations | Real benefit for 10+ repos but high complexity cost for v0.4 tool |
| 13 | `git rig init` auto-discover | Inverts explicit-is-better principle |
| 14 | Git version gate at startup | Too small for ideation — just a 15-minute fix |
| 15 | Manifest as TOML | Breaking format change at v0.4 for inline comments |
| 16 | Invert worktree model | Fundamental architecture rewrite; different product |
| 17 | git2 for read operations | 3MB dependency for marginal speed; two failure modes |
| 18 | Rig templates | Premature; `create --from` already serves this role |
| 19 | Global rig registry | New consistency problem (stale entries) for marginal benefit |
| 20 | Plan/execute split | Architectural astronautics for a 2670-line CLI |
| 21 | Structured git output capture | Infrastructure for infrastructure; build when forced |
| 22 | Command context object | One function with too many args doesn't justify abstraction |
| 23 | Dry-run as trait | Only `destroy` needs dry-run; per-command flag is fine |
| 24 | Stale worktree detection | Natural extension of `doctor`, not standalone feature |
| 25 | Interrupt-safe journal | Enterprise recovery for CLI where "run again" works |

## Session Log
- 2026-04-01: Initial open-ended ideation — 48 raw ideas from 6 frames, 31 after dedupe, 6 survived adversarial filtering
- 2026-04-01: Brainstorm started for #1 (Manifest/Reality Drift Detection)
- 2026-04-02: Implemented #5 (sync --repo), #3 (branch conflict detection), #4 (shell completions)
- 2026-04-02: Brainstormed + implemented #2 (git rig doctor)
