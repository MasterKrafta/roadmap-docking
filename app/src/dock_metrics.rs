use dock_core::DockMetrics;
use ui::Theme;

/// Convert UI-owned theme metrics into plain numeric core metrics.
///
/// Keeping this conversion in the app boundary prevents dock-core from depending on
/// Theme or UI code. The returned value is passed to the pure geometry functions.
pub(crate) fn build(theme: &Theme) -> DockMetrics {
    let m = theme.metrics;
    DockMetrics {
        column_header_height: m.column_header_height,
        tab_strip_height: m.tab_strip_height,
        splitter_thickness: m.splitter_thickness,
        splitter_hit_zone: m.splitter_hit_zone,
        edge_drop_zone_width: m.edge_drop_zone_width,
        drop_edge_zone: m.drop_edge_zone,
        drag_start_threshold: m.drag_start_threshold,
        default_column_width: m.default_column_width,
        min_column_width: m.min_column_width,
        rail_width: m.rail_width,
        min_group_height: m.min_group_height,
        collapsed_group_height: m.collapsed_group_height,
        min_document_width: m.min_document_width,
        max_columns: m.max_columns,
    }
}

#[cfg(test)]
mod tests {
    use super::build;
    use dock_core::{compute_rects, CanvasState, DockSide, Layout, Profile, Size};
    use ui::Theme;

    #[test]
    fn theme_metrics_are_passed_through_to_core_geometry() {
        let theme = Theme::default();
        let metrics = build(&theme);
        assert_eq!(metrics.column_header_height, theme.metrics.column_header_height);
        assert_eq!(metrics.tab_strip_height, theme.metrics.tab_strip_height);
        assert_eq!(metrics.splitter_thickness, theme.metrics.splitter_thickness);
        assert_eq!(metrics.splitter_hit_zone, theme.metrics.splitter_hit_zone);
        assert_eq!(metrics.rail_width, theme.metrics.rail_width);
        assert_eq!(metrics.drop_edge_zone, theme.metrics.drop_edge_zone);

        let layout = Layout::new(Profile::Photoshop, CanvasState::default());
        let rects = compute_rects(&layout, Size::new(1000.0, 600.0), &metrics);
        assert_eq!(rects.canvas.size, Size::new(1000.0, 600.0));
        assert!(rects.dock_areas.is_empty());
    }

    #[test]
    fn geometry_width_matches_default_registered_right_dock_width() {
        let theme = Theme::default();
        let metrics = build(&theme);
        let registry = crate::panels::PanelRegistry::with_builtins()
            .expect("built-in panel descriptors are valid");
        let layout = registry
            .build_initial_photoshop_layout(&metrics)
            .expect("built-in panels form a valid model layout");
        assert!(
            layout.panels().values().all(|panel| panel.key().as_str() != "toolbar"),
            "Toolbar is fixed app chrome and must not be part of the dock panel model"
        );

        let rects = compute_rects(&layout, Size::new(1000.0, 600.0), &metrics);
        let right_dock = rects
            .dock_areas
            .iter()
            .find(|area| area.side == DockSide::Right)
            .expect("the initial Photoshop layout has a right dock");
        assert_eq!(right_dock.bounds.size.width, theme.metrics.default_column_width);
        assert_eq!(
            rects.canvas.size.width + right_dock.bounds.size.width,
            1000.0
        );
    }
}
