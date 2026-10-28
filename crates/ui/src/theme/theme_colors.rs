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
            window_background: 0x141D2A,
            canvas_background: 0x101923,
            dock_background: 0x1B2836,
            group_background: 0x203142,
            border: 0x435467,
            splitter: 0x47616F,
            primary_text: 0xEAF2F8,
            muted_text: 0xA8B9C8,
            active_tab: 0x2D5B79,
            hover: 0x3A536A,
            focus: 0x85DED2,
            drop_target: 0x39A58F,
            toolbar_background: 0x26394B,

            column_mode_button: 0x365E73,
            column_mode_button_hover: 0x47788D,
            drop_zone: 0x35A18D,
            drop_zone_border: 0x77E3CF,
            floating_title_background: 0x4A3C61,
            flyout_header_background: 0x285D63,
            group_frame_background: 0x20384A,
            group_frame_border: 0x54788F,
            group_grip_background: 0x6D593B,
            group_menu_button: 0x765239,
            group_menu_button_hover: 0x946849,
            icon_button_background: 0x374D64,
            icon_button_hover: 0x4A6580,
            icon_button_border: 0x5F7C96,
            panel_surface_background: 0x1D3541,
            panel_tab_active: 0x2D5D7A,
            panel_tab_inactive: 0x314556,
            panel_tab_hover: 0x3E5A71,
            popup_menu_background: 0x302E46,
            popup_menu_selected: 0x53476F,
            popup_menu_hover: 0x433D5D,
            popup_menu_border: 0x71658A,
            rail_entry_active: 0x2F5A45,
            rail_entry_inactive: 0x243C33,
            rail_entry_hover: 0x3A594A,
            resize_edge_handle: 0x956343,
            splitter_handle: 0x416E7C,
            tab_overflow_button: 0x584878,
            tab_overflow_button_hover: 0x6C5C91,
            tab_strip_background: 0x302F48,
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
