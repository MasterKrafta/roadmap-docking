//! Theme boundary for the docking adapter.
//!
//! The first integration keeps using the application's shared UI theme. This
//! alias gives DockHost and downstream code a stable dock-specific name while
//! dock-only colors, typography, radii, and icon tokens are extracted later.
pub use ui::Theme as DockTheme;

/// Converts shared UI theme layout values at the GPUI boundary.
///
/// dock-core stays independent from the UI crate. Metrics without an exact
/// ThemeMetrics counterpart retain DockMetrics' established defaults.
pub fn dock_metrics_from_theme(
    theme_metrics: ui::ThemeMetrics,
) -> Result<dock_core::DockMetrics, &'static str> {
    theme_metrics.validate()?;

    let metrics = dock_core::DockMetrics {
        default_column_width: theme_metrics.default_column_width,
        min_column_width: theme_metrics.min_column_width,
        rail_width: theme_metrics.rail_width,
        splitter_thickness: theme_metrics.splitter_thickness,
        splitter_hit_zone: theme_metrics.splitter_hit_zone,
        edge_drop_width: theme_metrics.drop_edge_zone,
        drag_threshold: theme_metrics.drag_start_threshold,
        min_group_height: theme_metrics.min_group_height,
        collapsed_group_height: theme_metrics.collapsed_group_height,
        column_header_height: theme_metrics.composition_header_height,
        header_height: theme_metrics.group_header_height,
        tab_strip_height: theme_metrics.tab_strip_height,
        group_control_button_width: theme_metrics.group_control_button_width,
        min_canvas_width: theme_metrics.min_document_width,
        max_columns: theme_metrics.max_columns,
        ..dock_core::DockMetrics::default()
    };
    metrics.validate()?;
    Ok(metrics)
}

#[cfg(test)]
mod tests {
    use super::dock_metrics_from_theme;

    #[test]
    fn maps_composition_group_and_tab_heights_to_separate_core_metrics() {
        let mut theme = ui::ThemeMetrics::default();
        theme.composition_header_height = 18.0;
        theme.group_header_height = 22.0;
        theme.tab_strip_height = 12.0;

        let dock = dock_metrics_from_theme(theme).expect("valid theme metrics");
        assert_eq!(dock.column_header_height, 18.0);
        assert_eq!(dock.header_height, 22.0);
        assert_eq!(dock.tab_strip_height, 12.0);
    }

    #[test]
    fn maps_shared_layout_metrics_and_preserves_unmapped_core_defaults() {
        let mut theme = ui::ThemeMetrics::default();
        theme.default_column_width = 310.0;
        theme.min_column_width = 210.0;
        theme.rail_width = 40.0;
        theme.splitter_thickness = 5.0;
        theme.splitter_hit_zone = 9.0;
        theme.drop_edge_zone = 26.0;
        theme.drag_start_threshold = 7.0;
        theme.min_group_height = 100.0;
        theme.collapsed_group_height = 30.0;
        theme.group_control_button_width = 36.0;
        theme.min_document_width = 340.0;
        theme.max_columns = 5;

        let dock = dock_metrics_from_theme(theme).expect("valid theme metrics");
        assert_eq!(dock.default_column_width, 310.0);
        assert_eq!(dock.min_column_width, 210.0);
        assert_eq!(dock.rail_width, 40.0);
        assert_eq!(dock.splitter_thickness, 5.0);
        assert_eq!(dock.splitter_hit_zone, 9.0);
        assert_eq!(dock.edge_drop_width, 26.0);
        assert_eq!(dock.drag_threshold, 7.0);
        assert_eq!(dock.min_group_height, 100.0);
        assert_eq!(dock.collapsed_group_height, 30.0);
        assert_eq!(dock.group_control_button_width, 36.0);
        assert_eq!(dock.min_canvas_width, 340.0);
        assert_eq!(dock.max_columns, 5);
        assert_eq!(dock.group_grip_width, dock_core::DockMetrics::default().group_grip_width);
    }

    #[test]
    fn rejects_theme_values_that_cannot_form_valid_core_metrics() {
        let mut theme = ui::ThemeMetrics::default();
        theme.group_header_height = 60.0;
        theme.tab_strip_height = 40.0;
        theme.min_group_height = 80.0;

        assert!(dock_metrics_from_theme(theme).is_err());
    }
}
