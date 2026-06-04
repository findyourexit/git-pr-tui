//! End-to-end proof that a successful write triggers a *real* refetch:
//!   `ExecuteWrite` → (executor) `WriteCompleted(Ok)`
//!     → (reducer) `Effect::Refetch`
//!       → (run-loop expansion) `refetch_effects` → `FetchPrDetail`
//!         → (executor) `PrDetailLoaded`
//!           → (reducer) refreshed `PrDetail` (version bumped).
//!
//! Guards against the `Effect::Refetch` being silently dropped — the C1
//! regression where the executor no-op'd it and writes never refreshed.

use std::sync::Arc;
use std::time::Duration;

use gprr::app::effect::{DataEvent, Effect};
use gprr::app::event::AppEvent;
use gprr::app::executor::execute_effect;
use gprr::app::state::{AppState, DetailTab, View};
use gprr::app::update::{refetch_effects, update};
use gprr::data::SharedGitHubClient;
use gprr::data::cache::WriteKind;
use gprr::data::github::fake::{FakeFixtures, FakeGitHubClient};
use gprr::data::models::{
    ChecksRollup, Mergeable, PrDetail, PrId, PrState, PrSummary, Repo, ReviewDecision,
};
use tokio::sync::mpsc::unbounded_channel;

fn pr_id() -> PrId {
    PrId {
        repo: Repo {
            owner: "acme".into(),
            name: "widgets".into(),
        },
        number: 7,
    }
}

fn sample_detail(id: &PrId) -> PrDetail {
    PrDetail {
        summary: PrSummary {
            id: id.clone(),
            title: "T".into(),
            author: "octocat".into(),
            state: PrState::Open,
            is_draft: false,
            base: "main".into(),
            head: "feat".into(),
            additions: 0,
            deletions: 0,
            changed_files: 0,
            comments: 0,
            review_decision: Some(ReviewDecision::ReviewRequired),
            checks: ChecksRollup::Success,
            mergeable: Mergeable::Clean,
            labels: vec![],
            updated_at: chrono::Utc::now(),
            created_at: chrono::Utc::now(),
        },
        body: String::new(),
        body_html: None,
        requested_reviewers: vec![],
        assignees: vec![],
        head_sha: "deadbeef".into(),
        merge_commit_sha: None,
        base_ref_oid: "cafe".into(),
        review_threads: vec![],
        timeline: vec![],
        available_merge_methods: vec![],
        version: 3,
    }
}

#[tokio::test]
async fn successful_write_refetches_pr_detail_through_the_full_loop() {
    let id = pr_id();
    let fixtures = FakeFixtures {
        pr_detail: vec![(id.clone(), sample_detail(&id))],
        ..FakeFixtures::default()
    };
    let client: SharedGitHubClient = Arc::new(FakeGitHubClient::with_fixtures(fixtures));
    let (tx, mut rx) = unbounded_channel();

    let mut state = AppState::default();
    gprr::app::seed_workspace_from_view(
        &mut state,
        View::PrDetail {
            id: id.clone(),
            tab: DetailTab::Conversation,
        },
    );
    state.pr_details.insert(id.clone(), sample_detail(&id));
    let version_before = state.pr_details.get(&id).unwrap().version;

    // 1. A successful write makes the reducer emit Refetch.
    let kind = WriteKind::PostPrComment { id: id.clone() };
    let refetch = update(
        &mut state,
        &AppEvent::Data(DataEvent::WriteCompleted {
            kind: kind.clone(),
            result: Ok(()),
        }),
    );
    assert!(
        matches!(refetch.as_slice(), [Effect::Refetch { kind: k }] if k == &kind),
        "write success must emit Refetch, got {refetch:?}"
    );

    // 2. The run loop expands Refetch into concrete fetch effects.
    let expanded = refetch_effects(&state, &kind);
    assert!(
        expanded
            .iter()
            .any(|e| matches!(e, Effect::FetchPrDetail { id: i } if *i == id)),
        "Refetch must expand to FetchPrDetail, got {expanded:?}"
    );

    // 3. Executing those effects hits the fake client and routes events back.
    for eff in expanded {
        execute_effect(eff, client.clone(), tx.clone());
    }
    drop(tx); // close the channel once the spawned tasks finish.

    let mut absorbed_detail = false;
    while let Ok(Some(ev)) = tokio::time::timeout(Duration::from_secs(2), rx.recv()).await {
        let is_detail = matches!(ev, DataEvent::PrDetailLoaded { .. });
        update(&mut state, &AppEvent::Data(ev));
        if is_detail {
            absorbed_detail = true;
        }
    }

    // 4. The PR detail was actually re-fetched and re-absorbed (version bumps).
    assert!(
        absorbed_detail,
        "FetchPrDetail must route a PrDetailLoaded back"
    );
    let version_after = state.pr_details.get(&id).unwrap().version;
    assert!(
        version_after > version_before,
        "refetched detail must bump the version ({version_before} -> {version_after})"
    );
}
