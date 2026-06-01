//! GitHub authentication via the `gh` CLI.
//!
//! We do NOT implement OAuth, accept tokens via env var, or use `gh auth status
//! --show-token`. The 401-refresh-and-retry loop in `octocrab_impl` calls
//! `fetch_gh_token` again to pick up a freshly-refreshed token, then propagates
//! the original 401 if the new token is identical.

use std::time::Duration;
use tokio::process::Command;
use tokio::time::timeout;

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("gh CLI not found in PATH; install it from https://cli.github.com")]
    GhMissing,
    #[error("gh is installed but not authenticated; run `gh auth login --scopes repo`")]
    GhUnauthenticated,
    #[error("gh CLI timed out after {0:?}")]
    Timeout(Duration),
    #[error("gh returned an unexpected error: {0}")]
    Other(String),
}

const GH_TIMEOUT: Duration = Duration::from_secs(5);

/// Map an error from *spawning* the `gh` process to an [`AuthError`]. A
/// `NotFound` error means the `gh` binary is not on `PATH`.
fn classify_gh_spawn_error(e: &std::io::Error) -> AuthError {
    if e.kind() == std::io::ErrorKind::NotFound {
        AuthError::GhMissing
    } else {
        AuthError::Other(e.to_string())
    }
}

pub async fn fetch_gh_token() -> Result<String, AuthError> {
    let fut = Command::new("gh").args(["auth", "token"]).output();
    let output = timeout(GH_TIMEOUT, fut)
        .await
        .map_err(|_| AuthError::Timeout(GH_TIMEOUT))?
        .map_err(|e| classify_gh_spawn_error(&e))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        if stderr.contains("not logged") || stderr.contains("authenticate") {
            return Err(AuthError::GhUnauthenticated);
        }
        return Err(AuthError::Other(stderr));
    }
    let token = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if token.is_empty() {
        return Err(AuthError::GhUnauthenticated);
    }
    Ok(token)
}

/// Returns true when `scopes` contains a scope that grants PR write access.
/// We accept `repo`, or both `public_repo` and `read:user`.
#[must_use]
pub fn has_required_scopes(scopes: &[String]) -> bool {
    let has = |s: &str| scopes.iter().any(|x| x == s);
    has("repo") || (has("public_repo") && has("read:user"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawn_not_found_maps_to_gh_missing() {
        // A missing `gh` binary surfaces as `NotFound` when the process is
        // spawned; it must map to `GhMissing`. Tested directly (no `PATH`
        // mutation) so it stays hermetic and never races other tests that
        // spawn subprocesses.
        let e = std::io::Error::from(std::io::ErrorKind::NotFound);
        assert!(matches!(classify_gh_spawn_error(&e), AuthError::GhMissing));
    }

    #[test]
    fn spawn_other_error_maps_to_other() {
        let e = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        assert!(matches!(classify_gh_spawn_error(&e), AuthError::Other(_)));
    }

    #[test]
    fn scope_accepts_repo() {
        assert!(has_required_scopes(&["repo".into()]));
    }

    #[test]
    fn scope_accepts_public_repo_plus_read_user() {
        assert!(has_required_scopes(&[
            "public_repo".into(),
            "read:user".into()
        ]));
    }

    #[test]
    fn scope_rejects_only_public_repo() {
        assert!(!has_required_scopes(&["public_repo".into()]));
    }

    #[test]
    fn scope_rejects_empty() {
        assert!(!has_required_scopes(&[]));
    }
}
