//! Two-pane Files split: file-tree on the left, diff of the selected file on
//! the right. Used by the Files sub-tab when the terminal is wide enough
//! (width ≥ `FILES_SPLIT_MIN_WIDTH`).

use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState},
};

use crate::app::state::{DiffMode, DiffSelection, FilesPane};
use crate::data::models::{FileDiff, FileStatus, PrFiles};
use crate::ui::theme::Theme;

/// Input parameters for the right-pane diff rendering.
#[derive(Debug)]
pub struct DiffViewInputs<'a> {
    /// The file whose diff is shown (if `None`, shows a placeholder).
    pub file: Option<&'a FileDiff>,
    /// Line-scroll offset for the diff.
    pub diff_cursor: usize,
    /// Whether to filter whitespace-only changes.
    pub whitespace_hidden: bool,
    /// Active line selection (inclusive range).
    pub diff_selection: Option<DiffSelection>,
    /// Unified vs side-by-side mode.
    pub diff_mode: DiffMode,
}

/// Selection-highlight style for a file-list row: a solid bar when the tree
/// pane is focused, a dim accent otherwise.
fn list_highlight(focused: bool, theme: &Theme) -> Style {
    if focused {
        Style::default()
            .bg(theme.primary)
            .fg(theme.bg)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(theme.primary)
            .add_modifier(Modifier::DIM)
    }
}

/// The active pane (`files_focus`) gets a primary-colored border; the other gets
/// a dim border so focus is always visible.
#[allow(clippy::too_many_arguments)]
pub fn render_files_split(
    f: &mut Frame,
    area: Rect,
    files: &PrFiles,
    selected: usize,
    files_focus: FilesPane,
    diff_inputs: &DiffViewInputs<'_>,
    theme: &Theme,
) {
    // Geometry: left pane = 36% of width, clamped to at least 24 cols.
    let min_left: u16 = 24;
    let left_width = u16::try_from(
        ((u32::from(area.width) * 36) / 100)
            .max(u32::from(min_left))
            .min(u32::from(area.width.saturating_sub(min_left))),
    )
    .unwrap_or(min_left);

    let [left_area, right_area] =
        Layout::horizontal([Constraint::Length(left_width), Constraint::Min(0)]).areas(area);

    let (left_border_style, right_border_style) = match files_focus {
        FilesPane::Tree => (
            Style::default()
                .fg(theme.primary)
                .add_modifier(Modifier::BOLD),
            Style::default().fg(theme.dim),
        ),
        FilesPane::Diff => (
            Style::default().fg(theme.dim),
            Style::default()
                .fg(theme.primary)
                .add_modifier(Modifier::BOLD),
        ),
    };

    // ── Left pane: file list ──────────────────────────────────────────────────
    let left_block = Block::default()
        .borders(Borders::ALL)
        .border_type(theme.border_type())
        .border_style(left_border_style)
        .title(Span::styled(" Files ", Style::default().fg(theme.fg)));
    let left_inner = left_block.inner(left_area);
    f.render_widget(left_block, left_area);

    if files.files.is_empty() {
        let msg = Line::styled("No files.", Style::default().fg(theme.dim));
        f.render_widget(msg, left_inner);
    } else {
        let items: Vec<ListItem> = files
            .files
            .iter()
            .map(|file| {
                let icon = match file.status {
                    FileStatus::Added => 'A',
                    FileStatus::Removed => 'D',
                    FileStatus::Modified => 'M',
                    FileStatus::Renamed => 'R',
                    FileStatus::Copied => 'C',
                    FileStatus::ChangedMode => 'X',
                    FileStatus::Unmerged => 'U',
                };
                // Truncate path to fit pane width (inner width minus gutter).
                let inner_w = usize::from(left_inner.width).saturating_sub(3);
                let path = crate::ui::text::truncate_to_width(&file.path, inner_w);
                let line = Line::from(vec![
                    Span::styled(
                        format!("{icon} "),
                        Style::default().fg(match file.status {
                            FileStatus::Added => theme.success,
                            FileStatus::Removed => theme.error,
                            _ => theme.fg,
                        }),
                    ),
                    Span::raw(path),
                ]);
                ListItem::new(line)
            })
            .collect();

        let list = List::new(items).highlight_style(list_highlight(
            matches!(files_focus, FilesPane::Tree),
            theme,
        ));
        let mut state = ListState::default();
        state.select(Some(selected.min(files.files.len().saturating_sub(1))));
        f.render_stateful_widget(list, left_inner, &mut state);
    }

    // ── Right pane: diff ──────────────────────────────────────────────────────
    let right_block = Block::default()
        .borders(Borders::ALL)
        .border_type(theme.border_type())
        .border_style(right_border_style)
        .title(Span::styled(" Diff ", Style::default().fg(theme.fg)));
    let right_inner = right_block.inner(right_area);
    f.render_widget(right_block, right_area);

    match diff_inputs.file {
        None => {
            let msg = Line::styled("No file selected.", Style::default().fg(theme.dim));
            f.render_widget(msg, right_inner);
        }
        Some(file) => {
            let selection = diff_inputs.diff_selection.map(|s| s.range());
            crate::ui::diff::render_diff_view(
                f,
                right_inner,
                file,
                diff_inputs.diff_cursor,
                diff_inputs.whitespace_hidden,
                selection,
                diff_inputs.diff_mode,
                theme,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::models::{FileDiff, FileStatus, PrFiles};
    use crate::ui::theme::Theme;
    use ratatui::{Terminal, backend::TestBackend};

    fn make_files() -> PrFiles {
        PrFiles {
            truncated: false,
            files: vec![
                FileDiff {
                    path: "src/main.rs".into(),
                    previous_path: None,
                    status: FileStatus::Modified,
                    additions: 3,
                    deletions: 1,
                    patch: Some(
                        "diff --git a/src/main.rs b/src/main.rs\nindex 1111111..2222222 100644\n--- a/src/main.rs\n+++ b/src/main.rs\n@@ -1,3 +1,4 @@\n fn main() {\n-    let x = 1;\n+    let x = 2;\n+    let y = 3;\n }\n"
                            .into(),
                    ),
                    is_binary: false,
                    is_submodule: false,
                    is_generated: None,
                    oversize: false,
                    blob_sha_before: None,
                    blob_sha_after: None,
                },
                FileDiff {
                    path: "src/lib.rs".into(),
                    previous_path: None,
                    status: FileStatus::Added,
                    additions: 5,
                    deletions: 0,
                    patch: Some(
                        "diff --git a/src/lib.rs b/src/lib.rs\nindex 0000000..1111111 100644\n--- /dev/null\n+++ b/src/lib.rs\n@@ -0,0 +1,5 @@\n+pub fn foo() {}\n"
                            .into(),
                    ),
                    is_binary: false,
                    is_submodule: false,
                    is_generated: None,
                    oversize: false,
                    blob_sha_before: None,
                    blob_sha_after: None,
                },
            ],
        }
    }

    #[test]
    fn renders_files_split_tree_focused() {
        let files = make_files();
        let selected = 0;
        let diff_inputs = DiffViewInputs {
            file: files.files.first(),
            diff_cursor: 0,
            whitespace_hidden: false,
            diff_selection: None,
            diff_mode: DiffMode::Unified,
        };

        let backend = TestBackend::new(120, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render_files_split(
                    f,
                    area,
                    &files,
                    selected,
                    FilesPane::Tree,
                    &diff_inputs,
                    &Theme::dark(),
                );
            })
            .unwrap();
        insta::assert_snapshot!("renders_files_split_tree_focused", terminal.backend());
    }

    #[test]
    fn renders_files_split_diff_focused() {
        let files = make_files();
        let selected = 1;
        let diff_inputs = DiffViewInputs {
            file: files.files.get(1),
            diff_cursor: 0,
            whitespace_hidden: false,
            diff_selection: None,
            diff_mode: DiffMode::Unified,
        };

        let backend = TestBackend::new(120, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render_files_split(
                    f,
                    area,
                    &files,
                    selected,
                    FilesPane::Diff,
                    &diff_inputs,
                    &Theme::dark(),
                );
            })
            .unwrap();
        insta::assert_snapshot!("renders_files_split_diff_focused", terminal.backend());
    }
}
