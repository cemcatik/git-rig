---
title: "Riginclude local file provisioning for worktrees"
date: 2026-04-01
category: feature-implementation
tags: [rust, file-provisioning, gitignore-patterns, worktree, code-review, ignore-crate]
severity: medium
module: provision
root_cause: "Git worktrees lack local-only files (.env, IDE config) that are gitignored in the source checkout, requiring an explicit copy mechanism triggered by a .riginclude manifest with gitignore-style pattern semantics"
---

# `.riginclude` Local File Provisioning

## Problem

Git worktrees are created from the bare repository structure — they get tracked files only. Gitignored local files (`.env`, `.env.local`, `.vscode/settings.json`, tool overrides) are never carried into new worktrees. Every time a developer ran `git rig add` or `git rig create --from`, they had to manually remember which local files existed in the source and copy them over. This was error-prone, tedious, and invisible — you'd only discover a missing `.env` when something broke at runtime.

## Solution

A new `src/provision.rs` module reads a `.riginclude` file (gitignore-style patterns) from the source directory and copies matching files into the target worktree. It hooks into the existing `add_repo_to_rig()` helper so both `add` and `create --from` paths are unified.

### Key patterns

#### 1. Directory matching requires manual tracking

The `ignore` crate's `Gitignore::matched()` checks individual paths but does **not** automatically include children of matched directories. If a user writes `.vscode/` in `.riginclude`, the matcher reports `.vscode/` as matching but `.vscode/settings.json` does not match.

The workaround tracks matched directories in a `Vec` and checks children against it:

```rust
let mut matched_dirs: Vec<PathBuf> = Vec::new();

for entry in WalkDir::new(source_dir).into_iter().filter_map(|e| e.ok()) {
    let is_dir = entry.file_type().is_dir();
    let under_matched_dir = matched_dirs.iter().any(|d| rel_path.starts_with(d));

    if is_dir {
        if !under_matched_dir && matcher.matched(rel_path, true).is_ignore() {
            matched_dirs.push(rel_path.to_path_buf());
        }
        continue;
    }

    // Include file if it matches directly or is under a matched directory
    if !under_matched_dir && !matcher.matched(rel_path, false).is_ignore() {
        continue;
    }
    // ... copy the file
}
```

Key details:
- Directories are tracked, not copied. Only files under them are copied.
- `WalkDir` traverses depth-first, so parents are seen before children.
- The `!under_matched_dir` guard avoids redundant entries for nested dirs.

#### 2. Path containment must use `match`, not `if let Ok`

Security checks that prevent path traversal (e.g., symlinks escaping the repo) must handle the error case explicitly. Using `if let Ok(...)` silently falls through on failure, bypassing the check:

```rust
// WRONG: broken symlinks bypass the containment check
if let Ok(canonical_file) = path.canonicalize()
    && !canonical_file.starts_with(&canonical_source)
{
    continue;
}

// RIGHT: unverifiable paths are skipped
match path.canonicalize() {
    Ok(canonical_file) if canonical_file.starts_with(&canonical_source) => {}
    _ => continue,
}
```

#### 3. Use `symlink_metadata` for existence checks, not `exists()`

`path.exists()` follows symlinks — it returns `false` for dangling symlinks. This caused broken symlinks to bypass the "skip if exists" guard and get silently replaced even without `--force`:

```rust
// WRONG: broken symlinks appear as "not existing"
if target_file.exists() && !opts.force { ... }

// RIGHT: catches dangling symlinks too
if target_file.symlink_metadata().is_ok() && !opts.force { ... }
```

#### 4. `Option<T>` for skip-or-provision (not a separate flags struct)

Initially, a `ProvisionFlags { no_provision, link, force }` struct was created alongside a `ProvisionOpts { link, force }` struct. Code review caught that these were near-identical with boilerplate conversion at every call site. The fix: use `Option<ProvisionOpts>` where `None` means skip provisioning entirely:

```rust
// main.rs: construct at the CLI boundary
let provision = if no_provision {
    None
} else {
    Some(ProvisionOpts { force: force_provision, link })
};

// commands.rs: chained let pattern skips all three None cases
if let Some(prov_source) = provision_source
    && let Some(opts) = provision_opts
    && let Some(report) = provision::provision_files(prov_source, &worktree_path, opts)
{
    provision::print_provision_report(&report);
}
```

#### 5. Provisioning failures are warnings, not errors

This is a **deliberate departure** from the codebase's established "partial failure must return error" pattern (see `docs/solutions/partial-failure-must-return-error.md`). The rationale:

- The worktree is the primary artifact; provisioning is supplementary
- A missing `.env` doesn't invalidate the worktree
- The user can manually copy files that failed
- The function returns `Option<ProvisionReport>` (not `Result`), making it structurally impossible to `?`-propagate a provisioning failure

#### 6. Flag naming: qualify when semantics differ across commands

`--force` already existed on `remove`/`destroy` (meaning: force-remove dirty worktree). Adding `--force` on `add`/`create` with a different meaning (overwrite provisioned files) was caught by code review. The fix: `--force-provision` for the provisioning-specific flag, reserving `--force` for destructive worktree operations.

## Prevention: Checklist for Future Features

When adding features that involve file operations, third-party crates, or CLI flags:

1. **Crate behavior**: Write a characterization test for the specific third-party API behavior you depend on before building on it. The `ignore` crate's directory matching semantics were a surprise.

2. **Security checks**: Every security-critical code path must use `match` with explicit deny/error on failure. No silent fallthrough via `if let Ok(...)`.

3. **Symlinks**: Use `symlink_metadata().is_ok()` instead of `exists()` when the question is "is something at this path?" — `exists()` follows symlinks and returns `false` for broken ones.

4. **Struct duplication**: Only introduce a second options struct if the `From` impl does more than field-by-field copy. Otherwise use one type.

5. **Flag naming**: When adding a flag that shares a name with an existing one, grep for all existing usages. If the meaning differs, qualify the name (e.g., `--force-provision`).

6. **Flag dependencies**: Use clap's `requires` attribute for flags that are meaningless without another flag (e.g., provisioning flags on `create` require `--from`).

## Related Documentation

- `docs/solutions/partial-failure-must-return-error.md` — The pattern this feature deliberately departs from. Provisioning is a justified exception because file copying is auxiliary to workspace creation.
- `docs/solutions/upstream-config-with-fallback.md` — The `Option<T>` + `--no-<field>` CLI flag pattern that `Option<ProvisionOpts>` + `--no-provision` follows.
- `docs/solutions/git-rs-review-findings.md` — UTF-8 path assumption (Finding #2) applies to provisioning's path handling.
