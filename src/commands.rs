use std::io::{IsTerminal, Write};
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, anyhow};
use colored::Colorize;

use crate::error::RigError;
use crate::git;
use crate::workspace::{self, Manifest, RepoEntry};

// ---------------------------------------------------------------------------
// create
// ---------------------------------------------------------------------------

pub fn create(name: &str, from: Option<&str>, skip: bool) -> Result<()> {
    let cwd = std::env::current_dir()?;
    create_from(&cwd, name, from, skip)
}

pub fn create_from(start_dir: &Path, name: &str, from: Option<&str>, skip: bool) -> Result<()> {
    let ws_dir = start_dir.join(name);

    if ws_dir.exists() {
        return Err(RigError::DirectoryAlreadyExists { path: ws_dir }.into());
    }

    if let Some(source_name) = from {
        return create_from_source(start_dir, name, &ws_dir, source_name, skip);
    }

    std::fs::create_dir_all(&ws_dir)?;

    let manifest = Manifest::new(name);
    manifest.save(&ws_dir)?;

    println!(
        "{} Created rig '{}' at {}",
        "ok".green(),
        name.bold(),
        ws_dir.display()
    );
    println!(
        "   Add repos with: {} or cd into it and run: {}",
        format!("git rig add {name} <path>").dimmed(),
        "git rig add <path>".dimmed()
    );

    Ok(())
}

fn create_from_source(
    start_dir: &Path,
    name: &str,
    ws_dir: &Path,
    source_name: &str,
    skip: bool,
) -> Result<()> {
    // Resolve source rig
    let (_, source_manifest) = workspace::resolve_workspace_from(start_dir, Some(source_name))?;

    // Pre-validate: check all source repo paths exist and are git repos
    let mut valid_entries = Vec::new();
    let mut invalid_entries: Vec<(String, String)> = Vec::new();

    for entry in &source_manifest.repos {
        if !entry.source.exists() {
            invalid_entries.push((
                entry.name.clone(),
                format!("source path not found: {}", entry.source.display()),
            ));
        } else if !git::is_git_repo(&entry.source) {
            invalid_entries.push((
                entry.name.clone(),
                format!("not a git repository: {}", entry.source.display()),
            ));
        } else {
            valid_entries.push(entry);
        }
    }

    if !invalid_entries.is_empty() {
        if skip {
            for (repo_name, reason) in &invalid_entries {
                println!(
                    "  {} Skipping '{}': {}",
                    "WARN".yellow(),
                    repo_name.bold(),
                    reason
                );
            }
            if valid_entries.is_empty() {
                return Err(anyhow!(
                    "no valid repos to clone from rig '{source_name}' (all {} skipped)",
                    invalid_entries.len()
                ));
            }
        } else {
            return Err(RigError::SourceReposInvalid {
                errors: invalid_entries,
            }
            .into());
        }
    }

    // Create the new rig directory + manifest
    std::fs::create_dir_all(ws_dir)?;
    let mut manifest = Manifest::new(name);
    manifest.save(ws_dir)?;

    println!(
        "Cloning rig '{}' -> '{}' ({} repos)\n",
        source_name.bold(),
        name.bold(),
        valid_entries.len()
    );

    // Add each repo from the source rig
    let mut errors: Vec<(String, String)> = Vec::new();

    for entry in &valid_entries {
        let detach = entry.branch == git::DETACHED;
        let result = add_repo_to_rig(
            ws_dir,
            &mut manifest,
            &entry.source,
            &entry.name,
            None, // branch defaults to rig/<new-name>
            &entry.remote,
            entry.upstream.as_deref(),
            detach,
        );

        match result {
            Ok(()) => println!("  {} {}", "ok".green(), entry.name.bold()),
            Err(e) => {
                println!("  {} {} ({})", "ERR".red(), entry.name.bold(), e);
                errors.push((entry.name.clone(), format!("{e}")));
            }
        }
    }

    println!();
    if errors.is_empty() {
        println!(
            "{} Created rig '{}' from '{}' ({} repos)",
            "ok".green(),
            name.bold(),
            source_name,
            valid_entries.len()
        );
    } else {
        let succeeded = valid_entries.len() - errors.len();
        println!(
            "{} Created rig '{}' from '{}' ({} repos added, {} failed)",
            "WARN".yellow(),
            name.bold(),
            source_name,
            succeeded,
            errors.len()
        );
        for (repo_name, err) in &errors {
            println!("  {} {}: {}", "ERR".red(), repo_name, err);
        }
        return Err(anyhow!("{} repo(s) failed to clone", errors.len()));
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// add
// ---------------------------------------------------------------------------

pub struct AddOptions<'a> {
    pub name: Option<&'a str>,
    pub branch: Option<&'a str>,
    pub remote: Option<&'a str>,
    pub detach: bool,
    pub upstream: Option<&'a str>,
    pub no_upstream: bool,
}

pub fn add(ws_name: Option<&str>, repo_path: &str, opts: AddOptions<'_>) -> Result<()> {
    let AddOptions {
        name,
        branch,
        remote,
        detach,
        upstream,
        no_upstream,
    } = opts;
    let (ws_dir, mut manifest) = workspace::resolve_workspace(ws_name)?;

    // Resolve the source repo path to absolute
    let source_dir = std::fs::canonicalize(repo_path)
        .with_context(|| format!("source repository not found at {repo_path}"))?;

    // Repo name defaults to directory basename
    let repo_name = name
        .map(str::to_string)
        .or_else(|| {
            source_dir
                .file_name()
                .map(|os| os.to_string_lossy().into_owned())
        })
        .ok_or_else(|| anyhow!("cannot determine repo name from path — use --name"))?;

    if manifest.has_repo(&repo_name) {
        if upstream.is_some() || no_upstream {
            let entry = manifest.find_repo_mut(&repo_name).unwrap();
            if no_upstream {
                entry.upstream = None;
                println!(
                    "{} Cleared upstream for '{}'",
                    "ok".green(),
                    repo_name.bold()
                );
            } else {
                let branch = upstream.unwrap().to_string();
                println!(
                    "{} Set upstream for '{}' to {}",
                    "ok".green(),
                    repo_name.bold(),
                    branch.cyan()
                );
                entry.upstream = Some(branch);
            }
            manifest.save(&ws_dir)?;
            return Ok(());
        }
        return Err(RigError::RepoAlreadyInRig {
            repo: repo_name,
            rig: manifest.name.clone(),
        }
        .into());
    }

    if !git::is_git_repo(&source_dir) {
        return Err(RigError::NotAGitRepo { path: source_dir }.into());
    }

    let remote = remote.unwrap_or("origin");

    add_repo_to_rig(
        &ws_dir,
        &mut manifest,
        &source_dir,
        &repo_name,
        branch,
        remote,
        upstream,
        detach,
    )?;

    println!(
        "{} Added '{}' to rig '{}'",
        "ok".green(),
        repo_name.bold(),
        manifest.name
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// shared: add a single repo worktree to a rig
// ---------------------------------------------------------------------------

/// Core logic for adding a repo worktree to a rig manifest.
///
/// Handles: fetch, default-branch detection, worktree creation (with
/// branch-existence checks), and `RepoEntry` construction. Used by both
/// `add` and `create --from`.
#[allow(clippy::too_many_arguments)]
fn add_repo_to_rig(
    ws_dir: &Path,
    manifest: &mut Manifest,
    source_dir: &Path,
    repo_name: &str,
    branch: Option<&str>,
    remote: &str,
    upstream: Option<&str>,
    detach: bool,
) -> Result<()> {
    // Fetch latest before creating the worktree
    print!("  Fetching {} ({})... ", repo_name.bold(), remote.dimmed());
    git::fetch(source_dir, remote)?;
    println!("{}", "ok".green());

    let default_branch = git::default_branch(source_dir, remote)?;
    let worktree_path = manifest.worktree_dir(ws_dir, repo_name);
    // When upstream is set, start the worktree from the upstream branch
    // so that git tracking and git log show the correct remote ref.
    let effective_start = upstream.unwrap_or(&default_branch);
    let start_point = format!("{remote}/{effective_start}");

    // If the worktree directory already exists (e.g., from a previous interrupted add),
    // skip worktree creation to make the operation retryable.
    let worktree_exists = worktree_path.exists();

    let recorded_branch = if worktree_exists {
        // Recover from a previous interrupted add
        println!("  Worktree already exists, recovering...");
        let b = git::current_branch(&worktree_path)?;
        if b == git::DETACHED {
            git::DETACHED.to_string()
        } else {
            b
        }
    } else if detach {
        println!(
            "  Creating worktree (detached at {})...",
            default_branch.dimmed()
        );
        git::worktree_add_detached(source_dir, &worktree_path, &start_point)?;
        git::DETACHED.to_string()
    } else {
        let branch_name = branch.map_or_else(|| format!("rig/{}", manifest.name), str::to_string);

        let branch_hint = || {
            format!(
                "branch '{}' may already be checked out in another worktree\n  \
                 hint: use --branch to specify a different branch name",
                branch_name
            )
        };

        if git::branch_exists(source_dir, &branch_name) {
            println!(
                "  Creating worktree (existing branch {})...",
                branch_name.cyan()
            );
            git::worktree_add_existing(source_dir, &worktree_path, &branch_name)
                .with_context(branch_hint)?;
        } else if git::remote_branch_exists(source_dir, &branch_name, remote) {
            println!(
                "  Creating worktree (tracking {remote}/{})...",
                branch_name.cyan()
            );
            git::worktree_add_new_branch(
                source_dir,
                &worktree_path,
                &branch_name,
                &format!("{remote}/{branch_name}"),
            )
            .with_context(branch_hint)?;
        } else {
            println!(
                "  Creating worktree (new branch {} from {})...",
                branch_name.cyan(),
                effective_start.dimmed()
            );
            git::worktree_add_new_branch(source_dir, &worktree_path, &branch_name, &start_point)
                .with_context(branch_hint)?;
        }

        branch_name
    };

    manifest.add_repo(RepoEntry {
        name: repo_name.to_string(),
        source: source_dir.to_path_buf(),
        branch: recorded_branch,
        default_branch,
        remote: remote.to_string(),
        upstream: upstream.map(str::to_string),
    });
    manifest.save(ws_dir)?;

    Ok(())
}

// ---------------------------------------------------------------------------
// worktree recovery helper
// ---------------------------------------------------------------------------

/// Remove a worktree with a 3-step recovery ladder:
/// 1. Try `git worktree remove [--force]`
/// 2. Try `git worktree repair` then retry remove
/// 3. Remove directory directly, then prune stale entries
///
/// The ordering in step 3 matters: `git worktree prune` only removes entries
/// whose directory is already gone, so we must delete the directory first.
fn remove_worktree_with_recovery(
    source_repo: &Path,
    worktree_path: &Path,
    force: bool,
) -> Result<()> {
    // Rung 1: normal remove
    if git::worktree_remove(source_repo, worktree_path, force).is_ok() {
        return Ok(());
    }

    // Rung 2: repair broken link, then retry
    println!(
        "  {} worktree not recognized, attempting repair...",
        "WARN".yellow()
    );
    if git::worktree_repair(source_repo, worktree_path).is_ok()
        && git::worktree_remove(source_repo, worktree_path, force).is_ok()
    {
        return Ok(());
    }

    // Rung 3: remove directory first, then prune stale metadata
    println!(
        "  {} repair failed, removing directory directly...",
        "WARN".yellow()
    );
    std::fs::remove_dir_all(worktree_path)
        .with_context(|| format!("failed to remove {}", worktree_path.display()))?;
    let _ = git::worktree_prune(source_repo);
    Ok(())
}

// ---------------------------------------------------------------------------
// remove
// ---------------------------------------------------------------------------

pub fn remove(ws_name: Option<&str>, repo: &str, force: bool, keep_branch: bool) -> Result<()> {
    let (ws_dir, mut manifest) = workspace::resolve_workspace(ws_name)?;

    let entry = manifest
        .find_repo(repo)
        .ok_or_else(|| RigError::RepoNotInRig {
            repo: repo.to_string(),
            rig: manifest.name.clone(),
        })?
        .clone();

    let worktree_path = manifest.worktree_dir(&ws_dir, repo);

    if worktree_path.exists() {
        if entry.source.exists() {
            if !force && git::is_dirty(&worktree_path)? {
                return Err(RigError::DirtyWorktree {
                    repo: repo.to_string(),
                }
                .into());
            }
            println!("  Removing worktree for {}...", repo.bold());
            remove_worktree_with_recovery(&entry.source, &worktree_path, force)?;
        } else {
            // Source repo is gone — skip git worktree remove, just clean up the directory
            println!(
                "  {} source repo missing, removing directory directly...",
                "WARN".yellow()
            );
            std::fs::remove_dir_all(&worktree_path)?;
        }
    }

    manifest.remove_repo(repo);
    manifest.save(&ws_dir)?;

    if !keep_branch && entry.branch != git::DETACHED {
        match git::delete_branch(&entry.source, &entry.branch) {
            Ok(()) => println!("  Deleted branch {}", entry.branch.cyan()),
            Err(e) => println!(
                "  {} Could not delete branch {}: {e}",
                "WARN".yellow(),
                entry.branch.cyan()
            ),
        }
    }

    println!(
        "{} Removed '{}' from rig '{}'",
        "ok".green(),
        repo.bold(),
        manifest.name
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// destroy
// ---------------------------------------------------------------------------

pub fn destroy(name: &str, dry_run: bool, yes: bool, keep_branches: bool) -> Result<()> {
    let cwd = std::env::current_dir()?;
    destroy_from(&cwd, name, dry_run, yes, keep_branches)
}

pub fn destroy_from(
    start_dir: &Path,
    name: &str,
    dry_run: bool,
    yes: bool,
    keep_branches: bool,
) -> Result<()> {
    let mut ws_dir = start_dir.join(name);

    // If not found at start_dir/<name>, try resolving via a parent workspace
    if !ws_dir.join(workspace::MANIFEST).exists()
        && let Ok((parent_ws_dir, _)) = workspace::resolve_workspace_from(start_dir, None)
        && let Some(parent) = parent_ws_dir.parent()
    {
        let candidate = parent.join(name);
        if candidate.join(workspace::MANIFEST).exists() {
            ws_dir = candidate;
        }
    }

    if !ws_dir.join(workspace::MANIFEST).exists() {
        return Err(RigError::RigNotFound {
            name: name.to_string(),
        }
        .into());
    }

    let manifest = Manifest::load(&ws_dir)?;

    if !dry_run && !yes {
        if std::io::stdin().is_terminal() {
            print!(
                "Destroy rig '{}' with {} repo(s)? [y/N] ",
                name,
                manifest.repos.len()
            );
            std::io::stdout().flush()?;
            let mut input = String::new();
            std::io::stdin().read_line(&mut input)?;
            if !input.trim_start().starts_with(['y', 'Y']) {
                println!("Aborted.");
                return Ok(());
            }
        } else {
            return Err(RigError::ConfirmationRequired.into());
        }
    }

    if dry_run {
        println!(
            "Would destroy rig '{}' ({} repos):",
            name.bold(),
            manifest.repos.len()
        );

        for repo in &manifest.repos {
            let worktree_path = manifest.worktree_dir(&ws_dir, &repo.name);

            if worktree_path.exists() {
                let dirty = git::is_dirty(&worktree_path).unwrap_or(false);
                let dirty_indicator = if dirty {
                    format!(" {}", "[dirty]".yellow())
                } else {
                    String::new()
                };
                println!(
                    "  Would remove worktree: {} ({}){}",
                    repo.name.bold(),
                    worktree_path.display(),
                    dirty_indicator
                );
                if !keep_branches && repo.branch != git::DETACHED {
                    println!("  Would delete branch: {}", repo.branch.cyan());
                }
            }
        }

        println!("  Would delete rig directory: {}", ws_dir.display());
        return Ok(());
    }

    println!(
        "Destroying rig '{}' ({} repos)...",
        name.bold(),
        manifest.repos.len()
    );

    let mut failed = 0usize;

    for repo in &manifest.repos {
        let worktree_path = manifest.worktree_dir(&ws_dir, &repo.name);

        if worktree_path.exists() {
            let dirty_warn = if git::is_dirty(&worktree_path).unwrap_or(false) {
                format!(" {}", "[dirty — uncommitted changes will be lost]".yellow())
            } else {
                String::new()
            };
            print!("  Removing {}{}... ", repo.name.bold(), dirty_warn);
            let remove_result = remove_worktree_with_recovery(&repo.source, &worktree_path, true);
            match remove_result {
                Ok(()) => {
                    println!("{}", "ok".green());
                    if !keep_branches && repo.branch != git::DETACHED {
                        match git::delete_branch(&repo.source, &repo.branch) {
                            Ok(()) => println!("    Deleted branch {}", repo.branch.cyan()),
                            Err(e) => println!(
                                "    {} Could not delete branch {}: {e}",
                                "WARN".yellow(),
                                repo.branch.cyan()
                            ),
                        }
                    }
                }
                Err(e) => {
                    println!("{}", "failed".red());
                    eprintln!("    {e}");
                    failed += 1;
                }
            }
        }
    }

    if failed > 0 {
        eprintln!(
            "{} Some worktrees could not be removed. Fix the issues above and retry.",
            "ERR".red()
        );
        return Err(anyhow!("{failed} worktree(s) could not be removed"));
    }

    std::fs::remove_dir_all(&ws_dir)?;
    println!("{} Destroyed rig '{}'", "ok".green(), name.bold());

    Ok(())
}

// ---------------------------------------------------------------------------
// list
// ---------------------------------------------------------------------------

pub fn list() -> Result<()> {
    let base_dir = workspace::resolve_base_dir()?;
    let workspaces = workspace::find_workspaces(&base_dir)?;

    if workspaces.is_empty() {
        println!("No rigs found in {}", base_dir.display());
        return Ok(());
    }

    println!("Rigs in {}:\n", base_dir.display());

    for ws in &workspaces {
        println!("  {} ({} repos)", ws.name.bold(), ws.repos.len());
        for repo in &ws.repos {
            if let Some(ref upstream) = repo.upstream {
                println!(
                    "    {} on {} {} {}",
                    repo.name,
                    repo.branch.cyan(),
                    "->".dimmed(),
                    upstream.cyan()
                );
            } else {
                println!("    {} on {}", repo.name, repo.branch.cyan());
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// status
// ---------------------------------------------------------------------------

pub fn status(name: Option<&str>) -> Result<()> {
    let (ws_dir, manifest) = workspace::resolve_workspace(name)?;

    println!("Rig: {} ({})\n", manifest.name.bold(), ws_dir.display());

    if manifest.repos.is_empty() {
        println!("  No repos. Add one with: git rig add <repo>");
        return Ok(());
    }

    for repo in &manifest.repos {
        let worktree_path = manifest.worktree_dir(&ws_dir, &repo.name);

        print!("  {}", repo.name.bold());

        if !worktree_path.exists() {
            println!(" {}", "(missing)".red());
            continue;
        }

        let branch = git::current_branch(&worktree_path).unwrap_or_else(|_| "(unknown)".into());
        let dirty = git::is_dirty(&worktree_path).unwrap_or(false);
        let effective = repo.effective_upstream();
        let (ahead, behind) = git::ahead_behind(&worktree_path, &branch, effective, &repo.remote);
        let last = git::last_commit_summary(&worktree_path).unwrap_or_else(|_| "no commits".into());

        print!(" on {}", branch.cyan());
        if dirty {
            print!(" {}", "[dirty]".yellow());
        }
        if ahead > 0 {
            print!(" {}", format!("+{ahead}").green());
        }
        if behind > 0 {
            print!(" {}", format!("-{behind}").red());
        }
        if repo.upstream.is_some() {
            print!(" {}", format!("(vs {effective})").dimmed());
        }
        println!();

        println!("    {} {}", "last:".dimmed(), last.dimmed());
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// refresh
// ---------------------------------------------------------------------------

#[allow(clippy::if_not_else)]
pub fn refresh(name: Option<&str>) -> Result<()> {
    let (ws_dir, mut manifest) = workspace::resolve_workspace(name)?;

    println!("Refreshing rig '{}'\n", manifest.name.bold());

    let mut updated = false;

    for repo in &mut manifest.repos {
        print!("  {}: ", repo.name.bold());

        if let Err(e) = git::fetch(&repo.source, &repo.remote) {
            println!("{} (fetch failed: {e})", "ERR".red());
            continue;
        }

        match git::default_branch(&repo.source, &repo.remote) {
            Ok(new_branch) => {
                if new_branch != repo.default_branch {
                    println!("{} → {}", repo.default_branch.dimmed(), new_branch.green());
                    repo.default_branch = new_branch;
                    updated = true;
                } else {
                    println!("{} (unchanged)", repo.default_branch.dimmed());
                }
            }
            Err(e) => {
                println!("{} (detect failed: {e})", "ERR".red());
            }
        }
    }

    if updated {
        manifest.save(&ws_dir)?;
    }

    println!();
    if updated {
        println!("{} Refreshed rig '{}'", "ok".green(), manifest.name);
    } else {
        println!("{} All default branches already up to date", "ok".green());
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// sync
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_lines)]
pub fn sync(name: Option<&str>, stash: bool) -> Result<()> {
    let (ws_dir, manifest) = workspace::resolve_workspace(name)?;

    println!("Syncing rig '{}'\n", manifest.name.bold());

    let mut errors: Vec<(String, String)> = Vec::new();

    for repo in &manifest.repos {
        let worktree_path = manifest.worktree_dir(&ws_dir, &repo.name);

        if !worktree_path.exists() {
            println!("  {} {} (missing, skipped)", "-".yellow(), repo.name.bold());
            continue;
        }

        if repo.branch == git::DETACHED {
            println!(
                "  {} {} (detached, skipped)",
                "-".yellow(),
                repo.name.bold()
            );
            continue;
        }

        let dirty = git::is_dirty(&worktree_path).unwrap_or(false);
        let mut stashed = false;

        if dirty && stash {
            match git::stash_push(&worktree_path) {
                Ok(did_stash) => stashed = did_stash,
                Err(e) => {
                    println!("  {} {} (stash failed: {e})", "ERR".red(), repo.name.bold());
                    errors.push((repo.name.clone(), format!("stash failed: {e}")));
                    continue;
                }
            }
        } else if dirty {
            println!(
                "  {} {} (dirty — use --stash to auto-stash)",
                "SKIP".yellow(),
                repo.name.bold()
            );
            continue;
        }

        // Snapshot HEAD before sync
        let before = git::rev_parse_short(&worktree_path, "HEAD").unwrap_or_default();

        // Fetch from the source repo (shares refs with worktree)
        if let Err(e) = git::fetch(&repo.source, &repo.remote) {
            println!("  {} {} (fetch failed: {e})", "ERR".red(), repo.name.bold());
            errors.push((repo.name.clone(), format!("fetch failed: {e}")));
            if stashed && let Err(e) = git::stash_pop(&worktree_path) {
                eprintln!(
                    "  {} stash pop failed for {}: {e} (changes still in git stash)",
                    "WARN".yellow(),
                    repo.name
                );
            }
            continue;
        }

        // Rebase worktree branch onto remote/<upstream>
        let effective = repo.effective_upstream();
        if git::rebase(&worktree_path, effective, &repo.remote).is_ok() {
            let after = git::rev_parse_short(&worktree_path, "HEAD").unwrap_or_default();
            let current =
                git::current_branch(&worktree_path).unwrap_or_else(|_| repo.branch.clone());
            let (_ahead, behind) =
                git::ahead_behind(&worktree_path, &current, effective, &repo.remote);

            let moved = if before == after {
                "already up to date".dimmed().to_string()
            } else {
                format!("{} -> {}", before.dimmed(), after.green())
            };

            let behind_info = if behind > 0 {
                format!(" (still {} behind)", format!("{behind}").red())
            } else {
                String::new()
            };

            let upstream_info = if repo.upstream.is_some() {
                format!(" {}", format!("(upstream: {effective})").dimmed())
            } else {
                String::new()
            };

            if stashed {
                match git::stash_pop(&worktree_path) {
                    Ok(()) => println!(
                        "  {} {} {}{}{} (stash restored)",
                        "ok".green(),
                        repo.name.bold(),
                        moved,
                        behind_info,
                        upstream_info
                    ),
                    Err(e) => println!(
                        "  {} {} {} (stash pop failed: {e})",
                        "WARN".yellow(),
                        repo.name.bold(),
                        moved
                    ),
                }
            } else {
                println!(
                    "  {} {} {}{}{}",
                    "ok".green(),
                    repo.name.bold(),
                    moved,
                    behind_info,
                    upstream_info
                );
            }
        } else {
            if let Err(e) = git::rebase_abort(&worktree_path) {
                eprintln!(
                    "  {} rebase abort failed for {}: {e}",
                    "WARN".yellow(),
                    repo.name
                );
            }
            if stashed && let Err(e) = git::stash_pop(&worktree_path) {
                eprintln!(
                    "  {} stash pop failed for {}: {e} (changes still in git stash)",
                    "WARN".yellow(),
                    repo.name
                );
            }
            println!(
                "  {} {} (rebase conflict — aborted)",
                "ERR".red(),
                repo.name.bold()
            );
            errors.push((repo.name.clone(), "rebase conflict".to_string()));
        }
    }

    println!();
    if errors.is_empty() {
        println!("{} All repos synced", "ok".green());
    } else {
        println!("{} {} repo(s) had issues:", "WARN".yellow(), errors.len());
        for (name, err) in &errors {
            println!("  {} {}: {}", "ERR".red(), name, err);
        }
        return Err(anyhow!("{} repo(s) had issues", errors.len()));
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// exec
// ---------------------------------------------------------------------------

pub fn exec(
    name: Option<&str>,
    filter_repos: &[String],
    cmd: &[String],
    fail_fast: bool,
) -> Result<()> {
    let (ws_dir, manifest) = workspace::resolve_workspace(name)?;

    // Validate --repo filters against manifest
    for r in filter_repos {
        if manifest.find_repo(r).is_none() {
            return Err(RigError::RepoNotInRig {
                repo: r.to_string(),
                rig: manifest.name.clone(),
            }
            .into());
        }
    }

    let repos: Vec<_> = manifest
        .repos
        .iter()
        .filter(|r| filter_repos.is_empty() || filter_repos.iter().any(|f| f == &r.name))
        .collect();

    let mut errors: Vec<(String, String)> = Vec::new();

    for repo in &repos {
        let worktree_path = manifest.worktree_dir(&ws_dir, &repo.name);

        println!("{} {}", ">>>".bold(), repo.name.bold());

        if !worktree_path.exists() {
            println!("{} worktree missing, skipped", "WARN".yellow());
            println!();
            continue;
        }

        let status = Command::new(&cmd[0])
            .args(&cmd[1..])
            .current_dir(&worktree_path)
            .status();

        match status {
            Ok(s) if s.success() => {}
            Ok(s) => {
                let code = s.code().unwrap_or(-1);
                errors.push((repo.name.clone(), format!("exit code {code}")));
                if fail_fast {
                    break;
                }
            }
            Err(e) => {
                errors.push((repo.name.clone(), format!("failed to execute: {e}")));
                if fail_fast {
                    break;
                }
            }
        }

        println!();
    }

    if !errors.is_empty() {
        println!("{} {} repo(s) had errors:", "WARN".yellow(), errors.len());
        for (name, err) in &errors {
            println!("  {} {}: {}", "ERR".red(), name, err);
        }
        return Err(anyhow!("{} repo(s) had errors", errors.len()));
    }

    Ok(())
}
