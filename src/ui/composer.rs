//! Composer overlay: shows the in-progress body + which context is being composed.
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};

use crate::app::state::{ComposerContext, ComposerState};
use crate::ui::theme::Theme;

pub fn render_composer(f: &mut Frame, area: Rect, composer: &ComposerState, theme: &Theme) {
    let label = match &composer.context {
        ComposerContext::PrComment { .. } => "PR comment",
        ComposerContext::ReplyToThread { .. } => "Reply to thread",
        ComposerContext::LineComment { .. } => "Line comment",
    };

    let width = area.width.saturating_sub(4).max(40).min(area.width);
    let height = area.height.saturating_sub(4).max(8).min(area.height);
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    let outer = Rect::new(x, y, width, height);

    f.render_widget(Clear, outer);

    let title = format!(" Composer — {label}  (Ctrl-S submit, Ctrl-P preview, Esc cancel) ");
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_type(theme.border_type())
        .style(Style::default().bg(theme.bg).fg(theme.fg))
        .border_style(Style::default().fg(theme.primary));

    let inner = block.inner(outer);
    f.render_widget(block, outer);

    if composer.preview_visible {
        let [edit_area, divider_area, preview_area] = Layout::vertical([
            Constraint::Percentage(60),
            Constraint::Length(1),
            Constraint::Min(0),
        ])
        .areas(inner);

        let body_lines: Vec<Line> = composer.body.lines().map(Line::from).collect();
        f.render_widget(
            Paragraph::new(body_lines).wrap(Wrap { trim: false }),
            edit_area,
        );

        // Divider
        let divider_label = " Preview ";
        let remaining = divider_area
            .width
            .saturating_sub(u16::try_from(divider_label.len()).unwrap_or(divider_area.width));
        let fill: String = "─".repeat(remaining as usize);
        let divider_line = Line::from(vec![
            Span::styled(divider_label, Style::default().fg(theme.dim)),
            Span::styled(fill, Style::default().fg(theme.dim)),
        ]);
        f.render_widget(Paragraph::new(divider_line), divider_area);

        // Preview
        if composer.body.trim().is_empty() {
            let placeholder = Line::from(Span::styled(
                "(nothing to preview)",
                Style::default().fg(theme.dim),
            ));
            f.render_widget(Paragraph::new(placeholder), preview_area);
        } else {
            let pv =
                crate::ui::markdown::render_markdown(&composer.body, preview_area.width, theme);
            f.render_widget(Paragraph::new(pv), preview_area);
        }
    } else {
        let body_lines: Vec<Line> = composer.body.lines().map(Line::from).collect();
        f.render_widget(Paragraph::new(body_lines).wrap(Wrap { trim: false }), inner);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::state::{ComposerContext, ComposerState};
    use crate::data::models::{PrId, Repo};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn buf_to_string(t: &Terminal<TestBackend>) -> String {
        let buf = t.backend().buffer();
        let mut s = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                s.push_str(buf[(x, y)].symbol());
            }
            s.push('\n');
        }
        s
    }

    #[test]
    fn renders_pr_comment_context_label_and_body() {
        let pr = PrId {
            repo: Repo {
                owner: "o".into(),
                name: "n".into(),
            },
            number: 1,
        };
        let composer = ComposerState {
            body: "draft body".into(),
            context: ComposerContext::PrComment { pr_id: pr },
            head_moved: false,
            preview_visible: true,
        };
        let backend = TestBackend::new(80, 12);
        let mut t = Terminal::new(backend).unwrap();
        t.draw(|f| render_composer(f, f.area(), &composer, &Theme::dark()))
            .unwrap();
        let rendered = buf_to_string(&t);
        assert!(rendered.contains("draft body"), "body absent:\n{rendered}");
        assert!(
            rendered.contains("PR comment"),
            "context label absent:\n{rendered}"
        );
    }

    #[test]
    fn renders_reply_to_thread_context() {
        let pr = PrId {
            repo: Repo {
                owner: "o".into(),
                name: "n".into(),
            },
            number: 1,
        };
        let composer = ComposerState {
            body: String::new(),
            context: ComposerContext::ReplyToThread {
                pr_id: pr,
                thread_id: "T_kw".into(),
            },
            head_moved: false,
            preview_visible: true,
        };
        let backend = TestBackend::new(80, 12);
        let mut t = Terminal::new(backend).unwrap();
        t.draw(|f| render_composer(f, f.area(), &composer, &Theme::dark()))
            .unwrap();
        let rendered = buf_to_string(&t);
        assert!(
            rendered.contains("Reply to thread"),
            "context label absent:\n{rendered}"
        );
    }

    #[test]
    fn renders_composer_with_preview() {
        let pr = PrId {
            repo: Repo {
                owner: "o".into(),
                name: "n".into(),
            },
            number: 1,
        };
        let composer = ComposerState {
            body: "# Hi\n\n- a".into(),
            context: ComposerContext::PrComment { pr_id: pr },
            head_moved: false,
            preview_visible: true,
        };
        let backend = TestBackend::new(80, 16);
        let mut t = Terminal::new(backend).unwrap();
        t.draw(|f| render_composer(f, f.area(), &composer, &Theme::dark()))
            .unwrap();
        let rendered = buf_to_string(&t);
        assert!(rendered.contains("Hi"), "heading absent:\n{rendered}");
        assert!(rendered.contains('•'), "bullet absent:\n{rendered}");
        assert!(rendered.contains("Preview"), "divider absent:\n{rendered}");
    }

    #[test]
    fn renders_composer_preview_hidden() {
        let pr = PrId {
            repo: Repo {
                owner: "o".into(),
                name: "n".into(),
            },
            number: 1,
        };
        let composer = ComposerState {
            body: "# Hi\n\n- a".into(),
            context: ComposerContext::PrComment { pr_id: pr },
            head_moved: false,
            preview_visible: false,
        };
        let backend = TestBackend::new(80, 16);
        let mut t = Terminal::new(backend).unwrap();
        t.draw(|f| render_composer(f, f.area(), &composer, &Theme::dark()))
            .unwrap();
        let rendered = buf_to_string(&t);
        assert!(
            !rendered.contains("Preview"),
            "preview divider should be absent:\n{rendered}"
        );
        assert!(
            rendered.contains("# Hi"),
            "raw body should be present:\n{rendered}"
        );
    }
}
