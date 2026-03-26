---
date: 2026-03-26
topic: clone-rig
---

# Clone Rig via `create --from`

## Problem Frame

When working in multi-repo workspaces, you often need to start a parallel workstream that touches the same set of repos. Today you'd have to manually `git rig create` a new rig and then `git rig add` each repo one by one, duplicating the source rig's configuration. This is tedious and error-prone when a rig has many repos.

## Requirements

- R1. `git rig create <name> --from <source-rig>` creates a new rig pre-populated with the same repos as the source rig.
- R2. Each repo in the new rig gets a fresh branch named `rig/<new-rig-name>` (consistent with default `add` behavior).
- R3. Upstream config is inherited from the source rig. If a source repo has `upstream: "release/v2"`, the cloned repo keeps that upstream.
- R4. New worktrees start from the effective upstream: `{remote}/{upstream}` if upstream is set, otherwise `{remote}/{default_branch}`.
- R5. Without `--from`, `create` behaves exactly as it does today (empty rig).
- R6. If the target rig directory already exists, error with the existing `DirectoryAlreadyExists` behavior.
- R7. If a source repo's path no longer exists or is invalid, the command fails with a clear error identifying which repo failed.

## Success Criteria

- A user can duplicate a multi-repo rig setup in a single command.
- The new rig is immediately usable with `sync`, `status`, `exec`, etc.
- No behavior change to existing `create` (without `--from`).

## Scope Boundaries

- No `--branch` override for cloned repos (use `add --branch` after if needed).
- No partial clone (subset of repos) — clone all or none.
- No copying of uncommitted work or worktree state from the source.

## Key Decisions

- **Extend `create` rather than add a new command**: Keeps the command set small. `--from` is a natural modifier on creation.
- **New branches, not same branches**: Avoids git's one-worktree-per-branch constraint and gives a clean starting point.
- **Inherit upstream config**: The parallel workstream is in the same domain, so it should sync against the same targets.
- **Start from upstream tip, not source commit**: The new rig is a fresh workspace, not a snapshot of in-progress work.

## Outstanding Questions

### Deferred to Planning

- [Affects R1][Technical] Should the command fetch before creating each worktree (like `add` does), or is it safe to skip the fetch since the source rig presumably has recent refs?
- [Affects R2][Technical] The `--branch` flag doesn't exist on `create` today. Confirm no conflict when adding `--from` to the clap derive struct.

## Next Steps

-> `/ce:plan` for structured implementation planning
