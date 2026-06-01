use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    widgets::{Paragraph, Tabs},
};

use crate::ui::theme::Theme;

pub fn render_tabs(
    f: &mut Frame,
    area: Rect,
    labels: &[&str],
    selected: usize,
    show_hint: bool,
    theme: &Theme,
) {
    let sum_of_labels: u16 = labels
        .iter()
        .map(|l| u16::try_from(l.len()).unwrap_or(u16::MAX).saturating_add(3))
        .sum();

    let show_hint = show_hint && area.width > sum_of_labels.saturating_add(8);

    let tabs = Tabs::new(labels.iter().copied().map(String::from))
        .select(selected)
        .style(Style::default().bg(theme.bg).fg(theme.fg))
        .highlight_style(
            Style::default()
                .fg(theme.primary)
                .add_modifier(Modifier::BOLD),
        )
        .divider(" | ");

    if show_hint {
        let [tabs_area, hint_area] =
            Layout::horizontal([Constraint::Min(0), Constraint::Length(5)]).areas(area);

        f.render_widget(tabs, tabs_area);
        f.render_widget(
            Paragraph::new(" g t ").style(Style::default().bg(theme.bg).fg(theme.fg)),
            hint_area,
        );
    } else {
        f.render_widget(tabs, area);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    #[test]
    fn renders_three_tabs_second_selected() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let labels = ["Tab 1", "Tab 2", "Tab 3"];
        terminal
            .draw(|f| {
                render_tabs(
                    f,
                    f.area(),
                    &labels,
                    1,
                    true,
                    &crate::ui::theme::Theme::dark(),
                );
            })
            .unwrap();
        insta::assert_snapshot!("renders_three_tabs_second_selected", terminal.backend());
    }
}
