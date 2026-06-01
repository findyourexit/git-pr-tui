//! Action menu overlay — opened by `Space`.
//!
//! Lists the runnable [`ActionKind::Action`] entries from [`context_actions`]
//! for the current context, filtered by a type-to-filter input.  Arrow keys
//! move the highlight; `Enter` dispatches the selected action's intent; `Esc`
//! closes without running anything.

use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
};

use crate::app::actions::{Action, ActionKind, context_actions};
use crate::app::keymap::{Keymap, key_label};
use crate::app::state::AppState;
use crate::ui::components::input::{InputBuffer, render_input};
use crate::ui::palette::score;
use crate::ui::theme::Theme;

/// Persistent state for the action menu overlay.
#[derive(Debug, Default, Clone)]
pub struct ActionMenuState {
    pub input: InputBuffer,
    pub selected: usize,
}

// ── filtering ────────────────────────────────────────────────────────────────

/// Return the `kind == Action` actions for the current context, filtered by
/// the menu's input buffer (empty input → all actions).
#[must_use]
pub fn filtered_actions(state: &AppState) -> Vec<Action> {
    let all = context_actions(state);
    let needle = state
        .action_menu
        .as_ref()
        .map_or("", |m| m.input.text.as_str());

    all.into_iter()
        .filter(|a| a.kind == ActionKind::Action)
        .filter(|a| {
            if needle.is_empty() {
                true
            } else {
                score(needle, &a.label).is_some()
            }
        })
        .collect()
}

// ── rendering ─────────────────────────────────────────────────────────────────

pub fn render_action_menu(f: &mut Frame, area: Rect, state: &AppState, theme: &Theme) {
    let Some(menu) = state.action_menu.as_ref() else {
        return;
    };

    let actions = filtered_actions(state);

    // Popup size: fixed width + height proportional to action count.
    let width = 60u16.min(area.width.saturating_sub(4));
    let content_rows = u16::try_from(actions.len()).unwrap_or(u16::MAX).max(1);
    // input (3) + list rows + footer (1) + border (2)
    let height = (3 + content_rows + 1 + 2).min(area.height.saturating_sub(2));
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    let popup = Rect::new(x, y, width, height);

    f.render_widget(Clear, popup);

    let block = Block::default()
        .title(" Actions ")
        .borders(Borders::ALL)
        .border_type(theme.border_type())
        .style(Style::default().bg(theme.bg).fg(theme.fg))
        .border_style(Style::default().fg(theme.primary));
    f.render_widget(block, popup);

    let inner = Rect::new(
        popup.x + 1,
        popup.y + 1,
        popup.width.saturating_sub(2),
        popup.height.saturating_sub(2),
    );

    // Input line (3 rows tall, including its border).
    let input_area = Rect::new(inner.x, inner.y, inner.width, 3.min(inner.height));
    render_input(f, input_area, &menu.input, theme);

    if inner.height <= 3 {
        return;
    }

    // Footer hint line at the very bottom of inner area.
    let footer_y = inner.y + inner.height.saturating_sub(1);
    let footer_area = Rect::new(inner.x, footer_y, inner.width, 1);
    let footer = Paragraph::new(Line::from(vec![Span::styled(
        "type to filter · ↑↓ move · ↵ run · esc",
        Style::default().fg(theme.dim).add_modifier(Modifier::DIM),
    )]));
    f.render_widget(footer, footer_area);

    // List area between input and footer.
    let list_height = inner.height.saturating_sub(3).saturating_sub(1);
    if list_height == 0 {
        return;
    }
    let list_area = Rect::new(inner.x, inner.y + 3, inner.width, list_height);

    let selected = menu.selected.min(actions.len().saturating_sub(1));
    let items: Vec<ListItem> = actions
        .iter()
        .enumerate()
        .map(|(i, action)| {
            let key_hint = key_hint_for(action, &state.keymap);
            let row = format!("{key_hint:<6}{}", action.label);
            let style = if i == selected {
                Style::default().fg(theme.bg).bg(theme.primary)
            } else if action.danger {
                Style::default().fg(theme.error)
            } else {
                Style::default().fg(theme.fg)
            };
            ListItem::new(Line::from(Span::styled(row, style)))
        })
        .collect();

    let list = List::new(items);
    f.render_widget(list, list_area);
}

/// Format the key hint column for a row (e.g. `"m "`, `"— "`, `"C "` …).
fn key_hint_for(action: &Action, keymap: &Keymap) -> String {
    match keymap.key_for(&action.intent) {
        Some(k) => {
            format!("{} ", key_label(&k))
        }
        None => "— ".to_string(),
    }
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::intent::UserIntent;
    use crate::app::state::AppState;
    use crate::app::workspace::TabKind;
    use crate::data::models::{PrId, Repo};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn pr_state() -> AppState {
        let mut s = AppState::default();
        let pr_id = PrId {
            repo: Repo {
                owner: "o".into(),
                name: "r".into(),
            },
            number: 1,
        };
        s.workspace.open(TabKind::Pr(pr_id));
        s
    }

    #[test]
    fn open_action_menu_opens_menu() {
        let mut s = AppState::default();
        assert!(s.action_menu.is_none());
        s.action_menu = Some(ActionMenuState::default());
        assert!(s.action_menu.is_some());
    }

    #[test]
    fn filtered_actions_only_returns_action_kind() {
        let s = AppState {
            action_menu: Some(ActionMenuState::default()),
            ..Default::default()
        };
        let actions = filtered_actions(&s);
        for a in &actions {
            assert_eq!(
                a.kind,
                ActionKind::Action,
                "expected only Action kind, got {a:?}"
            );
        }
    }

    #[test]
    fn pr_context_menu_contains_expected_actions() {
        let mut s = pr_state();
        s.action_menu = Some(ActionMenuState::default());
        let actions = filtered_actions(&s);
        let intents: Vec<&UserIntent> = actions.iter().map(|a| &a.intent).collect();
        // merge, checkout, close are all present
        assert!(intents.contains(&&UserIntent::Merge), "Merge missing");
        assert!(
            intents.contains(&&UserIntent::CheckoutBranch),
            "Checkout missing"
        );
        assert!(
            intents.contains(&&UserIntent::ToggleClose),
            "ToggleClose missing"
        );
    }

    #[test]
    fn pr_context_close_is_danger() {
        let mut s = pr_state();
        s.action_menu = Some(ActionMenuState::default());
        let actions = filtered_actions(&s);
        let close = actions.iter().find(|a| a.intent == UserIntent::ToggleClose);
        assert!(close.is_some(), "ToggleClose not in menu");
        assert!(close.unwrap().danger, "ToggleClose should be danger");
    }

    #[test]
    fn filter_by_text_narrows_list() {
        let mut s = pr_state();
        let mut menu = ActionMenuState::default();
        menu.input.text = "mer".into();
        menu.input.cursor = 3;
        s.action_menu = Some(menu);
        let actions = filtered_actions(&s);
        // All returned actions should match "mer"
        for a in &actions {
            assert!(
                score("mer", &a.label).is_some(),
                "action '{}' did not match 'mer'",
                a.label
            );
        }
        // Merge should be in results
        let has_merge = actions.iter().any(|a| a.intent == UserIntent::Merge);
        assert!(has_merge, "Merge should match 'mer'");
    }

    #[test]
    fn snapshot_action_menu_pr_context() {
        let mut s = pr_state();
        s.action_menu = Some(ActionMenuState::default());
        let theme = crate::ui::theme::Theme::dark();
        let backend = TestBackend::new(80, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                render_action_menu(f, f.area(), &s, &theme);
            })
            .unwrap();
        let rendered = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol().to_string())
            .collect::<String>();
        // The popup title should be visible
        assert!(
            rendered.contains("Actions"),
            "popup title missing: {rendered}"
        );
        // Footer hint should be visible
        assert!(
            rendered.contains("type to filter"),
            "footer hint missing: {rendered}"
        );
    }
}
