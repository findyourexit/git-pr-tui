#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutMode {
    Full,
    Compact,
    FullScreenModal,
}

/// Pick the rendering mode for a terminal of size `width × height`.
///
/// Small-resize fallback:
/// * `< 60 × 18` → `FullScreenModal` (only one pane fits comfortably)
/// * `< 80 × 24` → `Compact` (drop secondary chrome)
/// * otherwise → `Full`
#[must_use]
pub fn min_size_check(width: u16, height: u16) -> LayoutMode {
    if width < 60 || height < 18 {
        LayoutMode::FullScreenModal
    } else if width < 80 || height < 24 {
        LayoutMode::Compact
    } else {
        LayoutMode::Full
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_at_or_above_eighty_by_twentyfour() {
        assert_eq!(min_size_check(80, 24), LayoutMode::Full);
        assert_eq!(min_size_check(200, 60), LayoutMode::Full);
    }

    #[test]
    fn compact_below_eighty_by_twentyfour_but_at_or_above_sixty_by_eighteen() {
        assert_eq!(min_size_check(79, 24), LayoutMode::Compact);
        assert_eq!(min_size_check(80, 23), LayoutMode::Compact);
        assert_eq!(min_size_check(60, 18), LayoutMode::Compact);
    }

    #[test]
    fn fullscreen_modal_below_sixty_by_eighteen() {
        assert_eq!(min_size_check(59, 18), LayoutMode::FullScreenModal);
        assert_eq!(min_size_check(60, 17), LayoutMode::FullScreenModal);
        assert_eq!(min_size_check(20, 5), LayoutMode::FullScreenModal);
    }
}
