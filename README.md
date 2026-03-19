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

```bash
cd ~/projects/my-feature
ws add api-server                          # new branch ws/my-feature from default
ws add web-app --branch feature/PROJ-123     # specific branch
ws add auth-service --remote upstream           # fetch from a different remote
ws add api-server --detach                 # read-only, detached HEAD
```

When outside a workspace, pass the name first:

```bash
ws add my-feature api-server
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

### Remove a repo

```bash
ws remove api-server
```

### List workspaces

```bash
ws list
```

### Destroy a workspace

```bash
ws destroy my-feature
```

This removes all worktrees and deletes the workspace directory. Force-removes dirty worktrees.

## How it works

Each workspace is a directory containing a `.ws.json` manifest:

```json
{
  "name": "my-feature",
  "base_dir": "/Users/you/projects",
  "repos": [
    {
      "name": "api-server",
      "branch": "ws/my-feature",
      "default_branch": "master",
      "remote": "origin"
    }
  ]
}
```

- `base_dir` is where source repos and workspaces live (set at `ws create` time)
- Repos are resolved as `<base_dir>/<repo-name>`
- Worktrees are created inside the workspace directory
- Commands that accept an optional workspace name (`add`, `remove`, `status`, `sync`) auto-detect the workspace by walking up from CWD

## Things to know

- A git branch can only be checked out in one worktree at a time. If `ws add` fails with "already checked out", the branch exists in another worktree.
- Default branch detection requires `origin/HEAD` (or `<remote>/HEAD`) to be set. For repos not created via `git clone`, run: `git remote set-head origin --auto`
- `ws destroy` force-removes worktrees. `ws remove` does not — it fails on dirty worktrees.
- You can edit `.ws.json` directly to change remotes, branches, or other settings.
