//! `:log` overlay: scrollable view over [`crate::logging::LogRing::tail`].

use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState},
};

use crate::logging::LogRing;
use crate::ui::theme::Theme;

/// Default tail size used by [`render_log_view`].
pub const LOG_VIEW_TAIL: usize = 200;

/// View state for the `:log` overlay: selected line + viewport scroll.
#[derive(Debug, Default, Clone)]
pub struct LogViewState {
    pub selected: usize,
    pub scroll: usize,
}

impl LogViewState {
    /// Move selection down by one, clamped to `len.saturating_sub(1)`.
    pub fn scroll_down(&mut self, len: usize) {
        if len == 0 {
            self.selected = 0;
            return;
        }
        self.selected = (self.selected + 1).min(len - 1);
    }

    /// Move selection up by one (saturating at 0).
    pub fn scroll_up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    /// Jump to the most recent line.
    pub fn scroll_to_end(&mut self, len: usize) {
        self.selected = len.saturating_sub(1);
    }

    /// Jump to the oldest available line.
    pub fn scroll_to_start(&mut self) {
        self.selected = 0;
    }
}

/// Render the `:log` overlay as a centered Clear-backed modal listing
/// `ring.tail(LOG_VIEW_TAIL)` with the currently-selected line highlighted.
pub fn render_log_view(
    f: &mut Frame,
    area: Rect,
    ring: &LogRing,
    state: &LogViewState,
    theme: &Theme,
) {
    let width = area.width.saturating_sub(4).max(40).min(area.width);
    let height = area.height.saturating_sub(4).max(8).min(area.height);
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    let outer = Rect::new(x, y, width, height);

    f.render_widget(Clear, outer);

    let block = Block::default()
        .title(" :log ")
        .borders(Borders::ALL)
        .border_type(theme.border_type())
        .style(Style::default().bg(theme.bg).fg(theme.fg))
        .border_style(Style::default().fg(theme.primary));

    let lines = ring.tail(LOG_VIEW_TAIL);
    let items: Vec<ListItem> = lines
        .iter()
        .map(|l| ListItem::new(Line::from(Span::raw(l.clone()))))
        .collect();

    let list = List::new(items).block(block).highlight_style(
        Style::default()
            .bg(theme.primary)
            .fg(theme.bg)
            .add_modifier(Modifier::BOLD),
    );

    let mut list_state = ListState::default();
    if !lines.is_empty() {
        list_state.select(Some(state.selected.min(lines.len() - 1)));
    }

    f.render_stateful_widget(list, outer, &mut list_state);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    #[test]
    fn scroll_down_clamps_to_len_minus_one() {
        let mut s = LogViewState::default();
        s.scroll_down(3);
        s.scroll_down(3);
        s.scroll_down(3);
        s.scroll_down(3);
        assert_eq!(s.selected, 2);
    }

    #[test]
    fn scroll_up_saturates_at_zero() {
        let mut s = LogViewState {
            selected: 0,
            scroll: 0,
        };
        s.scroll_up();
        assert_eq!(s.selected, 0);
    }

    #[test]
    fn scroll_to_end_jumps_to_last_index() {
        let mut s = LogViewState::default();
        s.scroll_to_end(50);
        assert_eq!(s.selected, 49);
    }

    #[test]
    fn scroll_to_end_on_empty_stays_zero() {
        let mut s = LogViewState::default();
        s.scroll_to_end(0);
        assert_eq!(s.selected, 0);
    }

    #[test]
    fn renders_overlay_with_tail_of_ring() {
        let ring = LogRing::new();
        for i in 0..5 {
            ring.push(format!("entry-{i}"));
        }
        let state = LogViewState::default();
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                render_log_view(f, f.area(), &ring, &state, &Theme::dark());
            })
            .unwrap();

        let buf = terminal.backend().buffer();
        let mut rendered = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                rendered.push_str(buf[(x, y)].symbol());
            }
            rendered.push('\n');
        }
        for i in 0..5 {
            let needle = format!("entry-{i}");
            assert!(
                rendered.contains(&needle),
                "expected `{needle}` in overlay output, got:\n{rendered}"
            );
        }
        assert!(
            rendered.contains(":log"),
            "expected title `:log`, got:\n{rendered}"
        );
    }

    #[test]
    fn renders_overlay_caps_to_log_view_tail_lines() {
        let ring = LogRing::new();
        for i in 0..(LOG_VIEW_TAIL + 10) {
            ring.push(format!("e{i}"));
        }
        let visible = ring.tail(LOG_VIEW_TAIL);
        assert_eq!(visible.len(), LOG_VIEW_TAIL);
        // Oldest visible is the (10)th push (indices 0..9 evicted by ring cap of 200).
        assert_eq!(visible.first().map(String::as_str), Some("e10"));
    }
}
