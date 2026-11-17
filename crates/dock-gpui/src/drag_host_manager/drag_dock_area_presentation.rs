use std::collections::{HashMap, HashSet};

use gpui::AnyView;

use dock_core::{
    ColumnDisplayItem, ColumnMode, ColumnRect, DockMetrics, DockSide, GroupRect, NodeId, PanelRect,
    SplitterRect, TabHeaderRect, TabOverflowRect, TabStripRect, VerticalOverflow,
};

use crate::dock_components::GroupPresentation;

pub(super) struct DragDockAreaPresentation {
    pub(super) columns: Vec<ColumnRect>,
    pub(super) groups: Vec<GroupRect>,
    pub(super) panels: Vec<(PanelRect, Option<AnyView>)>,
    pub(super) tab_strips: Vec<TabStripRect>,
    pub(super) tab_headers: Vec<TabHeaderRect>,
    pub(super) tab_overflows: Vec<TabOverflowRect>,
    pub(super) presentations: HashMap<NodeId, GroupPresentation>,
    pub(super) rail_display_orders: HashMap<NodeId, Vec<ColumnDisplayItem>>,
    pub(super) node_ids: HashSet<NodeId>,
    pub(super) node_sides: HashMap<NodeId, DockSide>,
    pub(super) column_sides: HashMap<NodeId, DockSide>,
    pub(super) column_groups: HashMap<NodeId, Vec<NodeId>>,
    pub(super) column_modes: HashMap<NodeId, ColumnMode>,
    pub(super) splitters: Vec<SplitterRect>,
    pub(super) vertical_overflow: Vec<VerticalOverflow>,
    pub(super) metrics: DockMetrics,
}
