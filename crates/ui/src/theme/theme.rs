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
            ("column_mode_button", self.colors.column_mode_button),
            ("column_mode_button_hover", self.colors.column_mode_button_hover),
            ("drop_zone", self.colors.drop_zone),
            ("drop_zone_border", self.colors.drop_zone_border),
            ("floating_title_background", self.colors.floating_title_background),
            ("flyout_header_background", self.colors.flyout_header_background),
            ("group_frame_background", self.colors.group_frame_background),
            ("group_frame_border", self.colors.group_frame_border),
            ("group_grip_background", self.colors.group_grip_background),
            ("group_menu_button", self.colors.group_menu_button),
            ("group_menu_button_hover", self.colors.group_menu_button_hover),
            ("icon_button_background", self.colors.icon_button_background),
            ("icon_button_hover", self.colors.icon_button_hover),
            ("icon_button_border", self.colors.icon_button_border),
            ("panel_surface_background", self.colors.panel_surface_background),
            ("panel_tab_active", self.colors.panel_tab_active),
            ("panel_tab_inactive", self.colors.panel_tab_inactive),
            ("panel_tab_hover", self.colors.panel_tab_hover),
            ("popup_menu_background", self.colors.popup_menu_background),
            ("popup_menu_selected", self.colors.popup_menu_selected),
            ("popup_menu_hover", self.colors.popup_menu_hover),
            ("popup_menu_border", self.colors.popup_menu_border),
            ("rail_entry_active", self.colors.rail_entry_active),
            ("rail_entry_inactive", self.colors.rail_entry_inactive),
            ("rail_entry_hover", self.colors.rail_entry_hover),
            ("resize_edge_handle", self.colors.resize_edge_handle),
            ("splitter_handle", self.colors.splitter_handle),
            ("tab_overflow_button", self.colors.tab_overflow_button),
            ("tab_overflow_button_hover", self.colors.tab_overflow_button_hover),
            ("tab_strip_background", self.colors.tab_strip_background),
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
                    _ => "component theme colors must be 24-bit RGB values",
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

    #[test]
    fn rejects_non_rgb_component_palette_values() {
        let mut theme = Theme::default();
        theme.colors.popup_menu_background = 0x1FF_0000;

        assert_eq!(
            theme.validate(),
            Err("component theme colors must be 24-bit RGB values")
        );
    }
}
