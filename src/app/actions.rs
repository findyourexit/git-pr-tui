//! Action registry — the single source of truth for every discoverable
//! action in the application.
//!
//! [`context_actions`] is a pure function: given the current [`AppState`]
//! it returns the complete list of actions that make sense right now. The
//! footer, action menu, command palette, and cheat-sheet all consume this
//! list so they can never disagree with each other or with the real keymap.

use crate::app::intent::UserIntent;
use crate::app::state::{AppState, DetailTab, FilesPane, View};

/// Broad category used to group actions in menus and the palette.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionKind {
    /// A runnable operation (open, merge, comment, …).
    Action,
    /// A toggle or preference (whitespace, side-by-side diff, …).
    Setting,
    /// Navigation (tabs, dashboard, …).
    Nav,
}

/// A single discoverable action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Action {
    /// Short human-readable label shown in hints, menus, and the palette.
    pub label: String,
    /// The intent dispatched when the action is run.
    pub intent: UserIntent,
    pub kind: ActionKind,
    /// `true` for destructive / irreversible operations (e.g. close a PR).
    pub danger: bool,
    /// Footer priority.  `Some(p)` → show in the persistent footer row at
    /// priority `p` (lower value = leftmost / most prominent).  `None` →
    /// available in menus / palette only.
    pub footer: Option<u8>,
}

impl Action {
    fn new(
        label: &str,
        intent: UserIntent,
        kind: ActionKind,
        danger: bool,
        footer: Option<u8>,
    ) -> Self {
        Self {
            label: label.to_string(),
            intent,
            kind,
            danger,
            footer,
        }
    }
}

// ── helpers ──────────────────────────────────────────────────────────────────

fn action(label: &str, intent: UserIntent, footer: Option<u8>) -> Action {
    Action::new(label, intent, ActionKind::Action, false, footer)
}

fn setting(label: &str, intent: UserIntent, footer: Option<u8>) -> Action {
    Action::new(label, intent, ActionKind::Setting, false, footer)
}

fn nav(label: &str, intent: UserIntent, footer: Option<u8>) -> Action {
    Action::new(label, intent, ActionKind::Nav, false, footer)
}

fn danger_action(label: &str, intent: UserIntent, footer: Option<u8>) -> Action {
    Action::new(label, intent, ActionKind::Action, true, footer)
}

// ── global actions (present in every context) ─────────────────────────────

fn global_actions() -> Vec<Action> {
    vec![
        action("Actions", UserIntent::OpenActionMenu, Some(20)),
        nav("Command palette", UserIntent::EnterCommandPalette, Some(21)),
        nav("Help / keys", UserIntent::EnterHelp, Some(22)),
        action("Refresh", UserIntent::Refresh, None),
        action("Quit", UserIntent::Quit, None),
        nav("Close tab", UserIntent::CloseTab, None),
        nav("Next tab", UserIntent::NextTab, None),
        nav("Prev tab", UserIntent::PrevTab, None),
        nav("Dashboard", UserIntent::GotoDashboard, None),
    ]
}

// ── per-context actions ───────────────────────────────────────────────────

fn dashboard_actions() -> Vec<Action> {
    let mut v = vec![
        action("Open", UserIntent::OpenSelected, Some(1)),
        action("Refresh", UserIntent::Refresh, Some(2)),
        action("Filter", UserIntent::OpenFilterModal, Some(3)),
        action("Sort", UserIntent::OpenSortModal, Some(4)),
    ];
    v.extend(global_actions());
    v
}

fn pr_list_actions() -> Vec<Action> {
    let mut v = vec![
        action("Open PR", UserIntent::OpenSelected, Some(1)),
        action("Search", UserIntent::OpenSearch, Some(2)),
        action("Filter", UserIntent::OpenFilterModal, Some(3)),
        action("Sort", UserIntent::OpenSortModal, Some(4)),
        action("Open in browser", UserIntent::OpenInBrowser, Some(5)),
        action("Refresh", UserIntent::Refresh, Some(6)),
    ];
    v.extend(global_actions());
    v
}

fn pr_conversation_actions() -> Vec<Action> {
    let mut v = vec![
        action("Open thread", UserIntent::OpenSelected, Some(1)),
        action("Comment", UserIntent::StartComment, Some(2)),
        action("Reply to thread", UserIntent::StartReplyToThread, Some(3)),
        action("Start review", UserIntent::StartReview, Some(4)),
        action("Merge", UserIntent::Merge, Some(5)),
        danger_action("Close / reopen PR", UserIntent::ToggleClose, Some(6)),
        action("Checkout branch", UserIntent::CheckoutBranch, None),
        action("Open in browser", UserIntent::OpenInBrowser, None),
        action("Refresh", UserIntent::Refresh, None),
    ];
    v.extend(global_actions());
    v
}

fn pr_files_tree_actions() -> Vec<Action> {
    let mut v = vec![
        action("Open file", UserIntent::OpenSelected, Some(1)),
        action("Start review", UserIntent::StartReview, Some(2)),
        action("Merge", UserIntent::Merge, Some(3)),
        action("Open in browser", UserIntent::OpenInBrowser, Some(4)),
        // ToggleSideBySide has no default vim_defaults binding; key_for
        // returns None — consumers must render "—" or omit the key column.
        setting("Toggle side-by-side", UserIntent::ToggleSideBySide, None),
        setting("Toggle whitespace", UserIntent::ToggleWhitespace, None),
    ];
    v.extend(global_actions());
    v
}

fn pr_files_diff_actions() -> Vec<Action> {
    let mut v = vec![
        // StartLineComment has no default vim_defaults binding; key_for
        // returns None — consumers must render "—" or omit the key column.
        action("Comment on line", UserIntent::StartLineComment, Some(1)),
        action("Start review", UserIntent::StartReview, Some(2)),
        // ToggleSideBySide has no default vim_defaults binding — same as above.
        setting("Toggle side-by-side", UserIntent::ToggleSideBySide, Some(3)),
        setting("Toggle whitespace", UserIntent::ToggleWhitespace, Some(4)),
        action("Merge", UserIntent::Merge, None),
        action("Open in browser", UserIntent::OpenInBrowser, None),
    ];
    v.extend(global_actions());
    v
}

fn pr_checks_actions() -> Vec<Action> {
    let mut v = vec![
        action("Open in browser", UserIntent::OpenInBrowser, Some(1)),
        action("Refresh", UserIntent::Refresh, Some(2)),
        action("Merge", UserIntent::Merge, Some(3)),
    ];
    v.extend(global_actions());
    v
}

fn pr_commits_actions() -> Vec<Action> {
    let mut v = vec![
        action("Open in browser", UserIntent::OpenInBrowser, Some(1)),
        action("Refresh", UserIntent::Refresh, Some(2)),
        action("Merge", UserIntent::Merge, Some(3)),
    ];
    v.extend(global_actions());
    v
}

// ── public API ────────────────────────────────────────────────────────────

/// Returns all actions available in the current context.
///
/// This is the **single source of truth** consumed by the footer, action
/// menu, command palette, and cheat-sheet. It is pure (no side-effects) and
/// cheap to call on every frame.
#[must_use]
pub fn context_actions(state: &AppState) -> Vec<Action> {
    match state.current_view() {
        View::Dashboard => dashboard_actions(),
        View::PrList { .. } => pr_list_actions(),
        View::PrDetail { tab, .. } => match tab {
            DetailTab::Conversation => pr_conversation_actions(),
            DetailTab::Files => {
                let files_focus = state.active_tab().state.files_focus;
                match files_focus {
                    FilesPane::Diff => pr_files_diff_actions(),
                    FilesPane::Tree => pr_files_tree_actions(),
                }
            }
            DetailTab::Checks => pr_checks_actions(),
            DetailTab::Commits => pr_commits_actions(),
        },
        View::Diff { .. } => pr_files_diff_actions(),
    }
}

// ── tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::state::AppState;
    use crate::app::workspace::TabKind;
    use crate::data::models::{PrId, Repo};

    fn has_intent(actions: &[Action], intent: &UserIntent) -> bool {
        actions.iter().any(|a| &a.intent == intent)
    }

    fn footer_actions(actions: &[Action]) -> Vec<&Action> {
        actions.iter().filter(|a| a.footer.is_some()).collect()
    }

    fn make_repo() -> Repo {
        Repo {
            owner: "acme".to_string(),
            name: "widget".to_string(),
        }
    }

    #[test]
    fn dashboard_has_expected_actions_and_non_empty_footer() {
        let state = AppState::default();
        // Default state is Dashboard.
        let actions = context_actions(&state);
        assert!(has_intent(&actions, &UserIntent::OpenSelected));
        assert!(has_intent(&actions, &UserIntent::Refresh));
        assert!(has_intent(&actions, &UserIntent::OpenFilterModal));
        assert!(has_intent(&actions, &UserIntent::OpenSortModal));
        assert!(has_intent(&actions, &UserIntent::OpenActionMenu));
        assert!(
            !footer_actions(&actions).is_empty(),
            "footer must be non-empty"
        );
    }

    #[test]
    fn pr_list_has_search_and_non_empty_footer() {
        let mut state = AppState::default();
        state.workspace.open(TabKind::Repo(make_repo()));
        let actions = context_actions(&state);
        assert!(has_intent(&actions, &UserIntent::OpenSearch));
        assert!(has_intent(&actions, &UserIntent::OpenSelected));
        assert!(!footer_actions(&actions).is_empty());
    }

    #[test]
    fn pr_conversation_flags_toggle_close_as_danger() {
        let mut state = AppState::default();
        let pr_id = PrId {
            repo: make_repo(),
            number: 1,
        };
        state.workspace.open(TabKind::Pr(pr_id));
        // detail_tab defaults to Conversation.
        let actions = context_actions(&state);
        assert!(has_intent(&actions, &UserIntent::ToggleClose));
        let close = actions
            .iter()
            .find(|a| a.intent == UserIntent::ToggleClose)
            .expect("ToggleClose must be present");
        assert!(close.danger, "ToggleClose must be flagged danger = true");
        assert!(has_intent(&actions, &UserIntent::StartComment));
        assert!(has_intent(&actions, &UserIntent::Merge));
        assert!(!footer_actions(&actions).is_empty());
    }

    #[test]
    fn pr_files_tree_focus_includes_open_and_review() {
        let mut state = AppState::default();
        let pr_id = PrId {
            repo: make_repo(),
            number: 2,
        };
        state.workspace.open(TabKind::Pr(pr_id));
        {
            let tab = state.active_tab_mut();
            tab.state.detail_tab = DetailTab::Files;
            tab.state.files_focus = FilesPane::Tree;
        }
        let actions = context_actions(&state);
        assert!(has_intent(&actions, &UserIntent::OpenSelected));
        assert!(has_intent(&actions, &UserIntent::StartReview));
        assert!(has_intent(&actions, &UserIntent::ToggleSideBySide));
        assert!(!footer_actions(&actions).is_empty());
        // No danger actions for tree focus.
        assert!(!actions.iter().any(|a| a.danger));
    }

    #[test]
    fn pr_files_diff_focus_includes_line_comment_and_side_by_side() {
        let mut state = AppState::default();
        let pr_id = PrId {
            repo: make_repo(),
            number: 3,
        };
        state.workspace.open(TabKind::Pr(pr_id));
        {
            let tab = state.active_tab_mut();
            tab.state.detail_tab = DetailTab::Files;
            tab.state.files_focus = FilesPane::Diff;
        }
        let actions = context_actions(&state);
        assert!(has_intent(&actions, &UserIntent::StartLineComment));
        assert!(has_intent(&actions, &UserIntent::ToggleSideBySide));
        assert!(has_intent(&actions, &UserIntent::ToggleWhitespace));
        assert!(has_intent(&actions, &UserIntent::StartReview));
        assert!(!footer_actions(&actions).is_empty());
    }

    #[test]
    fn context_actions_never_panics_and_has_footer_across_all_contexts() {
        // Smoke test: every view / PR sub-tab / pane-focus combination yields a
        // non-empty action set with a non-empty footer subset, and never panics.
        let mut states: Vec<AppState> = Vec::new();

        // Dashboard.
        states.push(AppState::default());

        // Repo list.
        {
            let mut s = AppState::default();
            s.workspace.open(TabKind::Repo(make_repo()));
            states.push(s);
        }

        // PR tab: every sub-tab, and both pane foci for Files.
        for tab in [
            DetailTab::Conversation,
            DetailTab::Files,
            DetailTab::Checks,
            DetailTab::Commits,
        ] {
            for focus in [FilesPane::Tree, FilesPane::Diff] {
                let mut s = AppState::default();
                s.workspace.open(TabKind::Pr(PrId {
                    repo: make_repo(),
                    number: 7,
                }));
                {
                    let t = s.active_tab_mut();
                    t.state.detail_tab = tab;
                    t.state.files_focus = focus;
                }
                states.push(s);
            }
        }

        for s in &states {
            let actions = context_actions(s); // must not panic
            assert!(
                !actions.is_empty(),
                "context_actions empty for view {:?}",
                s.current_view()
            );
            assert!(
                !footer_actions(&actions).is_empty(),
                "footer subset empty for view {:?}",
                s.current_view()
            );
        }
    }
}
