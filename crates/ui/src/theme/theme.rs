use super::{ThemeColors, ThemeMetrics, ThemeTypography};

/// The single application-wide theme. It contains no docking model state.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Theme {
    pub colors: ThemeColors,
    pub metrics: ThemeMetrics,
    pub typography: ThemeTypography,
}

impl Theme {
    /// Validates the public theme data before it is used to build layout geometry.
    pub fn validate(&self) -> Result<(), &'static str> {
        self.metrics.validate()?;
        self.typography.validate()?;

        for (name, color) in [
            ("window_background", self.colors.window_background),
            ("canvas_background", self.colors.canvas_background),
            ("dock_background", self.colors.dock_background),
            ("group_background", self.colors.group_background),
            ("border", self.colors.border),
            ("splitter", self.colors.splitter),
            ("primary_text", self.colors.primary_text),
            ("muted_text", self.colors.muted_text),
            ("active_tab", self.colors.active_tab),
            ("hover", self.colors.hover),
            ("focus", self.colors.focus),
            ("drop_target", self.colors.drop_target),
            ("toolbar_background", self.colors.toolbar_background),
        ] {
            if color > 0xFF_FFFF {
                return Err(match name {
                    "window_background" => "window_background must be a 24-bit RGB color",
                    "canvas_background" => "canvas_background must be a 24-bit RGB color",
                    "dock_background" => "dock_background must be a 24-bit RGB color",
                    "group_background" => "group_background must be a 24-bit RGB color",
                    "border" => "border must be a 24-bit RGB color",
                    "splitter" => "splitter must be a 24-bit RGB color",
                    "primary_text" => "primary_text must be a 24-bit RGB color",
                    "muted_text" => "muted_text must be a 24-bit RGB color",
                    "active_tab" => "active_tab must be a 24-bit RGB color",
                    "hover" => "hover must be a 24-bit RGB color",
                    "focus" => "focus must be a 24-bit RGB color",
                    "drop_target" => "drop_target must be a 24-bit RGB color",
                    "toolbar_background" => "toolbar_background must be a 24-bit RGB color",
                    _ => "theme colors must be 24-bit RGB values",
                });
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::Theme;

    #[test]
    fn default_theme_is_valid() {
        assert_eq!(Theme::default().validate(), Ok(()));
    }

    #[test]
    fn rejects_invalid_rail_hysteresis_thresholds() {
        let mut theme = Theme::default();
        theme.metrics.rail_expand_width = theme.metrics.rail_collapse_width;

        assert_eq!(
            theme.validate(),
            Err("rail_expand_width must exceed rail_collapse_width")
        );
    }

    #[test]
    fn rejects_non_rgb_palette_values() {
        let mut theme = Theme::default();
        theme.colors.focus = 0x1FF_0000;

        assert_eq!(theme.validate(), Err("focus must be a 24-bit RGB color"));
    }
}
