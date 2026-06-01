//! Shared decoder for the `PrSummary` node shape.
//!
//! Both the `Dashboard` and `PrList` `GraphQL` queries select the identical
//! `PullRequest` sub-tree by design, so the node→`PrSummary` mapping lives
//! here and is consumed by `parse::dashboard` and `parse::pr_list`.
//! The `ctx` parameter prefixes parse errors so callers retain query
//! provenance (e.g. `"dashboard"` or `"pr_list"`).

use chrono::{DateTime, Utc};

use crate::data::github::GitHubError;
use crate::data::models::{
    ChecksRollup, Label, Mergeable, PrId, PrState, PrSummary, Repo, ReviewDecision,
};

pub fn decode(node: &serde_json::Value, ctx: &str) -> Result<PrSummary, GitHubError> {
    let name_with_owner = node
        .pointer("/repository/nameWithOwner")
        .and_then(|v| v.as_str())
        .ok_or_else(|| GitHubError::Parse(format!("{ctx}: repository.nameWithOwner missing")))?;
    let repo = parse_repo(name_with_owner, ctx)?;
    let number = node
        .get("number")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| GitHubError::Parse(format!("{ctx}: number missing")))?;
    let title = node
        .get("title")
        .and_then(|v| v.as_str())
        .ok_or_else(|| GitHubError::Parse(format!("{ctx}: title missing")))?
        .to_string();
    let author = node
        .pointer("/author/login")
        .and_then(|v| v.as_str())
        .ok_or_else(|| GitHubError::Parse(format!("{ctx}: author.login missing")))?
        .to_string();
    let state = parse_state(
        node.get("state")
            .and_then(|v| v.as_str())
            .ok_or_else(|| GitHubError::Parse(format!("{ctx}: state missing")))?,
        ctx,
    )?;
    let is_draft = node
        .get("isDraft")
        .and_then(serde_json::Value::as_bool)
        .ok_or_else(|| GitHubError::Parse(format!("{ctx}: isDraft missing")))?;
    let base = node
        .get("baseRefName")
        .and_then(|v| v.as_str())
        .ok_or_else(|| GitHubError::Parse(format!("{ctx}: baseRefName missing")))?
        .to_string();
    let head = node
        .get("headRefName")
        .and_then(|v| v.as_str())
        .ok_or_else(|| GitHubError::Parse(format!("{ctx}: headRefName missing")))?
        .to_string();
    let additions = u32_field(node, "additions", ctx)?;
    let deletions = u32_field(node, "deletions", ctx)?;
    let changed_files = u32_field(node, "changedFiles", ctx)?;
    let comments = u32_field(node, "totalCommentsCount", ctx)?;
    let review_decision = parse_review_decision(node.get("reviewDecision"), ctx)?;
    let checks = parse_checks(node.get("commits"), ctx)?;
    let mergeable = parse_mergeable(
        node.get("mergeStateStatus")
            .and_then(|v| v.as_str())
            .ok_or_else(|| GitHubError::Parse(format!("{ctx}: mergeStateStatus missing")))?,
        ctx,
    )?;
    let labels = parse_labels(node.get("labels"), ctx)?;
    let updated_at = parse_rfc3339(node, "updatedAt", ctx)?;
    let created_at = parse_rfc3339(node, "createdAt", ctx)?;

    Ok(PrSummary {
        id: PrId { repo, number },
        title,
        author,
        state,
        is_draft,
        base,
        head,
        additions,
        deletions,
        changed_files,
        comments,
        review_decision,
        checks,
        mergeable,
        labels,
        updated_at,
        created_at,
    })
}

fn parse_repo(name_with_owner: &str, ctx: &str) -> Result<Repo, GitHubError> {
    let (owner, name) = name_with_owner.split_once('/').ok_or_else(|| {
        GitHubError::Parse(format!(
            "{ctx}: repository.nameWithOwner not 'owner/name': {name_with_owner}"
        ))
    })?;
    if owner.is_empty() || name.is_empty() {
        return Err(GitHubError::Parse(format!(
            "{ctx}: repository.nameWithOwner has empty side: {name_with_owner}"
        )));
    }
    Ok(Repo {
        owner: owner.to_string(),
        name: name.to_string(),
    })
}

fn u32_field(node: &serde_json::Value, key: &str, ctx: &str) -> Result<u32, GitHubError> {
    let raw = node
        .get(key)
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| GitHubError::Parse(format!("{ctx}: {key} missing")))?;
    u32::try_from(raw).map_err(|_| GitHubError::Parse(format!("{ctx}: {key} overflows u32")))
}

fn parse_state(s: &str, ctx: &str) -> Result<PrState, GitHubError> {
    match s {
        "OPEN" => Ok(PrState::Open),
        "CLOSED" => Ok(PrState::Closed),
        "MERGED" => Ok(PrState::Merged),
        other => Err(GitHubError::Parse(format!(
            "{ctx}: unknown PR state {other}"
        ))),
    }
}

fn parse_review_decision(
    v: Option<&serde_json::Value>,
    ctx: &str,
) -> Result<Option<ReviewDecision>, GitHubError> {
    match v {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::String(s)) => match s.as_str() {
            "APPROVED" => Ok(Some(ReviewDecision::Approved)),
            "CHANGES_REQUESTED" => Ok(Some(ReviewDecision::ChangesRequested)),
            "REVIEW_REQUIRED" => Ok(Some(ReviewDecision::ReviewRequired)),
            other => Err(GitHubError::Parse(format!(
                "{ctx}: unknown reviewDecision {other}"
            ))),
        },
        Some(other) => Err(GitHubError::Parse(format!(
            "{ctx}: reviewDecision must be string|null, got {other}"
        ))),
    }
}

fn parse_checks(
    commits: Option<&serde_json::Value>,
    ctx: &str,
) -> Result<ChecksRollup, GitHubError> {
    let Some(commits) = commits else {
        return Ok(ChecksRollup::None);
    };
    let state = commits.pointer("/nodes/0/commit/statusCheckRollup/state");
    match state {
        None | Some(serde_json::Value::Null) => Ok(ChecksRollup::None),
        Some(serde_json::Value::String(s)) => match s.as_str() {
            "PENDING" | "EXPECTED" => Ok(ChecksRollup::Pending),
            "SUCCESS" => Ok(ChecksRollup::Success),
            "FAILURE" | "ERROR" => Ok(ChecksRollup::Failure),
            "NEUTRAL" => Ok(ChecksRollup::Neutral),
            "MIXED" => Ok(ChecksRollup::Mixed),
            other => Err(GitHubError::Parse(format!(
                "{ctx}: unknown statusCheckRollup state {other}"
            ))),
        },
        Some(other) => Err(GitHubError::Parse(format!(
            "{ctx}: statusCheckRollup.state must be string|null, got {other}"
        ))),
    }
}

fn parse_mergeable(s: &str, ctx: &str) -> Result<Mergeable, GitHubError> {
    match s {
        "CLEAN" => Ok(Mergeable::Clean),
        "UNSTABLE" => Ok(Mergeable::Unstable),
        "HAS_HOOKS" => Ok(Mergeable::HasHooks),
        "BEHIND" => Ok(Mergeable::Behind),
        "BLOCKED" => Ok(Mergeable::Blocked),
        "DIRTY" => Ok(Mergeable::Dirty),
        "DRAFT" => Ok(Mergeable::Draft),
        "UNKNOWN" => Ok(Mergeable::Unknown),
        other => Err(GitHubError::Parse(format!(
            "{ctx}: unknown mergeStateStatus {other}"
        ))),
    }
}

fn parse_labels(labels: Option<&serde_json::Value>, ctx: &str) -> Result<Vec<Label>, GitHubError> {
    let Some(labels) = labels else {
        return Ok(Vec::new());
    };
    let Some(nodes) = labels.get("nodes").and_then(|n| n.as_array()) else {
        return Ok(Vec::new());
    };
    let mut out = Vec::with_capacity(nodes.len());
    for node in nodes {
        let name = node
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| GitHubError::Parse(format!("{ctx}: label.name missing")))?
            .to_string();
        let color = node
            .get("color")
            .and_then(|v| v.as_str())
            .ok_or_else(|| GitHubError::Parse(format!("{ctx}: label.color missing")))?
            .to_string();
        out.push(Label { name, color });
    }
    Ok(out)
}

fn parse_rfc3339(
    node: &serde_json::Value,
    field: &str,
    ctx: &str,
) -> Result<DateTime<Utc>, GitHubError> {
    let s = node
        .get(field)
        .and_then(|v| v.as_str())
        .ok_or_else(|| GitHubError::Parse(format!("{ctx}: {field} missing")))?;
    DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| GitHubError::Parse(format!("{ctx}: {field} not RFC3339: {e}")))
}
