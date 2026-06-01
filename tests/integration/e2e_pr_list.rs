//! End-to-end `pr_list` happy-path with pagination.
//!
//! 1. Seed a 25-row `pr_list` fixture for `acme/widgets` (fake paginates at 20/page).
//! 2. Switch view to `PrList`, Tick → first `FetchPrList` → absorb page 1 (20 rows + cursor).
//! 3. Navigate selection to the bottom of page 1, send `M` (`LoadMore`) → second
//!    `FetchPrList` with stored cursor → absorb page 2 (5 rows, no cursor).
//! 4. Assert list grew from 20 → 25 and cursor cleared.

use chrono::TimeZone;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use gpr::app::App;
use gpr::app::effect::{DataEvent, Effect};
use gpr::app::event::AppEvent;
use gpr::app::state::{Selection, View};
use gpr::app::update::update;
use gpr::data::github::fake::FakeFixtures;
use gpr::data::models::{ChecksRollup, Mergeable, PrId, PrState, PrSummary, Repo, ReviewDecision};

use super::harness::fake_with;

fn pr(number: u64) -> PrSummary {
    PrSummary {
        id: PrId {
            repo: Repo {
                owner: "acme".into(),
                name: "widgets".into(),
            },
            number,
        },
        title: format!("pr #{number}"),
        is_draft: false,
        state: PrState::Open,
        author: "alice".into(),
        base: "main".into(),
        head: "feature".into(),
        additions: 1,
        deletions: 0,
        changed_files: 1,
        comments: 0,
        review_decision: Some(ReviewDecision::ReviewRequired),
        mergeable: Mergeable::Clean,
        labels: vec![],
        updated_at: chrono::Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
        created_at: chrono::Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
        checks: ChecksRollup::Success,
    }
}

#[tokio::test]
async fn pr_list_happy_path_with_pagination() {
    let repo = Repo {
        owner: "acme".into(),
        name: "widgets".into(),
    };
    let rows: Vec<PrSummary> = (1..=25).map(pr).collect();
    let fixtures = FakeFixtures {
        viewer: "octocat".into(),
        pr_list: vec![(repo.clone(), rows)],
        ..FakeFixtures::default()
    };
    let client = fake_with(fixtures);
    let mut app = App::new(client.clone());
    gpr::app::seed_workspace_from_view(&mut app.state, View::PrList { repo: repo.clone() });

    // ── Page 1 ────────────────────────────────────────────────────────────
    let effects = update(&mut app.state, &AppEvent::Tick);
    let fetch = effects
        .iter()
        .find_map(|e| match e {
            Effect::FetchPrList {
                repo: r,
                cursor,
                filter,
                sort,
            } if r == &repo => Some((cursor.clone(), *filter, *sort)),
            _ => None,
        })
        .expect("first tick must emit FetchPrList for the repo");
    assert_eq!(fetch.0, None, "page 1 has no cursor");
    assert_eq!(fetch.1, None);
    assert_eq!(fetch.2, None);
    assert!(
        app.state.pr_list_loading.contains(&repo),
        "loading flag set on emit"
    );

    let (page1, cursor1) = client
        .pr_list(&repo, fetch.0, fetch.1, fetch.2)
        .await
        .expect("page 1");
    assert_eq!(page1.len(), 20, "page size is 20");
    assert_eq!(
        cursor1.as_deref(),
        Some("20"),
        "cursor advances to offset 20"
    );

    update(
        &mut app.state,
        &AppEvent::Data(DataEvent::PrListLoaded {
            repo: repo.clone(),
            result: Ok(page1),
            next_cursor: cursor1,
            replace: true,
        }),
    );
    assert_eq!(app.state.pr_lists.get(&repo).map(Vec::len), Some(20));
    assert_eq!(
        app.state.pr_list_cursors.get(&repo).map(String::as_str),
        Some("20")
    );
    assert!(
        !app.state.pr_list_loading.contains(&repo),
        "loading flag cleared after PrListLoaded"
    );

    // ── Page 2 (triggered by LoadMore near end of list) ───────────────────
    *app.state.selection_mut() = Selection::PrListRow(15);
    let effects = update(
        &mut app.state,
        &AppEvent::Key(KeyEvent::new(KeyCode::Char('M'), KeyModifiers::SHIFT)),
    );
    let fetch2 = effects
        .iter()
        .find_map(|e| match e {
            Effect::FetchPrList {
                repo: r, cursor, ..
            } if r == &repo => Some(cursor.clone()),
            _ => None,
        })
        .expect("LoadMore near end of page 1 must emit FetchPrList");
    assert_eq!(fetch2.as_deref(), Some("20"), "page 2 uses stored cursor");

    let (page2, cursor2) = client
        .pr_list(&repo, fetch2, None, None)
        .await
        .expect("page 2");
    assert_eq!(page2.len(), 5, "remaining 5 rows on page 2");
    assert_eq!(cursor2, None, "no further pages");

    update(
        &mut app.state,
        &AppEvent::Data(DataEvent::PrListLoaded {
            repo: repo.clone(),
            result: Ok(page2),
            next_cursor: cursor2,
            replace: false,
        }),
    );
    assert_eq!(
        app.state.pr_lists.get(&repo).map(Vec::len),
        Some(25),
        "list grew to 25 after page 2 appended"
    );
    assert!(
        !app.state.pr_list_cursors.contains_key(&repo),
        "cursor cleared when next_cursor is None"
    );
}
