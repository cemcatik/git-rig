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
        /// Repository name, or workspace name if a second argument is provided
        #[arg(value_name = "REPO_OR_WORKSPACE")]
        first: String,

        /// Repository name (when first argument is workspace name)
        #[arg(value_name = "REPO")]
        second: Option<String>,

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
    },

    /// Destroy a workspace and all its worktrees
    Destroy {
        /// Workspace name
        name: String,
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
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Create { name } => commands::create(&name),
        Commands::Add {
            first,
            second,
            branch,
            remote,
            detach,
        } => {
            let (ws_name, repo) = split_ws_and_repo(first, second);
            commands::add(
                ws_name.as_deref(),
                &repo,
                branch.as_deref(),
                remote.as_deref(),
                detach,
            )
        }
        Commands::Remove { first, second } => {
            let (ws_name, repo) = split_ws_and_repo(first, second);
            commands::remove(ws_name.as_deref(), &repo)
        }
        Commands::Destroy { name } => commands::destroy(&name),
        Commands::List => commands::list(),
        Commands::Status { name } => commands::status(name.as_deref()),
        Commands::Sync { name, stash } => commands::sync(name.as_deref(), stash),
    }
}

/// When two positional args are given, first is workspace name and second is repo.
/// When only one is given, it's the repo name and workspace is inferred from CWD.
fn split_ws_and_repo(first: String, second: Option<String>) -> (Option<String>, String) {
    match second {
        Some(repo) => (Some(first), repo),
        None => (None, first),
    }
}
