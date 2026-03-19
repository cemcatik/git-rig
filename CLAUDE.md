# ws — Git Worktree Workspace Manager

CLI tool for managing multi-repo workspaces using git worktrees.

## Build & Run

```bash
cargo build                    # debug build
cargo build --release          # release build
cargo install --path .         # install to ~/.cargo/bin/ws
```

## Architecture

Single-binary Rust CLI. Four source files:

- `src/main.rs` — CLI definition (clap derive), dispatch
- `src/commands.rs` — Command implementations (create, add, remove, destroy, list, status, sync)
- `src/workspace.rs` — Manifest types (`.ws.json`), workspace resolution from CWD
- `src/git.rs` — Git operations (shells out to `git`, not libgit2)

## Key Design Decisions

- **Shells out to `git`** rather than using `git2` crate — worktree support in libgit2 is incomplete, and raw git gives better error messages.
- **`.ws.json` manifest** in each workspace root tracks repos, branches, and base_dir. Commands that take an optional workspace name resolve it by walking up from CWD to find this file.
- **`add`/`remove`/`status`/`sync`** infer workspace from CWD; `create`/`destroy` require explicit name.
- **Default branch naming**: `ws/<workspace-name>` when `--branch` is not specified.
- **`sync` conflict strategy**: fetch + rebase, abort on conflict (don't leave repo in broken state). `--stash` flag for auto-stashing dirty worktrees.

## Testing

No test suite yet. Smoke test manually:

```bash
mkdir /tmp/ws-test && cd /tmp/ws-test
git init repo-a && cd repo-a && git commit --allow-empty -m "init" && git remote add origin . && cd ..
./target/debug/ws create my-ws
cd my-ws && ../target/debug/ws add repo-a
../target/debug/ws status
cd .. && ../target/debug/ws destroy my-ws
```

## Gotchas

- Git worktrees require that a branch is checked out in only one worktree at a time. If `ws add` fails with "already checked out", the branch exists in another worktree.
- `default_branch()` detection requires `origin/HEAD` to be set (done by `git clone`). For repos created with `git init`, run: `git remote set-head origin <branch>`.
- `ws destroy` force-removes worktrees (even dirty ones). `ws remove` does not — it will fail on dirty worktrees.
