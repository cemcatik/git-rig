---
date: 2026-07-13
topic: post-merge-reconciliation
---

# Post-Merge Branch Reconciliation in `git rig sync`

## Problem Frame

When a rig's branch is squash-merged (or otherwise landed) upstream and then someone
runs `git rig sync`, the sync **dead-ends on an opaque conflict** even though the branch's
work is already 100% present upstream. `sync` does `git fetch --prune` + `git rebase` onto
`effective_upstream`; a squash-merge leaves the individual branch commits no longer
re-appliable onto the squashed target, so the rebase conflicts, `sync` runs
`rebase --abort`, and reports `rebase conflict — aborted`. The user is left with no signal
that the branch is *redundant* rather than *divergent*, and no path forward.

**Origin (real transcript).** A user hit this on a rig with `return-services` + `returns-ui`.
Both branches were squash-merged (PRs #5486 / #2738); `git rig sync` failed with rebase
conflicts even though the work was fully in `master`. `returns-ui` additionally had a **gone
upstream** — its tracking branch was deleted post-merge. The user's manual diagnosis was a
three-dot `diff --name-only` against the merge-base to find the touched files, then a two-dot
`git diff <branch> origin/master --` of *those files*: an empty result proving the branch was
byte-identical to `master`.

**Root cause.** `git rebase` (per-commit replay) and a real 3-way merge disagree precisely on
the squash-merge case, and the current `sync` only knows how to rebase. A redundant branch is
therefore indistinguishable from a genuinely divergent one — both surface as
`rebase conflict — aborted`.

**Current behavior in code.** `sync` = `git::fetch` (already `--prune`) + `git::rebase(onto, remote)`
onto `{remote}/{effective_upstream}`; on conflict it runs `rebase --abort` and reports.
See `src/commands.rs` `sync_sequential` / `sync_parallel` (~L973–1245) and `git::rebase`
(`src/git.rs:247`). `RepoEntry::effective_upstream()` (`src/workspace.rs:39`) returns
`upstream ?? default_branch`.

This document is the **locked design spec**. Implementation is a separate, later effort.

## Requirements

- **R1. Split the conflict bucket.** `sync` must distinguish a **post-merge redundant** branch
  (work already landed upstream, fully or via squash-merge, and/or upstream gone) from a
  genuinely divergent branch, instead of reporting both as `rebase conflict — aborted`.
- **R2. Reactive, zero-cost-on-clean detection.** Classification runs **only when the rebase
  actually conflicts** — a clean rig pays nothing.
- **R3. Offer to remediate, never surprise.** On a redundant branch, `sync` offers a
  remediation (reset the branch to the landed state; for a gone upstream, also repair the
  manifest). Mutation requires explicit consent (per-repo prompt or `--reconcile`); piped
  output never mutates.
- **R4. Provable redundancy only.** A branch is remediated only when its redundancy is *proven*
  by construction — no heuristic may cause committed, un-landed work to be discarded.
- **R5. Preserve today's behavior for genuine divergence.** A truly divergent branch keeps
  today's `rebase --abort` + `ERR (rebase conflict — aborted)`, unchanged.
- **R6. Parallel-safe.** Detection must run inside the existing parallel `sync` worker pool
  (in-memory, no index lock, no checkout); remediation runs in a deferred sequential pass so it
  can prompt.
- **R7. Multi-repo continue-and-report.** A per-repo remediation failure is collected into the
  summary and never aborts the other repos or the rest of `sync` (git-rig's standing pattern).

## Comparison Target — what "landed" is measured against

*(from decision "Decide what 'landed' is measured against")*

The branch is compared against the **ref `sync` already rebases onto**, via a safe-degrading
fallback chain. Every case terminates in a target or a non-fatal skip — never a guess, never a
hard error that strands the rest of the sync.

| Case | Comparison target | Signal to classifier |
|---|---|---|
| Custom `upstream` set in `.rig.json`, ref resolves | `{remote}/{upstream}` | — |
| No custom upstream | `{remote}/{default_branch}` (`origin/HEAD`) | — |
| `{remote}/{effective_upstream}` gone after prune | fall back to `{remote}/{default_branch}` | `upstream_gone = true` |
| Default-branch fallback also unresolvable | **skip repo** | report `cannot determine upstream target` (non-fatal) |
| Worktree detached | **skip repo** | report `detached — not reconcilable` |

**Cross-cutting rules:**

- **Branch side** of the merge-tree call is the worktree's currently checked-out branch (HEAD) —
  exactly what `sync` would rebase.
- **Timing:** resolve the target *after* `git fetch --prune`, from **remote-tracking refs only**.
  The `--prune` is what makes the gone-upstream case detectable (it removes the stale
  `origin/<upstream>` ref); remote-only prevents a stale local branch from masking a gone remote.
- **Gone-upstream fallback rationale:** a deleted post-merge topic/integration branch almost
  always merged into the default branch (the `returns-ui` case), so we retry against the default
  branch and raise `upstream_gone` rather than stranding the exact user this spec exists for.
- **Detached HEAD:** no branch means none of the remediation actions apply — skip and report.

## Detection & Classification

*(from decision "Decide the detection & classification algorithm", grounded in research ticket 001)*

**Primitive:** `git merge-tree --write-tree <target> <branch>` — an in-memory recursive 3-way
merge (no working tree, no index lock, no checkout) that prints the resulting tree OID; exit 0 =
clean, exit ≠ 0 = conflict. It is the **only** primitive correct across squash / merge / rebase
landings *and* the "upstream moved on" case, because it performs the real merge that answers the
actual question — "does merging the branch add anything?" — rather than an ancestry or
patch-identity proxy. (`git branch --merged`, `git cherry`/`patch-id`, and empty-content-diff
were all empirically shown to misclassify multi-commit squashes, renames, or
upstream-evolved-files; see research ticket 001.)

**Trigger — reactive, on the conflict path only.** `merge-tree` runs *only* for repos whose
rebase actually conflicts. Rationale: `rebase` (per-commit replay) and `merge-tree` (one 3-way
merge) use different algorithms, and the squash-merge bug *is* their disagreement — so running
`merge-tree` right after a rebase conflict asks exactly the right question.

**Per-repo flow:**

```
1. fetch --prune
2. resolve target                       # comparison-target chain; raise upstream_gone here
3. rebase <branch> onto target
   ├─ clean    → synced (applies any missing work silently)
   └─ conflict → rebase --abort, then:
4.    MT = git merge-tree --write-tree <target> <branch>
      ├─ exit ≠ 0                        → NOT LANDED → ERR aborted (unchanged)
      ├─ exit 0 AND MT == target^{tree}  → LANDED     → ~ reconcilable
      └─ exit 0 AND MT != target^{tree}  → NOT LANDED → ERR aborted   (the "merge-only-clean" case)
```

**Authoritative predicate — strict tree-OID equality, standing alone.**
`LANDED ≡ merge-tree exits 0 AND its result tree OID == git rev-parse <target>^{tree}`.
That predicate *is* the proof of redundancy: the real 3-way merge of the branch into the target
yields a byte-identical tree, so merging the branch adds nothing. **No corroborating check**
(`git cherry`, `--merged`, ancestry) may be ANDed in — each returns "not landed" on multi-commit
squashes, so requiring their agreement would reject the very branches we mean to reconcile.

**Classes collapse to LANDED / NOT-LANDED.** "Partially landed" is *not* a distinct remediable
state: a clean partial just syncs at step 3; a conflicting partial that only merges (not rebases)
cleanly — merge-tree clean but tree ≠ target — is left as a conflict, because remediating it
would require a **merge commit** and **linear history is a hard constraint** (git-rig is
rebase-only). Partial's defined behavior is therefore "no special handling."

**Version floor — single bump to git ≥ 2.38.** `merge-tree --write-tree` requires git 2.38.0
(Oct 2022). Decision: **bump git-rig's global minimum from 2.30 to 2.38** (single floor) rather
than feature-gate. `doctor`'s env check changes `2.30` → `2.38`; no per-feature gating code.
Trade accepted: drops git 2.30–2.37 users from the whole tool, judged low-impact for a ~4-year-old
git in 2026. (`merge-tree` is plumbing — identical behavior on the Windows builds git-rig ships.)

**Cost:** one `merge-tree` + one `rev-parse` per *conflicting* repo, in-memory, parallel-safe in
the existing worker pool. The origin case's 63-file branch is a single in-memory 3-way merge.

## Command Surface & UX

*(from decision "Prototype the reconcile UX and command surface"; asset: `.wayfinder/prototypes/003-reconcile-ux.sh`)*

**Surface — inline in `sync`.** Reconciliation lives where the dead-end happens. Classification
runs *inside* the normal sync pass — read-only, cheap, parallel-safe. `doctor` as a read-only
detector was explicitly *not* chosen here but stays open as a future layer (see Outstanding
Questions); it can be added later without reopening this decision.

**New third state — `~ reconcilable`** (cyan), distinct from both `ok` and
`ERR (rebase conflict — aborted)`, shown via a new `~` marker in the 4-char status column.
Splitting that bucket is the core win: a squash-merged branch no longer masquerades as a genuine
conflict.

**Interaction — both prompt and flag, prompted per-repo:**

- **TTY:** after the parallel spinner tears down, a **deferred sequential post-pass** prompts
  **per repo** `[y/N]`. Each repo is confirmed on its own line because the actions differ
  (fully-landed → reset; gone-upstream → reset + repair manifest).
- **`--reconcile` flag:** non-interactive "yes to every provably-redundant repo," for CI / `-jN` /
  piped use. Genuine conflicts still abort.
- **Piped / non-TTY without the flag:** **detect-and-hint, never mutate** — prints
  `~ <repo> already landed — run git rig sync --reconcile`.

**The parallel constraint is load-bearing.** `sync -jN` is a non-interactive `indicatif` spinner;
you cannot prompt mid-flight. The model is therefore: **classify in parallel (read-only) →
remediate in a deferred sequential pass** (prompt) or via the flag (auto). This is what lets an
interactive UX coexist with parallel sync at all.

**Output surface:** the new `~` marker, plus a post-pass summary line
`ok N reconciled · ERR M conflict(s) left untouched` sitting alongside the existing
`WARN … had issues` summary.

## Remediation & Safety Model

*(from decision "Decide remediation actions and the safety model")*

Remediation fires **only** on the LANDED verdict; every NOT-LANDED flavor keeps today's
`rebase --abort` + `ERR aborted`. The destructive op is `git reset --hard <target>`, chosen
because a squash-merge leaves the target neither ancestor nor descendant of the branch (so
`merge --ff-only` can't move it), and the tree-equality predicate proves the reset touches no
real content — it only swaps the squashed SHAs for the target's.

**Remediation matrix:**

| Classification | Action |
|---|---|
| **LANDED, upstream present** | `git reset --hard <target>` in the worktree (target = `{remote}/{effective_upstream}`) |
| **LANDED, upstream gone** (`upstream_gone`) | reset `--hard` to fallback target `{remote}/{default_branch}`, then **clear the `.rig.json` `upstream` field** (the durable fix), then `git branch --unset-upstream` (cosmetic) |
| **Partially landed** | no special handling — a clean partial just syncs; a merge-only-clean partial stays `ERR aborted` (linear-history constraint) |
| **NOT-LANDED** (incl. gone-upstream but still divergent) | unchanged — `rebase --abort` + `ERR aborted` |

**Per-repo remediation order** (LANDED only; the deferred sequential post-pass):

```
1. re-verify LANDED   # fresh merge-tree + rev-parse vs current HEAD/target
   └─ verdict changed → SKIP, mutate nothing  (report "changed since classification — skipped")
2. stash push --include-untracked            # only if --stash AND dirty; else clean is required
3. reset --hard <target>                     # print pre-reset SHA: "was 3a1f9c2 — recover via git reflog"
4. (gone-upstream only) clear .rig.json upstream field, then git branch --unset-upstream
5. stash pop (if stashed)                     # on conflict: reset STANDS, stash preserved, report
```

Reset precedes the `.rig.json` clear on purpose: if the reset fails we must **not** clear the
manifest — the `upstream` field is the surviving record of intent. The manifest write is race-free
because it happens in the single-threaded post-pass, never the parallel classify pass.

**Why clearing `.rig.json` is the real fix for a gone upstream:** `git branch --unset-upstream`
alone is cosmetic — `sync` reads its target from `.rig.json`, not from git tracking config. The
durable repair is clearing the manifest `upstream` field so future syncs fall back to the default
branch.

**Safety model:**

- **Committed un-landed work — impossible by construction.** The tree-equality predicate *is* the
  proof of redundancy and stands alone. The only addition at this layer is a **re-verify at the
  instant of reset** (one in-memory `merge-tree` + `rev-parse`) to close the TOCTOU window between
  the parallel classify pass and the deferred remediation pass. Verdict no longer LANDED ⇒ skip.
- **Uncommitted / untracked work — the one real hazard.** The classifier sees only commits, so
  `reset --hard` is gated on a **clean worktree**. Dirty reconcilable repos are skipped with a hint
  (`worktree dirty, not reset — commit/stash or use --stash`). **`--stash` is the sole override**
  and follows sync's *existing* stash contract exactly: push → reset → pop; on pop conflict the
  reset stands and the stash is preserved (`changes still in git stash`).
- **Recovery net — reflog only.** `reset --hard` is content-safe, so the only thing at risk is
  discarded SHAs, which reflog already retains ~90 days. Decision: **no backup ref**; the output
  **prints the pre-reset SHA** so recovery is discoverable at the moment it matters. Durable
  `refs/rig/backup/*` refs remain a clean additive change if ever wanted.
- **Confirmation** is inherited from the UX decision: per-repo `[y/N]` in a TTY, `--reconcile` for
  non-interactive / CI / `-jN`, detect-and-hint (never mutate) when piped.
- **Failure — continue-and-report, no rollback.** A per-repo failure (reset error, pop conflict,
  manifest write) is collected into the summary and never aborts the other repos or the rest of
  `sync`. Whatever completed is reported truthfully.

## Key Decisions

- **merge-tree tree-equality is the sole authority** for "landed." No ancestry or patch-id
  corroborator — each rejects multi-commit squashes, the exact case this exists for.
- **Reactive, not proactive.** Classify only on the conflict path, so clean rigs pay nothing and
  the check asks precisely "was that conflict real, or a squash artifact?".
- **Classify in parallel, remediate sequentially.** Forced by the non-interactive `-jN` spinner;
  it's what lets an interactive prompt coexist with parallel sync.
- **`reset --hard`, gated on a clean worktree, backed by reflog.** Content-safe by construction;
  `--stash` is the only override and reuses sync's existing stash contract.
- **Clear `.rig.json` `upstream` on a gone upstream** — the manifest, not git tracking config, is
  what `sync` reads, so this is the durable fix.
- **Single git floor bump (2.30 → 2.38)** rather than feature-gating on `merge-tree` availability.

## Scope Boundaries (Non-Goals)

- **Implementing the feature.** This document is the destination; the build is a separate, later
  effort.
- **Resolving genuine (non-redundant) rebase conflicts.** Today's `rebase --abort` + report
  behavior is retained unchanged for branches that truly diverge.
- **Remediating a "partially landed" branch.** Collapsed away by the classifier — a clean partial
  just syncs; a merge-only-clean partial stays a conflict (linear-history constraint). No merge
  commits, ever.
- **Touching a divergent branch that merely has a gone upstream.** A dead upstream on a
  still-divergent branch is left to `doctor` / future work, not reconciled here.

## Outstanding Questions

Carried from the map's "Not yet specified" — in scope for a later planning pass, not blocking the
spec:

- **Flag interactions.** How reconciliation composes with the existing `--repo` filter — expected
  to just scope classification the same way it scopes sync. (`--stash` composition is already
  resolved above: mirrors sync's push/reset/pop contract, reset stands on pop conflict.)
- **Other commands.** Whether `doctor` and/or `create --from` should share the same
  landed / gone-upstream awareness (read-only detection layered on the same predicate).
- **Config surface.** Whether `.rig.json` should carry an opt-in to auto-reconcile.

## Next Steps

The way to the destination is clear; every decision the build depends on is locked above. Hand off
to a structured implementation planning pass (`/ce:plan` or equivalent), taking the remediation
matrix, per-repo order, and safety rules as fixed inputs.
