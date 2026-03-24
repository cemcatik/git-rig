mod common;
#[allow(dead_code)]
#[path = "../src/error.rs"]
mod error;

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

    Command::cargo_bin("git-rig")
        .unwrap()
        .args(["add", "--upstream", "integration"])
        .arg(repo_path.to_str().unwrap())
        .current_dir(&ws_dir)
        .assert()
        .success()
        .stdout(predicate::str::contains("ok"));

    // Verify upstream is stored in manifest
    let raw = std::fs::read_to_string(ws_dir.join(".rig.json")).unwrap();
    assert!(raw.contains(r#""upstream": "integration""#));
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
}

#[test]
fn status_shows_upstream_indicator() {
    let sandbox = common::TestSandbox::new();
    let ws_dir = sandbox.create_workspace_with_repos("my-ws", &["repo-a"]);
    let repo_dir = sandbox.path().join("repo-a");

    // Set upstream to 'integration'
    Command::cargo_bin("git-rig")
        .unwrap()
        .args(["add", "--upstream", "integration"])
        .arg(repo_dir.to_str().unwrap())
        .current_dir(&ws_dir)
        .assert()
        .success();

    // Status should show "(vs integration)"
    Command::cargo_bin("git-rig")
        .unwrap()
        .arg("status")
        .current_dir(&ws_dir)
        .assert()
        .success()
        .stdout(predicate::str::contains("vs integration"));
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
