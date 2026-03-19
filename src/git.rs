use anyhow::{Context, Result, anyhow};
use std::path::Path;
use std::process::Command;

/// Run a git command, capture and return stdout. Errors on non-zero exit.
fn git_output(repo_dir: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_dir)
        .args(args)
        .output()
        .with_context(|| format!("failed to run git {:?}", args))?;

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
        .with_context(|| format!("failed to run git {:?}", args))?;

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
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_dir)
        .args(args)
        .output()
        .with_context(|| format!("failed to run git {:?}", args))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!("git {:?} failed: {}", args, stderr.trim()));
    }

    Ok(())
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

    Err(anyhow!(
        "cannot determine default branch for {}",
        repo_dir.display()
    ))
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
        Ok("(detached)".to_string())
    } else {
        Ok(branch)
    }
}

// ---------------------------------------------------------------------------
// Worktree operations
// ---------------------------------------------------------------------------

/// Create a worktree with a new branch starting from `start_point`.
pub fn worktree_add_new_branch(
    source_repo: &Path,
    worktree_path: &Path,
    branch: &str,
    start_point: &str,
) -> Result<()> {
    let path_str = worktree_path
        .to_str()
        .ok_or_else(|| anyhow!("non-UTF8 path"))?;
    git_run(
        source_repo,
        &["worktree", "add", "-b", branch, path_str, start_point],
    )
}

/// Create a worktree checking out an existing branch.
pub fn worktree_add_existing(source_repo: &Path, worktree_path: &Path, branch: &str) -> Result<()> {
    let path_str = worktree_path
        .to_str()
        .ok_or_else(|| anyhow!("non-UTF8 path"))?;
    git_run(source_repo, &["worktree", "add", path_str, branch])
}

/// Create a detached worktree at a specific commit.
pub fn worktree_add_detached(source_repo: &Path, worktree_path: &Path, commit: &str) -> Result<()> {
    let path_str = worktree_path
        .to_str()
        .ok_or_else(|| anyhow!("non-UTF8 path"))?;
    git_run(
        source_repo,
        &["worktree", "add", "--detach", path_str, commit],
    )
}

/// Remove a worktree. Use `force` to remove even if dirty.
pub fn worktree_remove(source_repo: &Path, worktree_path: &Path, force: bool) -> Result<()> {
    let path_str = worktree_path
        .to_str()
        .ok_or_else(|| anyhow!("non-UTF8 path"))?;
    if force {
        git_run(source_repo, &["worktree", "remove", "--force", path_str])
    } else {
        git_run(source_repo, &["worktree", "remove", path_str])
    }
}

// ---------------------------------------------------------------------------
// Status helpers
// ---------------------------------------------------------------------------

pub fn is_dirty(repo_dir: &Path) -> Result<bool> {
    let output = git_output(repo_dir, &["status", "--porcelain"])?;
    Ok(!output.is_empty())
}

/// Returns (ahead, behind) relative to `<remote>/<remote_branch>`.
pub fn ahead_behind(
    repo_dir: &Path,
    local: &str,
    remote_branch: &str,
    remote: &str,
) -> Result<(u32, u32)> {
    let range = format!("{remote}/{remote_branch}...{local}");
    match git_output(repo_dir, &["rev-list", "--left-right", "--count", &range]) {
        Ok(output) => {
            let parts: Vec<&str> = output.split_whitespace().collect();
            if parts.len() == 2 {
                let behind = parts[0].parse().unwrap_or(0);
                let ahead = parts[1].parse().unwrap_or(0);
                Ok((ahead, behind))
            } else {
                Ok((0, 0))
            }
        }
        Err(_) => Ok((0, 0)),
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

pub fn stash_push(repo_dir: &Path) -> Result<bool> {
    let before = git_output(repo_dir, &["stash", "list"])?;
    git_quiet(repo_dir, &["stash", "push", "-m", "ws sync auto-stash"])?;
    let after = git_output(repo_dir, &["stash", "list"])?;
    // If the stash list changed, something was stashed
    Ok(before != after)
}

pub fn stash_pop(repo_dir: &Path) -> Result<()> {
    git_run(repo_dir, &["stash", "pop"])
}

// ---------------------------------------------------------------------------
// Misc
// ---------------------------------------------------------------------------

pub fn is_git_repo(dir: &Path) -> bool {
    git_output(dir, &["rev-parse", "--git-dir"]).is_ok()
}
