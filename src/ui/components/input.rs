use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};

use crate::app::intent::UserIntent;
use crate::ui::theme::Theme;

#[derive(Default, Debug, Clone)]
pub struct InputBuffer {
    pub text: String,
    pub cursor: usize,
}

impl InputBuffer {
    pub fn insert_char(&mut self, c: char) {
        self.text.insert(self.cursor, c);
        self.cursor += c.len_utf8();
    }

    pub fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let current_cursor = self.cursor;
        let prev_char_idx = self.text[..current_cursor]
            .chars()
            .last()
            .map_or(0, |c| current_cursor - c.len_utf8());
        self.text.remove(prev_char_idx);
        self.cursor = prev_char_idx;
    }

    /// Map a key event to a high-level composer intent.
    ///
    /// Returns `Some(UserIntent::Submit)` for `Ctrl-S`, `Some(UserIntent::Cancel)`
    /// for `Esc`, and `None` for any other key (the caller is responsible for
    /// routing printable/backspace input to [`Self::insert_char`] /
    /// [`Self::backspace`]). Buffer contents are never mutated by this method,
    /// so they are preserved across repeated calls and across re-renders.
    #[must_use]
    pub fn map_key(&self, k: &KeyEvent) -> Option<UserIntent> {
        match (k.code, k.modifiers) {
            (KeyCode::Char('s'), m) if m.contains(KeyModifiers::CONTROL) => {
                Some(UserIntent::Submit)
            }
            (KeyCode::Esc, _) => Some(UserIntent::Cancel),
            _ => None,
        }
    }
}

pub fn render_input(f: &mut Frame, area: Rect, buffer: &InputBuffer, theme: &Theme) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(theme.border_type())
        .style(Style::default().bg(theme.bg).fg(theme.fg))
        .border_style(Style::default().fg(theme.primary));

    let before = &buffer.text[..buffer.cursor];
    let (at_cursor, after) = if buffer.cursor < buffer.text.len() {
        let c = buffer.text[buffer.cursor..].chars().next().unwrap_or(' ');
        let char_len = c.len_utf8();
        (
            &buffer.text[buffer.cursor..buffer.cursor + char_len],
            &buffer.text[buffer.cursor + char_len..],
        )
    } else {
        (" ", "")
    };

    let spans = vec![
        Span::raw(before),
        Span::styled(at_cursor, Style::default().add_modifier(Modifier::REVERSED)),
        Span::raw(after),
    ];

    let paragraph = Paragraph::new(Line::from(spans))
        .block(block)
        .wrap(Wrap { trim: false });

    f.render_widget(paragraph, area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    #[test]
    fn insert_then_backspace() {
        let mut buf = InputBuffer::default();
        buf.insert_char('h');
        buf.insert_char('i');
        assert_eq!(buf.text, "hi");
        assert_eq!(buf.cursor, 2);

        buf.backspace();
        assert_eq!(buf.text, "h");
        assert_eq!(buf.cursor, 1);
    }

    #[test]
    fn renders_with_cursor_mid_text() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let buf = InputBuffer {
            text: "Hello world".to_string(),
            cursor: 5,
        };
        terminal
            .draw(|f| {
                render_input(f, f.area(), &buf, &crate::ui::theme::Theme::dark());
            })
            .unwrap();
        insta::assert_snapshot!("renders_with_cursor_mid_text", terminal.backend());
    }

    #[test]
    fn ctrl_s_maps_to_submit() {
        let buf = InputBuffer {
            text: "hello".into(),
            cursor: 5,
        };
        let k = KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL);
        assert_eq!(buf.map_key(&k), Some(UserIntent::Submit));
    }

    #[test]
    fn esc_maps_to_cancel() {
        let buf = InputBuffer::default();
        let k = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        assert_eq!(buf.map_key(&k), Some(UserIntent::Cancel));
    }

    #[test]
    fn other_keys_map_to_none() {
        let buf = InputBuffer::default();
        let plain_s = KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE);
        let enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        let ctrl_x = KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL);
        assert_eq!(buf.map_key(&plain_s), None);
        assert_eq!(buf.map_key(&enter), None);
        assert_eq!(buf.map_key(&ctrl_x), None);
    }

    #[test]
    fn map_key_never_mutates_buffer() {
        let buf = InputBuffer {
            text: "preserve me".into(),
            cursor: 7,
        };
        let snapshot_text = buf.text.clone();
        let snapshot_cursor = buf.cursor;

        for k in [
            KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL),
            KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE),
        ] {
            let _ = buf.map_key(&k);
        }
        assert_eq!(buf.text, snapshot_text);
        assert_eq!(buf.cursor, snapshot_cursor);
    }

    #[test]
    fn content_preserved_across_rerender() {
        let backend = TestBackend::new(40, 5);
        let mut terminal = Terminal::new(backend).unwrap();
        let buf = InputBuffer {
            text: "drafty".into(),
            cursor: 6,
        };
        let theme = crate::ui::theme::Theme::dark();
        for _ in 0..3 {
            terminal
                .draw(|f| render_input(f, f.area(), &buf, &theme))
                .unwrap();
        }
        assert_eq!(buf.text, "drafty");
        assert_eq!(buf.cursor, 6);
    }
}
