//! Reducer handlers for workspace tab intents (spec §3.2, §7.1).
use crate::app::anim::{AnimationCue, TabSwitchDir, cue_allowed};
use crate::app::effect::Effect;
use crate::app::state::{AppState, Toast, ToastKind};
use std::time::Instant;

pub(super) fn next_tab(state: &mut AppState) -> Vec<Effect> {
    let before = state.workspace.active;
    state.workspace.next();
    if state.workspace.active != before {
        push_tab_cue(state, TabSwitchDir::Right);
    }
    Vec::new()
}

pub(super) fn prev_tab(state: &mut AppState) -> Vec<Effect> {
    let before = state.workspace.active;
    state.workspace.prev();
    if state.workspace.active != before {
        push_tab_cue(state, TabSwitchDir::Left);
    }
    Vec::new()
}

/// 1-based key → 0-based index.
pub(super) fn goto_tab(state: &mut AppState, n: u8) -> Vec<Effect> {
    let before = state.workspace.active;
    state.workspace.goto(usize::from(n).saturating_sub(1));
    let after = state.workspace.active;
    if after != before {
        let dir = if after > before {
            TabSwitchDir::Right
        } else {
            TabSwitchDir::Left
        };
        push_tab_cue(state, dir);
    }
    Vec::new()
}

pub(super) fn goto_dashboard(state: &mut AppState) -> Vec<Effect> {
    let before = state.workspace.active;
    state.workspace.goto(0);
    if state.workspace.active != before {
        push_tab_cue(state, TabSwitchDir::Left);
    }
    Vec::new()
}

/// Push a `TabSwitch` cue if the current intensity allows it.
pub(super) fn push_tab_cue(state: &mut AppState, dir: TabSwitchDir) {
    let cue = AnimationCue::TabSwitch { dir };
    if cue_allowed(state.animations, &cue) {
        state.animation_cues.push(cue);
    }
}

pub(super) fn close_tab(state: &mut AppState) -> Vec<Effect> {
    if !state.workspace.close_active() {
        state.toast = Some(Toast {
            kind: ToastKind::Info,
            message: "Dashboard is pinned".into(),
            created_at: Instant::now(),
        });
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use crate::app::state::AppState;
    use crate::app::workspace::TabKind;
    use crate::data::models::Repo;

    #[test]
    fn next_tab_advances_active() {
        let mut s = AppState::default();
        s.workspace.open(TabKind::Repo(Repo {
            owner: "a".into(),
            name: "b".into(),
        }));
        s.workspace.goto(0);
        super::next_tab(&mut s);
        assert_eq!(s.workspace.active, 1);
    }

    #[test]
    fn close_tab_on_dashboard_warns() {
        let mut s = AppState::default();
        let effects = super::close_tab(&mut s);
        assert!(effects.is_empty());
        assert!(s.toast.is_some(), "pinned-dashboard toast");
        assert_eq!(s.workspace.tabs.len(), 1);
    }
}
