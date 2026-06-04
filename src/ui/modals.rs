//! Centered overlay renderers for the PR-detail write modals: submit-review,
//! merge, and close/reopen confirmation.
//!
//! These mirror the key-handler state in [`crate::app::update::overlays`] and
//! are layered over the view by [`crate::app::render::render_app`]. On a
//! terminal below the 60×18 overlay threshold they fill the whole area, the
//! same fallback used by [`crate::ui::components::modal`].

use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};

use crate::app::state::{
    CloseReopenAction, CloseReopenConfirmState, MergeModalKind, MergeModalState,
    REVIEW_MODAL_OPTIONS, ReviewModalOption, ReviewModalState,
};
use crate::data::models::MergeMethod;
use crate::ui::layout::{LayoutMode, min_size_check};
use crate::ui::theme::Theme;

/// Compute a centered popup `Rect`, falling back to the full area on terminals
/// too small for the standard overlay layout.
fn centered(area: Rect, desired_w: u16, desired_h: u16) -> Rect {
    if matches!(
        min_size_check(area.width, area.height),
        LayoutMode::FullScreenModal
    ) {
        return area;
    }
    let w = desired_w.clamp(1, area.width.saturating_sub(2).max(1));
    let h = desired_h.clamp(1, area.height.saturating_sub(2).max(1));
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    Rect::new(x, y, w, h)
}

/// Clear `popup`, draw the bordered frame, and return the inner content area.
fn frame(f: &mut Frame, popup: Rect, title: &str, theme: &Theme) -> Rect {
    f.render_widget(Clear, popup);
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_type(theme.border_type())
        .style(Style::default().bg(theme.bg).fg(theme.fg))
        .border_style(Style::default().fg(theme.primary));
    let inner = block.inner(popup);
    f.render_widget(block, popup);
    inner
}

fn option_line(label: &str, selected: bool, theme: &Theme) -> Line<'static> {
    let (marker, style) = if selected {
        (
            "\u{203a} ",
            Style::default()
                .fg(theme.bg)
                .bg(theme.primary)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        ("  ", Style::default().fg(theme.fg))
    };
    Line::from(Span::styled(format!("{marker}{label}"), style))
}

fn hint_line(text: &'static str, theme: &Theme) -> Line<'static> {
    Line::from(Span::styled(
        text,
        Style::default().fg(theme.dim).add_modifier(Modifier::DIM),
    ))
}

/// Render the submit-review modal: a three-way decision picker plus an inline
/// comment field.
pub fn render_review_modal(f: &mut Frame, area: Rect, modal: &ReviewModalState, theme: &Theme) {
    let inner = frame(f, centered(area, 62, 12), " Submit Review ", theme);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let mut lines: Vec<Line> = REVIEW_MODAL_OPTIONS
        .iter()
        .enumerate()
        .map(|(i, opt)| option_line(review_option_label(*opt), i == modal.selected, theme))
        .collect();

    lines.push(Line::from(""));
    if modal.body.is_empty() {
        lines.push(Line::from(vec![
            Span::styled(
                "Comment ",
                Style::default().fg(theme.dim).add_modifier(Modifier::DIM),
            ),
            Span::styled(
                "(required for \u{201c}Comment\u{201d})",
                Style::default().fg(theme.dim).add_modifier(Modifier::DIM),
            ),
            Span::styled("\u{2588}", Style::default().fg(theme.primary)),
        ]));
    } else {
        lines.push(Line::from(vec![
            Span::styled(modal.body.clone(), Style::default().fg(theme.fg)),
            Span::styled("\u{2588}", Style::default().fg(theme.primary)),
        ]));
    }

    lines.push(Line::from(""));
    lines.push(hint_line(
        "\u{2191}\u{2193} choose \u{00b7} type comment \u{00b7} \u{2303}S submit \u{00b7} esc",
        theme,
    ));

    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

/// Render the merge modal: a method picker, or a "blocked" explanation.
pub fn render_merge_modal(f: &mut Frame, area: Rect, modal: &MergeModalState, theme: &Theme) {
    match &modal.kind {
        MergeModalKind::MethodPicker { methods, selected } => {
            let inner = frame(f, centered(area, 54, 8), " Merge ", theme);
            if inner.width == 0 || inner.height == 0 {
                return;
            }
            let mut lines: Vec<Line> = if methods.is_empty() {
                vec![Line::from(Span::styled(
                    "No merge methods are enabled for this repository.",
                    Style::default().fg(theme.warning),
                ))]
            } else {
                methods
                    .iter()
                    .enumerate()
                    .map(|(i, m)| option_line(merge_method_label(*m), i == *selected, theme))
                    .collect()
            };
            lines.push(Line::from(""));
            lines.push(hint_line(
                "\u{2191}\u{2193} choose \u{00b7} \u{21b5} merge \u{00b7} esc",
                theme,
            ));
            f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
        }
        MergeModalKind::Blocked { reasons } => {
            let inner = frame(f, centered(area, 60, 10), " Merge blocked ", theme);
            if inner.width == 0 || inner.height == 0 {
                return;
            }
            let mut lines = vec![Line::from(Span::styled(
                "This pull request can\u{2019}t be merged yet:",
                Style::default().fg(theme.fg),
            ))];
            if reasons.is_empty() {
                lines.push(Line::from(Span::styled(
                    "  \u{2022} Branch protection requirements are not satisfied.",
                    Style::default().fg(theme.warning),
                )));
            } else {
                for reason in reasons {
                    lines.push(Line::from(Span::styled(
                        format!("  \u{2022} {reason}"),
                        Style::default().fg(theme.warning),
                    )));
                }
            }
            lines.push(Line::from(""));
            lines.push(hint_line("esc to dismiss", theme));
            f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
        }
    }
}

/// Render the close/reopen confirmation prompt.
pub fn render_close_reopen_confirm(
    f: &mut Frame,
    area: Rect,
    confirm: &CloseReopenConfirmState,
    theme: &Theme,
) {
    let (title, prompt) = match confirm.action {
        CloseReopenAction::Close => (
            " Close PR ",
            format!("Close pull request #{}?", confirm.pr_id.number),
        ),
        CloseReopenAction::Reopen => (
            " Reopen PR ",
            format!("Reopen pull request #{}?", confirm.pr_id.number),
        ),
    };
    let inner = frame(f, centered(area, 52, 5), title, theme);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let lines = vec![
        Line::from(Span::styled(prompt, Style::default().fg(theme.fg))),
        Line::from(""),
        hint_line("y confirm \u{00b7} n cancel", theme),
    ];
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

fn review_option_label(opt: ReviewModalOption) -> &'static str {
    match opt {
        ReviewModalOption::Approve => "Approve",
        ReviewModalOption::RequestChanges => "Request changes",
        ReviewModalOption::Comment => "Comment",
    }
}

fn merge_method_label(method: MergeMethod) -> &'static str {
    match method {
        MergeMethod::Merge => "Create a merge commit",
        MergeMethod::Squash => "Squash and merge",
        MergeMethod::Rebase => "Rebase and merge",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::models::{PrId, Repo};
    use ratatui::{Terminal, backend::TestBackend};

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

    fn pr_id() -> PrId {
        PrId {
            repo: Repo {
                owner: "acme".into(),
                name: "widgets".into(),
            },
            number: 42,
        }
    }

    fn render<F: FnOnce(&mut Frame)>(draw: F) -> String {
        let mut t = Terminal::new(TestBackend::new(80, 24)).unwrap();
        t.draw(|f| draw(f)).unwrap();
        buf_string(&t)
    }

    #[test]
    fn review_modal_renders_options_body_placeholder_and_hint() {
        let modal = ReviewModalState {
            pr_id: pr_id(),
            selected: 1,
            body: String::new(),
        };
        let s = render(|f| render_review_modal(f, f.area(), &modal, &Theme::dark()));
        assert!(s.contains("Submit Review"), "title:\n{s}");
        assert!(
            s.contains("Approve") && s.contains("Request changes") && s.contains("Comment"),
            "options:\n{s}"
        );
        assert!(s.contains("submit"), "hint:\n{s}");
    }

    #[test]
    fn review_modal_renders_typed_body() {
        let modal = ReviewModalState {
            pr_id: pr_id(),
            selected: 2,
            body: "ship it".into(),
        };
        let s = render(|f| render_review_modal(f, f.area(), &modal, &Theme::dark()));
        assert!(s.contains("ship it"), "body text missing:\n{s}");
    }

    #[test]
    fn merge_modal_method_picker_lists_methods() {
        let modal = MergeModalState {
            pr_id: pr_id(),
            head_sha: "deadbeef".into(),
            kind: MergeModalKind::MethodPicker {
                methods: vec![MergeMethod::Squash, MergeMethod::Merge],
                selected: 0,
            },
        };
        let s = render(|f| render_merge_modal(f, f.area(), &modal, &Theme::dark()));
        assert!(s.contains("Merge"), "title:\n{s}");
        assert!(
            s.contains("Squash and merge") && s.contains("Create a merge commit"),
            "method labels:\n{s}"
        );
    }

    #[test]
    fn merge_modal_blocked_lists_reasons() {
        let modal = MergeModalState {
            pr_id: pr_id(),
            head_sha: "deadbeef".into(),
            kind: MergeModalKind::Blocked {
                reasons: vec!["Review required".into(), "Checks failing".into()],
            },
        };
        let s = render(|f| render_merge_modal(f, f.area(), &modal, &Theme::dark()));
        assert!(s.contains("blocked"), "title:\n{s}");
        assert!(
            s.contains("Review required") && s.contains("Checks failing"),
            "reasons:\n{s}"
        );
    }

    #[test]
    fn close_confirm_prompts_with_pr_number() {
        let confirm = CloseReopenConfirmState {
            pr_id: pr_id(),
            action: CloseReopenAction::Close,
        };
        let s = render(|f| render_close_reopen_confirm(f, f.area(), &confirm, &Theme::dark()));
        assert!(s.contains("Close pull request #42?"), "prompt:\n{s}");
        assert!(s.contains("confirm"), "hint:\n{s}");
    }

    #[test]
    fn reopen_confirm_uses_reopen_wording() {
        let confirm = CloseReopenConfirmState {
            pr_id: pr_id(),
            action: CloseReopenAction::Reopen,
        };
        let s = render(|f| render_close_reopen_confirm(f, f.area(), &confirm, &Theme::dark()));
        assert!(s.contains("Reopen pull request #42?"), "prompt:\n{s}");
    }
}
