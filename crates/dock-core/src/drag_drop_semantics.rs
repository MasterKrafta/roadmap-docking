use crate::{
    FloatingGroupId, Layout, Node, NodeId, PanelConstraints, PanelKey, Point, Rect, Size, Tabs,
};

fn key(value: &str) -> PanelKey {
    PanelKey::new(value).expect("test panel key must be valid")
}

fn constraints() -> PanelConstraints {
    PanelConstraints::new(Size::new(80.0, 60.0), Size::new(180.0, 140.0), None, false)
}

fn bounds(x: f32, y: f32) -> Rect {
    Rect::new(Point::new(x, y), Size::new(420.0, 320.0))
}

fn assert_tabs(layout: &Layout, group: NodeId, expected: &Tabs) {
    let Some(Node::Tabs(actual)) = layout.node(group) else {
        panic!("expected group {group:?} to remain a Tabs node");
    };
    assert_eq!(actual, expected);
}

fn tab_parent_of(layout: &Layout, panel_node: NodeId) -> Option<NodeId> {
    layout.nodes().find_map(|(id, node)| match node {
        Node::Tabs(tabs) if tabs.children.contains(&panel_node) => Some(id),
        _ => None,
    })
}

fn floating_root(layout: &Layout, id: FloatingGroupId) -> NodeId {
    let floating = layout.floating_group(id).expect("floating group exists");
    layout
        .dock_area(floating.area)
        .expect("floating area exists")
        .root
}

mod group_grip;
mod panel_tab;
