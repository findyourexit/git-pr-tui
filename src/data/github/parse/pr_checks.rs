//! REST `/commits/{ref}/check-runs` and `/commits/{ref}/status` page decoders.
//!
//! Pure mappers: GitHub's two check endpoints → `Vec<CheckRun>` and
//! `Vec<CommitStatus>`. HTTP, retries, and concurrency live in the trait
//! impl (`octocrab_impl`).

use crate::data::github::GitHubError;
use crate::data::models::{CheckConclusion, CheckRun, CheckStatus, CommitStatus, StatusState};
use chrono::{DateTime, Utc};

pub fn decode_runs(json: &serde_json::Value) -> Result<Vec<CheckRun>, GitHubError> {
    let arr = json
        .get("check_runs")
        .and_then(|v| v.as_array())
        .ok_or_else(|| GitHubError::Parse("pr_checks: check_runs missing or not array".into()))?;
    let mut out = Vec::with_capacity(arr.len());
    for entry in arr {
        let name = entry
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| GitHubError::Parse("pr_checks: check_run.name missing".into()))?
            .to_string();
        // GitHub returns `app: null` for some legacy entries; honest empty default.
        let app = entry
            .get("app")
            .and_then(|a| a.get("slug"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let status_str = entry
            .get("status")
            .and_then(|v| v.as_str())
            .ok_or_else(|| GitHubError::Parse("pr_checks: check_run.status missing".into()))?;
        let status = match status_str {
            "queued" => CheckStatus::Queued,
            "in_progress" => CheckStatus::InProgress,
            "completed" => CheckStatus::Completed,
            s => {
                return Err(GitHubError::Parse(format!(
                    "pr_checks: unknown status {s:?}"
                )));
            }
        };
        let conclusion = match entry.get("conclusion").and_then(|v| v.as_str()) {
            None => None,
            Some("success") => Some(CheckConclusion::Success),
            Some("failure") => Some(CheckConclusion::Failure),
            Some("neutral") => Some(CheckConclusion::Neutral),
            Some("cancelled") => Some(CheckConclusion::Cancelled),
            Some("skipped") => Some(CheckConclusion::Skipped),
            Some("timed_out") => Some(CheckConclusion::TimedOut),
            Some("action_required") => Some(CheckConclusion::ActionRequired),
            Some("stale") => Some(CheckConclusion::Stale),
            Some(s) => {
                return Err(GitHubError::Parse(format!(
                    "pr_checks: unknown conclusion {s:?}"
                )));
            }
        };
        let url = entry
            .get("html_url")
            .and_then(|v| v.as_str())
            .or_else(|| entry.get("url").and_then(|v| v.as_str()))
            .unwrap_or("")
            .to_string();
        let started_at = parse_ts(entry.get("started_at"));
        let completed_at = parse_ts(entry.get("completed_at"));
        out.push(CheckRun {
            name,
            app,
            status,
            conclusion,
            url,
            started_at,
            completed_at,
        });
    }
    Ok(out)
}

pub fn decode_statuses(json: &serde_json::Value) -> Result<Vec<CommitStatus>, GitHubError> {
    let arr = json
        .get("statuses")
        .and_then(|v| v.as_array())
        .ok_or_else(|| GitHubError::Parse("pr_checks: statuses missing or not array".into()))?;
    let mut out = Vec::with_capacity(arr.len());
    for entry in arr {
        let context = entry
            .get("context")
            .and_then(|v| v.as_str())
            .ok_or_else(|| GitHubError::Parse("pr_checks: status.context missing".into()))?
            .to_string();
        let state_str = entry
            .get("state")
            .and_then(|v| v.as_str())
            .ok_or_else(|| GitHubError::Parse("pr_checks: status.state missing".into()))?;
        let state = match state_str {
            "pending" => StatusState::Pending,
            "success" => StatusState::Success,
            "failure" => StatusState::Failure,
            "error" => StatusState::Error,
            s => {
                return Err(GitHubError::Parse(format!(
                    "pr_checks: unknown status state {s:?}"
                )));
            }
        };
        let target_url = entry
            .get("target_url")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
        let description = entry
            .get("description")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
        out.push(CommitStatus {
            context,
            state,
            target_url,
            description,
        });
    }
    Ok(out)
}

fn parse_ts(v: Option<&serde_json::Value>) -> Option<DateTime<Utc>> {
    v.and_then(|x| x.as_str())
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&Utc))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load(rel: &str) -> serde_json::Value {
        let path = format!("{}/{}", env!("CARGO_MANIFEST_DIR"), rel);
        let s = std::fs::read_to_string(&path).unwrap();
        serde_json::from_str(&s).unwrap()
    }

    #[test]
    fn decode_runs_parses_fixture() {
        let json = load("tests/fixtures/api/pr_checks.runs.json");
        let runs = decode_runs(&json).unwrap();
        let expected_len = json["check_runs"].as_array().unwrap().len();
        assert_eq!(runs.len(), expected_len);
        assert_eq!(runs.len(), 21);
        let first = &runs[0];
        assert_eq!(first.name, "check-requirements / close-unmet-requirements");
        assert_eq!(first.app, "github-actions");
        assert_eq!(first.status, CheckStatus::Completed);
        assert_eq!(first.conclusion, Some(CheckConclusion::Skipped));
        assert!(first.url.starts_with("https://github.com/cli/cli/actions/"));
        assert!(first.started_at.is_some());
        assert!(first.completed_at.is_some());
        assert!(
            runs.iter()
                .any(|r| r.conclusion == Some(CheckConclusion::Success))
        );
        assert!(
            runs.iter()
                .any(|r| r.conclusion == Some(CheckConclusion::Skipped))
        );
    }

    #[test]
    fn decode_statuses_parses_empty_fixture() {
        let json = load("tests/fixtures/api/pr_checks.statuses.json");
        let statuses = decode_statuses(&json).unwrap();
        assert!(statuses.is_empty());
    }

    #[test]
    fn decode_runs_rejects_missing_check_runs() {
        let err = decode_runs(&serde_json::json!({"total_count": 0})).unwrap_err();
        assert!(matches!(err, GitHubError::Parse(_)));
    }

    #[test]
    fn decode_runs_rejects_unknown_status() {
        let json = serde_json::json!({
            "check_runs": [{
                "name": "x",
                "status": "bogus",
                "conclusion": null,
            }],
        });
        let err = decode_runs(&json).unwrap_err();
        match err {
            GitHubError::Parse(msg) => assert!(msg.contains("bogus"), "msg was: {msg}"),
            other => panic!("expected Parse, got {other:?}"),
        }
    }
}
