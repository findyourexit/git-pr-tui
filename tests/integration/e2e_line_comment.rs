//! End-to-end line-comment submission via the diff viewer.
//!
//! Success path: `Shift-V` → `j` → `c` → type body → `Ctrl-S` drives
//! `Effect::ExecuteWrite { PostLineComment }` to `FakeGitHubClient.post_line_comment`,
//! which records a `RecordedCall::PostLineComment` carrying the right
//! `head_sha`, `path`, `side`, `line`, and `body`.
//!
//! Stale-sha path: when `FakeFixtures::next_error` injects
//! `GitHubError::StaleHeadSha { expected: "newsha" }`, the reducer sets
//! `PendingStatus::Stale("newsha")`, keeps the pending write around so the
//! body is recoverable, and the annotation
//! "Head moved from oldsha to newsha — press R to re-anchor on the latest
//! diff and resubmit." is derivable from
//! `pending_write.request.head_sha` (old) and `pending_status.Stale(...)` (new).

use std::sync::{Arc, Mutex};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use gprr::app::effect::{DataEvent, Effect, WriteRequest};
use gprr::app::event::AppEvent;
use gprr::app::state::{AppState, PendingStatus, View};
use gprr::app::update::update;
use gprr::data::cache::WriteKind;
use gprr::data::github::diff_position::DiffPositionIndex;
use gprr::data::github::fake::{FakeFixtures, FakeGitHubClient, RecordedCall, WriteOp};
use gprr::data::github::{GitHubClient, GitHubError};
use gprr::data::models::{
    ChecksRollup, DiffSide, FileDiff, FileStatus, Mergeable, PrDetail, PrFiles, PrId, PrState,
    PrSummary, Repo, ReviewDecision,
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

fn sample_patch() -> String {
    "diff --git a/src/lib.rs b/src/lib.rs\nindex 1111111..2222222 100644\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,3 +1,4 @@\n fn main() {\n     let x = 1;\n+    let y = 2;\n }\n".into()
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

fn sample_files(patch: &str) -> PrFiles {
    PrFiles {
        files: vec![FileDiff {
            path: "src/lib.rs".into(),
            previous_path: None,
            status: FileStatus::Modified,
            additions: 1,
            deletions: 0,
            patch: Some(patch.to_string()),
            is_binary: false,
            is_submodule: false,
            is_generated: None,
            oversize: false,
            blob_sha_before: None,
            blob_sha_after: None,
        }],
        truncated: false,
    }
}

fn diff_state(head_sha: &str) -> (AppState, PrId, String) {
    let id = pr_id();
    let patch = sample_patch();
    let mut state = AppState::default();
    gprr::app::seed_workspace_from_view(
        &mut state,
        View::Diff {
            id: id.clone(),
            file_index: 0,
        },
    );
    state.pr_files.insert(id.clone(), sample_files(&patch));
    state
        .pr_details
        .insert(id.clone(), sample_detail(&id, head_sha));
    (state, id, patch)
}

fn key(c: char) -> AppEvent {
    AppEvent::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
}

fn shift_v() -> AppEvent {
    AppEvent::Key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::SHIFT))
}

fn ctrl_s() -> AppEvent {
    AppEvent::Key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL))
}

fn position_at_added_line(patch: &str) -> usize {
    let idx = DiffPositionIndex::parse(patch);
    idx.lines
        .iter()
        .position(|p| matches!(p.side_and_line, Some((DiffSide::Right, 3))))
        .expect("the +let y = 2 line")
}

#[tokio::test]
async fn line_comment_success_path_records_post_line_comment_with_right_fields() {
    let (mut state, id, patch) = diff_state("oldsha");
    let target = position_at_added_line(&patch);
    state.active_tab_mut().state.diff_cursor = target;

    update(&mut state, &shift_v());
    update(&mut state, &key('c'));
    for ch in "looks wrong".chars() {
        update(&mut state, &key(ch));
    }
    let effects = update(&mut state, &ctrl_s());

    assert_eq!(effects.len(), 1);
    let (kind, head_sha, comment) = match &effects[0] {
        Effect::ExecuteWrite {
            kind,
            request: WriteRequest::PostLineComment { head_sha, comment },
            ..
        } => (kind.clone(), head_sha.clone(), comment.clone()),
        other => panic!("expected ExecuteWrite PostLineComment, got {other:?}"),
    };
    assert_eq!(
        kind,
        WriteKind::PostLineComment { id: id.clone() },
        "kind carries pr id"
    );
    assert_eq!(head_sha, "oldsha");
    assert_eq!(comment.path, "src/lib.rs");
    assert_eq!(comment.side, DiffSide::Right);
    assert_eq!(comment.line, 3);
    assert_eq!(comment.body, "looks wrong");

    let client = FakeGitHubClient::with_fixtures(FakeFixtures::default());
    client
        .post_line_comment(&id, &head_sha, comment.clone())
        .await
        .expect("fake accepts");
    let recorded = client.recorded_calls();
    let rec = recorded
        .iter()
        .find_map(|c| match c {
            RecordedCall::PostLineComment {
                id: rid,
                head_sha: hs,
                comment: rc,
            } => Some((rid, hs.as_str(), rc)),
            _ => None,
        })
        .expect("PostLineComment recorded");
    assert_eq!(rec.0, &id);
    assert_eq!(rec.1, "oldsha");
    assert_eq!(rec.2.path, "src/lib.rs");
    assert_eq!(rec.2.line, 3);
    assert_eq!(rec.2.body, "looks wrong");
}

#[tokio::test]
async fn line_comment_stale_sha_keeps_pending_write_for_recovery_and_annotation() {
    let (mut state, id, patch) = diff_state("oldsha");
    let target = position_at_added_line(&patch);
    state.active_tab_mut().state.diff_cursor = target;

    update(&mut state, &shift_v());
    update(&mut state, &key('c'));
    for ch in "needs work".chars() {
        update(&mut state, &key(ch));
    }
    let effects = update(&mut state, &ctrl_s());
    let (kind, request, version_at_submit) = match &effects[0] {
        Effect::ExecuteWrite {
            kind,
            request,
            version_at_submit,
        } => (kind.clone(), request.clone(), *version_at_submit),
        other => panic!("expected ExecuteWrite, got {other:?}"),
    };
    // Dispatcher (production) sets pending_write when it begins executing the
    // effect; reducer-only tests must seed it directly to model that state.
    state.pending_write = Some(gprr::app::state::PendingWrite {
        kind: kind.clone(),
        request: request.clone(),
        version_at_submit,
        status: PendingStatus::Submitting,
    });
    let pending = state.pending_write.clone().unwrap();

    let fixtures = FakeFixtures::default();
    *fixtures.next_error.lock().unwrap() = Some((
        WriteOp::PostLineComment,
        GitHubError::StaleHeadSha {
            expected: "newsha".into(),
        },
    ));
    let client = FakeGitHubClient::with_fixtures(fixtures);
    let head_sha = match &pending.request {
        WriteRequest::PostLineComment { head_sha, .. } => head_sha.clone(),
        other => panic!("pending request must be PostLineComment, got {other:?}"),
    };
    let comment = match &pending.request {
        WriteRequest::PostLineComment { comment, .. } => comment.clone(),
        _ => unreachable!(),
    };
    let result = client.post_line_comment(&id, &head_sha, comment).await;
    let err = result.expect_err("stale-sha injected");
    assert!(matches!(err, GitHubError::StaleHeadSha { ref expected } if expected == "newsha"));

    let refetch_effects = update(
        &mut state,
        &AppEvent::Data(DataEvent::WriteCompleted {
            kind: kind.clone(),
            result: Err(err),
        }),
    );
    assert!(
        refetch_effects.is_empty(),
        "stale-sha emits no effects; user must press R"
    );
    let pending_after = state
        .pending_write
        .as_ref()
        .expect("pending_write preserved on stale-sha");
    assert!(matches!(
        &pending_after.status,
        PendingStatus::Stale(s) if s == "newsha"
    ));

    let old_head = match &pending_after.request {
        WriteRequest::PostLineComment { head_sha, .. } => head_sha.as_str(),
        _ => unreachable!(),
    };
    assert_eq!(old_head, "oldsha", "old head_sha preserved for annotation");
    let new_head = match &pending_after.status {
        PendingStatus::Stale(s) => s.as_str(),
        _ => unreachable!(),
    };
    let annotation = format!(
        "Head moved from {old_head} to {new_head} \u{2014} press R to re-anchor on the latest diff and resubmit."
    );
    assert_eq!(
        annotation,
        "Head moved from oldsha to newsha \u{2014} press R to re-anchor on the latest diff and resubmit."
    );

    let body_preserved = match &pending_after.request {
        WriteRequest::PostLineComment { comment, .. } => comment.body.clone(),
        _ => unreachable!(),
    };
    assert_eq!(body_preserved, "needs work", "unsent body preserved");

    let _unused = Arc::new(Mutex::new(())); // keep imports honest
}
