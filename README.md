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

### Sync (fetch + rebase)

```bash
git rig sync              # all repos in current workspace
git rig sync --stash      # auto-stash dirty repos before rebasing
```

```
Syncing workspace 'my-feature'

  ok api-server 702a039 -> 069c37c
  ok web-app already up to date
  SKIP auth-service (dirty — use --stash to auto-stash)

ok All repos synced
```

### Run a command across repos

```bash
git rig exec -- git status                # run in all repos
git rig exec --repo api-server -- make    # run in specific repo(s)
git rig exec --fail-fast -- cargo test    # stop on first failure
git rig exec -w my-feature -- git pull    # target a workspace by name
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

- Each repo entry stores the absolute `source` path to the local git clone
- `upstream` is optional — when set, the worktree starts from this branch and `sync` rebases onto it instead of `default_branch`
- Repos can live anywhere on disk — they don't need to be siblings of the workspace
- Worktrees are created inside the workspace directory
- Commands that accept an optional workspace name (`add`, `remove`, `status`, `sync`, `refresh`, `exec`) auto-detect the workspace by walking up from CWD

## Development

Requires [just](https://github.com/casey/just) for task running:

```bash
just check                          # fmt + clippy + test (recommended before committing)
just test                           # all tests
just test-unit                      # unit tests — manifest ops, workspace resolution
just test-integration               # integration tests — git operations against real repos
just test-e2e                       # E2E tests — full CLI commands via assert_cmd
just coverage                       # generate lcov coverage report
just deny                           # license + advisory audit (requires cargo-deny)
```

Tests create temporary git repos (bare remote + clone) per test case — no shared state, no CWD mutation.

## Releasing

```bash
scripts/release.sh 0.2.0
```

This bumps the version in `Cargo.toml`, updates `Cargo.lock`, commits, tags, and pushes. The tag push triggers the [release workflow](.github/workflows/release.yml) which builds binaries, creates a GitHub Release, and publishes the Homebrew formula.

## Things to know

- A git branch can only be checked out in one worktree at a time. If `git rig add` fails with "already checked out", the branch exists in another worktree.
- Default branch detection requires `origin/HEAD` (or `<remote>/HEAD`) to be set. For repos not created via `git clone`, run: `git remote set-head origin --auto`
- `git rig destroy` force-removes worktrees. `git rig remove` does not — it fails on dirty worktrees unless `--force` is passed.
- `--upstream` sets the branch that the worktree starts from and that `sync` rebases onto. The upstream branch must exist on the remote at add time. Git tracking and `git log` will reference the upstream ref.
- You can edit `.rig.json` directly to change remotes, branches, or other settings.
