# git-rig — Git Worktree Workspace Manager

CLI tool for managing multi-repo workspaces using git worktrees.

## Build & Run

```bash
just check                     # fmt + clippy + deny + test (recommended)
just test                      # run all tests
just cross-check               # verify all release targets compile (no linker needed)
just install                   # install to ~/.cargo/bin/git-rig
cargo build                    # debug build
cargo build --release          # release build
```

## Architecture

Single-binary Rust CLI. Seven source files:

- `src/main.rs` — CLI definition (clap derive), dispatch
- `src/commands.rs` — Command implementations (create, add, remove, destroy, list, status, sync, refresh, exec, doctor, completions)
- `src/drift.rs` — Manifest/reality drift detection: checks worktree existence, branch match, and source repo accessibility before commands run
- `src/provision.rs` — `.riginclude` file parsing, pattern matching, and file provisioning (copy/symlink)
- `src/workspace.rs` — Manifest types (`.rig.json`), workspace resolution from CWD
- `src/git.rs` — Git operations (shells out to `git`, not libgit2)
- `src/error.rs` — `RigError` enum (structured errors via `thiserror`)

## Git Hooks

A pre-commit hook is auto-installed into `.git/hooks/` on the first `cargo build`/`cargo test` via `build.rs`. The hook source lives in `hooks/pre-commit` and is embedded at compile time with `include_str!`. The build script skips installation in CI (`$CI` set) or non-git environments (no `.git/` directory). To pick up changes to `hooks/pre-commit`, delete `.git/hooks/pre-commit` and rebuild.

## Key Design Decisions

- **Shells out to `git`** rather than using `git2` crate — worktree support in libgit2 is incomplete, and raw git gives better error messages.
- **`.rig.json` manifest** in each workspace root tracks repos, branches, remote, and optional upstream. Commands that take an optional workspace name resolve it by walking up from CWD to find this file.
- **`add`/`remove`/`status`/`sync`** infer workspace from CWD; `create`/`destroy` require explicit name.
- **`create --from`** clones a rig by creating new worktrees for each source repo. Pre-validates source paths; `--skip` excludes invalid repos instead of failing. Post-validation failures (branch conflicts, fetch errors) use continue-and-report. Detached repos stay detached. Upstream and remote config are inherited per-repo.
- **`add` doubles as update** — re-running `add` with `--upstream` on an existing repo updates the upstream field instead of erroring. `--no-upstream` clears it.
- **Default branch naming**: `rig/<workspace-name>` when `--branch` is not specified.
- **`sync` conflict strategy**: fetch + rebase onto the effective upstream (custom if set, otherwise default branch), abort on conflict (don't leave repo in broken state). `--stash` flag for auto-stashing dirty worktrees. `--repo` flag filters to specific repos (same pattern as `exec`).
- **Branch conflict detection**: When `add` fails because a branch is already checked out, the error message includes the path of the worktree that holds the conflicting branch (via `git worktree list --porcelain` parsing).
- **Shell completions**: `git rig completions <shell>` generates completion scripts for bash, zsh, fish, and PowerShell via `clap_complete`.
- **`doctor` command**: Two-tier health check — environment (git on PATH, git version >= 2.30) then per-repo (reuses `check_drift()` for worktree/source/branch checks, adds origin/HEAD, remote reachability, upstream branch existence). Works outside a rig (env checks only) or inside (full checks). PASS/WARN/FAIL output with copy-paste fix commands. Exit 1 on any issue. Environment failures short-circuit per-repo checks.
- **Optional per-repo config pattern**: new fields use `Option<T>` with `#[serde(default, skip_serializing_if = "Option::is_none")]` and an `effective_*()` method for fallback logic. See `docs/solutions/upstream-config-with-fallback.md`.
- **Manifest/reality drift detection**: Before `status`, `sync`, `exec`, and `refresh` run, an upfront pass checks every repo for: missing worktree, missing source repo, branch mismatch (manifest says one branch, worktree is on another), and unexpected detached HEAD. Drift is reported as `DRIFT`-prefixed warnings before command output. `sync` skips all drifted repos (prevents rebasing the wrong branch). `exec` skips only physically unavailable worktrees. `refresh` skips only missing source repos. `status` warns but displays everything. The check never errors — failures are absorbed as `WorktreeUnreachable` drift entries. The `DriftReport` caches `current_branch` results so `status` avoids redundant git subprocess calls. See `docs/brainstorms/2026-04-01-manifest-drift-detection-requirements.md` for design decisions.
- **`.riginclude` local file provisioning**: Per-repo file (`.gitignore`-style patterns) listing local files to copy into new worktrees. Typically gitignored for personal use, but teams can commit it for shared patterns. On `add`, files come from the base clone; on `create --from`, from the source rig's worktrees. `.riginclude` itself is always copied (self-propagating). Copy by default (`--link` for symlinks). Existing files skipped with warning (`--force-provision` to overwrite). `--no-provision` skips entirely. Provisioning flags on `create` require `--from`. Provisioning failures are warnings, not fatal errors — this is a deliberate departure from the partial-failure-must-return-error pattern since file copying is auxiliary to workspace creation.

## Testing

```bash
just check                          # fmt + clippy + test
just test                           # all tests
just test-unit                      # unit tests — manifest ops, workspace resolution
just test-integration               # integration tests — git operations against real repos
just test-e2e                       # E2E tests — full CLI commands via assert_cmd
just coverage                       # generate lcov coverage report
```

Test structure:
- `src/workspace.rs` (inline `#[cfg(test)]`) — Manifest CRUD, save/load roundtrip, migration, serde defaults, upstream config, workspace resolution via `_from` variants
- `tests/git_test.rs` — Branch detection, worktree add/remove, dirty checks, ahead/behind, stash, rebase
- `tests/cli_test.rs` — All subcommands end-to-end via `assert_cmd`, including upstream set/update/clear flows, drift detection (branch mismatch, missing worktree/source, detached HEAD, --repo filter scoping), and doctor (env checks, per-repo health, missing source/worktree, branch mismatch, origin/HEAD, remote reachability, upstream existence)
- `tests/common/mod.rs` — `TestSandbox` fixture: creates temp dirs with bare+clone repos, worktrees, and workspaces

Each test creates its own `TestSandbox` (temp dir) — no shared state, no CWD mutation. The `_from` variants (`resolve_workspace_from`, `resolve_base_dir_from`, `create_from`, `destroy_from`) accept a start directory so tests avoid `chdir`.

## Gotchas

- Git worktrees require that a branch is checked out in only one worktree at a time. If `git rig add` fails with "already checked out", the error message now tells you which worktree has the branch.
- `default_branch()` detection requires `origin/HEAD` to be set (done by `git clone`). For repos created with `git init`, run: `git remote set-head origin <branch>`.
- `git rig destroy` force-removes worktrees (even dirty ones). `git rig remove` does not — it will fail on dirty worktrees.
- `--upstream` sets the branch that `sync` rebases onto **and** the starting point for the worktree. The worktree is created from `{remote}/{upstream}`, so git tracking and `git log` show the upstream ref. The upstream branch must exist on the remote at add time. If it's later deleted, `sync` will fail with a git error.
- **Cross-platform: the project ships Windows builds via CI.** Never use `std::os::unix::*` without `#[cfg(unix)]` guards. The compiler won't warn you locally — it only fails on the Windows CI runner. When platform-specific code is needed, use a thin abstraction (see `symlink_or_copy` in `provision.rs`). See `docs/solutions/cross-platform-symlink-fallback.md`.
