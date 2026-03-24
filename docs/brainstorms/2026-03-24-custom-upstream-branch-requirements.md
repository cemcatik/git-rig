---
date: 2026-03-24
topic: custom-upstream-branch
---

# Custom Upstream Branch Per Repo

## Problem Frame

When a rig is created for work that targets a non-default branch (e.g., an `api-migration` rig where `payments-api` should sync against the `integration` branch instead of `main`), there is no way to tell git-rig which remote branch to rebase onto. `sync` and `status` always use the repo's detected default branch, forcing users to manually rebase outside the tool.

## Requirements

- R1. `RepoEntry` gains an optional `upstream` field. When set, `sync` rebases the local branch onto `{remote}/{upstream}` instead of `{remote}/{default_branch}`.
- R2. `git rig add <path> --upstream <branch>` sets the upstream when adding a repo for the first time.
- R3. Re-running `git rig add` for a repo that already exists in the rig with `--upstream <branch>` updates the upstream field (idempotent update, no error). Without `--upstream`, the existing error ("already in rig") is preserved.
- R4. `git rig add <path> --no-upstream` clears a previously set upstream, reverting to the default branch behavior.
- R5. `git rig status` shows ahead/behind relative to the effective upstream (custom upstream if set, otherwise default branch).
- R6. `git rig sync` rebases onto the effective upstream.
- R7. `git rig refresh` continues to update `default_branch` only — it does not modify the user-chosen `upstream`.

## Success Criteria

- A user can create a rig, add repos with `--upstream`, run `sync`, and see their local branch rebased onto the specified upstream branch.
- Existing rigs without `upstream` set behave identically to today (backward compatible).

## Scope Boundaries

- No new subcommand — `add` handles both initial set and update of upstream.
- `upstream` is a plain branch name (e.g., `integration`), not a full ref. The remote prefix is derived from the repo's `remote` field.
- Validation that the upstream branch exists on the remote is not required (the user is responsible; `sync` will fail with a clear git error if the branch doesn't exist).

## Key Decisions

- **Reuse `add` instead of new command**: Avoids command proliferation. `add` already takes `--branch` and `--remote`, so `--upstream` is a natural addition. When the repo already exists, `--upstream` triggers an update path instead of an error.
- **Optional field, not rename**: `upstream` is additive. `default_branch` stays as-is for `refresh` and as the fallback when no upstream is set.
- **Status follows sync**: `status` ahead/behind uses the same effective branch as `sync`, so the numbers match what sync would actually do.

## Dependencies / Assumptions

- The upstream branch must exist on the remote at `sync` time. No upfront validation.
- Serde default for `upstream` is `None` — existing manifests deserialize without changes.

## Outstanding Questions

### Deferred to Planning

- [Affects R3][Technical] When re-running `add` for an existing repo, should the repo be matched by name derived from the path, or should the user pass `--name` explicitly? Current `add` derives the name from the path basename — same logic should apply for matching.
- [Affects R5][Technical] Should `status` visually indicate when a custom upstream is active (e.g., showing `consolidation` instead of `main` in the output)?

## Next Steps

-> `/ce:plan` for structured implementation planning
