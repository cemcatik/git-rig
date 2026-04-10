mod common;
#[allow(dead_code)]
#[path = "../src/error.rs"]
mod error;

use std::os::unix::fs::PermissionsExt;

use assert_cmd::Command;
use predicates::prelude::*;

// ---------------------------------------------------------------------------
// create
// ---------------------------------------------------------------------------

#[test]
fn create_success() {
    let sandbox = common::TestSandbox::new();

    Command::cargo_bin("git-rig")
        .unwrap()
        .args(["create", "my-ws"])
        .current_dir(sandbox.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("ok"));

    assert!(sandbox.path().join("my-ws").join(".rig.json").exists());
}

#[test]
fn create_already_exists() {
    let sandbox = common::TestSandbox::new();

    Command::cargo_bin("git-rig")
        .unwrap()
        .args(["create", "my-ws"])
        .current_dir(sandbox.path())
        .assert()
        .success();

    Command::cargo_bin("git-rig")
        .unwrap()
        .args(["create", "my-ws"])
        .current_dir(sandbox.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("already exists"));
}

// ---------------------------------------------------------------------------
// add
// ---------------------------------------------------------------------------

#[test]
fn add_from_inside_workspace() {
    let sandbox = common::TestSandbox::new();
    let repo_path = sandbox.create_repo("repo-a");
    let ws_dir = sandbox.create_workspace("my-ws");

    Command::cargo_bin("git-rig")
        .unwrap()
        .arg("add")
        .arg(repo_path.to_str().unwrap())
        .current_dir(&ws_dir)
        .assert()
        .success()
        .stdout(predicate::str::contains("ok"));
}

#[test]
fn add_with_explicit_workspace_name() {
    let sandbox = common::TestSandbox::new();
    let repo_path = sandbox.create_repo("repo-a");
    sandbox.create_workspace("my-ws");

    Command::cargo_bin("git-rig")
        .unwrap()
        .arg("add")
        .arg("my-ws")
        .arg(repo_path.to_str().unwrap())
        .current_dir(sandbox.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("ok"));
}

#[test]
fn add_with_custom_name() {
    let sandbox = common::TestSandbox::new();
    let repo_path = sandbox.create_repo("repo-a");
    let ws_dir = sandbox.create_workspace("my-ws");

    Command::cargo_bin("git-rig")
        .unwrap()
        .args(["add", "--name", "custom"])
        .arg(repo_path.to_str().unwrap())
        .current_dir(&ws_dir)
        .assert()
        .success()
        .stdout(predicate::str::contains("ok"));

    assert!(ws_dir.join("custom").exists());
}

#[test]
fn add_with_custom_branch() {
    let sandbox = common::TestSandbox::new();
    let repo_path = sandbox.create_repo("repo-a");
    let ws_dir = sandbox.create_workspace("my-ws");

    Command::cargo_bin("git-rig")
        .unwrap()
        .args(["add", "--branch", "feature-branch"])
        .arg(repo_path.to_str().unwrap())
        .current_dir(&ws_dir)
        .assert()
        .success()
        .stdout(predicate::str::contains("ok"));
}

#[test]
fn add_detached() {
    let sandbox = common::TestSandbox::new();
    let repo_path = sandbox.create_repo("repo-a");
    let ws_dir = sandbox.create_workspace("my-ws");

    Command::cargo_bin("git-rig")
        .unwrap()
        .args(["add", "--detach"])
        .arg(repo_path.to_str().unwrap())
        .current_dir(&ws_dir)
        .assert()
        .success()
        .stdout(predicate::str::contains("ok"));
}

#[test]
fn add_duplicate_repo() {
    let sandbox = common::TestSandbox::new();
    let repo_path = sandbox.create_repo("repo-a");
    let ws_dir = sandbox.create_workspace("my-ws");

    Command::cargo_bin("git-rig")
        .unwrap()
        .arg("add")
        .arg(repo_path.to_str().unwrap())
        .current_dir(&ws_dir)
        .assert()
        .success();

    Command::cargo_bin("git-rig")
        .unwrap()
        .arg("add")
        .arg(repo_path.to_str().unwrap())
        .current_dir(&ws_dir)
        .assert()
        .failure()
        .stderr(predicate::str::contains("already in rig"));
}

#[test]
fn add_not_a_repo() {
    let sandbox = common::TestSandbox::new();
    let ws_dir = sandbox.create_workspace("my-ws");
    let non_git = sandbox.path().join("not-a-repo");
    std::fs::create_dir_all(&non_git).unwrap();

    Command::cargo_bin("git-rig")
        .unwrap()
        .arg("add")
        .arg(non_git.to_str().unwrap())
        .current_dir(&ws_dir)
        .assert()
        .failure()
        .stderr(predicate::str::contains("not a git repository"));
}

#[test]
fn add_with_upstream() {
    let sandbox = common::TestSandbox::new();
    let repo_path = sandbox.create_repo("repo-a");
    let ws_dir = sandbox.create_workspace("my-ws");

    // Create the upstream branch on the remote
    common::git(&repo_path, &["checkout", "-b", "integration"]);
    sandbox.commit_file("repo-a", "integration.txt", "new", "integration commit");
    common::git(&repo_path, &["push", "-u", "origin", "integration"]);
    common::git(&repo_path, &["checkout", "main"]);

    Command::cargo_bin("git-rig")
        .unwrap()
        .args(["add", "--upstream", "integration"])
        .arg(repo_path.to_str().unwrap())
        .current_dir(&ws_dir)
        .assert()
        .success()
        .stdout(predicate::str::contains("ok"))
        .stdout(predicate::str::contains("from integration"));

    // Verify upstream is stored in manifest
    let raw = std::fs::read_to_string(ws_dir.join(".rig.json")).unwrap();
    assert!(raw.contains(r#""upstream": "integration""#));

    // Verify worktree starts at the upstream branch's content
    assert!(ws_dir.join("repo-a").join("integration.txt").exists());
}

#[test]
fn add_upstream_update_existing_repo() {
    let sandbox = common::TestSandbox::new();
    let repo_path = sandbox.create_repo("repo-a");
    let ws_dir = sandbox.create_workspace("my-ws");

    // First add without upstream
    Command::cargo_bin("git-rig")
        .unwrap()
        .arg("add")
        .arg(repo_path.to_str().unwrap())
        .current_dir(&ws_dir)
        .assert()
        .success();

    // Verify no upstream in manifest
    let raw = std::fs::read_to_string(ws_dir.join(".rig.json")).unwrap();
    assert!(!raw.contains("upstream"));

    // Update with --upstream
    Command::cargo_bin("git-rig")
        .unwrap()
        .args(["add", "--upstream", "integration"])
        .arg(repo_path.to_str().unwrap())
        .current_dir(&ws_dir)
        .assert()
        .success()
        .stdout(predicate::str::contains("Set upstream"));

    // Verify upstream is now stored
    let raw = std::fs::read_to_string(ws_dir.join(".rig.json")).unwrap();
    assert!(raw.contains(r#""upstream": "integration""#));
}

#[test]
fn add_no_upstream_clears_existing() {
    let sandbox = common::TestSandbox::new();
    let repo_path = sandbox.create_repo("repo-a");
    let ws_dir = sandbox.create_workspace("my-ws");

    // Create the upstream branch on the remote
    common::git(&repo_path, &["checkout", "-b", "integration"]);
    sandbox.commit_file("repo-a", "integration.txt", "new", "integration commit");
    common::git(&repo_path, &["push", "-u", "origin", "integration"]);
    common::git(&repo_path, &["checkout", "main"]);

    // Add with upstream
    Command::cargo_bin("git-rig")
        .unwrap()
        .args(["add", "--upstream", "integration"])
        .arg(repo_path.to_str().unwrap())
        .current_dir(&ws_dir)
        .assert()
        .success();

    // Clear with --no-upstream
    Command::cargo_bin("git-rig")
        .unwrap()
        .args(["add", "--no-upstream"])
        .arg(repo_path.to_str().unwrap())
        .current_dir(&ws_dir)
        .assert()
        .success()
        .stdout(predicate::str::contains("Cleared upstream"));

    // Verify upstream is gone
    let raw = std::fs::read_to_string(ws_dir.join(".rig.json")).unwrap();
    assert!(!raw.contains("upstream"));
}

#[test]
fn add_duplicate_without_upstream_still_errors() {
    let sandbox = common::TestSandbox::new();
    let repo_path = sandbox.create_repo("repo-a");
    let ws_dir = sandbox.create_workspace("my-ws");

    Command::cargo_bin("git-rig")
        .unwrap()
        .arg("add")
        .arg(repo_path.to_str().unwrap())
        .current_dir(&ws_dir)
        .assert()
        .success();

    // Re-add without --upstream should still error
    Command::cargo_bin("git-rig")
        .unwrap()
        .arg("add")
        .arg(repo_path.to_str().unwrap())
        .current_dir(&ws_dir)
        .assert()
        .failure()
        .stderr(predicate::str::contains("already in rig"));
}

#[test]
fn add_upstream_conflicts_with_detach() {
    let sandbox = common::TestSandbox::new();
    let repo_path = sandbox.create_repo("repo-a");
    let ws_dir = sandbox.create_workspace("my-ws");

    Command::cargo_bin("git-rig")
        .unwrap()
        .args(["add", "--upstream", "integration", "--detach"])
        .arg(repo_path.to_str().unwrap())
        .current_dir(&ws_dir)
        .assert()
        .failure()
        .stderr(predicate::str::contains("cannot be used with"));
}

// ---------------------------------------------------------------------------
// remove
// ---------------------------------------------------------------------------

#[test]
fn remove_success() {
    let sandbox = common::TestSandbox::new();
    let ws_dir = sandbox.create_workspace_with_repos("my-ws", &["repo-a"]);

    Command::cargo_bin("git-rig")
        .unwrap()
        .args(["remove", "repo-a"])
        .current_dir(&ws_dir)
        .assert()
        .success()
        .stdout(predicate::str::contains("ok"));

    assert!(!ws_dir.join("repo-a").exists());
}

#[test]
fn remove_dirty_without_force() {
    let sandbox = common::TestSandbox::new();
    let ws_dir = sandbox.create_workspace_with_repos("my-ws", &["repo-a"]);

    // Write an untracked file into the worktree to make it dirty
    std::fs::write(ws_dir.join("repo-a").join("dirty.txt"), "dirty").unwrap();

    Command::cargo_bin("git-rig")
        .unwrap()
        .args(["remove", "repo-a"])
        .current_dir(&ws_dir)
        .assert()
        .failure()
        .stderr(predicate::str::contains("uncommitted changes"));
}

#[test]
fn remove_dirty_with_force() {
    let sandbox = common::TestSandbox::new();
    let ws_dir = sandbox.create_workspace_with_repos("my-ws", &["repo-a"]);

    std::fs::write(ws_dir.join("repo-a").join("dirty.txt"), "dirty").unwrap();

    Command::cargo_bin("git-rig")
        .unwrap()
        .args(["remove", "--force", "repo-a"])
        .current_dir(&ws_dir)
        .assert()
        .success()
        .stdout(predicate::str::contains("ok"));
}

#[test]
fn remove_nonexistent() {
    let sandbox = common::TestSandbox::new();
    let ws_dir = sandbox.create_workspace("my-ws");

    Command::cargo_bin("git-rig")
        .unwrap()
        .args(["remove", "repo-a"])
        .current_dir(&ws_dir)
        .assert()
        .failure()
        .stderr(predicate::str::contains("not in rig"));
}

#[test]
fn remove_deletes_branch_by_default() {
    let sandbox = common::TestSandbox::new();
    let ws_dir = sandbox.create_workspace_with_repos("my-ws", &["repo-a"]);

    Command::cargo_bin("git-rig")
        .unwrap()
        .args(["remove", "repo-a"])
        .current_dir(&ws_dir)
        .assert()
        .success()
        .stdout(predicate::str::contains("Deleted branch"));
}

#[test]
fn remove_keep_branch() {
    let sandbox = common::TestSandbox::new();
    let ws_dir = sandbox.create_workspace_with_repos("my-ws", &["repo-a"]);

    Command::cargo_bin("git-rig")
        .unwrap()
        .args(["remove", "--keep-branch", "repo-a"])
        .current_dir(&ws_dir)
        .assert()
        .success()
        .stdout(predicate::str::contains("Deleted branch").not());
}

#[test]
fn remove_after_workspace_moved() {
    let sandbox = common::TestSandbox::new();
    sandbox.create_workspace_with_repos("my-ws", &["repo-a"]);

    // Move the workspace directory, breaking git worktree links
    let new_dir = sandbox.move_workspace("my-ws", "moved-ws");

    Command::cargo_bin("git-rig")
        .unwrap()
        .args(["remove", "repo-a"])
        .current_dir(&new_dir)
        .assert()
        .success()
        .stdout(predicate::str::contains("ok"));

    assert!(!new_dir.join("repo-a").exists());

    // Branch should be deleted from the source repo
    let branches = sandbox.git("repo-a", &["branch", "--list", "rig/my-ws"]);
    assert!(
        branches.is_empty(),
        "branch rig/my-ws should have been deleted after moved-worktree remove"
    );
}

#[test]
fn remove_after_workspace_moved_with_force() {
    let sandbox = common::TestSandbox::new();
    sandbox.create_workspace_with_repos("my-ws", &["repo-a"]);

    let new_dir = sandbox.move_workspace("my-ws", "moved-ws");

    // Make worktree dirty then force-remove
    std::fs::write(new_dir.join("repo-a").join("dirty.txt"), "dirty").unwrap();

    Command::cargo_bin("git-rig")
        .unwrap()
        .args(["remove", "--force", "repo-a"])
        .current_dir(&new_dir)
        .assert()
        .success()
        .stdout(predicate::str::contains("ok"));

    assert!(!new_dir.join("repo-a").exists());

    // Branch should be deleted from the source repo even with force + moved worktree
    let branches = sandbox.git("repo-a", &["branch", "--list", "rig/my-ws"]);
    assert!(
        branches.is_empty(),
        "branch rig/my-ws should have been deleted after forced moved-worktree remove"
    );
}

#[test]
fn remove_with_corrupted_worktree_metadata() {
    let sandbox = common::TestSandbox::new();
    sandbox.create_workspace_with_repos("my-ws", &["repo-a"]);

    // Move workspace AND corrupt the source repo's worktree metadata.
    // This makes both worktree_remove and worktree_repair fail,
    // forcing the prune+rm fallback path (rung 3 of the recovery ladder).
    let new_dir = sandbox.move_workspace("my-ws", "moved-ws");
    sandbox.corrupt_worktree_metadata("repo-a");

    Command::cargo_bin("git-rig")
        .unwrap()
        .args(["remove", "--force", "repo-a"])
        .current_dir(&new_dir)
        .assert()
        .success()
        .stdout(predicate::str::contains("ok"));

    assert!(
        !new_dir.join("repo-a").exists(),
        "worktree directory should be removed via prune+rm fallback"
    );
}

// ---------------------------------------------------------------------------
// destroy
// ---------------------------------------------------------------------------

#[test]
fn destroy_success() {
    let sandbox = common::TestSandbox::new();
    sandbox.create_workspace("my-ws");

    Command::cargo_bin("git-rig")
        .unwrap()
        .args(["destroy", "--yes", "my-ws"])
        .current_dir(sandbox.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("ok"));

    assert!(!sandbox.path().join("my-ws").exists());
}

#[test]
fn destroy_dry_run() {
    let sandbox = common::TestSandbox::new();
    sandbox.create_workspace("my-ws");

    Command::cargo_bin("git-rig")
        .unwrap()
        .args(["destroy", "--dry-run", "my-ws"])
        .current_dir(sandbox.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Would destroy rig"));

    // Workspace must still exist after a dry run
    assert!(sandbox.path().join("my-ws").join(".rig.json").exists());
}

#[test]
fn destroy_nonexistent() {
    let sandbox = common::TestSandbox::new();

    Command::cargo_bin("git-rig")
        .unwrap()
        .args(["destroy", "--yes", "does-not-exist"])
        .current_dir(sandbox.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("not found"));
}

#[test]
fn destroy_without_yes_in_non_tty() {
    let sandbox = common::TestSandbox::new();
    sandbox.create_workspace("my-ws");

    Command::cargo_bin("git-rig")
        .unwrap()
        .args(["destroy", "my-ws"])
        .current_dir(sandbox.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("use --yes"));
}

#[test]
fn destroy_with_repos() {
    let sandbox = common::TestSandbox::new();
    let ws_dir = sandbox.create_workspace_with_repos("my-ws", &["repo-a", "repo-b"]);

    // Verify worktrees exist
    assert!(ws_dir.join("repo-a").exists());
    assert!(ws_dir.join("repo-b").exists());

    Command::cargo_bin("git-rig")
        .unwrap()
        .args(["destroy", "--yes", "my-ws"])
        .current_dir(sandbox.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("ok"));

    assert!(!ws_dir.exists());
}

#[test]
fn destroy_deletes_branches_by_default() {
    let sandbox = common::TestSandbox::new();
    let ws_dir = sandbox.create_workspace_with_repos("my-ws", &["repo-a"]);

    assert!(ws_dir.join("repo-a").exists());

    Command::cargo_bin("git-rig")
        .unwrap()
        .args(["destroy", "--yes", "my-ws"])
        .current_dir(sandbox.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Deleted branch"));

    // Branch should be gone from source repo
    let branches = sandbox.git("repo-a", &["branch", "--list", "rig/my-ws"]);
    assert!(
        branches.is_empty(),
        "branch rig/my-ws should have been deleted"
    );
}

#[test]
fn destroy_keep_branches() {
    let sandbox = common::TestSandbox::new();
    let ws_dir = sandbox.create_workspace_with_repos("my-ws", &["repo-a"]);

    assert!(ws_dir.join("repo-a").exists());

    Command::cargo_bin("git-rig")
        .unwrap()
        .args(["destroy", "--yes", "--keep-branches", "my-ws"])
        .current_dir(sandbox.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Deleted branch").not());

    // Branch should still exist in source repo
    let branches = sandbox.git("repo-a", &["branch", "--list", "rig/my-ws"]);
    assert!(!branches.is_empty(), "branch rig/my-ws should still exist");
}

#[test]
fn destroy_after_workspace_moved() {
    let sandbox = common::TestSandbox::new();
    sandbox.create_workspace_with_repos("my-ws", &["repo-a", "repo-b"]);

    // Move the workspace, breaking worktree links
    let new_dir = sandbox.move_workspace("my-ws", "moved-ws");

    assert!(new_dir.join("repo-a").exists());
    assert!(new_dir.join("repo-b").exists());

    Command::cargo_bin("git-rig")
        .unwrap()
        .args(["destroy", "--yes", "moved-ws"])
        .current_dir(sandbox.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("ok"));

    assert!(!new_dir.exists());

    // Branches should be deleted from source repos
    let branches_a = sandbox.git("repo-a", &["branch", "--list", "rig/my-ws"]);
    assert!(
        branches_a.is_empty(),
        "branch rig/my-ws should have been deleted from repo-a after moved-worktree destroy"
    );
    let branches_b = sandbox.git("repo-b", &["branch", "--list", "rig/my-ws"]);
    assert!(
        branches_b.is_empty(),
        "branch rig/my-ws should have been deleted from repo-b after moved-worktree destroy"
    );
}

#[test]
fn destroy_dry_run_with_repos() {
    let sandbox = common::TestSandbox::new();
    let ws_dir = sandbox.create_workspace_with_repos("my-ws", &["repo-a"]);

    Command::cargo_bin("git-rig")
        .unwrap()
        .args(["destroy", "--dry-run", "my-ws"])
        .current_dir(sandbox.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Would remove worktree: repo-a"));

    // Must still exist after dry run
    assert!(ws_dir.join("repo-a").exists());
}

// ---------------------------------------------------------------------------
// list
// ---------------------------------------------------------------------------

#[test]
fn list_empty() {
    let sandbox = common::TestSandbox::new();

    Command::cargo_bin("git-rig")
        .unwrap()
        .arg("list")
        .current_dir(sandbox.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("No rigs found"));
}

#[test]
fn list_multiple() {
    let sandbox = common::TestSandbox::new();
    sandbox.create_workspace("ws-alpha");
    sandbox.create_workspace("ws-beta");

    Command::cargo_bin("git-rig")
        .unwrap()
        .arg("list")
        .current_dir(sandbox.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("ws-alpha"))
        .stdout(predicate::str::contains("ws-beta"));
}

// ---------------------------------------------------------------------------
// status
// ---------------------------------------------------------------------------

#[test]
fn status_empty_workspace() {
    let sandbox = common::TestSandbox::new();
    let ws_dir = sandbox.create_workspace("my-ws");

    Command::cargo_bin("git-rig")
        .unwrap()
        .arg("status")
        .current_dir(&ws_dir)
        .assert()
        .success()
        .stdout(predicate::str::contains("No repos"));
}

#[test]
fn status_with_repos() {
    let sandbox = common::TestSandbox::new();
    let ws_dir = sandbox.create_workspace_with_repos("my-ws", &["repo-a", "repo-b"]);

    Command::cargo_bin("git-rig")
        .unwrap()
        .arg("status")
        .current_dir(&ws_dir)
        .assert()
        .success()
        .stdout(predicate::str::contains("repo-a"))
        .stdout(predicate::str::contains("repo-b"));
}

// ---------------------------------------------------------------------------
// sync
// ---------------------------------------------------------------------------

#[test]
fn sync_already_up_to_date() {
    let sandbox = common::TestSandbox::new();
    let ws_dir = sandbox.create_workspace_with_repos("my-ws", &["repo-a"]);

    Command::cargo_bin("git-rig")
        .unwrap()
        .arg("sync")
        .current_dir(&ws_dir)
        .assert()
        .success()
        .stdout(predicate::str::contains("already up to date"));
}

#[test]
fn sync_dirty_skip() {
    let sandbox = common::TestSandbox::new();
    let ws_dir = sandbox.create_workspace_with_repos("my-ws", &["repo-a"]);

    // Untracked file makes the worktree dirty without requiring a commit
    std::fs::write(ws_dir.join("repo-a").join("dirty.txt"), "dirty").unwrap();

    Command::cargo_bin("git-rig")
        .unwrap()
        .arg("sync")
        .current_dir(&ws_dir)
        .assert()
        .success()
        .stdout(predicate::str::contains("SKIP"));
}

#[test]
fn sync_with_stash() {
    let sandbox = common::TestSandbox::new();
    let ws_dir = sandbox.create_workspace_with_repos("my-ws", &["repo-a"]);

    // Modify a tracked file (README.md exists via origin/main) so that
    // `git stash push` has something to stash
    std::fs::write(ws_dir.join("repo-a").join("README.md"), "modified").unwrap();

    Command::cargo_bin("git-rig")
        .unwrap()
        .args(["sync", "--stash"])
        .current_dir(&ws_dir)
        .assert()
        .success()
        .stdout(predicate::str::contains("stash"));
}

#[test]
fn sync_fast_forward() {
    let sandbox = common::TestSandbox::new();
    let ws_dir = sandbox.create_workspace_with_repos("my-ws", &["repo-a"]);

    // Push a new commit to the bare remote from the source clone
    sandbox.commit_file("repo-a", "new-file.txt", "content", "upstream commit");
    // Push to bare remote so the workspace worktree can fetch it
    common::git(&sandbox.path().join("repo-a"), &["push"]);

    Command::cargo_bin("git-rig")
        .unwrap()
        .arg("sync")
        .current_dir(&ws_dir)
        .assert()
        .success()
        .stdout(predicate::str::contains("->")); // hash transition
}

#[test]
fn sync_with_custom_upstream() {
    let sandbox = common::TestSandbox::new();
    let ws_dir = sandbox.create_workspace_with_repos("my-ws", &["repo-a"]);
    let repo_dir = sandbox.path().join("repo-a");

    // Create an 'integration' branch on the remote with a new commit
    common::git(&repo_dir, &["checkout", "-b", "integration"]);
    sandbox.commit_file("repo-a", "integration.txt", "new", "integration commit");
    common::git(&repo_dir, &["push", "-u", "origin", "integration"]);
    common::git(&repo_dir, &["checkout", "main"]);

    // Set upstream to 'integration'
    Command::cargo_bin("git-rig")
        .unwrap()
        .args(["add", "--upstream", "integration"])
        .arg(repo_dir.to_str().unwrap())
        .current_dir(&ws_dir)
        .assert()
        .success()
        .stdout(predicate::str::contains("Set upstream"));

    // Sync should rebase onto origin/integration
    Command::cargo_bin("git-rig")
        .unwrap()
        .arg("sync")
        .current_dir(&ws_dir)
        .assert()
        .success()
        .stdout(predicate::str::contains("upstream: integration"));

    // Verify the worktree actually has the integration branch's content
    assert!(ws_dir.join("repo-a").join("integration.txt").exists());
}

#[test]
fn status_shows_upstream_indicator() {
    let sandbox = common::TestSandbox::new();
    let ws_dir = sandbox.create_workspace_with_repos("my-ws", &["repo-a"]);
    let repo_dir = sandbox.path().join("repo-a");

    // Create 'integration' branch on remote with an extra commit so we're behind
    common::git(&repo_dir, &["checkout", "-b", "integration"]);
    sandbox.commit_file("repo-a", "integration.txt", "new", "integration commit");
    common::git(&repo_dir, &["push", "-u", "origin", "integration"]);
    common::git(&repo_dir, &["checkout", "main"]);

    // Set upstream to 'integration' and fetch so the ref is available
    Command::cargo_bin("git-rig")
        .unwrap()
        .args(["add", "--upstream", "integration"])
        .arg(repo_dir.to_str().unwrap())
        .current_dir(&ws_dir)
        .assert()
        .success();

    // Fetch so the worktree knows about origin/integration
    common::git(&ws_dir.join("repo-a"), &["fetch", "origin"]);

    // Status should show "(vs integration)" and the behind count
    Command::cargo_bin("git-rig")
        .unwrap()
        .arg("status")
        .current_dir(&ws_dir)
        .assert()
        .success()
        .stdout(predicate::str::contains("vs integration"))
        .stdout(predicate::str::contains("-1"));
}

#[test]
fn sync_with_nonexistent_upstream_reports_error() {
    let sandbox = common::TestSandbox::new();
    let ws_dir = sandbox.create_workspace_with_repos("my-ws", &["repo-a"]);
    let repo_dir = sandbox.path().join("repo-a");

    // Set upstream to a branch that doesn't exist on the remote
    Command::cargo_bin("git-rig")
        .unwrap()
        .args(["add", "--upstream", "nonexistent-branch"])
        .arg(repo_dir.to_str().unwrap())
        .current_dir(&ws_dir)
        .assert()
        .success();

    // Sync should report an error for this repo
    Command::cargo_bin("git-rig")
        .unwrap()
        .arg("sync")
        .current_dir(&ws_dir)
        .assert()
        .failure()
        .stdout(predicate::str::contains("ERR"));
}

#[test]
fn list_shows_upstream_when_set() {
    let sandbox = common::TestSandbox::new();
    let ws_dir = sandbox.create_workspace_with_repos("my-ws", &["repo-a"]);
    let repo_dir = sandbox.path().join("repo-a");

    // Set upstream
    Command::cargo_bin("git-rig")
        .unwrap()
        .args(["add", "--upstream", "integration"])
        .arg(repo_dir.to_str().unwrap())
        .current_dir(&ws_dir)
        .assert()
        .success();

    // List should show the upstream arrow
    Command::cargo_bin("git-rig")
        .unwrap()
        .arg("list")
        .current_dir(&ws_dir)
        .assert()
        .success()
        .stdout(predicate::str::contains("integration"));
}

#[test]
fn refresh_does_not_modify_upstream() {
    let sandbox = common::TestSandbox::new();
    let ws_dir = sandbox.create_workspace_with_repos("my-ws", &["repo-a"]);
    let repo_dir = sandbox.path().join("repo-a");

    // Set upstream
    Command::cargo_bin("git-rig")
        .unwrap()
        .args(["add", "--upstream", "integration"])
        .arg(repo_dir.to_str().unwrap())
        .current_dir(&ws_dir)
        .assert()
        .success();

    // Run refresh
    Command::cargo_bin("git-rig")
        .unwrap()
        .arg("refresh")
        .current_dir(&ws_dir)
        .assert()
        .success();

    // Verify upstream is preserved
    let raw = std::fs::read_to_string(ws_dir.join(".rig.json")).unwrap();
    assert!(raw.contains(r#""upstream": "integration""#));
}

// ---------------------------------------------------------------------------
// refresh
// ---------------------------------------------------------------------------

#[test]
fn refresh_no_change() {
    let sandbox = common::TestSandbox::new();
    let ws_dir = sandbox.create_workspace_with_repos("my-ws", &["repo-a"]);

    Command::cargo_bin("git-rig")
        .unwrap()
        .arg("refresh")
        .current_dir(&ws_dir)
        .assert()
        .success()
        .stdout(predicate::str::contains("unchanged"));
}

// ---------------------------------------------------------------------------
// exec
// ---------------------------------------------------------------------------

#[test]
fn exec_all_repos() {
    let sandbox = common::TestSandbox::new();
    let ws_dir = sandbox.create_workspace_with_repos("my-ws", &["repo-a", "repo-b"]);

    Command::cargo_bin("git-rig")
        .unwrap()
        .args(["exec", "--", "echo", "hello"])
        .current_dir(&ws_dir)
        .assert()
        .success()
        .stdout(predicate::str::contains("repo-a"))
        .stdout(predicate::str::contains("repo-b"));
}

#[test]
fn exec_repo_filter() {
    let sandbox = common::TestSandbox::new();
    let ws_dir = sandbox.create_workspace_with_repos("my-ws", &["repo-a", "repo-b"]);

    Command::cargo_bin("git-rig")
        .unwrap()
        .args(["exec", "--repo", "repo-a", "--", "echo", "hello"])
        .current_dir(&ws_dir)
        .assert()
        .success()
        .stdout(predicate::str::contains(">>> repo-a"))
        .stdout(predicate::str::contains("hello"))
        .stdout(predicate::str::contains(">>> repo-b").not());
}

#[test]
fn exec_fail_fast() {
    let sandbox = common::TestSandbox::new();
    let ws_dir = sandbox.create_workspace_with_repos("my-ws", &["repo-a", "repo-b"]);

    // `false` exits 1; --fail-fast should stop after repo-a and skip repo-b
    Command::cargo_bin("git-rig")
        .unwrap()
        .args(["exec", "--fail-fast", "--", "false"])
        .current_dir(&ws_dir)
        .assert()
        .failure() // exec exits non-zero when any repo fails
        .stdout(predicate::str::contains("WARN"))
        .stdout(predicate::str::contains(">>> repo-b").not());
}

#[test]
fn exec_failure_continues_all_repos() {
    let sandbox = common::TestSandbox::new();
    let ws_dir = sandbox.create_workspace_with_repos("my-ws", &["repo-a", "repo-b"]);

    // false exits 1; without --fail-fast both repos should be attempted
    Command::cargo_bin("git-rig")
        .unwrap()
        .args(["exec", "--", "false"])
        .current_dir(&ws_dir)
        .assert()
        .failure()
        .stdout(predicate::str::contains(">>> repo-a"))
        .stdout(predicate::str::contains(">>> repo-b"))
        .stdout(predicate::str::contains("WARN"));
}

#[test]
fn exec_invalid_repo_filter() {
    let sandbox = common::TestSandbox::new();
    let ws_dir = sandbox.create_workspace_with_repos("my-ws", &["repo-a"]);

    Command::cargo_bin("git-rig")
        .unwrap()
        .args(["exec", "--repo", "nonexistent", "--", "echo", "hi"])
        .current_dir(&ws_dir)
        .assert()
        .failure()
        .stderr(predicate::str::contains("not in rig"));
}

// ---------------------------------------------------------------------------
// create --from
// ---------------------------------------------------------------------------

#[test]
fn create_from_happy_path() {
    let sandbox = common::TestSandbox::new();
    let _ws_dir = sandbox.create_workspace_with_repos("source-ws", &["repo-a", "repo-b"]);

    Command::cargo_bin("git-rig")
        .unwrap()
        .args(["create", "target-ws", "--from", "source-ws"])
        .current_dir(sandbox.path())
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Created rig 'target-ws' from 'source-ws'",
        ))
        .stdout(predicate::str::contains("2 repos"));

    // Verify worktrees exist
    let target = sandbox.path().join("target-ws");
    assert!(target.join("repo-a").exists());
    assert!(target.join("repo-b").exists());

    // Verify manifest has both repos
    let raw = std::fs::read_to_string(target.join(".rig.json")).unwrap();
    assert!(raw.contains("repo-a"));
    assert!(raw.contains("repo-b"));

    // Verify branches are rig/<target-name>, not rig/<source-name>
    assert!(raw.contains(r#""branch": "rig/target-ws""#));
    assert!(!raw.contains(r#""branch": "rig/source-ws""#));
}

#[test]
fn create_from_inherits_upstream() {
    let sandbox = common::TestSandbox::new();
    let ws_dir = sandbox.create_workspace_with_repos("source-ws", &["repo-a"]);
    let repo_dir = sandbox.path().join("repo-a");

    // Create an upstream branch on the remote
    common::git(&repo_dir, &["checkout", "-b", "integration"]);
    sandbox.commit_file("repo-a", "integration.txt", "new", "integration commit");
    common::git(&repo_dir, &["push", "-u", "origin", "integration"]);
    common::git(&repo_dir, &["checkout", "main"]);

    // Set upstream on source rig
    Command::cargo_bin("git-rig")
        .unwrap()
        .args(["add", "--upstream", "integration"])
        .arg(repo_dir.to_str().unwrap())
        .current_dir(&ws_dir)
        .assert()
        .success();

    // Clone the rig
    Command::cargo_bin("git-rig")
        .unwrap()
        .args(["create", "target-ws", "--from", "source-ws"])
        .current_dir(sandbox.path())
        .assert()
        .success();

    // Verify upstream is inherited
    let target = sandbox.path().join("target-ws");
    let raw = std::fs::read_to_string(target.join(".rig.json")).unwrap();
    assert!(raw.contains(r#""upstream": "integration""#));

    // Verify the worktree starts from the upstream branch's content
    assert!(target.join("repo-a").join("integration.txt").exists());
}

#[test]
fn create_from_detached_repos_stay_detached() {
    let sandbox = common::TestSandbox::new();
    let repo_path = sandbox.create_repo("repo-a");
    let ws_dir = sandbox.create_workspace("source-ws");

    // Add as detached
    Command::cargo_bin("git-rig")
        .unwrap()
        .args(["add", "--detach"])
        .arg(repo_path.to_str().unwrap())
        .current_dir(&ws_dir)
        .assert()
        .success();

    // Clone the rig
    Command::cargo_bin("git-rig")
        .unwrap()
        .args(["create", "target-ws", "--from", "source-ws"])
        .current_dir(sandbox.path())
        .assert()
        .success();

    // Verify repo is detached in target
    let target = sandbox.path().join("target-ws");
    let raw = std::fs::read_to_string(target.join(".rig.json")).unwrap();
    assert!(raw.contains(r#""branch": "(detached)""#));
}

#[test]
fn create_from_source_not_found() {
    let sandbox = common::TestSandbox::new();

    Command::cargo_bin("git-rig")
        .unwrap()
        .args(["create", "target-ws", "--from", "nonexistent"])
        .current_dir(sandbox.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("not found"));
}

#[test]
fn create_from_target_already_exists() {
    let sandbox = common::TestSandbox::new();
    sandbox.create_workspace_with_repos("source-ws", &["repo-a"]);
    sandbox.create_workspace("target-ws");

    Command::cargo_bin("git-rig")
        .unwrap()
        .args(["create", "target-ws", "--from", "source-ws"])
        .current_dir(sandbox.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("already exists"));
}

#[test]
fn create_from_invalid_source_repo_fails() {
    let sandbox = common::TestSandbox::new();
    let ws_dir = sandbox.create_workspace("source-ws");

    // Manually write a manifest entry pointing to a nonexistent path
    let manifest_path = ws_dir.join(".rig.json");
    std::fs::write(
        &manifest_path,
        serde_json::to_string_pretty(&serde_json::json!({
            "name": "source-ws",
            "repos": [{
                "name": "gone-repo",
                "source": "/nonexistent/path/gone-repo",
                "branch": "rig/source-ws",
                "default_branch": "main",
                "remote": "origin"
            }]
        }))
        .unwrap(),
    )
    .unwrap();

    Command::cargo_bin("git-rig")
        .unwrap()
        .args(["create", "target-ws", "--from", "source-ws"])
        .current_dir(sandbox.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("source repos invalid"))
        .stderr(predicate::str::contains("gone-repo"));

    // Target should not have been created
    assert!(!sandbox.path().join("target-ws").exists());
}

#[test]
fn create_from_skip_invalid_repos() {
    let sandbox = common::TestSandbox::new();
    let ws_dir = sandbox.create_workspace_with_repos("source-ws", &["repo-a"]);

    // Add a bad entry to the manifest
    let manifest_path = ws_dir.join(".rig.json");
    let raw = std::fs::read_to_string(&manifest_path).unwrap();
    let mut json: serde_json::Value = serde_json::from_str(&raw).unwrap();
    json["repos"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({
            "name": "gone-repo",
            "source": "/nonexistent/path/gone-repo",
            "branch": "rig/source-ws",
            "default_branch": "main",
            "remote": "origin"
        }));
    std::fs::write(&manifest_path, serde_json::to_string_pretty(&json).unwrap()).unwrap();

    Command::cargo_bin("git-rig")
        .unwrap()
        .args(["create", "target-ws", "--from", "source-ws", "--skip"])
        .current_dir(sandbox.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("WARN"))
        .stdout(predicate::str::contains("Skipping"))
        .stdout(predicate::str::contains("Created rig 'target-ws'"));

    // Valid repo should be in the target
    let target = sandbox.path().join("target-ws");
    assert!(target.join("repo-a").exists());

    // Invalid repo should not be in the target manifest
    let raw = std::fs::read_to_string(target.join(".rig.json")).unwrap();
    assert!(raw.contains("repo-a"));
    assert!(!raw.contains("gone-repo"));
}

#[test]
fn create_from_empty_source() {
    let sandbox = common::TestSandbox::new();
    sandbox.create_workspace("source-ws");

    Command::cargo_bin("git-rig")
        .unwrap()
        .args(["create", "target-ws", "--from", "source-ws"])
        .current_dir(sandbox.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("0 repos"));

    // Target should exist with empty manifest
    assert!(sandbox.path().join("target-ws").join(".rig.json").exists());
}

#[test]
fn create_from_status_works_on_cloned_rig() {
    let sandbox = common::TestSandbox::new();
    sandbox.create_workspace_with_repos("source-ws", &["repo-a"]);

    Command::cargo_bin("git-rig")
        .unwrap()
        .args(["create", "target-ws", "--from", "source-ws"])
        .current_dir(sandbox.path())
        .assert()
        .success();

    // Status should work on the cloned rig
    let target = sandbox.path().join("target-ws");
    Command::cargo_bin("git-rig")
        .unwrap()
        .arg("status")
        .current_dir(&target)
        .assert()
        .success()
        .stdout(predicate::str::contains("repo-a"))
        .stdout(predicate::str::contains("rig/target-ws"));
}

#[test]
fn create_from_inherits_custom_remote() {
    let sandbox = common::TestSandbox::new();
    let ws_dir = sandbox.create_workspace("source-ws");
    let repo_path = sandbox.create_repo("repo-a");

    // Add with custom remote name - first rename origin to upstream
    common::git(&repo_path, &["remote", "rename", "origin", "upstream"]);
    // Re-set remote HEAD after rename
    common::git(&repo_path, &["remote", "set-head", "upstream", "--auto"]);

    Command::cargo_bin("git-rig")
        .unwrap()
        .args(["add", "--remote", "upstream"])
        .arg(repo_path.to_str().unwrap())
        .current_dir(&ws_dir)
        .assert()
        .success();

    // Clone the rig
    Command::cargo_bin("git-rig")
        .unwrap()
        .args(["create", "target-ws", "--from", "source-ws"])
        .current_dir(sandbox.path())
        .assert()
        .success();

    // Verify remote is inherited
    let target = sandbox.path().join("target-ws");
    let raw = std::fs::read_to_string(target.join(".rig.json")).unwrap();
    assert!(raw.contains(r#""remote": "upstream""#));
}

#[test]
fn create_without_from_unchanged() {
    let sandbox = common::TestSandbox::new();

    Command::cargo_bin("git-rig")
        .unwrap()
        .args(["create", "my-ws"])
        .current_dir(sandbox.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("ok"))
        .stdout(predicate::str::contains("Add repos with"));

    assert!(sandbox.path().join("my-ws").join(".rig.json").exists());
}

#[test]
fn create_from_skip_requires_from() {
    let sandbox = common::TestSandbox::new();

    Command::cargo_bin("git-rig")
        .unwrap()
        .args(["create", "my-ws", "--skip"])
        .current_dir(sandbox.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("--from"));
}

#[test]
fn create_from_skip_all_invalid_fails() {
    let sandbox = common::TestSandbox::new();
    let ws_dir = sandbox.create_workspace("source-ws");

    // Manifest with only invalid entries
    std::fs::write(
        ws_dir.join(".rig.json"),
        serde_json::to_string_pretty(&serde_json::json!({
            "name": "source-ws",
            "repos": [
                {
                    "name": "gone-a",
                    "source": "/nonexistent/a",
                    "branch": "rig/source-ws",
                    "default_branch": "main",
                    "remote": "origin"
                },
                {
                    "name": "gone-b",
                    "source": "/nonexistent/b",
                    "branch": "rig/source-ws",
                    "default_branch": "main",
                    "remote": "origin"
                }
            ]
        }))
        .unwrap(),
    )
    .unwrap();

    Command::cargo_bin("git-rig")
        .unwrap()
        .args(["create", "target-ws", "--from", "source-ws", "--skip"])
        .current_dir(sandbox.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("no valid repos"));

    // Target should not have been created
    assert!(!sandbox.path().join("target-ws").exists());
}

#[test]
fn create_from_invalid_source_not_a_git_repo() {
    let sandbox = common::TestSandbox::new();
    let ws_dir = sandbox.create_workspace("source-ws");

    // Create a real directory that is NOT a git repo
    let not_git = sandbox.path().join("not-a-repo");
    std::fs::create_dir_all(&not_git).unwrap();

    std::fs::write(
        ws_dir.join(".rig.json"),
        serde_json::to_string_pretty(&serde_json::json!({
            "name": "source-ws",
            "repos": [{
                "name": "not-a-repo",
                "source": not_git.to_str().unwrap(),
                "branch": "rig/source-ws",
                "default_branch": "main",
                "remote": "origin"
            }]
        }))
        .unwrap(),
    )
    .unwrap();

    Command::cargo_bin("git-rig")
        .unwrap()
        .args(["create", "target-ws", "--from", "source-ws"])
        .current_dir(sandbox.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("not a git repository"));

    assert!(!sandbox.path().join("target-ws").exists());
}

#[test]
fn create_from_partial_runtime_failure() {
    let sandbox = common::TestSandbox::new();
    let _ws_dir = sandbox.create_workspace_with_repos("source-ws", &["repo-a", "repo-b"]);

    // Check out rig/target-ws in repo-b's source clone so that
    // worktree creation will fail with "already checked out"
    let repo_b_dir = sandbox.path().join("repo-b");
    common::git(&repo_b_dir, &["checkout", "-b", "rig/target-ws"]);

    Command::cargo_bin("git-rig")
        .unwrap()
        .args(["create", "target-ws", "--from", "source-ws"])
        .current_dir(sandbox.path())
        .assert()
        .failure()
        .stdout(predicate::str::contains("ok repo-a"))
        .stdout(predicate::str::contains("ERR"))
        .stdout(predicate::str::contains("1 repos added, 1 failed"));

    // repo-a should exist in target, repo-b should not
    let target = sandbox.path().join("target-ws");
    assert!(target.join("repo-a").exists());

    // Manifest should contain repo-a but not repo-b
    let raw = std::fs::read_to_string(target.join(".rig.json")).unwrap();
    assert!(raw.contains("repo-a"));
    assert!(!raw.contains("repo-b"));
}

// ---------------------------------------------------------------------------
// provision (.riginclude)
// ---------------------------------------------------------------------------

#[test]
fn add_provisions_local_files() {
    let sandbox = common::TestSandbox::new();
    let repo_path = sandbox.create_repo("repo-a");
    let ws_dir = sandbox.create_workspace("my-ws");

    sandbox.create_riginclude("repo-a", &[".env", ".env.local"]);
    sandbox.create_local_file("repo-a", ".env", "SECRET=abc");
    sandbox.create_local_file("repo-a", ".env.local", "LOCAL=xyz");

    Command::cargo_bin("git-rig")
        .unwrap()
        .arg("add")
        .arg(repo_path.to_str().unwrap())
        .current_dir(&ws_dir)
        .assert()
        .success()
        .stdout(predicate::str::contains("provisioned"));

    let wt = ws_dir.join("repo-a");
    assert_eq!(
        std::fs::read_to_string(wt.join(".env")).unwrap(),
        "SECRET=abc"
    );
    assert_eq!(
        std::fs::read_to_string(wt.join(".env.local")).unwrap(),
        "LOCAL=xyz"
    );
    // .riginclude itself should be copied (self-propagating)
    assert!(wt.join(".riginclude").exists());
}

#[test]
fn add_provisions_directory_recursively() {
    let sandbox = common::TestSandbox::new();
    let repo_path = sandbox.create_repo("repo-a");
    let ws_dir = sandbox.create_workspace("my-ws");

    sandbox.create_riginclude("repo-a", &[".vscode/"]);
    sandbox.create_local_file(
        "repo-a",
        ".vscode/settings.json",
        r#"{"editor.tabSize": 2}"#,
    );
    sandbox.create_local_file("repo-a", ".vscode/launch.json", r#"{"version": "0.2.0"}"#);

    Command::cargo_bin("git-rig")
        .unwrap()
        .arg("add")
        .arg(repo_path.to_str().unwrap())
        .current_dir(&ws_dir)
        .assert()
        .success()
        .stdout(predicate::str::contains("provisioned"));

    let wt = ws_dir.join("repo-a");
    assert_eq!(
        std::fs::read_to_string(wt.join(".vscode/settings.json")).unwrap(),
        r#"{"editor.tabSize": 2}"#
    );
    assert_eq!(
        std::fs::read_to_string(wt.join(".vscode/launch.json")).unwrap(),
        r#"{"version": "0.2.0"}"#
    );
}

#[test]
fn add_no_riginclude_no_provision_output() {
    let sandbox = common::TestSandbox::new();
    let repo_path = sandbox.create_repo("repo-a");
    let ws_dir = sandbox.create_workspace("my-ws");

    Command::cargo_bin("git-rig")
        .unwrap()
        .arg("add")
        .arg(repo_path.to_str().unwrap())
        .current_dir(&ws_dir)
        .assert()
        .success()
        .stdout(predicate::str::contains("provisioned").not());
}

#[test]
fn add_no_provision_flag_skips() {
    let sandbox = common::TestSandbox::new();
    let repo_path = sandbox.create_repo("repo-a");
    let ws_dir = sandbox.create_workspace("my-ws");

    sandbox.create_riginclude("repo-a", &[".env"]);
    sandbox.create_local_file("repo-a", ".env", "SECRET=abc");

    Command::cargo_bin("git-rig")
        .unwrap()
        .args(["add", "--no-provision"])
        .arg(repo_path.to_str().unwrap())
        .current_dir(&ws_dir)
        .assert()
        .success()
        .stdout(predicate::str::contains("provisioned").not());

    assert!(!ws_dir.join("repo-a").join(".env").exists());
    assert!(!ws_dir.join("repo-a").join(".riginclude").exists());
}

#[test]
fn add_skips_existing_files() {
    let sandbox = common::TestSandbox::new();
    let repo_path = sandbox.create_repo("repo-a");
    let ws_dir = sandbox.create_workspace("my-ws");

    sandbox.create_riginclude("repo-a", &[".env"]);
    sandbox.create_local_file("repo-a", ".env", "FROM_SOURCE");

    // Pre-create the worktree directory with a file already in it.
    // This simulates the worktree recovery path (interrupted add) where
    // the directory exists but isn't in the manifest.
    let wt_dir = ws_dir.join("repo-a");
    let wt_str = wt_dir.to_str().unwrap();
    common::git(
        &repo_path,
        &["worktree", "add", "-b", "rig/my-ws", wt_str, "origin/main"],
    );
    std::fs::write(wt_dir.join(".env"), "PRE_EXISTING").unwrap();

    // add should recover the existing worktree and skip the pre-existing .env
    Command::cargo_bin("git-rig")
        .unwrap()
        .arg("add")
        .arg(repo_path.to_str().unwrap())
        .current_dir(&ws_dir)
        .assert()
        .success()
        .stdout(predicate::str::contains("skipped"));

    // .env should NOT be overwritten
    assert_eq!(
        std::fs::read_to_string(wt_dir.join(".env")).unwrap(),
        "PRE_EXISTING"
    );
}

#[test]
fn add_force_overwrites_existing() {
    let sandbox = common::TestSandbox::new();
    let repo_path = sandbox.create_repo("repo-a");
    let ws_dir = sandbox.create_workspace("my-ws");

    sandbox.create_riginclude("repo-a", &[".env"]);
    sandbox.create_local_file("repo-a", ".env", "FROM_SOURCE");

    Command::cargo_bin("git-rig")
        .unwrap()
        .arg("add")
        .arg(repo_path.to_str().unwrap())
        .current_dir(&ws_dir)
        .assert()
        .success();

    std::fs::write(ws_dir.join("repo-a").join(".env"), "MODIFIED").unwrap();
    sandbox.create_local_file("repo-a", ".env", "UPDATED_SOURCE");

    Command::cargo_bin("git-rig")
        .unwrap()
        .args(["remove", "--force", "--keep-branch", "repo-a"])
        .current_dir(&ws_dir)
        .assert()
        .success();

    Command::cargo_bin("git-rig")
        .unwrap()
        .args(["add", "--force-provision"])
        .arg(repo_path.to_str().unwrap())
        .current_dir(&ws_dir)
        .assert()
        .success();

    assert_eq!(
        std::fs::read_to_string(ws_dir.join("repo-a").join(".env")).unwrap(),
        "UPDATED_SOURCE"
    );
}

#[test]
fn add_link_creates_symlinks() {
    let sandbox = common::TestSandbox::new();
    let repo_path = sandbox.create_repo("repo-a");
    let ws_dir = sandbox.create_workspace("my-ws");

    sandbox.create_riginclude("repo-a", &[".env"]);
    sandbox.create_local_file("repo-a", ".env", "SECRET=abc");

    Command::cargo_bin("git-rig")
        .unwrap()
        .args(["add", "--link"])
        .arg(repo_path.to_str().unwrap())
        .current_dir(&ws_dir)
        .assert()
        .success()
        .stdout(predicate::str::contains("linked"));

    let link = ws_dir.join("repo-a").join(".env");
    assert!(link.symlink_metadata().unwrap().file_type().is_symlink());
    assert_eq!(std::fs::read_to_string(&link).unwrap(), "SECRET=abc");
}

#[test]
fn add_riginclude_self_propagates() {
    let sandbox = common::TestSandbox::new();
    let repo_path = sandbox.create_repo("repo-a");
    let ws_dir = sandbox.create_workspace("my-ws");

    sandbox.create_riginclude("repo-a", &[".env"]);
    sandbox.create_local_file("repo-a", ".env", "SECRET=abc");

    Command::cargo_bin("git-rig")
        .unwrap()
        .arg("add")
        .arg(repo_path.to_str().unwrap())
        .current_dir(&ws_dir)
        .assert()
        .success();

    let wt = ws_dir.join("repo-a");
    assert!(wt.join(".riginclude").exists());
    assert_eq!(
        std::fs::read_to_string(wt.join(".riginclude")).unwrap(),
        ".env\n"
    );
}

#[test]
fn create_from_provisions_from_source_rig() {
    let sandbox = common::TestSandbox::new();
    let source_ws = sandbox.create_workspace_with_repos("source-ws", &["repo-a"]);

    let source_wt = source_ws.join("repo-a");
    std::fs::write(source_wt.join(".riginclude"), ".env\n").unwrap();
    std::fs::write(source_wt.join(".env"), "FROM_RIG").unwrap();

    Command::cargo_bin("git-rig")
        .unwrap()
        .args(["create", "target-ws", "--from", "source-ws"])
        .current_dir(sandbox.path())
        .assert()
        .success();

    let target_wt = sandbox.path().join("target-ws").join("repo-a");
    assert_eq!(
        std::fs::read_to_string(target_wt.join(".env")).unwrap(),
        "FROM_RIG"
    );
    assert!(target_wt.join(".riginclude").exists());
}

#[test]
fn create_from_no_provision() {
    let sandbox = common::TestSandbox::new();
    let source_ws = sandbox.create_workspace_with_repos("source-ws", &["repo-a"]);

    let source_wt = source_ws.join("repo-a");
    std::fs::write(source_wt.join(".riginclude"), ".env\n").unwrap();
    std::fs::write(source_wt.join(".env"), "FROM_RIG").unwrap();

    Command::cargo_bin("git-rig")
        .unwrap()
        .args([
            "create",
            "target-ws",
            "--from",
            "source-ws",
            "--no-provision",
        ])
        .current_dir(sandbox.path())
        .assert()
        .success();

    let target_wt = sandbox.path().join("target-ws").join("repo-a");
    assert!(!target_wt.join(".env").exists());
    assert!(!target_wt.join(".riginclude").exists());
}

#[test]
fn add_glob_patterns_work() {
    let sandbox = common::TestSandbox::new();
    let repo_path = sandbox.create_repo("repo-a");
    let ws_dir = sandbox.create_workspace("my-ws");

    sandbox.create_riginclude("repo-a", &[".env*"]);
    sandbox.create_local_file("repo-a", ".env", "BASE");
    sandbox.create_local_file("repo-a", ".env.local", "LOCAL");
    sandbox.create_local_file("repo-a", ".env.production", "PROD");

    Command::cargo_bin("git-rig")
        .unwrap()
        .arg("add")
        .arg(repo_path.to_str().unwrap())
        .current_dir(&ws_dir)
        .assert()
        .success();

    let wt = ws_dir.join("repo-a");
    assert_eq!(std::fs::read_to_string(wt.join(".env")).unwrap(), "BASE");
    assert_eq!(
        std::fs::read_to_string(wt.join(".env.local")).unwrap(),
        "LOCAL"
    );
    assert_eq!(
        std::fs::read_to_string(wt.join(".env.production")).unwrap(),
        "PROD"
    );
}

#[test]
fn add_negation_patterns_exclude_files() {
    let sandbox = common::TestSandbox::new();
    let repo_path = sandbox.create_repo("repo-a");
    let ws_dir = sandbox.create_workspace("my-ws");

    sandbox.create_riginclude("repo-a", &[".env*", "!.env.production"]);
    sandbox.create_local_file("repo-a", ".env", "BASE");
    sandbox.create_local_file("repo-a", ".env.local", "LOCAL");
    sandbox.create_local_file("repo-a", ".env.production", "PROD");

    Command::cargo_bin("git-rig")
        .unwrap()
        .arg("add")
        .arg(repo_path.to_str().unwrap())
        .current_dir(&ws_dir)
        .assert()
        .success();

    let wt = ws_dir.join("repo-a");
    assert!(wt.join(".env").exists());
    assert!(wt.join(".env.local").exists());
    assert!(!wt.join(".env.production").exists());
}

#[test]
fn add_provision_failure_is_warning_not_fatal() {
    let sandbox = common::TestSandbox::new();
    let repo_path = sandbox.create_repo("repo-a");
    let ws_dir = sandbox.create_workspace("my-ws");

    // Create .riginclude referencing a file that doesn't exist
    // plus one that does — the missing file should not fail the command
    sandbox.create_riginclude("repo-a", &[".env", "nonexistent-dir/"]);
    sandbox.create_local_file("repo-a", ".env", "SECRET=abc");

    Command::cargo_bin("git-rig")
        .unwrap()
        .arg("add")
        .arg(repo_path.to_str().unwrap())
        .current_dir(&ws_dir)
        .assert()
        .success()
        .stdout(predicate::str::contains("provisioned"));

    // .env should still be provisioned despite the nonexistent pattern
    let wt = ws_dir.join("repo-a");
    assert_eq!(
        std::fs::read_to_string(wt.join(".env")).unwrap(),
        "SECRET=abc"
    );
    // Repo should be in the manifest
    let raw = std::fs::read_to_string(ws_dir.join(".rig.json")).unwrap();
    assert!(raw.contains("repo-a"));
}

#[test]
fn add_provision_unreadable_source_still_succeeds() {
    let sandbox = common::TestSandbox::new();
    let repo_path = sandbox.create_repo("repo-a");
    let ws_dir = sandbox.create_workspace("my-ws");

    sandbox.create_riginclude("repo-a", &[".env", ".secret"]);
    sandbox.create_local_file("repo-a", ".env", "OK");
    sandbox.create_local_file("repo-a", ".secret", "HIDDEN");

    // Make .secret unreadable
    let secret_path = sandbox.path().join("repo-a").join(".secret");
    std::fs::set_permissions(&secret_path, std::fs::Permissions::from_mode(0o000)).unwrap();

    Command::cargo_bin("git-rig")
        .unwrap()
        .arg("add")
        .arg(repo_path.to_str().unwrap())
        .current_dir(&ws_dir)
        .assert()
        .success()
        .stdout(predicate::str::contains("warning"));

    // .env should still be provisioned
    assert_eq!(
        std::fs::read_to_string(ws_dir.join("repo-a").join(".env")).unwrap(),
        "OK"
    );
    // Repo should be in the manifest despite provisioning warning
    let raw = std::fs::read_to_string(ws_dir.join(".rig.json")).unwrap();
    assert!(raw.contains("repo-a"));

    // Restore permissions for cleanup
    std::fs::set_permissions(&secret_path, std::fs::Permissions::from_mode(0o644)).unwrap();
}

#[test]
fn create_from_link_flag_propagates() {
    let sandbox = common::TestSandbox::new();
    let source_ws = sandbox.create_workspace_with_repos("source-ws", &["repo-a"]);

    let source_wt = source_ws.join("repo-a");
    std::fs::write(source_wt.join(".riginclude"), ".env\n").unwrap();
    std::fs::write(source_wt.join(".env"), "FROM_RIG").unwrap();

    Command::cargo_bin("git-rig")
        .unwrap()
        .args(["create", "target-ws", "--from", "source-ws", "--link"])
        .current_dir(sandbox.path())
        .assert()
        .success();

    let target_env = sandbox.path().join("target-ws").join("repo-a").join(".env");
    assert!(
        target_env
            .symlink_metadata()
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(std::fs::read_to_string(&target_env).unwrap(), "FROM_RIG");
}

#[test]
fn create_provision_flags_require_from() {
    let sandbox = common::TestSandbox::new();

    // --no-provision without --from should fail
    Command::cargo_bin("git-rig")
        .unwrap()
        .args(["create", "my-ws", "--no-provision"])
        .current_dir(sandbox.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("--from"));

    // --link without --from should fail
    Command::cargo_bin("git-rig")
        .unwrap()
        .args(["create", "my-ws", "--link"])
        .current_dir(sandbox.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("--from"));

    // --force-provision without --from should fail
    Command::cargo_bin("git-rig")
        .unwrap()
        .args(["create", "my-ws", "--force-provision"])
        .current_dir(sandbox.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("--from"));
}

// ---------------------------------------------------------------------------
// drift detection
// ---------------------------------------------------------------------------

/// Helper: switch a worktree to a different branch to induce branch mismatch.
fn checkout_branch_in_worktree(worktree_path: &std::path::Path, branch: &str) {
    let output = std::process::Command::new("git")
        .args(["checkout", "-b", branch])
        .current_dir(worktree_path)
        .output()
        .expect("git checkout in worktree");
    assert!(
        output.status.success(),
        "git checkout -b {branch} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Helper: detach HEAD in a worktree.
fn detach_head_in_worktree(worktree_path: &std::path::Path) {
    let output = std::process::Command::new("git")
        .args(["checkout", "--detach"])
        .current_dir(worktree_path)
        .output()
        .expect("git checkout --detach in worktree");
    assert!(
        output.status.success(),
        "git checkout --detach failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Helper: get the current HEAD SHA in a worktree.
fn head_sha(worktree_path: &std::path::Path) -> String {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(worktree_path)
        .output()
        .expect("git rev-parse HEAD");
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

#[test]
fn drift_status_shows_branch_mismatch() {
    let sandbox = common::TestSandbox::new();
    let ws_dir = sandbox.create_workspace_with_repos("my-ws", &["repo-a"]);

    // Induce branch mismatch: manifest says rig/my-ws, switch to "other-branch"
    checkout_branch_in_worktree(&ws_dir.join("repo-a"), "other-branch");

    Command::cargo_bin("git-rig")
        .unwrap()
        .arg("status")
        .current_dir(&ws_dir)
        .assert()
        .success()
        .stdout(predicate::str::contains("DRIFT"))
        .stdout(predicate::str::contains("other-branch"))
        .stdout(predicate::str::contains("rig/my-ws"));
}

#[test]
fn drift_sync_skips_branch_drifted_repo() {
    let sandbox = common::TestSandbox::new();
    let ws_dir = sandbox.create_workspace_with_repos("my-ws", &["repo-a", "repo-b"]);

    // Push a new commit to repo-a's remote so sync has something to rebase
    sandbox.commit_file("repo-a", "new.txt", "content", "upstream change");
    std::process::Command::new("git")
        .args(["push"])
        .current_dir(sandbox.path().join("repo-a"))
        .output()
        .unwrap();

    // Drift repo-a by switching to a different branch
    checkout_branch_in_worktree(&ws_dir.join("repo-a"), "wrong-branch");

    // Record HEAD before sync to verify the drifted repo is not rebased
    let head_before = head_sha(&ws_dir.join("repo-a"));

    let output = Command::cargo_bin("git-rig")
        .unwrap()
        .arg("sync")
        .current_dir(&ws_dir)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8_lossy(&output);

    // Drifted repo-a should appear in DRIFT warning, not be synced
    assert!(stdout.contains("DRIFT"));
    assert!(stdout.contains("repo-a"));
    // repo-b should still sync normally
    assert!(stdout.contains("ok"));

    // Verify drifted repo-a was NOT rebased (HEAD unchanged)
    let head_after = head_sha(&ws_dir.join("repo-a"));
    assert_eq!(
        head_before, head_after,
        "drifted repo should not be rebased"
    );
}

#[test]
fn drift_sync_skips_missing_source() {
    let sandbox = common::TestSandbox::new();
    let ws_dir = sandbox.create_workspace_with_repos("my-ws", &["repo-a"]);

    // Remove the source repo directory
    let source_dir = sandbox.path().join("repo-a");
    std::fs::remove_dir_all(&source_dir).unwrap();

    Command::cargo_bin("git-rig")
        .unwrap()
        .arg("sync")
        .current_dir(&ws_dir)
        .assert()
        .success()
        .stdout(predicate::str::contains("DRIFT"))
        .stdout(predicate::str::contains("source repo missing"))
        .stdout(predicate::str::contains("ERR").not());
}

#[test]
fn drift_sync_skips_unexpected_detached() {
    let sandbox = common::TestSandbox::new();
    let ws_dir = sandbox.create_workspace_with_repos("my-ws", &["repo-a"]);

    // Detach HEAD — manifest says rig/my-ws but worktree is now detached
    detach_head_in_worktree(&ws_dir.join("repo-a"));

    Command::cargo_bin("git-rig")
        .unwrap()
        .arg("sync")
        .current_dir(&ws_dir)
        .assert()
        .success()
        .stdout(predicate::str::contains("DRIFT"))
        .stdout(predicate::str::contains("detached HEAD"));
}

#[test]
fn drift_exec_warns_but_runs_on_branch_mismatch() {
    let sandbox = common::TestSandbox::new();
    let ws_dir = sandbox.create_workspace_with_repos("my-ws", &["repo-a"]);

    // Induce branch mismatch
    checkout_branch_in_worktree(&ws_dir.join("repo-a"), "other-branch");

    Command::cargo_bin("git-rig")
        .unwrap()
        .args(["exec", "--", "echo", "hello"])
        .current_dir(&ws_dir)
        .assert()
        .success()
        // Should see both the DRIFT warning and the command output
        .stdout(predicate::str::contains("DRIFT"))
        .stdout(predicate::str::contains("hello"));
}

#[test]
fn drift_exec_skips_missing_worktree() {
    let sandbox = common::TestSandbox::new();
    let ws_dir = sandbox.create_workspace_with_repos("my-ws", &["repo-a"]);

    // Remove the worktree directory
    std::fs::remove_dir_all(ws_dir.join("repo-a")).unwrap();

    Command::cargo_bin("git-rig")
        .unwrap()
        .args(["exec", "--", "echo", "hello"])
        .current_dir(&ws_dir)
        .assert()
        .success()
        .stdout(predicate::str::contains("DRIFT"))
        .stdout(predicate::str::contains("worktree missing"))
        .stdout(predicate::str::contains("worktree unavailable, skipped"))
        .stdout(predicate::str::contains("hello").not());
}

#[test]
fn drift_exec_skips_unreachable_worktree() {
    let sandbox = common::TestSandbox::new();
    let ws_dir = sandbox.create_workspace_with_repos("my-ws", &["repo-a"]);

    // Corrupt worktree metadata — directory still exists but git commands fail
    sandbox.corrupt_worktree_metadata("repo-a");

    Command::cargo_bin("git-rig")
        .unwrap()
        .args(["exec", "--", "echo", "hello"])
        .current_dir(&ws_dir)
        .assert()
        .success()
        .stdout(predicate::str::contains("DRIFT"))
        .stdout(predicate::str::contains("worktree unreachable"))
        .stdout(predicate::str::contains("worktree unavailable, skipped"))
        .stdout(predicate::str::contains("hello").not());
}

#[test]
fn drift_no_output_when_clean() {
    let sandbox = common::TestSandbox::new();
    let ws_dir = sandbox.create_workspace_with_repos("my-ws", &["repo-a"]);

    Command::cargo_bin("git-rig")
        .unwrap()
        .arg("status")
        .current_dir(&ws_dir)
        .assert()
        .success()
        .stdout(predicate::str::contains("DRIFT").not());
}

#[test]
fn drift_exec_repo_filter_scopes_warnings() {
    let sandbox = common::TestSandbox::new();
    let ws_dir = sandbox.create_workspace_with_repos("my-ws", &["repo-a", "repo-b"]);

    // Drift repo-b, but only exec on repo-a
    checkout_branch_in_worktree(&ws_dir.join("repo-b"), "wrong-branch");

    Command::cargo_bin("git-rig")
        .unwrap()
        .args(["exec", "--repo", "repo-a", "--", "echo", "hello"])
        .current_dir(&ws_dir)
        .assert()
        .success()
        // Should NOT see DRIFT warning because repo-b is not in the filter
        .stdout(predicate::str::contains("DRIFT").not())
        .stdout(predicate::str::contains("hello"));
}

#[test]
fn drift_refresh_skips_missing_source() {
    let sandbox = common::TestSandbox::new();
    let ws_dir = sandbox.create_workspace_with_repos("my-ws", &["repo-a", "repo-b"]);

    // Remove repo-a's source clone
    let source_dir = sandbox.path().join("repo-a");
    std::fs::remove_dir_all(&source_dir).unwrap();

    let output = Command::cargo_bin("git-rig")
        .unwrap()
        .arg("refresh")
        .current_dir(&ws_dir)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8_lossy(&output);

    // Should show drift warning for repo-a
    assert!(stdout.contains("DRIFT"));
    assert!(stdout.contains("repo-a"));
    // repo-b should still refresh normally
    assert!(stdout.contains("repo-b"));
}

#[test]
fn drift_expected_detached_no_warning() {
    let sandbox = common::TestSandbox::new();
    let ws_dir = sandbox.create_workspace("my-ws");
    let repo_dir = sandbox.create_repo("repo-a");

    // Add repo as detached
    Command::cargo_bin("git-rig")
        .unwrap()
        .args(["add", &repo_dir.to_string_lossy(), "--detach"])
        .current_dir(&ws_dir)
        .assert()
        .success();

    // Status should have no DRIFT warning (detached is expected)
    Command::cargo_bin("git-rig")
        .unwrap()
        .arg("status")
        .current_dir(&ws_dir)
        .assert()
        .success()
        .stdout(predicate::str::contains("DRIFT").not());
}

#[test]
fn drift_worktree_unreachable_handled_gracefully() {
    let sandbox = common::TestSandbox::new();
    let ws_dir = sandbox.create_workspace_with_repos("my-ws", &["repo-a"]);

    // Corrupt the worktree metadata so git commands fail inside it
    sandbox.corrupt_worktree_metadata("repo-a");

    // Status should show specific drift kind and treat repo as missing
    Command::cargo_bin("git-rig")
        .unwrap()
        .arg("status")
        .current_dir(&ws_dir)
        .assert()
        .success()
        .stdout(predicate::str::contains("DRIFT"))
        .stdout(predicate::str::contains("worktree unreachable"))
        .stdout(predicate::str::contains("(missing)"));
}

#[test]
fn drift_dual_missing_worktree_and_source() {
    let sandbox = common::TestSandbox::new();
    let ws_dir = sandbox.create_workspace_with_repos("my-ws", &["repo-a"]);

    // Remove both the worktree directory and the source repo
    std::fs::remove_dir_all(ws_dir.join("repo-a")).unwrap();
    std::fs::remove_dir_all(sandbox.path().join("repo-a")).unwrap();

    // Status should show both drift types without crashing
    Command::cargo_bin("git-rig")
        .unwrap()
        .arg("status")
        .current_dir(&ws_dir)
        .assert()
        .success()
        .stdout(predicate::str::contains("DRIFT"))
        .stdout(predicate::str::contains("worktree missing"))
        .stdout(predicate::str::contains("source repo missing"));
}

#[test]
fn drift_refresh_only_shows_source_drift() {
    let sandbox = common::TestSandbox::new();
    let ws_dir = sandbox.create_workspace_with_repos("my-ws", &["repo-a", "repo-b"]);

    // Branch-drift repo-a (worktree drift, not source drift)
    checkout_branch_in_worktree(&ws_dir.join("repo-a"), "other-branch");

    // Refresh should NOT show the branch drift (SourceOnly scope)
    // but repo-a should still be refreshed (branch drift doesn't prevent refresh)
    Command::cargo_bin("git-rig")
        .unwrap()
        .arg("refresh")
        .current_dir(&ws_dir)
        .assert()
        .success()
        .stdout(predicate::str::contains("DRIFT").not())
        .stdout(predicate::str::contains("repo-a"));
}

#[test]
fn drift_detached_to_named_branch_detected() {
    let sandbox = common::TestSandbox::new();
    let ws_dir = sandbox.create_workspace("my-ws");
    let repo_dir = sandbox.create_repo("repo-a");

    // Add repo as detached
    Command::cargo_bin("git-rig")
        .unwrap()
        .args(["add", &repo_dir.to_string_lossy(), "--detach"])
        .current_dir(&ws_dir)
        .assert()
        .success();

    // Manually check out a named branch in what should be a detached worktree
    checkout_branch_in_worktree(&ws_dir.join("repo-a"), "sneaky-branch");

    // Status should detect drift: manifest says (detached), worktree is on sneaky-branch
    Command::cargo_bin("git-rig")
        .unwrap()
        .arg("status")
        .current_dir(&ws_dir)
        .assert()
        .success()
        .stdout(predicate::str::contains("DRIFT"))
        .stdout(predicate::str::contains("sneaky-branch"));
}

// ---------------------------------------------------------------------------
// sync --repo filtering
// ---------------------------------------------------------------------------

#[test]
fn sync_repo_filter() {
    let sandbox = common::TestSandbox::new();
    let ws_dir = sandbox.create_workspace_with_repos("my-ws", &["repo-a", "repo-b"]);

    // Push a new commit to both repos' remotes
    sandbox.commit_file("repo-a", "new-a.txt", "content", "upstream change a");
    common::git(&sandbox.path().join("repo-a"), &["push"]);
    sandbox.commit_file("repo-b", "new-b.txt", "content", "upstream change b");
    common::git(&sandbox.path().join("repo-b"), &["push"]);

    // Sync only repo-a
    Command::cargo_bin("git-rig")
        .unwrap()
        .args(["sync", "--repo", "repo-a"])
        .current_dir(&ws_dir)
        .assert()
        .success()
        .stdout(predicate::str::contains("repo-a"))
        .stdout(predicate::str::contains("->"));

    // Verify repo-a was synced (has the new file)
    assert!(ws_dir.join("repo-a").join("new-a.txt").exists());
    // Verify repo-b was NOT synced (excluded by filter)
    assert!(!ws_dir.join("repo-b").join("new-b.txt").exists());
}

#[test]
fn sync_repo_filter_invalid_repo() {
    let sandbox = common::TestSandbox::new();
    let ws_dir = sandbox.create_workspace_with_repos("my-ws", &["repo-a"]);

    Command::cargo_bin("git-rig")
        .unwrap()
        .args(["sync", "--repo", "nonexistent"])
        .current_dir(&ws_dir)
        .assert()
        .failure()
        .stderr(predicate::str::contains("not in rig"));
}

#[test]
fn sync_repo_filter_multiple() {
    let sandbox = common::TestSandbox::new();
    let ws_dir = sandbox.create_workspace_with_repos("my-ws", &["repo-a", "repo-b", "repo-c"]);

    // Push a new commit to repo-b so we can verify it wasn't synced
    sandbox.commit_file("repo-b", "new-b.txt", "content", "upstream change b");
    common::git(&sandbox.path().join("repo-b"), &["push"]);

    // Sync only repo-a and repo-c
    Command::cargo_bin("git-rig")
        .unwrap()
        .args(["sync", "--repo", "repo-a", "--repo", "repo-c"])
        .current_dir(&ws_dir)
        .assert()
        .success()
        .stdout(predicate::str::contains("repo-a"))
        .stdout(predicate::str::contains("repo-c"))
        .stdout(predicate::str::contains("repo-b").not());

    // Verify repo-b was NOT synced
    assert!(!ws_dir.join("repo-b").join("new-b.txt").exists());
}

#[test]
fn sync_repo_filter_skips_drifted() {
    let sandbox = common::TestSandbox::new();
    let ws_dir = sandbox.create_workspace_with_repos("my-ws", &["repo-a", "repo-b"]);

    // Drift repo-a
    checkout_branch_in_worktree(&ws_dir.join("repo-a"), "wrong-branch");

    // Sync with --repo targeting both — repo-a should show drift, repo-b should sync
    let output = Command::cargo_bin("git-rig")
        .unwrap()
        .args(["sync", "--repo", "repo-a", "--repo", "repo-b"])
        .current_dir(&ws_dir)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8_lossy(&output);

    assert!(stdout.contains("DRIFT"));
    assert!(stdout.contains("repo-a"));
    // repo-b should still sync
    assert!(stdout.contains("ok"));
}

#[test]
fn sync_repo_filter_scopes_drift_warnings() {
    let sandbox = common::TestSandbox::new();
    let ws_dir = sandbox.create_workspace_with_repos("my-ws", &["repo-a", "repo-b"]);

    // Drift repo-a
    checkout_branch_in_worktree(&ws_dir.join("repo-a"), "wrong-branch");

    // Sync only repo-b — should NOT see drift warning for repo-a
    Command::cargo_bin("git-rig")
        .unwrap()
        .args(["sync", "--repo", "repo-b"])
        .current_dir(&ws_dir)
        .assert()
        .success()
        .stdout(predicate::str::contains("DRIFT").not());
}

#[test]
fn sync_repo_filter_detached_skipped() {
    let sandbox = common::TestSandbox::new();
    let ws_dir = sandbox.create_workspace("my-ws");
    let repo_dir = sandbox.create_repo("repo-a");

    // Add repo as detached
    Command::cargo_bin("git-rig")
        .unwrap()
        .args(["add", &repo_dir.to_string_lossy(), "--detach"])
        .current_dir(&ws_dir)
        .assert()
        .success();

    // Sync with --repo targeting the detached repo
    Command::cargo_bin("git-rig")
        .unwrap()
        .args(["sync", "--repo", "repo-a"])
        .current_dir(&ws_dir)
        .assert()
        .success()
        .stdout(predicate::str::contains("detached, skipped"));
}

// ---------------------------------------------------------------------------
// branch conflict detection
// ---------------------------------------------------------------------------

#[test]
fn add_branch_conflict_shows_worktree_location() {
    let sandbox = common::TestSandbox::new();
    let _ws_dir = sandbox.create_workspace_with_repos("my-ws", &["repo-a"]);
    let repo_dir = sandbox.path().join("repo-a");

    // Create a second workspace that tries to use the same branch
    let ws2_dir = sandbox.create_workspace("my-ws2");
    Command::cargo_bin("git-rig")
        .unwrap()
        .args([
            "add",
            "--branch",
            "rig/my-ws", // same branch as first rig
            repo_dir.to_str().unwrap(),
        ])
        .current_dir(&ws2_dir)
        .assert()
        .failure()
        .stderr(predicate::str::contains("already checked out"))
        .stderr(predicate::str::contains("checked out in:"));
}

// ---------------------------------------------------------------------------
// completions
// ---------------------------------------------------------------------------

#[test]
fn completions_bash() {
    Command::cargo_bin("git-rig")
        .unwrap()
        .args(["completions", "bash"])
        .assert()
        .success()
        .stdout(predicate::str::contains("_git-rig"));
}

#[test]
fn completions_zsh() {
    Command::cargo_bin("git-rig")
        .unwrap()
        .args(["completions", "zsh"])
        .assert()
        .success()
        .stdout(predicate::str::contains("#compdef git-rig"));
}

#[test]
fn completions_fish() {
    Command::cargo_bin("git-rig")
        .unwrap()
        .args(["completions", "fish"])
        .assert()
        .success()
        .stdout(predicate::str::contains("complete -c git-rig"));
}

#[test]
fn completions_invalid_shell() {
    Command::cargo_bin("git-rig")
        .unwrap()
        .args(["completions", "invalid"])
        .assert()
        .failure();
}

// ---------------------------------------------------------------------------
// doctor
// ---------------------------------------------------------------------------

#[test]
fn doctor_healthy_rig_exits_zero() {
    let sandbox = common::TestSandbox::new();
    let ws_dir = sandbox.create_workspace_with_repos("my-ws", &["repo-a"]);

    Command::cargo_bin("git-rig")
        .unwrap()
        .arg("doctor")
        .current_dir(&ws_dir)
        .assert()
        .success()
        .stdout(predicate::str::contains("All checks passed"))
        .stdout(predicate::str::contains("source repo exists"))
        .stdout(predicate::str::contains("worktree exists and reachable"))
        .stdout(predicate::str::contains("branch matches manifest"))
        .stdout(predicate::str::contains("origin/HEAD set"))
        .stdout(predicate::str::contains("reachable"));
}

#[test]
fn doctor_shows_environment_checks() {
    let sandbox = common::TestSandbox::new();
    let ws_dir = sandbox.create_workspace_with_repos("my-ws", &["repo-a"]);

    let output = Command::cargo_bin("git-rig")
        .unwrap()
        .arg("doctor")
        .current_dir(&ws_dir)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8_lossy(&output);

    assert!(stdout.contains("git found on PATH"));
    assert!(stdout.contains("git version"));
}

#[test]
fn doctor_outside_rig_shows_env_only() {
    let sandbox = common::TestSandbox::new();

    let output = Command::cargo_bin("git-rig")
        .unwrap()
        .arg("doctor")
        .current_dir(sandbox.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8_lossy(&output);

    assert!(stdout.contains("PASS"));
    assert!(stdout.contains("git found on PATH"));
    assert!(stdout.contains("not inside a rig"));
    // Should NOT contain per-repo checks
    assert!(!stdout.contains("source repo"));
    assert!(!stdout.contains("worktree exists"));
}

#[test]
fn doctor_detects_missing_worktree() {
    let sandbox = common::TestSandbox::new();
    let ws_dir = sandbox.create_workspace_with_repos("my-ws", &["repo-a"]);

    // Delete the worktree directory to simulate missing worktree
    std::fs::remove_dir_all(ws_dir.join("repo-a")).unwrap();

    Command::cargo_bin("git-rig")
        .unwrap()
        .arg("doctor")
        .current_dir(&ws_dir)
        .assert()
        .failure()
        .stdout(predicate::str::contains("FAIL"))
        .stdout(predicate::str::contains("worktree missing"));
}

#[test]
fn doctor_detects_missing_source() {
    let sandbox = common::TestSandbox::new();
    let ws_dir = sandbox.create_workspace_with_repos("my-ws", &["repo-a"]);

    // Delete the source repo
    std::fs::remove_dir_all(sandbox.path().join("repo-a")).unwrap();

    Command::cargo_bin("git-rig")
        .unwrap()
        .arg("doctor")
        .current_dir(&ws_dir)
        .assert()
        .failure()
        .stdout(predicate::str::contains("FAIL"))
        .stdout(predicate::str::contains("source repo missing"));
}

#[test]
fn doctor_detects_branch_mismatch() {
    let sandbox = common::TestSandbox::new();
    let ws_dir = sandbox.create_workspace_with_repos("my-ws", &["repo-a"]);

    // Switch worktree to a different branch
    checkout_branch_in_worktree(&ws_dir.join("repo-a"), "other-branch");

    Command::cargo_bin("git-rig")
        .unwrap()
        .arg("doctor")
        .current_dir(&ws_dir)
        .assert()
        .failure()
        .stdout(predicate::str::contains("WARN"))
        .stdout(predicate::str::contains("branch mismatch"));
}

#[test]
fn doctor_detects_missing_origin_head() {
    let sandbox = common::TestSandbox::new();
    let ws_dir = sandbox.create_workspace_with_repos("my-ws", &["repo-a"]);

    // Remove origin/HEAD from the source repo
    let source_dir = sandbox.path().join("repo-a");
    std::process::Command::new("git")
        .args(["remote", "set-head", "origin", "--delete"])
        .current_dir(&source_dir)
        .output()
        .unwrap();

    Command::cargo_bin("git-rig")
        .unwrap()
        .arg("doctor")
        .current_dir(&ws_dir)
        .assert()
        .failure()
        .stdout(predicate::str::contains("WARN"))
        .stdout(predicate::str::contains("origin/HEAD not set"))
        .stdout(predicate::str::contains("git remote set-head"));
}

#[test]
fn doctor_detects_unreachable_remote() {
    let sandbox = common::TestSandbox::new();
    let ws_dir = sandbox.create_workspace_with_repos("my-ws", &["repo-a"]);

    // Point origin to a non-existent path
    let source_dir = sandbox.path().join("repo-a");
    std::process::Command::new("git")
        .args(["remote", "set-url", "origin", "/nonexistent/path.git"])
        .current_dir(&source_dir)
        .output()
        .unwrap();

    Command::cargo_bin("git-rig")
        .unwrap()
        .arg("doctor")
        .current_dir(&ws_dir)
        .assert()
        .failure()
        .stdout(predicate::str::contains("WARN"))
        .stdout(predicate::str::contains("not reachable"));
}

#[test]
fn doctor_detects_missing_upstream_branch() {
    let sandbox = common::TestSandbox::new();
    let repo_path = sandbox.create_repo("repo-a");
    let ws_dir = sandbox.create_workspace("my-ws");

    // Add repo normally first
    Command::cargo_bin("git-rig")
        .unwrap()
        .arg("add")
        .arg(repo_path.to_str().unwrap())
        .current_dir(&ws_dir)
        .assert()
        .success();

    // Then update upstream to a nonexistent branch (add doubles as update)
    Command::cargo_bin("git-rig")
        .unwrap()
        .args(["add", "--upstream", "nonexistent-branch"])
        .arg(repo_path.to_str().unwrap())
        .current_dir(&ws_dir)
        .assert()
        .success();

    Command::cargo_bin("git-rig")
        .unwrap()
        .arg("doctor")
        .current_dir(&ws_dir)
        .assert()
        .failure()
        .stdout(predicate::str::contains("WARN"))
        .stdout(predicate::str::contains("nonexistent-branch"))
        .stdout(predicate::str::contains("not found on remote"));
}

#[test]
fn doctor_with_explicit_rig_name() {
    let sandbox = common::TestSandbox::new();
    sandbox.create_workspace_with_repos("my-ws", &["repo-a"]);

    Command::cargo_bin("git-rig")
        .unwrap()
        .args(["doctor", "my-ws"])
        .current_dir(sandbox.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("All checks passed"));
}

#[test]
fn doctor_with_nonexistent_rig_name() {
    let sandbox = common::TestSandbox::new();

    Command::cargo_bin("git-rig")
        .unwrap()
        .args(["doctor", "nonexistent"])
        .current_dir(sandbox.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("not found"));
}

#[test]
fn doctor_detects_unexpected_detached_head() {
    let sandbox = common::TestSandbox::new();
    let ws_dir = sandbox.create_workspace_with_repos("my-ws", &["repo-a"]);

    // Detach HEAD in the worktree (manifest expects rig/my-ws)
    detach_head_in_worktree(&ws_dir.join("repo-a"));

    Command::cargo_bin("git-rig")
        .unwrap()
        .arg("doctor")
        .current_dir(&ws_dir)
        .assert()
        .failure()
        .stdout(predicate::str::contains("WARN"))
        .stdout(predicate::str::contains("detached HEAD"))
        .stdout(predicate::str::contains("rig/my-ws"));
}

#[test]
fn doctor_detects_unreachable_worktree() {
    let sandbox = common::TestSandbox::new();
    let ws_dir = sandbox.create_workspace_with_repos("my-ws", &["repo-a"]);

    // Corrupt the worktree metadata so git commands fail but directory still exists
    sandbox.corrupt_worktree_metadata("repo-a");

    Command::cargo_bin("git-rig")
        .unwrap()
        .arg("doctor")
        .current_dir(&ws_dir)
        .assert()
        .failure()
        .stdout(predicate::str::contains("FAIL"))
        .stdout(predicate::str::contains("worktree unreachable"));
}

#[test]
fn doctor_empty_rig() {
    let sandbox = common::TestSandbox::new();
    let ws_dir = sandbox.create_workspace("my-ws");

    Command::cargo_bin("git-rig")
        .unwrap()
        .arg("doctor")
        .current_dir(&ws_dir)
        .assert()
        .success()
        .stdout(predicate::str::contains("No repos"));
}

// ---------------------------------------------------------------------------
// alphabetical ordering
// ---------------------------------------------------------------------------

/// Helper: assert that `a` appears before `b` in `haystack`.
fn assert_appears_before(haystack: &str, a: &str, b: &str) {
    let pos_a = haystack.find(a).unwrap_or_else(|| panic!("'{a}' not found in output"));
    let pos_b = haystack.find(b).unwrap_or_else(|| panic!("'{b}' not found in output"));
    assert!(
        pos_a < pos_b,
        "Expected '{a}' (pos {pos_a}) before '{b}' (pos {pos_b}) in output:\n{haystack}"
    );
}

#[test]
fn status_repos_in_alphabetical_order() {
    let sandbox = common::TestSandbox::new();
    // Add repos in non-alphabetical order
    let ws_dir = sandbox.create_workspace_with_repos("my-ws", &["repo-c", "repo-a", "repo-b"]);

    let output = Command::cargo_bin("git-rig")
        .unwrap()
        .arg("status")
        .current_dir(&ws_dir)
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_appears_before(&stdout, "repo-a", "repo-b");
    assert_appears_before(&stdout, "repo-b", "repo-c");
}

#[test]
fn sync_repos_in_alphabetical_order() {
    let sandbox = common::TestSandbox::new();
    let ws_dir = sandbox.create_workspace_with_repos("my-ws", &["repo-c", "repo-a", "repo-b"]);

    let output = Command::cargo_bin("git-rig")
        .unwrap()
        .arg("sync")
        .current_dir(&ws_dir)
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_appears_before(&stdout, "repo-a", "repo-b");
    assert_appears_before(&stdout, "repo-b", "repo-c");
}

#[test]
fn exec_repos_in_alphabetical_order() {
    let sandbox = common::TestSandbox::new();
    let ws_dir = sandbox.create_workspace_with_repos("my-ws", &["repo-c", "repo-a", "repo-b"]);

    let output = Command::cargo_bin("git-rig")
        .unwrap()
        .args(["exec", "--", "echo", "hello"])
        .current_dir(&ws_dir)
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_appears_before(&stdout, "repo-a", "repo-b");
    assert_appears_before(&stdout, "repo-b", "repo-c");
}

#[test]
fn list_repos_in_alphabetical_order() {
    let sandbox = common::TestSandbox::new();
    let ws_dir = sandbox.create_workspace_with_repos("my-ws", &["repo-c", "repo-a", "repo-b"]);

    let output = Command::cargo_bin("git-rig")
        .unwrap()
        .arg("list")
        .current_dir(&ws_dir)
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_appears_before(&stdout, "repo-a", "repo-b");
    assert_appears_before(&stdout, "repo-b", "repo-c");
}
