---
title: "ce:review findings — git.rs hidden contracts"
category: code-review
tags: [git, review, contracts, risk]
date: 2026-03-24
trigger: modifying git.rs or adding new git shell-outs
---

# ce:review Findings: git.rs Hidden Contracts and Risks

## Overview

`src/git.rs` (263 lines) is the sole interface between git-rig and git. Every
command passes through one of three helper functions: `git_output`, `git_run`,
or `git_quiet`. This review surfaces the implicit contracts and risk areas.

## Hidden Contracts

### 1. Git Version Requirements (Undocumented)

The codebase implicitly requires **git 2.30+** but never checks:

| Feature | Minimum Git Version | Used In |
|---------|-------------------|---------|
| `branch --show-current` | 2.22 (Jun 2019) | `current_branch()` |
| `stash push --include-untracked` | 2.16 (Jan 2018) | `stash_push()` |
| `worktree repair` | 2.30 (Dec 2020) | `worktree_repair()` |

**Risk**: A user with git < 2.30 will hit confusing "unknown subcommand" errors
only when they try `remove` or `destroy` on a moved worktree — a rare path
that makes this hard to diagnose.

**Recommendation**: Add a `check_git_version()` function called once at startup
or on first git operation.

### 2. UTF-8 Path Assumption

`path_str()` (line 112) requires all paths to be valid UTF-8:

```rust
fn path_str(p: &Path) -> Result<&str> {
    p.to_str().ok_or_else(|| anyhow!("non-UTF8 path"))
}
```

This is used by all worktree operations. On Linux, filenames can contain
arbitrary bytes. A user with a non-UTF-8 directory name cannot use git-rig
for worktrees.

**Risk**: Low on macOS (enforces UTF-8 in HFS+), moderate on Linux.

**Recommendation**: Accept for now. Document as a known limitation if reported.

### 3. Git Output is Assumed UTF-8

`git_output()` (line 23) uses `String::from_utf8_lossy`, which replaces
invalid bytes with the Unicode replacement character. This means:

- Branch names containing non-UTF-8 bytes will be silently corrupted
- The corrupted string will be written to `.rig.json`
- Future operations on that branch name will fail

**Risk**: Very low (git itself discourages non-ASCII branch names).

## Risk Areas

### 4. Silent Error Swallowing in `ahead_behind()`

```rust
pub fn ahead_behind(...) -> (u32, u32) {
    match git_output(...) {
        Ok(output) => { /* parse */ },
        Err(_) => (0, 0),  // <-- silently returns "in sync"
    }
}
```

Returns `(0, 0)` on any error — indistinguishable from "branch is in sync."
This means:
- Network issues → silently shows as up-to-date
- Missing remote branch → silently shows as up-to-date
- Parse failure → silently shows as up-to-date

The `unwrap_or(0)` on the inner parse has the same effect.

**Risk**: Medium. `status` is an informational command, so wrong data doesn't
cause destructive actions, but it can mislead users into thinking they're
synced when they're not.

**Recommendation**: Return `Option<(u32, u32)>` or `Result<(u32, u32)>` and
let `status` display "unknown" instead of fabricating zeros.

### 5. `git_run()` Loses Error Detail

`git_run()` uses `.status()` which inherits the parent's stdout/stderr. On
error, the message only includes the exit code:

```rust
Err(anyhow!("git {:?} exited with code {:?}", args, status.code()))
```

Compare with `git_output()` which captures stderr and includes it in the error.
This means interactive commands (like `worktree remove`) show git's error to
the terminal but the programmatic error lacks the diagnostic detail.

**Risk**: Low for users (they see git's output). Higher for agents that only
see the error message.

### 6. Stash Detection via String Comparison

`stash_push()` detects whether anything was stashed by comparing the full
`stash list` output before and after:

```rust
let before = git_output(repo_dir, &["stash", "list"])?;
// ... stash push ...
let after = git_output(repo_dir, &["stash", "list"])?;
Ok(before != after)
```

This works correctly but has a theoretical race condition if another process
modifies the stash between the two `stash list` calls. In practice, this is
unlikely since git-rig operates on dedicated worktree directories.

**Risk**: Negligible. The approach is pragmatic and correct for the use case.

### 7. Force Delete in `delete_branch()`

```rust
pub fn delete_branch(repo_dir: &Path, branch: &str) -> Result<()> {
    git_quiet(repo_dir, &["branch", "-D", branch])
}
```

Uses `-D` (force) which deletes branches even if they haven't been merged
upstream. The comment documents this as intentional. The callers (`remove` and
`destroy`) are the right place to gate this — `remove` asks for `--keep-branch`
opt-out, `destroy` asks for `--keep-branches`.

**Risk**: Low — the API is correct; the safeguards live at the command level.

## Summary

| Finding | Severity | Action |
|---------|----------|--------|
| Git version undocumented (2.30+ required) | Medium | Add version check |
| `ahead_behind` swallows errors | Medium | Return Option/Result |
| `git_run` loses stderr in error messages | Low | Consider capturing stderr |
| UTF-8 path assumption | Low | Document as limitation |
| UTF-8 output assumption | Very Low | Accept |
| Stash detection race | Negligible | Accept |
| Force delete in `delete_branch` | Documented | Accept |
