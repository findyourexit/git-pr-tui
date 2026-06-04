//! Shared scaffolding for integration tests. Constructs `App` over the fake
//! client, drives the reducer through a list of `AppEvent`s, and exposes the
//! final `TestBackend` buffer for golden-frame assertions.
//!
//! Used by every `e2e_*` and `ui::*_test` file.

#![allow(dead_code)]

use gprr::data::{SharedGitHubClient, fake_client, github::fake::FakeFixtures};

pub fn fake_with(fixtures: FakeFixtures) -> SharedGitHubClient {
    fake_client(fixtures)
}
