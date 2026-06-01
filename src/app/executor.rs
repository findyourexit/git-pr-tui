//! Effect executor: spawns one `tokio::task` per [`Effect`] and routes the
//! resulting [`DataEvent`] back over an mpsc sender. Pure side-effect surface
//! between the reducer and the data layer.

use tokio::sync::mpsc::UnboundedSender;

use crate::app::effect::{DataEvent, Effect, WriteRequest};
use crate::data::SharedGitHubClient;

/// Dispatch a single [`Effect`] by spawning a `tokio::task`. The spawned task
/// awaits the matching `GitHubClient` call (or `git::fetch_pr_and_checkout` for
/// `CheckoutBranch`, via `spawn_blocking`) and sends one `DataEvent` back over
/// `tx`. Returns immediately. `Refetch` (expanded by the run loop) and
/// `OpenInBrowser` produce no `DataEvent`.
pub fn execute_effect(effect: Effect, client: SharedGitHubClient, tx: UnboundedSender<DataEvent>) {
    // `Refetch` is expanded by the run loop (it needs `AppState`); it never
    // reaches here as a standalone fetch, but the arm keeps the match total.
    if matches!(effect, Effect::Refetch { .. }) {
        return;
    }
    tokio::spawn(async move {
        let event: Option<DataEvent> = run_effect(effect, &client).await;
        if let Some(ev) = event {
            let _ = tx.send(ev);
        }
        // Surface the freshest rate-limit snapshot the client captured while
        // serving the request above. `None` until the data layer records one.
        if let Some(rl) = client.rate_limit() {
            let _ = tx.send(DataEvent::RateLimitUpdate(rl));
        }
    });
}

#[allow(clippy::too_many_lines)]
async fn run_effect(effect: Effect, client: &SharedGitHubClient) -> Option<DataEvent> {
    match effect {
        Effect::FetchDashboard { bucket, cursor } => {
            let request = crate::data::github::DashboardRequest { bucket, cursor };
            Some(DataEvent::DashboardLoaded(client.dashboard(request).await))
        }
        Effect::FetchPrList {
            repo,
            cursor,
            filter,
            sort,
        } => {
            // A fetch with no cursor is a fresh page 1 (initial load or
            // refresh) whose rows replace the cache; a cursored fetch is
            // pagination whose rows are appended.
            let replace = cursor.is_none();
            Some(match client.pr_list(&repo, cursor, filter, sort).await {
                Ok((rows, next_cursor)) => DataEvent::PrListLoaded {
                    repo,
                    result: Ok(rows),
                    next_cursor,
                    replace,
                },
                Err(e) => DataEvent::PrListLoaded {
                    repo,
                    result: Err(e),
                    next_cursor: None,
                    replace,
                },
            })
        }
        Effect::FetchPrDetail { id } => {
            let result = client.pr_detail(&id).await;
            Some(DataEvent::PrDetailLoaded {
                id,
                result: Box::new(result),
            })
        }
        Effect::FetchPrFiles { id, head_sha } => {
            let result = client.pr_files(&id, &head_sha).await;
            Some(DataEvent::PrFilesLoaded {
                id,
                head_sha,
                result: Box::new(result),
            })
        }
        Effect::FetchPrFileDiff { id, head_sha, path } => {
            let result = client.pr_file_diff(&id, &head_sha, &path).await;
            Some(DataEvent::PrFileDiffLoaded {
                id,
                head_sha,
                path,
                result: Box::new(result),
            })
        }
        Effect::FetchPrDiff { id, head_sha } => {
            let result = client.pr_diff(&id, &head_sha).await;
            Some(DataEvent::PrDiffLoaded {
                id,
                head_sha,
                result,
            })
        }
        Effect::FetchPrChecks { id } => {
            let result = client.pr_checks(&id).await;
            Some(DataEvent::PrChecksLoaded { id, result })
        }
        Effect::ExecuteWrite {
            kind,
            request,
            version_at_submit: _,
        } => {
            let result = execute_write(client.clone(), &request, &kind).await;
            Some(DataEvent::WriteCompleted { kind, result })
        }
        Effect::CheckoutBranch { id } => {
            let number = u32::try_from(id.number).unwrap_or(0);
            let result = tokio::task::spawn_blocking(move || {
                let cwd = std::env::current_dir().map_err(|e| format!("cwd unavailable: {e}"))?;
                crate::data::git::fetch_pr_and_checkout(&cwd, number).map_err(|e| e.to_string())
            })
            .await
            .unwrap_or_else(|e| Err(format!("join error: {e}")));
            Some(DataEvent::CheckoutCompleted { id, result })
        }
        Effect::OpenInBrowser { url } => {
            // Fire-and-forget; the spawned opener detaches immediately.
            let _ = std::process::Command::new(browser_opener())
                .arg(&url)
                .spawn();
            None
        }
        Effect::Refetch { .. } => None,
    }
}

/// Platform command that opens a URL in the default browser. Windows is an
/// explicit non-goal, so only macOS and the freedesktop `xdg-open` are wired.
fn browser_opener() -> &'static str {
    if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    }
}

async fn execute_write(
    client: SharedGitHubClient,
    request: &WriteRequest,
    kind: &crate::data::cache::WriteKind,
) -> Result<(), crate::data::github::GitHubError> {
    use crate::data::cache::WriteKind;
    let pr_id = match kind {
        WriteKind::SubmitReview { id }
        | WriteKind::PostPrComment { id }
        | WriteKind::PostLineComment { id }
        | WriteKind::ReplyToThread { id }
        | WriteKind::Merge { id }
        | WriteKind::Close { id }
        | WriteKind::Reopen { id } => id.clone(),
    };
    match request.clone() {
        WriteRequest::PostPrComment { body } => client.post_pr_comment(&pr_id, body).await,
        WriteRequest::SubmitReview {
            state,
            body,
            line_comments,
            head_sha,
        } => {
            let req = crate::data::github::SubmitReviewRequest {
                id: pr_id,
                head_sha,
                state,
                body,
                line_comments,
            };
            client.submit_review(req).await
        }
        WriteRequest::PostLineComment { head_sha, comment } => {
            client.post_line_comment(&pr_id, &head_sha, comment).await
        }
        WriteRequest::ReplyToThread { thread_id, body } => {
            client.reply_to_thread(&pr_id, &thread_id, body).await
        }
        WriteRequest::Merge { method, head_sha } => client.merge(&pr_id, method, &head_sha).await,
        WriteRequest::Close => client.close(&pr_id).await,
        WriteRequest::Reopen => client.reopen(&pr_id).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::github::DashboardBucket;
    use crate::data::github::fake::{FakeFixtures, FakeGitHubClient};
    use crate::data::models::{ChecksRollup, Mergeable, PrId, PrState, PrSummary, Repo};
    use std::sync::Arc;
    use tokio::sync::mpsc::unbounded_channel;

    fn sample_pr(repo: &Repo, number: u64) -> PrSummary {
        PrSummary {
            id: PrId {
                repo: repo.clone(),
                number,
            },
            title: format!("PR #{number}"),
            author: "alice".into(),
            state: PrState::Open,
            is_draft: false,
            base: "main".into(),
            head: "feature".into(),
            additions: 0,
            deletions: 0,
            changed_files: 0,
            comments: 0,
            review_decision: None,
            checks: ChecksRollup::None,
            mergeable: Mergeable::Unknown,
            labels: vec![],
            updated_at: chrono::Utc::now(),
            created_at: chrono::Utc::now(),
        }
    }

    #[tokio::test]
    async fn fetch_dashboard_emits_dashboard_loaded() {
        let repo = Repo {
            owner: "o".into(),
            name: "n".into(),
        };
        let pr = sample_pr(&repo, 1);
        let fixtures = FakeFixtures {
            dashboard: vec![(DashboardBucket::ReviewRequested, vec![pr.clone()])],
            ..FakeFixtures::default()
        };
        let client: SharedGitHubClient = Arc::new(FakeGitHubClient::with_fixtures(fixtures));
        let (tx, mut rx) = unbounded_channel();

        execute_effect(
            Effect::FetchDashboard {
                bucket: None,
                cursor: None,
            },
            client,
            tx,
        );

        let event = rx.recv().await.expect("event must arrive");
        match event {
            DataEvent::DashboardLoaded(Ok(rows)) => {
                assert_eq!(rows.len(), 1);
                assert_eq!(rows[0].0, DashboardBucket::ReviewRequested);
                assert_eq!(rows[0].1.len(), 1);
                assert_eq!(rows[0].1[0].id.number, 1);
            }
            other => panic!("expected DashboardLoaded(Ok), got {other:?}"),
        }
    }

    #[tokio::test]
    async fn fetch_pr_list_emits_pr_list_loaded() {
        let repo = Repo {
            owner: "o".into(),
            name: "n".into(),
        };
        let pr = sample_pr(&repo, 7);
        let fixtures = FakeFixtures {
            pr_list: vec![(repo.clone(), vec![pr.clone()])],
            ..FakeFixtures::default()
        };
        let client: SharedGitHubClient = Arc::new(FakeGitHubClient::with_fixtures(fixtures));
        let (tx, mut rx) = unbounded_channel();

        execute_effect(
            Effect::FetchPrList {
                repo: repo.clone(),
                cursor: None,
                filter: None,
                sort: None,
            },
            client,
            tx,
        );

        let event = rx.recv().await.expect("event must arrive");
        match event {
            DataEvent::PrListLoaded {
                repo: r,
                result: Ok(rows),
                ..
            } => {
                assert_eq!(r, repo);
                assert_eq!(rows.len(), 1);
                assert_eq!(rows[0].id.number, 7);
            }
            other => panic!("expected PrListLoaded(Ok), got {other:?}"),
        }
    }

    #[tokio::test]
    async fn refetch_produces_no_event() {
        use std::time::Duration;
        let repo = Repo {
            owner: "o".into(),
            name: "n".into(),
        };
        let pr_id = PrId {
            repo: repo.clone(),
            number: 1,
        };
        let fixtures = FakeFixtures::default();
        let client: SharedGitHubClient = Arc::new(FakeGitHubClient::with_fixtures(fixtures));
        let (tx, mut rx) = unbounded_channel();

        // `Refetch` returns early (no spawned task) — the run loop expands it,
        // not the executor. The sender drops → channel closes → recv() = None.
        execute_effect(
            Effect::Refetch {
                kind: crate::data::cache::WriteKind::PostPrComment { id: pr_id },
            },
            client,
            tx,
        );

        let outcome = tokio::time::timeout(Duration::from_millis(50), rx.recv()).await;
        match outcome {
            Err(_) | Ok(None) => {}
            Ok(Some(e)) => panic!("unexpected DataEvent from Refetch: {e:?}"),
        }
    }
}
