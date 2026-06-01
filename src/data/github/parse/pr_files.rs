//! REST `/repos/{owner}/{name}/pulls/{n}/files` page decoder.
//!
//! Maps one page of GitHub's REST file-list response into `Vec<FileDiff>`.
//! Pagination, retries, and 3000-file cap enforcement live in the trait
//! impl (`octocrab_impl`) — this module is intentionally pure.
//!
//! ## Fields the REST endpoint cannot supply
//! Four `FileDiff` fields are reported as honest defaults rather than
//! synthesized values, because the `/pulls/N/files` payload does not
//! carry enough information to compute them:
//!
//! * `is_binary` — REST does not flag binaries; detection requires a
//!   `HEAD raw_url` Content-Type probe. v1 reports `false` and the diff
//!   viewer falls back gracefully when `patch` is `None`.
//! * `is_submodule` — needs a `/contents` follow-up to inspect entry
//!   type. Deferred to v1.x.
//! * `is_generated` — Linguist classification is only exposed via the
//!   `/languages` family of endpoints, not per-file. Deferred to v1.x.
//! * `blob_sha_before` — REST returns only the after-side `sha`; the
//!   base-side blob requires a `/compare` call. Deferred to v1.x.
//!
//! `oversize` is inferred best-effort: a missing `patch` on a status
//! that *should* have one (anything except `removed` / `unmerged`) is
//! almost certainly GitHub truncating for size.

use crate::data::github::GitHubError;
use crate::data::models::{FileDiff, FileStatus};

pub fn decode_page(json: &serde_json::Value) -> Result<Vec<FileDiff>, GitHubError> {
    let arr = json
        .as_array()
        .ok_or_else(|| GitHubError::Parse("pr_files: expected array".into()))?;
    let mut out = Vec::with_capacity(arr.len());
    for entry in arr {
        let status_str = entry
            .get("status")
            .and_then(|v| v.as_str())
            .ok_or_else(|| GitHubError::Parse("pr_files: status missing".into()))?;
        // "unchanged" is REST noise (e.g. files only touched by a merge-
        // base shift); skip silently rather than invent a FileStatus for it.
        if status_str == "unchanged" {
            continue;
        }
        let status = parse_status(status_str)?;
        let path = entry
            .get("filename")
            .and_then(|v| v.as_str())
            .ok_or_else(|| GitHubError::Parse("pr_files: filename missing".into()))?
            .to_string();
        let previous_path = entry
            .get("previous_filename")
            .and_then(|v| v.as_str())
            .map(std::string::ToString::to_string);
        let additions = u32::try_from(
            entry
                .get("additions")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0),
        )
        .unwrap_or(u32::MAX);
        let deletions = u32::try_from(
            entry
                .get("deletions")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0),
        )
        .unwrap_or(u32::MAX);
        let patch_body = entry
            .get("patch")
            .and_then(|v| v.as_str())
            .map(std::string::ToString::to_string);
        let blob_sha_after = Some(
            entry
                .get("sha")
                .and_then(|v| v.as_str())
                .ok_or_else(|| GitHubError::Parse("pr_files: sha missing".into()))?
                .to_string(),
        );
        let oversize =
            patch_body.is_none() && !matches!(status, FileStatus::Removed | FileStatus::Unmerged);

        out.push(FileDiff {
            path,
            previous_path,
            status,
            additions,
            deletions,
            patch: patch_body,
            is_binary: false,
            is_submodule: false,
            is_generated: None,
            oversize,
            blob_sha_before: None,
            blob_sha_after,
        });
    }
    Ok(out)
}

fn parse_status(s: &str) -> Result<FileStatus, GitHubError> {
    Ok(match s {
        "added" => FileStatus::Added,
        "removed" => FileStatus::Removed,
        "modified" => FileStatus::Modified,
        "renamed" => FileStatus::Renamed,
        "copied" => FileStatus::Copied,
        "changed" => FileStatus::ChangedMode,
        "unmerged" => FileStatus::Unmerged,
        other => {
            return Err(GitHubError::Parse(format!(
                "pr_files: unknown status {other}"
            )));
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const FIXTURE: &str = include_str!("../../../../tests/fixtures/api/pr_files.json");

    #[test]
    fn decodes_recorded_fixture() {
        let v: serde_json::Value = serde_json::from_str(FIXTURE).unwrap();
        let files = decode_page(&v).unwrap();
        assert_eq!(files.len(), 1);
        let f = &files[0];
        assert_eq!(f.path, "README.md");
        assert_eq!(f.status, FileStatus::Modified);
        assert_eq!(f.additions, 2);
        assert_eq!(f.deletions, 0);
        assert!(f.patch.is_some());
        assert!(f.patch.as_ref().unwrap().starts_with("@@ -65,6 +65,8 @@"));
        assert_eq!(
            f.blob_sha_after,
            Some("f0961571b1041d18381a4a3958a319291d55be1e".into())
        );
        assert!(!f.is_binary);
        assert!(!f.is_submodule);
        assert!(f.is_generated.is_none());
        assert!(!f.oversize);
        assert!(f.blob_sha_before.is_none());
        assert!(f.previous_path.is_none());
    }

    #[test]
    fn status_unchanged_is_skipped() {
        let v = json!([
            {"sha": "a", "filename": "noop.rs", "status": "unchanged", "additions": 0, "deletions": 0},
            {"sha": "b", "filename": "real.rs", "status": "modified", "additions": 1, "deletions": 1, "patch": "@@"}
        ]);
        let files = decode_page(&v).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "real.rs");
    }

    #[test]
    fn status_unknown_surfaces_parse_error() {
        let v = json!([
            {"sha": "a", "filename": "x.rs", "status": "future_unknown_value"}
        ]);
        let err = decode_page(&v).unwrap_err();
        match err {
            GitHubError::Parse(m) => {
                assert!(m.contains("unknown status"), "got {m}");
                assert!(m.contains("future_unknown_value"), "got {m}");
            }
            other => panic!("expected Parse, got {other:?}"),
        }
    }

    #[test]
    fn renamed_file_carries_previous_path() {
        let v = json!([
            {"sha": "a", "filename": "new.rs", "previous_filename": "old.rs", "status": "renamed"}
        ]);
        let files = decode_page(&v).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].status, FileStatus::Renamed);
        assert_eq!(files[0].previous_path.as_deref(), Some("old.rs"));
    }

    #[test]
    fn oversize_inferred_when_patch_absent_on_modified() {
        let v = json!([
            {"sha": "a", "filename": "huge.bin", "status": "modified"}
        ]);
        let files = decode_page(&v).unwrap();
        assert!(files[0].oversize);
    }

    #[test]
    fn removed_with_no_patch_is_not_oversize() {
        let v = json!([
            {"sha": "a", "filename": "gone.rs", "status": "removed"}
        ]);
        let files = decode_page(&v).unwrap();
        assert!(!files[0].oversize);
    }

    #[test]
    fn missing_path_surfaces_parse_error() {
        let v = json!([
            {"sha": "a", "status": "modified"}
        ]);
        let err = decode_page(&v).unwrap_err();
        match err {
            GitHubError::Parse(m) => assert!(m.contains("filename"), "got {m}"),
            other => panic!("expected Parse, got {other:?}"),
        }
    }

    #[test]
    fn all_seven_statuses_round_trip() {
        let v = json!([
            {"sha":"1","filename":"a","status":"added"},
            {"sha":"2","filename":"b","status":"removed"},
            {"sha":"3","filename":"c","status":"modified"},
            {"sha":"4","filename":"d","status":"renamed"},
            {"sha":"5","filename":"e","status":"copied"},
            {"sha":"6","filename":"f","status":"changed"},
            {"sha":"7","filename":"g","status":"unmerged"}
        ]);
        let files = decode_page(&v).unwrap();
        assert_eq!(files.len(), 7);
        assert_eq!(
            files.iter().map(|f| f.status).collect::<Vec<_>>(),
            vec![
                FileStatus::Added,
                FileStatus::Removed,
                FileStatus::Modified,
                FileStatus::Renamed,
                FileStatus::Copied,
                FileStatus::ChangedMode,
                FileStatus::Unmerged,
            ]
        );
    }

    #[test]
    fn additions_deletions_default_to_zero_when_absent() {
        let v = json!([
            {"sha": "a", "filename": "x.rs", "status": "modified", "patch": "@@"}
        ]);
        let files = decode_page(&v).unwrap();
        assert_eq!(files[0].additions, 0);
        assert_eq!(files[0].deletions, 0);
    }

    #[test]
    fn non_array_top_level_errors() {
        let v = json!({"oops": "object not array"});
        let err = decode_page(&v).unwrap_err();
        match err {
            GitHubError::Parse(m) => assert_eq!(m, "pr_files: expected array"),
            other => panic!("expected Parse, got {other:?}"),
        }
    }
}
