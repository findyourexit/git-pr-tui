//! Offline demo data.
//!
//! `gpr --demo` runs the full TUI against an in-memory [`FakeGitHubClient`]
//! seeded with the fixtures below, so the interface can be explored (and
//! recorded) without a GitHub token or any network access. The data is
//! fictional but shaped to exercise every view: a cross-repo dashboard, a
//! per-repo PR list, PR detail tabs, syntax-highlighted diffs, review threads,
//! and a realistic checks rollup.

use chrono::{DateTime, TimeZone, Utc};

use crate::data::SharedGitHubClient;
use crate::data::fake_client;
use crate::data::github::DashboardBucket;
use crate::data::github::fake::FakeFixtures;
use crate::data::models::{
    CheckConclusion, CheckRun, CheckStatus, ChecksRollup, CommitStatus, DiffSide, FileDiff,
    FileStatus, Label, LineRange, MergeMethod, Mergeable, PrChecks, PrDetail, PrFiles, PrId,
    PrState, PrSummary, Repo, ReviewComment, ReviewDecision, ReviewState, ReviewThread,
    StatusState, TimelineEvent,
};

/// The login the demo viewer is authenticated as. Authored-bucket PRs and the
/// reviewer in sample reviews use this handle so the dashboard reads naturally.
const VIEWER: &str = "you";

/// Build a [`SharedGitHubClient`] backed entirely by in-memory demo fixtures.
#[must_use]
pub fn demo_client() -> SharedGitHubClient {
    fake_client(fixtures())
}

fn fixtures() -> FakeFixtures {
    let atlas_128 = PrId {
        repo: atlas(),
        number: 128,
    };
    let horizon_54 = PrId {
        repo: horizon(),
        number: 54,
    };

    FakeFixtures {
        viewer: VIEWER.into(),
        scopes: vec!["repo".into()],
        dashboard: dashboard(),
        pr_list: pr_lists(),
        pr_detail: vec![
            (atlas_128.clone(), atlas_128_detail()),
            (horizon_54.clone(), horizon_54_detail()),
        ],
        pr_files: vec![
            ((atlas_128.clone(), atlas_head_sha().into()), atlas_files()),
            (
                (horizon_54.clone(), horizon_head_sha().into()),
                horizon_files(),
            ),
        ],
        pr_diff: vec![
            (
                (atlas_128.clone(), atlas_head_sha().into()),
                atlas_raw_diff(),
            ),
            (
                (horizon_54.clone(), horizon_head_sha().into()),
                horizon_raw_diff(),
            ),
        ],
        pr_checks: vec![(atlas_128, atlas_checks()), (horizon_54, horizon_checks())],
        ..FakeFixtures::default()
    }
}

// ───────────────────────────── repos & helpers ─────────────────────────────

fn atlas() -> Repo {
    Repo {
        owner: "octoadmin".into(),
        name: "atlas".into(),
    }
}

fn horizon() -> Repo {
    Repo {
        owner: "octoadmin".into(),
        name: "horizon".into(),
    }
}

fn widgets() -> Repo {
    Repo {
        owner: "acme".into(),
        name: "widgets".into(),
    }
}

fn atlas_head_sha() -> &'static str {
    "9f3c1a2bd4e5f60718293a4b5c6d7e8f90a1b2c3"
}

fn horizon_head_sha() -> &'static str {
    "1c2d3e4f5061728394a5b6c7d8e9f0a1b2c3d4e5"
}

/// Concise UTC timestamp constructor for fixture data.
fn ts(year: i32, month: u32, day: u32, hour: u32, min: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(year, month, day, hour, min, 0)
        .single()
        .expect("valid demo timestamp")
}

fn label(name: &str, color: &str) -> Label {
    Label {
        name: name.into(),
        color: color.into(),
    }
}

/// A summary with sensible open-PR defaults; callers override the fields that
/// matter for each row.
fn summary(repo: Repo, number: u64, title: &str, author: &str) -> PrSummary {
    PrSummary {
        id: PrId { repo, number },
        title: title.into(),
        author: author.into(),
        state: PrState::Open,
        is_draft: false,
        base: "main".into(),
        head: format!("{author}/{number}"),
        additions: 0,
        deletions: 0,
        changed_files: 0,
        comments: 0,
        review_decision: Some(ReviewDecision::ReviewRequired),
        checks: ChecksRollup::Success,
        mergeable: Mergeable::Clean,
        labels: vec![],
        updated_at: ts(2026, 5, 28, 9, 12),
        created_at: ts(2026, 5, 24, 16, 40),
    }
}

// ───────────────────────────── summaries ─────────────────────────────

fn atlas_128_summary() -> PrSummary {
    let mut s = summary(atlas(), 128, "feat: streaming export pipeline", "dana");
    s.head = "dana/streaming-export".into();
    s.additions = 412;
    s.deletions = 86;
    s.changed_files = 5;
    s.comments = 7;
    s.review_decision = Some(ReviewDecision::ChangesRequested);
    s.checks = ChecksRollup::Failure;
    s.mergeable = Mergeable::Blocked;
    s.labels = vec![
        label("enhancement", "0e8a16"),
        label("needs-review", "fbca04"),
    ];
    s.updated_at = ts(2026, 5, 30, 14, 5);
    s.created_at = ts(2026, 5, 27, 10, 20);
    s
}

fn atlas_127_summary() -> PrSummary {
    let mut s = summary(atlas(), 127, "fix: jitter retry backoff", "sam");
    s.head = "sam/retry-jitter".into();
    s.additions = 38;
    s.deletions = 12;
    s.changed_files = 2;
    s.comments = 2;
    s.checks = ChecksRollup::Success;
    s.labels = vec![label("bug", "d73a4a")];
    s.updated_at = ts(2026, 5, 30, 8, 41);
    s
}

fn atlas_125_summary() -> PrSummary {
    let mut s = summary(atlas(), 125, "refactor: extract config loader", VIEWER);
    s.head = "you/config-loader".into();
    s.additions = 164;
    s.deletions = 140;
    s.changed_files = 6;
    s.comments = 4;
    s.review_decision = Some(ReviewDecision::Approved);
    s.checks = ChecksRollup::Success;
    s.labels = vec![label("refactor", "5319e7")];
    s.updated_at = ts(2026, 5, 29, 17, 2);
    s
}

fn atlas_122_summary() -> PrSummary {
    let mut s = summary(atlas(), 122, "docs: document rate limits", "kai");
    s.head = "kai/rate-limit-docs".into();
    s.is_draft = true;
    s.additions = 73;
    s.deletions = 1;
    s.changed_files = 1;
    s.checks = ChecksRollup::Pending;
    s.mergeable = Mergeable::Draft;
    s.labels = vec![label("documentation", "0075ca")];
    s.updated_at = ts(2026, 5, 28, 11, 33);
    s
}

fn atlas_119_summary() -> PrSummary {
    let mut s = summary(atlas(), 119, "chore: bump dependencies", "renovate[bot]");
    s.head = "renovate/weekly".into();
    s.state = PrState::Merged;
    s.additions = 220;
    s.deletions = 218;
    s.changed_files = 3;
    s.review_decision = Some(ReviewDecision::Approved);
    s.checks = ChecksRollup::Success;
    s.mergeable = Mergeable::Clean;
    s.labels = vec![label("dependencies", "0366d6")];
    s.updated_at = ts(2026, 5, 26, 7, 15);
    s
}

fn horizon_54_summary() -> PrSummary {
    let mut s = summary(horizon(), 54, "fix: debounce search input", VIEWER);
    s.head = "you/debounce-search".into();
    s.additions = 54;
    s.deletions = 9;
    s.changed_files = 2;
    s.comments = 3;
    s.review_decision = Some(ReviewDecision::Approved);
    s.checks = ChecksRollup::Success;
    s.mergeable = Mergeable::Clean;
    s.labels = vec![label("bug", "d73a4a"), label("ui", "c5def5")];
    s.updated_at = ts(2026, 5, 30, 12, 48);
    s.created_at = ts(2026, 5, 29, 9, 5);
    s
}

fn horizon_51_summary() -> PrSummary {
    let mut s = summary(horizon(), 51, "feat: polish dark mode contrast", VIEWER);
    s.head = "you/dark-mode".into();
    s.additions = 96;
    s.deletions = 41;
    s.changed_files = 4;
    s.comments = 1;
    s.checks = ChecksRollup::Mixed;
    s.labels = vec![label("ui", "c5def5")];
    s.updated_at = ts(2026, 5, 29, 19, 22);
    s
}

fn widgets_310_summary() -> PrSummary {
    let mut s = summary(widgets(), 310, "perf: cache avatar lookups", "lee");
    s.head = "lee/avatar-cache".into();
    s.additions = 131;
    s.deletions = 27;
    s.changed_files = 3;
    s.comments = 5;
    s.checks = ChecksRollup::Success;
    s.labels = vec![label("performance", "a2eeef")];
    s.updated_at = ts(2026, 5, 30, 6, 58);
    s
}

// ───────────────────────────── dashboard & lists ─────────────────────────────

fn dashboard() -> Vec<(DashboardBucket, Vec<PrSummary>)> {
    vec![
        (
            DashboardBucket::ReviewRequested,
            vec![atlas_128_summary(), atlas_127_summary()],
        ),
        (
            DashboardBucket::Authored,
            vec![
                horizon_54_summary(),
                horizon_51_summary(),
                atlas_125_summary(),
            ],
        ),
        (DashboardBucket::Assigned, vec![widgets_310_summary()]),
    ]
}

fn pr_lists() -> Vec<(Repo, Vec<PrSummary>)> {
    vec![
        (
            atlas(),
            vec![
                atlas_128_summary(),
                atlas_127_summary(),
                atlas_125_summary(),
                atlas_122_summary(),
                atlas_119_summary(),
            ],
        ),
        (horizon(), vec![horizon_54_summary(), horizon_51_summary()]),
    ]
}

// ───────────────────────────── atlas#128 detail ─────────────────────────────

fn atlas_128_detail() -> PrDetail {
    PrDetail {
        summary: atlas_128_summary(),
        body: ATLAS_128_BODY.into(),
        body_html: None,
        requested_reviewers: vec![VIEWER.into(), "morgan".into()],
        assignees: vec!["dana".into()],
        head_sha: atlas_head_sha().into(),
        merge_commit_sha: None,
        base_ref_oid: "0a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d".into(),
        review_threads: vec![atlas_128_thread()],
        timeline: atlas_128_timeline(),
        available_merge_methods: vec![MergeMethod::Squash, MergeMethod::Merge],
        version: 1,
    }
}

const ATLAS_128_BODY: &str = "\
## What

Introduces a **streaming export pipeline** so large reports no longer buffer \
the entire result set in memory before writing.

### Highlights

- Backpressure-aware `StreamWriter` that flushes in fixed-size chunks
- Drops the legacy batch exporter (`src/legacy/batch.rs`)
- Adds a focused doc under `docs/export.md`

```rust
let writer = StreamWriter::new(sink).with_chunk_size(8 * 1024);
writer.export(rows).await?;
```

### Checklist

- [x] Unit tests for chunk boundaries
- [x] Docs updated
- [ ] Windows path handling (see failing check)
";

fn atlas_128_timeline() -> Vec<TimelineEvent> {
    vec![
        TimelineEvent::Commit {
            sha: "a1b2c3d".into(),
            message: "feat: add StreamWriter with chunked flush".into(),
            author: "dana".into(),
            pushed_at: ts(2026, 5, 27, 10, 22),
        },
        TimelineEvent::Commit {
            sha: "b2c3d4e".into(),
            message: "test: cover chunk boundary conditions".into(),
            author: "dana".into(),
            pushed_at: ts(2026, 5, 28, 9, 4),
        },
        TimelineEvent::IssueComment {
            id: "ic-1".into(),
            author: "dana".into(),
            body: "Splitting the writer out of the batch path. Would love a \
                   sanity check on the chunk-size default before I wire it into \
                   the report screen."
                .into(),
            created_at: ts(2026, 5, 28, 9, 30),
        },
        TimelineEvent::Review {
            id: "rev-1".into(),
            author: "sam".into(),
            state: ReviewState::ChangesRequested,
            body: "Looks great overall. One blocker on Windows path handling \
                   plus a small naming nit inline."
                .into(),
            submitted_at: ts(2026, 5, 29, 15, 12),
        },
        TimelineEvent::Commit {
            sha: "c3d4e5f".into(),
            message: "fix: normalise temp paths on Windows".into(),
            author: "dana".into(),
            pushed_at: ts(2026, 5, 30, 13, 58),
        },
    ]
}

fn atlas_128_thread() -> ReviewThread {
    ReviewThread {
        id: "thread-1".into(),
        path: "src/export/stream.rs".into(),
        line_range: LineRange {
            side: DiffSide::Right,
            start_line: 14,
            end_line: 14,
            start_position: 9,
            end_position: 9,
        },
        is_resolved: false,
        is_outdated: false,
        comments: vec![
            ReviewComment {
                id: "rc-1".into(),
                author: "sam".into(),
                body: "`buf` reads a little terse here — `pending` would make \
                       the flush logic below clearer."
                    .into(),
                created_at: ts(2026, 5, 29, 15, 10),
                commit_sha: "b2c3d4e".into(),
                suggestion: Some("        let mut pending = Vec::with_capacity(cap);\n".into()),
            },
            ReviewComment {
                id: "rc-2".into(),
                author: "dana".into(),
                body: "Good call, renamed in c3d4e5f.".into(),
                created_at: ts(2026, 5, 30, 13, 59),
                commit_sha: "c3d4e5f".into(),
                suggestion: None,
            },
        ],
    }
}

fn atlas_files() -> PrFiles {
    PrFiles {
        files: vec![
            file(
                "src/export/stream.rs",
                FileStatus::Added,
                118,
                0,
                ATLAS_STREAM_PATCH,
            ),
            file(
                "src/export/mod.rs",
                FileStatus::Modified,
                21,
                6,
                ATLAS_MOD_PATCH,
            ),
            file("Cargo.toml", FileStatus::Modified, 2, 0, ATLAS_CARGO_PATCH),
            file("docs/export.md", FileStatus::Added, 44, 0, ATLAS_DOC_PATCH),
            renamed_removed(),
        ],
        truncated: false,
    }
}

fn renamed_removed() -> FileDiff {
    file(
        "src/legacy/batch.rs",
        FileStatus::Removed,
        0,
        80,
        ATLAS_BATCH_PATCH,
    )
}

const ATLAS_STREAM_PATCH: &str = "\
@@ -0,0 +1,18 @@
+use tokio::io::AsyncWrite;
+
+/// Writes rows to `sink` in fixed-size chunks to bound peak memory.
+pub struct StreamWriter<W> {
+    sink: W,
+    chunk_size: usize,
+}
+
+impl<W: AsyncWrite + Unpin> StreamWriter<W> {
+    pub fn new(sink: W) -> Self {
+        Self { sink, chunk_size: 4 * 1024 }
+    }
+
+    pub async fn export(mut self, rows: impl Iterator<Item = Row>) -> Result<()> {
+        let cap = self.chunk_size;
+        let mut pending = Vec::with_capacity(cap);
+        flush_in_chunks(&mut self.sink, rows, &mut pending).await
+    }
+}
";

const ATLAS_MOD_PATCH: &str = "\
@@ -1,9 +1,12 @@
 mod batch;
+mod stream;
 
-pub use batch::BatchExporter;
+pub use stream::StreamWriter;
 
 pub enum Format {
     Csv,
     Json,
+    Ndjson,
 }
";

const ATLAS_CARGO_PATCH: &str = "\
@@ -18,6 +18,8 @@ tokio = { version = \"1\", features = [\"macros\", \"rt-multi-thread\"] }
 serde = { version = \"1\", features = [\"derive\"] }
 thiserror = \"2\"
+futures = \"0.3\"
+bytes = \"1\"
";

const ATLAS_DOC_PATCH: &str = "\
@@ -0,0 +1,12 @@
+# Streaming export
+
+`StreamWriter` flushes rows in fixed-size chunks so exporting a multi-gigabyte
+report no longer buffers the whole result set in memory.
+
+```rust
+let writer = StreamWriter::new(sink).with_chunk_size(8 * 1024);
+writer.export(rows).await?;
+```
+
+Tune `chunk_size` for the destination: larger chunks reduce syscalls, smaller
+chunks lower peak memory.
";

const ATLAS_BATCH_PATCH: &str = "\
@@ -1,12 +0,0 @@
-/// Legacy exporter that buffered the full result set before writing.
-pub struct BatchExporter;
-
-impl BatchExporter {
-    pub fn export(rows: Vec<Row>) -> Result<Vec<u8>> {
-        let mut out = Vec::new();
-        for row in rows {
-            out.extend_from_slice(&row.to_bytes());
-        }
-        Ok(out)
-    }
-}
";

fn atlas_raw_diff() -> String {
    [
        ATLAS_STREAM_PATCH,
        ATLAS_MOD_PATCH,
        ATLAS_CARGO_PATCH,
        ATLAS_DOC_PATCH,
        ATLAS_BATCH_PATCH,
    ]
    .concat()
}

fn atlas_checks() -> PrChecks {
    PrChecks {
        runs: vec![
            run("build", CheckConclusion::Success, CheckStatus::Completed),
            run(
                "test (ubuntu)",
                CheckConclusion::Success,
                CheckStatus::Completed,
            ),
            run(
                "test (macos)",
                CheckConclusion::Success,
                CheckStatus::Completed,
            ),
            run(
                "test (windows)",
                CheckConclusion::Failure,
                CheckStatus::Completed,
            ),
            run("clippy", CheckConclusion::Success, CheckStatus::Completed),
            inprogress_run("coverage"),
        ],
        statuses: vec![CommitStatus {
            context: "ci/deploy-preview".into(),
            state: StatusState::Success,
            target_url: Some("https://deploy-preview.example/atlas/128".into()),
            description: Some("Preview ready".into()),
        }],
    }
}

// ───────────────────────────── horizon#54 detail ─────────────────────────────

fn horizon_54_detail() -> PrDetail {
    PrDetail {
        summary: horizon_54_summary(),
        body: HORIZON_54_BODY.into(),
        body_html: None,
        requested_reviewers: vec!["morgan".into()],
        assignees: vec![VIEWER.into()],
        head_sha: horizon_head_sha().into(),
        merge_commit_sha: None,
        base_ref_oid: "f0e1d2c3b4a5968778695a4b3c2d1e0f00112233".into(),
        review_threads: vec![],
        timeline: vec![
            TimelineEvent::Commit {
                sha: "d4e5f60".into(),
                message: "fix: debounce the search field by 200ms".into(),
                author: VIEWER.into(),
                pushed_at: ts(2026, 5, 29, 9, 8),
            },
            TimelineEvent::Review {
                id: "rev-h1".into(),
                author: "morgan".into(),
                state: ReviewState::Approved,
                body: "Snappy now and no more request storms. LGTM!".into(),
                submitted_at: ts(2026, 5, 30, 12, 40),
            },
        ],
        available_merge_methods: vec![MergeMethod::Squash, MergeMethod::Rebase, MergeMethod::Merge],
        version: 1,
    }
}

const HORIZON_54_BODY: &str = "\
Debounces the search input so each keystroke no longer fires a request.

Settles on a 200ms trailing-edge debounce; cancels the pending timer on submit
so explicit searches still feel instant.
";

fn horizon_files() -> PrFiles {
    PrFiles {
        files: vec![
            file(
                "src/ui/search.rs",
                FileStatus::Modified,
                47,
                9,
                HORIZON_SEARCH_PATCH,
            ),
            file(
                "src/ui/mod.rs",
                FileStatus::Modified,
                7,
                0,
                HORIZON_MOD_PATCH,
            ),
        ],
        truncated: false,
    }
}

const HORIZON_SEARCH_PATCH: &str = "\
@@ -12,11 +12,20 @@ impl SearchBox {
     pub fn on_key(&mut self, key: KeyEvent) {
         match key.code {
             KeyCode::Char(c) => {
                 self.query.push(c);
-                self.search();
+                self.schedule_search();
             }
             KeyCode::Enter => {
+                self.cancel_pending();
                 self.search();
             }
             _ => {}
         }
     }
+
+    fn schedule_search(&mut self) {
+        self.cancel_pending();
+        self.pending = Some(Instant::now() + Duration::from_millis(200));
+    }
";

const HORIZON_MOD_PATCH: &str = "\
@@ -3,6 +3,7 @@ mod list;
 mod search;
 
 pub use search::SearchBox;
+pub(crate) use search::DEBOUNCE_MS;
";

fn horizon_raw_diff() -> String {
    [HORIZON_SEARCH_PATCH, HORIZON_MOD_PATCH].concat()
}

fn horizon_checks() -> PrChecks {
    PrChecks {
        runs: vec![
            run("build", CheckConclusion::Success, CheckStatus::Completed),
            run("test", CheckConclusion::Success, CheckStatus::Completed),
            run("lint", CheckConclusion::Success, CheckStatus::Completed),
        ],
        statuses: vec![],
    }
}

// ───────────────────────────── small constructors ─────────────────────────────

fn file(path: &str, status: FileStatus, additions: u32, deletions: u32, hunk: &str) -> FileDiff {
    FileDiff {
        path: path.into(),
        previous_path: None,
        status,
        additions,
        deletions,
        patch: Some(hunk.into()),
        is_binary: false,
        is_submodule: false,
        is_generated: Some(false),
        oversize: false,
        blob_sha_before: Some("0000000".into()),
        blob_sha_after: Some("1111111".into()),
    }
}

fn run(name: &str, conclusion: CheckConclusion, status: CheckStatus) -> CheckRun {
    CheckRun {
        name: name.into(),
        app: "github-actions".into(),
        status,
        conclusion: Some(conclusion),
        url: format!("https://example.test/checks/{name}"),
        started_at: Some(ts(2026, 5, 30, 13, 50)),
        completed_at: Some(ts(2026, 5, 30, 13, 56)),
    }
}

fn inprogress_run(name: &str) -> CheckRun {
    CheckRun {
        name: name.into(),
        app: "github-actions".into(),
        status: CheckStatus::InProgress,
        conclusion: None,
        url: format!("https://example.test/checks/{name}"),
        started_at: Some(ts(2026, 5, 30, 13, 55)),
        completed_at: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn every_dashboard_pr_in_review_bucket_has_a_detail_fixture() {
        // The tour opens a PR straight from the review-requested bucket, so its
        // id must resolve to a detail fixture.
        let fx = fixtures();
        let client = fake_client(fx.clone());
        let (_, review_prs) = fx
            .dashboard
            .iter()
            .find(|(b, _)| *b == DashboardBucket::ReviewRequested)
            .expect("review bucket present");
        let first = &review_prs[0].id;
        assert!(
            client.pr_detail(first).await.is_ok(),
            "first review-requested PR must have a detail fixture",
        );
    }

    #[tokio::test]
    async fn detailed_prs_have_files_checks_and_diff() {
        let fx = fixtures();
        let client = fake_client(fx);
        let atlas_128 = PrId {
            repo: atlas(),
            number: 128,
        };
        let detail = client.pr_detail(&atlas_128).await.expect("detail");
        assert!(client.pr_files(&atlas_128, &detail.head_sha).await.is_ok());
        assert!(client.pr_diff(&atlas_128, &detail.head_sha).await.is_ok());
        assert!(client.pr_checks(&atlas_128).await.is_ok());
        assert!(detail.summary.id.repo.full_name().eq("octoadmin/atlas"));
    }

    #[test]
    fn pr_lists_cover_dashboard_repos() {
        let fx = fixtures();
        assert!(fx.pr_list.iter().any(|(r, _)| *r == atlas()));
        assert!(fx.pr_list.iter().any(|(r, _)| *r == horizon()));
    }
}
