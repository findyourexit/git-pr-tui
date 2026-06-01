//! Persistent contextual footer.
//!
//! The footer has two logical regions:
//!
//! * **Left:** `key label` pairs for the current context's most-important
//!   actions, separated by `  ` spaces.  Danger actions are styled with the
//!   error / warning colour.  When the terminal is narrow the lowest-priority
//!   hints are dropped first (they were sorted by priority before being
//!   passed in) so we never overflow into the tail.
//! * **Right (constant tail):** `␣ actions · : cmd · ? keys` — always present.

use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Paragraph},
};

use crate::ui::theme::Theme;
use unicode_width::UnicodeWidthStr;

/// A single footer hint derived from an [`crate::app::actions::Action`].
#[derive(Debug, Clone)]
pub struct FooterHint {
    /// Already-rendered key label (e.g. `"o"`, `"↵"`, `"Ctrl-m"`).
    pub key: String,
    /// Short action label (e.g. `"Open"`, `"Merge"`).
    pub label: String,
    /// Danger-level hint — rendered in a warning/error colour.
    pub danger: bool,
}

/// Constant right-hand tail text (always rendered, even when narrow).
const TAIL: &str = " ␣ actions · : cmd · ? keys ";

/// Render the persistent contextual footer.
///
/// `hints` must be **pre-sorted** by priority (lowest priority index first /
/// most important first).  When the available width is too small to fit all
/// hints the last entries in the slice are dropped first, preserving the
/// most-important ones.
pub fn render_footer(f: &mut Frame, area: Rect, hints: &[FooterHint], theme: &Theme) {
    let bg_style = Style::default().bg(theme.bg).fg(theme.dim);
    f.render_widget(Block::default().style(bg_style), area);

    let tail_len = u16::try_from(TAIL.width()).unwrap_or(u16::MAX);
    // Reserve space for the tail; left area gets whatever remains (min 0).
    let left_width = area.width.saturating_sub(tail_len);

    let [left_area, right_area] =
        Layout::horizontal([Constraint::Length(left_width), Constraint::Length(tail_len)])
            .areas(area);

    // Build the left line by appending hints until we run out of room.
    let mut spans: Vec<Span> = Vec::new();
    let mut used: u16 = 0;

    for (i, hint) in hints.iter().enumerate() {
        let separator = if i == 0 { " " } else { "  " };
        let text = format!("{}{} {}", separator, hint.key, hint.label);
        let needed = u16::try_from(text.width()).unwrap_or(u16::MAX);
        if used.saturating_add(needed) > left_width {
            break; // no room — drop this and all lower-priority hints
        }
        used = used.saturating_add(needed);

        let sep_span = Span::styled(separator.to_string(), Style::default().fg(theme.dim));
        let key_style = Style::default().fg(theme.fg).add_modifier(Modifier::BOLD);
        let key_span = Span::styled(hint.key.clone(), key_style);
        let label_style = if hint.danger {
            Style::default().fg(theme.error)
        } else {
            Style::default().fg(theme.dim)
        };
        let label_span = Span::styled(format!(" {}", hint.label), label_style);

        if i > 0 {
            spans.push(sep_span);
        } else {
            // Leading space only (no separator prefix for the first hint)
            spans.push(Span::styled(
                " ".to_string(),
                Style::default().fg(theme.dim),
            ));
        }
        spans.push(key_span);
        spans.push(label_span);
    }

    let left_line = Line::from(spans);
    f.render_widget(
        Paragraph::new(left_line).style(Style::default().bg(theme.bg)),
        left_area,
    );

    // Constant tail — right-aligned, dim style.
    let tail_style = Style::default().bg(theme.bg).fg(theme.dim);
    f.render_widget(Paragraph::new(TAIL).style(tail_style), right_area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn render(width: u16, hints: &[FooterHint]) -> String {
        let backend = TestBackend::new(width, 1);
        let mut t = Terminal::new(backend).unwrap();
        let theme = crate::ui::theme::Theme::dark();
        t.draw(|f| render_footer(f, f.area(), hints, &theme))
            .unwrap();
        let buf = t.backend().buffer().clone();
        (0..buf.area().width)
            .map(|x| buf[(x, 0)].symbol().to_string())
            .collect()
    }

    fn pr_hints() -> Vec<FooterHint> {
        vec![
            FooterHint {
                key: "↵".to_string(),
                label: "Open thread".to_string(),
                danger: false,
            },
            FooterHint {
                key: "c".to_string(),
                label: "Comment".to_string(),
                danger: false,
            },
            FooterHint {
                key: "m".to_string(),
                label: "Merge".to_string(),
                danger: false,
            },
            FooterHint {
                key: "X".to_string(),
                label: "Close PR".to_string(),
                danger: true,
            },
        ]
    }

    #[test]
    fn footer_pr_conversation_shows_hints_and_tail() {
        let row = render(120, &pr_hints());
        assert!(row.contains('↵'), "key ↵, got: {row:?}");
        assert!(row.contains("Open thread"), "label, got: {row:?}");
        assert!(row.contains("␣ actions"), "tail, got: {row:?}");
        assert!(row.contains(": cmd"), "palette, got: {row:?}");
        assert!(row.contains("? keys"), "help, got: {row:?}");
        insta::assert_snapshot!("footer_pr_conversation_wide", row);
    }

    #[test]
    fn footer_narrow_truncates_lower_priority_hints() {
        // Only 40 chars wide — can't fit all four hints; the tail is ~28 chars
        // so ~12 chars for hints.  We should see at least the first hint and
        // always the tail.
        let row = render(60, &pr_hints());
        assert!(
            row.contains("␣ actions"),
            "tail must always appear, got: {row:?}"
        );
        insta::assert_snapshot!("footer_narrow_truncation", row);
    }
}
