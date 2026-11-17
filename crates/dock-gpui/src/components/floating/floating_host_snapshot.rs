use std::collections::{HashMap, HashSet};

use dock_core::{
    ColumnDisplayItem, ColumnMode, DockMetrics, DragPreview, DropZoneRect, FloatingGroupRect,
    LayoutRects, NodeId,
};

use crate::{dock_components::GroupPresentation, input::ResizePreview};

pub(super) struct FloatingHostSnapshot {
    pub(super) rects: LayoutRects,
    pub(super) floating: FloatingGroupRect,
    pub(super) column_ids: HashSet<NodeId>,
    pub(super) column_groups: HashMap<NodeId, Vec<NodeId>>,
    pub(super) column_modes: HashMap<NodeId, ColumnMode>,
    pub(super) group_presentations: HashMap<NodeId, GroupPresentation>,
    pub(super) rail_display_orders: HashMap<NodeId, Vec<ColumnDisplayItem>>,
    pub(super) metrics: DockMetrics,
    pub(super) drag_preview: Option<DragPreview>,
    pub(super) drag_zones: Vec<DropZoneRect>,
    pub(super) active_drop_zone: Option<dock_core::DropZoneRect>,
    pub(super) resize_preview: Option<ResizePreview>,
}
