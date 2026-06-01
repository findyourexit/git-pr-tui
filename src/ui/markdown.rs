//! Markdown → `Vec<Line<'static>>` renderer using pulldown-cmark and syntect.

use std::cell::RefCell;
use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};
use syntect::easy::HighlightLines;
use unicode_width::UnicodeWidthStr;

use crate::ui::{
    highlight::{highlight_one, syntax_for_token, syntax_set, syntect_theme_for},
    theme::Theme,
};

const MD_CACHE_CAP: usize = 256;

thread_local! {
    static MD_CACHE: RefCell<HashMap<u64, Vec<Line<'static>>>> = RefCell::new(HashMap::new());
}

fn cache_key(body: &str, width: u16, theme: &Theme) -> u64 {
    let mut h = DefaultHasher::new();
    body.hash(&mut h);
    body.len().hash(&mut h);
    width.hash(&mut h);
    for c in [
        theme.primary,
        theme.secondary,
        theme.success,
        theme.warning,
        theme.error,
        theme.dim,
        theme.bg,
        theme.fg,
    ] {
        c.hash(&mut h);
    }
    h.finish()
}

/// Render Markdown into themed, width-wrapped lines. Memoized per
/// `(body, width, theme)`; the cache is invisible to output.
#[must_use]
pub fn render_markdown(body: &str, width: u16, theme: &Theme) -> Vec<Line<'static>> {
    let key = cache_key(body, width, theme);
    MD_CACHE.with(|cell| {
        if let Some(hit) = cell.borrow().get(&key).cloned() {
            return hit;
        }
        let lines = render_markdown_uncached(body, width, theme);
        let mut cache = cell.borrow_mut();
        if cache.len() >= MD_CACHE_CAP {
            cache.clear();
        }
        cache.insert(key, lines.clone());
        lines
    })
}

#[cfg(test)]
pub(crate) fn clear_markdown_cache() {
    MD_CACHE.with(|c| c.borrow_mut().clear());
}

fn render_markdown_uncached(body: &str, width: u16, theme: &Theme) -> Vec<Line<'static>> {
    let w = usize::from(width);
    let opts = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let parser = Parser::new_ext(body, opts);

    let mut state = RenderState::new(theme, w);
    for event in parser {
        state.process(event);
    }
    state.finish()
}

// ---------------------------------------------------------------------------
// Internal state
// ---------------------------------------------------------------------------

// Several independent mode flags (in code block / image / table / table head)
// drive a single-pass event walker; grouping them adds indirection without
// clarity for a self-contained render state machine.
#[allow(clippy::struct_excessive_bools)]
struct RenderState<'t> {
    theme: &'t Theme,
    width: usize,
    lines: Vec<Line<'static>>,
    /// Current inline spans being accumulated.
    cur_spans: Vec<Span<'static>>,
    /// Inline style stack (bold, italic, strikethrough, link, etc.).
    style_stack: Vec<Style>,
    /// Current computed inline style.
    cur_style: Style,

    // Block-level state
    list_stack: Vec<ListKind>,
    blockquote_depth: usize,
    heading_level: Option<HeadingLevel>,

    // Fenced code block
    in_code_block: bool,
    code_lang: Option<String>,
    code_lines: Vec<String>,

    // Link state
    link_url: Option<String>,

    // Image state
    in_image: bool,
    image_url: Option<String>,

    // Table state
    in_table: bool,
    in_table_head: bool,
    table_rows: Vec<Vec<Vec<Span<'static>>>>,
    table_cur_row: Vec<Vec<Span<'static>>>,

    // Item marker (may be overridden by task list marker)
    pending_item_marker: Option<String>,
}

#[derive(Clone)]
enum ListKind {
    Unordered,
    Ordered(u64),
}

impl<'t> RenderState<'t> {
    fn new(theme: &'t Theme, width: usize) -> Self {
        Self {
            theme,
            width,
            lines: Vec::new(),
            cur_spans: Vec::new(),
            style_stack: Vec::new(),
            cur_style: Style::default(),
            list_stack: Vec::new(),
            blockquote_depth: 0,
            heading_level: None,
            in_code_block: false,
            code_lang: None,
            code_lines: Vec::new(),
            link_url: None,
            in_image: false,
            image_url: None,
            in_table: false,
            in_table_head: false,
            table_rows: Vec::new(),
            table_cur_row: Vec::new(),
            pending_item_marker: None,
        }
    }

    fn process(&mut self, event: Event<'_>) {
        match event {
            Event::Start(tag) => self.start_tag(tag),
            Event::End(tag) => self.end_tag(tag),
            Event::Text(text) => self.handle_text(&text),
            Event::Code(code) => self.handle_inline_code(&code),
            Event::SoftBreak => self.handle_soft_break(),
            Event::HardBreak => self.handle_hard_break(),
            Event::Rule => self.handle_rule(),
            Event::TaskListMarker(checked) => {
                // Override the pending bullet/number marker
                let marker = if checked { "☑ " } else { "☐ " };
                self.pending_item_marker = Some(marker.to_string());
            }
            _ => {}
        }
    }

    fn finish(mut self) -> Vec<Line<'static>> {
        self.flush_line();
        self.lines
    }

    // -- Tag start ----------------------------------------------------------

    fn start_tag(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Heading { level, .. } => {
                self.heading_level = Some(level);
            }
            Tag::Strong => self.push_modifier(Modifier::BOLD),
            Tag::Emphasis => self.push_modifier(Modifier::ITALIC),
            Tag::Strikethrough => {
                let s = Style::default()
                    .add_modifier(Modifier::CROSSED_OUT)
                    .fg(self.theme.dim);
                self.style_stack.push(s);
                self.recompute_style();
            }
            Tag::BlockQuote(_) => {
                self.blockquote_depth += 1;
            }
            Tag::CodeBlock(kind) => {
                let lang = match kind {
                    CodeBlockKind::Fenced(info) => {
                        let token = info.split_whitespace().next().unwrap_or("").to_string();
                        if token.is_empty() { None } else { Some(token) }
                    }
                    CodeBlockKind::Indented => None,
                };
                self.in_code_block = true;
                self.code_lang = lang;
                self.code_lines = Vec::new();
            }
            Tag::List(start) => {
                self.flush_paragraph();
                let kind = match start {
                    Some(n) => ListKind::Ordered(n),
                    None => ListKind::Unordered,
                };
                self.list_stack.push(kind);
            }
            Tag::Item => {
                self.flush_paragraph();
                self.emit_item_prefix();
            }
            Tag::Link { dest_url, .. } => {
                self.link_url = Some(dest_url.to_string());
                let s = Style::default().fg(self.theme.primary);
                self.style_stack.push(s);
                self.recompute_style();
            }
            Tag::Image { dest_url, .. } => {
                self.in_image = true;
                self.image_url = Some(dest_url.to_string());
            }
            Tag::Table(_) => {
                self.in_table = true;
                self.table_rows = Vec::new();
                self.table_cur_row = Vec::new();
            }
            Tag::TableHead => {
                self.in_table_head = true;
                self.table_cur_row = Vec::new();
            }
            Tag::TableRow => {
                self.table_cur_row = Vec::new();
            }
            Tag::TableCell => {
                self.cur_spans = Vec::new();
            }
            _ => {}
        }
    }

    // -- Tag end ------------------------------------------------------------

    fn end_tag(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Heading(level) => {
                self.emit_heading(level);
                self.heading_level = None;
            }
            TagEnd::Paragraph => {
                self.flush_paragraph();
                self.push_blank_line();
            }
            TagEnd::Strong | TagEnd::Emphasis | TagEnd::Strikethrough => {
                self.style_stack.pop();
                self.recompute_style();
            }
            TagEnd::BlockQuote(_) => {
                self.blockquote_depth = self.blockquote_depth.saturating_sub(1);
            }
            TagEnd::CodeBlock => {
                self.emit_code_block();
            }
            TagEnd::List(_) => {
                self.list_stack.pop();
                if self.list_stack.is_empty() {
                    self.push_blank_line();
                }
            }
            TagEnd::Item => {
                self.flush_paragraph();
            }
            TagEnd::Link => {
                // Append URL if different from link text
                let url = self.link_url.take();
                self.style_stack.pop();
                self.recompute_style();
                if let Some(url) = url {
                    let text_content = spans_text(&self.cur_spans);
                    if url != text_content {
                        let url_span =
                            Span::styled(format!(" ({url})"), Style::default().fg(self.theme.dim));
                        self.cur_spans.push(url_span);
                    }
                }
            }
            TagEnd::Image => {
                self.in_image = false;
                self.image_url = None;
            }
            TagEnd::Table => {
                self.emit_table();
                self.in_table = false;
            }
            TagEnd::TableHead => {
                self.in_table_head = false;
                self.table_rows.insert(0, self.table_cur_row.clone());
                self.table_cur_row = Vec::new();
            }
            TagEnd::TableRow => {
                self.table_rows.push(self.table_cur_row.clone());
                self.table_cur_row = Vec::new();
            }
            TagEnd::TableCell => {
                let cell_spans = std::mem::take(&mut self.cur_spans);
                self.table_cur_row.push(cell_spans);
            }
            _ => {}
        }
    }

    // -- Inline content -----------------------------------------------------

    fn handle_text(&mut self, text: &str) {
        if self.in_code_block {
            self.code_lines.push(text.to_string());
            return;
        }
        self.flush_item_marker();
        if self.in_image {
            let prefix = if self.theme.use_unicode_glyphs() {
                "🖼 "
            } else {
                "[img] "
            };
            let img_span = Span::styled(
                format!("{prefix}{text}"),
                Style::default().fg(self.theme.dim),
            );
            self.cur_spans.push(img_span);
            return;
        }
        self.cur_spans
            .push(Span::styled(text.to_string(), self.cur_style));
    }

    fn handle_inline_code(&mut self, code: &str) {
        let style = Style::default().fg(self.theme.secondary);
        self.cur_spans.push(Span::styled(code.to_string(), style));
    }

    fn handle_soft_break(&mut self) {
        // Treat as space for re-wrapping
        self.cur_spans
            .push(Span::styled(" ".to_string(), self.cur_style));
    }

    fn handle_hard_break(&mut self) {
        self.flush_paragraph();
    }

    fn handle_rule(&mut self) {
        let line_str: String = "─".repeat(self.width);
        let span = Span::styled(line_str, Style::default().fg(self.theme.dim));
        self.lines.push(Line::from(vec![span]));
    }

    // -- Helpers ------------------------------------------------------------

    fn push_modifier(&mut self, modifier: Modifier) {
        let s = Style::default().add_modifier(modifier);
        self.style_stack.push(s);
        self.recompute_style();
    }

    fn recompute_style(&mut self) {
        let mut s = Style::default();
        for layer in &self.style_stack {
            s = s.patch(*layer);
        }
        self.cur_style = s;
    }

    fn build_prefix_spans(&self) -> Vec<Span<'static>> {
        let mut prefix = Vec::new();
        for _ in 0..self.blockquote_depth {
            prefix.push(Span::styled(
                "│ ".to_string(),
                Style::default().fg(self.theme.dim),
            ));
        }
        if self.list_stack.len() > 1 {
            let indent: String = " ".repeat((self.list_stack.len() - 1) * 2);
            prefix.push(Span::raw(indent));
        }
        prefix
    }

    fn emit_item_prefix(&mut self) {
        // Increment ordered counter but don't emit yet - TaskListMarker may follow
        let marker = match self.list_stack.last_mut() {
            Some(ListKind::Unordered) => "• ".to_string(),
            Some(ListKind::Ordered(n)) => {
                let s = format!("{n}. ");
                *n += 1;
                s
            }
            None => String::new(),
        };
        // Store for potential override by TaskListMarker
        self.pending_item_marker = Some(marker);
    }

    fn flush_item_marker(&mut self) {
        if let Some(marker) = self.pending_item_marker.take()
            && !marker.is_empty()
        {
            self.cur_spans.push(Span::styled(marker, self.cur_style));
        }
    }

    fn emit_heading(&mut self, level: HeadingLevel) {
        let mut style = Style::default()
            .fg(self.theme.primary)
            .add_modifier(Modifier::BOLD);
        if matches!(level, HeadingLevel::H1) {
            style = style.add_modifier(Modifier::UNDERLINED);
        }
        // Re-style all current spans
        let spans: Vec<Span<'static>> = std::mem::take(&mut self.cur_spans)
            .into_iter()
            .map(|sp| Span::styled(sp.content.into_owned(), style))
            .collect();
        if !spans.is_empty() {
            self.lines.push(Line::from(spans));
        }
        self.push_blank_line();
    }

    fn flush_paragraph(&mut self) {
        let spans = std::mem::take(&mut self.cur_spans);
        if spans.is_empty() {
            return;
        }
        let prefix = self.build_prefix_spans();
        let prefix_w = prefix.iter().map(|s| s.content.width()).sum::<usize>();
        let available = self.width.saturating_sub(prefix_w);
        let wrapped = wrap_spans(&spans, available);
        for line_spans in wrapped {
            let mut full = prefix.clone();
            full.extend(line_spans);
            self.lines.push(Line::from(full));
        }
    }

    fn flush_line(&mut self) {
        if !self.cur_spans.is_empty() {
            self.flush_paragraph();
        }
    }

    fn push_blank_line(&mut self) {
        self.lines.push(Line::from(vec![Span::raw(String::new())]));
    }

    fn emit_code_block(&mut self) {
        self.in_code_block = false;
        let lang = self.code_lang.take();
        let code_text = std::mem::take(&mut self.code_lines).join("");
        let code_lines: Vec<&str> = if code_text.is_empty() {
            Vec::new()
        } else {
            // Split but keep structure
            let mut lines: Vec<&str> = code_text.split('\n').collect();
            // Remove trailing empty line from trailing newline
            if lines.last() == Some(&"") {
                lines.pop();
            }
            lines
        };

        let set = syntax_set();
        let syn_theme = syntect_theme_for(self.theme);
        let syntax = lang.as_deref().map_or_else(
            || set.find_syntax_plain_text(),
            |l| syntax_for_token(set, l),
        );
        let mut hl = HighlightLines::new(syntax, syn_theme);

        for code_line in code_lines {
            let highlighted = highlight_one(&mut hl, set, code_line);
            let spans: Vec<Span<'static>> = highlighted
                .into_iter()
                .map(|(style, text)| Span::styled(text, style))
                .collect();
            if spans.is_empty() {
                self.lines.push(Line::from(vec![Span::raw(String::new())]));
            } else {
                self.lines.push(Line::from(spans));
            }
        }
    }

    fn emit_table(&mut self) {
        let rows = std::mem::take(&mut self.table_rows);
        if rows.is_empty() {
            return;
        }

        // Calculate column widths
        let num_cols = rows.iter().map(Vec::len).max().unwrap_or(0);
        if num_cols == 0 {
            return;
        }
        let mut col_widths: Vec<usize> = vec![0; num_cols];
        for row in &rows {
            for (i, cell) in row.iter().enumerate() {
                let cell_w = cell.iter().map(|s| s.content.width()).sum::<usize>();
                if cell_w > col_widths[i] {
                    col_widths[i] = cell_w;
                }
            }
        }

        // Clamp total to width
        let total: usize = col_widths.iter().sum::<usize>() + (num_cols.saturating_sub(1)) * 3; // " | " separators
        if total > self.width && self.width > 0 {
            // Simple proportional clamp
            let sep_space = (num_cols.saturating_sub(1)) * 3;
            let avail = self.width.saturating_sub(sep_space);
            let col_total: usize = col_widths.iter().sum();
            for w in &mut col_widths {
                if let Some(scaled) = (*w * avail).checked_div(col_total) {
                    *w = scaled.max(1);
                }
            }
        }

        let is_header_row = |idx: usize| idx == 0;

        for (row_idx, row) in rows.iter().enumerate() {
            let mut line_spans: Vec<Span<'static>> = Vec::new();
            for (col_idx, cell) in row.iter().enumerate() {
                if col_idx > 0 {
                    line_spans.push(Span::raw(" │ ".to_string()));
                }
                let cell_text: String = cell.iter().map(|s| s.content.as_ref()).collect();
                let cell_w = cell_text.width();
                let target_w = col_widths.get(col_idx).copied().unwrap_or(0);
                let pad = target_w.saturating_sub(cell_w);

                if is_header_row(row_idx) {
                    let style = Style::default().add_modifier(Modifier::BOLD);
                    line_spans.push(Span::styled(cell_text.clone(), style));
                } else {
                    for sp in cell {
                        line_spans.push(sp.clone());
                    }
                }
                if pad > 0 {
                    line_spans.push(Span::raw(" ".repeat(pad)));
                }
            }
            self.lines.push(Line::from(line_spans));

            // Separator after header
            if is_header_row(row_idx) {
                let sep_total: usize =
                    col_widths.iter().sum::<usize>() + (num_cols.saturating_sub(1)) * 3;
                let sep: String = "─".repeat(sep_total);
                self.lines
                    .push(Line::from(vec![Span::styled(sep, Style::default())]));
            }
        }
        self.push_blank_line();
    }
}

// ---------------------------------------------------------------------------
// Word-wrapping
// ---------------------------------------------------------------------------

/// Wrap a sequence of spans to fit within `max_width` display columns.
/// Returns groups of spans, one per output line.
fn wrap_spans(spans: &[Span<'static>], max_width: usize) -> Vec<Vec<Span<'static>>> {
    if max_width == 0 {
        return vec![spans.to_vec()];
    }

    let mut result: Vec<Vec<Span<'static>>> = Vec::new();
    let mut cur_line: Vec<Span<'static>> = Vec::new();
    let mut cur_w: usize = 0;

    for span in spans {
        let style = span.style;
        let text = span.content.as_ref();

        // Split text into word-like chunks (preserving spaces)
        let mut remaining = text;
        while !remaining.is_empty() {
            // Find next word boundary
            let trimmed_start = remaining.len() - remaining.trim_start().len();
            let leading_space = &remaining[..trimmed_start];
            let after_space = &remaining[trimmed_start..];

            let word_end = after_space
                .find(|c: char| c.is_whitespace())
                .unwrap_or(after_space.len());
            let word = &after_space[..word_end];

            let chunk = format!("{leading_space}{word}");
            let chunk_w = chunk.width();

            if cur_w > 0 && cur_w + chunk_w > max_width {
                // Wrap: start new line
                result.push(std::mem::take(&mut cur_line));
                cur_w = 0;
                // Drop leading spaces on new line
                let trimmed = chunk.trim_start().to_string();
                let tw = trimmed.width();
                if tw > 0 {
                    if tw > max_width {
                        // Hard-split long word
                        hard_split_push(
                            &mut result,
                            &mut cur_line,
                            &mut cur_w,
                            &trimmed,
                            style,
                            max_width,
                        );
                    } else {
                        cur_line.push(Span::styled(trimmed, style));
                        cur_w += tw;
                    }
                }
            } else if chunk_w > max_width && cur_w == 0 {
                // Single word exceeds width: hard-split
                hard_split_push(
                    &mut result,
                    &mut cur_line,
                    &mut cur_w,
                    &chunk,
                    style,
                    max_width,
                );
            } else {
                cur_line.push(Span::styled(chunk.clone(), style));
                cur_w += chunk_w;
            }

            remaining = &after_space[word_end..];
        }
    }

    if !cur_line.is_empty() {
        result.push(cur_line);
    }
    if result.is_empty() {
        result.push(Vec::new());
    }
    result
}

fn hard_split_push(
    result: &mut Vec<Vec<Span<'static>>>,
    cur_line: &mut Vec<Span<'static>>,
    cur_w: &mut usize,
    text: &str,
    style: Style,
    max_width: usize,
) {
    let mut buf = String::new();
    let mut buf_w: usize = 0;

    for c in text.chars() {
        let cw = UnicodeWidthStr::width(c.encode_utf8(&mut [0u8; 4]) as &str);
        if *cur_w + buf_w + cw > max_width && (*cur_w + buf_w > 0) {
            if !buf.is_empty() {
                cur_line.push(Span::styled(std::mem::take(&mut buf), style));
            }
            result.push(std::mem::take(cur_line));
            *cur_w = 0;
            buf_w = 0;
        }
        buf.push(c);
        buf_w += cw;
    }
    if !buf.is_empty() {
        cur_line.push(Span::styled(buf, style));
        *cur_w += buf_w;
    }
}

fn spans_text(spans: &[Span<'_>]) -> String {
    spans.iter().map(|s| s.content.as_ref()).collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use ratatui::style::{Color, Modifier};

    use super::*;
    use crate::ui::theme::Theme;

    fn dark() -> Theme {
        Theme::dark()
    }

    fn plain(lines: &[Line<'_>]) -> String {
        lines
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn has_mod(line: &Line<'_>, modifier: Modifier) -> bool {
        line.spans
            .iter()
            .any(|s| s.style.add_modifier.contains(modifier))
    }

    fn has_fg(line: &Line<'_>, color: Color) -> bool {
        line.spans.iter().any(|s| s.style.fg == Some(color))
    }

    #[test]
    fn heading_is_bold_primary_without_hash() {
        let lines = render_markdown("# Title", 80, &dark());
        let content: Vec<_> = lines
            .iter()
            .filter(|l| plain(std::slice::from_ref(*l)).trim() != "")
            .collect();
        assert!(!content.is_empty());
        assert!(plain(std::slice::from_ref(content[0])).contains("Title"));
        assert!(!plain(std::slice::from_ref(content[0])).contains('#'));
        assert!(has_mod(content[0], Modifier::BOLD));
        assert!(has_fg(content[0], dark().primary));
    }

    #[test]
    fn paragraph_wraps_to_width() {
        let long = "word ".repeat(30);
        let lines = render_markdown(&long, 20, &dark());
        assert!(lines.len() > 1);
        for line in &lines {
            let w: usize = line.spans.iter().map(|s| s.content.width()).sum();
            assert!(w <= 20, "line too wide: {w}");
        }
    }

    #[test]
    fn bold_italic_strike_modifiers() {
        let lines = render_markdown("**b** *i* ~~s~~", 80, &dark());
        let all_spans: Vec<_> = lines.iter().flat_map(|l| l.spans.iter()).collect();
        assert!(
            all_spans
                .iter()
                .any(|s| s.content.contains('b') && s.style.add_modifier.contains(Modifier::BOLD))
        );
        assert!(all_spans.iter().any(|s| s.content.contains('i')
            && s.style.add_modifier.contains(Modifier::ITALIC)));
        assert!(all_spans.iter().any(
            |s| s.content.contains('s') && s.style.add_modifier.contains(Modifier::CROSSED_OUT)
        ));
    }

    #[test]
    fn inline_code_uses_secondary() {
        let lines = render_markdown("`x`", 80, &dark());
        let all_spans: Vec<_> = lines.iter().flat_map(|l| l.spans.iter()).collect();
        assert!(
            all_spans
                .iter()
                .any(|s| s.content.contains('x') && s.style.fg == Some(dark().secondary))
        );
    }

    #[test]
    fn fenced_code_block_highlights_known_lang() {
        let md = "```rust\nlet x = 1;\n```";
        let lines = render_markdown(md, 80, &dark());
        // The code line should have multiple spans (syntax highlighting)
        let code_line = lines
            .iter()
            .find(|l| plain(std::slice::from_ref(*l)).contains("let"))
            .expect("should find code line");
        assert!(
            code_line.spans.len() > 1,
            "expected multiple spans from syntax highlighting, got {}",
            code_line.spans.len()
        );
    }

    #[test]
    fn fenced_code_block_unknown_lang_plain() {
        let md = "```unknownlang123\nhello world\n```";
        let lines = render_markdown(md, 80, &dark());
        assert!(plain(&lines).contains("hello world"));
    }

    #[test]
    fn unordered_list_bullets_and_nesting() {
        let md = "- a\n  - b";
        let lines = render_markdown(md, 80, &dark());
        let text = plain(&lines);
        assert!(text.contains('•'), "should have bullet: {text}");
        // Nested item should be more indented than its parent.
        let line_text =
            |l: &Line<'_>| -> String { l.spans.iter().map(|s| s.content.as_ref()).collect() };
        let indent_of = |needle: char| -> usize {
            let t = lines
                .iter()
                .map(line_text)
                .find(|t| t.contains(needle))
                .unwrap();
            t.len() - t.trim_start().len()
        };
        let indent_a = indent_of('a');
        let indent_b = indent_of('b');
        assert!(
            indent_b > indent_a,
            "nested should be more indented: {indent_a} vs {indent_b}"
        );
    }

    #[test]
    fn ordered_list_numbers() {
        let md = "1. first\n2. second";
        let lines = render_markdown(md, 80, &dark());
        let text = plain(&lines);
        assert!(text.contains("1."), "text: {text}");
        assert!(text.contains("2."), "text: {text}");
    }

    #[test]
    fn task_list_markers() {
        let md = "- [ ] todo\n- [x] done";
        let lines = render_markdown(md, 80, &dark());
        let text = plain(&lines);
        assert!(text.contains('☐'), "should have unchecked: {text}");
        assert!(text.contains('☑'), "should have checked: {text}");
    }

    #[test]
    fn blockquote_prefixed_dim() {
        let lines = render_markdown("> hi", 80, &dark());
        let first_content = lines
            .iter()
            .find(|l| plain(std::slice::from_ref(*l)).contains("hi"))
            .expect("should have content");
        assert!(plain(std::slice::from_ref(first_content)).starts_with("│ "));
        assert!(has_fg(first_content, dark().dim));
    }

    #[test]
    fn thematic_break_rule() {
        let lines = render_markdown("---", 40, &dark());
        let rule_line = lines
            .iter()
            .find(|l| plain(std::slice::from_ref(*l)).contains('─'))
            .expect("should have rule");
        let text = plain(std::slice::from_ref(rule_line));
        assert_eq!(text.chars().filter(|c| *c == '─').count(), 40);
    }

    #[test]
    fn link_text_and_url() {
        let lines = render_markdown("[text](http://x)", 80, &dark());
        let all_spans: Vec<_> = lines.iter().flat_map(|l| l.spans.iter()).collect();
        assert!(
            all_spans
                .iter()
                .any(|s| s.content.contains("text") && s.style.fg == Some(dark().primary))
        );
        assert!(
            all_spans
                .iter()
                .any(|s| s.content.contains("http://x") && s.style.fg == Some(dark().dim))
        );
    }

    #[test]
    fn cache_matches_uncached() {
        clear_markdown_cache();
        let t = dark();
        for md in [
            "# H",
            "- a\n  - b",
            "```rust\nlet x = 1;\n```",
            "[t](http://x)",
            "| a | b |\n| - | - |\n| 1 | 2 |",
        ] {
            let uncached = render_markdown_uncached(md, 80, &t);
            assert_eq!(render_markdown(md, 80, &t), uncached, "miss path");
            assert_eq!(render_markdown(md, 80, &t), uncached, "hit path");
        }
    }

    #[test]
    fn cache_keys_on_width() {
        clear_markdown_cache();
        let t = dark();
        let body = "a fairly long paragraph of words that wraps differently depending on the width";
        let narrow = render_markdown(body, 20, &t);
        let wide = render_markdown(body, 200, &t);
        assert_ne!(
            narrow.len(),
            wide.len(),
            "width must be part of the cache key"
        );
    }

    #[test]
    fn gfm_table_header_bold_and_separator() {
        let md = "| A | B |\n|---|---|\n| 1 | 2 |";
        let lines = render_markdown(md, 80, &dark());
        // Header row should be bold
        let header = &lines[0];
        assert!(has_mod(header, Modifier::BOLD));
        // Should have a separator line with ─
        assert!(
            lines
                .iter()
                .any(|l| plain(std::slice::from_ref(l)).contains('─'))
        );
    }
}
