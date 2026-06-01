//! Centralised status-glyph set.
//!
//! Every status icon used across the TUI lives here.  Callers pass
//! `unicode: bool` (from [`crate::ui::theme::Theme::use_unicode_glyphs`]) and
//! receive the appropriate `&'static str`.  All matches are exhaustive — no
//! `_` wildcard — so an unhandled variant becomes a compile error.

use ratatui::style::Color;

use crate::data::models::{
    CheckConclusion, CheckStatus, ChecksRollup, PrState, ReviewDecision, StatusState, TimelineEvent,
};
use crate::ui::theme::Theme;

// ── Checks-rollup glyphs ─────────────────────────────────────────────────────

pub const CHECK_SUCCESS: &str = "✓";
pub const CHECK_FAILURE: &str = "✗";
pub const CHECK_PENDING: &str = "●";
pub const CHECK_NEUTRAL: &str = "–";
pub const CHECK_MIXED: &str = "◑";
pub const CHECK_NONE: &str = "–";

// ── Review-decision glyphs ───────────────────────────────────────────────────

pub const REVIEW_APPROVED: &str = "✔";
pub const REVIEW_CHANGES: &str = "✎";
pub const REVIEW_REQUIRED: &str = "·";

// ── PR state / draft glyphs ──────────────────────────────────────────────────

pub const GLYPH_DRAFT: &str = "◌";
pub const GLYPH_OPEN: &str = " ";
pub const GLYPH_MERGED: &str = "✓";
pub const GLYPH_CLOSED: &str = "✗";

// ── Mapping functions ────────────────────────────────────────────────────────

/// Map a [`ChecksRollup`] to its display glyph.
///
/// The match is exhaustive (no `_` wildcard) so a future variant causes a
/// compile error rather than a silent fallback.
#[must_use]
pub fn glyph_for_checks_rollup(r: ChecksRollup, unicode: bool) -> &'static str {
    match r {
        ChecksRollup::Success => {
            if unicode {
                CHECK_SUCCESS
            } else {
                "+"
            }
        }
        ChecksRollup::Failure => {
            if unicode {
                CHECK_FAILURE
            } else {
                "!"
            }
        }
        ChecksRollup::Pending => {
            if unicode {
                CHECK_PENDING
            } else {
                "?"
            }
        }
        ChecksRollup::Neutral => {
            if unicode {
                CHECK_NEUTRAL
            } else {
                "-"
            }
        }
        ChecksRollup::Mixed => {
            if unicode {
                CHECK_MIXED
            } else {
                "~"
            }
        }
        ChecksRollup::None => {
            if unicode {
                CHECK_NONE
            } else {
                "-"
            }
        }
    }
}

/// Map an `Option<ReviewDecision>` to its display glyph.
#[must_use]
pub fn glyph_for_review_decision(d: Option<ReviewDecision>, unicode: bool) -> &'static str {
    match d {
        Some(ReviewDecision::Approved) => {
            if unicode {
                REVIEW_APPROVED
            } else {
                "A"
            }
        }
        Some(ReviewDecision::ChangesRequested) => {
            if unicode {
                REVIEW_CHANGES
            } else {
                "~"
            }
        }
        Some(ReviewDecision::ReviewRequired) | None => {
            if unicode {
                REVIEW_REQUIRED
            } else {
                "."
            }
        }
    }
}

/// Map `(is_draft, PrState)` to its display glyph.
///
/// Draft status takes priority over `PrState`.
#[must_use]
pub fn glyph_for_pr_state(is_draft: bool, state: PrState, unicode: bool) -> &'static str {
    if is_draft {
        return if unicode { GLYPH_DRAFT } else { "o" };
    }
    match state {
        PrState::Open => {
            if unicode {
                GLYPH_OPEN
            } else {
                " "
            }
        }
        PrState::Merged => {
            if unicode {
                GLYPH_MERGED
            } else {
                "M"
            }
        }
        PrState::Closed => {
            if unicode {
                GLYPH_CLOSED
            } else {
                "C"
            }
        }
    }
}

/// Individual check-run icon. Returns `(glyph, color_fn)`.
///
/// The `unicode` flag controls the glyph characters for ASCII-safe contexts,
/// but the non-Unicode path keeps the same symbols for now since the existing
/// code never had ASCII variants for individual runs.
#[must_use]
pub fn glyph_for_check_run(
    status: CheckStatus,
    conclusion: Option<CheckConclusion>,
    unicode: bool,
) -> (&'static str, fn(&Theme) -> Color) {
    match status {
        CheckStatus::Queued => (if unicode { "○" } else { "o" }, |t: &Theme| t.dim),
        CheckStatus::InProgress => (if unicode { "◐" } else { "*" }, |t: &Theme| t.warning),
        CheckStatus::Completed => match conclusion {
            Some(CheckConclusion::Success) => {
                (if unicode { "✓" } else { "+" }, |t: &Theme| t.success)
            }
            Some(CheckConclusion::Failure) => {
                (if unicode { "✗" } else { "x" }, |t: &Theme| t.error)
            }
            Some(CheckConclusion::TimedOut) => {
                (if unicode { "✗" } else { "x" }, |t: &Theme| t.error)
            }
            Some(CheckConclusion::ActionRequired) => ("!", |t: &Theme| t.warning),
            Some(CheckConclusion::Cancelled) => {
                (if unicode { "⊘" } else { "/" }, |t: &Theme| t.dim)
            }
            Some(CheckConclusion::Skipped) => {
                (if unicode { "→" } else { ">" }, |t: &Theme| t.dim)
            }
            Some(CheckConclusion::Neutral) => (if unicode { "·" } else { "." }, |t: &Theme| t.dim),
            Some(CheckConclusion::Stale) => (if unicode { "∼" } else { "~" }, |t: &Theme| t.dim),
            None => ("?", |t: &Theme| t.dim),
        },
    }
}

/// Commit-status icon. Returns `(glyph, color_fn)`.
#[must_use]
pub fn glyph_for_status_state(
    state: StatusState,
    unicode: bool,
) -> (&'static str, fn(&Theme) -> Color) {
    match state {
        StatusState::Pending => (if unicode { "○" } else { "o" }, |t: &Theme| t.warning),
        StatusState::Success => (if unicode { "✓" } else { "+" }, |t: &Theme| t.success),
        StatusState::Failure => (if unicode { "✗" } else { "x" }, |t: &Theme| t.error),
        StatusState::Error => (if unicode { "✗" } else { "x" }, |t: &Theme| t.error),
    }
}

// ── Timeline event glyphs ───────────────────────────────────────────────────

/// Return the glyph for a [`TimelineEvent`] arm.
///
/// The `unicode` flag selects between Unicode and ASCII fallbacks.
/// The arrow variant (`→` / `->`) is used for the `HeadRefForcePushed` separator.
#[must_use]
pub fn glyph_for_timeline_event(event: &TimelineEvent, unicode: bool) -> &'static str {
    match event {
        TimelineEvent::Commit { .. } => {
            if unicode {
                "●"
            } else {
                "*"
            }
        }
        TimelineEvent::IssueComment { .. } => {
            if unicode {
                "✎"
            } else {
                "~"
            }
        }
        TimelineEvent::Review { .. } => {
            if unicode {
                "★"
            } else {
                "*"
            }
        }
        TimelineEvent::Merged { .. } => {
            if unicode {
                "✓"
            } else {
                "+"
            }
        }
        TimelineEvent::Closed { .. } => {
            if unicode {
                "✗"
            } else {
                "x"
            }
        }
        TimelineEvent::Reopened { .. } => {
            if unicode {
                "↻"
            } else {
                "o"
            }
        }
        TimelineEvent::HeadRefForcePushed { .. } => {
            if unicode {
                "→"
            } else {
                "->"
            }
        }
    }
}

// ── Glyph-color helpers ───────────────────────────────────────────────────────

/// Return the [`Color`] for a PR state indicator.
#[must_use]
pub fn color_for_pr_state(is_draft: bool, state: PrState, theme: &Theme) -> Color {
    if is_draft {
        return theme.dim;
    }
    match state {
        PrState::Open => theme.success,
        PrState::Closed => theme.error,
        PrState::Merged => theme.primary,
    }
}

/// Return the [`Color`] for a checks-rollup indicator.
#[must_use]
pub fn color_for_checks_rollup(r: ChecksRollup, theme: &Theme) -> Color {
    match r {
        ChecksRollup::Success => theme.success,
        ChecksRollup::Failure => theme.error,
        ChecksRollup::Pending | ChecksRollup::Mixed => theme.warning,
        ChecksRollup::Neutral | ChecksRollup::None => theme.dim,
    }
}

/// Return the [`Color`] for a review-decision indicator.
#[must_use]
pub fn color_for_review_decision(d: Option<ReviewDecision>, theme: &Theme) -> Color {
    match d {
        Some(ReviewDecision::Approved) => theme.success,
        Some(ReviewDecision::ChangesRequested) => theme.error,
        Some(ReviewDecision::ReviewRequired) | None => theme.dim,
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ChecksRollup — all six variants, unicode and ASCII

    #[test]
    fn checks_rollup_unicode() {
        assert_eq!(glyph_for_checks_rollup(ChecksRollup::Success, true), "✓");
        assert_eq!(glyph_for_checks_rollup(ChecksRollup::Failure, true), "✗");
        assert_eq!(glyph_for_checks_rollup(ChecksRollup::Pending, true), "●");
        assert_eq!(glyph_for_checks_rollup(ChecksRollup::Neutral, true), "–");
        assert_eq!(glyph_for_checks_rollup(ChecksRollup::Mixed, true), "◑");
        assert_eq!(glyph_for_checks_rollup(ChecksRollup::None, true), "–");
    }

    #[test]
    fn checks_rollup_ascii() {
        assert_eq!(glyph_for_checks_rollup(ChecksRollup::Success, false), "+");
        assert_eq!(glyph_for_checks_rollup(ChecksRollup::Failure, false), "!");
        assert_eq!(glyph_for_checks_rollup(ChecksRollup::Pending, false), "?");
        assert_eq!(glyph_for_checks_rollup(ChecksRollup::Neutral, false), "-");
        assert_eq!(glyph_for_checks_rollup(ChecksRollup::Mixed, false), "~");
        assert_eq!(glyph_for_checks_rollup(ChecksRollup::None, false), "-");
    }

    // ReviewDecision — all variants

    #[test]
    fn review_decision_unicode() {
        assert_eq!(
            glyph_for_review_decision(Some(ReviewDecision::Approved), true),
            "✔"
        );
        assert_eq!(
            glyph_for_review_decision(Some(ReviewDecision::ChangesRequested), true),
            "✎"
        );
        assert_eq!(
            glyph_for_review_decision(Some(ReviewDecision::ReviewRequired), true),
            "·"
        );
        assert_eq!(glyph_for_review_decision(None, true), "·");
    }

    #[test]
    fn review_decision_ascii() {
        assert_eq!(
            glyph_for_review_decision(Some(ReviewDecision::Approved), false),
            "A"
        );
        assert_eq!(
            glyph_for_review_decision(Some(ReviewDecision::ChangesRequested), false),
            "~"
        );
        assert_eq!(
            glyph_for_review_decision(Some(ReviewDecision::ReviewRequired), false),
            "."
        );
        assert_eq!(glyph_for_review_decision(None, false), ".");
    }

    // PR state / draft

    #[test]
    fn draft_overrides_state() {
        assert_eq!(glyph_for_pr_state(true, PrState::Open, true), "◌");
        assert_eq!(glyph_for_pr_state(true, PrState::Merged, true), "◌");
        assert_eq!(glyph_for_pr_state(true, PrState::Closed, true), "◌");
    }

    #[test]
    fn draft_ascii_fallback() {
        assert_eq!(glyph_for_pr_state(true, PrState::Open, false), "o");
    }

    #[test]
    fn pr_state_non_draft() {
        assert_eq!(glyph_for_pr_state(false, PrState::Open, true), " ");
        assert_eq!(glyph_for_pr_state(false, PrState::Merged, true), "✓");
        assert_eq!(glyph_for_pr_state(false, PrState::Closed, true), "✗");
        assert_eq!(glyph_for_pr_state(false, PrState::Merged, false), "M");
        assert_eq!(glyph_for_pr_state(false, PrState::Closed, false), "C");
    }

    // glyph_for_check_run key cases

    #[test]
    fn check_run_completed_success() {
        let (g, _) =
            glyph_for_check_run(CheckStatus::Completed, Some(CheckConclusion::Success), true);
        assert_eq!(g, "✓");
    }

    #[test]
    fn check_run_completed_failure() {
        let (g, _) =
            glyph_for_check_run(CheckStatus::Completed, Some(CheckConclusion::Failure), true);
        assert_eq!(g, "✗");
    }

    #[test]
    fn check_run_in_progress() {
        let (g, _) = glyph_for_check_run(CheckStatus::InProgress, None, true);
        assert_eq!(g, "◐");
    }

    #[test]
    fn check_run_honors_ascii_fallback() {
        // unicode=false (e.g. --no-color) must yield ASCII, not Unicode.
        let (g, _) = glyph_for_check_run(
            CheckStatus::Completed,
            Some(CheckConclusion::Success),
            false,
        );
        assert_eq!(g, "+");
        let (g, _) = glyph_for_check_run(
            CheckStatus::Completed,
            Some(CheckConclusion::Failure),
            false,
        );
        assert_eq!(g, "x");
        let (g, _) = glyph_for_check_run(CheckStatus::InProgress, None, false);
        assert_eq!(g, "*");
    }

    #[test]
    fn status_state_honors_ascii_fallback() {
        assert_eq!(glyph_for_status_state(StatusState::Success, false).0, "+");
        assert_eq!(glyph_for_status_state(StatusState::Failure, false).0, "x");
        assert_eq!(glyph_for_status_state(StatusState::Success, true).0, "✓");
    }

    // Timeline event glyphs

    #[test]
    fn timeline_event_glyphs_unicode() {
        use chrono::Utc;
        assert_eq!(
            glyph_for_timeline_event(
                &TimelineEvent::Commit {
                    sha: String::new(),
                    message: String::new(),
                    author: String::new(),
                    pushed_at: Utc::now()
                },
                true
            ),
            "●"
        );
        assert_eq!(
            glyph_for_timeline_event(
                &TimelineEvent::IssueComment {
                    id: String::new(),
                    author: String::new(),
                    body: String::new(),
                    created_at: Utc::now()
                },
                true
            ),
            "✎"
        );
        assert_eq!(
            glyph_for_timeline_event(
                &TimelineEvent::Review {
                    id: String::new(),
                    author: String::new(),
                    state: crate::data::models::ReviewState::Approved,
                    body: String::new(),
                    submitted_at: Utc::now()
                },
                true
            ),
            "★"
        );
        assert_eq!(
            glyph_for_timeline_event(
                &TimelineEvent::Merged {
                    sha: String::new(),
                    by: String::new(),
                    at: Utc::now()
                },
                true
            ),
            "✓"
        );
        assert_eq!(
            glyph_for_timeline_event(
                &TimelineEvent::Closed {
                    by: String::new(),
                    at: Utc::now()
                },
                true
            ),
            "✗"
        );
        assert_eq!(
            glyph_for_timeline_event(
                &TimelineEvent::Reopened {
                    by: String::new(),
                    at: Utc::now()
                },
                true
            ),
            "↻"
        );
        assert_eq!(
            glyph_for_timeline_event(
                &TimelineEvent::HeadRefForcePushed {
                    before: String::new(),
                    after: String::new(),
                    at: Utc::now()
                },
                true
            ),
            "→"
        );
    }

    #[test]
    fn timeline_event_glyphs_ascii() {
        use chrono::Utc;
        assert_eq!(
            glyph_for_timeline_event(
                &TimelineEvent::Commit {
                    sha: String::new(),
                    message: String::new(),
                    author: String::new(),
                    pushed_at: Utc::now()
                },
                false
            ),
            "*"
        );
        assert_eq!(
            glyph_for_timeline_event(
                &TimelineEvent::IssueComment {
                    id: String::new(),
                    author: String::new(),
                    body: String::new(),
                    created_at: Utc::now()
                },
                false
            ),
            "~"
        );
        assert_eq!(
            glyph_for_timeline_event(
                &TimelineEvent::Merged {
                    sha: String::new(),
                    by: String::new(),
                    at: Utc::now()
                },
                false
            ),
            "+"
        );
        assert_eq!(
            glyph_for_timeline_event(
                &TimelineEvent::Closed {
                    by: String::new(),
                    at: Utc::now()
                },
                false
            ),
            "x"
        );
        assert_eq!(
            glyph_for_timeline_event(
                &TimelineEvent::Reopened {
                    by: String::new(),
                    at: Utc::now()
                },
                false
            ),
            "o"
        );
        assert_eq!(
            glyph_for_timeline_event(
                &TimelineEvent::HeadRefForcePushed {
                    before: String::new(),
                    after: String::new(),
                    at: Utc::now()
                },
                false
            ),
            "->"
        );
        assert_eq!(
            glyph_for_timeline_event(
                &TimelineEvent::Review {
                    id: String::new(),
                    author: String::new(),
                    state: crate::data::models::ReviewState::Approved,
                    body: String::new(),
                    submitted_at: Utc::now()
                },
                false
            ),
            "*"
        );
    }

    // Color helpers — representative cases

    #[test]
    fn color_for_pr_state_draft() {
        let theme = Theme::dark();
        assert_eq!(color_for_pr_state(true, PrState::Open, &theme), theme.dim);
        assert_eq!(
            color_for_pr_state(false, PrState::Open, &theme),
            theme.success
        );
        assert_eq!(
            color_for_pr_state(false, PrState::Merged, &theme),
            theme.primary
        );
        assert_eq!(
            color_for_pr_state(false, PrState::Closed, &theme),
            theme.error
        );
    }

    #[test]
    fn color_for_checks_rollup_cases() {
        let theme = Theme::dark();
        assert_eq!(
            color_for_checks_rollup(ChecksRollup::Success, &theme),
            theme.success
        );
        assert_eq!(
            color_for_checks_rollup(ChecksRollup::Failure, &theme),
            theme.error
        );
        assert_eq!(
            color_for_checks_rollup(ChecksRollup::Pending, &theme),
            theme.warning
        );
        assert_eq!(
            color_for_checks_rollup(ChecksRollup::Mixed, &theme),
            theme.warning
        );
        assert_eq!(
            color_for_checks_rollup(ChecksRollup::Neutral, &theme),
            theme.dim
        );
        assert_eq!(
            color_for_checks_rollup(ChecksRollup::None, &theme),
            theme.dim
        );
    }

    #[test]
    fn color_for_review_decision_cases() {
        let theme = Theme::dark();
        assert_eq!(
            color_for_review_decision(Some(ReviewDecision::Approved), &theme),
            theme.success
        );
        assert_eq!(
            color_for_review_decision(Some(ReviewDecision::ChangesRequested), &theme),
            theme.error
        );
        assert_eq!(
            color_for_review_decision(Some(ReviewDecision::ReviewRequired), &theme),
            theme.dim
        );
        assert_eq!(color_for_review_decision(None, &theme), theme.dim);
    }
}
