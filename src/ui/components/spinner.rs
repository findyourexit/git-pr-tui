use ratatui::{Frame, layout::Rect, style::Style, widgets::Paragraph};

use crate::ui::theme::Theme;

const FRAMES: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

#[must_use]
pub fn frame_for(tick: u64) -> char {
    let len = u64::try_from(FRAMES.len()).unwrap_or(10);
    let idx = usize::try_from(tick % len).unwrap_or(0);
    FRAMES[idx]
}

pub fn render_spinner(f: &mut Frame, area: Rect, tick: u64, label: &str, theme: &Theme) {
    let text = format!("{} {}", frame_for(tick), label);
    let style = Style::default().fg(theme.primary).bg(theme.bg);
    f.render_widget(Paragraph::new(text).style(style), area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    #[test]
    fn frame_cycles() {
        assert_eq!(frame_for(0), '⠋');
        assert_eq!(frame_for(1), '⠙');
        assert_eq!(frame_for(10), '⠋');
    }

    fn render_test_spinner(tick: u64, terminal: &mut Terminal<TestBackend>) {
        terminal
            .draw(|f| {
                render_spinner(
                    f,
                    f.area(),
                    tick,
                    "Loading...",
                    &crate::ui::theme::Theme::dark(),
                );
            })
            .unwrap();
    }

    #[test]
    fn renders_tick_0() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        render_test_spinner(0, &mut terminal);
        insta::assert_snapshot!("renders_tick_0", terminal.backend());
    }

    #[test]
    fn renders_tick_3() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        render_test_spinner(3, &mut terminal);
        insta::assert_snapshot!("renders_tick_3", terminal.backend());
    }

    #[test]
    fn renders_tick_9() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        render_test_spinner(9, &mut terminal);
        insta::assert_snapshot!("renders_tick_9", terminal.backend());
    }
}
