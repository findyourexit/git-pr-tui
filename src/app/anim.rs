//! Animation intensity gating, cue types, and effect builders for the motion layer.
//!
//! Provides the pure cue/intensity types and gating, plus the `effect_for`
//! cue→effect builder wired into `App::run` via `tachyonfx`'s `EffectManager`.

/// How much motion the user wants to see.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AnimationIntensity {
    /// All cues produce effects (default).
    #[default]
    Full,
    /// Only overlay-open and focus-change effects fire.
    Subtle,
    /// No effects at all (forced by `--no-color`/`NO_COLOR` or explicit config).
    Off,
}

impl AnimationIntensity {
    /// Parse from the config string value.  Unknown strings fall back to
    /// `Full` (same convention as `Theme::from_name`).
    #[must_use]
    pub fn from_config_str(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "subtle" => Self::Subtle,
            "off" => Self::Off,
            _ => Self::Full,
        }
    }
}

/// Direction of a tab switch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabSwitchDir {
    Left,
    Right,
}

/// An animation trigger produced by a reducer transition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnimationCue {
    /// User navigated between tabs; `dir` is the visual direction of travel.
    TabSwitch { dir: TabSwitchDir },
    /// An overlay (action menu, palette, composer, modal) was opened.
    OverlayOpen,
    /// A view's data first appeared after entering it. Fires once per
    /// view-entry (re-armed on view change), so staged async loads for one
    /// view (detail, checks, files) don't replay the transition.
    DataLoaded,
    /// The focused diff pane changed.
    FocusChange,
    /// A write action completed; `ok` indicates success vs failure.
    ActionLanded { ok: bool },
}

/// Returns `true` if `cue` should fire an effect under `intensity`.
///
/// | cue            | Full | Subtle | Off |
/// |----------------|------|--------|-----|
/// | TabSwitch      | ✓    | ✗      | ✗   |
/// | OverlayOpen    | ✓    | ✓      | ✗   |
/// | DataLoaded     | ✓    | ✗      | ✗   |
/// | FocusChange    | ✓    | ✓      | ✗   |
/// | ActionLanded   | ✓    | ✗      | ✗   |
#[must_use]
pub fn cue_allowed(intensity: AnimationIntensity, cue: &AnimationCue) -> bool {
    match intensity {
        AnimationIntensity::Off => false,
        AnimationIntensity::Full => true,
        AnimationIntensity::Subtle => {
            matches!(cue, AnimationCue::OverlayOpen | AnimationCue::FocusChange)
        }
    }
}

/// Map an [`AnimationCue`] to a fast tachyonfx 0.25 effect over `area`.
///
/// Most durations are kept ≤ 180 ms so the UI feels snappy. `DataLoaded` is the
/// exception: it now fires only once per view-entry (no replay as staged data
/// streams in), so it uses a longer window to remain clearly perceptible as a
/// single play. The `_area` and `_theme` parameters are accepted for future
/// per-region / themed effects.
#[must_use]
pub fn effect_for(
    cue: &AnimationCue,
    _area: ratatui::layout::Rect,
    _theme: &crate::ui::theme::Theme,
) -> tachyonfx::Effect {
    use ratatui::style::Color;
    use tachyonfx::{Interpolation, Motion, fx};

    match cue {
        AnimationCue::TabSwitch { dir } => {
            let motion = match dir {
                TabSwitchDir::Right => Motion::LeftToRight,
                TabSwitchDir::Left => Motion::RightToLeft,
            };
            fx::slide_in(motion, 10, 0, Color::Black, (120, Interpolation::QuadOut))
        }
        AnimationCue::OverlayOpen => {
            fx::fade_from(Color::Black, Color::Black, (160, Interpolation::QuadOut))
        }
        // Fires once per view-entry; longer than the other cues so a single
        // play is clearly visible (the old 140 ms only read as motion because
        // staged loads replayed it several times).
        AnimationCue::DataLoaded => fx::coalesce((320, Interpolation::QuadOut)),
        AnimationCue::FocusChange => {
            // Brief hue-shift pulse on the focused region.
            fx::hsl_shift(Some([20.0, 0.0, 15.0]), None, (100, Interpolation::QuadOut))
        }
        AnimationCue::ActionLanded { ok: true } => fx::sweep_in(
            Motion::LeftToRight,
            10,
            0,
            Color::Green,
            (140, Interpolation::QuadOut),
        ),
        AnimationCue::ActionLanded { ok: false } => {
            fx::fade_from(Color::Red, Color::Black, (160, Interpolation::QuadOut))
        }
    }
}

/// Returns the frame budget for the animation clock.
///
/// When `has_active` is `true`, returns the target frame interval (≈ 60 fps);
/// when idle, returns `None` so the caller can use `pending()` and block.
#[must_use]
pub fn frame_budget(has_active: bool) -> Option<std::time::Duration> {
    if has_active {
        Some(std::time::Duration::from_millis(16))
    } else {
        None
    }
}

/// Decide whether the run loop should restart its frame clock (`last_frame`)
/// after draining animation cues.
///
/// The clock must be reset on the **idle → active** transition: while no effect
/// is running, the run loop blocks in `select!` (the frame-wake branch is
/// `pending()` because [`frame_budget`] returns `None`), so `last_frame` keeps
/// pointing at the previous wake — up to the 500 ms tick interval, or the full
/// network-fetch latency for a data event. If that stale interval were charged
/// as the first `dt`, a freshly-added effect (≤ 180 ms) would advance to
/// completion in a single frame and be imperceptible. Resetting the clock makes
/// the effect's first rendered frame start at ~0 and then advance one
/// frame-budget (~16 ms) per frame.
///
/// While effects are already running the frame-wake fires every ~16 ms and
/// keeps `last_frame` fresh, so no reset is needed (and resetting would briefly
/// stall the in-flight effects) — hence the `was_running` guard.
#[must_use]
pub fn should_reset_frame_clock(was_running: bool, effects_added: bool) -> bool {
    effects_added && !was_running
}

/// Stable string key for the [`tachyonfx::EffectManager`] representing each
/// cue kind. Used to de-duplicate concurrent cues of the same kind.
#[must_use]
pub fn cue_key(cue: &AnimationCue) -> &'static str {
    match cue {
        AnimationCue::TabSwitch { .. } => "tab_switch",
        AnimationCue::OverlayOpen => "overlay_open",
        AnimationCue::DataLoaded => "data_loaded",
        AnimationCue::FocusChange => "focus_change",
        AnimationCue::ActionLanded { .. } => "action_landed",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- from_config_str ---

    #[test]
    fn parse_full() {
        assert_eq!(
            AnimationIntensity::from_config_str("full"),
            AnimationIntensity::Full
        );
    }

    #[test]
    fn parse_subtle() {
        assert_eq!(
            AnimationIntensity::from_config_str("subtle"),
            AnimationIntensity::Subtle
        );
    }

    #[test]
    fn parse_off() {
        assert_eq!(
            AnimationIntensity::from_config_str("off"),
            AnimationIntensity::Off
        );
    }

    #[test]
    fn parse_unknown_falls_back_to_full() {
        assert_eq!(
            AnimationIntensity::from_config_str("banana"),
            AnimationIntensity::Full
        );
        assert_eq!(
            AnimationIntensity::from_config_str(""),
            AnimationIntensity::Full
        );
    }

    #[test]
    fn parse_case_insensitive() {
        assert_eq!(
            AnimationIntensity::from_config_str("FULL"),
            AnimationIntensity::Full
        );
        assert_eq!(
            AnimationIntensity::from_config_str("OFF"),
            AnimationIntensity::Off
        );
    }

    // --- cue_allowed truth table ---

    fn all_cues() -> Vec<AnimationCue> {
        vec![
            AnimationCue::TabSwitch {
                dir: TabSwitchDir::Right,
            },
            AnimationCue::OverlayOpen,
            AnimationCue::DataLoaded,
            AnimationCue::FocusChange,
            AnimationCue::ActionLanded { ok: true },
        ]
    }

    #[test]
    fn off_blocks_all() {
        for cue in all_cues() {
            assert!(
                !cue_allowed(AnimationIntensity::Off, &cue),
                "Off should block {cue:?}"
            );
        }
    }

    #[test]
    fn full_allows_all() {
        for cue in all_cues() {
            assert!(
                cue_allowed(AnimationIntensity::Full, &cue),
                "Full should allow {cue:?}"
            );
        }
    }

    #[test]
    fn subtle_allows_only_overlay_open_and_focus_change() {
        let allowed = [AnimationCue::OverlayOpen, AnimationCue::FocusChange];
        let blocked = [
            AnimationCue::TabSwitch {
                dir: TabSwitchDir::Left,
            },
            AnimationCue::DataLoaded,
            AnimationCue::ActionLanded { ok: false },
        ];
        for cue in &allowed {
            assert!(
                cue_allowed(AnimationIntensity::Subtle, cue),
                "Subtle should allow {cue:?}"
            );
        }
        for cue in &blocked {
            assert!(
                !cue_allowed(AnimationIntensity::Subtle, cue),
                "Subtle should block {cue:?}"
            );
        }
    }

    // --- frame_budget ---

    #[test]
    fn frame_budget_active_is_16ms() {
        assert_eq!(
            frame_budget(true),
            Some(std::time::Duration::from_millis(16))
        );
    }

    #[test]
    fn frame_budget_idle_is_none() {
        assert_eq!(frame_budget(false), None);
    }

    // --- should_reset_frame_clock ---

    #[test]
    fn reset_clock_on_idle_to_active_transition() {
        // Adding effects while the manager was idle must reset the clock so the
        // idle/blocking interval is not charged to the new effect's first dt.
        assert!(should_reset_frame_clock(false, true));
    }

    #[test]
    fn no_reset_when_no_effects_added() {
        assert!(!should_reset_frame_clock(false, false));
        assert!(!should_reset_frame_clock(true, false));
    }

    #[test]
    fn no_reset_when_already_running() {
        // The frame-wake keeps last_frame fresh while effects run, so adding
        // more effects mid-flight must not stall the in-flight ones.
        assert!(!should_reset_frame_clock(true, true));
    }

    // --- cue_key de-dup ---

    #[test]
    fn cue_key_same_kind_same_key() {
        assert_eq!(
            cue_key(&AnimationCue::TabSwitch {
                dir: TabSwitchDir::Left
            }),
            cue_key(&AnimationCue::TabSwitch {
                dir: TabSwitchDir::Right
            })
        );
        assert_eq!(
            cue_key(&AnimationCue::ActionLanded { ok: true }),
            cue_key(&AnimationCue::ActionLanded { ok: false })
        );
    }

    // --- effect_for doesn’t panic ---

    #[test]
    fn effect_for_each_cue_does_not_panic() {
        use ratatui::layout::Rect;
        let area = Rect::new(0, 0, 80, 24);
        let theme = crate::ui::theme::Theme::dark();
        let cues = [
            AnimationCue::TabSwitch {
                dir: TabSwitchDir::Right,
            },
            AnimationCue::TabSwitch {
                dir: TabSwitchDir::Left,
            },
            AnimationCue::OverlayOpen,
            AnimationCue::DataLoaded,
            AnimationCue::FocusChange,
            AnimationCue::ActionLanded { ok: true },
            AnimationCue::ActionLanded { ok: false },
        ];
        for cue in &cues {
            let _effect = effect_for(cue, area, &theme);
        }
    }
}
