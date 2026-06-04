//! End-to-end PR-level comment via the `c` key: reducer opens the composer,
//! collects typed input, dispatches `Effect::ExecuteWrite`, the fake client
//! records the call, and the resulting `DataEvent::WriteCompleted(Ok)` clears
//! the pending write and emits `Effect::Refetch { PostPrComment }`.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use gprr::app::effect::{DataEvent, Effect, WriteRequest};
use gprr::app::event::AppEvent;
use gprr::app::state::{AppState, DetailTab, View};
use gprr::app::update::update;
use gprr::data::cache::WriteKind;
use gprr::data::github::GitHubClient;
use gprr::data::github::fake::{FakeFixtures, FakeGitHubClient, RecordedCall};
use gprr::data::models::{
    ChecksRollup, Mergeable, PrDetail, PrId, PrState, PrSummary, Repo, ReviewDecision,
};

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

fn key(c: char) -> AppEvent {
    AppEvent::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
}

#[tokio::test]
async fn c_opens_composer_submit_routes_to_fake_then_refetches() {
    let id = pr_id();
    let mut state = AppState::default();
    gprr::app::seed_workspace_from_view(
        &mut state,
        View::PrDetail {
            id: id.clone(),
            tab: DetailTab::Conversation,
        },
    );
    state.pr_details.insert(id.clone(), sample_detail(&id));

    update(&mut state, &key('c'));
    update(&mut state, &key('l'));
    update(&mut state, &key('g'));
    update(&mut state, &key('t'));
    update(&mut state, &key('m'));

    let effects = update(
        &mut state,
        &AppEvent::Key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL)),
    );
    assert_eq!(effects.len(), 1);
    let (kind, body, version) = match &effects[0] {
        Effect::ExecuteWrite {
            kind,
            request: WriteRequest::PostPrComment { body },
            version_at_submit,
        } => (kind.clone(), body.clone(), *version_at_submit),
        other => panic!("expected ExecuteWrite PostPrComment, got {other:?}"),
    };
    assert_eq!(kind, WriteKind::PostPrComment { id: id.clone() });
    assert_eq!(body, "lgtm");
    assert_eq!(version, 3);
    assert!(state.composer.is_none(), "composer closes on submit");

    let client = FakeGitHubClient::with_fixtures(FakeFixtures::default());
    client
        .post_pr_comment(&id, body.clone())
        .await
        .expect("fake accepts post");

    let recorded = client.recorded_calls();
    let posted = recorded
        .iter()
        .find_map(|c| match c {
            RecordedCall::PostPrComment {
                id: rid,
                body: rbody,
            } => Some((rid, rbody)),
            _ => None,
        })
        .expect("PostPrComment recorded");
    assert_eq!(posted.0, &id);
    assert_eq!(posted.1, "lgtm");

    let refetch_effects = update(
        &mut state,
        &AppEvent::Data(DataEvent::WriteCompleted {
            kind: kind.clone(),
            result: Ok(()),
        }),
    );
    assert!(state.pending_write.is_none());
    assert!(matches!(
        refetch_effects.as_slice(),
        [Effect::Refetch { kind: k }] if k == &kind
    ));
}
