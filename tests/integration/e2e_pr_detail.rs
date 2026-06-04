//! End-to-end PR-detail landing flow plus per-tab render snapshots.
//!
//! 1. Seed fixtures for one PR with body, timeline, files, and checks.
//! 2. Set view to `PrDetail` and Tick → assert the reducer emits both
//!    `FetchPrDetail` and `FetchPrChecks`.
//! 3. Execute both effects through the fake client and feed the results back
//!    via `DataEvent::PrDetailLoaded` / `PrChecksLoaded`; absorbers must
//!    populate `state.pr_details` and `state.pr_checks`.
//! 4. Manually drive `FetchPrFiles` + `PrFilesLoaded` (no reducer auto-fetch
//!    yet for the Files tab).
//! 5. Render each tab into a `TestBackend` and snapshot the buffer.

use chrono::TimeZone;
use ratatui::Terminal;
use ratatui::backend::TestBackend;

use gprr::app::App;
use gprr::app::effect::{DataEvent, Effect};
use gprr::app::event::AppEvent;
use gprr::app::update::update;
use gprr::data::github::fake::FakeFixtures;
use gprr::data::models::{
    ChecksRollup, FileDiff, FileStatus, Label, Mergeable, PrChecks, PrDetail, PrFiles, PrId,
    PrState, PrSummary, Repo, ReviewDecision, TimelineEvent,
};
use gprr::ui::pr_detail::{
    ConversationScroll, render_checks, render_commits, render_conversation, render_files,
};
use gprr::ui::theme::Theme;

use super::harness::fake_with;

fn sample_summary(id: &PrId) -> PrSummary {
    PrSummary {
        id: id.clone(),
        title: "feat: add export".into(),
        is_draft: false,
        state: PrState::Open,
        author: "alice".into(),
        base: "main".into(),
        head: "feature".into(),
        additions: 42,
        deletions: 7,
        changed_files: 3,
        comments: 1,
        review_decision: Some(ReviewDecision::ReviewRequired),
        mergeable: Mergeable::Clean,
        labels: vec![Label {
            name: "enhancement".into(),
            color: "00ff00".into(),
        }],
        updated_at: chrono::Utc.with_ymd_and_hms(2026, 5, 1, 12, 0, 0).unwrap(),
        created_at: chrono::Utc.with_ymd_and_hms(2026, 5, 1, 10, 0, 0).unwrap(),
        checks: ChecksRollup::Success,
    }
}

fn sample_detail(id: &PrId) -> PrDetail {
    PrDetail {
        summary: sample_summary(id),
        body: "Adds CSV export to the report screen.".into(),
        body_html: None,
        requested_reviewers: vec!["bob".into()],
        assignees: vec!["alice".into()],
        head_sha: "deadbeef".into(),
        merge_commit_sha: None,
        base_ref_oid: "cafe1234".into(),
        review_threads: vec![],
        timeline: vec![TimelineEvent::Commit {
            sha: "abc1234".into(),
            message: "feat: add export".into(),
            author: "alice".into(),
            pushed_at: chrono::Utc.with_ymd_and_hms(2026, 5, 1, 10, 30, 0).unwrap(),
        }],
        available_merge_methods: vec![],
        version: 1,
    }
}

fn sample_files() -> PrFiles {
    PrFiles {
        files: vec![FileDiff {
            path: "src/export.rs".into(),
            previous_path: None,
            status: FileStatus::Added,
            additions: 42,
            deletions: 0,
            patch: Some("@@ -0,0 +1,2 @@\n+pub fn export() {}\n+pub fn helper() {}\n".into()),
            is_binary: false,
            is_submodule: false,
            is_generated: None,
            oversize: false,
            blob_sha_before: None,
            blob_sha_after: Some("abc1234".into()),
        }],
        truncated: false,
    }
}

fn sample_checks() -> PrChecks {
    PrChecks {
        runs: vec![],
        statuses: vec![],
    }
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn pr_detail_landing_fetches_and_absorbs_detail_and_checks() {
    let id = PrId {
        repo: Repo {
            owner: "acme".into(),
            name: "widgets".into(),
        },
        number: 101,
    };
    let detail = sample_detail(&id);
    let checks = sample_checks();

    let fixtures = FakeFixtures {
        viewer: "octocat".into(),
        pr_detail: vec![(id.clone(), detail.clone())],
        pr_checks: vec![(id.clone(), checks.clone())],
        ..FakeFixtures::default()
    };
    let client = fake_with(fixtures);
    let mut app = App::new(client.clone());
    gprr::app::seed_workspace_from_view(
        &mut app.state,
        gprr::app::state::View::PrDetail {
            id: id.clone(),
            tab: gprr::app::state::DetailTab::Conversation,
        },
    );

    // ── 1. Tick emits both FetchPrDetail and FetchPrChecks ───────────────
    let effects = update(&mut app.state, &AppEvent::Tick);
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::FetchPrDetail { id: i } if i == &id)),
        "tick on PrDetail must emit FetchPrDetail",
    );
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::FetchPrChecks { id: i } if i == &id)),
        "tick on PrDetail must emit FetchPrChecks",
    );

    // ── 2. Execute fetches via fake and absorb DataEvents ────────────────
    let fetched_detail = client.pr_detail(&id).await.expect("fake pr_detail");
    update(
        &mut app.state,
        &AppEvent::Data(DataEvent::PrDetailLoaded {
            id: id.clone(),
            result: Box::new(Ok(fetched_detail)),
        }),
    );
    let fetched_checks = client.pr_checks(&id).await.expect("fake pr_checks");
    update(
        &mut app.state,
        &AppEvent::Data(DataEvent::PrChecksLoaded {
            id: id.clone(),
            result: Ok(fetched_checks),
        }),
    );
    assert!(
        app.state.pr_details.contains_key(&id),
        "PrDetailLoaded absorbed into pr_details",
    );
    assert!(
        app.state.pr_checks.contains_key(&id),
        "PrChecksLoaded absorbed into pr_checks",
    );

    // ── 3. Drive PrFiles fetch manually (no reducer auto-fetch yet) ──────
    let files = sample_files();
    let head_sha = detail.head_sha.clone();
    let fixtures_with_files = FakeFixtures {
        viewer: "octocat".into(),
        pr_detail: vec![(id.clone(), detail.clone())],
        pr_checks: vec![(id.clone(), checks.clone())],
        pr_files: vec![((id.clone(), head_sha.clone()), files.clone())],
        ..FakeFixtures::default()
    };
    let files_client = fake_with(fixtures_with_files);
    let fetched_files = files_client
        .pr_files(&id, &head_sha)
        .await
        .expect("fake pr_files");
    update(
        &mut app.state,
        &AppEvent::Data(DataEvent::PrFilesLoaded {
            id: id.clone(),
            head_sha: head_sha.clone(),
            result: Box::new(Ok(fetched_files)),
        }),
    );
    assert!(
        app.state.pr_files.contains_key(&id),
        "PrFilesLoaded absorbed into pr_files",
    );

    // ── 4. Render each tab and snapshot ──────────────────────────────────
    let theme = Theme::dark();
    let detail_ref = app.state.pr_details.get(&id).expect("detail").clone();
    let files_ref = app.state.pr_files.get(&id).expect("files").clone();
    let checks_ref = app.state.pr_checks.get(&id).expect("checks").clone();

    let snapshot_tab = |label: &str, draw: &dyn Fn(&mut ratatui::Frame, ratatui::layout::Rect)| {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                #[allow(deprecated)]
                let size = f.area();
                draw(f, size);
            })
            .unwrap();
        insta::assert_snapshot!(label, terminal.backend());
    };

    let fixed_now = chrono::Utc.with_ymd_and_hms(2026, 6, 1, 0, 0, 0).unwrap();
    snapshot_tab("pr_detail_conversation_tab", &|f, area| {
        render_conversation(
            f,
            area,
            &detail_ref,
            None,
            ConversationScroll {
                cursor: 0,
                scroll_to_focused: false,
                now: fixed_now,
            },
            &theme,
        );
    });
    snapshot_tab("pr_detail_files_tab", &|f, area| {
        render_files(f, area, &files_ref, 0, &theme);
    });
    snapshot_tab("pr_detail_checks_tab", &|f, area| {
        render_checks(f, area, &checks_ref, &theme);
    });
    snapshot_tab("pr_detail_commits_tab", &|f, area| {
        render_commits(f, area, &detail_ref, 0, &theme);
    });
}
