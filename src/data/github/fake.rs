//! In-memory `GitHubClient` for tests and offline development.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use super::{
    DashboardBucket, DashboardRequest, GitHubClient, GitHubError, NewLineComment,
    RateLimitSnapshot, SubmitReviewRequest,
};
use crate::data::models::{MergeMethod, PrChecks, PrDetail, PrFiles, PrId, PrSummary, Repo};

/// Page size for the fake `pr_list` paginator. Cursor format = next-page offset as decimal string.
const FAKE_PR_LIST_PAGE_SIZE: usize = 20;

#[derive(Default, Debug, Clone)]
pub struct FakeFixtures {
    pub viewer: String,
    pub scopes: Vec<String>,
    pub dashboard: Vec<(DashboardBucket, Vec<PrSummary>)>,
    pub pr_list: Vec<(Repo, Vec<PrSummary>)>,
    pub pr_detail: Vec<(PrId, PrDetail)>,
    pub pr_files: Vec<((PrId, String), PrFiles)>,
    pub pr_file_diffs: Vec<((PrId, String, String), crate::data::models::FileDiff)>,
    pub pr_diff: Vec<((PrId, String), String)>,
    pub pr_checks: Vec<(PrId, PrChecks)>,
    pub rate_limit: Option<RateLimitSnapshot>,
    /// If set, the next call to the matching method returns this error once.
    /// Wrapped in `Arc<Mutex<_>>` so the struct stays `Clone` and shared
    /// references to the same fixtures see consistent error-injection state.
    pub next_error: Arc<Mutex<Option<(WriteOp, GitHubError)>>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteOp {
    SubmitReview,
    PostPrComment,
    PostLineComment,
    ReplyToThread,
    Merge,
    Close,
    Reopen,
}

#[derive(Debug, Clone)]
pub enum RecordedCall {
    SubmitReview(SubmitReviewRequest),
    PostPrComment {
        id: PrId,
        body: String,
    },
    PostLineComment {
        id: PrId,
        head_sha: String,
        comment: NewLineComment,
    },
    ReplyToThread {
        id: PrId,
        thread_id: String,
        body: String,
    },
    Merge {
        id: PrId,
        method: MergeMethod,
        head_sha: String,
    },
    Close {
        id: PrId,
    },
    Reopen {
        id: PrId,
    },
}

#[derive(Debug)]
pub struct FakeGitHubClient {
    fixtures: FakeFixtures,
    calls: Mutex<Vec<RecordedCall>>,
}

impl FakeGitHubClient {
    #[must_use]
    pub fn with_fixtures(fixtures: FakeFixtures) -> Self {
        Self {
            fixtures,
            calls: Mutex::new(Vec::new()),
        }
    }
    #[must_use]
    pub fn recorded_calls(&self) -> Vec<RecordedCall> {
        self.calls.lock().unwrap().clone()
    }
    fn take_next_error(&self, op: &WriteOp) -> Option<GitHubError> {
        let mut slot = self.fixtures.next_error.lock().unwrap();
        if let Some((slot_op, _)) = slot.as_ref()
            && slot_op == op
        {
            return slot.take().map(|(_, e)| e);
        }
        None
    }
}

#[async_trait]
impl GitHubClient for FakeGitHubClient {
    async fn viewer_login(&self) -> Result<String, GitHubError> {
        Ok(self.fixtures.viewer.clone())
    }
    async fn auth_scopes(&self) -> Result<Vec<String>, GitHubError> {
        Ok(self.fixtures.scopes.clone())
    }
    async fn dashboard(
        &self,
        request: DashboardRequest,
    ) -> Result<Vec<(DashboardBucket, Vec<PrSummary>, Option<String>)>, GitHubError> {
        let buckets = self.fixtures.dashboard.iter().filter(|(b, _)| {
            request
                .bucket
                .as_ref()
                .is_none_or(|requested| requested == b)
        });
        Ok(buckets.map(|(b, v)| (*b, v.clone(), None)).collect())
    }
    async fn pr_list(
        &self,
        repo: &Repo,
        cursor: Option<String>,
        filter: Option<crate::data::github::PrListFilter>,
        sort: Option<crate::data::github::PrListSort>,
    ) -> Result<(Vec<PrSummary>, Option<String>), GitHubError> {
        let list = self
            .fixtures
            .pr_list
            .iter()
            .find(|(r, _)| r == repo)
            .map(|(_, v)| v.clone())
            .ok_or(GitHubError::NotFound)?;
        let filtered = match filter {
            None => list,
            Some(f) => list
                .into_iter()
                .filter(|pr| apply_pr_list_filter(f, pr, &self.fixtures.viewer))
                .collect(),
        };
        let sorted = apply_pr_list_sort(sort, filtered);

        let offset: usize = cursor.as_deref().and_then(|c| c.parse().ok()).unwrap_or(0);
        let end = (offset + FAKE_PR_LIST_PAGE_SIZE).min(sorted.len());
        let page: Vec<PrSummary> = sorted[offset.min(sorted.len())..end].to_vec();
        let next = if end < sorted.len() {
            Some(end.to_string())
        } else {
            None
        };
        Ok((page, next))
    }
    async fn pr_detail(&self, id: &PrId) -> Result<PrDetail, GitHubError> {
        self.fixtures
            .pr_detail
            .iter()
            .find(|(p, _)| p == id)
            .map(|(_, d)| d.clone())
            .ok_or(GitHubError::NotFound)
    }
    async fn pr_files(&self, id: &PrId, head_sha: &str) -> Result<PrFiles, GitHubError> {
        self.fixtures
            .pr_files
            .iter()
            .find(|((p, s), _)| p == id && s == head_sha)
            .map(|(_, f)| f.clone())
            .ok_or(GitHubError::NotFound)
    }
    async fn pr_file_diff(
        &self,
        id: &PrId,
        head_sha: &str,
        path: &str,
    ) -> Result<crate::data::models::FileDiff, GitHubError> {
        self.fixtures
            .pr_file_diffs
            .iter()
            .find(|((p, s, fp), _)| p == id && s == head_sha && fp == path)
            .map(|(_, fd)| fd.clone())
            .ok_or(GitHubError::NotFound)
    }
    async fn pr_diff(&self, id: &PrId, head_sha: &str) -> Result<String, GitHubError> {
        self.fixtures
            .pr_diff
            .iter()
            .find(|((p, s), _)| p == id && s == head_sha)
            .map(|(_, d)| d.clone())
            .ok_or(GitHubError::NotFound)
    }
    async fn pr_checks(&self, id: &PrId) -> Result<PrChecks, GitHubError> {
        self.fixtures
            .pr_checks
            .iter()
            .find(|(p, _)| p == id)
            .map(|(_, c)| c.clone())
            .ok_or(GitHubError::NotFound)
    }

    async fn submit_review(&self, request: SubmitReviewRequest) -> Result<(), GitHubError> {
        if let Some(e) = self.take_next_error(&WriteOp::SubmitReview) {
            return Err(e);
        }
        self.calls
            .lock()
            .unwrap()
            .push(RecordedCall::SubmitReview(request));
        Ok(())
    }
    async fn post_pr_comment(&self, id: &PrId, body: String) -> Result<(), GitHubError> {
        if let Some(e) = self.take_next_error(&WriteOp::PostPrComment) {
            return Err(e);
        }
        self.calls
            .lock()
            .unwrap()
            .push(RecordedCall::PostPrComment {
                id: id.clone(),
                body,
            });
        Ok(())
    }
    async fn post_line_comment(
        &self,
        id: &PrId,
        head_sha: &str,
        comment: NewLineComment,
    ) -> Result<(), GitHubError> {
        if let Some(e) = self.take_next_error(&WriteOp::PostLineComment) {
            return Err(e);
        }
        self.calls
            .lock()
            .unwrap()
            .push(RecordedCall::PostLineComment {
                id: id.clone(),
                head_sha: head_sha.into(),
                comment,
            });
        Ok(())
    }
    async fn reply_to_thread(
        &self,
        id: &PrId,
        thread_id: &str,
        body: String,
    ) -> Result<(), GitHubError> {
        if let Some(e) = self.take_next_error(&WriteOp::ReplyToThread) {
            return Err(e);
        }
        self.calls
            .lock()
            .unwrap()
            .push(RecordedCall::ReplyToThread {
                id: id.clone(),
                thread_id: thread_id.into(),
                body,
            });
        Ok(())
    }
    async fn merge(
        &self,
        id: &PrId,
        method: MergeMethod,
        head_sha: &str,
    ) -> Result<(), GitHubError> {
        if let Some(e) = self.take_next_error(&WriteOp::Merge) {
            return Err(e);
        }
        self.calls.lock().unwrap().push(RecordedCall::Merge {
            id: id.clone(),
            method,
            head_sha: head_sha.into(),
        });
        Ok(())
    }
    async fn close(&self, id: &PrId) -> Result<(), GitHubError> {
        if let Some(e) = self.take_next_error(&WriteOp::Close) {
            return Err(e);
        }
        self.calls
            .lock()
            .unwrap()
            .push(RecordedCall::Close { id: id.clone() });
        Ok(())
    }
    async fn reopen(&self, id: &PrId) -> Result<(), GitHubError> {
        if let Some(e) = self.take_next_error(&WriteOp::Reopen) {
            return Err(e);
        }
        self.calls
            .lock()
            .unwrap()
            .push(RecordedCall::Reopen { id: id.clone() });
        Ok(())
    }
    fn rate_limit(&self) -> Option<RateLimitSnapshot> {
        self.fixtures.rate_limit.clone()
    }
}

fn apply_pr_list_filter(
    filter: crate::data::github::PrListFilter,
    pr: &PrSummary,
    viewer: &str,
) -> bool {
    use crate::data::github::PrListFilter;
    use crate::data::models::PrState;
    match filter {
        PrListFilter::Open => matches!(pr.state, PrState::Open),
        PrListFilter::Closed => matches!(pr.state, PrState::Closed),
        PrListFilter::Merged => matches!(pr.state, PrState::Merged),
        PrListFilter::AuthoredByMe => pr.author == viewer,
        PrListFilter::NotDraft => !pr.is_draft,
    }
}

fn apply_pr_list_sort(
    sort: Option<crate::data::github::PrListSort>,
    mut prs: Vec<PrSummary>,
) -> Vec<PrSummary> {
    use crate::data::github::PrListSort;
    let Some(sort) = sort else {
        return prs;
    };
    match sort {
        PrListSort::UpdatedDesc => prs.sort_by_key(|pr| std::cmp::Reverse(pr.updated_at)),
        PrListSort::UpdatedAsc => prs.sort_by_key(|pr| pr.updated_at),
        PrListSort::CreatedDesc => prs.sort_by_key(|pr| std::cmp::Reverse(pr.created_at)),
        PrListSort::CommentsDesc => prs.sort_by_key(|pr| std::cmp::Reverse(pr.comments)),
    }
    prs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_fixtures() -> FakeFixtures {
        FakeFixtures {
            viewer: "octocat".into(),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn viewer_returns_fixture_login() {
        let client = FakeGitHubClient::with_fixtures(empty_fixtures());
        assert_eq!(client.viewer_login().await.unwrap(), "octocat");
    }

    #[tokio::test]
    async fn close_is_recorded() {
        let client = FakeGitHubClient::with_fixtures(empty_fixtures());
        let id = PrId {
            repo: Repo {
                owner: "a".into(),
                name: "b".into(),
            },
            number: 1,
        };
        client.close(&id).await.unwrap();
        let calls = client.recorded_calls();
        assert_eq!(calls.len(), 1);
        assert!(matches!(&calls[0], RecordedCall::Close { id: cid } if cid == &id));
    }

    #[tokio::test]
    async fn injected_error_fires_once_then_clears() {
        let fx = empty_fixtures();
        *fx.next_error.lock().unwrap() = Some((WriteOp::Close, GitHubError::Unauthorized));
        let client = FakeGitHubClient::with_fixtures(fx);
        let id = PrId {
            repo: Repo {
                owner: "a".into(),
                name: "b".into(),
            },
            number: 1,
        };
        assert!(matches!(
            client.close(&id).await,
            Err(GitHubError::Unauthorized)
        ));
        // Second call succeeds and gets recorded.
        client.close(&id).await.unwrap();
        assert_eq!(client.recorded_calls().len(), 1);
    }
}
