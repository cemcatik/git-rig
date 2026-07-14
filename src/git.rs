use anyhow::{Context, Result, anyhow};
use std::path::Path;
use std::process::Command;

use crate::error::RigError;

/// Sentinel value returned by `current_branch` and stored in `RepoEntry.branch`
/// when a worktree is in detached HEAD state.
pub const DETACHED: &str = "(detached)";

/// Run a git command, capture and return stdout. Errors on non-zero exit.
fn git_output(repo_dir: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_dir)
        .args(args)
        .output()
        .with_context(|| format!("failed to run git {args:?}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!("git {:?} failed: {}", args, stderr.trim()));
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Run a git command, streaming output to the terminal. Errors on non-zero exit.
fn git_run(repo_dir: &Path, args: &[&str]) -> Result<()> {
    let status = Command::new("git")
        .arg("-C")
        .arg(repo_dir)
        .args(args)
        .status()
        .with_context(|| format!("failed to run git {args:?}"))?;

    if !status.success() {
        return Err(anyhow!(
            "git {:?} exited with code {:?}",
            args,
            status.code()
        ));
    }

    Ok(())
}

/// Run a git command silently — capture both stdout and stderr, error on failure.
fn git_quiet(repo_dir: &Path, args: &[&str]) -> Result<()> {
    git_output(repo_dir, args).map(|_| ())
}

// ---------------------------------------------------------------------------
// Branch detection
// ---------------------------------------------------------------------------

/// Detect the default branch of a repository (main, master, etc.)
pub fn default_branch(repo_dir: &Path, remote: &str) -> Result<String> {
    // Try the symbolic-ref that `git clone` sets up
    let head_ref = format!("refs/remotes/{remote}/HEAD");
    let prefix = format!("refs/remotes/{remote}/");
    if let Ok(refname) = git_output(repo_dir, &["symbolic-ref", &head_ref])
        && let Some(branch) = refname.strip_prefix(&prefix)
    {
        return Ok(branch.to_string());
    }

    // Fallback: check common names
    for name in ["main", "master"] {
        if branch_exists(repo_dir, name) || remote_branch_exists(repo_dir, name, remote) {
            return Ok(name.to_string());
        }
    }

    Err(RigError::DefaultBranchNotFound {
        repo: repo_dir.to_path_buf(),
        remote: remote.to_string(),
    }
    .into())
}

pub fn branch_exists(repo_dir: &Path, branch: &str) -> bool {
    git_output(
        repo_dir,
        &["rev-parse", "--verify", &format!("refs/heads/{branch}")],
    )
    .is_ok()
}

pub fn remote_branch_exists(repo_dir: &Path, branch: &str, remote: &str) -> bool {
    git_output(
        repo_dir,
        &[
            "rev-parse",
            "--verify",
            &format!("refs/remotes/{remote}/{branch}"),
        ],
    )
    .is_ok()
}

pub fn current_branch(repo_dir: &Path) -> Result<String> {
    let branch = git_output(repo_dir, &["branch", "--show-current"])?;
    if branch.is_empty() {
        Ok(DETACHED.to_string())
    } else {
        Ok(branch)
    }
}

// ---------------------------------------------------------------------------
// Worktree operations
// ---------------------------------------------------------------------------

fn path_str(p: &Path) -> Result<&str> {
    p.to_str().ok_or_else(|| anyhow!("non-UTF8 path"))
}

/// Create a worktree with a new branch starting from `start_point`.
pub fn worktree_add_new_branch(
    source_repo: &Path,
    worktree_path: &Path,
    branch: &str,
    start_point: &str,
) -> Result<()> {
    git_run(
        source_repo,
        &[
            "worktree",
            "add",
            "-b",
            branch,
            path_str(worktree_path)?,
            start_point,
        ],
    )
}

/// Create a worktree checking out an existing branch.
pub fn worktree_add_existing(source_repo: &Path, worktree_path: &Path, branch: &str) -> Result<()> {
    git_run(
        source_repo,
        &["worktree", "add", path_str(worktree_path)?, branch],
    )
}

/// Create a detached worktree at a specific commit.
pub fn worktree_add_detached(source_repo: &Path, worktree_path: &Path, commit: &str) -> Result<()> {
    git_run(
        source_repo,
        &[
            "worktree",
            "add",
            "--detach",
            path_str(worktree_path)?,
            commit,
        ],
    )
}

/// Remove a worktree. Use `force` to remove even if dirty.
pub fn worktree_remove(source_repo: &Path, worktree_path: &Path, force: bool) -> Result<()> {
    let p = path_str(worktree_path)?;
    if force {
        git_run(source_repo, &["worktree", "remove", "--force", p])
    } else {
        git_run(source_repo, &["worktree", "remove", p])
    }
}

/// Repair worktree links after a worktree directory has been moved.
pub fn worktree_repair(source_repo: &Path, worktree_path: &Path) -> Result<()> {
    git_quiet(
        source_repo,
        &["worktree", "repair", path_str(worktree_path)?],
    )
}

/// Prune stale worktree entries from the source repo.
pub fn worktree_prune(source_repo: &Path) -> Result<()> {
    git_quiet(source_repo, &["worktree", "prune"])
}

/// Find which worktree has a branch checked out (if any).
///
/// Returns the worktree path where the given branch is currently checked out,
/// or `None` if the branch isn't checked out anywhere.
pub fn find_worktree_for_branch(repo_dir: &Path, branch: &str) -> Option<String> {
    let output = git_output(repo_dir, &["worktree", "list", "--porcelain"]).ok()?;
    let target_ref = format!("refs/heads/{branch}");

    let mut current_path: Option<String> = None;
    for line in output.lines() {
        if let Some(path) = line.strip_prefix("worktree ") {
            current_path = Some(path.to_string());
        } else if let Some(ref_name) = line.strip_prefix("branch ") {
            if ref_name == target_ref {
                return current_path;
            }
        } else if line.is_empty() {
            current_path = None;
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Status helpers
// ---------------------------------------------------------------------------

pub fn is_dirty(repo_dir: &Path) -> Result<bool> {
    let output = git_output(repo_dir, &["status", "--porcelain"])?;
    Ok(!output.is_empty())
}

/// Returns (ahead, behind) relative to `<remote>/<remote_branch>`.
pub fn ahead_behind(repo_dir: &Path, local: &str, remote_branch: &str, remote: &str) -> (u32, u32) {
    let range = format!("{remote}/{remote_branch}...{local}");
    match git_output(repo_dir, &["rev-list", "--left-right", "--count", &range]) {
        Ok(output) => {
            let parts: Vec<&str> = output.split_whitespace().collect();
            if parts.len() == 2 {
                let behind = parts[0].parse().unwrap_or(0);
                let ahead = parts[1].parse().unwrap_or(0);
                (ahead, behind)
            } else {
                (0, 0)
            }
        }
        Err(_) => (0, 0),
    }
}

pub fn last_commit_summary(repo_dir: &Path) -> Result<String> {
    git_output(repo_dir, &["log", "-1", "--format=%h %s (%cr)"])
}

// ---------------------------------------------------------------------------
// Sync operations
// ---------------------------------------------------------------------------

pub fn fetch(repo_dir: &Path, remote: &str) -> Result<()> {
    git_quiet(repo_dir, &["fetch", remote, "--prune"])
}

pub fn rebase(repo_dir: &Path, onto: &str, remote: &str) -> Result<()> {
    git_quiet(repo_dir, &["rebase", &format!("{remote}/{onto}")])
}

/// Resolve a ref to a short commit hash.
pub fn rev_parse_short(repo_dir: &Path, rev: &str) -> Result<String> {
    git_output(repo_dir, &["rev-parse", "--short", rev])
}

pub fn rebase_abort(repo_dir: &Path) -> Result<()> {
    git_quiet(repo_dir, &["rebase", "--abort"])
}

// ---------------------------------------------------------------------------
// Post-merge reconciliation primitives
// ---------------------------------------------------------------------------

/// Outcome of an in-memory 3-way merge via `git merge-tree --write-tree`.
///
/// `merge-tree` performs a recursive 3-way merge with no working tree, no index
/// lock, and no checkout — it just prints the resulting tree OID. It is the only
/// git primitive correct across squash / merge / rebase landings *and* the
/// "upstream moved on" case. Requires git >= 2.38.
pub enum MergeTreeOutcome {
    /// Merge applied cleanly; `tree` is the resulting top-level tree OID.
    Clean { tree: String },
    /// Merge had conflicts.
    Conflict,
}

/// Perform an in-memory 3-way merge of `branch` into `target`.
///
/// Returns [`MergeTreeOutcome::Clean`] (exit 0, with the result tree OID) or
/// [`MergeTreeOutcome::Conflict`] (exit 1). Any other exit code — a bad ref, an
/// unsupported git — is a hard error, so callers can distinguish "conflicts"
/// (a legitimate answer) from "could not run the check".
pub fn merge_tree(repo_dir: &Path, target: &str, branch: &str) -> Result<MergeTreeOutcome> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_dir)
        .args(["merge-tree", "--write-tree", target, branch])
        .output()
        .with_context(|| "failed to run git merge-tree")?;

    // `merge-tree --write-tree` prints the top-level tree OID as stdout line 1
    // on BOTH a clean merge (exit 0) and a conflicted one (exit 1, followed by
    // conflicted-path entries). A bad/unmergeable ref also exits 1 but writes
    // nothing to stdout (the error goes to stderr) — so we key on stdout, not
    // the exit code alone, to tell a real conflict from "couldn't run the check".
    let stdout = String::from_utf8_lossy(&output.stdout);
    let tree = stdout.lines().next().unwrap_or("").trim().to_string();

    match output.status.code() {
        Some(0) if !tree.is_empty() => Ok(MergeTreeOutcome::Clean { tree }),
        Some(1) if !tree.is_empty() => Ok(MergeTreeOutcome::Conflict),
        other => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(anyhow!(
                "git merge-tree failed (exit {other:?}): {}",
                stderr.trim()
            ))
        }
    }
}

/// Resolve a revision to the OID of the tree it points at (`<rev>^{tree}`).
pub fn tree_oid(repo_dir: &Path, rev: &str) -> Result<String> {
    git_output(repo_dir, &["rev-parse", &format!("{rev}^{{tree}}")])
}

/// Hard-reset the worktree to `target` (discards commits on top; content-safe
/// only when the caller has proven `target` is tree-equal — see reconciliation).
pub fn reset_hard(repo_dir: &Path, target: &str) -> Result<()> {
    git_quiet(repo_dir, &["reset", "--hard", target])
}

/// Clear the branch's upstream tracking config. Cosmetic — `sync` reads its
/// target from `.rig.json`, not git tracking config — so callers ignore errors
/// (e.g. no upstream was configured).
pub fn branch_unset_upstream(repo_dir: &Path) -> Result<()> {
    git_quiet(repo_dir, &["branch", "--unset-upstream"])
}

pub fn stash_push(repo_dir: &Path) -> Result<bool> {
    let before = git_output(repo_dir, &["stash", "list"])?;
    git_quiet(
        repo_dir,
        &[
            "stash",
            "push",
            "--include-untracked",
            "-m",
            "git-rig sync auto-stash",
        ],
    )?;
    let after = git_output(repo_dir, &["stash", "list"])?;
    // If the stash list changed, something was stashed
    Ok(before != after)
}

pub fn stash_pop(repo_dir: &Path) -> Result<()> {
    git_run(repo_dir, &["stash", "pop"])
}

/// Delete a local branch. Uses `-D` (force delete) since the caller explicitly requested branch deletion.
pub fn delete_branch(repo_dir: &Path, branch: &str) -> Result<()> {
    git_quiet(repo_dir, &["branch", "-D", branch])
}

// ---------------------------------------------------------------------------
// Doctor helpers
// ---------------------------------------------------------------------------

/// Check if git is available on PATH.
pub fn is_git_available() -> bool {
    Command::new("git")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok()
}

/// Parse `git --version` output into (major, minor, patch).
///
/// Handles formats like "git version 2.39.0" and "git version 2.39.0.windows.1".
pub fn git_version() -> Result<(u32, u32, u32)> {
    let output = Command::new("git")
        .arg("--version")
        .output()
        .context("failed to run git --version")?;

    let version_str = String::from_utf8_lossy(&output.stdout);
    parse_git_version(&version_str)
}

fn parse_git_version(version_str: &str) -> Result<(u32, u32, u32)> {
    // Handles: "git version 2.39.0", "git version 2.39.0.windows.1",
    // and "git version 2.39.5 (Apple Git-154)".
    let version_part = version_str
        .trim()
        .strip_prefix("git version ")
        .ok_or_else(|| anyhow!("unexpected git --version format: {version_str}"))?;

    // Isolate the version number before any suffix like " (Apple Git-154)"
    let version_number = version_part
        .split_whitespace()
        .next()
        .unwrap_or(version_part);

    let parts: Vec<&str> = version_number.split('.').collect();
    if parts.len() < 3 {
        return Err(anyhow!("unexpected git version format: {version_str}"));
    }

    let major: u32 = parts[0].parse().context("bad major version")?;
    let minor: u32 = parts[1].parse().context("bad minor version")?;
    let patch: u32 = parts[2].parse().context("bad patch version")?;

    Ok((major, minor, patch))
}

/// Check if `refs/remotes/{remote}/HEAD` is set for a repo.
pub fn has_remote_head(repo_dir: &Path, remote: &str) -> bool {
    let head_ref = format!("refs/remotes/{remote}/HEAD");
    git_output(repo_dir, &["symbolic-ref", &head_ref]).is_ok()
}

/// Probe a remote for reachability and list its branches in a single network call.
///
/// Returns `Some(branches)` if the remote is reachable (branch list may be empty),
/// or `None` if the remote is unreachable. This replaces separate reachability and
/// branch-existence checks to avoid redundant network round-trips.
pub fn probe_remote_branches(repo_dir: &Path, remote: &str) -> Option<Vec<String>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_dir)
        .args(["ls-remote", "--heads", remote])
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let branches = stdout
        .lines()
        .filter_map(|line| {
            // Format: "<sha>\trefs/heads/<branch>"
            line.split('\t')
                .nth(1)?
                .strip_prefix("refs/heads/")
                .map(String::from)
        })
        .collect();

    Some(branches)
}

// ---------------------------------------------------------------------------
// Misc
// ---------------------------------------------------------------------------

pub fn is_git_repo(dir: &Path) -> bool {
    git_output(dir, &["rev-parse", "--git-dir"]).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_git_version_standard() {
        assert_eq!(parse_git_version("git version 2.39.0").unwrap(), (2, 39, 0));
    }

    #[test]
    fn parse_git_version_windows() {
        assert_eq!(
            parse_git_version("git version 2.43.0.windows.1").unwrap(),
            (2, 43, 0)
        );
    }

    #[test]
    fn parse_git_version_with_trailing_newline() {
        assert_eq!(
            parse_git_version("git version 2.30.1\n").unwrap(),
            (2, 30, 1)
        );
    }

    #[test]
    fn parse_git_version_apple_git() {
        assert_eq!(
            parse_git_version("git version 2.39.5 (Apple Git-154)").unwrap(),
            (2, 39, 5)
        );
    }

    #[test]
    fn parse_git_version_garbage() {
        assert!(parse_git_version("not git").is_err());
    }

    #[test]
    fn parse_git_version_too_few_parts() {
        assert!(parse_git_version("git version 2.39").is_err());
    }
}
