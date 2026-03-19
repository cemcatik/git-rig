use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;

// Import production types so test fixtures stay in sync with the real schema.
#[path = "../../src/workspace.rs"]
mod workspace;
use workspace::{Manifest, RepoEntry};

pub struct TestSandbox {
    pub dir: TempDir,
    /// Cached canonical path (avoids repeated `canonicalize()` syscalls).
    canonical: PathBuf,
}

impl TestSandbox {
    pub fn new() -> Self {
        let dir = TempDir::new().expect("failed to create temp dir");
        let canonical = dir.path().canonicalize().expect("canonicalize sandbox");
        Self { dir, canonical }
    }

    /// Canonical root path (resolves macOS /var → /private/var).
    pub fn path(&self) -> PathBuf {
        self.canonical.clone()
    }

    /// Create a bare remote + clone with an initial commit and `origin/HEAD` set.
    /// Returns the path to the clone (not the bare remote).
    pub fn create_repo(&self, name: &str) -> PathBuf {
        self.create_repo_with_branch(name, "main")
    }

    /// Like `create_repo` but with a custom default branch name.
    pub fn create_repo_with_branch(&self, name: &str, branch: &str) -> PathBuf {
        let root = self.path();
        let bare_dir = root.join(format!("{name}.git"));
        let clone_dir = root.join(name);

        // Create bare repo with the requested default branch
        std::fs::create_dir_all(&bare_dir).expect("create bare dir");
        git(&bare_dir, &["init", "--bare", "-b", branch]);

        // Clone it
        let bare_str = bare_dir.to_str().unwrap();
        let clone_str = clone_dir.to_str().unwrap();
        run_git(&root, &["clone", bare_str, clone_str]);

        // Make an initial commit so HEAD is valid
        self.commit_file(name, "README.md", "# init\n", "init");

        // Push to set up remote tracking
        git(&clone_dir, &["push", "-u", "origin", branch]);

        // Ensure origin/HEAD is set (git clone usually does this, but be explicit)
        git(
            &clone_dir,
            &["remote", "set-head", "origin", "--auto"],
        );

        clone_dir
    }

    /// Create a workspace directory with a `.ws.json` manifest (no repos).
    pub fn create_workspace(&self, name: &str) -> PathBuf {
        let ws_dir = self.path().join(name);
        std::fs::create_dir_all(&ws_dir).expect("create ws dir");

        let manifest = Manifest::new(name);
        manifest.save(&ws_dir).expect("write .ws.json");

        ws_dir
    }

    /// Create a workspace and add repos as worktrees.
    /// Returns the workspace directory.
    pub fn create_workspace_with_repos(&self, ws_name: &str, repo_names: &[&str]) -> PathBuf {
        let ws_dir = self.create_workspace(ws_name);
        let branch = format!("ws/{ws_name}");

        let mut manifest = Manifest::load(&ws_dir).expect("load manifest");

        for &repo_name in repo_names {
            let repo_dir = self.create_repo(repo_name);
            let worktree_path = ws_dir.join(repo_name);
            let wt_str = worktree_path.to_str().unwrap();

            // Create a new branch worktree from origin/main
            git(
                &repo_dir,
                &["worktree", "add", "-b", &branch, wt_str, "origin/main"],
            );

            manifest.add_repo(RepoEntry {
                name: repo_name.to_string(),
                source: repo_dir,
                branch: branch.clone(),
                default_branch: "main".to_string(),
                remote: "origin".to_string(),
            });
        }

        manifest.save(&ws_dir).expect("write .ws.json");

        ws_dir
    }

    /// Add + commit a file in the clone directory (not bare).
    pub fn commit_file(&self, repo_name: &str, file: &str, content: &str, msg: &str) {
        let repo_dir = self.path().join(repo_name);
        let file_path = repo_dir.join(file);

        if let Some(parent) = file_path.parent() {
            std::fs::create_dir_all(parent).expect("create parent dirs");
        }
        std::fs::write(&file_path, content).expect("write file");
        git(&repo_dir, &["add", file]);
        git(&repo_dir, &["commit", "-m", msg]);
    }

    /// Modify a tracked file without committing (makes repo dirty).
    pub fn make_dirty(&self, repo_name: &str, file: &str, content: &str) {
        let file_path = self.path().join(repo_name).join(file);
        std::fs::write(&file_path, content).expect("write dirty file");
    }

    /// Run a git command in a directory within the sandbox, panic on failure.
    pub fn git(&self, dir_name: &str, args: &[&str]) -> String {
        let dir = self.path().join(dir_name);
        git(&dir, args)
    }
}

/// Run a git command in an arbitrary directory, returning stdout. Panics on failure.
pub fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@test.com")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@test.com")
        .output()
        .unwrap_or_else(|e| panic!("failed to run git {args:?}: {e}"));

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        panic!("git {args:?} failed in {}: {stderr}", dir.display());
    }

    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

/// Like `git()` but uses `current_dir` — needed for `git clone` where the target doesn't exist yet.
fn run_git(cwd: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(cwd)
        .args(args)
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@test.com")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@test.com")
        .output()
        .unwrap_or_else(|e| panic!("failed to run git {args:?}: {e}"));

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        panic!("git {args:?} failed in {}: {stderr}", cwd.display());
    }

    String::from_utf8_lossy(&output.stdout).trim().to_string()
}
