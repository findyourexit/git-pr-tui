use async_trait::async_trait;

use crate::data::models::{
    DiffSide, MergeMethod, PrChecks, PrDetail, PrFiles, PrId, PrSummary, Repo, ReviewState,
};

pub mod diff_position;
pub mod fake;
pub mod octocrab_impl;
pub mod parse;

#[derive(Debug, thiserror::Error, Clone)]
pub enum GitHubError {
    #[error("network: {0}")]
    Network(String),
    #[error("auth (401)")]
    Unauthorized,
    #[error("forbidden / scope (403): {0}")]
    Forbidden(String),
    #[error("not found (404)")]
    NotFound,
    #[error("validation (422): {0}")]
    Validation(String),
    /// 422 with the head-sha mismatch message; `expected` is what the server has now.
    #[error("stale head sha (422): expected {expected}")]
    StaleHeadSha { expected: String },
    #[error("rate limit exhausted; resets at {0}")]
    RateLimited(chrono::DateTime<chrono::Utc>),
    #[error("conflict / stale (409): {0}")]
    Conflict(String),
    #[error("server error ({0})")]
    Server(u16),
    #[error("parse: {0}")]
    Parse(String),
    #[error("timeout after {0:?}")]
    Timeout(std::time::Duration),
}

#[derive(Debug, Clone)]
pub struct RateLimitSnapshot {
    pub remaining: u32,
    pub limit: u32,
    pub resets_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DashboardBucket {
    ReviewRequested,
    Authored,
    Assigned,
}

/// Request shape for `dashboard()`. `bucket = None` fetches the first page of
/// every bucket; `bucket = Some(b)` fetches the next page of bucket `b` using
/// the provided `cursor` (which MUST originate from that same bucket's prior
/// response). Lazy "load more" hint, tracked per bucket.
#[derive(Debug, Clone, Default)]
pub struct DashboardRequest {
    pub bucket: Option<DashboardBucket>,
    pub cursor: Option<String>,
}

/// Filter applied to a PR list fetch. Each variant corresponds to one entry
/// in the filter modal. `None` (i.e. `Option::None`)
/// = no filter, default behavior. The data layer applies these client-side
/// in v0.1.0; server-side push-down is a v0.2 concern.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrListFilter {
    /// `is:open` — only `PrState::Open`.
    Open,
    /// `is:closed` — only `PrState::Closed`.
    Closed,
    /// `is:merged` — only `PrState::Merged`.
    Merged,
    /// `author:@me` — only PRs whose author matches the viewer login.
    /// The viewer login is supplied by the caller (reducer) at filter time.
    AuthoredByMe,
    /// `draft:false` — exclude draft PRs.
    NotDraft,
}

/// Sort applied to a PR list fetch. Each variant corresponds to one entry
/// in the sort modal. `None` (i.e. `Option::None`)
/// = default sort, which matches the GraphQL `pullRequests(orderBy:
/// UPDATED_AT DESC)` ordering already baked into the query. The data layer
/// applies these client-side in v0.1.0; server-side push-down via
/// `orderBy` variables is a v0.2 concern.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrListSort {
    /// `sort:updated-desc` — most recently updated first.
    UpdatedDesc,
    /// `sort:updated-asc` — least recently updated first.
    UpdatedAsc,
    /// `sort:created-desc` — most recently created first.
    CreatedDesc,
    /// `sort:comments-desc` — most-commented first.
    CommentsDesc,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewLineComment {
    pub path: String,
    pub side: DiffSide,
    pub start_line: Option<u32>,
    pub start_side: Option<DiffSide>,
    pub line: u32,
    pub body: String,
}

/// Arguments to `submit_review`. Replaces a 6-positional-arg method that
/// previously triggered `clippy::too_many_arguments`. Mirrors the eventual
/// reducer-side `Effect::SubmitReview` shape so call sites match 1:1.
#[derive(Debug, Clone)]
pub struct SubmitReviewRequest {
    pub id: PrId,
    pub head_sha: String,
    pub state: ReviewState,
    pub body: String,
    pub line_comments: Vec<NewLineComment>,
}

#[async_trait]
pub trait GitHubClient: Send + Sync + 'static {
    async fn viewer_login(&self) -> Result<String, GitHubError>;
    async fn auth_scopes(&self) -> Result<Vec<String>, GitHubError>;

    async fn dashboard(
        &self,
        request: DashboardRequest,
    ) -> Result<Vec<(DashboardBucket, Vec<PrSummary>, Option<String>)>, GitHubError>;

    async fn pr_list(
        &self,
        repo: &Repo,
        cursor: Option<String>,
        filter: Option<PrListFilter>,
        sort: Option<PrListSort>,
    ) -> Result<(Vec<PrSummary>, Option<String>), GitHubError>;

    async fn pr_detail(&self, id: &PrId) -> Result<PrDetail, GitHubError>;
    async fn pr_files(&self, id: &PrId, head_sha: &str) -> Result<PrFiles, GitHubError>;
    async fn pr_file_diff(
        &self,
        id: &PrId,
        head_sha: &str,
        path: &str,
    ) -> Result<crate::data::models::FileDiff, GitHubError>;
    async fn pr_diff(&self, id: &PrId, head_sha: &str) -> Result<String, GitHubError>;
    async fn pr_checks(&self, id: &PrId) -> Result<PrChecks, GitHubError>;

    async fn submit_review(&self, request: SubmitReviewRequest) -> Result<(), GitHubError>;

    async fn post_pr_comment(&self, id: &PrId, body: String) -> Result<(), GitHubError>;
    async fn post_line_comment(
        &self,
        id: &PrId,
        head_sha: &str,
        comment: NewLineComment,
    ) -> Result<(), GitHubError>;
    async fn reply_to_thread(
        &self,
        id: &PrId,
        thread_id: &str,
        body: String,
    ) -> Result<(), GitHubError>;

    async fn merge(
        &self,
        id: &PrId,
        method: MergeMethod,
        head_sha: &str,
    ) -> Result<(), GitHubError>;
    async fn close(&self, id: &PrId) -> Result<(), GitHubError>;
    async fn reopen(&self, id: &PrId) -> Result<(), GitHubError>;

    fn rate_limit(&self) -> Option<RateLimitSnapshot>;
}
