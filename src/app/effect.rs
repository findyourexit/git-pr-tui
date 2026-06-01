use crate::data::cache::WriteKind;
use crate::data::github::{DashboardBucket, GitHubError, NewLineComment, RateLimitSnapshot};
use crate::data::models::{
    MergeMethod, PrChecks, PrDetail, PrFiles, PrId, PrSummary, Repo, ReviewState,
};

/// One dashboard bucket's rows plus that bucket's next-page cursor
/// (`None` once the bucket is fully paged).
pub type DashboardBucketPage = (DashboardBucket, Vec<PrSummary>, Option<String>);

/// Data events flowing back from the data layer into the reducer.
#[derive(Debug, Clone)]
pub enum DataEvent {
    /// Per-bucket rows plus that bucket's next-page cursor (`None` at end).
    /// A full dashboard fetch returns every bucket; a bucket-scoped "load
    /// more" returns a single bucket whose rows are appended.
    DashboardLoaded(Result<Vec<DashboardBucketPage>, GitHubError>),
    PrListLoaded {
        repo: Repo,
        result: Result<Vec<PrSummary>, GitHubError>,
        next_cursor: Option<String>,
        /// `true` when this is a fresh page-1 fetch (initial load or refresh)
        /// whose rows replace the cached list; `false` when paginating, whose
        /// rows are appended. Prevents the refresh-duplicates-rows bug.
        replace: bool,
    },
    PrDetailLoaded {
        id: PrId,
        result: Box<Result<PrDetail, GitHubError>>,
    },
    PrFilesLoaded {
        id: PrId,
        head_sha: String,
        result: Box<Result<PrFiles, GitHubError>>,
    },
    PrDiffLoaded {
        id: PrId,
        head_sha: String,
        result: Result<String, GitHubError>,
    },
    PrChecksLoaded {
        id: PrId,
        result: Result<PrChecks, GitHubError>,
    },
    PrFileDiffLoaded {
        id: PrId,
        head_sha: String,
        path: String,
        result: Box<Result<crate::data::models::FileDiff, GitHubError>>,
    },
    WriteCompleted {
        kind: WriteKind,
        result: Result<(), GitHubError>,
    },
    CheckoutCompleted {
        id: PrId,
        result: Result<String, String>,
    },
    RateLimitUpdate(RateLimitSnapshot),
}

#[derive(Debug, Clone)]
pub enum Effect {
    FetchDashboard {
        bucket: Option<DashboardBucket>,
        cursor: Option<String>,
    },
    FetchPrList {
        repo: Repo,
        cursor: Option<String>,
        filter: Option<crate::data::github::PrListFilter>,
        sort: Option<crate::data::github::PrListSort>,
    },
    FetchPrDetail {
        id: PrId,
    },
    FetchPrFiles {
        id: PrId,
        head_sha: String,
    },
    FetchPrFileDiff {
        id: PrId,
        head_sha: String,
        path: String,
    },
    FetchPrDiff {
        id: PrId,
        head_sha: String,
    },
    FetchPrChecks {
        id: PrId,
    },
    ExecuteWrite {
        kind: WriteKind,
        request: WriteRequest,
        version_at_submit: u64,
    },
    /// Re-fetch the resources invalidated by a completed write. Expanded into
    /// concrete `Fetch*` effects by the run loop, which has read access to
    /// `AppState` (see `crate::app::update::refetch_effects`).
    Refetch {
        kind: WriteKind,
    },
    CheckoutBranch {
        id: PrId,
    },
    /// Open a URL in the user's default browser (best-effort, no result event).
    OpenInBrowser {
        url: String,
    },
}

#[derive(Debug, Clone)]
pub enum WriteRequest {
    PostPrComment {
        body: String,
    },
    SubmitReview {
        state: ReviewState,
        body: String,
        line_comments: Vec<NewLineComment>,
        head_sha: String,
    },
    PostLineComment {
        head_sha: String,
        comment: NewLineComment,
    },
    ReplyToThread {
        thread_id: String,
        body: String,
    },
    Merge {
        method: MergeMethod,
        head_sha: String,
    },
    Close,
    Reopen,
}
