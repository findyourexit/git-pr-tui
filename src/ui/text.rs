//! Small shared text helpers for the UI layer.

use chrono::{DateTime, Utc};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

const ELLIPSIS: &str = "…";

/// Compact human relative time, e.g. `just now`, `5m ago`, `3h ago`, `2d ago`,
/// `3w ago`, `4mo ago`, `2y ago`. A `from` in the future clamps to `just now`.
///
/// `now` is passed in (never read from the clock) so callers stay deterministic
/// and snapshot-testable.
#[must_use]
pub fn relative_time(from: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let secs = now.signed_duration_since(from).num_seconds();
    if secs < 60 {
        return "just now".to_string();
    }
    let mins = secs / 60;
    if mins < 60 {
        return format!("{mins}m ago");
    }
    let hours = mins / 60;
    if hours < 24 {
        return format!("{hours}h ago");
    }
    let days = hours / 24;
    if days < 7 {
        return format!("{days}d ago");
    }
    let weeks = days / 7;
    if days < 30 {
        return format!("{weeks}w ago");
    }
    let months = days / 30;
    if months < 12 {
        return format!("{months}mo ago");
    }
    let years = days / 365;
    format!("{years}y ago")
}

/// Truncate `s` to at most `width` display columns (Unicode-width aware),
/// appending `…` when truncation occurs.
///
/// * `width == 0` → empty string.
/// * If `s` already fits, it is returned unchanged.
/// * The ellipsis is included within the `width` budget, so the result is
///   never wider than `width` columns.
#[must_use]
pub fn truncate_to_width(s: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    if s.width() <= width {
        return s.to_string();
    }
    let ellipsis_w = ELLIPSIS.width(); // 1
    if width <= ellipsis_w {
        return ELLIPSIS.to_string();
    }
    let budget = width - ellipsis_w;
    let mut out = String::new();
    let mut w = 0usize;
    for ch in s.chars() {
        let cw = UnicodeWidthChar::width(ch).unwrap_or(0);
        if w + cw > budget {
            break;
        }
        out.push(ch);
        w += cw;
    }
    out.push_str(ELLIPSIS);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use unicode_width::UnicodeWidthStr;

    #[test]
    fn relative_time_buckets() {
        use chrono::Duration;
        let now = Utc::now();
        assert_eq!(relative_time(now, now), "just now");
        assert_eq!(relative_time(now - Duration::seconds(30), now), "just now");
        assert_eq!(relative_time(now - Duration::minutes(5), now), "5m ago");
        assert_eq!(relative_time(now - Duration::hours(3), now), "3h ago");
        assert_eq!(relative_time(now - Duration::days(2), now), "2d ago");
        assert_eq!(relative_time(now - Duration::weeks(3), now), "3w ago");
        assert_eq!(relative_time(now - Duration::days(60), now), "2mo ago");
        assert_eq!(relative_time(now - Duration::days(800), now), "2y ago");
        // future clamps to "just now"
        assert_eq!(relative_time(now + Duration::hours(1), now), "just now");
    }

    #[test]
    fn empty_when_width_zero() {
        assert_eq!(truncate_to_width("hello", 0), "");
    }

    #[test]
    fn unchanged_when_it_fits() {
        assert_eq!(truncate_to_width("hello", 5), "hello");
        assert_eq!(truncate_to_width("hello", 10), "hello");
    }

    #[test]
    fn appends_ellipsis_when_clipped() {
        // "hello" → width 4 → "hel…"
        let out = truncate_to_width("hello", 4);
        assert_eq!(out, "hel…");
        assert!(out.width() <= 4);
    }

    #[test]
    fn width_one_yields_just_ellipsis() {
        assert_eq!(truncate_to_width("hello", 1), "…");
    }

    #[test]
    fn handles_wide_chars_without_overflowing() {
        // CJK chars are width 2 each.
        let s = "日本語テスト";
        let out = truncate_to_width(s, 5);
        assert!(out.width() <= 5, "got width {} for {out:?}", out.width());
        assert!(out.ends_with('…'));
    }
}
