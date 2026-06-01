//! Parser for the `PrDetail` GraphQL query.
//!
//! The query paginates over four collections — review threads, commits,
//! timeline items, and per-thread review comments. This module decodes a
//! single page of any of them; the eager-pagination loop that stitches
//! pages together lives in `octocrab_impl` (methods on `OctocrabGitHubClient`).
//!
//! Issue comments are not a separate collection: they flow through
//! `timelineItems` as `IssueComment` fragments alongside reviews, commits,
//! and state-change events.

use chrono::{DateTime, Utc};

use super::pr_summary;
use crate::data::github::GitHubError;
use crate::data::models::{
    DiffSide, LineRange, MergeMethod, PrSummary, ReviewComment, ReviewState, ReviewThread,
    TimelineEvent,
};

/// First page of a `PrDetail` response. The eager-pagination loop in
/// `octocrab_impl` walks the embedded cursors until they are all `None`,
/// then assembles the final `PrDetail`.
#[derive(Debug)]
pub struct PrDetailPage {
    pub summary: PrSummary,
    pub body: String,
    pub body_html: Option<String>,
    pub requested_reviewers: Vec<String>,
    pub assignees: Vec<String>,
    pub head_sha: String,
    pub merge_commit_sha: Option<String>,
    pub base_ref_oid: String,
    pub available_merge_methods: Vec<MergeMethod>,
    pub review_threads_page: PagedThreads,
    pub commits_page: PagedCommits,
    pub timeline_page: PagedTimeline,
}

#[derive(Debug, Default)]
pub struct PagedThreads {
    pub threads: Vec<ReviewThread>,
    pub cursor: Option<String>,
    /// `(thread_id, next_comments_cursor)` for any thread whose
    /// `comments.pageInfo.hasNextPage` is `true` on this page.
    pub thread_comment_continuations: Vec<(String, String)>,
}

/// `commits` collection — every node is a `TimelineEvent::Commit`.
#[derive(Debug, Default)]
pub struct PagedCommits {
    pub commits: Vec<TimelineEvent>,
    pub cursor: Option<String>,
}

/// `timelineItems` collection — every variant *except* `Commit` (which is
/// pulled from the separate `commits` block to keep authoredDate/pushedDate
/// available; `PullRequestCommit` nodes here are skipped to avoid double-
/// counting).
#[derive(Debug, Default)]
pub struct PagedTimeline {
    pub events: Vec<TimelineEvent>,
    pub cursor: Option<String>,
}

const CTX: &str = "pr_detail";

// ─────────────────────────── public decoders ───────────────────────────

pub fn decode_first_page(json: &serde_json::Value) -> Result<PrDetailPage, GitHubError> {
    check_graphql_errors(json)?;
    let repo = data_repository(json)?;
    let pr = pull_request(repo)?;

    let summary = pr_summary::decode(&summary_view(repo, pr)?, CTX)?;

    let body = pr
        .get("body")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let body_html = pr
        .get("bodyHTML")
        .and_then(|v| v.as_str())
        .map(std::string::ToString::to_string);
    let requested_reviewers = decode_requested_reviewers(pr);
    let assignees = decode_assignees(pr);
    let head_sha = pr
        .get("headRefOid")
        .and_then(|v| v.as_str())
        .ok_or_else(|| GitHubError::Parse(format!("{CTX}: headRefOid missing")))?
        .to_string();
    let merge_commit_sha = pr
        .pointer("/mergeCommit/oid")
        .and_then(|v| v.as_str())
        .map(std::string::ToString::to_string);
    let base_ref_oid = pr
        .get("baseRefOid")
        .and_then(|v| v.as_str())
        .ok_or_else(|| GitHubError::Parse(format!("{CTX}: baseRefOid missing")))?
        .to_string();
    let available_merge_methods = decode_merge_methods(repo);

    let review_threads_page = decode_threads_from(
        pr.get("reviewThreads")
            .ok_or_else(|| GitHubError::Parse(format!("{CTX}: reviewThreads missing")))?,
    )?;
    let commits_page = decode_commits_from(
        pr.get("commits")
            .ok_or_else(|| GitHubError::Parse(format!("{CTX}: commits missing")))?,
    )?;
    let timeline_page = decode_timeline_from(
        pr.get("timelineItems")
            .ok_or_else(|| GitHubError::Parse(format!("{CTX}: timelineItems missing")))?,
    )?;

    Ok(PrDetailPage {
        summary,
        body,
        body_html,
        requested_reviewers,
        assignees,
        head_sha,
        merge_commit_sha,
        base_ref_oid,
        available_merge_methods,
        review_threads_page,
        commits_page,
        timeline_page,
    })
}

pub fn decode_threads_page(json: &serde_json::Value) -> Result<PagedThreads, GitHubError> {
    check_graphql_errors(json)?;
    let repo = data_repository(json)?;
    let pr = pull_request(repo)?;
    let review_threads = pr
        .get("reviewThreads")
        .ok_or_else(|| GitHubError::Parse(format!("{CTX}: reviewThreads missing")))?;
    decode_threads_from(review_threads)
}

pub fn decode_commits_page(json: &serde_json::Value) -> Result<PagedCommits, GitHubError> {
    check_graphql_errors(json)?;
    let repo = data_repository(json)?;
    let pr = pull_request(repo)?;
    let commits = pr
        .get("commits")
        .ok_or_else(|| GitHubError::Parse(format!("{CTX}: commits missing")))?;
    decode_commits_from(commits)
}

pub fn decode_timeline_page(json: &serde_json::Value) -> Result<PagedTimeline, GitHubError> {
    check_graphql_errors(json)?;
    let repo = data_repository(json)?;
    let pr = pull_request(repo)?;
    let timeline = pr
        .get("timelineItems")
        .ok_or_else(|| GitHubError::Parse(format!("{CTX}: timelineItems missing")))?;
    decode_timeline_from(timeline)
}

pub fn decode_thread_comments_page(
    json: &serde_json::Value,
) -> Result<(Vec<ReviewComment>, Option<String>), GitHubError> {
    check_graphql_errors(json)?;
    let node = json
        .pointer("/data/node")
        .and_then(|v| if v.is_null() { None } else { Some(v) })
        .ok_or_else(|| GitHubError::Parse(format!("{CTX}: data.node missing")))?;
    let comments = node
        .get("comments")
        .ok_or_else(|| GitHubError::Parse(format!("{CTX}: node.comments missing")))?;
    let (comments_vec, cursor, _) = decode_thread_comments(comments)?;
    Ok((comments_vec, cursor))
}

// ─────────────────────────── shared helpers ───────────────────────────

fn check_graphql_errors(json: &serde_json::Value) -> Result<(), GitHubError> {
    if let Some(errors) = json.get("errors").and_then(|e| e.as_array())
        && let Some(first) = errors.first()
    {
        let msg = first
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("(no message)");
        return Err(GitHubError::Parse(format!("{CTX}: {msg}")));
    }
    Ok(())
}

fn data_repository(json: &serde_json::Value) -> Result<&serde_json::Value, GitHubError> {
    let data = json
        .get("data")
        .and_then(|d| if d.is_null() { None } else { Some(d) })
        .ok_or_else(|| GitHubError::Parse(format!("{CTX}: data missing")))?;
    data.get("repository")
        .and_then(|r| if r.is_null() { None } else { Some(r) })
        .ok_or_else(|| GitHubError::Parse(format!("{CTX}: repository missing")))
}

fn pull_request(repo: &serde_json::Value) -> Result<&serde_json::Value, GitHubError> {
    repo.get("pullRequest")
        .and_then(|p| if p.is_null() { None } else { Some(p) })
        .ok_or_else(|| GitHubError::Parse(format!("{CTX}: pullRequest missing")))
}

/// Construct a synthetic JSON node that mirrors the shape the shared
/// `pr_summary::decode` helper expects. The `PrDetail` query selects every
/// `PrSummary` field at the pullRequest root, plus `nameWithOwner` on the
/// repository — we re-wrap them into the `{ ...prFields, repository: { ... } }`
/// shape the helper consumes.
fn summary_view(
    repo: &serde_json::Value,
    pr: &serde_json::Value,
) -> Result<serde_json::Value, GitHubError> {
    let mut node = pr.clone();
    let map = node
        .as_object_mut()
        .ok_or_else(|| GitHubError::Parse(format!("{CTX}: pullRequest is not an object")))?;
    let name_with_owner = repo
        .get("nameWithOwner")
        .cloned()
        .ok_or_else(|| GitHubError::Parse(format!("{CTX}: nameWithOwner missing")))?;
    map.insert(
        "repository".to_string(),
        serde_json::json!({ "nameWithOwner": name_with_owner }),
    );
    // pr_summary expects checks via commits.nodes[0].commit.statusCheckRollup.state;
    // pr_detail's commits selector already includes that.
    Ok(node)
}

fn decode_requested_reviewers(pr: &serde_json::Value) -> Vec<String> {
    let Some(nodes) = pr
        .pointer("/reviewRequests/nodes")
        .and_then(|n| n.as_array())
    else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(nodes.len());
    for n in nodes {
        let Some(reviewer) = n.get("requestedReviewer") else {
            continue;
        };
        if reviewer.is_null() {
            continue;
        }
        let name = reviewer
            .get("login")
            .and_then(|v| v.as_str())
            .or_else(|| reviewer.get("name").and_then(|v| v.as_str()));
        if let Some(name) = name {
            out.push(name.to_string());
        }
    }
    out
}

fn decode_assignees(pr: &serde_json::Value) -> Vec<String> {
    let Some(nodes) = pr.pointer("/assignees/nodes").and_then(|n| n.as_array()) else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(nodes.len());
    for n in nodes {
        if let Some(login) = n.get("login").and_then(|v| v.as_str()) {
            out.push(login.to_string());
        }
    }
    out
}

fn decode_merge_methods(repo: &serde_json::Value) -> Vec<MergeMethod> {
    let mut out = Vec::with_capacity(3);
    if repo
        .get("mergeCommitAllowed")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
    {
        out.push(MergeMethod::Merge);
    }
    if repo
        .get("squashMergeAllowed")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
    {
        out.push(MergeMethod::Squash);
    }
    if repo
        .get("rebaseMergeAllowed")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
    {
        out.push(MergeMethod::Rebase);
    }
    out
}

// ─────────────────────────── threads ───────────────────────────

fn decode_threads_from(review_threads: &serde_json::Value) -> Result<PagedThreads, GitHubError> {
    let nodes = review_threads
        .get("nodes")
        .and_then(|n| n.as_array())
        .ok_or_else(|| GitHubError::Parse(format!("{CTX}: reviewThreads.nodes missing")))?;

    let mut threads = Vec::with_capacity(nodes.len());
    let mut continuations = Vec::new();
    for node in nodes {
        // A single malformed thread must not sink the whole PR detail; skip
        // it (logged) and keep the rest of the conversation usable.
        match decode_review_thread(node) {
            Ok((thread, continuation)) => {
                if let Some(c) = continuation {
                    continuations.push(c);
                }
                threads.push(thread);
            }
            Err(e) => {
                tracing::warn!("skipping unparseable review thread: {e}");
            }
        }
    }
    let cursor = next_cursor(review_threads.get("pageInfo"));
    Ok(PagedThreads {
        threads,
        cursor,
        thread_comment_continuations: continuations,
    })
}

fn decode_review_thread(
    node: &serde_json::Value,
) -> Result<(ReviewThread, Option<(String, String)>), GitHubError> {
    let id = node
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| GitHubError::Parse(format!("{CTX}: thread.id missing")))?
        .to_string();
    let path = node
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| GitHubError::Parse(format!("{CTX}: thread.path missing")))?
        .to_string();
    let is_resolved = node
        .get("isResolved")
        .and_then(serde_json::Value::as_bool)
        .ok_or_else(|| GitHubError::Parse(format!("{CTX}: thread.isResolved missing")))?;
    let is_outdated = node
        .get("isOutdated")
        .and_then(serde_json::Value::as_bool)
        .ok_or_else(|| GitHubError::Parse(format!("{CTX}: thread.isOutdated missing")))?;

    let comments_block = node
        .get("comments")
        .ok_or_else(|| GitHubError::Parse(format!("{CTX}: thread.comments missing")))?;
    let (comments, more_cursor, first_position) = decode_thread_comments(comments_block)?;

    let line_range = decode_line_range(node, first_position);

    let continuation = more_cursor.map(|c| (id.clone(), c));
    Ok((
        ReviewThread {
            id,
            path,
            line_range,
            is_resolved,
            is_outdated,
            comments,
        },
        continuation,
    ))
}

/// Returns `(comments, next_cursor, first_comment_position)`. The position
/// of the originating comment seeds `LineRange.{start,end}_position`.
type DecodedThreadComments = (Vec<ReviewComment>, Option<String>, Option<u32>);

fn decode_thread_comments(
    comments: &serde_json::Value,
) -> Result<DecodedThreadComments, GitHubError> {
    let nodes = comments
        .get("nodes")
        .and_then(|n| n.as_array())
        .ok_or_else(|| GitHubError::Parse(format!("{CTX}: thread.comments.nodes missing")))?;
    let mut out = Vec::with_capacity(nodes.len());
    let mut first_position: Option<u32> = None;
    for (i, c) in nodes.iter().enumerate() {
        let comment = decode_review_comment(c)?;
        if i == 0 {
            first_position = comment_position(c);
        }
        out.push(comment);
    }
    Ok((out, next_cursor(comments.get("pageInfo")), first_position))
}

fn decode_review_comment(c: &serde_json::Value) -> Result<ReviewComment, GitHubError> {
    let id = c
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| GitHubError::Parse(format!("{CTX}: comment.id missing")))?
        .to_string();
    let author = c
        .pointer("/author/login")
        .and_then(|v| v.as_str())
        .ok_or_else(|| GitHubError::Parse(format!("{CTX}: comment.author.login missing")))?
        .to_string();
    let body = c
        .get("body")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let created_at = parse_rfc3339(
        c.get("createdAt")
            .and_then(|v| v.as_str())
            .ok_or_else(|| GitHubError::Parse(format!("{CTX}: comment.createdAt missing")))?,
        "comment.createdAt",
    )?;
    let commit_sha = c
        .pointer("/commit/oid")
        .and_then(|v| v.as_str())
        .ok_or_else(|| GitHubError::Parse(format!("{CTX}: comment.commit.oid missing")))?
        .to_string();
    let suggestion = extract_suggestion(&body);
    Ok(ReviewComment {
        id,
        author,
        body,
        created_at,
        commit_sha,
        suggestion,
    })
}

fn comment_position(c: &serde_json::Value) -> Option<u32> {
    c.get("position")
        .and_then(serde_json::Value::as_u64)
        .or_else(|| {
            c.get("originalPosition")
                .and_then(serde_json::Value::as_u64)
        })
        .and_then(|n| u32::try_from(n).ok())
}

/// Decode a thread's anchor into a single-side [`LineRange`]. Infallible:
/// GitHub returns `line: null` for *outdated* threads (the line no longer
/// exists in the current diff) and may omit `diffSide`, so we fall back to
/// `originalLine`/`originalStartLine` and default the side to the head
/// (`RIGHT`). One odd thread must never break the whole PR detail.
fn decode_line_range(node: &serde_json::Value, first_position: Option<u32>) -> LineRange {
    let side = node
        .get("diffSide")
        .and_then(|v| v.as_str())
        .and_then(parse_diff_side_opt)
        .unwrap_or(DiffSide::Right);
    let line = node
        .get("line")
        .and_then(serde_json::Value::as_u64)
        .or_else(|| node.get("originalLine").and_then(serde_json::Value::as_u64))
        .unwrap_or(0);
    let start_line = node
        .get("startLine")
        .and_then(serde_json::Value::as_u64)
        .or_else(|| {
            node.get("originalStartLine")
                .and_then(serde_json::Value::as_u64)
        })
        .unwrap_or(line);
    let line_u32 = u32::try_from(line).unwrap_or(0);
    let start_line_u32 = u32::try_from(start_line).unwrap_or(line_u32).min(line_u32);
    let position = first_position.unwrap_or(0);
    LineRange {
        side,
        start_line: start_line_u32,
        end_line: line_u32,
        start_position: position,
        end_position: position,
    }
}

fn parse_diff_side_opt(s: &str) -> Option<DiffSide> {
    match s {
        "LEFT" => Some(DiffSide::Left),
        "RIGHT" => Some(DiffSide::Right),
        _ => None,
    }
}

/// Extract the first fenced ```` ```suggestion ```` block from `body`.
/// Returns `None` if absent. The closing fence is matched only on a line
/// of its own (per GitHub's renderer).
fn extract_suggestion(body: &str) -> Option<String> {
    const OPEN: &str = "```suggestion\n";
    let open_idx = body.find(OPEN)?;
    let after_open = &body[open_idx + OPEN.len()..];
    // Closing fence: "```" on its own line. Either "\n```\n", "\n```" at
    // end of string, or — degenerately — "```" at the very start of the
    // remainder (empty suggestion body, written as ```suggestion\n```).
    if let Some(rel) = after_open.find("\n```") {
        return Some(after_open[..rel].to_string());
    }
    if after_open.starts_with("```") {
        return Some(String::new());
    }
    None
}

// ─────────────────────────── commits ───────────────────────────

fn decode_commits_from(commits: &serde_json::Value) -> Result<PagedCommits, GitHubError> {
    let nodes = commits
        .get("nodes")
        .and_then(|n| n.as_array())
        .ok_or_else(|| GitHubError::Parse(format!("{CTX}: commits.nodes missing")))?;
    let mut out = Vec::with_capacity(nodes.len());
    for n in nodes {
        let commit = n
            .get("commit")
            .ok_or_else(|| GitHubError::Parse(format!("{CTX}: commits.node.commit missing")))?;
        out.push(decode_commit(commit)?);
    }
    Ok(PagedCommits {
        commits: out,
        cursor: next_cursor(commits.get("pageInfo")),
    })
}

fn decode_commit(commit: &serde_json::Value) -> Result<TimelineEvent, GitHubError> {
    let sha = commit
        .get("oid")
        .and_then(|v| v.as_str())
        .ok_or_else(|| GitHubError::Parse(format!("{CTX}: commit.oid missing")))?
        .to_string();
    let message = commit
        .get("messageHeadline")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    // Ghost authors (commits without a linked GitHub user) are legitimate —
    // commits via web-edit, ancient history, or deleted accounts. Don't
    // fail the whole PR for one — fall back to empty author.
    let author = commit
        .pointer("/author/user/login")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    // `pushedDate` is null for web-UI / rebase commits; fall back to
    // `authoredDate` so the timeline still has a usable timestamp.
    let pushed_str = commit
        .get("pushedDate")
        .and_then(|v| v.as_str())
        .or_else(|| commit.get("authoredDate").and_then(|v| v.as_str()))
        .ok_or_else(|| {
            GitHubError::Parse(format!("{CTX}: commit pushedDate/authoredDate missing"))
        })?;
    let pushed_at = parse_rfc3339(pushed_str, "commit.pushedDate")?;
    Ok(TimelineEvent::Commit {
        sha,
        message,
        author,
        pushed_at,
    })
}

// ─────────────────────────── timeline ───────────────────────────

fn decode_timeline_from(timeline: &serde_json::Value) -> Result<PagedTimeline, GitHubError> {
    let nodes = timeline
        .get("nodes")
        .and_then(|n| n.as_array())
        .ok_or_else(|| GitHubError::Parse(format!("{CTX}: timelineItems.nodes missing")))?;
    let mut events = Vec::with_capacity(nodes.len());
    for n in nodes {
        if let Some(ev) = decode_timeline_node(n)? {
            events.push(ev);
        }
    }
    Ok(PagedTimeline {
        events,
        cursor: next_cursor(timeline.get("pageInfo")),
    })
}

fn decode_timeline_node(n: &serde_json::Value) -> Result<Option<TimelineEvent>, GitHubError> {
    let kind = n
        .get("__typename")
        .and_then(|v| v.as_str())
        .ok_or_else(|| GitHubError::Parse(format!("{CTX}: timeline node __typename missing")))?;
    // PullRequestCommit nodes share the wildcard `Ok(None)` arm with unknown
    // future event types: commits arrive through the dedicated `commits`
    // block (which carries authoredDate + pushedDate), so we skip them here
    // to avoid double-counting. Unknown __typenames are forward-compat skips.
    match kind {
        "IssueComment" => decode_issue_comment(n),
        "PullRequestReview" => decode_pr_review(n),
        "MergedEvent" => decode_merged_event(n).map(Some),
        "ClosedEvent" => decode_closed_event(n).map(Some),
        "ReopenedEvent" => decode_reopened_event(n).map(Some),
        "HeadRefForcePushedEvent" => decode_force_pushed_event(n).map(Some),
        _ => Ok(None),
    }
}

fn decode_issue_comment(n: &serde_json::Value) -> Result<Option<TimelineEvent>, GitHubError> {
    let Some(author) = n.pointer("/author/login").and_then(|v| v.as_str()) else {
        return Ok(None);
    };
    let id = n
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| GitHubError::Parse(format!("{CTX}: IssueComment.id missing")))?
        .to_string();
    let body = n
        .get("body")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let created_at = parse_rfc3339(
        n.get("createdAt")
            .and_then(|v| v.as_str())
            .ok_or_else(|| GitHubError::Parse(format!("{CTX}: IssueComment.createdAt missing")))?,
        "IssueComment.createdAt",
    )?;
    Ok(Some(TimelineEvent::IssueComment {
        id,
        author: author.to_string(),
        body,
        created_at,
    }))
}

fn decode_pr_review(n: &serde_json::Value) -> Result<Option<TimelineEvent>, GitHubError> {
    let Some(author) = n.pointer("/author/login").and_then(|v| v.as_str()) else {
        return Ok(None);
    };
    let id = n
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| GitHubError::Parse(format!("{CTX}: Review.id missing")))?
        .to_string();
    let state = parse_review_state(
        n.get("state")
            .and_then(|v| v.as_str())
            .ok_or_else(|| GitHubError::Parse(format!("{CTX}: Review.state missing")))?,
    )?;
    let body = n
        .get("body")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let submitted_at = parse_rfc3339(
        n.get("submittedAt")
            .and_then(|v| v.as_str())
            .ok_or_else(|| GitHubError::Parse(format!("{CTX}: Review.submittedAt missing")))?,
        "Review.submittedAt",
    )?;
    Ok(Some(TimelineEvent::Review {
        id,
        author: author.to_string(),
        state,
        body,
        submitted_at,
    }))
}

fn decode_merged_event(n: &serde_json::Value) -> Result<TimelineEvent, GitHubError> {
    let sha = n
        .pointer("/commit/oid")
        .and_then(|v| v.as_str())
        .ok_or_else(|| GitHubError::Parse(format!("{CTX}: MergedEvent.commit.oid missing")))?
        .to_string();
    let by = actor_login(n, "MergedEvent")?;
    let at = parse_event_created_at(n, "MergedEvent")?;
    Ok(TimelineEvent::Merged { sha, by, at })
}

fn decode_closed_event(n: &serde_json::Value) -> Result<TimelineEvent, GitHubError> {
    Ok(TimelineEvent::Closed {
        by: actor_login(n, "ClosedEvent")?,
        at: parse_event_created_at(n, "ClosedEvent")?,
    })
}

fn decode_reopened_event(n: &serde_json::Value) -> Result<TimelineEvent, GitHubError> {
    Ok(TimelineEvent::Reopened {
        by: actor_login(n, "ReopenedEvent")?,
        at: parse_event_created_at(n, "ReopenedEvent")?,
    })
}

fn decode_force_pushed_event(n: &serde_json::Value) -> Result<TimelineEvent, GitHubError> {
    let before = n
        .pointer("/beforeCommit/oid")
        .and_then(|v| v.as_str())
        .ok_or_else(|| GitHubError::Parse(format!("{CTX}: ForcePushed.beforeCommit.oid missing")))?
        .to_string();
    let after = n
        .pointer("/afterCommit/oid")
        .and_then(|v| v.as_str())
        .ok_or_else(|| GitHubError::Parse(format!("{CTX}: ForcePushed.afterCommit.oid missing")))?
        .to_string();
    let at = parse_event_created_at(n, "ForcePushed")?;
    Ok(TimelineEvent::HeadRefForcePushed { before, after, at })
}

fn actor_login(n: &serde_json::Value, ctx: &str) -> Result<String, GitHubError> {
    n.pointer("/actor/login")
        .and_then(|v| v.as_str())
        .map(std::string::ToString::to_string)
        .ok_or_else(|| GitHubError::Parse(format!("{CTX}: {ctx}.actor.login missing")))
}

fn parse_event_created_at(n: &serde_json::Value, ctx: &str) -> Result<DateTime<Utc>, GitHubError> {
    let s = n
        .get("createdAt")
        .and_then(|v| v.as_str())
        .ok_or_else(|| GitHubError::Parse(format!("{CTX}: {ctx}.createdAt missing")))?;
    parse_rfc3339(s, &format!("{ctx}.createdAt"))
}

fn parse_review_state(s: &str) -> Result<ReviewState, GitHubError> {
    match s {
        "PENDING" => Ok(ReviewState::Pending),
        "COMMENTED" => Ok(ReviewState::Commented),
        "APPROVED" => Ok(ReviewState::Approved),
        "CHANGES_REQUESTED" => Ok(ReviewState::ChangesRequested),
        "DISMISSED" => Ok(ReviewState::Dismissed),
        other => Err(GitHubError::Parse(format!(
            "{CTX}: unknown review state {other}"
        ))),
    }
}

// ─────────────────────────── utility ───────────────────────────

fn next_cursor(page_info: Option<&serde_json::Value>) -> Option<String> {
    let pi = page_info?;
    let has_next = pi
        .get("hasNextPage")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    if !has_next {
        return None;
    }
    pi.get("endCursor")
        .and_then(|v| v.as_str())
        .map(std::string::ToString::to_string)
}

fn parse_rfc3339(s: &str, ctx: &str) -> Result<DateTime<Utc>, GitHubError> {
    DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| GitHubError::Parse(format!("{CTX}: {ctx} not RFC3339: {e}")))
}

// ─────────────────────────── tests ───────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const FIXTURE: &str = include_str!("../../../../tests/fixtures/api/pr_detail.graphql.json");

    fn fixture() -> serde_json::Value {
        serde_json::from_str(FIXTURE).expect("fixture parses as JSON")
    }

    #[test]
    fn decodes_recorded_fixture_first_page() {
        let page = decode_first_page(&fixture()).expect("decode succeeds");
        assert_eq!(page.summary.id.number, 13518);
        assert_eq!(page.summary.id.repo.owner, "cli");
        assert_eq!(page.summary.id.repo.name, "cli");
        assert!(page.body_html.is_some());
        assert_eq!(
            page.merge_commit_sha.as_deref(),
            Some("9a593ce81b593dee752cc11737d1a3ef768e52b3")
        );
        assert_eq!(page.review_threads_page.threads.len(), 1);
        let thread = &page.review_threads_page.threads[0];
        assert!(!thread.comments.is_empty());
        assert!(!thread.comments[0].body.is_empty());
        assert!(matches!(
            thread.line_range.side,
            DiffSide::Left | DiffSide::Right
        ));
        assert_eq!(thread.comments[0].commit_sha.len(), 40);
        assert_eq!(thread.line_range.start_position, 4);
        assert_eq!(thread.line_range.end_position, 4);
        assert!(!page.available_merge_methods.is_empty());
        // Fixture timeline contains MergedEvent + ClosedEvent + 2x PullRequestReview.
        let has_merge_or_close = page.timeline_page.events.iter().any(|e| {
            matches!(
                e,
                TimelineEvent::Merged { .. } | TimelineEvent::Closed { .. }
            )
        });
        assert!(has_merge_or_close, "expected MergedEvent or ClosedEvent");
        assert!(!page.commits_page.commits.is_empty());
        // Recorded fixture has no continuations / no further pages.
        assert!(
            page.review_threads_page
                .thread_comment_continuations
                .is_empty()
        );
        assert!(page.review_threads_page.cursor.is_none());
        assert!(page.commits_page.cursor.is_none());
        assert!(page.timeline_page.cursor.is_none());
    }

    #[test]
    fn skips_pull_request_commit_in_timeline() {
        let payload = json!({
            "data": { "repository": { "nameWithOwner": "a/b", "pullRequest": {
                "timelineItems": {
                    "pageInfo": { "hasNextPage": false, "endCursor": null },
                    "nodes": [
                        { "__typename": "PullRequestCommit",
                          "commit": { "oid": "abc", "messageHeadline": "x",
                                      "pushedDate": "2026-01-01T00:00:00Z",
                                      "author": { "user": { "login": "u" } } } }
                    ]
                }
            }}}
        });
        let page = decode_timeline_page(&payload).expect("decode");
        assert!(page.events.is_empty(), "PullRequestCommit must be skipped");
    }

    #[test]
    fn force_pushed_event_round_trips() {
        let payload = json!({
            "data": { "repository": { "nameWithOwner": "a/b", "pullRequest": {
                "timelineItems": {
                    "pageInfo": { "hasNextPage": false },
                    "nodes": [
                        { "__typename": "HeadRefForcePushedEvent",
                          "createdAt": "2026-02-03T04:05:06Z",
                          "actor": { "login": "u" },
                          "beforeCommit": { "oid": "aaa" },
                          "afterCommit": { "oid": "bbb" } }
                    ]
                }
            }}}
        });
        let page = decode_timeline_page(&payload).expect("decode");
        assert_eq!(page.events.len(), 1);
        match &page.events[0] {
            TimelineEvent::HeadRefForcePushed { before, after, at } => {
                assert_eq!(before, "aaa");
                assert_eq!(after, "bbb");
                assert_eq!(at.to_rfc3339(), "2026-02-03T04:05:06+00:00");
            }
            other => panic!("expected HeadRefForcePushed, got {other:?}"),
        }
    }

    #[test]
    fn pushed_date_null_falls_back_to_authored_date() {
        let payload = json!({
            "data": { "repository": { "nameWithOwner": "a/b", "pullRequest": {
                "commits": {
                    "pageInfo": { "hasNextPage": false },
                    "nodes": [
                        { "commit": {
                            "oid": "abc",
                            "messageHeadline": "msg",
                            "authoredDate": "2026-01-01T00:00:00Z",
                            "pushedDate": null,
                            "author": { "user": { "login": "u" } }
                        }}
                    ]
                }
            }}}
        });
        let page = decode_commits_page(&payload).expect("decode");
        assert_eq!(page.commits.len(), 1);
        match &page.commits[0] {
            TimelineEvent::Commit { pushed_at, .. } => {
                assert_eq!(pushed_at.to_rfc3339(), "2026-01-01T00:00:00+00:00");
            }
            other => panic!("expected Commit, got {other:?}"),
        }
    }

    #[test]
    fn review_thread_with_both_diffsides_decodes_using_diffside() {
        // A both-sides thread is rare; we degrade to the head (`diffSide`)
        // rather than failing the whole PR.
        let payload = json!({
            "data": { "repository": { "nameWithOwner": "a/b", "pullRequest": {
                "reviewThreads": {
                    "pageInfo": { "hasNextPage": false },
                    "nodes": [{
                        "id": "T", "path": "f", "isResolved": false, "isOutdated": false,
                        "line": 1, "startLine": 1,
                        "diffSide": "RIGHT", "startDiffSide": "LEFT",
                        "comments": { "nodes": [] }
                    }]
                }
            }}}
        });
        let page = decode_threads_page(&payload).expect("decodes");
        assert_eq!(page.threads.len(), 1);
        assert_eq!(page.threads[0].line_range.side, DiffSide::Right);
    }

    #[test]
    fn outdated_thread_with_null_line_falls_back_to_original_line() {
        // Outdated threads carry `line: null` / `diffSide: null`; the parser
        // must fall back to `originalLine` instead of failing (regression).
        let payload = json!({
            "data": { "repository": { "nameWithOwner": "a/b", "pullRequest": {
                "reviewThreads": {
                    "pageInfo": { "hasNextPage": false },
                    "nodes": [{
                        "id": "T", "path": "f", "isResolved": false, "isOutdated": true,
                        "line": null, "startLine": null,
                        "originalLine": 42, "originalStartLine": 40,
                        "diffSide": null, "startDiffSide": null,
                        "comments": { "nodes": [] }
                    }]
                }
            }}}
        });
        let page = decode_threads_page(&payload).expect("outdated thread decodes");
        assert_eq!(page.threads.len(), 1);
        assert_eq!(page.threads[0].line_range.end_line, 42);
        assert_eq!(page.threads[0].line_range.start_line, 40);
        assert_eq!(page.threads[0].line_range.side, DiffSide::Right);
    }

    #[test]
    fn unparseable_thread_is_skipped_not_fatal() {
        // A thread missing required fields (e.g. `id`) is dropped, leaving
        // the well-formed sibling intact — the PR detail survives.
        let payload = json!({
            "data": { "repository": { "nameWithOwner": "a/b", "pullRequest": {
                "reviewThreads": {
                    "pageInfo": { "hasNextPage": false },
                    "nodes": [
                        { "path": "f", "isResolved": false, "isOutdated": false,
                          "diffSide": "RIGHT", "line": 1, "comments": { "nodes": [] } },
                        { "id": "T2", "path": "g", "isResolved": false, "isOutdated": false,
                          "diffSide": "RIGHT", "line": 2, "comments": { "nodes": [] } }
                    ]
                }
            }}}
        });
        let page = decode_threads_page(&payload).expect("survives one bad thread");
        assert_eq!(page.threads.len(), 1, "only the well-formed thread remains");
        assert_eq!(page.threads[0].id, "T2");
    }

    #[test]
    fn extract_suggestion_finds_only_suggestion_blocks() {
        assert_eq!(extract_suggestion("no fences at all"), None);
        assert_eq!(extract_suggestion("```rust\nfoo\n```"), None);
        assert_eq!(
            extract_suggestion("```suggestion\nfoo bar\n```"),
            Some("foo bar".to_string())
        );
        assert_eq!(
            extract_suggestion("```suggestion\n```"),
            Some(String::new())
        );
        assert_eq!(
            extract_suggestion("```suggestion\nfirst\n```\n```suggestion\nsecond\n```"),
            Some("first".to_string())
        );
        assert_eq!(
            extract_suggestion("intro\n```suggestion\nfoo\n```\nmore text"),
            Some("foo".to_string())
        );
    }

    #[test]
    fn ghost_author_on_issue_comment_is_skipped() {
        let payload = json!({
            "data": { "repository": { "nameWithOwner": "a/b", "pullRequest": {
                "timelineItems": {
                    "pageInfo": { "hasNextPage": false },
                    "nodes": [
                        { "__typename": "IssueComment",
                          "id": "C1",
                          "author": null,
                          "body": "ghost",
                          "createdAt": "2026-01-01T00:00:00Z" }
                    ]
                }
            }}}
        });
        let page = decode_timeline_page(&payload).expect("decode");
        assert!(page.events.is_empty(), "ghost IssueComment must be skipped");
    }

    #[test]
    fn available_merge_methods_respects_repo_settings() {
        // Build a minimal first-page payload exercising decode_merge_methods
        // via the public path.
        let payload = json!({
            "data": { "repository": {
                "nameWithOwner": "a/b",
                "mergeCommitAllowed": false,
                "squashMergeAllowed": true,
                "rebaseMergeAllowed": true,
                "pullRequest": minimal_pr_node()
            }}
        });
        let page = decode_first_page(&payload).expect("decode");
        assert_eq!(
            page.available_merge_methods,
            vec![MergeMethod::Squash, MergeMethod::Rebase]
        );
    }

    #[test]
    fn ghost_commit_author_decodes_with_empty_author() {
        let payload = json!({
            "data": { "repository": { "nameWithOwner": "a/b", "pullRequest": {
                "commits": {
                    "pageInfo": { "hasNextPage": false },
                    "nodes": [
                        { "commit": {
                            "oid": "abc",
                            "messageHeadline": "m",
                            "authoredDate": "2026-01-01T00:00:00Z",
                            "pushedDate": "2026-01-01T00:00:00Z",
                            "author": { "user": null }
                        }}
                    ]
                }
            }}}
        });
        let page = decode_commits_page(&payload).expect("decode");
        match &page.commits[0] {
            TimelineEvent::Commit { author, .. } => assert_eq!(author, ""),
            other => panic!("expected Commit, got {other:?}"),
        }
    }

    // Minimal PR node that satisfies pr_summary + the structural fields
    // pr_detail mandates. Used by tests that don't care about lists.
    fn minimal_pr_node() -> serde_json::Value {
        json!({
            "number": 1, "title": "t", "body": "", "bodyHTML": null,
            "state": "OPEN", "isDraft": false,
            "updatedAt": "2026-01-01T00:00:00Z", "createdAt": "2026-01-01T00:00:00Z",
            "mergeStateStatus": "UNKNOWN",
            "reviewDecision": null,
            "additions": 0, "deletions": 0, "changedFiles": 0,
            "totalCommentsCount": 0,
            "headRefOid": "abc",
            "baseRefOid": "def",
            "baseRefName": "main", "headRefName": "f",
            "author": { "login": "u" },
            "mergeCommit": null,
            "assignees": { "nodes": [] },
            "reviewRequests": { "nodes": [] },
            "labels": { "nodes": [] },
            "commits": { "pageInfo": { "hasNextPage": false }, "nodes": [] },
            "reviewThreads": { "pageInfo": { "hasNextPage": false }, "nodes": [] },
            "timelineItems": { "pageInfo": { "hasNextPage": false }, "nodes": [] }
        })
    }

    // Regression guard for parser/query drift: pr_summary::decode requires
    // `createdAt` + `updatedAt` on every PR node it parses. pr_detail.graphql
    // silently dropped `createdAt`; the fixture had it hand-baked so tests
    // passed while real responses failed with `pr_detail: createdAt missing`.
    const PR_DETAIL_QUERY_SOURCE: &str = include_str!("../graphql/pr_detail.graphql");

    #[test]
    fn pr_detail_query_requests_created_at_and_updated_at_on_pr() {
        let q = PR_DETAIL_QUERY_SOURCE;
        let header_start = q
            .find("pullRequest(number: $number)")
            .expect("pullRequest selection present");
        let after_header = &q[header_start..];
        let line_with_top_scalars = after_header
            .lines()
            .find(|l| l.contains("number title"))
            .expect("PR scalar line present");
        assert!(
            line_with_top_scalars.contains("createdAt"),
            "pr_detail.graphql top-level pullRequest selection must include \
             `createdAt` (pr_summary::decode requires it); got: {line_with_top_scalars}"
        );
        assert!(
            line_with_top_scalars.contains("updatedAt"),
            "pr_detail.graphql top-level pullRequest selection must include \
             `updatedAt`; got: {line_with_top_scalars}"
        );
    }

    #[test]
    fn decode_first_page_accepts_real_world_shape_with_null_pushed_date() {
        let raw = REAL_WORLD_PAYLOAD;
        let payload: serde_json::Value = serde_json::from_str(raw).expect("payload parses");
        let page = decode_first_page(&payload).expect("decode succeeds");
        assert_eq!(page.summary.id.number, 621);
        assert_eq!(page.timeline_page.events.len(), 2);
    }

    #[test]
    fn decode_first_page_errors_with_createdat_missing_when_pr_node_lacks_it() {
        let raw = REAL_WORLD_PAYLOAD.replace(r#""createdAt": "2026-05-30T03:50:06Z","#, "");
        let payload: serde_json::Value = serde_json::from_str(&raw).expect("payload parses");
        let err = decode_first_page(&payload).expect_err("decode should fail");
        let msg = format!("{err}");
        assert!(
            msg.contains("createdAt missing"),
            "expected `createdAt missing` failure, got: {msg}"
        );
    }

    const REAL_WORLD_PAYLOAD: &str = r#"{
        "data": { "repository": {
            "nameWithOwner": "o/n",
            "mergeCommitAllowed": false,
            "squashMergeAllowed": false,
            "rebaseMergeAllowed": true,
            "pullRequest": {
                "number": 621, "title": "t", "body": "", "bodyHTML": null,
                "state": "OPEN", "isDraft": false,
                "updatedAt": "2026-05-30T04:22:30Z",
                "createdAt": "2026-05-30T03:50:06Z",
                "mergeStateStatus": "CLEAN",
                "reviewDecision": null,
                "additions": 1, "deletions": 1, "changedFiles": 1,
                "totalCommentsCount": 0,
                "headRefOid": "9fc6a0e",
                "baseRefOid": "b678491",
                "baseRefName": "main", "headRefName": "f",
                "author": { "login": "u" },
                "mergeCommit": null,
                "assignees": { "nodes": [] },
                "reviewRequests": { "nodes": [] },
                "labels": { "nodes": [] },
                "commits": { "pageInfo": { "hasNextPage": false }, "nodes": [
                    { "commit": {
                        "oid": "9fc6a0e",
                        "messageHeadline": "Fix detekt issues",
                        "authoredDate": "2026-05-30T03:50:06Z",
                        "pushedDate": null,
                        "author": { "user": { "login": "u" } },
                        "statusCheckRollup": { "state": "SUCCESS" }
                    }}
                ]},
                "reviewThreads": { "pageInfo": { "hasNextPage": false }, "nodes": [] },
                "timelineItems": { "pageInfo": { "hasNextPage": false }, "nodes": [
                    { "__typename": "PullRequestReview",
                      "id": "PRR_x", "author": { "login": "claude" },
                      "state": "COMMENTED", "body": "review",
                      "submittedAt": "2026-05-30T04:18:04Z" },
                    { "__typename": "PullRequestCommit",
                      "commit": {
                          "oid": "9fc6a0e", "messageHeadline": "Fix detekt issues",
                          "pushedDate": null,
                          "author": { "user": { "login": "u" }, "name": "u" }
                      }},
                    { "__typename": "HeadRefForcePushedEvent",
                      "createdAt": "2026-05-30T04:22:30Z",
                      "actor": { "login": "u" },
                      "beforeCommit": { "oid": "b678491" },
                      "afterCommit": { "oid": "9fc6a0e" } }
                ]}
            }
        }}
    }"#;
}
