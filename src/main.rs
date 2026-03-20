use anyhow::Result;
use clap::{Parser, Subcommand};

mod commands;
mod git;
mod workspace;

#[derive(Parser)]
#[command(name = "git-rig", version, about = "Multi-repo rig manager using git worktrees")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Create a new rig in the current directory
    Create {
        /// Rig name (created as a subdirectory of CWD)
        name: String,
    },

    /// Add a repository worktree to a rig
    Add {
        /// Path to local repo, or rig name if a second argument is provided
        #[arg(value_name = "PATH_OR_RIG")]
        first: String,

        /// Path to local repo (when first argument is rig name)
        #[arg(value_name = "PATH")]
        second: Option<String>,

        /// Name for the repo in the rig (default: directory basename)
        #[arg(short, long)]
        name: Option<String>,

        /// Branch to check out or create (default: rig/<rig-name>)
        #[arg(short, long)]
        branch: Option<String>,

        /// Git remote to fetch from (default: origin)
        #[arg(short, long)]
        remote: Option<String>,

        /// Add as detached HEAD (read-only reference)
        #[arg(long)]
        detach: bool,
    },

    /// Remove a repository worktree from a rig
    Remove {
        /// Repository name, or rig name if a second argument is provided
        #[arg(value_name = "REPO_OR_RIG")]
        first: String,

        /// Repository name (when first argument is rig name)
        #[arg(value_name = "REPO")]
        second: Option<String>,

        /// Force removal even if worktree has uncommitted changes
        #[arg(short, long)]
        force: bool,

        /// Keep the branch in the source repo (default: branch is deleted)
        #[arg(long)]
        keep_branch: bool,
    },

    /// Destroy a rig and all its worktrees
    Destroy {
        /// Rig name
        name: String,

        /// Show what would be destroyed without actually removing anything
        #[arg(long)]
        dry_run: bool,

        /// Skip confirmation prompt
        #[arg(short = 'y', long)]
        yes: bool,

        /// Keep branches in source repos (default: branches are deleted)
        #[arg(long)]
        keep_branches: bool,
    },

    /// List all rigs
    List,

    /// Show rig status
    Status {
        /// Rig name (optional if inside a rig)
        name: Option<String>,
    },

    /// Fetch and rebase all repos onto their default branches
    Sync {
        /// Rig name (optional if inside a rig)
        name: Option<String>,

        /// Auto-stash uncommitted changes before rebasing
        #[arg(long)]
        stash: bool,
    },

    /// Re-detect default branches from remotes and update the manifest
    Refresh {
        /// Rig name (optional if inside a rig)
        name: Option<String>,
    },

    /// Run a command in every repo worktree (use -- before the command)
    #[command(after_help = "Examples:\n  git rig exec -- git status\n  git rig exec --repo my-repo -- make test\n  git rig exec -- sh -c 'grep foo | wc -l'")]
    Exec {
        /// Rig name (optional if inside a rig)
        #[arg(short = 'w', long = "rig")]
        rig: Option<String>,

        /// Run only in specific repos (can be repeated)
        #[arg(short, long = "repo", value_name = "REPO")]
        repos: Vec<String>,

        /// Stop at the first repo whose command fails
        #[arg(long)]
        fail_fast: bool,

        /// The command to run (must be preceded by --)
        #[arg(trailing_var_arg = true, allow_hyphen_values = true, required = true)]
        cmd: Vec<String>,
    },
}

fn main() -> Result<()> {
    // Reset SIGPIPE to default behavior so piping (e.g., `git rig status | head`)
    // doesn't cause a panic on broken pipe.
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }

    if std::process::Command::new("git")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_err()
    {
        eprintln!("error: git is not installed or not in PATH");
        std::process::exit(1);
    }

    let cli = Cli::parse();

    match cli.command {
        Commands::Create { name } => commands::create(&name),
        Commands::Add {
            first,
            second,
            name,
            branch,
            remote,
            detach,
        } => {
            let (ws_name, repo_path) = split_ws_and_arg(first, second);
            commands::add(
                ws_name.as_deref(),
                &repo_path,
                name.as_deref(),
                branch.as_deref(),
                remote.as_deref(),
                detach,
            )
        }
        Commands::Remove {
            first,
            second,
            force,
            keep_branch,
        } => {
            let (ws_name, repo) = split_ws_and_arg(first, second);
            commands::remove(ws_name.as_deref(), &repo, force, keep_branch)
        }
        Commands::Destroy { name, dry_run, yes, keep_branches } => commands::destroy(&name, dry_run, yes, keep_branches),
        Commands::List => commands::list(),
        Commands::Status { name } => commands::status(name.as_deref()),
        Commands::Sync { name, stash } => commands::sync(name.as_deref(), stash),
        Commands::Refresh { name } => commands::refresh(name.as_deref()),
        Commands::Exec {
            rig,
            repos,
            fail_fast,
            cmd,
        } => commands::exec(rig.as_deref(), &repos, &cmd, fail_fast),
    }
}

/// When two positional args are given, first is rig name and second is the arg (path/name).
/// When only one is given, it's the arg and rig is inferred from CWD.
fn split_ws_and_arg(first: String, second: Option<String>) -> (Option<String>, String) {
    match second {
        Some(arg) => (Some(first), arg),
        None => (None, first),
    }
}
