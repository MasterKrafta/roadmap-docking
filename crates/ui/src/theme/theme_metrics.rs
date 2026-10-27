#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ThemeMetrics {
    pub column_header_height: f32,
    pub tab_strip_height: f32,
    /// Width of the collapse/close/menu buttons in a Tabs group.
    pub group_control_button_width: f32,
    /// Width reserved at the end of the tab strip for a future drag target.
    pub tab_drag_zone_width: f32,
    pub button_size: f32,
    pub button_gap: f32,
    pub content_padding: f32,
    pub splitter_thickness: f32,
    pub splitter_hit_zone: f32,
    pub edge_drop_zone_width: f32,
    /// Logical-pixel hit-zone width for group and column docking edges.
    pub drop_edge_zone: f32,
    /// Thickness of the live drop insertion and split indicators in logical pixels.
    pub drop_indicator_thickness: f32,
    /// Thickness of the outline around a target group or floating-window preview.
    pub drop_outline_thickness: f32,
    pub drag_start_threshold: f32,
    pub default_column_width: f32,
    pub min_column_width: f32,
    /// Initial fraction of the column's content area allocated to the upper panel.
    pub default_column_split_ratio: f32,
    /// Width of each tab in the main window's simple column.
    pub column_tab_width: f32,
    pub rail_width: f32,
    /// Width at which resizing an expanded column automatically enters icon-rail mode.
    pub rail_collapse_width: f32,
    /// Width at which dragging an icon rail's outer edge restores expanded mode.
    pub rail_expand_width: f32,
    pub min_group_height: f32,
    pub collapsed_group_height: f32,
    pub min_document_width: f32,
    pub max_columns: usize,
}
impl Default for ThemeMetrics {
    fn default() -> Self {
        Self {
            column_header_height: 32.0,
            tab_strip_height: 28.0,
            group_control_button_width: 34.0,
            tab_drag_zone_width: 28.0,
            button_size: 24.0,
            button_gap: 4.0,
            content_padding: 8.0,
            splitter_thickness: 4.0,
            splitter_hit_zone: 8.0,
            edge_drop_zone_width: 24.0,
            drop_edge_zone: 24.0,
            drop_indicator_thickness: 4.0,
            drop_outline_thickness: 3.0,
            drag_start_threshold: 4.0,
            default_column_width: 280.0,
            min_column_width: 200.0,
            default_column_split_ratio: 0.5,
            column_tab_width: 104.0,
            rail_width: 36.0,
            rail_collapse_width: 220.0,
            rail_expand_width: 240.0,
            min_group_height: 80.0,
            collapsed_group_height: 28.0,
            min_document_width: 320.0,
            max_columns: 4,
        }
    }
}
#[cfg(test)]
mod tests {
    use super::ThemeMetrics;

    #[test]
    fn preserves_initial_metrics_from_the_roadmap_fragment() {
        let m = ThemeMetrics::default();
        assert!((m.default_column_width - 280.0).abs() < 1e-3);
        assert!((m.min_column_width - 200.0).abs() < 1e-3);
        assert!((m.rail_width - 36.0).abs() < 1e-3);
        assert!((m.rail_collapse_width - 220.0).abs() < 1e-3);
        assert!((m.rail_expand_width - 240.0).abs() < 1e-3);
        assert_eq!(m.max_columns, 4);
    }

    #[test]
    fn includes_drag_and_drop_threshold_metrics() {
        let m = ThemeMetrics::default();
        assert!((m.edge_drop_zone_width - 24.0).abs() < 1e-3);
        assert!((m.drop_edge_zone - 24.0).abs() < 1e-3);
        assert!((m.drag_start_threshold - 4.0).abs() < 1e-3);
        assert!((m.drop_indicator_thickness - 4.0).abs() < 1e-3);
        assert!((m.drop_outline_thickness - 3.0).abs() < 1e-3);
    }

    #[test]
    fn includes_tabs_group_chrome_metrics() {
        let m = ThemeMetrics::default();
        assert!((m.group_control_button_width - 34.0).abs() < 1e-3);
        assert!((m.tab_drag_zone_width - 28.0).abs() < 1e-3);
    }

    #[test]
    fn includes_main_window_column_metrics() {
        let m = ThemeMetrics::default();
        assert!((m.column_header_height - 32.0).abs() < 1e-3);
        assert!((m.default_column_width - 280.0).abs() < 1e-3);
        assert!((m.min_column_width - 200.0).abs() < 1e-3);
        assert!((m.default_column_split_ratio - 0.5).abs() < 1e-3);
        assert!((m.column_tab_width - 104.0).abs() < 1e-3);
        assert!((m.splitter_thickness - 4.0).abs() < 1e-3);
        assert!((m.splitter_hit_zone - 8.0).abs() < 1e-3);
    }
}
