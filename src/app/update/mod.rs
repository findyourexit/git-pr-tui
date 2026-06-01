mod data;
mod diff;
mod nav;
mod overlays;
mod pr_detail;
mod tabs;
mod write;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use data::handle_data_event;
use diff::handle_diff_viewer_key;
use nav::{
    nav_bottom, nav_down, nav_open_selected, nav_refresh, nav_top, nav_up, page_move,
    selected_pr_id_in_list, selected_pr_url,
};
use overlays::{
    handle_action_menu_key, handle_close_reopen_confirm_key, handle_composer_key,
    handle_filter_modal_key, handle_log_view_key, handle_merge_modal_key, handle_palette_key,
    handle_review_modal_key, handle_search_key, handle_sort_modal_key, load_more_effects,
    open_filter_modal, open_search, open_sort_modal,
};
use pr_detail::handle_pr_detail_key;
use write::start_checkout;

use super::{
    anim::{AnimationCue, cue_allowed},
    effect::Effect,
    event::AppEvent,
    intent::UserIntent,
    state::{AppState, DetailTab, View},
};

/// Push an `OverlayOpen` cue if the current intensity allows it.
fn push_overlay_cue(state: &mut AppState) {
    let cue = AnimationCue::OverlayOpen;
    if cue_allowed(state.animations, &cue) {
        state.animation_cues.push(cue);
    }
}

/// Push a `DataLoaded` cue iff the transition is armed (re-armed on view-entry
/// change, see `update`) and the current intensity allows it. Firing consumes
/// the arming so the staged async loads of one view (detail, checks, files)
/// animate once per entry rather than replaying the transition per data set.
fn push_data_loaded_cue(state: &mut AppState) {
    let cue = AnimationCue::DataLoaded;
    if state.data_anim_armed && cue_allowed(state.animations, &cue) {
        state.animation_cues.push(cue);
        state.data_anim_armed = false;
    }
}

/// Identity of the active view ignoring sub-tab / diff file. Used to detect a
/// view-entry change (navigating to a different PR/repo/dashboard), which
/// re-arms the `DataLoaded` transition.
#[derive(PartialEq)]
enum ViewEntryKey {
    Dashboard,
    PrList(crate::data::models::Repo),
    Pr(crate::data::models::PrId),
}

fn view_entry_key(state: &AppState) -> ViewEntryKey {
    match state.current_view() {
        View::Dashboard => ViewEntryKey::Dashboard,
        View::PrList { repo } => ViewEntryKey::PrList(repo),
        View::PrDetail { id, .. } | View::Diff { id, .. } => ViewEntryKey::Pr(id),
    }
}

/// Push an `ActionLanded` cue if the current intensity allows it.
fn push_action_landed_cue(state: &mut AppState, ok: bool) {
    let cue = AnimationCue::ActionLanded { ok };
    if cue_allowed(state.animations, &cue) {
        state.animation_cues.push(cue);
    }
}

// Re-export types used by tests via `use super::*`
#[cfg(test)]
use crate::{
    app::state::{ComposerContext, ComposerState, ToastKind},
    data::github::diff_position::DiffPositionIndex,
};

const LOAD_MORE_THRESHOLD: usize = 10;

/// Rows moved by a half-page key (`Ctrl-d` / `Ctrl-u`). The reducer has no
/// viewport height, so we use a fixed, predictable jump.
const HALF_PAGE_ROWS: usize = 10;

pub const LARGE_PR_FILE_THRESHOLD: u32 = 1500;

pub const BACKGROUND_REFRESH_INTERVAL_LOW: std::time::Duration =
    std::time::Duration::from_secs(300);
pub const USER_IDLE_GATE: std::time::Duration = std::time::Duration::from_secs(5);
pub const RATE_LIMIT_LOW_THRESHOLD: u32 = 200;
pub const RATE_LIMIT_CRITICAL_THRESHOLD: u32 = 50;

/// Toasts auto-dismiss after this long.
/// The 500ms tick sweeps expired toasts.
pub const TOAST_TTL: std::time::Duration = std::time::Duration::from_secs(5);

fn rate_limit_remaining(state: &AppState) -> Option<u32> {
    state.rate_limit.as_ref().map(|r| r.remaining)
}

fn background_refresh_allowed(state: &AppState) -> bool {
    if state.composer.is_some() {
        return false;
    }
    if state.user_idle_since.elapsed() < USER_IDLE_GATE {
        return false;
    }
    if let Some(remaining) = rate_limit_remaining(state)
        && remaining < RATE_LIMIT_CRITICAL_THRESHOLD
    {
        return false;
    }
    true
}

/// Base refresh interval for `key`, taken from the user's `[refresh]` config
/// (defaults: `dashboard`/`pr_list` 60s, `pr_detail` 30s). When the rate-limit
/// budget is low it is widened to at least the 5-minute floor.
fn effective_refresh_interval(
    state: &AppState,
    key: &crate::data::cache::CacheKey,
) -> std::time::Duration {
    use crate::data::cache::CacheKey as K;
    let secs = match key {
        K::DashboardReviewRequested | K::DashboardAuthored | K::DashboardAssigned => {
            state.refresh.dashboard
        }
        K::PrList(_) => state.refresh.pr_list,
        _ => state.refresh.pr_detail,
    };
    let base = std::time::Duration::from_secs(secs);
    if rate_limit_remaining(state).is_some_and(|r| r < RATE_LIMIT_LOW_THRESHOLD) {
        base.max(BACKGROUND_REFRESH_INTERVAL_LOW)
    } else {
        base
    }
}

fn refresh_due(state: &AppState, key: &crate::data::cache::CacheKey) -> bool {
    let interval = effective_refresh_interval(state, key);
    state
        .last_refresh
        .get(key)
        .is_some_and(|t| t.elapsed() >= interval)
}

pub fn update(state: &mut AppState, event: &AppEvent) -> Vec<Effect> {
    // Clear cues from the previous event so each event yields only its own.
    state.animation_cues.clear();
    let view_before = view_entry_key(state);
    let effects = match event {
        AppEvent::Key(k) => handle_key_event(state, k),
        AppEvent::Tick => handle_tick(state),
        AppEvent::Data(data) => handle_data_event(state, data),
        AppEvent::Resize { width, height } => {
            state.layout_mode = crate::ui::layout::min_size_check(*width, *height);
            state.viewport_width = *width;
            Vec::new()
        }
    };
    // Re-arm the data-load transition whenever the active view changes, so a
    // freshly-entered view animates once even though its data arrives as
    // several async loads. Loads that don't change the view (staged checks/
    // files, background/manual refresh) leave it disarmed and do not replay.
    if view_entry_key(state) != view_before {
        state.data_anim_armed = true;
    }
    effects
}

fn handle_tick(state: &mut AppState) -> Vec<Effect> {
    // Sweep expired toasts (auto-dismiss after 5s).
    if state
        .toast
        .as_ref()
        .is_some_and(|t| t.created_at.elapsed() >= TOAST_TTL)
    {
        state.toast = None;
    }
    let mut effects = Vec::new();
    let bg_ok = background_refresh_allowed(state);
    if matches!(state.current_view(), View::Dashboard) {
        let needs_initial = state.dashboard.is_none();
        let needs_refresh = bg_ok
            && state.dashboard.is_some()
            && refresh_due(
                state,
                &crate::data::cache::CacheKey::DashboardReviewRequested,
            );
        if needs_initial || needs_refresh {
            effects.push(Effect::FetchDashboard {
                bucket: None,
                cursor: None,
            });
        }
    }
    if let View::PrList { repo } = state.current_view() {
        let needs_initial = !state.pr_lists.contains_key(&repo);
        let needs_refresh = bg_ok
            && state.pr_lists.contains_key(&repo)
            && refresh_due(state, &crate::data::cache::CacheKey::PrList(repo.clone()));
        if needs_initial {
            state.pr_list_loading.insert(repo.clone());
        }
        if needs_initial || needs_refresh {
            effects.push(Effect::FetchPrList {
                repo: repo.clone(),
                cursor: None,
                filter: None,
                sort: None,
            });
        }
    }
    if let View::PrDetail { id, tab } = state.current_view() {
        let detail_initial = !state.pr_details.contains_key(&id);
        let detail_refresh = bg_ok
            && state.pr_details.contains_key(&id)
            && refresh_due(state, &crate::data::cache::CacheKey::PrDetail(id.clone()));
        if detail_initial || detail_refresh {
            effects.push(Effect::FetchPrDetail { id: id.clone() });
        }
        let checks_initial = !state.pr_checks.contains_key(&id);
        let checks_refresh = bg_ok
            && state.pr_checks.contains_key(&id)
            && refresh_due(state, &crate::data::cache::CacheKey::PrChecks(id.clone()));
        if checks_initial || checks_refresh {
            effects.push(Effect::FetchPrChecks { id: id.clone() });
        }
        if tab == DetailTab::Files
            && !state.pr_files.contains_key(&id)
            && let Some(detail) = state.pr_details.get(&id)
        {
            let changed_files = detail.summary.changed_files;
            let gated = changed_files > LARGE_PR_FILE_THRESHOLD
                && !state.pr_files_load_requested.contains(&id);
            if !gated {
                effects.push(Effect::FetchPrFiles {
                    id: id.clone(),
                    head_sha: detail.head_sha.clone(),
                });
            }
        }
    }
    effects
}

#[must_use]
pub fn refetch_effects(state: &AppState, kind: &crate::data::cache::WriteKind) -> Vec<Effect> {
    use crate::data::cache::WriteKind as W;
    let id = kind.pr_id();
    let mut effects = vec![Effect::FetchPrDetail { id: id.clone() }];

    // Merge changes files + checks state as well as the PR itself.
    if matches!(kind, W::Merge { .. }) {
        effects.push(Effect::FetchPrChecks { id: id.clone() });
        if let Some(detail) = state.pr_details.get(id) {
            effects.push(Effect::FetchPrFiles {
                id: id.clone(),
                head_sha: detail.head_sha.clone(),
            });
        }
    }

    // State-changing writes ripple into the dashboard buckets and the repo's
    // PR list; refresh only what's already cached so we don't over-fetch.
    if matches!(
        kind,
        W::SubmitReview { .. } | W::Merge { .. } | W::Close { .. } | W::Reopen { .. }
    ) {
        if state.dashboard.is_some() {
            effects.push(Effect::FetchDashboard {
                bucket: None,
                cursor: None,
            });
        }
        if state.pr_lists.contains_key(&id.repo) {
            effects.push(Effect::FetchPrList {
                repo: id.repo.clone(),
                cursor: None,
                filter: None,
                sort: None,
            });
        }
    }
    effects
}

fn handle_key_event(state: &mut AppState, k: &KeyEvent) -> Vec<Effect> {
    state.user_idle_since = std::time::Instant::now();
    // Ctrl-C is an unconditional hard quit. In raw mode the terminal does not
    // raise SIGINT, so we must handle it ourselves.
    if k.code == KeyCode::Char('c') && k.modifiers.contains(KeyModifiers::CONTROL) {
        state.should_quit = true;
        return Vec::new();
    }
    if state.action_menu.is_some() {
        return handle_action_menu_key(state, k);
    }
    if state.palette.is_some() {
        return handle_palette_key(state, k);
    }
    if state.log_view.is_some() {
        return handle_log_view_key(state, k);
    }
    if state.review_modal.is_some() {
        return handle_review_modal_key(state, k);
    }
    if state.merge_modal.is_some() {
        return handle_merge_modal_key(state, k);
    }
    if state.close_reopen_confirm.is_some() {
        return handle_close_reopen_confirm_key(state, k);
    }
    if state.composer.is_some() {
        return handle_composer_key(state, k);
    }
    if state.active_tab().state.filter_modal.is_some() {
        return handle_filter_modal_key(state, k);
    }
    if state.active_tab().state.sort_modal.is_some() {
        return handle_sort_modal_key(state, k);
    }
    if state.active_tab().state.search.is_some() {
        return handle_search_key(state, k);
    }
    if matches!(state.current_view(), View::PrDetail { .. })
        && let Some(effects) = handle_pr_detail_key(state, k)
    {
        return effects;
    }
    if matches!(state.current_view(), View::Diff { .. })
        && let Some(effects) = handle_diff_viewer_key(state, k)
    {
        return effects;
    }
    if let View::PrList { repo } = state.current_view()
        && k.code == KeyCode::Char('C')
        && (k.modifiers == KeyModifiers::NONE || k.modifiers == KeyModifiers::SHIFT)
    {
        if let Some(id) = selected_pr_id_in_list(state, &repo) {
            return start_checkout(state, &id);
        }
        return Vec::new();
    }
    match state.keymap.resolve(k) {
        Some(intent) => apply_intent(state, &intent),
        None => Vec::new(),
    }
}

fn apply_intent(state: &mut AppState, intent: &UserIntent) -> Vec<Effect> {
    match intent {
        UserIntent::EnterHelp => {
            state.help_overlay = !state.help_overlay;
            Vec::new()
        }
        UserIntent::EnterCommandPalette => {
            state.palette = Some(crate::ui::palette::PaletteState::default());
            push_overlay_cue(state);
            Vec::new()
        }
        UserIntent::OpenActionMenu => {
            state.action_menu = Some(crate::ui::action_menu::ActionMenuState::default());
            push_overlay_cue(state);
            Vec::new()
        }
        UserIntent::EnterLogView => {
            state.log_view = Some(crate::ui::log_view::LogViewState::default());
            push_overlay_cue(state);
            Vec::new()
        }
        UserIntent::Quit => {
            state.help_overlay = false;
            state.should_quit = true;
            Vec::new()
        }
        UserIntent::Cancel => {
            // Priority: overlays first, then diff -> files step-back, then no-op.
            if state.help_overlay {
                state.help_overlay = false;
                return Vec::new();
            }
            if state.action_menu.is_some() {
                state.action_menu = None;
                return Vec::new();
            }
            if state.palette.is_some() {
                state.palette = None;
                return Vec::new();
            }
            if state.log_view.is_some() {
                state.log_view = None;
                return Vec::new();
            }
            // Step back within a PR tab: Diff -> Files list.
            if state.active_tab().state.diff_file.is_some() {
                state.active_tab_mut().state.diff_file = None;
                return Vec::new();
            }
            Vec::new()
        }
        UserIntent::NextTab => tabs::next_tab(state),
        UserIntent::PrevTab => tabs::prev_tab(state),
        UserIntent::GotoTab(n) => tabs::goto_tab(state, *n),
        UserIntent::CloseTab => tabs::close_tab(state),
        UserIntent::GotoDashboard => tabs::goto_dashboard(state),
        UserIntent::PageDown => page_move(state, true),
        UserIntent::PageUp => page_move(state, false),
        UserIntent::SwitchTab(crate::app::intent::TabDirection::Prev) => {
            if let View::PrDetail { tab, .. } = state.current_view() {
                state.active_tab_mut().state.detail_tab = pr_detail::cycle_tab_backward(tab);
            }
            Vec::new()
        }
        UserIntent::SwitchTab(crate::app::intent::TabDirection::Next) => {
            if let View::PrDetail { tab, .. } = state.current_view() {
                state.active_tab_mut().state.detail_tab = pr_detail::cycle_tab_forward(tab);
            }
            Vec::new()
        }

        UserIntent::LoadMore => load_more_effects(state),
        UserIntent::OpenFilterModal => open_filter_modal(state),
        UserIntent::OpenSortModal => open_sort_modal(state),
        UserIntent::OpenSearch => open_search(state),
        UserIntent::Down => nav_down(state),
        UserIntent::Up => nav_up(state),
        UserIntent::FocusLeft => match state.current_view() {
            View::Dashboard => {
                nav::dashboard_focus(state, -1);
                Vec::new()
            }
            _ => diff::focus_pane(state, crate::app::state::FilesPane::Tree),
        },
        UserIntent::FocusRight => match state.current_view() {
            View::Dashboard => {
                nav::dashboard_focus(state, 1);
                Vec::new()
            }
            _ => diff::focus_pane(state, crate::app::state::FilesPane::Diff),
        },
        UserIntent::Top => nav_top(state),
        UserIntent::Bottom => nav_bottom(state),
        UserIntent::OpenSelected => nav_open_selected(state),
        UserIntent::Refresh => nav_refresh(state),
        UserIntent::OpenInBrowser => match selected_pr_url(state) {
            Some(url) => vec![Effect::OpenInBrowser { url }],
            None => Vec::new(),
        },
        _ => Vec::new(),
    }
}

#[must_use]
pub fn search_filter_pr_rows<'a>(
    rows: &'a [crate::data::models::PrSummary],
    query: &str,
) -> Vec<&'a crate::data::models::PrSummary> {
    if query.is_empty() {
        return rows.iter().collect();
    }
    let needle = query.to_lowercase();
    rows.iter()
        .filter(|pr| pr.title.to_lowercase().contains(&needle))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::effect::DataEvent;
    use crate::app::state::Selection;
    use crate::data::github::DashboardBucket;
    use crate::data::models::{
        ChecksRollup, Mergeable, PrId, PrState, PrSummary, Repo, ReviewDecision,
    };

    /// Build an `AppState` with the active tab derived from `view`.
    fn state_for(view: View) -> AppState {
        let mut s = AppState::default();
        crate::app::seed_workspace_from_view(&mut s, view);
        s
    }

    fn key(c: char) -> AppEvent {
        AppEvent::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
    }

    /// An `Instant` `secs` in the past.
    ///
    /// On platforms whose monotonic clock starts near zero early in the process
    /// (notably Windows CI), `Instant::checked_sub` underflows for large offsets
    /// and returns `None`. In that case we binary-search the largest representable
    /// offset, yielding the oldest instant the clock can express — still older
    /// than any refresh interval these tests assert against.
    fn secs_ago(secs: u64) -> std::time::Instant {
        use std::time::{Duration, Instant};
        let now = Instant::now();
        if let Some(t) = now.checked_sub(Duration::from_secs(secs)) {
            return t;
        }
        let (mut lo, mut hi) = (0_u64, secs);
        while lo < hi {
            let mid = lo + (hi - lo).div_ceil(2);
            if now.checked_sub(Duration::from_secs(mid)).is_some() {
                lo = mid;
            } else {
                hi = mid - 1;
            }
        }
        now.checked_sub(Duration::from_secs(lo)).unwrap_or(now)
    }

    fn make_summaries(n: usize) -> Vec<PrSummary> {
        (0..n)
            .map(|i| PrSummary {
                id: PrId {
                    repo: Repo {
                        owner: "acme".into(),
                        name: "widgets".into(),
                    },
                    number: i as u64,
                },
                title: format!("pr {i}"),
                is_draft: false,
                state: PrState::Open,
                author: "octocat".into(),
                base: "main".into(),
                head: "f".into(),
                additions: 0,
                deletions: 0,
                changed_files: 0,
                comments: 0,
                review_decision: Some(ReviewDecision::ReviewRequired),
                checks: ChecksRollup::Success,
                mergeable: Mergeable::Clean,
                labels: vec![],
                updated_at: chrono::Utc::now(),
                created_at: chrono::Utc::now(),
            })
            .collect()
    }

    #[test]
    fn q_sets_should_quit() {
        let mut s = AppState::default();
        let effects = update(&mut s, &key('q'));
        assert!(s.should_quit);
        assert!(effects.is_empty());
    }

    #[test]
    fn other_keys_do_nothing() {
        let mut s = AppState::default();
        update(&mut s, &key('x'));
        assert!(!s.should_quit);
    }

    #[test]
    fn first_tick_emits_fetch_dashboard_when_view_is_dashboard_and_data_empty() {
        let mut s = AppState::default();
        let effects = update(&mut s, &AppEvent::Tick);
        assert!(
            effects.iter().any(|e| matches!(
                e,
                Effect::FetchDashboard {
                    bucket: None,
                    cursor: None
                }
            )),
            "expected initial FetchDashboard, got {effects:?}"
        );
    }

    #[test]
    fn tick_after_dashboard_loaded_emits_no_effect() {
        let mut s = AppState {
            dashboard: Some(vec![]),
            ..AppState::default()
        };
        let effects = update(&mut s, &AppEvent::Tick);
        assert!(effects.is_empty());
    }

    #[test]
    fn first_tick_on_pr_list_emits_fetch_pr_list_when_cache_empty() {
        use crate::data::models::Repo;
        let repo = Repo {
            owner: "acme".into(),
            name: "widgets".into(),
        };
        let mut s = state_for(View::PrList { repo: repo.clone() });
        let effects = update(&mut s, &AppEvent::Tick);
        assert!(
            effects.iter().any(|e| matches!(
                e,
                Effect::FetchPrList {
                    repo: r,
                    cursor: None,
                    filter: None,
                    sort: None,
                } if *r == repo
            )),
            "expected FetchPrList for {repo:?}, got {effects:?}"
        );
    }

    #[test]
    fn tick_on_pr_list_emits_no_effect_when_cache_populated() {
        use crate::data::models::Repo;
        let repo = Repo {
            owner: "acme".into(),
            name: "widgets".into(),
        };
        let mut pr_lists = std::collections::HashMap::new();
        pr_lists.insert(repo.clone(), vec![]);
        let mut s = state_for(View::PrList { repo: repo.clone() });
        s.pr_lists = pr_lists;
        let effects = update(&mut s, &AppEvent::Tick);
        assert!(
            !effects
                .iter()
                .any(|e| matches!(e, Effect::FetchPrList { .. })),
            "expected no FetchPrList when cache populated, got {effects:?}"
        );
    }

    #[test]
    fn dashboard_data_event_populates_state() {
        let mut s = AppState::default();
        let payload = vec![(DashboardBucket::ReviewRequested, vec![], None)];
        update(
            &mut s,
            &AppEvent::Data(DataEvent::DashboardLoaded(Ok(payload.clone()))),
        );
        assert_eq!(s.dashboard.as_ref().unwrap().len(), 1);
    }

    #[test]
    fn bucket_scoped_dashboard_payload_preserves_other_buckets() {
        // Regression: nav_down past load-more threshold dispatches
        // FetchDashboard { bucket: Some(B), .. } and the data layer returns
        // ONLY bucket B's rows. absorb_dashboard_ok must merge that single
        // bucket into the existing dashboard, NOT replace all three.
        let mut s = AppState {
            dashboard: Some(vec![
                (DashboardBucket::ReviewRequested, make_summaries(20)),
                (DashboardBucket::Authored, make_summaries(5)),
                (DashboardBucket::Assigned, make_summaries(3)),
            ]),
            ..AppState::default()
        };
        // Simulate a bucket-scoped "load more": only ReviewRequested's NEXT
        // page (25 rows). Those rows are appended to the existing 20.
        let partial = vec![(DashboardBucket::ReviewRequested, make_summaries(25), None)];
        update(
            &mut s,
            &AppEvent::Data(DataEvent::DashboardLoaded(Ok(partial))),
        );
        let dash = s.dashboard.expect("dashboard must remain populated");
        assert_eq!(
            dash.len(),
            3,
            "all three buckets must survive a bucket-scoped fetch, got {dash:?}"
        );
        let rr = dash
            .iter()
            .find(|(b, _)| *b == DashboardBucket::ReviewRequested)
            .expect("ReviewRequested bucket present");
        assert_eq!(
            rr.1.len(),
            45,
            "paginated rows appended to existing page (20 + 25)"
        );
        let auth = dash
            .iter()
            .find(|(b, _)| *b == DashboardBucket::Authored)
            .expect("Authored bucket preserved");
        assert_eq!(auth.1.len(), 5, "Authored rows preserved untouched");
        let assigned = dash
            .iter()
            .find(|(b, _)| *b == DashboardBucket::Assigned)
            .expect("Assigned bucket preserved");
        assert_eq!(assigned.1.len(), 3, "Assigned rows preserved untouched");
    }

    #[test]
    fn dashboard_refresh_clamps_panel_cursors_when_bucket_shrinks() {
        let mut s = AppState {
            dashboard: Some(vec![
                (DashboardBucket::ReviewRequested, make_summaries(20)),
                (DashboardBucket::Authored, make_summaries(5)),
                (DashboardBucket::Assigned, make_summaries(3)),
            ]),
            ..AppState::default()
        };
        *s.selection_mut() = Selection::Dashboard {
            focused: 0,
            rows: [19, 4, 2],
        };
        // Full refresh: ReviewRequested shrinks from 20 → 5
        let full_payload = vec![
            (DashboardBucket::ReviewRequested, make_summaries(5), None),
            (DashboardBucket::Authored, make_summaries(5), None),
            (DashboardBucket::Assigned, make_summaries(3), None),
        ];
        update(
            &mut s,
            &AppEvent::Data(DataEvent::DashboardLoaded(Ok(full_payload))),
        );
        assert_eq!(
            *s.selection(),
            Selection::Dashboard {
                focused: 0,
                rows: [4, 4, 2]
            },
            "rows[0] must be clamped from 19 to 4 after bucket shrinks"
        );
    }

    #[test]
    fn dashboard_error_event_sets_error_toast() {
        use crate::data::github::GitHubError;
        let mut s = AppState::default();
        update(
            &mut s,
            &AppEvent::Data(DataEvent::DashboardLoaded(Err(GitHubError::NotFound))),
        );
        let toast = s.toast.expect("toast should be set");
        assert_eq!(toast.kind, ToastKind::Error);
        assert!(toast.message.contains("dashboard fetch failed"));
    }

    #[test]
    fn load_more_within_threshold_with_cursor_emits_fetch() {
        let rows = make_summaries(20);
        let mut cursors = std::collections::HashMap::new();
        cursors.insert(DashboardBucket::ReviewRequested, "cur-1".to_string());
        let mut s = AppState {
            dashboard: Some(vec![(DashboardBucket::ReviewRequested, rows)]),
            dashboard_cursors: cursors,
            ..AppState::default()
        };
        *s.selection_mut() = Selection::Dashboard {
            focused: 0,
            rows: [15, 0, 0],
        };
        let effects = apply_intent(&mut s, &UserIntent::LoadMore);
        assert_eq!(effects.len(), 1, "got {effects:?}");
        match &effects[0] {
            Effect::FetchDashboard { bucket, cursor } => {
                assert_eq!(*bucket, Some(DashboardBucket::ReviewRequested));
                assert_eq!(cursor.as_deref(), Some("cur-1"));
            }
            other => panic!("expected FetchDashboard, got {other:?}"),
        }
    }

    #[test]
    fn load_more_far_from_end_emits_nothing() {
        let rows = make_summaries(50);
        let mut cursors = std::collections::HashMap::new();
        cursors.insert(DashboardBucket::ReviewRequested, "cur".to_string());
        let mut s = AppState {
            dashboard: Some(vec![(DashboardBucket::ReviewRequested, rows)]),
            dashboard_cursors: cursors,
            ..AppState::default()
        };
        *s.selection_mut() = Selection::Dashboard {
            focused: 0,
            rows: [5, 0, 0],
        };
        let effects = apply_intent(&mut s, &UserIntent::LoadMore);
        assert!(effects.is_empty(), "got {effects:?}");
    }

    #[test]
    fn load_more_without_cursor_emits_nothing() {
        // No stored cursor means the bucket has no next page; "load more" must
        // NOT re-fetch page 1 (that was the duplication bug).
        let rows = make_summaries(20);
        let mut s = AppState {
            dashboard: Some(vec![(DashboardBucket::ReviewRequested, rows)]),
            ..AppState::default()
        };
        *s.selection_mut() = Selection::Dashboard {
            focused: 0,
            rows: [15, 0, 0],
        };
        let effects = apply_intent(&mut s, &UserIntent::LoadMore);
        assert!(
            effects.is_empty(),
            "no cursor → no fetch (avoids re-appending page 1), got {effects:?}"
        );
        assert!(
            s.dashboard_loading.is_empty(),
            "no in-flight marker set when nothing is fetched"
        );
    }

    #[test]
    fn load_more_does_not_refire_while_in_flight() {
        let mut cursors = std::collections::HashMap::new();
        cursors.insert(DashboardBucket::ReviewRequested, "cur-1".to_string());
        let mut s = AppState {
            dashboard: Some(vec![(DashboardBucket::ReviewRequested, make_summaries(20))]),
            dashboard_cursors: cursors,
            ..AppState::default()
        };
        *s.selection_mut() = Selection::Dashboard {
            focused: 0,
            rows: [15, 0, 0],
        };
        let first = apply_intent(&mut s, &UserIntent::LoadMore);
        assert_eq!(first.len(), 1, "first load-more fires, got {first:?}");
        let second = apply_intent(&mut s, &UserIntent::LoadMore);
        assert!(
            second.is_empty(),
            "second load-more is suppressed while the page is in flight, got {second:?}"
        );
    }

    #[test]
    fn load_more_outside_dashboard_view_emits_nothing() {
        let mut s = state_for(View::PrList {
            repo: Repo {
                owner: "a".into(),
                name: "b".into(),
            },
        });
        let effects = apply_intent(&mut s, &UserIntent::LoadMore);
        assert!(effects.is_empty(), "got {effects:?}");
    }

    fn pr_list_view() -> AppState {
        state_for(View::PrList {
            repo: Repo {
                owner: "acme".into(),
                name: "widgets".into(),
            },
        })
    }

    #[test]
    fn f_in_pr_list_opens_filter_modal() {
        let mut s = pr_list_view();
        let effects = update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE)),
        );
        assert!(effects.is_empty(), "opening modal should emit no effects");
        assert!(
            s.active_tab().state.filter_modal.is_some(),
            "modal should be open after f"
        );
        assert_eq!(
            s.active_tab().state.filter_modal.as_ref().unwrap().selected,
            0
        );
    }

    #[test]
    fn f_outside_pr_list_does_not_open_modal() {
        let mut s = AppState::default();
        update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE)),
        );
        assert!(s.active_tab().state.filter_modal.is_none());
    }

    #[test]
    fn j_in_filter_modal_moves_selection_down() {
        let mut s = pr_list_view();
        s.active_tab_mut().state.filter_modal =
            Some(crate::app::state::FilterModalState { selected: 0 });
        update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE)),
        );
        assert_eq!(
            s.active_tab().state.filter_modal.as_ref().unwrap().selected,
            1
        );
    }

    #[test]
    fn k_in_filter_modal_moves_selection_up() {
        let mut s = pr_list_view();
        s.active_tab_mut().state.filter_modal =
            Some(crate::app::state::FilterModalState { selected: 2 });
        update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE)),
        );
        assert_eq!(
            s.active_tab().state.filter_modal.as_ref().unwrap().selected,
            1
        );
    }

    #[test]
    fn j_at_last_filter_option_clamps() {
        let mut s = pr_list_view();
        let last = crate::app::state::PR_LIST_FILTER_OPTIONS.len() - 1;
        s.active_tab_mut().state.filter_modal =
            Some(crate::app::state::FilterModalState { selected: last });
        update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE)),
        );
        assert_eq!(
            s.active_tab().state.filter_modal.as_ref().unwrap().selected,
            last
        );
    }

    #[test]
    fn enter_in_filter_modal_emits_fetch_with_filter_and_closes() {
        use crate::data::github::PrListFilter;
        let mut s = pr_list_view();
        s.active_tab_mut().state.filter_modal =
            Some(crate::app::state::FilterModalState { selected: 2 });
        let effects = update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        );
        assert!(
            s.active_tab().state.filter_modal.is_none(),
            "modal should close"
        );
        assert_eq!(effects.len(), 1, "got {effects:?}");
        match &effects[0] {
            Effect::FetchPrList {
                repo,
                cursor,
                filter,
                sort,
            } => {
                assert_eq!(repo.owner, "acme");
                assert!(cursor.is_none());
                assert_eq!(*filter, Some(PrListFilter::Merged));
                assert!(sort.is_none(), "filter modal should not set sort");
            }
            other => panic!("expected FetchPrList, got {other:?}"),
        }
    }

    #[test]
    fn esc_in_filter_modal_closes_without_effect() {
        let mut s = pr_list_view();
        s.active_tab_mut().state.filter_modal =
            Some(crate::app::state::FilterModalState { selected: 1 });
        let effects = update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        );
        assert!(s.active_tab().state.filter_modal.is_none());
        assert!(effects.is_empty(), "got {effects:?}");
    }

    #[test]
    fn s_in_pr_list_opens_sort_modal() {
        let mut s = pr_list_view();
        let effects = update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE)),
        );
        assert!(effects.is_empty(), "opening modal should emit no effects");
        assert!(
            s.active_tab().state.sort_modal.is_some(),
            "modal should be open after s"
        );
        assert_eq!(
            s.active_tab().state.sort_modal.as_ref().unwrap().selected,
            0
        );
    }

    #[test]
    fn s_outside_pr_list_does_not_open_sort_modal() {
        let mut s = AppState::default();
        update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE)),
        );
        assert!(s.active_tab().state.sort_modal.is_none());
    }

    #[test]
    fn j_in_sort_modal_moves_selection_down() {
        let mut s = pr_list_view();
        s.active_tab_mut().state.sort_modal =
            Some(crate::app::state::SortModalState { selected: 0 });
        update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE)),
        );
        assert_eq!(
            s.active_tab().state.sort_modal.as_ref().unwrap().selected,
            1
        );
    }

    #[test]
    fn k_in_sort_modal_moves_selection_up() {
        let mut s = pr_list_view();
        s.active_tab_mut().state.sort_modal =
            Some(crate::app::state::SortModalState { selected: 2 });
        update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE)),
        );
        assert_eq!(
            s.active_tab().state.sort_modal.as_ref().unwrap().selected,
            1
        );
    }

    #[test]
    fn j_at_last_sort_option_clamps() {
        let mut s = pr_list_view();
        let last = crate::app::state::PR_LIST_SORT_OPTIONS.len() - 1;
        s.active_tab_mut().state.sort_modal =
            Some(crate::app::state::SortModalState { selected: last });
        update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE)),
        );
        assert_eq!(
            s.active_tab().state.sort_modal.as_ref().unwrap().selected,
            last
        );
    }

    #[test]
    fn enter_in_sort_modal_emits_fetch_with_sort_and_closes() {
        use crate::data::github::PrListSort;
        let mut s = pr_list_view();
        s.active_tab_mut().state.sort_modal =
            Some(crate::app::state::SortModalState { selected: 2 });
        let effects = update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        );
        assert!(
            s.active_tab().state.sort_modal.is_none(),
            "modal should close"
        );
        assert_eq!(effects.len(), 1, "got {effects:?}");
        match &effects[0] {
            Effect::FetchPrList {
                repo,
                cursor,
                filter,
                sort,
            } => {
                assert_eq!(repo.owner, "acme");
                assert!(cursor.is_none());
                assert!(filter.is_none(), "sort modal should not set filter");
                assert_eq!(*sort, Some(PrListSort::CreatedDesc));
            }
            other => panic!("expected FetchPrList, got {other:?}"),
        }
    }

    #[test]
    fn esc_in_sort_modal_closes_without_effect() {
        let mut s = pr_list_view();
        s.active_tab_mut().state.sort_modal =
            Some(crate::app::state::SortModalState { selected: 1 });
        let effects = update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        );
        assert!(s.active_tab().state.sort_modal.is_none());
        assert!(effects.is_empty(), "got {effects:?}");
    }

    #[test]
    fn slash_in_pr_list_opens_search() {
        let mut s = pr_list_view();
        let effects = update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE)),
        );
        assert!(effects.is_empty());
        assert!(s.active_tab().state.search.is_some());
        assert!(
            s.active_tab()
                .state
                .search
                .as_ref()
                .unwrap()
                .query
                .is_empty()
        );
    }

    #[test]
    fn slash_outside_pr_list_does_not_open_search() {
        let mut s = AppState::default();
        update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE)),
        );
        assert!(s.active_tab().state.search.is_none());
    }

    #[test]
    fn typing_in_search_appends_to_query() {
        let mut s = pr_list_view();
        s.active_tab_mut().state.search = Some(crate::app::state::SearchState::default());
        update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE)),
        );
        update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE)),
        );
        update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE)),
        );
        assert_eq!(s.active_tab_mut().state.search.take().unwrap().query, "foo");
    }

    #[test]
    fn backspace_in_search_pops_query() {
        let mut s = pr_list_view();
        s.active_tab_mut().state.search = Some(crate::app::state::SearchState {
            query: "foo".into(),
        });
        update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE)),
        );
        assert_eq!(s.active_tab_mut().state.search.take().unwrap().query, "fo");
    }

    #[test]
    fn esc_in_search_clears_and_closes() {
        let mut s = pr_list_view();
        s.active_tab_mut().state.search = Some(crate::app::state::SearchState {
            query: "foo".into(),
        });
        let effects = update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        );
        assert!(s.active_tab().state.search.is_none());
        assert!(effects.is_empty());
    }

    #[test]
    fn enter_in_search_closes_input_without_effect() {
        let mut s = pr_list_view();
        s.active_tab_mut().state.search = Some(crate::app::state::SearchState {
            query: "foo".into(),
        });
        let effects = update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        );
        assert!(
            s.active_tab().state.search.is_none(),
            "Enter closes search input"
        );
        assert!(effects.is_empty(), "search emits no effects (client-side)");
    }

    #[test]
    fn search_filter_pr_rows_empty_query_returns_all() {
        let rows = vec![sample_pr("first"), sample_pr("second")];
        let out = search_filter_pr_rows(&rows, "");
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn search_filter_pr_rows_matches_case_insensitive_substring() {
        let rows = vec![
            sample_pr("Fix login bug"),
            sample_pr("Add docs"),
            sample_pr("FIX flaky test"),
        ];
        let out = search_filter_pr_rows(&rows, "fix");
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].title, "Fix login bug");
        assert_eq!(out[1].title, "FIX flaky test");
    }

    fn sample_pr(title: &str) -> PrSummary {
        PrSummary {
            id: PrId {
                repo: Repo {
                    owner: "acme".into(),
                    name: "widgets".into(),
                },
                number: 1,
            },
            title: title.into(),
            author: "octocat".into(),
            state: PrState::Open,
            is_draft: false,
            base: "main".into(),
            head: "feature".into(),
            additions: 0,
            deletions: 0,
            changed_files: 0,
            comments: 0,
            review_decision: None,
            checks: ChecksRollup::None,
            mergeable: Mergeable::Clean,
            labels: vec![],
            updated_at: chrono::Utc::now(),
            created_at: chrono::Utc::now(),
        }
    }

    fn repo_acme() -> Repo {
        Repo {
            owner: "acme".into(),
            name: "widgets".into(),
        }
    }

    #[test]
    fn pr_list_loaded_inserts_rows_and_stores_cursor() {
        let repo = repo_acme();
        let mut s = AppState::default();
        let rows = vec![sample_pr("first"), sample_pr("second")];
        update(
            &mut s,
            &AppEvent::Data(DataEvent::PrListLoaded {
                repo: repo.clone(),
                result: Ok(rows.clone()),
                next_cursor: Some("CURSOR_2".into()),
                replace: true,
            }),
        );
        assert_eq!(s.pr_lists.get(&repo).map(Vec::len), Some(2));
        assert_eq!(
            s.pr_list_cursors.get(&repo).map(String::as_str),
            Some("CURSOR_2")
        );
    }

    #[test]
    fn pr_list_loaded_appends_on_subsequent_page() {
        let repo = repo_acme();
        let mut s = AppState::default();
        update(
            &mut s,
            &AppEvent::Data(DataEvent::PrListLoaded {
                repo: repo.clone(),
                result: Ok(vec![sample_pr("first")]),
                next_cursor: Some("CURSOR_2".into()),
                replace: true,
            }),
        );
        update(
            &mut s,
            &AppEvent::Data(DataEvent::PrListLoaded {
                repo: repo.clone(),
                result: Ok(vec![sample_pr("second"), sample_pr("third")]),
                next_cursor: None,
                replace: false,
            }),
        );
        assert_eq!(s.pr_lists.get(&repo).map(Vec::len), Some(3));
        assert!(
            !s.pr_list_cursors.contains_key(&repo),
            "cursor cleared when next page is None"
        );
    }

    #[test]
    fn pr_list_loaded_err_sets_toast() {
        let repo = repo_acme();
        let mut s = AppState::default();
        update(
            &mut s,
            &AppEvent::Data(DataEvent::PrListLoaded {
                repo,
                result: Err(crate::data::github::GitHubError::NotFound),
                next_cursor: None,
                replace: true,
            }),
        );
        let toast = s.toast.expect("toast set on err");
        assert!(matches!(toast.kind, ToastKind::Error));
        assert!(toast.message.contains("acme/widgets"));
    }

    #[test]
    fn load_more_on_pr_list_emits_fetch_with_stored_cursor_when_near_end() {
        let repo = repo_acme();
        let rows: Vec<PrSummary> = (0..15).map(|i| sample_pr(&format!("pr{i}"))).collect();
        let mut pr_lists = std::collections::HashMap::new();
        pr_lists.insert(repo.clone(), rows);
        let mut pr_list_cursors = std::collections::HashMap::new();
        pr_list_cursors.insert(repo.clone(), "NEXT".to_string());
        let mut s = AppState {
            pr_lists,
            pr_list_cursors,
            ..AppState::default()
        };
        crate::app::seed_workspace_from_view(&mut s, View::PrList { repo: repo.clone() });
        *s.selection_mut() = Selection::PrListRow(6);
        let effects = update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Char('M'), KeyModifiers::SHIFT)),
        );
        assert_eq!(effects.len(), 1, "got {effects:?}");
        match &effects[0] {
            Effect::FetchPrList {
                repo: r,
                cursor: Some(c),
                filter: None,
                sort: None,
            } => {
                assert_eq!(*r, repo);
                assert_eq!(c, "NEXT");
            }
            other => panic!("expected paginated FetchPrList, got {other:?}"),
        }
    }

    #[test]
    fn load_more_on_pr_list_emits_nothing_when_not_near_end() {
        let repo = repo_acme();
        let rows: Vec<PrSummary> = (0..30).map(|i| sample_pr(&format!("pr{i}"))).collect();
        let mut pr_lists = std::collections::HashMap::new();
        pr_lists.insert(repo.clone(), rows);
        let mut pr_list_cursors = std::collections::HashMap::new();
        pr_list_cursors.insert(repo.clone(), "NEXT".to_string());
        let mut s = AppState {
            pr_lists,
            pr_list_cursors,
            ..AppState::default()
        };
        crate::app::seed_workspace_from_view(&mut s, View::PrList { repo: repo.clone() });
        *s.selection_mut() = Selection::PrListRow(5);
        let effects = update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Char('M'), KeyModifiers::SHIFT)),
        );
        assert!(effects.is_empty(), "got {effects:?}");
    }

    #[test]
    fn load_more_on_pr_list_emits_nothing_when_cursor_absent() {
        let repo = repo_acme();
        let rows: Vec<PrSummary> = (0..15).map(|i| sample_pr(&format!("pr{i}"))).collect();
        let mut pr_lists = std::collections::HashMap::new();
        pr_lists.insert(repo.clone(), rows);
        let mut s = AppState {
            pr_lists,
            ..AppState::default()
        };
        crate::app::seed_workspace_from_view(&mut s, View::PrList { repo: repo.clone() });
        *s.selection_mut() = Selection::PrListRow(10);
        let effects = update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Char('M'), KeyModifiers::SHIFT)),
        );
        assert!(effects.is_empty(), "no cursor means no fetch");
    }

    #[test]
    fn entering_pr_list_view_marks_repo_loading() {
        let repo = repo_acme();
        let mut s = AppState {
            ..AppState::default()
        };
        crate::app::seed_workspace_from_view(&mut s, View::PrList { repo: repo.clone() });
        let effects = update(&mut s, &AppEvent::Tick);
        assert!(
            effects
                .iter()
                .any(|e| matches!(e, Effect::FetchPrList { .. })),
            "expected initial fetch"
        );
        assert!(
            s.pr_list_loading.contains(&repo),
            "loading flag should be set"
        );
    }

    #[test]
    fn pr_list_loaded_clears_loading_flag() {
        let repo = repo_acme();
        let mut s = AppState {
            ..AppState::default()
        };
        crate::app::seed_workspace_from_view(&mut s, View::PrList { repo: repo.clone() });
        s.pr_list_loading.insert(repo.clone());
        update(
            &mut s,
            &AppEvent::Data(DataEvent::PrListLoaded {
                repo: repo.clone(),
                result: Ok(vec![sample_pr("a")]),
                next_cursor: None,
                replace: true,
            }),
        );
        assert!(
            !s.pr_list_loading.contains(&repo),
            "loading flag should be cleared on Ok"
        );
    }

    #[test]
    fn pr_list_loaded_err_clears_loading_flag() {
        let repo = repo_acme();
        let mut s = AppState {
            ..AppState::default()
        };
        crate::app::seed_workspace_from_view(&mut s, View::PrList { repo: repo.clone() });
        s.pr_list_loading.insert(repo.clone());
        update(
            &mut s,
            &AppEvent::Data(DataEvent::PrListLoaded {
                repo: repo.clone(),
                result: Err(crate::data::github::GitHubError::Network("boom".into())),
                next_cursor: None,
                replace: true,
            }),
        );
        assert!(
            !s.pr_list_loading.contains(&repo),
            "loading flag should be cleared on Err"
        );
        assert!(
            s.toast
                .as_ref()
                .is_some_and(|t| t.message.contains("press R to retry")),
            "toast should contain retry hint"
        );
    }

    #[test]
    fn entering_pr_detail_emits_fetch_detail_and_checks() {
        let id = PrId {
            repo: repo_acme(),
            number: 42,
        };
        let mut s = state_for(View::PrDetail {
            id: id.clone(),
            tab: crate::app::state::DetailTab::Conversation,
        });
        let effects = update(&mut s, &AppEvent::Tick);
        assert!(
            effects
                .iter()
                .any(|e| matches!(e, Effect::FetchPrDetail { id: i } if i == &id)),
            "expected FetchPrDetail"
        );
        assert!(
            effects
                .iter()
                .any(|e| matches!(e, Effect::FetchPrChecks { id: i } if i == &id)),
            "expected FetchPrChecks"
        );
    }

    #[test]
    fn entering_pr_detail_skips_fetch_when_cached() {
        let id = PrId {
            repo: repo_acme(),
            number: 42,
        };
        let mut s = state_for(View::PrDetail {
            id: id.clone(),
            tab: crate::app::state::DetailTab::Conversation,
        });
        s.pr_details.insert(id.clone(), sample_pr_detail(&id));
        s.pr_checks.insert(id.clone(), sample_pr_checks(&id));
        let effects = update(&mut s, &AppEvent::Tick);
        assert!(
            !effects.iter().any(|e| matches!(
                e,
                Effect::FetchPrDetail { .. } | Effect::FetchPrChecks { .. }
            )),
            "no fetches when both caches populated"
        );
    }

    fn sample_pr_detail(id: &PrId) -> crate::data::models::PrDetail {
        let mut summary = sample_pr("x");
        summary.id = id.clone();
        crate::data::models::PrDetail {
            summary,
            body: String::new(),
            body_html: None,
            requested_reviewers: vec![],
            assignees: vec![],
            head_sha: "deadbeef".into(),
            merge_commit_sha: None,
            base_ref_oid: "cafe".into(),
            review_threads: vec![],
            timeline: vec![],
            available_merge_methods: vec![],
            version: 0,
        }
    }

    fn sample_pr_checks(_id: &PrId) -> crate::data::models::PrChecks {
        crate::data::models::PrChecks {
            runs: vec![],
            statuses: vec![],
        }
    }

    fn pr_detail_state(tab: DetailTab) -> AppState {
        let id = PrId {
            repo: Repo {
                owner: "acme".into(),
                name: "widgets".into(),
            },
            number: 1,
        };
        state_for(View::PrDetail { id, tab })
    }

    fn current_tab(state: &AppState) -> DetailTab {
        match state.current_view() {
            View::PrDetail { tab, .. } => tab,
            other => panic!("expected PrDetail, got {other:?}"),
        }
    }

    #[test]
    fn pr_detail_bracket_cycles_forward_through_all_tabs() {
        let mut s = pr_detail_state(DetailTab::Conversation);
        update(&mut s, &key('['));
        // [ → SwitchTab(Prev) → cycles backward from Conversation → Commits
        assert_eq!(current_tab(&s), DetailTab::Commits);
        update(&mut s, &key(']'));
        // ] → SwitchTab(Next) → cycles forward from Commits → Conversation
        assert_eq!(current_tab(&s), DetailTab::Conversation);
        update(&mut s, &key(']'));
        assert_eq!(current_tab(&s), DetailTab::Files);
        update(&mut s, &key(']'));
        assert_eq!(current_tab(&s), DetailTab::Checks);
        update(&mut s, &key(']'));
        assert_eq!(current_tab(&s), DetailTab::Commits);
    }

    #[test]
    fn pr_detail_bracket_cycles_backward() {
        let mut s = pr_detail_state(DetailTab::Files);
        update(&mut s, &key('['));
        assert_eq!(current_tab(&s), DetailTab::Conversation);
    }

    #[test]
    fn tab_navigation_inert_outside_pr_detail() {
        let mut s = AppState::default();
        update(&mut s, &key('1'));
        assert!(matches!(s.current_view(), View::Dashboard));
    }

    fn detail_with_threads(thread_ids: &[&str]) -> (AppState, PrId) {
        let id = PrId {
            repo: Repo {
                owner: "acme".into(),
                name: "widgets".into(),
            },
            number: 1,
        };
        let mut detail = sample_pr_detail(&id);
        detail.version = 7;
        detail.review_threads = thread_ids
            .iter()
            .enumerate()
            .map(|(i, tid)| crate::data::models::ReviewThread {
                id: (*tid).to_string(),
                path: format!("src/file_{i}.rs"),
                line_range: crate::data::models::LineRange {
                    side: crate::data::models::DiffSide::Right,
                    start_line: 10,
                    end_line: 10,
                    start_position: 1,
                    end_position: 1,
                },
                is_resolved: false,
                is_outdated: false,
                comments: vec![],
            })
            .collect();
        let mut s = state_for(View::PrDetail {
            id: id.clone(),
            tab: DetailTab::Conversation,
        });
        s.pr_details.insert(id.clone(), detail);
        (s, id)
    }

    #[test]
    fn n_cycles_forward_and_wraps() {
        let (mut s, _) = detail_with_threads(&["t1", "t2", "t3"]);
        update(&mut s, &key('n'));
        update(&mut s, &key('n'));
        assert!(matches!(
            s.selection(),
            Selection::ReviewThread { thread_index: 1 }
        ));
        update(&mut s, &key('n'));
        update(&mut s, &key('n'));
        assert!(matches!(
            s.selection(),
            Selection::ReviewThread { thread_index: 0 }
        ));
    }

    #[test]
    fn shift_n_cycles_backward_and_wraps_to_last() {
        let (mut s, _) = detail_with_threads(&["t1", "t2", "t3"]);
        update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Char('N'), KeyModifiers::SHIFT)),
        );
        assert!(matches!(
            s.selection(),
            Selection::ReviewThread { thread_index: 2 }
        ));
    }

    #[test]
    fn shift_r_on_focused_thread_opens_composer_with_thread_id() {
        let (mut s, _) = detail_with_threads(&["thread-abc"]);
        update(&mut s, &key('n'));
        update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Char('R'), KeyModifiers::SHIFT)),
        );
        let composer = s.composer.as_ref().expect("composer should open");
        assert!(composer.body.is_empty());
        match &composer.context {
            ComposerContext::ReplyToThread { thread_id, .. } => {
                assert_eq!(thread_id, "thread-abc");
            }
            ComposerContext::PrComment { .. } => panic!("expected ReplyToThread, got PrComment"),
            ComposerContext::LineComment { .. } => {
                panic!("expected ReplyToThread, got LineComment")
            }
        }
    }

    #[test]
    fn shift_r_without_focused_thread_does_not_open_composer() {
        let (mut s, _) = detail_with_threads(&["t1"]);
        update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Char('R'), KeyModifiers::SHIFT)),
        );
        assert!(s.composer.is_none());
    }

    #[test]
    fn composer_typing_appends_to_body() {
        let (mut s, _) = detail_with_threads(&["t1"]);
        update(&mut s, &key('n'));
        update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Char('R'), KeyModifiers::SHIFT)),
        );
        update(&mut s, &key('h'));
        update(&mut s, &key('i'));
        assert_eq!(s.composer.as_ref().unwrap().body, "hi");
    }

    #[test]
    fn composer_esc_cancels() {
        let (mut s, _) = detail_with_threads(&["t1"]);
        update(&mut s, &key('n'));
        update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Char('R'), KeyModifiers::SHIFT)),
        );
        update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        );
        assert!(s.composer.is_none());
    }

    #[test]
    fn composer_ctrl_s_emits_execute_write_with_body() {
        let (mut s, id) = detail_with_threads(&["thread-xyz"]);
        update(&mut s, &key('n'));
        update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Char('R'), KeyModifiers::SHIFT)),
        );
        update(&mut s, &key('o'));
        update(&mut s, &key('k'));
        let effects = update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL)),
        );
        assert!(s.composer.is_none(), "composer should close on submit");
        assert_eq!(effects.len(), 1);
        match &effects[0] {
            Effect::ExecuteWrite {
                kind: crate::data::cache::WriteKind::ReplyToThread { id: write_id },
                request: crate::app::effect::WriteRequest::ReplyToThread { thread_id, body },
                version_at_submit,
            } => {
                assert_eq!(write_id, &id);
                assert_eq!(thread_id, "thread-xyz");
                assert_eq!(body, "ok");
                assert_eq!(*version_at_submit, 7);
            }
            other => panic!("expected ExecuteWrite ReplyToThread, got {other:?}"),
        }
    }

    #[test]
    fn composer_ctrl_s_with_empty_body_does_not_submit() {
        let (mut s, _) = detail_with_threads(&["t1"]);
        update(&mut s, &key('n'));
        update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Char('R'), KeyModifiers::SHIFT)),
        );
        let effects = update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL)),
        );
        assert!(effects.is_empty());
        assert!(s.composer.is_some(), "composer stays open on empty submit");
    }

    #[test]
    fn write_completed_reply_to_thread_ok_clears_pending_and_emits_refetch() {
        let id = PrId {
            repo: Repo {
                owner: "acme".into(),
                name: "widgets".into(),
            },
            number: 42,
        };
        let kind = crate::data::cache::WriteKind::ReplyToThread { id: id.clone() };
        let mut s = AppState {
            pending_write: Some(crate::app::state::PendingWrite {
                kind: kind.clone(),
                request: crate::app::effect::WriteRequest::ReplyToThread {
                    thread_id: "t1".into(),
                    body: "ok".into(),
                },
                version_at_submit: 1,
                status: crate::app::state::PendingStatus::Submitting,
            }),
            ..AppState::default()
        };
        let effects = update(
            &mut s,
            &AppEvent::Data(DataEvent::WriteCompleted {
                kind: kind.clone(),
                result: Ok(()),
            }),
        );
        assert!(s.pending_write.is_none(), "Ok clears pending_write");
        assert_eq!(effects.len(), 1);
        match &effects[0] {
            Effect::Refetch { kind: k } => assert_eq!(k, &kind),
            other => panic!("expected Refetch, got {other:?}"),
        }
    }

    #[test]
    fn write_completed_validation_keeps_composer_and_sets_status() {
        let id = PrId {
            repo: Repo {
                owner: "acme".into(),
                name: "widgets".into(),
            },
            number: 42,
        };
        let kind = crate::data::cache::WriteKind::PostPrComment { id: id.clone() };
        let mut s = AppState {
            pending_write: Some(crate::app::state::PendingWrite {
                kind: kind.clone(),
                request: crate::app::effect::WriteRequest::PostPrComment { body: "x".into() },
                version_at_submit: 1,
                status: crate::app::state::PendingStatus::Submitting,
            }),
            ..AppState::default()
        };
        let effects = update(
            &mut s,
            &AppEvent::Data(DataEvent::WriteCompleted {
                kind,
                result: Err(crate::data::github::GitHubError::Validation(
                    "body required".into(),
                )),
            }),
        );
        assert!(effects.is_empty(), "validation emits no refetch");
        let pw = s
            .pending_write
            .as_ref()
            .expect("composer/pending stays open");
        assert_eq!(
            pw.status,
            crate::app::state::PendingStatus::Validation("body required".into())
        );
        assert!(s.toast.is_none(), "validation surfaces inline, no toast");
    }

    #[test]
    fn write_completed_server_5xx_sets_unknown_and_emits_refetch_and_toast() {
        let id = PrId {
            repo: Repo {
                owner: "acme".into(),
                name: "widgets".into(),
            },
            number: 42,
        };
        let kind = crate::data::cache::WriteKind::PostPrComment { id: id.clone() };
        let mut s = AppState {
            pending_write: Some(crate::app::state::PendingWrite {
                kind: kind.clone(),
                request: crate::app::effect::WriteRequest::PostPrComment { body: "x".into() },
                version_at_submit: 1,
                status: crate::app::state::PendingStatus::Submitting,
            }),
            ..AppState::default()
        };
        let effects = update(
            &mut s,
            &AppEvent::Data(DataEvent::WriteCompleted {
                kind: kind.clone(),
                result: Err(crate::data::github::GitHubError::Server(502)),
            }),
        );
        assert_eq!(effects.len(), 1);
        match &effects[0] {
            Effect::Refetch { kind: k } => assert_eq!(k, &kind),
            other => panic!("expected Refetch, got {other:?}"),
        }
        let pw = s
            .pending_write
            .as_ref()
            .expect("pending stays for inspection");
        assert_eq!(pw.status, crate::app::state::PendingStatus::UnknownOutcome);
        let toast = s.toast.as_ref().expect("warning toast emitted");
        assert_eq!(toast.message, "Outcome unknown — refreshed the PR");
        assert_eq!(toast.kind, crate::app::state::ToastKind::Warning);
    }

    #[test]
    fn write_completed_network_and_timeout_both_route_to_unknown_outcome() {
        for err in [
            crate::data::github::GitHubError::Network("dns".into()),
            crate::data::github::GitHubError::Timeout(std::time::Duration::from_secs(10)),
        ] {
            let kind = crate::data::cache::WriteKind::PostPrComment {
                id: PrId {
                    repo: Repo {
                        owner: "a".into(),
                        name: "b".into(),
                    },
                    number: 1,
                },
            };
            let mut s = AppState {
                pending_write: Some(crate::app::state::PendingWrite {
                    kind: kind.clone(),
                    request: crate::app::effect::WriteRequest::PostPrComment { body: "x".into() },
                    version_at_submit: 1,
                    status: crate::app::state::PendingStatus::Submitting,
                }),
                ..AppState::default()
            };
            let effects = update(
                &mut s,
                &AppEvent::Data(DataEvent::WriteCompleted {
                    kind: kind.clone(),
                    result: Err(err),
                }),
            );
            assert!(matches!(effects.as_slice(), [Effect::Refetch { .. }]));
            assert_eq!(
                s.pending_write.as_ref().map(|p| p.status.clone()),
                Some(crate::app::state::PendingStatus::UnknownOutcome)
            );
        }
    }

    #[test]
    fn write_completed_stale_head_sha_sets_stale_status_no_toast() {
        let id = PrId {
            repo: Repo {
                owner: "acme".into(),
                name: "widgets".into(),
            },
            number: 42,
        };
        let kind = crate::data::cache::WriteKind::SubmitReview { id: id.clone() };
        let mut s = AppState {
            pending_write: Some(crate::app::state::PendingWrite {
                kind: kind.clone(),
                request: crate::app::effect::WriteRequest::PostPrComment { body: "x".into() },
                version_at_submit: 1,
                status: crate::app::state::PendingStatus::Submitting,
            }),
            ..AppState::default()
        };
        let effects = update(
            &mut s,
            &AppEvent::Data(DataEvent::WriteCompleted {
                kind,
                result: Err(crate::data::github::GitHubError::StaleHeadSha {
                    expected: "abc123".into(),
                }),
            }),
        );
        assert!(effects.is_empty());
        assert_eq!(
            s.pending_write.as_ref().map(|p| p.status.clone()),
            Some(crate::app::state::PendingStatus::Stale("abc123".into()))
        );
        assert!(s.toast.is_none(), "stale stays inline on composer");
    }

    #[test]
    fn write_completed_conflict_sets_status_and_surfaces_toast() {
        let id = PrId {
            repo: Repo {
                owner: "acme".into(),
                name: "widgets".into(),
            },
            number: 42,
        };
        let kind = crate::data::cache::WriteKind::Merge { id: id.clone() };
        let mut s = AppState {
            pending_write: Some(crate::app::state::PendingWrite {
                kind: kind.clone(),
                request: crate::app::effect::WriteRequest::Merge {
                    method: crate::data::models::MergeMethod::Merge,
                    head_sha: "abc".into(),
                },
                version_at_submit: 1,
                status: crate::app::state::PendingStatus::Submitting,
            }),
            ..AppState::default()
        };
        let effects = update(
            &mut s,
            &AppEvent::Data(DataEvent::WriteCompleted {
                kind,
                result: Err(crate::data::github::GitHubError::Conflict(
                    "merge conflict".into(),
                )),
            }),
        );
        assert!(effects.is_empty());
        assert_eq!(
            s.pending_write.as_ref().map(|p| p.status.clone()),
            Some(crate::app::state::PendingStatus::Conflict(
                "merge conflict".into()
            ))
        );
        let toast = s.toast.as_ref().expect("conflict toast emitted");
        assert_eq!(toast.kind, crate::app::state::ToastKind::Error);
        assert!(toast.message.contains("merge conflict"));
    }

    #[test]
    fn validation_pending_write_never_auto_retries_on_tick() {
        let id = PrId {
            repo: Repo {
                owner: "acme".into(),
                name: "widgets".into(),
            },
            number: 42,
        };
        let kind = crate::data::cache::WriteKind::PostPrComment { id: id.clone() };
        let mut s = state_for(View::PrDetail {
            id: id.clone(),
            tab: DetailTab::Conversation,
        });
        s.pending_write = Some(crate::app::state::PendingWrite {
            kind,
            request: crate::app::effect::WriteRequest::PostPrComment { body: "x".into() },
            version_at_submit: 1,
            status: crate::app::state::PendingStatus::Validation("body required".into()),
        });
        let effects = update(&mut s, &AppEvent::Tick);
        assert!(
            !effects
                .iter()
                .any(|e| matches!(e, Effect::ExecuteWrite { .. })),
            "tick must never auto-retry a Validation-pending write"
        );
    }

    fn sample_pr_files(_id: &PrId) -> crate::data::models::PrFiles {
        crate::data::models::PrFiles {
            files: vec![],
            truncated: false,
        }
    }

    #[test]
    fn pr_detail_loaded_ok_inserts_detail() {
        let id = PrId {
            repo: Repo {
                owner: "acme".into(),
                name: "widgets".into(),
            },
            number: 7,
        };
        let detail = sample_pr_detail(&id);
        let mut s = AppState::default();
        let effects = update(
            &mut s,
            &AppEvent::Data(DataEvent::PrDetailLoaded {
                id: id.clone(),
                result: Box::new(Ok(detail.clone())),
            }),
        );
        assert!(effects.is_empty());
        assert_eq!(
            s.pr_details.get(&id).map(|d| d.head_sha.clone()),
            Some(detail.head_sha)
        );
    }

    #[test]
    fn pr_detail_loaded_err_sets_toast() {
        let id = PrId {
            repo: Repo {
                owner: "acme".into(),
                name: "widgets".into(),
            },
            number: 7,
        };
        let mut s = AppState::default();
        let effects = update(
            &mut s,
            &AppEvent::Data(DataEvent::PrDetailLoaded {
                id: id.clone(),
                result: Box::new(Err(crate::data::github::GitHubError::NotFound)),
            }),
        );
        assert!(effects.is_empty());
        assert!(!s.pr_details.contains_key(&id));
        let toast = s.toast.expect("toast on err");
        assert!(matches!(toast.kind, ToastKind::Error));
        assert!(toast.message.contains("pr_detail"));
    }

    #[test]
    fn pr_files_loaded_ok_inserts_files() {
        let id = PrId {
            repo: Repo {
                owner: "acme".into(),
                name: "widgets".into(),
            },
            number: 7,
        };
        let files = sample_pr_files(&id);
        let mut s = AppState::default();
        let effects = update(
            &mut s,
            &AppEvent::Data(DataEvent::PrFilesLoaded {
                id: id.clone(),
                head_sha: "deadbeef".into(),
                result: Box::new(Ok(files.clone())),
            }),
        );
        assert!(effects.is_empty());
        assert!(s.pr_files.contains_key(&id));
    }

    #[test]
    fn pr_files_loaded_err_sets_toast() {
        let id = PrId {
            repo: Repo {
                owner: "acme".into(),
                name: "widgets".into(),
            },
            number: 7,
        };
        let mut s = AppState::default();
        let effects = update(
            &mut s,
            &AppEvent::Data(DataEvent::PrFilesLoaded {
                id: id.clone(),
                head_sha: "deadbeef".into(),
                result: Box::new(Err(crate::data::github::GitHubError::NotFound)),
            }),
        );
        assert!(effects.is_empty());
        assert!(!s.pr_files.contains_key(&id));
        assert!(matches!(s.toast.as_ref().unwrap().kind, ToastKind::Error));
    }

    #[test]
    fn pr_checks_loaded_ok_inserts_checks() {
        let id = PrId {
            repo: Repo {
                owner: "acme".into(),
                name: "widgets".into(),
            },
            number: 7,
        };
        let checks = sample_pr_checks(&id);
        let mut s = AppState::default();
        let effects = update(
            &mut s,
            &AppEvent::Data(DataEvent::PrChecksLoaded {
                id: id.clone(),
                result: Ok(checks),
            }),
        );
        assert!(effects.is_empty());
        assert!(s.pr_checks.contains_key(&id));
    }

    #[test]
    fn staged_pr_loads_fire_data_loaded_cue_once() {
        // Opening a PR streams data in as separate async events (detail, then
        // checks). The DataLoaded transition must fire exactly once per entry,
        // not replay per data set.
        let id = PrId {
            repo: Repo {
                owner: "acme".into(),
                name: "widgets".into(),
            },
            number: 1,
        };
        let mut s = state_for(View::PrDetail {
            id: id.clone(),
            tab: DetailTab::Conversation,
        });
        assert!(s.data_anim_armed, "a freshly-entered view starts armed");

        let _ = update(
            &mut s,
            &AppEvent::Data(DataEvent::PrDetailLoaded {
                id: id.clone(),
                result: Box::new(Ok(sample_pr_detail(&id))),
            }),
        );
        assert_eq!(
            s.animation_cues
                .iter()
                .filter(|c| **c == AnimationCue::DataLoaded)
                .count(),
            1,
            "first data set (detail) fires the transition once"
        );
        assert!(!s.data_anim_armed, "firing consumes the arming");

        let _ = update(
            &mut s,
            &AppEvent::Data(DataEvent::PrChecksLoaded {
                id: id.clone(),
                result: Ok(sample_pr_checks(&id)),
            }),
        );
        assert!(
            s.animation_cues
                .iter()
                .all(|c| *c != AnimationCue::DataLoaded),
            "staged checks load for the same view must NOT replay the transition"
        );
    }

    #[test]
    fn navigating_to_new_view_rearms_data_anim() {
        let mut s = dashboard_state_with(&[(DashboardBucket::ReviewRequested, 1)]);
        s.data_anim_armed = false; // simulate already consumed on the dashboard
        // Enter opens the selected PR: active view changes Dashboard -> PrDetail.
        let _ = update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        );
        assert!(matches!(s.current_view(), View::PrDetail { .. }));
        assert!(
            s.data_anim_armed,
            "entering a different view must re-arm the data-load transition"
        );
    }

    #[test]
    fn pr_checks_loaded_err_sets_toast() {
        let id = PrId {
            repo: Repo {
                owner: "acme".into(),
                name: "widgets".into(),
            },
            number: 7,
        };
        let mut s = AppState::default();
        let effects = update(
            &mut s,
            &AppEvent::Data(DataEvent::PrChecksLoaded {
                id: id.clone(),
                result: Err(crate::data::github::GitHubError::NotFound),
            }),
        );
        assert!(effects.is_empty());
        assert!(!s.pr_checks.contains_key(&id));
        assert!(matches!(s.toast.as_ref().unwrap().kind, ToastKind::Error));
    }

    fn sample_id() -> PrId {
        PrId {
            repo: Repo {
                owner: "acme".into(),
                name: "widgets".into(),
            },
            number: 7,
        }
    }

    fn diff_with_two_hunks() -> String {
        "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,2 +1,2 @@\n a\n-b\n+B\n@@ -10,2 +10,2 @@\n c\n-d\n+D\n".into()
    }

    fn files_for(id: &PrId, patches: Vec<&str>) -> crate::data::models::PrFiles {
        use crate::data::models::{FileDiff, FileStatus, PrFiles};
        let _ = id;
        PrFiles {
            files: patches
                .into_iter()
                .enumerate()
                .map(|(i, p)| FileDiff {
                    path: format!("src/file_{i}.rs"),
                    previous_path: None,
                    status: FileStatus::Modified,
                    additions: 1,
                    deletions: 1,
                    patch: Some(p.to_string()),
                    is_binary: false,
                    is_submodule: false,
                    is_generated: None,
                    oversize: false,
                    blob_sha_before: None,
                    blob_sha_after: None,
                })
                .collect(),
            truncated: false,
        }
    }

    fn diff_state_with_files(files: crate::data::models::PrFiles) -> (AppState, PrId) {
        let id = sample_id();
        let mut s = state_for(View::Diff {
            id: id.clone(),
            file_index: 0,
        });
        s.pr_files.insert(id.clone(), files);
        (s, id)
    }

    #[test]
    fn files_tab_enter_transitions_to_diff_view() {
        use crate::app::state::FilesPane;
        let id = PrId {
            repo: Repo {
                owner: "acme".into(),
                name: "widgets".into(),
            },
            number: 1,
        };
        let mut s = pr_detail_state(DetailTab::Files);
        // Start with Tree focus (default).
        assert_eq!(s.active_tab().state.files_focus, FilesPane::Tree);
        s.pr_files
            .insert(id.clone(), files_for(&id, vec![&diff_with_two_hunks()]));
        // Enter in wide split mode now focuses the Diff pane (not full-screen Diff).
        update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        );
        // View stays PrDetail (split mode — diff_file stays None).
        assert!(
            matches!(
                s.current_view(),
                View::PrDetail {
                    tab: DetailTab::Files,
                    ..
                }
            ),
            "expected View::PrDetail {{Files}}, got {:?}",
            s.current_view()
        );
        assert_eq!(s.active_tab().state.diff_file, None);
        assert_eq!(s.active_tab().state.files_focus, FilesPane::Diff);
    }

    // ── Split-mode Files pane tests ─────────────────────────────────────────

    /// Build a PR-detail state with the Files sub-tab active.
    fn files_split_state(files: crate::data::models::PrFiles) -> (AppState, PrId) {
        let id = PrId {
            repo: Repo {
                owner: "acme".into(),
                name: "widgets".into(),
            },
            number: 1, // matches pr_detail_state
        };
        let mut s = pr_detail_state(DetailTab::Files);
        s.pr_files.insert(id.clone(), files);
        (s, id)
    }

    #[test]
    fn split_focus_right_sets_diff_pane_then_j_moves_diff_cursor() {
        use crate::app::state::FilesPane;
        let (mut s, _id) = files_split_state(files_for(&sample_id(), vec![&diff_with_two_hunks()]));
        assert_eq!(s.active_tab().state.files_focus, FilesPane::Tree);

        // l / FocusRight → focus Diff
        update(&mut s, &key('l'));
        assert_eq!(s.active_tab().state.files_focus, FilesPane::Diff);
        assert_eq!(
            s.active_tab().state.diff_file,
            None,
            "diff_file must stay None in split mode"
        );

        // j → diff cursor moves, not file selection
        let Selection::DetailFile(file_sel_before) = *s.selection() else {
            panic!("expected DetailFile");
        };
        let cursor_before = s.active_tab().state.diff_cursor;
        update(&mut s, &key('j'));
        let cursor_after = s.active_tab().state.diff_cursor;
        let Selection::DetailFile(file_sel_after) = *s.selection() else {
            panic!("expected DetailFile");
        };
        // diff cursor advanced
        assert!(
            cursor_after > cursor_before,
            "j must move diff_cursor when Diff pane focused"
        );
        // file selection unchanged
        assert_eq!(
            file_sel_after, file_sel_before,
            "j must NOT change file selection when Diff pane focused"
        );
    }

    #[test]
    fn split_focus_left_sets_tree_pane_then_j_moves_file_selection() {
        use crate::app::state::FilesPane;
        let (mut s, _id) = files_split_state(files_for(
            &sample_id(),
            vec![&diff_with_two_hunks(), &diff_with_two_hunks()],
        ));
        // Start in Diff pane.
        update(&mut s, &key('l'));
        assert_eq!(s.active_tab().state.files_focus, FilesPane::Diff);

        // h / FocusLeft → focus Tree
        update(&mut s, &key('h'));
        assert_eq!(s.active_tab().state.files_focus, FilesPane::Tree);

        // j → file selection moves, not diff cursor
        let cursor_before = s.active_tab().state.diff_cursor;
        let Selection::DetailFile(file_sel_before) = *s.selection() else {
            panic!("expected DetailFile");
        };
        update(&mut s, &key('j'));
        let cursor_after = s.active_tab().state.diff_cursor;
        let Selection::DetailFile(file_sel_after) = *s.selection() else {
            panic!("expected DetailFile");
        };
        assert_eq!(
            cursor_after, cursor_before,
            "j must NOT move diff_cursor when Tree pane focused"
        );
        assert!(
            file_sel_after > file_sel_before,
            "j must move file selection when Tree pane focused"
        );
    }

    #[test]
    fn split_enter_while_tree_focused_focuses_diff() {
        use crate::app::state::FilesPane;
        let (mut s, _id) = files_split_state(files_for(&sample_id(), vec![&diff_with_two_hunks()]));
        assert_eq!(s.active_tab().state.files_focus, FilesPane::Tree);
        update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        );
        assert_eq!(s.active_tab().state.files_focus, FilesPane::Diff);
        assert_eq!(s.active_tab().state.diff_file, None);
    }

    #[test]
    fn split_diff_pane_t_toggles_diff_mode() {
        use crate::app::state::{DiffMode, FilesPane};
        let (mut s, _id) = files_split_state(files_for(&sample_id(), vec![&diff_with_two_hunks()]));
        update(&mut s, &key('l')); // focus Diff
        assert_eq!(s.active_tab().state.files_focus, FilesPane::Diff);
        assert_eq!(s.active_tab().state.diff_mode, DiffMode::Unified);
        update(&mut s, &key('t'));
        assert_eq!(s.active_tab().state.diff_mode, DiffMode::SideBySide);
    }

    #[test]
    fn split_diff_pane_w_toggles_whitespace() {
        use crate::app::state::FilesPane;
        let (mut s, _id) = files_split_state(files_for(&sample_id(), vec![&diff_with_two_hunks()]));
        update(&mut s, &key('l')); // focus Diff
        assert_eq!(s.active_tab().state.files_focus, FilesPane::Diff);
        assert!(!s.active_tab().state.diff_whitespace_hidden);
        update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Char('W'), KeyModifiers::SHIFT)),
        );
        assert!(s.active_tab().state.diff_whitespace_hidden);
    }

    #[test]
    fn split_diff_pane_diff_file_stays_none() {
        use crate::app::state::FilesPane;
        let (mut s, _id) = files_split_state(files_for(
            &sample_id(),
            vec![&diff_with_two_hunks(), &diff_with_two_hunks()],
        ));
        update(&mut s, &key('l')); // focus Diff
        assert_eq!(s.active_tab().state.files_focus, FilesPane::Diff);
        // `}` moves file selection, NOT diff_file
        update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Char('}'), KeyModifiers::NONE)),
        );
        assert_eq!(
            s.active_tab().state.diff_file,
            None,
            "diff_file must stay None in split mode"
        );
        let Selection::DetailFile(sel) = *s.selection() else {
            panic!("expected DetailFile");
        };
        assert_eq!(sel, 1, "`}}` must advance file selection");
    }

    #[test]
    fn diff_viewer_bracket_advances_to_next_hunk_start() {
        let (mut s, _id) =
            diff_state_with_files(files_for(&sample_id(), vec![&diff_with_two_hunks()]));
        update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Char(']'), KeyModifiers::NONE)),
        );
        let cursor1 = s.active_tab().state.diff_cursor;
        assert!(cursor1 > 0, "first ] should advance off line 0");
        update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Char(']'), KeyModifiers::NONE)),
        );
        let cursor2 = s.active_tab().state.diff_cursor;
        assert!(cursor2 > cursor1, "second ] should advance past first hunk");
        update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Char(']'), KeyModifiers::NONE)),
        );
        assert_eq!(
            s.active_tab().state.diff_cursor,
            cursor2,
            "no third hunk; cursor stays"
        );
    }

    #[test]
    fn diff_viewer_left_bracket_goes_to_previous_hunk() {
        let (mut s, _id) =
            diff_state_with_files(files_for(&sample_id(), vec![&diff_with_two_hunks()]));
        let index = DiffPositionIndex::parse(&diff_with_two_hunks());
        let second_hunk = index
            .next_hunk_after(0)
            .and_then(|i| index.next_hunk_after(i))
            .unwrap();
        s.active_tab_mut().state.diff_cursor = second_hunk;
        update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Char('['), KeyModifiers::NONE)),
        );
        let cursor = s.active_tab().state.diff_cursor;
        assert!(cursor < second_hunk);
    }

    #[test]
    fn diff_viewer_brace_cycles_to_next_file_and_resets_cursor() {
        let (mut s, _id) = diff_state_with_files(files_for(
            &sample_id(),
            vec![&diff_with_two_hunks(), &diff_with_two_hunks()],
        ));
        s.active_tab_mut().state.diff_cursor = 5;
        update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Char('}'), KeyModifiers::NONE)),
        );
        match s.current_view() {
            View::Diff { file_index, .. } => assert_eq!(file_index, 1),
            other => panic!("expected View::Diff, got {other:?}"),
        }
        assert_eq!(Some(&s.active_tab().state.diff_cursor), Some(&0));
        update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Char('}'), KeyModifiers::NONE)),
        );
        match s.current_view() {
            View::Diff { file_index, .. } => assert_eq!(file_index, 0, "should wrap"),
            other => panic!("expected View::Diff, got {other:?}"),
        }
    }

    #[test]
    fn diff_viewer_open_brace_cycles_to_previous_file_with_wrap() {
        let (mut s, _id) = diff_state_with_files(files_for(
            &sample_id(),
            vec![&diff_with_two_hunks(), &diff_with_two_hunks()],
        ));
        update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Char('{'), KeyModifiers::NONE)),
        );
        match s.current_view() {
            View::Diff { file_index, .. } => assert_eq!(file_index, 1, "wrap from 0 to last"),
            other => panic!("expected View::Diff, got {other:?}"),
        }
    }

    #[test]
    fn diff_viewer_esc_returns_to_files_tab() {
        let (mut s, _id) =
            diff_state_with_files(files_for(&sample_id(), vec![&diff_with_two_hunks()]));
        update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        );
        assert!(matches!(
            s.current_view(),
            View::PrDetail {
                tab: DetailTab::Files,
                ..
            }
        ));
        assert_eq!(Some(&s.active_tab().state.diff_cursor), Some(&0));
    }

    #[test]
    fn diff_viewer_shift_w_toggles_whitespace_hidden_flag() {
        let (mut s, _id) =
            diff_state_with_files(files_for(&sample_id(), vec![&diff_with_two_hunks()]));
        assert!(!s.active_tab().state.diff_whitespace_hidden);
        update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Char('W'), KeyModifiers::SHIFT)),
        );
        assert_eq!(
            Some(&s.active_tab().state.diff_whitespace_hidden),
            Some(&true)
        );
        update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Char('W'), KeyModifiers::SHIFT)),
        );
        assert_eq!(
            Some(&s.active_tab().state.diff_whitespace_hidden),
            Some(&false)
        );
    }

    #[test]
    fn diff_viewer_t_toggles_diff_mode_between_unified_and_side_by_side() {
        use crate::app::state::DiffMode;
        let (mut s, _id) =
            diff_state_with_files(files_for(&sample_id(), vec![&diff_with_two_hunks()]));
        assert_eq!(s.active_tab().state.diff_mode, DiffMode::Unified);
        let effects = update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE)),
        );
        assert_eq!(s.active_tab().state.diff_mode, DiffMode::SideBySide);
        assert!(effects.is_empty(), "toggle must not emit effects");
        update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE)),
        );
        assert_eq!(s.active_tab().state.diff_mode, DiffMode::Unified);
    }

    #[test]
    fn diff_viewer_t_does_nothing_outside_diff_view() {
        use crate::app::state::DiffMode;
        let mut s = AppState::default();
        assert_eq!(s.active_tab().state.diff_mode, DiffMode::Unified);
        update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE)),
        );
        assert_eq!(
            s.active_tab().state.diff_mode,
            DiffMode::Unified,
            "'t' outside View::Diff must not toggle diff_mode"
        );
    }

    fn large_pr_detail(id: &PrId, changed_files: u32) -> crate::data::models::PrDetail {
        let mut detail = sample_pr_detail(id);
        detail.summary.changed_files = changed_files;
        detail
    }

    fn files_tab_state_with_detail(id: PrId, detail: crate::data::models::PrDetail) -> AppState {
        let mut s = state_for(View::PrDetail {
            id: id.clone(),
            tab: DetailTab::Files,
        });
        s.pr_details.insert(id, detail);
        s
    }

    #[test]
    fn tick_files_tab_skips_fetch_when_changed_files_exceeds_threshold() {
        let id = sample_id();
        let detail = large_pr_detail(&id, LARGE_PR_FILE_THRESHOLD + 1);
        let mut s = files_tab_state_with_detail(id.clone(), detail);
        let effects = update(&mut s, &AppEvent::Tick);
        assert!(
            !effects
                .iter()
                .any(|e| matches!(e, Effect::FetchPrFiles { .. })),
            "large PR Files tab must NOT auto-fetch; got {effects:?}"
        );
    }

    #[test]
    fn shift_l_in_files_tab_sets_load_requested_and_next_tick_fetches() {
        let id = sample_id();
        let detail = large_pr_detail(&id, LARGE_PR_FILE_THRESHOLD + 1);
        let mut s = files_tab_state_with_detail(id.clone(), detail);
        let load_effects = update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Char('L'), KeyModifiers::SHIFT)),
        );
        assert!(load_effects.is_empty(), "L returns no effects directly");
        assert!(s.pr_files_load_requested.contains(&id));
        let tick_effects = update(&mut s, &AppEvent::Tick);
        assert!(
            tick_effects
                .iter()
                .any(|e| matches!(e, Effect::FetchPrFiles { id: i, .. } if i == &id)),
            "tick after L must emit FetchPrFiles; got {tick_effects:?}"
        );
    }

    #[test]
    fn pr_file_diff_loaded_replaces_file_in_pr_files_and_clears_inflight() {
        use crate::data::models::{FileDiff, FileStatus, PrFiles};
        let id = sample_id();
        let mut s = AppState::default();
        let path = "src/a.rs".to_string();
        s.pr_files.insert(
            id.clone(),
            PrFiles {
                files: vec![FileDiff {
                    path: path.clone(),
                    previous_path: None,
                    status: FileStatus::Modified,
                    additions: 1,
                    deletions: 1,
                    patch: None,
                    is_binary: false,
                    is_submodule: false,
                    is_generated: None,
                    oversize: false,
                    blob_sha_before: None,
                    blob_sha_after: None,
                }],
                truncated: false,
            },
        );
        s.pr_file_diff_inflight.insert((id.clone(), path.clone()));
        let loaded = FileDiff {
            path: path.clone(),
            previous_path: None,
            status: FileStatus::Modified,
            additions: 1,
            deletions: 1,
            patch: Some("@@ -1 +1 @@\n-a\n+b\n".into()),
            is_binary: false,
            is_submodule: false,
            is_generated: None,
            oversize: false,
            blob_sha_before: None,
            blob_sha_after: None,
        };
        update(
            &mut s,
            &AppEvent::Data(DataEvent::PrFileDiffLoaded {
                id: id.clone(),
                head_sha: "abc".into(),
                path: path.clone(),
                result: Box::new(Ok(loaded)),
            }),
        );
        assert!(!s.pr_file_diff_inflight.contains(&(id.clone(), path)));
        assert!(s.pr_files.get(&id).unwrap().files[0].patch.is_some());
    }

    #[test]
    fn c_on_pr_detail_opens_pr_comment_composer() {
        let (mut s, id) = detail_with_threads(&["t1"]);
        update(&mut s, &key('c'));
        let composer = s.composer.as_ref().expect("composer should open");
        assert!(composer.body.is_empty());
        match &composer.context {
            ComposerContext::PrComment { pr_id } => assert_eq!(pr_id, &id),
            ComposerContext::ReplyToThread { .. } => {
                panic!("expected PrComment, got ReplyToThread")
            }
            ComposerContext::LineComment { .. } => {
                panic!("expected PrComment, got LineComment")
            }
        }
    }

    #[test]
    fn c_does_not_open_composer_outside_pr_detail() {
        let mut s = AppState::default();
        update(&mut s, &key('c'));
        assert!(s.composer.is_none());
    }

    #[test]
    fn pr_comment_composer_ctrl_s_emits_post_pr_comment() {
        let (mut s, id) = detail_with_threads(&["t1"]);
        update(&mut s, &key('c'));
        update(&mut s, &key('h'));
        update(&mut s, &key('i'));
        let effects = update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL)),
        );
        assert!(s.composer.is_none());
        assert_eq!(effects.len(), 1);
        match &effects[0] {
            Effect::ExecuteWrite {
                kind: crate::data::cache::WriteKind::PostPrComment { id: write_id },
                request: crate::app::effect::WriteRequest::PostPrComment { body },
                version_at_submit,
            } => {
                assert_eq!(write_id, &id);
                assert_eq!(body, "hi");
                assert_eq!(*version_at_submit, 7);
            }
            other => panic!("expected ExecuteWrite PostPrComment, got {other:?}"),
        }
    }

    #[test]
    fn pr_comment_composer_empty_body_does_not_submit() {
        let (mut s, _) = detail_with_threads(&["t1"]);
        update(&mut s, &key('c'));
        let effects = update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL)),
        );
        assert!(effects.is_empty());
        assert!(s.composer.is_some(), "composer stays open on empty submit");
    }

    #[test]
    fn v_on_pr_detail_opens_review_modal_default_approve() {
        let (mut s, id) = detail_with_threads(&["t1"]);
        update(&mut s, &key('v'));
        let modal = s.review_modal.as_ref().expect("modal should open");
        assert_eq!(modal.pr_id, id);
        assert_eq!(modal.selected, 0);
        assert!(modal.body.is_empty());
    }

    #[test]
    fn v_does_not_open_modal_outside_pr_detail() {
        let mut s = AppState::default();
        update(&mut s, &key('v'));
        assert!(s.review_modal.is_none());
    }

    #[test]
    fn review_modal_arrows_cycle_and_clamp() {
        let (mut s, _) = detail_with_threads(&["t1"]);
        update(&mut s, &key('v'));
        let down = AppEvent::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        let up = AppEvent::Key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        update(&mut s, &down);
        assert_eq!(s.review_modal.as_ref().unwrap().selected, 1);
        update(&mut s, &down);
        update(&mut s, &down);
        assert_eq!(s.review_modal.as_ref().unwrap().selected, 2);
        update(&mut s, &up);
        assert_eq!(s.review_modal.as_ref().unwrap().selected, 1);
    }

    #[test]
    fn review_modal_typing_appends_to_body_including_jk_and_digits() {
        let (mut s, _) = detail_with_threads(&["t1"]);
        update(&mut s, &key('v'));
        for c in "jk123".chars() {
            update(&mut s, &key(c));
        }
        assert_eq!(s.review_modal.as_ref().unwrap().body, "jk123");
    }

    #[test]
    fn review_modal_esc_cancels() {
        let (mut s, _) = detail_with_threads(&["t1"]);
        update(&mut s, &key('v'));
        update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        );
        assert!(s.review_modal.is_none());
    }

    #[test]
    fn review_modal_ctrl_s_approve_emits_submit_review_with_head_sha() {
        let (mut s, id) = detail_with_threads(&["t1"]);
        update(&mut s, &key('v'));
        update(&mut s, &key('l'));
        update(&mut s, &key('g'));
        update(&mut s, &key('t'));
        update(&mut s, &key('m'));
        let effects = update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL)),
        );
        assert!(s.review_modal.is_none());
        assert_eq!(effects.len(), 1);
        match &effects[0] {
            Effect::ExecuteWrite {
                kind: crate::data::cache::WriteKind::SubmitReview { id: wid },
                request:
                    crate::app::effect::WriteRequest::SubmitReview {
                        state,
                        body,
                        line_comments,
                        head_sha,
                    },
                version_at_submit,
            } => {
                assert_eq!(wid, &id);
                assert_eq!(*state, crate::data::models::ReviewState::Approved);
                assert_eq!(body, "lgtm");
                assert!(line_comments.is_empty());
                assert_eq!(head_sha, "deadbeef");
                assert_eq!(*version_at_submit, 7);
            }
            other => panic!("expected ExecuteWrite SubmitReview, got {other:?}"),
        }
    }

    #[test]
    fn review_modal_ctrl_s_comment_empty_body_does_not_submit() {
        let (mut s, _) = detail_with_threads(&["t1"]);
        update(&mut s, &key('v'));
        let down = AppEvent::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        update(&mut s, &down);
        update(&mut s, &down);
        let effects = update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL)),
        );
        assert!(effects.is_empty());
        assert!(
            s.review_modal.is_some(),
            "modal stays open on empty Comment"
        );
    }

    #[test]
    fn review_modal_ctrl_s_approve_with_empty_body_still_submits() {
        let (mut s, _) = detail_with_threads(&["t1"]);
        update(&mut s, &key('v'));
        let effects = update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL)),
        );
        assert!(s.review_modal.is_none());
        assert_eq!(effects.len(), 1);
        match &effects[0] {
            Effect::ExecuteWrite {
                request: crate::app::effect::WriteRequest::SubmitReview { state, body, .. },
                ..
            } => {
                assert_eq!(*state, crate::data::models::ReviewState::Approved);
                assert!(body.is_empty());
            }
            other => panic!("expected SubmitReview, got {other:?}"),
        }
    }

    #[test]
    fn diff_viewer_shift_v_enters_line_select_at_cursor() {
        let (mut s, _id) =
            diff_state_with_files(files_for(&sample_id(), vec![&diff_with_two_hunks()]));
        s.active_tab_mut().state.diff_cursor = 3;
        update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::SHIFT)),
        );
        let sel = s
            .active_tab()
            .state
            .diff_selection
            .as_ref()
            .expect("selection set");
        assert_eq!(sel.anchor, 3);
        assert_eq!(sel.cursor, 3);
        assert_eq!(sel.range(), (3, 3));
    }

    #[test]
    fn diff_viewer_line_select_j_extends_cursor_anchor_stays() {
        let (mut s, _id) =
            diff_state_with_files(files_for(&sample_id(), vec![&diff_with_two_hunks()]));
        s.active_tab_mut().state.diff_cursor = 2;
        update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::SHIFT)),
        );
        update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE)),
        );
        update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE)),
        );
        let sel = s
            .active_tab()
            .state
            .diff_selection
            .as_ref()
            .expect("selection still set");
        assert_eq!(sel.anchor, 2);
        assert_eq!(sel.cursor, 4);
        assert_eq!(sel.range(), (2, 4));
    }

    #[test]
    fn diff_viewer_line_select_k_above_anchor_inverts_range() {
        let (mut s, _id) =
            diff_state_with_files(files_for(&sample_id(), vec![&diff_with_two_hunks()]));
        s.active_tab_mut().state.diff_cursor = 5;
        update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::SHIFT)),
        );
        update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE)),
        );
        update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE)),
        );
        let sel = s
            .active_tab()
            .state
            .diff_selection
            .as_ref()
            .expect("selection still set");
        assert_eq!(sel.anchor, 5);
        assert_eq!(sel.cursor, 3);
        assert_eq!(sel.range(), (3, 5));
    }

    #[test]
    fn diff_viewer_line_select_esc_clears_selection_stays_in_diff_view() {
        let (mut s, _id) =
            diff_state_with_files(files_for(&sample_id(), vec![&diff_with_two_hunks()]));
        update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::SHIFT)),
        );
        assert!(s.active_tab().state.diff_selection.is_some());
        update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        );
        assert!(s.active_tab().state.diff_selection.is_none());
        assert!(matches!(s.current_view(), View::Diff { .. }));
    }

    #[test]
    fn diff_viewer_line_select_blocks_hunk_nav_keys() {
        let (mut s, _id) =
            diff_state_with_files(files_for(&sample_id(), vec![&diff_with_two_hunks()]));
        s.active_tab_mut().state.diff_cursor = 2;
        update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::SHIFT)),
        );
        let cursor_before = s.active_tab().state.diff_cursor;
        update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Char(']'), KeyModifiers::NONE)),
        );
        let cursor_after = s.active_tab().state.diff_cursor;
        assert_eq!(cursor_before, cursor_after, "] no-op in line-select mode");
        let sel = s
            .active_tab()
            .state
            .diff_selection
            .as_ref()
            .expect("selection still set");
        assert_eq!(sel.cursor, 2, "] does not move selection cursor");
    }

    #[test]
    fn diff_viewer_line_select_j_clamps_at_last_line() {
        let patch = diff_with_two_hunks();
        let line_count = patch.split_inclusive('\n').count();
        let (mut s, _id) = diff_state_with_files(files_for(&sample_id(), vec![&patch]));
        s.active_tab_mut().state.diff_cursor = line_count - 1;
        update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::SHIFT)),
        );
        update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE)),
        );
        let sel = s.active_tab().state.diff_selection.as_ref().unwrap();
        assert_eq!(sel.cursor, line_count - 1, "j clamps at last line");
    }

    fn diff_state_with_detail(head_sha: &str) -> (AppState, PrId) {
        let (mut detail_state, id) = detail_with_threads(&[]);
        if let Some(detail) = detail_state.pr_details.get_mut(&id) {
            detail.head_sha = head_sha.to_string();
        }
        let mut s = state_for(View::Diff {
            id: id.clone(),
            file_index: 0,
        });
        s.pr_files
            .insert(id.clone(), files_for(&id, vec![&diff_with_two_hunks()]));
        s.pr_details = detail_state.pr_details;
        (s, id)
    }

    #[test]
    fn line_select_c_opens_composer_with_built_line_comment_and_head_sha() {
        let (mut s, id) = diff_state_with_detail("oldsha");
        let patch = diff_with_two_hunks();
        let idx = DiffPositionIndex::parse(&patch);
        let target = idx
            .lines
            .iter()
            .position(|p| p.side_and_line.is_some())
            .expect("at least one positioned line");
        s.active_tab_mut().state.diff_cursor = target;
        update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::SHIFT)),
        );
        update(&mut s, &key('c'));

        assert!(
            s.active_tab().state.diff_selection.is_none(),
            "selection cleared when composer opens"
        );
        let composer = s.composer.as_ref().expect("composer open");
        assert!(composer.body.is_empty());
        match &composer.context {
            ComposerContext::LineComment {
                pr_id,
                head_sha,
                comment,
            } => {
                assert_eq!(pr_id, &id);
                assert_eq!(head_sha, "oldsha");
                let expected = idx.build_line_comment(target, target).unwrap();
                assert_eq!(comment.path, expected.path);
                assert_eq!(comment.side, expected.side);
                assert_eq!(comment.line, expected.line);
                assert_eq!(comment.start_line, None);
            }
            other => panic!("expected LineComment context, got {other:?}"),
        }
    }

    #[test]
    fn line_select_c_with_header_only_range_toasts_and_does_not_open() {
        let (mut s, _id) = diff_state_with_detail("oldsha");
        s.active_tab_mut().state.diff_cursor = 0;
        update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::SHIFT)),
        );
        update(&mut s, &key('c'));
        assert!(s.composer.is_none(), "no composer for header-only range");
        assert!(matches!(s.toast.as_ref().unwrap().kind, ToastKind::Warning));
    }

    #[test]
    fn line_comment_composer_ctrl_s_emits_execute_write_with_head_sha_and_body() {
        let (mut s, id) = diff_state_with_detail("oldsha");
        let patch = diff_with_two_hunks();
        let idx = DiffPositionIndex::parse(&patch);
        let target = idx
            .lines
            .iter()
            .position(|p| p.side_and_line.is_some())
            .unwrap();
        s.active_tab_mut().state.diff_cursor = target;
        update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::SHIFT)),
        );
        update(&mut s, &key('c'));
        for ch in "looks wrong".chars() {
            update(&mut s, &key(ch));
        }
        let effects = update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL)),
        );
        assert!(s.composer.is_none(), "composer closes on submit");
        assert_eq!(effects.len(), 1);
        match &effects[0] {
            Effect::ExecuteWrite {
                kind,
                request,
                version_at_submit: _,
            } => {
                assert_eq!(
                    kind,
                    &crate::data::cache::WriteKind::PostLineComment { id: id.clone() }
                );
                match request {
                    crate::app::effect::WriteRequest::PostLineComment { head_sha, comment } => {
                        assert_eq!(head_sha, "oldsha");
                        assert_eq!(comment.body, "looks wrong");
                    }
                    other => panic!("expected PostLineComment request, got {other:?}"),
                }
            }
            other => panic!("expected ExecuteWrite, got {other:?}"),
        }
    }

    fn detail_state_with_mergeable(
        mergeable: crate::data::models::Mergeable,
        methods: Vec<crate::data::models::MergeMethod>,
    ) -> (AppState, PrId) {
        let id = PrId {
            repo: Repo {
                owner: "acme".into(),
                name: "widgets".into(),
            },
            number: 1,
        };
        let mut detail = sample_pr_detail(&id);
        detail.summary.mergeable = mergeable;
        detail.available_merge_methods = methods;
        let mut s = state_for(View::PrDetail {
            id: id.clone(),
            tab: DetailTab::Conversation,
        });
        s.pr_details.insert(id.clone(), detail);
        (s, id)
    }

    fn press_m() -> KeyEvent {
        KeyEvent::new(KeyCode::Char('m'), KeyModifiers::NONE)
    }

    #[test]
    fn merge_modal_clean_opens_method_picker_with_available_methods() {
        use crate::app::state::{MergeModalKind, MergeModalState};
        use crate::data::models::{MergeMethod, Mergeable};
        let (mut s, id) = detail_state_with_mergeable(
            Mergeable::Clean,
            vec![MergeMethod::Squash, MergeMethod::Merge],
        );
        let effects = update(&mut s, &AppEvent::Key(press_m()));
        assert!(effects.is_empty());
        let modal = s.merge_modal.as_ref().expect("merge modal opens on m");
        assert_eq!(modal.pr_id, id);
        assert_eq!(modal.head_sha, "deadbeef");
        let MergeModalState {
            kind: MergeModalKind::MethodPicker { methods, selected },
            ..
        } = modal
        else {
            panic!("expected MethodPicker, got {:?}", modal.kind);
        };
        assert_eq!(methods, &vec![MergeMethod::Squash, MergeMethod::Merge]);
        assert_eq!(*selected, 0);
        assert!(s.toast.is_none(), "method picker emits no toast");
    }

    #[test]
    fn merge_modal_unstable_and_has_hooks_also_open_method_picker() {
        use crate::app::state::MergeModalKind;
        use crate::data::models::{MergeMethod, Mergeable};
        for variant in [Mergeable::Unstable, Mergeable::HasHooks] {
            let (mut s, _) = detail_state_with_mergeable(variant, vec![MergeMethod::Rebase]);
            let _ = update(&mut s, &AppEvent::Key(press_m()));
            let modal = s.merge_modal.as_ref().unwrap_or_else(|| {
                panic!("{variant:?} should open method picker");
            });
            assert!(
                matches!(&modal.kind, MergeModalKind::MethodPicker { .. }),
                "{variant:?} kind was {:?}",
                modal.kind
            );
        }
    }

    #[test]
    fn merge_modal_behind_emits_rebase_locally_toast_no_modal() {
        use crate::data::models::Mergeable;
        let (mut s, _) = detail_state_with_mergeable(Mergeable::Behind, vec![]);
        let _ = update(&mut s, &AppEvent::Key(press_m()));
        assert!(s.merge_modal.is_none(), "Behind opens no modal");
        let toast = s.toast.as_ref().expect("Behind emits toast");
        assert_eq!(
            toast.message,
            "Base branch moved \u{2014} merge would create a non-fast-forward; rebase locally"
        );
        assert_eq!(toast.kind, ToastKind::Warning);
    }

    #[test]
    fn merge_modal_blocked_opens_blocked_modal_with_reasons() {
        use crate::app::state::MergeModalKind;
        use crate::data::models::Mergeable;
        let (mut s, _) = detail_state_with_mergeable(Mergeable::Blocked, vec![]);
        let _ = update(&mut s, &AppEvent::Key(press_m()));
        let modal = s.merge_modal.as_ref().expect("Blocked opens blocked modal");
        let MergeModalKind::Blocked { reasons } = &modal.kind else {
            panic!("expected Blocked kind, got {:?}", modal.kind);
        };
        assert!(!reasons.is_empty(), "Blocked modal shows >=1 reason");
    }

    #[test]
    fn merge_modal_dirty_emits_resolve_locally_toast() {
        use crate::data::models::Mergeable;
        let (mut s, _) = detail_state_with_mergeable(Mergeable::Dirty, vec![]);
        let _ = update(&mut s, &AppEvent::Key(press_m()));
        assert!(s.merge_modal.is_none());
        let toast = s.toast.as_ref().expect("Dirty emits toast");
        assert_eq!(toast.message, "Merge conflicts; resolve locally");
        assert_eq!(toast.kind, ToastKind::Error);
    }

    #[test]
    fn merge_modal_draft_emits_promote_to_ready_toast() {
        use crate::data::models::Mergeable;
        let (mut s, _) = detail_state_with_mergeable(Mergeable::Draft, vec![]);
        let _ = update(&mut s, &AppEvent::Key(press_m()));
        assert!(s.merge_modal.is_none());
        let toast = s.toast.as_ref().expect("Draft emits toast");
        assert_eq!(
            toast.message,
            "PR is a draft \u{2014} promote-to-ready required (v2)"
        );
        assert_eq!(toast.kind, ToastKind::Warning);
    }

    #[test]
    fn merge_modal_unknown_emits_computing_mergeability_toast() {
        use crate::data::models::Mergeable;
        let (mut s, _) = detail_state_with_mergeable(Mergeable::Unknown, vec![]);
        let _ = update(&mut s, &AppEvent::Key(press_m()));
        assert!(s.merge_modal.is_none());
        let toast = s.toast.as_ref().expect("Unknown emits toast");
        assert_eq!(
            toast.message,
            "GitHub still computing mergeability; try again in a moment"
        );
        assert_eq!(toast.kind, ToastKind::Info);
    }

    #[test]
    fn merge_modal_method_picker_arrows_cycle_and_clamp_esc_closes() {
        use crate::app::state::MergeModalKind;
        use crate::data::models::{MergeMethod, Mergeable};
        let (mut s, _) = detail_state_with_mergeable(
            Mergeable::Clean,
            vec![MergeMethod::Squash, MergeMethod::Merge, MergeMethod::Rebase],
        );
        let _ = update(&mut s, &AppEvent::Key(press_m()));
        let selected_of = |s: &AppState| match &s.merge_modal.as_ref().unwrap().kind {
            MergeModalKind::MethodPicker { selected, .. } => *selected,
            MergeModalKind::Blocked { .. } => panic!("expected MethodPicker"),
        };
        let _ = update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)),
        );
        assert_eq!(selected_of(&s), 1);
        let _ = update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)),
        );
        assert_eq!(selected_of(&s), 2);
        let _ = update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)),
        );
        assert_eq!(selected_of(&s), 2, "clamped at last method");
        let _ = update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)),
        );
        assert_eq!(selected_of(&s), 1);
        let _ = update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        );
        assert!(s.merge_modal.is_none(), "Esc closes merge modal");
    }

    #[test]
    fn merge_modal_enter_emits_execute_write_merge_with_method_and_head_sha() {
        use crate::app::effect::WriteRequest;
        use crate::data::cache::WriteKind;
        use crate::data::models::{MergeMethod, Mergeable};
        let (mut s, id) = detail_state_with_mergeable(
            Mergeable::Clean,
            vec![MergeMethod::Squash, MergeMethod::Merge],
        );
        let _ = update(&mut s, &AppEvent::Key(press_m()));
        let _ = update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)),
        );
        let effects = update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        );
        assert!(
            s.merge_modal.is_none(),
            "modal closes on submit so user can't double-fire"
        );
        assert_eq!(effects.len(), 1);
        match &effects[0] {
            Effect::ExecuteWrite {
                kind: WriteKind::Merge { id: kid },
                request: WriteRequest::Merge { method, head_sha },
                ..
            } => {
                assert_eq!(kid, &id);
                assert_eq!(
                    *method,
                    MergeMethod::Merge,
                    "uses currently-selected method"
                );
                assert_eq!(head_sha, "deadbeef");
            }
            other => panic!("expected ExecuteWrite Merge, got {other:?}"),
        }
    }

    #[test]
    fn merge_modal_enter_on_empty_methods_is_no_op() {
        use crate::data::models::{MergeMethod, Mergeable};
        let (mut s, _) = detail_state_with_mergeable(Mergeable::Clean, vec![] as Vec<MergeMethod>);
        let _ = update(&mut s, &AppEvent::Key(press_m()));
        let effects = update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        );
        assert!(effects.is_empty(), "no methods → no submit");
        assert!(s.merge_modal.is_some(), "modal stays open");
    }

    fn detail_state_with_pr_state(pr_state: crate::data::models::PrState) -> (AppState, PrId) {
        let id = PrId {
            repo: Repo {
                owner: "acme".into(),
                name: "widgets".into(),
            },
            number: 1,
        };
        let mut detail = sample_pr_detail(&id);
        detail.summary.state = pr_state;
        let mut s = state_for(View::PrDetail {
            id: id.clone(),
            tab: DetailTab::Conversation,
        });
        s.pr_details.insert(id.clone(), detail);
        (s, id)
    }

    fn press_x() -> KeyEvent {
        KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE)
    }

    #[test]
    fn x_on_open_pr_opens_close_confirm_with_close_action() {
        use crate::app::state::CloseReopenAction;
        use crate::data::models::PrState;
        let (mut s, id) = detail_state_with_pr_state(PrState::Open);
        let _ = update(&mut s, &AppEvent::Key(press_x()));
        let confirm = s
            .close_reopen_confirm
            .as_ref()
            .expect("x opens close-confirm");
        assert_eq!(confirm.pr_id, id);
        assert_eq!(confirm.action, CloseReopenAction::Close);
    }

    #[test]
    fn x_on_closed_pr_opens_reopen_confirm() {
        use crate::app::state::CloseReopenAction;
        use crate::data::models::PrState;
        let (mut s, _) = detail_state_with_pr_state(PrState::Closed);
        let _ = update(&mut s, &AppEvent::Key(press_x()));
        let confirm = s
            .close_reopen_confirm
            .as_ref()
            .expect("opens reopen-confirm");
        assert_eq!(confirm.action, CloseReopenAction::Reopen);
    }

    #[test]
    fn x_on_merged_pr_emits_toast_no_confirm() {
        use crate::data::models::PrState;
        let (mut s, _) = detail_state_with_pr_state(PrState::Merged);
        let _ = update(&mut s, &AppEvent::Key(press_x()));
        assert!(s.close_reopen_confirm.is_none());
        let toast = s.toast.as_ref().expect("merged → toast");
        assert!(toast.message.contains("merged"));
    }

    #[test]
    fn close_confirm_esc_cancels_no_effects() {
        use crate::data::models::PrState;
        let (mut s, _) = detail_state_with_pr_state(PrState::Open);
        let _ = update(&mut s, &AppEvent::Key(press_x()));
        let effects = update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        );
        assert!(effects.is_empty());
        assert!(s.close_reopen_confirm.is_none());
    }

    #[test]
    fn close_confirm_enter_emits_execute_write_close() {
        use crate::app::effect::WriteRequest;
        use crate::data::cache::WriteKind;
        use crate::data::models::PrState;
        let (mut s, id) = detail_state_with_pr_state(PrState::Open);
        let _ = update(&mut s, &AppEvent::Key(press_x()));
        let effects = update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        );
        assert!(s.close_reopen_confirm.is_none(), "confirm closes on submit");
        assert_eq!(effects.len(), 1);
        match &effects[0] {
            Effect::ExecuteWrite {
                kind: WriteKind::Close { id: kid },
                request: WriteRequest::Close,
                ..
            } => assert_eq!(kid, &id),
            other => panic!("expected ExecuteWrite Close, got {other:?}"),
        }
    }

    #[test]
    fn reopen_confirm_y_emits_execute_write_reopen() {
        use crate::app::effect::WriteRequest;
        use crate::data::cache::WriteKind;
        use crate::data::models::PrState;
        let (mut s, id) = detail_state_with_pr_state(PrState::Closed);
        let _ = update(&mut s, &AppEvent::Key(press_x()));
        let effects = update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE)),
        );
        assert_eq!(effects.len(), 1);
        match &effects[0] {
            Effect::ExecuteWrite {
                kind: WriteKind::Reopen { id: kid },
                request: WriteRequest::Reopen,
                ..
            } => assert_eq!(kid, &id),
            other => panic!("expected ExecuteWrite Reopen, got {other:?}"),
        }
    }

    fn press_cap_c() -> KeyEvent {
        KeyEvent::new(KeyCode::Char('C'), KeyModifiers::NONE)
    }

    #[test]
    fn checkout_c_with_dirty_worktree_emits_toast_no_effect() {
        use crate::app::state::RepoStatus;
        use crate::data::models::PrState;
        let (mut s, _) = detail_state_with_pr_state(PrState::Open);
        s.repo_status = Some(RepoStatus { dirty: true });
        let effects = update(&mut s, &AppEvent::Key(press_cap_c()));
        assert!(effects.is_empty());
        assert!(s.checkout_inflight.is_none());
        let toast = s.toast.as_ref().expect("dirty → toast");
        assert!(toast.message.to_lowercase().contains("dirty"));
    }

    #[test]
    fn checkout_c_on_clean_worktree_emits_checkout_branch_and_sets_inflight() {
        use crate::app::state::RepoStatus;
        use crate::data::models::PrState;
        let (mut s, id) = detail_state_with_pr_state(PrState::Open);
        s.repo_status = Some(RepoStatus { dirty: false });
        let effects = update(&mut s, &AppEvent::Key(press_cap_c()));
        assert_eq!(effects.len(), 1);
        match &effects[0] {
            Effect::CheckoutBranch { id: kid } => assert_eq!(kid, &id),
            other => panic!("expected CheckoutBranch, got {other:?}"),
        }
        assert_eq!(s.checkout_inflight.as_ref(), Some(&id));
    }

    #[test]
    fn checkout_c_outside_git_repo_emits_toast_no_effect() {
        use crate::data::models::PrState;
        let (mut s, _) = detail_state_with_pr_state(PrState::Open);
        let effects = update(&mut s, &AppEvent::Key(press_cap_c()));
        assert!(effects.is_empty());
        let toast = s.toast.as_ref().expect("no-repo → toast");
        assert!(toast.message.to_lowercase().contains("not in a git repo"));
    }

    #[test]
    fn checkout_c_while_inflight_is_no_op() {
        use crate::app::state::RepoStatus;
        use crate::data::models::PrState;
        let (mut s, id) = detail_state_with_pr_state(PrState::Open);
        s.repo_status = Some(RepoStatus { dirty: false });
        s.checkout_inflight = Some(id.clone());
        let effects = update(&mut s, &AppEvent::Key(press_cap_c()));
        assert!(effects.is_empty());
    }

    #[test]
    fn checkout_completed_ok_clears_inflight_and_toasts_branch() {
        use crate::app::effect::DataEvent;
        use crate::app::state::RepoStatus;
        use crate::data::models::PrState;
        let (mut s, id) = detail_state_with_pr_state(PrState::Open);
        s.repo_status = Some(RepoStatus { dirty: false });
        let _ = update(&mut s, &AppEvent::Key(press_cap_c()));
        let effects = update(
            &mut s,
            &AppEvent::Data(DataEvent::CheckoutCompleted {
                id: id.clone(),
                result: Ok("pr/1".into()),
            }),
        );
        assert!(effects.is_empty());
        assert!(s.checkout_inflight.is_none());
        assert!(s.toast.as_ref().unwrap().message.contains("pr/1"));
    }

    #[test]
    fn checkout_completed_err_clears_inflight_and_toasts_error() {
        use crate::app::effect::DataEvent;
        use crate::app::state::RepoStatus;
        use crate::data::models::PrState;
        let (mut s, id) = detail_state_with_pr_state(PrState::Open);
        s.repo_status = Some(RepoStatus { dirty: false });
        let _ = update(&mut s, &AppEvent::Key(press_cap_c()));
        let _ = update(
            &mut s,
            &AppEvent::Data(DataEvent::CheckoutCompleted {
                id: id.clone(),
                result: Err("network down".into()),
            }),
        );
        assert!(s.checkout_inflight.is_none());
        let t = s.toast.as_ref().unwrap();
        assert!(matches!(t.kind, crate::app::state::ToastKind::Error));
        assert!(t.message.contains("network down"));
    }

    #[test]
    fn keymap_default_q_navigates_back_from_pr_list() {
        // q now always quits; use w (CloseTab) to go back.
        let mut s = pr_list_view();
        let _ = update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE)),
        );
        assert!(s.should_quit, "q quits from PR list");
    }

    #[test]
    fn keymap_default_q_quits_from_dashboard_root() {
        let mut s = AppState::default();
        let _ = update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE)),
        );
        assert!(s.should_quit, "q at the Dashboard root quits");
    }

    #[test]
    fn keymap_override_quit_to_capital_q_strips_default_q() {
        // Remapping `quit` from `q` to `Q` means `q` no longer triggers Quit.
        let mut s = AppState::default();
        let mut over = std::collections::HashMap::new();
        over.insert("quit".to_string(), "Q".to_string());
        s.keymap = {
            let mut km = crate::app::keymap::Keymap::vim_defaults();
            km.apply_overrides(&over);
            km
        };
        let _ = update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE)),
        );
        assert!(!s.should_quit, "default q must be stripped by override");
        let _ = update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Char('Q'), KeyModifiers::SHIFT)),
        );
        assert!(s.should_quit, "overridden Q must quit");
    }

    #[test]
    fn dashboard_re_fetches_after_60s_when_idle_and_no_composer() {
        let mut s = AppState {
            dashboard: Some(Vec::new()),
            user_idle_since: secs_ago(10),
            ..AppState::default()
        };
        s.last_refresh.insert(
            crate::data::cache::CacheKey::DashboardReviewRequested,
            secs_ago(61),
        );
        let effects = update(&mut s, &AppEvent::Tick);
        assert!(
            effects
                .iter()
                .any(|e| matches!(e, Effect::FetchDashboard { .. })),
            "expected background re-fetch after 60s, got {effects:?}"
        );
    }

    #[test]
    fn dashboard_does_not_re_fetch_before_60s() {
        let mut s = AppState {
            dashboard: Some(Vec::new()),
            user_idle_since: secs_ago(10),
            ..AppState::default()
        };
        s.last_refresh.insert(
            crate::data::cache::CacheKey::DashboardReviewRequested,
            secs_ago(30),
        );
        let effects = update(&mut s, &AppEvent::Tick);
        assert!(
            !effects
                .iter()
                .any(|e| matches!(e, Effect::FetchDashboard { .. })),
            "must not re-fetch before 60s elapsed, got {effects:?}"
        );
    }

    #[test]
    fn background_refresh_suppressed_while_composer_open() {
        let mut s = AppState {
            dashboard: Some(Vec::new()),
            user_idle_since: secs_ago(60),
            composer: Some(ComposerState {
                context: ComposerContext::PrComment {
                    pr_id: PrId {
                        repo: repo_acme(),
                        number: 1,
                    },
                },
                body: String::new(),
                head_moved: false,
                preview_visible: true,
            }),
            ..AppState::default()
        };
        s.last_refresh.insert(
            crate::data::cache::CacheKey::DashboardReviewRequested,
            secs_ago(120),
        );
        let effects = update(&mut s, &AppEvent::Tick);
        assert!(
            !effects
                .iter()
                .any(|e| matches!(e, Effect::FetchDashboard { .. })),
            "composer-open must suppress background refresh, got {effects:?}"
        );
    }

    #[test]
    fn background_refresh_suppressed_when_user_active_within_5s() {
        let mut s = AppState {
            dashboard: Some(Vec::new()),
            user_idle_since: secs_ago(2),
            ..AppState::default()
        };
        s.last_refresh.insert(
            crate::data::cache::CacheKey::DashboardReviewRequested,
            secs_ago(120),
        );
        let effects = update(&mut s, &AppEvent::Tick);
        assert!(
            !effects
                .iter()
                .any(|e| matches!(e, Effect::FetchDashboard { .. })),
            "user-active within 5s must suppress background refresh, got {effects:?}"
        );
    }

    #[test]
    fn pr_detail_re_fetches_after_60s_when_idle_and_no_composer() {
        let id = PrId {
            repo: repo_acme(),
            number: 7,
        };
        let mut s = state_for(View::PrDetail {
            id: id.clone(),
            tab: crate::app::state::DetailTab::Conversation,
        });
        s.user_idle_since = secs_ago(10);
        s.pr_details.insert(id.clone(), sample_pr_detail(&id));
        s.pr_checks.insert(id.clone(), sample_pr_checks(&id));
        s.last_refresh.insert(
            crate::data::cache::CacheKey::PrDetail(id.clone()),
            secs_ago(61),
        );
        let effects = update(&mut s, &AppEvent::Tick);
        assert!(
            effects
                .iter()
                .any(|e| matches!(e, Effect::FetchPrDetail { id: i } if i == &id)),
            "expected PrDetail background re-fetch after 60s, got {effects:?}"
        );
    }

    #[test]
    fn pr_list_re_fetches_after_60s_when_idle_and_no_composer() {
        let repo = repo_acme();
        let mut s = AppState {
            user_idle_since: secs_ago(10),
            ..AppState::default()
        };
        crate::app::seed_workspace_from_view(&mut s, View::PrList { repo: repo.clone() });
        s.pr_lists.insert(repo.clone(), Vec::new());
        s.last_refresh.insert(
            crate::data::cache::CacheKey::PrList(repo.clone()),
            secs_ago(61),
        );
        let effects = update(&mut s, &AppEvent::Tick);
        assert!(
            effects
                .iter()
                .any(|e| matches!(e, Effect::FetchPrList { repo: r, .. } if r == &repo)),
            "expected PrList background re-fetch after 60s, got {effects:?}"
        );
    }

    // --- rate-limit gating -----------------------------------------------
    fn rl_snapshot(remaining: u32) -> crate::data::github::RateLimitSnapshot {
        crate::data::github::RateLimitSnapshot {
            remaining,
            limit: 5000,
            resets_at: chrono::Utc::now() + chrono::Duration::minutes(15),
        }
    }

    #[test]
    fn rate_limit_below_200_lengthens_interval_to_300s_and_warns_once() {
        let mut s = AppState {
            dashboard: Some(Vec::new()),
            user_idle_since: secs_ago(10),
            ..AppState::default()
        };
        s.last_refresh.insert(
            crate::data::cache::CacheKey::DashboardReviewRequested,
            secs_ago(61),
        );
        let _ = update(
            &mut s,
            &AppEvent::Data(DataEvent::RateLimitUpdate(rl_snapshot(150))),
        );
        let toast1 = s.toast.as_ref().expect("low-rate-limit toast expected");
        assert_eq!(toast1.kind, ToastKind::Warning);
        assert!(
            toast1.message.to_lowercase().contains("rate"),
            "toast must mention rate, got {:?}",
            toast1.message
        );
        s.toast = None;
        let _ = update(
            &mut s,
            &AppEvent::Data(DataEvent::RateLimitUpdate(rl_snapshot(140))),
        );
        assert!(
            s.toast.is_none(),
            "low-rate-limit toast must fire only once, got {:?}",
            s.toast
        );
        let effects = update(&mut s, &AppEvent::Tick);
        assert!(
            !effects
                .iter()
                .any(|e| matches!(e, Effect::FetchDashboard { .. })),
            "below-200 must extend refresh interval to 300s, got {effects:?}"
        );
        s.last_refresh.insert(
            crate::data::cache::CacheKey::DashboardReviewRequested,
            secs_ago(301),
        );
        let effects = update(&mut s, &AppEvent::Tick);
        assert!(
            effects
                .iter()
                .any(|e| matches!(e, Effect::FetchDashboard { .. })),
            "after 300s elapsed at low rate-limit, refresh resumes, got {effects:?}"
        );
    }

    #[test]
    fn rate_limit_below_50_halts_background_refresh_and_errors() {
        let mut s = AppState {
            dashboard: Some(Vec::new()),
            user_idle_since: secs_ago(10),
            ..AppState::default()
        };
        s.last_refresh.insert(
            crate::data::cache::CacheKey::DashboardReviewRequested,
            secs_ago(3600),
        );
        let _ = update(
            &mut s,
            &AppEvent::Data(DataEvent::RateLimitUpdate(rl_snapshot(30))),
        );
        let toast = s
            .toast
            .as_ref()
            .expect("critical-rate-limit toast expected");
        assert_eq!(toast.kind, ToastKind::Error);
        assert!(toast.message.to_lowercase().contains("rate"));
        let effects = update(&mut s, &AppEvent::Tick);
        assert!(
            !effects
                .iter()
                .any(|e| matches!(e, Effect::FetchDashboard { .. })),
            "critical rate-limit must halt background refresh entirely, got {effects:?}"
        );
    }

    #[test]
    fn tab_strip_displays_remaining_limit_and_reset_time() {
        use crate::ui::components::tab_strip::{StatusCluster, render_tab_strip};
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let resets_at = chrono::Utc::now() + chrono::Duration::seconds(742);
        let rl = crate::data::github::RateLimitSnapshot {
            remaining: 123,
            limit: 5000,
            resets_at,
        };
        let backend = TestBackend::new(120, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        let labels = ["\u{2302}".to_string()];
        let status = StatusCluster {
            rate_limit: Some(&rl),
            ..Default::default()
        };
        terminal
            .draw(|f| {
                render_tab_strip(
                    f,
                    f.area(),
                    &labels,
                    0,
                    Some(&status),
                    &crate::ui::theme::Theme::dark(),
                );
            })
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let rendered: String = (0..buf.area().width)
            .map(|x| buf[(x, 0)].symbol().to_string())
            .collect();
        assert!(
            rendered.contains("123/5000"),
            "tab strip must show remaining/limit, got {rendered:?}"
        );
        assert!(
            rendered.contains("reset") && rendered.contains("12m"),
            "tab strip must show time-until-reset, got {rendered:?}"
        );
    }

    fn pr_detail_with_version(id: &PrId, version: u64) -> crate::data::models::PrDetail {
        let mut d = sample_pr_detail(id);
        d.version = version;
        d
    }

    #[test]
    fn write_completed_with_stale_version_emits_warning_and_refetches() {
        let id = PrId {
            repo: repo_acme(),
            number: 9,
        };
        let mut s = state_for(View::PrDetail {
            id: id.clone(),
            tab: crate::app::state::DetailTab::Conversation,
        });
        s.pending_write = Some(crate::app::state::PendingWrite {
            kind: crate::data::cache::WriteKind::PostPrComment { id: id.clone() },
            request: crate::app::effect::WriteRequest::PostPrComment { body: "hi".into() },
            version_at_submit: 3,
            status: crate::app::state::PendingStatus::Submitting,
        });
        s.pr_details
            .insert(id.clone(), pr_detail_with_version(&id, 5));
        let effects = update(
            &mut s,
            &AppEvent::Data(DataEvent::WriteCompleted {
                kind: crate::data::cache::WriteKind::PostPrComment { id: id.clone() },
                result: Ok(()),
            }),
        );
        assert!(s.pending_write.is_none());
        let toast = s.toast.as_ref().expect("stale-write warning expected");
        assert_eq!(toast.kind, ToastKind::Warning);
        assert!(
            toast.message.to_lowercase().contains("updated remotely"),
            "toast must mention remote update, got {:?}",
            toast.message
        );
        assert!(
            effects.iter().any(|e| matches!(
                e,
                Effect::Refetch {
                    kind: crate::data::cache::WriteKind::PostPrComment { id: i }
                } if i == &id
            )),
            "expected Refetch on stale write, got {effects:?}"
        );
    }

    #[test]
    fn write_completed_with_fresh_version_does_not_warn() {
        let id = PrId {
            repo: repo_acme(),
            number: 10,
        };
        let mut s = state_for(View::PrDetail {
            id: id.clone(),
            tab: crate::app::state::DetailTab::Conversation,
        });
        s.pending_write = Some(crate::app::state::PendingWrite {
            kind: crate::data::cache::WriteKind::PostPrComment { id: id.clone() },
            request: crate::app::effect::WriteRequest::PostPrComment { body: "hi".into() },
            version_at_submit: 3,
            status: crate::app::state::PendingStatus::Submitting,
        });
        s.pr_details
            .insert(id.clone(), pr_detail_with_version(&id, 3));
        let _ = update(
            &mut s,
            &AppEvent::Data(DataEvent::WriteCompleted {
                kind: crate::data::cache::WriteKind::PostPrComment { id: id.clone() },
                result: Ok(()),
            }),
        );
        assert!(
            s.toast.is_none() || s.toast.as_ref().unwrap().kind != ToastKind::Warning,
            "no stale-write warning when version unchanged, got {:?}",
            s.toast
        );
    }

    #[test]
    fn absorb_pr_detail_increments_version_on_overwrite() {
        let id = PrId {
            repo: repo_acme(),
            number: 11,
        };
        let mut s = AppState::default();
        s.pr_details
            .insert(id.clone(), pr_detail_with_version(&id, 4));
        let fresh = pr_detail_with_version(&id, 4);
        let _ = update(
            &mut s,
            &AppEvent::Data(DataEvent::PrDetailLoaded {
                id: id.clone(),
                result: Box::new(Ok(fresh)),
            }),
        );
        let stored = s.pr_details.get(&id).expect("detail must persist");
        assert_eq!(
            stored.version, 5,
            "absorbing a refreshed PrDetail must increment version"
        );
    }

    #[test]
    fn force_push_detected_when_head_sha_changes_between_fetches() {
        let id = PrId {
            repo: repo_acme(),
            number: 12,
        };
        let mut s = AppState::default();
        let mut prev = sample_pr_detail(&id);
        prev.head_sha = "oldsha".into();
        s.pr_details.insert(id.clone(), prev);
        let mut fresh = sample_pr_detail(&id);
        fresh.head_sha = "newsha".into();
        let _ = update(
            &mut s,
            &AppEvent::Data(DataEvent::PrDetailLoaded {
                id: id.clone(),
                result: Box::new(Ok(fresh)),
            }),
        );
        let alert = s
            .force_push_alert
            .as_ref()
            .expect("force-push alert expected");
        assert_eq!(alert.pr_id, id);
        assert_eq!(alert.before, "oldsha");
        assert_eq!(alert.after, "newsha");
    }

    #[test]
    fn force_push_drops_stale_files_diffs_and_refetches() {
        let id = PrId {
            repo: repo_acme(),
            number: 13,
        };
        let mut s = AppState::default();
        let mut prev = sample_pr_detail(&id);
        prev.head_sha = "oldsha".into();
        s.pr_details.insert(id.clone(), prev);
        s.pr_files.insert(id.clone(), sample_pr_files(&id));
        s.pr_diffs.insert(id.clone(), "stale-diff".into());
        s.pr_file_diff_inflight
            .insert((id.clone(), "src/main.rs".into()));
        let mut fresh = sample_pr_detail(&id);
        fresh.head_sha = "newsha".into();
        let effects = update(
            &mut s,
            &AppEvent::Data(DataEvent::PrDetailLoaded {
                id: id.clone(),
                result: Box::new(Ok(fresh)),
            }),
        );
        assert!(
            !s.pr_files.contains_key(&id),
            "pr_files for old head_sha must be dropped"
        );
        assert!(
            !s.pr_diffs.contains_key(&id),
            "pr_diffs for old head_sha must be dropped"
        );
        assert!(
            !s.pr_file_diff_inflight.iter().any(|(i, _)| i == &id),
            "in-flight pr_file_diff for old head_sha must be cleared"
        );
        assert!(
            effects.iter().any(|e| matches!(
                e,
                Effect::FetchPrFiles { id: i, head_sha } if i == &id && head_sha == "newsha"
            )),
            "expected FetchPrFiles with new head_sha, got {effects:?}"
        );
    }

    #[test]
    fn force_push_during_open_composer_sets_head_moved_flag() {
        let id = PrId {
            repo: repo_acme(),
            number: 14,
        };
        let mut s = AppState::default();
        let mut prev = sample_pr_detail(&id);
        prev.head_sha = "oldsha".into();
        s.pr_details.insert(id.clone(), prev);
        s.composer = Some(ComposerState {
            body: "drafting...".into(),
            context: ComposerContext::PrComment { pr_id: id.clone() },
            head_moved: false,
            preview_visible: true,
        });
        let mut fresh = sample_pr_detail(&id);
        fresh.head_sha = "newsha".into();
        let _ = update(
            &mut s,
            &AppEvent::Data(DataEvent::PrDetailLoaded {
                id: id.clone(),
                result: Box::new(Ok(fresh)),
            }),
        );
        let composer = s.composer.as_ref().expect("composer must remain open");
        assert!(
            composer.head_moved,
            "head_moved must be set when force-push lands while composing for the same PR"
        );
        assert_eq!(
            composer.body, "drafting...",
            "composer body must NOT be cleared"
        );
    }

    #[test]
    fn force_push_does_not_flag_composer_for_different_pr() {
        let id_a = PrId {
            repo: repo_acme(),
            number: 15,
        };
        let id_b = PrId {
            repo: repo_acme(),
            number: 16,
        };
        let mut s = AppState::default();
        let mut prev = sample_pr_detail(&id_a);
        prev.head_sha = "oldsha".into();
        s.pr_details.insert(id_a.clone(), prev);
        s.composer = Some(ComposerState {
            body: String::new(),
            context: ComposerContext::PrComment {
                pr_id: id_b.clone(),
            },
            head_moved: false,
            preview_visible: true,
        });
        let mut fresh = sample_pr_detail(&id_a);
        fresh.head_sha = "newsha".into();
        let _ = update(
            &mut s,
            &AppEvent::Data(DataEvent::PrDetailLoaded {
                id: id_a.clone(),
                result: Box::new(Ok(fresh)),
            }),
        );
        let composer = s.composer.as_ref().expect("composer remains open");
        assert!(
            !composer.head_moved,
            "head_moved must NOT be set for an unrelated PR's force-push"
        );
    }

    #[test]
    fn resize_event_updates_layout_mode_and_does_not_panic_on_tiny_dimensions() {
        use crate::ui::layout::LayoutMode;

        let mut s = AppState::default();
        assert_eq!(s.layout_mode, LayoutMode::Full);

        let effects = update(
            &mut s,
            &AppEvent::Resize {
                width: 50,
                height: 16,
            },
        );
        assert!(effects.is_empty(), "Resize must not emit effects");
        assert_eq!(s.layout_mode, LayoutMode::FullScreenModal);

        let _ = update(
            &mut s,
            &AppEvent::Resize {
                width: 70,
                height: 22,
            },
        );
        assert_eq!(s.layout_mode, LayoutMode::Compact);

        let _ = update(
            &mut s,
            &AppEvent::Resize {
                width: 100,
                height: 30,
            },
        );
        assert_eq!(s.layout_mode, LayoutMode::Full);

        let _ = update(
            &mut s,
            &AppEvent::Resize {
                width: 0,
                height: 0,
            },
        );
        assert_eq!(s.layout_mode, LayoutMode::FullScreenModal);
        let _ = update(
            &mut s,
            &AppEvent::Resize {
                width: 1,
                height: 1,
            },
        );
        assert_eq!(s.layout_mode, LayoutMode::FullScreenModal);
    }

    #[test]
    fn help_overlay_toggle_open_and_dismiss() {
        let mut s = AppState::default();
        assert!(!s.help_overlay, "default: help closed");

        let question = KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE);
        let effects = update(&mut s, &AppEvent::Key(question));
        assert!(effects.is_empty(), "EnterHelp must not emit effects");
        assert!(s.help_overlay, "? opens help overlay");
        assert!(!s.should_quit, "? must not quit");

        let _ = update(&mut s, &AppEvent::Key(question));
        assert!(!s.help_overlay, "? again closes help overlay");

        let _ = update(&mut s, &AppEvent::Key(question));
        assert!(s.help_overlay);
        let q = KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE);
        let effects = update(&mut s, &AppEvent::Key(q));
        assert!(effects.is_empty(), "q must not emit effects");
        // q (Quit) now always sets should_quit=true and clears help_overlay.
        assert!(!s.help_overlay, "q clears help overlay");
        assert!(s.should_quit, "q always quits");
    }

    #[test]
    fn colon_key_opens_command_palette() {
        let mut s = AppState::default();
        let effects = update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Char(':'), KeyModifiers::NONE)),
        );
        assert!(effects.is_empty(), ": must not emit effects");
        assert!(s.palette.is_some(), ": must open palette");
    }

    #[test]
    fn esc_with_palette_open_closes_palette() {
        let s = AppState {
            palette: Some(crate::ui::palette::PaletteState::default()),
            ..AppState::default()
        };
        let mut s = s;
        let _ = update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        );
        assert!(s.palette.is_none(), "Esc must close palette overlay");
    }

    #[test]
    fn esc_with_log_view_open_closes_log_view() {
        let s = AppState {
            log_view: Some(crate::ui::log_view::LogViewState::default()),
            ..AppState::default()
        };
        let mut s = s;
        let _ = update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        );
        assert!(s.log_view.is_none(), "Esc must close log_view overlay");
    }

    #[test]
    fn palette_printable_chars_extend_input() {
        let s = AppState {
            palette: Some(crate::ui::palette::PaletteState::default()),
            ..AppState::default()
        };
        let mut s = s;
        let _ = update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE)),
        );
        let _ = update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE)),
        );
        assert_eq!(s.palette.as_ref().unwrap().input.text, "re");
    }

    #[test]
    fn palette_enter_pr_n_navigates_to_pr_detail() {
        use crate::data::models::Repo;
        let repo = Repo {
            owner: "o".into(),
            name: "n".into(),
        };
        let mut s = state_for(View::PrList { repo: repo.clone() });
        s.palette = Some(crate::ui::palette::PaletteState {
            input: crate::ui::components::input::InputBuffer {
                text: "pr 42".into(),
                cursor: 5,
            },
            selected: 0,
        });
        let _ = update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        );
        assert!(s.palette.is_none(), "Enter must close palette");
        match s.current_view() {
            View::PrDetail { id, .. } => {
                assert_eq!(id.number, 42);
                assert_eq!(id.repo, repo);
            }
            other => panic!("expected PrDetail, got {other:?}"),
        }
    }

    #[test]
    fn palette_enter_quit_sets_should_quit() {
        let s = AppState {
            palette: Some(crate::ui::palette::PaletteState {
                input: crate::ui::components::input::InputBuffer {
                    text: "quit".into(),
                    cursor: 4,
                },
                selected: 0,
            }),
            ..AppState::default()
        };
        let mut s = s;
        let _ = update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        );
        assert!(s.should_quit, "quit command must set should_quit");
        assert!(s.palette.is_none());
    }

    #[test]
    fn palette_enter_dashboard_switches_view() {
        use crate::data::models::Repo;
        let mut s = state_for(View::PrList {
            repo: Repo {
                owner: "o".into(),
                name: "n".into(),
            },
        });
        s.palette = Some(crate::ui::palette::PaletteState {
            input: crate::ui::components::input::InputBuffer {
                text: "dashboard".into(),
                cursor: 9,
            },
            selected: 0,
        });
        let _ = update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        );
        assert!(matches!(s.current_view(), View::Dashboard));
        assert!(s.palette.is_none());
    }

    #[test]
    fn palette_enter_log_opens_log_view() {
        let s = AppState {
            palette: Some(crate::ui::palette::PaletteState {
                input: crate::ui::components::input::InputBuffer {
                    text: "log".into(),
                    cursor: 3,
                },
                selected: 0,
            }),
            ..AppState::default()
        };
        let mut s = s;
        let _ = update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        );
        assert!(s.palette.is_none());
        assert!(s.log_view.is_some(), "log command must open log_view");
    }

    #[test]
    fn log_view_j_advances_selection() {
        let ring = crate::logging::global_ring();
        for i in 0..5 {
            ring.push(format!("t5-j-{i}"));
        }
        let s = AppState {
            log_view: Some(crate::ui::log_view::LogViewState::default()),
            ..AppState::default()
        };
        let mut s = s;
        let _ = update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE)),
        );
        assert_eq!(s.log_view.as_ref().unwrap().selected, 1);
    }

    #[test]
    fn log_view_k_decreases_selection() {
        let ring = crate::logging::global_ring();
        for i in 0..5 {
            ring.push(format!("t5-k-{i}"));
        }
        let s = AppState {
            log_view: Some(crate::ui::log_view::LogViewState {
                selected: 3,
                scroll: 0,
            }),
            ..AppState::default()
        };
        let mut s = s;
        let _ = update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE)),
        );
        assert_eq!(s.log_view.as_ref().unwrap().selected, 2);
    }

    #[test]
    fn log_view_shift_g_jumps_to_end() {
        let ring = crate::logging::global_ring();
        for i in 0..7 {
            ring.push(format!("t5-G-{i}"));
        }
        let s = AppState {
            log_view: Some(crate::ui::log_view::LogViewState::default()),
            ..AppState::default()
        };
        let mut s = s;
        let _ = update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Char('G'), KeyModifiers::SHIFT)),
        );
        let sel = s.log_view.as_ref().unwrap().selected;
        let len = crate::logging::global_ring()
            .tail(crate::ui::log_view::LOG_VIEW_TAIL)
            .len();
        assert_eq!(sel, len.saturating_sub(1));
    }

    #[test]
    fn log_view_g_jumps_to_start() {
        let s = AppState {
            log_view: Some(crate::ui::log_view::LogViewState {
                selected: 5,
                scroll: 0,
            }),
            ..AppState::default()
        };
        let mut s = s;
        let _ = update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE)),
        );
        assert_eq!(s.log_view.as_ref().unwrap().selected, 0);
    }

    fn dashboard_state_with(rows_per_bucket: &[(DashboardBucket, usize)]) -> AppState {
        let dashboard = rows_per_bucket
            .iter()
            .map(|(b, n)| (*b, make_summaries(*n)))
            .collect();
        AppState {
            dashboard: Some(dashboard),
            ..AppState::default()
        }
    }

    #[test]
    fn dashboard_down_advances_row_within_bucket() {
        let mut s = dashboard_state_with(&[(DashboardBucket::ReviewRequested, 5)]);
        let _ = apply_intent(&mut s, &UserIntent::Down);
        match s.selection().clone() {
            Selection::Dashboard { focused, rows } => {
                assert_eq!(focused, 0);
                assert_eq!(rows[0], 1);
            }
            other => panic!("expected Dashboard selection, got {other:?}"),
        }
    }

    #[test]
    fn dashboard_down_at_last_row_clamps() {
        let mut s = dashboard_state_with(&[(DashboardBucket::ReviewRequested, 3)]);
        *s.selection_mut() = Selection::Dashboard {
            focused: 0,
            rows: [2, 0, 0],
        };
        let _ = apply_intent(&mut s, &UserIntent::Down);
        match s.selection().clone() {
            Selection::Dashboard { rows, .. } => assert_eq!(rows[0], 2),
            other => panic!("expected Dashboard selection, got {other:?}"),
        }
    }

    #[test]
    fn dashboard_up_at_first_row_clamps_to_zero() {
        let mut s = dashboard_state_with(&[(DashboardBucket::ReviewRequested, 3)]);
        let _ = apply_intent(&mut s, &UserIntent::Up);
        match s.selection().clone() {
            Selection::Dashboard { rows, .. } => assert_eq!(rows[0], 0),
            other => panic!("expected Dashboard selection, got {other:?}"),
        }
    }

    #[test]
    fn dashboard_top_and_bottom_jump_to_edges() {
        let mut s = dashboard_state_with(&[(DashboardBucket::ReviewRequested, 7)]);
        *s.selection_mut() = Selection::Dashboard {
            focused: 0,
            rows: [3, 0, 0],
        };
        let _ = apply_intent(&mut s, &UserIntent::Bottom);
        match s.selection().clone() {
            Selection::Dashboard { rows, .. } => assert_eq!(rows[0], 6),
            other => panic!("expected Dashboard selection, got {other:?}"),
        }
        let _ = apply_intent(&mut s, &UserIntent::Top);
        match s.selection().clone() {
            Selection::Dashboard { rows, .. } => assert_eq!(rows[0], 0),
            other => panic!("expected Dashboard selection, got {other:?}"),
        }
    }

    #[test]
    fn dashboard_open_selected_transitions_to_pr_detail() {
        let mut s = dashboard_state_with(&[(DashboardBucket::ReviewRequested, 3)]);
        *s.selection_mut() = Selection::Dashboard {
            focused: 0,
            rows: [1, 0, 0],
        };
        let _ = apply_intent(&mut s, &UserIntent::OpenSelected);
        match s.current_view() {
            View::PrDetail { id, tab } => {
                assert_eq!(id.number, 1);
                assert_eq!(tab, DetailTab::Conversation);
            }
            other => panic!("expected PrDetail view, got {other:?}"),
        }
    }

    #[test]
    fn dashboard_refresh_emits_fetch_dashboard_with_no_bucket() {
        let mut s = dashboard_state_with(&[(DashboardBucket::ReviewRequested, 3)]);
        let effects = apply_intent(&mut s, &UserIntent::Refresh);
        assert_eq!(effects.len(), 1, "got {effects:?}");
        match &effects[0] {
            Effect::FetchDashboard { bucket, cursor } => {
                assert!(bucket.is_none());
                assert!(cursor.is_none());
            }
            other => panic!("expected FetchDashboard, got {other:?}"),
        }
    }

    #[test]
    fn dashboard_nav_all_empty_is_noop() {
        let mut s = dashboard_state_with(&[
            (DashboardBucket::ReviewRequested, 0),
            (DashboardBucket::Authored, 0),
            (DashboardBucket::Assigned, 0),
        ]);
        *s.selection_mut() = Selection::Dashboard {
            focused: 0,
            rows: [0; 3],
        };
        for intent in [
            UserIntent::Down,
            UserIntent::Up,
            UserIntent::Top,
            UserIntent::Bottom,
        ] {
            let _ = apply_intent(&mut s, &intent);
            assert!(
                matches!(s.selection(), Selection::Dashboard { focused: 0, rows } if *rows == [0; 3]),
                "all-empty dashboard nav must be a no-op, got {:?}",
                s.selection()
            );
        }
    }

    #[test]
    fn dashboard_down_stays_within_focused_panel() {
        let mut s = dashboard_state_with(&[
            (DashboardBucket::ReviewRequested, 3),
            (DashboardBucket::Authored, 2),
            (DashboardBucket::Assigned, 0),
        ]);
        for _ in 0..5 {
            let _ = apply_intent(&mut s, &UserIntent::Down);
        }
        match s.selection().clone() {
            Selection::Dashboard { focused, rows } => {
                assert_eq!(focused, 0);
                assert_eq!(rows[0], 2, "clamped to last row of focused panel 0");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn focus_right_moves_to_next_panel_and_remembers_rows() {
        let mut s = dashboard_state_with(&[
            (DashboardBucket::ReviewRequested, 3),
            (DashboardBucket::Authored, 2),
            (DashboardBucket::Assigned, 1),
        ]);
        let _ = apply_intent(&mut s, &UserIntent::Down); // panel 0 row -> 1
        let _ = apply_intent(&mut s, &UserIntent::FocusRight); // focus -> 1
        match s.selection().clone() {
            Selection::Dashboard { focused, rows } => {
                assert_eq!(focused, 1);
                assert_eq!(rows[0], 1, "panel 0 row remembered");
                assert_eq!(rows[1], 0, "panel 1 starts at 0");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn focus_left_saturates_at_first_panel() {
        let mut s = dashboard_state_with(&[(DashboardBucket::ReviewRequested, 3)]);
        let _ = apply_intent(&mut s, &UserIntent::FocusLeft);
        match s.selection().clone() {
            Selection::Dashboard { focused, .. } => assert_eq!(focused, 0),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn focus_right_saturates_at_last_panel() {
        let mut s = dashboard_state_with(&[
            (DashboardBucket::ReviewRequested, 1),
            (DashboardBucket::Authored, 1),
            (DashboardBucket::Assigned, 1),
        ]);
        *s.selection_mut() = Selection::Dashboard {
            focused: 2,
            rows: [0; 3],
        };
        let _ = apply_intent(&mut s, &UserIntent::FocusRight);
        match s.selection().clone() {
            Selection::Dashboard { focused, .. } => assert_eq!(focused, 2),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn focus_can_land_on_empty_panel_and_nav_is_noop() {
        let mut s = dashboard_state_with(&[
            (DashboardBucket::ReviewRequested, 1),
            (DashboardBucket::Authored, 0),
            (DashboardBucket::Assigned, 1),
        ]);
        let _ = apply_intent(&mut s, &UserIntent::FocusRight); // focus empty panel 1
        let _ = apply_intent(&mut s, &UserIntent::Down); // no-op (empty)
        match s.selection().clone() {
            Selection::Dashboard { focused, rows } => {
                assert_eq!(focused, 1);
                assert_eq!(rows[1], 0);
            }
            other => panic!("{other:?}"),
        }
    }

    fn pr_list_state_with(repo: Repo, n: usize) -> AppState {
        let mut lists = std::collections::HashMap::new();
        lists.insert(repo.clone(), make_summaries(n));
        let mut s = state_for(View::PrList { repo });
        s.pr_lists = lists;
        s
    }

    #[test]
    fn pr_list_down_and_up_move_selection() {
        let repo = Repo {
            owner: "acme".into(),
            name: "widgets".into(),
        };
        let mut s = pr_list_state_with(repo, 4);
        let _ = apply_intent(&mut s, &UserIntent::Down);
        let _ = apply_intent(&mut s, &UserIntent::Down);
        match s.selection().clone() {
            Selection::PrListRow(r) => assert_eq!(r, 2),
            other => panic!("expected PrListRow, got {other:?}"),
        }
        let _ = apply_intent(&mut s, &UserIntent::Up);
        match s.selection().clone() {
            Selection::PrListRow(r) => assert_eq!(r, 1),
            other => panic!("expected PrListRow, got {other:?}"),
        }
    }

    #[test]
    fn pr_list_down_clamps_to_last_row() {
        let repo = Repo {
            owner: "acme".into(),
            name: "widgets".into(),
        };
        let mut s = pr_list_state_with(repo, 3);
        *s.selection_mut() = Selection::PrListRow(2);
        let _ = apply_intent(&mut s, &UserIntent::Down);
        match s.selection().clone() {
            Selection::PrListRow(r) => assert_eq!(r, 2),
            other => panic!("expected PrListRow, got {other:?}"),
        }
    }

    #[test]
    fn pr_list_open_selected_transitions_to_pr_detail() {
        let repo = Repo {
            owner: "acme".into(),
            name: "widgets".into(),
        };
        let mut s = pr_list_state_with(repo, 5);
        *s.selection_mut() = Selection::PrListRow(2);
        let _ = apply_intent(&mut s, &UserIntent::OpenSelected);
        match s.current_view() {
            View::PrDetail { id, tab } => {
                assert_eq!(id.number, 2);
                assert_eq!(tab, DetailTab::Conversation);
            }
            other => panic!("expected PrDetail view, got {other:?}"),
        }
    }

    #[test]
    fn pr_list_refresh_emits_fetch_pr_list() {
        let repo = Repo {
            owner: "acme".into(),
            name: "widgets".into(),
        };
        let mut s = pr_list_state_with(repo.clone(), 3);
        let effects = apply_intent(&mut s, &UserIntent::Refresh);
        assert_eq!(effects.len(), 1, "got {effects:?}");
        match &effects[0] {
            Effect::FetchPrList {
                repo: r, cursor, ..
            } => {
                assert_eq!(*r, repo);
                assert!(cursor.is_none());
            }
            other => panic!("expected FetchPrList, got {other:?}"),
        }
    }

    #[test]
    fn palette_dashboard_resets_selection_when_coming_from_pr_list() {
        use crate::data::models::Repo;
        let mut s = state_for(View::PrList {
            repo: Repo {
                owner: "o".into(),
                name: "n".into(),
            },
        });
        *s.selection_mut() = Selection::PrListRow(7);
        s.palette = Some(crate::ui::palette::PaletteState {
            input: crate::ui::components::input::InputBuffer {
                text: "dashboard".into(),
                cursor: 9,
            },
            selected: 0,
        });
        let _ = update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        );
        assert!(matches!(s.current_view(), View::Dashboard));
    }

    #[test]
    fn palette_dashboard_emits_fetch_dashboard_when_data_empty() {
        use crate::data::models::Repo;
        let mut s = state_for(View::PrList {
            repo: Repo {
                owner: "o".into(),
                name: "n".into(),
            },
        });
        s.palette = Some(crate::ui::palette::PaletteState {
            input: crate::ui::components::input::InputBuffer {
                text: "dashboard".into(),
                cursor: 9,
            },
            selected: 0,
        });
        let effects = update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        );
        assert!(
            effects
                .iter()
                .any(|e| matches!(e, Effect::FetchDashboard { .. })),
            "expected FetchDashboard among {effects:?}"
        );
    }

    fn repo_on() -> crate::data::models::Repo {
        crate::data::models::Repo {
            owner: "o".into(),
            name: "n".into(),
        }
    }

    #[test]
    fn ctrl_c_quits_from_any_view() {
        let id = PrId {
            repo: repo_on(),
            number: 1,
        };
        let mut s = state_for(View::PrDetail {
            id,
            tab: DetailTab::Conversation,
        });
        let _ = update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
        );
        assert!(s.should_quit, "Ctrl-C is a hard quit from any view");
    }

    #[test]
    fn q_quits_immediately_from_any_view() {
        let id = PrId {
            repo: repo_on(),
            number: 1,
        };
        let mut s = AppState::default();
        crate::app::seed_workspace_from_view(&mut s, View::Diff { id, file_index: 0 });
        let q = AppEvent::Key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE));
        let _ = update(&mut s, &q);
        assert!(s.should_quit, "q quits immediately from any view");
    }

    #[test]
    fn esc_steps_back_from_diff_to_files_list() {
        let id = PrId {
            repo: repo_on(),
            number: 1,
        };
        let mut s = AppState::default();
        crate::app::seed_workspace_from_view(&mut s, View::Diff { id, file_index: 0 });
        let esc = AppEvent::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        let _ = update(&mut s, &esc);
        assert!(
            matches!(s.current_view(), View::PrDetail { .. }),
            "Esc from Diff clears diff_file, showing PrDetail"
        );
        assert!(!s.should_quit, "Esc never quits");
    }

    #[test]
    fn refetch_effects_for_comment_is_pr_detail_only() {
        let id = PrId {
            repo: repo_on(),
            number: 5,
        };
        let s = AppState::default();
        let eff = refetch_effects(
            &s,
            &crate::data::cache::WriteKind::PostPrComment { id: id.clone() },
        );
        assert!(
            matches!(eff.as_slice(), [Effect::FetchPrDetail { id: i }] if *i == id),
            "comment refetch is PR detail only, got {eff:?}"
        );
    }

    #[test]
    fn refetch_effects_for_merge_includes_checks_files_and_lists() {
        let id = PrId {
            repo: repo_on(),
            number: 5,
        };
        let mut s = AppState {
            dashboard: Some(Vec::new()),
            ..AppState::default()
        };
        s.pr_details.insert(id.clone(), sample_pr_detail(&id));
        s.pr_lists.insert(id.repo.clone(), Vec::new());
        let eff = refetch_effects(&s, &crate::data::cache::WriteKind::Merge { id: id.clone() });
        assert!(
            eff.iter()
                .any(|e| matches!(e, Effect::FetchPrDetail { .. }))
        );
        assert!(
            eff.iter()
                .any(|e| matches!(e, Effect::FetchPrChecks { .. }))
        );
        assert!(eff.iter().any(|e| matches!(e, Effect::FetchPrFiles { .. })));
        assert!(
            eff.iter()
                .any(|e| matches!(e, Effect::FetchDashboard { .. }))
        );
        assert!(eff.iter().any(|e| matches!(e, Effect::FetchPrList { .. })));
    }

    #[test]
    fn composer_enter_inserts_newline() {
        let id = PrId {
            repo: repo_on(),
            number: 1,
        };
        let mut s = AppState {
            composer: Some(ComposerState {
                body: "ab".into(),
                context: ComposerContext::PrComment { pr_id: id },
                head_moved: false,
                preview_visible: true,
            }),
            ..AppState::default()
        };
        let _ = update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        );
        assert_eq!(s.composer.as_ref().unwrap().body, "ab\n");
    }

    #[test]
    fn line_comment_submit_blocked_when_head_moved() {
        let id = PrId {
            repo: repo_on(),
            number: 1,
        };
        let comment = crate::data::github::NewLineComment {
            path: "a.rs".into(),
            side: crate::data::models::DiffSide::Right,
            start_line: None,
            start_side: None,
            line: 3,
            body: String::new(),
        };
        let mut s = AppState {
            composer: Some(ComposerState {
                body: "nit".into(),
                context: ComposerContext::LineComment {
                    pr_id: id,
                    head_sha: "old".into(),
                    comment,
                },
                head_moved: true,
                preview_visible: true,
            }),
            ..AppState::default()
        };
        let effects = update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL)),
        );
        assert!(effects.is_empty(), "blocked submit emits no write");
        assert!(s.composer.is_some(), "composer stays open for re-compose");
        assert!(s.toast.is_some(), "a toast explains the block");
    }

    #[test]
    fn pr_palette_command_from_dashboard_warns_without_repo() {
        let mut s = AppState {
            palette: Some(crate::ui::palette::PaletteState {
                input: crate::ui::components::input::InputBuffer {
                    text: "pr 7".into(),
                    cursor: 4,
                },
                selected: 0,
            }),
            ..AppState::default()
        };
        let _ = update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        );
        assert!(s.palette.is_none());
        assert!(
            matches!(s.current_view(), View::Dashboard),
            "view unchanged"
        );
        assert!(s.toast.is_some(), "warns the user to open a repo first");
    }

    #[test]
    fn o_emits_open_in_browser_with_pr_url() {
        let id = PrId {
            repo: crate::data::models::Repo {
                owner: "acme".into(),
                name: "widgets".into(),
            },
            number: 99,
        };
        let mut s = state_for(View::PrDetail {
            id,
            tab: DetailTab::Conversation,
        });
        let effects = update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE)),
        );
        assert!(
            matches!(
                effects.as_slice(),
                [Effect::OpenInBrowser { url }]
                    if url == "https://github.com/acme/widgets/pull/99"
            ),
            "got {effects:?}"
        );
    }

    #[test]
    fn page_down_advances_selection_by_half_page() {
        let repo = crate::data::models::Repo {
            owner: "acme".into(),
            name: "widgets".into(),
        };
        let mut pr_lists = std::collections::HashMap::new();
        pr_lists.insert(repo.clone(), make_summaries(30));
        let mut s = AppState {
            pr_lists,
            ..AppState::default()
        };
        crate::app::seed_workspace_from_view(&mut s, View::PrList { repo: repo.clone() });
        *s.selection_mut() = Selection::PrListRow(0);
        let _ = update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL)),
        );
        assert!(
            matches!(s.selection(), Selection::PrListRow(r) if *r == HALF_PAGE_ROWS),
            "Ctrl-d advances by a half page, got {:?}",
            s.selection()
        );
    }

    #[test]
    fn close_tab_from_pr_opened_on_dashboard_preserves_dashboard_selection() {
        let mut s = AppState::default(); // workspace = [Dashboard]
        // Place Dashboard cursor at a non-default position.
        *s.selection_mut() = Selection::Dashboard {
            focused: 1,
            rows: [0, 2, 0],
        };
        // Open a PR tab directly (no Repo tab) -> workspace = [Dashboard, Pr].
        s.workspace
            .open(crate::app::workspace::TabKind::Pr(sample_id()));
        // Sanity: active view is PrDetail.
        assert!(
            matches!(s.current_view(), View::PrDetail { .. }),
            "expected PrDetail, got {:?}",
            s.current_view()
        );
        // Close the PR tab via CloseTab (w) -> should land on Dashboard with selection preserved.
        let _ = update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE)),
        );
        assert_eq!(s.workspace.active, 0, "should land on tab 0 (Dashboard)");
        assert!(
            matches!(s.current_view(), View::Dashboard),
            "expected Dashboard after CloseTab, got {:?}",
            s.current_view()
        );
        assert!(
            matches!(s.selection(), Selection::Dashboard { focused: 1, rows } if *rows == [0, 2, 0]),
            "Dashboard selection should be preserved, got {:?}",
            s.selection()
        );
    }

    // ── viewport_width / narrow-terminal tests ───────────────────────────────

    #[test]
    fn narrow_enter_opens_full_screen_diff_with_selected_index() {
        // Width 80 < FILES_SPLIT_MIN_WIDTH (90) → narrow path.
        // Must use the same PrId that pr_detail_state creates (number: 1).
        let id = PrId {
            repo: Repo {
                owner: "acme".into(),
                name: "widgets".into(),
            },
            number: 1,
        };
        let mut s = pr_detail_state(DetailTab::Files);
        s.viewport_width = 80;
        s.pr_files.insert(
            id.clone(),
            files_for(&id, vec![&diff_with_two_hunks(), &diff_with_two_hunks()]),
        );
        // Select second file (index 1).
        *s.selection_mut() = Selection::DetailFile(1);

        update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        );

        // diff_file must be Some(1) — NOT Some(0).
        assert_eq!(
            s.active_tab().state.diff_file,
            Some(1),
            "narrow Enter must open the SELECTED file, not always index 0"
        );
        assert!(
            matches!(s.current_view(), View::Diff { file_index: 1, .. }),
            "expected View::Diff{{file_index:1}}, got {:?}",
            s.current_view()
        );
    }

    #[test]
    fn narrow_focus_right_is_noop() {
        use crate::app::state::FilesPane;
        // On a narrow terminal FocusRight (l) must NOT change files_focus.
        let id = PrId {
            repo: Repo {
                owner: "acme".into(),
                name: "widgets".into(),
            },
            number: 1,
        };
        let mut s = pr_detail_state(DetailTab::Files);
        s.viewport_width = 80;
        s.pr_files
            .insert(id.clone(), files_for(&id, vec![&diff_with_two_hunks()]));
        assert_eq!(s.active_tab().state.files_focus, FilesPane::Tree);

        update(&mut s, &key('l'));

        assert_eq!(
            s.active_tab().state.files_focus,
            FilesPane::Tree,
            "FocusRight must be a no-op on narrow terminals"
        );
    }

    #[test]
    fn wide_enter_focuses_diff_pane_not_full_screen() {
        use crate::app::state::FilesPane;
        // Width 120 >= FILES_SPLIT_MIN_WIDTH → wide path.
        let id = PrId {
            repo: Repo {
                owner: "acme".into(),
                name: "widgets".into(),
            },
            number: 1,
        };
        let mut s = pr_detail_state(DetailTab::Files);
        s.viewport_width = 120;
        s.pr_files
            .insert(id.clone(), files_for(&id, vec![&diff_with_two_hunks()]));

        update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        );

        assert_eq!(
            s.active_tab().state.files_focus,
            FilesPane::Diff,
            "wide Enter must focus the Diff pane"
        );
        assert_eq!(
            s.active_tab().state.diff_file,
            None,
            "diff_file must stay None in wide split mode"
        );
    }

    #[test]
    fn default_zero_width_treated_as_wide() {
        use crate::app::state::FilesPane;
        // viewport_width == 0 (default) must behave identically to wide.
        let id = PrId {
            repo: Repo {
                owner: "acme".into(),
                name: "widgets".into(),
            },
            number: 1,
        };
        let mut s = pr_detail_state(DetailTab::Files);
        assert_eq!(s.viewport_width, 0);
        s.pr_files
            .insert(id.clone(), files_for(&id, vec![&diff_with_two_hunks()]));

        update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        );

        assert_eq!(
            s.active_tab().state.files_focus,
            FilesPane::Diff,
            "default (0) viewport_width must use the wide path"
        );
        assert_eq!(s.active_tab().state.diff_file, None);
    }

    #[test]
    fn focus_pane_is_noop_on_non_files_tab() {
        let mut s = AppState::default(); // Dashboard tab
        let _ = apply_intent(&mut s, &UserIntent::FocusRight);
        // files_focus on Dashboard tab still Tree (unchanged)
        assert_eq!(
            s.active_tab().state.files_focus,
            crate::app::state::FilesPane::Tree
        );
    }

    #[test]
    fn focus_pane_sets_focus_on_files_subtab() {
        let mut s = AppState::default();
        s.workspace
            .open(crate::app::workspace::TabKind::Pr(sample_id()));
        // Default detail_tab is Conversation; FocusRight should be no-op
        let _ = apply_intent(&mut s, &UserIntent::FocusRight);
        assert_eq!(
            s.active_tab().state.files_focus,
            crate::app::state::FilesPane::Tree,
            "FocusRight on Conversation sub-tab should be no-op"
        );
        // Switch to Files sub-tab
        s.active_tab_mut().state.detail_tab = crate::app::state::DetailTab::Files;
        let _ = apply_intent(&mut s, &UserIntent::FocusRight);
        assert_eq!(
            s.active_tab().state.files_focus,
            crate::app::state::FilesPane::Diff,
            "FocusRight on Files sub-tab should focus Diff"
        );
        let _ = apply_intent(&mut s, &UserIntent::FocusLeft);
        assert_eq!(
            s.active_tab().state.files_focus,
            crate::app::state::FilesPane::Tree,
            "FocusLeft should return to Tree"
        );
    }

    #[test]
    fn conversation_down_scrolls_cursor() {
        let mut s = pr_detail_state(DetailTab::Conversation);
        let _ = apply_intent(&mut s, &UserIntent::Down);
        assert_eq!(s.active_tab().state.conversation_cursor, 1);
        assert!(!s.active_tab().state.conversation_scroll_to_focused);
    }

    #[test]
    fn conversation_up_clamps_at_zero() {
        let mut s = pr_detail_state(DetailTab::Conversation);
        let _ = apply_intent(&mut s, &UserIntent::Up);
        assert_eq!(s.active_tab().state.conversation_cursor, 0);
        assert!(!s.active_tab().state.conversation_scroll_to_focused);
    }

    #[test]
    fn conversation_top_resets_cursor() {
        let mut s = pr_detail_state(DetailTab::Conversation);
        s.active_tab_mut().state.conversation_cursor = 5;
        let _ = apply_intent(&mut s, &UserIntent::Top);
        assert_eq!(s.active_tab().state.conversation_cursor, 0);
        assert!(!s.active_tab().state.conversation_scroll_to_focused);
    }

    #[test]
    fn conversation_bottom_sets_sentinel() {
        let mut s = pr_detail_state(DetailTab::Conversation);
        let _ = apply_intent(&mut s, &UserIntent::Bottom);
        assert_eq!(s.active_tab().state.conversation_cursor, usize::MAX);
        assert!(!s.active_tab().state.conversation_scroll_to_focused);
    }

    #[test]
    fn composer_ctrl_p_toggles_preview() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut s = AppState {
            composer: Some(ComposerState {
                body: "x".into(),
                context: ComposerContext::PrComment { pr_id: sample_id() },
                head_moved: false,
                preview_visible: true,
            }),
            ..AppState::default()
        };
        let _ = update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL)),
        );
        assert!(!s.composer.as_ref().unwrap().preview_visible);
        let _ = update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE)),
        );
        assert_eq!(s.composer.as_ref().unwrap().body, "xp");
        assert!(!s.composer.as_ref().unwrap().preview_visible);
        let _ = update(
            &mut s,
            &AppEvent::Key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL)),
        );
        assert!(s.composer.as_ref().unwrap().preview_visible);
    }

    #[test]
    fn thread_cycle_sets_scroll_to_focused() {
        let (mut s, _) = detail_with_threads(&["t1", "t2"]);
        update(&mut s, &key('n'));
        assert!(matches!(s.selection(), Selection::ReviewThread { .. }));
        assert!(s.active_tab().state.conversation_scroll_to_focused);
    }
}
