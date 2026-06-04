//! Large-PR safeguards: Files-tab load gate, lazy per-file diff fetch,
//! 3000-cap truncation banner.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::TestBackend;

use gprr::app::effect::{DataEvent, Effect};
use gprr::app::event::AppEvent;
use gprr::app::state::{AppState, DetailTab, View};
use gprr::app::update::{LARGE_PR_FILE_THRESHOLD, update};
use gprr::data::github::fake::FakeFixtures;
use gprr::data::models::{
    ChecksRollup, FileDiff, FileStatus, Mergeable, PrDetail, PrFiles, PrId, PrState, PrSummary,
    Repo, ReviewDecision,
};
use gprr::ui::pr_detail::render_files;
use gprr::ui::theme::Theme;

use super::harness::fake_with;

fn sample_id() -> PrId {
    PrId {
        repo: Repo {
            owner: "acme".into(),
            name: "widgets".into(),
        },
        number: 9001,
    }
}

fn detail_with_changed_files(id: &PrId, changed_files: u32) -> PrDetail {
    PrDetail {
        summary: PrSummary {
            id: id.clone(),
            title: "huge".into(),
            is_draft: false,
            state: PrState::Open,
            author: "alice".into(),
            base: "main".into(),
            head: "huge".into(),
            additions: 100_000,
            deletions: 50_000,
            changed_files,
            comments: 0,
            review_decision: Some(ReviewDecision::ReviewRequired),
            mergeable: Mergeable::Clean,
            labels: vec![],
            updated_at: chrono::Utc::now(),
            created_at: chrono::Utc::now(),
            checks: ChecksRollup::Success,
        },
        body: String::new(),
        body_html: None,
        requested_reviewers: vec![],
        assignees: vec![],
        head_sha: "deadbeef".into(),
        merge_commit_sha: None,
        base_ref_oid: "cafe1234".into(),
        review_threads: vec![],
        timeline: vec![],
        available_merge_methods: vec![],
        version: 1,
    }
}

fn make_file(file_path: &str, patch: Option<&str>) -> FileDiff {
    FileDiff {
        path: file_path.into(),
        previous_path: None,
        status: FileStatus::Modified,
        additions: 1,
        deletions: 1,
        patch: patch.map(str::to_string),
        is_binary: false,
        is_submodule: false,
        is_generated: None,
        oversize: false,
        blob_sha_before: None,
        blob_sha_after: None,
    }
}

fn files_tab_state(id: &PrId, detail: PrDetail) -> AppState {
    let mut s = AppState::default();
    gprr::app::seed_workspace_from_view(
        &mut s,
        View::PrDetail {
            id: id.clone(),
            tab: DetailTab::Files,
        },
    );
    s.pr_details.insert(id.clone(), detail);
    s
}

async fn absorb_file_diff(
    s: &mut AppState,
    client: &gprr::data::SharedGitHubClient,
    id: &PrId,
    head_sha: &str,
    file_path: &str,
) {
    let fd = client
        .pr_file_diff(id, head_sha, file_path)
        .await
        .expect("fake pr_file_diff");
    update(
        s,
        &AppEvent::Data(DataEvent::PrFileDiffLoaded {
            id: id.clone(),
            head_sha: head_sha.to_string(),
            path: file_path.to_string(),
            result: Box::new(Ok(fd)),
        }),
    );
}

fn press_key(s: &mut AppState, code: KeyCode, mods: KeyModifiers) -> Vec<Effect> {
    update(s, &AppEvent::Key(KeyEvent::new(code, mods)))
}

#[tokio::test]
async fn large_pr_files_tab_does_not_fetch_until_l_pressed() {
    let id = sample_id();
    let detail = detail_with_changed_files(&id, LARGE_PR_FILE_THRESHOLD + 1);
    let mut s = files_tab_state(&id, detail);

    let effects = update(&mut s, &AppEvent::Tick);
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::FetchPrFiles { .. })),
        "Tick on Files tab for large PR must NOT fetch; got {effects:?}"
    );

    let l_effects = update(
        &mut s,
        &AppEvent::Key(KeyEvent::new(KeyCode::Char('L'), KeyModifiers::SHIFT)),
    );
    assert!(l_effects.is_empty());
    assert!(s.pr_files_load_requested.contains(&id));

    let tick_effects = update(&mut s, &AppEvent::Tick);
    let fetch_count = tick_effects
        .iter()
        .filter(|e| matches!(e, Effect::FetchPrFiles { id: i, .. } if i == &id))
        .count();
    assert_eq!(
        fetch_count, 1,
        "expected exactly one FetchPrFiles after L; got {tick_effects:?}"
    );
}

#[tokio::test]
async fn large_pr_l_triggers_load_and_lazy_per_file_fetch_on_scroll() {
    let id = sample_id();
    let detail = detail_with_changed_files(&id, LARGE_PR_FILE_THRESHOLD + 1);
    let head_sha = detail.head_sha.clone();

    let initial_files = PrFiles {
        files: vec![
            make_file("src/a.rs", None),
            make_file("src/b.rs", None),
            make_file("src/c.rs", None),
        ],
        truncated: false,
    };
    let b_loaded = make_file("src/b.rs", Some("@@ -1 +1 @@\n-x\n+y\n"));
    let c_loaded = make_file("src/c.rs", Some("@@ -1 +1 @@\n-p\n+q\n"));

    let fixtures = FakeFixtures {
        viewer: "octocat".into(),
        pr_detail: vec![(id.clone(), detail.clone())],
        pr_files: vec![((id.clone(), head_sha.clone()), initial_files.clone())],
        pr_file_diffs: vec![
            (
                (id.clone(), head_sha.clone(), "src/b.rs".to_string()),
                b_loaded.clone(),
            ),
            (
                (id.clone(), head_sha.clone(), "src/c.rs".to_string()),
                c_loaded.clone(),
            ),
        ],
        ..FakeFixtures::default()
    };
    let client = fake_with(fixtures);

    let mut s = files_tab_state(&id, detail);

    press_key(&mut s, KeyCode::Char('L'), KeyModifiers::SHIFT);
    let tick_effects = update(&mut s, &AppEvent::Tick);
    assert!(
        tick_effects
            .iter()
            .any(|e| matches!(e, Effect::FetchPrFiles { .. }))
    );
    let fetched = client
        .pr_files(&id, &head_sha)
        .await
        .expect("fake pr_files");
    update(
        &mut s,
        &AppEvent::Data(DataEvent::PrFilesLoaded {
            id: id.clone(),
            head_sha: head_sha.clone(),
            result: Box::new(Ok(fetched)),
        }),
    );
    assert_eq!(s.pr_files.get(&id).unwrap().files.len(), 3);
    assert!(s.pr_files.get(&id).unwrap().files[1].patch.is_none());

    let j_effects = press_key(&mut s, KeyCode::Char('j'), KeyModifiers::NONE);
    assert!(
        j_effects.iter().any(|e| matches!(
            e,
            Effect::FetchPrFileDiff { id: i, head_sha: h, path }
                if i == &id && h == &head_sha && path == "src/b.rs"
        )),
        "j must emit FetchPrFileDiff for src/b.rs; got {j_effects:?}"
    );
    absorb_file_diff(&mut s, &client, &id, &head_sha, "src/b.rs").await;
    assert!(s.pr_files.get(&id).unwrap().files[1].patch.is_some());

    let next_effects = press_key(&mut s, KeyCode::Char('j'), KeyModifiers::NONE);
    assert!(
        next_effects.iter().any(|e| matches!(
            e,
            Effect::FetchPrFileDiff { path, .. } if path == "src/c.rs"
        )),
        "second j must emit fetch for src/c.rs; got {next_effects:?}"
    );
    absorb_file_diff(&mut s, &client, &id, &head_sha, "src/c.rs").await;

    let k_effects = press_key(&mut s, KeyCode::Char('k'), KeyModifiers::NONE);
    assert!(
        !k_effects
            .iter()
            .any(|e| matches!(e, Effect::FetchPrFileDiff { .. })),
        "scrolling back to a loaded file must not re-fetch; got {k_effects:?}"
    );
}

#[tokio::test]
async fn truncated_files_tab_shows_3000_cap_banner() {
    let files = PrFiles {
        files: vec![make_file("src/only.rs", Some("@@ -1 +1 @@\n-a\n+b\n"))],
        truncated: true,
    };
    let theme = Theme::dark();
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| {
            #[allow(deprecated)]
            let area = f.area();
            render_files(f, area, &files, 0, &theme);
        })
        .unwrap();

    let buffer = terminal.backend().buffer();
    let mut first_row = String::new();
    for x in 0..buffer.area.width {
        first_row.push_str(buffer[(x, 0)].symbol());
    }
    assert!(
        first_row.contains("GitHub truncated the file list at 3000 files; some files are hidden."),
        "expected banner on first row, got: {first_row:?}"
    );
    let cell = &buffer[(0, 0)];
    assert_eq!(
        cell.fg, theme.warning,
        "banner first cell must use theme.warning; got {:?}",
        cell.fg
    );
}
