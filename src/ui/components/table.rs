//! Shared reactive table renderer. All tabular views build a `&[Column]` plus
//! one `Vec<Line>` per row and delegate layout/scroll/focus here so column
//! widths reflow proportionally with the panel on every render.

use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Flex, Layout, Rect},
    style::{Modifier, Style},
    text::Line,
    widgets::{
        Block, Cell, Row, Scrollbar, ScrollbarOrientation, ScrollbarState, Table, TableState,
    },
};

use crate::ui::text::truncate_to_width;
use crate::ui::theme::Theme;

/// Single space between columns; also used by the internal width solve so it
/// matches `ratatui::Table`'s own column layout exactly.
pub const COLUMN_SPACING: u16 = 1;

/// One column of a reactive table.
#[derive(Debug, Clone)]
pub struct Column {
    /// Header-row label.
    pub header: String,
    /// Width constraint, forwarded to `Table::widths`.
    pub constraint: Constraint,
    /// Cell alignment (Left for text, Right for numeric columns).
    pub align: Alignment,
    /// When true, overflowing cell text is truncated with `…` to the resolved
    /// column width. Numeric/fixed columns set this false (they never overflow).
    pub ellipsis: bool,
}

impl Column {
    #[must_use]
    pub fn new(header: impl Into<String>, constraint: Constraint) -> Self {
        Self {
            header: header.into(),
            constraint,
            align: Alignment::Left,
            ellipsis: false,
        }
    }

    #[must_use]
    pub fn right(mut self) -> Self {
        self.align = Alignment::Right;
        self
    }

    #[must_use]
    pub fn ellipsis(mut self) -> Self {
        self.ellipsis = true;
        self
    }
}

/// Resolve per-column widths exactly as `ratatui::Table` will (no selection
/// column because we render no highlight symbol).
fn resolved_widths(columns: &[Column], inner: Rect) -> Vec<u16> {
    let constraints: Vec<Constraint> = columns.iter().map(|c| c.constraint).collect();
    Layout::horizontal(constraints)
        .flex(Flex::Legacy)
        .spacing(COLUMN_SPACING)
        .split(inner)
        .iter()
        .map(|r| r.width)
        .collect()
}

/// Apply per-column ellipsis truncation to a row's cell lines, using the
/// resolved widths. Columns without `ellipsis` pass through unchanged.
/// Truncates cells that are marked as ellipsis columns to fit within their
/// allocated width, appending `…` as needed.
///
/// **Single-span assumption:** when truncation occurs the cell is rebuilt as
/// one span carrying the first span's style, so any per-span styling beyond
/// the first is lost. All current ellipsis columns (Title / Path / Name /
/// Context / Summary / Details) are single-styled, so this is safe.
fn ellipsize_row<'a>(columns: &[Column], widths: &[u16], row: Vec<Line<'a>>) -> Vec<Line<'a>> {
    row.into_iter()
        .enumerate()
        .map(|(i, line)| {
            let want_ellipsis = columns.get(i).is_some_and(|c| c.ellipsis);
            let width = usize::from(widths.get(i).copied().unwrap_or(0));
            if !want_ellipsis {
                return line;
            }
            let plain: String = line
                .spans
                .iter()
                .map(|s| s.content.as_ref())
                .collect::<String>();
            if unicode_width::UnicodeWidthStr::width(plain.as_str()) <= width {
                return line;
            }
            let style = line.spans.first().map_or_else(Style::default, |s| s.style);
            Line::from(ratatui::text::Span::styled(
                truncate_to_width(&plain, width),
                style,
            ))
        })
        .collect()
}

/// Render a reactive table into `area`.
///
/// * `columns` — column specs (header row + widths + alignment).
/// * `rows`    — one `Vec<Line>` per row, each of length `columns.len()`.
/// * `state`   — `Some` for selectable tables (highlight + auto-scroll +
///   scrollbar); `None` for display-only tables.
/// * `focused` — selects border/highlight brightness.
/// * `block`   — optional bordered block; its border style is set from `focused`.
#[allow(clippy::too_many_arguments)]
pub fn render_table(
    f: &mut Frame,
    area: Rect,
    columns: &[Column],
    rows: Vec<Vec<Line<'_>>>,
    state: Option<&mut TableState>,
    focused: bool,
    block: Option<Block<'_>>,
    theme: &Theme,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let block = block.map(|b| {
        let style = if focused {
            Style::default()
                .fg(theme.primary)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.dim)
        };
        b.border_style(style)
    });

    let inner = block.as_ref().map_or(area, |b| b.inner(area));
    let widths_resolved = resolved_widths(columns, inner);

    let header = Row::new(
        columns
            .iter()
            .map(|c| Cell::from(Line::from(c.header.clone()).alignment(c.align))),
    )
    .style(
        Style::default()
            .fg(theme.primary)
            .add_modifier(Modifier::BOLD),
    );

    let constraints: Vec<Constraint> = columns.iter().map(|c| c.constraint).collect();

    let table_rows: Vec<Row> = rows
        .into_iter()
        .map(|row| {
            let cells = ellipsize_row(columns, &widths_resolved, row)
                .into_iter()
                .enumerate()
                .map(|(i, line)| {
                    let align = columns.get(i).map_or(Alignment::Left, |c| c.align);
                    Cell::from(line.alignment(align))
                });
            Row::new(cells)
        })
        .collect();
    let row_count = table_rows.len();

    let highlight = if focused {
        Style::default()
            .bg(theme.primary)
            .fg(theme.bg)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(theme.primary)
            .add_modifier(Modifier::DIM)
    };

    let mut table = Table::new(table_rows, constraints)
        .header(header)
        .flex(Flex::Legacy)
        .column_spacing(COLUMN_SPACING)
        .row_highlight_style(highlight);
    if let Some(b) = block {
        table = table.block(b);
    }

    let visible_rows = usize::from(inner.height.saturating_sub(1)); // minus header

    match state {
        Some(st) => {
            f.render_stateful_widget(table, area, st);
            if row_count > visible_rows {
                let mut sb = ScrollbarState::default()
                    .content_length(row_count)
                    .position(st.offset());
                let scrollbar = Scrollbar::default()
                    .orientation(ScrollbarOrientation::VerticalRight)
                    .begin_symbol(None)
                    .end_symbol(None);
                f.render_stateful_widget(scrollbar, inner, &mut sb);
            }
        }
        None => {
            f.render_widget(table, area);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend, text::Line};

    fn cols() -> Vec<Column> {
        vec![
            Column::new("#", Constraint::Min(4)).right(),
            Column::new("Title", Constraint::Fill(1)).ellipsis(),
            Column::new("Author", Constraint::Min(8)),
        ]
    }

    fn rows() -> Vec<Vec<Line<'static>>> {
        vec![
            vec![
                Line::from("1"),
                Line::from("A very long pull request title that should ellipsize when narrow"),
                Line::from("octocat"),
            ],
            vec![Line::from("2"), Line::from("short"), Line::from("hubot")],
        ]
    }

    fn render_at(width: u16) -> String {
        let backend = TestBackend::new(width, 6);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let mut st = TableState::default();
                st.select(Some(0));
                render_table(
                    f,
                    f.area(),
                    &cols(),
                    rows(),
                    Some(&mut st),
                    true,
                    None,
                    &crate::ui::theme::Theme::dark(),
                );
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
    fn title_ellipsizes_when_narrow_but_not_when_wide() {
        let narrow = render_at(40);
        let wide = render_at(120);
        assert!(
            narrow.contains('…'),
            "narrow render should ellipsize:\n{narrow}"
        );
        assert!(
            !wide.contains('…'),
            "wide render should NOT ellipsize:\n{wide}"
        );
    }

    #[test]
    fn reflow_snapshots() {
        insta::assert_snapshot!("table_reflow_60", render_at(60));
        insta::assert_snapshot!("table_reflow_100", render_at(100));
        insta::assert_snapshot!("table_reflow_160", render_at(160));
    }
}
