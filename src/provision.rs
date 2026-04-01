use std::fs;
use std::path::Path;

use colored::Colorize;
use ignore::gitignore::GitignoreBuilder;
use walkdir::WalkDir;

const RIGINCLUDE: &str = ".riginclude";

/// Create a symlink, or copy if symlinks are not supported.
/// Returns `true` if a real symlink was created, `false` if it fell back to copy.
fn symlink_or_copy(original: &Path, link: &Path) -> std::io::Result<bool> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(original, link)?;
        Ok(true)
    }
    #[cfg(not(unix))]
    {
        // Windows symlinks require SeCreateSymbolicLinkPrivilege.
        // Fall back to copying, which works everywhere.
        fs::copy(original, link)?;
        Ok(false)
    }
}

/// Options controlling provisioning behavior.
///
/// When passed as `Option<ProvisionOpts>`, `None` means skip provisioning
/// entirely (equivalent to `--no-provision`).
pub struct ProvisionOpts {
    pub force: bool,
    pub link: bool,
}

/// Result of provisioning a single file.
pub enum FileResult {
    Copied(String),
    Linked(String),
    Skipped { rel_path: String, reason: String },
    Failed { rel_path: String, error: String },
}

/// Result of provisioning a repo.
pub struct ProvisionReport {
    pub files: Vec<FileResult>,
}

/// Read `.riginclude` from `source_dir`, copy/link matching files into `target_dir`.
///
/// Returns `None` if no `.riginclude` exists in the source.
///
/// Provisioning failures are intentionally warnings, not fatal errors:
/// file copying is auxiliary to workspace creation. The caller should
/// print the report but not propagate failures as `Err`.
pub fn provision_files(
    source_dir: &Path,
    target_dir: &Path,
    opts: &ProvisionOpts,
) -> Option<ProvisionReport> {
    let riginclude_path = source_dir.join(RIGINCLUDE);

    if !riginclude_path.is_file() {
        return None;
    }

    let mut report = ProvisionReport { files: Vec::new() };

    // Always copy .riginclude itself first — it self-propagates to new worktrees
    copy_or_link_file(
        source_dir,
        target_dir,
        Path::new(RIGINCLUDE),
        opts,
        &mut report,
    );

    // Parse .riginclude patterns using gitignore-style matching
    let mut builder = GitignoreBuilder::new(source_dir);
    if let Some(err) = builder.add(&riginclude_path) {
        report.files.push(FileResult::Failed {
            rel_path: RIGINCLUDE.to_string(),
            error: format!("failed to parse: {err}"),
        });
        return Some(report);
    }

    let matcher = match builder.build() {
        Ok(m) => m,
        Err(e) => {
            report.files.push(FileResult::Failed {
                rel_path: RIGINCLUDE.to_string(),
                error: format!("failed to build matcher: {e}"),
            });
            return Some(report);
        }
    };

    // Canonicalize source for path containment checks
    let canonical_source = match source_dir.canonicalize() {
        Ok(p) => p,
        Err(e) => {
            report.files.push(FileResult::Failed {
                rel_path: ".".to_string(),
                error: format!("failed to canonicalize source: {e}"),
            });
            return Some(report);
        }
    };

    // Walk source directory, match files against patterns.
    // Track matched directories so their children are included automatically
    // (gitignore semantics: matching a directory includes everything inside it).
    let mut matched_dirs: Vec<std::path::PathBuf> = Vec::new();

    for entry in WalkDir::new(source_dir).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();

        // Skip the source root itself
        let rel_path = match path.strip_prefix(source_dir) {
            Ok(r) if r.as_os_str().is_empty() => continue,
            Ok(r) => r,
            Err(_) => continue,
        };

        // Skip .riginclude itself (already handled above)
        if rel_path == Path::new(RIGINCLUDE) {
            continue;
        }

        // Skip .git directory
        if rel_path.starts_with(".git") {
            continue;
        }

        let is_dir = entry.file_type().is_dir();

        // Check if this path is under an already-matched directory
        let under_matched_dir = matched_dirs.iter().any(|d| rel_path.starts_with(d));

        if is_dir {
            // If the directory matches a pattern, track it for child inclusion
            if !under_matched_dir && matcher.matched(rel_path, true).is_ignore() {
                matched_dirs.push(rel_path.to_path_buf());
            }
            continue;
        }

        // Include file if it matches directly or is under a matched directory
        if !under_matched_dir && !matcher.matched(rel_path, false).is_ignore() {
            continue;
        }

        // Path containment: skip files outside source root (e.g., symlinks escaping the repo)
        // and files that cannot be verified (broken symlinks, permission issues).
        match path.canonicalize() {
            Ok(canonical_file) if canonical_file.starts_with(&canonical_source) => {}
            _ => continue,
        }

        copy_or_link_file(source_dir, target_dir, rel_path, opts, &mut report);
    }

    Some(report)
}

/// Copy or symlink a single file, recording the result in the report.
fn copy_or_link_file(
    source_dir: &Path,
    target_dir: &Path,
    rel_path: &Path,
    opts: &ProvisionOpts,
    report: &mut ProvisionReport,
) {
    let source_file = source_dir.join(rel_path);
    let target_file = target_dir.join(rel_path);
    let rel_str = rel_path.to_string_lossy().to_string();

    // Check if target already exists (symlink_metadata catches dangling symlinks too)
    if target_file.symlink_metadata().is_ok() && !opts.force {
        report.files.push(FileResult::Skipped {
            rel_path: rel_str,
            reason: "already exists".to_string(),
        });
        return;
    }

    // Create parent directories
    if let Some(parent) = target_file.parent()
        && let Err(e) = fs::create_dir_all(parent)
    {
        report.files.push(FileResult::Failed {
            rel_path: rel_str,
            error: format!("failed to create directory: {e}"),
        });
        return;
    }

    // Remove existing file/symlink when force is set
    if target_file.symlink_metadata().is_ok()
        && let Err(e) = fs::remove_file(&target_file)
    {
        report.files.push(FileResult::Failed {
            rel_path: rel_str,
            error: format!("failed to remove existing file: {e}"),
        });
        return;
    }

    if opts.link {
        // Create absolute symlink
        let link_target = match source_file.canonicalize() {
            Ok(p) => p,
            Err(e) => {
                report.files.push(FileResult::Failed {
                    rel_path: rel_str,
                    error: format!("failed to resolve source: {e}"),
                });
                return;
            }
        };

        match symlink_or_copy(&link_target, &target_file) {
            Ok(true) => report.files.push(FileResult::Linked(rel_str)),
            Ok(false) => report.files.push(FileResult::Copied(rel_str)),
            Err(e) => report.files.push(FileResult::Failed {
                rel_path: rel_str,
                error: format!("failed to create symlink: {e}"),
            }),
        }
    } else {
        match fs::copy(&source_file, &target_file) {
            Ok(_) => report.files.push(FileResult::Copied(rel_str)),
            Err(e) => report.files.push(FileResult::Failed {
                rel_path: rel_str,
                error: format!("{e}"),
            }),
        }
    }
}

/// Print provisioning results inline with existing command output.
pub fn print_provision_report(report: &ProvisionReport) {
    let mut copied = Vec::new();
    let mut linked = Vec::new();
    let mut skipped = Vec::new();
    let mut failed = Vec::new();

    for result in &report.files {
        match result {
            FileResult::Copied(p) => copied.push(p.as_str()),
            FileResult::Linked(p) => linked.push(p.as_str()),
            FileResult::Skipped { rel_path, reason } => {
                skipped.push((rel_path.as_str(), reason.as_str()))
            }
            FileResult::Failed { rel_path, error } => {
                failed.push((rel_path.as_str(), error.as_str()))
            }
        }
    }

    if !copied.is_empty() {
        let names = copied.join(", ");
        println!(
            "  {}: {} ({} {})",
            "provisioned".dimmed(),
            names,
            copied.len(),
            if copied.len() == 1 { "file" } else { "files" }
        );
    }

    if !linked.is_empty() {
        let names = linked.join(", ");
        println!(
            "  {}: {} ({} {})",
            "linked".dimmed(),
            names,
            linked.len(),
            if linked.len() == 1 { "file" } else { "files" }
        );
    }

    for (path, reason) in &skipped {
        println!("  {}: {} ({})", "skipped".yellow(), path, reason);
    }

    for (path, error) in &failed {
        println!(
            "  {}: failed to provision {}: {}",
            "warning".yellow(),
            path,
            error
        );
    }
}
