---
title: "Custom upstream branch per repo for sync and status"
category: feature-patterns
date: 2026-03-24
tags:
  - sync
  - upstream
  - rebase
  - manifest
  - serde
  - optional-config
  - backward-compat
components:
  - src/workspace.rs
  - src/commands.rs
  - src/main.rs
severity: medium
resolution_time_estimate: 4h
related:
  - docs/brainstorms/2026-03-24-custom-upstream-branch-requirements.md
  - docs/plans/2026-03-24-001-feat-custom-upstream-branch-per-repo-plan.md
  - docs/solutions/git-rs-review-findings.md
---

# Custom Upstream Branch Per Repo

## Problem

`git rig sync` always rebased each repo's worktree branch onto `{remote}/{default_branch}` (typically `origin/main`). Users with rigs targeting long-lived feature or integration branches had to manually rebase outside the tool, defeating the purpose of automated sync.

Similarly, `git rig status` showed ahead/behind counts relative to the default branch, which was misleading for repos targeting a different upstream.

## Root Cause

Both `sync` and `status` hardcoded the use of `repo.default_branch` as the rebase/comparison target:

```rust
// sync — always rebased onto default
git::rebase(&worktree_path, &repo.default_branch, &repo.remote)

// status — always compared against default
git::ahead_behind(&worktree_path, &branch, &repo.default_branch, &repo.remote)
```

There was no per-repo mechanism to override which remote branch to sync against.

## Solution

Added an optional `upstream` field to `RepoEntry` with a centralized fallback method. Four-point change:

### 1. Data model (`src/workspace.rs`)

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoEntry {
    // ... existing fields ...
    /// Remote branch to rebase onto during sync. When None, uses default_branch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upstream: Option<String>,
}

impl RepoEntry {
    pub fn effective_upstream(&self) -> &str {
        self.upstream.as_deref().unwrap_or(&self.default_branch)
    }
}
```

### 2. CLI flags (`src/main.rs`)

```rust
#[arg(long, conflicts_with = "detach")]
upstream: Option<String>,

#[arg(long, conflicts_with_all = ["detach", "upstream"])]
no_upstream: bool,
```

### 3. Add update path (`src/commands.rs`)

When `add` is called for an already-added repo with `--upstream` or `--no-upstream`, it updates the manifest entry instead of erroring:

```rust
if manifest.has_repo(&repo_name) {
    if upstream.is_some() || no_upstream {
        let entry = manifest.find_repo_mut(&repo_name).unwrap();
        if no_upstream {
            entry.upstream = None;
        } else {
            entry.upstream = Some(upstream.unwrap().to_string());
        }
        manifest.save(&ws_dir)?;
        return Ok(());
    }
    return Err(RigError::RepoAlreadyInRig { .. }.into());
}
```

### 4. Sync/status substitution

All consumers replaced `&repo.default_branch` with `repo.effective_upstream()`.

## Key Design Decisions

| Decision | Rationale |
|---|---|
| `serde(default, skip_serializing_if = "Option::is_none")` | Old manifests without the field deserialize to `None`. Manifests without a custom upstream stay clean (no `"upstream": null`). |
| `conflicts_with = "detach"` | Detached worktrees skip sync, so an upstream on them is nonsensical. Clap rejects at parse time. |
| `effective_upstream()` method | Single place for fallback logic. All consumers call one method instead of inlining the Option unwrap. |
| Reuse `add` for updates | Avoids subcommand proliferation. The `has_repo` guard already existed; the update path is a natural branch before the error. |
| `--no-upstream` flag | Explicit way to clear back to default, rather than requiring `--upstream main` (fragile if default branch changes). |

## Verification

196 tests pass (16 new). Key E2E tests:

- `sync_with_custom_upstream` — creates an `integration` branch on the remote, sets `--upstream integration`, runs sync, verifies the worktree contains the integration branch's content
- `status_shows_upstream_indicator` — creates upstream branch with extra commits, verifies behind count is computed against the custom upstream
- `sync_with_nonexistent_upstream_reports_error` — sets upstream to a branch that doesn't exist, verifies sync reports an error
- `add_upstream_update_existing_repo` / `add_no_upstream_clears_existing` — full None->Some->None lifecycle

## Prevention: Pattern for Future Per-Repo Config Fields

When adding a new optional per-repo field to the manifest:

1. **Always `Option<T>`** with `serde(default, skip_serializing_if = "Option::is_none")` for backward compat
2. **Always `effective_*()`** method on `RepoEntry` — single place for fallback logic, individually testable
3. **Always a reset flag** (`--no-<field>`) so users can undo without hand-editing `.rig.json`
4. **Always `conflicts_with`** for nonsensical combinations (detach + upstream, etc.)
5. **Test behavior, not output** — verify worktree state/manifest content, not just printed strings
6. **Update help text** — when a command gains new behavior, its `--help` description must reflect it

### Common Pitfalls

- **Variable shadowing in update paths**: local variables from the "add" path can shadow the "update" path. Only modify fields for which the user explicitly provided a flag.
- **Inconsistent output formats**: pick one format for the new field's display and use it consistently across `list`, `status`, and `sync`.
- **Help text going stale**: `cargo test` doesn't check help text accuracy. Consider a test that asserts `--help` output contains key phrases.
- **Manifest save on no-op**: if the user sets a field to its current value, consider skipping the save to avoid spurious diffs.

## Known Limitations

- **No upstream validation at set time**: the upstream branch is not checked against the remote when `--upstream` is passed. If it doesn't exist, `sync` will fail with a git error at rebase time. This was a deliberate design decision (see plan).
- **`ahead_behind()` silent failure**: if the upstream ref doesn't exist, `git::ahead_behind()` returns `(0, 0)` instead of an error. This is pre-existing behavior documented in `docs/solutions/git-rs-review-findings.md`.
