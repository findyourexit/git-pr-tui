use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::data::models::PrId;

use super::super::{
    effect::Effect,
    state::{
        AppState, ComposerContext, FilterModalState, PR_LIST_FILTER_OPTIONS, PR_LIST_SORT_OPTIONS,
        REVIEW_MODAL_OPTIONS, ReviewModalOption, ReviewModalState, SearchState, Selection,
        SortModalState, Toast, ToastKind, View,
    },
};
use super::{
    LOAD_MORE_THRESHOLD,
    nav::current_repo,
    push_overlay_cue,
    write::{blocked_reasons, execute_merge_from_modal, review_option_to_state},
};

pub(super) fn handle_composer_key(state: &mut AppState, k: &KeyEvent) -> Vec<Effect> {
    // Refuse to submit a line comment whose anchor commit moved under us
    // (force-push detected while composing). Other contexts are sha-agnostic.
    if matches!(
        (k.code, k.modifiers),
        (KeyCode::Char('s'), KeyModifiers::CONTROL)
    ) && let Some(c) = state.composer.as_ref()
        && c.head_moved
        && matches!(c.context, ComposerContext::LineComment { .. })
    {
        state.toast = Some(Toast {
            kind: ToastKind::Error,
            message: "Head moved — press R to load the new diff, then re-compose".into(),
            created_at: std::time::Instant::now(),
        });
        return Vec::new();
    }
    let Some(composer) = state.composer.as_mut() else {
        return Vec::new();
    };
    match (k.code, k.modifiers) {
        (KeyCode::Esc, _) => {
            state.composer = None;
            Vec::new()
        }
        (KeyCode::Enter, KeyModifiers::NONE | KeyModifiers::SHIFT) => {
            composer.body.push('\n');
            Vec::new()
        }
        (KeyCode::Char('p'), KeyModifiers::CONTROL) => {
            composer.preview_visible = !composer.preview_visible;
            Vec::new()
        }
        (KeyCode::Char('s'), KeyModifiers::CONTROL) => {
            if composer.body.is_empty() {
                return Vec::new();
            }
            let body = std::mem::take(&mut composer.body);
            let context = composer.context.clone();
            state.composer = None;
            match context {
                ComposerContext::ReplyToThread { pr_id, thread_id } => {
                    let version_at_submit = state.pr_details.get(&pr_id).map_or(0, |d| d.version);
                    vec![Effect::ExecuteWrite {
                        kind: crate::data::cache::WriteKind::ReplyToThread { id: pr_id },
                        request: crate::app::effect::WriteRequest::ReplyToThread {
                            thread_id,
                            body,
                        },
                        version_at_submit,
                    }]
                }
                ComposerContext::PrComment { pr_id } => {
                    let version_at_submit = state.pr_details.get(&pr_id).map_or(0, |d| d.version);
                    vec![Effect::ExecuteWrite {
                        kind: crate::data::cache::WriteKind::PostPrComment { id: pr_id },
                        request: crate::app::effect::WriteRequest::PostPrComment { body },
                        version_at_submit,
                    }]
                }
                ComposerContext::LineComment {
                    pr_id,
                    head_sha,
                    comment,
                } => {
                    let version_at_submit = state.pr_details.get(&pr_id).map_or(0, |d| d.version);
                    let mut comment = comment;
                    comment.body = body;
                    vec![Effect::ExecuteWrite {
                        kind: crate::data::cache::WriteKind::PostLineComment { id: pr_id },
                        request: crate::app::effect::WriteRequest::PostLineComment {
                            head_sha,
                            comment,
                        },
                        version_at_submit,
                    }]
                }
            }
        }
        (KeyCode::Backspace, KeyModifiers::NONE) => {
            composer.body.pop();
            Vec::new()
        }
        (KeyCode::Char(c), KeyModifiers::NONE | KeyModifiers::SHIFT) => {
            composer.body.push(c);
            Vec::new()
        }
        _ => Vec::new(),
    }
}

pub(super) fn open_filter_modal(state: &mut AppState) -> Vec<Effect> {
    if !matches!(state.current_view(), View::PrList { .. }) {
        return Vec::new();
    }
    state.active_tab_mut().state.filter_modal = Some(FilterModalState::default());
    push_overlay_cue(state);
    Vec::new()
}

pub(super) fn open_sort_modal(state: &mut AppState) -> Vec<Effect> {
    if !matches!(state.current_view(), View::PrList { .. }) {
        return Vec::new();
    }
    state.active_tab_mut().state.sort_modal = Some(SortModalState::default());
    push_overlay_cue(state);
    Vec::new()
}

pub(super) fn open_search(state: &mut AppState) -> Vec<Effect> {
    if !matches!(state.current_view(), View::PrList { .. }) {
        return Vec::new();
    }
    state.active_tab_mut().state.search = Some(SearchState::default());
    Vec::new()
}

pub(super) fn handle_search_key(state: &mut AppState, k: &KeyEvent) -> Vec<Effect> {
    let Some(search) = state.active_tab_mut().state.search.as_mut() else {
        return Vec::new();
    };
    match (k.code, k.modifiers) {
        (KeyCode::Char(c), KeyModifiers::NONE | KeyModifiers::SHIFT) => {
            search.query.push(c);
            Vec::new()
        }
        (KeyCode::Backspace, KeyModifiers::NONE) => {
            search.query.pop();
            Vec::new()
        }
        (KeyCode::Esc | KeyCode::Enter, KeyModifiers::NONE) => {
            state.active_tab_mut().state.search = None;
            Vec::new()
        }
        _ => Vec::new(),
    }
}

pub(super) fn handle_filter_modal_key(state: &mut AppState, k: &KeyEvent) -> Vec<Effect> {
    let Some(modal) = state.active_tab_mut().state.filter_modal.as_mut() else {
        return Vec::new();
    };
    match (k.code, k.modifiers) {
        (KeyCode::Char('j') | KeyCode::Down, KeyModifiers::NONE) => {
            let max = PR_LIST_FILTER_OPTIONS.len().saturating_sub(1);
            modal.selected = (modal.selected + 1).min(max);
            Vec::new()
        }
        (KeyCode::Char('k') | KeyCode::Up, KeyModifiers::NONE) => {
            modal.selected = modal.selected.saturating_sub(1);
            Vec::new()
        }
        (KeyCode::Enter, KeyModifiers::NONE) => {
            let selected_filter = PR_LIST_FILTER_OPTIONS
                .get(
                    state
                        .active_tab()
                        .state
                        .filter_modal
                        .as_ref()
                        .map_or(0, |m| m.selected),
                )
                .map(|(_, f)| *f);
            state.active_tab_mut().state.filter_modal = None;
            let View::PrList { repo } = state.current_view() else {
                return Vec::new();
            };
            state.pr_list_loading.insert(repo.clone());
            vec![Effect::FetchPrList {
                repo,
                cursor: None,
                filter: selected_filter,
                sort: None,
            }]
        }
        (KeyCode::Esc, KeyModifiers::NONE) => {
            state.active_tab_mut().state.filter_modal = None;
            Vec::new()
        }
        _ => Vec::new(),
    }
}

pub(super) fn handle_sort_modal_key(state: &mut AppState, k: &KeyEvent) -> Vec<Effect> {
    let Some(modal) = state.active_tab_mut().state.sort_modal.as_mut() else {
        return Vec::new();
    };
    match (k.code, k.modifiers) {
        (KeyCode::Char('j') | KeyCode::Down, KeyModifiers::NONE) => {
            let max = PR_LIST_SORT_OPTIONS.len().saturating_sub(1);
            modal.selected = (modal.selected + 1).min(max);
            Vec::new()
        }
        (KeyCode::Char('k') | KeyCode::Up, KeyModifiers::NONE) => {
            modal.selected = modal.selected.saturating_sub(1);
            Vec::new()
        }
        (KeyCode::Enter, KeyModifiers::NONE) => {
            let selected_sort = PR_LIST_SORT_OPTIONS
                .get(
                    state
                        .active_tab()
                        .state
                        .sort_modal
                        .as_ref()
                        .map_or(0, |m| m.selected),
                )
                .map(|(_, s)| *s);
            state.active_tab_mut().state.sort_modal = None;
            let View::PrList { repo } = state.current_view() else {
                return Vec::new();
            };
            state.pr_list_loading.insert(repo.clone());
            vec![Effect::FetchPrList {
                repo,
                cursor: None,
                filter: None,
                sort: selected_sort,
            }]
        }
        (KeyCode::Esc, KeyModifiers::NONE) => {
            state.active_tab_mut().state.sort_modal = None;
            Vec::new()
        }
        _ => Vec::new(),
    }
}

pub(super) fn open_review_modal(state: &mut AppState, id: &PrId) {
    state.review_modal = Some(ReviewModalState {
        pr_id: id.clone(),
        selected: 0,
        body: String::new(),
    });
    push_overlay_cue(state);
}

pub(super) fn open_merge_modal_for(state: &mut AppState, id: &PrId) {
    open_merge_modal(state, id);
}

pub(super) fn open_close_reopen_confirm(state: &mut AppState, id: &PrId) {
    use crate::app::state::{CloseReopenAction, CloseReopenConfirmState};
    use crate::data::models::PrState;
    let pr_state = state.pr_details.get(id).map(|d| d.summary.state);
    if matches!(pr_state, Some(PrState::Merged)) {
        state.toast = Some(Toast {
            kind: ToastKind::Warning,
            message: "PR is merged \u{2014} cannot close or reopen".into(),
            created_at: std::time::Instant::now(),
        });
        return;
    }
    let action = match pr_state {
        Some(PrState::Closed) => CloseReopenAction::Reopen,
        _ => CloseReopenAction::Close,
    };
    state.close_reopen_confirm = Some(CloseReopenConfirmState {
        pr_id: id.clone(),
        action,
    });
    push_overlay_cue(state);
}

/// Open the merge modal (or emit a toast) for the PR at `id`, dispatching on
/// the PR's `Mergeable` state. Returns true if any state was
/// changed (modal opened or toast set).
pub(super) fn open_merge_modal(state: &mut AppState, id: &PrId) -> bool {
    use crate::app::state::{MergeModalKind, MergeModalState};
    use crate::data::models::Mergeable;
    let Some(detail) = state.pr_details.get(id) else {
        state.toast = Some(Toast {
            kind: ToastKind::Warning,
            message: "PR details not loaded yet".into(),
            created_at: std::time::Instant::now(),
        });
        return true;
    };
    match detail.summary.mergeable {
        Mergeable::Clean | Mergeable::Unstable | Mergeable::HasHooks => {
            state.merge_modal = Some(MergeModalState {
                pr_id: id.clone(),
                head_sha: detail.head_sha.clone(),
                kind: MergeModalKind::MethodPicker {
                    methods: detail.available_merge_methods.clone(),
                    selected: 0,
                },
            });
            push_overlay_cue(state);
        }
        Mergeable::Behind => {
            state.toast = Some(Toast {
                kind: ToastKind::Warning,
                message: "Base branch moved \u{2014} merge would create a non-fast-forward; rebase locally".into(),
                created_at: std::time::Instant::now(),
            });
        }
        Mergeable::Blocked => {
            state.merge_modal = Some(MergeModalState {
                pr_id: id.clone(),
                head_sha: detail.head_sha.clone(),
                kind: MergeModalKind::Blocked {
                    reasons: blocked_reasons(detail),
                },
            });
            push_overlay_cue(state);
        }
        Mergeable::Dirty => {
            state.toast = Some(Toast {
                kind: ToastKind::Error,
                message: "Merge conflicts; resolve locally".into(),
                created_at: std::time::Instant::now(),
            });
        }
        Mergeable::Draft => {
            state.toast = Some(Toast {
                kind: ToastKind::Warning,
                message: "PR is a draft \u{2014} promote-to-ready required (v2)".into(),
                created_at: std::time::Instant::now(),
            });
        }
        Mergeable::Unknown => {
            state.toast = Some(Toast {
                kind: ToastKind::Info,
                message: "GitHub still computing mergeability; try again in a moment".into(),
                created_at: std::time::Instant::now(),
            });
        }
    }
    true
}

/// GitHub returns `Blocked` when required reviews / required checks fail. We
/// can't always know exactly which without an extra protected-branch query,
/// so derive what we can from the `PrDetail` and fall back to a generic note.
pub(super) fn handle_review_modal_key(state: &mut AppState, k: &KeyEvent) -> Vec<Effect> {
    let Some(modal) = state.review_modal.as_mut() else {
        return Vec::new();
    };
    match (k.code, k.modifiers) {
        (KeyCode::Esc, _) => {
            state.review_modal = None;
            Vec::new()
        }
        (KeyCode::Down, KeyModifiers::NONE) => {
            let max = REVIEW_MODAL_OPTIONS.len().saturating_sub(1);
            modal.selected = (modal.selected + 1).min(max);
            Vec::new()
        }
        (KeyCode::Up, KeyModifiers::NONE) => {
            modal.selected = modal.selected.saturating_sub(1);
            Vec::new()
        }
        (KeyCode::Char('s'), KeyModifiers::CONTROL) => {
            let option = REVIEW_MODAL_OPTIONS
                .get(modal.selected)
                .copied()
                .unwrap_or(ReviewModalOption::Comment);
            let review_state = review_option_to_state(option);
            let body = modal.body.clone();
            if matches!(option, ReviewModalOption::Comment) && body.is_empty() {
                return Vec::new();
            }
            let pr_id = modal.pr_id.clone();
            let Some(detail) = state.pr_details.get(&pr_id) else {
                return Vec::new();
            };
            let head_sha = detail.head_sha.clone();
            let version_at_submit = detail.version;
            state.review_modal = None;
            vec![Effect::ExecuteWrite {
                kind: crate::data::cache::WriteKind::SubmitReview { id: pr_id },
                request: crate::app::effect::WriteRequest::SubmitReview {
                    state: review_state,
                    body,
                    line_comments: vec![],
                    head_sha,
                },
                version_at_submit,
            }]
        }
        (KeyCode::Backspace, KeyModifiers::NONE) => {
            modal.body.pop();
            Vec::new()
        }
        (KeyCode::Char(c), KeyModifiers::NONE | KeyModifiers::SHIFT) => {
            modal.body.push(c);
            Vec::new()
        }
        _ => Vec::new(),
    }
}

pub(super) fn handle_merge_modal_key(state: &mut AppState, k: &KeyEvent) -> Vec<Effect> {
    use crate::app::state::MergeModalKind;
    let Some(modal) = state.merge_modal.as_mut() else {
        return Vec::new();
    };
    match (k.code, k.modifiers) {
        (KeyCode::Esc, _) => {
            state.merge_modal = None;
            Vec::new()
        }
        (KeyCode::Down, KeyModifiers::NONE) => {
            if let MergeModalKind::MethodPicker { methods, selected } = &mut modal.kind {
                let max = methods.len().saturating_sub(1);
                *selected = (*selected + 1).min(max);
            }
            Vec::new()
        }
        (KeyCode::Up, KeyModifiers::NONE) => {
            if let MergeModalKind::MethodPicker { selected, .. } = &mut modal.kind {
                *selected = selected.saturating_sub(1);
            }
            Vec::new()
        }
        (KeyCode::Enter, KeyModifiers::NONE) => execute_merge_from_modal(state),
        _ => Vec::new(),
    }
}

pub(super) fn handle_close_reopen_confirm_key(state: &mut AppState, k: &KeyEvent) -> Vec<Effect> {
    use crate::app::state::CloseReopenAction;
    let Some(confirm) = state.close_reopen_confirm.as_ref() else {
        return Vec::new();
    };
    match (k.code, k.modifiers) {
        (KeyCode::Esc | KeyCode::Char('n' | 'N'), _) => {
            state.close_reopen_confirm = None;
            Vec::new()
        }
        (KeyCode::Enter | KeyCode::Char('y' | 'Y'), _) => {
            let pr_id = confirm.pr_id.clone();
            let action = confirm.action;
            let version_at_submit = state.pr_details.get(&pr_id).map_or(0, |d| d.version);
            state.close_reopen_confirm = None;
            let (kind, request) = match action {
                CloseReopenAction::Close => (
                    crate::data::cache::WriteKind::Close { id: pr_id.clone() },
                    crate::app::effect::WriteRequest::Close,
                ),
                CloseReopenAction::Reopen => (
                    crate::data::cache::WriteKind::Reopen { id: pr_id.clone() },
                    crate::app::effect::WriteRequest::Reopen,
                ),
            };
            vec![Effect::ExecuteWrite {
                kind,
                request,
                version_at_submit,
            }]
        }
        _ => Vec::new(),
    }
}

pub(super) fn load_more_effects(state: &mut AppState) -> Vec<Effect> {
    match state.current_view() {
        View::Dashboard => load_more_dashboard(state),
        View::PrList { repo } => load_more_pr_list(state, &repo),
        _ => Vec::new(),
    }
}

pub(super) fn load_more_dashboard(state: &mut AppState) -> Vec<Effect> {
    use Selection;
    let Selection::Dashboard { focused, rows } = *state.selection() else {
        return Vec::new();
    };
    let focused_idx = usize::from(focused);
    let row = rows[focused_idx];
    let Some((bucket, rows_len)) = state
        .dashboard
        .as_ref()
        .and_then(|buckets| buckets.get(focused_idx))
        .map(|(b, rows)| (*b, rows.len()))
    else {
        return Vec::new();
    };
    if row + LOAD_MORE_THRESHOLD < rows_len {
        return Vec::new();
    }
    // Only paginate when the bucket actually has a next page. Firing with no
    // cursor would re-fetch page 1 and append it again (the duplication bug).
    let Some(cursor) = state.dashboard_cursors.get(&bucket).cloned() else {
        return Vec::new();
    };
    // Don't fire a second page request for a bucket while one is in flight
    // (same cursor would append the same page twice).
    if !state.dashboard_loading.insert(bucket) {
        return Vec::new();
    }
    vec![Effect::FetchDashboard {
        bucket: Some(bucket),
        cursor: Some(cursor),
    }]
}

pub(super) fn load_more_pr_list(
    state: &mut AppState,
    repo: &crate::data::models::Repo,
) -> Vec<Effect> {
    use Selection;
    let Selection::PrListRow(row) = *state.selection() else {
        return Vec::new();
    };
    let Some(rows) = state.pr_lists.get(repo) else {
        return Vec::new();
    };
    if row + LOAD_MORE_THRESHOLD < rows.len() {
        return Vec::new();
    }
    let Some(cursor) = state.pr_list_cursors.get(repo).cloned() else {
        return Vec::new();
    };
    // Don't fire another page request while one is in flight for this repo.
    if !state.pr_list_paginating.insert(repo.clone()) {
        return Vec::new();
    }
    vec![Effect::FetchPrList {
        repo: repo.clone(),
        cursor: Some(cursor),
        filter: None,
        sort: None,
    }]
}

pub(super) fn handle_palette_key(state: &mut AppState, k: &KeyEvent) -> Vec<Effect> {
    use crate::ui::palette::{PaletteEntry, matches as palette_matches};
    match (k.code, k.modifiers) {
        (KeyCode::Esc, _) => {
            state.palette = None;
            Vec::new()
        }
        (KeyCode::Backspace, _) => {
            if let Some(p) = state.palette.as_mut() {
                p.input.backspace();
                p.selected = 0;
            }
            Vec::new()
        }
        (KeyCode::Up, _) => {
            if let Some(p) = state.palette.as_mut() {
                p.selected = p.selected.saturating_sub(1);
            }
            Vec::new()
        }
        (KeyCode::Down, _) => {
            let query = state.palette.as_ref().map(|p| p.input.text.clone());
            if let Some(query) = query {
                let len = palette_matches(&query, state).len();
                if let Some(p) = state.palette.as_mut()
                    && p.selected + 1 < len
                {
                    p.selected += 1;
                }
            }
            Vec::new()
        }
        (KeyCode::Enter, _) => {
            let Some(p) = state.palette.as_ref() else {
                return Vec::new();
            };
            let query = p.input.text.clone();
            let selected = p.selected;
            let results = palette_matches(&query, state);
            let Some((entry, _)) = results.into_iter().nth(selected) else {
                state.palette = None;
                return Vec::new();
            };
            state.palette = None;
            match entry {
                PaletteEntry::Action(a) => super::apply_intent(state, &a.intent),
                PaletteEntry::Nav(cmd) => dispatch_palette_command(state, cmd),
            }
        }
        (KeyCode::Char(c), m) if !m.contains(KeyModifiers::CONTROL) => {
            if let Some(p) = state.palette.as_mut() {
                p.input.insert_char(c);
                p.selected = 0;
            }
            Vec::new()
        }
        _ => Vec::new(),
    }
}

#[allow(clippy::needless_pass_by_value)]
pub(super) fn dispatch_palette_command(
    state: &mut AppState,
    cmd: crate::ui::palette::Command,
) -> Vec<Effect> {
    use crate::ui::palette::Command;
    match cmd {
        Command::Quit => {
            state.should_quit = true;
            Vec::new()
        }
        Command::Dashboard => {
            state.workspace.goto(0);
            super::handle_tick(state)
        }
        Command::Log => {
            state.log_view = Some(crate::ui::log_view::LogViewState::default());
            Vec::new()
        }
        Command::Refresh => super::handle_tick(state),
        Command::Repo(repo) => {
            use crate::app::workspace::TabKind;
            state.workspace.open(TabKind::Repo(repo));
            super::handle_tick(state)
        }
        Command::Pr(n) => {
            if let Some(repo) = current_repo(state) {
                use crate::app::workspace::TabKind;
                state.workspace.open(TabKind::Pr(crate::data::models::PrId {
                    repo,
                    number: u64::from(n),
                }));
                // Fetch immediately rather than waiting for the next tick.
                super::handle_tick(state)
            } else {
                state.toast = Some(Toast {
                    kind: ToastKind::Warning,
                    message: "Open a repo first to jump to a PR by number".into(),
                    created_at: std::time::Instant::now(),
                });
                Vec::new()
            }
        }
    }
}

pub(super) fn handle_action_menu_key(state: &mut AppState, k: &KeyEvent) -> Vec<Effect> {
    use crate::ui::action_menu::filtered_actions;
    if state.action_menu.is_none() {
        return Vec::new();
    }
    match k.code {
        KeyCode::Esc => {
            state.action_menu = None;
            Vec::new()
        }
        KeyCode::Enter => {
            let selected = state.action_menu.as_ref().map_or(0, |m| m.selected);
            let actions = filtered_actions(state);
            let selected = selected.min(actions.len().saturating_sub(1));
            if let Some(action) = actions.into_iter().nth(selected) {
                let intent = action.intent.clone();
                state.action_menu = None;
                super::apply_intent(state, &intent)
            } else {
                state.action_menu = None;
                Vec::new()
            }
        }
        KeyCode::Up => {
            let actions = filtered_actions(state);
            if let Some(m) = state.action_menu.as_mut() {
                let len = actions.len();
                if len > 0 {
                    m.selected = m.selected.saturating_sub(1);
                }
            }
            Vec::new()
        }
        KeyCode::Down => {
            let actions = filtered_actions(state);
            if let Some(m) = state.action_menu.as_mut() {
                let len = actions.len();
                if len > 0 {
                    m.selected = (m.selected + 1).min(len - 1);
                }
            }
            Vec::new()
        }
        KeyCode::Backspace => {
            if let Some(m) = state.action_menu.as_mut() {
                m.input.backspace();
            }
            // Clamp selection after filter changes.
            let actions = filtered_actions(state);
            if let Some(m) = state.action_menu.as_mut() {
                let len = actions.len();
                m.selected = if len > 0 { m.selected.min(len - 1) } else { 0 };
            }
            Vec::new()
        }
        KeyCode::Char(c)
            if !k
                .modifiers
                .contains(crossterm::event::KeyModifiers::CONTROL)
                && !k.modifiers.contains(crossterm::event::KeyModifiers::ALT) =>
        {
            if let Some(m) = state.action_menu.as_mut() {
                m.input.insert_char(c);
            }
            // Clamp selection after filter changes.
            let actions = filtered_actions(state);
            if let Some(m) = state.action_menu.as_mut() {
                let len = actions.len();
                m.selected = if len > 0 { m.selected.min(len - 1) } else { 0 };
            }
            Vec::new()
        }
        _ => Vec::new(),
    }
}

pub(super) fn handle_log_view_key(state: &mut AppState, k: &KeyEvent) -> Vec<Effect> {
    let len = crate::logging::global_ring()
        .tail(crate::ui::log_view::LOG_VIEW_TAIL)
        .len();
    let Some(view) = state.log_view.as_mut() else {
        return Vec::new();
    };
    match (k.code, k.modifiers) {
        (KeyCode::Esc | KeyCode::Char('q'), _) => {
            state.log_view = None;
        }
        (KeyCode::Char('j') | KeyCode::Down, _) => view.scroll_down(len),
        (KeyCode::Char('k') | KeyCode::Up, _) => view.scroll_up(),
        (KeyCode::Char('g'), m) if !m.contains(KeyModifiers::SHIFT) => view.scroll_to_start(),
        (KeyCode::Char('G'), _) => view.scroll_to_end(len),
        _ => {}
    }
    Vec::new()
}
