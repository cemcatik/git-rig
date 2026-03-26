---
title: "feat: Add --from flag to create for cloning rigs"
type: feat
status: completed
date: 2026-03-26
origin: docs/brainstorms/2026-03-26-clone-rig-requirements.md
---

# feat: Add `--from` flag to `create` for cloning rigs

## Overview

Extend the existing `create` command with `--from <source-rig>` to duplicate a rig's repo configuration into a new workspace. Each repo gets a fresh worktree with a new `rig/<new-name>` branch, inheriting upstream and remote config from the source.

## Problem Statement / Motivation

When working in multi-repo workspaces, parallel workstreams on the same set of repos require manually creating a new rig and re-adding each repo. This is tedious and error-prone. A single command to clone a rig eliminates this friction. (see origin: `docs/brainstorms/2026-03-26-clone-rig-requirements.md`)

## Proposed Solution

Add `--from` as an optional flag on `Create`. When provided:

1. **Pre-validate** all source repo paths (exist, are git repos)
2. **Create** the new rig directory + manifest
3. **For each source repo**: fetch, re-detect default branch, create worktree with `rig/<new-name>` branch from the effective upstream, record in manifest
4. **Report** any worktree creation failures at the end (continue-and-report pattern, matching `sync`/`exec` behavior)

Without `--from`, `create` behaves exactly as today.

### Pre-validation with `--skip`

By default, pre-validation failures (source path missing, not a git repo) are fatal — nothing is created and all invalid repos are listed in the error message.

With `--skip`, invalid repos are excluded and the rig is created with only the valid repos. A warning lists what was skipped.

### Post-validation failures

Failures during worktree creation (branch conflicts, network errors during fetch) always use continue-and-report — the successfully-added repos remain, failures are reported at the end. No flag needed; this matches the existing `sync` pattern.

## Technical Approach

### Extract shared worktree creation logic

The `add` command (lines 116-213 in `commands.rs`) contains the core logic for creating a worktree from a source repo. Extract the inner loop into a reusable helper:

```rust
// New helper in commands.rs
fn add_repo_to_rig(
    ws_dir: &Path,
    manifest: &mut Manifest,
    source_dir: &Path,
    repo_name: &str,
    branch: &str,
    remote: &str,
    upstream: Option<&str>,
    detach: bool,
) -> Result<()>
```

This helper handles: fetch, default branch detection, start point resolution, worktree creation (with branch existence checks), and `RepoEntry` construction. Both `add` and the new `create --from` call this helper.

### CLI changes (`src/main.rs`)

```rust
Commands::Create {
    name: String,
    #[arg(long, value_name = "SOURCE_RIG")]
    from: Option<String>,
    #[arg(long)]
    skip: bool,
}
```

### Implementation flow (`src/commands.rs`)

New function `create_from_source()` (called when `--from` is provided):

1. Resolve source rig via `workspace::resolve_workspace_from(start_dir, Some(from))`
2. Check target directory doesn't exist → `RigError::DirectoryAlreadyExists`
3. **Pre-validate**: iterate source manifest repos, check each `source` path exists and is a git repo. Collect failures.
   - If failures exist and `--skip` is false: error listing all invalid repos, create nothing
   - If failures exist and `--skip` is true: warn, filter out invalid repos, continue
   - If no valid repos remain after filtering: error (nothing to clone)
4. Create target directory + empty manifest with new name
5. For each valid source repo entry:
   - Determine branch: if source was detached → detach; otherwise `rig/<new-name>`
   - Call `add_repo_to_rig()` with source entry's `source`, `remote`, `upstream`
   - On failure: record error, continue to next repo
6. Save manifest
7. Print summary: N repos added, M failures (with details)

### Edge cases

- **Detached repos**: Preserved. If source `branch == "(detached)"`, create worktree as detached in the target. (see origin: scope boundaries — no copying of worktree state, but detached is a structural choice, not state)
- **Remote field**: Inherited per-repo from source manifest. Source repos may use non-default remotes.
- **`default_branch`**: Re-detected via `git::default_branch()` during clone, not copied from source manifest. Consistent with `add` behavior and avoids stale data.
- **Branch `rig/<new-name>` already exists**: Handled by the same three-way logic in `add` (check local, check remote, create new). This covers the case where a previous rig with the same name was destroyed with `--keep-branches`.
- **Empty source rig**: Succeeds, creates an empty rig. No warning needed — equivalent to `create` without `--from`.
- **Self-clone**: Caught by `DirectoryAlreadyExists` since the target name matches the source directory.

### Error types (`src/error.rs`)

New variant for pre-validation:

```rust
SourceReposInvalid {
    errors: Vec<(String, String)>,  // (repo_name, reason)
}
```

Display: lists each invalid repo and why (path not found, not a git repo).

## Acceptance Criteria

- [ ] `git rig create new-rig --from existing-rig` creates a new rig with all repos from the source
- [ ] Each repo gets branch `rig/<new-name>`, not the source branch name
- [ ] Upstream config is inherited per-repo from the source
- [ ] New worktrees start from `{remote}/{effective_upstream}`
- [ ] `create` without `--from` is unchanged
- [ ] Target already exists → `DirectoryAlreadyExists` error
- [ ] Source repo path invalid → error lists all invalid repos, nothing created
- [ ] `--skip` with invalid source repos → creates rig with valid repos only, warns about skipped
- [ ] Detached repos in source are cloned as detached
- [ ] Remote field is inherited per-repo
- [ ] `default_branch` is re-detected, not copied
- [ ] Post-validation failures (branch conflict, fetch error) → continue-and-report
- [ ] `status`, `sync`, `exec` work on the cloned rig immediately after creation

## Implementation Phases

### Phase 1: Extract `add_repo_to_rig` helper

- Refactor `add` in `commands.rs` to call the new helper
- No behavior change — all existing tests must pass
- Files: `src/commands.rs`

### Phase 2: Add `--from` and `--skip` to CLI

- Add fields to `Commands::Create` in `main.rs`
- Add dispatch logic
- Add `SourceReposInvalid` error variant
- Files: `src/main.rs`, `src/error.rs`

### Phase 3: Implement `create_from_source`

- Pre-validation loop
- Worktree creation loop with continue-and-report
- Summary output
- Files: `src/commands.rs`

### Phase 4: Tests

- E2E tests in `tests/cli_test.rs`:
  - Happy path: clone rig with multiple repos, verify manifest + worktrees + branches
  - Upstream inheritance: clone rig with upstream-configured repos, verify upstream in new manifest
  - Detached repos: clone rig with detached repo, verify detached in target
  - Source not found: error message
  - Target exists: `DirectoryAlreadyExists` error
  - Invalid source repo: pre-validation error listing all bad repos
  - `--skip` with invalid repos: partial clone succeeds, warns
  - Empty source: succeeds, creates empty rig
  - Branch collision: `rig/<new-name>` already exists, handled gracefully
  - Post-creation `status`/`sync` work correctly
- Unit tests in `src/workspace.rs` if any manifest logic changes
- Files: `tests/cli_test.rs`

### Phase 5: Documentation

- Update `README.md` with `--from` usage
- Update `CLAUDE.md` if needed
- Files: `README.md`

## Dependencies & Risks

- **Refactoring `add`** (Phase 1) is the riskiest step — must not break existing behavior. Run full test suite after extraction.
- **Branch collision handling** inherits complexity from `add`'s three-way branch logic. Reusing the helper avoids reimplementing this.
- **Fetch performance**: cloning a rig with many repos fetches each one. This is correct (same as running `add` N times) but could be slow. Not worth optimizing now.

## Sources & References

- **Origin document:** [docs/brainstorms/2026-03-26-clone-rig-requirements.md](docs/brainstorms/2026-03-26-clone-rig-requirements.md) — Key decisions: extend `create` (not new command), new branches per rig, inherit upstream, start from upstream tip.
- **Upstream config pattern:** [docs/solutions/upstream-config-with-fallback.md](docs/solutions/upstream-config-with-fallback.md) — `Option<T>` + `effective_*()` pattern for per-repo config.
- **Worktree recovery ladder:** [docs/solutions/worktree-recovery-ladder.md](docs/solutions/worktree-recovery-ladder.md) — reuse `remove_worktree_with_recovery()` if cleanup is needed.
- **Worktree prune ordering:** [docs/solutions/worktree-prune-ordering-bug.md](docs/solutions/worktree-prune-ordering-bug.md) — fs delete before `git worktree prune`.
- Similar implementation: `commands::add` at `src/commands.rs:61-214`
- Test fixture: `TestSandbox::create_workspace_with_repos` at `tests/common/mod.rs`
