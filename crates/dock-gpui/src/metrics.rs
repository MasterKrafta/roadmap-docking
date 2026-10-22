use dock_core::DockMetrics;
use ui::Theme;

/// Translates the app's single theme metrics object into the Core geometry contract.
///
/// Metrics which are specific to Core tab overflow keep their Core defaults because
/// ThemeMetrics does not currently expose corresponding presentation settings.
pub fn dock_metrics_from_theme(theme: &Theme) -> DockMetrics {
    let source = theme.metrics;
    DockMetrics {
        default_column_width: source.default_column_width,
        min_column_width: source.min_column_width,
        rail_width: source.rail_width,
        splitter_thickness: source.splitter_thickness,
        splitter_hit_zone: source.splitter_hit_zone,
        edge_drop_width: source.drop_edge_zone,
        drag_threshold: source.drag_start_threshold,
        min_group_height: source.min_group_height,
        collapsed_group_height: source.collapsed_group_height,
        header_height: source.column_header_height,
        tab_strip_height: source.tab_strip_height,
        min_canvas_width: source.min_document_width,
        max_columns: source.max_columns,
        ..DockMetrics::default()
    }
}

#[cfg(test)]
mod tests {
    use super::dock_metrics_from_theme;
    use dock_core::DockMetrics;
    use ui::Theme;

    #[test]
    fn maps_shared_theme_metrics_without_redefining_core_defaults() {
        let mut theme = Theme::default();
        theme.metrics.default_column_width = 320.0;
        theme.metrics.drop_edge_zone = 26.0;
        theme.metrics.column_header_height = 34.0;
        theme.metrics.min_document_width = 360.0;

        let metrics = dock_metrics_from_theme(&theme);
        assert_eq!(metrics.default_column_width, 320.0);
        assert_eq!(metrics.edge_drop_width, 26.0);
        assert_eq!(metrics.header_height, 34.0);
        assert_eq!(metrics.min_canvas_width, 360.0);
        assert_eq!(
            metrics.min_tab_width,
            DockMetrics::default().min_tab_width
        );
        assert_eq!(
            metrics.tab_overflow_button_width,
            DockMetrics::default().tab_overflow_button_width
        );
        assert_eq!(metrics.validate(), Ok(()));
    }
}
