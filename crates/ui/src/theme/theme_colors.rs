/// Opaque 24-bit RGB value consumed by the GPUI adapter.
pub type Rgb = u32;

/// Central application palette, including distinct development colors for each UI component.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThemeColors {
    /// Main application window background.
    pub window_background: Rgb,
    /// Background behind the main document canvas.
    pub canvas_background: Rgb,
    /// Base background for the docking area.
    pub dock_background: Rgb,
    /// Generic group background retained for custom hosts.
    pub group_background: Rgb,
    /// Generic outline color retained for custom hosts.
    pub border: Rgb,
    /// Generic splitter color retained for custom hosts.
    pub splitter: Rgb,
    /// High-contrast primary text.
    pub primary_text: Rgb,
    /// Secondary text and less prominent labels.
    pub muted_text: Rgb,
    /// Generic selected-state background retained for custom hosts.
    pub active_tab: Rgb,
    /// Generic hover background retained for custom hosts.
    pub hover: Rgb,
    /// Keyboard focus outline.
    pub focus: Rgb,
    /// General docking preview accent.
    pub drop_target: Rgb,
    /// Generic toolbar surface retained for custom hosts.
    pub toolbar_background: Rgb,

    /// Background for the column-mode switch button.
    pub column_mode_button: Rgb,
    /// Hover background for the column-mode switch button.
    pub column_mode_button_hover: Rgb,
    /// Accent blended into an available drop zone.
    pub drop_zone: Rgb,
    /// Outline for an available or active drop zone.
    pub drop_zone_border: Rgb,
    /// Background of a floating-window title bar.
    pub floating_title_background: Rgb,
    /// Background of a flyout header.
    pub flyout_header_background: Rgb,
    /// Background of a dock group's outer frame.
    pub group_frame_background: Rgb,
    /// Border of a dock group's outer frame.
    pub group_frame_border: Rgb,
    /// Background of the draggable group grip.
    pub group_grip_background: Rgb,
    /// Background of the group's menu button.
    pub group_menu_button: Rgb,
    /// Hover background of the group's menu button.
    pub group_menu_button_hover: Rgb,
    /// Background of a regular icon button.
    pub icon_button_background: Rgb,
    /// Hover background of a regular icon button.
    pub icon_button_hover: Rgb,
    /// Border of icon buttons that opt into a border.
    pub icon_button_border: Rgb,
    /// Background behind the content of an active panel.
    pub panel_surface_background: Rgb,
    /// Background of the active panel tab.
    pub panel_tab_active: Rgb,
    /// Background of an inactive panel tab.
    pub panel_tab_inactive: Rgb,
    /// Hover background of an inactive panel tab.
    pub panel_tab_hover: Rgb,
    /// Background of a popup menu.
    pub popup_menu_background: Rgb,
    /// Background of the selected popup menu row.
    pub popup_menu_selected: Rgb,
    /// Hover background of a popup menu row.
    pub popup_menu_hover: Rgb,
    /// Border of a popup menu.
    pub popup_menu_border: Rgb,
    /// Background of the active rail entry.
    pub rail_entry_active: Rgb,
    /// Background of an inactive rail entry.
    pub rail_entry_inactive: Rgb,
    /// Hover background of an inactive rail entry.
    pub rail_entry_hover: Rgb,
    /// Background of the resize hit zone at an outer edge or floating edge.
    pub resize_edge_handle: Rgb,
    /// Background of an internal splitter handle.
    pub splitter_handle: Rgb,
    /// Background of the tab-overflow button.
    pub tab_overflow_button: Rgb,
    /// Hover background of the tab-overflow button.
    pub tab_overflow_button_hover: Rgb,
    /// Background of the tab strip behind tabs and group controls.
    pub tab_strip_background: Rgb,
}

impl Default for ThemeColors {
    fn default() -> Self {
        Self {
            window_background: 0x0A0406,
            canvas_background: 0x010101,
            dock_background: 0x14080B,
            group_background: 0x301015,
            border: 0x4D1B27,
            splitter: 0x4D1B27,
            primary_text: 0xF2E8EB,
            muted_text: 0xB08C97,
            active_tab: 0x77162B,
            hover: 0x421621,
            focus: 0xCB5780,
            drop_target: 0xAC0632,
            toolbar_background: 0x1E0A0E,

            column_mode_button: 0x421621,
            column_mode_button_hover: 0x5A1A2A,
            drop_zone: 0xAC0632,
            drop_zone_border: 0xCB5780,
            floating_title_background: 0x301015,
            flyout_header_background: 0x261016,
            group_frame_background: 0x1A0A0D,
            group_frame_border: 0x4D1B27,
            group_grip_background: 0x4D1B27,
            group_menu_button: 0x3A1523,
            group_menu_button_hover: 0x77162B,
            icon_button_background: 0x35121C,
            icon_button_hover: 0x5A1A2A,
            icon_button_border: 0x77162B,
            panel_surface_background: 0x15080B,
            panel_tab_active: 0x77162B,
            panel_tab_inactive: 0x230D11,
            panel_tab_hover: 0x3A1320,
            popup_menu_background: 0x2A0D13,
            popup_menu_selected: 0x77162B,
            popup_menu_hover: 0x421621,
            popup_menu_border: 0x5A1A2A,
            rail_entry_active: 0x5A1A2A,
            rail_entry_inactive: 0x14080B,
            rail_entry_hover: 0x301015,
            resize_edge_handle: 0x8A2947,
            splitter_handle: 0x5D2635,
            tab_overflow_button: 0x4A1729,
            tab_overflow_button_hover: 0x5A1A2A,
            tab_strip_background: 0x0A0406,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ThemeColors;

    #[test]
    fn component_surfaces_have_distinct_colors() {
        let colors = ThemeColors::default();
        let component_surfaces = [
            colors.column_mode_button,
            colors.drop_zone,
            colors.floating_title_background,
            colors.flyout_header_background,
            colors.group_frame_background,
            colors.group_grip_background,
            colors.group_menu_button,
            colors.icon_button_background,
            colors.panel_surface_background,
            colors.panel_tab_active,
            colors.panel_tab_inactive,
            colors.popup_menu_background,
            colors.rail_entry_active,
            colors.rail_entry_inactive,
            colors.resize_edge_handle,
            colors.splitter_handle,
            colors.tab_overflow_button,
            colors.tab_strip_background,
        ];

        for (index, color) in component_surfaces.iter().enumerate() {
            assert!(
                !component_surfaces[..index].contains(color),
                "each component surface should have its own color"
            );
        }
    }
}
