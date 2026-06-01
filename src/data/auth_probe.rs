//! First-run auth probe: shells out to `gh auth token`, verifies the
//! returned token grants required PR-write scopes, and reports a
//! deterministic outcome the caller (`main`) maps to stderr + exit code
//! BEFORE the TUI enters raw mode.

use std::future::Future;

use async_trait::async_trait;

use crate::data::auth::{AuthError, has_required_scopes};
use crate::data::github::GitHubError;

/// Indirection so tests can inject a fake token source. Production wires
/// [`GhTokenFetcher`] which shells out to `gh auth token`.
#[async_trait]
pub trait TokenFetcher: Send + Sync {
    async fn fetch(&self) -> Result<String, AuthError>;
}

/// Real implementation: shells out via [`crate::data::auth::fetch_gh_token`].
#[derive(Debug, Default)]
pub struct GhTokenFetcher;

#[async_trait]
impl TokenFetcher for GhTokenFetcher {
    async fn fetch(&self) -> Result<String, AuthError> {
        crate::data::auth::fetch_gh_token().await
    }
}

/// Result of the first-run probe. `Ok` means the caller can safely enter
/// raw mode; every other variant means the caller must print the bundled
/// message to stderr and exit 1.
#[derive(Debug)]
pub enum ProbeOutcome {
    Ok(String),
    /// `gh` not on PATH. Message is the exact user-facing string.
    GhMissing(String),
    /// `gh` installed but not authenticated.
    Unauthenticated(String),
    /// `gh auth token` returned a token but `auth_scopes` rejected it.
    InsufficientScopes {
        message: String,
        scopes: Vec<String>,
    },
    /// Any other auth error (timeout, unexpected gh failure).
    AuthOther(String),
    /// The probe got a token but `auth_scopes` itself failed (network/auth).
    ScopeFetch(String),
}

impl ProbeOutcome {
    /// Stderr message to print when the outcome is not `Ok`. Empty for `Ok`.
    #[must_use]
    pub fn stderr_message(&self) -> &str {
        match self {
            Self::Ok(_) => "",
            Self::GhMissing(m)
            | Self::Unauthenticated(m)
            | Self::InsufficientScopes { message: m, .. }
            | Self::AuthOther(m)
            | Self::ScopeFetch(m) => m,
        }
    }

    #[must_use]
    pub fn is_ok(&self) -> bool {
        matches!(self, Self::Ok(_))
    }
}

/// Run the first-run probe: fetch a token, then check the granted scopes
/// satisfy [`has_required_scopes`] by invoking `scope_check(token)`.
/// Never enters raw mode; never panics.
///
/// `scope_check` is a closure so the caller can construct the real
/// `GitHubClient` with the just-fetched token before calling `auth_scopes`.
pub async fn probe<F, S, Fut>(fetcher: &F, scope_check: S) -> ProbeOutcome
where
    F: TokenFetcher,
    S: FnOnce(String) -> Fut,
    Fut: Future<Output = Result<Vec<String>, GitHubError>>,
{
    let token = match fetcher.fetch().await {
        Ok(t) => t,
        Err(AuthError::GhMissing) => {
            return ProbeOutcome::GhMissing(AuthError::GhMissing.to_string());
        }
        Err(AuthError::GhUnauthenticated) => {
            return ProbeOutcome::Unauthenticated(AuthError::GhUnauthenticated.to_string());
        }
        Err(e) => return ProbeOutcome::AuthOther(e.to_string()),
    };
    let scopes = match scope_check(token.clone()).await {
        Ok(s) => s,
        Err(GitHubError::Unauthorized) => {
            return ProbeOutcome::Unauthenticated(AuthError::GhUnauthenticated.to_string());
        }
        Err(e) => return ProbeOutcome::ScopeFetch(e.to_string()),
    };
    if !has_required_scopes(&scopes) {
        return ProbeOutcome::InsufficientScopes {
            message: "Re-run `gh auth login --scopes repo`".to_string(),
            scopes,
        };
    }
    ProbeOutcome::Ok(token)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FailingFetcher(AuthError);

    #[async_trait]
    impl TokenFetcher for FailingFetcher {
        async fn fetch(&self) -> Result<String, AuthError> {
            Err(match &self.0 {
                AuthError::GhMissing => AuthError::GhMissing,
                AuthError::GhUnauthenticated => AuthError::GhUnauthenticated,
                AuthError::Timeout(d) => AuthError::Timeout(*d),
                AuthError::Other(s) => AuthError::Other(s.clone()),
            })
        }
    }

    struct OkFetcher;

    #[async_trait]
    impl TokenFetcher for OkFetcher {
        async fn fetch(&self) -> Result<String, AuthError> {
            Ok("ghp_fake".to_string())
        }
    }

    fn returns_scopes(scopes: Vec<String>) -> std::future::Ready<Result<Vec<String>, GitHubError>> {
        std::future::ready(Ok(scopes))
    }

    fn returns_unauthorized(
        _token: String,
    ) -> std::future::Ready<Result<Vec<String>, GitHubError>> {
        std::future::ready(Err(GitHubError::Unauthorized))
    }

    #[tokio::test]
    async fn gh_missing_returns_gh_missing_outcome_with_exact_message() {
        let out = probe(&FailingFetcher(AuthError::GhMissing), |_t| {
            returns_scopes(vec!["repo".into()])
        })
        .await;
        assert!(matches!(out, ProbeOutcome::GhMissing(_)), "got {out:?}");
        assert_eq!(
            out.stderr_message(),
            "gh CLI not found in PATH; install it from https://cli.github.com"
        );
        assert!(!out.is_ok());
    }

    #[tokio::test]
    async fn insufficient_scopes_returns_re_run_message() {
        let out = probe(&OkFetcher, |_t| returns_scopes(vec!["public_repo".into()])).await;
        assert!(
            matches!(out, ProbeOutcome::InsufficientScopes { .. }),
            "got {out:?}"
        );
        assert_eq!(out.stderr_message(), "Re-run `gh auth login --scopes repo`");
    }

    #[tokio::test]
    async fn empty_scopes_returns_re_run_message() {
        let out = probe(&OkFetcher, |_t| returns_scopes(vec![])).await;
        assert!(
            matches!(out, ProbeOutcome::InsufficientScopes { .. }),
            "got {out:?}"
        );
    }

    #[tokio::test]
    async fn repo_scope_returns_ok_with_token() {
        let out = probe(&OkFetcher, |_t| returns_scopes(vec!["repo".into()])).await;
        assert!(out.is_ok(), "got {out:?}");
        match out {
            ProbeOutcome::Ok(t) => assert_eq!(t, "ghp_fake"),
            other => panic!("expected Ok, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn gh_unauthenticated_returns_unauthenticated() {
        let out = probe(&FailingFetcher(AuthError::GhUnauthenticated), |_t| {
            returns_scopes(vec!["repo".into()])
        })
        .await;
        assert!(
            matches!(out, ProbeOutcome::Unauthenticated(_)),
            "got {out:?}"
        );
    }

    #[tokio::test]
    async fn scope_check_unauthorized_maps_to_unauthenticated() {
        let out = probe(&OkFetcher, returns_unauthorized).await;
        assert!(
            matches!(out, ProbeOutcome::Unauthenticated(_)),
            "got {out:?}"
        );
    }

    #[tokio::test]
    async fn scope_check_passes_just_fetched_token_to_closure() {
        let out = probe(&OkFetcher, |t| async move {
            assert_eq!(t, "ghp_fake", "scope_check must receive the probed token");
            Ok(vec!["repo".into()])
        })
        .await;
        assert!(out.is_ok());
    }
}
