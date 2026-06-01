//! Workspace tab strip: renders the tab bar with optional right-aligned status
//! cluster (rate-limit, refresh time, pending/checkout indicators).

use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Tabs},
};
use unicode_width::UnicodeWidthStr;

use crate::data::github::RateLimitSnapshot;
use crate::ui::theme::Theme;

const MAX_LABEL_COLS: usize = 18;

/// Right-aligned global status cluster shown on the tab strip (relocated from
/// the bottom row, which is now the contextual footer).
#[derive(Debug, Default)]
pub struct StatusCluster<'a> {
    pub rate_limit: Option<&'a RateLimitSnapshot>,
    pub last_refresh_secs: Option<u64>,
    pub pending_write: bool,
    pub checkout_inflight: bool,
}

/// Truncate `label` to at most `MAX_LABEL_COLS` display columns.
fn truncate_label(label: &str) -> String {
    crate::ui::text::truncate_to_width(label, MAX_LABEL_COLS)
}

/// Build the right-side status string from `cluster`.
fn build_status_text(cluster: &StatusCluster) -> String {
    let mut parts: Vec<String> = Vec::new();

    if let Some(rl) = cluster.rate_limit {
        let now = chrono::Utc::now();
        let secs = rl.resets_at.signed_duration_since(now).num_seconds().max(0);
        let reset_str = if secs >= 60 {
            format!("{}m{:02}s", secs / 60, secs % 60)
        } else {
            format!("{secs}s")
        };
        parts.push(format!(
            "rate: {}/{} reset {}",
            rl.remaining, rl.limit, reset_str
        ));
    }
    if let Some(age) = cluster.last_refresh_secs {
        parts.push(format!("refreshed {age}s ago"));
    }
    if cluster.checkout_inflight {
        parts.push("[Fetching…]".to_string());
    } else if cluster.pending_write {
        parts.push("[pending]".to_string());
    }

    if parts.is_empty() {
        String::new()
    } else {
        format!(" {}  ", parts.join("  ·  "))
    }
}

/// Render the workspace tab strip.
///
/// - `labels`: display label for each tab (caller computes them; index 0 should
///   be `"⌂"` for the pinned Dashboard).
/// - `active`: which tab is currently selected (0-based).
/// - `status`: optional right-aligned status cluster.
pub fn render_tab_strip(
    f: &mut Frame,
    area: Rect,
    labels: &[String],
    active: usize,
    status: Option<&StatusCluster>,
    theme: &Theme,
) {
    if area.height == 0 || area.width == 0 || labels.is_empty() {
        return;
    }

    let base_style = Style::default().bg(theme.bg).fg(theme.fg);
    let highlight = Style::default()
        .fg(theme.primary)
        .add_modifier(Modifier::BOLD);

    // Build status text and reserve its width.
    let status_text = status.map(build_status_text).unwrap_or_default();
    let status_width = u16::try_from(status_text.width()).unwrap_or(u16::MAX);

    // Determine the area available for the Tabs widget.
    let (tabs_area, status_area) = if status_width > 0 && area.width > status_width {
        let [ta, sa] =
            Layout::horizontal([Constraint::Min(0), Constraint::Length(status_width)]).areas(area);
        (ta, Some(sa))
    } else {
        (area, None)
    };

    // Truncate labels to ≤18 cols.
    let truncated: Vec<String> = labels.iter().map(|l| truncate_label(l)).collect();

    // Compute divider width (ratatui counts chars, not display cols; our divider is ASCII).
    let divider = " │ ";
    let divider_w = divider.width();

    // Compute total width of all tabs to decide if we need to scroll.
    let tab_widths: Vec<usize> = truncated.iter().map(|l| l.width()).collect();
    let total_w: usize =
        tab_widths.iter().sum::<usize>() + divider_w * tab_widths.len().saturating_sub(1);
    let avail = usize::from(tabs_area.width);

    // Compute a visible window [start, end) that keeps `active` in view.
    // We also leave room for overflow markers (1 col each side) when scrolling.
    let overflow_marker_w = 1usize; // '‹' or '›'
    let (visible_start, show_left_overflow, show_right_overflow) = if total_w <= avail {
        (0, false, false)
    } else {
        // Scroll to make `active` visible within the available width,
        // accounting for potential overflow markers.
        let inner_avail = avail.saturating_sub(overflow_marker_w * 2);
        let mut start = 0usize;
        // Advance start until active is within the window.
        loop {
            // Width of tabs from start onward.
            let w: usize = tab_widths[start..]
                .iter()
                .enumerate()
                .map(|(i, &tw)| if i == 0 { tw } else { tw + divider_w })
                .sum();
            if w <= inner_avail || start >= active {
                break;
            }
            start += 1;
        }
        let rendered_w: usize = tab_widths[start..]
            .iter()
            .enumerate()
            .map(|(i, &tw)| if i == 0 { tw } else { tw + divider_w })
            .sum();
        (start, start > 0, rendered_w + overflow_marker_w > avail)
    };

    // Build the visible slice.
    let visible_labels: Vec<String> = truncated[visible_start..].to_vec();
    let adjusted_active = active.saturating_sub(visible_start);

    // Adjust the area to leave space for overflow markers.
    let left_reserve = u16::from(show_left_overflow);
    let right_reserve = u16::from(show_right_overflow);

    let inner_tabs_area = Rect {
        x: tabs_area.x + left_reserve,
        y: tabs_area.y,
        width: tabs_area.width.saturating_sub(left_reserve + right_reserve),
        height: tabs_area.height,
    };

    let tabs_widget = Tabs::new(visible_labels)
        .select(adjusted_active)
        .style(base_style)
        .highlight_style(highlight)
        .divider(" │ ");

    f.render_widget(tabs_widget, inner_tabs_area);

    // Draw overflow markers.
    if show_left_overflow {
        let marker_area = Rect {
            x: tabs_area.x,
            y: tabs_area.y,
            width: 1,
            height: 1,
        };
        f.render_widget(Paragraph::new("‹").style(base_style), marker_area);
    }
    if show_right_overflow {
        let marker_area = Rect {
            x: tabs_area.x + tabs_area.width - 1,
            y: tabs_area.y,
            width: 1,
            height: 1,
        };
        f.render_widget(Paragraph::new("›").style(base_style), marker_area);
    }

    // Draw the status cluster.
    if let (Some(sa), false) = (status_area, status_text.is_empty()) {
        f.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(status_text, base_style)])),
            sa,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};

    #[test]
    fn renders_pinned_dashboard_and_active_repo() {
        let backend = TestBackend::new(80, 1);
        let mut t = Terminal::new(backend).unwrap();
        let labels = [
            "⌂".to_string(),
            "acme/widgets".to_string(),
            "#421 fix: flaky…".to_string(),
        ];
        t.draw(|f| {
            render_tab_strip(
                f,
                f.area(),
                &labels,
                1,
                None,
                &crate::ui::theme::Theme::dark(),
            );
        })
        .unwrap();
        insta::assert_snapshot!(t.backend());
    }

    #[test]
    fn renders_with_populated_status_cluster() {
        let backend = TestBackend::new(80, 1);
        let mut t = Terminal::new(backend).unwrap();
        let labels = ["⌂".to_string(), "acme/widgets".to_string()];
        let status = StatusCluster {
            last_refresh_secs: Some(12),
            pending_write: true,
            ..Default::default()
        };
        t.draw(|f| {
            render_tab_strip(
                f,
                f.area(),
                &labels,
                0,
                Some(&status),
                &crate::ui::theme::Theme::dark(),
            );
        })
        .unwrap();
        insta::assert_snapshot!(t.backend());
    }

    #[test]
    fn checkout_inflight_overrides_pending_in_cluster() {
        let backend = TestBackend::new(80, 1);
        let mut t = Terminal::new(backend).unwrap();
        let labels = ["\u{2302}".to_string()];
        let status = StatusCluster {
            pending_write: true,
            checkout_inflight: true,
            ..Default::default()
        };
        t.draw(|f| {
            render_tab_strip(
                f,
                f.area(),
                &labels,
                0,
                Some(&status),
                &crate::ui::theme::Theme::dark(),
            );
        })
        .unwrap();
        let buf = t.backend().buffer().clone();
        let row: String = (0..buf.area().width)
            .map(|x| buf[(x, 0)].symbol().to_string())
            .collect();
        assert!(
            row.contains("[Fetching"),
            "checkout-in-flight must show the fetching indicator, got {row:?}"
        );
        assert!(
            !row.contains("[pending]"),
            "checkout indicator must override the pending indicator, got {row:?}"
        );
    }
}
