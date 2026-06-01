use std::process::Command;

use gpr::app::initial_view;
use gpr::app::state::View;
use tempfile::tempdir;

#[test]
fn initial_view_is_dashboard_when_cwd_has_no_git_remote() {
    let dir = tempdir().expect("tempdir");
    assert!(matches!(initial_view(dir.path()), View::Dashboard));
}

#[test]
fn initial_view_is_dashboard_when_cwd_has_origin_but_not_github() {
    let dir = tempdir().expect("tempdir");
    Command::new("git")
        .args(["init", "--quiet"])
        .current_dir(dir.path())
        .status()
        .unwrap();
    Command::new("git")
        .args(["remote", "add", "origin", "git@gitlab.com:acme/widgets.git"])
        .current_dir(dir.path())
        .status()
        .unwrap();
    assert!(matches!(initial_view(dir.path()), View::Dashboard));
}

#[test]
fn initial_view_is_pr_list_when_cwd_origin_is_github() {
    let dir = tempdir().expect("tempdir");
    Command::new("git")
        .args(["init", "--quiet"])
        .current_dir(dir.path())
        .status()
        .unwrap();
    Command::new("git")
        .args(["remote", "add", "origin", "git@github.com:acme/widgets.git"])
        .current_dir(dir.path())
        .status()
        .unwrap();
    match initial_view(dir.path()) {
        View::PrList { repo } => {
            assert_eq!(repo.owner, "acme");
            assert_eq!(repo.name, "widgets");
        }
        other => panic!("expected PrList, got {other:?}"),
    }
}
