//! Slow integration test for `data::git::fetch_pr_and_checkout`. Gated
//! `#[ignore]` because it hits live GitHub via `git fetch`. Run manually:
//!
//!     cargo test --test integration git_checkout -- --ignored --nocapture
//!
//! Uses a small public repo with a known long-lived PR. If the PR is ever
//! force-deleted upstream this test will start failing; pick another stable
//! PR at that point.

use std::process::Command;

use gprr::data::git::{fetch_pr_and_checkout, is_worktree_dirty};
use tempfile::tempdir;

#[test]
#[ignore = "hits live GitHub via git fetch; run manually"]
fn fetch_pr_and_checkout_clones_and_creates_pr_branch() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path();

    let init = Command::new("git")
        .args(["init", "--quiet", "-b", "main"])
        .current_dir(path)
        .status()
        .expect("git init");
    assert!(init.success());
    let add_remote = Command::new("git")
        .args([
            "remote",
            "add",
            "origin",
            "https://github.com/octocat/Hello-World.git",
        ])
        .current_dir(path)
        .status()
        .expect("git remote add");
    assert!(add_remote.success());

    assert!(!is_worktree_dirty(path).expect("dirtiness check"));

    let branch = fetch_pr_and_checkout(path, 1).expect("fetch + checkout PR #1");
    assert_eq!(branch, "pr/1");

    let head = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(path)
        .output()
        .expect("rev-parse");
    let head = String::from_utf8_lossy(&head.stdout).trim().to_string();
    assert_eq!(head, "pr/1");
}
