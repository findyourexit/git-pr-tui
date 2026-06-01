//! Domain types shared across the data and UI layers.
//!
//! Every type in this module is part of the v1 contract: it mirrors the
//! GitHub data the rest of the crate depends on, so add or remove fields
//! deliberately.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// ─────────────────────────── enums ───────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrState {
    Open,
    Closed,
    Merged,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewDecision {
    Approved,
    ChangesRequested,
    ReviewRequired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewState {
    Pending,
    Commented,
    Approved,
    ChangesRequested,
    Dismissed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileStatus {
    Added,
    Removed,
    Modified,
    Renamed,
    Copied,
    ChangedMode,
    Unmerged,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiffSide {
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Queued,
    InProgress,
    Completed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckConclusion {
    Success,
    Failure,
    Neutral,
    Cancelled,
    Skipped,
    TimedOut,
    ActionRequired,
    Stale,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StatusState {
    Pending,
    Success,
    Failure,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChecksRollup {
    Pending,
    Success,
    Failure,
    Neutral,
    Mixed,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MergeMethod {
    Merge,
    Squash,
    Rebase,
}

/// GitHub `mergeStateStatus` — all 8 documented states.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mergeable {
    Clean,
    Unstable,
    HasHooks,
    Behind,
    Blocked,
    Dirty,
    Draft,
    Unknown,
}

// ─────────────────────────── identifiers ───────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Repo {
    pub owner: String,
    pub name: String,
}

impl Repo {
    #[must_use]
    pub fn full_name(&self) -> String {
        format!("{}/{}", self.owner, self.name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PrId {
    pub repo: Repo,
    pub number: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Label {
    pub name: String,
    pub color: String,
}

// ─────────────────────────── PR summary ───────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrSummary {
    pub id: PrId,
    pub title: String,
    pub author: String,
    pub state: PrState,
    pub is_draft: bool,
    pub base: String,
    pub head: String,
    pub additions: u32,
    pub deletions: u32,
    pub changed_files: u32,
    pub comments: u32,
    pub review_decision: Option<ReviewDecision>,
    pub checks: ChecksRollup,
    pub mergeable: Mergeable,
    pub labels: Vec<Label>,
    pub updated_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

// ─────────────────────────── PR detail ───────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrDetail {
    pub summary: PrSummary,
    pub body: String,
    pub body_html: Option<String>,
    pub requested_reviewers: Vec<String>,
    pub assignees: Vec<String>,
    /// Critical: write actions must echo this for stale-head detection.
    pub head_sha: String,
    pub merge_commit_sha: Option<String>,
    /// For diff-position mapping.
    pub base_ref_oid: String,
    /// Grouped, ordered, paginated to completion.
    pub review_threads: Vec<ReviewThread>,
    pub timeline: Vec<TimelineEvent>,
    /// Discovered from repo settings (merge/squash/rebase booleans).
    pub available_merge_methods: Vec<MergeMethod>,
    /// Monotonic per-resource, for stale-write detection.
    pub version: u64,
}

// ─────────────────────────── timeline ───────────────────────────
// v1: Commit, IssueComment, Review, Merged, Closed, Reopened, HeadRefForcePushed.
// v2 (deferred): labeled/unlabeled, assigned/unassigned, review_requested.

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TimelineEvent {
    Commit {
        sha: String,
        message: String,
        author: String,
        pushed_at: DateTime<Utc>,
    },
    IssueComment {
        id: String,
        author: String,
        body: String,
        created_at: DateTime<Utc>,
    },
    Review {
        id: String,
        author: String,
        state: ReviewState,
        body: String,
        submitted_at: DateTime<Utc>,
    },
    Merged {
        sha: String,
        by: String,
        at: DateTime<Utc>,
    },
    Closed {
        by: String,
        at: DateTime<Utc>,
    },
    Reopened {
        by: String,
        at: DateTime<Utc>,
    },
    HeadRefForcePushed {
        before: String,
        after: String,
        at: DateTime<Utc>,
    },
}

// ─────────────────────────── files ───────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrFiles {
    pub files: Vec<FileDiff>,
    /// True when GitHub's /files endpoint hit its 3000-file cap.
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileDiff {
    pub path: String,
    /// Present for renames and copies.
    pub previous_path: Option<String>,
    pub status: FileStatus,
    pub additions: u32,
    pub deletions: u32,
    /// None for binary, submodule, or oversized files.
    pub patch: Option<String>,
    pub is_binary: bool,
    pub is_submodule: bool,
    /// GitHub's linguist guess, when available.
    pub is_generated: Option<bool>,
    /// True when patch was omitted for size.
    pub oversize: bool,
    pub blob_sha_before: Option<String>,
    pub blob_sha_after: Option<String>,
}

// ─────────────────────────── review threads ───────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewThread {
    /// GraphQL node id, needed for replies.
    pub id: String,
    pub path: String,
    /// Resolved against the PR's `head_sha` at thread time.
    pub line_range: LineRange,
    pub is_resolved: bool,
    /// Line no longer exists in current head.
    pub is_outdated: bool,
    /// Includes the original + all replies, time-ordered.
    pub comments: Vec<ReviewComment>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewComment {
    pub id: String,
    pub author: String,
    pub body: String,
    pub created_at: DateTime<Utc>,
    /// Sha the comment was anchored against.
    pub commit_sha: String,
    /// Extracted fenced `suggestion` block, if any.
    pub suggestion: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct LineRange {
    /// Left (base) | Right (head).
    pub side: DiffSide,
    /// 1-based file-line numbers.
    pub start_line: u32,
    pub end_line: u32,
    /// Position within the diff hunk (GitHub's review-comment positioning).
    pub start_position: u32,
    pub end_position: u32,
}

// ─────────────────────────── checks ───────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrChecks {
    pub runs: Vec<CheckRun>,
    pub statuses: Vec<CommitStatus>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckRun {
    pub name: String,
    pub app: String,
    pub status: CheckStatus,
    pub conclusion: Option<CheckConclusion>,
    pub url: String,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitStatus {
    pub context: String,
    pub state: StatusState,
    pub target_url: Option<String>,
    pub description: Option<String>,
}

// ─────────────────────────── tests ───────────────────────────

#[cfg(test)]
mod enum_tests {
    use super::*;

    #[test]
    fn mergeable_covers_all_eight_states() {
        let all = [
            Mergeable::Clean,
            Mergeable::Unstable,
            Mergeable::HasHooks,
            Mergeable::Behind,
            Mergeable::Blocked,
            Mergeable::Dirty,
            Mergeable::Draft,
            Mergeable::Unknown,
        ];
        assert_eq!(all.len(), 8);
    }

    #[test]
    fn checks_rollup_covers_all_six_states() {
        let all = [
            ChecksRollup::Pending,
            ChecksRollup::Success,
            ChecksRollup::Failure,
            ChecksRollup::Neutral,
            ChecksRollup::Mixed,
            ChecksRollup::None,
        ];
        assert_eq!(all.len(), 6);
    }

    #[test]
    fn file_status_has_seven_variants() {
        let all = [
            FileStatus::Added,
            FileStatus::Removed,
            FileStatus::Modified,
            FileStatus::Renamed,
            FileStatus::Copied,
            FileStatus::ChangedMode,
            FileStatus::Unmerged,
        ];
        assert_eq!(all.len(), 7);
    }

    #[test]
    fn pr_state_round_trips() {
        let s = PrState::Merged;
        let j = serde_json::to_string(&s).unwrap();
        assert_eq!(j, "\"merged\"");
        let back: PrState = serde_json::from_str(&j).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn review_decision_serializes_optional_as_null() {
        let none: Option<ReviewDecision> = None;
        assert_eq!(serde_json::to_string(&none).unwrap(), "null");
        let some = Some(ReviewDecision::Approved);
        assert_eq!(serde_json::to_string(&some).unwrap(), "\"approved\"");
    }
}

#[cfg(test)]
mod id_tests {
    use super::*;

    #[test]
    fn pr_id_round_trips_json() {
        let id = PrId {
            repo: Repo {
                owner: "acme".into(),
                name: "widgets".into(),
            },
            number: 42,
        };
        let json = serde_json::to_string(&id).unwrap();
        let back: PrId = serde_json::from_str(&json).unwrap();
        assert_eq!(id, back);
    }

    #[test]
    fn pr_id_accepts_large_pr_numbers() {
        // GitHub's PR numbers grow without a documented u32 ceiling.
        let id = PrId {
            repo: Repo {
                owner: "a".into(),
                name: "b".into(),
            },
            number: 9_999_999_999,
        };
        let json = serde_json::to_string(&id).unwrap();
        assert!(json.contains("9999999999"));
    }

    #[test]
    fn repo_full_name_formats() {
        assert_eq!(
            Repo {
                owner: "a".into(),
                name: "b".into()
            }
            .full_name(),
            "a/b"
        );
    }
}

#[cfg(test)]
mod summary_detail_tests {
    use super::*;

    fn sample_summary() -> PrSummary {
        PrSummary {
            id: PrId {
                repo: Repo {
                    owner: "a".into(),
                    name: "b".into(),
                },
                number: 1,
            },
            title: "t".into(),
            author: "u".into(),
            state: PrState::Open,
            is_draft: false,
            base: "main".into(),
            head: "f".into(),
            additions: 1,
            deletions: 0,
            changed_files: 1,
            comments: 0,
            review_decision: None,
            checks: ChecksRollup::None,
            mergeable: Mergeable::Clean,
            labels: vec![],
            updated_at: chrono::Utc::now(),
            created_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn pr_summary_round_trips_json() {
        let s = sample_summary();
        let j = serde_json::to_string(&s).unwrap();
        let back: PrSummary = serde_json::from_str(&j).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn pr_detail_round_trips_with_optional_fields() {
        let d = PrDetail {
            summary: sample_summary(),
            body: "hello".into(),
            body_html: Some("<p>hello</p>".into()),
            requested_reviewers: vec!["alice".into()],
            assignees: vec!["bob".into()],
            head_sha: "abc123".into(),
            merge_commit_sha: None,
            base_ref_oid: "def456".into(),
            review_threads: vec![],
            timeline: vec![],
            available_merge_methods: vec![MergeMethod::Squash],
            version: 1,
        };
        let j = serde_json::to_string(&d).unwrap();
        let back: PrDetail = serde_json::from_str(&j).unwrap();
        assert_eq!(d, back);
    }

    #[test]
    fn timeline_commit_variant_round_trips() {
        let e = TimelineEvent::Commit {
            sha: "abc".into(),
            message: "fix".into(),
            author: "u".into(),
            pushed_at: chrono::Utc::now(),
        };
        let j = serde_json::to_string(&e).unwrap();
        let back: TimelineEvent = serde_json::from_str(&j).unwrap();
        assert_eq!(e, back);
    }

    #[test]
    fn timeline_force_push_round_trips() {
        let e = TimelineEvent::HeadRefForcePushed {
            before: "abc".into(),
            after: "def".into(),
            at: chrono::Utc::now(),
        };
        let j = serde_json::to_string(&e).unwrap();
        let back: TimelineEvent = serde_json::from_str(&j).unwrap();
        assert_eq!(e, back);
    }
}

#[cfg(test)]
mod files_tests {
    use super::*;

    #[test]
    fn line_range_round_trips_with_positions() {
        let r = LineRange {
            side: DiffSide::Right,
            start_line: 10,
            end_line: 14,
            start_position: 25,
            end_position: 29,
        };
        let j = serde_json::to_string(&r).unwrap();
        let back: LineRange = serde_json::from_str(&j).unwrap();
        assert_eq!(r, back);
    }

    #[test]
    fn file_diff_carries_full_metadata() {
        let f = FileDiff {
            path: "src/lib.rs".into(),
            previous_path: Some("src/old.rs".into()),
            status: FileStatus::Renamed,
            additions: 4,
            deletions: 2,
            patch: None,
            is_binary: false,
            is_submodule: false,
            is_generated: Some(false),
            oversize: false,
            blob_sha_before: Some("aaa".into()),
            blob_sha_after: Some("bbb".into()),
        };
        let j = serde_json::to_string(&f).unwrap();
        let back: FileDiff = serde_json::from_str(&j).unwrap();
        assert_eq!(f, back);
    }
}

#[cfg(test)]
mod thread_tests {
    use super::*;

    #[test]
    fn review_comment_round_trips_with_suggestion() {
        let c = ReviewComment {
            id: "n1".into(),
            author: "u".into(),
            body: "```suggestion\nx\n```".into(),
            commit_sha: "abc".into(),
            created_at: Utc::now(),
            suggestion: Some("x\n".into()),
        };
        let j = serde_json::to_string(&c).unwrap();
        let back: ReviewComment = serde_json::from_str(&j).unwrap();
        assert_eq!(c, back);
    }
}

#[cfg(test)]
mod check_tests {
    use super::*;

    #[test]
    fn check_run_includes_app_name() {
        let r = CheckRun {
            name: "test".into(),
            app: "github-actions".into(),
            status: CheckStatus::Completed,
            conclusion: Some(CheckConclusion::Success),
            url: "https://example.test/run/1".into(),
            started_at: None,
            completed_at: None,
        };
        let j = serde_json::to_string(&r).unwrap();
        let back: CheckRun = serde_json::from_str(&j).unwrap();
        assert_eq!(r, back);
    }
}
