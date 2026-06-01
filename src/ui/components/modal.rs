use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    widgets::{Block, Borders, Clear, Paragraph},
};

use crate::ui::layout::LayoutMode;
use crate::ui::theme::Theme;

pub fn render_modal(
    f: &mut Frame,
    area: Rect,
    title: &str,
    body: &str,
    mode: LayoutMode,
    theme: &Theme,
) {
    let modal_area = if matches!(mode, LayoutMode::FullScreenModal) {
        area
    } else {
        let width = (area.width * 60) / 100;
        let height = (area.height * 40) / 100;
        let x = area.x + (area.width.saturating_sub(width)) / 2;
        let y = area.y + (area.height.saturating_sub(height)) / 2;
        Rect::new(x, y, width, height)
    };

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_type(theme.border_type())
        .style(Style::default().bg(theme.bg).fg(theme.fg))
        .border_style(Style::default().fg(theme.primary));

    f.render_widget(Clear, modal_area);
    f.render_widget(Paragraph::new(body).block(block), modal_area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    #[test]
    fn renders_centered_in_full() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                render_modal(
                    f,
                    f.area(),
                    "Title",
                    "Body content",
                    LayoutMode::Full,
                    &crate::ui::theme::Theme::dark(),
                );
            })
            .unwrap();
        insta::assert_snapshot!("renders_centered_in_full", terminal.backend());
    }

    #[test]
    fn renders_fullscreen_in_modal_mode() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                render_modal(
                    f,
                    f.area(),
                    "Title",
                    "Body content",
                    LayoutMode::FullScreenModal,
                    &crate::ui::theme::Theme::dark(),
                );
            })
            .unwrap();
        insta::assert_snapshot!("renders_fullscreen_in_modal_mode", terminal.backend());
    }

    #[test]
    fn small_terminal_picks_fullscreen_modal_and_fills_area() {
        use crate::ui::layout::min_size_check;
        let backend = TestBackend::new(50, 16);
        let mut terminal = Terminal::new(backend).unwrap();
        let mode = min_size_check(50, 16);
        assert_eq!(mode, LayoutMode::FullScreenModal);

        terminal
            .draw(|f| {
                let size = f.area();
                render_modal(
                    f,
                    size,
                    "Submit Review",
                    "Body",
                    mode,
                    &crate::ui::theme::Theme::dark(),
                );
            })
            .unwrap();

        let backend_ref = terminal.backend();
        let buf = backend_ref.buffer();
        assert_eq!(buf[(0, 0)].symbol(), "╭");
        assert_eq!(buf[(49, 0)].symbol(), "╮");
        assert_eq!(buf[(0, 15)].symbol(), "╰");
        assert_eq!(buf[(49, 15)].symbol(), "╯");

        insta::assert_snapshot!(
            "small_terminal_picks_fullscreen_modal_and_fills_area",
            terminal.backend()
        );
    }
}
