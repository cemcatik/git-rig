use std::io::{IsTerminal, Write};
use std::path::Path;
use std::process::Command;

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

    // Add each repo from the source rig (already sorted via repos_sorted())
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

    for repo in active_repos {
        print!("  {}: ", repo.name.bold());

        if let Err(e) = git::fetch(&repo.source, &repo.remote) {
            println!("{} (fetch failed: {e})", "ERR".red());
            continue;
        }

        match git::default_branch(&repo.source, &repo.remote) {
            Ok(new_branch) => {
                if new_branch != repo.default_branch {
                    println!("{} → {}", repo.default_branch.dimmed(), new_branch.green());
                    if let Some(entry) = manifest.find_repo_mut(&repo.name) {
                        entry.default_branch = new_branch;
                    }
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
    use std::sync::atomic::AtomicBool;

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
    let mut updated = false;
    for (name, result) in &results {
        match result {
            Ok((old_branch, new_branch)) => {
                if old_branch != new_branch {
                    println!(
                        "  {}: {} → {}",
                        name.bold(),
                        old_branch.dimmed(),
                        new_branch.green()
                    );
                    if let Some(entry) = manifest.find_repo_mut(name) {
                        entry.default_branch = new_branch.clone();
                    }
                    updated = true;
                } else {
                    println!("  {}: {} (unchanged)", name.bold(), old_branch.dimmed());
                }
            }
            Err(e) => {
                println!("  {}: {} ({e})", name.bold(), "ERR".red());
            }
        }
    }

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

// ---------------------------------------------------------------------------
// sync
// ---------------------------------------------------------------------------

/// Sentinel value returned by the parallel sync closure for dirty-skipped repos.
/// Used to distinguish dirty skips from real successes in the result display.
const DIRTY_SENTINEL: &str = "[dirty]";

fn print_sync_summary(dirty_skipped: &[String], errors: &[(String, String)]) -> Result<()> {
    println!();
    if !dirty_skipped.is_empty() {
        println!(
            "{} {} repo(s) skipped (dirty — use {} to auto-stash):",
            "WARN".yellow(),
            dirty_skipped.len(),
            "--stash".bold()
        );
        for name in dirty_skipped {
            println!("  {} {}", "WARN".yellow(), name);
        }
        if !errors.is_empty() {
            println!();
        }
    }
    if !errors.is_empty() {
        println!("{} {} repo(s) had issues:", "WARN".yellow(), errors.len());
        for (name, err) in errors {
            println!("  {} {}: {}", "ERR".red(), name, err);
        }
        return Err(anyhow!("{} repo(s) had issues", errors.len()));
    }
    if dirty_skipped.is_empty() {
        println!("{} All repos synced", "ok".green());
    }
    Ok(())
}

#[allow(clippy::too_many_lines)]
pub fn sync(
    name: Option<&str>,
    filter_repos: &[String],
    stash: bool,
    cli_jobs: Option<usize>,
) -> Result<()> {
    let (ws_dir, manifest) = workspace::resolve_workspace(name)?;
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

    if jobs <= 1 {
        return sync_sequential(&manifest, &ws_dir, &active_repos, stash);
    }

    sync_parallel(&manifest, &ws_dir, &active_repos, stash, jobs)
}

fn sync_sequential(
    manifest: &Manifest,
    ws_dir: &std::path::Path,
    active_repos: &[&RepoEntry],
    stash: bool,
) -> Result<()> {
    let mut errors: Vec<(String, String)> = Vec::new();
    let mut dirty_skipped: Vec<String> = Vec::new();

    for repo in active_repos {
        let worktree_path = manifest.worktree_dir(ws_dir, &repo.name);

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
                "  {} {} (dirty — skipped)",
                "WARN".yellow(),
                repo.name.bold()
            );
            dirty_skipped.push(repo.name.clone());
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
            let (_ahead, behind) =
                git::ahead_behind(&worktree_path, &repo.branch, effective, &repo.remote);

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

    print_sync_summary(&dirty_skipped, &errors)
}

fn sync_parallel(
    manifest: &Manifest,
    ws_dir: &std::path::Path,
    active_repos: &[&RepoEntry],
    stash: bool,
    jobs: usize,
) -> Result<()> {
    use std::sync::atomic::AtomicBool;

    let fetch_cache = crate::parallel::FetchCache::new();

    let repo_names: Vec<String> = active_repos.iter().map(|r| r.name.clone()).collect();

    let cancel = AtomicBool::new(false);

    let results = crate::parallel::run_parallel(&repo_names, jobs, &cancel, |idx, progress| {
        let repo = active_repos[idx];
        let worktree_path = manifest.worktree_dir(ws_dir, &repo.name);

        // Skip detached repos
        if repo.branch == git::DETACHED {
            progress.set_status("detached, skipped");
            return Ok("detached, skipped".to_string());
        }

        // Dirty check + stash
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
            return Ok(DIRTY_SENTINEL.to_string());
        }

        let before = git::rev_parse_short(&worktree_path, "HEAD").unwrap_or_default();

        // Fetch with deduplication per (source, remote) pair
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

        // Rebase
        progress.set_status("rebasing...");
        let effective = repo.effective_upstream();
        if git::rebase(&worktree_path, effective, &repo.remote).is_ok() {
            let after = git::rev_parse_short(&worktree_path, "HEAD").unwrap_or_default();
            let (_ahead, behind) =
                git::ahead_behind(&worktree_path, &repo.branch, effective, &repo.remote);

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

            let upstream_info = if repo.upstream.is_some() {
                format!(" (upstream: {effective})")
            } else {
                String::new()
            };

            let detail = format!("{moved}{behind_info}{upstream_info}");

            if stashed {
                progress.set_status("restoring stash...");
                if let Err(e) = git::stash_pop(&worktree_path) {
                    return Ok(format!("{detail} (stash pop failed: {e})"));
                }
                Ok(format!("{detail} (stash restored)"))
            } else {
                Ok(detail)
            }
        } else {
            let _ = git::rebase_abort(&worktree_path);
            if stashed {
                let _ = git::stash_pop(&worktree_path);
            }
            Err("rebase conflict — aborted".to_string())
        }
    });

    // Print per-repo results to stdout (mirrors sequential output)
    let mut errors: Vec<(String, String)> = Vec::new();
    let mut dirty_skipped: Vec<String> = Vec::new();
    for (name, result) in &results {
        match result {
            Ok(msg) if msg == DIRTY_SENTINEL => {
                println!("  {} {} (dirty — skipped)", "WARN".yellow(), name.bold());
                dirty_skipped.push(name.clone());
            }
            Ok(msg) => println!("  {} {} {}", "ok".green(), name.bold(), msg),
            Err(e) => {
                println!("  {} {} ({e})", "ERR".red(), name.bold());
                errors.push((name.clone(), e.clone()));
            }
        }
    }

    print_sync_summary(&dirty_skipped, &errors)
}

// ---------------------------------------------------------------------------
// doctor
// ---------------------------------------------------------------------------

/// Minimum git version required by git-rig (for `git worktree repair`).
const MIN_GIT_VERSION: (u32, u32, u32) = (2, 30, 0);

pub fn doctor(name: Option<&str>) -> Result<()> {
    let mut has_issues = false;

    println!("{}", "Environment".bold().underline());
    println!();

    // R4a + R4b: Git on PATH and version >= 2.30
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
                println!("    git worktree repair requires git >= {min_major}.{min_minor}.");
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

    if !errors.is_empty() {
        println!("{} {} repo(s) had errors:", "WARN".yellow(), errors.len());
        for (name, err) in &errors {
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
    use std::sync::atomic::AtomicBool;

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

    if !errors.is_empty() {
        println!("{} {} repo(s) had errors:", "WARN".yellow(), errors.len());
        for (name, err) in &errors {
            println!("  {} {}: {}", "ERR".red(), name, err);
        }
        return Err(anyhow!("{} repo(s) had errors", errors.len()));
    }

    Ok(())
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
