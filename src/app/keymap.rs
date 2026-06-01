//! Global keymap: `KeyEvent → UserIntent`.
//!
//! Built from vim-style defaults; overridable from `[keys]` in
//! `config.toml`. Each `intent_name = "key-spec"` entry strips the
//! intent's default binding (if any) and installs the override.
//!
//! Per-mode handlers still own mode-conditional keys (e.g. `j` scrolls
//! a thread in PR Detail but the diff hunk in Diff Viewer). The keymap
//! is the *fallback* consulted after per-mode handlers decline a key.

use std::collections::HashMap;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::intent::{TabDirection, UserIntent};

/// `KeyEvent → UserIntent` table. Construct via [`Keymap::vim_defaults`] and
/// optionally [`Keymap::apply_overrides`] to merge user config.
#[derive(Debug, Clone, Default)]
pub struct Keymap {
    bindings: HashMap<KeyEvent, UserIntent>,
}

impl Keymap {
    /// Build the vim-default keymap. Modifier-bearing keys (`Ctrl-S`,
    /// shifted letters) are registered explicitly.
    #[must_use]
    pub fn vim_defaults() -> Self {
        let mut km = Self::default();
        // Plain letters / symbols.
        km.bind(key('q'), UserIntent::Quit);
        km.bind(key('j'), UserIntent::Down);
        km.bind(key('k'), UserIntent::Up);
        km.bind(key('h'), UserIntent::FocusLeft);
        km.bind(key('l'), UserIntent::FocusRight);
        km.bind(key('g'), UserIntent::Top);
        km.bind(shift('G'), UserIntent::Bottom);
        km.bind(key('c'), UserIntent::StartComment);
        km.bind(key('r'), UserIntent::StartReplyToThread);
        km.bind(key('R'), UserIntent::Refresh);
        km.bind(key('v'), UserIntent::StartReview);
        km.bind(key('m'), UserIntent::Merge);
        km.bind(key('x'), UserIntent::ToggleClose);
        km.bind(shift('C'), UserIntent::CheckoutBranch);
        km.bind(key('?'), UserIntent::EnterHelp);
        km.bind(key('o'), UserIntent::OpenInBrowser);
        km.bind(key('/'), UserIntent::OpenSearch);
        km.bind(key('f'), UserIntent::OpenFilterModal);
        km.bind(key('s'), UserIntent::OpenSortModal);
        km.bind(shift('M'), UserIntent::LoadMore);
        km.bind(key(':'), UserIntent::EnterCommandPalette);
        // Half-page scroll.
        km.bind(ctrl('d'), UserIntent::PageDown);
        km.bind(ctrl('u'), UserIntent::PageUp);
        // Submit / cancel.
        km.bind(ctrl('s'), UserIntent::Submit);
        km.bind(
            KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
            UserIntent::Cancel,
        );
        km.bind(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            UserIntent::OpenSelected,
        );
        // Arrows mirror hjkl for non-vim users.
        km.bind(
            KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
            UserIntent::Down,
        );
        km.bind(
            KeyEvent::new(KeyCode::Up, KeyModifiers::NONE),
            UserIntent::Up,
        );
        km.bind(
            KeyEvent::new(KeyCode::Left, KeyModifiers::NONE),
            UserIntent::FocusLeft,
        );
        km.bind(
            KeyEvent::new(KeyCode::Right, KeyModifiers::NONE),
            UserIntent::FocusRight,
        );
        // Tab navigation (browser-style).
        km.bind(
            KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE),
            UserIntent::NextTab,
        );
        km.bind(
            KeyEvent::new(KeyCode::Tab, KeyModifiers::SHIFT),
            UserIntent::PrevTab,
        );
        km.bind(key('1'), UserIntent::GotoTab(1));
        km.bind(key('2'), UserIntent::GotoTab(2));
        km.bind(key('3'), UserIntent::GotoTab(3));
        km.bind(key('4'), UserIntent::GotoTab(4));
        km.bind(key('5'), UserIntent::GotoTab(5));
        km.bind(key('6'), UserIntent::GotoTab(6));
        km.bind(key('7'), UserIntent::GotoTab(7));
        km.bind(key('8'), UserIntent::GotoTab(8));
        km.bind(key('9'), UserIntent::GotoTab(9));
        km.bind(key('w'), UserIntent::CloseTab);
        km.bind(key('['), UserIntent::SwitchTab(TabDirection::Prev));
        km.bind(key(']'), UserIntent::SwitchTab(TabDirection::Next));
        km.bind(shift('W'), UserIntent::ToggleWhitespace);
        km.bind(
            KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE),
            UserIntent::OpenActionMenu,
        );
        km
    }

    /// The key currently bound to `intent`, if any.
    ///
    /// **Deterministic:** when several keys map to the same intent, prefer a
    /// plain letter/symbol over arrow keys so hints read naturally (e.g. `j`
    /// over `Down`). The `vim_defaults` map has at most one non-arrow binding
    /// per intent, so this rule always yields a stable result regardless of
    /// `HashMap` iteration order.
    #[must_use]
    pub fn key_for(&self, intent: &UserIntent) -> Option<KeyEvent> {
        let mut best: Option<KeyEvent> = None;
        for (k, v) in &self.bindings {
            if v == intent {
                let prefer = !matches!(
                    k.code,
                    KeyCode::Up | KeyCode::Down | KeyCode::Left | KeyCode::Right
                );
                if best.is_none() || prefer {
                    best = Some(*k);
                    if prefer {
                        break;
                    }
                }
            }
        }
        best
    }

    /// Look up an intent for `k`. Returns `None` for unbound keys.
    #[must_use]
    pub fn resolve(&self, k: &KeyEvent) -> Option<UserIntent> {
        self.bindings.get(k).cloned()
    }

    fn bind(&mut self, k: KeyEvent, intent: UserIntent) {
        self.bindings.insert(k, intent);
    }

    /// Apply user overrides from `[keys]` in `config.toml`. Each entry
    /// `intent_name = "key-spec"` strips the intent's prior bindings and
    /// installs the override. Unknown intent names and unparseable key
    /// specs are silently ignored (config never panics startup).
    pub fn apply_overrides(&mut self, overrides: &HashMap<String, String>) {
        for (intent_name, key_spec) in overrides {
            let Some(intent) = parse_intent_name(intent_name) else {
                continue;
            };
            let Some(new_key) = parse_key_spec(key_spec) else {
                continue;
            };
            self.bindings.retain(|_, v| v != &intent);
            self.bindings.insert(new_key, intent);
        }
    }
}

/// Render a [`KeyEvent`] as a short human-readable label.
///
/// Examples: plain char → `"c"`, shifted letter → `"W"`, ctrl → `"Ctrl-s"`,
/// `Enter` → `"↵"`, `Esc` → `"esc"`, `Tab` → `"↹"`, `Char(' ')` → `"␣"`,
/// arrows → `"↑"`/`"↓"`/`"←"`/`"→"`, `F(n)` → `"F1"` etc.
#[must_use]
pub fn key_label(k: &KeyEvent) -> String {
    let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
    let shift = k.modifiers.contains(KeyModifiers::SHIFT);
    // `Char` keys already encode Shift in the character itself (e.g. `W`), so
    // they return directly. Non-`Char` keys surface Shift with a `⇧` prefix
    // (e.g. Shift-Tab → `⇧↹`, the binding for `PrevTab`).
    match k.code {
        KeyCode::Char(' ') if ctrl => return "Ctrl-Space".to_string(),
        KeyCode::Char(' ') => return "␣".to_string(),
        KeyCode::Char(c) if ctrl => return format!("Ctrl-{}", c.to_ascii_lowercase()),
        KeyCode::Char(c) => return c.to_string(),
        _ => {}
    }
    let base = match k.code {
        KeyCode::Enter => "↵",
        KeyCode::Esc => "esc",
        KeyCode::Tab => "↹",
        KeyCode::Up => "↑",
        KeyCode::Down => "↓",
        KeyCode::Left => "←",
        KeyCode::Right => "→",
        KeyCode::Backspace => "⌫",
        KeyCode::Delete => "Del",
        KeyCode::F(n) => {
            return if shift {
                format!("⇧F{n}")
            } else {
                format!("F{n}")
            };
        }
        _ => return format!("{:?}", k.code),
    };
    if shift {
        format!("⇧{base}")
    } else {
        base.to_string()
    }
}

fn key(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
}

fn shift(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::SHIFT)
}

fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

/// Map a config key like `"quit"` to [`UserIntent`]. Returns `None` for
/// unknown names (intents carrying payloads such as `GotoTab(_)`
/// are not user-overridable).
fn parse_intent_name(name: &str) -> Option<UserIntent> {
    match name {
        "quit" => Some(UserIntent::Quit),
        "refresh" => Some(UserIntent::Refresh),
        "down" => Some(UserIntent::Down),
        "up" => Some(UserIntent::Up),
        "left" => Some(UserIntent::Left),
        "right" => Some(UserIntent::Right),
        "focus_left" => Some(UserIntent::FocusLeft),
        "focus_right" => Some(UserIntent::FocusRight),
        "page_down" => Some(UserIntent::PageDown),
        "page_up" => Some(UserIntent::PageUp),
        "top" => Some(UserIntent::Top),
        "bottom" => Some(UserIntent::Bottom),
        "open_selected" => Some(UserIntent::OpenSelected),
        "start_review" => Some(UserIntent::StartReview),
        "start_comment" => Some(UserIntent::StartComment),
        "start_reply_to_thread" => Some(UserIntent::StartReplyToThread),
        "start_line_comment" => Some(UserIntent::StartLineComment),
        "checkout_branch" => Some(UserIntent::CheckoutBranch),
        "merge" => Some(UserIntent::Merge),
        "toggle_close" => Some(UserIntent::ToggleClose),
        "submit" => Some(UserIntent::Submit),
        "cancel" => Some(UserIntent::Cancel),
        "enter_command_palette" => Some(UserIntent::EnterCommandPalette),
        "enter_help" => Some(UserIntent::EnterHelp),
        "enter_repo_switcher" => Some(UserIntent::EnterRepoSwitcher),
        "enter_log_view" => Some(UserIntent::EnterLogView),
        "toggle_side_by_side" => Some(UserIntent::ToggleSideBySide),
        "toggle_whitespace" => Some(UserIntent::ToggleWhitespace),
        "search_forward" => Some(UserIntent::SearchForward),
        "next_hunk" => Some(UserIntent::NextHunk),
        "prev_hunk" => Some(UserIntent::PrevHunk),
        "next_file" => Some(UserIntent::NextFile),
        "prev_file" => Some(UserIntent::PrevFile),
        "load_more" => Some(UserIntent::LoadMore),
        "open_filter_modal" => Some(UserIntent::OpenFilterModal),
        "open_sort_modal" => Some(UserIntent::OpenSortModal),
        "open_search" => Some(UserIntent::OpenSearch),
        "open_in_browser" => Some(UserIntent::OpenInBrowser),
        "next_tab" => Some(UserIntent::NextTab),
        "prev_tab" => Some(UserIntent::PrevTab),
        "close_tab" => Some(UserIntent::CloseTab),
        "goto_dashboard" => Some(UserIntent::GotoDashboard),
        _ => None,
    }
}

/// Parse a key spec like `"Q"`, `"?"`, `"ctrl-s"`, `"shift-tab"`,
/// `"enter"`, `"esc"`, `"space"`, `"up"`, `"down"`, `"left"`, `"right"`,
/// `"f1"..="f12"`. Modifier chain is `ctrl-`, `shift-`, `alt-` (any order).
/// Returns `None` for malformed specs.
#[must_use]
pub fn parse_key_spec(spec: &str) -> Option<KeyEvent> {
    let lower = spec.to_ascii_lowercase();
    let mut modifiers = KeyModifiers::NONE;
    let mut rest = lower.as_str();
    loop {
        if let Some(r) = rest.strip_prefix("ctrl-") {
            modifiers |= KeyModifiers::CONTROL;
            rest = r;
        } else if let Some(r) = rest.strip_prefix("shift-") {
            modifiers |= KeyModifiers::SHIFT;
            rest = r;
        } else if let Some(r) = rest.strip_prefix("alt-") {
            modifiers |= KeyModifiers::ALT;
            rest = r;
        } else {
            break;
        }
    }
    let code = match rest {
        "enter" => KeyCode::Enter,
        "esc" | "escape" => KeyCode::Esc,
        "tab" => KeyCode::Tab,
        "space" => KeyCode::Char(' '),
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "backspace" => KeyCode::Backspace,
        "delete" | "del" => KeyCode::Delete,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "pageup" | "page_up" => KeyCode::PageUp,
        "pagedown" | "page_down" => KeyCode::PageDown,
        s if s.starts_with('f') && s.len() <= 3 => {
            let n: u8 = s[1..].parse().ok()?;
            if !(1..=12).contains(&n) {
                return None;
            }
            KeyCode::F(n)
        }
        s => {
            // Single character. Preserve case from ORIGINAL spec so that
            // "Q" is shift-q, "q" is plain q. We rely on the original
            // string here, not `lower`.
            let original_rest = &spec[spec.len() - s.len()..];
            let mut chars = original_rest.chars();
            let c = chars.next()?;
            if chars.next().is_some() {
                return None;
            }
            // Auto-set SHIFT for uppercase ASCII letters when no explicit
            // shift modifier given.
            if c.is_ascii_uppercase() && !modifiers.contains(KeyModifiers::SHIFT) {
                modifiers |= KeyModifiers::SHIFT;
            }
            KeyCode::Char(c)
        }
    };
    Some(KeyEvent::new(code, modifiers))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn h_l_and_arrows_map_to_focus_intents() {
        let km = Keymap::vim_defaults();
        assert_eq!(km.resolve(&key('h')), Some(UserIntent::FocusLeft));
        assert_eq!(km.resolve(&key('l')), Some(UserIntent::FocusRight));
        assert_eq!(
            km.resolve(&KeyEvent::new(KeyCode::Left, KeyModifiers::NONE)),
            Some(UserIntent::FocusLeft)
        );
        assert_eq!(
            km.resolve(&KeyEvent::new(KeyCode::Right, KeyModifiers::NONE)),
            Some(UserIntent::FocusRight)
        );
    }

    #[test]
    fn parse_intent_name_handles_focus_and_legacy() {
        assert_eq!(parse_intent_name("focus_left"), Some(UserIntent::FocusLeft));
        assert_eq!(
            parse_intent_name("focus_right"),
            Some(UserIntent::FocusRight)
        );
        assert_eq!(parse_intent_name("left"), Some(UserIntent::Left));
        assert_eq!(parse_intent_name("right"), Some(UserIntent::Right));
    }

    #[test]
    fn default_maps_core_vim_bindings() {
        let km = Keymap::vim_defaults();
        assert_eq!(km.resolve(&key('q')), Some(UserIntent::Quit));
        assert_eq!(km.resolve(&key('j')), Some(UserIntent::Down));
        assert_eq!(km.resolve(&key('k')), Some(UserIntent::Up));
        assert_eq!(km.resolve(&key('c')), Some(UserIntent::StartComment));
        assert_eq!(km.resolve(&key('?')), Some(UserIntent::EnterHelp));
        assert_eq!(km.resolve(&ctrl('s')), Some(UserIntent::Submit));
        assert_eq!(km.resolve(&shift('G')), Some(UserIntent::Bottom));
        assert_eq!(km.resolve(&shift('M')), Some(UserIntent::LoadMore));
        assert_eq!(km.resolve(&shift('C')), Some(UserIntent::CheckoutBranch));
        assert_eq!(km.resolve(&key('v')), Some(UserIntent::StartReview));
        assert_eq!(km.resolve(&key('m')), Some(UserIntent::Merge));
        assert_eq!(km.resolve(&key('x')), Some(UserIntent::ToggleClose));
        assert_eq!(km.resolve(&key('f')), Some(UserIntent::OpenFilterModal));
        assert_eq!(km.resolve(&key('s')), Some(UserIntent::OpenSortModal));
        assert_eq!(km.resolve(&key('/')), Some(UserIntent::OpenSearch));
    }

    #[test]
    fn unbound_key_returns_none() {
        let km = Keymap::vim_defaults();
        assert_eq!(km.resolve(&ctrl('z')), None);
    }

    #[test]
    fn override_remaps_intent_and_strips_default() {
        let mut km = Keymap::vim_defaults();
        let mut over = HashMap::new();
        over.insert("quit".to_string(), "Q".to_string());
        km.apply_overrides(&over);
        assert_eq!(km.resolve(&shift('Q')), Some(UserIntent::Quit));
        assert_eq!(km.resolve(&key('q')), None, "default q must be stripped");
    }

    #[test]
    fn override_unknown_intent_is_ignored() {
        let mut km = Keymap::vim_defaults();
        let mut over = HashMap::new();
        over.insert("not_a_real_intent".to_string(), "Q".to_string());
        km.apply_overrides(&over);
        assert_eq!(
            km.resolve(&key('q')),
            Some(UserIntent::Quit),
            "default preserved on unknown intent"
        );
    }

    #[test]
    fn override_unparseable_spec_is_ignored() {
        let mut km = Keymap::vim_defaults();
        let mut over = HashMap::new();
        over.insert("quit".to_string(), "not a key spec".to_string());
        km.apply_overrides(&over);
        assert_eq!(
            km.resolve(&key('q')),
            Some(UserIntent::Quit),
            "default preserved on bad spec"
        );
    }

    #[test]
    fn parse_key_spec_handles_modifiers_and_named_keys() {
        assert_eq!(parse_key_spec("Q"), Some(shift('Q')));
        assert_eq!(parse_key_spec("q"), Some(key('q')));
        assert_eq!(parse_key_spec("?"), Some(key('?')));
        assert_eq!(parse_key_spec("ctrl-s"), Some(ctrl('s')));
        assert_eq!(
            parse_key_spec("enter"),
            Some(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        );
        assert_eq!(
            parse_key_spec("esc"),
            Some(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        );
        assert_eq!(
            parse_key_spec("shift-tab"),
            Some(KeyEvent::new(KeyCode::Tab, KeyModifiers::SHIFT))
        );
        assert_eq!(
            parse_key_spec("f1"),
            Some(KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE))
        );
        assert_eq!(
            parse_key_spec("up"),
            Some(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE))
        );
    }

    #[test]
    fn parse_key_spec_rejects_garbage() {
        assert_eq!(parse_key_spec(""), None);
        assert_eq!(parse_key_spec("not a key"), None);
        assert_eq!(parse_key_spec("f99"), None);
        assert_eq!(parse_key_spec("ctrl-"), None);
    }

    #[test]
    fn tab_nav_keys_resolve_correctly() {
        let km = Keymap::vim_defaults();
        assert_eq!(
            km.resolve(&KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)),
            Some(UserIntent::NextTab)
        );
        assert_eq!(
            km.resolve(&KeyEvent::new(KeyCode::Tab, KeyModifiers::SHIFT)),
            Some(UserIntent::PrevTab)
        );
        assert_eq!(km.resolve(&key('w')), Some(UserIntent::CloseTab));
        assert_eq!(km.resolve(&key('1')), Some(UserIntent::GotoTab(1)));
        assert_eq!(km.resolve(&key('9')), Some(UserIntent::GotoTab(9)));
    }

    #[test]
    fn esc_resolves_to_cancel() {
        let km = Keymap::vim_defaults();
        assert_eq!(
            km.resolve(&KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            Some(UserIntent::Cancel)
        );
    }

    // ── key_for tests ────────────────────────────────────────────────────

    #[test]
    fn key_for_down_prefers_j_over_arrow() {
        let km = Keymap::vim_defaults();
        // Both `j` and Down are bound to Down; prefer the letter.
        assert_eq!(km.key_for(&UserIntent::Down), Some(key('j')));
    }

    #[test]
    fn key_for_quit_returns_q() {
        let km = Keymap::vim_defaults();
        assert_eq!(km.key_for(&UserIntent::Quit), Some(key('q')));
    }

    #[test]
    fn key_for_unbound_intent_returns_none() {
        let km = Keymap::vim_defaults();
        // ToggleSideBySide has no default binding.
        assert_eq!(km.key_for(&UserIntent::ToggleSideBySide), None);
    }

    // ── key_label tests ─────────────────────────────────────────────────

    #[test]
    fn key_label_special_keys() {
        use crossterm::event::KeyCode;
        let enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        let esc = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        let tab = KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE);
        let space = KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE);
        let up = KeyEvent::new(KeyCode::Up, KeyModifiers::NONE);
        let down = KeyEvent::new(KeyCode::Down, KeyModifiers::NONE);
        let left = KeyEvent::new(KeyCode::Left, KeyModifiers::NONE);
        let right = KeyEvent::new(KeyCode::Right, KeyModifiers::NONE);
        assert_eq!(key_label(&enter), "↵");
        assert_eq!(key_label(&esc), "esc");
        assert_eq!(key_label(&tab), "↹");
        assert_eq!(key_label(&space), "␣");
        assert_eq!(key_label(&up), "↑");
        assert_eq!(key_label(&down), "↓");
        assert_eq!(key_label(&left), "←");
        assert_eq!(key_label(&right), "→");
        // Shift on a non-Char key is surfaced (Shift-Tab is bound to PrevTab).
        let shift_tab = KeyEvent::new(KeyCode::Tab, KeyModifiers::SHIFT);
        assert_eq!(key_label(&shift_tab), "⇧↹");
    }

    #[test]
    fn key_label_plain_and_shifted_chars() {
        assert_eq!(key_label(&key('c')), "c");
        assert_eq!(key_label(&shift('W')), "W");
    }

    #[test]
    fn key_label_ctrl() {
        assert_eq!(key_label(&ctrl('s')), "Ctrl-s");
    }

    #[test]
    fn key_label_function_key() {
        use crossterm::event::KeyCode;
        let f1 = KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE);
        let f12 = KeyEvent::new(KeyCode::F(12), KeyModifiers::NONE);
        assert_eq!(key_label(&f1), "F1");
        assert_eq!(key_label(&f12), "F12");
    }

    #[test]
    fn bracket_keys_resolve_to_switch_tab_prev_next() {
        let km = Keymap::vim_defaults();
        assert_eq!(
            km.resolve(&key('[')),
            Some(UserIntent::SwitchTab(TabDirection::Prev))
        );
        assert_eq!(
            km.resolve(&key(']')),
            Some(UserIntent::SwitchTab(TabDirection::Next))
        );
    }
}
