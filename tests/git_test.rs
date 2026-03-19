mod common;

#[path = "../src/git.rs"]
mod git;

// ---------------------------------------------------------------------------
// Branch detection
// ---------------------------------------------------------------------------

#[test]
fn default_branch_detects_main() {
    let sandbox = common::TestSandbox::new();
    let clone = sandbox.create_repo("main-repo");
    let branch = git::default_branch(&clone, "origin").unwrap();
    assert_eq!(branch, "main");
}

#[test]
fn default_branch_detects_master() {
    let sandbox = common::TestSandbox::new();
    let clone = sandbox.create_repo_with_branch("master-repo", "master");
    let branch = git::default_branch(&clone, "origin").unwrap();
    assert_eq!(branch, "master");
}

#[test]
fn default_branch_detects_custom() {
    let sandbox = common::TestSandbox::new();
    let clone = sandbox.create_repo_with_branch("custom-repo", "develop");
    let branch = git::default_branch(&clone, "origin").unwrap();
    assert_eq!(branch, "develop");
}

#[test]
fn default_branch_errors_when_no_default() {
    let sandbox = common::TestSandbox::new();
    let clone = sandbox.create_repo_with_branch("no-default", "develop");
    // Unset origin/HEAD so symbolic-ref lookup fails
    common::git(&clone, &["remote", "set-head", "origin", "-d"]);
    // Neither "main" nor "master" exist — only "develop"
    let result = git::default_branch(&clone, "origin");
    assert!(result.is_err());
}

#[test]
fn branch_exists_true_for_existing() {
    let sandbox = common::TestSandbox::new();
    let clone = sandbox.create_repo("branch-exists-true");
    assert!(git::branch_exists(&clone, "main"));
}

#[test]
fn branch_exists_false_for_nonexistent() {
    let sandbox = common::TestSandbox::new();
    let clone = sandbox.create_repo("branch-exists-false");
    assert!(!git::branch_exists(&clone, "nonexistent-branch"));
}

#[test]
fn remote_branch_exists_true_for_pushed() {
    let sandbox = common::TestSandbox::new();
    let clone = sandbox.create_repo("remote-branch-true");
    assert!(git::remote_branch_exists(&clone, "main", "origin"));
}

#[test]
fn remote_branch_exists_false_for_nonexistent() {
    let sandbox = common::TestSandbox::new();
    let clone = sandbox.create_repo("remote-branch-false");
    assert!(!git::remote_branch_exists(&clone, "nonexistent-branch", "origin"));
}

#[test]
fn current_branch_returns_name() {
    let sandbox = common::TestSandbox::new();
    let clone = sandbox.create_repo("current-branch");
    let branch = git::current_branch(&clone).unwrap();
    assert_eq!(branch, "main");
}

#[test]
fn current_branch_detached_returns_detached() {
    let sandbox = common::TestSandbox::new();
    let clone = sandbox.create_repo("current-branch-detached");
    common::git(&clone, &["checkout", "--detach"]);
    let branch = git::current_branch(&clone).unwrap();
    assert_eq!(branch, "(detached)");
}

// ---------------------------------------------------------------------------
// Worktree ops
// ---------------------------------------------------------------------------

#[test]
fn worktree_add_new_branch_creates_worktree() {
    let sandbox = common::TestSandbox::new();
    let clone = sandbox.create_repo("wt-new-branch");
    let wt_path = sandbox.path().join("wt-new-branch-wt");
    git::worktree_add_new_branch(&clone, &wt_path, "feature/test", "origin/main").unwrap();
    assert!(wt_path.exists());
    let branch = git::current_branch(&wt_path).unwrap();
    assert_eq!(branch, "feature/test");
}

#[test]
fn worktree_add_existing_creates_worktree() {
    let sandbox = common::TestSandbox::new();
    let clone = sandbox.create_repo("wt-existing");
    common::git(&clone, &["checkout", "-b", "existing-branch"]);
    common::git(&clone, &["checkout", "main"]);
    let wt_path = sandbox.path().join("wt-existing-wt");
    git::worktree_add_existing(&clone, &wt_path, "existing-branch").unwrap();
    assert!(wt_path.exists());
    let branch = git::current_branch(&wt_path).unwrap();
    assert_eq!(branch, "existing-branch");
}

#[test]
fn worktree_add_detached_creates_detached() {
    let sandbox = common::TestSandbox::new();
    let clone = sandbox.create_repo("wt-detach");
    let wt_path = sandbox.path().join("wt-detach-wt");
    git::worktree_add_detached(&clone, &wt_path, "HEAD").unwrap();
    assert!(wt_path.exists());
    let branch = git::current_branch(&wt_path).unwrap();
    assert_eq!(branch, "(detached)");
}

#[test]
fn worktree_remove_clean_succeeds() {
    let sandbox = common::TestSandbox::new();
    let clone = sandbox.create_repo("wt-rm-clean");
    let wt_path = sandbox.path().join("wt-rm-clean-wt");
    git::worktree_add_new_branch(&clone, &wt_path, "rm-clean", "origin/main").unwrap();
    git::worktree_remove(&clone, &wt_path, false).unwrap();
    assert!(!wt_path.exists());
}

#[test]
fn worktree_remove_dirty_with_force_succeeds() {
    let sandbox = common::TestSandbox::new();
    let clone = sandbox.create_repo("wt-rm-dirty-force");
    let wt_path = sandbox.path().join("wt-rm-dirty-force-wt");
    git::worktree_add_new_branch(&clone, &wt_path, "rm-dirty-force", "origin/main").unwrap();
    // Modify a tracked file to make the worktree dirty
    std::fs::write(wt_path.join("README.md"), "modified content").unwrap();
    git::worktree_remove(&clone, &wt_path, true).unwrap();
    assert!(!wt_path.exists());
}

#[test]
fn worktree_remove_dirty_without_force_fails() {
    let sandbox = common::TestSandbox::new();
    let clone = sandbox.create_repo("wt-rm-dirty-noforce");
    let wt_path = sandbox.path().join("wt-rm-dirty-noforce-wt");
    git::worktree_add_new_branch(&clone, &wt_path, "rm-dirty-noforce", "origin/main").unwrap();
    // Modify a tracked file to make the worktree dirty
    std::fs::write(wt_path.join("README.md"), "modified content").unwrap();
    let result = git::worktree_remove(&clone, &wt_path, false);
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// Status
// ---------------------------------------------------------------------------

#[test]
fn is_dirty_clean_repo_returns_false() {
    let sandbox = common::TestSandbox::new();
    let clone = sandbox.create_repo("dirty-clean");
    assert!(!git::is_dirty(&clone).unwrap());
}

#[test]
fn is_dirty_modified_tracked_file_returns_true() {
    let sandbox = common::TestSandbox::new();
    let clone = sandbox.create_repo("dirty-modified");
    std::fs::write(clone.join("README.md"), "modified content").unwrap();
    assert!(git::is_dirty(&clone).unwrap());
}

#[test]
fn is_dirty_untracked_file_returns_true() {
    let sandbox = common::TestSandbox::new();
    let clone = sandbox.create_repo("dirty-untracked");
    std::fs::write(clone.join("untracked.txt"), "new file").unwrap();
    assert!(git::is_dirty(&clone).unwrap());
}

#[test]
fn ahead_behind_even_returns_zero_zero() {
    let sandbox = common::TestSandbox::new();
    let clone = sandbox.create_repo("ab-even");
    let (ahead, behind) = git::ahead_behind(&clone, "main", "main", "origin").unwrap();
    assert_eq!((ahead, behind), (0, 0));
}

#[test]
fn ahead_behind_local_ahead_returns_n_zero() {
    let sandbox = common::TestSandbox::new();
    let clone = sandbox.create_repo("ab-ahead");
    // Commit locally without pushing
    sandbox.commit_file("ab-ahead", "b.txt", "content", "second commit");
    let (ahead, behind) = git::ahead_behind(&clone, "main", "main", "origin").unwrap();
    assert_eq!((ahead, behind), (1, 0));
}

#[test]
fn last_commit_summary_includes_hash_and_message() {
    let sandbox = common::TestSandbox::new();
    let clone = sandbox.create_repo("last-commit");
    let summary = git::last_commit_summary(&clone).unwrap();
    assert!(!summary.is_empty());
    // Format: "<short-hash> <subject> (<relative-date>)"
    assert!(summary.contains("init"));
    // First token should be a hex short hash
    let hash = summary.split_whitespace().next().unwrap();
    assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
}

// ---------------------------------------------------------------------------
// Sync
// ---------------------------------------------------------------------------

#[test]
fn fetch_succeeds() {
    let sandbox = common::TestSandbox::new();
    let clone = sandbox.create_repo("fetch-test");
    git::fetch(&clone, "origin").unwrap();
}

#[test]
fn rebase_fast_forward_succeeds() {
    let sandbox = common::TestSandbox::new();
    let clone = sandbox.create_repo("rebase-test");
    // Commit + push so origin moves ahead
    sandbox.commit_file("rebase-test", "b.txt", "b", "second commit");
    common::git(&clone, &["push"]);
    // Roll local back one commit so it's behind origin/main
    common::git(&clone, &["reset", "--hard", "HEAD~1"]);
    git::fetch(&clone, "origin").unwrap();
    git::rebase(&clone, "main", "origin").unwrap();
    // Local HEAD should now match origin/main
    let head = git::rev_parse_short(&clone, "HEAD").unwrap();
    let origin_main = git::rev_parse_short(&clone, "origin/main").unwrap();
    assert_eq!(head, origin_main);
}

#[test]
fn stash_push_with_dirty_files_returns_true() {
    let sandbox = common::TestSandbox::new();
    let clone = sandbox.create_repo("stash-dirty");
    std::fs::write(clone.join("README.md"), "modified content").unwrap();
    let stashed = git::stash_push(&clone).unwrap();
    assert!(stashed);
}

#[test]
fn stash_push_with_clean_repo_returns_false() {
    let sandbox = common::TestSandbox::new();
    let clone = sandbox.create_repo("stash-clean");
    let stashed = git::stash_push(&clone).unwrap();
    assert!(!stashed);
}

#[test]
fn stash_pop_restores_stashed_changes() {
    let sandbox = common::TestSandbox::new();
    let clone = sandbox.create_repo("stash-pop");
    std::fs::write(clone.join("README.md"), "modified content").unwrap();
    assert!(git::is_dirty(&clone).unwrap());
    let stashed = git::stash_push(&clone).unwrap();
    assert!(stashed);
    assert!(!git::is_dirty(&clone).unwrap());
    git::stash_pop(&clone).unwrap();
    assert!(git::is_dirty(&clone).unwrap());
}

// ---------------------------------------------------------------------------
// Misc
// ---------------------------------------------------------------------------

#[test]
fn is_git_repo_true_for_git_repo() {
    let sandbox = common::TestSandbox::new();
    let clone = sandbox.create_repo("is-git-true");
    assert!(git::is_git_repo(&clone));
}

#[test]
fn is_git_repo_false_for_random_directory() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().canonicalize().unwrap();
    assert!(!git::is_git_repo(&path));
}

#[test]
fn delete_branch_succeeds_for_merged_branch() {
    let sandbox = common::TestSandbox::new();
    let clone = sandbox.create_repo("delete-branch");
    // Branch at same commit as main is considered merged
    common::git(&clone, &["checkout", "-b", "feature-to-delete"]);
    common::git(&clone, &["checkout", "main"]);
    git::delete_branch(&clone, "feature-to-delete").unwrap();
    assert!(!git::branch_exists(&clone, "feature-to-delete"));
}

#[test]
fn rev_parse_short_returns_short_hash() {
    let sandbox = common::TestSandbox::new();
    let clone = sandbox.create_repo("rev-parse");
    let short = git::rev_parse_short(&clone, "HEAD").unwrap();
    assert!(!short.is_empty());
    assert!(short.len() >= 4 && short.len() <= 12);
    assert!(short.chars().all(|c| c.is_ascii_hexdigit()));
}
