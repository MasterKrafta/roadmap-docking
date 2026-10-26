use super::{assert_tabs, bounds, constraints, floating_root, key, tab_parent_of};
use crate::layout_builder::LayoutBuilder;
use crate::{
    Axis, ColumnMode, DockCommand, DockMetrics, DockSide, DropTarget, DropZone, Node, Profile,
    Tabs,
};

#[test]
fn panel_tab_reorders_inside_tabs_without_becoming_a_group_drag() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);
    let (first_panel, first_node) = builder.panel(key("reorder-first"), constraints());
    let (_, second_node) = builder.panel(key("reorder-second"), constraints());
    let (_, third_node) = builder.panel(key("reorder-third"), constraints());
    let group = builder.tabs(vec![first_node, second_node, third_node], 2, false);
    let column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        vec![group],
        vec![1.0],
    );
    builder.main_area(DockSide::Left, column);
    let mut layout = builder.build();

    layout
        .apply(
            DockCommand::MovePanel {
                panel: first_panel,
                target: DropTarget::Tabs { group, index: 3 },
            },
            &metrics,
        )
        .unwrap();

    let Node::Tabs(tabs) = layout.node(group).unwrap() else {
        panic!("reordering must preserve the Tabs group");
    };
    assert_eq!(tabs.children, vec![second_node, third_node, first_node]);
    assert_eq!(tabs.active_index, 1);
    assert_eq!(tabs.children[tabs.active_index], third_node);
    assert_eq!(layout.validate(&metrics), Ok(()));
}

#[test]
fn panel_tab_joins_another_tabs_group_and_only_that_panel_moves() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);
    let (moving_panel, moving_node) = builder.panel(key("join-moving"), constraints());
    let (_, source_keep_node) = builder.panel(key("join-keep"), constraints());
    let source = builder.tabs(vec![moving_node, source_keep_node], 0, true);
    let (_, target_a) = builder.panel(key("join-target-a"), constraints());
    let (_, target_b) = builder.panel(key("join-target-b"), constraints());
    let target = builder.tabs(vec![target_a, target_b], 0, false);
    let column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        vec![source, target],
        vec![0.5, 0.5],
    );
    builder.main_area(DockSide::Left, column);
    let mut layout = builder.build();

    layout
        .apply(
            DockCommand::MovePanel {
                panel: moving_panel,
                target: DropTarget::Tabs {
                    group: target,
                    index: 1,
                },
            },
            &metrics,
        )
        .unwrap();

    assert_tabs(
        &layout,
        source,
        &Tabs {
            children: vec![source_keep_node],
            active_index: 0,
            collapsed: true,
        },
    );
    assert_tabs(
        &layout,
        target,
        &Tabs {
            children: vec![target_a, moving_node, target_b],
            active_index: 1,
            collapsed: false,
        },
    );
    assert_eq!(layout.panel_instances().count(), 4);
    assert_eq!(layout.validate(&metrics), Ok(()));
}

#[test]
fn panel_tab_can_split_into_a_new_group() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);
    let (moving_panel, moving_node) = builder.panel(key("split-panel"), constraints());
    let (_, source_keep) = builder.panel(key("split-source-keep"), constraints());
    let source_group = builder.tabs(vec![moving_node, source_keep], 0, false);
    let left_column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        vec![source_group],
        vec![1.0],
    );
    builder.main_area(DockSide::Left, left_column);

    let (_, target_node) = builder.panel(key("split-target"), constraints());
    let target_group = builder.tabs(vec![target_node], 0, false);
    let right_column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        vec![target_group],
        vec![1.0],
    );
    builder.main_area(DockSide::Right, right_column);
    let mut layout = builder.build();

    layout
        .apply(
            DockCommand::MovePanel {
                panel: moving_panel,
                target: DropTarget::GroupEdge {
                    group: target_group,
                    zone: DropZone::Top,
                },
            },
            &metrics,
        )
        .unwrap();

    let new_group =
        tab_parent_of(&layout, moving_node).expect("moved panel is wrapped as a new group");
    assert_ne!(new_group, source_group);
    assert_ne!(new_group, target_group);
    assert_tabs(
        &layout,
        new_group,
        &Tabs {
            children: vec![moving_node],
            active_index: 0,
            collapsed: false,
        },
    );
    let Node::Column(column) = layout.node(right_column).unwrap() else {
        panic!("right dock root remains a column for a top/bottom split");
    };
    assert_eq!(column.groups, vec![new_group, target_group]);
    assert_eq!(layout.validate(&metrics), Ok(()));
}

#[test]
fn panel_tab_can_become_a_new_main_dock_column() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);
    let (moving_panel, moving_node) = builder.panel(key("column-panel"), constraints());
    let (_, source_keep) = builder.panel(key("column-source-keep"), constraints());
    let source_group = builder.tabs(vec![moving_node, source_keep], 0, false);
    let left_column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        vec![source_group],
        vec![1.0],
    );
    builder.main_area(DockSide::Left, left_column);

    let (_, target_node) = builder.panel(key("column-target"), constraints());
    let target_group = builder.tabs(vec![target_node], 0, false);
    let target_column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        vec![target_group],
        vec![1.0],
    );
    builder.main_area(DockSide::Right, target_column);
    let mut layout = builder.build();

    layout
        .apply(
            DockCommand::MovePanel {
                panel: moving_panel,
                target: DropTarget::MainColumn {
                    side: DockSide::Right,
                    index: 0,
                },
            },
            &metrics,
        )
        .unwrap();

    let Some(Node::Split(split)) = layout
        .main_area(DockSide::Right)
        .and_then(|area| layout.node(area.root))
    else {
        panic!("adding a new column beside an existing one forms a horizontal split");
    };
    assert_eq!(split.axis, Axis::X);
    assert_eq!(split.children.len(), 2);
    let Node::Column(new_column) = layout.node(split.children[0]).unwrap() else {
        panic!("the inserted root child must be a column");
    };
    let new_group = new_column.groups[0];
    assert_tabs(
        &layout,
        new_group,
        &Tabs {
            children: vec![moving_node],
            active_index: 0,
            collapsed: false,
        },
    );
    assert_eq!(layout.panel_instances().count(), 3);
    assert_eq!(layout.validate(&metrics), Ok(()));
}

#[test]
fn panel_tab_can_float_without_detaching_its_sibling_tabs() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);
    let (moving_panel, moving_node) = builder.panel(key("float-panel"), constraints());
    let (_, keep_node) = builder.panel(key("float-keep"), constraints());
    let source_group = builder.tabs(vec![moving_node, keep_node], 1, true);
    let column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        vec![source_group],
        vec![1.0],
    );
    builder.main_area(DockSide::Left, column);
    let mut layout = builder.build();

    layout
        .apply(
            DockCommand::Float {
                node: moving_node,
                bounds: bounds(100.0, 120.0),
            },
            &metrics,
        )
        .unwrap();

    assert_tabs(
        &layout,
        source_group,
        &Tabs {
            children: vec![keep_node],
            active_index: 0,
            collapsed: true,
        },
    );
    assert_eq!(layout.floating_groups().count(), 1);
    let (floating_id, _) = layout.floating_groups().next().unwrap();
    let root = floating_root(&layout, floating_id);
    let Node::Tabs(float_tabs) = layout.node(root).unwrap() else {
        panic!("Photoshop floating panel roots are wrapped in Tabs");
    };
    assert_eq!(float_tabs.children, vec![moving_node]);
    assert!(
        matches!(layout.node(moving_node), Some(Node::Panel(panel)) if panel.panel_id == moving_panel)
    );
    assert_eq!(layout.validate(&metrics), Ok(()));
}

#[test]
fn panel_tab_can_move_between_floating_groups() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);
    let (moving_panel, moving_node) = builder.panel(key("float-source-moving"), constraints());
    let (_, source_keep) = builder.panel(key("float-source-keep"), constraints());
    let source_group = builder.tabs(vec![moving_node, source_keep], 0, true);
    let source_float = builder.floating_group(source_group, bounds(80.0, 90.0), 0);

    let (_, target_node) = builder.panel(key("float-destination"), constraints());
    let target_group = builder.tabs(vec![target_node], 0, false);
    let target_float = builder.floating_group(target_group, bounds(560.0, 140.0), 1);
    let mut layout = builder.build();

    layout
        .apply(
            DockCommand::MovePanel {
                panel: moving_panel,
                target: DropTarget::Tabs {
                    group: target_group,
                    index: 1,
                },
            },
            &metrics,
        )
        .unwrap();

    assert_tabs(
        &layout,
        source_group,
        &Tabs {
            children: vec![source_keep],
            active_index: 0,
            collapsed: true,
        },
    );
    assert_tabs(
        &layout,
        target_group,
        &Tabs {
            children: vec![target_node, moving_node],
            active_index: 1,
            collapsed: false,
        },
    );
    assert_eq!(layout.floating_groups().count(), 2);
    assert_eq!(floating_root(&layout, source_float), source_group);
    assert_eq!(floating_root(&layout, target_float), target_group);
    assert_eq!(layout.validate(&metrics), Ok(()));
}
