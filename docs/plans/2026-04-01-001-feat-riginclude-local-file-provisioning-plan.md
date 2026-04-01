---
title: "feat: Add .riginclude for local file provisioning"
type: feat
status: completed
date: 2026-04-01
origin: docs/brainstorms/2026-04-01-riginclude-local-file-provisioning-requirements.md
---

# feat: Add `.riginclude` for local file provisioning

## Overview

Add per-repo local file provisioning to `git rig add` and `git rig create --from`. Each repo can have a `.riginclude` file (gitignored, gitignore-style glob patterns) listing local files to copy into new worktrees automatically. This eliminates the tedious manual step of remembering and copying `.env` files, IDE configs, and tool overrides every time a new worktree is created.

## Problem Statement / Motivation

Developers maintain local, gitignored files per repository that are essential for working but absent from new worktrees. Today users must manually remember and copy these files after every `git rig add` or `git rig create --from`. As rig-based workflows become the primary development flow, this friction compounds. (see origin: `docs/brainstorms/2026-04-01-riginclude-local-file-provisioning-requirements.md`)

## Proposed Solution

A new `src/provision.rs` module handles reading `.riginclude`, matching files, and copying/symlinking them into target worktrees. The existing `add_repo_to_rig()` helper gains a call to provisioning after worktree creation, keeping both `add` and `create --from` paths unified.

### How it works

1. After a worktree is created in `add_repo_to_rig()`, check the **provision source** directory for a `.riginclude` file
2. If found, parse it as gitignore-style patterns (using the `ignore` crate)
3. Walk the source directory, collect files matching the patterns
4. Copy `.riginclude` itself first (self-propagating, R2)
5. Copy (or symlink with `--link`) each matched file to the target worktree, preserving relative directory structure
6. Report results inline with existing output

### Source resolution

- **`git rig add`**: provision source = the base clone (`source` path in RepoEntry, already passed as `source_dir`)
- **`git rig create --from`**: provision source = the source rig's worktree for each repo (`source_ws_dir.join(&entry.name)`)

### New CLI flags

| Flag | Commands | Effect |
|------|----------|--------|
| `--no-provision` | `add`, `create` | Skip provisioning entirely |
| `--link` | `add`, `create` | Symlink files instead of copying |
| `--force` | `add`, `create` | Overwrite existing files in target (default: skip with warning) |

`--force` and `--link` are silently ignored when `--no-provision` is set — no `conflicts_with` needed.

## Technical Approach

### Architecture

New file `src/provision.rs` containing:

```rust
// src/provision.rs

use std::path::Path;

/// Options controlling provisioning behavior.
pub struct ProvisionOpts {
    pub force: bool,
    pub link: bool,
}

/// Result of provisioning a single file.
pub enum FileResult {
    Copied { rel_path: String },
    Linked { rel_path: String },
    Skipped { rel_path: String, reason: String },
    Failed { rel_path: String, error: String },
}

/// Result of provisioning a repo.
pub struct ProvisionReport {
    pub files: Vec<FileResult>,
    pub unmatched_patterns: Vec<String>,
}

/// Read .riginclude from source, copy/link matching files into target.
/// Returns None if no .riginclude exists in source.
pub fn provision_files(
    source_dir: &Path,
    target_dir: &Path,
    opts: &ProvisionOpts,
) -> Option<ProvisionReport> {
    // 1. Check for .riginclude in source_dir
    // 2. Parse patterns using ignore::gitignore::GitignoreBuilder
    // 3. Copy .riginclude itself to target (R2)
    // 4. Walk source_dir with walkdir, match each file
    // 5. For each match: copy/link/skip based on opts
    // 6. Collect and return results
    todo!()
}
```

### Integration point in `add_repo_to_rig()`

Currently at `src/commands.rs` line ~275, the function signature is already large (`#[allow(clippy::too_many_arguments)]`). Add provisioning via a struct parameter:

```rust
fn add_repo_to_rig(
    source_dir: &Path,
    ws_dir: &Path,
    manifest: &mut Manifest,
    repo_name: &str,
    branch_name: &str,
    remote: &str,
    detach: bool,
    upstream: Option<&str>,
    provision_source: Option<&Path>,  // NEW: None = skip provisioning
    provision_opts: &ProvisionOpts,   // NEW: force/link flags
) -> Result<()> {
    // ... existing worktree creation logic ...

    // NEW: After worktree creation, before manifest save
    if let Some(prov_source) = provision_source {
        if let Some(report) = provision::provision_files(prov_source, &worktree_path, provision_opts) {
            print_provision_report(&report);
        }
    }

    manifest.add_repo(/* ... */);
    manifest.save(ws_dir)?;
    Ok(())
}
```

### Callers

**`add()`** (commands.rs line ~184):
```rust
let provision_source = if no_provision { None } else { Some(source_dir.as_path()) };
add_repo_to_rig(source_dir, ..., provision_source, &provision_opts)?;
```

**`create_from_source()`** (commands.rs line ~52):
```rust
for entry in &valid_entries {
    let provision_source = if no_provision {
        None
    } else {
        // R4: source is source rig's worktree, not base clone
        let worktree_path = source_ws_dir.join(&entry.name);
        if worktree_path.is_dir() { Some(worktree_path) } else { None }
    };
    add_repo_to_rig(&entry.source, ..., provision_source.as_deref(), &provision_opts)?;
}
```

### Pattern matching: `ignore` crate

The `ignore` crate's `gitignore` module provides a parser for gitignore-style files:

```rust
use ignore::gitignore::GitignoreBuilder;
use walkdir::WalkDir;

let mut builder = GitignoreBuilder::new(source_dir);
builder.add(source_dir.join(".riginclude"));
let matcher = builder.build()?;

for entry in WalkDir::new(source_dir).into_iter().filter_map(|e| e.ok()) {
    let path = entry.path();
    let rel = path.strip_prefix(source_dir)?;
    let is_dir = entry.file_type().is_dir();

    if matcher.matched(rel, is_dir).is_ignore() {
        // This file matches a .riginclude pattern — copy it
    }
}
```

This handles for free: `#` comments, blank lines, negation (`!`), trailing `/` for directory-only patterns, `**` for recursive matching, and all standard gitignore semantics. (see origin for decision against `.gitignore` cross-check)

**New dependencies**: `ignore` (which transitively brings `walkdir`, `globset`).

### Path containment

Resolved paths that escape the source repo root are silently skipped. After matching, canonicalize each file path and verify it starts with the canonicalized source root:

```rust
let canonical_source = source_dir.canonicalize()?;
let canonical_file = file_path.canonicalize()?;
if !canonical_file.starts_with(&canonical_source) {
    // Skip — path traversal outside repo root
    continue;
}
```

This is consistent with how `add()` already canonicalizes `source_dir` (commands.rs line ~196).

### Symlink behavior (`--link`)

Absolute symlinks, consistent with how git worktrees already use absolute paths to reference their source repos. The symlink target is the canonical source file path.

For source files that are themselves symlinks: `std::fs::copy` dereferences by default (copies contents). `--link` creates a symlink to the original source path (the symlink itself, not its target).

### Failure handling

Provisioning failures are **warnings, not fatal errors**. The worktree is the primary artifact of `add`; provisioning is supplementary convenience. This is a deliberate departure from the codebase's "partial failure returns error" pattern (documented in `docs/solutions/partial-failure-must-return-error.md`) because:
- The worktree was already created successfully
- The manifest entry should still be saved
- A provisioning failure (permissions, disk full) doesn't invalidate the worktree
- The user can manually copy the files that failed

Provisioning collects all results into `ProvisionReport` and prints warnings for failures/skips. The `add`/`create --from` command still exits successfully.

### Output format

Inline with existing output. Current `add` prints:

```
ok — added payments-api (branch: rig/my-rig)
```

With provisioning, append a summary:

```
ok — added payments-api (branch: rig/my-rig)
  provisioned: .env, .env.local, .vscode/settings.json (3 files)
```

With skips/warnings:

```
ok — added payments-api (branch: rig/my-rig)
  provisioned: .env, .vscode/settings.json (2 files)
  skipped: .env.local (already exists)
```

With failures:

```
ok — added payments-api (branch: rig/my-rig)
  provisioned: .env (1 file)
  warning: failed to copy .vscode/settings.json: permission denied
```

No provisioning (no `.riginclude` in source): no extra output.

## System-Wide Impact

- **Interaction graph**: `add_repo_to_rig()` gains provisioning call → `provision::provision_files()` reads filesystem + copies files. No callbacks, hooks, or observers involved.
- **Error propagation**: Provisioning errors are self-contained in `ProvisionReport`. They do not propagate as `Result::Err` from `add_repo_to_rig()`. The command exit code is unaffected by provisioning failures.
- **State lifecycle risks**: If the process is killed between worktree creation and manifest save, partially-provisioned files exist but the repo isn't in the manifest. A re-run of `add` hits the existing worktree recovery path (commands.rs line ~299) and re-provisions. No orphaned state.
- **API surface parity**: Only `add` and `create --from` are affected. `remove`, `destroy`, `sync`, `status`, `exec`, `list`, `refresh` are unchanged.
- **Integration test scenarios**: (1) `add` with `.riginclude` → files appear in worktree. (2) `create --from` with `.riginclude` in source rig → files appear in cloned rig's worktrees. (3) `add --no-provision` → no files copied even with `.riginclude` present. (4) `add` with existing files → skip with warning. (5) `add --force` → overwrite existing.

## Acceptance Criteria

### Functional Requirements

- [ ] `git rig add <repo>` reads `.riginclude` from the base clone and copies matching files into the new worktree
- [ ] `git rig create --from <rig>` reads `.riginclude` from each source rig worktree and copies matching files
- [ ] `.riginclude` itself is always copied from source (self-propagating)
- [ ] `.riginclude` supports gitignore-style patterns: globs, `**`, trailing `/`, `#` comments, `!` negation, blank lines
- [ ] Directory patterns (e.g., `.vscode/`) copy the entire directory tree recursively
- [ ] `--no-provision` skips all provisioning
- [ ] `--link` creates symlinks instead of copies (absolute paths)
- [ ] `--force` overwrites existing files (default: skip with warning)
- [ ] Missing source files / unmatched patterns are silently ignored
- [ ] Provisioning results are reported inline with existing output
- [ ] Provisioning failures are warnings, not fatal errors
- [ ] Resolved paths outside the source repo root are silently skipped

### Non-Functional Requirements

- [ ] No new `RigError` variants needed (provisioning uses warning output, not error returns)
- [ ] `ignore` and `walkdir` crate dependencies added to `Cargo.toml`

### Quality Gates

- [ ] Unit tests for `.riginclude` parsing and file matching in `src/provision.rs`
- [ ] E2E tests for `add` with `.riginclude`, `create --from` with `.riginclude`, `--no-provision`, `--force`, `--link`
- [ ] Partial-failure test: one file copies, one fails → warning output, worktree still added
- [ ] `just check` passes (fmt + clippy + deny + test)

## Implementation Phases

### Phase 1: Provisioning core (`src/provision.rs`)

New module with:
- `ProvisionOpts`, `FileResult`, `ProvisionReport` types
- `provision_files(source, target, opts) -> Option<ProvisionReport>` — reads `.riginclude`, walks source, copies/links matching files
- `print_provision_report(report)` — formats output
- Unit tests: pattern matching, directory recursion, path containment, skip-on-exist, force-overwrite, symlink mode, empty `.riginclude`

**Files**: `src/provision.rs` (new), `Cargo.toml` (add `ignore` dep)

### Phase 2: Integration with `add`

- Add `--no-provision`, `--link`, `--force` flags to `Add` in `src/main.rs`
- Thread flags through `add()` → `add_repo_to_rig()` in `src/commands.rs`
- Add `provision_source` and `provision_opts` parameters to `add_repo_to_rig()`
- Call `provision_files()` after worktree creation, before manifest save
- Wire `mod provision;` in `src/main.rs`

**Files**: `src/main.rs`, `src/commands.rs`

### Phase 3: Integration with `create --from`

- Add `--no-provision`, `--link`, `--force` flags to `Create` in `src/main.rs`
- Thread flags through `create()` → `create_from_source()` → `add_repo_to_rig()`
- Compute provision source as `source_ws_dir.join(&entry.name)` for each repo

**Files**: `src/main.rs`, `src/commands.rs`

### Phase 4: E2E tests

- `TestSandbox` helper: `create_riginclude(repo_name, patterns)` to write `.riginclude` files
- `TestSandbox` helper: `create_local_file(repo_name, path, content)` to create gitignored files in repos
- Tests:
  - `add_provisions_local_files` — happy path
  - `add_provisions_directory_recursively` — `.vscode/` pattern
  - `add_no_riginclude_no_provision` — no `.riginclude`, no output
  - `add_no_provision_flag_skips` — `--no-provision`
  - `add_skips_existing_files` — skip with warning
  - `add_force_overwrites_existing` — `--force`
  - `add_link_creates_symlinks` — `--link`
  - `add_riginclude_self_propagates` — `.riginclude` copied even if not listed
  - `create_from_provisions_from_source_rig` — `create --from` uses source rig worktrees
  - `create_from_no_provision` — `create --from --no-provision`
  - `add_provision_failure_is_warning` — partial failure doesn't fail the command

**Files**: `tests/cli_test.rs`, `tests/common/mod.rs`

## Dependencies & Risks

- **New crate dependency**: `ignore` (from ripgrep ecosystem, well-maintained, MIT licensed). Transitively brings `walkdir`, `globset`, `regex-automata`, `memchr`. These are all from the same ecosystem and widely used.
- **Parameter growth on `add_repo_to_rig`**: Already has `#[allow(clippy::too_many_arguments)]`. Two more parameters. Consider grouping into a struct in a follow-up refactor if it becomes unwieldy.
- **Low risk**: Feature is additive — repos without `.riginclude` are completely unaffected. No manifest schema changes. No breaking changes to existing commands.

## Sources & References

### Origin

- **Origin document:** [docs/brainstorms/2026-04-01-riginclude-local-file-provisioning-requirements.md](docs/brainstorms/2026-04-01-riginclude-local-file-provisioning-requirements.md) — Key decisions: per-repo declaration, gitignored `.riginclude`, copy-by-default, skip-by-default, self-propagating, no `.gitignore` cross-check.

### Internal References

- Optional per-repo config pattern: `docs/solutions/upstream-config-with-fallback.md` — CLI flag conventions (`--no-<field>`, `conflicts_with`)
- Partial-failure error handling: `docs/solutions/partial-failure-must-return-error.md` — deliberately not followed here (provisioning is warning-only)
- Worktree recovery: `docs/solutions/worktree-recovery-ladder.md` — helper extraction pattern
- UTF-8 path assumption: `docs/solutions/git-rs-review-findings.md` — all paths assumed UTF-8
- `add_repo_to_rig()`: `src/commands.rs:275` — insertion point
- `create_from_source()`: `src/commands.rs:52` — loop over entries
- CLI arg patterns: `src/main.rs:37` (Add), `src/main.rs:22` (Create)

### External References

- `ignore` crate: https://docs.rs/ignore — gitignore-compatible pattern matching
- `.worktreeinclude` convention: https://dev.to/satococoa/git-worktreeinclude-a-tiny-cli-for-safely-carrying-over-ignored-files-across-git-worktrees-5cdm — inspiration (git-rig takes a simpler approach)
