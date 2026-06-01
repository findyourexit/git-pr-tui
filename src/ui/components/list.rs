use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    widgets::{List, ListItem, ListState, Scrollbar, ScrollbarOrientation, ScrollbarState},
};

use crate::ui::theme::Theme;

pub fn render_list(f: &mut Frame, area: Rect, items: &[String], selected: usize, theme: &Theme) {
    let mut state = ListState::default();
    state.select(Some(selected));

    let list_items: Vec<ListItem> = items
        .iter()
        .map(|i| ListItem::new(i.as_str()).style(Style::default().fg(theme.fg)))
        .collect();

    let list =
        List::new(list_items).highlight_style(Style::default().bg(theme.primary).fg(theme.bg));

    f.render_stateful_widget(list, area, &mut state);

    let items_len = items.len();
    let area_height = usize::from(area.height);

    if items_len > area_height {
        let mut scrollbar_state = ScrollbarState::default()
            .content_length(items_len)
            .position(selected);

        let scrollbar = Scrollbar::default()
            .orientation(ScrollbarOrientation::VerticalRight)
            .begin_symbol(None)
            .end_symbol(None);

        f.render_stateful_widget(scrollbar, area, &mut scrollbar_state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    #[test]
    fn renders_ten_items_with_scrollbar() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let items: Vec<String> = (1..=10).map(|i| format!("Item {i}")).collect();
        terminal
            .draw(|f| {
                let mut area = f.area();
                area.height = 6;
                render_list(f, area, &items, 2, &crate::ui::theme::Theme::dark());
            })
            .unwrap();
        insta::assert_snapshot!("renders_ten_items_with_scrollbar", terminal.backend());
    }
}
