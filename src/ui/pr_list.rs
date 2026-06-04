use ratatui::{
    Frame,
    layout::{Constraint, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, TableState},
};

use crate::data::models::PrSummary;
use crate::ui::components::table::{Column, render_table};
use crate::ui::glyphs;
use crate::ui::layout::{LayoutMode, min_size_check};
use crate::ui::theme::Theme;

/// Hint line shown at the foot of the filter/sort pickers.
const PICKER_HINT: &str = "\u{2191}\u{2193} move \u{00b7} \u{21b5} apply \u{00b7} esc";

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

/// Render a centered single-select picker (used for the Filter and Sort
/// overlays). On a terminal too small for the standard overlay it fills the
/// whole area, matching [`crate::ui::components::modal::render_modal`].
pub fn render_list_picker(
    f: &mut Frame,
    area: Rect,
    title: &str,
    labels: &[&str],
    selected: usize,
    theme: &Theme,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let popup = if matches!(
        min_size_check(area.width, area.height),
        LayoutMode::FullScreenModal
    ) {
        area
    } else {
        let longest = labels.iter().map(|l| l.chars().count()).max().unwrap_or(0);
        let content_w = longest
            .max(title.chars().count())
            .max(PICKER_HINT.chars().count());
        let width = (u16::try_from(content_w)
            .unwrap_or(u16::MAX)
            .saturating_add(4))
        .clamp(12, area.width.saturating_sub(2));
        let rows = u16::try_from(labels.len()).unwrap_or(u16::MAX);
        // rows + 1 hint line + 2 border lines.
        let height = rows
            .saturating_add(3)
            .clamp(3, area.height.saturating_sub(2));
        let x = area.x + (area.width.saturating_sub(width)) / 2;
        let y = area.y + (area.height.saturating_sub(height)) / 2;
        Rect::new(x, y, width, height)
    };

    f.render_widget(Clear, popup);
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_type(theme.border_type())
        .style(Style::default().bg(theme.bg).fg(theme.fg))
        .border_style(Style::default().fg(theme.primary));
    f.render_widget(block, popup);

    if popup.width <= 2 || popup.height <= 2 {
        return;
    }
    let inner = Rect::new(popup.x + 1, popup.y + 1, popup.width - 2, popup.height - 2);

    let list_height = inner.height.saturating_sub(1);
    if list_height > 0 {
        let selected = selected.min(labels.len().saturating_sub(1));
        let items: Vec<ListItem> = labels
            .iter()
            .enumerate()
            .map(|(i, label)| {
                let style = if i == selected {
                    Style::default()
                        .fg(theme.bg)
                        .bg(theme.primary)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(theme.fg)
                };
                ListItem::new(Line::from(Span::styled((*label).to_string(), style)))
            })
            .collect();
        let list_area = Rect::new(inner.x, inner.y, inner.width, list_height);
        f.render_widget(List::new(items), list_area);
    }

    let hint_area = Rect::new(inner.x, inner.y + inner.height - 1, inner.width, 1);
    let hint = Paragraph::new(Line::from(Span::styled(
        PICKER_HINT,
        Style::default().fg(theme.dim).add_modifier(Modifier::DIM),
    )));
    f.render_widget(hint, hint_area);
}

/// Render the one-line incremental search bar (vim-style `/query`).
pub fn render_search_bar(f: &mut Frame, area: Rect, query: &str, theme: &Theme) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let line = Line::from(vec![
        Span::styled(
            "/",
            Style::default()
                .fg(theme.primary)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(query.to_string(), Style::default().fg(theme.fg)),
        Span::styled("\u{2588}", Style::default().fg(theme.primary)),
    ]);
    f.render_widget(
        Paragraph::new(line).style(Style::default().bg(theme.bg)),
        area,
    );
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

    fn buf_string(t: &Terminal<TestBackend>) -> String {
        let buf = t.backend().buffer().clone();
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
    fn list_picker_renders_title_labels_and_hint() {
        let mut t = Terminal::new(TestBackend::new(60, 16)).unwrap();
        t.draw(|f| {
            render_list_picker(
                f,
                f.area(),
                " Filter ",
                &["is:open", "is:closed", "is:merged"],
                1,
                &Theme::dark(),
            );
        })
        .unwrap();
        let s = buf_string(&t);
        assert!(s.contains("Filter"), "title missing:\n{s}");
        assert!(
            s.contains("is:open") && s.contains("is:merged"),
            "labels missing:\n{s}"
        );
        assert!(s.contains("apply"), "hint missing:\n{s}");
    }

    #[test]
    fn list_picker_fills_area_on_small_terminal() {
        // Below the 60×18 overlay threshold the picker fills the whole area.
        let mut t = Terminal::new(TestBackend::new(40, 12)).unwrap();
        t.draw(|f| {
            render_list_picker(
                f,
                f.area(),
                " Sort ",
                &["sort:updated-desc"],
                0,
                &Theme::dark(),
            );
        })
        .unwrap();
        let buf = t.backend().buffer();
        assert_eq!(buf[(0, 0)].symbol(), "\u{256d}", "top-left border");
        assert_eq!(buf[(39, 11)].symbol(), "\u{256f}", "bottom-right border");
    }

    #[test]
    fn search_bar_shows_query_with_prefix() {
        let mut t = Terminal::new(TestBackend::new(40, 1)).unwrap();
        t.draw(|f| render_search_bar(f, f.area(), "retry", &Theme::dark()))
            .unwrap();
        assert!(
            buf_string(&t).contains("/retry"),
            "search bar query missing"
        );
    }
}
