use std::io::IsTerminal;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, anyhow};
use colored::Colorize;

use crate::git;
use crate::workspace::{self, Manifest, RepoEntry};

// ---------------------------------------------------------------------------
// create
// ---------------------------------------------------------------------------

pub fn create(name: &str) -> Result<()> {
    let cwd = std::env::current_dir()?;
    create_from(&cwd, name)
}

pub fn create_from(start_dir: &Path, name: &str) -> Result<()> {
    let ws_dir = start_dir.join(name);

    if ws_dir.exists() {
        return Err(anyhow!("directory '{}' already exists", ws_dir.display()));
    }

    std::fs::create_dir_all(&ws_dir)?;

    let manifest = Manifest::new(name);
    manifest.save(&ws_dir)?;

    println!(
        "{} Created workspace '{}' at {}",
        "ok".green(),
        name.bold(),
        ws_dir.display()
    );
    println!(
        "   Add repos with: {} or cd into it and run: {}",
        format!("ws add {name} <path>").dimmed(),
        "ws add <path>".dimmed()
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// add
// ---------------------------------------------------------------------------

pub fn add(
    ws_name: Option<&str>,
    repo_path: &str,
    name: Option<&str>,
    branch: Option<&str>,
    remote: Option<&str>,
    detach: bool,
) -> Result<()> {
    let (ws_dir, mut manifest) = workspace::resolve_workspace(ws_name)?;

    // Resolve the source repo path to absolute
    let source_dir = std::fs::canonicalize(repo_path)
        .with_context(|| format!("source repository not found at {repo_path}"))?;

    // Repo name defaults to directory basename
    let repo_name = name
        .map(|n| n.to_string())
        .or_else(|| {
            source_dir
                .file_name()
                .map(|os| os.to_string_lossy().to_string())
        })
        .ok_or_else(|| anyhow!("cannot determine repo name from path — use --name"))?;

    if manifest.has_repo(&repo_name) {
        return Err(anyhow!(
            "'{}' is already in workspace '{}'",
            repo_name,
            manifest.name
        ));
    }

    if !git::is_git_repo(&source_dir) {
        return Err(anyhow!("{} is not a git repository", source_dir.display()));
    }

    let remote = remote.unwrap_or("origin");

    // Fetch latest before creating the worktree
    print!("  Fetching {} ({})... ", repo_name.bold(), remote.dimmed());
    git::fetch(&source_dir, remote)?;
    println!("{}", "ok".green());

    let default_branch = git::default_branch(&source_dir, remote)?;
    let worktree_path = manifest.worktree_dir(&ws_dir, &repo_name);
    let start_point = format!("{remote}/{default_branch}");

    let recorded_branch = if detach {
        println!(
            "  Creating worktree (detached at {})...",
            default_branch.dimmed()
        );
        git::worktree_add_detached(&source_dir, &worktree_path, &start_point)?;
        git::DETACHED.to_string()
    } else {
        let branch_name = branch
            .map(|b| b.to_string())
            .unwrap_or_else(|| format!("ws/{}", manifest.name));

        if git::branch_exists(&source_dir, &branch_name) {
            println!(
                "  Creating worktree (existing branch {})...",
                branch_name.cyan()
            );
            git::worktree_add_existing(&source_dir, &worktree_path, &branch_name)?;
        } else if git::remote_branch_exists(&source_dir, &branch_name, remote) {
            println!(
                "  Creating worktree (tracking {remote}/{})...",
                branch_name.cyan()
            );
            git::worktree_add_new_branch(
                &source_dir,
                &worktree_path,
                &branch_name,
                &format!("{remote}/{branch_name}"),
            )?;
        } else {
            println!(
                "  Creating worktree (new branch {} from {})...",
                branch_name.cyan(),
                default_branch.dimmed()
            );
            git::worktree_add_new_branch(&source_dir, &worktree_path, &branch_name, &start_point)?;
        }

        branch_name
    };

    manifest.add_repo(RepoEntry {
        name: repo_name.clone(),
        source: source_dir,
        branch: recorded_branch,
        default_branch,
        remote: remote.to_string(),
    });
    manifest.save(&ws_dir)?;

    println!(
        "{} Added '{}' to workspace '{}'",
        "ok".green(),
        repo_name.bold(),
        manifest.name
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// remove
// ---------------------------------------------------------------------------

pub fn remove(ws_name: Option<&str>, repo: &str, force: bool, delete_branch: bool) -> Result<()> {
    let (ws_dir, mut manifest) = workspace::resolve_workspace(ws_name)?;

    let entry = manifest
        .find_repo(repo)
        .ok_or_else(|| anyhow!("'{}' is not in workspace '{}'", repo, manifest.name))?
        .clone();

    let worktree_path = manifest.worktree_dir(&ws_dir, repo);

    if worktree_path.exists() {
        if !force && git::is_dirty(&worktree_path)? {
            return Err(anyhow!(
                "'{}' has uncommitted changes — use --force to remove anyway",
                repo
            ));
        }
        println!("  Removing worktree for {}...", repo.bold());
        git::worktree_remove(&entry.source, &worktree_path, force)?;
    }

    manifest.remove_repo(repo);
    manifest.save(&ws_dir)?;

    if delete_branch && entry.branch != git::DETACHED {
        match git::delete_branch(&entry.source, &entry.branch) {
            Ok(_) => println!("  Deleted branch {}", entry.branch.cyan()),
            Err(e) => println!(
                "  {} Could not delete branch {}: {e}",
                "WARN".yellow(),
                entry.branch.cyan()
            ),
        }
    }

    println!(
        "{} Removed '{}' from workspace '{}'",
        "ok".green(),
        repo.bold(),
        manifest.name
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// destroy
// ---------------------------------------------------------------------------

pub fn destroy(name: &str, dry_run: bool, yes: bool) -> Result<()> {
    let cwd = std::env::current_dir()?;
    destroy_from(&cwd, name, dry_run, yes)
}

pub fn destroy_from(start_dir: &Path, name: &str, dry_run: bool, yes: bool) -> Result<()> {
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
        return Err(anyhow!("workspace '{}' not found", name));
    }

    let manifest = Manifest::load(&ws_dir)?;

    if !dry_run && !yes {
        if std::io::stdin().is_terminal() {
            print!(
                "Destroy workspace '{}' with {} repo(s)? [y/N] ",
                name,
                manifest.repos.len()
            );
            use std::io::Write;
            std::io::stdout().flush()?;
            let mut input = String::new();
            std::io::stdin().read_line(&mut input)?;
            if !input.trim_start().starts_with(['y', 'Y']) {
                println!("Aborted.");
                return Ok(());
            }
        } else {
            return Err(anyhow!("use --yes to confirm (stdin is not a terminal)"));
        }
    }

    if dry_run {
        println!(
            "Would destroy workspace '{}' ({} repos):",
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
            }
        }

        println!("  Would delete workspace directory: {}", ws_dir.display());
        return Ok(());
    }

    println!(
        "Destroying workspace '{}' ({} repos)...",
        name.bold(),
        manifest.repos.len()
    );

    let mut failed = 0usize;

    for repo in &manifest.repos {
        let worktree_path = manifest.worktree_dir(&ws_dir, &repo.name);

        if worktree_path.exists() {
            print!("  Removing {}... ", repo.name.bold());
            match git::worktree_remove(&repo.source, &worktree_path, true) {
                Ok(_) => println!("{}", "ok".green()),
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
    println!("{} Destroyed workspace '{}'", "ok".green(), name.bold());

    Ok(())
}

// ---------------------------------------------------------------------------
// list
// ---------------------------------------------------------------------------

pub fn list() -> Result<()> {
    let base_dir = workspace::resolve_base_dir()?;
    let workspaces = workspace::find_workspaces(&base_dir)?;

    if workspaces.is_empty() {
        println!("No workspaces found in {}", base_dir.display());
        return Ok(());
    }

    println!("Workspaces in {}:\n", base_dir.display());

    for ws in &workspaces {
        println!("  {} ({} repos)", ws.name.bold(), ws.repos.len());
        for repo in &ws.repos {
            println!("    {} on {}", repo.name, repo.branch.cyan());
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// status
// ---------------------------------------------------------------------------

pub fn status(name: Option<&str>) -> Result<()> {
    let (ws_dir, manifest) = workspace::resolve_workspace(name)?;

    println!(
        "Workspace: {} ({})\n",
        manifest.name.bold(),
        ws_dir.display()
    );

    if manifest.repos.is_empty() {
        println!("  No repos. Add one with: ws add <repo>");
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
        let (ahead, behind) =
            git::ahead_behind(&worktree_path, &branch, &repo.default_branch, &repo.remote)
                .unwrap_or((0, 0));
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
        println!();

        println!("    {} {}", "last:".dimmed(), last.dimmed());
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// refresh
// ---------------------------------------------------------------------------

pub fn refresh(name: Option<&str>) -> Result<()> {
    let (ws_dir, mut manifest) = workspace::resolve_workspace(name)?;

    println!("Refreshing workspace '{}'\n", manifest.name.bold());

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
        println!("{} Refreshed workspace '{}'", "ok".green(), manifest.name);
    } else {
        println!("{} All default branches already up to date", "ok".green());
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// sync
// ---------------------------------------------------------------------------

pub fn sync(name: Option<&str>, stash: bool) -> Result<()> {
    let (ws_dir, manifest) = workspace::resolve_workspace(name)?;

    println!("Syncing workspace '{}'\n", manifest.name.bold());

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
            if stashed {
                if let Err(e) = git::stash_pop(&worktree_path) {
                    eprintln!("  {} stash pop failed for {}: {e} (changes still in git stash)", "WARN".yellow(), repo.name);
                }
            }
            continue;
        }

        // Rebase worktree branch onto origin/<default>
        match git::rebase(&worktree_path, &repo.default_branch, &repo.remote) {
            Ok(_) => {
                let after = git::rev_parse_short(&worktree_path, "HEAD").unwrap_or_default();
                let current = git::current_branch(&worktree_path).unwrap_or_else(|_| repo.branch.clone());
                let (_ahead, behind) = git::ahead_behind(
                    &worktree_path,
                    &current,
                    &repo.default_branch,
                    &repo.remote,
                )
                .unwrap_or((0, 0));

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

                if stashed {
                    match git::stash_pop(&worktree_path) {
                        Ok(_) => println!(
                            "  {} {} {}{} (stash restored)",
                            "ok".green(),
                            repo.name.bold(),
                            moved,
                            behind_info
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
                        "  {} {} {}{}",
                        "ok".green(),
                        repo.name.bold(),
                        moved,
                        behind_info
                    );
                }
            }
            Err(_) => {
                if let Err(e) = git::rebase_abort(&worktree_path) {
                    eprintln!("  {} rebase abort failed for {}: {e}", "WARN".yellow(), repo.name);
                }
                if stashed {
                    if let Err(e) = git::stash_pop(&worktree_path) {
                        eprintln!("  {} stash pop failed for {}: {e} (changes still in git stash)", "WARN".yellow(), repo.name);
                    }
                }
                println!(
                    "  {} {} (rebase conflict — aborted)",
                    "ERR".red(),
                    repo.name.bold()
                );
                errors.push((repo.name.clone(), "rebase conflict".to_string()));
            }
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
            return Err(anyhow!("'{}' is not in workspace '{}'", r, manifest.name));
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
