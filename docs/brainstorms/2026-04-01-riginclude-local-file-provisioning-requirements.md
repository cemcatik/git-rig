---
date: 2026-04-01
topic: riginclude-local-file-provisioning
---

# .riginclude — Local File Provisioning for Worktrees

## Problem Frame

Developers maintain local, gitignored files per repository — `.env` files, IDE configs, tool overrides — that are essential for working but absent from new worktrees. Today, users must manually remember and copy these files every time they run `git rig add` or `git rig create --from`. As rig-based workflows become the primary development flow, this friction compounds: the "source of truth" for these files may live in any rig, not just the original base clone.

## Requirements

- R1. Each repo may contain a `.riginclude` file in its root, using `.gitignore`-style glob patterns, listing local files to provision into new worktrees.
- R2. `.riginclude` is always copied from the source regardless of whether it lists itself (self-propagating). It is typically gitignored for personal use, but teams can commit it for shared patterns — git-rig makes no assumption either way.
- R3. On `git rig add`, matching files are copied from the base clone (the repo's `source` path in the manifest) into the new worktree.
- R4. On `git rig create --from`, matching files are copied from the source rig's worktree for each repo (not the base clone).
- R5. Files are copied by default. A `--link` flag creates symlinks instead.
- R6. If a matching file already exists in the target worktree, it is skipped with a warning. A `--force` flag overwrites existing files.
- R7. A `--no-provision` flag on `add` and `create` skips file provisioning entirely.
- R8. Missing source files (pattern matches nothing) are silently ignored — not every file needs to exist in every repo.
- R9. Provisioning results are reported to the user: which files were copied, which were skipped (with reason), which patterns matched nothing.

## Success Criteria

- Running `git rig add` on a repo with a `.riginclude` provisions local files without manual intervention.
- Running `git rig create --from` carries local files from the source rig's worktrees.
- A developer can set up `.riginclude` once per repo and never think about local file copying again.

## Scope Boundaries

- No standalone `git rig provision` command — provisioning only happens during `add` and `create --from`. Can be added later.
- No cross-rig provisioning for individual repos (e.g., "add repo X but copy .env from rig Y"). Source is always contextual: base clone for `add`, source rig for `create --from`.
- git-rig is agnostic about whether `.riginclude` is committed or gitignored — that's the team's choice.
- No safety check against `.gitignore` (unlike `.worktreeinclude` convention). Users own their `.riginclude` and are trusted to list the right files.
- No recursive/nested `.riginclude` files — only the one at the repo root is read.

## Key Decisions

- **Per-repo declaration over per-rig**: Different repos need different local files. The pattern list lives with the repo, not in `.rig.json`.
- **Agnostic about version control**: `.riginclude` can be gitignored (personal) or committed (team-shared). git-rig copies it either way; the team decides whether to track it.
- **Copy-by-default over symlink-by-default**: Copies give each worktree independence. Symlinks available via `--link` for files that should stay in sync.
- **Skip-by-default over overwrite-by-default**: Protects manually-placed files. `--force` available when needed.
- **Self-propagating `.riginclude`**: Always copied from source so the pattern file itself carries over without explicit listing.
- **No `.gitignore` cross-check**: Simpler implementation. The `.worktreeinclude` convention validates against `.gitignore` to prevent copying tracked files, but since `.riginclude` is user-maintained and local, the extra guard adds complexity without meaningful safety benefit.

## Outstanding Questions

### Deferred to Planning

- [Affects R1][Technical] What glob library or approach should be used for `.gitignore`-style pattern matching in Rust?
- [Affects R5][Technical] For `--link`, should symlinks be relative or absolute? Relative is more portable but may break if the source moves.
- [Affects R3, R4][Technical] Should provisioning happen before or after the worktree `git checkout`? Ordering may matter if provisioned files affect git hooks or build scripts.
- [Affects R9][Technical] What output format for provisioning reports — inline with existing add/create output, or a separate summary block?

## Next Steps

-> `/ce:plan` for structured implementation planning
