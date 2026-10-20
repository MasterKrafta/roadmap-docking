use dock_core::DockMetrics;
use ui::Theme;
/// Convert UI-owned theme metrics into plain numeric core metrics.
pub(crate) fn build(theme: &Theme) -> DockMetrics {
    let m = theme.metrics;
    DockMetrics { column_header_height: m.column_header_height, tab_strip_height: m.tab_strip_height,
        splitter_thickness: m.splitter_thickness, splitter_hit_zone: m.splitter_hit_zone,
        edge_drop_zone_width: m.edge_drop_zone_width, drop_edge_zone: m.drop_edge_zone,
        drag_start_threshold: m.drag_start_threshold,
        default_column_width: m.default_column_width, min_column_width: m.min_column_width,
        rail_width: m.rail_width, min_group_height: m.min_group_height,
        collapsed_group_height: m.collapsed_group_height, min_document_width: m.min_document_width, max_columns: m.max_columns }
}
