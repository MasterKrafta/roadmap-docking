use dock_core::DockMetrics;
use ui::Theme;

/// Convert UI-owned theme metrics into plain numeric core metrics.
/// The core crate intentionally has no dependency on `ui` or GPUI.
pub(crate) fn build(theme: &Theme) -> DockMetrics {
    let metrics = theme.metrics;
    DockMetrics {
        column_header_height: metrics.column_header_height,
        tab_strip_height: metrics.tab_strip_height,
        splitter_thickness: metrics.splitter_thickness,
        splitter_hit_zone: metrics.splitter_hit_zone,
        default_column_width: metrics.default_column_width,
        min_column_width: metrics.min_column_width,
        rail_width: metrics.rail_width,
        min_group_height: metrics.min_group_height,
        collapsed_group_height: metrics.collapsed_group_height,
        min_document_width: metrics.min_document_width,
        max_columns: metrics.max_columns,
    }
}
