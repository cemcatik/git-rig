use anyhow::Result;
use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::Shell;

mod commands;
mod drift;
mod error;
mod git;
mod parallel;
mod provision;
mod workspace;

#[derive(Parser)]
#[command(
    name = "git-rig",
    version,
    about = "Multi-repo rig manager using git worktrees"
)]
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

        /// Clone repos from an existing rig
        #[arg(long, value_name = "SOURCE_RIG")]
        from: Option<String>,

        /// Skip invalid source repos instead of failing
        #[arg(long, requires = "from")]
        skip: bool,

        /// Skip copying local files from .riginclude
        #[arg(long, requires = "from")]
        no_provision: bool,

        /// Symlink local files instead of copying
        #[arg(long, requires = "from")]
        link: bool,

        /// Overwrite existing local files in the target
        #[arg(long, requires = "from")]
        force_provision: bool,
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

        /// Remote branch to sync against (default: repo's default branch)
        #[arg(long, conflicts_with = "detach")]
        upstream: Option<String>,

        /// Clear a previously set upstream branch
        #[arg(long, conflicts_with_all = ["detach", "upstream"])]
        no_upstream: bool,

        /// Skip copying local files from .riginclude
        #[arg(long)]
        no_provision: bool,

        /// Symlink local files instead of copying
        #[arg(long)]
        link: bool,

        /// Overwrite existing local files in the target
        #[arg(long)]
        force_provision: bool,
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

    /// Fetch and rebase all repos onto their upstream branches
    Sync {
        /// Rig name (optional if inside a rig)
        name: Option<String>,

        /// Run only in specific repos (can be repeated)
        #[arg(short, long = "repo", value_name = "REPO")]
        repos: Vec<String>,

        /// Auto-stash uncommitted changes before rebasing
        #[arg(long)]
        stash: bool,

        /// Number of parallel jobs (default: auto, -j1 for sequential)
        #[arg(short, long)]
        jobs: Option<usize>,
    },

    /// Re-detect default branches from remotes and update the manifest
    Refresh {
        /// Rig name (optional if inside a rig)
        name: Option<String>,

        /// Number of parallel jobs (default: auto, -j1 for sequential)
        #[arg(short, long)]
        jobs: Option<usize>,
    },

    /// Run a command in every repo worktree (use -- before the command)
    #[command(
        after_help = "Examples:\n  git rig exec -- git status\n  git rig exec --repo my-repo -- make test\n  git rig exec -- sh -c 'grep foo | wc -l'"
    )]
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

        /// Number of parallel jobs (default: auto, -j1 for sequential)
        #[arg(short, long)]
        jobs: Option<usize>,

        /// The command to run (must be preceded by --)
        #[arg(trailing_var_arg = true, allow_hyphen_values = true, required = true)]
        cmd: Vec<String>,
    },

    /// Check environment and workspace health
    Doctor {
        /// Rig name (optional if inside a rig)
        name: Option<String>,
    },

    /// Generate shell completions
    #[command(
        after_help = "Examples:\n  git rig completions bash > ~/.bash_completion.d/git-rig\n  git rig completions zsh > ~/.zfunc/_git-rig\n  git rig completions fish > ~/.config/fish/completions/git-rig.fish"
    )]
    Completions {
        /// Shell to generate completions for
        shell: Shell,
    },
}

fn main() -> Result<()> {
    // Reset SIGPIPE to default behavior so piping (e.g., `git rig status | head`)
    // doesn't cause a panic on broken pipe.
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }

    let cli = Cli::parse();

    // Doctor handles its own git check (reports it as FAIL instead of hard exit).
    // Completions don't need git. All other commands require git on PATH.
    if !matches!(
        cli.command,
        Commands::Doctor { .. } | Commands::Completions { .. }
    ) && !git::is_git_available()
    {
        eprintln!("error: git is not installed or not in PATH");
        std::process::exit(1);
    }

    match cli.command {
        Commands::Create {
            name,
            from,
            skip,
            no_provision,
            link,
            force_provision,
        } => {
            let provision = if no_provision {
                None
            } else {
                Some(provision::ProvisionOpts {
                    force: force_provision,
                    link,
                })
            };
            commands::create(&name, from.as_deref(), skip, provision)
        }
        Commands::Add {
            first,
            second,
            name,
            branch,
            remote,
            detach,
            upstream,
            no_upstream,
            no_provision,
            link,
            force_provision,
        } => {
            let (ws_name, repo_path) = split_ws_and_arg(first, second);
            let provision = if no_provision {
                None
            } else {
                Some(provision::ProvisionOpts {
                    force: force_provision,
                    link,
                })
            };
            commands::add(
                ws_name.as_deref(),
                &repo_path,
                commands::AddOptions {
                    name: name.as_deref(),
                    branch: branch.as_deref(),
                    remote: remote.as_deref(),
                    detach,
                    upstream: upstream.as_deref(),
                    no_upstream,
                },
                provision,
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
        Commands::Destroy {
            name,
            dry_run,
            yes,
            keep_branches,
        } => commands::destroy(&name, dry_run, yes, keep_branches),
        Commands::List => commands::list(),
        Commands::Status { name } => commands::status(name.as_deref()),
        Commands::Sync {
            name,
            repos,
            stash,
            jobs,
        } => commands::sync(name.as_deref(), &repos, stash, jobs),
        Commands::Refresh { name, jobs } => commands::refresh(name.as_deref(), jobs),
        Commands::Exec {
            rig,
            repos,
            fail_fast,
            jobs,
            cmd,
        } => commands::exec(rig.as_deref(), &repos, &cmd, fail_fast, jobs),
        Commands::Doctor { name } => commands::doctor(name.as_deref()),
        Commands::Completions { shell } => {
            clap_complete::generate(
                shell,
                &mut Cli::command(),
                "git-rig",
                &mut std::io::stdout(),
            );
            Ok(())
        }
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
