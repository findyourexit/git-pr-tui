//! End-to-end review submit via the `v` key: reducer opens the modal,
//! selects an option, types a body, Ctrl-S dispatches `Effect::ExecuteWrite`
//! with `head_sha` echoed from `state.pr_details[id].head_sha`. The fake
//! client records `SubmitReview` carrying that exact `head_sha`, and
//! `WriteCompleted(Ok)` triggers cache invalidations per
//! `invalidations_for(SubmitReview)`.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use gpr::app::effect::{DataEvent, Effect, WriteRequest};
use gpr::app::event::AppEvent;
use gpr::app::state::{AppState, DetailTab, View};
use gpr::app::update::update;
use gpr::data::cache::{WriteKind, invalidations_for};
use gpr::data::github::fake::{FakeFixtures, FakeGitHubClient, RecordedCall};
use gpr::data::github::{GitHubClient, SubmitReviewRequest};
use gpr::data::models::{
    ChecksRollup, Mergeable, PrDetail, PrId, PrState, PrSummary, Repo, ReviewDecision, ReviewState,
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

fn sample_detail(id: &PrId, head_sha: &str) -> PrDetail {
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
        head_sha: head_sha.into(),
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

#[tokio::test]
async fn v_opens_modal_submit_routes_head_sha_to_fake_and_invalidates() {
    let id = pr_id();
    let head_sha = "abc123def";
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
        .insert(id.clone(), sample_detail(&id, head_sha));

    update(&mut state, &key('v'));
    let down = AppEvent::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    update(&mut state, &down);
    for c in "needs work".chars() {
        update(&mut state, &key(c));
    }

    let effects = update(
        &mut state,
        &AppEvent::Key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL)),
    );
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
        other => panic!("expected ExecuteWrite SubmitReview, got {other:?}"),
    };
    assert_eq!(kind, WriteKind::SubmitReview { id: id.clone() });
    let WriteRequest::SubmitReview {
        state: review_state,
        body,
        line_comments,
        head_sha: req_head_sha,
    } = request
    else {
        panic!("expected SubmitReview request");
    };
    assert_eq!(review_state, ReviewState::ChangesRequested);
    assert_eq!(body, "needs work");
    assert!(line_comments.is_empty());
    assert_eq!(req_head_sha, head_sha);
    assert!(state.review_modal.is_none(), "modal closes on submit");

    let client = FakeGitHubClient::with_fixtures(FakeFixtures::default());
    client
        .submit_review(SubmitReviewRequest {
            id: id.clone(),
            head_sha: req_head_sha.clone(),
            state: review_state,
            body: body.clone(),
            line_comments: line_comments.clone(),
        })
        .await
        .expect("fake accepts submit_review");

    let recorded = client.recorded_calls();
    let submitted = recorded
        .iter()
        .find_map(|c| match c {
            RecordedCall::SubmitReview(req) => Some(req),
            _ => None,
        })
        .expect("SubmitReview recorded");
    assert_eq!(submitted.id, id);
    assert_eq!(submitted.head_sha, head_sha);
    assert_eq!(submitted.state, ReviewState::ChangesRequested);
    assert_eq!(submitted.body, "needs work");

    let invalidations = invalidations_for(&WriteKind::SubmitReview { id: id.clone() });
    assert!(
        !invalidations.is_empty(),
        "SubmitReview must invalidate at least one cache predicate"
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
