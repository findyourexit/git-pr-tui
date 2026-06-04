//! Live-API smoke test. Hits real GitHub. Off by default.
//! Run with: `cargo test --test live_api -- --ignored`
//!
//! Requires `gh` installed and `gh auth login` completed.

use gprr::data::github::GitHubClient;
use gprr::data::github::octocrab_impl::{OctocrabConfig, OctocrabGitHubClient};

#[tokio::test]
#[ignore = "hits live GitHub API; run manually"]
async fn viewer_login_round_trips() {
    let client = OctocrabGitHubClient::from_gh_cli(OctocrabConfig::default())
        .await
        .expect("auth");
    let login = client.viewer_login().await.expect("viewer");
    assert!(!login.is_empty(), "viewer login was empty");
}
