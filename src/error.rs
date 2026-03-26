use std::path::PathBuf;

use thiserror::Error;

/// Typed errors for git-rig operations.
///
/// These are the error conditions that callers may want to distinguish
/// programmatically (e.g., for `--json` output or test assertions).
/// Anyhow is still used as the transport — all variants convert to
/// `anyhow::Error` automatically.
#[derive(Debug, Error)]
pub enum RigError {
    #[error("rig '{name}' not found")]
    RigNotFound { name: String },

    #[error("not inside a rig (no .rig.json found in any parent directory)")]
    NotInWorkspace,

    #[error("'{repo}' is not in rig '{rig}'")]
    RepoNotInRig { repo: String, rig: String },

    #[error("'{repo}' is already in rig '{rig}'")]
    RepoAlreadyInRig { repo: String, rig: String },

    #[error("{} is not a git repository", path.display())]
    NotAGitRepo { path: PathBuf },

    #[error("'{repo}' has uncommitted changes — use --force to remove anyway")]
    DirtyWorktree { repo: String },

    #[error("directory '{}' already exists", path.display())]
    DirectoryAlreadyExists { path: PathBuf },

    #[error(
        "cannot determine default branch for {}\n  \
         hint: run `git remote set-head {remote} <branch>` in the source repo\n  \
         hint: this is set automatically by `git clone` but not by `git init`",
        repo.display()
    )]
    DefaultBranchNotFound { repo: PathBuf, remote: String },

    #[error("use --yes to confirm (stdin is not a terminal)")]
    ConfirmationRequired,

    #[error("source repos invalid:\n{}", format_repo_errors(.errors))]
    SourceReposInvalid { errors: Vec<(String, String)> },
}

fn format_repo_errors(errors: &[(String, String)]) -> String {
    errors
        .iter()
        .map(|(name, reason)| format!("  {name}: {reason}"))
        .collect::<Vec<_>>()
        .join("\n")
}
