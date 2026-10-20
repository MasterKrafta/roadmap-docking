mod column_rect;
mod compute_rects;
mod dock_area_rect;
mod dock_metrics;
mod drop_zone_rect;
mod drop_payload;
mod drop_zones;
mod floating_group_rect;
mod group_rect;
mod hit_result;
mod hit_test;
mod layout_rects;
mod pixel_snap;
mod point;
mod rect;
mod resize_edge;
mod size;
mod splitter_kind;
mod tab_strip_rect;

pub use column_rect::ColumnRect;
pub use compute_rects::compute_rects;
pub use dock_area_rect::DockAreaRect;
pub use dock_metrics::DockMetrics;
pub use drop_zone_rect::DropZoneRect;
pub use drop_payload::DropPayload;
pub use drop_zones::{compute_drop_zones, resolve_drop};
pub use floating_group_rect::FloatingGroupRect;
pub use group_rect::GroupRect;
pub use hit_result::HitResult;
pub use hit_test::hit_test;
pub use layout_rects::{
    ColumnHeaderRect, FloatingResizeEdgeRect, FloatingTitleRect, LayoutRects, PanelRect,
    SplitterRect, TabHeaderRect,
};
pub use pixel_snap::{snap_logical_to_physical_pixel, snap_rect_to_physical_pixels};
pub use point::Point;
pub use rect::Rect;
pub use resize_edge::ResizeEdge;
pub use size::Size;
pub use splitter_kind::SplitterKind;
pub use tab_strip_rect::TabStripRect;
