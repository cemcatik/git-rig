---
title: "Cross-platform symlink with Windows copy fallback"
date: 2026-04-01
category: build-errors
tags: [rust, cross-platform, windows, symlink, cfg]
severity: medium
module: provision
root_cause: "Direct use of std::os::unix::fs::symlink fails to compile on Windows — the module is gated behind #[cfg(unix)]"
---

# Cross-Platform Symlink with Windows Copy Fallback

## Problem

The Windows release build failed with:

```
error[E0433]: failed to resolve: could not find `unix` in `os`
   --> src\provision.rs:206:24
    |
206 |         match std::os::unix::fs::symlink(&link_target, &target_file) {
    |                        ^^^^ could not find `unix` in `os`
```

## Root Cause

`std::os::unix` is conditionally compiled — it only exists on unix targets. Using it directly without `#[cfg(unix)]` breaks any non-unix build. The original code had platform guards (`#[cfg(unix)]` / `#[cfg(not(unix))]`) but they were removed during a code review that flagged the Windows branch as "dead code" since the project is primarily unix-focused.

The project does ship Windows builds via CI, so the code must compile on all targets.

## Solution

Replace the direct platform call with a thin abstraction that uses native symlinks on unix and falls back to `fs::copy` on other platforms:

```rust
/// Create a symlink, or copy if symlinks are not supported.
/// Returns `true` if a real symlink was created, `false` if it fell back to copy.
fn symlink_or_copy(original: &Path, link: &Path) -> std::io::Result<bool> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(original, link)?;
        Ok(true)
    }
    #[cfg(not(unix))]
    {
        fs::copy(original, link)?;
        Ok(false)
    }
}
```

The `bool` return lets the caller report accurately (`Linked` vs `Copied`) so the user isn't misled about what happened:

```rust
match symlink_or_copy(&link_target, &target_file) {
    Ok(true) => report.files.push(FileResult::Linked(rel_str)),
    Ok(false) => report.files.push(FileResult::Copied(rel_str)),
    Err(e) => report.files.push(FileResult::Failed { ... }),
}
```

Windows symlinks require `SeCreateSymbolicLinkPrivilege` (elevated permissions), so copying is the practical default there.

## Prevention

- **Never use `std::os::unix::*` or `std::os::windows::*` without `#[cfg]` guards.** The compiler won't warn you on your platform — it only fails on the other one.
- **When removing "dead code" flagged by review, check if the project ships builds for the platform that code serves.** A `#[cfg(not(unix))]` branch is dead in your local build but alive in CI.
- **Prefer a thin abstraction over inline `#[cfg]` blocks at the call site.** A `symlink_or_copy` helper centralizes the platform logic and makes the call site clean.

## Cross-References

- `docs/solutions/riginclude-local-file-provisioning.md` — The feature that introduced this code
