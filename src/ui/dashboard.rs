use crate::data::github::DashboardBucket;
use crate::data::models::PrSummary;
use crate::ui::theme::Theme;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Paragraph, TableState};

#[derive(Debug, Clone)]
pub enum DashboardState {
    Loading,
    Loaded(Vec<(DashboardBucket, Vec<PrSummary>)>),
    Error(String),
}

pub fn render_dashboard_state(
    f: &mut Frame,
    area: Rect,
    state: &DashboardState,
    focused: usize,
    rows: [usize; 3],
    theme: &Theme,
) {
    match state {
        DashboardState::Loading => {
            let block = Block::default()
                .borders(Borders::ALL)
                .border_type(theme.border_type())
                .title(" Dashboard ");
            let text = Paragraph::new("Loading dashboard…")
                .alignment(ratatui::layout::Alignment::Center)
                .block(block);
            f.render_widget(text, area);
        }
        DashboardState::Loaded(buckets) => {
            render_dashboard(f, area, buckets, focused, rows, theme);
        }
        DashboardState::Error(msg) => {
            let block = Block::default()
                .borders(Borders::ALL)
                .border_type(theme.border_type())
                .title(" Dashboard — error ")
                .title_style(Style::default().fg(theme.error))
                .border_style(Style::default().fg(theme.error));
            let text = Paragraph::new(msg.as_str())
                .style(Style::default().fg(theme.error))
                .block(block)
                .wrap(ratatui::widgets::Wrap { trim: true });
            f.render_widget(text, area);
        }
    }
}

pub fn render_dashboard(
    f: &mut Frame,
    area: Rect,
    buckets: &[(DashboardBucket, Vec<PrSummary>)],
    focused: usize,
    rows: [usize; 3],
    theme: &Theme,
) {
    if buckets.is_empty() {
        return;
    }

    let panels = Layout::vertical([Constraint::Ratio(1, 3); 3]).split(area);

    for (i, (bucket, prs)) in buckets.iter().enumerate() {
        let label = match bucket {
            DashboardBucket::ReviewRequested => "Review requested",
            DashboardBucket::Authored => "Authored",
            DashboardBucket::Assigned => "Assigned",
        };
        let title = format!(" {} ({}) ", label, prs.len());
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(theme.border_type())
            .title(title)
            .title_style(Style::default().fg(theme.primary));

        if prs.is_empty() {
            let placeholder = match bucket {
                DashboardBucket::ReviewRequested => "  Nothing waiting for review",
                DashboardBucket::Authored => "  Nothing authored is open",
                DashboardBucket::Assigned => "  Nothing assigned to you",
            };
            crate::ui::components::table::render_table(
                f,
                panels[i],
                &crate::ui::pr_list::pr_summary_columns(),
                vec![],
                None,
                i == focused,
                Some(block),
                theme,
            );
            // Draw placeholder text inside the block
            let inner = panels[i].inner(ratatui::layout::Margin {
                horizontal: 2,
                vertical: 2,
            });
            let p = Paragraph::new(placeholder).style(Style::default().fg(theme.dim));
            f.render_widget(p, inner);
        } else {
            let rows_cells: Vec<Vec<Line>> = prs
                .iter()
                .map(|pr| crate::ui::pr_list::pr_summary_cells(pr, theme))
                .collect();
            let selected = rows[i].min(prs.len() - 1);
            let mut st = TableState::default();
            st.select(Some(selected));
            crate::ui::components::table::render_table(
                f,
                panels[i],
                &crate::ui::pr_list::pr_summary_columns(),
                rows_cells,
                Some(&mut st),
                i == focused,
                Some(block),
                theme,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::github::DashboardBucket;
    use crate::data::models::{
        ChecksRollup, Mergeable, PrId, PrState, PrSummary, Repo, ReviewDecision,
    };
    use chrono::TimeZone;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

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

    fn three_bucket_data() -> Vec<(DashboardBucket, Vec<PrSummary>)> {
        vec![
            (
                DashboardBucket::ReviewRequested,
                vec![sample_summary(1, "fix: foo")],
            ),
            (
                DashboardBucket::Authored,
                vec![sample_summary(2, "feat: bar")],
            ),
            (DashboardBucket::Assigned, vec![]),
        ]
    }

    #[test]
    fn renders_three_buckets() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let buckets = three_bucket_data();
        terminal
            .draw(|f| {
                let area = f.area();
                render_dashboard(
                    f,
                    area,
                    &buckets,
                    0,
                    [0; 3],
                    &crate::ui::theme::Theme::dark(),
                );
            })
            .unwrap();
        insta::assert_snapshot!("renders_three_buckets", terminal.backend());
    }

    #[test]
    fn renders_loading_state() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let state = DashboardState::Loading;
        terminal
            .draw(|f| {
                let area = f.area();
                render_dashboard_state(
                    f,
                    area,
                    &state,
                    0,
                    [0; 3],
                    &crate::ui::theme::Theme::dark(),
                );
            })
            .unwrap();
        insta::assert_snapshot!("renders_loading_state", terminal.backend());
    }

    #[test]
    fn renders_empty_buckets() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let state = DashboardState::Loaded(vec![
            (DashboardBucket::ReviewRequested, vec![]),
            (DashboardBucket::Authored, vec![]),
            (DashboardBucket::Assigned, vec![]),
        ]);
        terminal
            .draw(|f| {
                let area = f.area();
                render_dashboard_state(
                    f,
                    area,
                    &state,
                    0,
                    [0; 3],
                    &crate::ui::theme::Theme::dark(),
                );
            })
            .unwrap();
        insta::assert_snapshot!("renders_empty_buckets", terminal.backend());
    }

    #[test]
    fn renders_error_state() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let state = DashboardState::Error("network unreachable".into());
        terminal
            .draw(|f| {
                let area = f.area();
                render_dashboard_state(
                    f,
                    area,
                    &state,
                    0,
                    [0; 3],
                    &crate::ui::theme::Theme::dark(),
                );
            })
            .unwrap();
        insta::assert_snapshot!("renders_error_state", terminal.backend());
    }

    #[test]
    fn renders_focused_middle_panel() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let buckets = vec![
            (
                DashboardBucket::ReviewRequested,
                vec![sample_summary(1, "fix: foo")],
            ),
            (
                DashboardBucket::Authored,
                vec![sample_summary(2, "feat: bar")],
            ),
            (
                DashboardBucket::Assigned,
                vec![sample_summary(3, "chore: baz")],
            ),
        ];
        terminal
            .draw(|f| {
                let area = f.area();
                render_dashboard(
                    f,
                    area,
                    &buckets,
                    1,
                    [0, 0, 0],
                    &crate::ui::theme::Theme::dark(),
                );
            })
            .unwrap();
        insta::assert_snapshot!("renders_focused_middle_panel", terminal.backend());
    }
}
