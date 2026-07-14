# git-rig

Git worktree workspace manager. Creates multi-repo workspaces using git worktrees instead of symlinks.

## Why

When you symlink repos into a workspace directory, tools that resolve paths (IDEs, sandboxes, Claude Code) see the real paths outside your workspace and prompt for permissions or lose context. Worktrees create real directories inside the workspace — same files, shared git object store, no path resolution issues.

## Install

### Homebrew

```bash
brew install cemcatik/tap/git-rig
```

### Shell (Linux/macOS)

```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/cemcatik/git-rig/releases/latest/download/git-rig-installer.sh | sh
```

### PowerShell (Windows)

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://github.com/cemcatik/git-rig/releases/latest/download/git-rig-installer.ps1 | iex"
```

### Cargo

```bash
cargo install --git https://github.com/cemcatik/git-rig
```

### Download

Pre-built binaries are available on the [releases page](https://github.com/cemcatik/git-rig/releases).

### From source (contributors)

```bash
cargo install --path .
```

The installed binary `git-rig` is also available as `git rig` (git auto-discovers `git-*` binaries on your PATH).

## Usage

### Create a workspace

From the directory where your repos live:

```bash
cd ~/projects
git rig create my-feature
```

### Add repos

`git rig add` takes a path to a locally cloned repo (relative or absolute):

```bash
cd ~/projects/my-feature
git rig add ../api-server                                    # new branch rig/my-feature from default
git rig add ../web-app --branch feature/PROJ-123             # specific branch
git rig add ~/projects/infra/auth-service --remote upstream  # repo from a different directory
git rig add ../api-server --detach                           # read-only, detached HEAD
git rig add ../api-server --name api                         # custom name in rig
git rig add ../api-server --upstream develop                 # start from and sync against develop
```

When outside a workspace, pass the workspace name first:

```bash
git rig add my-feature ../api-server
```

To change or clear the upstream for an existing repo, re-run `add`:

```bash
git rig add ../api-server --upstream integration   # change upstream
git rig add ../api-server --no-upstream            # revert to default branch
```

### Clone a workspace

To create a new workspace with the same repos as an existing one:

```bash
git rig create my-feature-v2 --from my-feature
```

Each repo gets a fresh `rig/my-feature-v2` branch. Upstream and remote config are inherited. Local files from `.riginclude` are carried over from the source rig's worktrees. Use `--skip` to continue past repos whose source paths are no longer valid.

### Local file provisioning

Repos often have local files (`.env`, IDE config, tool overrides) that are gitignored but needed for development. Create a `.riginclude` file in the repo root with gitignore-style patterns:

```
# .riginclude
.env*
.vscode/
docker-compose.override.yml
```

When `git rig add` or `git rig create --from` creates a worktree, matching files are automatically copied from the source:

```bash
git rig add ../api-server                    # copies .env, .vscode/, etc. from the clone
git rig create v2 --from v1                  # copies from source rig's worktrees
git rig add ../api-server --no-provision     # skip file provisioning
git rig add ../api-server --link             # symlink instead of copy
git rig add ../api-server --force-provision  # overwrite existing files
```

`.riginclude` itself is always copied to the new worktree so it propagates automatically. The file can be gitignored (personal) or committed (team-shared).

Provisioning failures are warnings — the worktree is still created and usable.

### Check status

```bash
git rig status
```

```
Workspace: my-feature (/Users/you/projects/my-feature)

  api-server on rig/my-feature [dirty] +2 -5 (vs develop)
    last: a1b2c3d fix auth token refresh (2 hours ago)
  web-app on feature/PROJ-123
    last: d4e5f6a add tenant validation (3 days ago)
```

Repos with a custom `--upstream` show `(vs <branch>)` to indicate what they sync against.

If the filesystem/git state has diverged from the manifest (e.g., someone ran `git checkout main` inside a worktree), drift warnings appear at the top:

```
  DRIFT api-server: on main, expected rig/my-feature
```

Drift detection runs automatically on `status`, `sync`, `exec`, and `refresh`. No flags needed.

### Sync (fetch + rebase)

```bash
git rig sync              # all repos in current workspace (parallel by default)
git rig sync --stash      # auto-stash dirty repos before rebasing
git rig sync --reconcile  # reset squash-merged branches to their landed state
git rig sync --repo api-server              # sync only specific repo(s)
git rig sync --repo api-server --repo web   # sync multiple
git rig sync -j1          # force sequential execution
git rig sync --jobs=4     # limit to 4 parallel workers
```

```
Syncing workspace 'my-feature'

  ok api-server 702a039 -> 069c37c
  ok web-app already up to date
  SKIP auth-service (dirty — use --stash to auto-stash)

ok All repos synced
```

Repos with drift (branch mismatch, unexpected detached HEAD, missing source) are automatically skipped during sync to prevent rebasing the wrong branch.

#### Post-merge reconciliation

When a branch is **squash-merged** upstream, its work is already in the default branch — but `git rebase` (per-commit replay) still conflicts, because the individual commits can no longer be applied onto the squashed result. `git rig sync` recognizes this: when a rebase conflicts, it runs an in-memory 3-way merge (`git merge-tree`) to ask "does this branch still add anything?" If the answer is no — the branch is **provably landed** — sync surfaces a distinct `~ reconcilable` state instead of a dead-end `ERR rebase conflict`:

```
  ~    return-services (already landed — reconcilable)
  ~    returns-ui      (already landed — reconcilable)
```

Offer to remediate with `--reconcile` (or answer the per-repo `[y/N]` prompt in a terminal). Reconciling resets the branch to the landed state (`git reset --hard`, safe by construction since the trees are byte-identical) and, if the branch's upstream was deleted post-merge, clears the stale `upstream` from `.rig.json`. The pre-reset SHA is printed for reflog recovery. Genuinely divergent branches keep the unchanged `ERR (rebase conflict — aborted)` behavior. Requires git ≥ 2.38 for `git merge-tree --write-tree`.

### Run a command across repos

```bash
git rig exec -- git status                # run in all repos (parallel by default)
git rig exec --repo api-server -- make    # run in specific repo(s)
git rig exec --fail-fast -- cargo test    # stop on first failure
git rig exec -w my-feature -- git pull    # target a workspace by name
git rig exec -j1 -- make build           # force sequential (streams output live)
```

### Refresh default branches

If upstream repos change their default branch (e.g. `master` → `main`), refresh the manifest:

```bash
git rig refresh
```

### Remove a repo

```bash
git rig remove api-server
git rig remove api-server --force            # remove even if worktree is dirty
git rig remove api-server --keep-branch      # keep the branch in the source repo (default: deleted)
```

### List workspaces

```bash
git rig list
```

### Destroy a workspace

```bash
git rig destroy my-feature
git rig destroy my-feature --dry-run         # preview what would be removed
git rig destroy my-feature --keep-branches   # keep branches in source repos
```

This removes all worktrees, deletes their branches, and removes the workspace directory. Force-removes dirty worktrees.

## How it works

Each workspace is a directory containing a `.rig.json` manifest:

```json
{
  "name": "my-feature",
  "jobs": 4,
  "repos": [
    {
      "name": "api-server",
      "source": "/Users/you/projects/api-server",
      "branch": "rig/my-feature",
      "default_branch": "master",
      "remote": "origin",
      "upstream": "develop"
    }
  ]
}
```

`upstream` and `jobs` are optional. `jobs` sets the default parallelism for `sync`, `exec`, and `refresh` (override per-invocation with `--jobs N`). Repos are always processed in alphabetical order.

- Each repo entry stores the absolute `source` path to the local git clone
- `upstream` is optional — when set, the worktree starts from this branch and `sync` rebases onto it instead of `default_branch`
- Repos can live anywhere on disk — they don't need to be siblings of the workspace
- Worktrees are created inside the workspace directory
- Commands that accept an optional workspace name (`add`, `remove`, `status`, `sync`, `refresh`, `exec`, `doctor`) auto-detect the workspace by walking up from CWD

## Development

Requires [just](https://github.com/casey/just) for task running:

```bash
just check                          # fmt + clippy + test (recommended before committing)
just test                           # all tests
just test-unit                      # unit tests — manifest ops, workspace resolution
just test-integration               # integration tests — git operations against real repos
just test-e2e                       # E2E tests — full CLI commands via assert_cmd
just cross-check                    # verify all release targets compile (no linker needed)
just coverage                       # generate lcov coverage report
just deny                           # license + advisory audit (requires cargo-deny)
```

`just cross-check` runs `cargo check` against all five release targets (macOS arm64/x86, Linux arm64/x86, Windows x86). This catches platform-specific compilation errors (like using `std::os::unix` without `#[cfg]` guards) without needing cross-compilers or linkers. Missing targets are installed automatically via `rustup`.

A pre-commit hook (`hooks/pre-commit`) is auto-installed into `.git/hooks/` on the first `cargo build` or `cargo test`. It runs `cargo fmt --check` and `cargo clippy` before each commit. To update the hook after changes to `hooks/pre-commit`, delete `.git/hooks/pre-commit` and rebuild.

Tests create temporary git repos (bare remote + clone) per test case — no shared state, no CWD mutation.

## Releasing

```bash
scripts/release.sh 0.2.0
```

This bumps the version in `Cargo.toml`, updates `Cargo.lock`, commits, tags, and pushes. The tag push triggers the [release workflow](.github/workflows/release.yml) which builds binaries, creates a GitHub Release, and publishes the Homebrew formula.

### Check workspace health

```bash
git rig doctor
```

```
Environment

  PASS  git found on PATH
  PASS  git version 2.39.0 (>= 2.38 required)

Rig: my-feature (2 repos)

  api-server
    PASS  source repo exists
    PASS  worktree exists and reachable
    PASS  branch matches manifest (rig/my-feature)
    PASS  origin/HEAD set
    PASS  remote 'origin' reachable
    PASS  upstream branch 'develop' exists on remote

  web-app
    PASS  source repo exists
    PASS  worktree exists and reachable
    WARN  origin/HEAD not set
      Default branch detection won't work.
      Fix: cd /Users/you/projects/web-app && git remote set-head origin --auto

ok All checks passed
```

Checks environment prerequisites (git version >= 2.38) and per-repo health (worktree integrity, branch state, remote reachability, upstream validity). Works outside a rig too — shows environment checks only. Exits 1 on any issue for scripting/CI use.

### Shell completions

```bash
git rig completions bash > ~/.bash_completion.d/git-rig
git rig completions zsh > ~/.zfunc/_git-rig
git rig completions fish > ~/.config/fish/completions/git-rig.fish
git rig completions powershell > _git-rig.ps1
```

## Things to know

- A git branch can only be checked out in one worktree at a time. If `git rig add` fails with "already checked out", the error tells you which worktree has the branch.
- Default branch detection requires `origin/HEAD` (or `<remote>/HEAD`) to be set. For repos not created via `git clone`, run: `git remote set-head origin --auto`
- `git rig destroy` force-removes worktrees. `git rig remove` does not — it fails on dirty worktrees unless `--force` is passed.
- `--upstream` sets the branch that the worktree starts from and that `sync` rebases onto. The upstream branch must exist on the remote at add time. Git tracking and `git log` will reference the upstream ref.
- `.riginclude` uses gitignore pattern syntax (globs, `**`, trailing `/` for directories, `!` for negation, `#` for comments). Only the file at the repo root is read — no recursive/nested `.riginclude` files.
- You can edit `.rig.json` directly to change remotes, branches, or other settings.
- If you manually switch branches inside a worktree (`git checkout main`), `status` and `sync` will detect the mismatch and warn. `sync` will skip the drifted repo to prevent rebasing the wrong branch.
