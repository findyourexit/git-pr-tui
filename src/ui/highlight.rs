//! Shared syntect highlighting plumbing, used by the diff view and the
//! Markdown renderer so both colorize code identically.

use std::sync::OnceLock;

use ratatui::style::{Color, Style};
use syntect::{
    easy::HighlightLines,
    highlighting::{Style as SynStyle, Theme as SynTheme, ThemeSet},
    parsing::{SyntaxReference, SyntaxSet},
    util::LinesWithEndings,
};

use crate::ui::theme::Theme;

static SYNTAX_SET: OnceLock<SyntaxSet> = OnceLock::new();
static THEME_SET: OnceLock<ThemeSet> = OnceLock::new();

#[must_use]
pub fn syntax_set() -> &'static SyntaxSet {
    SYNTAX_SET.get_or_init(SyntaxSet::load_defaults_newlines)
}

#[must_use]
pub fn theme_set() -> &'static ThemeSet {
    THEME_SET.get_or_init(ThemeSet::load_defaults)
}

#[must_use]
pub fn syntect_theme_for(theme: &Theme) -> &'static SynTheme {
    let ts = theme_set();
    let name = if theme.bg == Theme::dark().bg {
        "base16-ocean.dark"
    } else {
        "base16-ocean.light"
    };
    ts.themes
        .get(name)
        .or_else(|| ts.themes.values().next())
        .expect("syntect ships with default themes")
}

/// Resolve a syntax by file extension (diff view's path hint).
#[must_use]
pub fn syntax_for_path<'a>(set: &'a SyntaxSet, path_hint: &str) -> &'a SyntaxReference {
    let ext = path_hint.rsplit('.').next().unwrap_or("");
    set.find_syntax_by_extension(ext)
        .unwrap_or_else(|| set.find_syntax_plain_text())
}

/// Resolve a syntax by a fenced-code-block info string / language token.
#[must_use]
pub fn syntax_for_token<'a>(set: &'a SyntaxSet, token: &str) -> &'a SyntaxReference {
    let token = token.trim();
    set.find_syntax_by_token(token)
        .or_else(|| set.find_syntax_by_extension(token))
        .unwrap_or_else(|| set.find_syntax_plain_text())
}

#[must_use]
pub fn syn_style_to_ratatui(style: SynStyle) -> Style {
    Style::default().fg(Color::Rgb(
        style.foreground.r,
        style.foreground.g,
        style.foreground.b,
    ))
}

/// Highlight one body string into `(style, text)` runs (newline-stripped).
#[must_use]
pub fn highlight_one(
    highlighter: &mut HighlightLines<'_>,
    set: &SyntaxSet,
    body: &str,
) -> Vec<(Style, String)> {
    let owned = format!("{body}\n");
    let mut spans = Vec::new();
    for piece in LinesWithEndings::from(&owned) {
        let Ok(ranges) = highlighter.highlight_line(piece, set) else {
            spans.push((Style::default(), piece.trim_end_matches('\n').to_string()));
            continue;
        };
        for (syn_style, text) in ranges {
            let text = text.trim_end_matches('\n').to_string();
            if text.is_empty() {
                continue;
            }
            spans.push((syn_style_to_ratatui(syn_style), text));
        }
    }
    spans
}
