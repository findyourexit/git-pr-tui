//! End-to-end happy-path: reducer fan-out → fake client → reducer absorb → render.
//!
//! Exercises the full M5 dashboard flow without touching real I/O:
//! 1. Construct `App` over `FakeGitHubClient` with seeded fixtures.
//! 2. Drive `AppEvent::Tick` through the reducer; assert it emits
//!    `Effect::FetchDashboard { bucket: None, cursor: None }`.
//! 3. Execute the effect against the fake client.
//! 4. Feed the result back as `AppEvent::Data(DataEvent::DashboardLoaded(Ok(_)))`.
//! 5. Render `render_dashboard_state` to a `TestBackend` and snapshot the buffer.

use chrono::TimeZone;
use ratatui::Terminal;
use ratatui::backend::TestBackend;

use gpr::app::App;
use gpr::app::effect::{DataEvent, Effect};
use gpr::app::event::AppEvent;
use gpr::app::update::update;
use gpr::data::github::fake::FakeFixtures;
use gpr::data::github::{DashboardBucket, DashboardRequest};
use gpr::data::models::{ChecksRollup, Mergeable, PrId, PrState, PrSummary, Repo, ReviewDecision};
use gpr::ui::dashboard::{DashboardState, render_dashboard_state};
use gpr::ui::theme::Theme;

use super::harness::fake_with;

fn pr(number: u64, title: &str, author: &str) -> PrSummary {
    PrSummary {
        id: PrId {
            repo: Repo {
                owner: "acme".into(),
                name: "widgets".into(),
            },
            number,
        },
        title: title.into(),
        is_draft: false,
        state: PrState::Open,
        author: author.into(),
        base: "main".into(),
        head: "feature".into(),
        additions: 10,
        deletions: 2,
        changed_files: 1,
        comments: 3,
        review_decision: Some(ReviewDecision::ReviewRequired),
        mergeable: Mergeable::Clean,
        labels: vec![],
        updated_at: chrono::Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
        created_at: chrono::Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
        checks: ChecksRollup::Success,
    }
}

#[tokio::test]
async fn dashboard_happy_path_tick_fetch_render() {
    // 1. Seed fixtures: one PR in each bucket.
    let fixtures = FakeFixtures {
        viewer: "octocat".into(),
        dashboard: vec![
            (
                DashboardBucket::ReviewRequested,
                vec![pr(101, "fix: handle null user", "alice")],
            ),
            (
                DashboardBucket::Authored,
                vec![pr(202, "feat: add export", "octocat")],
            ),
            (DashboardBucket::Assigned, vec![]),
        ],
        ..FakeFixtures::default()
    };
    let client = fake_with(fixtures);
    let mut app = App::new(client.clone());

    // 2. First tick on Dashboard view emits FetchDashboard.
    let effects = update(&mut app.state, &AppEvent::Tick);
    let fetch = effects
        .iter()
        .find(|e| matches!(e, Effect::FetchDashboard { .. }))
        .expect("first tick must emit FetchDashboard");
    let (bucket, cursor) = match fetch {
        Effect::FetchDashboard { bucket, cursor } => (*bucket, cursor.clone()),
        _ => unreachable!(),
    };
    assert!(bucket.is_none(), "first fetch is for all buckets");
    assert!(cursor.is_none(), "first fetch has no cursor");

    // 3. Execute the effect via the fake client.
    let result = client
        .dashboard(DashboardRequest { bucket, cursor })
        .await
        .expect("fake client must return seeded fixtures");

    // 4. Feed the per-bucket (rows, next-cursor) triples straight back.
    let follow_up = update(
        &mut app.state,
        &AppEvent::Data(DataEvent::DashboardLoaded(Ok(result))),
    );
    assert!(
        follow_up.is_empty(),
        "DashboardLoaded absorption emits no further effects"
    );

    // 5. Render and snapshot the buffer.
    let dashboard = app
        .state
        .dashboard
        .clone()
        .expect("dashboard populated after absorb");
    let state = DashboardState::Loaded(dashboard);
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| {
            #[allow(deprecated)]
            let size = f.area();
            render_dashboard_state(f, size, &state, 0, [0; 3], &Theme::dark());
        })
        .unwrap();
    insta::assert_snapshot!("dashboard_happy_path", terminal.backend());
}
