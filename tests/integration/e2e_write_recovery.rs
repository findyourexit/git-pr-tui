//! End-to-end write-recovery: reducer emits `ExecuteWrite`, we route it to
//! the fake client (which honors `FakeFixtures::next_error`), feed the result
//! back as `DataEvent::WriteCompleted`, and assert the resulting
//! `PendingStatus` + emitted effects.

use gprr::app::effect::{DataEvent, Effect, WriteRequest};
use gprr::app::event::AppEvent;
use gprr::app::state::{AppState, PendingStatus, PendingWrite, ToastKind};
use gprr::app::update::update;
use gprr::data::cache::WriteKind;
use gprr::data::github::GitHubError;
use gprr::data::github::fake::{FakeFixtures, WriteOp};
use gprr::data::models::{PrId, Repo};

use super::harness::fake_with;

fn pr_id() -> PrId {
    PrId {
        repo: Repo {
            owner: "acme".into(),
            name: "widgets".into(),
        },
        number: 42,
    }
}

fn pending(kind: WriteKind, request: WriteRequest) -> PendingWrite {
    PendingWrite {
        kind,
        request,
        version_at_submit: 1,
        status: PendingStatus::Submitting,
    }
}

async fn execute_post_pr_comment(
    client: &gprr::data::SharedGitHubClient,
    id: &PrId,
    body: &str,
) -> Result<(), GitHubError> {
    client.post_pr_comment(id, body.to_string()).await
}

#[tokio::test]
async fn write_recovery_ok_clears_pending_and_emits_refetch() {
    let id = pr_id();
    let kind = WriteKind::PostPrComment { id: id.clone() };
    let client = fake_with(FakeFixtures::default());

    let mut state = AppState {
        pending_write: Some(pending(
            kind.clone(),
            WriteRequest::PostPrComment { body: "ok".into() },
        )),
        ..AppState::default()
    };

    let result = execute_post_pr_comment(&client, &id, "ok").await;
    assert!(result.is_ok());

    let effects = update(
        &mut state,
        &AppEvent::Data(DataEvent::WriteCompleted {
            kind: kind.clone(),
            result: Ok(()),
        }),
    );
    assert!(state.pending_write.is_none());
    assert!(matches!(effects.as_slice(), [Effect::Refetch { kind: k }] if k == &kind));
}

#[tokio::test]
async fn write_recovery_validation_keeps_pending_no_refetch() {
    let id = pr_id();
    let kind = WriteKind::PostPrComment { id: id.clone() };
    let fixtures = FakeFixtures::default();
    *fixtures.next_error.lock().unwrap() = Some((
        WriteOp::PostPrComment,
        GitHubError::Validation("body required".into()),
    ));
    let client = fake_with(fixtures);

    let mut state = AppState {
        pending_write: Some(pending(
            kind.clone(),
            WriteRequest::PostPrComment {
                body: String::new(),
            },
        )),
        ..AppState::default()
    };

    let err = execute_post_pr_comment(&client, &id, "")
        .await
        .expect_err("validation error injected");
    let effects = update(
        &mut state,
        &AppEvent::Data(DataEvent::WriteCompleted {
            kind,
            result: Err(err),
        }),
    );

    assert!(effects.is_empty(), "validation never auto-refetches");
    let pw = state.pending_write.as_ref().expect("composer stays open");
    assert_eq!(pw.status, PendingStatus::Validation("body required".into()));
    assert!(state.toast.is_none(), "validation surfaces inline only");
}

#[tokio::test]
async fn write_recovery_server_5xx_unknown_outcome_with_refetch_and_toast() {
    let id = pr_id();
    let kind = WriteKind::PostPrComment { id: id.clone() };
    let fixtures = FakeFixtures::default();
    *fixtures.next_error.lock().unwrap() = Some((WriteOp::PostPrComment, GitHubError::Server(502)));
    let client = fake_with(fixtures);

    let mut state = AppState {
        pending_write: Some(pending(
            kind.clone(),
            WriteRequest::PostPrComment {
                body: "hello".into(),
            },
        )),
        ..AppState::default()
    };

    let err = execute_post_pr_comment(&client, &id, "hello")
        .await
        .unwrap_err();
    let effects = update(
        &mut state,
        &AppEvent::Data(DataEvent::WriteCompleted {
            kind: kind.clone(),
            result: Err(err),
        }),
    );

    assert!(matches!(effects.as_slice(), [Effect::Refetch { kind: k }] if k == &kind));
    let pw = state
        .pending_write
        .as_ref()
        .expect("pending kept for inspection");
    assert_eq!(pw.status, PendingStatus::UnknownOutcome);
    let toast = state.toast.as_ref().expect("warning toast");
    assert_eq!(toast.kind, ToastKind::Warning);
    assert_eq!(toast.message, "Outcome unknown — refreshed the PR");
}

#[tokio::test]
async fn write_recovery_stale_head_sha_keeps_pending_with_expected_sha() {
    let id = pr_id();
    let kind = WriteKind::SubmitReview { id: id.clone() };
    let fixtures = FakeFixtures::default();
    *fixtures.next_error.lock().unwrap() = Some((
        WriteOp::SubmitReview,
        GitHubError::StaleHeadSha {
            expected: "deadbeef".into(),
        },
    ));
    let client = fake_with(fixtures);

    let request = WriteRequest::SubmitReview {
        state: gprr::data::models::ReviewState::Approved,
        body: "lgtm".into(),
        line_comments: vec![],
        head_sha: "stale".into(),
    };
    let mut state = AppState {
        pending_write: Some(pending(kind.clone(), request.clone())),
        ..AppState::default()
    };

    let err = client
        .submit_review(gprr::data::github::SubmitReviewRequest {
            id: id.clone(),
            head_sha: "stale".into(),
            state: gprr::data::models::ReviewState::Approved,
            body: "lgtm".into(),
            line_comments: vec![],
        })
        .await
        .unwrap_err();
    let effects = update(
        &mut state,
        &AppEvent::Data(DataEvent::WriteCompleted {
            kind,
            result: Err(err),
        }),
    );

    assert!(effects.is_empty());
    let pw = state.pending_write.as_ref().expect("composer stays open");
    assert_eq!(pw.status, PendingStatus::Stale("deadbeef".into()));
    assert!(state.toast.is_none(), "stale stays inline on composer");
}

#[tokio::test]
async fn write_recovery_conflict_surfaces_toast_and_keeps_pending() {
    let id = pr_id();
    let kind = WriteKind::Merge { id: id.clone() };
    let fixtures = FakeFixtures::default();
    *fixtures.next_error.lock().unwrap() = Some((
        WriteOp::Merge,
        GitHubError::Conflict("merge conflict".into()),
    ));
    let client = fake_with(fixtures);

    let request = WriteRequest::Merge {
        method: gprr::data::models::MergeMethod::Merge,
        head_sha: "abc".into(),
    };
    let mut state = AppState {
        pending_write: Some(pending(kind.clone(), request)),
        ..AppState::default()
    };

    let err = client
        .merge(&id, gprr::data::models::MergeMethod::Merge, "abc")
        .await
        .unwrap_err();
    let effects = update(
        &mut state,
        &AppEvent::Data(DataEvent::WriteCompleted {
            kind,
            result: Err(err),
        }),
    );

    assert!(effects.is_empty());
    let pw = state.pending_write.as_ref().expect("pending preserved");
    assert_eq!(pw.status, PendingStatus::Conflict("merge conflict".into()));
    let toast = state.toast.as_ref().expect("conflict toast");
    assert_eq!(toast.kind, ToastKind::Error);
    assert!(toast.message.contains("merge conflict"));
}
