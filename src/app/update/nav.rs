use crate::data::models::PrId;

use super::super::{
    effect::Effect,
    state::{AppState, DetailTab, Selection, View},
    workspace::TabKind,
};
use super::{HALF_PAGE_ROWS, handle_tick, load_more_effects};

/// GitHub web URL of the PR in context for the current view/selection.
pub(super) fn selected_pr_url(state: &AppState) -> Option<String> {
    let id = match (state.current_view(), state.selection()) {
        (View::Dashboard, Selection::Dashboard { focused, rows }) => state
            .dashboard
            .as_ref()
            .and_then(|buckets| buckets.get(usize::from(*focused)))
            .and_then(|(_, prs)| prs.get(rows[usize::from(*focused)]))
            .map(|s| s.id.clone())?,
        (View::PrList { repo }, Selection::PrListRow(row)) => {
            state.pr_lists.get(&repo)?.get(*row).map(|s| s.id.clone())?
        }
        (View::PrDetail { id, .. } | View::Diff { id, .. }, _) => id.clone(),
        _ => return None,
    };
    Some(format!(
        "https://github.com/{}/{}/pull/{}",
        id.repo.owner, id.repo.name, id.number
    ))
}

pub(super) fn nav_down(state: &mut AppState) -> Vec<Effect> {
    match (state.current_view(), state.selection().clone()) {
        (View::Dashboard, Selection::Dashboard { focused, mut rows }) => {
            let idx = usize::from(focused);
            let len = bucket_len(state, idx);
            if len > 0 {
                rows[idx] = (rows[idx] + 1).min(len - 1);
                *state.selection_mut() = Selection::Dashboard { focused, rows };
            }
            load_more_effects(state)
        }
        (View::PrList { repo }, Selection::PrListRow(row)) => {
            let len = state.pr_lists.get(&repo).map_or(0, Vec::len);
            if len == 0 {
                return Vec::new();
            }
            *state.selection_mut() = Selection::PrListRow((row + 1).min(len - 1));
            load_more_effects(state)
        }
        (
            View::PrDetail {
                tab: DetailTab::Conversation,
                ..
            },
            _,
        ) => {
            let c = &mut state.active_tab_mut().state;
            c.conversation_cursor = c.conversation_cursor.saturating_add(1);
            c.conversation_scroll_to_focused = false;
            Vec::new()
        }
        _ => Vec::new(),
    }
}

pub(super) fn nav_up(state: &mut AppState) -> Vec<Effect> {
    match (state.current_view(), state.selection().clone()) {
        (View::Dashboard, Selection::Dashboard { focused, mut rows }) => {
            let idx = usize::from(focused);
            rows[idx] = rows[idx].saturating_sub(1);
            *state.selection_mut() = Selection::Dashboard { focused, rows };
        }
        (View::PrList { .. }, Selection::PrListRow(row)) => {
            *state.selection_mut() = Selection::PrListRow(row.saturating_sub(1));
        }
        (
            View::PrDetail {
                tab: DetailTab::Conversation,
                ..
            },
            _,
        ) => {
            let c = &mut state.active_tab_mut().state;
            c.conversation_cursor = c.conversation_cursor.saturating_sub(1);
            c.conversation_scroll_to_focused = false;
        }
        _ => {}
    }
    Vec::new()
}

pub(super) fn nav_top(state: &mut AppState) -> Vec<Effect> {
    match (state.current_view(), state.selection().clone()) {
        (View::Dashboard, Selection::Dashboard { focused, mut rows }) => {
            rows[usize::from(focused)] = 0;
            *state.selection_mut() = Selection::Dashboard { focused, rows };
        }
        (View::PrList { .. }, Selection::PrListRow(_)) => {
            *state.selection_mut() = Selection::PrListRow(0);
        }
        (
            View::PrDetail {
                tab: DetailTab::Conversation,
                ..
            },
            _,
        ) => {
            let c = &mut state.active_tab_mut().state;
            c.conversation_cursor = 0;
            c.conversation_scroll_to_focused = false;
        }
        _ => {}
    }
    Vec::new()
}

pub(super) fn nav_bottom(state: &mut AppState) -> Vec<Effect> {
    match (state.current_view(), state.selection().clone()) {
        (View::Dashboard, Selection::Dashboard { focused, mut rows }) => {
            let idx = usize::from(focused);
            let len = bucket_len(state, idx);
            rows[idx] = len.saturating_sub(1);
            *state.selection_mut() = Selection::Dashboard { focused, rows };
        }
        (View::PrList { repo }, Selection::PrListRow(_)) => {
            let len = state.pr_lists.get(&repo).map_or(0, Vec::len);
            if len > 0 {
                *state.selection_mut() = Selection::PrListRow(len - 1);
            }
        }
        (
            View::PrDetail {
                tab: DetailTab::Conversation,
                ..
            },
            _,
        ) => {
            let c = &mut state.active_tab_mut().state;
            c.conversation_cursor = usize::MAX;
            c.conversation_scroll_to_focused = false;
        }
        _ => {}
    }
    Vec::new()
}

pub(super) fn nav_open_selected(state: &mut AppState) -> Vec<Effect> {
    let id_opt: Option<PrId> = match (state.current_view(), state.selection()) {
        (View::Dashboard, Selection::Dashboard { focused, rows }) => state
            .dashboard
            .as_ref()
            .and_then(|buckets| buckets.get(usize::from(*focused)))
            .and_then(|(_, prs)| prs.get(rows[usize::from(*focused)]))
            .map(|s| s.id.clone()),
        (View::PrList { repo }, Selection::PrListRow(row)) => state
            .pr_lists
            .get(&repo)
            .and_then(|rows| rows.get(*row))
            .map(|s| s.id.clone()),
        _ => None,
    };
    let Some(id) = id_opt else {
        return Vec::new();
    };
    state.workspace.open(TabKind::Pr(id));
    handle_tick(state)
}

pub(super) fn nav_refresh(state: &AppState) -> Vec<Effect> {
    match state.current_view() {
        View::Dashboard => vec![Effect::FetchDashboard {
            bucket: None,
            cursor: None,
        }],
        View::PrList { repo } => vec![Effect::FetchPrList {
            repo: repo.clone(),
            cursor: None,
            filter: None,
            sort: None,
        }],
        View::PrDetail { id, .. } => vec![
            Effect::FetchPrDetail { id: id.clone() },
            Effect::FetchPrChecks { id: id.clone() },
        ],
        View::Diff { .. } => Vec::new(),
    }
}

fn bucket_len(state: &AppState, idx: usize) -> usize {
    state
        .dashboard
        .as_ref()
        .and_then(|b| b.get(idx))
        .map_or(0, |(_, rows)| rows.len())
}

pub(super) fn dashboard_focus(state: &mut AppState, delta: i8) {
    if let Selection::Dashboard { focused, rows } = *state.selection() {
        let next = (i16::from(focused) + i16::from(delta)).clamp(0, 2);
        *state.selection_mut() = Selection::Dashboard {
            focused: u8::try_from(next).unwrap_or(0),
            rows,
        };
    }
}

/// Move the list selection by a half page in the focused list. `down = true`
/// advances, `false` retreats; selection is clamped to the list bounds.
pub(super) fn page_move(state: &mut AppState, down: bool) -> Vec<Effect> {
    let mut effects = Vec::new();
    for _ in 0..HALF_PAGE_ROWS {
        effects = if down { nav_down(state) } else { nav_up(state) };
    }
    effects
}

pub(super) fn selected_pr_id_in_list(
    state: &AppState,
    repo: &crate::data::models::Repo,
) -> Option<PrId> {
    let Selection::PrListRow(row) = *state.selection() else {
        return None;
    };
    state.pr_lists.get(repo)?.get(row).map(|s| s.id.clone())
}

pub(super) fn current_repo(state: &AppState) -> Option<crate::data::models::Repo> {
    match state.current_view() {
        View::PrList { repo } => Some(repo.clone()),
        View::PrDetail { id, .. } | View::Diff { id, .. } => Some(id.repo.clone()),
        View::Dashboard => None,
    }
}
