use ratatui::{
    Frame,
    layout::{Constraint, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, TableState},
};

use crate::data::models::PrSummary;
use crate::ui::components::table::{Column, render_table};
use crate::ui::glyphs;
use crate::ui::theme::Theme;

#[derive(Debug)]
pub struct PrListView<'a> {
    pub rows: &'a [PrSummary],
    pub selected: usize,
    pub loading: bool,
    pub filter_label: Option<&'a str>,
}

fn columns() -> Vec<Column> {
    pr_summary_columns()
}

#[must_use]
pub fn pr_summary_columns() -> Vec<Column> {
    vec![
        Column::new("", Constraint::Length(7)),
        Column::new("#", Constraint::Min(6)).right(),
        Column::new("Title", Constraint::Fill(1)).ellipsis(),
        Column::new("Author", Constraint::Min(14)),
        Column::new("+", Constraint::Min(6)).right(),
        Column::new("\u{2212}", Constraint::Min(6)).right(),
    ]
}

#[must_use]
pub fn pr_summary_cells(pr: &PrSummary, theme: &Theme) -> Vec<Line<'static>> {
    let unicode = theme.use_unicode_glyphs();
    let state_icon = glyphs::glyph_for_pr_state(pr.is_draft, pr.state, unicode);
    let checks_icon = glyphs::glyph_for_checks_rollup(pr.checks, unicode);
    let review_icon = glyphs::glyph_for_review_decision(pr.review_decision, unicode);

    let state_color = glyphs::color_for_pr_state(pr.is_draft, pr.state, theme);
    let checks_color = glyphs::color_for_checks_rollup(pr.checks, theme);
    let review_color = glyphs::color_for_review_decision(pr.review_decision, theme);

    let glyphs_line = Line::from(vec![
        Span::styled(state_icon, Style::default().fg(state_color)),
        Span::raw(" "),
        Span::styled(checks_icon, Style::default().fg(checks_color)),
        Span::raw(" "),
        Span::styled(review_icon, Style::default().fg(review_color)),
    ]);

    vec![
        glyphs_line,
        Line::from(format!("#{}", pr.id.number)),
        Line::from(Span::styled(
            pr.title.clone(),
            Style::default().fg(theme.fg).add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            pr.author.clone(),
            Style::default().fg(theme.dim),
        )),
        Line::from(Span::styled(
            format!("+{}", pr.additions),
            Style::default().fg(theme.success),
        )),
        Line::from(Span::styled(
            format!("-{}", pr.deletions),
            Style::default().fg(theme.error),
        )),
    ]
}

pub fn render_pr_list(f: &mut Frame, area: Rect, view: &PrListView, theme: &Theme) {
    if area.height == 0 || area.width == 0 {
        return;
    }

    if view.rows.is_empty() {
        render_empty_or_loading(f, area, view, theme);
        return;
    }

    let cols = columns();
    let rows: Vec<Vec<Line>> = view
        .rows
        .iter()
        .map(|pr| pr_summary_cells(pr, theme))
        .collect();

    let mut state = TableState::default();
    state.select(Some(view.selected.min(view.rows.len().saturating_sub(1))));

    render_table(f, area, &cols, rows, Some(&mut state), true, None, theme);
}

fn render_empty_or_loading(f: &mut Frame, area: Rect, view: &PrListView, theme: &Theme) {
    let message = if view.loading {
        "Loading PRs…".to_string()
    } else {
        match view.filter_label {
            Some(label) => format!("No PRs match {label}."),
            None => "No PRs to display.".to_string(),
        }
    };
    let paragraph = Paragraph::new(message).style(Style::default().fg(theme.dim));
    f.render_widget(paragraph, area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::models::{
        ChecksRollup, Mergeable, PrId, PrState, PrSummary, Repo, ReviewDecision,
    };
    use chrono::TimeZone;
    use ratatui::{Terminal, backend::TestBackend};

    fn sample_summary(number: u64, title: &str) -> PrSummary {
        PrSummary {
            id: PrId {
                repo: Repo {
                    owner: "acme".into(),
                    name: "widgets".into(),
                },
                number,
            },
            title: title.into(),
            is_draft: false,
            state: PrState::Open,
            author: "octocat".into(),
            base: "main".into(),
            head: "f".into(),
            additions: 10,
            deletions: 2,
            changed_files: 1,
            comments: 3,
            review_decision: Some(ReviewDecision::ReviewRequired),
            mergeable: Mergeable::Clean,
            labels: vec![],
            updated_at: chrono::Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
            created_at: chrono::Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
            checks: ChecksRollup::Success,
        }
    }

    fn five_rows() -> Vec<PrSummary> {
        vec![
            sample_summary(1, "Fix the flux capacitor"),
            sample_summary(2, "Add new widgets to dashboard"),
            sample_summary(3, "Refactor core rendering engine"),
            sample_summary(4, "Update dependencies"),
            sample_summary(5, "Drop legacy support"),
        ]
    }

    #[test]
    fn renders_header_and_five_rows() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let rows = five_rows();
        terminal
            .draw(|f| {
                let area = f.area();
                let view = PrListView {
                    rows: &rows,
                    selected: 0,
                    loading: false,
                    filter_label: None,
                };
                render_pr_list(f, area, &view, &Theme::dark());
            })
            .unwrap();
        insta::assert_snapshot!("renders_header_and_five_rows", terminal.backend());
    }

    #[test]
    fn renders_with_scrollbar_when_overflowing() {
        let backend = TestBackend::new(80, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        let rows: Vec<PrSummary> = (1..=30)
            .map(|n| sample_summary(n, &format!("PR number {n}")))
            .collect();
        terminal
            .draw(|f| {
                let area = f.area();
                let view = PrListView {
                    rows: &rows,
                    selected: 0,
                    loading: false,
                    filter_label: None,
                };
                render_pr_list(f, area, &view, &Theme::dark());
            })
            .unwrap();
        insta::assert_snapshot!(
            "renders_with_scrollbar_when_overflowing",
            terminal.backend()
        );
    }

    #[test]
    fn renders_loading_message_when_loading_and_empty() {
        let backend = TestBackend::new(80, 5);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                let view = PrListView {
                    rows: &[],
                    selected: 0,
                    loading: true,
                    filter_label: None,
                };
                render_pr_list(f, area, &view, &Theme::dark());
            })
            .unwrap();
        insta::assert_snapshot!("renders_loading_message", terminal.backend());
    }

    #[test]
    fn renders_empty_message_with_filter_label() {
        let backend = TestBackend::new(80, 5);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                let view = PrListView {
                    rows: &[],
                    selected: 0,
                    loading: false,
                    filter_label: Some("is:merged"),
                };
                render_pr_list(f, area, &view, &Theme::dark());
            })
            .unwrap();
        insta::assert_snapshot!("renders_empty_with_filter", terminal.backend());
    }

    fn render_at_width(width: u16) -> String {
        let backend = TestBackend::new(width, 12);
        let mut terminal = Terminal::new(backend).unwrap();
        let rows = five_rows();
        terminal
            .draw(|f| {
                let area = f.area();
                let view = PrListView {
                    rows: &rows,
                    selected: 0,
                    loading: false,
                    filter_label: None,
                };
                render_pr_list(f, area, &view, &Theme::dark());
            })
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn pr_list_reflow_snapshots() {
        insta::assert_snapshot!("pr_list_reflow_60", render_at_width(60));
        insta::assert_snapshot!("pr_list_reflow_100", render_at_width(100));
        insta::assert_snapshot!("pr_list_reflow_160", render_at_width(160));
    }
}
