use ratatui::style::Color;
use ratatui::widgets::BorderType;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    pub primary: Color,
    pub secondary: Color,
    pub success: Color,
    pub warning: Color,
    pub error: Color,
    pub dim: Color,
    pub bg: Color,
    pub fg: Color,
}

impl Theme {
    #[must_use]
    pub const fn dark() -> Self {
        Self {
            primary: Color::Rgb(0x58, 0xa6, 0xff),
            secondary: Color::Rgb(0xbc, 0x8c, 0xff),
            success: Color::Rgb(0x3f, 0xb9, 0x50),
            warning: Color::Rgb(0xd2, 0x9e, 0x2e),
            error: Color::Rgb(0xf8, 0x51, 0x49),
            dim: Color::Rgb(0x8b, 0x94, 0x9e),
            bg: Color::Rgb(0x0d, 0x11, 0x17),
            fg: Color::Rgb(0xc9, 0xd1, 0xd9),
        }
    }

    /// Monochrome theme that defers every color to the terminal's defaults
    /// (`Color::Reset`). Selected by `--no-color` / `NO_COLOR`.
    #[must_use]
    pub const fn no_color() -> Self {
        Self {
            primary: Color::Reset,
            secondary: Color::Reset,
            success: Color::Reset,
            warning: Color::Reset,
            error: Color::Reset,
            dim: Color::Reset,
            bg: Color::Reset,
            fg: Color::Reset,
        }
    }

    /// Resolve a built-in theme by config name. Unknown names fall back to
    /// `dark` so a typo never crashes startup.
    #[must_use]
    pub fn from_name(name: &str) -> Self {
        match name.trim().to_ascii_lowercase().as_str() {
            "light" => Self::light(),
            _ => Self::dark(),
        }
    }

    #[must_use]
    pub const fn light() -> Self {
        Self {
            primary: Color::Rgb(0x05, 0x69, 0xd6),
            secondary: Color::Rgb(0x8a, 0x4f, 0xff),
            success: Color::Rgb(0x1a, 0x7f, 0x37),
            warning: Color::Rgb(0x9a, 0x69, 0x00),
            error: Color::Rgb(0xcf, 0x22, 0x2e),
            dim: Color::Rgb(0x65, 0x6d, 0x76),
            bg: Color::Rgb(0xff, 0xff, 0xff),
            fg: Color::Rgb(0x1f, 0x23, 0x28),
        }
    }

    /// `BorderType` suited to this theme's rendering environment.
    /// Rounded corners for colour themes; plain box-drawing for no-colour.
    #[must_use]
    pub fn border_type(&self) -> BorderType {
        if self.primary == Color::Reset {
            BorderType::Plain
        } else {
            BorderType::Rounded
        }
    }

    /// Whether this theme's terminal is expected to render Unicode glyphs.
    /// Returns `false` for `no_color()` since `--no-color` is the common
    /// flag for degraded / pipe / limited terminal contexts.
    #[must_use]
    pub fn use_unicode_glyphs(&self) -> bool {
        self.primary != Color::Reset
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn border_type_rounded_for_color_themes() {
        assert_eq!(
            Theme::dark().border_type(),
            ratatui::widgets::BorderType::Rounded
        );
        assert_eq!(
            Theme::light().border_type(),
            ratatui::widgets::BorderType::Rounded
        );
    }

    #[test]
    fn border_type_plain_for_no_color() {
        assert_eq!(
            Theme::no_color().border_type(),
            ratatui::widgets::BorderType::Plain
        );
    }

    #[test]
    fn use_unicode_glyphs_for_color_themes() {
        assert!(Theme::dark().use_unicode_glyphs());
        assert!(Theme::light().use_unicode_glyphs());
        assert!(!Theme::no_color().use_unicode_glyphs());
    }

    #[test]
    fn dark_and_light_have_distinct_backgrounds() {
        assert_ne!(Theme::dark().bg, Theme::light().bg);
        assert_ne!(Theme::dark().fg, Theme::light().fg);
    }

    #[test]
    fn semantic_colors_are_distinct_within_palette() {
        for theme in [Theme::dark(), Theme::light()] {
            let palette = [
                theme.primary,
                theme.secondary,
                theme.success,
                theme.warning,
                theme.error,
                theme.dim,
                theme.bg,
                theme.fg,
            ];
            for (i, a) in palette.iter().enumerate() {
                for b in &palette[i + 1..] {
                    assert_ne!(a, b, "duplicate color in palette: {a:?}");
                }
            }
        }
    }
}
