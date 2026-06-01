pub mod auth;
pub mod auth_probe;
pub mod cache;
pub mod git;
pub mod github;
pub mod models;

use std::sync::Arc;

pub type SharedGitHubClient = Arc<dyn github::GitHubClient>;

#[must_use]
pub fn fake_client(fixtures: github::fake::FakeFixtures) -> SharedGitHubClient {
    Arc::new(github::fake::FakeGitHubClient::with_fixtures(fixtures))
}
