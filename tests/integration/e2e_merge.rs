//! End-to-end merge via the `m` key: reducer opens the merge modal, user
//! picks a method with arrows, Enter dispatches `Effect::ExecuteWrite` whose
//! `WriteRequest::Merge` carries the chosen method and `head_sha`. The fake
//! client records `Merge` with that exact triple, and `invalidations_for(Merge)`
//! is non-empty; `WriteCompleted(Ok)` emits a single `Refetch{Merge}`.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use gprr::app::effect::{DataEvent, Effect, WriteRequest};
use gprr::app::event::AppEvent;
use gprr::app::state::{AppState, DetailTab, View};
use gprr::app::update::update;
use gprr::data::cache::{WriteKind, invalidations_for};
use gprr::data::github::GitHubClient;
use gprr::data::github::fake::{FakeFixtures, FakeGitHubClient, RecordedCall};
use gprr::data::models::{
    ChecksRollup, MergeMethod, Mergeable, PrDetail, PrId, PrState, PrSummary, Repo, ReviewDecision,
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

fn sample_detail(id: &PrId, head_sha: &str, methods: Vec<MergeMethod>) -> PrDetail {
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
        head_sha: head_sha.into(),
        merge_commit_sha: None,
        base_ref_oid: "cafe".into(),
        review_threads: vec![],
        timeline: vec![],
        available_merge_methods: methods,
        version: 4,
    }
}

fn key(c: char) -> AppEvent {
    AppEvent::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
}

#[tokio::test]
async fn m_opens_merge_modal_enter_routes_method_and_head_sha_to_fake_and_invalidates() {
    let id = pr_id();
    let head_sha = "abc123def";
    let mut state = AppState::default();
    gprr::app::seed_workspace_from_view(
        &mut state,
        View::PrDetail {
            id: id.clone(),
            tab: DetailTab::Conversation,
        },
    );
    state.pr_details.insert(
        id.clone(),
        sample_detail(&id, head_sha, vec![MergeMethod::Squash, MergeMethod::Merge]),
    );

    update(&mut state, &key('m'));
    assert!(state.merge_modal.is_some(), "m opens merge modal");
    update(
        &mut state,
        &AppEvent::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)),
    );

    let effects = update(
        &mut state,
        &AppEvent::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
    );
    assert!(state.merge_modal.is_none(), "submit closes the modal");
    assert_eq!(effects.len(), 1);
    let (kind, request) = match &effects[0] {
        Effect::ExecuteWrite {
            kind,
            request,
            version_at_submit,
        } => {
            assert_eq!(*version_at_submit, 4);
            (kind.clone(), request.clone())
        }
        other => panic!("expected ExecuteWrite Merge, got {other:?}"),
    };
    assert_eq!(kind, WriteKind::Merge { id: id.clone() });
    let WriteRequest::Merge {
        method,
        head_sha: req_head_sha,
    } = request
    else {
        panic!("expected Merge request");
    };
    assert_eq!(method, MergeMethod::Merge);
    assert_eq!(req_head_sha, head_sha);

    let client = FakeGitHubClient::with_fixtures(FakeFixtures::default());
    client
        .merge(&id, method, &req_head_sha)
        .await
        .expect("fake accepts merge");
    let recorded = client.recorded_calls();
    let merged = recorded
        .iter()
        .find_map(|c| match c {
            RecordedCall::Merge {
                id: rid,
                method: m,
                head_sha: hs,
            } => Some((rid, *m, hs.as_str())),
            _ => None,
        })
        .expect("Merge recorded");
    assert_eq!(merged.0, &id);
    assert_eq!(merged.1, MergeMethod::Merge);
    assert_eq!(merged.2, head_sha);

    let invalidations = invalidations_for(&WriteKind::Merge { id: id.clone() });
    assert!(
        !invalidations.is_empty(),
        "Merge must invalidate at least one cache predicate"
    );

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
