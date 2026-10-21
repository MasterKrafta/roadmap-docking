//! C7.1 regression coverage for CollapseGroup semantics and geometry.

use crate::layout_builder::LayoutBuilder;
use crate::{
    ColumnMode, DockCommand, DockMetrics, DockSide, Layout, Node, NodeId, PanelConstraints,
    PanelId, PanelKey, Point, Profile, Rect, Size,
};

fn constraints() -> PanelConstraints {
    PanelConstraints::new(
        Size::new(80.0, 60.0),
        Size::new(180.0, 140.0),
        Some(Size::new(800.0, 600.0)),
        false,
    )
}

fn key(value: &str) -> PanelKey {
    PanelKey::new(value).unwrap()
}

fn viewport(width: f32, height: f32) -> Rect {
    Rect::new(Point::ZERO, Size::new(width, height))
}

fn layout_with_tabs() -> (Layout, NodeId, NodeId, PanelId, PanelId, PanelId) {
    let mut builder = LayoutBuilder::new(Profile::Photoshop);

    let (first_panel, first_node) = builder.panel(key("collapse-first"), constraints());
    let (active_panel, active_node) = builder.panel(key("collapse-active"), constraints());
    let (other_panel, other_node) = builder.panel(key("collapse-other"), constraints());

    let group = builder.tabs(vec![first_node, active_node], 1, false);
    let other_group = builder.tabs(vec![other_node], 0, false);
    let column = builder.column(
        280.0,
        ColumnMode::Expanded,
        vec![group, other_group],
        vec![0.25, 0.75],
    );
    builder.main_area(DockSide::Left, column);

    (builder.build(), group, column, first_panel, active_panel, other_panel)
}

#[test]
fn collapse_and_expand_preserve_tabs_active_index_and_saved_group_ratios() {
    let metrics = DockMetrics::default();
    let (mut layout, group, column, _, active_panel, _) = layout_with_tabs();

    let (children_before, active_index_before) = match layout.node(group).unwrap() {
        Node::Tabs(tabs) => (tabs.children.clone(), tabs.active_index),
        other => panic!("expected Tabs group, found {other:?}"),
    };
    assert_eq!(active_index_before, 1);
    assert_eq!(
        match layout.node(column).unwrap() {
            Node::Column(column) => column.group_ratios.clone(),
            other => panic!("expected Column, found {other:?}"),
        },
        vec![0.25, 0.75],
    );

    let version_before = layout.version();
    let collapsed = layout
        .apply(
            DockCommand::CollapseGroup {
                group,
                collapsed: true,
            },
            &metrics,
        )
        .unwrap();

    assert_eq!(collapsed.new_version, version_before + 1);
    assert_eq!(
        collapsed.inverse,
        Some(DockCommand::CollapseGroup {
            group,
            collapsed: false,
        })
    );
    match layout.node(group).unwrap() {
        Node::Tabs(tabs) => {
            assert!(tabs.collapsed);
            assert_eq!(tabs.children, children_before);
            assert_eq!(tabs.active_index, active_index_before);
            assert_eq!(tabs.active_child(), Some(children_before[active_index_before]));
        }
        other => panic!("expected Tabs group, found {other:?}"),
    }
    assert_eq!(
        match layout.node(column).unwrap() {
            Node::Column(column) => column.group_ratios.clone(),
            other => panic!("expected Column, found {other:?}"),
        },
        vec![0.25, 0.75],
        "collapsing must not overwrite the last expanded ratio distribution",
    );

    let collapsed_version = layout.version();
    let repeated = layout
        .apply(
            DockCommand::CollapseGroup {
                group,
                collapsed: true,
            },
            &metrics,
        )
        .unwrap();
    assert_eq!(repeated.new_version, collapsed_version);
    assert_eq!(layout.version(), collapsed_version);
    assert!(repeated.inverse.is_none());

    let expanded = layout
        .apply(
            DockCommand::CollapseGroup {
                group,
                collapsed: false,
            },
            &metrics,
        )
        .unwrap();
    assert_eq!(expanded.new_version, collapsed_version + 1);
    assert_eq!(
        expanded.inverse,
        Some(DockCommand::CollapseGroup {
            group,
            collapsed: true,
        })
    );
    match layout.node(group).unwrap() {
        Node::Tabs(tabs) => {
            assert!(!tabs.collapsed);
            assert_eq!(tabs.children, children_before);
            assert_eq!(tabs.active_index, active_index_before);
            assert_eq!(tabs.active_child(), Some(children_before[active_index_before]));
        }
        other => panic!("expected Tabs group, found {other:?}"),
    }
    assert_eq!(
        match layout.node(column).unwrap() {
            Node::Column(column) => column.group_ratios.clone(),
            other => panic!("expected Column, found {other:?}"),
        },
        vec![0.25, 0.75],
    );
    assert!(layout.node(children_before[active_index_before]).is_some());
    assert!(matches!(
        layout.node(children_before[active_index_before]),
        Some(Node::Panel(panel)) if panel.panel_id == active_panel
    ));
}

#[test]
fn collapsed_geometry_keeps_tab_strip_and_headers_but_hides_panel_content() {
    let metrics = DockMetrics::default();
    let (mut layout, group, _, first_panel, active_panel, other_panel) = layout_with_tabs();
    let viewport = viewport(1200.0, 800.0);

    let expanded_rects = layout.compute_rects(viewport, &metrics).unwrap();
    assert!(expanded_rects.panels.iter().any(|panel| panel.panel == active_panel));

    let children_before = match layout.node(group).unwrap() {
        Node::Tabs(tabs) => tabs.children.clone(),
        other => panic!("expected Tabs group, found {other:?}"),
    };

    layout
        .apply(
            DockCommand::CollapseGroup {
                group,
                collapsed: true,
            },
            &metrics,
        )
        .unwrap();

    let rects = layout.compute_rects(viewport, &metrics).unwrap();
    let group_rect = rects.groups.iter().find(|item| item.group == group).unwrap().rect;
    let strip = rects.tab_strips.iter().find(|item| item.group == group).unwrap();
    let collapsed_height = metrics.collapsed_group_height.max(metrics.tab_strip_height);

    assert_eq!(group_rect.size.height, collapsed_height);
    assert_eq!(strip.rect, group_rect);
    let headers: Vec<_> = rects
        .tab_headers
        .iter()
        .filter(|header| header.group == group)
        .map(|header| (header.panel, header.index))
        .collect();
    assert_eq!(headers, vec![(first_panel, 0), (active_panel, 1)]);

    assert!(
        rects.panels.iter().all(|panel| panel.panel != first_panel && panel.panel != active_panel),
        "collapsed tabs must not emit visible content rectangles for their child panels",
    );
    assert!(rects.panels.iter().any(|panel| panel.panel == other_panel));

    match layout.node(group).unwrap() {
        Node::Tabs(tabs) => {
            assert!(tabs.collapsed);
            assert_eq!(tabs.children, children_before);
            assert_eq!(tabs.active_index, 1);
        }
        other => panic!("expected Tabs group, found {other:?}"),
    }
}
