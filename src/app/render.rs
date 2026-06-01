//! Unified render dispatcher: switches on `state.current_view()` and layers
//! overlays in fixed z-order
//! (toast → composer → help → palette → `action_menu` → `log_view`).

use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::Style,
    widgets::Paragraph,
};

use crate::app::actions::context_actions;
use crate::app::keymap::key_label;
use crate::app::state::{AppState, View};
use crate::app::workspace::TabKind;
use crate::ui::components::tab_strip::{StatusCluster, render_tab_strip};
use crate::ui::components::tabs::render_tabs;
use crate::ui::components::toast::render_toast;
use crate::ui::composer::render_composer;
use crate::ui::dashboard::{DashboardState, render_dashboard_state};
use crate::ui::footer::{FooterHint, render_footer};
use crate::ui::help::render_help_for_view;
use crate::ui::log_view::render_log_view;
use crate::ui::palette::render_palette;
/// Minimum terminal width to show the two-pane file-tree + diff split.
pub const FILES_SPLIT_MIN_WIDTH: u16 = 90;

/// Returns `true` when the viewport is wide enough to show the split Files
/// pane.  A width of `0` (unknown / unseeded) is treated as wide so that the
/// default path doesn't incorrectly restrict key behaviour.
#[must_use]
pub fn is_files_split_width(w: u16) -> bool {
    w == 0 || w >= FILES_SPLIT_MIN_WIDTH
}

use crate::ui::files_split::{DiffViewInputs, render_files_split};
use crate::ui::pr_detail::{
    ConversationScroll, render_checks, render_commits, render_conversation, render_files,
};
use crate::ui::pr_list::{PrListView, render_pr_list};
use crate::ui::theme::Theme;

/// Top-level render entry point. Splits `frame.area()` into tab strip + body +
/// contextual footer, dispatches to the view-specific renderer, then layers overlays.
pub fn render_app(f: &mut Frame, state: &AppState, theme: &Theme) {
    let area = f.area();
    if area.height == 0 || area.width == 0 {
        return;
    }
    let [strip_area, body_area, status_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(area);

    // Build tab strip labels from the workspace.
    let labels: Vec<String> = state
        .workspace
        .tabs
        .iter()
        .map(|tab| match &tab.kind {
            TabKind::Dashboard => "⌂".to_string(),
            TabKind::Repo(repo) => format!("{}/{}", repo.owner, repo.name),
            TabKind::Pr(id) => {
                // Use cached detail title if available, else just "#<n>".
                let title = state
                    .pr_details
                    .get(id)
                    .map(|d| d.summary.title.as_str())
                    .or_else(|| {
                        state.pr_lists.get(&id.repo).and_then(|list| {
                            list.iter()
                                .find(|s| s.id.number == id.number)
                                .map(|s| s.title.as_str())
                        })
                    });
                match title {
                    Some(t) => format!("#{} {}", id.number, t),
                    None => format!("#{}", id.number),
                }
            }
        })
        .collect();
    let strip_status = StatusCluster {
        rate_limit: state.rate_limit.as_ref(),
        last_refresh_secs: status_last_refresh_secs(state),
        pending_write: state.pending_write.is_some(),
        checkout_inflight: state.checkout_inflight.is_some(),
    };
    render_tab_strip(
        f,
        strip_area,
        &labels,
        state.workspace.active,
        Some(&strip_status),
        theme,
    );

    render_view(f, body_area, state, theme);

    // Build footer hints from the action registry.
    let mut actions = context_actions(state);
    // Keep only footer-eligible actions, sort by priority (lowest = leftmost).
    actions.retain(|a| a.footer.is_some());
    actions.sort_by_key(|a| a.footer.unwrap_or(u8::MAX));
    let hints: Vec<FooterHint> = actions
        .iter()
        .filter_map(|a| {
            // Drop actions that have no bound key — we can't show a useful hint.
            let key_ev = state.keymap.key_for(&a.intent)?;
            Some(FooterHint {
                key: key_label(&key_ev),
                label: a.label.clone(),
                danger: a.danger,
            })
        })
        .collect();
    render_footer(f, status_area, &hints, theme);

    // Overlay z-order: toast → composer → help → palette → action_menu → log_view.
    if let Some(toast) = state.toast.as_ref() {
        render_toast(f, area, toast, theme);
    }
    if let Some(composer) = state.composer.as_ref() {
        render_composer(f, area, composer, theme);
    }
    if state.help_overlay {
        render_help_for_view(f, area, state, theme);
    }
    if let Some(palette) = state.palette.as_ref() {
        render_palette(f, area, palette, state, theme);
    }
    if state.action_menu.is_some() {
        crate::ui::action_menu::render_action_menu(f, area, state, theme);
    }
    if let Some(log_view) = state.log_view.as_ref() {
        let ring = crate::logging::global_ring();
        render_log_view(f, area, &ring, log_view, theme);
    }
}

/// Seconds since the current view's primary resource was last refreshed.
fn status_last_refresh_secs(state: &AppState) -> Option<u64> {
    use crate::data::cache::CacheKey;
    let key = match state.current_view() {
        View::Dashboard => CacheKey::DashboardReviewRequested,
        View::PrList { repo } => CacheKey::PrList(repo.clone()),
        View::PrDetail { id, .. } | View::Diff { id, .. } => CacheKey::PrDetail(id.clone()),
    };
    state.last_refresh.get(&key).map(|t| t.elapsed().as_secs())
}

fn render_view(f: &mut Frame, area: Rect, state: &AppState, theme: &Theme) {
    match state.current_view() {
        View::Dashboard => render_dashboard_view(f, area, state, theme),
        View::PrList { repo } => render_pr_list_view(f, area, state, &repo, theme),
        View::PrDetail { id, tab } => render_pr_detail_view(f, area, state, &id, tab, theme),
        View::Diff { id, file_index } => {
            render_diff_view_wrap(f, area, state, &id, file_index, theme);
        }
    }
}

fn render_dashboard_view(f: &mut Frame, area: Rect, state: &AppState, theme: &Theme) {
    let dash_state = match state.dashboard.as_ref() {
        Some(buckets) => DashboardState::Loaded(buckets.clone()),
        None => DashboardState::Loading,
    };
    let (focused, rows) = match *state.selection() {
        crate::app::state::Selection::Dashboard { focused, rows } => (usize::from(focused), rows),
        _ => (0, [0; 3]),
    };
    render_dashboard_state(f, area, &dash_state, focused, rows, theme);
}

fn render_pr_list_view(
    f: &mut Frame,
    area: Rect,
    state: &AppState,
    repo: &crate::data::models::Repo,
    theme: &Theme,
) {
    let empty: Vec<crate::data::models::PrSummary> = Vec::new();
    let rows = state.pr_lists.get(repo).unwrap_or(&empty);
    let selected = match *state.selection() {
        crate::app::state::Selection::PrListRow(i) => i,
        _ => 0,
    };
    let view = PrListView {
        rows,
        selected,
        loading: state.pr_list_loading.contains(repo),
        filter_label: None,
    };
    render_pr_list(f, area, &view, theme);
}

fn render_pr_detail_view(
    f: &mut Frame,
    area: Rect,
    state: &AppState,
    id: &crate::data::models::PrId,
    tab: crate::app::state::DetailTab,
    theme: &Theme,
) {
    use crate::app::state::DetailTab;

    const SUB_TAB_LABELS: &[&str] = &["Conversation", "Files", "Checks", "Commits"];

    // Split area: 1-line sub-tab strip + remaining content.
    let [strip_area, content_area] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(area);

    let active_index = match tab {
        DetailTab::Conversation => 0,
        DetailTab::Files => 1,
        DetailTab::Checks => 2,
        DetailTab::Commits => 3,
    };
    render_tabs(f, strip_area, SUB_TAB_LABELS, active_index, false, theme);

    let Some(detail) = state.pr_details.get(id) else {
        let p = Paragraph::new("Loading PR…").style(Style::default().fg(theme.dim));
        f.render_widget(p, content_area);
        return;
    };
    match tab {
        DetailTab::Conversation => {
            let focused_thread = match *state.selection() {
                crate::app::state::Selection::ReviewThread { thread_index } => Some(thread_index),
                _ => None,
            };
            render_conversation(
                f,
                content_area,
                detail,
                focused_thread,
                ConversationScroll {
                    cursor: state.active_tab().state.conversation_cursor,
                    scroll_to_focused: state.active_tab().state.conversation_scroll_to_focused,
                    now: chrono::Utc::now(),
                },
                theme,
            );
        }
        DetailTab::Files => {
            let Some(files) = state.pr_files.get(id) else {
                let p = Paragraph::new("Loading files…").style(Style::default().fg(theme.dim));
                f.render_widget(p, content_area);
                return;
            };
            let selected = match *state.selection() {
                crate::app::state::Selection::DetailFile(i) => i,
                _ => 0,
            };
            let tab_state = &state.active_tab().state;
            let diff_file = tab_state.diff_file;
            if content_area.width >= FILES_SPLIT_MIN_WIDTH && diff_file.is_none() {
                // Wide mode: two-pane split.
                let file = files.files.get(selected);
                let diff_inputs = DiffViewInputs {
                    file,
                    diff_cursor: tab_state.diff_cursor,
                    whitespace_hidden: tab_state.diff_whitespace_hidden,
                    diff_selection: tab_state.diff_selection,
                    diff_mode: tab_state.diff_mode,
                };
                render_files_split(
                    f,
                    content_area,
                    files,
                    selected,
                    tab_state.files_focus,
                    &diff_inputs,
                    theme,
                );
            } else {
                // Narrow mode (or full-screen diff is open): full-width list.
                render_files(f, content_area, files, selected, theme);
            }
        }
        DetailTab::Checks => {
            let Some(checks) = state.pr_checks.get(id) else {
                let p = Paragraph::new("Loading checks…").style(Style::default().fg(theme.dim));
                f.render_widget(p, content_area);
                return;
            };
            render_checks(f, content_area, checks, theme);
        }
        DetailTab::Commits => {
            render_commits(f, content_area, detail, 0, theme);
        }
    }
}

fn render_diff_view_wrap(
    f: &mut Frame,
    area: Rect,
    state: &AppState,
    id: &crate::data::models::PrId,
    file_index: usize,
    theme: &Theme,
) {
    let Some(files) = state.pr_files.get(id) else {
        let p = Paragraph::new("Loading diff…").style(Style::default().fg(theme.dim));
        f.render_widget(p, area);
        return;
    };
    let Some(file) = files.files.get(file_index) else {
        let p = Paragraph::new("File not found.").style(Style::default().fg(theme.dim));
        f.render_widget(p, area);
        return;
    };
    let tab_state = &state.active_tab().state;
    let line_idx = tab_state.diff_cursor;
    let ws_hidden = tab_state.diff_whitespace_hidden;
    let selection = tab_state.diff_selection.map(|s| s.range());
    crate::ui::diff::render_diff_view(
        f,
        area,
        file,
        line_idx,
        ws_hidden,
        selection,
        tab_state.diff_mode,
        theme,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn buf_to_string(t: &Terminal<TestBackend>) -> String {
        let buf = t.backend().buffer();
        let mut s = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                s.push_str(buf[(x, y)].symbol());
            }
            s.push('\n');
        }
        s
    }

    #[test]
    fn renders_dashboard_loading_with_footer_non_blank() {
        let state = AppState::default();
        let backend = TestBackend::new(80, 24);
        let mut t = Terminal::new(backend).unwrap();
        t.draw(|f| render_app(f, &state, &Theme::dark())).unwrap();
        let rendered = buf_to_string(&t);
        assert!(
            rendered.contains("Loading dashboard"),
            "expected loading text, got:\n{rendered}"
        );
        assert!(
            rendered.contains("actions"),
            "expected footer tail, got:\n{rendered}"
        );
    }

    #[test]
    fn renders_palette_overlay_above_dashboard() {
        let state = AppState {
            palette: Some(crate::ui::palette::PaletteState::default()),
            ..AppState::default()
        };
        let backend = TestBackend::new(80, 24);
        let mut t = Terminal::new(backend).unwrap();
        t.draw(|f| render_app(f, &state, &Theme::dark())).unwrap();
        let rendered = buf_to_string(&t);
        // Palette title is ` : ` (with surrounding spaces); fuzzy commands are
        // shown when the input is empty.
        assert!(
            rendered.contains("dashboard"),
            "expected palette static command, got:\n{rendered}"
        );
    }

    #[test]
    fn pr_detail_sub_tab_strip_shows_active_tab() {
        use crate::app::state::{DetailTab, Selection};
        use crate::app::workspace::TabKind;
        use crate::data::models::{
            ChecksRollup, Mergeable, PrDetail, PrId, PrState, PrSummary, Repo, ReviewDecision,
        };
        use chrono::TimeZone;
        use chrono::Utc;

        let pr_id = PrId {
            repo: Repo {
                owner: "acme".into(),
                name: "widgets".into(),
            },
            number: 42,
        };
        let summary = PrSummary {
            id: pr_id.clone(),
            title: "Add warp drive".into(),
            is_draft: false,
            state: PrState::Open,
            author: "octocat".into(),
            base: "main".into(),
            head: "feature/warp".into(),
            additions: 10,
            deletions: 2,
            changed_files: 1,
            comments: 3,
            review_decision: Some(ReviewDecision::ReviewRequired),
            mergeable: Mergeable::Clean,
            checks: ChecksRollup::Success,
            labels: vec![],
            created_at: Utc.timestamp_opt(1_600_000_000, 0).unwrap(),
            updated_at: Utc.timestamp_opt(1_600_000_000, 0).unwrap(),
        };
        let detail = PrDetail {
            summary,
            body: "Body text.".into(),
            body_html: None,
            requested_reviewers: vec![],
            assignees: vec![],
            head_sha: "abc123".into(),
            merge_commit_sha: None,
            base_ref_oid: "base_sha".into(),
            available_merge_methods: vec![],
            version: 1,
            timeline: vec![],
            review_threads: vec![],
        };

        let mut state = AppState::default();
        state.workspace.open(TabKind::Pr(pr_id.clone()));
        state.active_tab_mut().state.detail_tab = DetailTab::Files;
        state.active_tab_mut().state.selection = Selection::DetailFile(0);
        state.pr_details.insert(pr_id.clone(), detail);

        let backend = TestBackend::new(80, 10);
        let mut t = Terminal::new(backend).unwrap();
        t.draw(|f| render_app(f, &state, &Theme::dark())).unwrap();
        insta::assert_snapshot!("pr_detail_sub_tab_strip_files_active", t.backend());
    }

    #[test]
    fn renders_log_view_overlay() {
        let ring = crate::logging::global_ring();
        ring.push("render-test-log-line");
        let state = AppState {
            log_view: Some(crate::ui::log_view::LogViewState::default()),
            ..AppState::default()
        };
        let backend = TestBackend::new(80, 24);
        let mut t = Terminal::new(backend).unwrap();
        t.draw(|f| render_app(f, &state, &Theme::dark())).unwrap();
        let rendered = buf_to_string(&t);
        assert!(
            rendered.contains(":log"),
            "expected log overlay title, got:\n{rendered}"
        );
        assert!(
            rendered.contains("render-test-log-line"),
            "expected log line, got:\n{rendered}"
        );
    }
}
