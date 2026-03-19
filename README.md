# ws

Git worktree workspace manager. Creates multi-repo workspaces using git worktrees instead of symlinks.

## Why

When you symlink repos into a workspace directory, tools that resolve paths (IDEs, sandboxes, Claude Code) see the real paths outside your workspace and prompt for permissions or lose context. Worktrees create real directories inside the workspace — same files, shared git object store, no path resolution issues.

## Install

```bash
cargo install --path .
```

## Usage

### Create a workspace

From the directory where your repos live:

```bash
cd ~/projects
ws create my-feature
```

### Add repos

`ws add` takes a path to a locally cloned repo (relative or absolute):

```bash
cd ~/projects/my-feature
ws add ../api-server                                # new branch ws/my-feature from default
ws add ../web-app --branch feature/PROJ-123           # specific branch
ws add ~/work/infra/auth-service --remote upstream       # repo from a different directory
ws add ../api-server --detach                       # read-only, detached HEAD
ws add ../api-server --name api                      # custom name in workspace
```

When outside a workspace, pass the workspace name first:

```bash
ws add my-feature ../api-server
```

### Check status

```bash
ws status
```

```
Workspace: my-feature (/Users/you/projects/my-feature)

  api-server on ws/my-feature [dirty] +2 -5
    last: a1b2c3d fix auth token refresh (2 hours ago)
  web-app on feature/PROJ-123
    last: d4e5f6a add tenant validation (3 days ago)
```

### Sync (fetch + rebase)

```bash
ws sync              # all repos in current workspace
ws sync --stash      # auto-stash dirty repos before rebasing
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
ws exec -- git status               # run in all repos
ws exec --repo api-server -- make   # run in specific repo(s)
ws exec --fail-fast -- cargo test    # stop on first failure
ws exec -w my-feature -- git pull    # target a workspace by name
```

### Refresh default branches

If upstream repos change their default branch (e.g. `master` → `main`), refresh the manifest:

```bash
ws refresh
```

### Remove a repo

```bash
ws remove api-server
ws remove api-server --force            # remove even if worktree is dirty
ws remove api-server --delete-branch    # also delete the branch from the source repo
```

### List workspaces

```bash
ws list
```

### Destroy a workspace

```bash
ws destroy my-feature
ws destroy my-feature --dry-run    # preview what would be removed
```

This removes all worktrees and deletes the workspace directory. Force-removes dirty worktrees.

## How it works

Each workspace is a directory containing a `.ws.json` manifest:

```json
{
  "name": "my-feature",
  "repos": [
    {
      "name": "api-server",
      "source": "/Users/you/projects/api-server",
      "branch": "ws/my-feature",
      "default_branch": "master",
      "remote": "origin"
    }
  ]
}
```

- Each repo entry stores the absolute `source` path to the local git clone
- Repos can live anywhere on disk — they don't need to be siblings of the workspace
- Worktrees are created inside the workspace directory
- Commands that accept an optional workspace name (`add`, `remove`, `status`, `sync`, `refresh`, `exec`) auto-detect the workspace by walking up from CWD

## Testing

```bash
cargo test                    # all tests (89)
cargo test --bin ws           # unit tests — manifest ops, workspace resolution
cargo test --test git_test    # integration tests — git operations against real repos
cargo test --test cli_test    # E2E tests — full CLI commands via assert_cmd
```

Tests create temporary git repos (bare remote + clone) per test case — no shared state, no CWD mutation.

## Things to know

- A git branch can only be checked out in one worktree at a time. If `ws add` fails with "already checked out", the branch exists in another worktree.
- Default branch detection requires `origin/HEAD` (or `<remote>/HEAD`) to be set. For repos not created via `git clone`, run: `git remote set-head origin --auto`
- `ws destroy` force-removes worktrees. `ws remove` does not — it fails on dirty worktrees unless `--force` is passed.
- You can edit `.ws.json` directly to change remotes, branches, or other settings.
