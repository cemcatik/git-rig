# git-rig — Git Worktree Workspace Manager

CLI tool for managing multi-repo workspaces using git worktrees.

## Build & Run

```bash
just check                     # fmt + clippy + test (recommended)
just test                      # run all tests
just install                   # install to ~/.cargo/bin/git-rig
cargo build                    # debug build
cargo build --release          # release build
```

## Architecture

Single-binary Rust CLI. Five source files:

- `src/main.rs` — CLI definition (clap derive), dispatch
- `src/commands.rs` — Command implementations (create, add, remove, destroy, list, status, sync, refresh, exec)
- `src/workspace.rs` — Manifest types (`.rig.json`), workspace resolution from CWD
- `src/git.rs` — Git operations (shells out to `git`, not libgit2)
- `src/error.rs` — `RigError` enum (structured errors via `thiserror`)

## Key Design Decisions

- **Shells out to `git`** rather than using `git2` crate — worktree support in libgit2 is incomplete, and raw git gives better error messages.
- **`.rig.json` manifest** in each workspace root tracks repos, branches, remote, and optional upstream. Commands that take an optional workspace name resolve it by walking up from CWD to find this file.
- **`add`/`remove`/`status`/`sync`** infer workspace from CWD; `create`/`destroy` require explicit name.
- **`add` doubles as update** — re-running `add` with `--upstream` on an existing repo updates the upstream field instead of erroring. `--no-upstream` clears it.
- **Default branch naming**: `rig/<workspace-name>` when `--branch` is not specified.
- **`sync` conflict strategy**: fetch + rebase onto the effective upstream (custom if set, otherwise default branch), abort on conflict (don't leave repo in broken state). `--stash` flag for auto-stashing dirty worktrees.
- **Optional per-repo config pattern**: new fields use `Option<T>` with `#[serde(default, skip_serializing_if = "Option::is_none")]` and an `effective_*()` method for fallback logic. See `docs/solutions/upstream-config-with-fallback.md`.

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
- `tests/cli_test.rs` — All subcommands end-to-end via `assert_cmd`, including upstream set/update/clear flows
- `tests/common/mod.rs` — `TestSandbox` fixture: creates temp dirs with bare+clone repos, worktrees, and workspaces

Each test creates its own `TestSandbox` (temp dir) — no shared state, no CWD mutation. The `_from` variants (`resolve_workspace_from`, `resolve_base_dir_from`, `create_from`, `destroy_from`) accept a start directory so tests avoid `chdir`.

## Gotchas

- Git worktrees require that a branch is checked out in only one worktree at a time. If `git rig add` fails with "already checked out", the branch exists in another worktree.
- `default_branch()` detection requires `origin/HEAD` to be set (done by `git clone`). For repos created with `git init`, run: `git remote set-head origin <branch>`.
- `git rig destroy` force-removes worktrees (even dirty ones). `git rig remove` does not — it will fail on dirty worktrees.
- `--upstream` sets the branch that `sync` rebases onto. The upstream branch is not validated at set time — if it doesn't exist on the remote, `sync` will fail with a git error at rebase time.
