use ratatui::{
    Frame,
    layout::{Constraint, Rect},
    style::{Modifier, Style},
    widgets::{Block, Borders, Cell, Clear, Row, Table},
};

use crate::app::actions::context_actions;
use crate::app::keymap::key_label;
use crate::app::state::AppState;
use crate::ui::theme::Theme;

/// Render the `?` cheat-sheet overlay, driven entirely by the action registry
/// and the live keymap.  The table is rebuilt on every render so it can never
/// drift from the real bindings.
pub fn render_help_for_view(f: &mut Frame, area: Rect, state: &AppState, theme: &Theme) {
    let actions = context_actions(state);

    // Split into context-specific and global (nav/always-available) rows.
    let global_intents: &[crate::app::intent::UserIntent] = &[
        crate::app::intent::UserIntent::OpenActionMenu,
        crate::app::intent::UserIntent::EnterCommandPalette,
        crate::app::intent::UserIntent::EnterHelp,
        crate::app::intent::UserIntent::Refresh,
        crate::app::intent::UserIntent::Quit,
        crate::app::intent::UserIntent::CloseTab,
        crate::app::intent::UserIntent::NextTab,
        crate::app::intent::UserIntent::PrevTab,
        crate::app::intent::UserIntent::GotoDashboard,
    ];

    let mut context_rows: Vec<Row> = Vec::new();
    let mut global_rows: Vec<Row> = Vec::new();

    // Deduplicate: track which intents we've already emitted.
    let mut seen: Vec<crate::app::intent::UserIntent> = Vec::new();

    for action in &actions {
        if seen.contains(&action.intent) {
            continue;
        }
        seen.push(action.intent.clone());
        let key_str = state
            .keymap
            .key_for(&action.intent)
            .map_or_else(|| "—".to_string(), |k| key_label(&k));

        let key_style = if action.danger {
            Style::default()
                .fg(theme.error)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.primary)
        };
        let label_style = if action.danger {
            Style::default().fg(theme.error)
        } else {
            Style::default()
        };

        let row = Row::new(vec![
            Cell::from(key_str).style(key_style),
            Cell::from(action.label.clone()).style(label_style),
        ]);

        if global_intents.contains(&action.intent) {
            global_rows.push(row);
        } else {
            context_rows.push(row);
        }
    }

    // Determine a human-readable context title.
    let view_name = match state.current_view() {
        crate::app::state::View::Dashboard => "Dashboard",
        crate::app::state::View::PrList { .. } => "PR List",
        crate::app::state::View::PrDetail {
            tab: crate::app::state::DetailTab::Conversation,
            ..
        } => "PR · Conversation",
        crate::app::state::View::PrDetail {
            tab: crate::app::state::DetailTab::Files,
            ..
        } => "PR · Files",
        crate::app::state::View::PrDetail {
            tab: crate::app::state::DetailTab::Checks,
            ..
        } => "PR · Checks",
        crate::app::state::View::PrDetail {
            tab: crate::app::state::DetailTab::Commits,
            ..
        } => "PR · Commits",
        crate::app::state::View::Diff { .. } => "Diff Viewer",
    };

    let title = format!(" Help — {view_name} ");

    // Build separator rows between sections.
    let sep = Row::new(vec![Cell::from(""), Cell::from("── Global ──")])
        .style(Style::default().add_modifier(Modifier::DIM));

    let mut all_rows = context_rows;
    if !global_rows.is_empty() {
        all_rows.push(sep);
        all_rows.extend(global_rows);
    }

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_type(theme.border_type())
        .style(Style::default().bg(theme.bg).fg(theme.fg))
        .border_style(Style::default().fg(theme.primary));

    let table = Table::new(all_rows, [Constraint::Length(10), Constraint::Min(0)]).block(block);

    f.render_widget(Clear, area);
    f.render_widget(table, area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::state::AppState;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn make_state() -> AppState {
        AppState::default()
    }

    /// The cheat-sheet for the default (Dashboard) view renders without
    /// panicking and includes both context and global entries.
    #[test]
    fn renders_help_overlay_snapshot() {
        let backend = TestBackend::new(80, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        let state = make_state();
        let theme = crate::ui::theme::Theme::dark();
        terminal
            .draw(|f| {
                render_help_for_view(f, f.area(), &state, &theme);
            })
            .unwrap();
        insta::assert_snapshot!("renders_help_overlay", terminal.backend());
    }

    /// Anti-drift proof: remap a key via `apply_overrides` and assert that
    /// the cheat-sheet reflects the NEW binding, not any hardcoded old one.
    #[test]
    fn cheatsheet_reflects_keymap_overrides() {
        use std::collections::HashMap;

        // Render-to-text helper (declared first to satisfy `items_after_statements`).
        fn render_text(state: &AppState, theme: &crate::ui::theme::Theme) -> String {
            let backend = TestBackend::new(80, 30);
            let mut terminal = Terminal::new(backend).unwrap();
            terminal
                .draw(|f| render_help_for_view(f, f.area(), state, theme))
                .unwrap();
            let buf = terminal.backend().buffer().clone();
            (0..buf.area.height)
                .flat_map(|y| (0..buf.area.width).map(move |x| (x, y)))
                .map(|(x, y)| buf[(x, y)].symbol().to_string())
                .collect()
        }

        let theme = crate::ui::theme::Theme::dark();

        // Use a distinctive override key whose label ("F8") cannot appear
        // incidentally in any action label (unlike a bare 'F', which collides
        // with words like "Filter"). This makes the assertion specific.
        let default_state = make_state();
        assert!(
            !render_text(&default_state, &theme).contains("F8"),
            "sanity: the default cheat-sheet must NOT contain 'F8'"
        );

        let mut state = make_state();
        assert!(
            state
                .keymap
                .key_for(&crate::app::intent::UserIntent::Refresh)
                .is_some(),
            "Refresh should have a default binding"
        );
        // apply_overrides maps intent_name -> key_spec.
        let mut overrides = HashMap::new();
        overrides.insert("refresh".to_string(), "F8".to_string());
        state.keymap.apply_overrides(&overrides);

        // The generated cheat-sheet must now show the NEW key for Refresh,
        // proving it is derived from the live keymap (not hardcoded).
        let rendered = render_text(&state, &theme);
        assert!(
            rendered.contains("F8"),
            "cheat-sheet must show the overridden Refresh key 'F8'; rendered:\n{rendered}"
        );
    }

    /// For a PR·Files diff context the cheat-sheet includes diff-related
    /// actions with their actual bound keys.
    #[test]
    fn pr_files_diff_cheatsheet_has_diff_actions() {
        use crate::data::models::{PrId, Repo};

        let repo = Repo {
            owner: "acme".into(),
            name: "widgets".into(),
        };
        let id = PrId { repo, number: 1 };

        let mut state = make_state();
        state.workspace.open(crate::app::workspace::TabKind::Pr(id));
        {
            let tab = state.active_tab_mut();
            tab.state.detail_tab = crate::app::state::DetailTab::Files;
            tab.state.files_focus = crate::app::state::FilesPane::Diff;
        }

        let actions = context_actions(&state);
        // Toggle side-by-side and Toggle whitespace must be present.
        assert!(
            actions
                .iter()
                .any(|a| a.intent == crate::app::intent::UserIntent::ToggleSideBySide),
            "diff context must include ToggleSideBySide"
        );
        assert!(
            actions
                .iter()
                .any(|a| a.intent == crate::app::intent::UserIntent::ToggleWhitespace),
            "diff context must include ToggleWhitespace"
        );

        // Render the cheat-sheet — must not panic.
        let backend = TestBackend::new(80, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        let theme = crate::ui::theme::Theme::dark();
        terminal
            .draw(|f| {
                render_help_for_view(f, f.area(), &state, &theme);
            })
            .unwrap();

        let buf = terminal.backend().buffer().clone();
        let rendered: String = (0..buf.area.height)
            .flat_map(|y| (0..buf.area.width).map(move |x| (x, y)))
            .map(|(x, y)| buf[(x, y)].symbol().to_string())
            .collect();
        assert!(
            rendered.contains("Toggle"),
            "diff cheat-sheet must mention Toggle actions; got:\n{rendered}"
        );
    }
}
