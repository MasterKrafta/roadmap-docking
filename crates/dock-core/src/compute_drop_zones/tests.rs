use super::*;
use crate::layout_builder::LayoutBuilder;
use crate::{
    AllowedArea, Axis, ColumnMode, DockCommand, DockMetrics, DockSide, DragDropAction,
    DragDropRequest, DragHandle, DragPayload, DropSource, DropTarget, DropZone, PanelConstraints,
    PanelKey, Point, Profile, Rect, Size,
};

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
fn main_panel_targets_append_to_the_end_of_a_tab_strip() {
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
    let append_zones: Vec<_> = zones
        .iter()
        .filter(|zone| {
            zone.zone == DropZone::TabInsertion
                && zone.target
                    == DropTarget::Tabs {
                        group: target_group,
                        index: 1,
                    }
        })
        .collect();
    assert_eq!(
        append_zones.len(),
        1,
        "a Main Dock panel drag must expose one append-only tab-strip zone"
    );
    let target_strip = rects
        .tab_strips
        .iter()
        .find(|strip| strip.group == target_group)
        .expect("the target group should have a tab strip");
    assert_eq!(append_zones[0].rect, target_strip.rect);
    let strip_center = Point::new(
        target_strip.rect.origin.x + target_strip.rect.size.width * 0.5,
        target_strip.rect.origin.y + target_strip.rect.size.height * 0.5,
    );
    assert_eq!(
        layout
            .resolve_drop(
                strip_center,
                DropSource::Panel { panel: source_panel },
                &rects,
                &metrics,
            )
            .unwrap(),
        Some(DropTarget::Tabs {
            group: target_group,
            index: 1,
        }),
        "dropping on a Main Dock tab strip must append to its end"
    );
    assert!(
        !zones.iter().any(|zone| {
            zone.target
                == DropTarget::Tabs {
                    group: target_group,
                    index: 0,
                }
        }),
        "a Main Dock panel drag must not expose insertion before an existing tab"
    );

    let group_zones = layout
        .compute_drop_zones(
            DropSource::Group { group: source_group },
            &rects,
            &metrics,
        )
        .unwrap();
    let group_append_zones: Vec<_> = group_zones
        .iter()
        .filter(|zone| {
            zone.zone == DropZone::TabInsertion
                && zone.target
                    == DropTarget::GroupTabs {
                        group: target_group,
                        index: 1,
                    }
        })
        .collect();
    assert_eq!(
        group_append_zones.len(),
        1,
        "a Main Dock Group drag must expose one append-only tab-strip zone"
    );
    assert_eq!(group_append_zones[0].rect, target_strip.rect);
    assert!(
        !group_zones.iter().any(|zone| {
            zone.target
                == DropTarget::GroupTabs {
                    group: target_group,
                    index: 0,
                }
                && zone.zone == DropZone::TabInsertion
        }),
        "a Main Dock Group drag must not expose insertion between existing tabs"
    );
    assert_eq!(
        layout
            .resolve_drop(
                strip_center,
                DropSource::Group { group: source_group },
                &rects,
                &metrics,
            )
            .unwrap(),
        Some(DropTarget::GroupTabs {
            group: target_group,
            index: 1,
        }),
        "dropping a Group on a Main Dock tab strip must append it to the end"
    );
    assert!(
        !zones.iter().any(|zone| {
            zone.target
                == DropTarget::GroupEdge {
                    group: target_group,
                    zone: DropZone::Top,
                }
                && zone.zone == DropZone::Top
        }),
        "the top edge of a later group is represented by the previous group's bottom strip"
    );
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
                == (DropTarget::MainColumn {
                    side: DockSide::Left,
                    index: 0,
                })
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
fn adjacent_main_groups_share_one_full_width_vertical_drop_strip() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);

    let (_, first_node) = builder.panel(key("boundary-first"), constraints());
    let first_group = builder.tabs(vec![first_node], 0, false);
    let (_, second_node) = builder.panel(key("boundary-second"), constraints());
    let second_group = builder.tabs(vec![second_node], 0, false);
    let (_, third_node) = builder.panel(key("boundary-third"), constraints());
    let third_group = builder.tabs(vec![third_node], 0, false);
    let target_column = builder.column(
        280.0,
        ColumnMode::Expanded,
        vec![first_group, second_group, third_group],
        vec![0.34, 0.33, 0.33],
    );
    builder.main_area(DockSide::Left, target_column);

    let (source_panel, source_node) = builder.panel(key("boundary-source"), constraints());
    let source_group = builder.tabs(vec![source_node], 0, false);
    let source_column =
        builder.column(280.0, ColumnMode::Expanded, vec![source_group], vec![1.0]);
    builder.main_area(DockSide::Right, source_column);

    let layout = builder.build();
    let rects = layout.compute_rects(viewport(), &metrics).unwrap();
    let target_column_rect = rects
        .columns
        .iter()
        .find(|column| column.column == target_column)
        .expect("the target Column should have computed bounds")
        .rect;

    let group_source = DropSource::Group { group: source_group };
    let zones = layout
        .compute_drop_zones(group_source, &rects, &metrics)
        .unwrap();

    let first_group_rect = rects
        .groups
        .iter()
        .find(|group| group.group == first_group)
        .expect("the first group should have computed bounds")
        .rect;
    let first_top = zones
        .iter()
        .find(|zone| {
            zone.zone == DropZone::Top
                && zone.target
                    == DropTarget::GroupEdge {
                        group: first_group,
                        zone: DropZone::Top,
                    }
        })
        .expect("the first group should expose a drop band in the composition header");
    assert_eq!(
        first_top.rect.origin.y + first_top.rect.size.height,
        first_group_rect.origin.y,
        "the bright top guide must align with the bottom edge of the composition header"
    );
    assert_eq!(
        first_top.rect.size.height,
        metrics.edge_drop_width.min(metrics.column_header_height)
    );

    let first_bottom = zones
        .iter()
        .find(|zone| {
            zone.zone == DropZone::Bottom
                && zone.target
                    == DropTarget::GroupEdge {
                        group: first_group,
                        zone: DropZone::Bottom,
                    }
        })
        .expect("the first group should own the shared boundary below it");
    assert!(
        !zones.iter().any(|zone| {
            zone.zone == DropZone::Top
                && zone.target
                    == DropTarget::GroupEdge {
                        group: second_group,
                        zone: DropZone::Top,
                    }
        }),
        "the lower group must not create a second overlapping target for the same boundary"
    );

    let splitter = rects
        .splitters
        .iter()
        .find(|splitter| {
            splitter.owner == target_column
                && splitter.axis == Axis::Y
                && splitter.boundary_index == 0
        })
        .expect("the target Column should expose its first vertical splitter");
    let boundary_point = Point::new(
        splitter.rect.origin.x + splitter.rect.size.width * 0.5,
        splitter.rect.origin.y + splitter.rect.size.height * 0.5,
    );
    assert_eq!(first_bottom.rect.size.width, target_column_rect.size.width);
    assert_eq!(
        first_bottom.rect.origin.x,
        target_column_rect.origin.x
    );
    assert!(
        first_bottom.rect.contains(boundary_point).unwrap(),
        "the shared drop strip must cover the splitter hit area"
    );

    let group_resolver = layout
        .prepare_drop_resolver(group_source, &rects, &metrics)
        .unwrap();
    let group_resolved = group_resolver
        .resolve_zone(boundary_point)
        .unwrap()
        .expect("a Group drag must resolve on the shared inter-group strip");
    assert_eq!(group_resolved.target, first_bottom.target);
    assert_eq!(group_resolved.zone, DropZone::Bottom);
    assert_eq!(group_resolved.rect, first_bottom.rect);

    let panel_source = DropSource::Panel { panel: source_panel };
    let panel_zones = layout
        .compute_drop_zones(panel_source, &rects, &metrics)
        .unwrap();
    let panel_top = panel_zones
        .iter()
        .find(|zone| {
            zone.zone == DropZone::Top
                && zone.target
                    == DropTarget::GroupEdge {
                        group: first_group,
                        zone: DropZone::Top,
                    }
        })
        .expect("a detached Panel must use the same header-bottom drop band");
    assert_eq!(panel_top.rect, first_top.rect);

    let panel_boundary = panel_zones
        .iter()
        .find(|zone| {
            zone.zone == DropZone::Bottom
                && zone.target
                    == DropTarget::GroupEdge {
                        group: first_group,
                        zone: DropZone::Bottom,
                    }
        })
        .expect("a detached Panel must use the same shared inter-group strip");
    assert_eq!(panel_boundary.rect, first_bottom.rect);

    let panel_resolver = layout
        .prepare_drop_resolver(panel_source, &rects, &metrics)
        .unwrap();
    let panel_resolved = panel_resolver
        .resolve_zone(boundary_point)
        .unwrap()
        .expect("a Panel drag must resolve on the shared inter-group strip");
    assert_eq!(panel_resolved.target, panel_boundary.target);
    assert_eq!(panel_resolved.zone, DropZone::Bottom);
    assert_eq!(panel_resolved.rect, panel_boundary.rect);

    let first_top = zones
        .iter()
        .find(|zone| {
            zone.zone == DropZone::Top
                && zone.target
                    == DropTarget::GroupEdge {
                        group: first_group,
                        zone: DropZone::Top,
                    }
        })
        .expect("the top of the first group remains a separate outer drop strip");
    let last_bottom = zones
        .iter()
        .find(|zone| {
            zone.zone == DropZone::Bottom
                && zone.target
                    == DropTarget::GroupEdge {
                        group: third_group,
                        zone: DropZone::Bottom,
                    }
        })
        .expect("the bottom of the last group remains a separate outer drop strip");
    assert_eq!(first_top.rect.size.width, target_column_rect.size.width);
    assert_eq!(last_bottom.rect.size.width, target_column_rect.size.width);
}

#[test]
fn photoshop_side_drop_targets_are_column_scoped_and_cover_full_height() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);

    let (_, first_node) = builder.panel(key("full-height-first"), constraints());
    let first_group = builder.tabs(vec![first_node], 0, false);
    let (_, second_node) = builder.panel(key("full-height-second"), constraints());
    let second_group = builder.tabs(vec![second_node], 0, false);
    let (_, third_node) = builder.panel(key("full-height-third"), constraints());
    let third_group = builder.tabs(vec![third_node], 0, false);
    let target_column = builder.column(
        280.0,
        ColumnMode::Expanded,
        vec![first_group, second_group, third_group],
        vec![0.34, 0.33, 0.33],
    );
    builder.main_area(DockSide::Left, target_column);

    let (source_panel, source_node) = builder.panel(key("full-height-source"), constraints());
    let (_, source_sibling_node) = builder.panel(key("full-height-source-sibling"), constraints());
    let source_group = builder.tabs(vec![source_node, source_sibling_node], 0, false);
    let source_column =
        builder.column(280.0, ColumnMode::Expanded, vec![source_group], vec![1.0]);
    builder.main_area(DockSide::Right, source_column);

    let layout = builder.build();
    let rects = layout.compute_rects(viewport(), &metrics).unwrap();
    let target_column_rect = rects
        .columns
        .iter()
        .find(|column| column.column == target_column)
        .expect("the target Main Column should have computed bounds")
        .rect;

    let group_source = DropSource::Group { group: source_group };
    let group_left_target = DropTarget::GroupEdge {
        group: first_group,
        zone: DropZone::Left,
    };
    let group_right_target = DropTarget::GroupEdge {
        group: third_group,
        zone: DropZone::Right,
    };
    let group_zones = layout
        .compute_drop_zones(group_source, &rects, &metrics)
        .unwrap();

    let group_left_zone = group_zones
        .iter()
        .find(|zone| {
            zone.target == group_left_target && zone.zone == DropZone::ColumnLeft
        })
        .expect("the whole column's left edge should accept a Group");
    let group_right_zone = group_zones
        .iter()
        .find(|zone| {
            zone.target == group_right_target && zone.zone == DropZone::ColumnRight
        })
        .expect("the whole column's right edge should accept a Group");

    for zone in [group_left_zone, group_right_zone] {
        assert_eq!(zone.rect.origin.y, target_column_rect.origin.y);
        assert_eq!(zone.rect.size.height, target_column_rect.size.height);
    }
    assert!(
        !group_zones.iter().any(|zone| {
            matches!(
                zone.target,
                DropTarget::GroupEdge {
                    group: target_group,
                    zone: DropZone::Left | DropZone::Right,
                } if target_group == second_group
            ) && matches!(zone.zone, DropZone::ColumnLeft | DropZone::ColumnRight)
        }),
        "an individual Group must not add its own horizontal side target"
    );

    let group_resolver = layout
        .prepare_drop_resolver(group_source, &rects, &metrics)
        .unwrap();
    for fraction in [0.05, 0.5, 0.95] {
        let point = Point::new(
            group_right_zone.rect.origin.x + group_right_zone.rect.size.width * 0.5,
            target_column_rect.origin.y + target_column_rect.size.height * fraction,
        );
        let resolved = group_resolver
            .resolve_zone(point)
            .unwrap()
            .expect("the column-wide right side should resolve at every vertical position");
        assert_eq!(resolved.target, group_right_target);
        assert_eq!(resolved.zone, DropZone::ColumnRight);
        assert_eq!(resolved.rect.origin.y, target_column_rect.origin.y);
        assert_eq!(resolved.rect.size.height, target_column_rect.size.height);
    }

    let panel_source = DropSource::Panel { panel: source_panel };
    let panel_target = DropTarget::MainColumn {
        side: DockSide::Left,
        index: 1,
    };
    let panel_zones = layout
        .compute_drop_zones(panel_source, &rects, &metrics)
        .unwrap();
    let panel_zone = panel_zones
        .iter()
        .find(|zone| zone.target == panel_target && zone.zone == DropZone::ColumnRight)
        .expect("the whole column's right edge should accept a detached Panel");
    assert_eq!(panel_zone.rect.origin.y, target_column_rect.origin.y);
    assert_eq!(panel_zone.rect.size.height, target_column_rect.size.height);

    let panel_resolver = layout
        .prepare_drop_resolver(panel_source, &rects, &metrics)
        .unwrap();
    for fraction in [0.05, 0.5, 0.95] {
        let point = Point::new(
            panel_zone.rect.origin.x + panel_zone.rect.size.width * 0.5,
            target_column_rect.origin.y + target_column_rect.size.height * fraction,
        );
        let resolved = panel_resolver
            .resolve_zone(point)
            .unwrap()
            .expect("the column-wide right side should resolve for a Panel at every height");
        assert_eq!(resolved.target, panel_target);
        assert_eq!(resolved.zone, DropZone::ColumnRight);
        assert_eq!(resolved.rect.origin.y, target_column_rect.origin.y);
        assert_eq!(resolved.rect.size.height, target_column_rect.size.height);
    }
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
    let (_, source_sibling_node) = builder.panel(key("floating-only-sibling"), constraints());
    let source_group = builder.tabs(vec![source_node, source_sibling_node], 0, false);
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

    let main_rects = layout.compute_rects(viewport(), &metrics).unwrap();
    let main_zones = layout
        .compute_drop_zones(
            DropSource::Panel { panel: source_panel },
            &main_rects,
            &metrics,
        )
        .unwrap();
    assert!(!main_zones.iter().any(|zone| matches!(
        zone.target,
        DropTarget::Tabs { group, .. } | DropTarget::GroupEdge { group, .. }
            if group == source_group
    )));
    assert!(main_zones.iter().any(|zone| {
        zone.target == DropTarget::FreeArea && zone.zone == DropZone::FreeArea
    }));

    let floating_rects = layout.compute_floating_rects(float_id, &metrics).unwrap();
    let floating_zones = layout
        .compute_drop_zones(
            DropSource::Panel { panel: source_panel },
            &floating_rects,
            &metrics,
        )
        .unwrap();
    assert!(floating_zones.iter().any(|zone| {
        zone.target == DropTarget::Tabs { group: float_tabs, index: 1 }
            && zone.zone == DropZone::FloatingCenter
    }));
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
    let floating = builder.floating_group(
        target_group,
        Rect::new(Point::new(500.0, 120.0), Size::new(420.0, 320.0)),
        1,
    );

    let layout = builder.build();
    let rects = layout.compute_floating_rects(floating, &metrics).unwrap();
    let zones = layout
        .compute_drop_zones(DropSource::Group { group: source_group }, &rects, &metrics)
        .unwrap();
    assert!(
        !zones.iter().any(|zone| {
            matches!(
                zone.target,
                DropTarget::MainColumn { .. } | DropTarget::FreeArea
            ) || matches!(zone.zone, DropZone::DockEdge)
        }),
        "a floating-window snapshot must not expose main-window dock edges or free Canvas"
    );

    for (zone_kind, edge) in [
        (DropZone::Left, DropZone::Left),
        (DropZone::Right, DropZone::Right),
    ] {
        assert!(zones.iter().any(|zone| {
            zone.zone == zone_kind
                && zone.target == DropTarget::GroupEdge { group: target_group, zone: edge }
        }));
    }
    let left = zones
        .iter()
        .find(|zone| {
            zone.zone == DropZone::Left
                && zone.target
                    == DropTarget::GroupEdge { group: target_group, zone: DropZone::Left }
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
                DropSource::Group { group: source_group },
                &rects,
                &metrics,
            )
            .unwrap(),
        Some(DropTarget::GroupEdge { group: target_group, zone: DropZone::Left })
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
        DropTarget::MainColumn {
            side: DockSide::Left,
            ..
        } if matches!(zone.zone, DropZone::ColumnLeft | DropZone::ColumnRight | DropZone::DockEdge)
    )), "the full left dock must not offer another column slot");
}

#[test]
fn column_limit_is_applied_independently_to_left_and_right_drop_zones() {
    let metrics = DockMetrics {
        max_columns: 1,
        ..DockMetrics::default()
    };
    let mut builder = LayoutBuilder::new(Profile::Photoshop);

    let (source_panel, source_node) = builder.panel(key("per-side-limit-source"), constraints());
    let (_, retained_node) = builder.panel(key("per-side-limit-retained"), constraints());
    let source_group = builder.tabs(vec![source_node, retained_node], 0, false);
    let left_column = builder.column(280.0, ColumnMode::Expanded, vec![source_group], vec![1.0]);
    builder.main_area(DockSide::Left, left_column);

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

    assert!(
        zones.iter().any(|zone| {
            zone.target
                == (DropTarget::MainColumn {
                    side: DockSide::Right,
                    index: 0,
                })
                && zone.zone == DropZone::DockEdge
        }),
        "the right dock must still accept its first column when the left dock is full"
    );
    assert!(
        !zones.iter().any(|zone| {
            matches!(
                zone.target,
                DropTarget::MainColumn {
                    side: DockSide::Left,
                    ..
                }
            ) && matches!(
                zone.zone,
                DropZone::ColumnLeft | DropZone::ColumnRight | DropZone::DockEdge
            )
        }),
        "the full left dock must not offer a new-column target"
    );
}

#[test]
fn moving_from_another_dock_does_not_free_a_full_destination_column_slot() {
    let metrics = DockMetrics {
        max_columns: 1,
        ..DockMetrics::default()
    };
    let mut builder = LayoutBuilder::new(Profile::Photoshop);

    let (source_panel, source_node) =
        builder.panel(key("per-side-limit-left-source"), constraints());
    let (_, source_retained) = builder.panel(key("per-side-limit-left-retained"), constraints());
    let source_group = builder.tabs(vec![source_node, source_retained], 0, false);
    let left_column = builder.column(280.0, ColumnMode::Expanded, vec![source_group], vec![1.0]);
    builder.main_area(DockSide::Left, left_column);

    let (_, right_node) = builder.panel(key("per-side-limit-right-target"), constraints());
    let right_group = builder.tabs(vec![right_node], 0, false);
    let right_column = builder.column(280.0, ColumnMode::Expanded, vec![right_group], vec![1.0]);
    builder.main_area(DockSide::Right, right_column);

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

    assert!(
        !zones.iter().any(|zone| {
            matches!(
                zone.target,
                DropTarget::MainColumn {
                    side: DockSide::Right,
                    ..
                }
            ) && matches!(
                zone.zone,
                DropZone::ColumnLeft | DropZone::ColumnRight | DropZone::DockEdge
            )
        }),
        "moving a panel from the left dock must not free a slot in the full right dock"
    );
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
    let floating = builder.floating_group(float_group, bounds, 4);
    let layout = builder.build();
    let rects = layout.compute_floating_rects(floating, &metrics).unwrap();
    let zones = layout
        .compute_drop_zones(
            DropSource::Panel { panel: source_panel },
            &rects,
            &metrics,
        )
        .unwrap();

    assert!(zones.iter().any(|zone| {
        zone.zone == DropZone::FloatingCenter
            && zone.target == DropTarget::Tabs { group: float_group, index: 1 }
    }));
    assert!(zones.iter().any(|zone| {
        zone.zone == DropZone::FloatingTop
            && zone.target
                == DropTarget::GroupEdge { group: float_group, zone: DropZone::Top }
    }));
    assert!(zones.iter().any(|zone| {
        zone.zone == DropZone::FloatingBottom
            && zone.target
                == DropTarget::GroupEdge { group: float_group, zone: DropZone::Bottom }
    }));

    let center = zones.iter().find(|zone| zone.zone == DropZone::FloatingCenter).unwrap();
    let center_point = Point::new(
        center.rect.origin.x + center.rect.size.width / 2.0,
        center.rect.origin.y + center.rect.size.height / 2.0,
    );
    assert_eq!(
        layout
            .resolve_drop(
                center_point,
                DropSource::Panel { panel: source_panel },
                &rects,
                &metrics,
            )
            .unwrap(),
        Some(DropTarget::Tabs { group: float_group, index: 1 })
    );

    let top = zones.iter().find(|zone| zone.zone == DropZone::FloatingTop).unwrap();
    let top_point = Point::new(
        top.rect.origin.x + top.rect.size.width / 2.0,
        top.rect.origin.y + top.rect.size.height / 2.0,
    );
    assert_eq!(
        layout
            .resolve_drop(
                top_point,
                DropSource::Panel { panel: source_panel },
                &rects,
                &metrics,
            )
            .unwrap(),
        Some(DropTarget::GroupEdge { group: float_group, zone: DropZone::Top })
    );
}

#[test]
fn floating_top_zone_tracks_the_actual_header_for_column_and_tabs_roots() {
    let mut metrics = DockMetrics::default();
    metrics.column_header_height = 40.0;
    metrics.edge_drop_width = 18.0;

    let mut builder = LayoutBuilder::new(Profile::Generic);
    let (source_panel, source_node) =
        builder.panel(key("floating-top-zone-source"), constraints());
    let source_group = builder.tabs(vec![source_node], 0, false);
    let source_column =
        builder.column(280.0, ColumnMode::Expanded, vec![source_group], vec![1.0]);
    builder.main_area(DockSide::Left, source_column);

    let (_, first_column_panel) =
        builder.panel(key("floating-top-column-first"), constraints());
    let first_column_group = builder.tabs(vec![first_column_panel], 0, false);
    let (_, second_column_panel) =
        builder.panel(key("floating-top-column-second"), constraints());
    let second_column_group = builder.tabs(vec![second_column_panel], 0, false);
    let column_root = builder.column(
        320.0,
        ColumnMode::Expanded,
        vec![first_column_group, second_column_group],
        vec![0.5, 0.5],
    );
    let floating_column = builder.floating_group(
        column_root,
        Rect::new(Point::new(500.0, 120.0), Size::new(420.0, 260.0)),
        2,
    );

    let (_, tabs_panel) = builder.panel(key("floating-top-tabs-root"), constraints());
    let tabs_root = builder.tabs(vec![tabs_panel], 0, false);
    let floating_tabs = builder.floating_group(
        tabs_root,
        Rect::new(Point::new(980.0, 120.0), Size::new(680.0, 520.0)),
        3,
    );

    let layout = builder.build();
    let assert_top_zone = |floating| {
        let rects = layout.compute_floating_rects(floating, &metrics).unwrap();
        let bounds = rects
            .floating_groups
            .first()
            .expect("the Floating window should have bounds")
            .rect;
        let first_group = rects
            .groups
            .first()
            .expect("the Floating composition should have a first Group");
        let tab_strip = rects
            .tab_strips
            .iter()
            .find(|strip| strip.group == first_group.group)
            .expect("the first Group should expose its tab strip");
        let top_zone = layout
            .compute_drop_zones(
                DropSource::Panel { panel: source_panel },
                &rects,
                &metrics,
            )
            .unwrap()
            .into_iter()
            .find(|zone| {
                zone.zone == DropZone::FloatingTop
                    && zone.target
                        == DropTarget::GroupEdge {
                            group: first_group.group,
                            zone: DropZone::Top,
                        }
            })
            .expect("the Floating header should expose its lower top-drop band");

        let actual_header_height = first_group.rect.origin.y - bounds.origin.y;
        let expected_height = metrics.edge_drop_width.min(actual_header_height);
        assert_eq!(actual_header_height, metrics.column_header_height);
        assert_eq!(top_zone.rect.size.height, expected_height);
        assert_eq!(
            top_zone.rect.origin.y + top_zone.rect.size.height,
            first_group.rect.origin.y,
            "the top-drop band must end exactly where the first Group begins"
        );
        assert_eq!(
            tab_strip.rect.origin.y,
            first_group.rect.origin.y,
            "the first tab strip starts at the first Group boundary"
        );
        assert!(
            top_zone.rect.origin.y + top_zone.rect.size.height <= tab_strip.rect.origin.y,
            "the Floating top-drop band must not overlap the tab strip"
        );

        let point = Point::new(
            top_zone.rect.origin.x + top_zone.rect.size.width * 0.5,
            top_zone.rect.origin.y + top_zone.rect.size.height * 0.5,
        );
        assert_eq!(
            layout
                .resolve_drop(
                    point,
                    DropSource::Panel { panel: source_panel },
                    &rects,
                    &metrics,
                )
                .unwrap(),
            Some(DropTarget::GroupEdge {
                group: first_group.group,
                zone: DropZone::Top,
            })
        );
    };

    assert_top_zone(floating_column);
    assert_top_zone(floating_tabs);
}

#[test]
fn floating_horizontal_composition_drop_zones_target_the_branch_under_pointer() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Generic);

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

    let root = builder.split(Axis::X, vec![left_column, right_column], vec![0.5, 0.5]);
    let floating = builder.floating_group(
        root,
        Rect::new(Point::new(500.0, 120.0), Size::new(604.0, 320.0)),
        4,
    );
    let layout = builder.build();
    let rects = layout.compute_floating_rects(floating, &metrics).unwrap();
    let zones = layout
        .compute_drop_zones(
            DropSource::Panel { panel: source_panel },
            &rects,
            &metrics,
        )
        .unwrap();

    let right_group_rect = rects.groups.iter().find(|entry| entry.group == right_group).unwrap().rect;
    let right_center = Point::new(
        right_group_rect.origin.x + right_group_rect.size.width / 2.0,
        right_group_rect.origin.y + metrics.header_height + metrics.tab_strip_height + 40.0,
    );
    assert_eq!(
        layout
            .resolve_drop(
                right_center,
                DropSource::Panel { panel: source_panel },
                &rects,
                &metrics,
            )
            .unwrap(),
        Some(DropTarget::Tabs { group: right_group, index: 1 })
    );

    for (x, expected_group, edge, zone) in [
        (
            rects.columns.iter().find(|entry| entry.column == left_column).unwrap().rect.origin.x + 100.0,
            left_group,
            DropZone::Top,
            DropZone::FloatingTop,
        ),
        (
            rects.columns.iter().find(|entry| entry.column == right_column).unwrap().rect.origin.x + 100.0,
            right_group,
            DropZone::Top,
            DropZone::FloatingTop,
        ),
        (
            rects.columns.iter().find(|entry| entry.column == left_column).unwrap().rect.origin.x + 100.0,
            left_group,
            DropZone::Bottom,
            DropZone::FloatingBottom,
        ),
        (
            rects.columns.iter().find(|entry| entry.column == right_column).unwrap().rect.origin.x + 100.0,
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
                        == DropTarget::GroupEdge { group: expected_group, zone: edge }
            })
            .expect("each horizontal branch should have its own floating edge target");
        let point = Point::new(x, edge_zone.rect.origin.y + edge_zone.rect.size.height / 2.0);
        assert_eq!(
            layout
                .resolve_drop(
                    point,
                    DropSource::Panel { panel: source_panel },
                    &rects,
                    &metrics,
                )
                .unwrap(),
            Some(DropTarget::GroupEdge { group: expected_group, zone: edge })
        );
    }

    assert!(layout.floating_group(floating).is_some());
}
#[test]
fn icon_rail_targets_flattened_insertion_order() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);
    let (source_panel, source_node) = builder.panel(key("rail-source"), constraints());
    let (_, source_sibling_node) = builder.panel(key("rail-source-sibling"), constraints());
    let source_group = builder.tabs(vec![source_node, source_sibling_node], 0, false);
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

    let composition_zones = layout
        .compute_drop_zones(
            DropSource::Composition {
                root: source_column,
            },
            &rects,
            &metrics,
        )
        .unwrap();
    let group_zones = layout
        .compute_drop_zones(DropSource::Group { group: source_group }, &rects, &metrics)
        .unwrap();
    assert_eq!(zones, composition_zones);
    assert_eq!(zones, group_zones);

    assert!(!zones.iter().any(|zone| match zone.target {
        DropTarget::Tabs { group, .. }
        | DropTarget::GroupTabs { group, .. }
        | DropTarget::GroupEdge { group, .. } => group == first || group == second,
        _ => false,
    }));
}


#[test]
fn resolver_uses_only_the_supplied_floating_window_snapshot() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);
    let (source_panel, source_node) = builder.panel(key("resolve-source"), constraints());
    let (_, source_sibling_node) = builder.panel(key("resolve-source-sibling"), constraints());
    let source_group = builder.tabs(vec![source_node, source_sibling_node], 0, false);
    let source_column = builder.column(280.0, ColumnMode::Expanded, vec![source_group], vec![1.0]);
    builder.main_area(DockSide::Left, source_column);

    let (_, lower_node) = builder.panel(key("resolve-lower"), constraints());
    let lower_group = builder.tabs(vec![lower_node], 0, false);
    let bounds = Rect::new(Point::new(420.0, 120.0), Size::new(400.0, 300.0));
    let lower = builder.floating_group(lower_group, bounds, 1);

    let (_, upper_node) = builder.panel(key("resolve-upper"), constraints());
    let upper_group = builder.tabs(vec![upper_node], 0, false);
    let upper = builder.floating_group(upper_group, bounds, 2);

    let layout = builder.build();
    let upper_rects = layout.compute_floating_rects(upper, &metrics).unwrap();
    let inside = Point::new(180.0, 285.0);
    let source = DropSource::Panel { panel: source_panel };
    let resolved = layout
        .resolve_drop(inside, source, &upper_rects, &metrics)
        .unwrap();
    assert!(matches!(
        resolved,
        Some(DropTarget::GroupEdge { group, zone: DropZone::Bottom }) if group == upper_group
    ));

    let prepared = layout
        .prepare_drop_resolver(source, &upper_rects, &metrics)
        .unwrap();
    assert_eq!(prepared.resolve(inside).unwrap(), resolved);
    for _ in 0..4 {
        assert_eq!(prepared.resolve(inside).unwrap(), resolved);
    }

    let lower_rects = layout.compute_floating_rects(lower, &metrics).unwrap();
    assert_eq!(
        layout
            .resolve_drop(inside, source, &lower_rects, &metrics)
            .unwrap(),
        Some(DropTarget::GroupEdge {
            group: lower_group,
            zone: DropZone::Bottom
        })
    );
    assert_ne!(lower_group, upper_group);
    assert_ne!(lower, upper);
}

#[test]
fn photoshop_suppresses_own_floating_side_targets_but_accepts_external_column_docking() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);
    let (_, main_node) = builder.panel(key("profile-main"), constraints());
    let main_group = builder.tabs(vec![main_node], 0, false);
    let main_column = builder.column(280.0, ColumnMode::Expanded, vec![main_group], vec![1.0]);
    builder.main_area(DockSide::Left, main_column);

    let (source_panel, source_node) = builder.panel(key("profile-source"), constraints());
    let source_group = builder.tabs(vec![source_node], 0, false);
    let (_, second_source_node) =
        builder.panel(key("profile-source-second"), constraints());
    let second_source_group = builder.tabs(vec![second_source_node], 0, false);
    let source_column = builder.column(
        280.0,
        ColumnMode::Expanded,
        vec![source_group, second_source_group],
        vec![0.5, 0.5],
    );
    let source_float = builder.floating_group(
        source_column,
        Rect::new(Point::new(500.0, 120.0), Size::new(420.0, 320.0)),
        2,
    );
    let mut layout = builder.build();

    let floating_rects = layout.compute_floating_rects(source_float, &metrics).unwrap();
    let floating_zones = layout
        .compute_drop_zones(
            DropSource::Panel { panel: source_panel },
            &floating_rects,
            &metrics,
        )
        .unwrap();
    assert!(!floating_zones.iter().any(|zone| {
        matches!(
            zone.target,
            DropTarget::GroupEdge {
                group,
                zone: DropZone::Left | DropZone::Right
            } if group == source_group || group == second_source_group
        )
    }));

    let main_rects = layout.compute_rects(viewport(), &metrics).unwrap();
    let main_zones = layout
        .compute_drop_zones(
            DropSource::Panel { panel: source_panel },
            &main_rects,
            &metrics,
        )
        .unwrap();
    assert!(main_zones.iter().any(|zone| {
        zone.zone == DropZone::ColumnLeft
            && matches!(
                zone.target,
                DropTarget::MainColumn { .. }
                    | DropTarget::GroupEdge { zone: DropZone::Left, .. }
            )
    }));

    let version = layout.version();
    layout
        .apply(
            DockCommand::SplitGroup {
                group: main_group,
                target_group: source_group,
                zone: DropZone::Left,
                ratio: 0.5,
            },
            &metrics,
        )
        .unwrap();

    assert_eq!(layout.version(), version + 1);
    assert_eq!(layout.validate(&metrics), Ok(()));
    let floating_area = layout.floating_group(source_float).unwrap().area;
    let floating_root = layout.dock_area(floating_area).unwrap().root;
    let Some(Node::Split(split)) = layout.node(floating_root) else {
        panic!("external side docking must create a horizontal Floating composition");
    };
    assert_eq!(split.axis, Axis::X);
    assert_eq!(split.children.len(), 2);
    assert_eq!(split.children[1], source_column);
    assert!(matches!(
        layout.node(split.children[0]),
        Some(Node::Column(column)) if column.groups == vec![main_group]
    ));
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

#[test]
fn resolver_exposes_the_exact_legal_zone_selected_for_a_pointer() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);
    let (source_panel, source_node) = builder.panel(key("resolver-source"), constraints());
    let (_, target_node) = builder.panel(key("resolver-target"), constraints());
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
    let resolver = layout
        .prepare_drop_resolver(
            DropSource::Panel {
                panel: source_panel,
            },
            &rects,
            &metrics,
        )
        .unwrap();

    // Resolve the center of an actual zone. The selected affordance must be
    // copied directly from the resolver's authoritative list.
    let candidate = resolver
        .zones()
        .first()
        .copied()
        .expect("fixture should emit legal drop zones");
    let point = Point::new(
        candidate.rect.origin.x + candidate.rect.size.width * 0.5,
        candidate.rect.origin.y + candidate.rect.size.height * 0.5,
    );
    let resolved_zone = resolver
        .resolve_zone(point)
        .unwrap()
        .expect("Core should return the winning legal affordance");

    assert!(resolver.zones().contains(&resolved_zone));
    assert_eq!(resolver.resolve(point).unwrap(), Some(resolved_zone.target));
}

#[test]
fn main_column_drop_zones_are_centered_on_the_splitter_between_columns() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);

    let (source_panel, source_node) = builder.panel(key("splitter-source"), constraints());
    let source_group = builder.tabs(vec![source_node], 0, false);
    let source_column =
        builder.column(280.0, ColumnMode::Expanded, vec![source_group], vec![1.0]);
    builder.main_area(DockSide::Right, source_column);

    let (_, target_left_node) =
        builder.panel(key("splitter-target-left"), constraints());
    let target_left_group = builder.tabs(vec![target_left_node], 0, false);
    let target_left_column =
        builder.column(280.0, ColumnMode::Expanded, vec![target_left_group], vec![1.0]);

    let (_, target_right_node) =
        builder.panel(key("splitter-target-right"), constraints());
    let target_right_group = builder.tabs(vec![target_right_node], 0, false);
    let target_right_column =
        builder.column(280.0, ColumnMode::Expanded, vec![target_right_group], vec![1.0]);

    let root = builder.split(
        Axis::X,
        vec![target_left_column, target_right_column],
        vec![0.5, 0.5],
    );
    builder.main_area(DockSide::Left, root);

    let layout = builder.build();
    let rects = layout.compute_rects(viewport(), &metrics).unwrap();
    let splitter = rects
        .splitters
        .iter()
        .find(|splitter| {
            splitter.owner == root && splitter.axis == Axis::X && splitter.boundary_index == 0
        })
        .expect("the horizontal main split should expose its inter-column splitter");
    let splitter_center = splitter.rect.origin.x + splitter.rect.size.width * 0.5;

    let zones = layout
        .compute_drop_zones(DropSource::Panel { panel: source_panel }, &rects, &metrics)
        .unwrap();
    let boundary_zones: Vec<_> = zones
        .iter()
        .filter(|zone| {
            zone.target
                == DropTarget::MainColumn {
                    side: DockSide::Left,
                    index: 1,
                }
                && matches!(zone.zone, DropZone::ColumnLeft | DropZone::ColumnRight)
        })
        .collect();

    assert!(!boundary_zones.is_empty());
    for zone in boundary_zones {
        let zone_center = zone.rect.origin.x + zone.rect.size.width * 0.5;
        assert!((zone_center - splitter_center).abs() < 0.001);
        assert_eq!(zone.rect.size.width, metrics.edge_drop_width);
    }
}

#[test]
fn canvas_facing_column_drop_zone_shifts_slightly_into_the_free_space() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);

    let (source_panel, source_node) = builder.panel(key("canvas-edge-source"), constraints());
    let source_group = builder.tabs(vec![source_node], 0, false);
    let source_column =
        builder.column(280.0, ColumnMode::Expanded, vec![source_group], vec![1.0]);
    builder.main_area(DockSide::Right, source_column);

    let (_, target_node) =
        builder.panel(key("canvas-edge-target"), constraints());
    let target_group = builder.tabs(vec![target_node], 0, false);
    let target_column =
        builder.column(280.0, ColumnMode::Expanded, vec![target_group], vec![1.0]);
    builder.main_area(DockSide::Left, target_column);

    let layout = builder.build();
    let rects = layout.compute_rects(viewport(), &metrics).unwrap();
    let splitter = rects
        .splitters
        .iter()
        .find(|splitter| {
            splitter.owner == target_column
                && splitter.axis == Axis::X
                && splitter.boundary_index == crate::OUTER_COLUMN_EDGE_BOUNDARY_INDEX
        })
        .expect("the canvas-facing column edge should expose its sentinel splitter");
    let splitter_center = splitter.rect.origin.x + splitter.rect.size.width * 0.5;

    let zones = layout
        .compute_drop_zones(DropSource::Panel { panel: source_panel }, &rects, &metrics)
        .unwrap();
    let boundary_zones: Vec<_> = zones
        .iter()
        .filter(|zone| {
            zone.target
                == DropTarget::MainColumn {
                    side: DockSide::Left,
                    index: 1,
                }
                && zone.zone == DropZone::ColumnRight
        })
        .collect();

    assert!(!boundary_zones.is_empty());
    let expected_center = splitter_center + metrics.splitter_thickness;
    for zone in boundary_zones {
        let zone_center = zone.rect.origin.x + zone.rect.size.width * 0.5;
        assert!((zone_center - expected_center).abs() < 0.001);
        assert_eq!(zone.rect.size.width, metrics.edge_drop_width);
    }
}

#[test]
fn floating_panel_can_drop_into_a_main_dock_group() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);

    let (moving_panel, moving_node) =
        builder.panel(key("floating-to-main-moving"), constraints());
    let (_, source_keep_node) =
        builder.panel(key("floating-to-main-keep"), constraints());
    let source_group = builder.tabs(vec![moving_node, source_keep_node], 0, false);
    let source_float = builder.floating_group(
        source_group,
        Rect::new(Point::new(120.0, 100.0), Size::new(420.0, 320.0)),
        1,
    );

    let (_, target_node) = builder.panel(key("floating-to-main-target"), constraints());
    let target_group = builder.tabs(vec![target_node], 0, false);
    let target_column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        vec![target_group],
        vec![1.0],
    );
    builder.main_area(DockSide::Right, target_column);

    let mut layout = builder.build();
    let rects = layout.compute_rects(viewport(), &metrics).unwrap();
    let zones = layout
        .compute_drop_zones(DropSource::Panel { panel: moving_panel }, &rects, &metrics)
        .unwrap();
    let target_zone = zones
        .iter()
        .find(|zone| {
            zone.zone == DropZone::Center
                && zone.target
                    == (DropTarget::Tabs {
                        group: target_group,
                        index: 1,
                    })
        })
        .expect("a floating panel must expose the center of a main Dock group");

    let request = crate::DragDropRequest {
        payload: crate::DragPayload::Panel { panel: moving_panel },
        handle: crate::DragHandle::PanelTab,
        origin: Point::ZERO,
        point: Point::new(
            target_zone.rect.origin.x + target_zone.rect.size.width * 0.5,
            target_zone.rect.origin.y + target_zone.rect.size.height * 0.5,
        ),
        target: Some(target_zone.target),
        floating_bounds: None,
    };
    assert_eq!(
        layout.resolve_drag_drop(&request, &metrics).unwrap(),
        crate::DragDropAction::Command(DockCommand::MovePanel {
            panel: moving_panel,
            target: target_zone.target,
        })
    );
    layout
        .apply(
            DockCommand::MovePanel {
                panel: moving_panel,
                target: target_zone.target,
            },
            &metrics,
        )
        .unwrap();

    let Some(Node::Tabs(source)) = layout.node(source_group) else {
        panic!("the source floating group must remain for its sibling panel");
    };
    assert_eq!(source.children, vec![source_keep_node]);
    let Some(Node::Tabs(target)) = layout.node(target_group) else {
        panic!("the destination main group must remain Tabs");
    };
    assert_eq!(target.children, vec![target_node, moving_node]);
    assert!(layout.floating_group(source_float).is_some());
    assert_eq!(layout.validate(&metrics), Ok(()));
}

#[test]
fn main_dock_panel_can_drop_into_a_floating_group() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);

    let (moving_panel, moving_node) =
        builder.panel(key("main-to-floating-moving"), constraints());
    let (_, source_keep_node) =
        builder.panel(key("main-to-floating-keep"), constraints());
    let source_group = builder.tabs(vec![moving_node, source_keep_node], 0, false);
    let source_column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        vec![source_group],
        vec![1.0],
    );
    builder.main_area(DockSide::Left, source_column);

    let (_, target_node) = builder.panel(key("main-to-floating-target"), constraints());
    let target_group = builder.tabs(vec![target_node], 0, false);
    let target_float = builder.floating_group(
        target_group,
        Rect::new(Point::new(540.0, 100.0), Size::new(420.0, 320.0)),
        1,
    );

    let mut layout = builder.build();
    let rects = layout.compute_floating_rects(target_float, &metrics).unwrap();
    let zones = layout
        .compute_drop_zones(DropSource::Panel { panel: moving_panel }, &rects, &metrics)
        .unwrap();
    let target_zone = zones
        .iter()
        .find(|zone| {
            zone.zone == DropZone::FloatingCenter
                && zone.target
                    == (DropTarget::Tabs {
                        group: target_group,
                        index: 1,
                    })
        })
        .expect("a floating group must expose its center for a main-dock panel");

    let request = crate::DragDropRequest {
        payload: crate::DragPayload::Panel { panel: moving_panel },
        handle: crate::DragHandle::PanelTab,
        origin: Point::ZERO,
        point: Point::new(
            target_zone.rect.origin.x + target_zone.rect.size.width * 0.5,
            target_zone.rect.origin.y + target_zone.rect.size.height * 0.5,
        ),
        target: Some(target_zone.target),
        floating_bounds: None,
    };
    assert_eq!(
        layout.resolve_drag_drop(&request, &metrics).unwrap(),
        crate::DragDropAction::Command(DockCommand::MovePanel {
            panel: moving_panel,
            target: target_zone.target,
        })
    );
    layout
        .apply(
            DockCommand::MovePanel {
                panel: moving_panel,
                target: target_zone.target,
            },
            &metrics,
        )
        .unwrap();

    let Some(Node::Tabs(source)) = layout.node(source_group) else {
        panic!("the source main group must remain for its sibling panel");
    };
    assert_eq!(source.children, vec![source_keep_node]);
    let Some(Node::Tabs(target)) = layout.node(target_group) else {
        panic!("the destination floating group must remain Tabs");
    };
    assert_eq!(target.children, vec![target_node, moving_node]);
    assert!(layout.floating_group(target_float).is_some());
    assert_eq!(layout.validate(&metrics), Ok(()));
}

#[test]
fn empty_main_dock_side_exposes_first_column_drop_zone() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);
    let (source_panel, source_node) = builder.panel(key("empty-side-source"), constraints());
    let source_group = builder.tabs(vec![source_node], 0, false);
    let right_column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        vec![source_group],
        vec![1.0],
    );
    builder.main_area(DockSide::Right, right_column);
    let layout = builder.build();
    let rects = layout.compute_rects(viewport(), &metrics).unwrap();
    let zones = layout
        .compute_drop_zones(DropSource::Panel { panel: source_panel }, &rects, &metrics)
        .unwrap();

    assert!(zones.iter().any(|zone| {
        zone.zone == DropZone::DockEdge
            && zone.target
                == DropTarget::MainColumn {
                    side: DockSide::Left,
                    index: 0,
                }
    }));
}


#[test]
fn floating_panel_tab_strip_uses_one_append_zone() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);

    let (source_panel, source_node) =
        builder.panel(key("floating-strip-source"), constraints());
    let (_, source_sibling_node) =
        builder.panel(key("floating-strip-source-sibling"), constraints());
    let source_group = builder.tabs(vec![source_node, source_sibling_node], 0, false);
    let source_column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        vec![source_group],
        vec![1.0],
    );
    builder.main_area(DockSide::Left, source_column);

    let (_, first_target_node) =
        builder.panel(key("floating-strip-target-first"), constraints());
    let (_, second_target_node) =
        builder.panel(key("floating-strip-target-second"), constraints());
    let target_group = builder.tabs(vec![first_target_node, second_target_node], 0, false);
    let target_float = builder.floating_group(
        target_group,
        Rect::new(Point::new(520.0, 120.0), Size::new(420.0, 320.0)),
        1,
    );

    let layout = builder.build();
    let rects = layout.compute_floating_rects(target_float, &metrics).unwrap();
    let strip = rects
        .tab_strips
        .iter()
        .find(|strip| strip.group == target_group)
        .expect("the Floating target must have a tab strip");
    let zones = layout
        .compute_drop_zones(DropSource::Panel { panel: source_panel }, &rects, &metrics)
        .unwrap();

    let append_zones: Vec<_> = zones
        .iter()
        .filter(|zone| {
            zone.zone == DropZone::TabInsertion
                && zone.target
                    == (DropTarget::Tabs {
                        group: target_group,
                        index: 2,
                    })
        })
        .collect();
    assert_eq!(append_zones.len(), 1);
    assert_eq!(append_zones[0].rect, strip.rect);
    assert!(!zones.iter().any(|zone| {
        zone.zone == DropZone::TabInsertion
            && matches!(
                zone.target,
                DropTarget::Tabs { group, index } if group == target_group && index < 2
            )
    }));

    let point = Point::new(
        strip.rect.origin.x + strip.rect.size.width / 2.0,
        strip.rect.origin.y + strip.rect.size.height / 2.0,
    );
    assert_eq!(
        layout.resolve_drop(
            point,
            DropSource::Panel { panel: source_panel },
            &rects,
            &metrics,
        ).unwrap(),
        Some(DropTarget::Tabs {
            group: target_group,
            index: 2,
        })
    );
}

#[test]
fn floating_group_tab_strip_uses_one_full_strip_append_zone_for_all_group_tabs() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);

    let (_, source_first_node) =
        builder.panel(key("floating-group-strip-source-first"), constraints());
    let (_, source_second_node) =
        builder.panel(key("floating-group-strip-source-second"), constraints());
    let source_group = builder.tabs(vec![source_first_node, source_second_node], 1, true);
    let (_, source_sibling_node) =
        builder.panel(key("floating-group-strip-source-sibling"), constraints());
    let source_sibling = builder.tabs(vec![source_sibling_node], 0, false);
    let source_column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        vec![source_group, source_sibling],
        vec![0.6, 0.4],
    );
    let source_float = builder.floating_group(
        source_column,
        Rect::new(Point::new(80.0, 100.0), Size::new(420.0, 520.0)),
        2,
    );

    let (_, target_first_node) =
        builder.panel(key("floating-group-strip-target-first"), constraints());
    let (_, target_second_node) =
        builder.panel(key("floating-group-strip-target-second"), constraints());
    let (_, target_third_node) =
        builder.panel(key("floating-group-strip-target-third"), constraints());
    let target_group = builder.tabs(
        vec![target_first_node, target_second_node, target_third_node],
        0,
        true,
    );
    let target_float = builder.floating_group(
        target_group,
        Rect::new(Point::new(560.0, 100.0), Size::new(520.0, 420.0)),
        1,
    );

    let mut layout = builder.build();
    let source = DropSource::Group { group: source_group };
    let rects = layout.compute_floating_rects(target_float, &metrics).unwrap();
    let strip = rects
        .tab_strips
        .iter()
        .find(|strip| strip.group == target_group)
        .expect("the Floating target must have a tab strip");
    let zones = layout.compute_drop_zones(source, &rects, &metrics).unwrap();
    let expected_target = DropTarget::GroupTabs {
        group: target_group,
        index: 3,
    };

    let append_zones: Vec<_> = zones
        .iter()
        .filter(|zone| {
            zone.zone == DropZone::TabInsertion
                && matches!(
                    zone.target,
                    DropTarget::GroupTabs { group, .. } if group == target_group
                )
        })
        .collect();
    assert_eq!(
        append_zones.len(),
        1,
        "a whole Group drag must expose exactly one insertion zone for the target strip"
    );
    assert_eq!(append_zones[0].target, expected_target);
    assert_eq!(append_zones[0].rect, strip.rect);
    assert!(
        !zones.iter().any(|zone| {
            zone.zone == DropZone::TabInsertion
                && matches!(
                    zone.target,
                    DropTarget::GroupTabs { group, index }
                        if group == target_group && index < 3
                )
        }),
        "a whole Group drag must not expose intermediate tab insertion slots"
    );
    assert!(
        !zones.iter().any(|zone| {
            zone.zone == DropZone::TabInsertion
                && matches!(zone.target, DropTarget::Tabs { group, .. } if group == target_group)
        }),
        "a whole Group drag must not be converted into a single-Panel tab insertion"
    );

    let strip_y = strip.rect.origin.y + strip.rect.size.height * 0.5;
    for x in [
        strip.rect.origin.x + 1.0,
        strip.rect.origin.x + strip.rect.size.width * 0.25,
        strip.rect.origin.x + strip.rect.size.width * 0.5,
        strip.rect.origin.x + strip.rect.size.width * 0.75,
        strip.rect.origin.x + strip.rect.size.width - 1.0,
    ] {
        assert_eq!(
            layout
                .resolve_drop(Point::new(x, strip_y), source, &rects, &metrics)
                .unwrap(),
            Some(expected_target),
            "every point across the Floating tab strip must resolve to the same append target"
        );
    }

    let request = DragDropRequest {
        payload: DragPayload::Group { group: source_group },
        handle: DragHandle::GroupGrip,
        origin: Point::ZERO,
        point: Point::new(
            strip.rect.origin.x + strip.rect.size.width * 0.5,
            strip_y,
        ),
        target: Some(expected_target),
        floating_bounds: None,
    };
    assert_eq!(
        layout.resolve_drag_drop(&request, &metrics).unwrap(),
        DragDropAction::Command(DockCommand::MergeGroup {
            group: source_group,
            target_group,
            index: 3,
        }),
        "releasing a Group over the Floating strip must merge the complete group"
    );

    layout
        .apply(
            DockCommand::MergeGroup {
                group: source_group,
                target_group,
                index: 3,
            },
            &metrics,
        )
        .unwrap();
    assert!(layout.node(source_group).is_none());
    assert!(layout.floating_group(source_float).is_some());
    assert!(layout.floating_group(target_float).is_some());
    assert!(matches!(
        layout.node(source_column),
        Some(crate::Node::Column(column)) if column.groups == vec![source_sibling]
    ));
    assert!(matches!(
        layout.node(target_group),
        Some(crate::Node::Tabs(tabs))
            if tabs.children == vec![
                target_first_node,
                target_second_node,
                target_third_node,
                source_first_node,
                source_second_node,
            ] && tabs.active_index == 4
    ));
    assert_eq!(layout.validate(&metrics), Ok(()));
}

#[test]
fn floating_group_edges_resolve_above_and_below_each_group() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);

    let (_, source_node) = builder.panel(key("floating-edge-source"), constraints());
    let source_group = builder.tabs(vec![source_node], 0, false);
    let source_column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        vec![source_group],
        vec![1.0],
    );
    builder.main_area(DockSide::Left, source_column);

    let (_, first_node) = builder.panel(key("floating-edge-first"), constraints());
    let first_group = builder.tabs(vec![first_node], 0, false);
    let (_, second_node) = builder.panel(key("floating-edge-second"), constraints());
    let second_group = builder.tabs(vec![second_node], 0, false);
    let target_column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        vec![first_group, second_group],
        vec![0.5, 0.5],
    );
    let target_float = builder.floating_group(
        target_column,
        Rect::new(Point::new(520.0, 120.0), Size::new(420.0, 520.0)),
        1,
    );

    let layout = builder.build();
    let rects = layout.compute_floating_rects(target_float, &metrics).unwrap();
    let zones = layout
        .compute_drop_zones(DropSource::Group { group: source_group }, &rects, &metrics)
        .unwrap();

    let second_top = zones.iter().find(|zone| {
        zone.zone == DropZone::Top
            && zone.target
                == (DropTarget::GroupEdge {
                    group: second_group,
                    zone: DropZone::Top,
                })
    }).expect("the second Floating group must expose its top edge");
    let first_bottom = zones.iter().find(|zone| {
        zone.zone == DropZone::Bottom
            && zone.target
                == (DropTarget::GroupEdge {
                    group: first_group,
                    zone: DropZone::Bottom,
                })
    }).expect("the first Floating group must expose its bottom edge");

    for zone in [second_top, first_bottom] {
        let point = Point::new(
            zone.rect.origin.x + zone.rect.size.width / 2.0,
            zone.rect.origin.y + zone.rect.size.height / 2.0,
        );
        assert_eq!(
            layout.resolve_drop(
                point,
                DropSource::Group { group: source_group },
                &rects,
                &metrics,
            ).unwrap(),
            Some(zone.target)
        );
    }
}

#[test]
fn floating_stack_exposes_column_end_for_another_floating_column() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);

    let (_, source_first_node) =
        builder.panel(key("floating-stack-source-first"), constraints());
    let source_first_group = builder.tabs(vec![source_first_node], 0, false);
    let (_, source_second_node) =
        builder.panel(key("floating-stack-source-second"), constraints());
    let source_second_group = builder.tabs(vec![source_second_node], 0, false);
    let source_stack = builder.split(
        Axis::Y,
        vec![source_first_group, source_second_group],
        vec![0.25, 0.75],
    );
    let source_float = builder.floating_group(
        source_stack,
        Rect::new(Point::new(80.0, 80.0), Size::new(420.0, 520.0)),
        2,
    );

    let (_, target_first_node) =
        builder.panel(key("floating-stack-target-first"), constraints());
    let target_first_group = builder.tabs(vec![target_first_node], 0, false);
    let (_, target_second_node) =
        builder.panel(key("floating-stack-target-second"), constraints());
    let target_second_group = builder.tabs(vec![target_second_node], 0, false);
    let target_column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        vec![target_first_group, target_second_group],
        vec![0.4, 0.6],
    );
    let target_float = builder.floating_group(
        target_column,
        Rect::new(Point::new(520.0, 120.0), Size::new(420.0, 520.0)),
        1,
    );

    let mut layout = builder.build();
    let rects = layout.compute_floating_rects(target_float, &metrics).unwrap();
    let source = DropSource::Composition { root: source_stack };
    let zones = layout.compute_drop_zones(source, &rects, &metrics).unwrap();
    let expected_target = DropTarget::ColumnSlot {
        column: target_column,
        index: 2,
    };
    let end_zone = zones.iter().find(|zone| {
        zone.zone == DropZone::ColumnEnd && zone.target == expected_target
    }).expect("a Floating Column must expose its end slot to a whole vertical stack");

    assert!(!zones
        .iter()
        .any(|zone| matches!(zone.target, DropTarget::GroupTabs { .. })));
    let point = Point::new(
        end_zone.rect.origin.x + end_zone.rect.size.width / 2.0,
        end_zone.rect.origin.y + end_zone.rect.size.height / 2.0,
    );
    assert_eq!(
        layout.resolve_drop(point, source, &rects, &metrics).unwrap(),
        Some(expected_target)
    );

    layout.apply(
        DockCommand::DockFloating {
            group: source_float,
            target: expected_target,
        },
        &metrics,
    ).unwrap();
    let Some(crate::Node::Column(column)) = layout.node(target_column) else {
        panic!("the target Floating composition must remain a Column");
    };
    assert_eq!(
        column.groups,
        vec![
            target_first_group,
            target_second_group,
            source_first_group,
            source_second_group,
        ]
    );
    assert!(layout.floating_group(source_float).is_none());
    assert_eq!(layout.validate(&metrics), Ok(()));
}

#[test]
fn single_group_floating_column_can_merge_into_another_floating_tab_strip() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);

    let (_, source_first_node) =
        builder.panel(key("floating-column-source-first"), constraints());
    let (_, source_second_node) =
        builder.panel(key("floating-column-source-second"), constraints());
    let source_group = builder.tabs(vec![source_first_node, source_second_node], 0, false);
    let source_column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        vec![source_group],
        vec![1.0],
    );
    let source_float = builder.floating_group(
        source_column,
        Rect::new(Point::new(80.0, 80.0), Size::new(420.0, 320.0)),
        2,
    );

    let (_, target_node) = builder.panel(key("floating-column-target"), constraints());
    let target_group = builder.tabs(vec![target_node], 0, false);
    let target_float = builder.floating_group(
        target_group,
        Rect::new(Point::new(520.0, 120.0), Size::new(420.0, 320.0)),
        1,
    );

    let mut layout = builder.build();
    assert!(layout.floating_group(source_float).is_some());
    let rects = layout.compute_floating_rects(target_float, &metrics).unwrap();
    let source = DropSource::Composition { root: source_column };
    let zones = layout.compute_drop_zones(source, &rects, &metrics).unwrap();
    let expected_target = DropTarget::GroupTabs {
        group: target_group,
        index: 1,
    };
    let strip = rects.tab_strips.iter().find(|strip| strip.group == target_group).unwrap();
    let zone = zones.iter().find(|zone| {
        zone.zone == DropZone::TabInsertion && zone.target == expected_target
    }).expect("a one-group Floating composition must expose the target tab strip");

    assert_eq!(zone.rect, strip.rect);
    let point = Point::new(
        strip.rect.origin.x + strip.rect.size.width / 2.0,
        strip.rect.origin.y + strip.rect.size.height / 2.0,
    );
    assert_eq!(
        layout.resolve_drop(point, source, &rects, &metrics).unwrap(),
        Some(expected_target)
    );
    layout.apply(
        DockCommand::DockFloating {
            group: source_float,
            target: expected_target,
        },
        &metrics,
    ).unwrap();
    let Some(crate::Node::Tabs(tabs)) = layout.node(target_group) else {
        panic!("the destination must remain a Tabs group");
    };
    assert_eq!(tabs.children, vec![target_node, source_first_node, source_second_node]);
    assert!(layout.floating_group(source_float).is_none());
    assert_eq!(layout.validate(&metrics), Ok(()));
}

#[test]
fn photoshop_single_group_composition_uses_group_edges_in_floating_horizontal_composition() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);

    let (_, source_panel) = builder.panel(key("floating-side-source"), constraints());
    let source_group = builder.tabs(vec![source_panel], 0, false);
    let source_column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        vec![source_group],
        vec![1.0],
    );
    builder.main_area(DockSide::Left, source_column);

    let (_, left_panel) = builder.panel(key("floating-side-left"), constraints());
    let left_group = builder.tabs(vec![left_panel], 0, false);
    let left_column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        vec![left_group],
        vec![1.0],
    );

    let (_, right_panel) = builder.panel(key("floating-side-right"), constraints());
    let right_group = builder.tabs(vec![right_panel], 0, false);
    let right_column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        vec![right_group],
        vec![1.0],
    );

    let target_root = builder.split(
        Axis::X,
        vec![left_column, right_column],
        vec![0.5, 0.5],
    );
    let target_floating = builder.floating_group(
        target_root,
        Rect::new(Point::new(620.0, 140.0), Size::new(420.0, 320.0)),
        1,
    );
    let mut layout = builder.build();

    layout
        .apply(
            DockCommand::Float {
                node: source_column,
                bounds: Rect::new(Point::new(120.0, 140.0), Size::new(420.0, 320.0)),
            },
            &metrics,
        )
        .unwrap();

    let target_rects = layout
        .compute_floating_rects(target_floating, &metrics)
        .unwrap();
    let zones = layout
        .compute_drop_zones(
            DropSource::Composition {
                root: source_column,
            },
            &target_rects,
            &metrics,
        )
        .unwrap();
    let group_zones = layout
        .compute_drop_zones(DropSource::Group { group: source_group }, &target_rects, &metrics)
        .unwrap();
    assert_eq!(
        zones, group_zones,
        "a single-group composition must generate exactly the same zones as its Group"
    );

    let has_edge = |group, edge| {
        zones.iter().any(|zone| {
            zone.target == DropTarget::GroupEdge { group, zone: edge } && zone.zone == edge
        })
    };

    assert!(has_edge(left_group, DropZone::Left));
    assert!(!has_edge(left_group, DropZone::Right));
    assert!(!has_edge(right_group, DropZone::Left));
    assert!(has_edge(right_group, DropZone::Right));
}


#[test]
fn floating_panel_and_group_use_the_same_main_dock_drop_zones() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);

    let (source_panel, source_node) =
        builder.panel(key("floating-main-zone-source"), constraints());
    let (_, source_sibling) =
        builder.panel(key("floating-main-zone-sibling"), constraints());
    let source_group = builder.tabs(vec![source_node, source_sibling], 0, false);
    builder.floating_group(
        source_group,
        Rect::new(Point::new(900.0, 100.0), Size::new(260.0, 320.0)),
        1,
    );

    let (_, first_node) = builder.panel(key("floating-main-zone-first"), constraints());
    let first_group = builder.tabs(vec![first_node], 0, false);
    let (_, second_node) = builder.panel(key("floating-main-zone-second"), constraints());
    let second_group = builder.tabs(vec![second_node], 0, false);
    let (_, third_node) = builder.panel(key("floating-main-zone-third"), constraints());
    let third_group = builder.tabs(vec![third_node], 0, false);
    let target_column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        vec![first_group, second_group, third_group],
        vec![0.34, 0.33, 0.33],
    );
    builder.main_area(DockSide::Left, target_column);

    let layout = builder.build();
    let rects = layout.compute_rects(viewport(), &metrics).unwrap();
    let column_rect = rects
        .columns
        .iter()
        .find(|column| column.column == target_column)
        .expect("the Main target Column should have computed bounds")
        .rect;
    let first_rect = rects
        .groups
        .iter()
        .find(|group| group.group == first_group)
        .expect("the first Main Group should have computed bounds")
        .rect;
    let target_strip = rects
        .tab_strips
        .iter()
        .find(|strip| strip.group == first_group)
        .expect("the first Main Group should have a tab strip");

    let sources = [
        (
            DropSource::Panel { panel: source_panel },
            DropTarget::Tabs {
                group: first_group,
                index: 1,
            },
            DropTarget::MainColumn {
                side: DockSide::Left,
                index: 0,
            },
            DropTarget::MainColumn {
                side: DockSide::Left,
                index: 1,
            },
        ),
        (
            DropSource::Group { group: source_group },
            DropTarget::GroupTabs {
                group: first_group,
                index: 1,
            },
            DropTarget::GroupEdge {
                group: first_group,
                zone: DropZone::Left,
            },
            DropTarget::GroupEdge {
                group: third_group,
                zone: DropZone::Right,
            },
        ),
    ];

    for (source, tab_target, left_target, right_target) in sources {
        let zones = layout
            .compute_drop_zones(source, &rects, &metrics)
            .unwrap();

        let tab_zone = zones
            .iter()
            .find(|zone| {
                zone.zone == DropZone::TabInsertion && zone.target == tab_target
            })
            .expect("a Floating source must expose the full Main tab-strip target");
        assert_eq!(tab_zone.rect, target_strip.rect);
        let before_target = match tab_target {
            DropTarget::Tabs { group, .. } => DropTarget::Tabs { group, index: 0 },
            DropTarget::GroupTabs { group, .. } => DropTarget::GroupTabs { group, index: 0 },
            _ => unreachable!("the test only uses Main tab-strip targets"),
        };
        assert!(!zones.iter().any(|zone| {
            zone.zone == DropZone::TabInsertion && zone.target == before_target
        }));

        let top_zone = zones
            .iter()
            .find(|zone| {
                zone.zone == DropZone::Top
                    && zone.target
                        == DropTarget::GroupEdge {
                            group: first_group,
                            zone: DropZone::Top,
                        }
            })
            .expect("a Floating source must use the Main composition-header top band");
        assert_eq!(
            top_zone.rect.origin.y + top_zone.rect.size.height,
            first_rect.origin.y
        );
        assert_eq!(
            top_zone.rect.size.height,
            metrics.edge_drop_width.min(metrics.column_header_height)
        );

        let shared_bottom = zones
            .iter()
            .find(|zone| {
                zone.zone == DropZone::Bottom
                    && zone.target
                        == DropTarget::GroupEdge {
                            group: first_group,
                            zone: DropZone::Bottom,
                        }
            })
            .expect("the upper Main Group must own the shared inter-group strip");
        assert_eq!(shared_bottom.rect.origin.x, column_rect.origin.x);
        assert_eq!(shared_bottom.rect.size.width, column_rect.size.width);
        assert!(!zones.iter().any(|zone| {
            zone.zone == DropZone::Top
                && zone.target
                    == DropTarget::GroupEdge {
                        group: second_group,
                        zone: DropZone::Top,
                    }
        }));

        let splitter = rects
            .splitters
            .iter()
            .find(|splitter| {
                splitter.owner == target_column
                    && splitter.axis == Axis::Y
                    && splitter.boundary_index == 0
            })
            .expect("the Main Column must have a splitter between its first two groups");
        let split_point = Point::new(
            splitter.rect.origin.x + splitter.rect.size.width * 0.5,
            splitter.rect.origin.y + splitter.rect.size.height * 0.5,
        );
        assert!(shared_bottom.rect.contains(split_point).unwrap());

        let left_zone = zones
            .iter()
            .find(|zone| zone.zone == DropZone::ColumnLeft && zone.target == left_target)
            .expect("a Floating source must expose the full-height Main left side");
        let right_zone = zones
            .iter()
            .find(|zone| zone.zone == DropZone::ColumnRight && zone.target == right_target)
            .expect("a Floating source must expose the full-height Main right side");
        for zone in [left_zone, right_zone] {
            assert_eq!(zone.rect.origin.y, column_rect.origin.y);
            assert_eq!(zone.rect.size.height, column_rect.size.height);
        }

        let resolver = layout.prepare_drop_resolver(source, &rects, &metrics).unwrap();
        let resolved_tab = resolver
            .resolve_zone(Point::new(
                target_strip.rect.origin.x + target_strip.rect.size.width * 0.5,
                target_strip.rect.origin.y + target_strip.rect.size.height * 0.5,
            ))
            .unwrap()
            .expect("the Main tab strip must resolve for a Floating source");
        assert_eq!(resolved_tab.target, tab_target);

        let resolved_split = resolver
            .resolve_zone(split_point)
            .unwrap()
            .expect("the shared Main group strip must resolve for a Floating source");
        assert_eq!(resolved_split.target, shared_bottom.target);

        for fraction in [0.05, 0.5, 0.95] {
            let point = Point::new(
                right_zone.rect.origin.x + right_zone.rect.size.width * 0.5,
                column_rect.origin.y + column_rect.size.height * fraction,
            );
            let resolved = resolver
                .resolve_zone(point)
                .unwrap()
                .expect("the Main side zone must resolve over the complete Column height");
            assert_eq!(resolved.target, right_target);
            assert_eq!(resolved.zone, DropZone::ColumnRight);
        }
    }
}


#[test]
fn single_group_floating_column_can_merge_into_a_main_tab_strip() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);

    let (_, source_first_node) =
        builder.panel(key("floating-main-column-source-first"), constraints());
    let (_, source_second_node) =
        builder.panel(key("floating-main-column-source-second"), constraints());
    let source_group = builder.tabs(vec![source_first_node, source_second_node], 0, false);
    let source_column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        vec![source_group],
        vec![1.0],
    );
    let source_float = builder.floating_group(
        source_column,
        Rect::new(Point::new(80.0, 80.0), Size::new(420.0, 320.0)),
        2,
    );

    let (_, target_node) = builder.panel(key("floating-main-column-target"), constraints());
    let target_group = builder.tabs(vec![target_node], 0, false);
    let target_column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        vec![target_group],
        vec![1.0],
    );
    builder.main_area(DockSide::Right, target_column);

    let mut layout = builder.build();
    let rects = layout.compute_rects(viewport(), &metrics).unwrap();
    let source = DropSource::Composition { root: source_column };
    let zones = layout.compute_drop_zones(source, &rects, &metrics).unwrap();
    let expected_target = DropTarget::GroupTabs {
        group: target_group,
        index: 1,
    };
    let strip = rects
        .tab_strips
        .iter()
        .find(|strip| strip.group == target_group)
        .expect("the Main target must have a tab strip");
    let zone = zones
        .iter()
        .find(|zone| {
            zone.zone == DropZone::TabInsertion && zone.target == expected_target
        })
        .expect("a one-group Floating composition must expose the Main tab strip");
    assert_eq!(zone.rect, strip.rect);

    let point = Point::new(
        strip.rect.origin.x + strip.rect.size.width * 0.5,
        strip.rect.origin.y + strip.rect.size.height * 0.5,
    );
    assert_eq!(
        layout.resolve_drop(point, source, &rects, &metrics).unwrap(),
        Some(expected_target)
    );
    layout
        .apply(
            DockCommand::DockFloating {
                group: source_float,
                target: expected_target,
            },
            &metrics,
        )
        .unwrap();

    let Some(crate::Node::Tabs(tabs)) = layout.node(target_group) else {
        panic!("the Main destination must remain a Tabs group");
    };
    assert_eq!(
        tabs.children,
        vec![target_node, source_first_node, source_second_node]
    );
    assert!(layout.floating_group(source_float).is_none());
    assert_eq!(layout.validate(&metrics), Ok(()));
}


#[test]
fn floating_column_composition_exposes_main_group_vertical_drop_zones() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);

    let (_, source_first_node) =
        builder.panel(key("floating-main-vertical-source-first"), constraints());
    let source_first_group = builder.tabs(vec![source_first_node], 0, false);
    let (_, source_second_node) =
        builder.panel(key("floating-main-vertical-source-second"), constraints());
    let source_second_group = builder.tabs(vec![source_second_node], 0, false);
    let source_column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        vec![source_first_group, source_second_group],
        vec![0.5, 0.5],
    );
    let source_float = builder.floating_group(
        source_column,
        Rect::new(Point::new(820.0, 100.0), Size::new(420.0, 520.0)),
        1,
    );

    let (_, first_node) = builder.panel(key("floating-main-vertical-first"), constraints());
    let first_group = builder.tabs(vec![first_node], 0, false);
    let (_, second_node) = builder.panel(key("floating-main-vertical-second"), constraints());
    let second_group = builder.tabs(vec![second_node], 0, false);
    let (_, third_node) = builder.panel(key("floating-main-vertical-third"), constraints());
    let third_group = builder.tabs(vec![third_node], 0, false);
    let target_column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        vec![first_group, second_group, third_group],
        vec![0.34, 0.33, 0.33],
    );
    builder.main_area(DockSide::Left, target_column);

    let layout = builder.build();
    let rects = layout.compute_rects(viewport(), &metrics).unwrap();
    let source = DropSource::Composition {
        root: source_column,
    };
    let zones = layout.compute_drop_zones(source, &rects, &metrics).unwrap();
    let target_column_rect = rects
        .columns
        .iter()
        .find(|column| column.column == target_column)
        .expect("the Main target Column should have computed bounds")
        .rect;

    let first_rect = rects
        .groups
        .iter()
        .find(|group| group.group == first_group)
        .expect("the first Main Group should have computed bounds")
        .rect;
    let first_top = zones
        .iter()
        .find(|zone| {
            zone.zone == DropZone::Top
                && zone.target
                    == DropTarget::GroupEdge {
                        group: first_group,
                        zone: DropZone::Top,
                    }
        })
        .expect("a Floating composition must expose the top of the Main stack");
    assert_eq!(
        first_top.rect.origin.y + first_top.rect.size.height,
        first_rect.origin.y
    );
    assert_eq!(
        first_top.rect.size.height,
        metrics.edge_drop_width.min(metrics.column_header_height)
    );

    let shared_bottom = zones
        .iter()
        .find(|zone| {
            zone.zone == DropZone::Bottom
                && zone.target
                    == DropTarget::GroupEdge {
                        group: first_group,
                        zone: DropZone::Bottom,
                    }
        })
        .expect("a Floating composition must expose the shared inter-group strip");
    let splitter = rects
        .splitters
        .iter()
        .find(|splitter| {
            splitter.owner == target_column
                && splitter.axis == Axis::Y
                && splitter.boundary_index == 0
        })
        .expect("the Main Column should have a splitter after its first Group");
    let splitter_center = Point::new(
        splitter.rect.origin.x + splitter.rect.size.width * 0.5,
        splitter.rect.origin.y + splitter.rect.size.height * 0.5,
    );
    assert_eq!(shared_bottom.rect.origin.x, target_column_rect.origin.x);
    assert_eq!(shared_bottom.rect.size.width, target_column_rect.size.width);
    assert!(shared_bottom.rect.contains(splitter_center).unwrap());
    assert!(!zones.iter().any(|zone| {
        zone.zone == DropZone::Top
            && zone.target
                == DropTarget::GroupEdge {
                    group: second_group,
                    zone: DropZone::Top,
                }
    }), "Main exposes the shared inter-group boundary only once as the Bottom edge of the preceding Group");

    let third_rect = rects
        .groups
        .iter()
        .find(|group| group.group == third_group)
        .expect("the last Main Group should have computed bounds")
        .rect;
    let last_bottom = zones
        .iter()
        .find(|zone| {
            zone.zone == DropZone::Bottom
                && zone.target
                    == DropTarget::GroupEdge {
                        group: third_group,
                        zone: DropZone::Bottom,
                    }
        })
        .expect("a Floating composition must expose the bottom of the Main stack");
    assert_eq!(
        last_bottom.rect.origin.y + last_bottom.rect.size.height,
        third_rect.origin.y + third_rect.size.height
    );

    let resolver = layout.prepare_drop_resolver(source, &rects, &metrics).unwrap();
    for zone in [first_top, shared_bottom, last_bottom] {
        let point = Point::new(
            zone.rect.origin.x + zone.rect.size.width * 0.5,
            zone.rect.origin.y + zone.rect.size.height * 0.5,
        );
        let resolved = resolver
            .resolve_zone(point)
            .unwrap()
            .expect("each Main vertical drop zone must resolve for a Floating Column");
        assert_eq!(resolved.target, zone.target);
        assert_eq!(resolved.zone, zone.zone);
    }

    let request = crate::DragDropRequest {
        payload: crate::DragPayload::FloatingGroup { group: source_float },
        handle: crate::DragHandle::FloatingCapture,
        origin: Point::ZERO,
        point: Point::new(
            first_top.rect.origin.x + first_top.rect.size.width * 0.5,
            first_top.rect.origin.y + first_top.rect.size.height * 0.5,
        ),
        target: Some(first_top.target),
        floating_bounds: None,
    };
    assert_eq!(
        layout.resolve_drag_drop(&request, &metrics).unwrap(),
        crate::DragDropAction::Command(DockCommand::DockFloating {
            group: source_float,
            target: first_top.target,
        })
    );
}



#[test]
fn floating_column_composition_exposes_vertical_zones_in_another_floating_window() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);

    let (_, source_first_node) =
        builder.panel(key("floating-floating-source-first"), constraints());
    let source_first_group = builder.tabs(vec![source_first_node], 0, false);
    let (_, source_second_node) =
        builder.panel(key("floating-floating-source-second"), constraints());
    let source_second_group = builder.tabs(vec![source_second_node], 0, false);
    let source_column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        vec![source_first_group, source_second_group],
        vec![0.5, 0.5],
    );
    let source_float = builder.floating_group(
        source_column,
        Rect::new(Point::new(100.0, 120.0), Size::new(420.0, 520.0)),
        1,
    );

    let (_, first_node) = builder.panel(key("floating-floating-target-first"), constraints());
    let first_group = builder.tabs(vec![first_node], 0, false);
    let (_, second_node) = builder.panel(key("floating-floating-target-second"), constraints());
    let second_group = builder.tabs(vec![second_node], 0, false);
    let (_, third_node) = builder.panel(key("floating-floating-target-third"), constraints());
    let third_group = builder.tabs(vec![third_node], 0, false);
    let target_column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        vec![first_group, second_group, third_group],
        vec![0.34, 0.33, 0.33],
    );
    let target_float = builder.floating_group(
        target_column,
        Rect::new(Point::new(620.0, 100.0), Size::new(420.0, 620.0)),
        2,
    );

    let mut layout = builder.build();
    let rects = layout
        .compute_floating_rects(target_float, &metrics)
        .unwrap();
    let source = DropSource::Composition {
        root: source_column,
    };
    let zones = layout.compute_drop_zones(source, &rects, &metrics).unwrap();
    let target_column_rect = rects
        .columns
        .iter()
        .find(|column| column.column == target_column)
        .expect("the target Floating Column should have computed bounds")
        .rect;
    let floating_bounds = rects
        .floating_groups
        .first()
        .expect("the target Floating window should have bounds")
        .rect;

    let first_top = zones
        .iter()
        .find(|zone| {
            zone.zone == DropZone::FloatingTop
                && zone.target
                    == DropTarget::GroupEdge {
                        group: first_group,
                        zone: DropZone::Top,
                    }
        })
        .expect("a Floating composition must expose the top edge of another Floating window");
    let shared_bottom = zones
        .iter()
        .find(|zone| {
            zone.zone == DropZone::Bottom
                && zone.target
                    == DropTarget::GroupEdge {
                        group: first_group,
                        zone: DropZone::Bottom,
                    }
        })
        .expect("a Floating composition must expose the boundary between Floating groups");
    let splitter = rects
        .splitters
        .iter()
        .find(|splitter| {
            splitter.owner == target_column
                && splitter.axis == Axis::Y
                && splitter.boundary_index == 0
        })
        .expect("the target Floating Column should have a splitter after its first Group");
    let splitter_center = Point::new(
        splitter.rect.origin.x + splitter.rect.size.width * 0.5,
        splitter.rect.origin.y + splitter.rect.size.height * 0.5,
    );
    assert_eq!(shared_bottom.rect.origin.x, target_column_rect.origin.x);
    assert_eq!(shared_bottom.rect.size.width, target_column_rect.size.width);
    assert!(shared_bottom.rect.contains(splitter_center).unwrap());
    assert!(zones.iter().any(|zone| {
        zone.zone == DropZone::Top
            && zone.target
                == DropTarget::GroupEdge {
                    group: second_group,
                    zone: DropZone::Top,
                }
    }), "the single-group composition must expose the same per-Group top edge as a Group drag");

    let last_bottom = zones
        .iter()
        .find(|zone| {
            zone.zone == DropZone::ColumnEnd
                && zone.target
                    == DropTarget::ColumnSlot {
                        column: target_column,
                        index: 3,
                    }
        })
        .expect("a whole multi-group Column must expose the append-to-end target");

    assert!(zones.iter().any(|zone| {
        zone.zone == DropZone::Left
            && zone.target
                == DropTarget::GroupEdge {
                    group: first_group,
                    zone: DropZone::Left,
                }
    }), "a Main composition must expose the Floating left side");
    let left_side = zones
        .iter()
        .find(|zone| {
            zone.zone == DropZone::Left
                && zone.target
                    == DropTarget::GroupEdge {
                        group: first_group,
                        zone: DropZone::Left,
                    }
        })
        .expect("a Main composition must expose one full-height Floating left side");
    let right_side = zones
        .iter()
        .find(|zone| {
            zone.zone == DropZone::Right
                && zone.target
                    == DropTarget::GroupEdge {
                        group: third_group,
                        zone: DropZone::Right,
                    }
        })
        .expect("a Main composition must expose one full-height Floating right side");
    assert_eq!(left_side.rect.origin.y, floating_bounds.origin.y);
    assert_eq!(right_side.rect.origin.y, floating_bounds.origin.y);
    assert_eq!(left_side.rect.size.height, floating_bounds.size.height);
    assert_eq!(right_side.rect.size.height, floating_bounds.size.height);
    assert_eq!(left_side.rect.size.width, metrics.edge_drop_width);
    assert_eq!(right_side.rect.size.width, metrics.edge_drop_width);
    assert_eq!(
        zones.iter().filter(|zone| zone.zone == DropZone::Left).count(),
        1,
        "the left edge is one composition-wide zone, not one zone per Group"
    );
    assert_eq!(
        zones.iter().filter(|zone| zone.zone == DropZone::Right).count(),
        1,
        "the right edge is one composition-wide zone, not one zone per Group"
    );

    let resolver = layout
        .prepare_drop_resolver(source, &rects, &metrics)
        .unwrap();
    for zone in [left_side, right_side] {
        for fraction in [0.02, 0.5, 0.98] {
            let point = Point::new(
                zone.rect.origin.x + zone.rect.size.width * 0.5,
                floating_bounds.origin.y + floating_bounds.size.height * fraction,
            );
            let resolved = resolver
                .resolve_zone(point)
                .unwrap()
                .expect("the side zone must resolve across the entire Floating height");
            assert_eq!(resolved.target, zone.target);
            assert_eq!(resolved.zone, zone.zone);
            assert_eq!(resolved.rect, zone.rect);
        }

        for splitter in rects
            .splitters
            .iter()
            .filter(|splitter| splitter.axis == Axis::Y)
        {
            let point = Point::new(
                zone.rect.origin.x + zone.rect.size.width * 0.5,
                splitter.rect.origin.y + splitter.rect.size.height * 0.5,
            );
            let resolved = resolver
                .resolve_zone(point)
                .unwrap()
                .expect("the side zone must remain reachable at an internal Group splitter");
            assert_eq!(resolved.zone, zone.zone);
            assert_eq!(resolved.target, zone.target);
            assert_eq!(resolved.rect, zone.rect);
        }
    }

    for zone in [first_top, shared_bottom, last_bottom] {
        let point = Point::new(
            zone.rect.origin.x + zone.rect.size.width * 0.5,
            zone.rect.origin.y + zone.rect.size.height * 0.5,
        );
        let resolved = resolver
            .resolve_zone(point)
            .unwrap()
            .expect("each Floating vertical target must resolve");
        assert_eq!(resolved.target, zone.target);
        assert_eq!(resolved.zone, zone.zone);
    }

    let request = crate::DragDropRequest {
        payload: crate::DragPayload::FloatingGroup { group: source_float },
        handle: crate::DragHandle::FloatingCapture,
        origin: Point::ZERO,
        point: Point::new(
            shared_bottom.rect.origin.x + shared_bottom.rect.size.width * 0.5,
            shared_bottom.rect.origin.y + shared_bottom.rect.size.height * 0.5,
        ),
        target: Some(shared_bottom.target),
        floating_bounds: None,
    };
    assert_eq!(
        layout.resolve_drag_drop(&request, &metrics).unwrap(),
        crate::DragDropAction::Command(DockCommand::DockFloating {
            group: source_float,
            target: shared_bottom.target,
        })
    );

    let side_point = Point::new(
        right_side.rect.origin.x + right_side.rect.size.width * 0.5,
        right_side.rect.origin.y + right_side.rect.size.height * 0.5,
    );
    let side_request = crate::DragDropRequest {
        payload: crate::DragPayload::FloatingGroup { group: source_float },
        handle: crate::DragHandle::FloatingCapture,
        origin: Point::ZERO,
        point: side_point,
        target: Some(right_side.target),
        floating_bounds: None,
    };
    assert_eq!(
        layout.resolve_drag_drop(&side_request, &metrics).unwrap(),
        crate::DragDropAction::Command(DockCommand::DockFloating {
            group: source_float,
            target: right_side.target,
        })
    );
    layout
        .apply(
            DockCommand::DockFloating {
                group: source_float,
                target: right_side.target,
            },
            &metrics,
        )
        .unwrap();

    let target_area = layout.floating_group(target_float).unwrap().area;
    let composition_root = layout.dock_area(target_area).unwrap().root;
    let Some(crate::Node::Split(composition)) = layout.node(composition_root) else {
        panic!("a Floating side drop must create a horizontal composition of Columns");
    };
    assert_eq!(composition.axis, Axis::X);
    assert_eq!(composition.children, vec![target_column, source_column]);
    assert!(layout.floating_group(source_float).is_none());
    assert_eq!(layout.validate(&metrics), Ok(()));
}


#[test]
fn photoshop_floating_side_zones_span_the_composition_and_insert_a_column() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);

    let (source_panel, source_panel_node) =
        builder.panel(key("floating-side-full-height-source"), constraints());
    let (_, source_sibling_node) =
        builder.panel(key("floating-side-full-height-sibling"), constraints());
    let source_group = builder.tabs(vec![source_panel_node, source_sibling_node], 0, false);
    let source_column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        vec![source_group],
        vec![1.0],
    );
    builder.main_area(DockSide::Left, source_column);

    let (_, left_first_node) =
        builder.panel(key("floating-side-left-first"), constraints());
    let left_first_group = builder.tabs(vec![left_first_node], 0, false);
    let (_, left_second_node) =
        builder.panel(key("floating-side-left-second"), constraints());
    let left_second_group = builder.tabs(vec![left_second_node], 0, false);
    let left_column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        vec![left_first_group, left_second_group],
        vec![0.5, 0.5],
    );

    let (_, right_first_node) =
        builder.panel(key("floating-side-right-first"), constraints());
    let right_first_group = builder.tabs(vec![right_first_node], 0, false);
    let (_, right_second_node) =
        builder.panel(key("floating-side-right-second"), constraints());
    let right_second_group = builder.tabs(vec![right_second_node], 0, false);
    let right_column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        vec![right_first_group, right_second_group],
        vec![0.5, 0.5],
    );

    let target_root = builder.split(
        Axis::X,
        vec![left_column, right_column],
        vec![0.5, 0.5],
    );
    let target_float = builder.floating_group(
        target_root,
        Rect::new(Point::new(520.0, 100.0), Size::new(620.0, 620.0)),
        2,
    );

    let mut layout = builder.build();
    let rects = layout.compute_floating_rects(target_float, &metrics).unwrap();
    let bounds = rects
        .floating_groups
        .first()
        .expect("the Floating composition should have bounds")
        .rect;
    let group_source = DropSource::Group { group: source_group };
    let panel_source = DropSource::Panel { panel: source_panel };
    let group_zones = layout
        .compute_drop_zones(group_source, &rects, &metrics)
        .unwrap();
    let panel_zones = layout
        .compute_drop_zones(panel_source, &rects, &metrics)
        .unwrap();

    let side_zones = |zones: &[crate::DropZoneRect]| {
        zones
            .iter()
            .filter(|zone| matches!(zone.zone, DropZone::Left | DropZone::Right))
            .copied()
            .collect::<Vec<_>>()
    };
    let group_sides = side_zones(&group_zones);
    let panel_sides = side_zones(&panel_zones);
    assert_eq!(group_sides.len(), 2, "one left and one right side zone are expected");
    assert_eq!(panel_sides, group_sides, "Panel and Group handles share side geometry");

    let left_side = group_sides
        .iter()
        .find(|zone| zone.zone == DropZone::Left)
        .unwrap();
    let right_side = group_sides
        .iter()
        .find(|zone| zone.zone == DropZone::Right)
        .unwrap();
    assert_eq!(
        left_side.target,
        DropTarget::GroupEdge {
            group: left_first_group,
            zone: DropZone::Left,
        }
    );
    assert_eq!(
        right_side.target,
        DropTarget::GroupEdge {
            group: right_second_group,
            zone: DropZone::Right,
        }
    );

    for zone in [left_side, right_side] {
        assert_eq!(zone.rect.origin.y, bounds.origin.y);
        assert_eq!(zone.rect.size.height, bounds.size.height);
        assert_eq!(zone.rect.size.width, metrics.edge_drop_width);

        for fraction in [0.02, 0.5, 0.98] {
            let point = Point::new(
                zone.rect.origin.x + zone.rect.size.width * 0.5,
                bounds.origin.y + bounds.size.height * fraction,
            );
            for source in [group_source, panel_source] {
                let resolved = layout
                    .resolve_drop(point, source, &rects, &metrics)
                    .unwrap()
                    .expect("the full-height side must resolve at every vertical position");
                assert_eq!(resolved, zone.target);
            }
        }

        let mut overlapping_group_strips = 0;
        for strip in &rects.tab_strips {
            let group_append_zone = group_zones
                .iter()
                .find(|candidate| {
                    candidate.zone == DropZone::TabInsertion
                        && matches!(
                            candidate.target,
                            DropTarget::GroupTabs { group, .. } if group == strip.group
                        )
                })
                .expect("a whole-Group drag must expose one append target for each tab strip");

            // Only test the overlap where the pointer is physically inside both
            // this tab strip and the composition-wide side band. A strip in the
            // opposite Column cannot be selected from this edge of the window.
            let intersection_left = zone.rect.origin.x.max(group_append_zone.rect.origin.x);
            let intersection_top = zone.rect.origin.y.max(group_append_zone.rect.origin.y);
            let intersection_right = (zone.rect.origin.x + zone.rect.size.width)
                .min(group_append_zone.rect.origin.x + group_append_zone.rect.size.width);
            let intersection_bottom = (zone.rect.origin.y + zone.rect.size.height)
                .min(group_append_zone.rect.origin.y + group_append_zone.rect.size.height);
            if intersection_right <= intersection_left || intersection_bottom <= intersection_top {
                continue;
            }
            overlapping_group_strips += 1;

            let point = Point::new(
                (intersection_left + intersection_right) * 0.5,
                (intersection_top + intersection_bottom) * 0.5,
            );
            let group_resolved = layout
                .resolve_drop(point, group_source, &rects, &metrics)
                .unwrap()
                .expect("the overlapping Group tab-strip append zone must resolve");
            assert_eq!(
                group_resolved,
                group_append_zone.target,
                "a complete Group appends to the strip inside its actual overlap with the side band"
            );

            let panel_resolved = layout
                .resolve_drop(point, panel_source, &rects, &metrics)
                .unwrap()
                .expect("an individual Panel drag must retain the full-height side target");
            assert_eq!(
                panel_resolved,
                zone.target,
                "individual Panel drags keep the established Floating side-zone behavior"
            );
        }
        assert!(
            overlapping_group_strips > 0,
            "each outer side band must overlap tab strips in its own boundary Column"
        );
    }

    let point = Point::new(
        right_side.rect.origin.x + right_side.rect.size.width * 0.5,
        bounds.origin.y + bounds.size.height * 0.5,
    );
    let request = crate::DragDropRequest {
        payload: crate::DragPayload::Group { group: source_group },
        handle: crate::DragHandle::GroupGrip,
        origin: Point::ZERO,
        point,
        target: Some(right_side.target),
        floating_bounds: None,
    };
    assert_eq!(
        layout.resolve_drag_drop(&request, &metrics).unwrap(),
        crate::DragDropAction::Command(DockCommand::SplitGroup {
            group: source_group,
            target_group: right_second_group,
            zone: DropZone::Right,
            ratio: 0.5,
        })
    );

    layout.apply(
        DockCommand::SplitGroup {
            group: source_group,
            target_group: right_second_group,
            zone: DropZone::Right,
            ratio: 0.5,
        },
        &metrics,
    ).unwrap();

    let target_area = layout.floating_group(target_float).unwrap().area;
    let resulting_root = layout.dock_area(target_area).unwrap().root;
    let Some(crate::Node::Split(split)) = layout.node(resulting_root) else {
        panic!("the Floating drop must create a horizontal Split(X) of Columns");
    };
    assert_eq!(split.axis, Axis::X);
    assert_eq!(split.children.len(), 3);
    assert_eq!(&split.children[..2], &[left_column, right_column]);
    let inserted_column_id = *split
        .children
        .last()
        .expect("the new right-side Column should be appended");
    let Some(crate::Node::Column(inserted_column)) = layout.node(inserted_column_id) else {
        panic!("the dropped Group must be wrapped in a new Column");
    };
    assert_eq!(inserted_column.groups, vec![source_group]);
    assert!(layout.floating_group(target_float).is_some());
    assert_eq!(layout.validate(&metrics), Ok(()));
}

#[test]
fn a_single_group_with_multiple_tabs_keeps_panel_tab_drop_semantics() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);

    let (_, first_node) =
        builder.panel(key("multi-tab-simple-first"), constraints());
    let (source_panel, second_node) =
        builder.panel(key("multi-tab-simple-second"), constraints());
    let source_group = builder.tabs(vec![first_node, second_node], 0, false);
    let source_column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        vec![source_group],
        vec![1.0],
    );
    builder.main_area(DockSide::Left, source_column);

    let (_, target_node) =
        builder.panel(key("multi-tab-simple-target"), constraints());
    let (_, target_second_node) =
        builder.panel(key("multi-tab-simple-target-second"), constraints());
    let target_group = builder.tabs(vec![target_node, target_second_node], 0, false);
    let target_float = builder.floating_group(
        target_group,
        Rect::new(Point::new(520.0, 100.0), Size::new(420.0, 320.0)),
        1,
    );

    let layout = builder.build();
    let rects = layout.compute_floating_rects(target_float, &metrics).unwrap();
    let zones = layout
        .compute_drop_zones(
            DropSource::Panel { panel: source_panel },
            &rects,
            &metrics,
        )
        .unwrap();

    assert!(zones.iter().any(|zone| {
        zone.zone == DropZone::TabInsertion
            && zone.target == (DropTarget::Tabs {
                group: target_group,
                index: 2,
            })
    }));
    assert!(!zones.iter().any(|zone| {
        matches!(
            zone.target,
            DropTarget::GroupTabs { group, .. } if group == target_group
        )
    }));
}

#[test]
fn simple_composition_sources_share_identical_drop_zones_in_floating() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);

    let (source_panel, source_panel_node) =
        builder.panel(key("main-floating-group-source"), constraints());
    let source_group = builder.tabs(vec![source_panel_node], 0, false);
    let source_column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        vec![source_group],
        vec![1.0],
    );
    builder.main_area(DockSide::Left, source_column);

    let (_, main_target_panel) = builder.panel(key("main-center-target"), constraints());
    let main_target_group = builder.tabs(vec![main_target_panel], 0, false);
    let main_target_column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        vec![main_target_group],
        vec![1.0],
    );
    builder.main_area(DockSide::Right, main_target_column);

    let (_, first_panel) = builder.panel(key("main-floating-target-first"), constraints());
    let first_group = builder.tabs(vec![first_panel], 0, false);
    let (_, second_panel) = builder.panel(key("main-floating-target-second"), constraints());
    let second_group = builder.tabs(vec![second_panel], 0, false);
    let (_, third_panel) = builder.panel(key("main-floating-target-third"), constraints());
    let third_group = builder.tabs(vec![third_panel], 0, false);
    let target_column = builder.column(
        metrics.default_column_width,
        ColumnMode::Expanded,
        vec![first_group, second_group, third_group],
        vec![0.34, 0.33, 0.33],
    );
    let target_float = builder.floating_group(
        target_column,
        Rect::new(Point::new(620.0, 100.0), Size::new(420.0, 620.0)),
        2,
    );

    let layout = builder.build();
    let rects = layout.compute_floating_rects(target_float, &metrics).unwrap();
    let source = DropSource::Composition {
        root: source_column,
    };
    let zones = layout.compute_drop_zones(source, &rects, &metrics).unwrap();
    let group_zones = layout
        .compute_drop_zones(DropSource::Group { group: source_group }, &rects, &metrics)
        .unwrap();
    let panel_zones = layout
        .compute_drop_zones(DropSource::Panel { panel: source_panel }, &rects, &metrics)
        .unwrap();
    assert_eq!(
        zones, group_zones,
        "Main→Floating GroupGrip must use the same simple-composition zones"
    );
    assert_eq!(
        zones, panel_zones,
        "Main→Floating PanelTab must use the same simple-composition zones"
    );
    assert!(zones.iter().any(|zone| {
        zone.zone == DropZone::TabInsertion
            && zone.target == DropTarget::GroupTabs {
                group: first_group,
                index: 1,
            }
    }), "the simple composition must be able to merge into another Floating group");
    assert!(zones.iter().any(|zone| {
        zone.zone == DropZone::FloatingCenter
            && zone.target == DropTarget::GroupTabs {
                group: first_group,
                index: 1,
            }
    }), "the simple composition must expose Floating group-center docking");
    let main_rects = layout
        .compute_rects(
            Rect::new(Point::ZERO, Size::new(1200.0, 800.0)),
            &metrics,
        )
        .unwrap();
    let main_zones = layout
        .compute_drop_zones(source, &main_rects, &metrics)
        .unwrap();
    let main_group_zones = layout
        .compute_drop_zones(DropSource::Group { group: source_group }, &main_rects, &metrics)
        .unwrap();
    let main_panel_zones = layout
        .compute_drop_zones(DropSource::Panel { panel: source_panel }, &main_rects, &metrics)
        .unwrap();
    assert_eq!(main_zones, main_group_zones);
    assert_eq!(main_zones, main_panel_zones);
    assert!(main_zones.iter().any(|zone| {
        zone.zone == DropZone::Center
            && zone.target == DropTarget::GroupTabs {
                group: main_target_group,
                index: 1,
            }
    }), "the simple composition must expose Main group-center docking");

    let target_column_rect = rects
        .columns
        .iter()
        .find(|entry| entry.column == target_column)
        .expect("the target Floating Column should have computed bounds")
        .rect;

    let top = zones
        .iter()
        .find(|zone| {
            zone.zone == DropZone::FloatingTop
                && zone.target
                    == DropTarget::GroupEdge {
                        group: first_group,
                        zone: DropZone::Top,
                    }
        })
        .expect("a single-group Main composition must expose the Floating top edge");

    let between_groups = zones
        .iter()
        .find(|zone| {
            zone.zone == DropZone::Bottom
                && zone.target
                    == DropTarget::GroupEdge {
                        group: first_group,
                        zone: DropZone::Bottom,
                    }
        })
        .expect("a single-group Main composition must expose the boundary between Floating groups");

    let splitter = rects
        .splitters
        .iter()
        .find(|splitter| {
            splitter.owner == target_column
                && splitter.axis == Axis::Y
                && splitter.boundary_index == 0
        })
        .expect("the target Floating Column should have a splitter after its first Group");
    let splitter_center = Point::new(
        splitter.rect.origin.x + splitter.rect.size.width * 0.5,
        splitter.rect.origin.y + splitter.rect.size.height * 0.5,
    );
    assert_eq!(between_groups.rect.origin.x, target_column_rect.origin.x);
    assert_eq!(between_groups.rect.size.width, target_column_rect.size.width);
    assert!(between_groups.rect.contains(splitter_center).unwrap());
    assert!(zones.iter().any(|zone| {
        zone.zone == DropZone::Top
            && zone.target
                == DropTarget::GroupEdge {
                    group: second_group,
                    zone: DropZone::Top,
                }
    }), "the second Group must expose the same top edge as it would for a Group drag");

    let bottom = zones
        .iter()
        .find(|zone| {
            zone.zone == DropZone::ColumnEnd
                && zone.target
                    == DropTarget::ColumnSlot {
                        column: target_column,
                        index: 3,
                    }
        })
        .expect("a simple Main composition must expose the Floating Column end slot");

    let resolver = layout
        .prepare_drop_resolver(source, &rects, &metrics)
        .unwrap();
    for zone in [top, between_groups, bottom] {
        let point = Point::new(
            zone.rect.origin.x + zone.rect.size.width * 0.5,
            zone.rect.origin.y + zone.rect.size.height * 0.5,
        );
        let resolved = resolver
            .resolve_zone(point)
            .unwrap()
            .expect("each Floating vertical target must resolve");
        assert_eq!(resolved.target, zone.target);
        assert_eq!(resolved.zone, zone.zone);
    }
}


fn add_test_column(
    builder: &mut LayoutBuilder,
    prefix: &str,
    group_sizes: &[usize],
) -> (crate::NodeId, Vec<crate::NodeId>, Vec<crate::PanelId>) {
    let mut groups = Vec::with_capacity(group_sizes.len());
    let mut panels = Vec::new();
    for (group_index, panel_count) in group_sizes.iter().copied().enumerate() {
        let mut panel_nodes = Vec::with_capacity(panel_count);
        for panel_index in 0..panel_count {
            let (panel, node) = builder.panel(
                key(&format!("{prefix}-{group_index}-{panel_index}")),
                constraints(),
            );
            panels.push(panel);
            panel_nodes.push(node);
        }
        groups.push(builder.tabs(panel_nodes, 0, false));
    }
    let ratios = if groups.is_empty() {
        Vec::new()
    } else {
        vec![1.0 / groups.len() as f32; groups.len()]
    };
    let column = builder.column(
        280.0,
        ColumnMode::Expanded,
        groups.clone(),
        ratios,
    );
    (column, groups, panels)
}

#[test]
fn photoshop_floating_internal_splitters_expose_a_zone_for_every_column_boundary() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);

    let (source_column, source_groups, source_panels) =
        add_test_column(&mut builder, "internal-boundary-source", &[1]);
    builder.main_area(DockSide::Left, source_column);

    let (left_column, left_groups, _) =
        add_test_column(&mut builder, "internal-boundary-left", &[1, 1]);
    let (middle_column, middle_groups, _) =
        add_test_column(&mut builder, "internal-boundary-middle", &[1, 1]);
    let (right_column, right_groups, _) =
        add_test_column(&mut builder, "internal-boundary-right", &[1, 1]);
    let target_root = builder.split(
        Axis::X,
        vec![left_column, middle_column, right_column],
        vec![0.2, 0.3, 0.5],
    );
    let target_float = builder.floating_group(
        target_root,
        Rect::new(Point::new(500.0, 80.0), Size::new(900.0, 620.0)),
        2,
    );
    let layout = builder.build();

    let rects = layout.compute_floating_rects(target_float, &metrics).unwrap();
    let splitters: Vec<_> = rects
        .splitters
        .iter()
        .filter(|splitter| splitter.owner == target_root && splitter.axis == Axis::X)
        .collect();
    assert_eq!(splitters.len(), 2, "each adjacent Column pair has one splitter");

    let sources = [
        DropSource::Composition {
            root: source_column,
        },
        DropSource::Group {
            group: source_groups[0],
        },
        DropSource::Panel {
            panel: source_panels[0],
        },
    ];
    let expected_groups = [middle_groups[0], right_groups[0]];
    let mut reference_zones = None;

    for source in sources {
        let zones = layout
            .compute_drop_zones(source, &rects, &metrics)
            .unwrap();
        let mut internal_zones: Vec<_> = zones
            .iter()
            .copied()
            .filter(|zone| zone.zone == DropZone::FloatingColumnInsertion)
            .collect();
        internal_zones.sort_by(|left, right| {
            left.rect
                .origin
                .x
                .partial_cmp(&right.rect.origin.x)
                .unwrap()
        });
        assert_eq!(
            internal_zones.len(),
            2,
            "there must be one internal zone per Split(X) boundary"
        );

        if let Some(reference) = &reference_zones {
            assert_eq!(
                &internal_zones, reference,
                "Panel, Group and Composition sources share the same internal boundary geometry"
            );
        } else {
            reference_zones = Some(internal_zones.clone());
        }

        for (boundary_index, zone) in internal_zones.iter().enumerate() {
            assert_eq!(
                zone.target,
                DropTarget::GroupEdge {
                    group: expected_groups[boundary_index],
                    zone: DropZone::FloatingColumnInsertion,
                }
            );
            let splitter = splitters
                .iter()
                .find(|splitter| splitter.boundary_index == boundary_index)
                .unwrap();
            let splitter_center = Point::new(
                splitter.rect.origin.x + splitter.rect.size.width * 0.5,
                splitter.rect.origin.y + splitter.rect.size.height * 0.5,
            );
            assert!(zone.rect.contains(splitter_center).unwrap());
            assert_eq!(zone.rect.origin.y, splitter.rect.origin.y);
            assert_eq!(zone.rect.size.height, splitter.rect.size.height);
            assert!(zone.rect.size.width >= metrics.edge_drop_width);

            assert_eq!(
                layout
                    .resolve_drop(splitter_center, source, &rects, &metrics)
                    .unwrap(),
                Some(zone.target),
                "hovering the physical splitter must resolve to its insertion target"
            );
        }
    }

    assert_eq!(left_groups.len(), 2);
    assert_eq!(middle_groups.len(), 2);
}

#[test]
fn photoshop_inserts_a_column_between_floating_columns_and_preserves_other_ratios() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);

    let (source_column, _, _) =
        add_test_column(&mut builder, "insert-column-source", &[1]);
    builder.main_area(DockSide::Left, source_column);

    let (left_column, _, _) = add_test_column(&mut builder, "insert-column-left", &[1]);
    let (middle_column, middle_groups, _) =
        add_test_column(&mut builder, "insert-column-middle", &[1]);
    let (right_column, _, _) = add_test_column(&mut builder, "insert-column-right", &[1]);
    let target_root = builder.split(
        Axis::X,
        vec![left_column, middle_column, right_column],
        vec![0.2, 0.3, 0.5],
    );
    let target_float = builder.floating_group(
        target_root,
        Rect::new(Point::new(500.0, 80.0), Size::new(900.0, 620.0)),
        2,
    );
    let mut layout = builder.build();

    let rects = layout.compute_floating_rects(target_float, &metrics).unwrap();
    let zones = layout
        .compute_drop_zones(
            DropSource::Composition {
                root: source_column,
            },
            &rects,
            &metrics,
        )
        .unwrap();
    let zone = zones
        .iter()
        .find(|zone| {
            zone.zone == DropZone::FloatingColumnInsertion
                && zone.target
                    == DropTarget::GroupEdge {
                        group: middle_groups[0],
                        zone: DropZone::FloatingColumnInsertion,
                    }
        })
        .copied()
        .expect("the splitter before the middle Column must expose an insertion zone");
    let point = Point::new(
        zone.rect.origin.x + zone.rect.size.width * 0.5,
        zone.rect.origin.y + zone.rect.size.height * 0.5,
    );
    let request = crate::DragDropRequest {
        payload: crate::DragPayload::DockArea {
            side: DockSide::Left,
        },
        handle: crate::DragHandle::DockAreaHeader,
        origin: Point::ZERO,
        point,
        target: Some(zone.target),
        floating_bounds: None,
    };
    assert_eq!(
        layout.resolve_drag_drop(&request, &metrics).unwrap(),
        crate::DragDropAction::Command(DockCommand::MoveColumnToTarget {
            column: source_column,
            target: zone.target,
        })
    );

    layout
        .apply(
            DockCommand::MoveColumnToTarget {
                column: source_column,
                target: zone.target,
            },
            &metrics,
        )
        .unwrap();

    let floating_area = layout.floating_group(target_float).unwrap().area;
    let floating_root = layout.dock_area(floating_area).unwrap().root;
    let Some(crate::Node::Split(split)) = layout.node(floating_root) else {
        panic!("the Floating composition must remain a horizontal Split(X)");
    };
    assert_eq!(
        split.children,
        vec![left_column, source_column, middle_column, right_column]
    );
    assert!((split.ratios[0] - 0.2).abs() < 1.0e-5);
    assert!((split.ratios[1] - 0.15).abs() < 1.0e-5);
    assert!((split.ratios[2] - 0.15).abs() < 1.0e-5);
    assert!((split.ratios[3] - 0.5).abs() < 1.0e-5);
    assert_eq!(layout.validate(&metrics), Ok(()));
}

#[test]
fn photoshop_group_and_panel_drops_create_columns_at_floating_splitters() {
    let metrics = DockMetrics::default();

    let mut group_builder = LayoutBuilder::new(Profile::Photoshop);
    let (source_column, source_groups, _) =
        add_test_column(&mut group_builder, "group-internal-source", &[2]);
    builder_main_area(&mut group_builder, source_column);
    let (left_column, _, _) =
        add_test_column(&mut group_builder, "group-internal-left", &[1]);
    let (right_column, right_groups, _) =
        add_test_column(&mut group_builder, "group-internal-right", &[1]);
    let group_target_root = group_builder.split(Axis::X, vec![left_column, right_column], vec![0.4, 0.6]);
    let group_target_float = group_builder.floating_group(
        group_target_root,
        Rect::new(Point::new(520.0, 90.0), Size::new(620.0, 500.0)),
        2,
    );
    let mut group_layout = group_builder.build();
    let group_rects = group_layout
        .compute_floating_rects(group_target_float, &metrics)
        .unwrap();
    let group_zones = group_layout
        .compute_drop_zones(
            DropSource::Group {
                group: source_groups[0],
            },
            &group_rects,
            &metrics,
        )
        .unwrap();
    let group_zone = group_zones
        .iter()
        .find(|zone| zone.zone == DropZone::FloatingColumnInsertion)
        .copied()
        .expect("a Group drag must expose the internal Floating splitter");
    let group_point = Point::new(
        group_zone.rect.origin.x + group_zone.rect.size.width * 0.5,
        group_zone.rect.origin.y + group_zone.rect.size.height * 0.5,
    );
    let group_request = crate::DragDropRequest {
        payload: crate::DragPayload::Group {
            group: source_groups[0],
        },
        handle: crate::DragHandle::GroupGrip,
        origin: Point::ZERO,
        point: group_point,
        target: Some(group_zone.target),
        floating_bounds: None,
    };
    let group_command = DockCommand::SplitGroup {
        group: source_groups[0],
        target_group: right_groups[0],
        zone: DropZone::FloatingColumnInsertion,
        ratio: 0.5,
    };
    assert_eq!(
        group_layout.resolve_drag_drop(&group_request, &metrics).unwrap(),
        crate::DragDropAction::Command(group_command.clone())
    );
    group_layout.apply(group_command, &metrics).unwrap();
    let group_target_area = group_layout.floating_group(group_target_float).unwrap().area;
    let group_root = group_layout.dock_area(group_target_area).unwrap().root;
    let Some(crate::Node::Split(group_split)) = group_layout.node(group_root) else {
        panic!("the Group drop must preserve a horizontal Floating composition");
    };
    assert_eq!(group_split.children.len(), 3);
    let Some(crate::Node::Column(inserted_group_column)) =
        group_layout.node(group_split.children[1])
    else {
        panic!("the dropped Group must be wrapped in a new Column");
    };
    assert_eq!(inserted_group_column.groups, vec![source_groups[0]]);
    assert_eq!(group_layout.validate(&metrics), Ok(()));

    let mut panel_builder = LayoutBuilder::new(Profile::Photoshop);
    let (source_column, source_groups, source_panels) =
        add_test_column(&mut panel_builder, "panel-internal-source", &[2]);
    builder_main_area(&mut panel_builder, source_column);
    let (left_column, _, _) =
        add_test_column(&mut panel_builder, "panel-internal-left", &[1]);
    let (right_column, right_groups, _) =
        add_test_column(&mut panel_builder, "panel-internal-right", &[1]);
    let panel_target_root =
        panel_builder.split(Axis::X, vec![left_column, right_column], vec![0.4, 0.6]);
    let panel_target_float = panel_builder.floating_group(
        panel_target_root,
        Rect::new(Point::new(520.0, 90.0), Size::new(620.0, 500.0)),
        2,
    );
    let mut panel_layout = panel_builder.build();
    let panel_rects = panel_layout
        .compute_floating_rects(panel_target_float, &metrics)
        .unwrap();
    let panel_zones = panel_layout
        .compute_drop_zones(
            DropSource::Panel {
                panel: source_panels[0],
            },
            &panel_rects,
            &metrics,
        )
        .unwrap();
    let panel_zone = panel_zones
        .iter()
        .find(|zone| zone.zone == DropZone::FloatingColumnInsertion)
        .copied()
        .expect("a Panel drag must expose the internal Floating splitter");
    let panel_point = Point::new(
        panel_zone.rect.origin.x + panel_zone.rect.size.width * 0.5,
        panel_zone.rect.origin.y + panel_zone.rect.size.height * 0.5,
    );
    let panel_request = crate::DragDropRequest {
        payload: crate::DragPayload::Panel {
            panel: source_panels[0],
        },
        handle: crate::DragHandle::PanelTab,
        origin: Point::ZERO,
        point: panel_point,
        target: Some(panel_zone.target),
        floating_bounds: None,
    };
    let panel_target = DropTarget::GroupEdge {
        group: right_groups[0],
        zone: DropZone::FloatingColumnInsertion,
    };
    let panel_command = DockCommand::MovePanel {
        panel: source_panels[0],
        target: panel_target,
    };
    assert_eq!(
        panel_layout.resolve_drag_drop(&panel_request, &metrics).unwrap(),
        crate::DragDropAction::Command(panel_command.clone())
    );
    panel_layout.apply(panel_command, &metrics).unwrap();
    let panel_target_area = panel_layout.floating_group(panel_target_float).unwrap().area;
    let panel_root = panel_layout.dock_area(panel_target_area).unwrap().root;
    let Some(crate::Node::Split(panel_split)) = panel_layout.node(panel_root) else {
        panic!("the Panel drop must preserve a horizontal Floating composition");
    };
    assert_eq!(panel_split.children.len(), 3);
    let Some(crate::Node::Column(inserted_panel_column)) =
        panel_layout.node(panel_split.children[1])
    else {
        panic!("the dropped Panel must be wrapped in a new Column");
    };
    assert_eq!(inserted_panel_column.groups.len(), 1);
    assert!(matches!(
        panel_layout.node(inserted_panel_column.groups[0]),
        Some(crate::Node::Tabs(tabs)) if tabs.children.len() == 1
    ));
    assert!(matches!(
        panel_layout.node(source_groups[0]),
        Some(crate::Node::Tabs(tabs))
            if tabs.children.len() == 1
                && matches!(
                    panel_layout.node(tabs.children[0]),
                    Some(crate::Node::Panel(panel)) if panel.panel_id == source_panels[1]
                )
    ));
    assert_eq!(panel_layout.validate(&metrics), Ok(()));
}

fn builder_main_area(builder: &mut LayoutBuilder, column: crate::NodeId) {
    builder.main_area(DockSide::Left, column);
}

#[test]
fn photoshop_docks_a_floating_column_at_an_internal_splitter() {
    let metrics = DockMetrics::default();
    let mut builder = LayoutBuilder::new(Profile::Photoshop);

    let (source_column, _, _) =
        add_test_column(&mut builder, "floating-source-column", &[1, 1]);
    let source_float = builder.floating_group(
        source_column,
        Rect::new(Point::new(40.0, 80.0), Size::new(320.0, 520.0)),
        1,
    );

    let (left_column, _, _) =
        add_test_column(&mut builder, "floating-target-left", &[1]);
    let (middle_column, _, _) =
        add_test_column(&mut builder, "floating-target-middle", &[1]);
    let (right_column, _, _) =
        add_test_column(&mut builder, "floating-target-right", &[1]);
    let target_root = builder.split(
        Axis::X,
        vec![left_column, middle_column, right_column],
        vec![0.2, 0.3, 0.5],
    );
    let target_float = builder.floating_group(
        target_root,
        Rect::new(Point::new(500.0, 80.0), Size::new(900.0, 620.0)),
        2,
    );
    let mut layout = builder.build();

    let rects = layout.compute_floating_rects(target_float, &metrics).unwrap();
    let zones = layout
        .compute_drop_zones(
            DropSource::Composition {
                root: source_column,
            },
            &rects,
            &metrics,
        )
        .unwrap();
    let zone = zones
        .iter()
        .find(|zone| zone.zone == DropZone::FloatingColumnInsertion)
        .copied()
        .expect("the target Floating composition must expose its internal splitter");
    let point = Point::new(
        zone.rect.origin.x + zone.rect.size.width * 0.5,
        zone.rect.origin.y + zone.rect.size.height * 0.5,
    );
    let request = crate::DragDropRequest {
        payload: crate::DragPayload::FloatingGroup { group: source_float },
        handle: crate::DragHandle::FloatingCapture,
        origin: Point::ZERO,
        point,
        target: Some(zone.target),
        floating_bounds: None,
    };
    let action = layout.resolve_drag_drop(&request, &metrics).unwrap();
    assert_eq!(
        action,
        crate::DragDropAction::Command(DockCommand::DockFloating {
            group: source_float,
            target: zone.target,
        })
    );
    layout
        .apply(
            DockCommand::DockFloating {
                group: source_float,
                target: zone.target,
            },
            &metrics,
        )
        .unwrap();

    assert!(layout.floating_group(source_float).is_none());
    assert!(layout.floating_group(target_float).is_some());
    let target_area = layout.floating_group(target_float).unwrap().area;
    let floating_root = layout.dock_area(target_area).unwrap().root;
    let Some(crate::Node::Split(split)) = layout.node(floating_root) else {
        panic!("the merged Floating composition must remain Split(X)");
    };
    assert_eq!(
        split.children,
        vec![left_column, source_column, middle_column, right_column]
    );
    assert_eq!(layout.validate(&metrics), Ok(()));
}
