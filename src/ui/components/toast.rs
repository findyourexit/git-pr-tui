use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::Style,
    widgets::{Block, Borders, Paragraph},
};

use crate::app::state::{Toast, ToastKind};
use crate::ui::theme::Theme;

pub fn render_toast(f: &mut Frame, area: Rect, toast: &Toast, theme: &Theme) {
    let border_color = match toast.kind {
        ToastKind::Info => theme.primary,
        ToastKind::Warning => theme.warning,
        ToastKind::Error => theme.error,
    };

    let style = Style::default().fg(theme.fg).bg(theme.bg);
    let border_style = Style::default().fg(border_color);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(theme.border_type())
        .style(style)
        .border_style(border_style);

    let width = u16::try_from(toast.message.len())
        .unwrap_or(u16::MAX)
        .saturating_add(2);
    let width = width.min(area.width);

    let [top_area, _] = Layout::vertical([Constraint::Length(3), Constraint::Min(0)]).areas(area);

    let [_, right_area] =
        Layout::horizontal([Constraint::Min(0), Constraint::Length(width)]).areas(top_area);

    f.render_widget(
        Paragraph::new(toast.message.as_str()).block(block),
        right_area,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use std::time::Instant;

    fn render_test_toast(kind: ToastKind, msg: &str, terminal: &mut Terminal<TestBackend>) {
        let toast = Toast {
            kind,
            message: msg.to_string(),
            created_at: Instant::now(),
        };
        terminal
            .draw(|f| render_toast(f, f.area(), &toast, &crate::ui::theme::Theme::dark()))
            .unwrap();
    }

    #[test]
    fn renders_info() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        render_test_toast(ToastKind::Info, "Information message", &mut terminal);
        insta::assert_snapshot!("renders_info", terminal.backend());
    }

    #[test]
    fn renders_warning() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        render_test_toast(ToastKind::Warning, "Warning message", &mut terminal);
        insta::assert_snapshot!("renders_warning", terminal.backend());
    }

    #[test]
    fn renders_error() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        render_test_toast(ToastKind::Error, "Error message", &mut terminal);
        insta::assert_snapshot!("renders_error", terminal.backend());
    }
}
