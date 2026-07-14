use std::io::{IsTerminal, Write};
use std::path::Path;
use std::process::Command;
use std::sync::atomic::AtomicBool;

use anyhow::{Context, Result, anyhow};
use colored::Colorize;

use crate::drift;
use crate::error::RigError;
use crate::git;
use crate::provision::{self, ProvisionOpts};
use crate::workspace::{self, Manifest, RepoEntry};

// ---------------------------------------------------------------------------
// parallel job resolution
// ---------------------------------------------------------------------------

const AUTO_JOBS_CAP: usize = 8;

/// Resolve effective job count: CLI flag > manifest > auto.
/// Returns 1 for sequential, >1 for parallel.
pub(crate) fn resolve_jobs(
    cli_jobs: Option<usize>,
    manifest: &Manifest,
    repo_count: usize,
) -> usize {
    let base = cli_jobs.unwrap_or_else(|| manifest.jobs.unwrap_or(0));
    if base == 0 {
        repo_count.clamp(1, AUTO_JOBS_CAP)
    } else {
        base
    }
}

// ---------------------------------------------------------------------------
// create
// ---------------------------------------------------------------------------

pub fn create(
    name: &str,
    from: Option<&str>,
    skip: bool,
    provision: Option<ProvisionOpts>,
) -> Result<()> {
    let cwd = std::env::current_dir()?;
    create_from(&cwd, name, from, skip, provision)
}

pub fn create_from(
    start_dir: &Path,
    name: &str,
    from: Option<&str>,
    skip: bool,
    provision: Option<ProvisionOpts>,
) -> Result<()> {
    let ws_dir = start_dir.join(name);

    if ws_dir.exists() {
        return Err(RigError::DirectoryAlreadyExists { path: ws_dir }.into());
    }

    if let Some(source_name) = from {
        return create_from_source(start_dir, name, &ws_dir, source_name, skip, &provision);
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
    provision: &Option<ProvisionOpts>,
) -> Result<()> {
    // Resolve source rig
    let (source_ws_dir, source_manifest) =
        workspace::resolve_workspace_from(start_dir, Some(source_name))?;

    // Pre-validate: check all source repo paths exist and are git repos
    let mut valid_entries = Vec::new();
    let mut invalid_entries: Vec<(String, String)> = Vec::new();

    for entry in source_manifest.repos_sorted() {
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

    // repos_sorted() guarantees alphabetical order
    let mut errors: Vec<(String, String)> = Vec::new();

    for entry in &valid_entries {
        let detach = entry.branch == git::DETACHED;
        // Provision from source rig's worktree, not the base clone
        let provision_source = provision.as_ref().and_then(|_| {
            let worktree_path = source_ws_dir.join(&entry.name);
            worktree_path.is_dir().then_some(worktree_path)
        });
        let result = add_repo_to_rig(
            ws_dir,
            &mut manifest,
            &entry.source,
            &entry.name,
            None, // branch defaults to rig/<new-name>
            &entry.remote,
            entry.upstream.as_deref(),
            detach,
            provision_source.as_deref(),
            provision,
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

pub fn add(
    ws_name: Option<&str>,
    repo_path: &str,
    opts: AddOptions<'_>,
    provision: Option<ProvisionOpts>,
) -> Result<()> {
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

    let provision_source = provision.as_ref().map(|_| source_dir.as_path());

    add_repo_to_rig(
        &ws_dir,
        &mut manifest,
        &source_dir,
        &repo_name,
        branch,
        remote,
        upstream,
        detach,
        provision_source,
        &provision,
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
    provision_source: Option<&Path>,
    provision_opts: &Option<ProvisionOpts>,
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
            let location = git::find_worktree_for_branch(source_dir, &branch_name)
                .map(|p| format!("\n  checked out in: {p}"))
                .unwrap_or_default();
            format!(
                "branch '{}' is already checked out in another worktree{location}\n  \
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

    // Provision local files from .riginclude (after worktree creation, before manifest save).
    // Provisioning failures are intentionally warnings: file copying is auxiliary to
    // workspace creation and should not fail the add/create command.
    if let Some(prov_source) = provision_source
        && let Some(opts) = provision_opts
        && let Some(report) = provision::provision_files(prov_source, &worktree_path, opts)
    {
        provision::print_provision_report(&report);
    }

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

        for repo in manifest.repos_sorted() {
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

    for repo in manifest.repos_sorted() {
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
        for repo in ws.repos_sorted() {
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

    let report = drift::check_drift(&manifest, &ws_dir);
    drift::print_drift_warnings(&report, &[], false);

    println!("Rig: {} ({})\n", manifest.name.bold(), ws_dir.display());

    if manifest.repos.is_empty() {
        println!("  No repos. Add one with: git rig add <repo>");
        return Ok(());
    }

    for repo in manifest.repos_sorted() {
        let worktree_path = manifest.worktree_dir(&ws_dir, &repo.name);

        print!("  {}", repo.name.bold());

        if report.has_worktree_unavailable(&repo.name) {
            println!(" {}", "(missing)".red());
            continue;
        }

        let branch = report
            .branches
            .get(&repo.name)
            .expect("branch should be cached if worktree is reachable")
            .clone();
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
pub fn refresh(name: Option<&str>, cli_jobs: Option<usize>) -> Result<()> {
    let (ws_dir, mut manifest) = workspace::resolve_workspace(name)?;

    let report = drift::check_drift(&manifest, &ws_dir);
    drift::print_drift_warnings(&report, &[], true);

    // Collect active repos by cloning (avoids borrow conflict with mut manifest)
    let active_repos: Vec<RepoEntry> = manifest
        .repos_sorted()
        .into_iter()
        .filter(|repo| !report.has_source_missing(&repo.name))
        .cloned()
        .collect();

    let jobs = resolve_jobs(cli_jobs, &manifest, active_repos.len());

    println!("Refreshing rig '{}'\n", manifest.name.bold());

    if jobs <= 1 {
        return refresh_sequential(&mut manifest, &ws_dir, &active_repos);
    }

    refresh_parallel(&mut manifest, &ws_dir, &active_repos, jobs)
}

fn refresh_sequential(
    manifest: &mut Manifest,
    ws_dir: &std::path::Path,
    active_repos: &[RepoEntry],
) -> Result<()> {
    let mut updated = false;
    let name_width = active_repos.iter().map(|r| r.name.len()).max().unwrap_or(0);

    for repo in active_repos {
        let padded = format!("{:<width$}", repo.name, width = name_width).bold();

        if let Err(e) = git::fetch(&repo.source, &repo.remote) {
            println!("  {padded} {} (fetch failed: {e})", "ERR".red());
            continue;
        }

        match git::default_branch(&repo.source, &repo.remote) {
            Ok(new_branch) => {
                if new_branch != repo.default_branch {
                    println!(
                        "  {padded} {} → {}",
                        repo.default_branch.dimmed(),
                        new_branch.green()
                    );
                    if let Some(entry) = manifest.find_repo_mut(&repo.name) {
                        entry.default_branch = new_branch;
                    }
                    updated = true;
                } else {
                    println!("  {padded} {} (unchanged)", repo.default_branch.dimmed());
                }
            }
            Err(e) => {
                println!("  {padded} {} (detect failed: {e})", "ERR".red());
            }
        }
    }

    finish_refresh(manifest, ws_dir, updated)
}

fn finish_refresh(manifest: &mut Manifest, ws_dir: &Path, updated: bool) -> Result<()> {
    if updated {
        manifest.save(ws_dir)?;
    }

    println!();
    if updated {
        println!("{} Refreshed rig '{}'", "ok".green(), manifest.name);
    } else {
        println!("{} All default branches already up to date", "ok".green());
    }

    Ok(())
}

fn refresh_parallel(
    manifest: &mut Manifest,
    ws_dir: &std::path::Path,
    active_repos: &[RepoEntry],
    jobs: usize,
) -> Result<()> {
    let fetch_cache = crate::parallel::FetchCache::new();

    let repo_names: Vec<String> = active_repos.iter().map(|r| r.name.clone()).collect();
    let cancel = AtomicBool::new(false);

    // Each Ok result is (old_branch, new_branch)
    let results = crate::parallel::run_parallel(&repo_names, jobs, &cancel, |idx, progress| {
        let repo = &active_repos[idx];

        // Fetch with deduplication per (source, remote) pair
        progress.set_status("fetching...");
        let fetch_result = fetch_cache.fetch_once(&repo.source, &repo.remote, || {
            git::fetch(&repo.source, &repo.remote).map_err(|e| e.to_string())
        });

        if let Err(e) = fetch_result {
            return Err(format!("fetch failed: {e}"));
        }

        // Detect default branch
        progress.set_status("detecting default branch...");
        match git::default_branch(&repo.source, &repo.remote) {
            Ok(new_branch) => Ok((repo.default_branch.clone(), new_branch)),
            Err(e) => Err(format!("detect failed: {e}")),
        }
    });

    // Sequential merge: apply updates to manifest
    let name_width = results.iter().map(|(n, _)| n.len()).max().unwrap_or(0);
    let mut updated = false;
    for (name, result) in &results {
        let padded = format!("{:<width$}", name, width = name_width).bold();
        match result {
            Ok((old_branch, new_branch)) => {
                if old_branch != new_branch {
                    println!(
                        "  {padded} {} → {}",
                        old_branch.dimmed(),
                        new_branch.green()
                    );
                    if let Some(entry) = manifest.find_repo_mut(name) {
                        entry.default_branch = new_branch.clone();
                    }
                    updated = true;
                } else {
                    println!("  {padded} {} (unchanged)", old_branch.dimmed());
                }
            }
            Err(e) => {
                println!("  {padded} {} ({e})", "ERR".red());
            }
        }
    }

    finish_refresh(manifest, ws_dir, updated)
}

// ---------------------------------------------------------------------------
// sync
// ---------------------------------------------------------------------------

/// Outcome of a per-repo sync operation (used by both sequential and parallel paths).
enum SyncOutcome {
    Synced(String),
    DirtySkipped,
    Detached,
    /// Upstream target could not be resolved — non-fatal skip.
    TargetSkipped(String),
    /// Rebase conflicted but the branch's work is provably landed upstream.
    /// Carries what the deferred remediation pass needs.
    Reconcilable {
        target_branch: String,
        upstream_gone: bool,
    },
}

/// A repo whose work is provably landed upstream, awaiting the deferred
/// (sequential, consent-gated) remediation pass. Owned copies of the fields
/// remediation needs, so it never has to borrow the manifest.
struct Reconcilable {
    name: String,
    remote: String,
    branch: String,
    /// Remote branch name to reset onto (may differ from the manifest upstream
    /// when the upstream is gone and we fell back to the default branch).
    target_branch: String,
    upstream_gone: bool,
}

/// What the classify pass produced. The final summary and remediation are
/// driven from this, so classification stays free of manifest mutation.
struct ClassifyResults {
    dirty_skipped: Vec<String>,
    target_skipped: Vec<(String, String)>,
    errors: Vec<(String, String)>,
    reconcilables: Vec<Reconcilable>,
}

/// What the deferred remediation pass produced.
struct RemediationResults {
    reconciled: Vec<String>,
    untouched: Vec<String>,
    errors: Vec<(String, String)>,
}

/// The comparison target `sync` rebases onto and classifies against.
///
/// See the spec's "Comparison Target" table: a safe-degrading chain that reuses
/// sync's rebase ref, resolved from remote-tracking refs *after* `fetch --prune`.
struct SyncTarget {
    /// Remote branch name (e.g. `master`); the full ref is `{remote}/{branch}`.
    branch: String,
    /// The configured upstream ref was gone after prune and we fell back to the
    /// default branch — the manifest `upstream` field should be repaired.
    upstream_gone: bool,
}

/// Resolve the branch `sync` should rebase onto, per the comparison-target chain.
///
/// `Err` is a **non-fatal** skip reason (the repo is left alone), never a hard
/// error that would strand the rest of the sync.
fn resolve_sync_target(
    repo: &RepoEntry,
    worktree: &Path,
) -> std::result::Result<SyncTarget, String> {
    let effective = repo.effective_upstream();
    if git::remote_branch_exists(worktree, effective, &repo.remote) {
        return Ok(SyncTarget {
            branch: effective.to_string(),
            upstream_gone: false,
        });
    }

    // The configured upstream ref is gone after prune. A post-merge topic branch
    // almost always merged into the default branch, so fall back to it rather
    // than stranding the user this feature exists for.
    let default = &repo.default_branch;
    if effective != default && git::remote_branch_exists(worktree, default, &repo.remote) {
        return Ok(SyncTarget {
            branch: default.clone(),
            upstream_gone: true,
        });
    }

    Err("cannot determine upstream target".to_string())
}

/// The authoritative LANDED predicate: an in-memory 3-way merge of `branch` into
/// `target_ref` yields a tree byte-identical to the target — so merging the
/// branch adds nothing and it is provably redundant. Any failure (old git, bad
/// ref) degrades to `false`, i.e. today's "genuine conflict" behavior.
fn is_landed(worktree: &Path, target_ref: &str, branch: &str) -> bool {
    match git::merge_tree(worktree, target_ref, branch) {
        Ok(git::MergeTreeOutcome::Clean { tree }) => {
            git::tree_oid(worktree, target_ref).is_ok_and(|target_tree| tree == target_tree)
        }
        _ => false,
    }
}

fn print_sync_summary(classify: &ClassifyResults, remediation: &RemediationResults) -> Result<()> {
    println!();

    if !classify.dirty_skipped.is_empty() {
        println!(
            "{} {} repo(s) skipped (dirty — use {} to auto-stash):",
            "WARN".yellow(),
            classify.dirty_skipped.len(),
            "--stash".bold()
        );
        for name in &classify.dirty_skipped {
            println!("  {} {}", "WARN".yellow(), name);
        }
    }

    if !classify.target_skipped.is_empty() {
        println!(
            "{} {} repo(s) skipped (upstream unresolvable):",
            "WARN".yellow(),
            classify.target_skipped.len()
        );
        for (name, reason) in &classify.target_skipped {
            println!("  {} {}: {}", "WARN".yellow(), name, reason);
        }
    }

    // Reconciliation summary.
    let reconciled = remediation.reconciled.len();
    let left = remediation.untouched.len();
    if reconciled > 0 {
        println!("{} {reconciled} branch(es) reconciled", "ok".green());
    }
    if left > 0 {
        println!(
            "{} {left} reconcilable branch(es) left untouched — run {}",
            "~".cyan(),
            "git rig sync --reconcile".bold(),
        );
    }

    // Errors: genuine conflicts + fetch/stash failures + remediation failures.
    let mut errors: Vec<&(String, String)> = classify.errors.iter().collect();
    errors.extend(remediation.errors.iter());
    if !errors.is_empty() {
        println!("{} {} repo(s) had issues:", "WARN".yellow(), errors.len());
        for (name, err) in &errors {
            println!("  {} {}: {}", "ERR".red(), name, err);
        }
        return Err(anyhow!("{} repo(s) had issues", errors.len()));
    }

    if classify.dirty_skipped.is_empty()
        && classify.target_skipped.is_empty()
        && reconciled == 0
        && left == 0
    {
        println!("{} All repos synced", "ok".green());
    }

    Ok(())
}

pub fn sync(
    name: Option<&str>,
    filter_repos: &[String],
    stash: bool,
    reconcile: bool,
    cli_jobs: Option<usize>,
) -> Result<()> {
    let (ws_dir, mut manifest) = workspace::resolve_workspace(name)?;
    manifest.validate_repo_filter(filter_repos)?;

    let report = drift::check_drift(&manifest, &ws_dir);
    drift::print_drift_warnings(&report, filter_repos, false);

    // Collect active repos (sorted, filtered, non-drifted, non-detached)
    let active_repos: Vec<&RepoEntry> = manifest
        .repos_sorted()
        .into_iter()
        .filter(|repo| {
            if !filter_repos.is_empty() && !filter_repos.iter().any(|f| f == &repo.name) {
                return false;
            }
            if report.has_any_drift(&repo.name) {
                return false;
            }
            true
        })
        .collect();

    let jobs = resolve_jobs(cli_jobs, &manifest, active_repos.len());

    println!("Syncing rig '{}'\n", manifest.name.bold());

    // Phase 1: classify (read-only, parallel-safe). Surfaces the ~ reconcilable
    // third state without mutating anything.
    let classify = if jobs <= 1 {
        sync_sequential(&manifest, &ws_dir, &active_repos, stash)
    } else {
        sync_parallel(&manifest, &ws_dir, &active_repos, stash, jobs)
    };

    // Phase 2: remediate landed branches (sequential, consent-gated). This is
    // where the only mutation happens — reset --hard + manifest repair.
    let remediation = remediate_reconcilables(
        &mut manifest,
        &ws_dir,
        &classify.reconcilables,
        stash,
        reconcile,
    );

    print_sync_summary(&classify, &remediation)
}

/// Deferred, single-threaded remediation of provably-landed branches.
///
/// Consent model: `--reconcile` auto-approves; an interactive TTY prompts per
/// repo; piped output only detects-and-hints (never mutates). Mutation order
/// per repo — re-verify → clean/stash gate → print pre-reset SHA → reset --hard
/// → (gone upstream) repair manifest → stash pop — is fixed by the spec so a
/// partial failure never lies. Continue-and-report: a per-repo failure never
/// aborts the others.
fn remediate_reconcilables(
    manifest: &mut Manifest,
    ws_dir: &Path,
    reconcilables: &[Reconcilable],
    stash: bool,
    reconcile: bool,
) -> RemediationResults {
    let mut results = RemediationResults {
        reconciled: Vec::new(),
        untouched: Vec::new(),
        errors: Vec::new(),
    };

    if reconcilables.is_empty() {
        return results;
    }

    let interactive = std::io::stdin().is_terminal();
    let mut manifest_changed = false;

    println!();
    println!("{}", "Reconcilable (work already landed upstream):".bold());

    for r in reconcilables {
        let worktree_path = manifest.worktree_dir(ws_dir, &r.name);
        let target_ref = format!("{}/{}", r.remote, r.target_branch);

        // Consent gate.
        let proceed = if reconcile {
            true
        } else if interactive {
            print!("  reset {} to {target_ref}? [y/N] ", r.name.bold());
            let _ = std::io::stdout().flush();
            let mut input = String::new();
            let _ = std::io::stdin().read_line(&mut input);
            input.trim_start().starts_with(['y', 'Y'])
        } else {
            // Piped / non-TTY without the flag: detect-and-hint, never mutate.
            println!(
                "  {} {} already landed — run {}",
                "~".cyan(),
                r.name.bold(),
                "git rig sync --reconcile".bold()
            );
            false
        };

        if !proceed {
            results.untouched.push(r.name.clone());
            continue;
        }

        // 1. Re-verify LANDED at the instant of reset (closes the TOCTOU window
        //    between the parallel classify pass and now).
        if !is_landed(&worktree_path, &target_ref, &r.branch) {
            println!(
                "  {} {} changed since classification — skipped",
                "~".cyan(),
                r.name.bold()
            );
            results.untouched.push(r.name.clone());
            continue;
        }

        // 2. Clean-worktree gate. The classifier only sees commits, so a dirty
        //    reset could discard uncommitted work — require clean, or --stash.
        let dirty = git::is_dirty(&worktree_path).unwrap_or(false);
        let mut stashed = false;
        if dirty {
            if stash {
                match git::stash_push(&worktree_path) {
                    Ok(did) => stashed = did,
                    Err(e) => {
                        println!("  {} {} stash failed: {e}", "ERR".red(), r.name.bold());
                        results
                            .errors
                            .push((r.name.clone(), format!("stash failed: {e}")));
                        continue;
                    }
                }
            } else {
                println!(
                    "  {} {} worktree dirty, not reset — commit/stash or use {}",
                    "~".cyan(),
                    r.name.bold(),
                    "--stash".bold()
                );
                results.untouched.push(r.name.clone());
                continue;
            }
        }

        // 3. Capture the pre-reset SHA so recovery is discoverable (reflog).
        let pre_sha = git::rev_parse_short(&worktree_path, "HEAD").unwrap_or_default();

        // 4. reset --hard. Content-safe by the tree-equality proof.
        if let Err(e) = git::reset_hard(&worktree_path, &target_ref) {
            if stashed {
                let _ = git::stash_pop(&worktree_path);
            }
            println!("  {} {} reset failed: {e}", "ERR".red(), r.name.bold());
            results
                .errors
                .push((r.name.clone(), format!("reset failed: {e}")));
            continue;
        }

        // 5. Gone upstream: repair the manifest (the durable fix — sync reads
        //    its target from .rig.json, not git tracking). Do this *after* a
        //    successful reset so a failed reset never clears the record of intent.
        if r.upstream_gone {
            if let Some(entry) = manifest.find_repo_mut(&r.name) {
                entry.upstream = None;
            }
            manifest_changed = true;
            let _ = git::branch_unset_upstream(&worktree_path); // cosmetic
        }

        // 6. Restore stash. On pop conflict the reset stands and the stash is
        //    preserved — report, don't roll back.
        if stashed && let Err(e) = git::stash_pop(&worktree_path) {
            println!(
                "  {} {} reset done, stash pop failed: {e} (changes still in git stash)",
                "ERR".red(),
                r.name.bold()
            );
            results.errors.push((
                r.name.clone(),
                format!("reset done, stash pop failed: {e} (changes still in git stash)"),
            ));
            continue;
        }

        let gone_note = if r.upstream_gone {
            " (upstream cleared)"
        } else {
            ""
        };
        println!(
            "  {} {} reset to {target_ref}{gone_note} (was {pre_sha} — recover via git reflog)",
            "ok".green(),
            r.name.bold()
        );
        results.reconciled.push(r.name.clone());
    }

    if manifest_changed && let Err(e) = manifest.save(ws_dir) {
        results.errors.push((
            "<manifest>".to_string(),
            format!("failed to save .rig.json: {e}"),
        ));
    }

    results
}

#[allow(clippy::too_many_lines)]
fn sync_sequential(
    manifest: &Manifest,
    ws_dir: &std::path::Path,
    active_repos: &[&RepoEntry],
    stash: bool,
) -> ClassifyResults {
    let mut out = ClassifyResults {
        dirty_skipped: Vec::new(),
        target_skipped: Vec::new(),
        errors: Vec::new(),
        reconcilables: Vec::new(),
    };
    let name_width = active_repos.iter().map(|r| r.name.len()).max().unwrap_or(0);

    for repo in active_repos {
        let worktree_path = manifest.worktree_dir(ws_dir, &repo.name);
        let padded = format!("{:<width$}", repo.name, width = name_width).bold();

        if repo.branch == git::DETACHED {
            println!(
                "  {} {padded} (detached, skipped)",
                format!("{:<4}", "-").yellow(),
            );
            continue;
        }

        let dirty = git::is_dirty(&worktree_path).unwrap_or(false);
        let mut stashed = false;

        if dirty && stash {
            match git::stash_push(&worktree_path) {
                Ok(did_stash) => stashed = did_stash,
                Err(e) => {
                    println!(
                        "  {} {padded} (stash failed: {e})",
                        format!("{:<4}", "ERR").red()
                    );
                    out.errors
                        .push((repo.name.clone(), format!("stash failed: {e}")));
                    continue;
                }
            }
        } else if dirty {
            println!(
                "  {} {padded} (dirty — skipped)",
                format!("{:<4}", "WARN").yellow(),
            );
            out.dirty_skipped.push(repo.name.clone());
            continue;
        }

        // Snapshot HEAD before sync
        let before = git::rev_parse_short(&worktree_path, "HEAD").unwrap_or_default();

        // Fetch from the source repo (shares refs with worktree)
        if let Err(e) = git::fetch(&repo.source, &repo.remote) {
            println!(
                "  {} {padded} (fetch failed: {e})",
                format!("{:<4}", "ERR").red()
            );
            out.errors
                .push((repo.name.clone(), format!("fetch failed: {e}")));
            if stashed && let Err(e) = git::stash_pop(&worktree_path) {
                eprintln!(
                    "  {} stash pop failed for {}: {e} (changes still in git stash)",
                    format!("{:<4}", "WARN").yellow(),
                    repo.name
                );
            }
            continue;
        }

        // Resolve the comparison target *after* prune, from remote-tracking refs.
        let target = match resolve_sync_target(repo, &worktree_path) {
            Ok(t) => t,
            Err(reason) => {
                if stashed {
                    let _ = git::stash_pop(&worktree_path);
                }
                println!(
                    "  {} {padded} ({reason})",
                    format!("{:<4}", "WARN").yellow()
                );
                out.target_skipped.push((repo.name.clone(), reason));
                continue;
            }
        };

        // Rebase worktree branch onto the resolved target.
        if git::rebase(&worktree_path, &target.branch, &repo.remote).is_ok() {
            let after = git::rev_parse_short(&worktree_path, "HEAD").unwrap_or_default();
            let (_ahead, behind) =
                git::ahead_behind(&worktree_path, &repo.branch, &target.branch, &repo.remote);

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

            let target_note = target_note(repo, &target);

            if stashed {
                match git::stash_pop(&worktree_path) {
                    Ok(()) => println!(
                        "  {} {padded} {moved}{behind_info}{target_note} (stash restored)",
                        format!("{:<4}", "ok").green(),
                    ),
                    Err(e) => println!(
                        "  {} {padded} {moved} (stash pop failed: {e})",
                        format!("{:<4}", "WARN").yellow(),
                    ),
                }
            } else {
                println!(
                    "  {} {padded} {moved}{behind_info}{target_note}",
                    format!("{:<4}", "ok").green(),
                );
            }
        } else {
            if let Err(e) = git::rebase_abort(&worktree_path) {
                eprintln!(
                    "  {} rebase abort failed for {}: {e}",
                    format!("{:<4}", "WARN").yellow(),
                    repo.name
                );
            }
            if stashed && let Err(e) = git::stash_pop(&worktree_path) {
                eprintln!(
                    "  {} stash pop failed for {}: {e} (changes still in git stash)",
                    format!("{:<4}", "WARN").yellow(),
                    repo.name
                );
            }

            // Reactive classification: was that conflict real, or a squash artifact?
            let target_ref = format!("{}/{}", repo.remote, target.branch);
            if is_landed(&worktree_path, &target_ref, &repo.branch) {
                println!(
                    "  {} {padded} (already landed — reconcilable)",
                    format!("{:<4}", "~").cyan(),
                );
                out.reconcilables.push(Reconcilable {
                    name: repo.name.clone(),
                    remote: repo.remote.clone(),
                    branch: repo.branch.clone(),
                    target_branch: target.branch.clone(),
                    upstream_gone: target.upstream_gone,
                });
            } else {
                println!(
                    "  {} {padded} (rebase conflict — aborted)",
                    format!("{:<4}", "ERR").red(),
                );
                out.errors
                    .push((repo.name.clone(), "rebase conflict".to_string()));
            }
        }
    }

    out
}

/// The dimmed target annotation appended to a synced line: surfaces a gone
/// upstream, or a custom upstream, and stays silent for the plain default case.
fn target_note(repo: &RepoEntry, target: &SyncTarget) -> String {
    if target.upstream_gone {
        format!(
            " {}",
            format!("(upstream gone → {})", target.branch).dimmed()
        )
    } else if repo.upstream.is_some() {
        format!(" {}", format!("(upstream: {})", target.branch).dimmed())
    } else {
        String::new()
    }
}

#[allow(clippy::too_many_lines)]
fn sync_parallel(
    manifest: &Manifest,
    ws_dir: &std::path::Path,
    active_repos: &[&RepoEntry],
    stash: bool,
    jobs: usize,
) -> ClassifyResults {
    let fetch_cache = crate::parallel::FetchCache::new();

    let repo_names: Vec<String> = active_repos.iter().map(|r| r.name.clone()).collect();

    let cancel = AtomicBool::new(false);

    let results = crate::parallel::run_parallel(&repo_names, jobs, &cancel, |idx, progress| {
        let repo = active_repos[idx];
        let worktree_path = manifest.worktree_dir(ws_dir, &repo.name);

        if repo.branch == git::DETACHED {
            progress.set_status("detached, skipped");
            return Ok(SyncOutcome::Detached);
        }

        let dirty = git::is_dirty(&worktree_path).unwrap_or(false);
        let mut stashed = false;

        if dirty && stash {
            progress.set_status("stashing...");
            match git::stash_push(&worktree_path) {
                Ok(did_stash) => stashed = did_stash,
                Err(e) => return Err(format!("stash failed: {e}")),
            }
        } else if dirty {
            progress.set_status("dirty, skipped");
            return Ok(SyncOutcome::DirtySkipped);
        }

        let before = git::rev_parse_short(&worktree_path, "HEAD").unwrap_or_default();

        progress.set_status("fetching...");
        let fetch_result = fetch_cache.fetch_once(&repo.source, &repo.remote, || {
            git::fetch(&repo.source, &repo.remote).map_err(|e| e.to_string())
        });

        if let Err(e) = fetch_result {
            if stashed {
                let _ = git::stash_pop(&worktree_path);
            }
            return Err(format!("fetch failed: {e}"));
        }

        // Resolve the comparison target *after* prune, from remote-tracking refs.
        let target = match resolve_sync_target(repo, &worktree_path) {
            Ok(t) => t,
            Err(reason) => {
                if stashed {
                    let _ = git::stash_pop(&worktree_path);
                }
                progress.set_status(&reason);
                return Ok(SyncOutcome::TargetSkipped(reason));
            }
        };

        progress.set_status("rebasing...");
        if git::rebase(&worktree_path, &target.branch, &repo.remote).is_ok() {
            let after = git::rev_parse_short(&worktree_path, "HEAD").unwrap_or_default();
            let (_ahead, behind) =
                git::ahead_behind(&worktree_path, &repo.branch, &target.branch, &repo.remote);

            let moved = if before == after {
                "already up to date".to_string()
            } else {
                format!("{before} -> {after}")
            };

            let behind_info = if behind > 0 {
                format!(" (still {behind} behind)")
            } else {
                String::new()
            };

            let target_info = if target.upstream_gone {
                format!(" (upstream gone → {})", target.branch)
            } else if repo.upstream.is_some() {
                format!(" (upstream: {})", target.branch)
            } else {
                String::new()
            };

            let detail = format!("{moved}{behind_info}{target_info}");

            if stashed {
                progress.set_status("restoring stash...");
                if let Err(e) = git::stash_pop(&worktree_path) {
                    return Ok(SyncOutcome::Synced(format!(
                        "{detail} (stash pop failed: {e})"
                    )));
                }
                Ok(SyncOutcome::Synced(format!("{detail} (stash restored)")))
            } else {
                Ok(SyncOutcome::Synced(detail))
            }
        } else {
            let _ = git::rebase_abort(&worktree_path);
            if stashed {
                let _ = git::stash_pop(&worktree_path);
            }

            // Reactive classification (read-only, parallel-safe): was that
            // conflict real, or a squash artifact?
            let target_ref = format!("{}/{}", repo.remote, target.branch);
            if is_landed(&worktree_path, &target_ref, &repo.branch) {
                progress.set_status("already landed — reconcilable");
                Ok(SyncOutcome::Reconcilable {
                    target_branch: target.branch.clone(),
                    upstream_gone: target.upstream_gone,
                })
            } else {
                Err("rebase conflict — aborted".to_string())
            }
        }
    });

    let name_width = results.iter().map(|(n, _)| n.len()).max().unwrap_or(0);

    // Repo names are unique within a rig, so look the entry up by name.
    let repo_by_name = |name: &str| active_repos.iter().find(|r| r.name == name);

    let mut out = ClassifyResults {
        dirty_skipped: Vec::new(),
        target_skipped: Vec::new(),
        errors: Vec::new(),
        reconcilables: Vec::new(),
    };
    for (name, result) in &results {
        let padded = format!("{:<width$}", name, width = name_width).bold();
        match result {
            Ok(SyncOutcome::DirtySkipped) => {
                println!(
                    "  {} {padded} (dirty — skipped)",
                    format!("{:<4}", "WARN").yellow()
                );
                out.dirty_skipped.push(name.clone());
            }
            Ok(SyncOutcome::Detached) => {
                println!(
                    "  {} {padded} (detached, skipped)",
                    format!("{:<4}", "ok").green()
                );
            }
            Ok(SyncOutcome::TargetSkipped(reason)) => {
                println!(
                    "  {} {padded} ({reason})",
                    format!("{:<4}", "WARN").yellow()
                );
                out.target_skipped.push((name.clone(), reason.clone()));
            }
            Ok(SyncOutcome::Synced(msg)) => {
                println!("  {} {padded} {msg}", format!("{:<4}", "ok").green());
            }
            Ok(SyncOutcome::Reconcilable {
                target_branch,
                upstream_gone,
            }) => {
                println!(
                    "  {} {padded} (already landed — reconcilable)",
                    format!("{:<4}", "~").cyan()
                );
                if let Some(repo) = repo_by_name(name) {
                    out.reconcilables.push(Reconcilable {
                        name: name.clone(),
                        remote: repo.remote.clone(),
                        branch: repo.branch.clone(),
                        target_branch: target_branch.clone(),
                        upstream_gone: *upstream_gone,
                    });
                }
            }
            Err(e) => {
                println!("  {} {padded} ({e})", format!("{:<4}", "ERR").red());
                out.errors.push((name.clone(), e.clone()));
            }
        }
    }

    out
}

// ---------------------------------------------------------------------------
// doctor
// ---------------------------------------------------------------------------

/// Minimum git version required by git-rig.
///
/// Bumped 2.30 → 2.38 for `git merge-tree --write-tree` (Oct 2022), the
/// in-memory 3-way merge that powers post-merge reconciliation in `sync`.
/// A single global floor is simpler than feature-gating the classifier.
const MIN_GIT_VERSION: (u32, u32, u32) = (2, 38, 0);

pub fn doctor(name: Option<&str>) -> Result<()> {
    let mut has_issues = false;

    println!("{}", "Environment".bold().underline());
    println!();

    // R4a + R4b: Git on PATH and version >= 2.38
    // Single git_version() call checks both — saves a subprocess on the happy path.
    match git::git_version() {
        Ok((major, minor, patch)) => {
            print_pass("git found on PATH");
            let (min_major, min_minor, _) = MIN_GIT_VERSION;
            if (major, minor, patch) >= MIN_GIT_VERSION {
                print_pass(&format!(
                    "git version {major}.{minor}.{patch} (>= {min_major}.{min_minor} required)"
                ));
            } else {
                print_fail(&format!(
                    "git version {major}.{minor}.{patch} is below minimum {min_major}.{min_minor}"
                ));
                println!(
                    "    git rig sync requires `git merge-tree --write-tree` (git >= {min_major}.{min_minor})."
                );
                println!("    Fix: upgrade git to {min_major}.{min_minor}+");
                // R10: short-circuit
                println!();
                println!("{} per-repo checks skipped (git too old)", "SKIP".yellow());
                std::process::exit(1);
            }
        }
        Err(_) if !git::is_git_available() => {
            print_fail("git not found on PATH");
            println!("    Install git: https://git-scm.com/downloads");
            println!();
            println!(
                "{} per-repo checks skipped (git not available)",
                "SKIP".yellow()
            );
            std::process::exit(1);
        }
        Err(e) => {
            print_fail(&format!("could not parse git version: {e}"));
            std::process::exit(1);
        }
    }

    println!();

    // Tier 2: Per-repo checks (R1, R2, R3)
    let ws_result = workspace::resolve_workspace(name);
    let (ws_dir, manifest) = match ws_result {
        Ok(pair) => pair,
        Err(_) if name.is_none() => {
            // R2: outside a rig, no error
            println!(
                "{}",
                "(not inside a rig — per-repo checks skipped)".dimmed()
            );
            return Ok(());
        }
        Err(e) => return Err(e),
    };

    println!(
        "{} ({} repos)",
        format!("Rig: {}", manifest.name).bold().underline(),
        manifest.repos.len()
    );
    println!();

    if manifest.repos.is_empty() {
        println!("  No repos. Add one with: git rig add <repo>");
        return Ok(());
    }

    // Run drift detection to reuse for R5a-R5e
    let drift_report = drift::check_drift(&manifest, &ws_dir);

    for repo in manifest.repos_sorted() {
        println!("  {}", repo.name.bold());

        let worktree_path = manifest.worktree_dir(&ws_dir, &repo.name);

        // R5a: Source repo exists (from drift: MissingSource)
        let source_missing = drift_report.has_source_missing(&repo.name);
        if source_missing {
            has_issues = true;
            print_repo_fail("source repo missing");
            println!(
                "      Re-clone or run: git rig remove {} && git rig add <path>",
                repo.name
            );
        } else {
            print_repo_pass("source repo exists");
        }

        // R5b + R5c: Worktree exists and is reachable (from drift: MissingWorktree, WorktreeUnreachable)
        let worktree_ok = if !worktree_path.exists() {
            has_issues = true;
            print_repo_fail("worktree missing");
            println!(
                "      Fix: git rig remove {} && git rig add <path>",
                repo.name
            );
            false
        } else if drift_report.has_worktree_unavailable(&repo.name) {
            has_issues = true;
            print_repo_fail("worktree unreachable");
            println!("      Fix: git worktree repair {}", worktree_path.display());
            false
        } else {
            print_repo_pass("worktree exists and reachable");
            true
        };

        // R5d + R5e: Branch matches manifest (from drift: BranchMismatch, UnexpectedDetached)
        if worktree_ok {
            let mut found_branch_drift = false;
            for d in &drift_report.drifts {
                if d.repo_name != repo.name {
                    continue;
                }
                match &d.kind {
                    drift::DriftKind::BranchMismatch { expected, actual } => {
                        has_issues = true;
                        found_branch_drift = true;
                        print_repo_warn(&format!(
                            "branch mismatch: on {actual}, expected {expected}"
                        ));
                        println!(
                            "      Fix: cd {} && git checkout {expected}",
                            worktree_path.display()
                        );
                    }
                    drift::DriftKind::UnexpectedDetached { expected } => {
                        has_issues = true;
                        found_branch_drift = true;
                        print_repo_warn(&format!("detached HEAD, expected branch {expected}"));
                        println!(
                            "      Fix: cd {} && git checkout {expected}",
                            worktree_path.display()
                        );
                    }
                    _ => {}
                }
            }
            if !found_branch_drift {
                print_repo_pass(&format!("branch matches manifest ({})", repo.branch));
            }
        }

        // R5f, R5g, R5h: checks that require the source repo to exist
        if !source_missing {
            // R5f: origin/HEAD is set
            if git::has_remote_head(&repo.source, &repo.remote) {
                print_repo_pass(&format!("{}/HEAD set", repo.remote));
            } else {
                has_issues = true;
                print_repo_warn(&format!("{}/HEAD not set", repo.remote));
                println!("      Default branch detection won't work.");
                println!(
                    "      Fix: cd {} && git remote set-head {} --auto",
                    repo.source.display(),
                    repo.remote
                );
            }

            // R5g + R5h: Remote reachability and upstream branch existence (single network call)
            let remote_branches = git::probe_remote_branches(&repo.source, &repo.remote);
            if let Some(ref branches) = remote_branches {
                print_repo_pass(&format!("remote '{}' reachable", repo.remote));

                if let Some(upstream) = &repo.upstream {
                    if branches.iter().any(|b| b == upstream) {
                        print_repo_pass(&format!("upstream branch '{upstream}' exists on remote"));
                    } else {
                        has_issues = true;
                        print_repo_warn(&format!(
                            "upstream branch '{upstream}' not found on remote"
                        ));
                        println!("      sync will fail for this repo.");
                        println!(
                            "      Fix: git rig add {} --upstream <valid-branch> or --no-upstream",
                            repo.name
                        );
                    }
                }
            } else {
                has_issues = true;
                print_repo_warn(&format!("remote '{}' not reachable", repo.remote));
                println!("      Check network connection or remote URL.");
                println!(
                    "      Verify: cd {} && git remote -v",
                    repo.source.display()
                );
            }
        }

        println!();
    }

    if has_issues {
        std::process::exit(1);
    }

    println!("{} All checks passed", "ok".green());
    Ok(())
}

fn print_pass(msg: &str) {
    println!("  {} {}", "PASS".green(), msg);
}

fn print_fail(msg: &str) {
    println!("  {} {}", "FAIL".red(), msg);
}

fn print_repo_pass(msg: &str) {
    println!("    {} {}", "PASS".green(), msg);
}

fn print_repo_warn(msg: &str) {
    println!("    {} {}", "WARN".yellow(), msg);
}

fn print_repo_fail(msg: &str) {
    println!("    {} {}", "FAIL".red(), msg);
}

// ---------------------------------------------------------------------------
// exec
// ---------------------------------------------------------------------------

pub fn exec(
    name: Option<&str>,
    filter_repos: &[String],
    cmd: &[String],
    fail_fast: bool,
    cli_jobs: Option<usize>,
) -> Result<()> {
    let (ws_dir, manifest) = workspace::resolve_workspace(name)?;
    manifest.validate_repo_filter(filter_repos)?;

    let report = drift::check_drift(&manifest, &ws_dir);
    drift::print_drift_warnings(&report, filter_repos, false);

    // Collect active repos
    let active_repos: Vec<&RepoEntry> = manifest
        .repos_sorted()
        .into_iter()
        .filter(|repo| {
            if !filter_repos.is_empty() && !filter_repos.iter().any(|f| f == &repo.name) {
                return false;
            }
            true
        })
        .collect();

    let jobs = resolve_jobs(cli_jobs, &manifest, active_repos.len());

    if jobs <= 1 {
        return exec_sequential(&manifest, &ws_dir, &active_repos, cmd, fail_fast, &report);
    }

    exec_parallel(
        &manifest,
        &ws_dir,
        &active_repos,
        cmd,
        fail_fast,
        jobs,
        &report,
    )
}

fn exec_sequential(
    manifest: &Manifest,
    ws_dir: &std::path::Path,
    active_repos: &[&RepoEntry],
    cmd: &[String],
    fail_fast: bool,
    report: &drift::DriftReport,
) -> Result<()> {
    let mut errors: Vec<(String, String)> = Vec::new();

    for repo in active_repos {
        let worktree_path = manifest.worktree_dir(ws_dir, &repo.name);

        println!("{} {}", ">>>".bold(), repo.name.bold());

        if report.has_worktree_unavailable(&repo.name) {
            println!("{} worktree unavailable, skipped", "WARN".yellow());
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

    print_exec_summary(&errors)
}

fn print_exec_summary(errors: &[(String, String)]) -> Result<()> {
    if !errors.is_empty() {
        println!("{} {} repo(s) had errors:", "WARN".yellow(), errors.len());
        for (name, err) in errors {
            println!("  {} {}: {}", "ERR".red(), name, err);
        }
        return Err(anyhow!("{} repo(s) had errors", errors.len()));
    }
    Ok(())
}

fn exec_parallel(
    manifest: &Manifest,
    ws_dir: &std::path::Path,
    active_repos: &[&RepoEntry],
    cmd: &[String],
    fail_fast: bool,
    jobs: usize,
    report: &drift::DriftReport,
) -> Result<()> {
    let repo_names: Vec<String> = active_repos.iter().map(|r| r.name.clone()).collect();
    let cancel = AtomicBool::new(false);

    // Each result is (stdout, stderr, Option<error_msg>)
    let results = crate::parallel::run_parallel(&repo_names, jobs, &cancel, |idx, progress| {
        let repo = active_repos[idx];
        let worktree_path = manifest.worktree_dir(ws_dir, &repo.name);

        if report.has_worktree_unavailable(&repo.name) {
            progress.set_status("unavailable, skipped");
            return Err("worktree unavailable, skipped".to_string());
        }

        progress.set_status("running...");

        let output = Command::new(&cmd[0])
            .args(&cmd[1..])
            .current_dir(&worktree_path)
            .output();

        match output {
            Ok(o) => {
                let stdout = String::from_utf8_lossy(&o.stdout).to_string();
                let stderr = String::from_utf8_lossy(&o.stderr).to_string();

                if o.status.success() {
                    Ok((stdout, stderr, None))
                } else {
                    let code = o.status.code().unwrap_or(-1);
                    if fail_fast {
                        cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                    Ok((stdout, stderr, Some(format!("exit code {code}"))))
                }
            }
            Err(e) => {
                if fail_fast {
                    cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                }
                Ok((
                    String::new(),
                    String::new(),
                    Some(format!("failed to execute: {e}")),
                ))
            }
        }
    });

    // Print captured output in alphabetical order
    let mut errors: Vec<(String, String)> = Vec::new();
    for (name, result) in &results {
        println!("{} {}", ">>>".bold(), name.bold());
        match result {
            Ok((stdout, stderr, error)) => {
                if !stdout.is_empty() {
                    print!("{stdout}");
                }
                if !stderr.is_empty() {
                    eprint!("{stderr}");
                }
                if let Some(e) = error {
                    errors.push((name.clone(), e.clone()));
                }
            }
            Err(e) => {
                // Skipped repos (e.g., unavailable worktree)
                println!("{} {e}", "WARN".yellow());
            }
        }
        println!();
    }

    print_exec_summary(&errors)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_jobs_auto_caps_at_8() {
        let m = Manifest::new("ws");
        assert_eq!(resolve_jobs(None, &m, 20), 8);
    }

    #[test]
    fn resolve_jobs_auto_uses_repo_count_when_small() {
        let m = Manifest::new("ws");
        assert_eq!(resolve_jobs(None, &m, 3), 3);
    }

    #[test]
    fn resolve_jobs_auto_minimum_is_1() {
        let m = Manifest::new("ws");
        assert_eq!(resolve_jobs(None, &m, 0), 1);
    }

    #[test]
    fn resolve_jobs_cli_overrides_manifest() {
        let mut m = Manifest::new("ws");
        m.jobs = Some(4);
        assert_eq!(resolve_jobs(Some(2), &m, 10), 2);
    }

    #[test]
    fn resolve_jobs_manifest_overrides_auto() {
        let mut m = Manifest::new("ws");
        m.jobs = Some(4);
        assert_eq!(resolve_jobs(None, &m, 10), 4);
    }

    #[test]
    fn resolve_jobs_zero_means_auto() {
        let mut m = Manifest::new("ws");
        m.jobs = Some(0);
        assert_eq!(resolve_jobs(None, &m, 5), 5);
    }

    #[test]
    fn resolve_jobs_cli_1_forces_sequential() {
        let mut m = Manifest::new("ws");
        m.jobs = Some(4);
        assert_eq!(resolve_jobs(Some(1), &m, 10), 1);
    }
}
