use dock_core::{Axis, CanvasState, Column, ColumnMode, DockArea, DockMetrics, DockSide, FloatingGroup, Node, NodeId, PanelId, Point, Profile, Rect, Size, Split, Tabs};
use slotmap::SlotMap;

#[test]
fn photoshop_composition_keeps_canvas_separate_from_dock_and_floating_groups() {
    let mut panels: SlotMap<PanelId, ()> = SlotMap::with_key();
    let layers_panel = panels.insert(());
    let color_panel = panels.insert(());
    let floating_panel = panels.insert(());
    let mut nodes: SlotMap<NodeId, Node> = SlotMap::with_key();
    let layers = nodes.insert(Node::Tabs(Tabs { panels: vec![layers_panel], active_index: 0 }));
    let color = nodes.insert(Node::Tabs(Tabs { panels: vec![color_panel], active_index: 0 }));
    let dock = DockArea { side: DockSide::Right, columns: vec![Column {
        width: 280.0, mode: ColumnMode::Expanded, groups: vec![layers, color], group_ratios: vec![0.5, 0.5]
    }]};
    let canvas = CanvasState { bounds: Rect::new(Point::new(0.0, 0.0), Size::new(1200.0, 800.0)) };
    let floating = FloatingGroup { panels: vec![floating_panel], active_index: 0,
        bounds: Rect::new(Point::new(40.0, 50.0), Size::new(320.0, 240.0)), z_order: 1 };
    assert_eq!(dock.side, DockSide::Right);
    assert_eq!(dock.columns.len(), 1);
    assert_eq!(dock.columns[0].groups.len(), 2);
    assert!(matches!(nodes.get(dock.columns[0].groups[0]), Some(Node::Tabs(_))));
    assert!(matches!(nodes.get(dock.columns[0].groups[1]), Some(Node::Tabs(_))));
    assert!((canvas.bounds.size.width - 1200.0).abs() < 1e-3);
    assert!((canvas.bounds.size.height - 800.0).abs() < 1e-3);
    assert_eq!(floating.active_index, 0);
    assert_eq!(floating.z_order, 1);
}

#[test]
fn split_exposes_axis_children_and_proportions_for_generic_profiles() {
    let mut nodes: SlotMap<NodeId, Node> = SlotMap::with_key();
    let first = nodes.insert(Node::Tabs(Tabs { panels: vec![], active_index: 0 }));
    let second = nodes.insert(Node::Tabs(Tabs { panels: vec![], active_index: 0 }));
    let split = Split { axis: Axis::Vertical, children: vec![first, second], ratios: vec![0.5, 0.5] };
    let _split_node = nodes.insert(Node::Split(split.clone()));
    assert_eq!(split.axis, Axis::Vertical);
    assert_eq!(split.children.len(), 2);
    assert_eq!(split.ratios.len(), split.children.len());
    assert!((split.ratios[0] - 0.5).abs() < 1e-4);
}

#[test]
fn column_can_be_represented_with_a_single_group() {
    let mut nodes: SlotMap<NodeId, Node> = SlotMap::with_key();
    let group = nodes.insert(Node::Tabs(Tabs { panels: vec![], active_index: 0 }));
    let column = Column { width: 280.0, mode: ColumnMode::IconsOnly, groups: vec![group], group_ratios: vec![1.0] };
    assert_eq!(column.groups.len(), 1);
    assert_eq!(column.group_ratios.len(), 1);
}

#[test]
fn profile_remains_explicit_and_dock_metrics_are_plain_numeric_values() {
    let _profile = Profile::Photoshop;
    let metrics = DockMetrics { column_header_height: 32.0, tab_strip_height: 28.0,
        splitter_thickness: 4.0, splitter_hit_zone: 8.0, edge_drop_zone_width: 24.0,
        drag_start_threshold: 4.0, default_column_width: 280.0, min_column_width: 200.0,
        rail_width: 36.0, min_group_height: 80.0, collapsed_group_height: 28.0,
        min_document_width: 320.0, max_columns: 4 };
    assert!(metrics.validate().is_ok());
}
