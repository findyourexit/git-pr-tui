//! End-to-end close + reopen via `x`: the reducer opens a confirm modal,
//! Enter dispatches `Effect::ExecuteWrite { Close|Reopen }`, the fake client
//! records the call, `invalidations_for` is non-empty, and `WriteCompleted(Ok)`
//! emits a single matching `Refetch`.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use gpr::app::effect::{DataEvent, Effect, WriteRequest};
use gpr::app::event::AppEvent;
use gpr::app::state::{AppState, DetailTab, View};
use gpr::app::update::update;
use gpr::data::cache::{WriteKind, invalidations_for};
use gpr::data::github::GitHubClient;
use gpr::data::github::fake::{FakeFixtures, FakeGitHubClient, RecordedCall};
use gpr::data::models::{
    ChecksRollup, Mergeable, PrDetail, PrId, PrState, PrSummary, Repo, ReviewDecision,
};

fn pr_id() -> PrId {
    PrId {
        repo: Repo {
            owner: "acme".into(),
            name: "widgets".into(),
        },
        number: 9,
    }
}

fn sample_detail(id: &PrId, state: PrState) -> PrDetail {
    PrDetail {
        summary: PrSummary {
            id: id.clone(),
            title: "T".into(),
            author: "octocat".into(),
            state,
            is_draft: false,
            base: "main".into(),
            head: "feat".into(),
            additions: 0,
            deletions: 0,
            changed_files: 0,
            comments: 0,
            review_decision: Some(ReviewDecision::Approved),
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
        head_sha: "abc".into(),
        merge_commit_sha: None,
        base_ref_oid: "cafe".into(),
        review_threads: vec![],
        timeline: vec![],
        available_merge_methods: vec![],
        version: 4,
    }
}

fn key(c: char) -> AppEvent {
    AppEvent::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
}

fn enter() -> AppEvent {
    AppEvent::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
}

#[tokio::test]
async fn x_on_open_pr_confirm_routes_close_to_fake_and_invalidates() {
    let id = pr_id();
    let mut state = AppState::default();
    gpr::app::seed_workspace_from_view(
        &mut state,
        View::PrDetail {
            id: id.clone(),
            tab: DetailTab::Conversation,
        },
    );
    state
        .pr_details
        .insert(id.clone(), sample_detail(&id, PrState::Open));

    update(&mut state, &key('x'));
    assert!(state.close_reopen_confirm.is_some());
    let effects = update(&mut state, &enter());
    assert!(state.close_reopen_confirm.is_none());
    assert_eq!(effects.len(), 1);
    let (kind, request) = match &effects[0] {
        Effect::ExecuteWrite { kind, request, .. } => (kind.clone(), request.clone()),
        other => panic!("expected ExecuteWrite, got {other:?}"),
    };
    assert_eq!(kind, WriteKind::Close { id: id.clone() });
    assert!(matches!(request, WriteRequest::Close));

    let client = FakeGitHubClient::with_fixtures(FakeFixtures::default());
    client.close(&id).await.expect("fake accepts close");
    let recorded = client.recorded_calls();
    assert!(
        recorded
            .iter()
            .any(|c| matches!(c, RecordedCall::Close { id: rid } if rid == &id)),
        "Close recorded"
    );

    let invs = invalidations_for(&WriteKind::Close { id: id.clone() });
    assert!(!invs.is_empty(), "Close must invalidate cache predicates");

    let refetch = update(
        &mut state,
        &AppEvent::Data(DataEvent::WriteCompleted {
            kind: kind.clone(),
            result: Ok(()),
        }),
    );
    assert!(state.pending_write.is_none());
    assert!(matches!(
        refetch.as_slice(),
        [Effect::Refetch { kind: k }] if k == &kind
    ));
}

#[tokio::test]
async fn x_on_closed_pr_confirm_routes_reopen_to_fake_and_invalidates() {
    let id = pr_id();
    let mut state = AppState::default();
    gpr::app::seed_workspace_from_view(
        &mut state,
        View::PrDetail {
            id: id.clone(),
            tab: DetailTab::Conversation,
        },
    );
    state
        .pr_details
        .insert(id.clone(), sample_detail(&id, PrState::Closed));

    update(&mut state, &key('x'));
    let effects = update(&mut state, &enter());
    let (kind, request) = match &effects[0] {
        Effect::ExecuteWrite { kind, request, .. } => (kind.clone(), request.clone()),
        other => panic!("expected ExecuteWrite, got {other:?}"),
    };
    assert_eq!(kind, WriteKind::Reopen { id: id.clone() });
    assert!(matches!(request, WriteRequest::Reopen));

    let client = FakeGitHubClient::with_fixtures(FakeFixtures::default());
    client.reopen(&id).await.expect("fake accepts reopen");
    let recorded = client.recorded_calls();
    assert!(
        recorded
            .iter()
            .any(|c| matches!(c, RecordedCall::Reopen { id: rid } if rid == &id)),
        "Reopen recorded"
    );

    let invs = invalidations_for(&WriteKind::Reopen { id: id.clone() });
    assert!(!invs.is_empty(), "Reopen must invalidate cache predicates");

    let refetch = update(
        &mut state,
        &AppEvent::Data(DataEvent::WriteCompleted {
            kind: kind.clone(),
            result: Ok(()),
        }),
    );
    assert!(matches!(
        refetch.as_slice(),
        [Effect::Refetch { kind: k }] if k == &kind
    ));
}
