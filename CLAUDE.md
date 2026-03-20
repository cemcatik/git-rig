# git-rig — Git Worktree Workspace Manager

CLI tool for managing multi-repo workspaces using git worktrees.

## Build & Run

```bash
cargo build                    # debug build
cargo build --release          # release build
cargo install --path .         # install to ~/.cargo/bin/git-rig
```

## Architecture

Single-binary Rust CLI. Four source files:

- `src/main.rs` — CLI definition (clap derive), dispatch
- `src/commands.rs` — Command implementations (create, add, remove, destroy, list, status, sync)
- `src/workspace.rs` — Manifest types (`.rig.json`), workspace resolution from CWD
- `src/git.rs` — Git operations (shells out to `git`, not libgit2)

## Key Design Decisions

- **Shells out to `git`** rather than using `git2` crate — worktree support in libgit2 is incomplete, and raw git gives better error messages.
- **`.rig.json` manifest** in each workspace root tracks repos, branches, and base_dir. Commands that take an optional workspace name resolve it by walking up from CWD to find this file.
- **`add`/`remove`/`status`/`sync`** infer workspace from CWD; `create`/`destroy` require explicit name.
- **Default branch naming**: `rig/<workspace-name>` when `--branch` is not specified.
- **`sync` conflict strategy**: fetch + rebase, abort on conflict (don't leave repo in broken state). `--stash` flag for auto-stashing dirty worktrees.

## Testing

```bash
cargo test                          # all tests (89)
cargo test --bin git-rig            # unit tests — manifest ops, workspace resolution
cargo test --test git_test          # integration tests — git operations against real repos
cargo test --test cli_test          # E2E tests — full CLI commands via assert_cmd
```

Test structure:
- `src/workspace.rs` (inline `#[cfg(test)]`) — Manifest CRUD, save/load roundtrip, migration, workspace resolution via `_from` variants
- `tests/git_test.rs` — Branch detection, worktree add/remove, dirty checks, ahead/behind, stash, rebase
- `tests/cli_test.rs` — All 9 subcommands end-to-end via `assert_cmd`
- `tests/common/mod.rs` — `TestSandbox` fixture: creates temp dirs with bare+clone repos, worktrees, and workspaces

Each test creates its own `TestSandbox` (temp dir) — no shared state, no CWD mutation. The `_from` variants (`resolve_workspace_from`, `resolve_base_dir_from`, `create_from`, `destroy_from`) accept a start directory so tests avoid `chdir`.

## Gotchas

- Git worktrees require that a branch is checked out in only one worktree at a time. If `git rig add` fails with "already checked out", the branch exists in another worktree.
- `default_branch()` detection requires `origin/HEAD` to be set (done by `git clone`). For repos created with `git init`, run: `git remote set-head origin <branch>`.
- `git rig destroy` force-removes worktrees (even dirty ones). `git rig remove` does not — it will fail on dirty worktrees.
