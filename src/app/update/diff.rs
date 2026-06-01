use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::data::github::diff_position::DiffPositionIndex;
use crate::data::models::PrId;

use super::super::{
    anim::{AnimationCue, cue_allowed},
    effect::Effect,
    state::{AppState, ComposerContext, ComposerState, DiffSelection, Toast, ToastKind, View},
};
use super::push_overlay_cue;

/// Shared core: handles the diff-key arms that are identical between the
/// full-screen viewer and the split-layout diff pane.
///
/// Handles: `V` (start line selection), `j`/`k`/Down/Up (move `diff_cursor`),
/// `]`/`[` (hunk nav), `W`+SHIFT (toggle whitespace), `t` (toggle diff mode).
///
/// Returns `None` for any key it does not handle so the caller can fall through.
fn handle_diff_common_key(
    state: &mut AppState,
    id: &PrId,
    file_index: usize,
    k: &KeyEvent,
) -> Option<Vec<Effect>> {
    match (k.code, k.modifiers) {
        (KeyCode::Char('V'), KeyModifiers::SHIFT) => {
            let anchor = state.active_tab().state.diff_cursor;
            state.active_tab_mut().state.diff_selection = Some(DiffSelection::new(anchor));
            Some(Vec::new())
        }
        (KeyCode::Char('j') | KeyCode::Down, KeyModifiers::NONE) => {
            let patch = state
                .pr_files
                .get(id)
                .and_then(|f| f.files.get(file_index))
                .and_then(|f| f.patch.clone());
            let line_count = patch
                .as_deref()
                .map_or(0, |p| p.split_inclusive('\n').count());
            let cursor = &mut state.active_tab_mut().state.diff_cursor;
            if line_count > 0 && *cursor + 1 < line_count {
                *cursor += 1;
            }
            Some(Vec::new())
        }
        (KeyCode::Char('k') | KeyCode::Up, KeyModifiers::NONE) => {
            let cursor = &mut state.active_tab_mut().state.diff_cursor;
            *cursor = cursor.saturating_sub(1);
            Some(Vec::new())
        }
        (KeyCode::Char(']'), KeyModifiers::NONE) => {
            let patch = state
                .pr_files
                .get(id)
                .and_then(|f| f.files.get(file_index))
                .and_then(|f| f.patch.clone());
            let Some(patch) = patch.as_deref() else {
                return Some(Vec::new());
            };
            let index = DiffPositionIndex::parse(patch);
            let cursor_ref = &mut state.active_tab_mut().state.diff_cursor;
            if let Some(next) = index.next_hunk_after(*cursor_ref) {
                *cursor_ref = next;
            }
            Some(Vec::new())
        }
        (KeyCode::Char('['), KeyModifiers::NONE) => {
            let patch = state
                .pr_files
                .get(id)
                .and_then(|f| f.files.get(file_index))
                .and_then(|f| f.patch.clone());
            let Some(patch) = patch.as_deref() else {
                return Some(Vec::new());
            };
            let index = DiffPositionIndex::parse(patch);
            let cursor_ref = &mut state.active_tab_mut().state.diff_cursor;
            if let Some(prev) = index.prev_hunk_before(*cursor_ref) {
                *cursor_ref = prev;
            }
            Some(Vec::new())
        }
        (KeyCode::Char('W'), KeyModifiers::SHIFT) => {
            let flag = &mut state.active_tab_mut().state.diff_whitespace_hidden;
            *flag = !*flag;
            Some(Vec::new())
        }
        (KeyCode::Char('t'), KeyModifiers::NONE) => {
            toggle_diff_mode(state);
            Some(Vec::new())
        }
        _ => None,
    }
}

pub(super) fn handle_diff_viewer_key(state: &mut AppState, k: &KeyEvent) -> Option<Vec<Effect>> {
    let View::Diff { id, file_index } = state.current_view() else {
        return None;
    };

    if state.active_tab().state.diff_selection.is_some() {
        return handle_diff_selection_key(state, &id, file_index, k);
    }

    if k.code == KeyCode::Esc && k.modifiers == KeyModifiers::NONE {
        state.active_tab_mut().state.diff_file = None;
        state.active_tab_mut().state.detail_tab = crate::app::state::DetailTab::Files;
        return Some(Vec::new());
    }

    let files = state.pr_files.get(&id)?;
    if files.files.is_empty() {
        return Some(Vec::new());
    }
    let file_count = files.files.len();

    match (k.code, k.modifiers) {
        // full-screen wraps; split clamps
        (KeyCode::Char('}'), KeyModifiers::NONE) => {
            let next = (file_index + 1) % file_count;
            state.active_tab_mut().state.diff_file = Some(next);
            state.active_tab_mut().state.diff_cursor = 0;
            Some(Vec::new())
        }
        // full-screen wraps; split clamps
        (KeyCode::Char('{'), KeyModifiers::NONE) => {
            let prev = if file_index == 0 {
                file_count - 1
            } else {
                file_index - 1
            };
            state.active_tab_mut().state.diff_file = Some(prev);
            state.active_tab_mut().state.diff_cursor = 0;
            Some(Vec::new())
        }
        _ => handle_diff_common_key(state, &id, file_index, k),
    }
}

pub(super) fn toggle_diff_mode(state: &mut AppState) {
    use crate::app::state::DiffMode;
    let mode = &mut state.active_tab_mut().state.diff_mode;
    *mode = match *mode {
        DiffMode::Unified => DiffMode::SideBySide,
        DiffMode::SideBySide => DiffMode::Unified,
    };
}

pub(super) fn handle_diff_selection_key(
    state: &mut AppState,
    id: &PrId,
    file_index: usize,
    k: &KeyEvent,
) -> Option<Vec<Effect>> {
    let files = state.pr_files.get(id)?;
    let file = files.files.get(file_index)?;
    let line_count = file
        .patch
        .as_deref()
        .map_or(0, |p| p.split_inclusive('\n').count());

    match (k.code, k.modifiers) {
        (KeyCode::Esc, KeyModifiers::NONE) => {
            state.active_tab_mut().state.diff_selection = None;
            Some(Vec::new())
        }
        (KeyCode::Char('j') | KeyCode::Down, KeyModifiers::NONE) => {
            if let Some(sel) = state.active_tab_mut().state.diff_selection.as_mut()
                && line_count > 0
                && sel.cursor + 1 < line_count
            {
                sel.cursor += 1;
            }
            Some(Vec::new())
        }
        (KeyCode::Char('k') | KeyCode::Up, KeyModifiers::NONE) => {
            if let Some(sel) = state.active_tab_mut().state.diff_selection.as_mut() {
                sel.cursor = sel.cursor.saturating_sub(1);
            }
            Some(Vec::new())
        }
        (KeyCode::Char('c'), KeyModifiers::NONE) => {
            let Some(sel) = state.active_tab().state.diff_selection else {
                return Some(Vec::new());
            };
            let Some(patch) = file.patch.as_deref() else {
                return Some(Vec::new());
            };
            let (lo, hi) = sel.range();
            let index = DiffPositionIndex::parse(patch);
            let Some(comment) = index.build_line_comment(lo, hi) else {
                state.toast = Some(Toast {
                    kind: ToastKind::Warning,
                    message: "Select at least one diff line to comment on".into(),
                    created_at: std::time::Instant::now(),
                });
                return Some(Vec::new());
            };
            let head_sha = state
                .pr_details
                .get(id)
                .map(|d| d.head_sha.clone())
                .unwrap_or_default();
            state.active_tab_mut().state.diff_selection = None;
            state.composer = Some(ComposerState {
                body: String::new(),
                context: ComposerContext::LineComment {
                    pr_id: id.clone(),
                    head_sha,
                    comment,
                },
                head_moved: false,
                preview_visible: true,
            });
            push_overlay_cue(state);
            Some(Vec::new())
        }
        _ => Some(Vec::new()),
    }
}

/// Handle diff-viewer keys when the Diff pane is focused in the split layout.
/// Returns `None` if the key is not consumed (caller may fall through).
/// Returns `Some(effects)` if the key was handled.
pub(super) fn handle_split_diff_key(
    state: &mut AppState,
    id: &crate::data::models::PrId,
    k: &crossterm::event::KeyEvent,
) -> Option<Vec<Effect>> {
    use crossterm::event::{KeyCode, KeyModifiers};

    // Determine file index from the current DetailFile selection.
    let file_index = match *state.selection() {
        crate::app::state::Selection::DetailFile(i) => i,
        _ => 0,
    };

    // If line-selection mode is active, delegate to its handler.
    if state.active_tab().state.diff_selection.is_some() {
        return handle_diff_selection_key(state, id, file_index, k);
    }

    let files = state.pr_files.get(id)?;
    if files.files.is_empty() {
        return None;
    }
    let file_count = files.files.len();

    match (k.code, k.modifiers) {
        // full-screen wraps; split clamps
        (KeyCode::Char('}'), KeyModifiers::NONE) => {
            let next = (file_index + 1).min(file_count - 1);
            *state.selection_mut() = crate::app::state::Selection::DetailFile(next);
            state.active_tab_mut().state.diff_cursor = 0;
            Some(Vec::new())
        }
        // full-screen wraps; split clamps
        (KeyCode::Char('{'), KeyModifiers::NONE) => {
            let prev = file_index.saturating_sub(1);
            *state.selection_mut() = crate::app::state::Selection::DetailFile(prev);
            state.active_tab_mut().state.diff_cursor = 0;
            Some(Vec::new())
        }
        _ => handle_diff_common_key(state, id, file_index, k),
    }
}

/// Set the focused pane in the Files split. Only acts when the active tab's
/// `detail_tab` is `DetailTab::Files`; otherwise this is a no-op.
pub(super) fn focus_pane(state: &mut AppState, pane: crate::app::state::FilesPane) -> Vec<Effect> {
    use crate::app::state::DetailTab;
    // No-op on narrow terminals: the split pane isn't rendered so focus
    // changes via h/l would only confuse the key router.
    if state.active_tab().state.detail_tab == DetailTab::Files
        && crate::app::render::is_files_split_width(state.viewport_width)
    {
        let before = state.active_tab().state.files_focus;
        state.active_tab_mut().state.files_focus = pane;
        if state.active_tab().state.files_focus != before {
            let cue = AnimationCue::FocusChange;
            if cue_allowed(state.animations, &cue) {
                state.animation_cues.push(cue);
            }
        }
    }
    Vec::new()
}
