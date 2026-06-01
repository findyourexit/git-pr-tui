//! Local git repo introspection: detect the GitHub repo from `origin`,
//! check worktree dirtiness, and fetch + checkout a PR branch. All
//! operations shell out to the `git` CLI so this module has no extra
//! dependencies and faithfully respects the user's git config.

use std::path::Path;
use std::process::Command;

use crate::data::models::Repo;

#[derive(Debug, thiserror::Error)]
pub enum GitError {
    #[error("not a git repository: {0}")]
    NotARepo(String),
    #[error("no `origin` remote configured")]
    NoOrigin,
    #[error("origin remote does not point at github.com: {0}")]
    NotGitHub(String),
    #[error("git command failed: {0}")]
    CommandFailed(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

/// Resolve the `origin` remote of the git repo at `path` to a `Repo` if and
/// only if origin points at github.com. Supports both forms:
///   - ssh:  `git@github.com:owner/name.git`
///   - https: `https://github.com/owner/name(.git)?`
pub fn detect_repo(path: &Path) -> Result<Repo, GitError> {
    let url = origin_url(path)?;
    parse_github_remote(&url).ok_or(GitError::NotGitHub(url))
}

fn origin_url(path: &Path) -> Result<String, GitError> {
    let output = Command::new("git")
        .args(["config", "--get", "remote.origin.url"])
        .current_dir(path)
        .output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        if stderr.is_empty() {
            return Err(GitError::NoOrigin);
        }
        return Err(GitError::CommandFailed(stderr));
    }
    let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if url.is_empty() {
        return Err(GitError::NoOrigin);
    }
    Ok(url)
}

fn parse_github_remote(url: &str) -> Option<Repo> {
    let owner_name = url
        .strip_prefix("git@github.com:")
        .or_else(|| url.strip_prefix("https://github.com/"))
        .or_else(|| url.strip_prefix("ssh://git@github.com/"))?;
    let owner_name = owner_name.trim_end_matches('/').trim_end_matches(".git");
    let (owner, name) = owner_name.split_once('/')?;
    if owner.is_empty() || name.is_empty() || name.contains('/') {
        return None;
    }
    Some(Repo {
        owner: owner.to_string(),
        name: name.to_string(),
    })
}

/// Returns `true` if the worktree at `path` has unstaged or staged changes.
/// Mirrors `git diff --quiet && git diff --cached --quiet`: a non-zero exit
/// from either means dirty.
pub fn is_worktree_dirty(path: &Path) -> Result<bool, GitError> {
    if !diff_quiet(path, &["diff", "--quiet"])? {
        return Ok(true);
    }
    if !diff_quiet(path, &["diff", "--cached", "--quiet"])? {
        return Ok(true);
    }
    Ok(false)
}

/// Returns `true` if `git <args>` exited 0 (clean), `false` if exited 1
/// (dirty). Any other exit (e.g. fatal repo error) is surfaced as `GitError`.
fn diff_quiet(path: &Path, args: &[&str]) -> Result<bool, GitError> {
    let output = Command::new("git").args(args).current_dir(path).output()?;
    match output.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            Err(GitError::CommandFailed(stderr))
        }
    }
}

/// Fetch PR `pr_number` from `origin` into a local branch `pr/<n>` and
/// check it out. Equivalent to
/// `git fetch origin pull/<n>/head:pr/<n> && git checkout pr/<n>`.
/// Callers MUST verify the worktree is clean first via `is_worktree_dirty`.
pub fn fetch_pr_and_checkout(path: &Path, pr_number: u32) -> Result<String, GitError> {
    let branch = format!("pr/{pr_number}");
    let refspec = format!("pull/{pr_number}/head:{branch}");
    run_git(path, &["fetch", "origin", &refspec, "--force"])?;
    run_git(path, &["checkout", &branch])?;
    Ok(branch)
}

fn run_git(path: &Path, args: &[&str]) -> Result<(), GitError> {
    let output = Command::new("git").args(args).current_dir(path).output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        return Err(GitError::CommandFailed(format!("git {args:?}: {stderr}")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn init_repo_with_origin(url: &str) -> tempfile::TempDir {
        let dir = tempdir().expect("tempdir");
        let path = dir.path();
        let init = Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(path)
            .status()
            .expect("git init");
        assert!(init.success(), "git init failed");
        let add = Command::new("git")
            .args(["remote", "add", "origin", url])
            .current_dir(path)
            .status()
            .expect("git remote add");
        assert!(add.success(), "git remote add failed");
        dir
    }

    #[test]
    fn detect_repo_parses_ssh_form() {
        let dir = init_repo_with_origin("git@github.com:acme/widgets.git");
        let repo = detect_repo(dir.path()).expect("ssh form parses");
        assert_eq!(repo.owner, "acme");
        assert_eq!(repo.name, "widgets");
    }

    #[test]
    fn detect_repo_parses_https_form_with_git_suffix() {
        let dir = init_repo_with_origin("https://github.com/acme/widgets.git");
        let repo = detect_repo(dir.path()).expect("https.git parses");
        assert_eq!(repo.owner, "acme");
        assert_eq!(repo.name, "widgets");
    }

    #[test]
    fn detect_repo_parses_https_form_without_git_suffix() {
        let dir = init_repo_with_origin("https://github.com/acme/widgets");
        let repo = detect_repo(dir.path()).expect("https parses");
        assert_eq!(repo.owner, "acme");
        assert_eq!(repo.name, "widgets");
    }

    #[test]
    fn detect_repo_parses_ssh_protocol_form() {
        let dir = init_repo_with_origin("ssh://git@github.com/acme/widgets.git");
        let repo = detect_repo(dir.path()).expect("ssh:// parses");
        assert_eq!(repo.owner, "acme");
        assert_eq!(repo.name, "widgets");
    }

    #[test]
    fn detect_repo_rejects_non_github_origin() {
        let dir = init_repo_with_origin("git@gitlab.com:acme/widgets.git");
        let err = detect_repo(dir.path()).expect_err("gitlab is not github");
        assert!(matches!(err, GitError::NotGitHub(_)));
    }

    #[test]
    fn detect_repo_rejects_repo_without_origin() {
        let dir = tempdir().unwrap();
        Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(dir.path())
            .status()
            .unwrap();
        let err = detect_repo(dir.path()).expect_err("no origin");
        assert!(matches!(err, GitError::NoOrigin));
    }

    fn init_committable_repo() -> tempfile::TempDir {
        let dir = tempdir().expect("tempdir");
        let path = dir.path();
        for args in [
            &["init", "--quiet", "-b", "main"][..],
            &["config", "user.email", "t@e.st"][..],
            &["config", "user.name", "t"][..],
            &["commit", "--allow-empty", "-m", "init", "--quiet"][..],
        ] {
            let st = Command::new("git")
                .args(args)
                .current_dir(path)
                .status()
                .expect("git");
            assert!(st.success(), "git {args:?} failed");
        }
        dir
    }

    #[test]
    fn is_worktree_dirty_false_on_clean_repo() {
        let dir = init_committable_repo();
        assert!(!is_worktree_dirty(dir.path()).unwrap());
    }

    #[test]
    fn is_worktree_dirty_true_with_unstaged_change() {
        let dir = init_committable_repo();
        std::fs::write(dir.path().join("file"), "foo").unwrap();
        Command::new("git")
            .args(["add", "file"])
            .current_dir(dir.path())
            .status()
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", "add", "--quiet"])
            .current_dir(dir.path())
            .status()
            .unwrap();
        std::fs::write(dir.path().join("file"), "bar").unwrap();
        assert!(is_worktree_dirty(dir.path()).unwrap());
    }

    #[test]
    fn is_worktree_dirty_true_with_staged_change() {
        let dir = init_committable_repo();
        std::fs::write(dir.path().join("file"), "foo").unwrap();
        Command::new("git")
            .args(["add", "file"])
            .current_dir(dir.path())
            .status()
            .unwrap();
        assert!(is_worktree_dirty(dir.path()).unwrap());
    }
}
