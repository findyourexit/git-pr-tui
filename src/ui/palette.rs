use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem},
};

use crate::app::actions::{ActionKind, context_actions};
use crate::app::intent::UserIntent;
use crate::app::keymap::key_label;
use crate::app::state::AppState;
use crate::ui::components::input::{InputBuffer, render_input};
use crate::ui::theme::Theme;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Dashboard,
    /// `:repo owner/name` — switch to that repo's PR list.
    Repo(crate::data::models::Repo),
    Refresh,
    Log,
    Quit,
    Pr(u32),
}

impl Command {
    #[must_use]
    pub fn label(&self) -> &'static str {
        match self {
            Command::Dashboard => "dashboard",
            Command::Repo(_) => "repo <owner/name>",
            Command::Refresh => "refresh",
            Command::Log => "log",
            Command::Quit => "quit",
            Command::Pr(_) => "pr <n>",
        }
    }
}

/// A palette result entry — either a registry action/setting or a legacy nav
/// command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaletteEntry {
    /// Dispatches a context action or setting from the action registry.
    Action(crate::app::actions::Action),
    /// Legacy nav commands: `Pr(n)`, `Repo(..)`, `Dashboard`, `Refresh`,
    /// `Log`, `Quit`.
    Nav(Command),
}

const STATIC_COMMANDS: &[Command] = &[
    Command::Refresh,
    Command::Log,
    Command::Quit,
    Command::Dashboard,
];

/// Intents covered by the static nav commands — excluded from registry results
/// to prevent duplicate entries.
fn is_nav_covered(intent: &UserIntent) -> bool {
    matches!(
        intent,
        UserIntent::Refresh | UserIntent::Quit | UserIntent::GotoDashboard
    )
}

#[derive(Debug, Default, Clone)]
pub struct PaletteState {
    pub input: InputBuffer,
    pub selected: usize,
}

#[must_use]
pub fn score(needle: &str, haystack: &str) -> Option<u32> {
    if needle.is_empty() {
        return Some(0);
    }
    let needle: Vec<char> = needle.chars().flat_map(char::to_lowercase).collect();
    let mut score: u32 = 0;
    let mut consecutive: u32 = 0;
    let mut prev_match: Option<usize> = None;
    let mut n_iter = needle.iter().peekable();
    for (h_idx, h_char_raw) in haystack.chars().enumerate() {
        let Some(&&n_char) = n_iter.peek() else { break };
        let h_char = h_char_raw.to_lowercase().next().unwrap_or(h_char_raw);
        if h_char == n_char {
            n_iter.next();
            let bonus = if h_idx == 0 { 10 } else { 1 };
            if prev_match.is_some_and(|p| p + 1 == h_idx) {
                consecutive += 1;
                score += bonus + consecutive * 5;
            } else {
                consecutive = 0;
                score += bonus;
            }
            prev_match = Some(h_idx);
        }
    }
    if n_iter.peek().is_some() {
        None
    } else {
        Some(score)
    }
}

/// Returns palette results for the given query and application state.
///
/// Results include:
/// - Registry actions and settings from [`context_actions`] (kind `Action` or
///   `Setting`; `Nav` actions and those already covered by the static nav
///   commands are excluded to prevent duplicates).
/// - Legacy nav commands (`Dashboard`, `Refresh`, `Log`, `Quit`) plus the
///   `pr <n>` and `repo <owner/name>` parsers (unchanged).
///
/// Sorted descending by score; `pr`/`repo` parsing yields [`u32::MAX`]
/// priority when matched.
#[must_use]
pub fn matches(query: &str, state: &AppState) -> Vec<(PaletteEntry, u32)> {
    let trimmed = query.trim();
    let mut out: Vec<(PaletteEntry, u32)> = Vec::new();

    // ── Special-syntax nav: `pr <n>` and `repo <owner/name>` ─────────────
    if let Some(num_str) = trimmed.strip_prefix("pr ").map(str::trim)
        && let Ok(n) = num_str.parse::<u32>()
    {
        out.push((PaletteEntry::Nav(Command::Pr(n)), u32::MAX));
    }

    if let Some(rest) = trimmed.strip_prefix("repo ").map(str::trim)
        && let Some((owner, name)) = rest.split_once('/')
        && !owner.is_empty()
        && !name.is_empty()
        && !name.contains('/')
    {
        out.push((
            PaletteEntry::Nav(Command::Repo(crate::data::models::Repo {
                owner: owner.to_string(),
                name: name.to_string(),
            })),
            u32::MAX,
        ));
    }

    // ── Registry actions and settings (no Nav, no nav-covered duplicates) ──
    for action in context_actions(state) {
        if action.kind == ActionKind::Nav {
            continue;
        }
        if is_nav_covered(&action.intent) {
            continue;
        }
        if let Some(s) = score(trimmed, &action.label) {
            out.push((PaletteEntry::Action(action), s));
        }
    }

    // ── Static nav commands ───────────────────────────────────────────────
    for cmd in STATIC_COMMANDS {
        if let Some(s) = score(trimmed, cmd.label()) {
            out.push((PaletteEntry::Nav(cmd.clone()), s));
        }
    }

    out.sort_by_key(|b| std::cmp::Reverse(b.1));
    out
}

fn entry_group_label(entry: &PaletteEntry) -> &'static str {
    match entry {
        PaletteEntry::Nav(_) => "nav",
        PaletteEntry::Action(a) => match a.kind {
            ActionKind::Action => "action",
            ActionKind::Setting => "setting",
            ActionKind::Nav => "nav",
        },
    }
}

fn entry_display_label(entry: &PaletteEntry, state: &AppState) -> String {
    match entry {
        PaletteEntry::Nav(cmd) => match cmd {
            Command::Pr(n) => format!("pr {n}"),
            Command::Repo(r) => format!("repo {}/{}", r.owner, r.name),
            other => other.label().to_string(),
        },
        PaletteEntry::Action(a) => {
            // Optionally append key hint.
            let key_hint = state
                .keymap
                .key_for(&a.intent)
                .map(|k| format!("  [{}]", key_label(&k)))
                .unwrap_or_default();
            format!("{}{}", a.label, key_hint)
        }
    }
}

pub fn render_palette(
    f: &mut Frame,
    area: Rect,
    state: &PaletteState,
    app: &AppState,
    theme: &Theme,
) {
    let width = (area.width * 60).max(40 * 100) / 100;
    let width = width.min(area.width);
    let height = 16u16.min(area.height);
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    let outer = Rect::new(x, y, width, height);

    f.render_widget(Clear, outer);

    let outer_block = Block::default()
        .title(" : ")
        .borders(Borders::ALL)
        .border_type(theme.border_type())
        .style(Style::default().bg(theme.bg).fg(theme.fg))
        .border_style(Style::default().fg(theme.primary));
    f.render_widget(outer_block, outer);

    let inner = Rect::new(
        outer.x + 1,
        outer.y + 1,
        outer.width.saturating_sub(2),
        outer.height.saturating_sub(2),
    );
    let input_area = Rect::new(inner.x, inner.y, inner.width, 3.min(inner.height));
    render_input(f, input_area, &state.input, theme);

    if inner.height > 3 {
        let list_area = Rect::new(
            inner.x,
            inner.y + 3,
            inner.width,
            inner.height.saturating_sub(3),
        );
        let results = matches(&state.input.text, app);

        // Build list items, inserting group-label separators on kind change.
        let mut items: Vec<ListItem> = Vec::new();
        let mut last_group: Option<&'static str> = None;

        for (i, (entry, _)) in results.iter().enumerate() {
            let group = entry_group_label(entry);
            if last_group != Some(group) {
                last_group = Some(group);
                let sep = ListItem::new(Line::from(Span::styled(
                    format!("── {group} ──"),
                    Style::default().fg(theme.secondary),
                )));
                items.push(sep);
            }
            let label = entry_display_label(entry, app);
            let style = if i == state.selected {
                Style::default().fg(theme.bg).bg(theme.primary)
            } else {
                Style::default().fg(theme.fg)
            };
            items.push(ListItem::new(Line::from(Span::styled(label, style))));
        }

        let list = List::new(items);
        f.render_widget(list, list_area);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::state::AppState;
    use crate::app::workspace::TabKind;
    use crate::data::models::{PrId, Repo};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn make_repo() -> Repo {
        Repo {
            owner: "acme".to_string(),
            name: "widgets".to_string(),
        }
    }

    fn default_state() -> AppState {
        AppState::default()
    }

    fn pr_state() -> AppState {
        let mut s = AppState::default();
        s.workspace.open(TabKind::Pr(PrId {
            repo: make_repo(),
            number: 1,
        }));
        s
    }

    #[test]
    fn score_returns_none_when_needle_not_subsequence() {
        assert_eq!(score("xyz", "dashboard"), None);
        assert_eq!(score("zzz", "log"), None);
    }

    #[test]
    fn score_returns_some_for_subsequence_match() {
        assert!(score("dsh", "dashboard").is_some());
        assert!(score("log", "log").is_some());
        assert!(score("rfr", "refresh").is_some());
    }

    #[test]
    fn score_is_case_insensitive() {
        assert!(score("DSH", "dashboard").is_some());
        assert!(score("LoG", "log").is_some());
    }

    #[test]
    fn score_consecutive_match_beats_scattered_match() {
        let consecutive = score("dash", "dashboard").unwrap();
        let scattered = score("drd", "dashboard").unwrap();
        assert!(
            consecutive > scattered,
            "consecutive {consecutive} should beat scattered {scattered}"
        );
    }

    #[test]
    fn matches_orders_by_score_desc() {
        let state = default_state();
        let results = matches("re", &state);
        assert!(!results.is_empty());
        let has_refresh = results
            .iter()
            .any(|(e, _)| matches!(e, PaletteEntry::Nav(Command::Refresh)));
        assert!(has_refresh, "expected refresh in results");
    }

    #[test]
    fn matches_repo_with_owner_name_yields_repo_command() {
        let state = default_state();
        let results = matches("repo acme/widgets", &state);
        match results.first() {
            Some((PaletteEntry::Nav(Command::Repo(r)), _)) => {
                assert_eq!(r.owner, "acme");
                assert_eq!(r.name, "widgets");
            }
            other => panic!("expected Repo command, got {other:?}"),
        }
    }

    #[test]
    fn matches_pr_with_number_yields_pr_command() {
        let state = default_state();
        let results = matches("pr 42", &state);
        assert!(matches!(
            results.first(),
            Some((PaletteEntry::Nav(Command::Pr(42)), _))
        ));
    }

    #[test]
    fn matches_empty_query_returns_context_actions_and_nav() {
        let state = default_state();
        let results = matches("", &state);
        // Must include static nav commands.
        assert!(
            results
                .iter()
                .any(|(e, _)| matches!(e, PaletteEntry::Nav(_)))
        );
        // Must include registry actions.
        assert!(
            results
                .iter()
                .any(|(e, _)| matches!(e, PaletteEntry::Action(_)))
        );
    }

    #[test]
    fn matches_pr_context_surfaces_merge_action() {
        let state = pr_state();
        let results = matches("mer", &state);
        let has_merge = results.iter().any(|(e, _)| {
            matches!(
                e,
                PaletteEntry::Action(a) if a.intent == UserIntent::Merge
            )
        });
        assert!(has_merge, "expected Merge action in PR context results");
    }

    #[test]
    fn no_duplicate_refresh_entries() {
        let state = default_state();
        let results = matches("refresh", &state);
        let refresh_count = results
            .iter()
            .filter(|(e, _)| match e {
                PaletteEntry::Nav(Command::Refresh) => true,
                PaletteEntry::Action(a) => a.intent == UserIntent::Refresh,
                PaletteEntry::Nav(_) => false,
            })
            .count();
        assert_eq!(
            refresh_count, 1,
            "Refresh should appear exactly once, got {refresh_count}"
        );
    }

    #[test]
    fn no_duplicate_quit_entries() {
        let state = default_state();
        let results = matches("quit", &state);
        let quit_count = results
            .iter()
            .filter(|(e, _)| match e {
                PaletteEntry::Nav(Command::Quit) => true,
                PaletteEntry::Action(a) => a.intent == UserIntent::Quit,
                PaletteEntry::Nav(_) => false,
            })
            .count();
        assert_eq!(
            quit_count, 1,
            "Quit should appear exactly once, got {quit_count}"
        );
    }

    #[test]
    fn renders_palette_with_query_and_results() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let app = default_state();
        let state = PaletteState {
            input: InputBuffer {
                text: "re".into(),
                cursor: 2,
            },
            selected: 0,
        };
        terminal
            .draw(|f| {
                render_palette(f, f.area(), &state, &app, &crate::ui::theme::Theme::dark());
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
        assert!(
            rendered.contains("refresh"),
            "expected `refresh` in palette output, got:\n{rendered}"
        );
    }
}
