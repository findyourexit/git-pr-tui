use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::data::models::PrId;

use super::super::{
    effect::Effect,
    state::{AppState, ComposerContext, ComposerState, DetailTab, FilesPane, Selection, View},
};
use super::{
    overlays::{open_close_reopen_confirm, open_merge_modal_for, open_review_modal},
    push_overlay_cue,
    write::start_checkout,
};

pub(super) fn handle_pr_detail_key(state: &mut AppState, k: &KeyEvent) -> Option<Vec<Effect>> {
    let View::PrDetail { id, tab } = state.current_view() else {
        return None;
    };
    let current_tab = tab;

    if let Some(effects) = handle_thread_navigation_key(state, k, &id) {
        return Some(effects);
    }

    // Route Files-sub-tab keys based on terminal width.
    if current_tab == DetailTab::Files && state.active_tab().state.diff_file.is_none() {
        if crate::app::render::is_files_split_width(state.viewport_width) {
            // ── Wide: split pane is visible; route by focused pane ──────────
            let files_focus = state.active_tab().state.files_focus;
            if files_focus == FilesPane::Diff {
                // Diff-focused: delegate diff keys to the shared core.
                if let Some(effects) = super::diff::handle_split_diff_key(state, &id, k) {
                    return Some(effects);
                }
                // Fall through for non-diff keys (e.g. `c`, `v`).
            } else {
                // Tree-focused: Enter/`l` focus the Diff pane.
                if k.code == KeyCode::Enter && k.modifiers == KeyModifiers::NONE {
                    let files = state.pr_files.get(&id)?;
                    if files.files.is_empty() {
                        return Some(Vec::new());
                    }
                    state.active_tab_mut().state.files_focus = FilesPane::Diff;
                    return Some(Vec::new());
                }
            }
        } else {
            // ── Narrow: single-focus list; Enter opens full-screen diff ─────
            if k.code == KeyCode::Enter && k.modifiers == KeyModifiers::NONE {
                let files = state.pr_files.get(&id)?;
                if files.files.is_empty() {
                    return Some(Vec::new());
                }
                let selected = match *state.selection() {
                    Selection::DetailFile(i) => i.min(files.files.len() - 1),
                    _ => 0,
                };
                state.active_tab_mut().state.diff_file = Some(selected);
                return Some(Vec::new());
            }
            // j/k navigation is handled by the shared cursor block below.
            // Do NOT route by files_focus on narrow terminals.
        }
    }

    // Legacy: Enter with diff_file open is a no-op here (Esc is handled by Cancel intent).
    if current_tab == DetailTab::Files
        && k.code == KeyCode::Enter
        && k.modifiers == KeyModifiers::NONE
        && state.active_tab().state.diff_file.is_some()
    {
        return Some(Vec::new());
    }

    if current_tab == DetailTab::Files
        && k.code == KeyCode::Char('L')
        && (k.modifiers == KeyModifiers::NONE || k.modifiers == KeyModifiers::SHIFT)
    {
        if !state.pr_files.contains_key(&id) {
            state.pr_files_load_requested.insert(id);
        }
        return Some(Vec::new());
    }

    if current_tab == DetailTab::Files
        && state.active_tab().state.diff_file.is_none()
        && state.active_tab().state.files_focus == FilesPane::Tree
        && matches!(
            (k.code, k.modifiers),
            (
                KeyCode::Char('j' | 'k') | KeyCode::Down | KeyCode::Up,
                KeyModifiers::NONE
            )
        )
        && let Some(effects) = handle_files_tab_cursor(state, k, &id)
    {
        return Some(effects);
    }
    // Also handle Tree-focused cursor when diff_file is Some (legacy full-screen path).
    if current_tab == DetailTab::Files
        && state.active_tab().state.diff_file.is_some()
        && matches!(
            (k.code, k.modifiers),
            (
                KeyCode::Char('j' | 'k') | KeyCode::Down | KeyCode::Up,
                KeyModifiers::NONE
            )
        )
        && let Some(effects) = handle_files_tab_cursor(state, k, &id)
    {
        return Some(effects);
    }

    if k.code == KeyCode::Char('c') && k.modifiers == KeyModifiers::NONE {
        state.composer = Some(ComposerState {
            body: String::new(),
            context: ComposerContext::PrComment { pr_id: id.clone() },
            head_moved: false,
            preview_visible: true,
        });
        push_overlay_cue(state);
        return Some(Vec::new());
    }

    if (k.modifiers == KeyModifiers::NONE
        || (k.code == KeyCode::Char('C') && k.modifiers == KeyModifiers::SHIFT))
        && let Some(effects) = handle_pr_detail_action_key(state, &id, k.code)
    {
        return Some(effects);
    }
    None
}

pub(super) fn cycle_tab_forward(tab: DetailTab) -> DetailTab {
    match tab {
        DetailTab::Conversation => DetailTab::Files,
        DetailTab::Files => DetailTab::Checks,
        DetailTab::Checks => DetailTab::Commits,
        DetailTab::Commits => DetailTab::Conversation,
    }
}

pub(super) fn cycle_tab_backward(tab: DetailTab) -> DetailTab {
    match tab {
        DetailTab::Conversation => DetailTab::Commits,
        DetailTab::Files => DetailTab::Conversation,
        DetailTab::Checks => DetailTab::Files,
        DetailTab::Commits => DetailTab::Checks,
    }
}

pub(super) fn handle_files_tab_cursor(
    state: &mut AppState,
    k: &KeyEvent,
    id: &PrId,
) -> Option<Vec<Effect>> {
    let files = state.pr_files.get(id)?;
    let file_count = files.files.len();
    if file_count == 0 {
        return Some(Vec::new());
    }
    let current = match *state.selection() {
        Selection::DetailFile(i) => i.min(file_count - 1),
        _ => 0,
    };
    let next = match (k.code, k.modifiers) {
        (KeyCode::Char('j') | KeyCode::Down, KeyModifiers::NONE) => {
            (current + 1).min(file_count - 1)
        }
        (KeyCode::Char('k') | KeyCode::Up, KeyModifiers::NONE) => current.saturating_sub(1),
        _ => return None,
    };
    *state.selection_mut() = Selection::DetailFile(next);

    let head_sha = state.pr_details.get(id).map(|d| d.head_sha.clone());
    let file = state.pr_files.get(id).and_then(|f| f.files.get(next))?;
    let path = file.path.clone();
    let needs_fetch = file.patch.is_none()
        && state.pr_files_load_requested.contains(id)
        && !state
            .pr_file_diff_inflight
            .contains(&(id.clone(), path.clone()));
    if let (Some(head_sha), true) = (head_sha, needs_fetch) {
        state
            .pr_file_diff_inflight
            .insert((id.clone(), path.clone()));
        return Some(vec![Effect::FetchPrFileDiff {
            id: id.clone(),
            head_sha,
            path,
        }]);
    }
    Some(Vec::new())
}

pub(super) fn handle_thread_navigation_key(
    state: &mut AppState,
    k: &KeyEvent,
    id: &PrId,
) -> Option<Vec<Effect>> {
    let detail = state.pr_details.get(id)?;
    let thread_count = detail.review_threads.len();
    if thread_count == 0 {
        return None;
    }
    match (k.code, k.modifiers) {
        (KeyCode::Char('n'), KeyModifiers::NONE) => {
            let next = match *state.selection() {
                Selection::ReviewThread { thread_index } => (thread_index + 1) % thread_count,
                _ => 0,
            };
            *state.selection_mut() = Selection::ReviewThread { thread_index: next };
            state.active_tab_mut().state.conversation_scroll_to_focused = true;
            Some(Vec::new())
        }
        (KeyCode::Char('N'), KeyModifiers::SHIFT) => {
            let prev = match *state.selection() {
                Selection::ReviewThread { thread_index } => {
                    if thread_index == 0 {
                        thread_count - 1
                    } else {
                        thread_index - 1
                    }
                }
                _ => thread_count - 1,
            };
            *state.selection_mut() = Selection::ReviewThread { thread_index: prev };
            state.active_tab_mut().state.conversation_scroll_to_focused = true;
            Some(Vec::new())
        }
        (KeyCode::Char('R'), KeyModifiers::SHIFT) => {
            let Selection::ReviewThread { thread_index } = *state.selection() else {
                return Some(Vec::new());
            };
            let detail = state.pr_details.get(id)?;
            let thread = detail.review_threads.get(thread_index)?;
            state.composer = Some(ComposerState {
                body: String::new(),
                context: ComposerContext::ReplyToThread {
                    pr_id: id.clone(),
                    thread_id: thread.id.clone(),
                },
                head_moved: false,
                preview_visible: true,
            });
            push_overlay_cue(state);
            Some(Vec::new())
        }
        _ => None,
    }
}

pub(super) fn handle_pr_detail_action_key(
    state: &mut AppState,
    id: &PrId,
    code: KeyCode,
) -> Option<Vec<Effect>> {
    match code {
        KeyCode::Char('v') => {
            open_review_modal(state, id);
            Some(Vec::new())
        }
        KeyCode::Char('m') => {
            open_merge_modal_for(state, id);
            Some(Vec::new())
        }
        KeyCode::Char('x') => {
            open_close_reopen_confirm(state, id);
            Some(Vec::new())
        }
        KeyCode::Char('C') => Some(start_checkout(state, id)),
        _ => None,
    }
}
