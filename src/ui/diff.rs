//! Render a unified diff (the same text consumed by `DiffPositionIndex`) into
//! ratatui `Line`s with syntect-based syntax highlighting on body lines.
//!
//! Highlighting rules:
//! - File header lines (`diff --git`, `index`, `---`, `+++`) → dim foreground.
//! - Hunk header lines (`@@ ... @@`) → primary color.
//! - `+` body lines → gutter in `theme.success`; content syntax-highlighted.
//! - `-` body lines → gutter in `theme.error`; content syntax-highlighted.
//! - Context (` `) body lines → gutter in `theme.fg`; content syntax-highlighted.
//! - `\ No newline at end of file` → dim, un-highlighted.
//!
//! Syntect's `SyntaxSet` and `ThemeSet` are loaded once into `OnceLock`s.

use ratatui::layout::Rect;
use ratatui::{
    Frame,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};
use syntect::easy::HighlightLines;

use crate::data::models::FileDiff;
use crate::ui::highlight::{highlight_one, syntax_for_path, syntax_set, syntect_theme_for};
use crate::ui::theme::Theme;

/// Above this patch size, syntect highlighting is skipped but content is still rendered.
pub const HIGHLIGHT_CEILING_BYTES: usize = 50 * 1024;
/// Above this patch size, the patch is not rendered at all — only a placeholder.
pub const RENDER_CEILING_BYTES: usize = 250 * 1024;

/// Render the unified diff text into a `Vec<Line>` suitable for a
/// `Paragraph` or scrollable widget.
#[must_use]
pub fn render_diff_lines<'a>(diff: &'a str, path_hint: &str, theme: &Theme) -> Vec<Line<'a>> {
    let set = syntax_set();
    let syn_theme = syntect_theme_for(theme);
    let mut current_syntax = syntax_for_path(set, path_hint);
    let mut highlighter = HighlightLines::new(current_syntax, syn_theme);

    let mut out = Vec::with_capacity(diff.lines().count());

    for raw in diff.split_inclusive('\n') {
        let line = raw.strip_suffix('\n').unwrap_or(raw);

        // Switch syntax when a new file starts.
        if let Some(rest) = line.strip_prefix("+++ ") {
            if let Some(p) = parse_marker_path(rest) {
                current_syntax = syntax_for_path(set, &p);
                highlighter = HighlightLines::new(current_syntax, syn_theme);
            }
            out.push(header_line(line, theme));
            continue;
        }

        if line.starts_with("diff --git ")
            || line.starts_with("index ")
            || line.starts_with("--- ")
            || line.starts_with("new file ")
            || line.starts_with("deleted file ")
            || line.starts_with("similarity ")
            || line.starts_with("rename ")
        {
            out.push(header_line(line, theme));
            continue;
        }

        if line.starts_with("@@") {
            out.push(Line::styled(
                line.to_string(),
                Style::default()
                    .fg(theme.primary)
                    .add_modifier(Modifier::BOLD),
            ));
            continue;
        }

        if line.starts_with('\\') {
            out.push(Line::styled(
                line.to_string(),
                Style::default().fg(theme.dim),
            ));
            continue;
        }

        let first = line.chars().next();
        let (gutter_char, gutter_color, body) = match first {
            Some('+') => ('+', theme.success, &line[1..]),
            Some('-') => ('-', theme.error, &line[1..]),
            Some(' ') => (' ', theme.fg, &line[1..]),
            _ => {
                out.push(Line::raw(line.to_string()));
                continue;
            }
        };

        let colored_spans = highlight_one(&mut highlighter, set, body);
        let mut spans = Vec::with_capacity(colored_spans.len() + 1);
        spans.push(Span::styled(
            gutter_char.to_string(),
            Style::default()
                .fg(gutter_color)
                .add_modifier(Modifier::BOLD),
        ));
        for (style, text) in colored_spans {
            spans.push(Span::styled(text, style));
        }
        out.push(Line::from(spans));
    }

    out
}

fn header_line(line: &str, theme: &Theme) -> Line<'static> {
    Line::styled(line.to_string(), Style::default().fg(theme.dim))
}

/// Render one `FileDiff` into `Vec<Line>`, applying size ceilings:
/// - `file.oversize` or missing patch → placeholder, no patch text rendered.
/// - patch length > `RENDER_CEILING_BYTES` → same placeholder.
/// - patch length > `HIGHLIGHT_CEILING_BYTES` → content rendered without syntect highlighting.
/// - otherwise → fully highlighted via [`render_diff_lines`].
#[must_use]
pub fn render_file_diff<'a>(file: &'a FileDiff, theme: &Theme) -> Vec<Line<'a>> {
    render_file_diff_with(file, theme, false)
}

/// Same as [`render_file_diff`] but with an optional whitespace-only filter.
/// When `whitespace_hidden` is true, paired `-`/`+` body lines whose
/// whitespace-collapsed contents are equal are dropped from the rendered list.
#[must_use]
pub fn render_file_diff_with<'a>(
    file: &'a FileDiff,
    theme: &Theme,
    whitespace_hidden: bool,
) -> Vec<Line<'a>> {
    if file.oversize {
        return vec![too_large_placeholder(theme)];
    }
    let Some(patch) = file.patch.as_deref() else {
        return vec![Line::styled(
            "No diff available.",
            Style::default().fg(theme.dim),
        )];
    };
    if patch.len() > RENDER_CEILING_BYTES {
        return vec![too_large_placeholder(theme)];
    }
    let filtered;
    let effective_patch: &str = if whitespace_hidden {
        filtered = strip_whitespace_only_pairs(patch);
        &filtered
    } else {
        patch
    };
    if effective_patch.len() > HIGHLIGHT_CEILING_BYTES {
        return render_diff_lines_plain(effective_patch, theme)
            .into_iter()
            .map(into_static_line)
            .collect();
    }
    render_diff_lines(effective_patch, &file.path, theme)
        .into_iter()
        .map(into_static_line)
        .collect()
}

fn into_static_line(line: Line<'_>) -> Line<'static> {
    let spans: Vec<Span<'static>> = line
        .spans
        .into_iter()
        .map(|s| Span::styled(s.content.into_owned(), s.style))
        .collect();
    Line::from(spans).style(line.style)
}

/// Drop paired `-`/`+` lines whose contents are equal after collapsing
/// whitespace. Used by the diff viewer's `w` toggle.
fn strip_whitespace_only_pairs(patch: &str) -> String {
    let mut out = String::with_capacity(patch.len());
    let lines: Vec<&str> = patch.split_inclusive('\n').collect();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let body = line.strip_suffix('\n').unwrap_or(line);
        if body.starts_with('-') && i + 1 < lines.len() {
            let next = lines[i + 1];
            let next_body = next.strip_suffix('\n').unwrap_or(next);
            if next_body.starts_with('+') && whitespace_equal(&body[1..], &next_body[1..]) {
                i += 2;
                continue;
            }
        }
        out.push_str(line);
        i += 1;
    }
    out
}

fn whitespace_equal(a: &str, b: &str) -> bool {
    a.split_whitespace().eq(b.split_whitespace())
}

fn too_large_placeholder(theme: &Theme) -> Line<'static> {
    Line::styled(
        "Diff too large, press `o` to open in browser.",
        Style::default().fg(theme.warning),
    )
}

/// Render `file`'s diff into `area`, inverting the cursor line at `line_idx`.
///
/// Applies the same size ceilings as [`render_file_diff`]; the cursor is a
/// no-op when the placeholder branch fires (cursor is clamped to 0).
/// `whitespace_hidden=true` drops paired `-`/`+` whitespace-only lines.
/// `selection`, if `Some((start, end))`, paints an inclusive range with a
/// reversed background to indicate line-select mode.
/// `diff_mode` switches between unified rendering and a two-column
/// side-by-side layout that pairs `-`/`+` lines in encounter order.
/// Areas narrower than [`SIDE_BY_SIDE_MIN_WIDTH`] silently fall back to
/// unified for that render call.
#[allow(clippy::too_many_arguments)]
pub fn render_diff_view(
    f: &mut Frame,
    area: Rect,
    file: &FileDiff,
    line_idx: usize,
    whitespace_hidden: bool,
    selection: Option<(usize, usize)>,
    diff_mode: crate::app::state::DiffMode,
    theme: &Theme,
) {
    let mut lines = render_file_diff_with(file, theme, whitespace_hidden);
    if let Some((start, end)) = selection {
        let select_style = Style::default()
            .bg(theme.dim)
            .fg(theme.bg)
            .add_modifier(Modifier::BOLD);
        for i in start..=end.min(lines.len().saturating_sub(1)) {
            let original = std::mem::take(&mut lines[i]);
            let restyled: Vec<Span<'_>> = original
                .spans
                .into_iter()
                .map(|s| Span::styled(s.content, select_style))
                .collect();
            let mut new_line = Line::from(restyled);
            new_line.style = select_style;
            lines[i] = new_line;
        }
    }
    if line_idx < lines.len() {
        let cursor_style = Style::default()
            .bg(theme.primary)
            .fg(theme.bg)
            .add_modifier(Modifier::BOLD);
        let original = std::mem::take(&mut lines[line_idx]);
        let restyled: Vec<Span<'_>> = original
            .spans
            .into_iter()
            .map(|s| Span::styled(s.content, cursor_style))
            .collect();
        let mut new_line = Line::from(restyled);
        new_line.style = cursor_style;
        lines[line_idx] = new_line;
    }
    let final_lines = if diff_mode == crate::app::state::DiffMode::SideBySide
        && area.width >= SIDE_BY_SIDE_MIN_WIDTH
    {
        to_side_by_side(lines, area.width)
    } else {
        lines
    };
    f.render_widget(Paragraph::new(final_lines), area);
}

fn render_diff_lines_plain<'a>(diff: &'a str, theme: &Theme) -> Vec<Line<'a>> {
    let mut out = Vec::with_capacity(diff.lines().count());
    for raw in diff.split_inclusive('\n') {
        let line = raw.strip_suffix('\n').unwrap_or(raw);
        if line.starts_with("@@") {
            out.push(Line::styled(
                line.to_string(),
                Style::default()
                    .fg(theme.primary)
                    .add_modifier(Modifier::BOLD),
            ));
            continue;
        }
        let (gutter_color, _body) = match line.chars().next() {
            Some('+') => (theme.success, &line[1..]),
            Some('-') => (theme.error, &line[1..]),
            Some(' ') => (theme.fg, &line[1..]),
            Some('\\') => (theme.dim, line),
            _ => {
                out.push(Line::styled(
                    line.to_string(),
                    Style::default().fg(theme.dim),
                ));
                continue;
            }
        };
        out.push(Line::styled(
            line.to_string(),
            Style::default().fg(gutter_color),
        ));
    }
    out
}

fn parse_marker_path(rest: &str) -> Option<String> {
    let trimmed = rest.trim();
    if trimmed == "/dev/null" {
        return None;
    }
    trimmed.strip_prefix("b/").map(str::to_string)
}

pub const SIDE_BY_SIDE_MIN_WIDTH: u16 = 20;

fn to_side_by_side<'a>(unified: Vec<Line<'a>>, area_width: u16) -> Vec<Line<'a>> {
    let column_width = usize::from(area_width) / 2;
    let gutter = " ";
    let mut out: Vec<Line<'a>> = Vec::with_capacity(unified.len());
    let mut pending_minus: Vec<Line<'a>> = Vec::new();

    let flush = |pending: &mut Vec<Line<'a>>,
                 pluses: &mut Vec<Line<'a>>,
                 out: &mut Vec<Line<'a>>,
                 column_width: usize,
                 gutter: &'static str| {
        let pair_count = pending.len().max(pluses.len());
        let mut p_iter = std::mem::take(pending).into_iter();
        let mut a_iter = std::mem::take(pluses).into_iter();
        for _ in 0..pair_count {
            let left = p_iter.next().unwrap_or_default();
            let right = a_iter.next().unwrap_or_default();
            out.push(join_columns(left, right, column_width, gutter));
        }
    };

    let mut pending_plus: Vec<Line<'a>> = Vec::new();
    for line in unified {
        match classify_line(&line) {
            LineKind::Deletion => pending_minus.push(line),
            LineKind::Addition => pending_plus.push(line),
            LineKind::FullWidth => {
                flush(
                    &mut pending_minus,
                    &mut pending_plus,
                    &mut out,
                    column_width,
                    gutter,
                );
                out.push(line);
            }
            LineKind::Context => {
                flush(
                    &mut pending_minus,
                    &mut pending_plus,
                    &mut out,
                    column_width,
                    gutter,
                );
                out.push(join_columns(line.clone(), line, column_width, gutter));
            }
        }
    }
    flush(
        &mut pending_minus,
        &mut pending_plus,
        &mut out,
        column_width,
        gutter,
    );
    out
}

#[derive(Debug)]
enum LineKind {
    Deletion,
    Addition,
    Context,
    FullWidth,
}

fn classify_line(line: &Line<'_>) -> LineKind {
    let Some(first) = line.spans.first() else {
        return LineKind::FullWidth;
    };
    let content = first.content.as_ref();
    if content.starts_with("@@")
        || content.starts_with("diff ")
        || content.starts_with("--- ")
        || content.starts_with("+++ ")
        || content.starts_with("index ")
        || content.starts_with('\\')
    {
        return LineKind::FullWidth;
    }
    match content.chars().next() {
        Some('-') => LineKind::Deletion,
        Some('+') => LineKind::Addition,
        _ => LineKind::Context,
    }
}

fn join_columns<'a>(
    left: Line<'a>,
    right: Line<'a>,
    column_width: usize,
    gutter: &'static str,
) -> Line<'a> {
    let mut spans: Vec<Span<'a>> = Vec::new();
    let mut left_width = 0usize;
    for s in left.spans {
        let remaining = column_width.saturating_sub(left_width);
        if remaining == 0 {
            break;
        }
        let truncated = truncate_span(s, remaining);
        left_width += truncated.content.chars().count();
        spans.push(truncated);
    }
    if left_width < column_width {
        spans.push(Span::raw(" ".repeat(column_width - left_width)));
    }
    spans.push(Span::raw(gutter));
    let mut right_width = 0usize;
    for s in right.spans {
        let remaining = column_width.saturating_sub(right_width);
        if remaining == 0 {
            break;
        }
        let truncated = truncate_span(s, remaining);
        right_width += truncated.content.chars().count();
        spans.push(truncated);
    }
    Line::from(spans)
}

fn truncate_span(span: Span<'_>, max_chars: usize) -> Span<'_> {
    if span.content.chars().count() <= max_chars {
        return span;
    }
    let truncated: String = span.content.chars().take(max_chars).collect();
    Span::styled(truncated, span.style)
}

#[cfg(test)]
mod tests {
    use ratatui::{Terminal, backend::TestBackend, widgets::Paragraph};

    use super::*;

    #[test]
    fn renders_small_rust_diff_with_highlighted_added_line() {
        let diff = "\
diff --git a/src/lib.rs b/src/lib.rs
index 1111111..2222222 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,3 +1,4 @@
 fn main() {
     let x = 1;
+    let y = 2;
 }
";
        let theme = Theme::dark();
        let lines = render_diff_lines(diff, "src/lib.rs", &theme);

        // The added line is the 8th rendered line.
        let added = &lines[7];
        // Gutter span is `+` colored in success.
        let gutter = &added.spans[0];
        assert_eq!(gutter.content, "+");
        assert_eq!(gutter.style.fg, Some(theme.success));
        // Body has at least one highlighted span and the joined body
        // text matches the original (sans gutter).
        let body_text: String = added
            .spans
            .iter()
            .skip(1)
            .map(|s| s.content.as_ref())
            .collect();
        assert_eq!(body_text, "    let y = 2;");
        assert!(
            added.spans.iter().skip(1).any(|s| s.style.fg.is_some()),
            "expected at least one syntect-styled span on the added line"
        );

        // Hunk header is bold + primary.
        let hunk = &lines[4];
        assert_eq!(hunk.style.fg, Some(theme.primary));

        // Snapshot the full render to lock the look.
        let backend = TestBackend::new(60, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let para = Paragraph::new(lines.clone());
                let area = f.area();
                f.render_widget(para, area);
            })
            .unwrap();
        insta::assert_snapshot!("renders_small_rust_diff", terminal.backend());
    }

    use crate::data::models::FileStatus;

    fn file_diff_with_patch(patch: String, oversize: bool) -> FileDiff {
        FileDiff {
            path: "src/big.rs".into(),
            previous_path: None,
            status: FileStatus::Modified,
            additions: 1,
            deletions: 0,
            patch: if oversize { None } else { Some(patch) },
            is_binary: false,
            is_submodule: false,
            is_generated: None,
            oversize,
            blob_sha_before: None,
            blob_sha_after: None,
        }
    }

    fn synthesize_patch_of_size(bytes: usize) -> String {
        let mut out = String::from("@@ -1,1 +1,1 @@\n");
        while out.len() < bytes {
            out.push_str("+aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n");
        }
        out
    }

    #[test]
    fn patch_above_highlight_ceiling_renders_without_syntect_styling() {
        let patch = synthesize_patch_of_size(HIGHLIGHT_CEILING_BYTES + 1024);
        assert!(patch.len() > HIGHLIGHT_CEILING_BYTES);
        assert!(patch.len() < RENDER_CEILING_BYTES);

        let file = file_diff_with_patch(patch.clone(), false);
        let lines = render_file_diff(&file, &Theme::dark());

        assert_eq!(lines.len(), patch.split_inclusive('\n').count());

        let body_line = lines
            .iter()
            .find(|l| l.spans.len() == 1 && l.spans[0].content.starts_with('+'))
            .expect("expected at least one + body line");
        assert_eq!(
            body_line.spans.len(),
            1,
            "plain render emits one span per line — got {}",
            body_line.spans.len()
        );

        let highlighted = render_diff_lines(&patch, "src/big.rs", &Theme::dark());
        let highlighted_added = highlighted
            .iter()
            .find(|l| l.spans.first().is_some_and(|s| s.content.as_ref() == "+"))
            .expect("highlighted render should have a + line");
        assert!(
            highlighted_added.spans.len() > 1,
            "syntect render should split into multiple spans"
        );
    }

    #[test]
    fn oversize_file_renders_too_large_placeholder_and_does_not_load_patch() {
        let file = file_diff_with_patch(String::new(), true);
        assert!(
            file.patch.is_none(),
            "oversize fixture must not carry patch text"
        );

        let lines = render_file_diff(&file, &Theme::dark());

        assert_eq!(lines.len(), 1);
        let text: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "Diff too large, press `o` to open in browser.");
        assert_eq!(lines[0].style.fg, Some(Theme::dark().warning));
    }

    #[test]
    fn patch_above_render_ceiling_renders_too_large_placeholder() {
        let patch = synthesize_patch_of_size(RENDER_CEILING_BYTES + 1024);
        let file = file_diff_with_patch(patch, false);

        let lines = render_file_diff(&file, &Theme::dark());

        assert_eq!(lines.len(), 1);
        let text: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "Diff too large, press `o` to open in browser.");
    }

    #[test]
    fn renders_diff_view_with_cursor_on_added_line() {
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\nindex 1111111..2222222 100644\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,3 +1,4 @@\n fn main() {\n     let x = 1;\n+    let y = 2;\n }\n".to_string();
        let mut file = file_diff_with_patch(patch, false);
        file.path = "src/lib.rs".into();

        let backend = TestBackend::new(60, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render_diff_view(
                    f,
                    area,
                    &file,
                    7,
                    false,
                    None,
                    crate::app::state::DiffMode::Unified,
                    &Theme::dark(),
                );
            })
            .unwrap();
        insta::assert_snapshot!("renders_diff_view_cursor_on_added", terminal.backend());
    }

    #[test]
    fn renders_diff_view_with_line_selection_range_highlighted() {
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\nindex 1111111..2222222 100644\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,3 +1,4 @@\n fn main() {\n     let x = 1;\n+    let y = 2;\n }\n".to_string();
        let mut file = file_diff_with_patch(patch, false);
        file.path = "src/lib.rs".into();

        let theme = Theme::dark();
        let backend = TestBackend::new(60, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render_diff_view(
                    f,
                    area,
                    &file,
                    0,
                    false,
                    Some((5, 7)),
                    crate::app::state::DiffMode::Unified,
                    &theme,
                );
            })
            .unwrap();

        let buf = terminal.backend().buffer();
        let select_bg = theme.dim;
        let row_has_select_bg = |row: u16| -> bool {
            (0..buf.area.width).any(|x| buf[(x, row)].style().bg == Some(select_bg))
        };

        assert!(
            row_has_select_bg(5),
            "row 5 (selection start) should be highlighted"
        );
        assert!(
            row_has_select_bg(6),
            "row 6 (middle of selection) should be highlighted"
        );
        assert!(
            row_has_select_bg(7),
            "row 7 (selection end, inclusive) should be highlighted"
        );
        assert!(
            !row_has_select_bg(4),
            "row 4 (above selection) must not be highlighted"
        );
        assert!(
            !row_has_select_bg(8),
            "row 8 (below selection) must not be highlighted"
        );
    }

    #[test]
    fn whitespace_hidden_drops_paired_whitespace_only_change_but_keeps_real_changes() {
        let patch = "diff --git a/x.rs b/x.rs\n--- a/x.rs\n+++ b/x.rs\n@@ -1,4 +1,4 @@\n ctx\n-foo  bar\n+foo bar\n-old\n+new\n".to_string();
        let mut file = file_diff_with_patch(patch, false);
        file.path = "x.rs".into();

        let without_filter = render_file_diff_with(&file, &Theme::dark(), false);
        let with_filter = render_file_diff_with(&file, &Theme::dark(), true);

        let count_marker = |lines: &[Line<'_>], mark: &str| -> usize {
            lines
                .iter()
                .filter(|l| l.spans.first().is_some_and(|s| s.content.as_ref() == mark))
                .count()
        };

        assert_eq!(
            count_marker(&without_filter, "-"),
            2,
            "unfiltered: two - lines"
        );
        assert_eq!(
            count_marker(&without_filter, "+"),
            2,
            "unfiltered: two + lines"
        );
        assert_eq!(
            count_marker(&with_filter, "-"),
            1,
            "filtered: ws-only - dropped"
        );
        assert_eq!(
            count_marker(&with_filter, "+"),
            1,
            "filtered: ws-only + dropped"
        );

        let minus_body: String = with_filter
            .iter()
            .find(|l| l.spans.first().is_some_and(|s| s.content.as_ref() == "-"))
            .unwrap()
            .spans
            .iter()
            .skip(1)
            .map(|s| s.content.as_ref())
            .collect();
        assert_eq!(minus_body, "old", "real change retained, not ws-only");
    }

    fn small_two_change_diff() -> FileDiff {
        let patch = "diff --git a/x.rs b/x.rs\nindex 1111111..2222222 100644\n--- a/x.rs\n+++ b/x.rs\n@@ -1,3 +1,3 @@\n ctx\n-old\n+new\n".to_string();
        let mut file = file_diff_with_patch(patch, false);
        file.path = "x.rs".into();
        file
    }

    #[test]
    fn renders_diff_view_unified_mode_snapshot() {
        use crate::app::state::DiffMode;
        let file = small_two_change_diff();
        let backend = TestBackend::new(60, 8);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render_diff_view(
                    f,
                    area,
                    &file,
                    0,
                    false,
                    None,
                    DiffMode::Unified,
                    &Theme::dark(),
                );
            })
            .unwrap();
        insta::assert_snapshot!("renders_diff_view_unified_mode", terminal.backend());
    }

    #[test]
    fn renders_diff_view_side_by_side_mode_places_deletion_left_addition_right() {
        use crate::app::state::DiffMode;
        let file = small_two_change_diff();
        let backend = TestBackend::new(60, 8);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render_diff_view(
                    f,
                    area,
                    &file,
                    0,
                    false,
                    None,
                    DiffMode::SideBySide,
                    &Theme::dark(),
                );
            })
            .unwrap();

        let buf = terminal.backend().buffer();
        let row_text = |row: u16| -> String {
            (0..buf.area.width)
                .map(|x| buf[(x, row)].symbol())
                .collect::<String>()
        };
        let split = buf.area.width as usize / 2;

        let mut paired_row: Option<u16> = None;
        for row in 0..buf.area.height {
            let text = row_text(row);
            let left = &text[..split.min(text.len())];
            let right = if text.len() > split {
                &text[split..]
            } else {
                ""
            };
            if left.contains("-old") && right.contains("+new") {
                paired_row = Some(row);
                break;
            }
        }
        assert!(
            paired_row.is_some(),
            "expected a row with '-old' on the left half and '+new' on the right half; got buffer:\n{}",
            (0..buf.area.height)
                .map(row_text)
                .collect::<Vec<_>>()
                .join("\n")
        );

        insta::assert_snapshot!("renders_diff_view_side_by_side_mode", terminal.backend());
    }

    #[test]
    fn renders_diff_view_side_by_side_narrow_terminal_falls_back_without_panic() {
        use crate::app::state::DiffMode;
        let file = small_two_change_diff();
        let backend = TestBackend::new(18, 6);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render_diff_view(
                    f,
                    area,
                    &file,
                    0,
                    false,
                    None,
                    DiffMode::SideBySide,
                    &Theme::dark(),
                );
            })
            .unwrap();
    }
}
