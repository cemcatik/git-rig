---
date: 2026-04-29
topic: rig-root-gitignore-management
---

# Rig Root `.gitignore` Management — Decision Record

## Problem Frame

When the rig root is itself a git repo (a common pattern for users who keep personal scripts/notes/docs alongside the rig, and may share the rig structure later), every `git rig add` produces a new untracked worktree directory in the rig root's `git status`. This is visual noise, and creates a small footgun: `git add .` in the rig root could pull in worktree contents in surprising ways (worktrees contain a `.git` *file*, not directory, so git doesn't auto-treat them as submodules).

The question: should `git rig` itself manage entries in the rig root's `.gitignore` to suppress this?

## Decision

**Do nothing.** git-rig will not detect, hint about, or modify the rig root's `.gitignore`. Users handle `.gitignore` themselves the same way they handle it for any other directory.

## Alternatives Considered

- **A. Hint only.** Print a one-time tip on `git rig add` when the rig root is a git repo, suggesting the user add `<repo>/` to `.gitignore`. Rejected: even the minimal version has hidden complexity (every-time vs. first-time, checking if entry already exists, where to store "first-time" state). Once you check whether an entry exists, you're 80% of the way to option B.
- **B. Append-only on `add`.** Write `<repo>/` to `.gitignore` on `add`; do nothing on `remove` (stale entries are harmless no-ops). Rejected: invasive for marginal benefit. Modifying user-owned files is a different kind of feature than worktree management.
- **C. Fully managed block.** Marker comments bound a region kept in sync with the manifest on every `add`/`remove`/`destroy`. Rejected: most code, most edge cases (user edits, missing markers, hand-removed entries), least proportionate to the underlying pain.

## Rationale

- **Pain is genuinely small.** Adding `<repo>/` to `.gitignore` is ~5 seconds of one-time work per repo, and only when the rig root happens to be git-managed (not the common case). Building any of A/B/C costs more than the lifetime cost of the manual workaround.
- **Scope coherence.** git-rig manages worktrees and a `.rig.json` manifest. Modifying user-owned `.gitignore` is editing git config — a category of intervention the project has so far avoided (the project shells out to `git` for *clarity*, not to become a git config layer).
- **Two sources of truth.** `.rig.json` already authoritatively lists the repos. A managed `.gitignore` block becomes a derived view that has to be kept in sync, which is the classic shape of drift bugs.
- **YAGNI on carrying cost.** Even option A has ongoing maintenance: detection logic, tests, conditional branches in the `add` flow. The simplest version that actually delivers value is zero.

## Scope Boundaries

- This decision applies to the rig root's `.gitignore` only. Per-repo `.riginclude` provisioning (an existing feature) is unrelated and unaffected.
- `.git/info/exclude` (local-only, per-clone) is also out of scope. Users wanting a personal-only ignore can use it themselves.

## When to Revisit

Reconsider if any of the following changes:
- Multiple users report friction around this in practice (not just theoretical).
- A high-churn workflow emerges where users routinely add/remove repos and the manual `.gitignore` step becomes a real cost.
- The project takes on related "manage user git config" responsibilities for other reasons, making this incremental rather than scope creep.
