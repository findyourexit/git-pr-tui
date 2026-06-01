//! Parser for the Dashboard GraphQL query. Pure: takes `&serde_json::Value`, returns Result.
//!
//! Pagination uses a per-bucket cursor carried on `DashboardRequest`.
//!
//! The node→`PrSummary` mapping lives in `super::pr_summary` and is shared
//! verbatim with `super::pr_list` (both queries select the same fields).

use std::collections::HashSet;

use super::pr_summary;
use crate::data::github::{DashboardBucket, GitHubError};
use crate::data::models::{PrId, PrSummary};

type BucketPage = (DashboardBucket, Vec<PrSummary>, Option<String>);

/// Decode a full dashboard GraphQL response (three bucket searches) into a
/// vector of `(bucket, prs, next_cursor)` triples. Buckets missing from the
/// response decode as empty lists with no cursor; intra-bucket duplicates are
/// removed (first occurrence wins) but cross-bucket presence is preserved.
pub fn decode(json: &serde_json::Value) -> Result<Vec<BucketPage>, GitHubError> {
    let data = json
        .get("data")
        .ok_or_else(|| GitHubError::Parse("dashboard: data missing".into()))?;

    let buckets = [
        ("reviewRequested", DashboardBucket::ReviewRequested),
        ("authored", DashboardBucket::Authored),
        ("assigned", DashboardBucket::Assigned),
    ];

    let mut out = Vec::with_capacity(3);
    for (key, bucket) in buckets {
        let (prs, cursor) = match data.get(key) {
            Some(v) if !v.is_null() => decode_bucket(v)?,
            _ => (Vec::new(), None),
        };
        out.push((bucket, prs, cursor));
    }
    Ok(out)
}

fn decode_bucket(
    value: &serde_json::Value,
) -> Result<(Vec<PrSummary>, Option<String>), GitHubError> {
    let nodes = value
        .get("nodes")
        .and_then(|n| n.as_array())
        .ok_or_else(|| GitHubError::Parse("dashboard: bucket nodes missing".into()))?;

    let mut seen: HashSet<PrId> = HashSet::with_capacity(nodes.len());
    let mut prs = Vec::with_capacity(nodes.len());
    for node in nodes {
        // GitHub's search returns `{}` for non-PR results when union fragments
        // don't match; the live query restricts to PullRequest, but we skip
        // empty nodes defensively rather than erroring.
        if node.as_object().is_some_and(serde_json::Map::is_empty) {
            continue;
        }
        let pr = pr_summary::decode(node, "dashboard")?;
        if seen.insert(pr.id.clone()) {
            prs.push(pr);
        }
    }

    let page_info = value
        .get("pageInfo")
        .ok_or_else(|| GitHubError::Parse("dashboard: pageInfo missing".into()))?;
    let has_next = page_info
        .get("hasNextPage")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let cursor = if has_next {
        page_info
            .get("endCursor")
            .and_then(|v| v.as_str())
            .map(std::string::ToString::to_string)
    } else {
        None
    };
    Ok((prs, cursor))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::models::{ChecksRollup, Mergeable, ReviewDecision};
    use serde_json::json;

    const FIXTURE: &str = include_str!("../../../../tests/fixtures/api/dashboard.graphql.json");

    fn fixture() -> serde_json::Value {
        serde_json::from_str(FIXTURE).expect("fixture parses as JSON")
    }

    #[test]
    fn decodes_recorded_fixture_into_three_buckets() {
        let result = decode(&fixture()).expect("decode succeeds");
        assert_eq!(result.len(), 3);
        assert_eq!(result[0].0, DashboardBucket::ReviewRequested);
        assert_eq!(result[0].1.len(), 5);
        assert_eq!(result[1].0, DashboardBucket::Authored);
        assert_eq!(result[1].1.len(), 2);
        assert_eq!(result[2].0, DashboardBucket::Assigned);
        assert_eq!(result[2].1.len(), 5);
        // reviewRequested has hasNextPage=true so cursor must be Some.
        assert!(result[0].2.is_some());
        // authored has hasNextPage=false so cursor must be None.
        assert!(result[1].2.is_none());
    }

    #[test]
    fn parses_review_decision_and_merge_state_enums() {
        let result = decode(&fixture()).expect("decode succeeds");
        let first = &result[0].1[0];
        assert_eq!(first.review_decision, Some(ReviewDecision::ReviewRequired));
        assert_eq!(first.mergeable, Mergeable::Blocked);
    }

    #[test]
    fn parses_checks_rollup_from_last_commit() {
        let result = decode(&fixture()).expect("decode succeeds");
        let first = &result[0].1[0];
        assert_eq!(first.checks, ChecksRollup::Success);
    }

    #[test]
    fn dedupes_within_bucket_only() {
        let node = json!({
            "number": 1,
            "title": "t",
            "state": "OPEN",
            "isDraft": false,
            "updatedAt": "2026-05-29T00:00:00Z", "createdAt": "2026-05-29T00:00:00Z",
            "author": { "login": "u" },
            "baseRefName": "main",
            "headRefName": "feat",
            "additions": 0, "deletions": 0, "changedFiles": 0,
            "totalCommentsCount": 0,
            "reviewDecision": null,
            "mergeStateStatus": "CLEAN",
            "repository": { "nameWithOwner": "o/r" },
            "labels": { "nodes": [] },
            "commits": { "nodes": [] }
        });
        let payload = json!({
            "data": {
                "reviewRequested": {
                    "pageInfo": { "hasNextPage": false, "endCursor": null },
                    "nodes": [node, node]
                },
                "authored": {
                    "pageInfo": { "hasNextPage": false, "endCursor": null },
                    "nodes": [node]
                },
                "assigned": {
                    "pageInfo": { "hasNextPage": false, "endCursor": null },
                    "nodes": []
                }
            }
        });
        let result = decode(&payload).expect("decode succeeds");
        assert_eq!(result[0].1.len(), 1, "intra-bucket dedup");
        assert_eq!(result[1].1.len(), 1, "cross-bucket presence preserved");
    }

    #[test]
    fn missing_required_field_surfaces_parse_error() {
        let node = json!({
            // title intentionally missing
            "number": 1,
            "state": "OPEN",
            "isDraft": false,
            "updatedAt": "2026-05-29T00:00:00Z", "createdAt": "2026-05-29T00:00:00Z",
            "author": { "login": "u" },
            "baseRefName": "main",
            "headRefName": "feat",
            "additions": 0, "deletions": 0, "changedFiles": 0,
            "totalCommentsCount": 0,
            "reviewDecision": null,
            "mergeStateStatus": "CLEAN",
            "repository": { "nameWithOwner": "o/r" },
            "labels": { "nodes": [] },
            "commits": { "nodes": [] }
        });
        let payload = json!({
            "data": {
                "reviewRequested": {
                    "pageInfo": { "hasNextPage": false, "endCursor": null },
                    "nodes": [node]
                },
                "authored": { "pageInfo": { "hasNextPage": false, "endCursor": null }, "nodes": [] },
                "assigned": { "pageInfo": { "hasNextPage": false, "endCursor": null }, "nodes": [] }
            }
        });
        let err = decode(&payload).expect_err("decode fails");
        match err {
            GitHubError::Parse(msg) => assert!(msg.contains("title"), "msg was: {msg}"),
            other => panic!("expected Parse, got {other:?}"),
        }
    }

    #[test]
    fn null_review_decision_maps_to_none() {
        let node = json!({
            "number": 1,
            "title": "t",
            "state": "OPEN",
            "isDraft": false,
            "updatedAt": "2026-05-29T00:00:00Z", "createdAt": "2026-05-29T00:00:00Z",
            "author": { "login": "u" },
            "baseRefName": "main",
            "headRefName": "feat",
            "additions": 0, "deletions": 0, "changedFiles": 0,
            "totalCommentsCount": 0,
            "reviewDecision": null,
            "mergeStateStatus": "CLEAN",
            "repository": { "nameWithOwner": "o/r" },
            "labels": { "nodes": [] },
            "commits": { "nodes": [] }
        });
        let payload = json!({
            "data": {
                "reviewRequested": {
                    "pageInfo": { "hasNextPage": false, "endCursor": null },
                    "nodes": [node]
                },
                "authored": { "pageInfo": { "hasNextPage": false, "endCursor": null }, "nodes": [] },
                "assigned": { "pageInfo": { "hasNextPage": false, "endCursor": null }, "nodes": [] }
            }
        });
        let result = decode(&payload).expect("decode succeeds");
        assert_eq!(result[0].1.len(), 1);
        assert_eq!(result[0].1[0].review_decision, None);
    }
}
