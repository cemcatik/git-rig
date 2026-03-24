---
title: "feat: Add custom upstream branch per repo"
type: feat
status: active
date: 2026-03-24
origin: docs/brainstorms/2026-03-24-custom-upstream-branch-requirements.md
---

# feat: Add custom upstream branch per repo

## Overview

Allow each repo in a rig to track a custom remote branch for sync and status,
instead of always using the detected default branch. This enables rigs built
around feature/integration branches (e.g., an `api-migration` rig that
syncs `payments-api` against `origin/integration` instead of `origin/main`).

## Problem Statement / Motivation

When a team works on an integration effort that targets a long-lived feature
branch, `git rig sync` always rebases onto the default branch (main/master).
Users must manually rebase against the correct upstream, defeating the purpose
of the tool. (See origin: `docs/brainstorms/2026-03-24-custom-upstream-branch-requirements.md`)

## Proposed Solution

Add an optional `upstream` field to `RepoEntry`. When set, `sync` and `status`
use `{remote}/{upstream}` as the rebase/comparison target. When unset, behavior
is unchanged (`{remote}/{default_branch}`). The field is set via `--upstream` on
`git rig add` (both for new repos and to update existing ones).

## Technical Approach

### 1. Data Model — `RepoEntry` change

**File:** `src/workspace.rs:18-29`

Add to `RepoEntry`:

```rust
/// Remote branch to rebase onto during sync. When None, uses default_branch.
#[serde(default, skip_serializing_if = "Option::is_none")]
pub upstream: Option<String>,
```

- `skip_serializing_if` keeps `.rig.json` clean for repos that don't use the feature.
- Existing manifests deserialize with `upstream: None` — no migration needed.

Add a helper method on `RepoEntry`:

```rust
/// The branch that sync/status should compare against.
pub fn effective_upstream(&self) -> &str {
    self.upstream.as_deref().unwrap_or(&self.default_branch)
}
```

Add `find_repo_mut` to `Manifest` (needed for the update path):

```rust
pub fn find_repo_mut(&mut self, name: &str) -> Option<&mut RepoEntry> {
    self.repos.iter_mut().find(|r| r.name == name)
}
```

### 2. CLI — new flags on `add`

**File:** `src/main.rs:28-53`

Add to the `Add` variant:

```rust
/// Remote branch to sync against (default: repo's default branch)
#[arg(long, conflicts_with = "detach")]
upstream: Option<String>,

/// Clear a previously set upstream branch
#[arg(long, conflicts_with_all = ["detach", "upstream"])]
no_upstream: bool,
```

`conflicts_with` enforces:
- `--upstream` and `--detach` are mutually exclusive (detached repos skip sync, so an upstream is meaningless).
- `--upstream` and `--no-upstream` are mutually exclusive.

### 3. `add` command — new repo path

**File:** `src/commands.rs:52-178`

When adding a **new** repo with `--upstream`, two things change:

1. The worktree `start_point` uses the upstream branch instead of the default branch, so git tracking and `git log` reference the upstream ref:

```rust
let effective_start = upstream.unwrap_or(&default_branch);
let start_point = format!("{remote}/{effective_start}");
```

2. The `upstream` value is stored in the `RepoEntry`:

```rust
manifest.add_repo(RepoEntry {
    name: repo_name.clone(),
    source: source_dir,
    branch: recorded_branch,
    default_branch,
    remote: remote.to_string(),
    upstream: upstream.map(str::to_string),
});
```

`--no-upstream` on a new repo is a no-op (upstream is already `None`).

### 4. `add` command — update path for existing repos (R3, R4)

**File:** `src/commands.rs`, modify the `has_repo` check at lines 76-82

Current behavior: if `has_repo` is true, return `RepoAlreadyInRig`.

New behavior:

```rust
if manifest.has_repo(&repo_name) {
    // Update path: only --upstream or --no-upstream triggers an update
    if upstream.is_some() || no_upstream {
        let entry = manifest.find_repo_mut(&repo_name).unwrap();
        if no_upstream {
            entry.upstream = None;
            println!("{} Cleared upstream for '{}'", "ok".green(), repo_name.bold());
        } else {
            let branch = upstream.unwrap();
            entry.upstream = Some(branch.clone());
            println!(
                "{} Set upstream for '{}' to {}",
                "ok".green(), repo_name.bold(), branch.cyan()
            );
        }
        manifest.save(&ws_dir)?;
        return Ok(());
    }
    return Err(RigError::RepoAlreadyInRig { ... }.into());
}
```

**Design decisions** (from SpecFlow analysis):
- **Repo identity matching:** Match by derived name (basename of path), consistent with current `add` logic. Users who used `--name` originally must pass `--name` again. (See origin: deferred Q about name matching.)
- **Other flags rejected in update path:** When updating, only `--upstream` / `--no-upstream` are meaningful. If `--branch`, `--remote`, or `--detach` are also passed alongside an existing repo, the early return means they're simply ignored (the update path returns before reaching worktree creation). This is acceptable — the update path does nothing with those flags. A future enhancement could warn, but it's not needed now.

### 5. `sync` command — use effective upstream (R6)

**File:** `src/commands.rs:620`

Replace:
```rust
git::rebase(&worktree_path, &repo.default_branch, &repo.remote)
```

With:
```rust
let upstream = repo.effective_upstream();
git::rebase(&worktree_path, upstream, &repo.remote)
```

Same substitution for the ahead/behind call at line 624-625:
```rust
git::ahead_behind(&worktree_path, &current, upstream, &repo.remote)
```

**Sync output:** When upstream differs from default_branch, include it in the output:
```
ok  payments-api abc1234 -> def5678 (upstream: integration)
```

### 6. `status` command — use effective upstream (R5)

**File:** `src/commands.rs:480-481`

Replace `&repo.default_branch` with `repo.effective_upstream()` in the `ahead_behind` call.

**Status output:** When upstream differs from default_branch, show it:
```
payments-api on rig/api-migration +2 -3 (vs integration)
```

This addresses the SpecFlow concern about ambiguous ahead/behind numbers.

### 7. `list` command — show upstream when set

**File:** `src/commands.rs:446-448`

When upstream is `Some`, show it alongside the branch:
```
api-migration (3 repos)
  payments-api on rig/api-migration -> integration
  shared-models on rig/api-migration
```

### 8. `refresh` command — no change (R7)

`refresh` only updates `default_branch`. The `upstream` field is user-chosen and must not be modified by refresh. No code change needed — confirmed by inspection.

## System-Wide Impact

- **`destroy` / `remove`:** Not affected. They don't use `default_branch` for their core logic (only for branch deletion). The `upstream` field is metadata for sync/status only.
- **`exec`:** Not affected. Runs arbitrary commands in worktree directories; no branch logic.
- **Error propagation:** If the upstream branch doesn't exist on the remote, `git rebase origin/nonexistent` fails. The existing sync error handling (rebase abort + continue) handles this correctly, though the abort will warn since there's nothing to abort. The error message from git (`fatal: invalid upstream`) is sufficiently clear.
- **`ahead_behind` silent failure:** `git::ahead_behind()` returns `(0, 0)` on any error (known behavior from `docs/solutions/git-rs-review-findings.md`). If the upstream ref doesn't exist, status will show 0/0 rather than an error. This is pre-existing behavior, not introduced by this change.

## Acceptance Criteria

- [ ] `git rig add <path> --upstream integration` adds repo with custom upstream stored in `.rig.json`
- [ ] `git rig add <path> --upstream integration` for existing repo updates the upstream field
- [ ] `git rig add <path> --no-upstream` clears a previously set upstream
- [ ] `git rig add <path>` for existing repo (without --upstream) still errors with "already in rig"
- [ ] `git rig add <path> --upstream X --detach` is rejected by clap
- [ ] `git rig sync` rebases onto `{remote}/{upstream}` when upstream is set
- [ ] `git rig sync` rebases onto `{remote}/{default_branch}` when upstream is not set (backward compat)
- [ ] `git rig status` shows ahead/behind relative to effective upstream
- [ ] `git rig status` indicates the upstream name when it differs from default
- [ ] `git rig list` shows upstream target when set
- [ ] `git rig refresh` does not modify the upstream field
- [ ] Existing `.rig.json` files without `upstream` field load correctly (upstream = None)
- [ ] `.rig.json` files with upstream set serialize the field; without upstream, the field is omitted

## Implementation Sequence

### Phase 1: Data model + manifest (no CLI changes yet)

1. Add `upstream: Option<String>` to `RepoEntry` with serde annotations
2. Add `effective_upstream()` method to `RepoEntry`
3. Add `find_repo_mut()` to `Manifest`
4. Update `make_repo_entry` in workspace.rs tests and `common/mod.rs` test helper
5. Add unit tests: serde round-trip with/without upstream, effective_upstream logic

### Phase 2: CLI flags + add command

6. Add `--upstream` and `--no-upstream` flags to clap `Add` variant
7. Modify `add` command: pass upstream through for new repos
8. Modify `add` command: implement update path for existing repos
9. Add CLI tests: add with --upstream, update upstream, clear upstream, conflict with --detach

### Phase 3: sync + status + list

10. Modify `sync` to use `effective_upstream()`
11. Modify `status` to use `effective_upstream()` and show upstream name
12. Modify `list` to show upstream when set
13. Add integration tests: sync against custom upstream, status output with custom upstream

## Success Metrics

- All existing tests pass unchanged (backward compatibility).
- New tests cover: serde with/without upstream, add+update+clear flows, sync against custom upstream, status output.

## Dependencies & Risks

- **No new crate dependencies.** All changes use existing serde, clap, and git plumbing.
- **Risk: silent (0, 0) from ahead_behind.** Pre-existing behavior — if upstream doesn't exist on remote, status shows 0/0 instead of an error. Not new, but worth noting.

## Sources & References

- **Origin document:** [docs/brainstorms/2026-03-24-custom-upstream-branch-requirements.md](docs/brainstorms/2026-03-24-custom-upstream-branch-requirements.md) — Key decisions: reuse `add` for updates (no new command), optional field with serde default, status follows sync target.
- **Existing pattern:** `RepoEntry.remote` field with `#[serde(default = "default_remote")]` — `src/workspace.rs:27-33`
- **Known risk:** `ahead_behind()` swallows errors — `docs/solutions/git-rs-review-findings.md`
- **SpecFlow analysis:** Identified `find_repo_mut` gap, `--detach` conflict, update-path flag scoping, and output clarity needs.
