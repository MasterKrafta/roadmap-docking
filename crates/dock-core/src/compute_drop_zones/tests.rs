use super::*;
use crate::layout_builder::LayoutBuilder;
use crate::{
    AllowedArea, ColumnMode, DockCommand, DockMetrics, DockSide, DropSource, DropTarget, DropZone,
    PanelConstraints, PanelKey, Point, Profile, Rect, Size,
};
use slotmap::Key;

fn key(name: &str) -> PanelKey {
    PanelKey::new(name).unwrap()
}

fn constraints() -> PanelConstraints {
    PanelConstraints::new(Size::new(80.0, 60.0), Size::new(180.0, 140.0), None, false)
}

fn viewport() -> Rect {
    Rect::new(Point::ZERO, Size::new(1200.0, 800.0))
}

#[test]
fn groups_have_center_and_tab_insertion_targets_but_not_static_default_zones() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);
    let (source_panel, source_node) = builder.panel(key("drop-source-tab"), constraints());
    let (target_panel, target_node) = builder.panel(key("drop-target-tab"), constraints());
    let source_group = builder.tabs(vec![source_node], 0, false);
    let target_group = builder.tabs(vec![target_node], 0, false);
    let column = builder.column(
        280.0,
        ColumnMode::Expanded,
        vec![source_group, target_group],
        vec![0.5, 0.5],
    );
    builder.main_area(DockSide::Left, column);
    let layout = builder.build();
    let rects = layout.compute_rects(viewport(), &metrics).unwrap();
    assert!(rects.drop_zones.is_empty());
    let zones = layout
        .compute_drop_zones(
            DropSource::Panel {
                panel: source_panel,
            },
            &rects,
            &metrics,
        )
        .unwrap();

    assert!(zones.iter().any(|zone| {
        zone.target
            == DropTarget::Tabs {
                group: target_group,
                index: 1,
            }
            && zone.zone == DropZone::Center
    }));
    assert!(zones.iter().any(|zone| {
        zone.target
            == DropTarget::Tabs {
                group: target_group,
                index: 0,
            }
            && zone.zone == DropZone::TabInsertion
    }));
    assert!(zones.iter().any(|zone| {
        zone.target
            == DropTarget::Tabs {
                group: target_group,
                index: 1,
            }
            && zone.zone == DropZone::TabInsertion
    }));
    assert!(zones.iter().any(|zone| {
        zone.target
            == DropTarget::GroupEdge {
                group: target_group,
                zone: DropZone::Top,
            }
            && zone.zone == DropZone::Top
    }));
    assert!(zones.iter().any(|zone| {
        zone.target
            == DropTarget::GroupEdge {
                group: target_group,
                zone: DropZone::Bottom,
            }
            && zone.zone == DropZone::Bottom
    }));
    assert!(zones.iter().any(|zone| {
        zone.target
            == DropTarget::MainColumn {
                side: DockSide::Left,
                index: 0,
            }
            && zone.zone == DropZone::ColumnLeft
    }));
    assert!(zones.iter().any(|zone| {
        zone.target
            == DropTarget::MainColumn {
                side: DockSide::Left,
                index: 1,
            }
            && zone.zone == DropZone::ColumnRight
    }));
    assert!(zones.iter().any(|zone| {
        zone.target == DropTarget::ColumnSlot { column, index: 2 }
            && zone.zone == DropZone::ColumnEnd
    }));
    assert!(zones.iter().any(|zone| {
        zone.target
            == DropTarget::MainColumn {
                side: DockSide::Left,
                index: 0,
            }
            && zone.zone == DropZone::DockEdge
    }));
    let left_drop_edge = zones
        .iter()
        .find(|zone| {
            zone.target
                == DropTarget::MainColumn {
                    side: DockSide::Left,
                    index: 0,
                }
                && zone.zone == DropZone::ColumnLeft
        })
        .unwrap();
    assert_eq!(left_drop_edge.rect.size.width, 24.0);

    let mut wide_metrics = metrics;
    wide_metrics.edge_drop_width = 32.0;
    let wide_rects = layout.compute_rects(viewport(), &wide_metrics).unwrap();
    let wide_zones = layout
        .compute_drop_zones(
            DropSource::Panel {
                panel: source_panel,
            },
            &wide_rects,
            &wide_metrics,
        )
        .unwrap();
    let wide_left = wide_zones
        .iter()
        .find(|zone| {
            zone.target
                == DropTarget::MainColumn {
                    side: DockSide::Left,
                    index: 0,
                }
                && zone.zone == DropZone::ColumnLeft
        })
        .unwrap();
    assert_eq!(wide_left.rect.size.width, 32.0);
    assert_eq!(wide_metrics.splitter_thickness, 4.0);
    assert_eq!(wide_metrics.splitter_hit_zone, 8.0);
    assert_ne!(source_panel, target_panel);
}

#[test]
fn a_group_cannot_drop_onto_itself_or_a_noop_neighbor_slot() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);
    let (_, first_node) = builder.panel(key("self-first"), constraints());
    let first = builder.tabs(vec![first_node], 0, false);
    let (_, second_node) = builder.panel(key("self-second"), constraints());
    let second = builder.tabs(vec![second_node], 0, false);
    let column = builder.column(
        280.0,
        ColumnMode::Expanded,
        vec![first, second],
        vec![0.5, 0.5],
    );
    builder.main_area(DockSide::Left, column);
    let mut layout = builder.build();
    let before_self_drop = format!("{layout:?}");
    let before_version = layout.version();
    layout
        .apply(
            DockCommand::MergeGroup {
                group: first,
                target_group: first,
                index: 0,
            },
            &metrics,
        )
        .unwrap();
    assert_eq!(format!("{layout:?}"), before_self_drop);
    assert_eq!(layout.version(), before_version);

    let rects = layout.compute_rects(viewport(), &metrics).unwrap();
    let zones = layout
        .compute_drop_zones(DropSource::Group { group: first }, &rects, &metrics)
        .unwrap();

    assert!(!zones.iter().any(|zone| matches!(
        zone.target,
        DropTarget::GroupTabs { group, .. } | DropTarget::GroupEdge { group, .. }
            if group == first
    )));
    assert!(!zones.iter().any(|zone| {
        zone.target == DropTarget::ColumnSlot { column, index: 0 }
            || zone.target == DropTarget::ColumnSlot { column, index: 1 }
    }));
}

#[test]
fn allowed_areas_hide_disallowed_main_dock_targets_but_keep_floating_and_free_area() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);
    let (source_panel, source_node) = builder.panel(
        key("floating-only"),
        constraints().with_allowed_areas(AllowedArea::FLOATING),
    );
    let source_group = builder.tabs(vec![source_node], 0, false);
    let column = builder.column(280.0, ColumnMode::Expanded, vec![source_group], vec![1.0]);
    builder.main_area(DockSide::Left, column);

    let (_, float_node) = builder.panel(key("floating-target"), constraints());
    let float_tabs = builder.tabs(vec![float_node], 0, false);
    let float_id = builder.floating_group(
        float_tabs,
        Rect::new(Point::new(450.0, 150.0), Size::new(400.0, 300.0)),
        1,
    );
    let layout = builder.build();
    let rects = layout.compute_rects(viewport(), &metrics).unwrap();
    let zones = layout
        .compute_drop_zones(
            DropSource::Panel {
                panel: source_panel,
            },
            &rects,
            &metrics,
        )
        .unwrap();

    assert!(!zones.iter().any(|zone| matches!(
        zone.target,
        DropTarget::Tabs { group, .. } | DropTarget::GroupEdge { group, .. }
            if group == source_group
    )));
    assert!(zones.iter().any(|zone| {
        zone.target
            == DropTarget::Tabs {
                group: float_tabs,
                index: 1,
            }
            && zone.zone == DropZone::FloatingCenter
    }));
    assert!(
        zones
            .iter()
            .any(|zone| zone.target == DropTarget::FreeArea && zone.zone == DropZone::FreeArea)
    );
    assert!(layout.floating_group(float_id).is_some());
}

#[test]
fn command_application_revalidates_allowed_area_even_for_forged_targets() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);
    let (source_panel, source_node) = builder.panel(
        key("command-area-source"),
        constraints().with_allowed_areas(AllowedArea::MAIN_DOCK),
    );
    let source_group = builder.tabs(vec![source_node], 0, false);
    let source_column = builder.column(280.0, ColumnMode::Expanded, vec![source_group], vec![1.0]);
    builder.main_area(DockSide::Right, source_column);

    let (_, rail_node) = builder.panel(key("command-area-rail"), constraints());
    let rail_group = builder.tabs(vec![rail_node], 0, true);
    let rail = builder.column(
        metrics.rail_width,
        ColumnMode::IconsOnly,
        vec![rail_group],
        vec![1.0],
    );
    builder.main_area(DockSide::Left, rail);

    let (_, float_node) = builder.panel(
        key("command-area-float"),
        constraints().with_allowed_areas(AllowedArea::FLOATING),
    );
    let float_group = builder.tabs(vec![float_node], 0, false);
    let float_id = builder.floating_group(
        float_group,
        Rect::new(Point::new(450.0, 140.0), Size::new(400.0, 300.0)),
        1,
    );

    let mut layout = builder.build();
    for target in [
        DropTarget::Tabs {
            group: float_group,
            index: 1,
        },
        DropTarget::RailSlot {
            column: rail,
            index: 1,
        },
    ] {
        let before = format!("{layout:?}");
        let version = layout.version();
        assert!(
            layout
                .apply(
                    DockCommand::MovePanel {
                        panel: source_panel,
                        target
                    },
                    &metrics
                )
                .is_err()
        );
        assert_eq!(format!("{layout:?}"), before);
        assert_eq!(layout.version(), version);
    }

    for command in [
        DockCommand::MoveGroup {
            group: source_group,
            column: rail,
            index: 1,
        },
        DockCommand::SetColumnMode {
            column: source_column,
            mode: ColumnMode::IconsOnly,
        },
        DockCommand::DockFloating {
            group: float_id,
            target: DropTarget::Tabs {
                group: source_group,
                index: 1,
            },
        },
        DockCommand::Float {
            node: source_group,
            bounds: Rect::new(Point::new(500.0, 200.0), Size::new(400.0, 300.0)),
        },
    ] {
        let before = format!("{layout:?}");
        let version = layout.version();
        assert!(layout.apply(command, &metrics).is_err());
        assert_eq!(format!("{layout:?}"), before);
        assert_eq!(layout.version(), version);
    }

    let before = format!("{layout:?}");
    let version = layout.version();
    assert!(
        layout
            .apply(
                DockCommand::OpenPanel {
                    key: key("forged-floating-only"),
                    constraints: constraints().with_allowed_areas(AllowedArea::FLOATING),
                    target: Some(DropTarget::Tabs {
                        group: source_group,
                        index: 1
                    }),
                },
                &metrics
            )
            .is_err()
    );
    assert_eq!(format!("{layout:?}"), before);
    assert_eq!(layout.version(), version);
}

#[test]
fn command_application_revalidates_stale_target_ids() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);
    let (source_panel, source_node) = builder.panel(key("stale-source"), constraints());
    let source_group = builder.tabs(vec![source_node], 0, false);
    let (_, stale_node) = builder.panel(key("stale-target"), constraints());
    let stale_group = builder.tabs(vec![stale_node], 0, false);
    let column = builder.column(
        280.0,
        ColumnMode::Expanded,
        vec![source_group, stale_group],
        vec![0.5, 0.5],
    );
    builder.main_area(DockSide::Left, column);
    let mut layout = builder.build();

    layout
        .apply(DockCommand::CloseGroup { group: stale_group }, &metrics)
        .unwrap();
    assert!(layout.node(stale_group).is_none());
    let before = format!("{layout:?}");
    let version = layout.version();
    assert!(
        layout
            .apply(
                DockCommand::MovePanel {
                    panel: source_panel,
                    target: DropTarget::Tabs {
                        group: stale_group,
                        index: 0
                    },
                },
                &metrics,
            )
            .is_err()
    );
    assert_eq!(format!("{layout:?}"), before);
    assert_eq!(layout.version(), version);
    assert_eq!(layout.validate(&metrics), Ok(()));
}

#[test]
fn generic_floating_group_exposes_horizontal_split_targets() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Generic);
    let (_, source_node) = builder.panel(key("generic-floating-source"), constraints());
    let source_group = builder.tabs(vec![source_node], 0, false);
    builder.main_area(DockSide::Left, source_group);

    let (_, target_node) = builder.panel(key("generic-floating-target"), constraints());
    let target_group = builder.tabs(vec![target_node], 0, false);
    builder.floating_group(
        target_group,
        Rect::new(Point::new(500.0, 120.0), Size::new(420.0, 320.0)),
        1,
    );

    let layout = builder.build();
    let rects = layout.compute_rects(viewport(), &metrics).unwrap();
    let zones = layout
        .compute_drop_zones(
            DropSource::Group {
                group: source_group,
            },
            &rects,
            &metrics,
        )
        .unwrap();

    for (zone_kind, edge) in [
        (DropZone::Left, DropZone::Left),
        (DropZone::Right, DropZone::Right),
    ] {
        assert!(zones.iter().any(|zone| {
            zone.zone == zone_kind
                && zone.target
                    == DropTarget::GroupEdge {
                        group: target_group,
                        zone: edge,
                    }
        }));
    }
    let left = zones
        .iter()
        .find(|zone| {
            zone.zone == DropZone::Left
                && zone.target
                    == DropTarget::GroupEdge {
                        group: target_group,
                        zone: DropZone::Left,
                    }
        })
        .unwrap();
    let point = Point::new(
        left.rect.origin.x + left.rect.size.width / 2.0,
        left.rect.origin.y + left.rect.size.height / 2.0,
    );
    assert_eq!(
        layout
            .resolve_drop(
                point,
                DropSource::Group {
                    group: source_group
                },
                &rects,
                &metrics
            )
            .unwrap(),
        Some(DropTarget::GroupEdge {
            group: target_group,
            zone: DropZone::Left
        })
    );
}

#[test]
fn max_columns_hides_new_column_targets_when_the_source_column_cannot_collapse() {
    let metrics = DockMetrics {
        max_columns: 1,
        ..DockMetrics::default()
    };
    let mut builder = LayoutBuilder::new(Profile::Photoshop);
    let (source_panel, source_node) = builder.panel(key("max-column-source"), constraints());
    let (_, other_node) = builder.panel(key("max-column-other"), constraints());
    let group = builder.tabs(vec![source_node, other_node], 0, false);
    let column = builder.column(280.0, ColumnMode::Expanded, vec![group], vec![1.0]);
    builder.main_area(DockSide::Left, column);
    let layout = builder.build();
    let rects = layout.compute_rects(viewport(), &metrics).unwrap();
    let zones = layout
        .compute_drop_zones(
            DropSource::Panel {
                panel: source_panel,
            },
            &rects,
            &metrics,
        )
        .unwrap();
    assert!(!zones.iter().any(|zone| matches!(
        zone.target,
        DropTarget::MainColumn { .. } if matches!(zone.zone, DropZone::ColumnLeft | DropZone::ColumnRight | DropZone::DockEdge)
    )));
}

#[test]
fn floating_center_and_edges_resolve_to_tab_or_stack_targets() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Generic);
    let (source_panel, source_node) = builder.panel(key("float-drop-source"), constraints());
    let main_group = builder.tabs(vec![source_node], 0, false);
    let column = builder.column(280.0, ColumnMode::Expanded, vec![main_group], vec![1.0]);
    builder.main_area(DockSide::Left, column);
    let (_, float_node) = builder.panel(key("float-drop-target"), constraints());
    let float_group = builder.tabs(vec![float_node], 0, false);
    let bounds = Rect::new(Point::new(500.0, 120.0), Size::new(420.0, 320.0));
    builder.floating_group(float_group, bounds, 4);
    let layout = builder.build();
    let rects = layout.compute_rects(viewport(), &metrics).unwrap();
    let zones = layout
        .compute_drop_zones(
            DropSource::Panel {
                panel: source_panel,
            },
            &rects,
            &metrics,
        )
        .unwrap();

    assert!(zones.iter().any(|zone| {
        zone.zone == DropZone::FloatingCenter
            && zone.target
                == DropTarget::Tabs {
                    group: float_group,
                    index: 1,
                }
    }));
    assert!(zones.iter().any(|zone| {
        zone.zone == DropZone::FloatingTop
            && zone.target
                == DropTarget::GroupEdge {
                    group: float_group,
                    zone: DropZone::Top,
                }
    }));
    assert!(zones.iter().any(|zone| {
        zone.zone == DropZone::FloatingBottom
            && zone.target
                == DropTarget::GroupEdge {
                    group: float_group,
                    zone: DropZone::Bottom,
                }
    }));

    let center = zones
        .iter()
        .find(|zone| zone.zone == DropZone::FloatingCenter)
        .unwrap();
    let center_point = Point::new(
        center.rect.origin.x + center.rect.size.width / 2.0,
        center.rect.origin.y + center.rect.size.height / 2.0,
    );
    assert_eq!(
        layout
            .resolve_drop(
                center_point,
                DropSource::Panel {
                    panel: source_panel
                },
                &rects,
                &metrics
            )
            .unwrap(),
        Some(DropTarget::Tabs {
            group: float_group,
            index: 1
        })
    );

    let top = zones
        .iter()
        .find(|zone| zone.zone == DropZone::FloatingTop)
        .unwrap();
    let top_point = Point::new(
        top.rect.origin.x + top.rect.size.width / 2.0,
        top.rect.origin.y + top.rect.size.height / 2.0,
    );
    assert_eq!(
        layout
            .resolve_drop(
                top_point,
                DropSource::Panel {
                    panel: source_panel
                },
                &rects,
                &metrics
            )
            .unwrap(),
        Some(DropTarget::GroupEdge {
            group: float_group,
            zone: DropZone::Top
        })
    );
}

#[test]
fn floating_horizontal_composition_drop_zones_target_the_branch_under_pointer() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);

    let (source_panel, source_node) = builder.panel(key("float-x-source"), constraints());
    let source_group = builder.tabs(vec![source_node], 0, false);
    let source_column = builder.column(280.0, ColumnMode::Expanded, vec![source_group], vec![1.0]);
    builder.main_area(DockSide::Left, source_column);

    let (_, left_panel_node) = builder.panel(key("float-x-left"), constraints());
    let left_group = builder.tabs(vec![left_panel_node], 0, false);
    let left_column = builder.column(300.0, ColumnMode::Expanded, vec![left_group], vec![1.0]);

    let (_, right_panel_node) = builder.panel(key("float-x-right"), constraints());
    let right_group = builder.tabs(vec![right_panel_node], 0, false);
    let right_column = builder.column(300.0, ColumnMode::Expanded, vec![right_group], vec![1.0]);

    let root = builder.split(
        crate::Axis::X,
        vec![left_column, right_column],
        vec![0.5, 0.5],
    );
    let bounds = Rect::new(Point::new(500.0, 120.0), Size::new(604.0, 320.0));
    let floating_id = builder.floating_group(root, bounds, 4);
    let layout = builder.build();
    let rects = layout.compute_rects(viewport(), &metrics).unwrap();
    let zones = layout
        .compute_drop_zones(
            DropSource::Panel {
                panel: source_panel,
            },
            &rects,
            &metrics,
        )
        .unwrap();

    let right_group_rect = rects
        .groups
        .iter()
        .find(|entry| entry.group == right_group)
        .unwrap()
        .rect;
    let right_center = Point::new(
        right_group_rect.origin.x + right_group_rect.size.width / 2.0,
        right_group_rect.origin.y + metrics.header_height + metrics.tab_strip_height + 40.0,
    );
    assert_eq!(
        layout
            .resolve_drop(
                right_center,
                DropSource::Panel {
                    panel: source_panel
                },
                &rects,
                &metrics,
            )
            .unwrap(),
        Some(DropTarget::Tabs {
            group: right_group,
            index: 1,
        })
    );

    for (x, expected_group, edge, zone) in [
        (
            rects
                .columns
                .iter()
                .find(|entry| entry.column == left_column)
                .unwrap()
                .rect
                .origin
                .x
                + 100.0,
            left_group,
            DropZone::Top,
            DropZone::FloatingTop,
        ),
        (
            rects
                .columns
                .iter()
                .find(|entry| entry.column == right_column)
                .unwrap()
                .rect
                .origin
                .x
                + 100.0,
            right_group,
            DropZone::Top,
            DropZone::FloatingTop,
        ),
        (
            rects
                .columns
                .iter()
                .find(|entry| entry.column == left_column)
                .unwrap()
                .rect
                .origin
                .x
                + 100.0,
            left_group,
            DropZone::Bottom,
            DropZone::FloatingBottom,
        ),
        (
            rects
                .columns
                .iter()
                .find(|entry| entry.column == right_column)
                .unwrap()
                .rect
                .origin
                .x
                + 100.0,
            right_group,
            DropZone::Bottom,
            DropZone::FloatingBottom,
        ),
    ] {
        let edge_zone = zones
            .iter()
            .find(|entry| {
                entry.zone == zone
                    && entry.target
                        == DropTarget::GroupEdge {
                            group: expected_group,
                            zone: edge,
                        }
            })
            .expect("each horizontal branch should have its own floating edge target");
        let point = Point::new(
            x,
            edge_zone.rect.origin.y + edge_zone.rect.size.height / 2.0,
        );
        assert_eq!(
            layout
                .resolve_drop(
                    point,
                    DropSource::Panel {
                        panel: source_panel
                    },
                    &rects,
                    &metrics,
                )
                .unwrap(),
            Some(DropTarget::GroupEdge {
                group: expected_group,
                zone: edge,
            })
        );
    }

    assert!(layout.floating_group(floating_id).is_some());
}

#[test]
fn icon_rail_targets_flattened_insertion_order() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);
    let (source_panel, source_node) = builder.panel(key("rail-source"), constraints());
    let source_group = builder.tabs(vec![source_node], 0, false);
    let source_column = builder.column(280.0, ColumnMode::Expanded, vec![source_group], vec![1.0]);
    builder.main_area(DockSide::Right, source_column);

    let (_, first_node) = builder.panel(key("rail-first"), constraints());
    let first = builder.tabs(vec![first_node], 0, true);
    let (_, second_node) = builder.panel(key("rail-second"), constraints());
    let second = builder.tabs(vec![second_node], 0, true);
    let rail = builder.column(
        metrics.rail_width,
        ColumnMode::IconsOnly,
        vec![first, second],
        vec![0.5, 0.5],
    );
    builder.main_area(DockSide::Left, rail);
    let layout = builder.build();
    let rects = layout.compute_rects(viewport(), &metrics).unwrap();
    let zones = layout
        .compute_drop_zones(
            DropSource::Panel {
                panel: source_panel,
            },
            &rects,
            &metrics,
        )
        .unwrap();

    for index in 0..=2 {
        assert!(zones.iter().any(|zone| {
            zone.target
                == DropTarget::RailSlot {
                    column: rail,
                    index,
                }
                && zone.zone == DropZone::RailInsertion
        }));
    }
}

#[test]
fn labeled_rail_targets_flattened_insertion_order() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);
    let (source_panel, source_node) = builder.panel(key("rail-source"), constraints());
    let source_group = builder.tabs(vec![source_node], 0, false);
    let source_column = builder.column(280.0, ColumnMode::Expanded, vec![source_group], vec![1.0]);
    builder.main_area(DockSide::Right, source_column);

    let (_, first_node) = builder.panel(key("rail-first"), constraints());
    let first = builder.tabs(vec![first_node], 0, true);
    let (_, second_node) = builder.panel(key("rail-second"), constraints());
    let second = builder.tabs(vec![second_node], 0, true);
    let rail = builder.column(
        metrics.min_column_width,
        ColumnMode::IconsAndLabels,
        vec![first, second],
        vec![0.5, 0.5],
    );
    builder.main_area(DockSide::Left, rail);
    let layout = builder.build();
    let rects = layout.compute_rects(viewport(), &metrics).unwrap();
    let zones = layout
        .compute_drop_zones(
            DropSource::Panel {
                panel: source_panel,
            },
            &rects,
            &metrics,
        )
        .unwrap();

    for index in 0..=2 {
        assert!(zones.iter().any(|zone| {
            zone.target
                == DropTarget::RailSlot {
                    column: rail,
                    index,
                }
                && zone.zone == DropZone::RailInsertion
        }));
    }

    assert!(!zones.iter().any(|zone| match zone.target {
        DropTarget::Tabs { group, .. }
        | DropTarget::GroupTabs { group, .. }
        | DropTarget::GroupEdge { group, .. } => group == first || group == second,
        _ => false,
    }));
}

#[test]
fn resolver_prefers_tab_insertion_and_blocks_click_through_a_front_float() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);
    let (source_panel, source_node) = builder.panel(key("resolve-source"), constraints());
    let source_group = builder.tabs(vec![source_node], 0, false);
    let column = builder.column(280.0, ColumnMode::Expanded, vec![source_group], vec![1.0]);
    builder.main_area(DockSide::Left, column);

    let (_, lower_node) = builder.panel(key("resolve-lower"), constraints());
    let lower_group = builder.tabs(vec![lower_node], 0, false);
    let bounds = Rect::new(Point::new(420.0, 120.0), Size::new(400.0, 300.0));
    builder.floating_group(lower_group, bounds, 1);

    let (_, upper_node) = builder.panel(key("resolve-upper"), constraints());
    let upper_group = builder.tabs(vec![upper_node], 0, false);
    builder.floating_group(upper_group, bounds, 2);

    let mut layout = builder.build();
    let rects = layout.compute_rects(viewport(), &metrics).unwrap();
    let inside = Point::new(600.0, 405.0);
    let resolved = layout
        .resolve_drop(
            inside,
            DropSource::Panel {
                panel: source_panel,
            },
            &rects,
            &metrics,
        )
        .unwrap();
    assert!(matches!(
        resolved,
        Some(DropTarget::GroupEdge { group, zone: DropZone::Bottom }) if group == upper_group
    ));
    let prepared = layout
        .prepare_drop_resolver(
            DropSource::Panel {
                panel: source_panel,
            },
            &rects,
            &metrics,
        )
        .unwrap();
    assert_eq!(prepared.resolve(inside).unwrap(), resolved);
    for _ in 0..4 {
        assert_eq!(
            prepared.resolve(inside).unwrap(),
            resolved,
            "cached drop resolution must be deterministic for identical geometry and input"
        );
    }

    // The resolver owns its prepared data, so it can live in adapter state instead
    // of borrowing the Layout/geometry across GPUI callbacks. The adapter must still
    // invalidate it before using it against a changed live layout or viewport.
    assert!(
        layout
            .update_canvas_bounds(Some(Rect::new(
                Point::new(20.0, 20.0),
                Size::new(800.0, 600.0),
            )))
            .unwrap()
    );
    assert_eq!(prepared.resolve(inside).unwrap(), resolved);
    assert_ne!(lower_group, upper_group);
    assert!(
        layout
            .resolve_drop(
                Point::new(f32::NAN, 0.0),
                DropSource::Panel {
                    panel: source_panel
                },
                &rects,
                &metrics
            )
            .is_err()
    );
}

#[test]
fn photoshop_suppresses_horizontal_group_splits_inside_a_floating_stack() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);
    let (_, main_node) = builder.panel(key("profile-main"), constraints());
    let main_group = builder.tabs(vec![main_node], 0, false);
    let main_column = builder.column(280.0, ColumnMode::Expanded, vec![main_group], vec![1.0]);
    builder.main_area(DockSide::Left, main_column);

    let (source_panel, source_node) = builder.panel(key("profile-source"), constraints());
    let source_group = builder.tabs(vec![source_node], 0, false);
    let source_float = builder.floating_group(
        source_group,
        Rect::new(Point::new(500.0, 120.0), Size::new(420.0, 320.0)),
        2,
    );
    let mut layout = builder.build();
    let rects = layout.compute_rects(viewport(), &metrics).unwrap();
    let zones = layout
        .compute_drop_zones(
            DropSource::Panel {
                panel: source_panel,
            },
            &rects,
            &metrics,
        )
        .unwrap();
    assert!(!zones.iter().any(|zone| {
        matches!(zone.target, DropTarget::GroupEdge { group, zone: DropZone::Left | DropZone::Right } if group == source_group)
    }));
    assert!(zones.iter().any(|zone| {
        zone.zone == DropZone::ColumnLeft
            && matches!(
                zone.target,
                DropTarget::MainColumn { .. }
                    | DropTarget::GroupEdge {
                        zone: DropZone::Left,
                        ..
                    }
            )
    }));
    assert_ne!(source_float, crate::FloatingGroupId::null());

    let before = format!("{layout:?}");
    let version = layout.version();
    assert!(
        layout
            .apply(
                DockCommand::SplitGroup {
                    group: main_group,
                    target_group: source_group,
                    zone: DropZone::Left,
                    ratio: 0.5,
                },
                &metrics
            )
            .is_err()
    );
    assert_eq!(format!("{layout:?}"), before);
    assert_eq!(layout.version(), version);
}

#[test]
fn group_center_merge_resolves_and_preserves_panel_instances() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);
    let (source_panel, source_node) = builder.panel(key("merge-source"), constraints());
    let (target_panel, target_node) = builder.panel(key("merge-target"), constraints());
    let source_group = builder.tabs(vec![source_node], 0, false);
    let target_group = builder.tabs(vec![target_node], 0, false);
    let column = builder.column(
        280.0,
        ColumnMode::Expanded,
        vec![source_group, target_group],
        vec![0.5, 0.5],
    );
    builder.main_area(DockSide::Left, column);
    let mut layout = builder.build();
    let rects = layout.compute_rects(viewport(), &metrics).unwrap();
    let source = DropSource::Group {
        group: source_group,
    };
    let zones = layout.compute_drop_zones(source, &rects, &metrics).unwrap();
    let center = zones
        .iter()
        .find(|zone| {
            zone.target
                == DropTarget::GroupTabs {
                    group: target_group,
                    index: 1,
                }
                && zone.zone == DropZone::Center
        })
        .expect("target group's center should accept a group merge");
    let point = Point::new(
        center.rect.origin.x + center.rect.size.width / 2.0,
        center.rect.origin.y + center.rect.size.height / 2.0,
    );
    assert_eq!(
        layout
            .resolve_drop(point, source, &rects, &metrics)
            .unwrap(),
        Some(DropTarget::GroupTabs {
            group: target_group,
            index: 1
        })
    );

    let before_version = layout.version();
    layout
        .apply(
            DockCommand::MergeGroup {
                group: source_group,
                target_group,
                index: 1,
            },
            &metrics,
        )
        .unwrap();
    assert_eq!(layout.version(), before_version + 1);
    assert!(layout.node(source_group).is_none());
    assert!(matches!(
        layout.node(target_group),
        Some(Node::Tabs(tabs)) if tabs.children == vec![target_node, source_node] && tabs.active_index == 1
    ));
    assert!(layout.panel_instance(source_panel).is_some());
    assert!(layout.panel_instance(target_panel).is_some());
    assert_eq!(layout.validate(&metrics), Ok(()));
}

fn synthetic_drop_zone_layout(
    column_count: usize,
    groups_per_column: usize,
    panels_per_group: usize,
    metrics: &DockMetrics,
) -> (crate::Layout, crate::PanelId, NodeId, Rect) {
    assert!(column_count > 0);
    assert!(groups_per_column > 0);
    assert!(panels_per_group > 0);

    let mut builder = LayoutBuilder::new(Profile::Generic);
    let mut columns = Vec::with_capacity(column_count);
    let mut source_panel = None;
    let mut source_group = None;

    for column_index in 0..column_count {
        let mut groups = Vec::with_capacity(groups_per_column);
        for group_index in 0..groups_per_column {
            let mut panel_nodes = Vec::with_capacity(panels_per_group);
            for panel_index in 0..panels_per_group {
                let panel_key = format!("bench-c{column_index}-g{group_index}-p{panel_index}");
                let (panel, panel_node) = builder.panel(key(&panel_key), constraints());
                source_panel.get_or_insert(panel);
                panel_nodes.push(panel_node);
            }
            let group = builder.tabs(panel_nodes, 0, false);
            source_group.get_or_insert(group);
            groups.push(group);
        }

        columns.push(builder.column(
            metrics.default_column_width,
            ColumnMode::Expanded,
            groups,
            vec![1.0 / groups_per_column as f32; groups_per_column],
        ));
    }

    let root = if columns.len() == 1 {
        columns[0]
    } else {
        builder.split(
            crate::Axis::X,
            columns,
            vec![1.0 / column_count as f32; column_count],
        )
    };
    builder.main_area(DockSide::Left, root);

    // Keep one floating group in both fixtures to include the floating-area
    // ancestry lookups performed by the same public drop-zone pipeline.
    let (_, floating_panel_node) = builder.panel(key("bench-floating-panel"), constraints());
    let floating_tabs = builder.tabs(vec![floating_panel_node], 0, false);

    let viewport_width =
        column_count as f32 * metrics.default_column_width + metrics.min_canvas_width + 256.0;
    let viewport_height = groups_per_column as f32
        * (metrics.header_height
            + metrics.tab_strip_height
            + constraints().min_size.height
            + metrics.splitter_thickness)
        + 800.0;
    builder.floating_group(
        floating_tabs,
        Rect::new(
            Point::new(viewport_width - 460.0, 48.0),
            Size::new(420.0, 320.0),
        ),
        1,
    );

    (
        builder.build(),
        source_panel.expect("synthetic layout has at least one panel"),
        source_group.expect("synthetic layout has at least one group"),
        Rect::new(Point::ZERO, Size::new(viewport_width, viewport_height)),
    )
}

fn measure_drop_zone_case(
    label: &str,
    layout: &crate::Layout,
    source: DropSource,
    rects: &crate::LayoutRects,
    metrics: &DockMetrics,
    iterations: usize,
) {
    use std::hint::black_box;
    use std::time::Instant;

    let warmup_iterations = iterations.min(10);
    for _ in 0..warmup_iterations {
        let zones = layout
            .compute_drop_zones(source, rects, metrics)
            .expect("benchmark layout must produce drop zones");
        let _ = black_box(zones.len());
    }

    let start = Instant::now();
    let mut zone_count = 0;
    for _ in 0..iterations {
        let zones = layout
            .compute_drop_zones(source, rects, metrics)
            .expect("benchmark layout must produce drop zones");
        zone_count = zones.len();
        let _ = black_box(zones);
    }
    assert!(
        zone_count > 0,
        "benchmark case {label} should emit legal drop zones"
    );
    let elapsed = start.elapsed();
    let micros_per_call = elapsed.as_secs_f64() * 1_000_000.0 / iterations as f64;
    let calls_per_second = iterations as f64 / elapsed.as_secs_f64();

    println!(
        "drop-zone measurement [{}]: nodes={}, panels={}, floating={}, groups={}, headers={}, zones/call={}, iterations={}, elapsed_ms={:.3}, us/call={:.3}, calls/s={:.1}",
        label,
        layout.nodes().count(),
        layout.panel_instances().count(),
        layout.floating_groups().count(),
        rects.groups.len(),
        rects.tab_headers.len(),
        zone_count,
        iterations,
        elapsed.as_secs_f64() * 1000.0,
        micros_per_call,
        calls_per_second,
    );
}

/// Manual baseline only: this is deliberately ignored during normal test runs.
///
/// Run in release mode with one test thread to compare typical and large tree
/// traversal cost before changing the drop-zone algorithm.
/// Command: cargo test -p dock-core --release benchmark_compute_drop_zones_typical_and_large_layouts -- --ignored --nocapture --test-threads=1
#[test]
#[ignore = "manual performance baseline; run the documented release-mode command"]
fn benchmark_compute_drop_zones_typical_and_large_layouts() {
    let metrics = DockMetrics::default();
    let cases = [
        ("typical-2x4x2", 2, 4, 2, 500, 200),
        ("large-8x12x3", 8, 12, 3, 20, 5),
    ];

    for (
        label,
        column_count,
        groups_per_column,
        panels_per_group,
        panel_iterations,
        group_iterations,
    ) in cases
    {
        let (layout, source_panel, source_group, viewport) =
            synthetic_drop_zone_layout(column_count, groups_per_column, panels_per_group, &metrics);
        assert_eq!(layout.validate(&metrics), Ok(()));
        let rects = layout
            .compute_rects(viewport, &metrics)
            .expect("synthetic benchmark layout must resolve geometry");

        measure_drop_zone_case(
            &format!("{label}-panel-source"),
            &layout,
            DropSource::Panel {
                panel: source_panel,
            },
            &rects,
            &metrics,
            panel_iterations,
        );
        measure_drop_zone_case(
            &format!("{label}-group-source"),
            &layout,
            DropSource::Group {
                group: source_group,
            },
            &rects,
            &metrics,
            group_iterations,
        );
    }
}
