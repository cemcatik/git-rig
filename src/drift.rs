use std::collections::HashMap;
use std::path::Path;

use colored::Colorize;

use crate::git;
use crate::workspace::Manifest;

/// What kind of drift was detected for a repo.
pub enum DriftKind {
    /// Worktree directory does not exist on disk.
    MissingWorktree,
    /// Source clone directory no longer exists.
    MissingSource,
    /// Worktree is on a different branch than the manifest records.
    BranchMismatch { expected: String, actual: String },
    /// Manifest records a named branch but worktree is in detached HEAD.
    UnexpectedDetached { expected: String },
    /// Worktree directory exists but git commands fail in it (e.g., broken .git link).
    WorktreeUnreachable { error: String },
}

/// A single drift finding for one repo.
pub struct RepoDrift {
    pub repo_name: String,
    pub kind: DriftKind,
}

/// Result of running drift detection across all repos in a manifest.
pub struct DriftReport {
    pub drifts: Vec<RepoDrift>,
    /// Cached `current_branch` results keyed by repo name.
    /// Commands can reuse this to avoid redundant git subprocess calls.
    pub branches: HashMap<String, String>,
}

impl DriftReport {
    /// True if the named repo has any drift at all.
    pub fn has_any_drift(&self, repo_name: &str) -> bool {
        self.drifts.iter().any(|d| d.repo_name == repo_name)
    }

    /// True if the repo's worktree is physically unavailable (missing or unreachable).
    pub fn has_worktree_unavailable(&self, repo_name: &str) -> bool {
        self.drifts.iter().any(|d| {
            d.repo_name == repo_name
                && matches!(
                    d.kind,
                    DriftKind::MissingWorktree | DriftKind::WorktreeUnreachable { .. }
                )
        })
    }

    /// True if the repo's source clone directory is missing.
    pub fn has_source_missing(&self, repo_name: &str) -> bool {
        self.drifts
            .iter()
            .any(|d| d.repo_name == repo_name && matches!(d.kind, DriftKind::MissingSource))
    }
}

/// Run drift detection across all repos in a manifest.
///
/// This never returns an error — all failures (git errors, permission problems)
/// are absorbed into the report as drift entries. This ensures one broken repo
/// cannot prevent other commands from running.
///
/// See `docs/brainstorms/2026-04-01-manifest-drift-detection-requirements.md`
/// for the full requirements and design decisions.
pub fn check_drift(manifest: &Manifest, ws_dir: &Path) -> DriftReport {
    let mut drifts = Vec::new();
    let mut branches = HashMap::new();

    for repo in manifest.repos_sorted() {
        let worktree_path = manifest.worktree_dir(ws_dir, &repo.name);

        // Missing worktree
        if !worktree_path.exists() {
            drifts.push(RepoDrift {
                repo_name: repo.name.clone(),
                kind: DriftKind::MissingWorktree,
            });
            // Can't check branch state without a worktree, but can still check source
            if !repo.source.exists() {
                drifts.push(RepoDrift {
                    repo_name: repo.name.clone(),
                    kind: DriftKind::MissingSource,
                });
            }
            continue;
        }

        // Missing source repo
        if !repo.source.exists() {
            drifts.push(RepoDrift {
                repo_name: repo.name.clone(),
                kind: DriftKind::MissingSource,
            });
        }

        // Check current branch for mismatch and detached-HEAD drift
        match git::current_branch(&worktree_path) {
            Err(e) => {
                drifts.push(RepoDrift {
                    repo_name: repo.name.clone(),
                    kind: DriftKind::WorktreeUnreachable {
                        error: e.to_string(),
                    },
                });
            }
            Ok(actual_branch) => {
                branches.insert(repo.name.clone(), actual_branch.clone());

                if repo.branch != git::DETACHED {
                    if actual_branch == git::DETACHED {
                        // Unexpected detached HEAD: manifest expects a named branch
                        drifts.push(RepoDrift {
                            repo_name: repo.name.clone(),
                            kind: DriftKind::UnexpectedDetached {
                                expected: repo.branch.clone(),
                            },
                        });
                    } else if actual_branch != repo.branch {
                        // Branch mismatch: worktree on a different branch
                        drifts.push(RepoDrift {
                            repo_name: repo.name.clone(),
                            kind: DriftKind::BranchMismatch {
                                expected: repo.branch.clone(),
                                actual: actual_branch,
                            },
                        });
                    }
                } else if actual_branch != git::DETACHED {
                    // Manifest says detached, but worktree is on a named branch
                    drifts.push(RepoDrift {
                        repo_name: repo.name.clone(),
                        kind: DriftKind::BranchMismatch {
                            expected: git::DETACHED.to_string(),
                            actual: actual_branch,
                        },
                    });
                }
            }
        }
    }

    DriftReport { drifts, branches }
}

/// Print the drift warning block to stdout.
///
/// - `repo_filter`: if non-empty, only print warnings for these repos (for exec/sync --repo scoping).
/// - `source_only`: if `true`, only print `MissingSource` drift (for refresh, which doesn't touch worktrees).
pub fn print_drift_warnings(report: &DriftReport, repo_filter: &[String], source_only: bool) {
    let visible: Vec<&RepoDrift> = report
        .drifts
        .iter()
        .filter(|d| repo_filter.is_empty() || repo_filter.iter().any(|r| r == &d.repo_name))
        .filter(|d| {
            if source_only {
                matches!(d.kind, DriftKind::MissingSource)
            } else {
                true
            }
        })
        .collect();

    if visible.is_empty() {
        return;
    }

    for drift in &visible {
        match &drift.kind {
            DriftKind::MissingWorktree => {
                println!(
                    "  {} {}: worktree missing",
                    "DRIFT".yellow(),
                    drift.repo_name.bold()
                );
            }
            DriftKind::MissingSource => {
                println!(
                    "  {} {}: source repo missing",
                    "DRIFT".yellow(),
                    drift.repo_name.bold()
                );
            }
            DriftKind::BranchMismatch { expected, actual } => {
                println!(
                    "  {} {}: on {}, expected {}",
                    "DRIFT".yellow(),
                    drift.repo_name.bold(),
                    actual.cyan(),
                    expected.cyan()
                );
            }
            DriftKind::UnexpectedDetached { expected } => {
                println!(
                    "  {} {}: detached HEAD, expected {}",
                    "DRIFT".yellow(),
                    drift.repo_name.bold(),
                    expected.cyan()
                );
            }
            DriftKind::WorktreeUnreachable { error } => {
                println!(
                    "  {} {}: worktree unreachable ({})",
                    "DRIFT".yellow(),
                    drift.repo_name.bold(),
                    error
                );
            }
        }
    }
    println!();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_report(drifts: Vec<RepoDrift>) -> DriftReport {
        DriftReport {
            drifts,
            branches: HashMap::new(),
        }
    }

    #[test]
    fn has_any_drift_true() {
        let report = make_report(vec![RepoDrift {
            repo_name: "repo-a".into(),
            kind: DriftKind::MissingWorktree,
        }]);
        assert!(report.has_any_drift("repo-a"));
    }

    #[test]
    fn has_any_drift_false() {
        let report = make_report(vec![]);
        assert!(!report.has_any_drift("repo-a"));
    }

    #[test]
    fn has_any_drift_different_repo() {
        let report = make_report(vec![RepoDrift {
            repo_name: "repo-b".into(),
            kind: DriftKind::MissingWorktree,
        }]);
        assert!(!report.has_any_drift("repo-a"));
    }

    #[test]
    fn has_worktree_unavailable_missing() {
        let report = make_report(vec![RepoDrift {
            repo_name: "repo-a".into(),
            kind: DriftKind::MissingWorktree,
        }]);
        assert!(report.has_worktree_unavailable("repo-a"));
    }

    #[test]
    fn has_worktree_unavailable_unreachable() {
        let report = make_report(vec![RepoDrift {
            repo_name: "repo-a".into(),
            kind: DriftKind::WorktreeUnreachable {
                error: "broken".into(),
            },
        }]);
        assert!(report.has_worktree_unavailable("repo-a"));
    }

    #[test]
    fn has_worktree_unavailable_false_for_branch_mismatch() {
        let report = make_report(vec![RepoDrift {
            repo_name: "repo-a".into(),
            kind: DriftKind::BranchMismatch {
                expected: "rig/ws".into(),
                actual: "main".into(),
            },
        }]);
        assert!(!report.has_worktree_unavailable("repo-a"));
    }

    #[test]
    fn has_source_missing_true() {
        let report = make_report(vec![RepoDrift {
            repo_name: "repo-a".into(),
            kind: DriftKind::MissingSource,
        }]);
        assert!(report.has_source_missing("repo-a"));
    }

    #[test]
    fn has_source_missing_false_for_other_drift() {
        let report = make_report(vec![RepoDrift {
            repo_name: "repo-a".into(),
            kind: DriftKind::MissingWorktree,
        }]);
        assert!(!report.has_source_missing("repo-a"));
    }
}
