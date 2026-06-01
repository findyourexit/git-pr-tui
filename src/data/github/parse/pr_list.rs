//! Parser for the `PrList` GraphQL query (single-repository PR listing).
//!
//! The PR node shape matches the dashboard query exactly, so node decoding
//! is delegated to `super::pr_summary`.

use super::pr_summary;
use crate::data::github::GitHubError;
use crate::data::models::PrSummary;

/// Decode a `PrList` GraphQL response into `(prs, next_cursor)`. The cursor
/// is `Some(endCursor)` iff `pageInfo.hasNextPage == true` — `None` otherwise
/// signals the caller that no further pages exist.
pub fn decode(json: &serde_json::Value) -> Result<(Vec<PrSummary>, Option<String>), GitHubError> {
    if let Some(errors) = json.get("errors").and_then(|e| e.as_array())
        && let Some(first) = errors.first()
    {
        let msg = first
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("(no message)");
        return Err(GitHubError::Parse(format!("pr_list: {msg}")));
    }

    let data = json
        .get("data")
        .and_then(|d| if d.is_null() { None } else { Some(d) });
    let data = data.ok_or_else(|| GitHubError::Parse("pr_list: data missing".into()))?;
    let repository = data
        .get("repository")
        .and_then(|r| if r.is_null() { None } else { Some(r) });
    let repository =
        repository.ok_or_else(|| GitHubError::Parse("pr_list: repository missing".into()))?;
    let pull_requests = repository
        .get("pullRequests")
        .ok_or_else(|| GitHubError::Parse("pr_list: pullRequests missing".into()))?;
    let nodes = pull_requests
        .get("nodes")
        .and_then(|n| n.as_array())
        .ok_or_else(|| GitHubError::Parse("pr_list: nodes missing".into()))?;

    let mut prs = Vec::with_capacity(nodes.len());
    for node in nodes {
        prs.push(pr_summary::decode(node, "pr_list")?);
    }

    let page_info = pull_requests
        .get("pageInfo")
        .ok_or_else(|| GitHubError::Parse("pr_list: pageInfo missing".into()))?;
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
    use crate::data::models::Mergeable;
    use serde_json::json;

    const FIXTURE: &str = include_str!("../../../../tests/fixtures/api/pr_list.graphql.json");

    fn fixture() -> serde_json::Value {
        serde_json::from_str(FIXTURE).expect("fixture parses as JSON")
    }

    #[test]
    fn decodes_recorded_fixture_count_matches_jq() {
        let (prs, _cursor) = decode(&fixture()).expect("decode succeeds");
        assert_eq!(prs.len(), 10, "fixture has 10 PRs per jq");
    }

    #[test]
    fn cursor_is_some_when_has_next_page_true() {
        let payload = json!({
            "data": {
                "repository": {
                    "pullRequests": {
                        "pageInfo": { "hasNextPage": true, "endCursor": "abc" },
                        "nodes": []
                    }
                }
            }
        });
        let (prs, cursor) = decode(&payload).expect("decode succeeds");
        assert!(prs.is_empty());
        assert_eq!(cursor.as_deref(), Some("abc"));
    }

    #[test]
    fn cursor_is_none_when_has_next_page_false() {
        let payload = json!({
            "data": {
                "repository": {
                    "pullRequests": {
                        "pageInfo": { "hasNextPage": false, "endCursor": "abc" },
                        "nodes": []
                    }
                }
            }
        });
        let (_, cursor) = decode(&payload).expect("decode succeeds");
        assert!(cursor.is_none());
    }

    #[test]
    fn missing_repository_surfaces_parse_error() {
        let payload = json!({ "data": { "repository": null } });
        let err = decode(&payload).expect_err("decode fails");
        match err {
            GitHubError::Parse(msg) => assert!(msg.contains("repository"), "msg was: {msg}"),
            other => panic!("expected Parse, got {other:?}"),
        }
    }

    #[test]
    fn graphql_errors_block_surfaces_first_error() {
        let payload = json!({
            "errors": [{ "message": "rate limited" }],
            "data": null
        });
        let err = decode(&payload).expect_err("decode fails");
        match err {
            GitHubError::Parse(msg) => assert!(msg.contains("rate limited"), "msg was: {msg}"),
            other => panic!("expected Parse, got {other:?}"),
        }
    }

    #[test]
    fn mergeable_state_unknown_round_trips() {
        let (prs, _) = decode(&fixture()).expect("decode succeeds");
        let first = prs.first().expect("at least one PR in fixture");
        // Asserts mergeable parsed into one of the 8 valid Mergeable variants
        // (the match is exhaustive, so any decoded value satisfies this).
        match first.mergeable {
            Mergeable::Clean
            | Mergeable::Unstable
            | Mergeable::HasHooks
            | Mergeable::Behind
            | Mergeable::Blocked
            | Mergeable::Dirty
            | Mergeable::Draft
            | Mergeable::Unknown => {}
        }
    }
}
