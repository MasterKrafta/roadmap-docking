use dock_core::{
    ColumnRef, DockCommand, DockMetrics, DockSide, DropPayload, DropTarget, DropZone,
    FloatSource, Layout, Node, PanelId, Point, Rect, Size, SplitSide, compute_drop_zones,
    compute_rects, resolve_drop,
};

fn metrics() -> DockMetrics {
    DockMetrics {
        column_header_height: 32.0,
        tab_strip_height: 28.0,
        splitter_thickness: 4.0,
        splitter_hit_zone: 8.0,
        edge_drop_zone_width: 24.0,
        drop_edge_zone: 24.0,
        drag_start_threshold: 4.0,
        default_column_width: 280.0,
        min_column_width: 200.0,
        rail_width: 36.0,
        min_group_height: 80.0,
        collapsed_group_height: 28.0,
        min_document_width: 320.0,
        max_columns: 4,
    }
}

fn initial_layout(metrics: &DockMetrics) -> Layout {
    Layout::initial_photoshop(metrics).expect("initial Photoshop layout is valid")
}

fn panel_id(layout: &Layout, key: &str) -> PanelId {
    layout
        .panels()
        .iter()
        .find_map(|(id, panel)| (panel.key().as_str() == key).then_some(id))
        .unwrap_or_else(|| panic!("panel {key} exists"))
}

fn group_for_panel(layout: &Layout, panel: PanelId) -> dock_core::NodeId {
    layout
        .dock_areas()
        .iter()
        .flat_map(|area| area.columns.iter())
        .flat_map(|column| column.groups.iter().copied())
        .find(|group| {
            matches!(
                layout.nodes().get(*group),
                Some(Node::Tabs(tabs)) if tabs.panels.contains(&panel)
            )
        })
        .expect("docked panel group exists")
}

fn group_containing_key(layout: &Layout, key: &str) -> dock_core::NodeId {
    group_for_panel(layout, panel_id(layout, key))
}

/// Moves History into a second column so both horizontal column edges are real targets.
fn layout_with_two_columns(metrics: &DockMetrics) -> Layout {
    let mut layout = initial_layout(metrics);
    let history = panel_id(&layout, "history");
    let group = group_for_panel(&layout, history);
    layout
        .apply(
            DockCommand::SplitGroup {
                panel: history,
                group,
                side: SplitSide::Right,
            },
            metrics,
        )
        .expect("History can be split into a neighboring column");
    assert_eq!(
        layout
            .dock_areas()
            .iter()
            .find(|area| area.side == DockSide::Right)
            .expect("right dock area exists")
            .columns
            .len(),
        2
    );
    layout
}

fn panel_placement_count(layout: &Layout, panel: PanelId) -> usize {
    let in_dock = layout
        .dock_areas()
        .iter()
        .flat_map(|area| area.columns.iter())
        .flat_map(|column| column.groups.iter())
        .filter_map(|group| match layout.nodes().get(*group) {
            Some(Node::Tabs(tabs)) => Some(tabs.panels.iter().filter(|id| **id == panel).count()),
            Some(Node::Split(_)) | None => None,
        })
        .sum::<usize>();
    let in_floating = layout
        .floating_groups()
        .values()
        .map(|floating| floating.panels.iter().filter(|id| **id == panel).count())
        .sum::<usize>();
    in_dock + in_floating
}

fn column_count(layout: &Layout, side: DockSide) -> usize {
    layout
        .dock_areas()
        .iter()
        .find(|area| area.side == side)
        .map_or(0, |area| area.columns.len())
}

fn zone_center(bounds: Rect) -> Point {
    Point::new(
        bounds.origin.x + bounds.size.width * 0.5,
        bounds.origin.y + bounds.size.height * 0.5,
    )
}

fn center_of_zone(
    zones: &[dock_core::DropZoneRect],
    target: &DropTarget,
) -> Point {
    let zone = zones
        .iter()
        .find(|zone| &zone.target == target)
        .unwrap_or_else(|| panic!("expected drop zone {target:?}"));
    zone_center(zone.bounds)
}

#[test]
fn center_top_bottom_left_and_right_drop_targets_keep_photoshop_semantics() {
    let metrics = metrics();
    let layout = layout_with_two_columns(&metrics);
    let source = panel_id(&layout, "history");
    let source_group = group_for_panel(&layout, source);
    let target_group = group_containing_key(&layout, "layers");
    let lower_group = group_containing_key(&layout, "properties");
    let rects = compute_rects(&layout, Size::new(1280.0, 800.0), &metrics);
    let payload = DropPayload::Panel(source);
    let zones = compute_drop_zones(&layout, &rects, payload, &metrics);

    assert!(
        zones.iter().all(|zone| !matches!(
            &zone.target,
            DropTarget::Group {
                zone: DropZone::Left | DropZone::Right,
                ..
            }
        )),
        "Photoshop groups cannot accept horizontal sibling drops"
    );

    let targets = [
        (
            DropTarget::Group {
                group: target_group,
                zone: DropZone::Center,
            },
            "center inserts a tab",
        ),
        (
            DropTarget::Group {
                group: lower_group,
                zone: DropZone::Top,
            },
            "top creates an adjacent group",
        ),
        (
            DropTarget::Group {
                group: target_group,
                zone: DropZone::Bottom,
            },
            "bottom creates an adjacent group",
        ),
        (
            DropTarget::Column {
                column: ColumnRef::new(DockSide::Right, 0),
                zone: DropZone::Left,
            },
            "left creates a neighboring column",
        ),
        (
            DropTarget::Column {
                column: ColumnRef::new(DockSide::Right, 0),
                zone: DropZone::Right,
            },
            "right creates a neighboring column",
        ),
    ];

    for (target, description) in targets {
        let point = center_of_zone(&zones, &target);
        assert_eq!(
            resolve_drop(&layout, point, payload, &rects, &metrics),
            Some(target.clone()),
            "hit-testing must select the expected semantic target: {description}"
        );

        let mut candidate = layout.clone();
        candidate
            .apply(
                DockCommand::MovePanel {
                    panel: source,
                    target: target.clone(),
                },
                &metrics,
            )
            .unwrap_or_else(|error| panic!("{description}: {error}"));

        assert!(candidate.validate(&metrics).is_ok(), "{description}");
        assert_eq!(
            panel_placement_count(&candidate, source),
            1,
            "a drop must preserve exactly one instance of History: {description}"
        );

        match target {
            DropTarget::Group {
                zone: DropZone::Center,
                group,
            } => {
                let Node::Tabs(tabs) = candidate.nodes().get(group).expect("target group remains")
                else {
                    panic!("center target must remain a tab group");
                };
                assert!(tabs.panels.contains(&source), "center inserts a tab");
                assert_eq!(
                    tabs.panels[tabs.active_index], source,
                    "the transferred tab becomes active"
                );
            }
            DropTarget::Group {
                zone: DropZone::Top | DropZone::Bottom,
                group,
            } => {
                let incoming_group = group_for_panel(&candidate, source);
                assert_ne!(incoming_group, group, "side drop creates a distinct group");
                assert_ne!(incoming_group, source_group, "empty source group is removed");
            }
            DropTarget::Column {
                column: ColumnRef { side, .. },
                zone: DropZone::Left | DropZone::Right,
            } => {
                assert_eq!(column_count(&candidate, side), 2);
                let incoming_group = group_for_panel(&candidate, source);
                let right_area = candidate
                    .dock_areas()
                    .iter()
                    .find(|area| area.side == side)
                    .expect("target dock area remains");
                assert!(
                    right_area.columns.iter().any(|column| column.groups.contains(&incoming_group)),
                    "lateral drop must place History in a column"
                );
            }
            _ => unreachable!("test covers exactly the five intended targets"),
        }
    }
}

#[test]
fn horizontal_group_drop_is_rejected_without_mutating_the_layout() {
    let metrics = metrics();
    let mut layout = layout_with_two_columns(&metrics);
    let panel = panel_id(&layout, "history");
    let target_group = group_containing_key(&layout, "layers");
    let before = format!("{layout:?}");
    let version_before = layout.version();

    assert!(
        layout
            .apply(
                DockCommand::MovePanel {
                    panel,
                    target: DropTarget::Group {
                        group: target_group,
                        zone: DropZone::Left,
                    },
                },
                &metrics,
            )
            .is_err(),
        "horizontal neighbors of a group are forbidden by the Photoshop profile"
    );
    assert_eq!(format!("{layout:?}"), before);
    assert_eq!(layout.version(), version_before);
    assert!(layout.validate(&metrics).is_ok());
    assert_eq!(panel_placement_count(&layout, panel), 1);
}

#[test]
fn transferring_the_last_panel_removes_its_empty_group_and_column() {
    let metrics = metrics();
    let mut layout = layout_with_two_columns(&metrics);
    let panel = panel_id(&layout, "history");
    let source_group = group_for_panel(&layout, panel);
    let target_group = group_containing_key(&layout, "layers");
    assert_eq!(column_count(&layout, DockSide::Right), 2);

    layout
        .apply(
            DockCommand::MovePanel {
                panel,
                target: DropTarget::Group {
                    group: target_group,
                    zone: DropZone::Center,
                },
            },
            &metrics,
        )
        .expect("last tab can move into the existing target group");

    assert!(
        layout.nodes().get(source_group).is_none(),
        "empty source group is removed after a successful transfer"
    );
    assert_eq!(column_count(&layout, DockSide::Right), 1);
    assert_eq!(panel_placement_count(&layout, panel), 1);
    assert!(matches!(
        layout.nodes().get(target_group),
        Some(Node::Tabs(tabs)) if tabs.panels.contains(&panel)
    ));
    assert!(layout.validate(&metrics).is_ok());
}

#[test]
fn floating_move_resize_and_dock_preserve_the_same_panel_instance() {
    let metrics = metrics();
    let mut layout = initial_layout(&metrics);
    let panel = panel_id(&layout, "history");
    let target_group = group_containing_key(&layout, "layers");
    let floating_bounds = Rect::new(Point::new(320.0, 120.0), Size::new(320.0, 240.0));

    layout
        .apply(
            DockCommand::Float {
                source: FloatSource::Panel(panel),
                rect: floating_bounds,
            },
            &metrics,
        )
        .expect("panel can detach into an internal floating group");
    let floating = layout
        .floating_groups()
        .iter()
        .find_map(|(id, group)| group.panels.contains(&panel).then_some(id))
        .expect("floating source exists");
    assert_eq!(panel_placement_count(&layout, panel), 1);

    let moved_bounds = Rect::new(Point::new(350.0, 140.0), Size::new(320.0, 240.0));
    layout
        .apply(
            DockCommand::MoveFloating {
                floating,
                rect: moved_bounds,
            },
            &metrics,
        )
        .expect("floating group can move");
    layout
        .apply(
            DockCommand::ResizeFloating {
                floating,
                rect: Rect::new(Point::new(350.0, 140.0), Size::new(360.0, 280.0)),
            },
            &metrics,
        )
        .expect("floating group can resize");
    assert!(layout.validate(&metrics).is_ok());

    layout
        .apply(
            DockCommand::DockFloating {
                floating,
                target: DropTarget::Group {
                    group: target_group,
                    zone: DropZone::Center,
                },
            },
            &metrics,
        )
        .expect("floating group can dock back into the existing tab group");

    assert!(!layout.floating_groups().contains_key(floating));
    assert_eq!(panel_placement_count(&layout, panel), 1);
    assert!(matches!(
        layout.nodes().get(target_group),
        Some(Node::Tabs(tabs)) if tabs.panels.contains(&panel)
    ));
    assert!(layout.validate(&metrics).is_ok());
}
