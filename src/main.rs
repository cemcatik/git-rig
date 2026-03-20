use anyhow::Result;
use clap::{Parser, Subcommand};

mod commands;
mod git;
mod workspace;

#[derive(Parser)]
#[command(name = "ws", version, about = "Git worktree workspace manager")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Create a new workspace in the current directory
    Create {
        /// Workspace name (created as a subdirectory of CWD)
        name: String,
    },

    /// Add a repository worktree to a workspace
    Add {
        /// Path to local repo, or workspace name if a second argument is provided
        #[arg(value_name = "PATH_OR_WORKSPACE")]
        first: String,

        /// Path to local repo (when first argument is workspace name)
        #[arg(value_name = "PATH")]
        second: Option<String>,

        /// Name for the repo in the workspace (default: directory basename)
        #[arg(short, long)]
        name: Option<String>,

        /// Branch to check out or create (default: ws/<workspace-name>)
        #[arg(short, long)]
        branch: Option<String>,

        /// Git remote to fetch from (default: origin)
        #[arg(short, long)]
        remote: Option<String>,

        /// Add as detached HEAD (read-only reference)
        #[arg(long)]
        detach: bool,
    },

    /// Remove a repository worktree from a workspace
    Remove {
        /// Repository name, or workspace name if a second argument is provided
        #[arg(value_name = "REPO_OR_WORKSPACE")]
        first: String,

        /// Repository name (when first argument is workspace name)
        #[arg(value_name = "REPO")]
        second: Option<String>,

        /// Force removal even if worktree has uncommitted changes
        #[arg(short, long)]
        force: bool,

        /// Also delete the branch from the source repo after removal
        #[arg(long)]
        delete_branch: bool,
    },

    /// Destroy a workspace and all its worktrees
    Destroy {
        /// Workspace name
        name: String,

        /// Show what would be destroyed without actually removing anything
        #[arg(long)]
        dry_run: bool,

        /// Skip confirmation prompt
        #[arg(short = 'y', long)]
        yes: bool,
    },

    /// List all workspaces
    List,

    /// Show workspace status
    Status {
        /// Workspace name (optional if inside a workspace)
        name: Option<String>,
    },

    /// Fetch and rebase all repos onto their default branches
    Sync {
        /// Workspace name (optional if inside a workspace)
        name: Option<String>,

        /// Auto-stash uncommitted changes before rebasing
        #[arg(long)]
        stash: bool,
    },

    /// Re-detect default branches from remotes and update the manifest
    Refresh {
        /// Workspace name (optional if inside a workspace)
        name: Option<String>,
    },

    /// Run a command in every repo worktree
    Exec {
        /// Workspace name (optional if inside a workspace)
        #[arg(short = 'w', long = "workspace")]
        workspace: Option<String>,

        /// Run only in specific repos (can be repeated)
        #[arg(short, long = "repo", value_name = "REPO")]
        repos: Vec<String>,

        /// Stop at the first repo whose command fails
        #[arg(long)]
        fail_fast: bool,

        /// The command to run (everything after --)
        #[arg(trailing_var_arg = true, allow_hyphen_values = true, required = true)]
        cmd: Vec<String>,
    },
}

fn main() -> Result<()> {
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
            delete_branch,
        } => {
            let (ws_name, repo) = split_ws_and_arg(first, second);
            commands::remove(ws_name.as_deref(), &repo, force, delete_branch)
        }
        Commands::Destroy { name, dry_run, yes } => commands::destroy(&name, dry_run, yes),
        Commands::List => commands::list(),
        Commands::Status { name } => commands::status(name.as_deref()),
        Commands::Sync { name, stash } => commands::sync(name.as_deref(), stash),
        Commands::Refresh { name } => commands::refresh(name.as_deref()),
        Commands::Exec {
            workspace,
            repos,
            fail_fast,
            cmd,
        } => commands::exec(workspace.as_deref(), &repos, &cmd, fail_fast),
    }
}

/// When two positional args are given, first is workspace name and second is the arg (path/name).
/// When only one is given, it's the arg and workspace is inferred from CWD.
fn split_ws_and_arg(first: String, second: Option<String>) -> (Option<String>, String) {
    match second {
        Some(arg) => (Some(first), arg),
        None => (None, first),
    }
}
