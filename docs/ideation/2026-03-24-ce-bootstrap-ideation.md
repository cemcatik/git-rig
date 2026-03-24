---
date: 2026-03-24
topic: ce-bootstrap
focus: bootstrapping compound-engineering workflows for git-rig
---

# Ideation: Compound-Engineering Bootstrap for git-rig

## Codebase Context

- **Project**: Rust CLI (4 source files, ~1100 LOC, 89 tests), git worktree workspace manager
- **Already set up**: CI (fmt/clippy/deny/coverage), cargo-dist releases, beads issue tracking, CLAUDE.md + AGENTS.md
- **Gaps**: No pre-commit hooks, no justfile, no changelog, no docs/solutions/, empty GitHub issue tracker
- **Past learnings**: None documented (no docs/solutions/ directory)
- **Issue intelligence**: Zero issues filed -- early-stage project

## Ranked Ideas

### 1. ce:review on git.rs -- First Focused Review
**Description:** Run ce:review on git.rs (260 lines), the load-bearing module every command passes through. Document hidden assumptions: git output is UTF-8, `--show-current` requires git 2.22+, `--left-right --count` format stability. Surface risks: stderr swallowed in git_run, stash detection via string diffing.
**Rationale:** Cheapest way to produce actionable findings. Small, critical module with hidden contracts. Review output becomes the first real CE artifact.
**Downsides:** Narrow scope. Not all findings may be immediately actionable.
**Confidence:** 90%
**Complexity:** Low
**Status:** Unexplored

### 2. ce:compound the Worktree Repair Pattern -- First Solution Doc
**Description:** Document the 3-rung recovery ladder (worktree_remove -> worktree_repair -> prune + rm -rf) in docs/solutions/. Pattern appears in both remove() and destroy(), hard-won in commit f076dc8.
**Rationale:** Most non-obvious logic in the codebase, freshly debugged. Core CE motion: convert incident into reusable solution. Without capture, next person touching remove/destroy won't know why each rung exists.
**Downsides:** One doc, narrow scope. Only valuable if docs/solutions/ becomes a habit.
**Confidence:** 95%
**Complexity:** Low
**Status:** Unexplored

### 3. Seed Issue Tracker from CLAUDE.md Gotchas -- Bootstrap the Backlog
**Description:** File three known gotchas as beads issues: (1) "already checked out" branch error on add, (2) origin/HEAD must be set for default_branch() detection, (3) destroy force-removes dirty worktrees while remove does not.
**Rationale:** Empty tracker creates false signal. These are real, documented friction points. Cost: three bd create calls.
**Downsides:** These are known-and-accepted behaviors, not bugs. Might feel like make-work.
**Confidence:** 85%
**Complexity:** Low
**Status:** Unexplored

### 4. Justfile as Canonical Dev Interface
**Description:** Add Justfile with recipes: check (fmt + clippy), test (all), test-unit, test-integration, test-e2e, coverage, install. One command per workflow.
**Rationale:** CLAUDE.md lists 4 separate cargo test invocations. CI repeats them. Justfile collapses into discoverable interface for agents and humans. Future CE workflows (bd preflight, pre-commit) can call just check.
**Downsides:** Another tool (just) to install. Marginal for solo dev who knows commands.
**Confidence:** 80%
**Complexity:** Low
**Status:** Unexplored

### 5. Pre-commit Hooks (fmt + clippy)
**Description:** Git pre-commit hook running cargo fmt --check and cargo clippy -- -D warnings. Installable via just install-hooks. Slow tests stay in CI.
**Rationale:** CI enforces quality but feedback loop is slow. Sub-5-second local gate catches format/lint before commits. Clean diffs prerequisite for useful CE review cycles.
**Downsides:** Bypassable with --no-verify. Adds friction to small commits.
**Confidence:** 85%
**Complexity:** Low
**Status:** Unexplored

### 6. Structured Error Taxonomy (RigError enum)
**Description:** Introduce RigError enum (WorktreeAlreadyCheckedOut, BranchConflict, ManifestNotFound, DirtyWorktree, etc.) replacing ad-hoc anyhow!() strings. Keep anyhow as transport. Enables machine-readable errors and proper test assertions.
**Rationale:** 20+ distinct anyhow!() callsites with no shared identity. Foundation for future --json output, exit-code differentiation, and agent-friendly diagnostics.
**Downsides:** Touches every error site. Higher effort than items 1-5.
**Confidence:** 75%
**Complexity:** Medium
**Status:** Unexplored

## Rejection Summary

| # | Idea | Reason Rejected |
|---|------|-----------------|
| 1 | Machine-readable JSON output | Product feature; depends on error taxonomy first |
| 2 | Manifest schema versioning | Good but conditional; defer |
| 3 | Changelog automation (git-cliff) | Low CE value; cargo-dist handles release notes |
| 4 | Property-based testing | Hard bugs are filesystem/git-state, not input-space |
| 5 | Shell completions | UX polish, zero CE value |
| 6 | Rig templates / snapshot | High cost, speculative |
| 7 | Parallel sync/exec | Performance optimization, not CE concern |
| 8 | Doctor command | Good feature, but sequence after solution doc |
| 9 | TestSandbox upgrade | Upgrade when gaps found, not speculatively |
| 10 | Snapshot testing (insta) | Wait for output format to stabilize |
| 11 | Manifest as AI context artifact | Conditional on JSON output |
| 12 | Tests as solutions library | Philosophy, not actionable step |
| 13 | exec as task runner | Different purpose than justfile |
| 14 | Trace instrumentation | Over-engineering for shell-out wrapper |
| 15 | Manifest-encoded quality gates | Pre-commit hooks cover this |
| 16 | Shell environment injection | Product feature |
| 17 | Cross-worktree awareness hook | Complex, solves unreported problem |
| 18 | Unified cross-repo git log | Product feature |
| 19 | Positional-arg ambiguity fix | Valid but product quality, not CE bootstrap |

## Session Log
- 2026-03-24: Initial ideation -- 25 candidates generated across 5 frames, 6 survived filtering
