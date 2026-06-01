//! PR Detail sub-tab rendering (conversation, files, checks, commits).

use chrono::{DateTime, Utc};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, TableState},
};
use unicode_width::UnicodeWidthStr;

use crate::data::models::{
    CheckConclusion, CheckRun, CheckStatus, FileStatus, Mergeable, PrChecks, PrDetail, PrFiles,
    PrState, ReviewComment, ReviewThread, StatusState, TimelineEvent,
};
use crate::ui::components::table::{Column, render_table};
use crate::ui::markdown::render_markdown;
use crate::ui::text::relative_time;
use crate::ui::{glyphs, theme::Theme};

enum UnifiedEvent<'a> {
    Timeline(&'a TimelineEvent),
    Thread(&'a ReviewThread),
}

impl UnifiedEvent<'_> {
    fn key(&self) -> DateTime<Utc> {
        match self {
            UnifiedEvent::Timeline(TimelineEvent::Commit { pushed_at, .. }) => *pushed_at,
            UnifiedEvent::Timeline(TimelineEvent::IssueComment { created_at, .. }) => *created_at,
            UnifiedEvent::Timeline(TimelineEvent::Review { submitted_at, .. }) => *submitted_at,
            UnifiedEvent::Timeline(
                TimelineEvent::Merged { at, .. }
                | TimelineEvent::Closed { at, .. }
                | TimelineEvent::Reopened { at, .. }
                | TimelineEvent::HeadRefForcePushed { at, .. },
            ) => *at,
            UnifiedEvent::Thread(t) => t
                .comments
                .first()
                .map_or(DateTime::<Utc>::MIN_UTC, |c| c.created_at),
        }
    }
}

fn strip_suggestion_fence(body: &str) -> String {
    const OPEN: &str = "```suggestion\n";
    let Some(start) = body.find(OPEN) else {
        return body.to_string();
    };
    let after_open = start + OPEN.len();
    let Some(close_rel) = body[after_open..].find("```") else {
        return body.to_string();
    };
    let close_abs = after_open + close_rel + 3;
    let mut out = String::with_capacity(body.len());
    out.push_str(body[..start].trim_end_matches('\n'));
    let tail = &body[close_abs..];
    let tail = tail.strip_prefix('\n').unwrap_or(tail);
    if !tail.is_empty() {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(tail);
    }
    out
}

fn render_review_comment(
    lines: &mut Vec<Line<'static>>,
    comment: &ReviewComment,
    width: u16,
    theme: &Theme,
) {
    let body_without_suggestion = strip_suggestion_fence(&comment.body);
    let label = format!("    {}: ", comment.author);
    let indent_width = width.saturating_sub(u16::try_from(label.chars().count()).unwrap_or(0));
    let md_lines = render_markdown(&body_without_suggestion, indent_width, theme);
    let mut md_iter = md_lines.into_iter();
    if let Some(first_md) = md_iter.next() {
        let mut spans = vec![Span::raw(label)];
        spans.extend(first_md.spans);
        lines.push(Line::from(spans));
    } else {
        lines.push(Line::raw(format!("    {}: ", comment.author)));
    }
    for md_line in md_iter {
        let mut spans = vec![Span::raw("      ")];
        spans.extend(md_line.spans);
        lines.push(Line::from(spans));
    }

    if let Some(suggestion) = &comment.suggestion {
        lines.push(Line::from(vec![
            Span::raw("    "),
            Span::styled(
                "[suggestion]",
                Style::default()
                    .fg(theme.success)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
        for s_line in suggestion.lines() {
            lines.push(Line::from(vec![
                Span::raw("      "),
                Span::styled(format!("+{s_line}"), Style::default().fg(theme.success)),
            ]));
        }
    }
}

struct ConversationLines {
    lines: Vec<Line<'static>>,
    focused_thread_start: Option<usize>,
}

/// Scroll / focus state for the conversation tab.
#[derive(Debug, Clone, Copy)]
pub struct ConversationScroll {
    pub cursor: usize,
    pub scroll_to_focused: bool,
    pub now: DateTime<Utc>,
}

#[allow(clippy::too_many_lines)]
fn build_conversation_lines(
    detail: &PrDetail,
    focused_thread: Option<usize>,
    now: DateTime<Utc>,
    width: u16,
    theme: &Theme,
) -> ConversationLines {
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut focused_thread_start: Option<usize> = None;

    let is_draft = detail.summary.is_draft;
    let state_str = if is_draft {
        "DRAFT"
    } else {
        match detail.summary.state {
            PrState::Open => "OPEN",
            PrState::Closed => "CLOSED",
            PrState::Merged => "MERGED",
        }
    };
    let state_color = glyphs::color_for_pr_state(is_draft, detail.summary.state, theme);

    let (merge_str, merge_color) = match detail.summary.mergeable {
        Mergeable::Clean => ("MERGEABLE", theme.success),
        Mergeable::Dirty => ("CONFLICTING", theme.error),
        _ => ("UNKNOWN", theme.dim),
    };

    let unicode = theme.use_unicode_glyphs();
    let checks_glyph = glyphs::glyph_for_checks_rollup(detail.summary.checks, unicode);
    let checks_color = glyphs::color_for_checks_rollup(detail.summary.checks, theme);
    let review_glyph = glyphs::glyph_for_review_decision(detail.summary.review_decision, unicode);
    let review_color = glyphs::color_for_review_decision(detail.summary.review_decision, theme);
    let header_line = Line::from(vec![
        Span::styled(
            format!("#{}  {}  ", detail.summary.id.number, detail.summary.title),
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!("[{state_str}]"), Style::default().fg(state_color)),
        Span::raw("  "),
        Span::styled(format!("[{merge_str}]"), Style::default().fg(merge_color)),
        Span::raw("  "),
        Span::styled(checks_glyph, Style::default().fg(checks_color)),
        Span::raw(" "),
        Span::styled(review_glyph, Style::default().fg(review_color)),
    ]);
    lines.push(header_line);
    lines.push(Line::raw(""));

    for bl in render_markdown(&detail.body, width, theme) {
        lines.push(bl);
    }
    lines.push(Line::raw(""));

    let mut events: Vec<UnifiedEvent> = Vec::new();
    for evt in &detail.timeline {
        events.push(UnifiedEvent::Timeline(evt));
    }
    for thread in &detail.review_threads {
        events.push(UnifiedEvent::Thread(thread));
    }

    if events.is_empty() {
        lines.push(Line::styled(
            "No activity yet.",
            Style::default().fg(theme.dim),
        ));
    } else {
        events.sort_by_key(UnifiedEvent::key);
        let separator_style = Style::default().fg(theme.dim);
        let sep_width = usize::from(width);

        for (i, evt) in events.into_iter().enumerate() {
            if i > 0 {
                lines.push(Line::styled("─".repeat(sep_width), separator_style));
            }
            match evt {
                UnifiedEvent::Timeline(tl_event) => {
                    let g = glyphs::glyph_for_timeline_event(tl_event, unicode);
                    let ts = UnifiedEvent::Timeline(tl_event).key();
                    let rel = relative_time(ts, now);
                    match tl_event {
                        TimelineEvent::Commit {
                            sha,
                            message,
                            author,
                            ..
                        } => {
                            let sha_short = &sha[..sha.len().min(7)];
                            let msg_first = message.lines().next().unwrap_or("");
                            lines.push(Line::from(vec![
                                Span::styled(format!("{g} "), Style::default()),
                                Span::styled(
                                    author.clone(),
                                    Style::default().add_modifier(Modifier::BOLD),
                                ),
                                Span::styled(format!(" {rel}"), Style::default().fg(theme.dim)),
                                Span::raw(format!("  commit {sha_short}  {msg_first}")),
                            ]));
                        }
                        TimelineEvent::IssueComment { author, body, .. } => {
                            lines.push(Line::from(vec![
                                Span::styled(format!("{g} "), Style::default()),
                                Span::styled(
                                    author.clone(),
                                    Style::default().add_modifier(Modifier::BOLD),
                                ),
                                Span::styled(format!(" {rel}"), Style::default().fg(theme.dim)),
                            ]));
                            for md_line in render_markdown(body, width, theme) {
                                lines.push(md_line);
                            }
                        }
                        TimelineEvent::Review {
                            author,
                            state,
                            body,
                            ..
                        } => {
                            let state_str = format!("{state:?}");
                            lines.push(Line::from(vec![
                                Span::styled(format!("{g} "), Style::default()),
                                Span::styled(
                                    author.clone(),
                                    Style::default().add_modifier(Modifier::BOLD),
                                ),
                                Span::styled(format!(" {rel}"), Style::default().fg(theme.dim)),
                                Span::raw(format!("  {state_str}")),
                            ]));
                            let body_text = if body.trim().is_empty() {
                                "(no body)"
                            } else {
                                body
                            };
                            for md_line in render_markdown(body_text, width, theme) {
                                lines.push(md_line);
                            }
                        }
                        TimelineEvent::Merged { by, sha, .. } => {
                            let sha_short = &sha[..sha.len().min(7)];
                            lines.push(Line::from(vec![
                                Span::styled(format!("{g} "), Style::default()),
                                Span::styled(
                                    by.clone(),
                                    Style::default().add_modifier(Modifier::BOLD),
                                ),
                                Span::styled(format!(" {rel}"), Style::default().fg(theme.dim)),
                                Span::raw(format!("  merged {sha_short}")),
                            ]));
                        }
                        TimelineEvent::Closed { by, .. } => {
                            lines.push(Line::from(vec![
                                Span::styled(format!("{g} "), Style::default()),
                                Span::styled(
                                    by.clone(),
                                    Style::default().add_modifier(Modifier::BOLD),
                                ),
                                Span::styled(format!(" {rel}"), Style::default().fg(theme.dim)),
                                Span::raw("  closed"),
                            ]));
                        }
                        TimelineEvent::Reopened { by, .. } => {
                            lines.push(Line::from(vec![
                                Span::styled(format!("{g} "), Style::default()),
                                Span::styled(
                                    by.clone(),
                                    Style::default().add_modifier(Modifier::BOLD),
                                ),
                                Span::styled(format!(" {rel}"), Style::default().fg(theme.dim)),
                                Span::raw("  reopened"),
                            ]));
                        }
                        TimelineEvent::HeadRefForcePushed { before, after, .. } => {
                            let before_short = &before[..before.len().min(7)];
                            let after_short = &after[..after.len().min(7)];
                            let content =
                                format!(" {before_short} {g} {after_short} (force-pushed) ");
                            let dashes_len = width.saturating_sub(
                                u16::try_from(content.chars().count()).unwrap_or(0),
                            ) / 2;
                            let dashes = "─".repeat(usize::from(dashes_len));
                            let mut line_str = format!("{dashes}{content}{dashes}");
                            let rem = width.saturating_sub(
                                u16::try_from(line_str.chars().count()).unwrap_or(0),
                            );
                            line_str.push_str(&"─".repeat(usize::from(rem)));
                            lines.push(Line::styled(line_str, Style::default().fg(theme.dim)));
                        }
                    }
                }
                UnifiedEvent::Thread(thread) => {
                    let thread_index = detail.review_threads.iter().position(|t| t.id == thread.id);
                    let is_focused = focused_thread.is_some() && focused_thread == thread_index;
                    let resolved = if thread.is_resolved {
                        " [resolved]"
                    } else {
                        ""
                    };
                    let outdated = if thread.is_outdated {
                        " [outdated]"
                    } else {
                        ""
                    };

                    let collapse = (thread.is_resolved || thread.is_outdated) && !is_focused;
                    let header_idx = lines.len();

                    if is_focused {
                        focused_thread_start = Some(header_idx);
                    }

                    if collapse {
                        let summary = format!(
                            "▸ {}:{}{}{}  ({} comments)",
                            thread.path,
                            thread.line_range.start_line,
                            resolved,
                            outdated,
                            thread.comments.len()
                        );
                        lines.push(Line::styled(summary, Style::default().fg(theme.dim)));
                    } else {
                        let marker = if is_focused { "▶" } else { "▸" };
                        let header = format!(
                            "{marker} {}:{}{}{}  ({} comments)",
                            thread.path,
                            thread.line_range.start_line,
                            resolved,
                            outdated,
                            thread.comments.len()
                        );
                        let style = if is_focused {
                            Style::default().fg(theme.primary)
                        } else {
                            Style::default()
                        };
                        lines.push(Line::styled(header, style));
                        for comment in &thread.comments {
                            render_review_comment(&mut lines, comment, width, theme);
                        }
                    }
                }
            }
        }
    }

    ConversationLines {
        lines,
        focused_thread_start,
    }
}

pub fn render_conversation(
    f: &mut Frame,
    area: Rect,
    detail: &PrDetail,
    focused_thread: Option<usize>,
    scroll: ConversationScroll,
    theme: &Theme,
) {
    let conv = build_conversation_lines(detail, focused_thread, scroll.now, area.width, theme);
    let lines = conv.lines;
    let viewport = usize::from(area.height);
    let total = lines.len();
    let max_offset = total.saturating_sub(viewport);
    let offset = if scroll.scroll_to_focused {
        conv.focused_thread_start.map_or(0, |s| s.min(max_offset))
    } else {
        scroll.cursor.min(max_offset)
    };

    let visible: Vec<Line<'static>> = lines.into_iter().skip(offset).take(viewport).collect();
    let paragraph = Paragraph::new(visible);
    f.render_widget(paragraph, area);

    if total > viewport {
        let mut scrollbar_state = ScrollbarState::new(total).position(offset);
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(None)
            .end_symbol(None);
        f.render_stateful_widget(scrollbar, area, &mut scrollbar_state);
    }
}

fn tag_string(file: &crate::data::models::FileDiff) -> String {
    let mut tags = String::new();
    if file.is_binary {
        tags.push_str("  [binary]");
    }
    if file.is_submodule {
        tags.push_str("  [submodule]");
    }
    if file.oversize {
        tags.push_str("  [oversize]");
    }
    tags
}

#[allow(clippy::cast_possible_truncation)]
pub fn render_files(f: &mut Frame, area: Rect, files: &PrFiles, selected: usize, theme: &Theme) {
    if files.files.is_empty() {
        let msg = Line::styled("No files changed.", Style::default().fg(theme.dim));
        f.render_widget(msg, area);
        return;
    }

    let mut list_area = area;
    if files.truncated {
        let header = Line::styled(
            "GitHub truncated the file list at 3000 files; some files are hidden.",
            Style::default().fg(theme.warning),
        );
        let header_area = Rect {
            x: list_area.x,
            y: list_area.y,
            width: list_area.width,
            height: 1,
        };
        f.render_widget(header, header_area);
        list_area.y = list_area.y.saturating_add(1);
        list_area.height = list_area.height.saturating_sub(1);
    }

    let tags_w: u16 = files
        .files
        .iter()
        .map(|f| tag_string(f).width())
        .max()
        .unwrap_or(0)
        .min(24) as u16;

    let columns = vec![
        Column::new("", Constraint::Length(1)),
        Column::new("Path", Constraint::Fill(1)).ellipsis(),
        Column::new("+", Constraint::Min(6)).right(),
        Column::new("\u{2212}", Constraint::Min(6)).right(),
        Column::new("", Constraint::Length(tags_w)),
    ];

    let rows: Vec<Vec<Line>> = files
        .files
        .iter()
        .map(|file| {
            let (status_char, status_style) = match file.status {
                FileStatus::Added => ('A', Style::default().fg(theme.success)),
                FileStatus::Removed => ('D', Style::default().fg(theme.error)),
                FileStatus::Modified => ('M', Style::default().fg(theme.fg)),
                FileStatus::Renamed => ('R', Style::default().fg(theme.fg)),
                FileStatus::Copied => ('C', Style::default().fg(theme.fg)),
                FileStatus::ChangedMode => ('X', Style::default().fg(theme.fg)),
                FileStatus::Unmerged => ('U', Style::default().fg(theme.fg)),
            };

            let rename_suffix = match file.status {
                FileStatus::Renamed | FileStatus::Copied => {
                    if let Some(ref prev) = file.previous_path {
                        format!(" (from {prev})")
                    } else {
                        String::new()
                    }
                }
                _ => String::new(),
            };

            vec![
                Line::from(Span::styled(status_char.to_string(), status_style)),
                Line::raw(format!("{}{rename_suffix}", file.path)),
                Line::from(Span::styled(
                    format!("+{}", file.additions),
                    Style::default().fg(theme.success),
                )),
                Line::from(Span::styled(
                    format!("-{}", file.deletions),
                    Style::default().fg(theme.error),
                )),
                Line::from(Span::styled(
                    tag_string(file),
                    Style::default().fg(theme.dim),
                )),
            ]
        })
        .collect();

    let mut state = TableState::default();
    state.select(Some(selected.min(files.files.len().saturating_sub(1))));

    render_table(
        f,
        list_area,
        &columns,
        rows,
        Some(&mut state),
        true,
        None,
        theme,
    );
}

pub fn render_files_summary(f: &mut Frame, area: Rect, changed_files: u32, theme: &Theme) {
    let lines = vec![
        Line::styled(
            format!("{changed_files} files changed in this PR."),
            Style::default().fg(theme.fg),
        ),
        Line::styled(
            "Press L to load the file list.",
            Style::default().fg(theme.warning),
        ),
    ];
    let paragraph = Paragraph::new(lines);
    f.render_widget(paragraph, area);
}

fn check_conclusion_label(
    status: CheckStatus,
    conclusion: Option<CheckConclusion>,
) -> &'static str {
    match status {
        CheckStatus::Queued => "queued",
        CheckStatus::InProgress => "in progress",
        CheckStatus::Completed => match conclusion {
            Some(CheckConclusion::Success) => "success",
            Some(CheckConclusion::Failure) => "failure",
            Some(CheckConclusion::Neutral) => "neutral",
            Some(CheckConclusion::Cancelled) => "cancelled",
            Some(CheckConclusion::Skipped) => "skipped",
            Some(CheckConclusion::TimedOut) => "timed out",
            Some(CheckConclusion::ActionRequired) => "action required",
            Some(CheckConclusion::Stale) => "stale",
            None => "completed",
        },
    }
}

fn status_state_label(state: StatusState) -> &'static str {
    match state {
        StatusState::Pending => "pending",
        StatusState::Success => "success",
        StatusState::Failure => "failure",
        StatusState::Error => "error",
    }
}

fn check_run_is_failed(run: &CheckRun) -> bool {
    matches!(
        run.conclusion,
        Some(
            CheckConclusion::Failure | CheckConclusion::TimedOut | CheckConclusion::ActionRequired
        )
    )
}

fn render_check_runs_section(
    f: &mut Frame,
    area: Rect,
    runs: &[crate::data::models::CheckRun],
    unicode: bool,
    theme: &Theme,
) {
    let app_w = runs.iter().map(|r| r.app.width()).max().unwrap_or(0);
    let app_w = u16::try_from(app_w.min(24)).unwrap_or(24);
    let result_w = runs
        .iter()
        .map(|r| check_conclusion_label(r.status, r.conclusion).width())
        .max()
        .unwrap_or(0);
    let result_w = u16::try_from(result_w.min(16)).unwrap_or(16);

    let columns = vec![
        Column::new("", Constraint::Length(3)),
        Column::new("Name", Constraint::Fill(2)).ellipsis(),
        Column::new("App", Constraint::Length(app_w)),
        Column::new("Result", Constraint::Length(result_w)),
        Column::new("Details", Constraint::Fill(3)).ellipsis(),
    ];

    let rows: Vec<Vec<Line>> = runs
        .iter()
        .map(|run| {
            let (icon, color_fn) = glyphs::glyph_for_check_run(run.status, run.conclusion, unicode);
            let label = check_conclusion_label(run.status, run.conclusion);
            let details = if check_run_is_failed(run) {
                run.url.clone()
            } else {
                String::new()
            };
            vec![
                Line::from(Span::styled(
                    icon.to_string(),
                    Style::default().fg(color_fn(theme)),
                )),
                Line::raw(run.name.clone()),
                Line::from(Span::styled(
                    run.app.clone(),
                    Style::default().fg(theme.dim),
                )),
                Line::from(Span::styled(label, Style::default().fg(color_fn(theme)))),
                Line::from(Span::styled(details, Style::default().fg(theme.dim))),
            ]
        })
        .collect();

    let label_area = Rect { height: 1, ..area };
    let table_area = Rect {
        y: area.y + 1,
        height: area.height.saturating_sub(1),
        ..area
    };
    f.render_widget(
        Line::styled(
            "Check Runs",
            Style::default()
                .fg(theme.primary)
                .add_modifier(Modifier::BOLD),
        ),
        label_area,
    );
    render_table(f, table_area, &columns, rows, None, false, None, theme);
}

fn render_commit_statuses_section(
    f: &mut Frame,
    area: Rect,
    statuses: &[crate::data::models::CommitStatus],
    unicode: bool,
    theme: &Theme,
) {
    let state_w = statuses
        .iter()
        .map(|s| status_state_label(s.state).width())
        .max()
        .unwrap_or(0);
    let state_w = u16::try_from(state_w.min(12)).unwrap_or(12);

    let columns = vec![
        Column::new("", Constraint::Length(3)),
        Column::new("Context", Constraint::Fill(2)).ellipsis(),
        Column::new("State", Constraint::Length(state_w)),
        Column::new("Details", Constraint::Fill(3)).ellipsis(),
    ];

    let rows: Vec<Vec<Line>> = statuses
        .iter()
        .map(|status| {
            let (icon, color_fn) = glyphs::glyph_for_status_state(status.state, unicode);
            let label = status_state_label(status.state);
            let details = if matches!(status.state, StatusState::Failure | StatusState::Error) {
                status
                    .description
                    .clone()
                    .or_else(|| status.target_url.clone())
                    .unwrap_or_default()
            } else {
                String::new()
            };
            vec![
                Line::from(Span::styled(
                    icon.to_string(),
                    Style::default().fg(color_fn(theme)),
                )),
                Line::raw(status.context.clone()),
                Line::from(Span::styled(label, Style::default().fg(color_fn(theme)))),
                Line::from(Span::styled(details, Style::default().fg(theme.dim))),
            ]
        })
        .collect();

    let label_area = Rect { height: 1, ..area };
    let table_area = Rect {
        y: area.y + 1,
        height: area.height.saturating_sub(1),
        ..area
    };
    f.render_widget(
        Line::styled(
            "Commit Statuses",
            Style::default()
                .fg(theme.primary)
                .add_modifier(Modifier::BOLD),
        ),
        label_area,
    );
    render_table(f, table_area, &columns, rows, None, false, None, theme);
}

pub fn render_checks(f: &mut Frame, area: Rect, checks: &PrChecks, theme: &Theme) {
    let unicode = theme.use_unicode_glyphs();
    if checks.runs.is_empty() && checks.statuses.is_empty() {
        let msg = Line::styled("No checks reported.", Style::default().fg(theme.dim));
        f.render_widget(msg, area);
        return;
    }

    let has_runs = !checks.runs.is_empty();
    let has_statuses = !checks.statuses.is_empty();

    let mut constraints: Vec<Constraint> = Vec::new();
    if has_runs {
        let n = u16::try_from(checks.runs.len()).unwrap_or(u16::MAX);
        constraints.push(Constraint::Length(2 + n));
    }
    if has_runs && has_statuses {
        constraints.push(Constraint::Length(1));
    }
    if has_statuses {
        let n = u16::try_from(checks.statuses.len()).unwrap_or(u16::MAX);
        constraints.push(Constraint::Length(2 + n));
    }
    constraints.push(Constraint::Min(0));

    let chunks = Layout::vertical(constraints).split(area);
    let mut idx = 0;

    if has_runs {
        render_check_runs_section(f, chunks[idx], &checks.runs, unicode, theme);
        idx += 1;
    }
    if has_runs && has_statuses {
        idx += 1; // skip gap
    }
    if has_statuses {
        render_commit_statuses_section(f, chunks[idx], &checks.statuses, unicode, theme);
    }
}

pub fn render_commits(
    f: &mut Frame,
    area: Rect,
    detail: &PrDetail,
    selected: usize,
    theme: &Theme,
) {
    let commits: Vec<(&str, &str, &str)> = detail
        .timeline
        .iter()
        .filter_map(|ev| match ev {
            TimelineEvent::Commit {
                sha,
                message,
                author,
                ..
            } => Some((sha.as_str(), message.as_str(), author.as_str())),
            _ => None,
        })
        .collect();

    if commits.is_empty() {
        let msg = Line::styled("No commits.", Style::default().fg(theme.dim));
        f.render_widget(msg, area);
        return;
    }

    let columns = vec![
        Column::new("SHA", Constraint::Length(7)),
        Column::new("Summary", Constraint::Fill(1)).ellipsis(),
        Column::new("Author", Constraint::Min(14)),
    ];

    let rows: Vec<Vec<Line>> = commits
        .iter()
        .map(|(sha, message, author)| {
            let short: String = sha.chars().take(7).collect();
            let headline = message.lines().next().unwrap_or("").to_string();
            vec![
                Line::styled(short, Style::default().fg(theme.warning)),
                Line::raw(headline),
                Line::styled(author.to_string(), Style::default().fg(theme.dim)),
            ]
        })
        .collect();

    let mut state = TableState::default();
    state.select(Some(selected.min(commits.len().saturating_sub(1))));

    render_table(f, area, &columns, rows, Some(&mut state), true, None, theme);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::models::{
        CheckConclusion, CheckRun, CheckStatus, ChecksRollup, CommitStatus, DiffSide, FileDiff,
        FileStatus, LineRange, Mergeable, PrChecks, PrFiles, PrId, PrSummary, Repo, ReviewComment,
        ReviewDecision, ReviewState, StatusState,
    };
    use chrono::TimeZone;
    use ratatui::{Terminal, backend::TestBackend};

    fn sample_summary() -> PrSummary {
        PrSummary {
            id: PrId {
                repo: Repo {
                    owner: "acme".into(),
                    name: "widgets".into(),
                },
                number: 42,
            },
            title: "Add warp drive".into(),
            is_draft: false,
            state: PrState::Open,
            author: "octocat".into(),
            base: "main".into(),
            head: "feature/warp".into(),
            additions: 10,
            deletions: 2,
            changed_files: 1,
            comments: 3,
            review_decision: Some(ReviewDecision::ReviewRequired),
            mergeable: Mergeable::Clean,
            checks: ChecksRollup::Success,
            labels: vec![],
            created_at: Utc.timestamp_opt(1_600_000_000, 0).unwrap(),
            updated_at: Utc.timestamp_opt(1_600_000_000, 0).unwrap(),
        }
    }

    #[test]
    fn renders_header_body_and_mixed_timeline() {
        let backend = TestBackend::new(80, 30);
        let mut terminal = Terminal::new(backend).unwrap();

        let t1 = Utc.timestamp_opt(1_600_000_000, 0).unwrap();
        let t2 = Utc.timestamp_opt(1_600_000_100, 0).unwrap();
        let t3 = Utc.timestamp_opt(1_600_000_200, 0).unwrap();
        let t4 = Utc.timestamp_opt(1_600_000_300, 0).unwrap();

        let detail = PrDetail {
            summary: sample_summary(),
            body: "Fixes #42.\nDetails here.".into(),
            body_html: None,
            requested_reviewers: vec![],
            assignees: vec![],
            head_sha: "abcd1234efgh".into(),
            merge_commit_sha: None,
            base_ref_oid: "main_sha".into(),
            available_merge_methods: vec![],
            version: 1,
            timeline: vec![
                TimelineEvent::IssueComment {
                    id: "c1".into(),
                    author: "alice".into(),
                    body: "Looks good but needs tests.".into(),
                    created_at: t1,
                },
                TimelineEvent::HeadRefForcePushed {
                    before: "old_sha".into(),
                    after: "abcd1234efgh".into(),
                    at: t2,
                },
                TimelineEvent::Review {
                    id: "r1".into(),
                    author: "bob".into(),
                    state: ReviewState::Approved,
                    body: "LGTM".into(),
                    submitted_at: t3,
                },
            ],
            review_threads: vec![ReviewThread {
                id: "rt1".into(),
                path: "src/lib.rs".into(),
                line_range: LineRange {
                    side: DiffSide::Right,
                    start_line: 10,
                    end_line: 10,
                    start_position: 1,
                    end_position: 1,
                },
                is_resolved: true,
                is_outdated: false,
                comments: vec![
                    ReviewComment {
                        id: "rc1".into(),
                        author: "charlie".into(),
                        body: "Typo here".into(),
                        created_at: t4,
                        commit_sha: "abcd1234efgh".into(),
                        suggestion: None,
                    },
                    ReviewComment {
                        id: "rc2".into(),
                        author: "octocat".into(),
                        body: "Fixed".into(),
                        created_at: Utc.timestamp_opt(1_600_000_400, 0).unwrap(),
                        commit_sha: "abcd1234efgh".into(),
                        suggestion: None,
                    },
                ],
            }],
        };

        let now = Utc.with_ymd_and_hms(2026, 6, 1, 0, 0, 0).unwrap();

        terminal
            .draw(|f| {
                let area = f.area();
                render_conversation(
                    f,
                    area,
                    &detail,
                    None,
                    ConversationScroll {
                        cursor: 0,
                        scroll_to_focused: false,
                        now,
                    },
                    &Theme::dark(),
                );
            })
            .unwrap();

        insta::assert_snapshot!("renders_header_body_and_mixed_timeline", terminal.backend());
    }

    #[test]
    fn renders_thread_with_all_comments_and_suggestion_block() {
        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).unwrap();

        let t1 = Utc.timestamp_opt(1_600_000_000, 0).unwrap();
        let t2 = Utc.timestamp_opt(1_600_000_100, 0).unwrap();

        let detail = PrDetail {
            summary: sample_summary(),
            body: String::new(),
            body_html: None,
            requested_reviewers: vec![],
            assignees: vec![],
            head_sha: "abc1234".into(),
            merge_commit_sha: None,
            base_ref_oid: "main_sha".into(),
            available_merge_methods: vec![],
            version: 1,
            timeline: vec![],
            review_threads: vec![ReviewThread {
                id: "rt1".into(),
                path: "src/lib.rs".into(),
                line_range: LineRange {
                    side: DiffSide::Right,
                    start_line: 10,
                    end_line: 10,
                    start_position: 1,
                    end_position: 1,
                },
                is_resolved: false,
                is_outdated: false,
                comments: vec![
                    ReviewComment {
                        id: "rc1".into(),
                        author: "alice".into(),
                        body: "Consider this:\n```suggestion\nlet x = 1;\n```".into(),
                        created_at: t1,
                        commit_sha: "abc1234".into(),
                        suggestion: Some("let x = 1;\n".into()),
                    },
                    ReviewComment {
                        id: "rc2".into(),
                        author: "bob".into(),
                        body: "Thanks!".into(),
                        created_at: t2,
                        commit_sha: "abc1234".into(),
                        suggestion: None,
                    },
                ],
            }],
        };

        let now = Utc.with_ymd_and_hms(2026, 6, 1, 0, 0, 0).unwrap();

        terminal
            .draw(|f| {
                let area = f.area();
                render_conversation(
                    f,
                    area,
                    &detail,
                    None,
                    ConversationScroll {
                        cursor: 0,
                        scroll_to_focused: false,
                        now,
                    },
                    &Theme::dark(),
                );
            })
            .unwrap();

        let backend_ref = terminal.backend();
        let buf = backend_ref.buffer();
        let mut rendered = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                rendered.push_str(buf[(x, y)].symbol());
            }
            rendered.push('\n');
        }

        // Both comments must render (not just the first).
        assert!(
            rendered.contains("alice") && rendered.contains("bob"),
            "expected both comment authors to render, got:\n{rendered}"
        );
        assert!(
            rendered.contains("Thanks!"),
            "expected second comment body to render, got:\n{rendered}"
        );
        // Suggestion block must render with a badge and the suggested line prefixed `+`.
        assert!(
            rendered.contains("[suggestion]"),
            "expected `[suggestion]` badge to render, got:\n{rendered}"
        );
        assert!(
            rendered.contains("+let x = 1;"),
            "expected suggestion body to render as `+let x = 1;`, got:\n{rendered}"
        );

        insta::assert_snapshot!(
            "renders_thread_with_all_comments_and_suggestion_block",
            terminal.backend()
        );
    }

    #[test]
    fn renders_draft_with_empty_timeline() {
        let backend = TestBackend::new(80, 10);
        let mut terminal = Terminal::new(backend).unwrap();

        let mut summary = sample_summary();
        summary.is_draft = true;

        let detail = PrDetail {
            summary,
            body: String::new(),
            body_html: None,
            requested_reviewers: vec![],
            assignees: vec![],
            head_sha: "abcd123".into(),
            merge_commit_sha: None,
            base_ref_oid: "main_sha".into(),
            available_merge_methods: vec![],
            version: 1,
            timeline: vec![],
            review_threads: vec![],
        };

        let now = Utc.with_ymd_and_hms(2026, 6, 1, 0, 0, 0).unwrap();

        terminal
            .draw(|f| {
                let area = f.area();
                render_conversation(
                    f,
                    area,
                    &detail,
                    None,
                    ConversationScroll {
                        cursor: 0,
                        scroll_to_focused: false,
                        now,
                    },
                    &Theme::dark(),
                );
            })
            .unwrap();

        insta::assert_snapshot!("renders_draft_with_empty_timeline", terminal.backend());
    }

    fn sample_file_diff(
        path: &str,
        status: FileStatus,
        additions: u32,
        deletions: u32,
        previous_path: Option<&str>,
        is_binary: bool,
        oversize: bool,
    ) -> FileDiff {
        FileDiff {
            path: path.into(),
            previous_path: previous_path.map(String::from),
            status,
            additions,
            deletions,
            patch: None,
            is_binary,
            is_submodule: false,
            is_generated: None,
            oversize,
            blob_sha_before: None,
            blob_sha_after: None,
        }
    }

    fn sample_detail_with_resolved_thread() -> PrDetail {
        let t1 = Utc.timestamp_opt(1_600_000_000, 0).unwrap();
        let t2 = Utc.timestamp_opt(1_600_000_100, 0).unwrap();
        PrDetail {
            summary: sample_summary(),
            body: "PR body.".into(),
            body_html: None,
            requested_reviewers: vec![],
            assignees: vec![],
            head_sha: "abc1234".into(),
            merge_commit_sha: None,
            base_ref_oid: "main_sha".into(),
            available_merge_methods: vec![],
            version: 1,
            timeline: vec![],
            review_threads: vec![ReviewThread {
                id: "rt1".into(),
                path: "src/lib.rs".into(),
                line_range: LineRange {
                    side: DiffSide::Right,
                    start_line: 5,
                    end_line: 5,
                    start_position: 1,
                    end_position: 1,
                },
                is_resolved: true,
                is_outdated: false,
                comments: vec![
                    ReviewComment {
                        id: "rc1".into(),
                        author: "alice".into(),
                        body: "Needs fix.".into(),
                        created_at: t1,
                        commit_sha: "abc1234".into(),
                        suggestion: None,
                    },
                    ReviewComment {
                        id: "rc2".into(),
                        author: "bob".into(),
                        body: "Done.".into(),
                        created_at: t2,
                        commit_sha: "abc1234".into(),
                        suggestion: None,
                    },
                ],
            }],
        }
    }

    #[test]
    fn renders_resolved_thread_collapsed() {
        let backend = TestBackend::new(80, 15);
        let mut terminal = Terminal::new(backend).unwrap();
        let detail = sample_detail_with_resolved_thread();
        let now = Utc.with_ymd_and_hms(2026, 6, 1, 0, 0, 0).unwrap();

        terminal
            .draw(|f| {
                let area = f.area();
                // Not focused → thread is resolved → collapsed
                render_conversation(
                    f,
                    area,
                    &detail,
                    None,
                    ConversationScroll {
                        cursor: 0,
                        scroll_to_focused: false,
                        now,
                    },
                    &Theme::dark(),
                );
            })
            .unwrap();

        insta::assert_snapshot!("renders_resolved_thread_collapsed", terminal.backend());
    }

    #[test]
    fn renders_resolved_thread_focused_expanded() {
        let backend = TestBackend::new(80, 15);
        let mut terminal = Terminal::new(backend).unwrap();
        let detail = sample_detail_with_resolved_thread();
        let now = Utc.with_ymd_and_hms(2026, 6, 1, 0, 0, 0).unwrap();

        terminal
            .draw(|f| {
                let area = f.area();
                // focused thread 0 → expanded even though resolved
                render_conversation(
                    f,
                    area,
                    &detail,
                    Some(0),
                    ConversationScroll {
                        cursor: 0,
                        scroll_to_focused: true,
                        now,
                    },
                    &Theme::dark(),
                );
            })
            .unwrap();

        insta::assert_snapshot!(
            "renders_resolved_thread_focused_expanded",
            terminal.backend()
        );
    }

    #[test]
    fn renders_conversation_mid_scroll() {
        // Use a tall content / small viewport to trigger the scrollbar.
        let backend = TestBackend::new(80, 8);
        let mut terminal = Terminal::new(backend).unwrap();

        let t1 = Utc.timestamp_opt(1_600_000_000, 0).unwrap();
        let t2 = Utc.timestamp_opt(1_600_000_100, 0).unwrap();
        let t3 = Utc.timestamp_opt(1_600_000_200, 0).unwrap();
        let detail = PrDetail {
            summary: sample_summary(),
            body: "Line 1\nLine 2\nLine 3\nLine 4\nLine 5".into(),
            body_html: None,
            requested_reviewers: vec![],
            assignees: vec![],
            head_sha: "abc1234".into(),
            merge_commit_sha: None,
            base_ref_oid: "main_sha".into(),
            available_merge_methods: vec![],
            version: 1,
            timeline: vec![
                TimelineEvent::IssueComment {
                    id: "c1".into(),
                    author: "alice".into(),
                    body: "Nice PR!".into(),
                    created_at: t1,
                },
                TimelineEvent::IssueComment {
                    id: "c2".into(),
                    author: "bob".into(),
                    body: "Agreed!".into(),
                    created_at: t2,
                },
                TimelineEvent::IssueComment {
                    id: "c3".into(),
                    author: "carol".into(),
                    body: "Merging.".into(),
                    created_at: t3,
                },
            ],
            review_threads: vec![],
        };
        let now = Utc.with_ymd_and_hms(2026, 6, 1, 0, 0, 0).unwrap();

        terminal
            .draw(|f| {
                let area = f.area();
                render_conversation(
                    f,
                    area,
                    &detail,
                    None,
                    ConversationScroll {
                        cursor: 5,
                        scroll_to_focused: false,
                        now,
                    },
                    &Theme::dark(),
                );
            })
            .unwrap();

        insta::assert_snapshot!("renders_conversation_mid_scroll", terminal.backend());
    }

    #[test]
    fn renders_files_tab_with_mixed_statuses() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();

        let files = PrFiles {
            truncated: false,
            files: vec![
                sample_file_diff("src/main.rs", FileStatus::Added, 10, 0, None, false, false),
                sample_file_diff("src/lib.rs", FileStatus::Modified, 5, 2, None, false, false),
                sample_file_diff(
                    "new/foo.rs",
                    FileStatus::Renamed,
                    1,
                    1,
                    Some("old/foo.rs"),
                    false,
                    false,
                ),
                sample_file_diff(
                    "removed.txt",
                    FileStatus::Removed,
                    0,
                    100,
                    None,
                    false,
                    false,
                ),
                sample_file_diff("image.png", FileStatus::Modified, 0, 0, None, true, false),
                sample_file_diff("large.txt", FileStatus::Added, 5000, 0, None, false, true),
            ],
        };

        terminal
            .draw(|f| {
                let area = f.area();
                render_files(f, area, &files, 2, &Theme::dark());
            })
            .unwrap();

        insta::assert_snapshot!("renders_files_tab_with_mixed_statuses", terminal.backend());
    }

    fn sample_check_run(
        name: &str,
        status: CheckStatus,
        conclusion: Option<CheckConclusion>,
    ) -> CheckRun {
        CheckRun {
            name: name.into(),
            app: "ci-bot".into(),
            status,
            conclusion,
            url: format!("https://example.com/{name}"),
            started_at: None,
            completed_at: None,
        }
    }

    fn sample_commit_status(
        context: &str,
        state: StatusState,
        description: Option<&str>,
    ) -> CommitStatus {
        CommitStatus {
            context: context.into(),
            state,
            target_url: Some(format!("https://example.com/status/{context}")),
            description: description.map(str::to_string),
        }
    }

    #[test]
    fn renders_checks_tab_with_runs_and_statuses() {
        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).unwrap();

        let checks = PrChecks {
            runs: vec![
                sample_check_run(
                    "build",
                    CheckStatus::Completed,
                    Some(CheckConclusion::Success),
                ),
                sample_check_run(
                    "tests",
                    CheckStatus::Completed,
                    Some(CheckConclusion::Failure),
                ),
                sample_check_run("lint", CheckStatus::InProgress, None),
                sample_check_run("deploy", CheckStatus::Queued, None),
            ],
            statuses: vec![
                sample_commit_status("ci/circleci", StatusState::Success, Some("Build OK")),
                sample_commit_status(
                    "ci/heroku",
                    StatusState::Failure,
                    Some("Deploy failed: timeout"),
                ),
            ],
        };

        terminal
            .draw(|f| {
                let area = f.area();
                render_checks(f, area, &checks, &Theme::dark());
            })
            .unwrap();

        insta::assert_snapshot!(
            "renders_checks_tab_with_runs_and_statuses",
            terminal.backend()
        );
    }

    #[test]
    fn renders_checks_tab_empty() {
        let backend = TestBackend::new(80, 5);
        let mut terminal = Terminal::new(backend).unwrap();

        let checks = PrChecks {
            runs: vec![],
            statuses: vec![],
        };

        terminal
            .draw(|f| {
                let area = f.area();
                render_checks(f, area, &checks, &Theme::dark());
            })
            .unwrap();

        insta::assert_snapshot!("renders_checks_tab_empty", terminal.backend());
    }

    fn sample_pr_detail_with_commits(commits: Vec<(&str, &str, &str)>) -> PrDetail {
        PrDetail {
            summary: sample_summary(),
            body: String::new(),
            body_html: None,
            requested_reviewers: vec![],
            assignees: vec![],
            head_sha: "deadbeef".into(),
            merge_commit_sha: None,
            base_ref_oid: "feedface".into(),
            review_threads: vec![],
            timeline: commits
                .into_iter()
                .enumerate()
                .map(|(i, (sha, msg, author))| TimelineEvent::Commit {
                    sha: sha.into(),
                    message: msg.into(),
                    author: author.into(),
                    pushed_at: chrono::Utc
                        .with_ymd_and_hms(2025, 1, 1, 0, 0, u32::try_from(i).unwrap_or(0))
                        .unwrap(),
                })
                .collect(),
            available_merge_methods: vec![],
            version: 1,
        }
    }

    #[test]
    fn renders_commits_tab_with_multiple_commits() {
        let backend = TestBackend::new(80, 10);
        let mut terminal = Terminal::new(backend).unwrap();

        let detail = sample_pr_detail_with_commits(vec![
            ("abc1234567890", "Add feature X", "alice"),
            ("def7654321098", "Fix bug in Y\n\nDetails follow.", "bob"),
            ("9876543210abc", "Refactor Z", "carol"),
        ]);

        terminal
            .draw(|f| {
                let area = f.area();
                render_commits(f, area, &detail, 1, &Theme::dark());
            })
            .unwrap();

        insta::assert_snapshot!(
            "renders_commits_tab_with_multiple_commits",
            terminal.backend()
        );
    }

    #[test]
    fn renders_commits_tab_empty() {
        let backend = TestBackend::new(80, 5);
        let mut terminal = Terminal::new(backend).unwrap();

        let detail = sample_pr_detail_with_commits(vec![]);

        terminal
            .draw(|f| {
                let area = f.area();
                render_commits(f, area, &detail, 0, &Theme::dark());
            })
            .unwrap();

        insta::assert_snapshot!("renders_commits_tab_empty", terminal.backend());
    }
}
